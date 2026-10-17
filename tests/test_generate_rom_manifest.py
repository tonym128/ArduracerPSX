#!/usr/bin/env python3
"""Tests for tools/generate_rom_manifest.py."""

import json
import os
import shutil
import tempfile
import unittest

from tools.generate_rom_manifest import (
    compute_file_sha256,
    generate_manifest_data,
    write_manifest,
)


class TestGenerateRomManifest(unittest.TestCase):
    def setUp(self):
        self.test_dir = tempfile.mkdtemp(prefix="arduracer_rom_test_")

    def tearDown(self):
        shutil.rmtree(self.test_dir, ignore_errors=True)

    def test_compute_file_sha256(self):
        test_file = os.path.join(self.test_dir, "test.bin")
        content = b"PlayStation 1 Arduracer Binary Test Payload"
        with open(test_file, "wb") as f:
            f.write(content)

        import hashlib
        expected_sha = hashlib.sha256(content).hexdigest()
        self.assertEqual(compute_file_sha256(test_file), expected_sha)

    def test_generate_manifest_data(self):
        # Create dummy rom files
        files = {
            "arduracer.cue": b"FILE \"arduracer.bin\" BINARY\n  TRACK 01 MODE2/2352\n",
            "arduracer.bin": b"\x00" * 4096,
            "arduracer.iso": b"\x01\x02\x03\x04",
            "arduracer.exe": b"PS-X EXE" + b"\x00" * 100,
            "ignored.txt": b"Should be ignored",
        }
        for name, data in files.items():
            with open(os.path.join(self.test_dir, name), "wb") as f:
                f.write(data)

        manifest = generate_manifest_data(self.test_dir)
        self.assertEqual(manifest["version"], 1)
        self.assertTrue("build_id" in manifest)
        self.assertEqual(len(manifest["files"]), 4)
        self.assertIn("arduracer.cue", manifest["files"])
        self.assertIn("arduracer.bin", manifest["files"])
        self.assertIn("arduracer.iso", manifest["files"])
        self.assertIn("arduracer.exe", manifest["files"])
        self.assertNotIn("ignored.txt", manifest["files"])

        self.assertEqual(manifest["files"]["arduracer.bin"]["size"], 4096)
        self.assertEqual(manifest["files"]["arduracer.iso"]["size"], 4)

    def test_write_manifest_and_reload(self):
        cue_path = os.path.join(self.test_dir, "arduracer.cue")
        with open(cue_path, "wb") as f:
            f.write(b"FILE \"arduracer.bin\" BINARY\n")

        manifest_path = write_manifest(self.test_dir)
        self.assertTrue(os.path.isfile(manifest_path))

        with open(manifest_path, "r", encoding="utf-8") as f:
            data = json.load(f)

        self.assertIn("arduracer.cue", data["files"])


if __name__ == "__main__":
    unittest.main()
