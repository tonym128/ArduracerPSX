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
from dataclasses import dataclass
import struct
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

import qoi_codec

#: Maximum bytes per compressed 1024x1024 block. 500 KiB = 512,000 bytes is
#: exactly 250 CD sectors, which is what the runtime's single 500 KB RAM buffer
#: and the `BLOCK_SECTORS` sector-table ceiling are both sized against.
QOI_TARGET_BYTES = 500 * 1024

# --- ARSV container (SVG raster + surface code map), mirrored in svgtex.rs ---
ARSV_MAGIC = b"ARSV"
ARSV_VERSION = 1
#: magic(4) + version(1) + flags(2) + base_codec(1) + width(4) + height(4)
#:   + grid_w(2) + grid_h(2) + code_bytes(4) + raster_bytes(4) = 28,
#: padded to 32 so the code map and the QOI payload stay naturally aligned.
ARSV_HEADER_BYTES = 32
#: The base raster is stored as a segmented-QOI payload.
ARSV_BASE_QOI = 1

#: Visual image size in texels: `GRID * VISUAL_PX`, square.
VISUAL_SIZE = (gen_svg.SIZE, gen_svg.SIZE)

#: Edge of one uncompressed TIM tile, in texels. The runtime DMAs exactly this
#: into a VRAM slot, so it has to match `visual_tex::TILE_DIM` and the renderer's
#: `VRAM_TILE_DIM`. Named `TIM_`-prefixed because `TILE_DIM` below is the much
#: larger 256-texel atlas chunk used by the physics compiler, and the two have
#: nothing to do with each other.
TIM_TILE_DIM = 64
#: TIM tiles along each axis of a 1024x1024 visual block: 16x16 = 256 tiles.
TIM_TILES_PER_BLOCK_AXIS = 1024 // TIM_TILE_DIM
#: CD sector size. A TIM tile is 64x64x2 = 8192 bytes = 4 sectors, which is what
#: lets the runtime read one tile with a single seekless run of `read_sectors`.
SECTOR_SIZE_TIM = 2048
#: CD sectors per uncompressed TIM tile.
TIM_TILE_SECTORS = TIM_TILE_DIM * TIM_TILE_DIM * 2 // SECTOR_SIZE_TIM
#: CD sectors per uncompressed TIM block: 256 tiles of 4 sectors.
TIM_BLOCK_SECTORS = TIM_TILES_PER_BLOCK_AXIS * TIM_TILES_PER_BLOCK_AXIS * TIM_TILE_SECTORS
#: Palette entries in a 4bpp TIM4 block.
TIM4_PALETTE_ENTRIES = 16
#: Bytes of BGR555 palette entries in one TIM4 block.
TIM4_PALETTE_BYTES = TIM4_PALETTE_ENTRIES * 2
#: A 64x64 4bpp tile is 4096 texels at four per halfword = exactly 2048 bytes,
#: so one tile is exactly one CD sector. This is the property the whole TIM4
#: streaming design rests on.
TIM4_TILE_SECTORS = TIM_TILE_DIM * TIM_TILE_DIM // 2 // SECTOR_SIZE_TIM
#: Halfwords in one 4bpp tile: four indices share a 16-bit word.
TIM4_TILE_HALFWORDS = TIM_TILE_DIM * TIM_TILE_DIM // 4
#: Tiles along each axis of a 1024x1024 TIM4 block.
TIM4_TILES_PER_BLOCK_AXIS = 1024 // TIM_TILE_DIM
#: Sectors per TIM4 block: one palette sector, then one per tile.
TIM4_BLOCK_SECTORS = 1 + TIM4_TILES_PER_BLOCK_AXIS * TIM4_TILES_PER_BLOCK_AXIS * TIM4_TILE_SECTORS
assert TIM_TILE_SECTORS == 4, "a 15bpp TIM tile must be four CD sectors"
assert TIM4_TILE_SECTORS == 1, "a 4bpp TIM4 tile must be exactly one CD sector"
assert TIM_BLOCK_SECTORS == 1024, "an uncompressed 15bpp block is exactly 2 MB"
assert TIM4_BLOCK_SECTORS == 257, "a TIM4 block is one palette sector plus 256 tiles"
# A 4bpp tile is one sector of bytes, a quarter as many halfwords as it has
# texels, and a 32x32 halfword rect. These are three different divisions that
# are easy to confuse with each other and with the 8bpp case.
assert TIM4_TILE_HALFWORDS == TIM_TILE_DIM * TIM_TILE_DIM // 4, "4 texels per halfword"
assert TIM4_TILE_HALFWORDS * 2 == SECTOR_SIZE_TIM, "one 4bpp tile is one CD sector"
assert TIM4_TILE_HALFWORDS == (TIM_TILE_DIM // 2) * (TIM_TILE_DIM // 2), (
    "a 64x64 4bpp image is a 32x32 rect of halfwords, because GP0(A0h) counts "
    "width in words and height in rows and 4bpp packs two texel rows per row"
)
#: Palette entries in an 8bpp TIM8 block.
TIM8_PALETTE_ENTRIES = 256
#: Bytes of BGR555 palette entries in one TIM8 block.
TIM8_PALETTE_BYTES = TIM8_PALETTE_ENTRIES * 2
#: A 64x64 8bpp tile is 4096 texels at two per halfword = exactly 4096 bytes,
#: so one tile is exactly two CD sectors.
TIM8_TILE_SECTORS = TIM_TILE_DIM * TIM_TILE_DIM // SECTOR_SIZE_TIM
#: Halfwords in one 8bpp tile: two indices share a 16-bit word.
TIM8_TILE_HALFWORDS = TIM_TILE_DIM * TIM_TILE_DIM // 2
#: Tiles along each axis of a 1024x1024 TIM8 block.
TIM8_TILES_PER_BLOCK_AXIS = 1024 // TIM_TILE_DIM
#: Sectors per TIM8 block: one palette sector, then two per tile (256 * 2 = 512).
TIM8_BLOCK_SECTORS = 1 + TIM8_TILES_PER_BLOCK_AXIS * TIM8_TILES_PER_BLOCK_AXIS * TIM8_TILE_SECTORS
assert TIM8_TILE_SECTORS == 2, "an 8bpp TIM8 tile must be exactly two CD sectors"
assert TIM8_BLOCK_SECTORS == 513, "a TIM8 block is one palette sector plus 256*2 tile sectors"
assert TIM8_TILE_HALFWORDS == TIM_TILE_DIM * TIM_TILE_DIM // 2, "2 texels per halfword"
assert TIM8_TILE_HALFWORDS * 2 == 2 * SECTOR_SIZE_TIM, "one 8bpp tile is two CD sectors"
assert TIM8_TILE_HALFWORDS == (TIM_TILE_DIM // 2) * TIM_TILE_DIM, (
    "a 64x64 8bpp image is a 32x64 rect of halfwords, because GP0(A0h) counts "
    "width in words and height in rows and 8bpp packs two texels per halfword"
)
TILE_DIM = 256
TILES_X = VISUAL_SIZE[0] // TILE_DIM
TILES_Y = VISUAL_SIZE[1] // TILE_DIM
TILE_COUNT = TILES_X * TILES_Y

#: The level palette, in code order. Written once into `visual_tex.rs` and
#: shared by every circuit: the palette is part of the level format
#: (`palette.json`), not of any one drawing, so every circuit's packed texture
#: indexes the same CLUT and the GPU needs one upload of it per track load.
CLUT_RGB = [tuple(int(v) for v in gen_svg.RGB_TUPLE[i]) for i in range(16)]

# There used to be a `TILE_SIZE = 64` here, a third copy of the cell size,
# left behind when the cell shrank to 32. Nothing failed for as long as it was
# unused; the moment `discover_circuits` used it to convert a road half-width
# from cells to world units, every road in the game compiled half as wide as it
# was drawn, and `half_width` is a number in a struct literal that nothing
# cross-checks. It is gone rather than corrected: a definition nothing needs is
# a definition that can only be wrong. The cell size is `cc.TILE_SIZE`, read out
# of `arduracer_core::track`, and `main` asserts it against the generator's.

#: Slots in `ALL_TRACKS`. Fixed by the game's positional indexing, not by how
#: many circuits are authored.
TRACK_SLOTS = 25

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

#: Sidecar written by the level editor (`tools/level_editor`) next to a circuit's
#: SVG. Its presence is what marks a circuit in `tracks/` as editor-authored, and
#: `road_half_cells` in it is what lets one level be a wide circuit and the next
#: a narrow one.
#:
#: The road half-width used to be a module constant shared by every circuit,
#: because every circuit came from one hard-coded table in
#: `generate_svg_circuit.CIRCUITS`. A level drawn elsewhere has no entry in that
#: table, and reading its width out of the drawn SVG instead of a descriptor
#: would mean parsing the tarmac stroke back out of a file whose whole contract
#: is "exact palette colours and document order". So the editor writes the width
#: down, once, beside the drawing, and this is where it is picked up.
LEVEL_SIDECAR_SUFFIX = ".level.json"


