#!/usr/bin/env python3
"""
Compile the authored circuits into `crates/arduracer-core/src/levels.rs`.

    python3 tools/track_cook/generate_svg_circuit.py   # SVG -> tracks/*.png
    python3 tools/track_cook/build_atlas.py            # tracks/*.png -> levels.rs
    cargo run --manifest-path tools/playtest/Cargo.toml --release

This is the single source of truth for circuit geometry, replacing the old
parametric cooker. It runs `compile_circuit.py`'s validation over every image
first and refuses to emit if any circuit fails, so a broken map can never reach
the game.

Each circuit becomes a `TrackDef` built from its compiled surface grid and its
centreline. The surface grid is exactly the tile grid `TrackDef` already wants:
a colour image is a new *authoring* format for an existing runtime type, not a
new type. That is deliberate -- it keeps the game running while the renderer is
replaced, and it means the image pipeline can be adopted without a flag day.

Several circuits, replicated round-robin across every slot
---------------------------------------------------------

`ALL_TRACKS` is `[&TrackDef; 24]` and the game's cup, championship and
track-select code all index it positionally, so it cannot shrink to the number of
circuits that exist without touching every one of those. This emits each authored
`TrackDef` several times, `ALL_TRACKS[i] = AUTHORED_TRACKS[i % N]`. That is honest
about what exists -- four geometries, reachable from every slot -- and it means
the cup structure still divides by six without a rewrite.

Round-robin rather than "first N slots then repeats" because the cups are
six tracks each: with four circuits, blocks would put all four in cup 1 and leave
cups 2-4 as the same four again, while round-robin gives every cup one of each and
varies the running order within it. `cargo fmt` reflows the emitted array either
way, so both are the same size on disk.

Each circuit is replicated as a *pointer*, not a copy. The tiles are one static
per circuit referenced from every slot that names it, so RAM cost is
`N * (80*80 + 320*320/2)` bytes -- about 4 x 57 KB -- against a 999 KB ceiling
that the rest of the game already fills to 47%.

`ALL_TRACK_VISUALS` carries the same mapping for the renderer. `visual_tex::PACKED`
holds one texture per authored circuit, so `gpu::tracktex` is handed a slot index
and needs to know which circuit that slot is; emitting the table here from the
same loop that built `ALL_TRACKS` is what keeps the two from drifting.
"""
import numpy as np
from PIL import Image

import json
import os
import subprocess
import sys
import importlib.util

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
sys.path.insert(0, HERE)

_spec = importlib.util.spec_from_file_location(
    "cc", os.path.join(HERE, "compile_circuit.py"))
assert _spec is not None and _spec.loader is not None
cc = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(cc)

# The drawing step, imported for its two constants the compiler has to agree
# with: the visual image size, and the palette both images are drawn in. Read
# rather than restated, because `gpu::tracktex`'s page arithmetic is written
# against a particular image size and restating it here means a grid change
# silently emits textures the renderer addresses wrongly.
_gen_spec = importlib.util.spec_from_file_location(
    "gen_svg_circuit", os.path.join(HERE, "generate_svg_circuit.py"))
assert _gen_spec is not None and _gen_spec.loader is not None
gen_svg = importlib.util.module_from_spec(_gen_spec)
_gen_spec.loader.exec_module(gen_svg)

#: Visual image size in texels: `GRID * VISUAL_PX`, square.
VISUAL_SIZE = (gen_svg.SIZE, gen_svg.SIZE)
TILE_DIM = 256
TILES_X = VISUAL_SIZE[0] // TILE_DIM
TILES_Y = VISUAL_SIZE[1] // TILE_DIM
TILE_COUNT = TILES_X * TILES_Y

#: The level palette, in code order. Written once into `visual_tex.rs` and
#: shared by every circuit: the palette is part of the level format
#: (`palette.json`), not of any one drawing, so every circuit's packed texture
#: indexes the same CLUT and the GPU needs one upload of it per track load.
CLUT_RGB = [tuple(int(v) for v in gen_svg.RGB_TUPLE[i]) for i in range(16)]

TILE_SIZE = 64

