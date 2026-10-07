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


def pack_visual(name: str) -> bytes:
    """Packs one circuit's visual PNG into 4bpp nibble order, plus its palette.

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
    rgb = np.asarray(
        Image.open(stem + ".visual.png").convert("RGB"), dtype=np.int32
    )
    h, w = rgb.shape[:2]
    assert (w, h) == VISUAL_SIZE, (
        f"{name}: visual is {w}x{h}, expected {VISUAL_SIZE[0]}x{VISUAL_SIZE[1]}. "
        f"`gpu::tracktex` hardcodes the page arithmetic for that size; if the "
        f"grid changed, change it in the same commit."
    )

    pal_arr = np.asarray(CLUT_RGB, dtype=np.int32)
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
    return bytes(packed)


def emit_visual_tex(packed_per_circuit: list[bytes]) -> None:
    """Writes `crates/arduracer-core/src/visual_tex.rs` from the packed visuals.

    **One texture per circuit**, indexed by `AUTHORED_TRACK_COUNT` order, and the
    renderer uploads the one the active track wants. This is the change that makes
    a multi-circuit atlas affordable, and it is why the visual is 320x320 rather
    than 768x768: a 4bpp world texture is `WIDTH * HEIGHT / 2` bytes and lives in
    `.rodata` for the whole life of the program whether or not it is on screen, so
    four of them at 768 square would be 1.15 MB against a static-RAM ceiling of
    under a megabyte. Four at 320 square is 205 KB.

    A single shared texture -- which is what the one-circuit pipeline emitted --
    cannot be kept, because each circuit has a different *image*: two circuits
    sharing a texture would show each other's road.
    """
    assert packed_per_circuit, "no circuits to pack a visual for"
    palette = CLUT_RGB
    out = os.path.join(ROOT, "crates", "arduracer-core", "src", "visual_tex.rs")
    n = len(packed_per_circuit)
    with open(out, "w") as f:
        f.write(
            "//! Per-circuit visual texture data: the baked appearance of each\n"
            "//! circuit, packed the way a 4bpp texture lives in VRAM.\n"
            "//!\n"
            "//! Generated by `tools/track_cook/build_atlas.py`. Do not edit by hand.\n"
            "\n"
            "/// Visual image width in texels.\n"
            f"pub const WIDTH: usize = {VISUAL_SIZE[0]};\n"
            "/// Visual image height in texels.\n"
            f"pub const HEIGHT: usize = {VISUAL_SIZE[1]};\n"
            "/// Number of authored circuits, one texture each.\n"
            f"pub const COUNT: usize = {n};\n"
            "\n"
            "/// Packed 4-bit palette indices, one image per circuit in\n"
            "/// `CIRCUITS` order: two texels per byte, the texel at (x, y) is\n"
            "/// code `PACKED[i][y * (WIDTH / 2) + x / 2]`, low nibble when `x`\n"
            "/// is even, high nibble when odd. This is the exact byte stream a\n"
            "/// 4bpp TIM upload consumes, so `gpu::tracktex` can `upload_bytes`\n"
            "/// one of them with no conversion on device.\n"
            "///\n"
            "/// Nearest-palette quantisation: the painted visual carries small\n"
            "/// shading deltas around the sixteen level-palette colours, and\n"
            "/// snapping to the nearest entry by squared RGB distance keeps\n"
            "/// kerb blocks and racing lines sharp rather than smeared.\n"
            f"pub static PACKED: [[u8; WIDTH * HEIGHT / 2]; COUNT] = [\n"
        )
        for packed in packed_per_circuit:
            f.write("    [\n")
            for i in range(0, len(packed), 32):
                chunk = ", ".join(f"0x{b:02X}" for b in packed[i : i + 32])
                f.write(f"        {chunk},\n")
            f.write("    ],\n")
        f.write("];\n\n")
        f.write(
            "/// The level palette in code order 0..=15, as 8-bit RGB triples.\n"
            "///\n"
            "/// Shared by every circuit: the palette is part of the level format\n"
            "/// (`tools/track_cook/palette.json`), not of any one drawing.\n"
            "pub static CLUT_RGB: [(u8, u8, u8); 16] = [\n"
        )
        palette = CLUT_RGB
        for r, g, b in palette:
            f.write(f"    ({r}, {g}, {b}),\n")
        f.write("];\n")
    print(f"wrote {out} ({len(packed_per_circuit)} textures, "
          f"{sum(len(p) for p in packed_per_circuit)} packed bytes)")


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
             if f.endswith((".data.png", ".visual.png", ".line.json",
                            ".palette.json"))
             and not any(f.startswith(k) for k in keep)]
    if stale:
        print(f"\nremoving {len(stale)} superseded image file(s) for the "
              f"circuits that no longer exist", file=sys.stderr)
        git(["rm", "-q", "--"] + [f"tracks/{f}" for f in stale])

    idents = ["TRACK_" + n.replace(" ", "_").upper() for n, _, _ in results]
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