LEVEL_SIDECAR_SUFFIX = ".level.json"


@dataclass
class DiscoveredCircuit:
    name: str
    stem: str
    mode: str           # "grand_prix", "mania", "city"
    cup: str            # "bronze", "silver", "gold", "platinum", "endurance", "mania_cup", etc.
    cup_name: str       # "BRONZE CUP", "MANIA CUP", etc.
    order: int          # index within cup
    road_half_cells: float
    stem_path: str      # path to stem (without extension)
    grid: int = 80
    is_large: bool = False


def find_track_stem(name: str) -> str:
    clean = name.replace(" ", "_")
    tracks_dir = os.path.join(ROOT, "tracks")
    for root, dirs, files in os.walk(tracks_dir):
        if (clean + ".data.png" in files) or (clean + ".svg" in files) or (clean + LEVEL_SIDECAR_SUFFIX in files):
            return os.path.join(root, clean)
    return os.path.join(tracks_dir, clean)


CANONICAL_GP_CUPS = [
    ("bronze", "BRONZE CUP", [
        "Cascade Falls", "Copper Gorge", "Longbow", "Amber Mesa", "Emerald Glen", "Sunburst Valley"
    ]),
    ("silver", "SILVER CUP", [
        "Whispering Pines", "Ironstone Quarry", "Silverstone Mist", "Falcon Crest", "Willow Creek", "Cobalt Run"
    ]),
    ("gold", "GOLD CUP", [
        "Dragon's Spine", "Redwood Pass", "Thunder Basin", "Canyon Mirage", "Apex Predator", "Viper's Tail"
    ]),
    ("platinum", "PLATINUM CUP", [
        "Frostpeak Summit", "Golden Horizon", "Phantom Ravine", "Frost Cascade", "Lunar Gorge", "Onyx Run"
    ]),
    ("endurance", "DIAMOND CUP", [
        "Titan Circuit"
    ]),
]

CANONICAL_MANIA_CUPS = [
    ("mania_cup", "MANIA CUP", [
        "Metro 8", "Speedway Boulevard", "Serpent Canyon", "Alleyway Chicanes", "Oil Rig Slipway", "Apex Arena"
    ]),
]


def discover_circuits_detailed() -> list[DiscoveredCircuit]:
    import generate_svg_circuit as gen
    tracks_dir = os.path.join(ROOT, "tracks")
    default_half = gen.ROAD_HALF_WORLD / cc.TILE_SIZE

    circuits: list[DiscoveredCircuit] = []
    seen: set[str] = set()

    # 1. Discover all Grand Prix cups & tracks
    for cup_id, cup_name, tr_names in CANONICAL_GP_CUPS:
        for order, name in enumerate(tr_names):
            stem = name.replace(" ", "_")
            stem_p = find_track_stem(name)
            if not os.path.exists(stem_p + ".data.png") and not os.path.exists(stem_p + ".svg"):
                continue
            half = default_half
            grid = 160 if name == "Titan Circuit" else 80
            side = stem_p + LEVEL_SIDECAR_SUFFIX
            if os.path.exists(side):
                try:
                    with open(side, encoding="utf-8") as f:
                        meta = json.load(f)
                    half = float(meta.get("road_half_cells", half))
                    grid = int(meta.get("grid", grid))
                except Exception:
                    pass
            circuits.append(DiscoveredCircuit(
                name=name,
                stem=stem,
                mode="grand_prix",
                cup=cup_id,
                cup_name=cup_name,
                order=order,
                road_half_cells=half,
                stem_path=stem_p,
                grid=grid,
                is_large=(grid > 80),
            ))
            seen.add(stem)

    # 2. Discover all Mania cups & tracks
    for cup_id, cup_name, tr_names in CANONICAL_MANIA_CUPS:
        for order, name in enumerate(tr_names):
            stem = name.replace(" ", "_")
            stem_p = find_track_stem(name)
            if not os.path.exists(stem_p + ".data.png") and not os.path.exists(stem_p + ".svg"):
                continue
            half = default_half
            grid = 80
            side = stem_p + LEVEL_SIDECAR_SUFFIX
            if os.path.exists(side):
                try:
                    with open(side, encoding="utf-8") as f:
                        meta = json.load(f)
                    half = float(meta.get("road_half_cells", half))
                    grid = int(meta.get("grid", grid))
                except Exception:
                    pass
            circuits.append(DiscoveredCircuit(
                name=name,
                stem=stem,
                mode="mania",
                cup=cup_id,
                cup_name=cup_name,
                order=order,
                road_half_cells=half,
                stem_path=stem_p,
                grid=grid,
            ))
            seen.add(stem)

    # 3. Discover any additional custom tracks in tracks/grand_prix or tracks/mania
    if os.path.isdir(tracks_dir):
        for root, dirs, files in os.walk(tracks_dir):
            for fname in sorted(files):
                if not fname.endswith(LEVEL_SIDECAR_SUFFIX):
                    continue
                stem = fname[: -len(LEVEL_SIDECAR_SUFFIX)]
                if stem in seen or stem.lower().startswith("cape") or stem.lower().startswith("melb") or stem.lower().startswith("lond"):
                    continue
                stem_p = os.path.join(root, stem)
                if not os.path.exists(stem_p + ".data.png"):
                    continue
                try:
                    with open(os.path.join(root, fname), encoding="utf-8") as f:
                        meta = json.load(f)
                except Exception:
                    continue
                tname = meta.get("name", stem.replace("_", " "))
                half = float(meta.get("road_half_cells", default_half))
                grid = int(meta.get("grid", 80))
                mode = meta.get("mode", "mania" if "mania" in root else "grand_prix")
                cup = meta.get("cup", os.path.basename(root) if root != tracks_dir else "custom")
                cname = meta.get("cup_name", cup.replace("_", " ").upper() + " CUP")
                circuits.append(DiscoveredCircuit(
                    name=tname,
                    stem=stem,
                    mode=mode,
                    cup=cup,
                    cup_name=cname,
                    order=len(circuits),
                    road_half_cells=half,
                    stem_path=stem_p,
                    grid=grid,
                    is_large=(grid > 80),
                ))
                seen.add(stem)

    return circuits


def discover_circuits(detailed: bool = False):
    c = discover_circuits_detailed()
    if detailed:
        return c
    return [(item.name, item.road_half_cells) for item in c]

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


def build_one(name: str, stem_path: str = None) -> tuple:
    """Generate both images for `name` into `tracks/`, then compile them."""
    if stem_path is None:
        stem = find_track_stem(name)
    else:
        stem = stem_path
    clean_name = os.path.basename(stem)
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

    # Emit build/circuits/<clean_name>/levelgrid.bin so make_levels_bin.py
    # always has the compiled disc cell bytes for every circuit.
    circ_build_dir = os.path.join(ROOT, "build", "circuits", clean_name)
    os.makedirs(circ_build_dir, exist_ok=True)
    disc = np.zeros(grid.size, dtype=np.uint8)
    for code_val, variant in cc.CODE_TO_DISC.items():
        disc[grid.ravel() == code_val] = variant
    disc.tofile(os.path.join(circ_build_dir, "levelgrid.bin"))

    with open(line_json) as f:
        d = json.load(f)
    # Kept at full float precision. Rounding to whole cells here is what made
    # the start heading collapse to north -- see `emit_rust`.
    centre = [(p[0], p[1]) for p in d["samples"]]
    if len(centre) < 3:
        return None, [f"centreline has only {len(centre)} samples"]

    cps = None
    level_json = stem + LEVEL_SIDECAR_SUFFIX
    if os.path.exists(level_json):
        try:
            with open(level_json, "r", encoding="utf-8") as f:
                meta = json.load(f)
            gate_fractions = [
                float(m["frac"]) for m in meta.get("marks", [])
                if m.get("kind") == "gate" and "frac" in m
            ]
            if gate_fractions:
                cps = [centre[int(round(frac * len(centre))) % len(centre)] for frac in gate_fractions]
        except Exception:
            pass

    return (grid, centre, cps), []


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


