#!/usr/bin/env python3
"""HTTP server with Range request (HTTP 206) support for local testing of PSoXide Web Player."""

import http.server
import os
import re
import socketserver
import sys


class RangeRequestHandler(http.server.SimpleHTTPRequestHandler):
    """SimpleHTTPRequestHandler with byte range (HTTP 206) support."""

    def send_head(self):
        if "Range" not in self.headers:
            self.range = None
            return super().send_head()

        path = self.translate_path(self.path)
        if not os.path.isfile(path):
            return super().send_head()

        range_header = self.headers["Range"].strip()
        m = re.match(r"^bytes=(\d*)-(\d*)$", range_header)
        if not m:
            return super().send_head()

        total = os.path.getsize(path)
        start_str, end_str = m.groups()
        start = int(start_str) if start_str else 0
        end = int(end_str) if end_str else total - 1

        if start >= total or end >= total or start > end:
            self.send_error(416, "Requested Range Not Satisfiable")
            return None

        self.send_response(206)
        self.send_header("Content-Type", self.guess_type(path))
        self.send_header("Content-Range", f"bytes {start}-{end}/{total}")
        self.send_header("Content-Length", str(end - start + 1))
        self.send_header("Accept-Ranges", "bytes")
        self.end_headers()

        f = open(path, "rb")
        f.seek(start)
        self.range = (start, end)
        return f

    def copyfile(self, source, outputfile):
        if not getattr(self, "range", None):
            return super().copyfile(source, outputfile)
        start, end = self.range
        remaining = end - start + 1
        bufsize = 64 * 1024
        while remaining > 0:
            chunk = source.read(min(bufsize, remaining))
            if not chunk:
                break
            outputfile.write(chunk)
            remaining -= len(chunk)


def main():
    port = int(sys.argv[1]) if len(sys.argv) > 1 else 8080
    directory = sys.argv[2] if len(sys.argv) > 2 else "web"

    class CustomHandler(RangeRequestHandler):
        def __init__(self, *args, **kwargs):
            super().__init__(*args, directory=directory, **kwargs)

    socketserver.TCPServer.allow_reuse_address = True
    with socketserver.TCPServer(("", port), CustomHandler) as httpd:
        print(f"Serving Arduracer PSX Web Arcade on http://localhost:{port} (directory: {directory}, HTTP 206 Range enabled) ...")
        try:
            httpd.serve_forever()
        except KeyboardInterrupt:
            print("\nShutting down server.")


if __name__ == "__main__":
    main()
