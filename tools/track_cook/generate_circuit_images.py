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

#: Pixels per map cell in the *data* image. Flat palette colours only; the
#: compiler reads this back out of `<name>.palette.json`.
DATA_PX = 4
#: Pixels per map cell in the *visual* image. Painted, so it wants more room.
VISUAL_PX = 16
#: Retained for the walker's step size and for callers that assume 16.
CELL = 16
ROAD_CELLS = 2.5               # drivable width in cells -- 2 to 3, per spec
KERB_CELLS = 0.7               # rumble band outside the tarmac
CURB_KNEE = 0.010              # |curvature| above which a corner gets a kerb
STEP = 0.05                    # walker step, in cells

SIZE = 768                     # master image edge, pixels (48 px/cell)
CELL = 16
GRID = SIZE // CELL            # cells per edge (48)
DATA_PX = 3                     # data image px/cell -> 144x144
VISUAL_PX = 16                  # visual image px/cell -> 768x768


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


def fit_scale(samples, road_cells):
    """Scale and offset that maps `samples` into the grid with room to spare.

    The scale is chosen so the *centreline* fits the **inner box**, not so that
    the padded bounding box fits the whole grid. Those are different, and getting
    it wrong is invisible until the validator complains: padding the span and
    then scaling span to the full width puts the centreline flush to the edge, and
    the pen radius -- tarmac, kerb and a wall ring -- spills off the map.

    `inner` reserves one cell of wall beyond the kerb on every side.
    """
    xs = [s[0] for s in samples]
    ys = [s[1] for s in samples]
    extent_x = max(1e-6, max(xs) - min(xs))
    extent_y = max(1e-6, max(ys) - min(ys))
    wall = 1.0
    inner = (GRID - 1) - 2.0 * (road_cells + KERB_CELLS + wall)
    if inner <= 2.0:
        raise ValueError("grid too small for the requested road width")
    scale = min(inner / extent_x, inner / extent_y)
    cx = (max(xs) + min(xs)) / 2.0
    cy = (max(ys) + min(ys)) / 2.0
    mid = (GRID - 1) / 2.0
    return scale, mid - cx * scale, mid - cy * scale


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


# --- The visual image -------------------------------------------------------
#
# The data image and the visual image have *different requirements* and one
# cannot be a reduction of the other:
#
#   data   -- exact flat palette values, no anti-aliasing, machine-parseable.
#             A single stray anti-aliased pixel makes a road cell unreadable.
#   visual -- appearance. Painted kerb blocks, worn racing line, textured
#             runoff, a chequered start line. Every one of those is a gradient
#             or a detail the data image must not contain.
#
# So the visual is *painted from* the data map, not resampled from it.

#: Deterministic hash noise, so a regenerated image is byte-identical.
def _noise(x, y, salt=0):
    n = (x * 374761393 + y * 668265263 + salt * 2246822519) & 0xFFFFFFFF
    n = (n ^ (n >> 13)) * 1274126177 & 0xFFFFFFFF
    return ((n ^ (n >> 16)) & 0xFF) / 255.0 - 0.5


def _lerp(c, d, t):
    return tuple(int(max(0, min(255, c[i] + (d[i] - c[i]) * t))) for i in range(3))


