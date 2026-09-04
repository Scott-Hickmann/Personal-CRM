import hashlib
import html
import http.client
import json
import socket
import tempfile
import threading
import unittest
from pathlib import Path
from unittest.mock import patch

from http.server import ThreadingHTTPServer
from review import Review
from crawler import Crawler
from search import ImageResults, download, public_url
from server import DemoContacts, handler


class FakeContacts(DemoContacts):
    def __init__(self):
        super().__init__()
        self.calls = []
        self.fail = False

    def call(self, command, payload):
        self.calls.append((command, payload))
        if command == "approve" and self.fail:
            raise ValueError("Contact already has a photo")
        return super().call(command, payload)


class ReviewTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.native = FakeContacts()
        self.searches = []

        def search(query, page):
            self.searches.append((query, page))
            prefix = hashlib.sha256(query.encode()).hexdigest()[:8]
            return [{"url": f"https://example.com/{prefix}/{page}-{i}", "source": "https://example.com", "title": "Candidate"} for i in range(3)]

        self.review = Review(self.temp.name, self.native, search, lambda url: url.encode())
        self.addCleanup(self.review.db.close)
        self.review.refresh()

    def test_rejection_survives_restart_and_never_writes_contacts(self):
        first = self.review.candidate("demo-1")
        self.review.decide("demo-1", first["id"], False)
        reopened = Review(self.temp.name, self.native, self.review.crawler, self.review.downloader)
        self.addCleanup(reopened.db.close)
        second = reopened.candidate("demo-1")
        self.assertNotEqual(first["id"], second["id"])
        self.assertFalse(any(command == "approve" for command, _ in self.native.calls))

    def test_save_uses_preview_bytes_and_rejects_duplicate_submission(self):
        first = self.review.candidate("demo-1")
        self.review.decide("demo-1", first["id"], True)
        command, payload = self.native.calls[-1]
        self.assertEqual(command, "approve")
        self.assertEqual(payload["sha256"], hashlib.sha256(Path(payload["image"]).read_bytes()).hexdigest())
        self.assertTrue(Path(payload["backup"]).exists())
        self.assertEqual(self.review.queue()["saved"], 1)
        with self.assertRaises(ValueError):
            self.review.decide("demo-1", first["id"], True)

    def test_cannot_approve_another_contacts_candidate_or_modified_bytes(self):
        first = self.review.candidate("demo-1")
        with self.assertRaises(ValueError):
            self.review.decide("demo-2", first["id"], True)
        (Path(self.temp.name) / "images" / (first["id"] + ".jpg")).write_bytes(b"changed")
        with self.assertRaisesRegex(ValueError, "changed"):
            self.review.decide("demo-1", first["id"], True)
        self.assertFalse(any(command == "approve" for command, _ in self.native.calls))

    def test_failed_save_keeps_review_pending_and_records_uncertainty(self):
        first = self.review.candidate("demo-1")
        self.native.fail = True
        with self.assertRaisesRegex(ValueError, "already has a photo"):
            self.review.decide("demo-1", first["id"], True)
        self.assertEqual(self.review.person("demo-1")["status"], "pending")
        self.assertEqual(self.review.db.execute("SELECT state FROM approvals").fetchone()[0], "uncertain")
        self.assertEqual(self.review.queue()["saved"], 0)

    def test_refresh_preserves_skips_and_removes_contacts_with_photos(self):
        self.review.skip("demo-1")
        self.review.refresh()
        self.assertEqual(self.review.queue()["contacts"][0]["status"], "skipped")
        self.native.people = self.native.people[:1]
        self.review.refresh()
        self.assertEqual(len(self.review.queue()["contacts"]), 1)
        self.review.resume()
        self.assertEqual(self.review.person("demo-1")["status"], "pending")

    def test_exhaustion_is_bounded_and_new_queries_restart_pagination(self):
        for _ in range(3):
            item = self.review.candidate("demo-1")
            self.review.decide("demo-1", item["id"], False)
        item = self.review.candidate("demo-1")
        self.assertEqual(self.searches[-1][1], 1)
        self.review.candidate("demo-1", "Alex new employer")
        self.assertIn(("Alex new employer", 0), self.searches)
        self.assertNotEqual(item["id"], self.review.candidate("demo-1")["id"])

    def test_duplicate_rejected_image_at_new_url_is_not_shown(self):
        self.review.downloader = lambda _: b"same image"
        first = self.review.candidate("demo-1")
        self.review.decide("demo-1", first["id"], False)
        with self.assertRaises(ValueError):
            self.review.candidate("demo-1")
        self.assertEqual(self.review.db.execute("SELECT count(*) FROM candidates WHERE status='rejected'").fetchone()[0], 1)

    def test_api_requires_token_origin_host_and_explicit_boolean(self):
        server = ThreadingHTTPServer(("127.0.0.1", 0), handler(self.review, "secret"))
        threading.Thread(target=server.serve_forever, daemon=True).start()
        self.addCleanup(server.server_close)
        self.addCleanup(server.shutdown)

        def request(path, body=None, **headers):
            connection = http.client.HTTPConnection("127.0.0.1", server.server_port)
            self.addCleanup(connection.close)
            connection.request("POST" if body is not None else "GET", path,
                               json.dumps(body) if body is not None else None, headers)
            response = connection.getresponse()
            response.read()
            return response.status

        self.assertEqual(request("/api/queue"), 403)
        self.assertEqual(request("/api/queue", **{"X-Review-Token": "secret", "Origin": "https://evil.test"}), 403)
        self.assertEqual(request("/api/queue", **{"X-Review-Token": "secret", "Host": "evil.test"}), 403)
        self.assertEqual(request("/api/queue", **{"X-Review-Token": "secret"}), 200)
        self.assertEqual(request("/api/decide", {"approved": "false"}, **{"X-Review-Token": "secret", "Content-Type": "application/json"}), 400)
        self.assertEqual(request("/images/../../contacts.swift", **{"X-Review-Token": "secret"}), 404)

    def test_background_crawler_discovers_candidates_for_entire_queue(self):
        crawler = Crawler(self.review)
        crawler.wakeup.set()
        for _ in range(100):
            if crawler.describe() == "Search pass complete":
                break
            threading.Event().wait(0.1)
        self.assertEqual(crawler.describe(), "Search pass complete")
        self.assertEqual(len(self.searches), 2)
        self.assertEqual(self.review.db.execute("SELECT count(DISTINCT person) FROM candidates").fetchone()[0], 2)
        self.review.candidate("demo-2")
        self.assertEqual(len(self.searches), 2, "Review should reuse background discoveries")


