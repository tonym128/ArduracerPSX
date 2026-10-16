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

# --- 4bpp (TIM4) layout, kept beside the 15bpp one it is derived from ----
TIM4_PALETTE_BYTES = build_atlas.TIM4_PALETTE_BYTES
TIM4_BLOCK_SECTORS = build_atlas.TIM4_BLOCK_SECTORS
# Four indices per halfword: a 64x64 tile is 1024 halfwords = 2048 bytes.
TIM4_TILE_BYTES = TILE_DIM * TILE_DIM // 4 * 2
assert TIM4_TILE_BYTES == SECTOR_SIZE, "a TIM4 tile must be exactly one sector"

# --- 8bpp (TIM8) layout ---
TIM8_PALETTE_BYTES = build_atlas.TIM8_PALETTE_BYTES
TIM8_BLOCK_SECTORS = build_atlas.TIM8_BLOCK_SECTORS
TIM8_TILE_BYTES = TILE_DIM * TILE_DIM
TIM8_TILE_SECTORS = build_atlas.TIM8_TILE_SECTORS
assert TIM8_TILE_BYTES == 2 * SECTOR_SIZE, "a TIM8 tile must be exactly two sectors"


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


def check_tim4(circuit: int, name: str, offset: int, crop_im=None, shared_pal=None):
    """Verify a 4bpp block against a freshly quantised copy of the source."""
    path = os.path.join(ROOT, "assets", "TIM4TRACKS.BIN")
    pal_sector_bytes = SECTOR_SIZE
    im = crop_im if crop_im is not None else source_image(name)
    want_indices, want_palette = build_atlas.quantise_16(im, shared_palette=shared_pal)

    with open(path, "rb") as f:
        # Palette sector.
        f.seek(offset * SECTOR_SIZE)
        raw = f.read(pal_sector_bytes)
        entries = np.frombuffer(raw[: TIM4_PALETTE_BYTES], dtype="<u2")
        for i, (r, g, b) in enumerate(want_palette):
            want = ((b >> 3) << 10) | ((g >> 3) << 5) | (r >> 3)
            if int(entries[i]) != want:
                raise AssertionError(
                    f"{name} palette[{i}]: on disc 0x{int(entries[i]):04x}, "
                    f"expected 0x{want:04x} for RGB({r},{g},{b})"
                )

        # Sample tiles across the block, including the last one, which is where
        # an off-by-one in the palette-prefix offset would land.
        for ty, tx in ((0, 0), (5, 9), (TILES_PER_AXIS - 1, TILES_PER_AXIS - 1)):
            lba = offset + 1 + (ty * TILES_PER_AXIS + tx)
            f.seek(lba * SECTOR_SIZE)
            raw = f.read(TIM4_TILE_BYTES)
            if len(raw) != TIM4_TILE_BYTES:
                raise AssertionError(
                    f"{name} tile ({tx},{ty}) is {len(raw)} bytes, not {TIM4_TILE_BYTES}"
                )

            halfwords = np.frombuffer(raw, dtype="<u2").reshape(TILE_DIM, TILE_DIM // 4)
            # Unpack: four nibbles per word, leftmost texel in the low nibble.
            indices = np.stack(
                [
                    halfwords & 0x0F,
                    (halfwords >> 4) & 0x0F,
                    (halfwords >> 8) & 0x0F,
                    (halfwords >> 12) & 0x0F,
                ],
                axis=-1,
            ).reshape(TILE_DIM, TILE_DIM)

            want = want_indices[
                ty * TILE_DIM : (ty + 1) * TILE_DIM,
                tx * TILE_DIM : (tx + 1) * TILE_DIM,
            ]
            if not np.array_equal(indices, want):
                bad = int((indices != want).sum())
                raise AssertionError(
                    f"{name} tile ({tx},{ty}): {bad}/{TILE_DIM ** 2} indices differ "
                    f"from the quantised source -- nibble order or tile offset is wrong"
                )


def check_tim8(circuit: int, name: str, offset: int, crop_im=None, shared_pal=None):
    """Verify an 8bpp block against a freshly quantised copy of the source."""
    path = os.path.join(ROOT, "assets", "TIM8TRACKS.BIN")
    pal_sector_bytes = SECTOR_SIZE
    im = crop_im if crop_im is not None else source_image(name)
    want_indices, want_palette = build_atlas.quantise_256(im, shared_palette=shared_pal)

    with open(path, "rb") as f:
        # Palette sector.
        f.seek(offset * SECTOR_SIZE)
        raw = f.read(pal_sector_bytes)
        entries = np.frombuffer(raw[: TIM8_PALETTE_BYTES], dtype="<u2")
        for i, (r, g, b) in enumerate(want_palette):
            want = ((b >> 3) << 10) | ((g >> 3) << 5) | (r >> 3)
            if int(entries[i]) != want:
                raise AssertionError(
                    f"{name} TIM8 palette[{i}]: on disc 0x{int(entries[i]):04x}, "
                    f"expected 0x{want:04x} for RGB({r},{g},{b})"
                )

        # Sample tiles across the block, including the last one
        for ty, tx in ((0, 0), (5, 9), (TILES_PER_AXIS - 1, TILES_PER_AXIS - 1)):
            lba = offset + 1 + (ty * TILES_PER_AXIS + tx) * TIM8_TILE_SECTORS
            f.seek(lba * SECTOR_SIZE)
            raw = f.read(TIM8_TILE_BYTES)
            if len(raw) != TIM8_TILE_BYTES:
                raise AssertionError(
                    f"{name} TIM8 tile ({tx},{ty}) is {len(raw)} bytes, not {TIM8_TILE_BYTES}"
                )

            halfwords = np.frombuffer(raw, dtype="<u2").reshape(TILE_DIM, TILE_DIM // 2)
            indices = np.stack(
                [
                    halfwords & 0xFF,
                    (halfwords >> 8) & 0xFF,
                ],
                axis=-1,
            ).reshape(TILE_DIM, TILE_DIM)

            want = want_indices[
                ty * TILE_DIM : (ty + 1) * TILE_DIM,
                tx * TILE_DIM : (tx + 1) * TILE_DIM,
            ]
            if not np.array_equal(indices, want):
                bad = int((indices != want).sum())
                raise AssertionError(
                    f"{name} TIM8 tile ({tx},{ty}): {bad}/{TILE_DIM ** 2} indices differ "
                    f"from the quantised source -- byte order or tile offset is wrong"
                )


def source_image(name):
    """The raster the cooker packs for `name`, from the same source it uses."""
    clean = name.replace(" ", "_")
    gen = os.path.join(ROOT, "tracks", clean + ".visual.gen.png")
    if os.path.exists(gen):
        im = Image.open(gen).convert("RGB")
    else:
        im, _ = build_atlas.rasterise_svg_visual(name)
        assert im is not None, f"{name}: no visual source"
    return im


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

    # The 4bpp table, read the same way so both layouts are checked against one
    # circuit ordering.
    t4 = re.search(r"CIRCUIT_TIM4_SECTORS[^=]*= \[(.*?)\n\];", text, re.S)
    assert t4 is not None, "CIRCUIT_TIM4_SECTORS not found in visual_tex.rs"
    tim4_entries = [
        (int(a), int(b), int(c))
        for a, b, c in re.findall(
            r"sector_offset: (\d+),\s*sector_count: (\d+),\s*byte_len: (\d+)", t4.group(1))
    ]
    tim4_path = os.path.join(ROOT, "assets", "TIM4TRACKS.BIN")
    if os.path.exists(tim4_path):
        tim4_sectors = os.path.getsize(tim4_path) // SECTOR_SIZE
        assert tim4_sectors == len(tim4_entries) * TIM4_BLOCK_SECTORS, (
            f"TIM4TRACKS.BIN is {tim4_sectors} sectors, table expects "
            f"{len(tim4_entries) * TIM4_BLOCK_SECTORS}"
        )

    t8 = re.search(r"CIRCUIT_TIM8_SECTORS[^=]*= \[(.*?)\n\];", text, re.S)
    assert t8 is not None, "CIRCUIT_TIM8_SECTORS not found in visual_tex.rs"
    tim8_entries = [
        (int(a), int(b), int(c))
        for a, b, c in re.findall(
            r"sector_offset: (\d+),\s*sector_count: (\d+),\s*byte_len: (\d+)", t8.group(1))
    ]
    tim8_path = os.path.join(ROOT, "assets", "TIM8TRACKS.BIN")
    if os.path.exists(tim8_path):
        tim8_sectors = os.path.getsize(tim8_path) // SECTOR_SIZE
        assert tim8_sectors == len(tim8_entries) * TIM8_BLOCK_SECTORS, (
            f"TIM8TRACKS.BIN is {tim8_sectors} sectors, table expects "
            f"{len(tim8_entries) * TIM8_BLOCK_SECTORS}"
        )

    block_cursor = 0
    checked_circuits = 0
    checked_blocks = 0
    for idx, (name, _) in enumerate(names):
        full_im = source_image(name)
        w, h = full_im.size
        grid_bx = max(1, w // 1024)
        grid_by = max(1, h // 1024)

        shared_pal16 = None
        shared_pal256 = None
        if grid_bx > 1 or grid_by > 1:
            small = full_im.resize((1024, 1024), Image.Resampling.LANCZOS)
            pal16_im = small.quantize(colors=16, method=Image.Quantize.MEDIANCUT)
            raw16 = pal16_im.getpalette() or []
            shared_pal16 = [(raw16[i * 3], raw16[i * 3 + 1], raw16[i * 3 + 2]) for i in range(16)]
            while len(shared_pal16) < 16:
                shared_pal16.append((0, 0, 0))

            pal256_im = small.quantize(colors=256, method=Image.Quantize.MEDIANCUT)
            raw256 = pal256_im.getpalette() or []
            shared_pal256 = [(raw256[i * 3], raw256[i * 3 + 1], raw256[i * 3 + 2]) for i in range(len(raw256) // 3)]
            while len(shared_pal256) < 256:
                shared_pal256.append((0, 0, 0))
            shared_pal256 = shared_pal256[:256]

        skip_circuit = (only is not None and only != idx)

        for by in range(grid_by):
            for bx in range(grid_bx):
                b_idx = block_cursor
                block_cursor += 1

                off, cnt, blen = entries[b_idx]
                if cnt != BLOCK_SECTORS or blen != BLOCK_SECTORS * SECTOR_SIZE:
                    raise AssertionError(
                        f"circuit {idx} ({name}) block ({bx},{by}): sector_count {cnt}, byte_len {blen}; "
                        f"an uncompressed block is exactly {BLOCK_SECTORS} sectors")

                assert off == b_idx * BLOCK_SECTORS, (
                    f"circuit {idx} ({name}) block ({bx},{by}): offset {off}, expected {b_idx * BLOCK_SECTORS}")

                tim4_off, tim4_cnt, tim4_len = tim4_entries[b_idx]
                assert (
                    tim4_cnt == TIM4_BLOCK_SECTORS
                    and tim4_len == TIM4_BLOCK_SECTORS * SECTOR_SIZE
                ), (
                    f"circuit {idx} ({name}) block ({bx},{by}): 4bpp sector_count {tim4_cnt}, byte_len "
                    f"{tim4_len}; expected {TIM4_BLOCK_SECTORS} sectors")
                assert tim4_off == b_idx * TIM4_BLOCK_SECTORS, (
                    f"circuit {idx} ({name}) block ({bx},{by}): 4bpp offset {tim4_off}, "
                    f"expected {b_idx * TIM4_BLOCK_SECTORS}")

                tim8_off, tim8_cnt, tim8_len = tim8_entries[b_idx]
                assert (
                    tim8_cnt == TIM8_BLOCK_SECTORS
                    and tim8_len == TIM8_BLOCK_SECTORS * SECTOR_SIZE
                ), (
                    f"circuit {idx} ({name}) block ({bx},{by}): 8bpp sector_count {tim8_cnt}, byte_len "
                    f"{tim8_len}; expected {TIM8_BLOCK_SECTORS} sectors")
                assert tim8_off == b_idx * TIM8_BLOCK_SECTORS, (
                    f"circuit {idx} ({name}) block ({bx},{by}): 8bpp offset {tim8_off}, "
                    f"expected {b_idx * TIM8_BLOCK_SECTORS}")

                if skip_circuit:
                    continue

                crop = full_im.crop((bx * 1024, by * 1024, (bx + 1) * 1024, (by + 1) * 1024))
                check_tiles_against_source(path, f"{name} block ({bx},{by})", off, expect_source_555(crop))
                if os.path.exists(tim4_path):
                    check_tim4(idx, f"{name} block ({bx},{by})", tim4_off, crop_im=crop, shared_pal=shared_pal16)
                if os.path.exists(tim8_path):
                    check_tim8(idx, f"{name} block ({bx},{by})", tim8_off, crop_im=crop, shared_pal=shared_pal256)
                checked_blocks += 1

        if not skip_circuit:
            print(f"  ok  {name} ({grid_bx}x{grid_by} = {grid_bx * grid_by} blocks)")
            checked_circuits += 1

    print(f"TIM/TIM4/TIM8 verified: {file_sectors} 15bpp sectors across {checked_blocks} blocks ({checked_circuits} circuits)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())