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
# SCENERY (15) is drivable-looking but is not road: it is placed in the infield,
# so counting it made every circuit look like it had a stray island of road.
ROAD = {1, 2, 3, 4, 8, 9, 10, 11, 13, 14}

#: Codes the *runtime* calls racing surface, i.e. `TrackTile::is_road`.
#:
#: `ROAD` above is the validator's looser set: it includes OIL (8) and BOOST (9)
#: because both sit on the tarmac and neither should read as a hole in the
#: circuit. But `TrackTile::is_road` is Tarmac | Curb | StartFinish | Checkpoint
#: only, so oil and boost are *not* road to the engine -- and a gate grown over
#: them produces a gate the engine considers to be in the runoff.
#:
#: Gate geometry has to agree with the engine's definition, not the validator's.
#: Growing over this set instead measured: 8 gates across the 24 circuits
#: straddled an oil slick or a boost pad and were rejected as off-road by
#: `make playtest`.
GATE_ROAD = {1, 2, 3, 4, 10, 11, 13, 14}


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

import math

# World units per grid cell. Must match `arduracer_core::track::TILE_SIZE`, which
# is the same constant the runtime indexes the map with: the two are the contract
# between this compiler and the game, and a mismatch shows up as gates and
# centreline points sitting at the wrong world position.
TILE_SIZE = 32

# World units per *authoring* cell. The circuits are drawn as 48x48 cells of 64
# units (a 3,072-unit world); `generate_circuit_images.SURF` splits each of those
# into `SURF` grid cells, so the grid cell is 64 / SURF world units. Anything
# expressed in authoring units -- road half-width above all -- has to use this,
# not `TILE_SIZE`, or the road comes out half or twice as wide as it was drawn.
AUTHORING_CELL = 64

#: Authoring cells per grid cell: `generate_circuit_images.SURF`. The centreline
#: sidecar is in authoring cells; the surface grid is `SURF` times finer, so every
#: centreline coordinate is scaled by this before it is used against the grid.
#: Without it the centreline lands in the top-left quarter of the map and the
#: gates sit away from the road they belong to.
SURF = 2

#: Drivable half-width in authoring cells, from `generate_circuit_images.ROAD_CELLS`.
#: The road is twice this many *grid* cells across, which is what gate spans and
#: anything else that has to cover the road is measured against.
ROAD_CELLS = 2.5
MAX_CHECKPOINTS = 24


def tile_centre(c):
    """World coordinate of a cell's centre.

    Not its corner. The old cooker emitted `cx * TILE_SIZE + TILE_SIZE // 2`;
    emitting the corner put every control point half a tile (32 world units) off
    the road, which on a 2.5-cell road is enough to land on a barrier, and it
    left the runtime spline cutting corners.
    """
    return int(c) * TILE_SIZE + TILE_SIZE // 2


def resample_uniform(points, count):
    """Picks `count` points at **equal arc length**.

    Decimating by index (`points[::n]`) is the obvious thing and it is wrong.
    The centreline walker moves in small steps through a corner and large steps
    along a straight, so index-decimation bunches control points up at exactly
    the tightest corners and leaves long gaps between them. Catmull-Rom takes its
    tangent at a control point from `(next - prev) / 2`, so a corner point with
    distant neighbours gets an enormous tangent and the spline swings off the
    road -- which is what put `Hairpin Ridge`'s first samples on a Barrier.

    Equal arc length gives evenly spaced control points and therefore stable
    tangents everywhere.
    """
    n = len(points)
    if n < 3 or count < 3:
        return list(points)
    cum = [0.0]
    for i in range(1, n):
        cum.append(cum[-1] + math.hypot(points[i][0] - points[i - 1][0],
                                        points[i][1] - points[i - 1][1]))
    total = cum[-1]
    if total <= 0:
        return list(points)
    out = []
    seg = 0
    for k in range(count):
        target = total * k / count
        while seg < n - 1 and cum[seg + 1] < target:
            seg += 1
        span = cum[seg + 1] - cum[seg]
        t = (target - cum[seg]) / span if span > 1e-9 else 0.0
        a, b = points[seg], points[seg + 1]
        out.append((a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t))
    return out


