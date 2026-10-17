#!/usr/bin/env python3
"""Assemble `assets/LEVELS.BIN` from the per-level grids the cooker produces.

The level tile grids used to be `static [TrackTile; N]` arrays compiled into the
executable: 326,400 bytes of them, resident in RAM for every level while exactly
one was ever in use. They now live in this asset and one buffer holds whichever
level is loaded.

Sources, in the order `LEVELS.BIN` lays them out:

* 24 circuits, from `build/circuits/<Name>/levelgrid.bin`. Written by
  `compile_circuit.py`, one byte per cell, row-major, already in `TrackTile`
  discriminants -- so this script copies bytes and interprets nothing.
* 3 free-roam cities, from `tracks/<city>_10km/<city>_10km_collision.bin`.
  Those are committed, because the city modules still embed them for their
  own dimensions.

Order matters and is not negotiable: the runtime slices the asset by a prefix
sum over `game::gpu::tracktex::LEVEL_DIMS`, which is 24 circuits then 3 cities.
`crates/arduracer-core/tests/level_grid_asset.rs` asserts the resulting layout
against a golden hash captured from the committed sources.

    python3 tools/track_cook/make_levels_bin.py
"""

import os
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

#: Circuit directory names, in `ALL_TRACKS` order.
#:
#: This order is load-bearing and is not the alphabetical one. The runtime
#: slices `LEVELS.BIN` by a prefix sum over `LEVEL_DIMS`, indexed by circuit
#: slot, so an entry out of place here does not fail -- it quietly loads some
#: other circuit's terrain under this one's name. The order below is taken from
#: the `ALL_TRACKS` emit in `levels.rs`, the only definition of "slot N is this
#: circuit".
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import build_atlas

circuits_detailed = build_atlas.discover_circuits_detailed()
CIRCUITS = [c.stem for c in circuits_detailed]
CIRCUIT_GRID = {c.stem: c.grid for c in circuits_detailed}

CITIES = ["capetown", "melbourne", "london"]

def circuit_cells(name):
    g = CIRCUIT_GRID.get(name, 160 if name == "Titan_Circuit" else 80)
    return g * g

CITY_CELLS = 240 * 240


def circuit_path(name):
    return os.path.join(ROOT, "build", "circuits", name, "levelgrid.bin")


def city_path(city):
    p1 = os.path.join(ROOT, "tracks", f"{city}_10km", f"{city}_10km_collision.bin")
    if os.path.exists(p1):
        return p1
    p2 = os.path.join(ROOT, "tracks", "cities", f"{city}_10km", f"{city}_10km_collision.bin")
    if os.path.exists(p2):
        return p2
    return os.path.join(ROOT, "build", "cities", city, "levelgrid.bin")


def main():
    missing = [n for n in CIRCUITS if not os.path.exists(circuit_path(n))]
    if missing:
        print(
            f"ERROR: {len(missing)} circuit grids missing, e.g. {missing[:3]}.\n"
            "       Run 'make atlas' to cook them from tracks/*.data.png first.",
            file=sys.stderr,
        )
        return 1

    parts = []
    for name in CIRCUITS:
        raw = open(circuit_path(name), "rb").read()
        expected = circuit_cells(name)
        if len(raw) != expected:
            print(
                f"ERROR: {name} grid is {len(raw)} bytes, expected {expected}",
                file=sys.stderr,
            )
            return 1
        parts.append(raw)
    for city in CITIES:
        path = city_path(city)
        if not os.path.exists(path):
            print(
                f"ERROR: {city} collision grid missing at {path}",
                file=sys.stderr,
            )
            return 1
        raw = open(path, "rb").read()
        if len(raw) != CITY_CELLS:
            print(
                f"ERROR: {city} grid is {len(raw)} bytes, expected {CITY_CELLS}",
                file=sys.stderr,
            )
            return 1
        parts.append(raw)

    out = os.path.join(ROOT, "assets", "LEVELS.BIN")
    os.makedirs(os.path.dirname(out), exist_ok=True)
    blob = b"".join(parts)
    with open(out, "wb") as f:
        f.write(blob)
    print(
        f"wrote assets/LEVELS.BIN: {len(blob):,} bytes "
        f"({len(CIRCUITS)} circuits + {len(CITIES)} cities)"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())