#: Slots in `ALL_TRACKS`. Fixed by the game's positional indexing, not by how
#: many circuits are authored.
TRACK_SLOTS = 24

#: Checkpoints per circuit. `MAX_TRACK_CHECKPOINTS` is 24; six is what the lap
#: timer needs to notice a skipped gate on a single-lap oval without making the
#: gate array mostly padding.
CHECKPOINTS = 6

#: Upper bound on route control points. The runtime samples the spline 8 times per
#: span (`DEFAULT_SAMPLES_PER_SPAN`) into a 768-sample reservoir, so 96 spans is
#: the ceiling. The compiler emits *fewer* on a short circuit, because control
#: points closer than a cell collapse onto a shared tile centre and break the
#: spline -- see `compile_circuit.control_points`.
ROUTE_NODES = 96

#: Where `playtest --calibrate` writes measured lap times.
PAR_CALIBRATION = os.path.join(HERE, "par_calibration.json")

#: Fallback par times, in ticks, when a circuit has no measurement. 3600 ticks is
#: 60 seconds at 60 Hz -- generous on purpose, since an uncalibrated par should
#: be beatable rather than impossible.
FALLBACK_PAR = (5400, 4800, 4300, 3900)


def calibrated_par(name: str) -> tuple[int, int, int, int]:
    """Measured par times for `name`, or the fallback.

    Read from `par_calibration.json` rather than hardcoded, which is the missing
    link `STAGE-ATLAS.md` handover item 2 describes: the calibration pass had been
    measuring real laps and writing them to a JSON file that this pipeline never
    opened, so the numbers in `levels.rs` could never be anything but placeholders.

    The calibration file lists one entry per *race*, not per circuit, so a circuit
    appears several times. Tiers are keyed by name because `bronze`/`silver` are
    multiplier-derived and `gold`/`dev` are measured directly, so the first entry
    carrying each is the one to take.
    """
    if not os.path.exists(PAR_CALIBRATION):
        return FALLBACK_PAR
    try:
        with open(PAR_CALIBRATION) as f:
            data = json.load(f)
    except (OSError, ValueError):
        return FALLBACK_PAR

    found: dict[str, int] = {}
    for entry in data.get("tracks", []):
        if entry.get("track", "").lower() != name.lower():
            continue
        for tier in ("bronze", "silver", "gold", "dev"):
            if tier not in found and isinstance(entry.get(tier), int):
                found[tier] = entry[tier]

    if len(found) != 4:
        return FALLBACK_PAR
    return (found["bronze"], found["silver"], found["gold"], found["dev"])


def build_one(name: str) -> tuple:
    """Generate both images for `name` into `tracks/`, then compile them."""
    outdir = os.path.join(ROOT, "tracks")
    stem = os.path.join(outdir, name.replace(" ", "_"))
    data_png = stem + ".data.png"
    palette_json = stem + ".palette.json"
    line_json = stem + ".line.json"

    for p in (data_png, palette_json, line_json):
        if not os.path.exists(p):
            raise SystemExit(
                f"missing {p}\n"
                f"Run tools/track_cook/generate_svg_circuit.py first -- this "
                f"script compiles authored images, it does not draw them."
            )

    codes, pal, unknown = cc.load_data(data_png, palette_json)
    cell_px = json.load(open(palette_json))["cell_px"]

    errors: list[str] = []
    grid = None
    res = cc.validate(codes, cell_px, errors)
    if res is not None:
        grid, _ = res
    if errors:
        return None, errors
    if grid is None:
        return None, ["no grid"]

    with open(line_json) as f:
        d = json.load(f)
    # Kept at full float precision. Rounding to whole cells here is what made
    # the start heading collapse to north -- see `emit_rust`.
    centre = [(p[0], p[1]) for p in d["samples"]]
    if len(centre) < 3:
        return None, [f"centreline has only {len(centre)} samples"]
    return (grid, centre), []


