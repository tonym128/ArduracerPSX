#!/usr/bin/env python3
"""Write the XA round-trip fixture consumed by `crates/arduracer-core/tests/xa_roundtrip.rs`.

The fixture has to be a real encoder output, not a hand-written sector: the
point of that test is to prove the Python encoder and the Rust decoder agree
about the group layout, and a hand-written sector would only prove the decoder
agrees with itself.

Left sweeps all fifteen signed 4-bit codes and right sweeps them in reverse, so
a packing bug that collapses the channels cannot pass.

Run from the repository root:  python3 tools/track_cook/make_xa_fixture.py
"""

import sys, math, struct
sys.path.insert(0, "tools/track_cook")
from make_xa import encode_sector

# Left sweeps all 15 signed 4-bit codes; right sweeps the same codes in the
# opposite direction. Both are ramped so the fixture also proves the channels
# never alias -- a packing bug that put left in both slots would still pass an
# absolute-error check on a mono fixture.
N = 2016
frames = []
for i in range(N):
    left = (i % 15) - 7
    right = 7 - (i % 15)
    frames += [left * 4096, right * 4096]
raw = bytearray(encode_sector(frames, 0))
assert len(raw) == 2336, len(raw)
open("crates/arduracer-core/tests/data/xa_4bit_stereo.xa", "wb").write(bytes(raw))
open("crates/arduracer-core/tests/data/xa_4bit_stereo.pcm", "wb").write(
    struct.pack("<%dh" % len(frames), *frames))
print("fixture written")
