#!/usr/bin/env python3
"""
Compile every circuit image pair into `crates/arduracer-core/src/levels.rs`.

    python3 tools/track_cook/build_atlas.py
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
"""
import json
import os
import subprocess
import sys
import importlib.util

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
sys.path.insert(0, HERE)

import generate_circuit_images as gen  # noqa: E402

spec = importlib.util.spec_from_file_location("cc", os.path.join(HERE, "compile_circuit.py"))
cc = importlib.util.module_from_spec(spec)
spec.loader.exec_module(cc)

TILE_SIZE = 64

#: Par times used when a circuit has no usable measured entry. The order is
#: `(bronze, silver, gold, dev)`, matching `emit_rust`.
DEFAULT_PAR = (6000, 5000, 4000, 3000)

#: Below this, a measured par is a failed measurement rather than a lap time.
#:
#: `timing::MIN_PLAUSIBLE_LAP_TICKS` is 30, and a par at or under half the
#: fastest reference lap becomes the floor that rejects real laps. So anything
#: this small cannot be a lap time on a circuit thousands of units long.
MIN_PLAUSIBLE_PAR = 100


def load_par_table():
    """Measured par times by circuit name, from the playtest calibration.

    `tools/playtest --calibrate` writes this file from real laps driven through
    `arduracer-core`. It has to be read *here* as well as by the retired
    `convert_levels.py`: the atlas pipeline is now the only thing that writes
    `levels.rs`, so a par table nothing reads is a par table that never reaches
    the game. Measured before this was wired up: `make playtest` reported all 24
    circuits as "par table is stale (dev 3000->1)", because every emitted circuit
    was carrying the placeholder above.
    """
    path = os.path.join(ROOT, "tools", "track_cook", "par_calibration.json")
    if not os.path.exists(path):
        print(f"  no {path}; emitting placeholder par times", file=sys.stderr)
        return {}
    with open(path) as f:
        doc = json.load(f)
    out = {}
    for entry in doc.get("tracks", []):
        name = entry.get("track")
        if not name:
            continue
        par = (
            int(entry["bronze"]),
            int(entry["silver"]),
            int(entry["gold"]),
            int(entry["dev"]),
        )
        # A measured par of 1 tick means the calibration could not complete a lap
        # on this circuit, not that a lap takes one tick. Emitting it would give
        # the circuit a par table that no lap can beat and that fails
        # `every_shipped_track_admits_its_measured_reference_lap`. Fall back to
        # the placeholder and say which circuit, so the measurement gap is
        # visible instead of silently becoming a medal target.
        if min(par) <= MIN_PLAUSIBLE_PAR:
            print(
                f"  {name}: calibration recorded {par}, which is not a lap; "
                f"using placeholder par times",
                file=sys.stderr,
            )
            continue
        out[name] = par
    return out


def build_one(circuit):
    """Generate both images for `circuit` into a scratch dir, then compile."""
    outdir = os.path.join(ROOT, "tracks")
    info = gen.build(circuit, outdir)
    codes, pal, unknown = cc.load_data(info["data"], info["data"].replace(
        ".data.png", ".palette.json"))
    cell_px = json.load(open(info["data"].replace(
        ".data.png", ".palette.json")))["cell_px"]

    errors = []
    grid = None
    res = cc.validate(codes, cell_px, errors)
    if res is not None:
        grid, _ = res
    if errors:
        return None, errors, info
    if grid is None:
        return None, ["no grid"], info

    with open(info["line"]) as f:
        d = json.load(f)
    # Sub-cell precision, deliberately. The walker emits fractional cell
    # coordinates, and rounding each sample to an integer cell *before*
    # resampling quantises the line onto the cell grid: on a tight corner that
    # alone moves the centreline off the road, and the resampled spline then
    # inherits the error. `compile_circuit.route_control_points` snaps the
    # resampled points back onto the racing surface, which needs the fractional
    # positions to snap from.
    centre = [(p[0], p[1]) for p in d["samples"]]
    return (grid, centre), [], info


def main():
    circuits = gen.CIRCUITS
    par_table = load_par_table()

    def par_for(name):
        return par_table.get(name, DEFAULT_PAR)

    results = []
    failures = []
    for c in circuits:
        data, errors, info = build_one(c)
        if errors:
            failures.append((c["name"], errors))
            print(f"  {c['name']:16} FAILED", file=sys.stderr)
            for e in errors:
                print(f"     {e}", file=sys.stderr)
            continue
        grid, centre = data
        results.append((c["name"], grid, centre, info))
        print(f"  {c['name']:16} {grid.shape[1]}x{grid.shape[0]} cells, "
              f"centreline {len(centre)}")

    if failures:
        print(f"\n{len(failures)} circuit(s) failed validation; not emitting",
              file=sys.stderr)
        return 1

    code = ["//! Racetrack definitions for Arduracer PSX.",
            "//!",
            "//! Generated by `tools/track_cook/build_atlas.py` from the colour-coded",
            "//! images in `tracks/`. Do not edit by hand.",
            "",
            "use crate::math::{Fixed, Vec2};",
            "use crate::timing::{CheckpointGate, ParTimes};",
            "use crate::track::{TrackDef, TrackTile};",
            "",
            "const EMPTY_GATE: CheckpointGate = CheckpointGate { x: 0, y: 0, width: 0, height: 0 };",
            ""]
    names = []
    for name, grid, centre, info in results:
        ident = "TRACK_" + name.replace(" ", "_").upper()
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
        code.append(cc.emit_rust(name.replace(" ", "_"), grid, centre,
                                 ident=ident, checkpoints=6,
                                 par=par_for(name)))
        names.append(ident)
    code.append("")
    code.append(f"pub const ALL_TRACKS: [&TrackDef; {len(names)}] = [")
    for i in range(0, len(names), 4):
        code.append("    " + ", ".join(f"&{n}" for n in names[i:i + 4]) + ",")
    code.append("];")
    code.append("")
    out = os.path.join(ROOT, "crates", "arduracer-core", "src", "levels.rs")
    with open(out, "w") as f:
        f.write("\n".join(code))
    print(f"\nwrote {out} ({len(results)} circuits)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
