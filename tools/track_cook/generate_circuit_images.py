#!/usr/bin/env python3
"""
Generate Arduracer PSX circuit images.

A circuit is a **colour-coded image** (see `LEVEL-FORMAT.md`). This script draws
the initial set: it walks a parametric description of each circuit, rasterises the
colour-coded master image, and bakes a texture for on-screen display.

Two outputs per circuit:

* `tracks/<name>.png`     -- the master colour-coded map. This is the map basis
  and the file an AI will later fill with track features.
* `tracks/<name>_tex.png` -- the baked gameplay texture, nearest-neighbour
  reduced to a PSX-friendly size and quantised to the 16-code palette.

Intersections
-------------

A self-crossing circuit is the interesting case, and it is why the format has
elevation. Each segment is drawn on a **level**: level 0 is the ground plane,
level 1 is elevated. Where level 1 crosses level 0, the elevated section is
stamped `BRIDGE` and the section passing beneath is stamped `TUNNEL`. They occupy
the same two-dimensional footprint, so a flat collision map would fuse them --
the elevation level is what keeps them apart, and it is carried per cell into the
compiled map.

Road width
----------

The drivable ring is **2 to 3 cells wide** here (`ROAD_CELLS`), per the format
spec. An earlier attempt used 4-6 cells and read as a car park.
"""

import argparse
import json
import math
import os

import numpy as np
from PIL import Image

NEAREST = 0  # Image.NEAREST, spelled out so the linter can see it

# --- Palette, mirroring tools/track_cook/palette.json exactly ----------------

VOID = 0
TARMAC = 1
TARMAC_WORN = 2
KERB_WHITE = 3
KERB_RED = 4
GRASS = 5
GRAVEL = 6
SAND = 7
OIL = 8
BOOST = 9
START_LINE = 10
GATE = 11
WALL = 12
TUNNEL = 13
BRIDGE = 14
SCENERY = 15

RGB = {
    VOID:        (0x00, 0x00, 0x00),
    TARMAC:      (0x3C, 0x3E, 0x44),
    TARMAC_WORN: (0x4A, 0x4C, 0x54),
    KERB_WHITE:  (0xE8, 0xE8, 0xF0),
    KERB_RED:    (0xD8, 0x28, 0x3C),
    GRASS:       (0x2E, 0x5A, 0x34),
    GRAVEL:      (0x6B, 0x54, 0x32),
    SAND:        (0x7A, 0x60, 0x90),
    OIL:         (0x8A, 0x1F, 0xB0),
    BOOST:       (0xFF, 0x8A, 0x10),
    START_LINE:  (0xF2, 0xF2, 0xF2),
    GATE:        (0x00, 0xD2, 0xFF),
    WALL:        (0x2A, 0x2E, 0x38),
    TUNNEL:      (0x5A, 0x46, 0x32),
    BRIDGE:      (0x3A, 0x5A, 0x7A),
    SCENERY:     (0xFF, 0x2E, 0x88),
}

# --- Geometry ----------------------------------------------------------------

CELL = 16                      # master image pixels per map cell
ROAD_CELLS = 2.5               # drivable width in cells -- 2 to 3, per spec
KERB_CELLS = 0.7               # rumble band outside the tarmac
CURB_KNEE = 0.010              # |curvature| above which a corner gets a kerb
STEP = 0.05                    # walker step, in cells

SIZE = 512                     # master image edge, pixels
GRID = SIZE // CELL            # cells per edge (32)


def walk(steps, start_xy, heading_deg):
    """Follows `steps` in unbounded float coordinates.

    Returns per-sample `(x, y, level, curvature)`. Deliberately does *not*
    rasterise: the first attempt picked coordinates against the 32-cell grid by
    hand and every circuit ended up as a small shape marooned in a corner, or as
    a flat smear. Authoring in unbounded coordinates and auto-fitting at the end
    (see `fit_scale`) means the shape is defined by its turns and nothing else.
    """
    x, y = float(start_xy[0]), float(start_xy[1])
    hdg = math.radians(heading_deg)
    level = 0
    samples = []

    for kind, a, b in steps:
        if kind == "straight":
            n = max(1, int(round(abs(a) / STEP)))
            d = a / n
            for _ in range(n):
                x += math.sin(hdg) * d
                y -= math.cos(hdg) * d
                samples.append((x, y, level, 0.0))
        elif kind == "corner":
            turn_deg, radius = a, b
            n = max(2, int(round(abs(turn_deg) / 2.0)))
            dtheta = math.radians(turn_deg) / n
            for _ in range(n):
                hdg += dtheta
                x += math.sin(hdg) * STEP
                y -= math.cos(hdg) * STEP
                samples.append((x, y, level, dtheta / STEP))
        elif kind == "level":
            level = int(a)
        else:
            raise ValueError(f"unknown step {kind!r}")

    return samples


