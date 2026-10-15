#!/usr/bin/env python3
"""Encode a raw 44.1 kHz stereo PCM track into CD-XA ADPCM sectors.

The music masters are Redbook CD-DA: raw interleaved 16-bit stereo at
44.1 kHz, no header. XA-ADPCM at 37.8 kHz stereo 4-bit is a different rate and a
different coding, so this does both jobs -- resample, then encode.

Resampling is linear interpolation. That is not high quality, but the format
loses far more to 4-bit quantisation than the resampler loses to linear
interpolation, and a better resampler would be a filter design project rather
than a cooker change.

Usage:
    make_music_xa.py <in.raw> <out.xa> [track_number]
"""

import os
import struct
import sys

sys.path.insert(0, "tools/track_cook")
from make_xa import encode_sector

SRC_HZ = 44_100
DST_HZ = 37_800
FRAMES_PER_SECTOR = 2016


def resample(stereo_i16, src_hz=SRC_HZ, dst_hz=DST_HZ):
    """Linear-interpolation resample of interleaved stereo frames."""
    if src_hz == dst_hz:
        return stereo_i16
    src_frames = len(stereo_i16) // 2
    dst_frames = (src_frames * dst_hz) // src_hz
    out = []
    ratio = src_hz / dst_hz
    for i in range(dst_frames):
        pos = i * ratio
        i0 = int(pos)
        frac = pos - i0
        i1 = min(i0 + 1, src_frames - 1)
        for ch in range(2):
            a = stereo_i16[i0 * 2 + ch]
            b = stereo_i16[i1 * 2 + ch]
            out.append(int(a + (b - a) * frac))
    return out


def main():
    force = "--force" in sys.argv
    args = [a for a in sys.argv[1:] if a != "--force"]
    if len(args) < 2:
        sys.exit(__doc__)
    src_path, dst_path = args[0], args[1]
    track = int(args[2]) if len(args) > 2 else 0

    if not force and os.path.exists(dst_path) and os.path.getsize(dst_path) > 0:
        if os.path.exists(src_path) and os.path.getmtime(dst_path) >= os.path.getmtime(src_path):
            size_mb = os.path.getsize(dst_path) / 1e6
            print(f"{dst_path}: cached -> track {track} ({size_mb:.1f} MB)")
            return

    with open(src_path, "rb") as f:
        raw = f.read()
    if len(raw) % 4:
        sys.exit(f"{src_path}: raw PCM must be a whole number of stereo frames")
    stereo = list(struct.unpack(f"<{len(raw) // 2}h", raw))

    resampled = resample(stereo)
    sectors = []
    offset = 0
    while offset + FRAMES_PER_SECTOR <= len(resampled) // 2:
        sectors.append(encode_sector(resampled, offset))
        offset += FRAMES_PER_SECTOR

    if not sectors:
        sys.exit(f"{src_path}: shorter than one sector ({FRAMES_PER_SECTOR} frames)")

    with open(dst_path, "wb") as f:
        f.write(b"".join(sectors))

    seconds = (len(sectors) * FRAMES_PER_SECTOR) / DST_HZ
    src_seconds = (len(stereo) // 2) / SRC_HZ
    print(
        f"{dst_path}: track {track}, {len(sectors)} sectors, "
        f"{seconds:.1f}s (from {src_seconds:.1f}s), "
        f"{len(sectors) * 2336 / 1e6:.1f} MB"
    )


if __name__ == "__main__":
    main()
