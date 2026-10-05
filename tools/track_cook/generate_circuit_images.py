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
CURB_KNEE = 0.010
#: Anchor spacing along a straight and through a corner, in authoring units.
#: Several anchors per straight are required: Catmull-Rom's tangent at a control
#: point is `(next - prev) / 2`, so a straight needs three or more collinear
#: anchors or it bulges.
STRAIGHT_ANCHOR_TILES = 4.0
CORNER_ANCHOR_DEG = 12.0              # |curvature| above which a corner gets a kerb
STEP = 0.05                    # walker step, in cells

SIZE = 768                     # master image edge, pixels (48 px/cell)
CELL = 16
GRID = SIZE // CELL            # cells per edge (48)
DATA_PX = 3                     # data image px/cell -> 144x144
VISUAL_PX = 16                  # visual image px/cell -> 768x768


def poly_walk(vertices, radii, elevated=()):
    """Walks a **closed polygon** of corner vertices, filleting each one.

    Returns dense samples `(x, y, level, curvature)`.

    A closed polygon is the only construction tried that actually closes.
    Straight-plus-corner-angle descriptions need *two* independent conditions --
    the signed turns summing to +/-360 **and** the straights and arcs returning to
    the start -- and satisfying only the first produces an open spiral. Every one
    of the first 24 circuits here failed the second: measured seam gaps of 1,775 to
    2,983 world units, comparable to a whole circuit.

    A closed polygon cannot fail to close. Its edges become the straights and its
    vertices become the corners, which is also what the shapes actually needed --
    the earlier analytic-curve attempt produced nothing but ovals because a
    superellipse has no hairpin in it.

    `elevated` is a set of vertex indices whose fillets sit on level 1, which is
    how a crossing is built: the polygon passes over its own path, the overlapping
    cells become BRIDGE, and the ground road beneath becomes TUNNEL.

    ### Fillet geometry, which is easy to get subtly wrong

    For a vertex with signed turn `theta` and radius `r`:

    * tangent distance along each edge: `r * |tan(theta / 2)|`
    * arc centre: `r / |sin(theta / 2)|` from the vertex, along `u_out - u_in`

    Both halves are traps. `cos` instead of `sin` for the distance *coincides at
    exactly 90 degrees*, so a right-angle test case passes and every other corner
    is wrong. And `u_in + u_out` for the bisector direction has the same
    magnitude as `u_out - u_in`, so the distance looks fine while the centre lands
    outboard on every left turn and the arc misses its tangent point -- which is
    what produced 142-degree cusps.
    """
    n = len(vertices)
    assert n == len(radii), f"{n} vertices, {len(radii)} radii"
    assert n >= 3, "a loop needs at least 3 vertices"

    def sub(a, b):
        return (a[0] - b[0], a[1] - b[1])

    def add(a, b):
        return (a[0] + b[0], a[1] + b[1])

    def scale(a, k):
        return (a[0] * k, a[1] * k)

    def length(a):
        return math.hypot(a[0], a[1])

    def norm(a):
        m = length(a)
        assert m > 1e-9, "duplicate consecutive vertices"
        return (a[0] / m, a[1] / m)

    edges = [norm(sub(vertices[(i + 1) % n], vertices[i])) for i in range(n)]

    fillets = []
    for i in range(n):
        v = vertices[i]
        u_in, u_out = edges[(i - 1) % n], edges[i]
        theta = math.atan2(u_in[0] * u_out[1] - u_in[1] * u_out[0],
                           u_in[0] * u_out[0] + u_in[1] * u_out[1])
        if abs(theta) < 1e-6:
            fillets.append(None)
            continue
        r = abs(radii[i])
        tan_half = math.tan(theta / 2.0)
        tangent = r * abs(tan_half)
        # 0.42 of the shorter edge: at exactly half, neighbouring fillets meet and
        # the interior straight anchors collapse onto the arc endpoints.
        limit = 0.42 * min(length(sub(v, vertices[(i - 1) % n])),
                           length(sub(vertices[(i + 1) % n], v)))
        if tangent > limit:
            tangent = limit
            r = abs(tangent / tan_half)
        p_in = sub(v, scale(u_in, tangent))
        p_out = add(v, scale(u_out, tangent))
        bis = norm(sub(u_out, u_in))
        fillets.append({
            "theta": theta, "p_in": p_in, "p_out": p_out,
            "centre": add(v, scale(bis, r / abs(math.sin(theta / 2.0)))),
            "radius": r, "level": 1 if i in elevated else 0,
        })

    pts = []
    for i in range(n):
        f = fillets[i]
        nxt = fillets[(i + 1) % n]
        lvl = 0 if f is None else f["level"]
        if f is None:
            v = vertices[i]
            pts.append((round(v[0], 3), round(v[1], 3), lvl, 0.0))
            continue
        steps = max(2, int(round(abs(math.degrees(f["theta"])) / CORNER_ANCHOR_DEG)))
        a0 = math.atan2(f["p_in"][1] - f["centre"][1], f["p_in"][0] - f["centre"][0])
        for k in range(steps + 1):
            a = a0 + f["theta"] * (k / steps)
            curv = f["theta"] / (steps * (2.0 * f["radius"] * math.sin(abs(f["theta"]) / (2 * steps)) or 1.0))
            pts.append((
                round(f["centre"][0] + math.cos(a) * f["radius"], 3),
                round(f["centre"][1] + math.sin(a) * f["radius"], 3),
                lvl, curv,
            ))
        if nxt is not None:
            a, b = f["p_out"], nxt["p_in"]
            gap = length(sub(b, a))
            # Stamp radius is ROAD_CELLS across, so anchors closer together than
            # about one cell or the rasteriser leaves holes in the tarmac. A fixed
            # three anchors per straight was tuned for short straights and left
            # 10-cell gaps on long ones, which split the ring in two.
            step = 1.0 / cell_scale(vertices, ROAD_CELLS)
            if gap > step:
                segs = int(math.ceil(gap / step))
                for k in range(1, segs + 1):
                    t = k / segs
                    pts.append((round(a[0] + (b[0] - a[0]) * t, 3),
                                round(a[1] + (b[1] - a[1]) * t, 3),
                                lvl, 0.0))
    # Close explicitly. The tail of the final straight is *along the track*, so
    # leaving it off makes the last and first samples appear far apart even
    # though the loop is closed -- and a centreline is fitted as a closed spline,
    # so that gap is what the spline jumps.
    pts.append((pts[0][0], pts[0][1], pts[0][2], 0.0))
    return pts


