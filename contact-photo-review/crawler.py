"""Discover candidates for the whole queue without blocking review requests."""
import sqlite3
import threading
import uuid


class Crawler:
    def __init__(self, review):
        self.review = review
        self.wakeup = threading.Event()
        self.status_lock = threading.Lock()
        self.status = "Waiting for contacts"
        threading.Thread(target=self.run, daemon=True).start()

    def describe(self):
        with self.status_lock:
            return self.status

    def report(self, message):
        with self.status_lock:
            self.status = message

    def run(self):
        db = sqlite3.connect(self.review.directory / "review.sqlite3", timeout=15)
        db.row_factory = sqlite3.Row
        while True:
            self.wakeup.wait()
            self.wakeup.clear()
            rows = db.execute("SELECT id,query FROM people WHERE status='pending' AND page=0").fetchall()
            failed = 0
            for index, person in enumerate(rows):
                self.report(f"Searching address book · {index + 1} of {len(rows)}")
                try:
                    results = self.review.crawler(person["query"], 0)
                    with db:
                        current = db.execute("SELECT query,page,status FROM people WHERE id=?", (person["id"],)).fetchone()
                        if current and current["query"] == person["query"] and current["page"] == 0 and current["status"] == "pending":
                            for item in results:
                                db.execute("INSERT OR IGNORE INTO candidates(id,person,url,source,title) VALUES(?,?,?,?,?)",
                                           (uuid.uuid4().hex, person["id"], item["url"], item["source"], item["title"]))
                            db.execute("UPDATE people SET page=1 WHERE id=?", (person["id"],))
                except Exception:
                    failed += 1  # Foreground review offers detailed errors and retry.
                # Pace public requests; the UI remains usable during discovery.
                threading.Event().wait(2)
            self.report(f"Search pass complete · {failed} need a retry" if failed else "Search pass complete")
