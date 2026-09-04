#!/usr/bin/env python3
"""Loopback-only review UI. All API reads and writes require a session token."""
import argparse
import fcntl
import json
import os
import secrets
import threading
import webbrowser
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import urlsplit

from review import Contacts, Review
from crawler import Crawler

ROOT = Path(__file__).resolve().parent


def handler(review, token, crawler=None):
    lock = threading.Lock()

    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *_args):
            pass  # Names, URLs, and the session secret do not belong in logs.

        def reply(self, code, body, kind="application/json"):
            if not isinstance(body, bytes):
                body = json.dumps(body).encode()
            self.send_response(code)
            self.send_header("Content-Type", kind)
            self.send_header("Content-Length", str(len(body)))
            self.send_header("Cache-Control", "no-store")
            self.send_header("X-Content-Type-Options", "nosniff")
            self.send_header("Referrer-Policy", "no-referrer")
            self.send_header("Content-Security-Policy", "default-src 'self'; img-src 'self' blob:; script-src 'self'; style-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'")
            self.end_headers()
            self.wfile.write(body)

        def authorized(self):
            host = f"127.0.0.1:{self.server.server_port}"
            if self.headers.get("Host") != host:
                self.reply(403, {"error": "Invalid host"})
                return False
            origin = self.headers.get("Origin")
            if origin and origin != "http://" + host:
                self.reply(403, {"error": "Invalid origin"})
                return False
            if not secrets.compare_digest(self.headers.get("X-Review-Token", ""), token):
                self.reply(403, {"error": "Open the session link printed by the app"})
                return False
            return True

        def do_GET(self):
            path = urlsplit(self.path).path
            static = {"/": ("index.html", "text/html; charset=utf-8"),
                      "/app.js": ("app.js", "text/javascript"), "/style.css": ("style.css", "text/css"),
                      "/favicon.svg": ("favicon.svg", "image/svg+xml")}
            if path in static:
                name, kind = static[path]
                self.reply(200, (ROOT / "static" / name).read_bytes(), kind)
                return
            if not self.authorized():
                return
            with lock:
                if path == "/api/queue":
                    self.reply(200, {**review.queue(), "demo": isinstance(review.contacts, DemoContacts),
                                     "crawl": crawler.describe() if crawler else "Demo search"})
                elif path.startswith("/images/"):
                    name = path.removeprefix("/images/")
                    if len(name) != 36 or not name.endswith(".jpg") or any(c not in "0123456789abcdef" for c in name[:-4]):
                        self.reply(404, {"error": "Unknown photo"})
                        return
                    file = review.directory / "images" / name
                    if not file.is_file():
                        self.reply(404, {"error": "Photo not found"})
                        return
                    kind = "image/svg+xml" if isinstance(review.contacts, DemoContacts) else "image/jpeg"
                    self.reply(200, file.read_bytes(), kind)
                else:
                    self.reply(404, {"error": "Not found"})

        def do_POST(self):
            if not self.authorized():
                return
            try:
                length = int(self.headers.get("Content-Length", "0"))
                if not 0 < length <= 4096 or self.headers.get("Content-Type") != "application/json":
                    raise ValueError("Expected a small JSON request")
                body = json.loads(self.rfile.read(length))
                if not isinstance(body, dict):
                    raise ValueError("Expected a JSON object")
                with lock:
                    path = urlsplit(self.path).path
                    if path == "/api/refresh":
                        result = review.refresh()
                        if crawler:
                            crawler.wakeup.set()
                    elif path == "/api/candidate":
                        result = review.candidate(str(body["person"]), body.get("query"))
                    elif path == "/api/decide":
                        if type(body.get("approved")) is not bool:
                            raise ValueError("Approval must be explicitly true or false")
                        result = review.decide(str(body["person"]), str(body["candidate"]), body["approved"])
                    elif path == "/api/skip":
                        result = review.skip(str(body["person"]))
                    elif path == "/api/resume":
                        result = review.resume()
                        if crawler:
                            crawler.wakeup.set()
                    else:
                        self.reply(404, {"error": "Unknown action"})
                        return
                self.reply(200, result)
            except Exception as error:
                self.reply(400, {"error": str(error)})

    return Handler


class DemoContacts:
    """Explicit, isolated demo; cannot call the native Contacts helper."""
    def __init__(self):
        self.people = [{"id": "demo-1", "name": "Alex Morgan", "organization": "Northstar Studio",
                        "job": "Designer", "email": "alex@example.com", "fingerprint": "demo"},
                       {"id": "demo-2", "name": "Jamie Chen", "organization": "Fieldwork",
                        "job": "Engineer", "email": "jamie@example.com", "fingerprint": "demo"}]

    def call(self, command, payload):
        if command == "list":
            return {"contacts": self.people, "total": len(self.people)}
        if command == "normalize":
            Path(payload["output"]).write_bytes(Path(payload["input"]).read_bytes())
            return {}
        if command == "approve":
            Path(payload["backup"]).write_text("Demo backup; no real contact was accessed.\n")
            self.people = [p for p in self.people if p["id"] != payload["id"]]
            return {"saved": True}
        raise ValueError("Unknown demo command")


def demo_search(query, page):
    return [{"url": f"https://example.com/{page}-{i}", "source": "https://example.com",
             "title": f"Illustrated demo candidate {page * 3 + i + 1}"} for i in range(3)]


def demo_download(url):
    color = ["#698f83", "#8e809e", "#b08264"][int(url[-1])]
    return f'<svg xmlns="http://www.w3.org/2000/svg" width="600" height="600"><rect width="600" height="600" fill="{color}"/><circle cx="300" cy="235" r="95" fill="#eee7da"/><path d="M110 600v-60a190 190 0 0 1 380 0v60" fill="#eee7da"/></svg>'.encode()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--demo", action="store_true", help="Use fake contacts; never access Apple Contacts")
    parser.add_argument("--no-open", action="store_true")
    parser.add_argument("--port", type=int, default=0, help="Local port; default chooses an available port")
    args = parser.parse_args()
    os.umask(0o077)
    directory = ROOT / ".local" / ("demo" if args.demo else "review")
    directory.mkdir(parents=True, exist_ok=True)
    instance = (directory / "instance.lock").open("w")
    try:
        fcntl.flock(instance, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except BlockingIOError:
        raise SystemExit("Photo review is already running. Use its existing browser window.")
    review = (Review(directory, DemoContacts(), demo_search, demo_download) if args.demo
              else Review(directory, Contacts(ROOT / ".local" / "contacts")))
    token = secrets.token_urlsafe(32)
    crawler = None if args.demo else Crawler(review)
    if crawler:
        crawler.wakeup.set()
    server = ThreadingHTTPServer(("127.0.0.1", args.port), handler(review, token, crawler))
    server.daemon_threads = True
    url = f"http://127.0.0.1:{server.server_port}/#" + token
    print(("DEMO — no real contacts\n" if args.demo else "") + "Photo review: " + url, flush=True)
    if not args.no_open:
        webbrowser.open(url)
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        print("\nReview stopped. Your progress is saved.")
    finally:
        server.server_close()