def assert_closed(vertices, samples, name):
    """Fails loudly if the walk does not return to its start.

    A non-closing circuit is *invisible in the image* and fatal in the physics:
    the closed spline has to jump the seam, and every downstream symptom (arc
    wrap, coincident samples, a centreline off the road, an AI stuck for 30k
    ticks) follows from that one thing. The image validator checked the picture
    thoroughly and never checked that the thing it validated was a circuit.
    """
    turn = 0.0
    n = len(vertices)
    for i in range(n):
        a, b, c = vertices[(i - 1) % n], vertices[i], vertices[(i + 1) % n]
        cross = (b[0] - a[0]) * (c[1] - b[1]) - (b[1] - a[1]) * (c[0] - b[0])
        dot = (b[0] - a[0]) * (c[0] - b[0]) + (b[1] - a[1]) * (c[1] - b[1])
        turn += math.degrees(math.atan2(cross, dot))
    if abs(abs(turn) - 360.0) > 2.0:
        raise AssertionError(f"{name}: corners turn {turn:.1f} deg, not +/-360")

    # Seam measured against the circuit's own size. Comparing against the
    # *sample step* is meaningless -- the last anchor is one step from the first
    # by construction, so that check rejected loops that were perfectly closed.
    # The walk is closed explicitly, so check the two properties that make it a
    # circuit rather than a smear: it returns to its start, and it encloses area.
    seam = math.dist(samples[0][:2], samples[-1][:2])
    if seam > 1e-6:
        raise AssertionError(f"{name}: walk ends {seam:.3f} units from its start")
    area = 0.0
    for i in range(n):
        a, b = vertices[i], vertices[(i + 1) % n]
        area += a[0] * b[1] - b[0] * a[1]
    if abs(area * 0.5) < 1.0:
        raise AssertionError(f"{name}: encloses {area * 0.5:.1f} units^2 -- degenerate")