class SearchTests(unittest.TestCase):
    def test_parser_extracts_metadata_and_rejects_unsafe_schemes(self):
        parser = ImageResults()
        good = html.escape(json.dumps({"murl": "https://example.com/a.jpg", "purl": "https://example.com/person", "t": "A person"}), quote=True)
        bad = html.escape(json.dumps({"murl": "file:///etc/passwd", "purl": "https://example.com"}), quote=True)
        parser.feed(f'<a m="{good}"></a><a m="{bad}"></a><a m="broken"></a>')
        self.assertEqual(len(parser.results), 1)
        self.assertEqual(parser.results[0]["title"], "A person")

    def test_private_addresses_and_credentials_are_blocked(self):
        for url in ("file:///etc/passwd", "http://name:password@example.com/x", "http://example.com:123/x"):
            with self.assertRaises(ValueError):
                public_url(url)
        for address in ("127.0.0.1", "10.0.0.1", "169.254.169.254", "::1", "fd00::1"):
            with patch("socket.getaddrinfo", return_value=[(socket.AF_INET, socket.SOCK_STREAM, 6, "", (address, 80))]):
                with self.assertRaisesRegex(ValueError, "blocked"):
                    download("http://example.com/photo")

    def test_redirect_to_private_host_is_blocked(self):
        with patch("socket.getaddrinfo", side_effect=[[(2, 1, 6, "", ("8.8.8.8", 80))], [(2, 1, 6, "", ("127.0.0.1", 80))]]), \
             patch("socket.create_connection"), patch("http.client.HTTPConnection") as connection:
            response = connection.return_value.getresponse.return_value
            response.status = 302
            response.getheader.return_value = "http://localhost/secret"
            with self.assertRaisesRegex(ValueError, "blocked"):
                download("http://example.com/photo")


if __name__ == "__main__":
    unittest.main()
