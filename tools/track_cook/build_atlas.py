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

One authored circuit, replicated to every slot
----------------------------------------------

`ALL_TRACKS` is `[&TrackDef; 24]` and the game's cup, championship and
track-select code all index it positionally, so it cannot shrink to one entry
without touching every one of those. Rather than fake 24 distinct circuits, this
emits the *same* `TrackDef` 24 times. That is honest about what exists: one
geometry, reachable from every slot, with the AI left switched off in the game
until there is a second circuit for it to race on.

Replicating the `TrackDef` rather than the tile array matters: the tiles are a
single static referenced 24 times, so RAM cost stays at one circuit's worth
(~4.6 KB) instead of 24 copies (~110 KB) against a 999 KB budget.
"""
import numpy as np
from PIL import Image

import json
import os
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
        if entry.get("track") != name:
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


def git(args: list[str]) -> None:
    """Runs git, ignoring failure.

    Used only to drop superseded generated images, and only when this script runs
    from a git checkout. A tarball build has no git and still produces correct
    output; it just keeps the stale files on disk.
    """
    import subprocess

    try:
        subprocess.run(["git"] + args, cwd=ROOT, check=False,
                       capture_output=True)
    except OSError:
        pass


def emit_visual_tex(name: str) -> None:
    """Writes `crates/arduracer-core/src/visual_tex.rs` from the visual PNG.

    The game's per-pixel track renderer samples a VRAM-resident 4-bit
    direct-palette texture of the whole world, so the cooker has to ship the
    visual in PAL/TIM nibble order rather than as RGB bytes: two texels per
    byte, the left one in the low nibble, row stride `WIDTH/2` bytes. That is
    exactly the byte layout the GPU unwraps when a 4bpp primitive fetches a
    texel, so a plain `upload_bytes` of this data at the right VRAM rect is a
    valid texture -- no conversion on device.

    The visual PNG is painted with small shading variations around the sixteen
    level-palette colours (103 distinct RGBs against a 16-entry palette), so
    every pixel is snapped to its *nearest* palette entry by squared RGB
    distance before packing. Averaging or blurring would have smeared the
    kerb blocks and the racing line; nearest-match is the only mapping that
    keeps the painted shapes sharp against a 16-entry CLUT.
    """
    stem = os.path.join(ROOT, "tracks", name.replace(" ", "_"))
    with open(stem + ".palette.json") as f:
        pal = json.load(f)["palette"]
    palette = [tuple(int(v) for v in pal[str(i)]) for i in range(16)]
    rgb = np.asarray(
        Image.open(stem + ".visual.png").convert("RGB"), dtype=np.int32
    )
    h, w = rgb.shape[:2]
    assert w == 768 and h == 768, f"visual is {w}x{h}, expected 768x768"

    pal_arr = np.asarray(palette, dtype=np.int32)
    # (h, w, 16) squared distances, then argmin over the palette axis.
    dist = (
        (rgb[:, :, None, 0] - pal_arr[None, None, :, 0]) ** 2
        + (rgb[:, :, None, 1] - pal_arr[None, None, :, 1]) ** 2
        + (rgb[:, :, None, 2] - pal_arr[None, None, :, 2]) ** 2
    )
    codes = dist.argmin(axis=2).astype(np.uint8)

    packed = bytearray(w * h // 2)
    for y in range(h):
        row = codes[y]
        packed[y * (w // 2) : (y + 1) * (w // 2)] = bytes(
            row[2 * i] | (int(row[2 * i + 1]) << 4) for i in range(w // 2)
        )

    out = os.path.join(ROOT, "crates", "arduracer-core", "src", "visual_tex.rs")
    with open(out, "w") as f:
        f.write(
            "//! Per-circuit visual texture data: the baked appearance of the\n"
            "//! active circuit, packed the way a 4bpp texture lives in VRAM.\n"
            "//!\n"
            "//! Generated by `tools/track_cook/build_atlas.py`. Do not edit by hand.\n"
            "\n"
            "/// Visual image width in texels.\n"
            "pub const WIDTH: usize = 768;\n"
            "/// Visual image height in texels.\n"
            "pub const HEIGHT: usize = 768;\n"
            "\n"
            "/// Packed 4-bit palette indices, two texels per byte: the texel at\n"
            "/// (x, y) is code `PACKED[y * (WIDTH / 2) + x / 2]`, low nibble\n"
            "/// when `x` is even, high nibble when odd. This is the exact byte\n"
            "/// stream a 4bpp TIM upload consumes, so `gpu::tracktex` can\n"
            "/// `upload_bytes` it with no conversion on device.\n"
            "///\n"
            "/// Nearest-palette quantisation: the painted visual carries small\n"
            "/// shading deltas around the sixteen level-palette colours, and\n"
            "/// snapping to the nearest entry by squared RGB distance keeps\n"
            "/// kerb blocks and racing lines sharp rather than smeared.\n"
            "pub static PACKED: [u8; WIDTH * HEIGHT / 2] = [\n"
        )
        for i in range(0, len(packed), 32):
            chunk = ", ".join(f"0x{b:02X}" for b in packed[i : i + 32])
            f.write(f"    {chunk},\n")
        f.write("];\n\n")
        f.write(
            "/// The level palette in code order 0..=15, as 8-bit RGB triples.\n"
            "pub static CLUT_RGB: [(u8, u8, u8); 16] = [\n"
        )
        for r, g, b in palette:
            f.write(f"    ({r}, {g}, {b}),\n")
        f.write("];\n")
    print(f"wrote {out} ({len(packed)} packed bytes)")


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

    if len(results) != 1:
        print(f"\nexpecting exactly 1 authored circuit for now, got "
              f"{len(results)}", file=sys.stderr)
        return 1

    # The 23 superseded image sets from the parametric pipeline are still tracked
    # in `tracks/`. Nothing reads them -- `build_atlas` iterates
    # `generate_svg_circuit.CIRCUITS` -- so they are dead weight that looks like
    # the source of truth. Removed here, where the irrelevance is provable, rather
    # than by hand.
    stale = [f for f in sorted(os.listdir(os.path.join(ROOT, "tracks")))
             if f.endswith((".data.png", ".visual.png", ".line.json",
                            ".palette.json"))
             and not f.startswith(results[0][0].replace(" ", "_"))]
    if stale:
        print(f"\nremoving {len(stale)} superseded image file(s) for the "
              f"circuits that no longer exist", file=sys.stderr)
        git(["rm", "-q", "--"] + [f"tracks/{f}" for f in stale])

    idents = ["TRACK_" + n.replace(" ", "_").upper() for n, _, _ in results]
    name, grid, centre = results[0]
    ident = idents[0]
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
    code.append(f"static {ident}_TILES: [TrackTile; {grid.shape[1] * grid.shape[0]}] = [")
    flat = []
    for y in range(grid.shape[0]):
        for x in range(grid.shape[1]):
            flat.append(cc.CODE_TO_TILE.get(int(grid[y, x]), "OffRoad"))
    while len(flat) % 16:
        flat.append("OffRoad")
    for i in range(0, len(flat), 16):
        code.append("    " + ", ".join(flat[i:i + 16]) + ",")
    code.append("];")
    # Route control points. Capped by the runtime: `MAX_ROUTE_SAMPLES / 8` spans,
    # where 8 is `DEFAULT_SAMPLES_PER_SPAN`. The compiler derives the actual count
    # from the circuit's perimeter, since control points closer than a cell
    # collapse onto a shared tile centre -- see `compile_circuit.control_points`.
    par = calibrated_par(name)
    # The road half-width is specified in world units by the generator and
    # converted here, rather than left as a cell count. The cell size has already
    # changed once (64 -> 32 world units); a cell count baked into this call would
    # have halved the width of every road in the game the next time it did, with
    # nothing failing -- `half_width` is just a number in a struct literal.
    code.append(cc.emit_rust(name.replace(" ", "_"), grid, centre,
                             ident=ident, checkpoints=CHECKPOINTS,
                             par=par, route_nodes=ROUTE_NODES,
                             half_width_cells=gen.ROAD_HALF_WORLD / cc.TILE_SIZE))
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

    # One geometry, every slot. See the module docstring: `ALL_TRACKS` is
    # positionally indexed by the game's cup and track-select code, so it has to
    # stay 24 long until those are reworked. The tiles are a single static, so
    # this costs one copy of the geometry and 24 pointers.
    code.append("")
    code.append(f"/// Authored circuits: {len(results)}.")
    code.append(f"pub const AUTHORED_TRACK_COUNT: usize = {len(results)};")
    code.append("")
    code.append("/// The authored circuits, deduplicated.")
    code.append("///")
    code.append("/// `ALL_TRACKS` is a fixed 24 slots and currently holds one geometry")
    code.append("/// repeated, so anything that wants *the circuits* rather than *the slots*")
    code.append("/// wants this. Without it a caller iterating `ALL_TRACKS` to validate")
    code.append("/// circuits walks the same oval 24 times and reports 24 passes where")
    code.append("/// there is one thing that could have passed.")
    code.append(f"pub const AUTHORED_TRACKS: [&TrackDef; AUTHORED_TRACK_COUNT] = "
                f"[{', '.join('&' + n for n in idents)}];")
    code.append("")
    code.append(f"pub const ALL_TRACKS: [&TrackDef; {TRACK_SLOTS}] = [")
    # One entry per line. `cargo fmt` reflows a repeated item that would fit on
    # shared lines, so a compact emit is not stable under `fmt-check` -- the
    # generator and the formatter fight and CI fails on generated output.
    for _ in range(TRACK_SLOTS):
        code.append(f"    &{ident},")
    code.append("];")
    code.append("")

    out = os.path.join(ROOT, "crates", "arduracer-core", "src", "levels.rs")
    with open(out, "w") as f:
        f.write("\n".join(code))
    emit_visual_tex(name)
    print(f"\nwrote {out} ({len(results)} authored circuit, {TRACK_SLOTS} slots)")
    if par == FALLBACK_PAR:
        print(f"  WARNING: {name} has no calibration; par times are placeholders "
              f"{FALLBACK_PAR}. Run:")
        print("    cargo run --manifest-path tools/playtest/Cargo.toml "
              "--release -- --calibrate")
    else:
        print(f"  par times measured (bronze/silver/gold/dev "
              f"{par[0]}/{par[1]}/{par[2]}/{par[3]})")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())