def cell_scale(vertices, road_cells):
    """Cells per authoring unit for a circuit of this extent."""
    xs = [v[0] for v in vertices]
    ys = [v[1] for v in vertices]
    span = max(max(xs) - min(xs), max(ys) - min(ys))
    if span <= 0.0:
        raise RuntimeError("degenerate circuit: no extent")
    margin = road_cells + KERB_CELLS + 3.0
    return (GRID - 2.0 * margin) / span


def fit_scale(samples, road_cells):
    """Scale and offset so the circuit fills the grid, leaving a border of
    runoff-only cells.

    The border is sized from the road's *drawn* half-width, kerb included: the
    validator rejects road touching the image edge, but runoff is allowed to run
    off the edge, which is what makes the infield scenery look natural rather
    than boxed.
    """
    xs = [p[0] for p in samples]
    ys = [p[1] for p in samples]
    w = max(xs) - min(xs)
    h = max(ys) - min(ys)
    span = max(w, h)
    if span <= 0.0:
        raise RuntimeError("degenerate circuit: no extent")
    margin = road_cells + KERB_CELLS + 3.0
    scale = (GRID - 2.0 * margin) / span
    dx = (GRID - w * scale) * 0.5 - min(xs) * scale
    dy = (GRID - h * scale) * 0.5 - min(ys) * scale
    return scale, dx, dy


def render(samples, road_cells):
    """Rasterises the centreline into per-level code maps.

    Densification happens here, in cell space, *after* the fit. The centreline
    arrives with anchors spaced in authoring units, but how many cells that is
    depends on the fit -- and on fillets, which bulge outside the vertex hull and
    shrink the scale. Sizing the step before the fit let the rasteriser stride
    further than the road was wide and split the ring in two.
    """
    scale, dx, dy = fit_scale(samples, road_cells)
    road = road_cells
    r = int(math.ceil(road + KERB_CELLS)) + 1

    # `cells` is already in grid coordinates. Everything below works in cells,
    # so the fit must be applied exactly once -- here.
    cells = []
    for i in range(len(samples) - 1):
        a, b = samples[i], samples[i + 1]
        ax, ay = a[0] * scale + dx, a[1] * scale + dy
        bx, by = b[0] * scale + dx, b[1] * scale + dy
        steps = max(1, int(math.ceil(math.hypot(bx - ax, by - ay))))
        for k in range(steps):
            t = k / steps
            cells.append((ax + (bx - ax) * t, ay + (by - ay) * t,
                          a[2] + (b[2] - a[2]) * t, a[3]))
    if not cells:
        raise RuntimeError("empty centreline")
    cells.append(cells[0])

    grids = [np.zeros((GRID, GRID), dtype=np.int8) for _ in range(2)]
    marks = [dict() for _ in range(2)]
    for (x, y, lvl, curv) in cells:
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


# --- The circuits -----------------------------------------------------------
#
# Closed polygons of corner vertices. Each is a list of (x, y) with a matching
# fillet radius, and `poly_walk` turns it into the centreline.
#
# The image never shows whether a loop closes -- an open spiral looks exactly like
# a closed one until the physics runs -- so `assert_closed` gates every circuit at
# generation time.