def fmt_generated(path: str) -> None:
    """Runs `cargo fmt` over a freshly written generated file.

    The generator's output is not `rustfmt`-clean and making it so by hand is not
    an option: `levels.rs` is 30,000 lines of tile literals and control points,
    regenerated on every circuit change. `make ci` runs `fmt-check` over the
    committed tree, so a generated file that is not formatted fails CI the moment
    anything is regenerated -- which means the next person to touch a circuit
    finds a red build about formatting on a file they did not edit.

    Formatting here rather than leaving it to a `make fmt` afterwards for the
    same reason: the file is generated, so the generator is the only place that
    can be relied on to have formatted it. Emitting one-item-per-line (see the
    `ALL_TRACKS` emit) is a separate measure -- it keeps the emit readable and
    stops `rustfmt` reflowing a 24-entry array into something nobody can diff --
    but it does not make the output stable, and this does.

    Best effort, and deliberately quiet on failure. A tarball build with no
    `cargo` on PATH still gets correct levels.rs; it just gets it unformatted,
    which is what the pre-`fmt_generated` pipeline produced anyway.
    """
    manifest = os.path.join(ROOT, "crates", "arduracer-core", "Cargo.toml")
    try:
        subprocess.run(["cargo", "fmt", "--manifest-path", manifest],
                       cwd=ROOT, check=False, capture_output=True)
    except OSError:
        pass


def git(args: list[str]) -> None:
    """Runs git, ignoring failure.

    Used only to drop superseded generated images, and only when this script runs
    from a git checkout. A tarball build has no git and still produces correct
    output; it just keeps the stale files on disk.
    """
    try:
        subprocess.run(["git"] + args, cwd=ROOT, check=False,
                       capture_output=True)
    except OSError:
        pass


def lz_stream_compress(src: bytes, window: int = 8192, max_chain: int = 48) -> bytes:
    """Compresses raw 15-bit BGR555 halfwords using a fast streaming LZ codec."""
    dst = bytearray()
    i = 0
    n = len(src)
    MAX_MATCH = 258
    MIN_MATCH = 3
    while i < n:
        flag_pos = len(dst)
        dst.append(0)
        flag = 0
        bit = 0
        while bit < 8 and i < n:
            win_start = max(0, i - window)
            best_len = 0
            best_dist = 0
            target = src[i : i + MAX_MATCH]
            target_len = len(target)
            if target_len >= MIN_MATCH:
                p = src.rfind(target[:MIN_MATCH], win_start, i)
                chain = 0
                while p != -1 and p >= win_start and chain < max_chain:
                    mlen = 0
                    while mlen < target_len and src[p + mlen] == target[mlen]:
                        mlen += 1
                    if mlen > best_len:
                        best_len = mlen
                        best_dist = i - p
                        if best_len == MAX_MATCH:
                            break
                    p = src.rfind(target[:MIN_MATCH], win_start, p)
                    chain += 1
            if best_len >= MIN_MATCH:
                dst.append((best_dist >> 8) & 0xFF)
                dst.append(best_dist & 0xFF)
                dst.append((best_len - 3) & 0xFF)
                i += best_len
            else:
                flag |= 1 << bit
                dst.append(src[i])
                i += 1
            bit += 1
        dst[flag_pos] = flag
    return bytes(dst)


def compress_1024_jpeg_500kb(im: Image.Image) -> bytes:
    """Compresses an image to a 1024x1024 baseline JPEG block targeting <= 500 KB (512,000 bytes).

    Uses restart_marker_blocks=4 (DRI=4) so that each 64x16 MCU row segment is preceded by
    a restart marker (0xFFD0..0xFFD7), allowing independent 64x64 block decompression directly
    into VRAM with zero CD-ROM access during gameplay.
    """
    import io

    if im.size != (1024, 1024):
        im = im.resize((1024, 1024), Image.Resampling.LANCZOS)

    TARGET_BYTES = 500 * 1024  # 512,000 bytes = exactly 250 CD sectors
    low = 5
    high = 98
    best_data = None

    while low <= high:
        mid = (low + high) // 2
        buf = io.BytesIO()
        im.save(buf, format="JPEG", quality=mid, restart_marker_blocks=4)
        data = buf.getvalue()
        if len(data) <= TARGET_BYTES:
            best_data = data
            low = mid + 1
        else:
            high = mid - 1

    if best_data is None:
        buf = io.BytesIO()
        im.save(buf, format="JPEG", quality=5, restart_marker_blocks=4)
        best_data = buf.getvalue()

    assert len(best_data) <= TARGET_BYTES, f"JPEG exceeds 500 KB: {len(best_data)} bytes"
    return best_data