#: Most cells a gate may span across the road.
#:
#: Scoped from the road half-width, so it tracks resolution: the road is
#: `ROAD_CELLS * SURF` cells across, and a gate narrower than the road is a gate
#: the car drives just outside of. At `SURF = 2` the road is 10 cells and an
#: uncapped-5 span covered half of it -- measured on The Esses, where every lap
#: went unscored for that reason.
#:
#: The cap still matters: growing "while the tiles stay road" is unbounded where
#: the circuit folds back on itself, and measured on Sunset Ridgeway that
#: produced a 21-cell gate whose corners sat in the infield.
MAX_GATE_SPAN = int(ROAD_CELLS * SURF) + 1


def gate_span(grid, cx, cy, dx, dy):
    """Road extent across the direction of travel at `(cx, cy)`, in cells.

    Grown symmetrically from the centreline cell along whichever single axis is
    most nearly perpendicular to the heading, and only while the tiles stay
    racing surface -- so the gate is a line across the road, its corners cannot
    land in the runoff, and it stays bounded by [`MAX_GATE_SPAN`] where the
    circuit folds back on itself.

    Returns `(lo, hi)` inclusive cell indices along that axis.
    """
    h, w = grid.shape
    def road(x, y):
        return 0 <= x < w and 0 <= y < h and int(grid[y, x]) in GATE_ROAD

    cx = max(0, min(w - 1, int(cx)))
    cy = max(0, min(h - 1, int(cy)))
    if not road(cx, cy):
        return (0, 0)

    # `dx`/`dy` are the heading in cells. The axis to span across is the one the
    # heading barely moves along; a diagonal heading spans across the larger
    # component's other axis, which keeps the gate a thin slice rather than a
    # block.
    if abs(dx) >= abs(dy):
        lo = hi = cy
        while lo - 1 >= 0 and road(cx, lo - 1) and cy - lo + 1 < MAX_GATE_SPAN:
            lo -= 1
        while hi + 1 < h and road(cx, hi + 1) and hi - cy + 1 < MAX_GATE_SPAN:
            hi += 1
        return (lo, hi)

    lo = hi = cx
    while lo - 1 >= 0 and road(lo - 1, cy) and cx - lo + 1 < MAX_GATE_SPAN:
        lo -= 1
    while hi + 1 < w and road(hi + 1, cy) and hi - cx + 1 < MAX_GATE_SPAN:
        hi += 1
    return (lo, hi)


def gate_box(grid, cx, cy, dx=1, dy=0):
    """`CheckpointGate` cell rectangle spanning the road across the direction of travel.

    A 1x1 gate is a point target on a road 2-3 cells wide, so a car anywhere else
    on the tarmac drives straight past it: measured on the 24 circuits, a rival
    following the racing line missed 1x1 gates on 21 of 24 circuits, up to three
    per lap, and a lap only scores when every gate is touched -- so no lap ever
    completed. The gate has to span the road.

    Returns `(x, y, width, height)`, always containing `(cx, cy)`.
    """
    if abs(dx) >= abs(dy):
        lo, hi = gate_span(grid, cx, cy, dx, dy)
        return int(cx), int(lo), 1, int(hi - lo + 1)
    lo, hi = gate_span(grid, cx, cy, dx, dy)
    return int(lo), int(cy), int(hi - lo + 1), 1


def gate_is_disjoint(box, other):
    """Whether two gate rectangles share no cell.

    `TrackDef::min_route_length` sums the gaps between consecutive gate
    rectangles, so overlapping gates report a zero-length circuit and take the
    lap-time floor with them.
    """
    ax, ay, aw, ah = box
    bx, by, bw, bh = other
    return not (ax < bx + bw and bx < ax + aw and ay < by + bh and by < ay + ah)


def trim_to_road(grid, box, cx, cy):
    """Narrow `box` to the contiguous run of racing surface containing `(cx, cy)`.

    `(cx, cy)` is the centreline cell the gate was placed on, so it is always
    road and always inside `box` -- the run is therefore never empty. Cells are
    dropped from whichever end reaches non-road first, which keeps the gate a
    line across the road rather than a rectangle with a corner in the runoff.
    """
    h, w = grid.shape
    def road(x, y):
        return 0 <= x < w and 0 <= y < h and int(grid[y, x]) in GATE_ROAD

    x, y, bw, bh = box
    if not road(cx, cy):
        return (cx, cy, 1, 1)

    while bw > 1 and not road(x, y):
        x += 1
        bw -= 1
    while bw > 1 and not road(x + bw - 1, y):
        bw -= 1
    while bh > 1 and not road(x, y):
        y += 1
        bh -= 1
    while bh > 1 and not road(x, y + bh - 1):
        bh -= 1
    return (x, y, bw, bh)