def fit_scale(samples, road_cells, margin_cells=2.0):
    """Scale and offset that maps `samples` into the grid with a margin.

    Returns `(scale, dx, dy)` in cell units.
    """
    xs = [s[0] for s in samples]
    ys = [s[1] for s in samples]
    pad = road_cells + KERB_CELLS + margin_cells
    span_x = (max(xs) - min(xs)) + 2 * pad
    span_y = (max(ys) - min(ys)) + 2 * pad
    scale = min((GRID - 1) / span_x, (GRID - 1) / span_y)
    cx = (max(xs) + min(xs)) / 2.0
    cy = (max(ys) + min(ys)) / 2.0
    dx = (GRID - 1) / 2.0 - cx * scale
    dy = (GRID - 1) / 2.0 - cy * scale
    return scale, dx, dy


def render(samples, road_cells):
    """Rasterises fitted samples into per-level code maps."""
    scale, dx, dy = fit_scale(samples, road_cells)
    grids = [np.zeros((GRID, GRID), dtype=np.int8) for _ in range(2)]
    marks = [dict() for _ in range(2)]
    road = road_cells

    r = int(math.ceil(road + KERB_CELLS)) + 1
    for (fx, fy, lvl, curv) in samples:
        x = fx * scale + dx
        y = fy * scale + dy
        li = int(lvl)
        buf, g = marks[li], grids[li]
        kerby = abs(curv) > CURB_KNEE
        for yy in range(int(y) - r, int(y) + r + 1):
            for xx in range(int(x) - r, int(x) + r + 1):
                if not (0 <= xx < GRID and 0 <= yy < GRID):
                    continue
                d = math.hypot(xx + 0.5 - x, yy + 0.5 - y)
                if d <= road:
                    if g[yy, xx] <= li:
                        g[yy, xx] = li
                    buf[(xx, yy)] = TARMAC
                elif kerby and d <= road + KERB_CELLS and (xx, yy) not in buf:
                    if g[yy, xx] <= li:
                        g[yy, xx] = li
                    buf[(xx, yy)] = -1  # kerb, resolved to an alternating code below
    return grids, marks, scale, dx, dy


def compose(grids, marks, runoff):
    """Flattens the level buffers into one colour-coded image.

    Kerbs are resolved here rather than during stamping, because alternation is
    a property of the *sequence* along the track and cannot be decided cell by
    cell.

    Where level 1 crosses level 0 the elevated cells become `BRIDGE` and the
    cells beneath become `TUNNEL`. That is the whole point of carrying
    elevation: the two sections share a footprint, so a flat map would fuse
    them into one unrecognisable blob.
    """
    img = np.full((GRID, GRID), runoff, dtype=np.uint8)
    roadish = (TARMAC, BOOST, OIL, GATE, START_LINE)

    painted = {}
    for li, buf in enumerate(marks):
        painted[li] = {k: v for k, v in buf.items() if v >= 0}

    # Kerb runs, per level, in scan order so alternation follows the scan. Good
    # enough to read as a rumble stripe, which is all a colour-coded map needs.
    for li, buf in enumerate(marks):
        keys = sorted(k for k, v in buf.items() if v == -1)
        for i, k in enumerate(keys):
            buf[k] = KERB_RED if i % 2 else KERB_WHITE
        painted[li].update({k: v for k, v in buf.items() if v >= 0})

    for li in (0, 1):
        for (x, y), v in painted[li].items():
            # Level 1 draws over level 0; the crossing cells are relabelled after.
            if li == 0 or img[y, x] not in roadish:
                img[y, x] = v

    ground_road = {(x, y) for (x, y), v in painted[0].items() if v in roadish}
    upper_road = {(x, y) for (x, y), v in painted[1].items() if v in roadish}
    overlap = ground_road & upper_road
    for (x, y) in overlap:
        img[y, x] = BRIDGE
    # A crossing cannot be shown twice in one 2D cell -- the elevated section and
    # the section beneath it occupy the *same footprint*. Only one of them can be
    # the cell's colour, so the under-pass is marked as the ring of ground road
    # either side of the crossing. That is a rendering compromise, not the data
    # model: the compiled map carries a level per cell, and the level is what
    # keeps the two sections apart for physics.
    for (x, y) in ground_road:
        if (x, y) in overlap:
            continue
        if any((x + dx, y + dy) in overlap
               for dx, dy in ((1, 0), (-1, 0), (0, 1), (0, -1))):
            img[y, x] = TUNNEL
    return img, overlap


