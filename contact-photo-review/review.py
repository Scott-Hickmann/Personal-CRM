"""Persistent review state, bound to concrete Contacts IDs and cached image bytes."""
import hashlib
import json
import sqlite3
import subprocess
import uuid
from pathlib import Path

import search


class Contacts:
    def __init__(self, executable):
        self.executable = executable

    def call(self, command, payload):
        result = subprocess.run([str(self.executable), command], input=json.dumps(payload),
                                capture_output=True, text=True, timeout=120)
        if result.returncode:
            raise ValueError(result.stderr.strip() or "Contacts helper failed")
        return json.loads(result.stdout)


class Review:
    def __init__(self, directory, contacts, crawler=search.search, downloader=search.download):
        self.directory = Path(directory)
        self.directory.mkdir(parents=True, exist_ok=True, mode=0o700)
        for name in ("images", "originals", "backups"):
            (self.directory / name).mkdir(exist_ok=True, mode=0o700)
        self.contacts, self.crawler, self.downloader = contacts, crawler, downloader
        self.db = sqlite3.connect(self.directory / "review.sqlite3", check_same_thread=False)
        self.db.row_factory = sqlite3.Row
        self.db.executescript("""
            CREATE TABLE IF NOT EXISTS people(id TEXT PRIMARY KEY, data TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'pending', query TEXT NOT NULL, page INTEGER NOT NULL DEFAULT 0);
            CREATE TABLE IF NOT EXISTS candidates(id TEXT PRIMARY KEY, person TEXT NOT NULL,
                url TEXT NOT NULL, source TEXT NOT NULL, title TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'pending', hash TEXT, error TEXT,
                UNIQUE(person, url));
            CREATE TABLE IF NOT EXISTS approvals(id TEXT PRIMARY KEY, person TEXT NOT NULL,
                candidate TEXT NOT NULL, backup TEXT NOT NULL, state TEXT NOT NULL,
                created TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP, error TEXT);
        """)
        if "crop_version" not in {row[1] for row in self.db.execute("PRAGMA table_info(candidates)")}:
            with self.db:
                self.db.execute("ALTER TABLE candidates ADD COLUMN crop_version INTEGER NOT NULL DEFAULT 0")
        if "crop_data" not in {row[1] for row in self.db.execute("PRAGMA table_info(candidates)")}:
            with self.db:
                self.db.execute("ALTER TABLE candidates ADD COLUMN crop_data TEXT")

    def refresh(self):
        result = self.contacts.call("list", {})
        with self.db:
            present = {person["id"] for person in result["contacts"]}
            for row in self.db.execute("SELECT id FROM people WHERE status IN ('pending','skipped')").fetchall():
                if row["id"] not in present:
                    self.db.execute("UPDATE people SET status='unavailable' WHERE id=?", (row["id"],))
            for person in result["contacts"]:
                query = '"' + person["name"].replace('"', '') + '" ' + person["organization"] + " portrait"
                self.db.execute("""INSERT INTO people(id,data,query) VALUES(?,?,?)
                    ON CONFLICT(id) DO UPDATE SET data=excluded.data,
                    status=CASE WHEN people.status IN ('unavailable','saved') THEN 'pending' ELSE people.status END""",
                                (person["id"], json.dumps(person), query.strip()))
        return self.queue()

    def queue(self):
        people = [{**json.loads(row["data"]), "status": row["status"], "query": row["query"]}
                  for row in self.db.execute("SELECT * FROM people WHERE status IN ('pending','skipped') ORDER BY json_extract(data,'$.name')")]
        saved = self.db.execute("SELECT count(*) FROM approvals WHERE state='saved'").fetchone()[0]
        return {"contacts": people, "saved": saved}

    def person(self, person_id):
        person = self.db.execute("SELECT * FROM people WHERE id=? AND status='pending'", (person_id,)).fetchone()
        if not person:
            raise ValueError("Contact is no longer pending; refresh the queue")
        return person

    def candidate(self, person_id, query=None):
        person = self.person(person_id)
        if query is not None:
            query = query.strip()
            if not query or len(query) > 300:
                raise ValueError("Enter a search between 1 and 300 characters")
            with self.db:
                self.db.execute("UPDATE people SET query=?,page=0 WHERE id=?", (query, person_id))
                self.db.execute("UPDATE candidates SET status='unused' WHERE person=? AND status='pending'", (person_id,))
            person = self.person(person_id)
        failures = 0
        # Bounded work per request; another click can continue pagination.
        for _ in range(2):
            rows = self.db.execute("SELECT * FROM candidates WHERE person=? AND status='pending' ORDER BY rowid", (person_id,)).fetchall()
            for row in rows:
                try:
                    path = self.directory / "images" / (row["id"] + ".jpg")
                    original = self.directory / "originals" / path.name
                    if not row["hash"] or not path.exists() or row["crop_version"] != 2 or not original.exists():
                        raw = path.with_suffix(".download")
                        try:
                            raw.write_bytes(self.downloader(row["url"]))
                            metadata = self.contacts.call("normalize", {"input": str(raw), "output": str(path), "original": str(original)})
                        finally:
                            raw.unlink(missing_ok=True)
                        digest = hashlib.sha256(path.read_bytes()).hexdigest()
                        duplicate = self.db.execute("""SELECT 1 FROM candidates WHERE person=? AND status='rejected'
                            AND (hash=? OR json_extract(crop_data,'$.original_sha256')=?)""",
                                                    (person_id, digest, metadata["original_sha256"])).fetchone()
                        if duplicate:
                            raise ValueError("This rejected image also appeared at another URL")
                        with self.db:
                            self.db.execute("UPDATE candidates SET hash=?,crop_version=2,crop_data=? WHERE id=?",
                                            (digest, json.dumps(metadata), row["id"]))
                    return self.preview(row["id"], person)
                except (ValueError, OSError, subprocess.SubprocessError) as error:
                    with self.db:
                        self.db.execute("UPDATE candidates SET status='failed',error=? WHERE id=?", (str(error), row["id"]))
                    failures += 1
                    if failures >= 4:
                        raise ValueError(f"Four candidates could not supply a clear single-face photo. Last issue: {error}. Try more candidates or refine the search.") from error
            page = self.person(person_id)["page"]
            if page >= 10:
                raise ValueError("Reached the search limit. Refine the query to find more candidates.")
            results = self.crawler(person["query"], page)
            with self.db:
                for item in results:
                    self.db.execute("""INSERT INTO candidates(id,person,url,source,title) VALUES(?,?,?,?,?)
                        ON CONFLICT(person,url) DO UPDATE SET status='pending' WHERE candidates.status='unused'""",
                                    (uuid.uuid4().hex, person_id, item["url"], item["source"], item["title"]))
                self.db.execute("UPDATE people SET page=page+1 WHERE id=?", (person_id,))
        raise ValueError("No new candidates on these pages. Try more candidates or refine the search.")

    def decide(self, person_id, candidate_id, approved, expected_hash=None):
        person = self.person(person_id)
        row = self.db.execute("SELECT * FROM candidates WHERE id=? AND person=? AND status='pending' AND hash IS NOT NULL",
                              (candidate_id, person_id)).fetchone()
        if not row:
            raise ValueError("Candidate is stale or has not been previewed")
        if not approved:
            with self.db:
                self.db.execute("UPDATE candidates SET status='rejected' WHERE id=?", (candidate_id,))
            return {"rejected": True}
        if row["crop_version"] != 2 or expected_hash != row["hash"]:
            raise ValueError("Photo crop changed; reload and review the current crop before saving")
        image = self.directory / "images" / (candidate_id + ".jpg")
        if hashlib.sha256(image.read_bytes()).hexdigest() != row["hash"]:
            raise ValueError("Cached photo changed; refusing to save")
        approval_id = uuid.uuid4().hex
        backup = self.directory / "backups" / (approval_id + ".vcf")
        with self.db:
            self.db.execute("INSERT INTO approvals(id,person,candidate,backup,state) VALUES(?,?,?,?,'saving')",
                            (approval_id, person_id, candidate_id, str(backup)))
        try:
            result = self.contacts.call("approve", {"id": person_id, "fingerprint": json.loads(person["data"])["fingerprint"],
                "image": str(image), "sha256": row["hash"], "backup": str(backup)})
            if result.get("saved") is not True:
                raise ValueError("Contacts did not confirm the save")
        except Exception as error:
            with self.db:
                self.db.execute("UPDATE approvals SET state='uncertain',error=? WHERE id=?", (str(error), approval_id))
            raise ValueError(f"Save was not confirmed: {error}. Refresh before retrying; an existing photo will never be overwritten.") from error
        with self.db:
            self.db.execute("UPDATE approvals SET state='saved' WHERE id=?", (approval_id,))
            self.db.execute("UPDATE candidates SET status='approved' WHERE id=?", (candidate_id,))
            self.db.execute("UPDATE people SET status='saved' WHERE id=?", (person_id,))
        return {"saved": True}

    def preview(self, candidate_id, person):
        row = self.db.execute("SELECT * FROM candidates WHERE id=?", (candidate_id,)).fetchone()
        return {"id": row["id"], "person": row["person"], "source": row["source"], "title": row["title"],
                "image": "/images/" + row["id"] + ".jpg", "query": person["query"], "sha256": row["hash"],
                "original": "/originals/" + row["id"] + ".jpg", "framing": json.loads(row["crop_data"])}

    def recrop(self, person_id, candidate_id, expected_hash, crop):
        person = self.person(person_id)
        row = self.db.execute("SELECT * FROM candidates WHERE id=? AND person=? AND status='pending'",
                              (candidate_id, person_id)).fetchone()
        if not row or row["crop_version"] != 2 or not expected_hash or row["hash"] != expected_hash:
            raise ValueError("Photo changed; reload before adjusting the crop")
        metadata = json.loads(row["crop_data"])
        if not isinstance(crop, dict) or any(type(crop.get(key)) is not int for key in ("x", "y", "size")):
            raise ValueError("Crop coordinates must be whole pixels")
        x, y, size = (crop[key] for key in ("x", "y", "size"))
        if x < 0 or y < 0 or size < 96 or x + size > metadata["width"] or y + size > metadata["height"]:
            raise ValueError("Choose a square of at least 96 pixels inside the original photo")
        original = self.directory / "originals" / (candidate_id + ".jpg")
        if hashlib.sha256(original.read_bytes()).hexdigest() != metadata["original_sha256"]:
            raise ValueError("Original photo changed; reload before cropping")
        path = self.directory / "images" / original.name
        temporary = path.with_suffix(".recrop")
        try:
            self.contacts.call("recrop", {"input": str(original), "output": str(temporary),
                "original_sha256": metadata["original_sha256"], "x": str(x), "y": str(y), "size": str(size)})
            digest = hashlib.sha256(temporary.read_bytes()).hexdigest()
            temporary.replace(path)
        finally:
            temporary.unlink(missing_ok=True)
        metadata["crop"] = {"x": x, "y": y, "size": size}
        with self.db:
            self.db.execute("UPDATE candidates SET hash=?,crop_data=? WHERE id=?", (digest, json.dumps(metadata), candidate_id))
        return self.preview(candidate_id, person)

    def skip(self, person_id):
        self.person(person_id)
        with self.db:
            self.db.execute("UPDATE people SET status='skipped' WHERE id=?", (person_id,))
        return self.queue()

    def resume(self):
        with self.db:
            self.db.execute("UPDATE people SET status='pending' WHERE status='skipped'")
        return self.queue()
