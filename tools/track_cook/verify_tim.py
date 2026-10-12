#!/usr/bin/env python3
"""Verifies `assets/TIMTRACKS.BIN` against the raster the cooker packed it from.

The TIM runtime path has no decoder, so a cooker bug cannot be caught at
runtime as wrong-looking pixels alone -- a swapped channel or a host-endian
leak ships a disc that boots, renders, and is subtly wrong everywhere. This
checks the on-disc bytes against the source instead, which is the only place
the mistake is visible.

Checks, per circuit:
  * block size is exactly 1024 sectors, and the whole file is n x 1024
  * every one of the 256 tiles round-trips through the runtime's addressing
    arithmetic (`index = ty * 16 + tx`, 4 sectors each) back to the same texels
  * the 15bpp encoding is BGR555 with bit 15 clear, and matches the source
    raster quantised the same way the cooker quantises it

Run: python3 tools/track_cook/verify_tim.py [circuit-index]
"""

import importlib.util
import os
import re
import sys

import numpy as np
from PIL import Image

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, os.path.join(ROOT, "tools", "track_cook"))

_spec = importlib.util.spec_from_file_location(
    "build_atlas", os.path.join(ROOT, "tools", "track_cook", "build_atlas.py"))
build_atlas = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(build_atlas)

SECTOR_SIZE = 2048
TILE_DIM = build_atlas.TIM_TILE_DIM
TILES_PER_AXIS = build_atlas.TIM_TILES_PER_BLOCK_AXIS
TILE_SECTORS = build_atlas.TIM_TILE_SECTORS
BLOCK_SECTORS = build_atlas.TIM_BLOCK_SECTORS
TILE_BYTES = TILE_DIM * TILE_DIM * 2


def expect_source_555(im):
    """Re-derives the halfwords the cooker should have written."""
    rgb = np.array(im.convert("RGB"), dtype=np.uint32)
    if rgb.shape[:2] != (1024, 1024):
        im = im.resize((1024, 1024), 3)
        rgb = np.array(im.convert("RGB"), dtype=np.uint32)
    r5 = (rgb[:, :, 0] >> 3) & 0x1F
    g5 = (rgb[:, :, 1] >> 3) & 0x1F
    b5 = (rgb[:, :, 2] >> 3) & 0x1F
    return r5 | (g5 << 5) | (b5 << 10)


def check_tiles_against_source(path, name, offset_sectors, expected_halfwords):
    with open(path, "rb") as f:
        for ty in (0, 7, TILES_PER_AXIS - 1):
            for tx in (0, 5, TILES_PER_AXIS - 1):
                f.seek((offset_sectors + (ty * TILES_PER_AXIS + tx) * TILE_SECTORS) * SECTOR_SIZE)
                raw = f.read(TILE_BYTES)
                assert len(raw) == TILE_BYTES, (
                    f"{name} tile ({tx},{ty}) is {len(raw)} bytes, not {TILE_BYTES}")
                got = np.frombuffer(raw, dtype="<u2").reshape(TILE_DIM, TILE_DIM)

                want = expected_halfwords[
                    ty * TILE_DIM:(ty + 1) * TILE_DIM,
                    tx * TILE_DIM:(tx + 1) * TILE_DIM,
                ]
                if not np.array_equal(got, want):
                    bad = int((got != want).sum())
                    raise AssertionError(
                        f"{name} tile ({tx},{ty}): {bad}/{TILE_DIM ** 2} texels differ from "
                        f"the source raster -- channel order or byte order is wrong")

                if int(got.max()) & 0x8000:
                    raise AssertionError(
                        f"{name} tile ({tx},{ty}): bit 15 set, which the GPU reads as "
                        f"semi-transparent. Circuit surfaces are opaque.")
    return True


def source_halfwords(name):
    """The halfwords the cooker packed for `name`, from the same source it uses."""
    clean = name.replace(" ", "_")
    gen = os.path.join(ROOT, "tracks", clean + ".visual.gen.png")
    if os.path.exists(gen):
        return expect_source_555(Image.open(gen).convert("RGB"))
    im, _ = build_atlas.rasterise_svg_visual(name)
    assert im is not None, f"{name}: no visual source to verify against"
    return expect_source_555(im)