def add_walls(img):
    """Walls the outer edge of the drivable region; everything beyond is runoff.

    Flooded inward from the image border, so interior pockets (an infield, the
    space inside a hairpin) stay runoff rather than becoming wall.
    """
    drivable = np.isin(img, [TARMAC, TARMAC_WORN, KERB_WHITE, KERB_RED,
                             BOOST, OIL, GATE, START_LINE, TUNNEL, BRIDGE,
                             SCENERY])
    h, w = drivable.shape
    seen = np.zeros_like(drivable)
    stack = []
    for x in range(w):
        stack += [(x, 0), (x, h - 1)]
    for y in range(h):
        stack += [(0, y), (w - 1, y)]
    while stack:
        x, y = stack.pop()
        if not (0 <= x < w and 0 <= y < h) or seen[y, x] or drivable[y, x]:
            continue
        seen[y, x] = True
        stack += [(x + 1, y), (x - 1, y), (x, y + 1), (x, y - 1)]
    out = img.copy()
    # Only cells immediately outside the circuit become wall; further out is
    # runoff, so the map reads as "road inside a barrier inside grass".
    ring = seen & ~np.isin(img, [TARMAC, BOOST, OIL, GATE, START_LINE,
                                 TUNNEL, BRIDGE, SCENERY])
    return out, seen


def wall_the_edge(img):
    """One cell of WALL just outside the drivable region."""
    drivable = np.isin(img, [TARMAC, TARMAC_WORN, KERB_WHITE, KERB_RED,
                             BOOST, OIL, GATE, START_LINE, TUNNEL, BRIDGE])
    out = img.copy()
    h, w = drivable.shape
    for y in range(h):
        for x in range(w):
            if drivable[y, x]:
                continue
            # Adjacent to road, and not itself a road cell's neighbour-of-neighbour.
            for dy, dx in ((1, 0), (-1, 0), (0, 1), (0, -1)):
                ny, nx = y + dy, x + dx
                if 0 <= ny < h and 0 <= nx < w and drivable[ny, nx]:
                    out[y, x] = WALL
                    break
    return out


def stamp_at_frac(samples, frac):
    """Index of the centreline sample at `frac` of the lap."""
    return int(len(samples) * frac) % len(samples)


def apply_features(img, samples, start_idx, boost_fracs=(), oil_fracs=(),
                   gate_fracs=()):
    """Paints BOOST / OIL / GATE / START_LINE onto the road.

    `samples` are fitted cell coordinates.
    """
    def paint(idx, code, radius=1.2):
        x, y, _lvl = samples[idx][0], samples[idx][1], samples[idx][2]
        r = int(math.ceil(radius))
        for yy in range(int(y) - r, int(y) + r + 1):
            for xx in range(int(x) - r, int(x) + r + 1):
                if 0 <= yy < GRID and 0 <= xx < GRID:
                    if math.hypot(xx + 0.5 - x, yy + 0.5 - y) <= radius:
                        img[yy, xx] = code

    paint(start_idx, START_LINE, 1.4)
    for f in gate_fracs:
        paint(stamp_at_frac(samples, f), GATE, 1.2)
    for f in boost_fracs:
        paint(stamp_at_frac(samples, f), BOOST, 1.1)
    for f in oil_fracs:
        paint(stamp_at_frac(samples, f), OIL, 1.1)
    return img


def to_rgb(img):
    """Expands the code map to RGB."""
    rgb = np.zeros((img.shape[0], img.shape[1], 3), dtype=np.uint8)
    for code, (r, g, b) in RGB.items():
        rgb[img == code] = (r, g, b)
    return rgb