def pack_visual(name: str) -> list[bytes]:
    """Packs one circuit's visual image into a 1024x1024 JPEG block (<= 500 KB) and exports it."""
    clean_name = name.replace(" ", "_")
    stem = os.path.join(ROOT, "tracks", clean_name)
    vis_path = stem + ".visual.png"
    gen_path = stem + ".visual.gen.png"
    jpg_path = stem + ".jpg"

    # Prioritize visual.gen.png if present, otherwise visual.png, or existing .jpg
    if os.path.exists(gen_path):
        im = Image.open(gen_path).convert("RGB")
        src_desc = f"tracks/{clean_name}.visual.gen.png"
    elif os.path.exists(vis_path):
        im = Image.open(vis_path).convert("RGB")
        src_desc = f"tracks/{clean_name}.visual.png"
    elif os.path.exists(jpg_path):
        im = Image.open(jpg_path).convert("RGB")
        src_desc = f"tracks/{clean_name}.jpg"
    else:
        raise FileNotFoundError(f"No visual image found for {name} ({vis_path} or {gen_path})")

    jpeg_data = compress_1024_jpeg_500kb(im)

    # Export to tracks/<Circuit>.jpg
    with open(jpg_path, "wb") as f_jpg:
        f_jpg.write(jpeg_data)

    # Export to dist/textures/<Circuit>.jpg
    dist_dir = os.path.join(ROOT, "dist", "textures")
    os.makedirs(dist_dir, exist_ok=True)
    dist_jpg = os.path.join(dist_dir, f"{clean_name}.jpg")
    with open(dist_jpg, "wb") as f_dist:
        f_dist.write(jpeg_data)

    print(f"  exported JPEG: {jpg_path} ({len(jpeg_data)} bytes = {len(jpeg_data)/1024:.2f} KB from {src_desc})")
    return [jpeg_data]


