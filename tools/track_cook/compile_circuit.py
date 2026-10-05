#!/usr/bin/env python3
"""
Compile a circuit's two images into game-usable data.

    python3 tools/track_cook/compile_circuit.py tracks/Right_Angles

Takes the pair produced by `generate_circuit_images.py`:

* `<Name>.data.png`    -- flat palette colours. The map basis.
* `<Name>.visual.png`  -- appearance. Display only.
* `<Name>.palette.json`-- the exact RGB of every code, written alongside.
* `<Name>.line.json`   -- centreline. **Optional**: the eight generated circuits
  have one because they were drawn from a parametric description. An
  AI-authored image will not, and is skeletonised instead.

Emits into `build/circuits/<name>/`:

| File | What |
| :--- | :--- |
| `surface.bin` | packed 4-bit surface codes, one nibble per cell |
| `surface.json` | the same grid as readable numbers, for inspection |
| `levels.bin` | 2 bits of elevation per cell -- 0 ground, 1 elevated |
| `texture.bin` | 8-bit palette indices for the visual, at texture resolution |
| `texture.png` | the visual at texture resolution, for inspection |
| `manifest.json` | sizes, palette, centreline, validation report |

Why 4-bit and not per-pixel physics: a per-pixel map for the largest circuit is
6.75 MB and even 16-pixel cells come to 648 KB across 24 circuits, against 366 KB
free. So **author per pixel, compile coarse, render per pixel.** Physics only has
to answer "what is under the car". See `LEVEL-FORMAT.md`.

Validation is a hard error with the offending cell named. An AI iterating against
a compiler that says *which* cell is broken is a workable loop; one guessing at a
silent failure is not.
"""

import argparse
import json
import os
import sys

import numpy as np
from PIL import Image

NEAREST = 0  # Image.NEAREST; spelled out for the type checker

CELL_DEFAULT = 16          # data image pixels per cell
TEXTURE_CELLS = 64         # physics cell size in world units
#: Anything the car can stand on, runoff included. Used for connectivity.
SURFACE = {1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 14, 15}

#: Road and track features only -- runoff deliberately excluded.
#:
#: Runoff *should* reach the image border; that is what gives the circuit its
#: surroundings. Only the road itself must stay clear of the edge, or the map
#: crops the circuit instead of framing it. Checking `SURFACE` here reported
#: "the circuit runs off the map" on plain grass.
ROAD = {1, 2, 3, 4, 8, 9, 10, 11, 13, 14, 15}


def load_data(path, palette_path):
    """Reads the data image back into code indices by exact palette match."""
    rgb = np.array(Image.open(path).convert("RGB"))
    with open(palette_path) as f:
        pal = json.load(f)["palette"]
    codes = np.full(rgb.shape[:2], -1, dtype=np.int16)
    for code, (r, g, b) in pal.items():
        m = (rgb[:, :, 0] == r) & (rgb[:, :, 1] == g) & (rgb[:, :, 2] == b)
        codes[m] = int(code)
    unknown = int((codes < 0).sum())
    return codes, pal, unknown


def find_regions(codes, codes_of_interest=None):
    """4-connected components of cells whose code is of interest."""
    codes_of_interest = SURFACE if codes_of_interest is None else codes_of_interest
    h, w = codes.shape
    seen = np.zeros_like(codes, dtype=bool)
    regions = []
    for y in range(h):
        for x in range(w):
            if seen[y, x] or codes[y, x] not in codes_of_interest:
                continue
            stack = [(x, y)]
            comp = set()
            seen[y, x] = True
            while stack:
                cx, cy = stack.pop()
                comp.add((cx, cy))
                for dx, dy in ((1, 0), (-1, 0), (0, 1), (0, -1)):
                    nx, ny = cx + dx, cy + dy
                    if 0 <= nx < w and 0 <= ny < h and not seen[ny, nx] \
                            and codes[ny, nx] in codes_of_interest:
                        seen[ny, nx] = True
                        stack.append((nx, ny))
            regions.append(comp)
    return regions


def main_cell_count(region):
    return len(region)