def to_png(img, path):
    rgb = to_rgb(img)
    Image.fromarray(rgb, "RGB").save(path)
    return path


def bake_texture(img, path, size=128):
    """Nearest-neighbour reduction to a PSX-friendly texture, palette-quantised."""
    im = Image.fromarray(to_rgb(img), "RGB")
    # Rebuild from the index map so the reduction is exact, not a colour average.
    small = im.resize((size, size), NEAREST)
    a = np.array(small)
    # Vectorised nearest-palette assignment over the 16 known colours.
    flat = a.reshape(-1, 3).astype(np.int32)
    best = np.zeros(len(flat), dtype=np.uint8)
    bestd = np.full(len(flat), 1 << 30, dtype=np.int32)
    for code, (r, g, b) in RGB.items():
        d = ((flat[:, 0] - r) ** 2 + (flat[:, 1] - g) ** 2 + (flat[:, 2] - b) ** 2)
        m = d < bestd
        bestd[m] = d[m]
        best[m] = code
    Image.fromarray(best.reshape(size, size), "L").save(path)
    return path


# --- The initial circuits ----------------------------------------------------
#
# Deliberately varied, because the first pass came out as 24 ovals and that was
# the complaint that started this. Between them these cover: hairpins, 90-degree
# corners, esses, chicanes, long straights, and two circuits where the track
# crosses over itself on an elevated level.

def S(tiles):
    return ("straight", tiles, 0)


def C(degrees, radius):
    return ("corner", degrees, radius)


def L(level):
    return ("level", level, 0)


#: Runoff theme per circuit, so the map does not read as 24 copies.
THEMES = {"grass": GRASS, "gravel": GRAVEL, "sand": SAND}

CIRCUITS = [
    {
        "name": "Hairpin Ridge",
        "start": (6, 6), "heading": 90,
        "steps": [S(16), C(-170, 2.6), S(10), C(170, 2.6), S(12),
                  C(-170, 2.6), S(10), C(170, 2.6), S(14), C(-150, 3.2)],
        "runoff": GRASS,
        "boost": (0.35,), "oil": (0.62,), "gates": (0.15, 0.45, 0.8),
        "note": "Four hairpins. The sharpest thing a 2.5-cell road can ask for.",
    },
    {
        "name": "Right Angles",
        "start": (5, 5), "heading": 90,
        "steps": [S(9), C(90, 2.4), S(7), C(90, 2.4), S(9), C(-90, 2.4),
                  S(6), C(-90, 2.4), S(8), C(60, 3.0), C(-120, 3.0)],
        "runoff": GRAVEL,
        "boost": (0.2, 0.7), "oil": (), "gates": (0.2, 0.5, 0.8),
        "note": "Right angles only. A street circuit with kerbs on every corner.",
    },
    {
        "name": "The Esses",
        "start": (5, 22), "heading": 90,
        "steps": [S(7), C(70, 3.4), S(6), C(-70, 3.4), S(7), C(70, 3.4),
                  S(6), C(-70, 3.4), S(9), C(50, 4.5), S(10), C(-50, 4.5)],
        "runoff": GRASS,
        "boost": (0.4,), "oil": (0.75,), "gates": (0.25, 0.6, 0.9),
        "note": "Alternating esses, then two sweepers.",
    },
    {
        "name": "Chicane Park",
        "start": (4, 16), "heading": 0,
        "steps": [S(10), C(110, 1.9), C(-110, 1.9), C(110, 1.9), C(-110, 1.9),
                  S(12), C(120, 2.6), S(8), C(-120, 2.6)],
        "runoff": GRAVEL,
        "boost": (), "oil": (0.3, 0.7), "gates": (0.2, 0.55, 0.85),
        "note": "Four chicanes at the tightest radius that still drives.",
    },
    {
        # --- the intersection cases ---
        "name": "Overpass",
        "start": (5, 20), "heading": 0,
        "steps": [S(9), C(90, 3.0), S(11), L(1), S(12), C(-90, 3.0), S(9),
                  L(0), C(-90, 3.0), S(11), C(90, 3.0)],
        "runoff": GRASS,
        "boost": (0.5,), "oil": (), "gates": (0.3, 0.7),
        "note": "The outgoing straight crosses the incoming one. The crossing "
                "section is elevated: BRIDGE over TUNNEL.",
    },
    {
        "name": "Crossover",
        "start": (4, 4), "heading": 90,
        "steps": [S(8), C(80, 2.8), S(9), C(-70, 2.8), S(10), L(1), S(9),
                  C(-80, 3.0), L(0), S(8), C(70, 3.0)],
        "runoff": SAND,
        "boost": (0.28, 0.74), "oil": (0.5,), "gates": (0.15, 0.45, 0.85),
        "note": "Two sweeping corners and one crossing, elevated.",
    },
    {
        "name": "Longbow",
        "start": (6, 16), "heading": 90,
        "steps": [S(14), C(60, 5.5), S(12), C(-70, 6.0), S(13), C(65, 5.5),
                  S(11), C(-55, 5.0)],
        "runoff": GRASS,
        "boost": (0.2, 0.66), "oil": (), "gates": (0.3, 0.7),
        "note": "Fast sweepers and long straights. The contrast circuit.",
    },
    {
        "name": "Switchback",
        "start": (5, 26), "heading": 0,
        "steps": [S(8), C(-155, 2.4), S(9), C(155, 2.4), S(8), C(-155, 2.4),
                  S(9), C(155, 2.4), S(7), C(-120, 3.2), S(6), C(120, 3.2)],
        "runoff": GRAVEL,
        "boost": (0.45,), "oil": (0.8,), "gates": (0.2, 0.55, 0.85),
        "note": "Alternating hairpins climbing then descending.",
    },
]