def emit_visual_tex(packed_per_circuit: list[list[bytes]]) -> None:
    """Writes `assets/TRACKS.BIN` and `crates/arduracer-core/src/visual_tex.rs`.

    All 1024x1024 JPEG blocks are written sector-aligned (100 sectors = 200 KB each) into
    `assets/TRACKS.BIN` on the CD-ROM disc image. `visual_tex.rs` receives the sector index table
    (offset, sector count, and byte len), completely decoupling runtime textures from static executable size.
    """
    assert packed_per_circuit, "no circuits to pack a visual for"
    out_rs = os.path.join(ROOT, "crates", "arduracer-core", "src", "visual_tex.rs")
    assets_dir = os.path.join(ROOT, "assets")
    os.makedirs(assets_dir, exist_ok=True)
    out_bin = os.path.join(assets_dir, "TRACKS.BIN")

    n = len(packed_per_circuit)
    SECTOR_SIZE = 2048
    SECTORS_PER_CIRCUIT = 250
    SECTORS_PER_CITY = 100
    BLOCK_MAX_BYTES = 512000

    # Build TRACKS.BIN with sector-aligned blocks
    current_sector = 0
    tile_entries: list[tuple[int, int, int]] = []

    with open(out_bin, "wb") as f_bin:
        for c, blocks in enumerate(packed_per_circuit):
            assert len(blocks) >= 1, f"Circuit {c} has no blocks"
            packed = blocks[0]
            byte_len = len(packed)
            sector_count = SECTORS_PER_CIRCUIT
            tile_entries.append((current_sector, sector_count, byte_len))
            f_bin.write(packed)
            pad_len = (sector_count * SECTOR_SIZE) - byte_len
            if pad_len > 0:
                f_bin.write(b"\x00" * pad_len)
            current_sector += sector_count

    total_sectors = current_sector
    total_bin_bytes = total_sectors * SECTOR_SIZE

    # Emit visual_tex.rs with block sector index
    with open(out_rs, "w") as f:
        f.write(
            "//! Per-circuit visual texture metadata and JPEG streaming index.\n"
            "//!\n"
            "//! Generated by `tools/track_cook/build_atlas.py`. Do not edit by hand.\n"
            "\n"
            "/// Visual block dimension in texels (1024x1024).\n"
            "pub const BLOCK_DIM: usize = 1024;\n"
            "/// Width of a single visual block in texels.\n"
            "pub const WIDTH: usize = 1024;\n"
            "/// Height of a single visual block in texels.\n"
            "pub const HEIGHT: usize = 1024;\n"
            "/// Decompressed VRAM tile dimension in texels (64x64).\n"
            "pub const TILE_DIM: usize = 64;\n"
            "/// Number of 64x64 tiles along each axis of a 1024x1024 block (16).\n"
            "pub const TILES_PER_BLOCK_AXIS: usize = 16;\n"
            "/// Total number of 64x64 tiles per 1024x1024 block (256).\n"
            "pub const TILES_PER_BLOCK: usize = 256;\n"
            "/// Number of authored circuits, one 1024x1024 JPEG block each.\n"
            f"pub const COUNT: usize = {n};\n"
            "/// Maximum bytes per 1024x1024 compressed JPEG block (500 KB = 512,000 bytes).\n"
            f"pub const BLOCK_MAX_BYTES: usize = {BLOCK_MAX_BYTES};\n"
            "/// Number of 2048-byte CD sectors allocated per JPEG circuit block (250 sectors = 500 KB).\n"
            f"pub const BLOCK_SECTORS: usize = {SECTORS_PER_CIRCUIT};\n"
            "/// Restart marker interval in MCUs (4 MCUs = 64x16 pixels).\n"
            "pub const RESTART_INTERVAL: usize = 4;\n"
            "/// Total restart intervals in one 1024x1024 block.\n"
            "pub const RESTART_INTERVAL_COUNT: usize = 1024;\n"
            "/// Raw uncompressed 16-bit BGR555 halfwords per 64x64 tile.\n"
            "pub const RAW_TILE_HALFWORDS: usize = TILE_DIM * TILE_DIM;\n"
            "/// Raw uncompressed byte count per 64x64 tile (8,192 bytes = 8 KB).\n"
            "pub const RAW_TILE_BYTES: usize = RAW_TILE_HALFWORDS * 2;\n"
            "/// Name of the track visual asset file on the CD-ROM disc.\n"
            "pub const TRACKS_BIN_NAME: &[u8] = b\"TRACKS.BIN\";\n"
            "/// Name of the Cape Town 10 km^2 city visual asset file on the CD-ROM disc.\n"
            "pub const CAPETOWN_BIN_NAME: &[u8] = b\"CAPETOWN.BIN\";\n"
            "/// Name of the Melbourne 10 km^2 city visual asset file on the CD-ROM disc.\n"
            "pub const MELBOURNE_BIN_NAME: &[u8] = b\"MELBOURNE.BIN\";\n"
            "/// Name of the London 10 km^2 city visual asset file on the CD-ROM disc.\n"
            "pub const LONDON_BIN_NAME: &[u8] = b\"LONDON.BIN\";\n"
            "\n"
            "/// CD-ROM sector index entry for a compressed 1024x1024 JPEG block in TRACKS.BIN.\n"
            "#[derive(Copy, Clone, Debug, PartialEq, Eq)]\n"
            "pub struct TileSectorEntry {\n"
            "    /// 2048-byte sector offset from the start of TRACKS.BIN.\n"
            "    pub sector_offset: u32,\n"
            "    /// Number of 2048-byte CD sectors allocated for this block.\n"
            "    pub sector_count: u32,\n"
            "    /// Exact payload length of the compressed JPEG stream in bytes.\n"
            "    pub byte_len: u32,\n"
            "}\n"
            "\n"
            "/// Streaming compressed 1024x1024 JPEG block sector index [circuit].\n"
            f"pub static CIRCUIT_BLOCK_SECTORS: [TileSectorEntry; COUNT] = [\n"
        )
        for c, (sec_off, sec_cnt, b_len) in enumerate(tile_entries):
            f.write(
                f"    TileSectorEntry {{ sector_offset: {sec_off}, "
                f"sector_count: {sec_cnt}, byte_len: {b_len} }},\n"
            )
        f.write("];\n\n")

        # Emit city block sector mappings from local tracks/<city>_10km/ blocks
        for city_prefix, var_name, city_title in [
            ("capetown", "CAPETOWN_BLOCK_SECTORS", "Cape Town"),
            ("melbourne", "MELBOURNE_BLOCK_SECTORS", "Melbourne"),
            ("london", "LONDON_BLOCK_SECTORS", "London"),
        ]:
            city_dir = os.path.join(ROOT, "tracks", f"{city_prefix}_10km")
            f.write(f"/// {city_title} 10 km^2 3x3 block sector mapping [block_y * 3 + block_x].\n")
            f.write(f"pub static {var_name}: [TileSectorEntry; 9] = [\n")
            current_sec = 0
            for by in range(3):
                for bx in range(3):
                    jpg = os.path.join(city_dir, f"{city_prefix}_b{bx}_b{by}.jpg")
                    b_sz = os.path.getsize(jpg) if os.path.exists(jpg) else 200000
                    f.write(
                        f"    TileSectorEntry {{ sector_offset: {current_sec}, "
                        f"sector_count: {SECTORS_PER_CITY}, byte_len: {b_sz} }},\n"
                    )
                    current_sec += SECTORS_PER_CITY
            f.write("];\n\n")

        f.write("/// Alias matching single-entry lookup for backwards compatibility.\n")
        f.write("pub static CIRCUIT_TILE_SECTORS: [TileSectorEntry; COUNT] = CIRCUIT_BLOCK_SECTORS;\n\n")

        f.write(
            "/// The 16 level-palette core reference colours in code order 0..=15.\n"
            "pub static CLUT_RGB: [(u8, u8, u8); 16] = [\n"
        )
        for r, g, b in CLUT_RGB:
            f.write(f"    ({r}, {g}, {b}),\n")
        f.write("];\n")

    fmt_generated(out_rs)
    print(
        f"wrote {out_bin} ({total_sectors} sectors = {total_bin_bytes} bytes) and "
        f"{out_rs} ({n} circuits x 1024x1024 100KB JPEG blocks)"
    )