def check_packing_matches_runtime():
    """The cooker and the runtime quantise colour separately, in two languages.

    `build_atlas.pack_tim_block` writes the texels and `texlayout::pack_bgr555`
    is what the rest of the renderer packs flat colours with, so if the two
    drift the game shows seams: streamed tiles in one colour against polygons in
    another. Nothing in the game can detect that, so the two expressions are
    compared here by reading the Rust one back out of its source.

    This is a textual check, which is the honest trade: it cannot evaluate the
    Rust, but it does catch the realistic failure -- one side being edited for a
    reason (a rounding change, an alpha bit) without the other.
    """
    src = os.path.join(ROOT, "game", "src", "gpu", "texlayout.rs")
    text = open(src).read()
    m = re.search(r"pub const fn pack_bgr555.*?\{(.*?)\n\}", text, re.S)
    assert m is not None, "texlayout::pack_bgr555 not found"

    def normalise(s):
        # Strip comments, then all whitespace. Both sides go through the same
        # function so `as u16` and `as  u16` cannot disagree, which a naive
        # strip would turn into a false failure.
        s = re.sub(r"//.*", "", s)
        return "".join(s.split())

    body = normalise(m.group(1))
    expected = normalise("(((b as u16) >> 3) << 10) | (((g as u16) >> 3) << 5) | ((r as u16) >> 3)")
    assert expected in body, (
        "texlayout::pack_bgr555 no longer matches the expression this cooker "
        f"writes.\n  runtime: {body}\n  cooker:  r5 | (g5 << 5) | (b5 << 10)\n"
        "Fix both, or streamed tiles will not match flat-shaded geometry."
    )


def main() -> int:
    path = os.path.join(ROOT, "assets", "TIMTRACKS.BIN")
    if not os.path.exists(path):
        print(f"ERROR: {path} missing; run 'make atlas'")
        return 1

    check_packing_matches_runtime()

    rs_path = os.path.join(ROOT, "crates", "arduracer-core", "src", "visual_tex.rs")
    text = open(rs_path).read()
    table = re.search(r"CIRCUIT_TIM_SECTORS[^=]*= \[(.*?)\n\];", text, re.S)
    if table is None:
        print("ERROR: CIRCUIT_TIM_SECTORS not found in visual_tex.rs")
        return 1
    entries = [
        (int(a), int(b), int(c))
        for a, b, c in re.findall(
            r"sector_offset: (\d+),\s*sector_count: (\d+),\s*byte_len: (\d+)", table.group(1))
    ]

    file_sectors = os.path.getsize(path) // SECTOR_SIZE
    assert file_sectors == len(entries) * BLOCK_SECTORS, (
        f"{path} is {file_sectors} sectors, table expects {len(entries) * BLOCK_SECTORS}")

    names = build_atlas.discover_circuits()
    only = int(sys.argv[1]) if len(sys.argv) > 1 else None

    for idx, (name, _) in enumerate(names):
        off, cnt, blen = entries[idx]
        if cnt != BLOCK_SECTORS or blen != BLOCK_SECTORS * SECTOR_SIZE:
            raise AssertionError(
                f"circuit {idx} ({name}): sector_count {cnt}, byte_len {blen}; "
                f"an uncompressed block is exactly {BLOCK_SECTORS} sectors")

        # The table must be dense and in order, or the runtime's arithmetic
        # (`block_lba + tile_index * 4`) lands on the wrong circuit's texels.
        assert off == idx * BLOCK_SECTORS, (
            f"circuit {idx} ({name}): offset {off}, expected {idx * BLOCK_SECTORS}")

        if only is not None and only != idx:
            continue

        check_tiles_against_source(path, name, off, source_halfwords(name))
        print(f"  ok  {name} (block {idx}, {cnt} sectors)")

    checked = len(names) if only is None else 1
    print(f"TIMTRACKS.BIN verified: {file_sectors} sectors, {checked} circuits checked")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())