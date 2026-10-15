#!/usr/bin/env python3
"""Generate or verify manifest.json for web arcade disc images and ROMs."""

import argparse
import hashlib
import json
import os
import subprocess
import sys
from datetime import datetime, timezone

TARGET_EXTENSIONS = {".bin", ".cue", ".iso", ".exe"}


def get_git_commit(cwd: str) -> str:
    """Attempt to get current git commit hash."""
    try:
        res = subprocess.run(
            ["git", "rev-parse", "HEAD"],
            cwd=cwd,
            capture_output=True,
            text=True,
            check=True,
        )
        return res.stdout.strip()
    except Exception:
        return "unknown"


def compute_file_sha256(path: str) -> str:
    """Compute sha256 checksum for a file using 1MB chunks."""
    h = hashlib.sha256()
    with open(path, "rb") as f:
        while chunk := f.read(1024 * 1024):
            h.update(chunk)
    return h.hexdigest()


def generate_manifest_data(roms_dir: str) -> dict:
    """Generate manifest dictionary for all ROM files in roms_dir."""
    if not os.path.isdir(roms_dir):
        raise FileNotFoundError(f"ROMs directory does not exist: {roms_dir}")

    files_info = {}
    hasher = hashlib.sha256()

    # Sort filenames for deterministic ordering
    for filename in sorted(os.listdir(roms_dir)):
        if filename == "manifest.json":
            continue
        _, ext = os.path.splitext(filename.lower())
        if ext in TARGET_EXTENSIONS:
            filepath = os.path.join(roms_dir, filename)
            if os.path.isfile(filepath):
                file_size = os.path.getsize(filepath)
                file_hash = compute_file_sha256(filepath)
                mtime = int(os.path.getmtime(filepath))
                files_info[filename] = {
                    "path": filename,
                    "size": file_size,
                    "sha256": file_hash,
                    "modified": mtime,
                }
                hasher.update(f"{filename}:{file_size}:{file_hash}".encode("utf-8"))

    commit = get_git_commit(roms_dir)
    manifest_hash = hasher.hexdigest()
    build_id = f"{commit[:10]}-{manifest_hash[:16]}" if commit != "unknown" else manifest_hash[:20]

    return {
        "version": 1,
        "build_id": build_id,
        "commit": commit,
        "build_time": datetime.now(timezone.utc).isoformat(),
        "files": files_info,
    }


def write_manifest(roms_dir: str, output_path: str | None = None) -> str:
    """Write manifest.json to roms_dir (or output_path)."""
    data = generate_manifest_data(roms_dir)
    out = output_path or os.path.join(roms_dir, "manifest.json")
    with open(out, "w", encoding="utf-8") as f:
        json.dump(data, f, indent=2)
        f.write("\n")
    return out


def main():
    parser = argparse.ArgumentParser(description="Generate ROM manifest for Arduracer Web Arcade")
    parser.add_argument("dir", nargs="?", default="web/roms", help="ROM directory (default: web/roms)")
    parser.add_argument("--output", "-o", help="Custom output manifest path")
    parser.add_argument("--check", action="store_true", help="Check if existing manifest matches without modifying")

    args = parser.parse_args()
    roms_dir = os.path.abspath(args.dir)

    if not os.path.isdir(roms_dir):
        print(f"Error: Directory '{roms_dir}' not found.", file=sys.stderr)
        sys.exit(1)

    out_file = args.output or os.path.join(roms_dir, "manifest.json")

    if args.check:
        if not os.path.isfile(out_file):
            print(f"Manifest missing: {out_file}", file=sys.stderr)
            sys.exit(1)
        try:
            with open(out_file, "r", encoding="utf-8") as f:
                existing = json.load(f)
        except Exception as e:
            print(f"Invalid manifest JSON: {e}", file=sys.stderr)
            sys.exit(1)

        current = generate_manifest_data(roms_dir)
        # Compare file hashes and sizes
        if existing.get("files") == current.get("files"):
            print("Manifest matches current ROM files.")
            sys.exit(0)
        else:
            print("Manifest does NOT match current ROM files!", file=sys.stderr)
            sys.exit(1)

    written = write_manifest(roms_dir, out_file)
    print(f"Generated ROM manifest: {written}")


if __name__ == "__main__":
    main()