def shrink_until_disjoint(box, *others):
    """Narrowest sub-rectangle of `box` that touches none of `others`.

    Drops one cell at a time from whichever side is currently overlapping, so the
    gate keeps as much of the road as it can while staying disjoint. Returns
    `None` if even a 1x1 core cannot be made disjoint -- the caller then places
    the gate on the bare centreline cell, which is the last resort.

    Bounded: at most `w + h` reductions, each of which strictly shrinks the
    rectangle, so this cannot loop.
    """
    x, y, w, h = box
    for _ in range(w + h):
        if all(gate_is_disjoint((x, y, w, h), o) for o in others):
            return (x, y, w, h)
        # Prefer narrowing the longer axis: it is where the slack is.
        if h >= w and h > 1:
            if not any(gate_is_disjoint((x, y, w, h - 1), o) for o in others):
                h -= 1
                continue
        if w > 1 and not any(gate_is_disjoint((x, y, w - 1, h), o) for o in others):
            w -= 1
            continue
        if h > 1 and not any(gate_is_disjoint((x, y, w, h - 1), o) for o in others):
            h -= 1
            continue
        if w > 1:
            w -= 1
            continue
        if h > 1:
            h -= 1
            continue
        return None
    return None


def snap_to_road(grid, cx, cy, prev=None, limit=None, also_near=None, also_limit=None):
    """Nearest racing-surface cell centre to `(cx, cy)`, as `(x, y)` cells.

    The centreline is resampled by equal arc length, which is what makes the
    runtime spline's tangents stable -- but it also cuts the corner the walker
    went round, because a straight chord between two control points 60-200 units
    apart passes inside the turn. Measured across the 24 circuits that put 868 of
    the resampled samples (4.7%) on runoff rather than tarmac, worst case Neon
    Causeway at 70, and the worst cases are all tight corners.

    Snapping each control point back to the nearest racing-surface cell is what
    makes the centreline agree with the image by construction, which is the whole
    premise of the format: the line the car drives and the road it drives on are
    then the same thing.

    `prev` and `limit` keep the snap *local*, which is not optional on a circuit
    whose road passes close to itself. Snapping each point independently lets a
    point cut off at one hairpin snap onto a different hairpin a few cells away:
    measured on Alpine Serpent that turned steps of 64-128 units into 286 and
    404, and a step that long makes the runtime walk leap across the circuit, so
    the car aims at the far side of the map once per lap. With `prev` given, a
    candidate further than `limit` from it is rejected, so a point can only ever
    move as far as the walk itself would.

    Falls back to the input cell when nothing qualifies, so a badly-placed point
    is left alone rather than teleported.
    """
    h, w = grid.shape
    def road(x, y):
        return 0 <= x < w and 0 <= y < h and int(grid[y, x]) in GATE_ROAD

    cx, cy = int(cx), int(cy)
    if road(cx, cy):
        return cx, cy
    best = None
    best_d = None
    r = MAX_SNAP_CELLS
    for dy in range(-r, r + 1):
        for dx in range(-r, r + 1):
            x, y = cx + dx, cy + dy
            if not road(x, y):
                continue
            if prev is not None and limit is not None:
                if math.dist((x, y), prev) > limit:
                    continue
            if also_near is not None and also_limit is not None:
                if math.dist((x, y), also_near) > also_limit:
                    continue
            d = dx * dx + dy * dy
            if best_d is None or d < best_d:
                best_d, best = d, (x, y)
    return best if best is not None else (cx, cy)


#: Farthest a control point may be pulled to find racing surface, in cells.
#:
#: Three cells is a little over one road width, so a corner cut by a single cell
#: is recovered without the line jumping to a different part of the circuit.
MAX_SNAP_CELLS = 3