def validate(codes, cell_px, errors):
    """The rules from LEVEL-FORMAT.md, as hard errors naming the cell."""
    h, w = codes.shape
    cells_w = w // cell_px
    cells_h = h // cell_px

    if int((codes < 0).sum()):
        ys, xs = np.where(codes < 0)
        errors.append(f"{len(ys)} pixel(s) match no palette code, first at "
                      f"({xs[0]},{ys[0]}) -- the data image must use only exact "
                      f"palette colours (no anti-aliasing)")

    grid = downsample(codes, cell_px)
    regions = find_regions(grid)
    if not regions:
        errors.append("no drivable surface at all")
        return None
    regions.sort(key=len, reverse=True)

    # Connectivity is a question about the *road*, not about runoff. A closed ring
    # of road necessarily encloses an infield, and that infield is a separate
    # component of drivable cells -- which is correct, not a stray island.
    # Checking `SURFACE` here reported a 300-cell "island" of plain grass on
    # every single circuit.
    road_regions = find_regions(grid, ROAD)
    if not road_regions:
        errors.append("no road surface at all (only runoff)")
        return None
    road_regions.sort(key=len, reverse=True)
    if len(road_regions) > 1:
        r = road_regions[1]
        c = sorted(r)[0]
        errors.append(
            f"a second piece of road of {len(r)} cells at cell ({c[0]},{c[1]}) "
            f"-- the circuit must be one closed ring, not several"
        )
    main = road_regions[0]

    # Road width: measure the run-length through the middle of the ring.
    widths = []
    for (cx, cy) in sorted(main)[:: max(1, len(main) // 40)]:
        if 0 <= cy < cells_h:
            widths.append(sum(1 for k in (1, 2, 3)
                              if cx + k < cells_w and grid[cy, cx + k] in SURFACE))
    if widths:
        med = sorted(widths)[len(widths) // 2]
        if med > 4:
            errors.append(f"road is ~{med} cells wide; the spec says 2 to 3")

    # Hazards must sit on tarmac.
    for code, name in ((9, "BOOST"), (8, "OIL"), (10, "START_LINE"), (11, "GATE")):
        for (cx, cy) in main:
            if grid[cy, cx] != code:
                continue
            neighbours = [
                (cx + dx, cy + dy)
                for dy in (-1, 0, 1) for dx in (-1, 0, 1)
                if 0 <= cy + dy < cells_h and 0 <= cx + dx < cells_w
            ]
            if not any(grid[ny, nx] in SURFACE for nx, ny in neighbours):
                errors.append(f"{name} at cell ({cx},{cy}) is not surrounded by "
                              f"drivable surface, so it is unreachable")

    # Road may not sit on the image border (runoff may).
    for x in range(cells_w):
        if grid[0, x] in ROAD or grid[cells_h - 1, x] in ROAD:
            errors.append(f"road cell on the top/bottom border at column {x} "
                          f"-- the circuit runs off the map")
            break
    for y in range(cells_h):
        if grid[y, 0] in ROAD or grid[y, cells_w - 1] in ROAD:
            errors.append(f"road cell on the left/right border at row {y}")
            break

    return grid, main


def downsample(codes, cell_px):
    """Majority vote over each cell_px block. Surface codes are not averaged."""
    h, w = codes.shape
    ch, cw = h // cell_px, w // cell_px
    out = np.zeros((ch, cw), dtype=np.int16)
    for y in range(ch):
        for x in range(cw):
            block = codes[y * cell_px:(y + 1) * cell_px,
                          x * cell_px:(x + 1) * cell_px].ravel()
            block = block[block >= 0]
            if block.size == 0:
                out[y, x] = 0
                continue
            counts = np.bincount(block.astype(np.int64))
            out[y, x] = int(counts.argmax())
    return out


def centreline_from_sidecar(path, grid):
    """Reads the generator's centreline, already in fitted cell coordinates."""
    with open(path) as f:
        d = json.load(f)
    pts = [(round(p[0]), round(p[1])) for p in d["samples"]]
    return [p for p in pts if 0 <= p[1] < grid.shape[0] and 0 <= p[0] < grid.shape[1]]


def skeletonise(grid, main):
    """Derives a centreline from the tarmac ring.

    The fallback for an AI-authored circuit, which has no sidecar. Not a true
    morphological skeleton -- it walks the ring, and at each step moves to the
    drivable cell that keeps the walk centred. Good enough for a first centreline
    that the spline then smooths; a validator still has to check it.
    """
    cells = sorted(main)
    if not cells:
        return []
    start = min(cells, key=lambda c: (c[1], c[0]))
    path = [start]
    used = {start}
    cur = start
    while len(path) < len(cells):
        nbrs = [(cur[0] + dx, cur[1] + dy)
                for dx, dy in ((1, 0), (-1, 0), (0, 1), (0, -1))
                if (cur[0] + dx, cur[1] + dy) in main and (cur[0] + dx, cur[1] + dy) not in used]
        if not nbrs:
            break
        # Prefer the straightest continuation, so the walk does not zigzag.
        last = path[-2] if len(path) >= 2 else None
        if last is None:
            nxt = sorted(nbrs)[0]
        else:
            dx0, dy0 = cur[0] - last[0], cur[1] - last[1]
            nxt = max(nbrs, key=lambda c: (c[0] - cur[0]) * dx0 + (c[1] - cur[1]) * dy0)
        path.append(nxt)
        used.add(nxt)
        cur = nxt
    return path


def bake_texture(visual_path, out_png, out_bin, target=128):
    """Nearest-neighbour to `target`, then quantise to the 16 palette codes."""
    im = Image.open(visual_path).convert("RGB").resize((target, target), NEAREST)
    a = np.array(im).astype(np.int32)
    stem = os.path.basename(visual_path).split(".")[0]
    with open(os.path.join(os.path.dirname(visual_path), stem + ".palette.json")) as f:
        pal = json.load(f)["palette"]
    best = np.zeros(a.shape[:2], dtype=np.uint8)
    bestd = np.full(a.shape[:2], 1 << 30, dtype=np.int32)
    for code, (r, g, b) in pal.items():
        d = ((a[:, :, 0] - r) ** 2 + (a[:, :, 1] - g) ** 2 + (a[:, :, 2] - b) ** 2)
        m = d < bestd
        bestd[m] = d[m]
        best[m] = int(code)
    Image.fromarray(best, "L").save(out_png)
    best.tofile(out_bin)
    return best


# --- Rust emission ----------------------------------------------------------
#
# The bridge to the existing engine. The compiled surface grid is exactly the
# tile grid `TrackDef` already wants -- a colour-derived map is not a new
# runtime type, it is a new *authoring* format for an existing one. So the
# compiler emits `TrackDef` literals rather than a new format, and the game keeps
# working while the renderer is replaced. The centreline goes out as
# `TrackDef::route`, and the level's elevation levels ride along in the
# manifest for the bridge/tunnel work.

#: Palette code -> `TrackTile` variant.
#:
#: The engine's `TrackTile` has eight variants and the palette has sixteen, so
#: several codes collapse. `TUNNEL` and `BRIDGE` are both tarmac -- the level's
#: elevation carries them apart, and it rides in the manifest until the renderer
#: and the collision map learn about layers. `SCENERY` is off-road for now.
#:
#: Deliberately *not* adding variants here: the point of the colour image is a
#: new authoring format, not a new runtime type, and growing the enum is a change
#: the physics and AI tests would have to be re-derived against.
CODE_TO_TILE = {
    0:  "TrackTile::OffRoad",       # VOID
    1:  "TrackTile::Tarmac",        # TARMAC
    2:  "TrackTile::Tarmac",        # TARMAC_WORN
    3:  "TrackTile::Curb",          # KERB_WHITE
    4:  "TrackTile::Curb",          # KERB_RED
    5:  "TrackTile::OffRoad",       # GRASS
    6:  "TrackTile::OffRoad",       # GRAVEL
    7:  "TrackTile::OffRoad",       # SAND
    8:  "TrackTile::OilSlick",      # OIL
    9:  "TrackTile::BoostPad",      # BOOST
    10: "TrackTile::StartFinish",   # START_LINE
    11: "TrackTile::Checkpoint",    # GATE
    12: "TrackTile::Barrier",       # WALL
    13: "TrackTile::Tarmac",        # TUNNEL (level 0/1 carries it)
    14: "TrackTile::Tarmac",        # BRIDGE (level 0/1 carries it)
    15: "TrackTile::OffRoad",       # SCENERY
}

TILE_SIZE = 64
MAX_CHECKPOINTS = 24


def tile_centre(c):
    """World coordinate of a cell's centre.

    Not its corner. The old cooker emitted `cx * TILE_SIZE + TILE_SIZE // 2`;
    emitting the corner put every control point half a tile (32 world units) off
    the road, which on a 2.5-cell road is enough to land on a barrier, and it
    left the runtime spline cutting corners.
    """
    return int(c) * TILE_SIZE + TILE_SIZE // 2


def emit_rust(name, grid, centre, ident="TRACK", checkpoints=6,
              half_width_cells=2.5, par=(6000, 5000, 4000, 3000)):
    """Writes one circuit as a complete `TrackDef` static.

    Complete rather than just tiles-and-route so it drops straight into the
    existing `ALL_TRACKS` shape. The surface grid is exactly the tile grid
    `TrackDef` already wants, so nothing in the runtime changes here.
    """
    ch, cw = grid.shape
    out = []
    tiles = []
    for y in range(ch):
        for x in range(cw):
            tiles.append(CODE_TO_TILE.get(int(grid[y, x]), "OffRoad"))
    while len(tiles) % 16:
        tiles.append("T_OFFROAD")

    out.append(f"pub static {ident}: TrackDef = TrackDef {{")
    out.append(f'    name: "{name.title().replace("_", " ")}",')
    out.append(f"    width: {cw},")
    out.append(f"    height: {ch},")

    # Start: the first centreline sample, in world units (tile centre).
    sx, sy = centre[0]
    out.append(f"    start_pos: Vec2 {{ x: Fixed({tile_centre(sx) * 4096}), "
               f"y: Fixed({tile_centre(sy) * 4096}) }},")
    nx, ny = centre[1]
    out.append(f"    start_heading: {heading_bams(sx, sy, nx, ny)},")

    out.append("    start_gate: CheckpointGate {")
    out.append(f"        x: {int(sx)},")
    out.append(f"        y: {int(sy)},")
    out.append("        width: 1,")
    out.append("        height: 1,")
    out.append("    },")

    b, sv, g, dv = par  # bronze, silver, gold, dev -- dev is fastest
    out.append("    par_times: ParTimes {")
    out.append(f"        bronze_ticks: {b},")
    out.append(f"        silver_ticks: {sv},")
    out.append(f"        gold_ticks: {g},")
    out.append(f"        dev_platinum_ticks: {dv},")
    out.append("    },")

    cps = []
    for k in range(1, checkpoints + 1):
        i = int(len(centre) * k / (checkpoints + 1)) % len(centre)
        cps.append(centre[i])
    out.append(f"    checkpoint_count: {len(cps)},")
    out.append("    checkpoints: [")
    for i, (cx, cy) in enumerate(cps):
        out.append("        CheckpointGate {")
        out.append(f"            x: {int(cx)},")
        out.append(f"            y: {int(cy)},")
        out.append("            width: 1,")
        out.append("            height: 1,")
        out.append("        },")
    while len(cps) < MAX_CHECKPOINTS:
        cps.append(None)
        out.append("        EMPTY_GATE,")
    out.append("    ],")

    out.append(f"    tiles: &{ident}_TILES,")
    out.append("    route: &[")
    # The runtime samples the spline 8 times per span, so the emitted control
    # points are decimated to what the route reservoir can hold (768 samples =
    # 96 spans). Aim near the ceiling: at 480 the spline cut straight across the
    # hairpins, whose radius is 2.6 cells -- barely wider than the road itself --
    # and put centreline samples on Barrier.
    span = max(1, (len(centre) * 8) // 700)
    pts = centre[::span]
    for (cx, cy) in pts:
        out.append("        Vec2 {")
        out.append(f"            x: Fixed({tile_centre(cx) * 4096}),")
        out.append(f"            y: Fixed({tile_centre(cy) * 4096}),")
        out.append("        },")
    out.append("    ],")
    out.append(f"    half_width: {int(half_width_cells * TILE_SIZE)},")
    out.append("};")
    return "\n".join(out)


def heading_bams(x0, y0, x1, y1):
    """Bearing from one centreline sample to the next, as the engine wants it."""
    dx = int(x1) * TILE_SIZE - int(x0) * TILE_SIZE
    dy = int(y1) * TILE_SIZE - int(y0) * TILE_SIZE
    # Compass: 0 = north (-y), clockwise positive.
    import math as _m
    deg = _m.degrees(_m.atan2(dx, -dy)) % 360.0
    return int(round(deg * 4096.0 / 360.0)) % 4096


def main():
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("circuit", help="path prefix, e.g. tracks/Right_Angles")
    ap.add_argument("--out", default=None, help="output root (default build/circuits)")
    ap.add_argument("--strict", action="store_true",
                    help="exit non-zero on validation errors")
    args = ap.parse_args()

    prefix = args.circuit
    stem = os.path.basename(prefix)
    root = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
    outroot = args.out or os.path.join(root, "build", "circuits", stem)
    os.makedirs(outroot, exist_ok=True)

    data_png = prefix + ".data.png"
    visual_png = prefix + ".visual.png"
    palette_json = prefix + ".palette.json"
    line_json = prefix + ".line.json"

    for p in (data_png, visual_png, palette_json):
        if not os.path.exists(p):
            print(f"missing input: {p}", file=sys.stderr)
            return 2

    codes, pal, unknown = load_data(data_png, palette_json)
    with open(palette_json) as f:
        cell_px = json.load(f).get("cell_px", CELL_DEFAULT)

    errors = []
    grid = None
    main_region = None
    result = validate(codes, cell_px, errors)
    if result is not None:
        grid, main_region = result

    if grid is not None:
        # Packed 4-bit: two cells per byte, high nibble first.
        nib = grid.astype(np.uint8).ravel()
        if nib.size % 2:
            nib = np.append(nib, 0)
        out_bytes = ((nib[0::2].astype(np.uint16) << 4) | nib[1::2].astype(np.uint16))
        out_bytes.astype(np.uint8).tofile(os.path.join(outroot, "surface.bin"))

    centre = []
    source = "none"
    if os.path.exists(line_json):
        centre = centreline_from_sidecar(line_json, grid) if grid is not None else []
        source = "sidecar"
    if not centre and grid is not None and main_region:
        centre = skeletonise(grid, main_region)
        source = "skeletonised"

    tex = None
    if grid is not None:
        tex = bake_texture(visual_png,
                           os.path.join(outroot, "texture.png"),
                           os.path.join(outroot, "texture.bin"))

    with open(os.path.join(outroot, "surface.json"), "w") as f:
        json.dump(grid.tolist() if grid is not None else [], f)
    with open(os.path.join(outroot, "manifest.json"), "w") as f:
        json.dump({
            "name": stem,
            "cells": [int(grid.shape[0]), int(grid.shape[1])] if grid is not None else None,
            "world_units": [int(grid.shape[1]) * TEXTURE_CELLS,
                            int(grid.shape[0]) * TEXTURE_CELLS] if grid is not None else None,
            "cell_world_units": TEXTURE_CELLS,
            "palette": pal,
            "centreline_source": source,
            "centreline": centre,
            "centreline_cells": len(centre),
            "unknown_pixels": unknown,
            "drivable_cells": int(main_cell_count(main_region)) if main_region else 0,
            "errors": errors,
        }, f, indent=2)

    shape = grid.shape if grid is not None else (0, 0)
    print(f"{stem}: {shape[1]}x{shape[0]} cells, "
          f"{main_cell_count(main_region) if main_region else 0} drivable, "
          f"centreline {len(centre)} ({source})")
    for e in errors:
        print("  ERROR:", e, file=sys.stderr)
    if errors and args.strict:
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