def build(circuit, outdir):
    samples = walk(circuit["steps"], circuit["start"], circuit["heading"])
    if len(samples) < 50:
        raise RuntimeError(f"{circuit['name']}: only {len(samples)} samples")
    grids, marks, scale, dx, dy = render(samples, ROAD_CELLS)
    img, overlap = compose(grids, marks, circuit["runoff"])
    # Features are stamped in fitted cell coordinates, so they land on the road
    # rather than at the pre-fit position.
    fitted = [(x * scale + dx, y * scale + dy, lvl) for (x, y, lvl, _c) in samples]
    img = apply_features(img, fitted, 0, circuit["boost"], circuit["oil"],
                         circuit["gates"])
    img = wall_the_edge(img)

    base = os.path.join(outdir, circuit["name"].replace(" ", "_"))
    to_png(img, base + ".png")
    bake_texture(img, base + "_tex.png")

    # Sidecar: the centreline, so the compiler need not skeletonise it. An
    # AI-authored circuit will not have this and will be skeletonised instead.
    with open(base + ".line.json", "w") as f:
        json.dump({
            "cell_px": CELL,
            "grid": GRID,
            "levels": 2,
            "road_cells": ROAD_CELLS,
            "scale": round(scale, 6), "dx": round(dx, 4), "dy": round(dy, 4),
            "samples": [[round(x, 2), round(y, 2), lvl] for x, y, lvl in fitted],
        }, f)

    counts = {RGB[i]: int((img == i).sum()) for i in RGB}
    return {
        "name": circuit["name"],
        "png": base + ".png",
        "tex": base + "_tex.png",
        "line": base + ".line.json",
        "samples": len(samples),
        "bridge": counts[RGB[BRIDGE]],
        "tunnel": counts[RGB[TUNNEL]],
        "kerb": counts[RGB[KERB_WHITE]] + counts[RGB[KERB_RED]],
        "note": circuit["note"],
    }


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--out", default=None,
                    help="output directory (default: <repo>/tracks)")
    ap.add_argument("--only", default=None, help="generate just this circuit")
    args = ap.parse_args()
    root = os.path.dirname(os.path.dirname(os.path.dirname(
        os.path.abspath(__file__))))
    outdir = args.out or os.path.join(root, "tracks")
    os.makedirs(outdir, exist_ok=True)

    for c in CIRCUITS:
        if args.only and args.only not in c["name"]:
            continue
        info = build(c, outdir)
        print(f"{info['name']:16} {info['samples']:6} samples  "
              f"kerb {info['kerb']:5}  bridge {info['bridge']:4}  "
              f"tunnel {info['tunnel']:4}  -> {os.path.basename(info['png'])}")
    print(f"\nwrote to {outdir}")


if __name__ == "__main__":
    main()
