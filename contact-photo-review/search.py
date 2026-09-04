"""Small public-image crawler; no credentials, cookies, or private URLs."""
import html.parser
import http.client
import ipaddress
import json
import socket
import ssl
import time
from urllib.parse import urlencode, urljoin, urlsplit


def public_url(url):
    parsed = urlsplit(url)
    if (parsed.scheme not in {"http", "https"} or not parsed.hostname
            or parsed.username or parsed.password or parsed.port not in {None, 80, 443}):
        raise ValueError("Only public HTTP(S) image URLs are allowed")
    return parsed


def download(url, limit=10_000_000):
    """Pin the connection to a validated public IP, including every redirect."""
    deadline = time.monotonic() + 35
    for _ in range(5):
        parsed = public_url(url)
        port = parsed.port or (443 if parsed.scheme == "https" else 80)
        addresses = socket.getaddrinfo(parsed.hostname, port, type=socket.SOCK_STREAM)
        if not addresses or any(not ipaddress.ip_address(a[4][0]).is_global for a in addresses):
            raise ValueError("Private and local network addresses are blocked")
        timeout = min(12, deadline - time.monotonic())
        if timeout <= 0:
            raise ValueError("Image download timed out")
        connection = http.client.HTTPConnection(parsed.hostname, port, timeout=timeout)
        sock = socket.create_connection((addresses[0][4][0], port), timeout=timeout)
        try:
            if parsed.scheme == "https":
                sock = ssl.create_default_context().wrap_socket(sock, server_hostname=parsed.hostname)
            connection.sock = sock
            path = parsed.path or "/"
            if parsed.query:
                path += "?" + parsed.query
            connection.request("GET", path, headers={"User-Agent": "Mozilla/5.0 ContactPhotoReview/1.0",
                                                     "Accept-Encoding": "identity"})
            response = connection.getresponse()
            if response.status in {301, 302, 303, 307, 308}:
                location = response.getheader("Location")
                if not location:
                    raise ValueError("Invalid redirect")
                url = urljoin(url, location)
                continue
            if response.status != 200:
                raise ValueError(f"Website returned HTTP {response.status}")
            chunks, size = [], 0
            while True:
                if time.monotonic() >= deadline:
                    raise ValueError("Image download timed out")
                chunk = response.read1(min(65536, limit + 1 - size))
                if not chunk:
                    break
                chunks.append(chunk)
                size += len(chunk)
                if size > limit:
                    raise ValueError("Download exceeds size limit")
            return b"".join(chunks)
        finally:
            connection.close()
            sock.close()
    raise ValueError("Too many redirects")


class ImageResults(html.parser.HTMLParser):
    def __init__(self):
        super().__init__()
        self.results = []

    def handle_starttag(self, tag, attrs):
        attributes = dict(attrs)
        if tag != "a" or "m" not in attributes:
            return
        try:
            item = json.loads(attributes["m"])
            image, source = item["murl"], item["purl"]
            public_url(image)
            public_url(source)
            self.results.append({"url": image, "source": source, "title": str(item.get("t", "Source page"))})
        except (KeyError, ValueError, TypeError):
            pass


def search(query, page):
    params = urlencode({"q": query, "first": page * 35, "count": 35, "adlt": "strict", "mmasync": 1})
    parser = ImageResults()
    parser.feed(download("https://www.bing.com/images/async?" + params, 4_000_000).decode("utf-8", errors="replace"))
    if not parser.results:
        raise ValueError("Search returned no images or was blocked. Refine the search or try again later.")
    return parser.results