def route_control_points(grid, centre, count=96):
    """The control points the runtime's `route` array will actually hold.

    Single source of truth for both the emitted route and the gate positions, so
    a gate cannot drift off the line the car is driving. Deduplicates the
    walker's repeated closing point (a zero-length span gives Catmull-Rom a
    degenerate tangent neighbour at the seam), resamples to equal arc length, and
    snaps each point back onto the racing surface (`snap_to_road`).
    """
    pts = list(centre)
    while (len(pts) > 2
           and math.dist(pts[0][:2], pts[-1][:2]) < 1e-6):
        pts.pop()
    # Resample the *closed* loop. `resample_uniform` distributes points by arc
    # length along an open polyline, so its last point sits one step short of
    # where it started and the seam -- the span from the last point back to the
    # first -- absorbs whatever the walker's own closing gap happened to be.
    # Measured on Alpine Serpent: every step was ~2.85 cells and the seam was
    # 8.75, so the runtime walk had to jump three times further than usual once
    # per lap, at a point where the road turns back on itself. Appending the
    # first point makes the loop closed before the arc lengths are accumulated,
    # so the seam is one step like all the others.
    # No trailing point is dropped: with the loop closed, the `count` samples are
    # already spaced `total / count` apart *including* the seam, which is what
    # makes the last span back to the first an ordinary step.
    out = resample_uniform(pts + [pts[0]], count)

    # Snap sequentially, each point bounded by the previous one, so a point cut
    # off at a corner can only move as far as the walk itself would. Snapping
    # independently let Alpine Serpent's steps grow from 64-128 units to 286 and
    # 404 -- see `snap_to_road`.
    raw_steps = [math.dist(out[i][:2], out[i + 1][:2]) for i in range(len(out) - 1)]
    # A generous multiple of the walk's own step. The bound exists to stop a
    # point snapping onto a *different* part of the circuit -- on a serpentine the
    # road passes within a couple of cells of itself, so "nearest racing surface"
    # is sometimes on the far side of the track. It must not be so tight that
    # ordinary corner-cutting cannot be corrected at all: measured, a bound of one
    # step left Chicane Park's centreline 25 units off the road at a hairpin, and
    # the car left the racing surface there on every lap.
    step_limit = (sorted(raw_steps)[len(raw_steps) // 2] * 3.0) if raw_steps else 6.0
    snapped = [snap_to_road(grid, out[0][0], out[0][1])]
    for i in range(1, len(out) - 1):
        snapped.append(
            snap_to_road(grid, out[i][0], out[i][1],
                         prev=snapped[-1], limit=step_limit)
        )

    # Close the seam last, and by moving only the *last* point.
    #
    # The first and last control points of a closed loop are neighbours, so the
    # span between them must be one step like every other. Sequential snapping
    # cannot guarantee that: each end drifts to its own nearest racing-surface
    # cell, and on a serpentine those are on different sides of the circuit.
    # Measured: the seam grew from 2.9 cells to 9.1 on Alpine Serpent against a
    # 2.8-cell median step, and the runtime walk then aimed across the map once
    # per lap.
    #
    # Moving the first point instead would be wrong: it is the start position,
    # and `start_heading` is taken from the first two control points, so
    # displacing it both moves the grid and rotates the field. Measured: doing so
    # left steps of 286 and 404 units where the walk's own is ~190.
    #
    # So the last point is bounded by *both* its neighbours -- the point before
    # it, so the span stays a step, and the first point, so the loop stays shut.
    last = snap_to_road(grid, out[-1][0], out[-1][1],
                        prev=snapped[-2], limit=step_limit,
                        also_near=snapped[0], also_limit=step_limit)
    if last is None:
        last = snapped[-2]
    snapped.append(last)
    return rotate_to_straightest(snapped)


def rotate_to_straightest(points):
    """Rotates a closed route so `points[0]` is its straightest point.

    A closed loop has no natural start, and the one it arrives with -- the first
    centreline sample -- is wherever the walker happened to begin, which is
    wherever the *drawing* began. On a technical circuit that can be the apex of a
    hairpin, and an apex is the worst possible place to put three things at once:
    the start pose, the start/finish gate, and the seam.

    Measured on The Esses: the route passed through (83,53), (81,55), (77,55),
    (73,54) and then turned south. That last turn sits exactly on the gate line,
    so the emitted start/finish was a one-cell-thick strip at y=54 spanning
    x=68..78 while the route crossed y=54 three times -- once at x=82, well
    outside it. Every lap the car crossed the line where the road was, missed the
    gate, and never closed a lap.

    Rotating to the straightest point fixes all three at once: the car starts
    facing down a straight, the seam is a straight span, and the gate is a
    transverse line the route crosses once. The rotation is cyclic, so the
    direction of travel and every span are preserved.
    """
    n = len(points)
    if n < 5:
        return points
    best = 0
    best_turn = None
    for i in range(n):
        prev = points[(i - 1) % n]
        cur = points[i]
        nxt = points[(i + 1) % n]
        ax, ay = cur[0] - prev[0], cur[1] - prev[1]
        bx, by = nxt[0] - cur[0], nxt[1] - cur[1]
        alen = math.hypot(ax, ay)
        blen = math.hypot(bx, by)
        if alen == 0 or blen == 0:
            turn = math.pi
        else:
            # cos of the turn angle: 1 is dead straight, -1 is a hairpin.
            cos = (ax * bx + ay * by) / (alen * blen)
            turn = 1.0 - max(-1.0, min(1.0, cos))
        # Ties broken towards the earliest point, so the output is deterministic.
        if best_turn is None or turn < best_turn - 1e-12:
            best_turn = turn
            best = i
    if best == 0:
        return points
    return points[best:] + points[:best]


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

    # The centreline arrives in authoring cells and the grid is `SURF` times
    # finer, so it is scaled once here. Everything below -- the route control
    # points, the gates, the start pose -- is in grid cells from this point on,
    # which is the resolution the game indexes with.
    centre = [(cx * SURF, cy * SURF) for (cx, cy) in centre]

    out.append(f"pub static {ident}: TrackDef = TrackDef {{")
    out.append(f'    name: "{name.title().replace("_", " ")}",')
    out.append(f"    width: {cw},")
    out.append(f"    height: {ch},")

    # The emitted centreline, computed up front: the start pose and the gates
    # must both agree with the route the car actually drives.
    route_pts = [p[:2] for p in route_control_points(grid, centre)]

    # Start: the first centreline control point, in world units (tile centre).
    sx, sy = route_pts[0]
    out.append(f"    start_pos: Vec2 {{ x: Fixed({tile_centre(sx) * 4096}), "
               f"y: Fixed({tile_centre(sy) * 4096}) }},")
    nx, ny = route_pts[1]
    out.append(f"    start_heading: {heading_bams(sx, sy, nx, ny)},")

    b, sv, g, dv = par  # bronze, silver, gold, dev -- dev is fastest
    out.append("    par_times: ParTimes {")
    out.append(f"        bronze_ticks: {b},")
    out.append(f"        silver_ticks: {sv},")
    out.append(f"        gold_ticks: {g},")
    out.append(f"        dev_platinum_ticks: {dv},")
    out.append("    },")

    # Gates are picked from the *emitted* centreline, not the raw walk.
    #
    # The route is resampled to 96 equal-arc-length control points
    # (`route_control_points` below), so its sample positions do not line up with
    # `centre`'s. Indexing gates into `centre` therefore places them wherever
    # that index happened to fall in the *original* parameterisation, which can
    # be a tile the emitted line never passes through: measured across the 24
    # circuits, 21 gates sat off the emitted centreline, up to 143 units (2
    # tiles) away. A gate the racing line does not touch can never be scored, so
    # the car drives a whole lap past it and the lap does not count.
    def heading_at(i):
        """Centreline heading in cells at control point `i`, from its neighbours."""
        n = len(route_pts)
        a = route_pts[(i - 1) % n]
        b = route_pts[(i + 1) % n]
        return int(b[0]) - int(a[0]), int(b[1]) - int(a[1])

    # Gate rectangles, each spanning the road across the direction of travel.
    # Consecutive gates must not touch: `TrackDef::min_route_length` measures the
    # circuit by summing the gaps between gate rectangles, so an overlap reports
    # a zero-length circuit and collapses the lap-time floor with it. Growing the
    # span "while the tiles stay road" is what caused the overlaps -- at a corner
    # that runs the full length of the straight -- so a gate that would touch its
    # neighbour collapses to the single centreline cell.
    boxes = []
    for k in range(1, checkpoints + 1):
        i = int(len(route_pts) * k / (checkpoints + 1)) % len(route_pts)
        cx, cy = int(route_pts[i][0]), int(route_pts[i][1])
        gdx, gdy = heading_at(i)
        boxes.append((i, gate_box(grid, cx, cy, gdx, gdy)))

    start_box = gate_box(grid, int(sx), int(sy),
                         int(nx) - int(sx), int(ny) - int(sy))

    out.append("    start_gate: CheckpointGate {")
    out.append(f"        x: {start_box[0]},")
    out.append(f"        y: {start_box[1]},")
    out.append(f"        width: {start_box[2]},")
    out.append(f"        height: {start_box[3]},")
    out.append("    },")

    # Trim an over-wide gate rather than dropping it to 1x1. A gate that touches
    # its neighbour is only *just* too wide, and collapsing it to a single cell
    # puts back exactly the bug the span exists to fix: measured before this,
    # 30 of the 24 circuits' gates ended up 1x1 and rivals drove past them.
    # Shrinking from the far end keeps the gate on the racing surface, keeps
    # consecutive gates disjoint, and still spans most of the road.
    for slot, (i, box) in enumerate(boxes):
        cx, cy = int(route_pts[i][0]), int(route_pts[i][1])
        box = trim_to_road(grid, box, cx, cy)
        boxes[slot] = (i, box)

    # Now narrow for disjointness. Road-trimming first, then disjointness, in
    # that order: `trim_to_road` keeps the contiguous run of tarmac under the
    # gate, and shrinking a *road* rectangle can only make it safer, whereas
    # disjointness-shrinking first is what used to leave gates straddling the
    # road's edge (Foundry Spiral's checkpoint 0 spanned (29,38)-(37,38) with a
    # gap at (31,38); Longbow's checkpoint 4 spanned (14,15)-(14,20) with three
    # gaps).
    #
    # Two passes, because narrowing one gate can bring it into its neighbour.
    for _ in range(2):
        for slot in range(len(boxes)):
            i, box = boxes[slot]
            prev_box = boxes[slot - 1][1] if slot > 0 else start_box
            next_box = boxes[slot + 1][1] if slot + 1 < len(boxes) else start_box
            if gate_is_disjoint(box, prev_box) and gate_is_disjoint(box, next_box):
                continue
            trimmed = shrink_until_disjoint(box, prev_box, next_box)
            if trimmed is None:
                cx, cy = int(route_pts[i][0]), int(route_pts[i][1])
                trimmed = (cx, cy, 1, 1)
            cx, cy = int(route_pts[i][0]), int(route_pts[i][1])
            boxes[slot] = (i, trim_to_road(grid, trimmed, cx, cy))

    # Same disjointness rule for the start/finish against its two neighbours: the
    # last checkpoint and the first.
    for _ in range(2):
        if not boxes:
            break
        if not gate_is_disjoint(start_box, boxes[0][1]):
            start_box = (int(sx), int(sy), 1, 1)
        if not gate_is_disjoint(start_box, boxes[-1][1]):
            start_box = (int(sx), int(sy), 1, 1)
        for slot in range(len(boxes)):
            i, box = boxes[slot]
            prev_box = boxes[slot - 1][1] if slot > 0 else start_box
            next_box = boxes[slot + 1][1] if slot + 1 < len(boxes) else start_box
            if gate_is_disjoint(box, prev_box) and gate_is_disjoint(box, next_box):
                continue
            trimmed = shrink_until_disjoint(box, prev_box, next_box)
            if trimmed is None:
                trimmed = (int(route_pts[i][0]), int(route_pts[i][1]), 1, 1)
            cx, cy = int(route_pts[i][0]), int(route_pts[i][1])
            boxes[slot] = (i, trim_to_road(grid, trimmed, cx, cy))
    sgx, sgy, sgw, sgh = start_box

    out.append(f"    checkpoint_count: {len(boxes)},")
    out.append("    checkpoints: [")
    for (_, (gx0, gy0, gw, gh)) in boxes:
        out.append("        CheckpointGate {")
        out.append(f"            x: {gx0},")
        out.append(f"            y: {gy0},")
        out.append(f"            width: {gw},")
        out.append(f"            height: {gh},")
        out.append("        },")
    while len(boxes) < MAX_CHECKPOINTS:
        boxes.append(None)
        out.append("        EMPTY_GATE,")
    out.append("    ],")

    out.append(f"    tiles: &{ident}_TILES,")
    out.append("    route: &[")
    # The runtime samples the spline 8 times per span, so at most
    # `MAX_ROUTE_SAMPLES / 8` control points fit. Resampled to equal arc length
    # rather than decimated by index -- see `resample_uniform`.
    # The walker closes the polyline by repeating its first point. Carrying that
    # repeat into the route puts control point 95 exactly on control point 0,
    # giving a zero-length span -- and Catmull-Rom takes its tangent at a control
    # point from `(next - prev) / 2`, so one degenerate neighbour throws the
    # tangent at the seam. That is what put route nodes off-road and made the
    # hinted and full arc scans disagree. The route is closed by the runtime, so
    # it wants 96 *distinct* control points.
    pts = route_control_points(grid, centre)
    for (cx, cy) in pts:
        out.append("        Vec2 {")
        out.append(f"            x: Fixed({tile_centre(cx) * 4096}),")
        out.append(f"            y: Fixed({tile_centre(cy) * 4096}),")
        out.append("        },")
    out.append("    ],")
    out.append(f"    half_width: {int(half_width_cells * AUTHORING_CELL)},")
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
