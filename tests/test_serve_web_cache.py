#!/usr/bin/env python3
"""Integration tests for serve_web.py HTTP server with Range, ETag, and caching headers."""

import http.client
import json
import os
import shutil
import socket
import subprocess
import sys
import tempfile
import time
import unittest


def find_free_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
        s.bind(("", 0))
        return s.getsockname()[1]


class TestServeWebCacheIntegration(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.test_dir = tempfile.mkdtemp(prefix="arduracer_serve_test_")
        cls.roms_dir = os.path.join(cls.test_dir, "roms")
        os.makedirs(cls.roms_dir)

        # Create dummy sw.js, manifest.json, and ROM files
        with open(os.path.join(cls.test_dir, "sw.js"), "w", encoding="utf-8") as f:
            f.write("// Service Worker\n")

        with open(os.path.join(cls.roms_dir, "manifest.json"), "w", encoding="utf-8") as f:
            json.dump({"build_id": "test-build-1"}, f)

        # Create a 64KB dummy test disc
        cls.disc_data = bytes([i % 256 for i in range(64 * 1024)])
        with open(os.path.join(cls.roms_dir, "arduracer.bin"), "wb") as f:
            f.write(cls.disc_data)

        cls.port = find_free_port()
        serve_script = os.path.abspath("tools/serve_web.py")
        cls.proc = subprocess.Popen(
            [sys.executable, serve_script, str(cls.port), cls.test_dir],
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )
        time.sleep(0.5)

    @classmethod
    def tearDownClass(cls):
        cls.proc.terminate()
        cls.proc.wait()
        shutil.rmtree(cls.test_dir, ignore_errors=True)

    def test_sw_headers(self):
        conn = http.client.HTTPConnection("127.0.0.1", self.port)
        conn.request("GET", "/sw.js")
        res = conn.getresponse()
        self.assertEqual(res.status, 200)
        self.assertEqual(res.getheader("Service-Worker-Allowed"), "/")
        self.assertIn("no-cache", res.getheader("Cache-Control", ""))
        conn.close()

    def test_manifest_no_cache_headers(self):
        conn = http.client.HTTPConnection("127.0.0.1", self.port)
        conn.request("GET", "/roms/manifest.json")
        res = conn.getresponse()
        self.assertEqual(res.status, 200)
        self.assertIn("no-cache", res.getheader("Cache-Control", ""))
        conn.close()

    def test_rom_range_request_206(self):
        conn = http.client.HTTPConnection("127.0.0.1", self.port)
        headers = {"Range": "bytes=100-199"}
        conn.request("GET", "/roms/arduracer.bin", headers=headers)
        res = conn.getresponse()
        self.assertEqual(res.status, 206)
        self.assertEqual(res.getheader("Content-Range"), "bytes 100-199/65536")
        self.assertEqual(res.getheader("Content-Length"), "100")
        self.assertEqual(res.getheader("Accept-Ranges"), "bytes")
        self.assertTrue(res.getheader("ETag") is not None)
        self.assertTrue(res.getheader("Last-Modified") is not None)

        body = res.read()
        self.assertEqual(len(body), 100)
        self.assertEqual(body, self.disc_data[100:200])
        conn.close()

    def test_rom_full_get_200(self):
        conn = http.client.HTTPConnection("127.0.0.1", self.port)
        conn.request("GET", "/roms/arduracer.bin")
        res = conn.getresponse()
        self.assertEqual(res.status, 200)
        body = res.read()
        self.assertEqual(len(body), 64 * 1024)
        self.assertEqual(body, self.disc_data)
        conn.close()


if __name__ == "__main__":
    unittest.main()