def rasterise_svg_visual(name: str):
    """Rasterises a circuit's authored SVG at full visual resolution.

    Returns `(image, code_map)`.
    """
    stem = find_track_stem(name)
    clean_name = os.path.basename(stem)
    svg_path = stem + ".svg"
    data_path = stem + ".data.png"
    palette_json = stem + ".palette.json"

    vis_w, vis_h = VISUAL_SIZE
    if os.path.exists(palette_json):
        try:
            with open(palette_json) as f_pal:
                meta = json.load(f_pal)
                vis_sz = meta.get("visual_size")
                if vis_sz:
                    vis_w = vis_sz
                    vis_h = vis_sz
        except Exception:
            pass

    if not os.path.exists(svg_path):
        return None, None

    import io as _io

    try:
        import cairosvg

        svg_text = open(svg_path, encoding="utf-8").read()
        png = cairosvg.svg2png(
            bytestring=svg_text.encode("utf-8"),
            output_width=vis_w,
            output_height=vis_h,
            background_color="black",
        )
        assert png is not None, "cairosvg returned no image"
        im = Image.open(_io.BytesIO(png)).convert("RGB")
    except ImportError:
        visual_path = stem + ".visual.png"
        if os.path.exists(visual_path):
            im = Image.open(visual_path).convert("RGB")
        else:
            raise

    if not os.path.exists(data_path):
        return im, None

    data_rgb = np.array(Image.open(data_path).convert("RGB"))
    codes = np.full(data_rgb.shape[:2], -1, dtype=np.int16)
    for code, (r, g, b) in gen_svg.RGB_TUPLE.items():
        codes[(data_rgb[:, :, 0] == r)
              & (data_rgb[:, :, 1] == g)
              & (data_rgb[:, :, 2] == b)] = int(code)
    unknown = int((codes < 0).sum())
    assert unknown == 0, f"{clean_name}: {unknown} data pixels match no palette colour"
    assert data_rgb.shape[0] % gen_svg.DATA_PX == 0, f"{clean_name}: data image is not cell-aligned"
    cell = codes[:: gen_svg.DATA_PX, :: gen_svg.DATA_PX]
    return im, cell


def pack_arsv_from_image(im: Image.Image, cell: np.ndarray | None) -> bytes | None:
    """Builds an `ARSV` block for a 1024x1024 image block and its corresponding cell code map."""
    if im is None or cell is None:
        return None

    rgb = np.array(im, dtype=np.uint8)
    h, w = rgb.shape[:2]
    if (w, h) != (1024, 1024):
        rgb = np.array(im.resize((1024, 1024), Image.Resampling.LANCZOS), dtype=np.uint8)
        w, h = (1024, 1024)

    gh, gw = cell.shape
    flat = cell.astype(np.uint8).reshape(-1)
    assert int(flat.max(initial=0)) <= 0x0F, "code map has values outside the 4-bit range"
    if flat.size % 2:
        flat = np.concatenate([flat, np.zeros(1, dtype=np.uint8)])
    pairs = flat.reshape(-1, 2)
    packed = (pairs[:, 0] & 0x0F) | ((pairs[:, 1] & 0x0F) << 4)

    raster = qoi_codec.encode_block(rgb.tobytes(), w, h, 3)
    code_bytes = packed.tobytes()

    if len(code_bytes) + len(raster) + ARSV_HEADER_BYTES > QOI_TARGET_BYTES:
        return None

    out = bytearray()
    out += ARSV_MAGIC
    out.append(ARSV_VERSION)
    out += (0).to_bytes(2, "big")          # flags, reserved
    out.append(ARSV_BASE_QOI)
    out += w.to_bytes(4, "big")
    out += h.to_bytes(4, "big")
    out += gw.to_bytes(2, "big")
    out += gh.to_bytes(2, "big")
    out += len(code_bytes).to_bytes(4, "big")
    out += len(raster).to_bytes(4, "big")
    out += (0).to_bytes(4, "big")   # reserved, pads the header to 32
    assert len(out) == ARSV_HEADER_BYTES, len(out)
    out += code_bytes
    out += raster
    return bytes(out)


def pack_arsv(name: str) -> bytes | None:
    im, cell = rasterise_svg_visual(name)
    return pack_arsv_from_image(im, cell)