def paint_visual(codes, road_codes=(TARMAC, BOOST, OIL, GATE, START_LINE,
                                   TUNNEL, BRIDGE, KERB_WHITE, KERB_RED,
                                   TARMAC_WORN)):
    """Paints an appearance image from the code map.

    Everything here is presentation only. Physics reads the code map, never this.
    """
    h, w = codes.shape
    out = np.zeros((h, w, 3), dtype=np.uint8)

    for y in range(h):
        for x in range(w):
            c = codes[y, x]
            n = _noise(x, y)
            n2 = _noise(x // 4, y // 4, salt=7)

            if c in (TARMAC, TARMAC_WORN):
                # Tarmac with a coarse patchiness and a fine grain.
                base = RGB[TARMAC] if c == TARMAC else RGB[TARMAC_WORN]
                out[y, x] = _lerp(base, (0, 0, 0), 0.10 + n * 0.10 + n2 * 0.06)

            elif c in (KERB_WHITE, KERB_RED):
                # Paint kerbs as blocks along the direction of travel, not as the
                # 1-cell fringe the data map uses. The data map only has to say
                # "kerb here"; the visual says "red and white, in 2-cell blocks".
                blk = ((x + y) // 2) % 2
                base = RGB[KERB_WHITE] if blk == 0 else RGB[KERB_RED]
                out[y, x] = _lerp(base, (0, 0, 0), n * 0.12)

            elif c == GRASS:
                out[y, x] = _lerp(RGB[GRASS], (0, 0, 0), 0.14 + n * 0.22 + n2 * 0.10)
            elif c == GRAVEL:
                out[y, x] = _lerp(RGB[GRAVEL], (0, 0, 0), 0.16 + n * 0.26)
            elif c == SAND:
                out[y, x] = _lerp(RGB[SAND], (0, 0, 0), 0.12 + n * 0.20 + n2 * 0.08)

            elif c == WALL:
                # Barrier with a lit top edge, so the track boundary reads.
                edge = any(
                    0 <= y + dy < h and 0 <= x + dx < w
                    and codes[y + dy, x + dx] in road_codes
                    for dy, dx in ((-1, 0), (1, 0), (0, -1), (0, 1))
                )
                base = RGB[WALL]
                out[y, x] = _lerp(base, (255, 255, 255), 0.22 if edge else 0.0 + n * 0.08)

            elif c == START_LINE:
                # Chequered, 2x2 blocks.
                blk = ((x // 2) + (y // 2)) % 2
                out[y, x] = (245, 245, 245) if blk == 0 else (28, 28, 32)

            elif c == BOOST:
                # Chevron pointing along +x, on an orange field.
                band = (y + x // 2) % 6
                out[y, x] = (255, 240, 200) if band < 2 else _lerp(RGB[BOOST], (0, 0, 0), n * 0.10)

            elif c == OIL:
                # Dark, with an iridescent sheen that shifts across the patch.
                t = 0.5 + 0.5 * ((x * 0.3 + y * 0.2) % 6) / 6.0
                out[y, x] = _lerp((18, 14, 26), (60, 30, 80), t)

            elif c == GATE:
                out[y, x] = _lerp(RGB[GATE], (255, 255, 255), 0.15 + n * 0.15)

            elif c == TUNNEL:
                # Dark opening: the road is visible but in shadow.
                out[y, x] = _lerp((22, 20, 24), (0, 0, 0), 0.10 + n * 0.10)

            elif c == BRIDGE:
                # Concrete deck with a lighter kerb line either side.
                out[y, x] = _lerp((120, 138, 158), (0, 0, 0), 0.10 + n2 * 0.14 + n * 0.06)

            elif c == SCENERY:
                out[y, x] = _lerp(RGB[SCENERY], (0, 0, 0), 0.18 + n * 0.2)

            else:  # VOID
                out[y, x] = (0, 0, 0)
    return out


def write_visual(codes, path, data_path):
    """Writes the visual PNG and the exact palette the data PNG uses.

    The data image is rewritten in the same pass so the two are guaranteed to be
    the same size and aligned cell-for-cell, and the palette travels with them as
    a JSON sidecar -- the compiler needs it to read the data image.
    """
    # Visual: painted at VISUAL_PX px/cell so there is room for grain, kerb
    # blocks and markings. The code map is upscaled nearest-neighbour first, so
    # every painted feature still lands inside the right cell.
    vis = np.repeat(np.repeat(codes, VISUAL_PX, axis=0), VISUAL_PX, axis=1)
    Image.fromarray(paint_visual(vis), "RGB").save(path)

    # Data: exact palette colours at DATA_PX px/cell. Flat by requirement --
    # one stray anti-aliased pixel makes a cell unreadable.
    dat = np.repeat(np.repeat(codes, DATA_PX, axis=0), DATA_PX, axis=1)
    to_png(dat, data_path)

    stem = os.path.basename(data_path).split(".")[0]
    with open(os.path.join(os.path.dirname(data_path), stem + ".palette.json"), "w") as f:
        json.dump({"cell_px": DATA_PX, "grid": GRID,
                   "visual_px_per_cell": VISUAL_PX,
                   "palette": {str(k): list(v) for k, v in RGB.items()}}, f)


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


# --- The remaining sixteen ---------------------------------------------------
#
# Composed rather than hand-written, so each is still a distinct shape but the
# file stays readable. The helpers are deliberately blunt: the point is variety
# of *shape*, and a hand-tuned generator would drift back toward ovals.

def _hairpins(n, radius, straights, start, heading, runoff, kind):
    steps = []
    for i in range(n):
        steps.append(S(straights[0]))
        steps.append(C(180 if i % 2 == 0 else -180, radius))
        steps.append(S(straights[1]))
        if i < n - 1:
            steps.append(C(-70 if i % 2 == 0 else 70, straights[2]))
    return {"name": kind[0], "start": start, "heading": heading,
            "steps": steps, "runoff": runoff,
            "boost": kind[1], "oil": kind[2], "gates": kind[3],
            "note": kind[4]}


def _esses(n, radius, start, heading, runoff, kind):
    steps = []
    for i in range(n):
        steps.append(S(7))
        steps.append(C(70 if i % 2 == 0 else -70, radius))
    steps += [S(11), C(45, 4.5), S(10), C(-45, 4.5)]
    return {"name": kind[0], "start": start, "heading": heading,
            "steps": steps, "runoff": runoff,
            "boost": kind[1], "oil": kind[2], "gates": kind[3],
            "note": kind[4]}


def _rect(n, radius, start, heading, runoff, kind):
    """Right angles: every turn is +/-90."""
    steps = [S(9)]
    for i in range(n):
        steps.append(C(90 if i % 2 == 0 else -90, radius))
        steps.append(S(7 if i % 2 == 0 else 9))
    return {"name": kind[0], "start": start, "heading": heading,
            "steps": steps, "runoff": runoff,
            "boost": kind[1], "oil": kind[2], "gates": kind[3],
            "note": kind[4]}


def _sweepers(n, radius, start, heading, runoff, kind):
    steps = [S(13)]
    for i in range(n):
        steps.append(C(50 if i % 2 == 0 else -55, radius))
        steps.append(S(11))
    return {"name": kind[0], "start": start, "heading": heading,
            "steps": steps, "runoff": runoff,
            "boost": kind[1], "oil": kind[2], "gates": kind[3],
            "note": kind[4]}


def _chicane(n, radius, start, heading, runoff, kind):
    steps = [S(10)]
    for _ in range(n):
        steps += [C(105, radius), C(-105, radius)]
    steps += [S(11), C(110, 2.6), S(8), C(-110, 2.6)]
    return {"name": kind[0], "start": start, "heading": heading,
            "steps": steps, "runoff": runoff,
            "boost": kind[1], "oil": kind[2], "gates": kind[3],
            "note": kind[4]}


#: Sixteen composed circuits, four per cup. The eight hand-written circuits
#: above are kept as well, so the atlas is 24 -- four cups of six, which is what
#: `TOTAL_TRACKS` and the championship expect.
_GENERATED = [
    # --- Cup 1, Bronze ---
    _rect(6, 2.4, (5, 5), 90, GRAVEL,
          ("Old Town", (0.2, 0.7), (0.45,), (0.2, 0.5, 0.8),
           "Six right angles, alternating directions.")),
    _hairpins(3, 2.6, (12, 9, 6), (5, 24), 0, GRASS,
              ("Pigeon Ravine", (0.3,), (0.7,), (0.25, 0.6, 0.85),
               "Three hairpins in a narrowing valley.")),
    _esses(4, 3.0, (5, 20), 90, SAND,
           ("Salt Serpent", (0.35,), (0.6,), (0.3, 0.65, 0.9),
            "Long esses over salt, then two sweepers.")),
    _sweepers(3, 5.0, (6, 16), 90, GRASS,
              ("Breeze Hill", (0.25, 0.7), (), (0.35, 0.75),
               "Fast sweepers and long straights.")),
    _chicane(3, 2.0, (4, 14), 0, GRAVEL,
             ("Weir Raceway", (0.4,), (0.65, 0.85), (0.25, 0.55, 0.8),
              "Three chicanes at the tight limit.")),
    _hairpins(4, 3.0, (11, 8, 6), (6, 6), 90, SAND,
              ("Dustbowl", (0.2, 0.66), (0.5,), (0.15, 0.5, 0.85),
               "Four hairpins, wide entry, sand runoff.")),
    # --- Cup 2, Silver ---
    _rect(8, 2.2, (5, 5), 0, GRAVEL,
          ("Grid Nine", (), (0.3, 0.7), (0.2, 0.45, 0.7, 0.9),
           "A long rectilinear street grid.")),
    _esses(5, 2.8, (4, 22), 90, GRASS,
           ("Knot Garden", (0.5,), (0.75,), (0.25, 0.5, 0.75),
            "Five esses. The tightest rhythm circuit.")),
    _hairpins(2, 3.4, (14, 11, 7), (6, 18), 0, SAND,
              ("Twin Sisters", (0.4, 0.8), (), (0.3, 0.7),
               "Two big hairpins around the middle of the lap.")),
    _sweepers(4, 4.4, (5, 20), 90, GRASS,
              ("Long Meadow", (0.22, 0.68), (0.5,), (0.3, 0.7),
               "Four sweepers, all the same radius.")),
    _rect(4, 2.6, (6, 6), 90, GRASS,
          ("Foundry Block", (0.35,), (0.7,), (0.25, 0.6, 0.85),
           "Right angles with a longer entry to each corner.")),
    _chicane(4, 2.2, (5, 16), 0, SAND,
             ("Chalk Works", (), (0.45, 0.8), (0.3, 0.6, 0.85),
              "Four chicanes and a slow final sector.")),
    # --- Cup 3, Gold ---
    _hairpins(5, 2.8, (11, 8, 6), (5, 26), 0, GRASS,
              ("Camel Back", (0.25, 0.7), (0.55,), (0.2, 0.5, 0.8),
               "Five hairpins. The longest sequence in the game.")),
    _sweepers(3, 6.0, (7, 14), 90, GRAVEL,
              ("Oasis Run", (0.3, 0.72), (), (0.35, 0.75),
               "Big-radius sweepers, gravel traps either side.")),
    _esses(3, 4.0, (6, 8), 90, GRASS,
           ("Cedar Bend", (0.45,), (0.7,), (0.3, 0.65, 0.9),
            "Loose esses between two fast sweepers.")),
    _rect(6, 2.8, (5, 6), 0, SAND,
          ("Old Quarry", (0.3,), (0.65,), (0.2, 0.5, 0.8),
           "Right angles cut into a quarry floor.")),
    _chicane(5, 2.1, (4, 18), 0, GRASS,
             ("Reed Bank", (0.5,), (0.3, 0.75), (0.25, 0.55, 0.85),
              "Five chicanes, the longest rhythm in the game.")),
    _hairpins(3, 3.6, (13, 10, 7), (6, 12), 90, GRAVEL,
              ("Pass du Vent", (0.35,), (0.7,), (0.3, 0.6, 0.85),
               "A mountain pass: three hairpins, fast between them.")),
    # --- Cup 4, Platinum ---
    _sweepers(5, 5.2, (6, 18), 90, GRASS,
              ("Glacier Bends", (0.2, 0.62), (0.75,), (0.28, 0.7),
               "Five sweepers. The fastest circuit in the game.")),
    _hairpins(4, 2.4, (10, 8, 6), (5, 22), 0, SAND,
              ("Harbour Hairpins", (0.4,), (0.6,), (0.22, 0.55, 0.82),
               "Four tight hairpins, sand runoff, little room.")),
    _esses(6, 2.6, (4, 24), 90, GRAVEL,
           ("Alpine Serpent", (0.48,), (0.78,), (0.2, 0.42, 0.64, 0.86),
            "Six esses. The busiest rhythm circuit.")),
    _rect(8, 2.0, (5, 5), 90, GRASS,
          ("Marina Grid", (0.3, 0.75), (0.45, 0.85), (0.2, 0.4, 0.6, 0.8),
           "Eight right angles at minimum radius. Technical.")),
]

#: Four per cup from each group of six, to make 16 alongside the 8 above.
CIRCUITS += (
    [c for c in _GENERATED[0:6] if c in (None,) or True][0:4] +
    _GENERATED[6:10] + _GENERATED[12:16] + _GENERATED[18:22]
)


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
    # Two images, same size, aligned: the data map and the visual.
    write_visual(img, base + ".visual.png", base + ".data.png")

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
        "data": base + ".data.png",
        "visual": base + ".visual.png",
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
              f"tunnel {info['tunnel']:4}  -> "
              f"{os.path.basename(info['data'])} + .visual.png")
    print(f"\nwrote to {outdir}")


if __name__ == "__main__":
    main()
