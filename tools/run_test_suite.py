#!/usr/bin/env python3
"""Run a Cargo test suite only when it actually registers tests."""

from __future__ import annotations

import re
import subprocess
import sys
from pathlib import Path


def main() -> int:
    if len(sys.argv) != 2:
        print(f"usage: {Path(sys.argv[0]).name} <Cargo.toml>", file=sys.stderr)
        return 2

    manifest = Path(sys.argv[1])
    if not manifest.is_file():
        print(f"ERROR: required test manifest is missing: {manifest}", file=sys.stderr)
        return 2

    command = ["cargo", "test", "--manifest-path", str(manifest), "--", "--list"]
    listed = subprocess.run(command, text=True, capture_output=True, check=False)
    if listed.returncode:
        sys.stdout.write(listed.stdout)
        sys.stderr.write(listed.stderr)
        return listed.returncode

    # libtest lists each unit/integration test as `path::name: test`; rustdoc
    # tests use `file.rs - name: test`, which this intentionally counts too.
    test_count = sum(
        1 for line in listed.stdout.splitlines() if re.search(r": test\s*$", line)
    )
    if test_count == 0:
        sys.stdout.write(listed.stdout)
        sys.stderr.write(listed.stderr)
        print(
            f"ERROR: {manifest} registered zero Cargo tests; refusing a green test gate",
            file=sys.stderr,
        )
        return 1

    print(f"[test gate] {manifest}: {test_count} registered tests")
    return subprocess.run(
        ["cargo", "test", "--manifest-path", str(manifest)], check=False
    ).returncode


if __name__ == "__main__":
    raise SystemExit(main())