def pack_visual(
    name: str,
) -> tuple[str, int, int, list[tuple[bytes, bytes | None, bytes | None, bytes | None, bytes | None, bytes | None]]]:
    """Packs one circuit's visual image into 1024x1024 blocks and exports them.

    Returns `(name, grid_bx, grid_by, blocks)`.
    """
    stem = find_track_stem(name)
    clean_name = os.path.basename(stem)
    vis_path = stem + ".visual.png"
    gen_path = stem + ".visual.gen.png"
    jpg_path = stem + ".jpg"

    if clean_name.lower() in ("cape_town", "capetown", "melbourne", "london"):
        city_slug = "capetown" if "cape" in clean_name.lower() else clean_name.lower()
        city_dir = os.path.join(ROOT, "tracks", "cities", f"{city_slug}_10km")
        if not os.path.exists(city_dir):
            city_dir = os.path.join(ROOT, "tracks", f"{city_slug}_10km")
        grid_bx = 3
        grid_by = 3
        crops = []
        for by in range(3):
            for bx in range(3):
                p = os.path.join(city_dir, f"{city_slug}_b{bx}_b{by}.jpg")
                crops.append(Image.open(p).convert("RGB"))

        thumb = Image.new("RGB", (384, 384))
        for idx, crop in enumerate(crops):
            bx = idx % 3
            by = idx // 3
            thumb.paste(crop.resize((128, 128), Image.Resampling.BILINEAR), (bx * 128, by * 128))

        pal16_im = thumb.quantize(colors=16, method=Image.Quantize.MEDIANCUT)
        raw16 = pal16_im.getpalette() or []
        shared_pal16 = [(raw16[i * 3], raw16[i * 3 + 1], raw16[i * 3 + 2]) for i in range(16)]
        while len(shared_pal16) < 16:
            shared_pal16.append((0, 0, 0))

        pal256_im = thumb.quantize(colors=256, method=Image.Quantize.MEDIANCUT)
        raw256 = pal256_im.getpalette() or []
        shared_pal256 = [(raw256[i * 3], raw256[i * 3 + 1], raw256[i * 3 + 2]) for i in range(len(raw256) // 3)]
        while len(shared_pal256) < 256:
            shared_pal256.append((0, 0, 0))
        shared_pal256 = shared_pal256[:256]

        blocks = []
        for crop in crops:
            jpeg_data = compress_1024_jpeg_500kb(crop)
            qoi_data = compress_1024_qoi_500kb(crop)
            arsv_data = None
            tim_data = pack_tim_block(crop)
            tim4_data = pack_tim4_block(crop, shared_palette=shared_pal16)
            tim8_data = pack_tim8_block(crop, shared_palette=shared_pal256)
            blocks.append((jpeg_data, qoi_data, arsv_data, tim_data, tim4_data, tim8_data))

        print(f"  packed visual: {name} (3x3 = 9 blocks from tracks/{city_slug}_10km)", flush=True)
        return name, grid_bx, grid_by, blocks

    svg_im, cell = rasterise_svg_visual(name)
    if os.path.exists(gen_path):
        im = Image.open(gen_path).convert("RGB")
        src_desc = f"tracks/{clean_name}.visual.gen.png"
    elif svg_im is not None:
        im = svg_im
        src_desc = f"tracks/{clean_name}.svg (rasterised at {im.size[0]}px)"
    elif os.path.exists(vis_path):
        im = Image.open(vis_path).convert("RGB")
        src_desc = f"tracks/{clean_name}.visual.png"
    elif os.path.exists(jpg_path):
        im = Image.open(jpg_path).convert("RGB")
        src_desc = f"tracks/{clean_name}.jpg"
    else:
        raise FileNotFoundError(f"No visual image found for {name} ({vis_path} or {gen_path})")

    w, h = im.size
    grid_bx = max(1, w // 1024)
    grid_by = max(1, h // 1024)

    # For multi-block circuits (e.g. 4096x4096), compute circuit-wide shared
    # palettes so all blocks share the same CLUT in VRAM.
    shared_pal16 = None
    shared_pal256 = None
    if grid_bx > 1 or grid_by > 1:
        small = im.resize((1024, 1024), Image.Resampling.LANCZOS)
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

    blocks = []
    cell_per_bx = cell.shape[1] // grid_bx if cell is not None else 0
    cell_per_by = cell.shape[0] // grid_by if cell is not None else 0

    for by in range(grid_by):
        for bx in range(grid_bx):
            crop = im.crop((bx * 1024, by * 1024, (bx + 1) * 1024, (by + 1) * 1024))
            cell_crop = None
            if cell is not None:
                cell_crop = cell[by * cell_per_by : (by + 1) * cell_per_by, bx * cell_per_bx : (bx + 1) * cell_per_bx]

            jpeg_data = compress_1024_jpeg_500kb(crop)
            qoi_data = compress_1024_qoi_500kb(crop)
            arsv_data = pack_arsv_from_image(crop, cell_crop)
            tim_data = pack_tim_block(crop)
            tim4_data = pack_tim4_block(crop, shared_palette=shared_pal16)
            tim8_data = pack_tim8_block(crop, shared_palette=shared_pal256)
            blocks.append((jpeg_data, qoi_data, arsv_data, tim_data, tim4_data, tim8_data))

    # Export main JPEG block to tracks/<Circuit>.jpg
    with open(jpg_path, "wb") as f_jpg:
        f_jpg.write(blocks[0][0])
    dist_dir = os.path.join(ROOT, "dist", "textures")
    os.makedirs(dist_dir, exist_ok=True)
    dist_jpg = os.path.join(dist_dir, f"{clean_name}.jpg")
    with open(dist_jpg, "wb") as f_dist:
        f_dist.write(blocks[0][0])

    print(
        f"  packed visual: {clean_name} ({grid_bx}x{grid_by} = {len(blocks)} blocks from {src_desc})",
        flush=True,
    )
    return name, grid_bx, grid_by, blocks


def compress_1024_qoi_500kb(im: Image.Image) -> bytes | None:
    """Encodes an image as a segmented QOI block, or `None` if it does not fit.

    Returns `None` rather than degrading the image, deliberately. QOI is
    lossless, so the only way to shrink an over-budget block is to throw away
    detail beforehand -- quantising, blurring or resampling -- and at the detail
    levels needed to fit a busy 1024x1024 image the result is visibly worse than
    the JPEG the game already streams. Measured on the one circuit whose art
    currently exceeds the budget (`Amber_Mesa.visual.gen.png`): JPEG reaches
    486 KB at RMSE 3.7 against its source, while forcing QOI to fit needs 1-bit
    quantisation at RMSE 39. Shipping a much worse image to save a 4x decode win
    would be the wrong trade, so that circuit keeps its JPEG block and the
    runtime falls back automatically.

    Callers must treat `None` as "this circuit has no QOI block".
    """
    if im.size != (1024, 1024):
        im = im.resize((1024, 1024), Image.Resampling.LANCZOS)

    rgb = im.convert("RGB").tobytes()
    data = qoi_codec.encode_block(rgb, 1024, 1024, 3)

    if len(data) > QOI_TARGET_BYTES:
        return None

    # Guard against an encoder/decoder disagreement before shipping the block:
    # a block the game cannot decode would surface as a checkerboard at runtime,
    # which is far more expensive to diagnose than a failure here.
    assert qoi_codec.decode_block(data, 3) == rgb, f"QOI roundtrip failed: {len(data)} bytes"

    return data


def quantise_16(im: Image.Image, shared_palette=None) -> tuple[np.ndarray, list[tuple[int, int, int]]]:
    """Reduces an image to 16 palette indices with Floyd-Steinberg dithering.

    Returns `(indices, palette)` where `indices` is an HxW `uint8` array of
    palette entries (0..15) and `palette` is a 16-entry list of `(r, g, b)` tuples.
    """
    rgb_im = im.convert("RGB")
    if shared_palette is not None:
        palette = list(shared_palette)[:16]
        while len(palette) < 16:
            palette.append((0, 0, 0))
        pal_im = Image.new("P", (1, 1))
        flat_pal = []
        for r, g, b in palette:
            flat_pal.extend([r, g, b])
        while len(flat_pal) < 768:
            flat_pal.extend(flat_pal[:48])
        pal_im.putpalette(flat_pal[:768])
        q = rgb_im.quantize(palette=pal_im, dither=Image.Dither.FLOYDSTEINBERG)
        indices = (np.array(q, dtype=np.uint8) % 16).astype(np.uint8)
        return indices, palette

    q = rgb_im.quantize(colors=16, method=Image.Quantize.MEDIANCUT, dither=Image.Dither.FLOYDSTEINBERG)
    raw = q.getpalette() or []
    palette: list[tuple[int, int, int]] = []
    for i in range(min(16, len(raw) // 3)):
        palette.append((raw[i * 3], raw[i * 3 + 1], raw[i * 3 + 2]))
    while len(palette) < 16:
        palette.append((0, 0, 0))
    indices = (np.array(q, dtype=np.uint8) % 16).astype(np.uint8)
    return indices, palette


def pack_tim4_block(im: Image.Image, shared_palette=None) -> bytes | None:
    """Packs an image as uncompressed 4bpp CLUT-indexed texels, 16 colours.

    Returns one sector of palette followed by one sector per 64x64 tile.
    """
    if im.size != (1024, 1024):
        im = im.resize((1024, 1024), Image.Resampling.LANCZOS)

    indices, palette = quantise_16(im, shared_palette=shared_palette)

    # Palette sector: 16 BGR555 entries, low halfword first, padded to a sector.
    pal_bytes = bytearray()
    for (r, g, b) in palette:
        half = ((b >> 3) << 10) | ((g >> 3) << 5) | (r >> 3)
        pal_bytes += struct.pack("<H", half)
    assert len(pal_bytes) == TIM4_PALETTE_BYTES, "palette must be exactly 16 entries"
    palette_sector = bytes(pal_bytes) + b"\x00" * (SECTOR_SIZE_TIM - len(pal_bytes))

    tiles = []
    for ty in range(TIM4_TILES_PER_BLOCK_AXIS):
        for tx in range(TIM4_TILES_PER_BLOCK_AXIS):
            tile = indices[
                ty * TIM_TILE_DIM : (ty + 1) * TIM_TILE_DIM,
                tx * TIM_TILE_DIM : (tx + 1) * TIM_TILE_DIM,
            ]
            quads = tile.reshape(TIM_TILE_DIM, TIM_TILE_DIM // 4, 4).astype(np.uint16) & 0x0F
            packed = (
                quads[..., 0] | (quads[..., 1] << 4) | (quads[..., 2] << 8) | (quads[..., 3] << 12)
            )
            tiles.append(np.ascontiguousarray(packed, dtype="<u2").tobytes())

    tile_sector = TIM_TILE_DIM * TIM_TILE_DIM // 4 * 2
    assert all(len(t) == tile_sector for t in tiles), (
        f"a TIM4 tile must be exactly one sector ({tile_sector} bytes), got "
        f"{sorted({len(t) for t in tiles})}"
    )

    block = palette_sector + b"".join(tiles)
    expected = SECTOR_SIZE_TIM + tile_sector * TIM4_TILES_PER_BLOCK_AXIS**2
    assert len(block) == expected, f"TIM4 block is {len(block)} bytes, expected {expected}"
    assert len(block) % SECTOR_SIZE_TIM == 0, "TIM4 block is not sector aligned"
    return block


def quantise_256(im: Image.Image, shared_palette=None) -> tuple[np.ndarray, list[tuple[int, int, int]]]:
    """Reduces an image to 256 palette indices using median-cut and Floyd-Steinberg dithering.

    Returns `(indices, palette)` where `indices` is an HxW `uint8` array of palette entries
    and `palette` is a 256-entry list of `(r, g, b)` tuples.
    """
    rgb_im = im.convert("RGB")
    if shared_palette is not None:
        palette = list(shared_palette)
        pal_im = Image.new("P", (1, 1))
        flat_pal = []
        for r, g, b in palette:
            flat_pal.extend([r, g, b])
        while len(flat_pal) < 768:
            flat_pal.append(0)
        pal_im.putpalette(flat_pal[:768])
        q = rgb_im.quantize(palette=pal_im, dither=Image.Dither.FLOYDSTEINBERG)
        indices = np.array(q, dtype=np.uint8)
        return indices, palette[:256]

    q = rgb_im.quantize(colors=256, method=Image.Quantize.MEDIANCUT, dither=Image.Dither.FLOYDSTEINBERG)
    raw_pal = q.getpalette() or []
    palette: list[tuple[int, int, int]] = []
    for i in range(len(raw_pal) // 3):
        palette.append((raw_pal[i * 3], raw_pal[i * 3 + 1], raw_pal[i * 3 + 2]))
    while len(palette) < 256:
        palette.append((0, 0, 0))
    indices = np.array(q, dtype=np.uint8)
    return indices, palette[:256]


def pack_tim8_block(im: Image.Image, shared_palette=None) -> bytes | None:
    """Packs an image as uncompressed 8bpp CLUT-indexed texels, 256 colours.

    Returns one sector of palette followed by two sectors per 64x64 tile.
    """
    if im.size != (1024, 1024):
        im = im.resize((1024, 1024), Image.Resampling.LANCZOS)

    indices, palette = quantise_256(im, shared_palette=shared_palette)

    # Palette sector: 256 BGR555 entries, low halfword first, padded to a sector.
    pal_bytes = bytearray()
    for (r, g, b) in palette:
        half = ((b >> 3) << 10) | ((g >> 3) << 5) | (r >> 3)
        pal_bytes += struct.pack("<H", half)
    assert len(pal_bytes) == TIM8_PALETTE_BYTES, "palette must be exactly 256 entries"
    palette_sector = bytes(pal_bytes) + b"\x00" * (SECTOR_SIZE_TIM - len(pal_bytes))

    tiles = []
    for ty in range(TIM8_TILES_PER_BLOCK_AXIS):
        for tx in range(TIM8_TILES_PER_BLOCK_AXIS):
            tile = indices[
                ty * TIM_TILE_DIM : (ty + 1) * TIM_TILE_DIM,
                tx * TIM_TILE_DIM : (tx + 1) * TIM_TILE_DIM,
            ]
            pairs = tile.reshape(TIM_TILE_DIM, TIM_TILE_DIM // 2, 2).astype(np.uint16) & 0xFF
            packed = pairs[..., 0] | (pairs[..., 1] << 8)
            tiles.append(np.ascontiguousarray(packed, dtype="<u2").tobytes())

    tile_bytes = TIM_TILE_DIM * TIM_TILE_DIM
    assert all(len(t) == tile_bytes for t in tiles), (
        f"a TIM8 tile must be exactly two sectors ({tile_bytes} bytes), got "
        f"{sorted({len(t) for t in tiles})}"
    )

    block = palette_sector + b"".join(tiles)
    expected = SECTOR_SIZE_TIM + tile_bytes * TIM8_TILES_PER_BLOCK_AXIS**2
    assert len(block) == expected, f"TIM8 block is {len(block)} bytes, expected {expected}"
    assert len(block) % SECTOR_SIZE_TIM == 0, "TIM8 block is not sector aligned"
    return block


def pack_tim_block(im: Image.Image) -> bytes:
    """Packs an image as uncompressed 15bpp texels, tiled for direct DMA.

    Returns exactly 256 tiles of 64x64, four 2048-byte CD sectors each.
    """
    if im.size != (1024, 1024):
        im = im.resize((1024, 1024), Image.Resampling.LANCZOS)

    rgb = np.array(im.convert("RGB"), dtype=np.uint32)
    r5 = (rgb[:, :, 0] >> 3) & 0x1F
    g5 = (rgb[:, :, 1] >> 3) & 0x1F
    b5 = (rgb[:, :, 2] >> 3) & 0x1F
    halfwords = r5 | (g5 << 5) | (b5 << 10)

    tiles = []
    for ty in range(TIM_TILES_PER_BLOCK_AXIS):
        for tx in range(TIM_TILES_PER_BLOCK_AXIS):
            tile = halfwords[
                ty * TIM_TILE_DIM : (ty + 1) * TIM_TILE_DIM,
                tx * TIM_TILE_DIM : (tx + 1) * TIM_TILE_DIM,
            ]
            tiles.append(tile.astype("<u2").tobytes())

    block = b"".join(tiles)
    expected = TIM_TILE_DIM * TIM_TILE_DIM * 2 * (TIM_TILES_PER_BLOCK_AXIS**2)
    assert len(block) == expected, f"TIM block is {len(block)} bytes, expected {expected}"
    assert len(block) % SECTOR_SIZE_TIM == 0, "TIM block is not sector aligned"
    return block


def emit_visual_tex(
    packed_per_circuit: list[
        tuple[str, int, int, list[tuple[bytes, bytes | None, bytes | None, bytes | None, bytes | None, bytes | None]]]
    ]
) -> None:
    """Writes the track texture BINs and `crates/arduracer-core/src/visual_tex.rs`."""
    assert packed_per_circuit, "no circuits to pack a visual for"
    out_rs = os.path.join(ROOT, "crates", "arduracer-core", "src", "visual_tex.rs")
    assets_dir = os.path.join(ROOT, "assets")
    os.makedirs(assets_dir, exist_ok=True)
    out_bin = os.path.join(assets_dir, "TRACKS.BIN")
    out_qoi_bin = os.path.join(assets_dir, "QOITRACKS.BIN")
    out_arsv_bin = os.path.join(assets_dir, "SVGTRACKS.BIN")
    out_tim_bin = os.path.join(assets_dir, "TIMTRACKS.BIN")
    out_tim4_bin = os.path.join(assets_dir, "TIM4TRACKS.BIN")
    out_tim8_bin = os.path.join(assets_dir, "TIM8TRACKS.BIN")

    all_blocks = []
    circuit_block_base = []
    circuit_block_grid = []
    for name, g_bx, g_by, blocks in packed_per_circuit:
        circuit_block_base.append(len(all_blocks))
        circuit_block_grid.append((g_bx, g_by))
        all_blocks.extend(blocks)

    n = len(all_blocks)
    circuit_count = len(packed_per_circuit)
    SECTOR_SIZE = 2048
    SECTORS_PER_CIRCUIT = 250
    SECTORS_PER_CITY = 100
    BLOCK_MAX_BYTES = 512000

    def write_aligned(f_bin, payload: bytes, sector_offset: int) -> int:
        f_bin.write(payload)
        pad_len = (SECTORS_PER_CIRCUIT * SECTOR_SIZE) - len(payload)
        assert pad_len >= 0, f"block of {len(payload)} bytes exceeds its sector allocation"
        if pad_len > 0:
            f_bin.write(b"\x00" * pad_len)
        return sector_offset + SECTORS_PER_CIRCUIT

    # Build the BINs with sector-aligned blocks.
    current_sector = 0
    qoi_sector = 0
    arsv_sector = 0
    tim_sector = 0
    tim4_sector = 0
    tim8_sector = 0
    tile_entries: list[tuple[int, int, int]] = []
    qoi_entries: list[tuple[int, int, int]] = []
    arsv_entries: list[tuple[int, int, int]] = []
    tim_entries: list[tuple[int, int, int]] = []
    tim4_entries: list[tuple[int, int, int]] = []
    tim8_entries: list[tuple[int, int, int]] = []

    with open(out_bin, "wb") as f_bin, open(out_qoi_bin, "wb") as f_qoi, \
            open(out_arsv_bin, "wb") as f_arsv, open(out_tim_bin, "wb") as f_tim, \
            open(out_tim4_bin, "wb") as f_tim4, open(out_tim8_bin, "wb") as f_tim8:
        for c, (jpeg_data, qoi_data, arsv_data, tim_data, tim4_data, tim8_data) in enumerate(
            all_blocks
        ):
            tile_entries.append(
                (current_sector, SECTORS_PER_CIRCUIT, len(jpeg_data))
            )
            current_sector = write_aligned(f_bin, jpeg_data, current_sector)

            if qoi_data is None:
                qoi_entries.append((0, 0, 0))
            else:
                assert len(qoi_data) <= BLOCK_MAX_BYTES, (
                    f"block {c}: QOI block is {len(qoi_data)} bytes, "
                    f"over the {BLOCK_MAX_BYTES}-byte budget"
                )
                qoi_entries.append(
                    (qoi_sector, SECTORS_PER_CIRCUIT, len(qoi_data))
                )
                qoi_sector = write_aligned(f_qoi, qoi_data, qoi_sector)

            if arsv_data is None:
                arsv_entries.append((0, 0, 0))
            else:
                assert len(arsv_data) <= BLOCK_MAX_BYTES, (
                    f"block {c}: ARSV block is {len(arsv_data)} bytes, "
                    f"over the {BLOCK_MAX_BYTES}-byte budget"
                )
                arsv_entries.append(
                    (arsv_sector, SECTORS_PER_CIRCUIT, len(arsv_data))
                )
                arsv_sector = write_aligned(f_arsv, arsv_data, arsv_sector)

            if tim_data is None:
                tim_entries.append((0, 0, 0))
            else:
                assert len(tim_data) % SECTOR_SIZE == 0, (
                    f"block {c}: TIM block is {len(tim_data)} bytes, not sector aligned"
                )
                tim_entries.append(
                    (tim_sector, len(tim_data) // SECTOR_SIZE, len(tim_data))
                )
                f_tim.write(tim_data)
                tim_sector += len(tim_data) // SECTOR_SIZE

            if tim4_data is None:
                tim4_entries.append((0, 0, 0))
            else:
                assert len(tim4_data) % SECTOR_SIZE == 0, (
                    f"block {c}: TIM4 block is {len(tim4_data)} bytes, not sector aligned"
                )
                assert len(tim4_data) // SECTOR_SIZE == TIM4_BLOCK_SECTORS, (
                    f"block {c}: TIM4 block is {len(tim4_data) // SECTOR_SIZE} sectors, "
                    f"expected {TIM4_BLOCK_SECTORS}"
                )
                tim4_entries.append(
                    (tim4_sector, len(tim4_data) // SECTOR_SIZE, len(tim4_data))
                )
                f_tim4.write(tim4_data)
                tim4_sector += len(tim4_data) // SECTOR_SIZE

            if tim8_data is None:
                tim8_entries.append((0, 0, 0))
            else:
                assert len(tim8_data) % SECTOR_SIZE == 0, (
                    f"block {c}: TIM8 block is {len(tim8_data)} bytes, not sector aligned"
                )
                assert len(tim8_data) // SECTOR_SIZE == TIM8_BLOCK_SECTORS, (
                    f"block {c}: TIM8 block is {len(tim8_data) // SECTOR_SIZE} sectors, "
                    f"expected {TIM8_BLOCK_SECTORS}"
                )
                tim8_entries.append(
                    (tim8_sector, len(tim8_data) // SECTOR_SIZE, len(tim8_data))
                )
                f_tim8.write(tim8_data)
                tim8_sector += len(tim8_data) // SECTOR_SIZE

    total_sectors = current_sector
    total_bin_bytes = total_sectors * SECTOR_SIZE
    qoi_sectors_total = qoi_sector
    qoi_count = sum(1 for e in qoi_entries if e[1] > 0)
    arsv_sectors_total = arsv_sector
    arsv_count = sum(1 for e in arsv_entries if e[1] > 0)
    tim_sectors_total = tim_sector
    tim_count = sum(1 for e in tim_entries if e[1] > 0)
    tim4_sectors_total = tim4_sector
    tim4_count = sum(1 for e in tim4_entries if e[1] > 0)
    tim8_sectors_total = tim8_sector
    tim8_count = sum(1 for e in tim8_entries if e[1] > 0)

    # Emit visual_tex.rs with block sector index
    with open(out_rs, "w") as f:
        f.write(
            "//! Per-circuit visual texture metadata and streaming indexes.\n"
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
            "/// Number of authored circuits.\n"
            f"pub const CIRCUIT_COUNT: usize = {circuit_count};\n"
            "/// Total number of 1024x1024 visual blocks across all circuits.\n"
            f"pub const COUNT: usize = {n};\n"
            "/// Maximum bytes per 1024x1024 compressed block (500 KB = 512,000 bytes).\n"
            f"pub const BLOCK_MAX_BYTES: usize = {BLOCK_MAX_BYTES};\n"
            "/// Number of 2048-byte CD sectors allocated per circuit block (250 sectors = 500 KB).\n"
            f"pub const BLOCK_SECTORS: usize = {SECTORS_PER_CIRCUIT};\n"
            "/// Restart marker interval in MCUs (4 MCUs = 64x16 pixels).\n"
            "pub const RESTART_INTERVAL: usize = 4;\n"
            "/// Total restart intervals in one 1024x1024 block.\n"
            "pub const RESTART_INTERVAL_COUNT: usize = 1024;\n"
            "/// Independently decodable 64x16 QOI segments in one 1024x1024 block.\n"
            "pub const SEGMENT_COUNT: usize = 1024;\n"
            "/// CD sectors per uncompressed 15bpp tile (64x64x2 bytes = 8192 = 4 sectors).\n"
            f"pub const TIM_TILE_SECTORS: u32 = {TIM_TILE_SECTORS};\n"
            "/// CD sectors per uncompressed 15bpp block (256 tiles x 4 sectors).\n"
            f"pub const TIM_BLOCK_SECTORS: u32 = {TIM_BLOCK_SECTORS};\n"
            "/// Raw uncompressed 16-bit BGR555 halfwords per 64x64 tile.\n"
            "pub const RAW_TILE_HALFWORDS: usize = TILE_DIM * TILE_DIM;\n"
            "/// Raw uncompressed byte count per 64x64 tile (8,192 bytes = 8 KB).\n"
            "pub const RAW_TILE_BYTES: usize = RAW_TILE_HALFWORDS * 2;\n"
            "/// Name of the track visual asset file on the CD-ROM disc.\n"
            "pub const TRACKS_BIN_NAME: &[u8] = b\"TRACKS.BIN\";\n"
            "/// Name of the segmented-QOI track visual asset file on the CD-ROM disc.\n"
            "pub const QOITRACKS_BIN_NAME: &[u8] = b\"QOITRACKS.BIN\";\n"
            "/// Number of visual blocks with a segmented-QOI block on this disc.\n"
            f"pub const QOI_CIRCUIT_COUNT: usize = {qoi_count};\n"
            "/// Name of the SVG-derived track visual asset file on the CD-ROM disc.\n"
            "pub const SVGTRACKS_BIN_NAME: &[u8] = b\"SVGTRACKS.BIN\";\n"
            "/// Number of visual blocks with an ARSV block on this disc.\n"
            f"pub const ARSV_CIRCUIT_COUNT: usize = {arsv_count};\n"
            "/// Name of the uncompressed-TIM track visual asset file on the CD-ROM disc.\n"
            "pub const TIMTRACKS_BIN_NAME: &[u8] = b\"TIMTRACKS.BIN\";\n"
            "/// Number of visual blocks with an uncompressed TIM block on this disc.\n"
            f"pub const TIM_CIRCUIT_COUNT: usize = {tim_count};\n"
            "/// Name of the uncompressed 4bpp CLUT-indexed track visual asset file.\n"
            "pub const TIM4TRACKS_BIN_NAME: &[u8] = b\"TIM4TRACKS.BIN\";\n"
            "/// Number of visual blocks with an uncompressed 4bpp block on this disc.\n"
            f"pub const TIM4_CIRCUIT_COUNT: usize = {tim4_count};\n"
            "/// Texels per 4bpp TIM tile, on each axis.\n"
            "pub const TIM4_TILE_DIM: usize = TILE_DIM;\n"
            "/// Bytes per 4bpp TIM tile: four indices per halfword.\n"
            "pub const TIM4_TILE_BYTES: usize = TIM4_TILE_DIM * TIM4_TILE_DIM / 2;\n"
            "/// CD sectors per 4bpp TIM tile. 2048 bytes is exactly one sector.\n"
            f"pub const TIM4_TILE_SECTORS: u32 = {TIM4_TILE_SECTORS};\n"
            "/// Colour entries in a 4bpp TIM palette.\n"
            "pub const TIM4_PALETTE_ENTRIES: usize = 16;\n"
            "/// Bytes of 15bpp BGR555 palette entries per circuit.\n"
            "pub const TIM4_PALETTE_BYTES: usize = TIM4_PALETTE_ENTRIES * 2;\n"
            "/// CD sectors holding one circuit's 4bpp palette, padded to a sector.\n"
            f"pub const TIM4_PALETTE_SECTORS: u32 = 1;\n"
            "/// CD sectors per uncompressed 4bpp block.\n"
            f"pub const TIM4_BLOCK_SECTORS: u32 = {TIM4_BLOCK_SECTORS};\n"
            "/// Halfwords of VRAM a 64x64 4bpp image occupies.\n"
            "pub const TIM4_TILE_HALFWORDS: usize = TIM4_TILE_DIM * TIM4_TILE_DIM / TIM4_TEXELS_PER_HALFWORD;\n"
            "/// Texels packed into one VRAM halfword at this depth.\n"
            "pub const TIM4_TEXELS_PER_HALFWORD: usize = 4;\n"
            "/// VRAM rows a 64-texel-tall 4bpp image occupies.\n"
            "pub const TIM4_TILE_ROWS: usize = TIM4_TILE_DIM;\n"
            "/// VRAM halfword columns a 64-texel-wide 4bpp image occupies.\n"
            "pub const TIM4_TILE_HALFWORDS_WIDE: usize = TIM4_TILE_DIM / TIM4_TEXELS_PER_HALFWORD;\n"
            "/// Name of the uncompressed 8bpp CLUT-indexed track visual asset file.\n"
            "pub const TIM8TRACKS_BIN_NAME: &[u8] = b\"TIM8TRACKS.BIN\";\n"
            "/// Number of visual blocks with an uncompressed 8bpp block on this disc.\n"
            f"pub const TIM8_CIRCUIT_COUNT: usize = {tim8_count};\n"
            "/// Texels per 8bpp TIM tile, on each axis.\n"
            "pub const TIM8_TILE_DIM: usize = TILE_DIM;\n"
            "/// Bytes per 8bpp TIM tile: two indices per halfword.\n"
            "pub const TIM8_TILE_BYTES: usize = TIM8_TILE_DIM * TIM8_TILE_DIM;\n"
            "/// CD sectors per 8bpp TIM tile. 4096 bytes is exactly two sectors.\n"
            f"pub const TIM8_TILE_SECTORS: u32 = {TIM8_TILE_SECTORS};\n"
            "/// Colour entries in an 8bpp TIM palette.\n"
            "pub const TIM8_PALETTE_ENTRIES: usize = 256;\n"
            "/// Bytes of 15bpp BGR555 palette entries per circuit.\n"
            "pub const TIM8_PALETTE_BYTES: usize = TIM8_PALETTE_ENTRIES * 2;\n"
            "/// CD sectors holding one circuit's 8bpp palette, padded to a sector.\n"
            f"pub const TIM8_PALETTE_SECTORS: u32 = 1;\n"
            "/// CD sectors per uncompressed 8bpp block.\n"
            f"pub const TIM8_BLOCK_SECTORS: u32 = {TIM8_BLOCK_SECTORS};\n"
            "/// Halfwords of VRAM a 64x64 8bpp image occupies.\n"
            "pub const TIM8_TILE_HALFWORDS: usize = TIM8_TILE_DIM * TIM8_TILE_DIM / TIM8_TEXELS_PER_HALFWORD;\n"
            "/// Texels packed into one VRAM halfword at this depth (2).\n"
            "pub const TIM8_TEXELS_PER_HALFWORD: usize = 2;\n"
            "/// VRAM rows a 64-texel-tall 8bpp image occupies.\n"
            "pub const TIM8_TILE_ROWS: usize = TIM8_TILE_DIM;\n"
            "/// VRAM halfword columns a 64-texel-wide 8bpp image occupies (32).\n"
            "pub const TIM8_TILE_HALFWORDS_WIDE: usize = TIM8_TILE_DIM / TIM8_TEXELS_PER_HALFWORD;\n"
            "/// Name of the disc asset holding every level's tile grid.\n"
            "pub const LEVELS_BIN_NAME: &[u8] = b\"LEVELS.BIN\";\n"
            "/// Name of the Cape Town 10 km^2 city visual asset file on the CD-ROM disc.\n"
            "pub const CAPETOWN_BIN_NAME: &[u8] = b\"CAPETOWN.BIN\";\n"
            "/// Name of the Melbourne 10 km^2 city visual asset file on the CD-ROM disc.\n"
            "pub const MELBOURNE_BIN_NAME: &[u8] = b\"MELBOURNE.BIN\";\n"
            "/// Name of the London 10 km^2 city visual asset file on the CD-ROM disc.\n"
            "pub const LONDON_BIN_NAME: &[u8] = b\"LONDON.BIN\";\n"
            "\n"
            "/// CD-ROM sector index entry for a compressed 1024x1024 block.\n"
            "#[derive(Copy, Clone, Debug, PartialEq, Eq)]\n"
            "pub struct TileSectorEntry {\n"
            "    pub sector_offset: u32,\n"
            "    pub sector_count: u32,\n"
            "    pub byte_len: u32,\n"
            "}\n"
            "\n"
            "/// Base flat block index in `CIRCUIT_*_SECTORS` for each circuit [circuit].\n"
            f"pub static CIRCUIT_BLOCK_BASE: [usize; CIRCUIT_COUNT] = [\n"
        )
        for base in circuit_block_base:
            f.write(f"    {base},\n")
        f.write("];\n\n")

        f.write(
            "/// Number of 1024x1024 blocks along (x, y) for each circuit [circuit].\n"
            f"pub static CIRCUIT_BLOCK_GRID: [(u8, u8); CIRCUIT_COUNT] = [\n"
        )
        for gx, gy in circuit_block_grid:
            f.write(f"    ({gx}, {gy}),\n")
        f.write("];\n\n")

        f.write(
            "/// Returns the `(blocks_x, blocks_y)` grid of 1024x1024 visual blocks for `circuit`.\n"
            "#[inline(always)]\n"
            "pub const fn circuit_block_grid(circuit: usize) -> (usize, usize) {\n"
            "    if circuit < CIRCUIT_COUNT {\n"
            "        let (gx, gy) = CIRCUIT_BLOCK_GRID[circuit];\n"
            "        (gx as usize, gy as usize)\n"
            "    } else {\n"
            "        (1, 1)\n"
            "    }\n"
            "}\n\n"
            "/// Maps `(circuit, block_x, block_y)` to the flat index in `CIRCUIT_*_SECTORS`.\n"
            "#[inline(always)]\n"
            "pub const fn circuit_block_index(circuit: usize, block_x: usize, block_y: usize) -> usize {\n"
            "    if circuit < CIRCUIT_COUNT {\n"
            "        let (gx, gy) = CIRCUIT_BLOCK_GRID[circuit];\n"
            "        let bx = if block_x < gx as usize { block_x } else { (gx as usize).saturating_sub(1) };\n"
            "        let by = if block_y < gy as usize { block_y } else { (gy as usize).saturating_sub(1) };\n"
            "        CIRCUIT_BLOCK_BASE[circuit] + by * (gx as usize) + bx\n"
            "    } else {\n"
            "        0\n"
            "    }\n"
            "}\n\n"
        )

        f.write(
            "/// Streaming compressed 1024x1024 JPEG block sector index [block_idx].\n"
            f"pub static CIRCUIT_BLOCK_SECTORS: [TileSectorEntry; COUNT] = [\n"
        )
        for c, (sec_off, sec_cnt, b_len) in enumerate(tile_entries):
            f.write(
                f"    TileSectorEntry {{ sector_offset: {sec_off}, "
                f"sector_count: {sec_cnt}, byte_len: {b_len} }},\n"
            )
        f.write("];\n\n")

        f.write(
            "/// Streaming segmented-QOI 1024x1024 block sector index [block_idx].\n"
            f"pub static CIRCUIT_QOI_SECTORS: [TileSectorEntry; COUNT] = [\n"
        )
        for c, (sec_off, sec_cnt, b_len) in enumerate(qoi_entries):
            f.write(
                f"    TileSectorEntry {{ sector_offset: {sec_off}, "
                f"sector_count: {sec_cnt}, byte_len: {b_len} }},\n"
            )
        f.write("];\n\n")

        f.write(
            "/// Streaming SVG-derived (`ARSV`) block sector index [block_idx].\n"
            f"pub static CIRCUIT_ARSV_SECTORS: [TileSectorEntry; COUNT] = [\n"
        )
        for c, (sec_off, sec_cnt, b_len) in enumerate(arsv_entries):
            f.write(
                f"    TileSectorEntry {{ sector_offset: {sec_off}, "
                f"sector_count: {sec_cnt}, byte_len: {b_len} }},\n"
            )
        f.write("];\n\n")

        f.write(
            "/// Uncompressed 15bpp 1024x1024 block sector index [block_idx].\n"
            f"pub static CIRCUIT_TIM_SECTORS: [TileSectorEntry; COUNT] = [\n"
        )
        for c, (sec_off, sec_cnt, b_len) in enumerate(tim_entries):
            f.write(
                f"    TileSectorEntry {{ sector_offset: {sec_off}, "
                f"sector_count: {sec_cnt}, byte_len: {b_len} }},\n"
            )
        f.write("];\n\n")

        f.write(
            "/// Uncompressed 4bpp block sector index [block_idx].\n"
            f"pub static CIRCUIT_TIM4_SECTORS: [TileSectorEntry; COUNT] = [\n"
        )
        for c, (sec_off, sec_cnt, b_len) in enumerate(tim4_entries):
            f.write(
                f"    TileSectorEntry {{ sector_offset: {sec_off}, "
                f"sector_count: {sec_cnt}, byte_len: {b_len} }},\n"
            )
        f.write("];\n\n")

        f.write(
            "/// Uncompressed 8bpp block sector index [block_idx].\n"
            f"pub static CIRCUIT_TIM8_SECTORS: [TileSectorEntry; COUNT] = [\n"
        )
        for c, (sec_off, sec_cnt, b_len) in enumerate(tim8_entries):
            f.write(
                f"    TileSectorEntry {{ sector_offset: {sec_off}, "
                f"sector_count: {sec_cnt}, byte_len: {b_len} }},\n"
            )
        f.write("];\n\n")

        for city_prefix, var_name, city_title, base_idx in [
            ("capetown", "CAPETOWN_BLOCK_SECTORS", "Cape Town", 28),
            ("melbourne", "MELBOURNE_BLOCK_SECTORS", "Melbourne", 37),
            ("london", "LONDON_BLOCK_SECTORS", "London", 46),
        ]:
            f.write(f"/// {city_title} 10 km^2 3x3 block sector mapping [block_y * 3 + block_x].\n")
            f.write(f"pub static {var_name}: [TileSectorEntry; 9] = [\n")
            for i in range(9):
                f.write(f"    CIRCUIT_BLOCK_SECTORS[{base_idx + i}],\n")
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
        f"{out_rs} ({circuit_count} circuits -> {n} x 1024x1024 blocks)"
    )





def main() -> int:
    import generate_svg_circuit as gen

    circuits = discover_circuits_detailed()

    results = []
    failures = []
    for c in circuits:
        data, errors = build_one(c.name, stem_path=c.stem_path)
        if errors:
            failures.append((c.name, errors))
            print(f"  {c.name:16} FAILED", file=sys.stderr)
            for e in errors:
                print(f"     {e}", file=sys.stderr)
            continue
        grid, centre, cps = data
        results.append((c, grid, centre, cps))
        print(f"  {c.name:16} [{c.mode}/{c.cup}] {grid.shape[1]}x{grid.shape[0]} cells, "
              f"centreline {len(centre)}, road {int(c.road_half_cells * 2)} cells wide")

    if failures:
        print(f"\n{len(failures)} circuit(s) failed validation; not emitting",
              file=sys.stderr)
        return 1

    if not results:
        print("\nno authored circuits; not emitting", file=sys.stderr)
        return 1

    idents = ["TRACK_" + "".join(ch if ch.isalnum() else "_" for ch in c.name).upper() for c, _, _, _ in results]
    code = [
        "//! Racetrack definitions for Arduracer PSX.",
        "//!",
        "//! Generated by `tools/track_cook/build_atlas.py` from the authored",
        "//! colour-coded images in `tracks/`. Do not edit by hand.",
        "",
        "use crate::math::{Fixed, Vec2};",
        "use crate::timing::{CheckpointGate, ParTimes};",
        "use crate::track::TrackDef;",
        "",
        "const EMPTY_GATE: CheckpointGate = CheckpointGate { x: 0, y: 0, width: 0, height: 0 };",
        "",
    ]
    for (c, grid, centre, cps), ident in zip(results, idents):
        par = calibrated_par(c.name)
        checkpoints_arg = cps if cps is not None else CHECKPOINTS
        code.append(cc.emit_rust(
            c.name.replace(" ", "_"), grid, centre, ident=ident,
            checkpoints=checkpoints_arg, par=par, route_nodes=ROUTE_NODES,
            half_width_cells=c.road_half_cells))
        code.append("")

    assert int(gen.ROAD_HALF_WORLD / cc.TILE_SIZE * cc.TILE_SIZE) == gen.ROAD_HALF_WORLD, (
        "ROAD_HALF_WORLD is not a whole number of cells wide at the current "
        "TILE_SIZE; the compiled road would be narrower than authored"
    )
    for c, _grid, _centre, _cps in results:
        assert abs(c.road_half_cells * cc.TILE_SIZE - round(c.road_half_cells * cc.TILE_SIZE)) < 1e-9, (
            f"{c.name} was drawn {2 * c.road_half_cells:g} cells wide, which is not a "
            f"whole number of {cc.TILE_SIZE}-world-unit cells; TrackDef.half_width "
            f"could not state it and the compiled road would be narrower than drawn"
        )
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
    code.append(f"pub const TOTAL_TRACKS: usize = {len(results)};")
    code.append("")
    code.append("/// Game mode classification.")
    code.append("#[derive(Copy, Clone, Debug, PartialEq, Eq)]")
    code.append("#[repr(u8)]")
    code.append("pub enum GameMode {")
    code.append("    GrandPrix = 0,")
    code.append("    Mania = 1,")
    code.append("    FreeRoam = 2,")
    code.append("}")
    code.append("")
    code.append("/// Cup specification grouping a sequence of circuit indices.")
    code.append("#[derive(Clone, Debug, PartialEq, Eq)]")
    code.append("pub struct CupDef {")
    code.append("    pub name: &'static str,")
    code.append("    pub mode: GameMode,")
    code.append("    pub track_indices: &'static [usize],")
    code.append("}")
    code.append("")
    code.append("impl CupDef {")
    code.append("    pub fn track_at(&self, stage: usize) -> Option<&'static TrackDef> {")
    code.append("        self.track_indices.get(stage).map(|&idx| ALL_TRACKS[idx])")
    code.append("    }")
    code.append("    pub fn total_stages(&self) -> usize {")
    code.append("        self.track_indices.len()")
    code.append("    }")
    code.append("}")
    code.append("")

    gp_cup_map = {}
    mania_cup_map = {}
    for idx, (c, _, _, _) in enumerate(results):
        if c.mode == "mania":
            mania_cup_map.setdefault(c.cup_name, []).append(idx)
        else:
            gp_cup_map.setdefault(c.cup_name, []).append(idx)

    code.append(f"pub static GRAND_PRIX_CUPS: [CupDef; {len(gp_cup_map)}] = [")
    for cname, indices in gp_cup_map.items():
        code.append("    CupDef {")
        code.append(f'        name: "{cname}",')
        code.append("        mode: GameMode::GrandPrix,")
        code.append(f"        track_indices: &[{', '.join(str(i) for i in indices)}],")
        code.append("    },")
    code.append("];")
    code.append("")

    code.append(f"pub static MANIA_CUPS: [CupDef; {len(mania_cup_map)}] = [")
    for cname, indices in mania_cup_map.items():
        code.append("    CupDef {")
        code.append(f'        name: "{cname}",')
        code.append("        mode: GameMode::Mania,")
        code.append(f"        track_indices: &[{', '.join(str(i) for i in indices)}],")
        code.append("    },")
    code.append("];")
    code.append("")

    total_cups = len(gp_cup_map) + len(mania_cup_map)
    code.append(f"pub static ALL_CUPS: [CupDef; {total_cups}] = [")
    for cname, indices in gp_cup_map.items():
        code.append("    CupDef {")
        code.append(f'        name: "{cname}",')
        code.append("        mode: GameMode::GrandPrix,")
        code.append(f"        track_indices: &[{', '.join(str(i) for i in indices)}],")
        code.append("    },")
    for cname, indices in mania_cup_map.items():
        code.append("    CupDef {")
        code.append(f'        name: "{cname}",')
        code.append("        mode: GameMode::Mania,")
        code.append(f"        track_indices: &[{', '.join(str(i) for i in indices)}],")
        code.append("    },")
    code.append("];")
    code.append("")

    code.append(f"pub const ALL_TRACKS: [&TrackDef; {len(results)}] = [")
    for ident in idents:
        code.append(f"    &{ident},")
    code.append("];")
    code.append("")
    code.append(f"pub const AUTHORED_TRACKS: [&TrackDef; {len(results)}] = ALL_TRACKS;")
    code.append("")
    code.append(f"pub const ALL_TRACK_VISUALS: [usize; {len(results)}] = [")
    for i in range(len(results)):
        code.append(f"    {i},")
    code.append("];")
    code.append("")
    code.append(f"pub const LEVEL_DIMS: [(u8, u8); {len(results) + 3}] = [")
    for c, grid, _, _ in results:
        code.append(f"    ({grid.shape[1]}, {grid.shape[0]}),")
    code.append("    (240, 240),")
    code.append("    (240, 240),")
    code.append("    (240, 240),")
    code.append("];")
    code.append("")

    out = os.path.join(ROOT, "crates", "arduracer-core", "src", "levels.rs")
    with open(out, "w") as f:
        f.write("\n".join(code))
    import concurrent.futures
    track_names = [c.name for c, _, _, _ in results] + ["Cape Town", "Melbourne", "London"]
    with concurrent.futures.ProcessPoolExecutor() as executor:
        visual_packs = list(executor.map(pack_visual, track_names))
    emit_visual_tex(visual_packs)
    fmt_generated(out)
    print(f"\nwrote {out} ({len(results)} authored circuits across {total_cups} cups)")
    try:
        import make_levels_bin
        make_levels_bin.main()
    except Exception as e:
        print(f"  warning: failed to update LEVELS.BIN ({e})", file=sys.stderr)

    uncalibrated = []
    for c, _, _, _ in results:
        par = calibrated_par(c.name)
        if par == FALLBACK_PAR:
            uncalibrated.append(c.name)
        else:
            print(f"  {c.name:16} par measured (bronze/silver/gold/dev "
                  f"{par[0]}/{par[1]}/{par[2]}/{par[3]})")
    if uncalibrated:
        print(f"  WARNING: no calibration for {', '.join(uncalibrated)}; par "
              f"times are placeholders {FALLBACK_PAR}. Run:")
        print("    cargo run --manifest-path tools/playtest/Cargo.toml "
              "--release -- --calibrate")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())