def main() -> int:
    # Importing the generator for its CIRCUITS table keeps the name list in one
    # place: the drawing step and the compiling step cannot disagree about what
    # exists.
    import generate_svg_circuit as gen

    results = []
    failures = []
    for spec_ in gen.CIRCUITS:
        name = spec_["name"]
        data, errors = build_one(name)
        if errors:
            failures.append((name, errors))
            print(f"  {name:16} FAILED", file=sys.stderr)
            for e in errors:
                print(f"     {e}", file=sys.stderr)
            continue
        grid, centre = data
        results.append((name, grid, centre))
        print(f"  {name:16} {grid.shape[1]}x{grid.shape[0]} cells, "
              f"centreline {len(centre)}")

    if failures:
        print(f"\n{len(failures)} circuit(s) failed validation; not emitting",
              file=sys.stderr)
        return 1

    if not results:
        print("\nno authored circuits; not emitting", file=sys.stderr)
        return 1

    # More circuits than slots would silently drop some of them: `ALL_TRACKS` is
    # filled round-robin, so slot 24 would be the only place a fifth circuit
    # appeared and it would appear nowhere. Refused here rather than discovered in
    # the track-select carousel, where a missing circuit looks like a bug in the
    # carousel.
    if len(results) > TRACK_SLOTS:
        print(f"\n{len(results)} circuits authored but only {TRACK_SLOTS} "
              f"slots exist; not emitting", file=sys.stderr)
        return 1

    # The 23 superseded image sets from the parametric pipeline are still tracked
    # in `tracks/`. Nothing reads them -- `build_atlas` iterates
    # `generate_svg_circuit.CIRCUITS` -- so they are dead weight that looks like
    # the source of truth. Removed here, where the irrelevance is provable, rather
    # than by hand.
    keep = {n.replace(" ", "_") for n, _, _ in results}
    stale = [f for f in sorted(os.listdir(os.path.join(ROOT, "tracks")))
             if f.endswith((".data.png", ".visual.png", ".visual.gen.png", ".visual.detailed.png",
                            ".line.json", ".palette.json"))
             and not any(f.startswith(k) for k in keep)]
    if stale:
        print(f"\nremoving {len(stale)} superseded image file(s) for the "
              f"circuits that no longer exist", file=sys.stderr)
        git(["rm", "-q", "--"] + [f"tracks/{f}" for f in stale])

    idents = ["TRACK_" + "".join(c if c.isalnum() else "_" for c in n).upper() for n, _, _ in results]
    code = [
        "//! Racetrack definitions for Arduracer PSX.",
        "//!",
        "//! Generated by `tools/track_cook/build_atlas.py` from the authored",
        "//! colour-coded images in `tracks/`. Do not edit by hand.",
        "",
        "use crate::math::{Fixed, Vec2};",
        "use crate::timing::{CheckpointGate, ParTimes};",
        "use crate::track::{TrackDef, TrackTile};",
        "",
        "const EMPTY_GATE: CheckpointGate = CheckpointGate { x: 0, y: 0, width: 0, height: 0 };",
        "",
    ]
    for (name, grid, centre), ident in zip(results, idents):
        code.append(f"static {ident}_TILES: "
                    f"[TrackTile; {grid.shape[1] * grid.shape[0]}] = [")
        flat = []
        for y in range(grid.shape[0]):
            for x in range(grid.shape[1]):
                flat.append(cc.CODE_TO_TILE.get(int(grid[y, x]), "OffRoad"))
        while len(flat) % 16:
            flat.append("OffRoad")
        for i in range(0, len(flat), 16):
            code.append("    " + ", ".join(flat[i:i + 16]) + ",")
        code.append("];")
        # Route control points. Capped by the runtime: `MAX_ROUTE_SAMPLES / 8`
        # spans, where 8 is `DEFAULT_SAMPLES_PER_SPAN`. The compiler derives the
        # actual count from the circuit's perimeter, since control points closer
        # than a cell collapse onto a shared tile centre -- see
        # `compile_circuit.control_points`.
        par = calibrated_par(name)
        # The road half-width is specified in world units by the generator and
        # converted here, rather than left as a cell count. The cell size has
        # already changed once (64 -> 32 world units); a cell count baked into
        # this call would have halved the width of every road in the game the
        # next time it did, with nothing failing -- `half_width` is just a number
        # in a struct literal.
        code.append(cc.emit_rust(
            name.replace(" ", "_"), grid, centre, ident=ident,
            checkpoints=CHECKPOINTS, par=par, route_nodes=ROUTE_NODES,
            half_width_cells=gen.ROAD_HALF_WORLD / cc.TILE_SIZE))
        code.append("")
    assert int(gen.ROAD_HALF_WORLD / cc.TILE_SIZE * cc.TILE_SIZE) == gen.ROAD_HALF_WORLD, (
        "ROAD_HALF_WORLD is not a whole number of cells wide at the current "
        "TILE_SIZE; the compiled road would be narrower than authored"
    )
    # The generator sizes its shapes in cells; the compiler sizes the world. If
    # those two definitions of a cell disagree, every circuit silently changes
    # size and nothing downstream can tell -- the images still validate and the
    # tiles still compile. They are the same constant, asserted rather than
    # restated.
    assert gen.WORLD_PER_CELL == cc.TILE_SIZE, (
        f"generator assumes {gen.WORLD_PER_CELL} world units per cell, the engine "
        f"says {cc.TILE_SIZE}; every circuit would be the wrong size"
    )
    assert gen.GRID <= cc.MAX_TRACK_DIM, (
        f"the generator authors {gen.GRID} cells per side but the engine "
        f"addresses at most {cc.MAX_TRACK_DIM}"
    )
    assert gen.SIZE == VISUAL_SIZE[0] == VISUAL_SIZE[1], (
        f"generator draws {gen.SIZE}x{gen.SIZE} visuals; this script expects "
        f"{VISUAL_SIZE[0]}x{VISUAL_SIZE[1]}"
    )

    code.append(f"/// Authored circuits: {len(results)}.")
    code.append(f"pub const AUTHORED_TRACK_COUNT: usize = {len(results)};")
    code.append("")
    code.append("/// The authored circuits, deduplicated.")
    code.append("///")
    code.append(f"/// `ALL_TRACKS` is a fixed {TRACK_SLOTS} slots and currently holds these")
    code.append(f"/// {len(results)} repeated round-robin, so anything that wants *the circuits*")
    code.append("/// rather than *the slots* wants this. Without it a caller iterating")
    code.append("/// `ALL_TRACKS` to validate circuits walks the same oval many times and")
    code.append("/// reports many passes where there is one thing that could have passed.")
    code.append(f"pub const AUTHORED_TRACKS: [&TrackDef; AUTHORED_TRACK_COUNT] = "
                f"[{', '.join('&' + n for n in idents)}];")
    code.append("")
    code.append(f"pub const ALL_TRACKS: [&TrackDef; {TRACK_SLOTS}] = [")
    # One entry per line. `cargo fmt` reflows a repeated item that would fit on
    # shared lines, so a compact emit is not stable under `fmt-check` -- the
    # generator and the formatter fight and CI fails on generated output.
    slot_of = [idents[i % len(idents)] for i in range(TRACK_SLOTS)]
    for ident in slot_of:
        code.append(f"    &{ident},")
    code.append("];")
    code.append("")
    # Which authored circuit each slot's *visual* belongs to, for the renderer.
    #
    # `gpu::tracktex` has to upload the right one of `visual_tex::PACKED` when a
    # track loads, and it is handed a slot index, not a `TrackDef` it can compare
    # against anything. Emitting the mapping here -- from the same round-robin that
    # built `ALL_TRACKS` -- is what keeps the two in step. Deriving it in the game
    # as `slot % AUTHORED_TRACK_COUNT` would be the same arithmetic written down
    # twice, and the day the round-robin order changes the renderer would show
    # the wrong circuit's road with nothing failing.
    code.append("/// Authored-circuit index for each `ALL_TRACKS` slot, for the renderer.")
    code.append("///")
    code.append("/// `gpu::tracktex::init_track_texture` takes one of these and uploads the")
    code.append("/// matching entry of `visual_tex::PACKED`.")
    code.append(f"pub const ALL_TRACK_VISUALS: [usize; {TRACK_SLOTS}] = [")
    for i in range(TRACK_SLOTS):
        code.append(f"    {i % len(idents)},")
    code.append("];")
    code.append("")

    out = os.path.join(ROOT, "crates", "arduracer-core", "src", "levels.rs")
    with open(out, "w") as f:
        f.write("\n".join(code))
    emit_visual_tex([pack_visual(n) for n, _, _ in results])
    # Last, so it covers both generated files. `visual_tex.rs` in particular is
    # one 200 KB byte array, which `rustfmt` reflows -- emitting it unformatted and
    # formatting only `levels.rs` left the other one failing `fmt-check`.
    fmt_generated(out)
    print(f"\nwrote {out} ({len(results)} authored circuits, {TRACK_SLOTS} slots)")
    # Per-circuit, not aggregate: a missing calibration is a property of one
    # circuit's par row, and a single summary line would either hide it or blame
    # the wrong circuit. Reported in `CIRCUITS` order so it lines up with the
    # table `generate_svg_circuit.py` printed.
    uncalibrated = []
    for name, _, _ in results:
        par = calibrated_par(name)
        if par == FALLBACK_PAR:
            uncalibrated.append(name)
        else:
            print(f"  {name:16} par measured (bronze/silver/gold/dev "
                  f"{par[0]}/{par[1]}/{par[2]}/{par[3]})")
    if uncalibrated:
        print(f"  WARNING: no calibration for {', '.join(uncalibrated)}; par "
              f"times are placeholders {FALLBACK_PAR}. Run:")
        print("    cargo run --manifest-path tools/playtest/Cargo.toml "
              "--release -- --calibrate")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())