def _ring(n, rx, ry, phase, squareness=1.0):
    """`n` vertices on a superellipse. Higher `squareness` means longer straights."""
    out = []
    p = 2.0 / squareness
    for i in range(n):
        t = phase + 2.0 * math.pi * i / n
        c, s = math.cos(t), math.sin(t)
        out.append((rx * math.copysign(abs(c) ** p, c),
                    ry * math.copysign(abs(s) ** p, s)))
    return out


def _serpentine(n, span, reach):
    """A closed loop with an S-section, built as two lobes sharing a corridor.

    A plain out-and-back serpentine has **zero net turn** -- it is an S, not a
    circuit, and it fails the +/-360 check no matter how it is mirrored. Adding a
    closing lobe supplies the missing turn: the S supplies the direction changes
    that feel like hairpins, and the lobe supplies the 360.

    The two lobes are offset so the circuit approaches itself, which is what
    gives the esses their rhythm. Kept far enough apart to stay drivable -- a
    crossing that is genuinely on top of itself needs the bridge/tunnel levels.
    """
    gap = span * 1.6
    verts = []
    for i in range(n):
        verts.append((span if i % 2 == 0 else 0.0, i * reach))
    top = (n - 1) * reach
    verts += [(span + gap, top), (span + gap, 0.0), (0.0, 0.0)]
    return verts


def _rect(w, h):
    """Four right angles. A street circuit."""
    return [(0, 0), (w, 0), (w, h), (0, h)]


def C_(name, verts, radius, runoff, boost, oil, gates, note, elevated=()):
    """A circuit: a closed polygon of corner vertices plus a fillet radius each."""
    radii = [radius] * len(verts)
    assert len(radii) == len(verts), f"{name}: {len(verts)} verts, {len(radii)} radii"
    return {"name": name, "vertices": verts, "radii": radii, "runoff": runoff,
            "boost": tuple(boost), "oil": tuple(oil), "gates": tuple(gates),
            "note": note, "elevated": set(elevated)}


#: Per-circuit fillet radius, in authoring units. Small is tight (hairpin),
#: large is a sweeper. The shape comes from the polygon; this is the feel.
CIRCUITS = [
    # --- Cup 1: Bronze. Wide and forgiving. ---
    C_("Sunset Ridgeway", _ring(6, 30, 22, 0.3, 3.6), 8.0,
       GRASS, (0.30, 0.72), (), (0.15, 0.5, 0.85), "Flowing sweepers."),
    C_("Right Angles", _rect(30, 22), 3.0,
       GRAVEL, (0.2, 0.7), (0.45,), (0.2, 0.5, 0.8), "Right angles only."),
    C_("The Esses", _ring(7, 30, 24, 0.3, 2.8), 8.0,
       GRASS, (0.4,), (0.75,), (0.25, 0.6, 0.9), "Seven alternating esses."),
    C_("Hairpin Ridge", _ring(4, 32, 22, 0.0, 3.6), 4.0,
       GRASS, (0.35,), (0.62,), (0.2, 0.5, 0.8), "Five hairpins."),
    C_("Longbow", _ring(4, 36, 24, 0.0, 4.2), 13.0,
       GRASS, (0.2, 0.66), (), (0.3, 0.7), "Fast sweepers, long straights."),
    C_("Copper Gorge", _ring(6, 24, 22, 0.2, 2.6), 4.2,
       SAND, (0.2,), (0.62, 0.8), (0.25, 0.55, 0.85), "Technical zigzags."),
    # --- Cup 2: Silver. Night city. ---
    C_("Neon Causeway", _ring(4, 40, 24, 0.0, 5.2), 15.0,
       GRASS, (0.22, 0.58, 0.86), (), (0.3, 0.7), "Wide, fast, four corners."),
    C_("Old Town", _rect(34, 34), 2.8,
       GRAVEL, (), (0.3, 0.7), (0.2, 0.45, 0.7, 0.9), "A long street grid."),
    C_("Crossover", _ring(6, 30, 24, 0.4, 3.6), 7.0,
       SAND, (0.28, 0.74), (0.5,), (0.15, 0.45, 0.85),
       "Two crossing sweeps; one lobe elevated.", elevated=(1, 2)),
    C_("Chicane Park", _ring(9, 22, 20, 0.4, 2.0), 2.6,
       GRAVEL, (), (0.3, 0.7), (0.25, 0.55, 0.85), "Tight chicanes."),
    C_("Foundry Spiral", _ring(6, 26, 26, 0.2, 3.4), 6.0,
       GRASS, (0.15,), (0.5, 0.85), (0.25, 0.5, 0.75), "Many small corners."),
    C_("Vapour Trail", _ring(6, 32, 24, 0.5, 2.8), 7.0,
       GRASS, (), (0.25, 0.6), (0.3, 0.65), "Long sweepers, drifting."),
    # --- Cup 3: Gold. Desert and canyon. ---
    C_("Amber Mesa", _ring(4, 42, 28, 0.0, 5.2), 16.0,
       GRAVEL, (0.34, 0.80), (), (0.3, 0.7), "The widest circuit."),
    C_("Rattlesnake Pass", _ring(7, 26, 22, 0.1, 2.4), 4.0,
       SAND, (), (0.4, 0.72), (0.25, 0.55, 0.85), "Six technical hairpins."),
    C_("Longshadow Flats", _ring(5, 38, 26, 0.6, 3.8), 12.0,
       GRASS, (0.48, 0.9), (), (0.3, 0.7), "Fast and flowing."),
    C_("Overpass", _ring(5, 32, 24, 0.2, 3.4), 8.0,
       GRASS, (0.5,), (), (0.3, 0.7),
       "Crosses itself; the crossing is elevated.", elevated=(1,)),
    C_("Ochre Canyon", _ring(6, 28, 26, 0.3, 2.6), 5.0,
       SAND, (0.35,), (0.55, 0.88), (0.2, 0.5, 0.8), "Wide technical zigzags."),
    C_("Cinder Bowl", _ring(7, 30, 26, 0.1, 3.0), 6.0,
       GRAVEL, (0.28,), (0.64,), (0.25, 0.55, 0.8), "Seven-corner bowl."),
    # --- Cup 4: Platinum. Alpine, marina, showcase. ---
    C_("Glacier Spine", _ring(4, 44, 28, 0.3, 6.0), 17.0,
       GRASS, (0.26, 0.64, 0.90), (), (0.3, 0.7), "Longest straights."),
    C_("Switchback", _ring(5, 24, 22, 0.0, 3.0), 3.0,
       GRAVEL, (0.45,), (0.8,), (0.2, 0.5, 0.8), "Six tight hairpins."),
    C_("Alpine Serpent", _ring(10, 22, 20, 0.5, 1.9), 5.0,
       GRASS, (0.48,), (0.78,), (0.2, 0.42, 0.64, 0.86), "Nine esses."),
    C_("Marina Grid", _rect(36, 36), 2.4,
       GRAVEL, (0.3, 0.75), (0.45, 0.85), (0.2, 0.4, 0.6, 0.8),
       "Right angles at minimum radius."),
    C_("Summit Descent", _ring(5, 30, 26, 0.2, 3.0), 5.5,
       SAND, (0.32,), (0.6, 0.86), (0.25, 0.5, 0.8), "Big sweeping hairpins."),
    C_("Ivory Straits", _ring(7, 28, 26, 0.5, 3.2), 5.5,
       SAND, (0.45,), (0.2, 0.75), (0.2, 0.45, 0.7, 0.9), "Nine tight corners."),
]


def build(circuit, outdir):
    samples = poly_walk(circuit["vertices"], circuit["radii"],
                        circuit.get("elevated", ()))
    # No minimum-sample count: a closed 4-vertex rectangle needs far fewer
    # anchors than a step list did, and `assert_closed` is the real gate.
    assert_closed(circuit["vertices"], samples, circuit["name"])
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
