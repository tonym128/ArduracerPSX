#!/usr/bin/env python3
"""
Generate a circuit from an **SVG** description, for the colour-image level
format described in `LEVEL-FORMAT.md`.

Why SVG rather than the parametric walker in `generate_circuit_images.py`: a
circuit is a *drawing*. Asking for one as a shape description -- corners,
radii, turn angles -- means the result is whatever the description evaluates to,
and the two failure modes are both invisible in the output. A corner sum that
does not close produces a spiral that *looks* like a lap; a radius that is too
large for the edge it sits on produces a cusp. An SVG cannot silently do
either, because the path is the geometry: what you read is what is drawn.

The build is then strictly one-directional and inspectable at every stage:

    <name>.svg            authored by hand or by this script, in cell units
      -> raster            cairosvg, anti-aliasing resolved by majority vote
      -> <name>.data.png   flat 4-bit-indexed palette, exact colours only
      -> <name>.visual.png appearance, display only
      -> <name>.line.json  centreline sidecar (see below)
      -> build_atlas.py    -> crates/arduracer-core/src/levels.rs

**Anti-aliasing is the trap here.** Every rasteriser blends edges, and a single
blended pixel makes a data cell unreadable -- `compile_circuit.validate` rejects
the whole circuit with "matches no palette code". So the raster is taken at
`SUPERSAMPLE` times the cell resolution and each cell is resolved by majority
vote over its own pixels. That is what makes the flat data image legal without
post-hoc colour snapping, which would be a lie about the drawing.

The centreline sidecar is written because we can compute it *exactly* from the
shape rather than skeletonising the raster: the road is a stroke on a path we
generated, so its centreline is the path. A skeleton would be a lossy
reconstruction of something already known.

Usage:
    python3 tools/track_cook/generate_svg_circuit.py
    python3 tools/track_cook/generate_svg_circuit.py --only Hells_Bells
"""
from __future__ import annotations

import argparse
import json
import math
import os
from collections import Counter

import numpy as np
from PIL import Image

# --- Grid ---------------------------------------------------------------------

#: Cells per side. Matches `MAX_TRACK_DIM`, and the cell is the atom of both
#: physics and authoring: one cell is one `TrackTile` byte and one square of the
#: data image.
#:
#: 80 cells of 32 world units is a 2,560-unit world, down from 96 cells / 3,072
#: units. The grid was shrunk for the reason the player gave: the stages felt
#: like too much space. Almost all of that space was *grid* rather than circuit.
#: An oval that fills the grid still leaves the whole grid addressable, and the
#: camera clamp, the minimap bake and the world texture all scale with it, so a
#: 96-cell world spends a quarter of a megabyte of RAM and 288 KB of VRAM
#: rendering grass.
#:
#: 80 cells of 32 world units is a 2,560-unit world, down from 96 cells / 3,072
#: units, and 80 is the smallest grid that still hosts the *largest* circuit this
#: design wants. Two constraints pull against each other:
#:
#: * **The grid is the world, and the world is the space.** A 96-cell grid is a
#:   3,072-unit world whose only content was grass beyond the circuit. The player
#:   complained of drowning in space, and most of that space was *grid* rather
#:   than track: the camera clamp, the minimap bake and the world texture all
#:   scale with it, so 96 cells spends a quarter of a megabyte of RAM and 288 KB
#:   of VRAM rendering grass. The density that matters is circuit-over-grid, and
#:   it goes from 46% (a 20/16 stadium in 96 cells) to 78% (13/18 in 80).
#: * **The largest lap has to fit.** A lap is `4 * straight_half + 2 * pi *
#:   radius` cells for a stadium, or `4 * (half_x + half_y) - 8 * radius +
#:   2 * pi * radius` for a rounded rectangle, and both need room inside
#:   `GRID/2`. Only the *road* has to clear the border -- that is what
#:   `compile_circuit.validate` checks -- so the bound is
#:   `half_x + radius <= GRID/2 - 1 - ROAD_HALF_CELLS`, which at 80 cells is 36,
#:   and `CIRCUITS` below spends nearly all of it: the largest is 29/17/7.
#:
#: One grid step smaller and the biggest lap lands nearer 15 seconds, which is not
#: a different circuit, it is the same circuit with the ends cut off.
GRID = 80

#: World units per cell. Mirrors `TILE_SIZE` in `arduracer_core::track`; asserted
#: against the compiled circuit rather than trusted, because the two drifting
#: apart would silently halve the width of every road.
WORLD_PER_CELL = 32

#: Raster resolution per cell for the *data* image. 3 is the smallest that
#: survives majority-vote resolution of the kerb fringe, which is ~0.7 cells.
DATA_PX = 3

#: Raster resolution per cell for the *visual* image. Painted, so it wants room.
#:
#: 4, not 8, and the reason is RAM rather than looks. `build_atlas` bakes **one**
#: 4bpp world texture per circuit, and unlike the tile grids -- a few kilobytes,
#: with every `ALL_TRACKS` slot pointing at one shared static -- those cannot
#: share: four circuits are four different pictures, and the PSX static-RAM gate
#: has under 300 KB of headroom over the rest of the game. At 8 px/cell a
#: 80-cell world is a 640x640 texture = 205 KB, and four of those is 820 KB,
#: which does not fit. At 4 px/cell it is 320x320 = 51 KB each, 205 KB for all
#: four, and the gate stays green with room to spare.
#:
#: The visible cost is texel size: `32 / VISUAL_PX` = 8 world units per texel,
#: so at rest zoom (1 world unit = 1 screen pixel) a texel is an 8x8 block of
#: screen. The road is still 24 texels across and the kerb is nearly 3, so both
#: survive the resolution drop; what is lost is edge smoothness, on a PSX
#: palette that has sixteen flat colours to quantise to anyway.
#:
#: 320 also lands the image on a page grid that `gpu::tracktex` can address with
#: no fold: a 4bpp page is 256 texels, so 320 is one full page plus a 64-texel
#: Visual image pixel resolution: 1024x1024 master visual images for streaming.
SIZE = 1024
VISUAL_PX = SIZE // GRID

#: Raster is taken at this multiple of the data resolution, then majority-voted
#: down. 4x4 = 16 samples per cell is ample to resolve a 0.7-cell fringe; the
#: default 3 px/cell with no supersampling leaves edges ambiguous.
SUPERSAMPLE = 3

# --- Palette, mirroring tools/track_cook/palette.json exactly ------------------
#
# These values are the level format. `compile_circuit.load_data` matches data
# pixels by *exact* RGB, so drifting from palette.json is a silent corruption,
# not a compile error: every cell stops matching and validation reports the whole
# image as unknown.

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
    VOID: "#000000",
    TARMAC: "#3C3E44",
    TARMAC_WORN: "#4A4C54",
    KERB_WHITE: "#E8E8F0",
    KERB_RED: "#D8283C",
    GRASS: "#2E5A34",
    GRAVEL: "#6B5432",
    SAND: "#7A6090",
    OIL: "#8A1FB0",
    BOOST: "#FF8A10",
    START_LINE: "#F2F2F2",
    GATE: "#00D2FF",
    WALL: "#2A2E38",
    TUNNEL: "#5A4632",
    BRIDGE: "#3A5A7A",
    SCENERY: "#FF2E88",
}

RGB_TUPLE = {
    code: (int(hexv[1:3], 16), int(hexv[3:5], 16), int(hexv[5:7], 16))
    for code, hexv in RGB.items()
}

# --- Road cross-section, in cells ---------------------------------------------
#
# Half-widths, because the road is drawn as a *stroke* on the centreline and a
# stroke is centred.
#
# The road is specified in **world units**, not cells, and converted. A 2.5-cell
# road was 160 world units across when a cell was 64; halving the cell would have
# silently halved it to 80, which is less than three car lengths -- the circuit
# would have felt like a corridor. Anchoring to `half_width` and deriving cells
# from `WORLD_PER_CELL` is what stops the two definitions drifting apart when the
# cell size changes again. `build_atlas` asserts the compiled `half_width` still
# matches `ROAD_HALF_WORLD`.

#: Tarmac half-width, world units.
#:
#: 96, down from 160. This is the "drowning in the space" complaint's other half:
#: a 320-unit road is 20 car lengths across, wide enough that the car can lose
#: the racing line entirely without ever being off the tarmac, so a corner taken
#: badly is not punished and a corner taken well is not rewarded. 96 is 3 cells
#: at the current cell size -- wide enough for two cars to race side by side plus
#: a car's width of margin each side, which is the narrowest road that still
#: admits a pass, and no wider.
#:
#: The 3.0-cell figure is load-bearing in three separate places that all read
#: `ROAD_HALF_CELLS`: the stadium sizing arithmetic in `CIRCUITS`, the `RUNOFF`
#: and `WALL` bands stacked outside it, and `build_atlas`'s `half_width`, which
#: `test_game_logic::test_circuits_are_wide_and_have_long_straights` floors. It
#: has to stay a whole number of cells -- `build_atlas` asserts that, because a
#: fractional road half-width compiles to a road a half-cell narrower than the
#: one that was drawn and nothing else fails.
ROAD_HALF_WORLD = 96       # tarmac half-width, world units
KERB_CELLS = 0.7           # rumble band outside the tarmac
RUNOFF_CELLS = 2.4         # gravel band outside the kerb
WALL_CELLS = 1.0           # barrier band outside the runoff

ROAD_HALF_CELLS = ROAD_HALF_WORLD / WORLD_PER_CELL   # 3.0 at 32 units per cell

#: Everything between the centreline and the outside of the barrier, in cells.
#: The single number the `CIRCUITS` sizing has to fit inside the grid: a stadium
#: or rounded rectangle of half-extent `h` and corner radius `r` has its outer
#: barrier at `h + r + BORDER_CELLS`, and `compile_circuit.validate` rejects road
#: on the image border. Stated once here so the arithmetic in `CIRCUITS` cannot
#: silently disagree with the bands that are actually painted.
BORDER_CELLS = ROAD_HALF_CELLS + KERB_CELLS + RUNOFF_CELLS + WALL_CELLS  # 7.1

#: Infield surface, and the surface beyond the runoff. Both are scenery-adjacent:
#: `compile_circuit` treats anything not in its ROAD set as surroundings, which is
#: what lets a closed ring enclose an infield without it reading as a stray island.
INFIELD = GRASS
OUTSIDE = GRASS


# --- SVG document -------------------------------------------------------------


def svg_header(size: int) -> list[str]:
    return [
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{size}" height="{size}" '
        f'viewBox="0 0 {GRID} {GRID}" shape-rendering="geometricPrecision">',
    ]


def stadium_path(cx: float, cy: float, straight_half: float, radius: float) -> str:
    """A closed **stadium**: two parallel straights joined by two semicircles.

    Chosen as the default oval because it is *closed by construction* -- the path
    starts and ends on the same point and there is no closure condition to fail.
    The parametric route in `generate_circuit_images.py` spent four attempts
    discovering that a corner-angle description needs two independent conditions
    (turn sum +/-360 *and* displacement closing) and silently produces an open
    spiral when only the first holds. A stadium has neither failure mode.

    Returned as an SVG path, centred on `(cx, cy)`.
    """
    x0, x1 = cx - straight_half, cx + straight_half
    top, bot = cy - radius, cy + radius
    return (
        f"M {x0:.4f} {top:.4f} "
        f"L {x1:.4f} {top:.4f} "
        f"A {radius:.4f} {radius:.4f} 0 0 1 {x1:.4f} {bot:.4f} "
        f"L {x0:.4f} {bot:.4f} "
        f"A {radius:.4f} {radius:.4f} 0 0 1 {x0:.4f} {top:.4f} "
        f"Z"
    )


def rounded_rect_path(cx: float, cy: float, half_x: float, half_y: float,
                      radius: float) -> str:
    """A closed **rounded rectangle**: four straights joined by four quarter arcs.

    The second shape, and the reason the circuits are not four concentric ovals.
    A stadium has exactly two corners and both are 180 degrees, so every circuit
    built from one has the same four-corner silhouette however it is scaled; a
    driver learns "turn left, turn right" and never learns a shape. This has four
    distinct corners, and `half_x != half_y` gives genuinely unequal corners --
    the long-and-fast layout the largest circuit wants.

    Closed by construction for the same reason the stadium is: the four arcs sum
    to exactly 360 degrees of turn and the path returns to its start point, with
    no closure condition left over to get wrong. Perimeter is
    `4 * (half_x + half_y) - 8 * radius + 2 * pi * radius`.

    Returned as an SVG path, centred on `(cx, cy)`.
    """
    # The four straights are inset by `radius` at both ends so the corner arcs
    # have somewhere to go. This is not cosmetic: written as one arc from (x1, y0)
    # to (x1, y1) -- the obvious transcription -- SVG *scales the radius up* to
    # span the gap, because the radii are too small to connect those points. The
    # result is a semicircle of radius `half_y` bulging out past `x1` by
    # `half_y`, which runs the road off the side of the grid, and
    # `compile_circuit.validate` reports it as "road cell on the border" plus a
    # phantom second piece of road. Both errors, one bug, neither naming it.
    x0, x1 = cx - half_x, cx + half_x
    y0, y1 = cy - half_y, cy + half_y
    r = radius
    return (
        f"M {x0 + r:.4f} {y0:.4f} "
        f"L {x1 - r:.4f} {y0:.4f} "
        f"A {r:.4f} {r:.4f} 0 0 1 {x1:.4f} {y0 + r:.4f} "
        f"L {x1:.4f} {y1 - r:.4f} "
        f"A {r:.4f} {r:.4f} 0 0 1 {x1 - r:.4f} {y1:.4f} "
        f"L {x0 + r:.4f} {y1:.4f} "
        f"A {r:.4f} {r:.4f} 0 0 1 {x0:.4f} {y1 - r:.4f} "
        f"L {x0:.4f} {y0 + r:.4f} "
        f"A {r:.4f} {r:.4f} 0 0 1 {x0 + r:.4f} {y0:.4f} "
        f"Z"
    )


def rounded_rect_centreline(cx: float, cy: float, half_x: float, half_y: float,
                            radius: float,
                            step: float = 0.25) -> list[tuple[float, float]]:
    """Exact centreline of a rounded rectangle, sampled at ~`step` cells.

    Same argument as [`stadium_centreline`]: the road is a stroke on a path this
    module built, so the centreline *is* that path. Walking the same four-straights
    - four-arcs order the SVG path uses also means the centreline sample index and
    the arc-length position agree, which is what puts the boost pads and gates on
    the road instead of near it.
    """
    pts: list[tuple[float, float]] = []
    quarter = math.pi / 2 * radius

    def push(p):
        if not pts or math.dist(pts[-1], p) > 1e-9:
            pts.append(p)

    def run_line(ax: float, ay: float, bx: float, by: float):
        n = max(1, int(round(math.hypot(bx - ax, by - ay) / step)))
        for k in range(n):
            push((ax + (bx - ax) * k / n, ay + (by - ay) * k / n))

    def run_arc(ox: float, oy: float, a0: float):
        n = max(1, int(round(quarter / step)))
        for k in range(n):
            t = a0 + (math.pi / 2) * k / n
            push((ox + radius * math.cos(t), oy + radius * math.sin(t)))

    # Straights are inset by `radius` at both ends, matching `rounded_rect_path`.
    # They must be: the corner arc's endpoints *are* (x1 - r, y0) and (x1,
    # y0 + r), so a straight run out to x1 would leave the arc starting 7 cells
    # back up the same row.
    #
    # The failure is invisible in the drawn image -- the SVG road and its
    # centreline sidecar disagree, and only the sidecar is used for the racing
    # line -- and it is exactly the kind of disagreement that has no useful
    # error message. `Route::nearest` on the emitted centreline reports the arc
    # going *backwards* at that point (9 times per lap on Longbow), so
    # `test_route_arc_follows_the_car` fails with "arc wrapped 9 times in one
    # lap", which reads as a physics bug and is a drawing one. The control-point
    # dump shows the cause immediately: consecutive nodes on the same row
    # stepping backwards, `(64,29) -> (62,29)`.
    x0, x1 = cx - half_x, cx + half_x
    y0, y1 = cy - half_y, cy + half_y
    r = radius
    run_line(x0 + r, y0, x1 - r, y0)                  # top, left to right
    run_arc(x1 - r, y0 + r, -math.pi / 2)             # top-right
    run_line(x1, y0 + r, x1, y1 - r)                 # right, down
    run_arc(x1 - r, y1 - r, 0.0)                     # bottom-right
    run_line(x1 - r, y1, x0 + r, y1)                 # bottom, right to left
    run_arc(x0 + r, y1 - r, math.pi / 2)             # bottom-left
    run_line(x0, y1 - r, x0, y0 + r)                 # left, up
    run_arc(x0 + r, y0 + r, math.pi)                 # top-left
    return pts


def stadium_centreline(cx: float, cy: float, straight_half: float,
                       radius: float, step: float = 0.25) -> list[tuple[float, float]]:
    """Exact centreline of a stadium, sampled at ~`step` cells.

    Computed rather than skeletonised. The road is a stroke on a path we built,
    so its centreline *is* that path; a morphological skeleton would be a lossy
    reconstruction of geometry already known exactly, and it is exactly the kind
    of reconstruction that fails on tight corners and wide junctions.
    """
    pts: list[tuple[float, float]] = []
    arc = math.pi * radius

    def push(p):
        if not pts or math.dist(pts[-1], p) > 1e-9:
            pts.append(p)

    # Top straight, left to right.
    n = max(1, int(round(2 * straight_half / step)))
    for k in range(n):
        push((cx - straight_half + 2 * straight_half * k / n, cy - radius))
    # Right semicircle, top to bottom (clockwise).
    n = max(1, int(round(arc / step)))
    for k in range(n):
        t = -math.pi / 2 + math.pi * k / n
        push((cx + straight_half + radius * math.cos(t), cy + radius * math.sin(t)))
    # Bottom straight, right to left.
    n = max(1, int(round(2 * straight_half / step)))
    for k in range(n):
        push((cx + straight_half - 2 * straight_half * k / n, cy + radius))
    # Left semicircle, bottom to top.
    n = max(1, int(round(arc / step)))
    for k in range(n):
        t = math.pi / 2 + math.pi * k / n
        push((cx - straight_half + radius * math.cos(t), cy + radius * math.sin(t)))
    return pts


def transverse_band(cx: float, cy: float, at: tuple[float, float],
                    nxt: tuple[float, float], half_len: float, half_thick: float,
                    code: int) -> str:
    """A rectangle centred on `at`, squared up to the direction of travel.

    Start lines, gates and boost pads all cross the road at right angles, so they
    are drawn as a band whose long axis is the road's *normal*. Sizing it from the
    local tangent rather than assuming `x` or `y` is what keeps a gate on the road
    through a corner -- the alternative is a pad that slides off into the runoff on
    any curve, which still validates as "surrounded by drivable surface" only by
    luck.
    """
    dx, dy = nxt[0] - at[0], nxt[1] - at[1]
    m = math.hypot(dx, dy) or 1.0
    tx, ty = dx / m, dy / m
    nx, ny = -ty, tx
    corners = []
    for a, b in ((-1, -1), (1, -1), (1, 1), (-1, 1)):
        corners.append((at[0] + nx * half_len * a + tx * half_thick * b,
                        at[1] + ny * half_len * a + ty * half_thick * b))
    pts = " ".join(f"{x:.4f},{y:.4f}" for x, y in corners)
    return (f'<polygon points="{pts}" fill="{RGB[code]}" '
            f'stroke="none"/>')


def poly_path_and_samples(vertices: list[tuple[float, float]],
                          radii: list[float],
                          step: float = 0.25) -> tuple[str, list[tuple[float, float]]]:
    """Generates an SVG path and densely sampled centreline for a polygon with filleted corners.

    Filleting fillets each corner with a circular arc of specified radius.
    """
    n = len(vertices)
    edges = []
    lengths = []
    for i in range(n):
        p1 = vertices[i]
        p2 = vertices[(i + 1) % n]
        dx, dy = p2[0] - p1[0], p2[1] - p1[1]
        L = math.hypot(dx, dy)
        assert L > 1e-6, f"Duplicate vertex {i}: {p1}"
        edges.append((dx / L, dy / L))
        lengths.append(L)

    fillets = []
    for i in range(n):
        p = vertices[i]
        u_in = edges[(i - 1) % n]
        u_out = edges[i]
        cross = u_in[0] * u_out[1] - u_in[1] * u_out[0]
        dot = u_in[0] * u_out[0] + u_in[1] * u_out[1]
        theta = math.atan2(cross, dot)
        r = radii[i]
        tan_half = math.tan(abs(theta) / 2.0)
        t_dist = r * tan_half
        max_t = min(lengths[(i - 1) % n], lengths[i]) * 0.48
        if t_dist > max_t:
            t_dist = max_t
            r = t_dist / max(1e-4, tan_half)

        p_start = (p[0] - u_in[0] * t_dist, p[1] - u_in[1] * t_dist)
        p_end = (p[0] + u_out[0] * t_dist, p[1] + u_out[1] * t_dist)

        if cross > 0:
            n_in = (-u_in[1], u_in[0])
            sweep_flag = 1
        else:
            n_in = (u_in[1], -u_in[0])
            sweep_flag = 0

        c_arc = (p_start[0] + n_in[0] * r, p_start[1] + n_in[1] * r)
        ang_start = math.atan2(p_start[1] - c_arc[1], p_start[0] - c_arc[0])
        ang_end = math.atan2(p_end[1] - c_arc[1], p_end[0] - c_arc[0])

        fillets.append({
            "p_start": p_start, "p_end": p_end,
            "r": r, "sweep_flag": sweep_flag,
            "c_arc": c_arc, "ang_start": ang_start, "ang_end": ang_end,
            "cross": cross, "abs_theta": abs(theta),
        })

    path_cmds = []
    samples: list[tuple[float, float]] = []

    def push(pt: tuple[float, float]) -> None:
        if not samples or math.hypot(samples[-1][0] - pt[0], samples[-1][1] - pt[1]) > 1e-5:
            samples.append(pt)

    for i in range(n):
        f_curr = fillets[i]
        f_next = fillets[(i + 1) % n]
        if i == 0:
            path_cmds.append(f"M {f_curr['p_start'][0]:.4f} {f_curr['p_start'][1]:.4f}")

        r = f_curr["r"]
        p_end = f_curr["p_end"]
        sw = f_curr["sweep_flag"]
        path_cmds.append(f"A {r:.4f} {r:.4f} 0 0 {sw} {p_end[0]:.4f} {p_end[1]:.4f}")

        c = f_curr["c_arc"]
        a_start = f_curr["ang_start"]
        if sw == 1:
            d_ang = (f_curr["ang_end"] - a_start) % (2 * math.pi)
        else:
            d_ang = -((a_start - f_curr["ang_end"]) % (2 * math.pi))
        arc_len = abs(d_ang) * r
        n_arc = max(1, int(round(arc_len / step)))
        for k in range(n_arc):
            ang = a_start + d_ang * (k / n_arc)
            push((c[0] + r * math.cos(ang), c[1] + r * math.sin(ang)))

        p_next_start = f_next["p_start"]
        path_cmds.append(f"L {p_next_start[0]:.4f} {p_next_start[1]:.4f}")

        seg_len = math.hypot(p_next_start[0] - p_end[0], p_next_start[1] - p_end[1])
        n_seg = max(1, int(round(seg_len / step)))
        for k in range(n_seg):
            t = k / n_seg
            push((p_end[0] + (p_next_start[0] - p_end[0]) * t,
                  p_end[1] + (p_next_start[1] - p_end[1]) * t))

    path_cmds.append("Z")
    svg_d = " ".join(path_cmds)
    return svg_d, samples


def shape_path(spec: dict, cx: float, cy: float) -> str:
    """The centreline of `spec` as an SVG path, for whichever shape it declares.

    Dispatch lives here so `build_svg` never branches on the shape and the
    comment block explaining the paint order stays readable. A spec with an
    unknown `shape` raises rather than defaulting: a silent fallback would paint
    one circuit's image under another circuit's name, and the compiler would
    happily emit it.
    """
    shape = spec.get("shape", "stadium")
    if shape == "stadium":
        return stadium_path(cx, cy, spec["straight_half"], spec["radius"])
    if shape == "rounded_rect":
        return rounded_rect_path(cx, cy, spec["half_x"], spec["half_y"],
                                 spec["radius"])
    if shape == "polygon":
        path_d, _ = poly_path_and_samples(spec["vertices"], spec["radii"])
        return path_d
    raise ValueError(f'{spec["name"]}: unknown shape {shape!r}')


def shape_centreline(spec: dict, cx: float, cy: float) -> list[tuple[float, float]]:
    """The centreline of `spec`, sampled, for whichever shape it declares.

    Kept beside [`shape_path`] because the two must walk the same geometry in the
    same direction: the marks are placed by *index into this list*, so a
    centreline that runs the other way round puts the start line halfway round the
    lap and every boost pad in the runoff.
    """
    shape = spec.get("shape", "stadium")
    if shape == "stadium":
        return stadium_centreline(cx, cy, spec["straight_half"], spec["radius"])
    if shape == "rounded_rect":
        return rounded_rect_centreline(cx, cy, spec["half_x"], spec["half_y"],
                                       spec["radius"])
    if shape == "polygon":
        _, samples = poly_path_and_samples(spec["vertices"], spec["radii"])
        return samples
    raise ValueError(f'{spec["name"]}: unknown shape {shape!r}')


def shape_note(spec: dict) -> str:
    """One-line human-readable description of a spec's geometry, for the SVG."""
    shape = spec.get("shape", "stadium")
    if shape == "stadium":
        return (f'centreline: stadium, straight_half={spec["straight_half"]}, '
                f'radius={spec["radius"]}')
    if shape == "rounded_rect":
        return (f'centreline: rounded rect, half_x={spec["half_x"]}, '
                f'half_y={spec["half_y"]}, radius={spec["radius"]}')
    if shape == "polygon":
        return (f'centreline: polygon, {len(spec["vertices"])} vertices, '
                f'exciting corners, chicanes & shortcuts')
    raise ValueError(f'{spec["name"]}: unknown shape {shape!r}')


def build_svg(spec: dict) -> tuple[str, list[tuple[float, float]]]:
    """Renders one circuit spec to an SVG document, and returns its centreline.

    Drawn back to front: ground, runoff, kerb, road, racing line, markings. The
    order is the whole trick -- SVG has no z-buffer beyond document order, so a
    kerb painted before the road is simply painted over.
    """
    cx = cy = GRID / 2.0
    centre = shape_centreline(spec, cx, cy)
    path = shape_path(spec, cx, cy)

    road_hw = ROAD_HALF_CELLS
    kerb_hw = road_hw + KERB_CELLS
    runoff_hw = kerb_hw + RUNOFF_CELLS
    wall_hw = runoff_hw + WALL_CELLS

    out = svg_header(SIZE)
    out.append(f'<!-- {spec["name"]}: authored circuit, cell units. -->')
    out.append(f'<!-- {shape_note(spec)} -->')

    # Ground. Everything outside the circuit is grass; the infield is grass too,
    # which is what gives the ring its surroundings.
    out.append(f'<rect x="0" y="0" width="{GRID}" height="{GRID}" '
               f'fill="{RGB[OUTSIDE]}"/>')

    # Barrier, then runoff over its inner half, so the barrier reads as a rim.
    out.append(f'<path d="{path}" fill="none" stroke="{RGB[WALL]}" '
               f'stroke-width="{2 * wall_hw:.4f}" stroke-linecap="butt"/>')
    for sc_p in spec.get("shortcuts", []):
        out.append(f'<path d="{sc_p}" fill="none" stroke="{RGB[WALL]}" '
                   f'stroke-width="{2 * wall_hw:.4f}" stroke-linecap="round" stroke-linejoin="round"/>')

    runoff_code = int(spec["runoff"])
    out.append(f'<path d="{path}" fill="none" stroke="{RGB[runoff_code]}" '
               f'stroke-width="{2 * runoff_hw:.4f}" stroke-linecap="butt"/>')
    for sc_p in spec.get("shortcuts", []):
        out.append(f'<path d="{sc_p}" fill="none" stroke="{RGB[runoff_code]}" '
                   f'stroke-width="{2 * runoff_hw:.4f}" stroke-linecap="round" stroke-linejoin="round"/>')

    # Kerb as two dashes of opposite phase.
    dash = 2.0
    out.append(f'<path d="{path}" fill="none" stroke="{RGB[KERB_WHITE]}" '
               f'stroke-width="{2 * kerb_hw:.4f}" stroke-linecap="butt"/>')
    for sc_p in spec.get("shortcuts", []):
        out.append(f'<path d="{sc_p}" fill="none" stroke="{RGB[KERB_WHITE]}" '
                   f'stroke-width="{2 * kerb_hw:.4f}" stroke-linecap="round" stroke-linejoin="round"/>')

    out.append(
        f'<path d="{path}" fill="none" stroke="{RGB[KERB_RED]}" '
        f'stroke-width="{2 * kerb_hw:.4f}" stroke-linecap="butt" '
        f'stroke-dasharray="{dash:.2f} {dash:.2f}" '
        f'stroke-dashoffset="0.00"/>'
    )

    # Road tarmac for main path and any shortcuts.
    out.append(f'<path d="{path}" fill="none" stroke="{RGB[TARMAC]}" '
               f'stroke-width="{2 * road_hw:.4f}" stroke-linecap="butt"/>')
    for sc_p in spec.get("shortcuts", []):
        out.append(f'<path d="{sc_p}" fill="none" stroke="{RGB[TARMAC]}" '
                   f'stroke-width="{2 * road_hw:.4f}" stroke-linecap="round" stroke-linejoin="round"/>')

    # No centre racing stripe. `TARMAC_WORN` differs from `TARMAC` by one 5-bit
    # step per channel, and the road is only ~2.5 cells wide, so a centre band is
    # sub-cell: majority-vote resolution turns it into a stripe of single cells
    # that appears and vanishes with the rasteriser's sub-pixel phase. It reads as
    # noise, not as a racing line. Both compile to `TrackTile::Tarmac`, so there
    # is nothing to gain in physics either -- the visual image is the only place
    # it could ever show, and there it does not survive the cell grid.

    # Markings, placed at fractions of the lap.
    n = len(centre)
    for frac, code, half_len, half_thick in spec["marks"]:
        i = int(n * frac) % n
        out.append(transverse_band(cx, cy, centre[i], centre[(i + 1) % n],
                                   half_len, half_thick, code))

    out.append("</svg>")
    return "\n".join(out), centre


# --- Rasterisation ------------------------------------------------------------


def rasterise(svg_text: str, px_per_cell: int) -> np.ndarray:
    """SVG -> HxWx3 uint8, at an exact multiple of the grid."""
    import io

    import cairosvg  # imported lazily: only the generator needs it

    size = GRID * px_per_cell
    png = cairosvg.svg2png(bytestring=svg_text.encode("utf-8"),
                           output_width=size, output_height=size,
                           background_color="black")
    assert png is not None, "cairosvg returned no image"
    return np.array(Image.open(io.BytesIO(png)).convert("RGB"))


def snap_to_codes(rgb: np.ndarray) -> np.ndarray:
    """Nearest-palette-code per pixel.

    A *distance* snap, not an equality match, because the raster is anti-aliased:
    a pixel on a tarmac/grass boundary is a blend of both and matches neither.
    It must land somewhere for the majority vote to have anything to vote on.
    """
    codes = np.array(list(RGB_TUPLE.values()), dtype=np.int16)   # (16, 3)
    d = np.abs(rgb[..., None, :].astype(np.int16) - codes[None, None, :, :])
    # Weighted to approximate perceptual distance; green matters most to the eye,
    # and a flat RGB metric overweights blue on dark surfaces.
    w = np.array([3, 6, 1], dtype=np.int32)
    cost = (d.astype(np.int32) * w).sum(axis=-1)
    return cost.argmin(axis=-1).astype(np.int8)


def resolve_cells(rgb: np.ndarray, px_per_cell: int) -> np.ndarray:
    """Majority vote per cell -> GRID x GRID code map.

    This is the step that makes a flat data image legal. Supersampling and
    majority vote together mean an edge pixel is outvoted by its own cell's
    interior, so the data image needs no colour snapping and the result matches
    what the drawing actually says.
    """
    codes = snap_to_codes(rgb)
    n = codes.shape[0] // px_per_cell
    blocks = codes[:n * px_per_cell, :n * px_per_cell]
    blocks = blocks.reshape(n, px_per_cell, n, px_per_cell).transpose(0, 2, 1, 3)
    out = np.zeros((n, n), dtype=np.uint8)
    for y in range(n):
        for x in range(n):
            out[y, x] = Counter(blocks[y, x].ravel().tolist()).most_common(1)[0][0]
    return out


# --- Output -------------------------------------------------------------------


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


def paint_visual(codes: np.ndarray, size: int = SIZE) -> np.ndarray:
    """Paints the appearance image from the code map at size x size (1024x1024). Display only.

    Deliberately does *not* read the SVG raster. The visual image is what the
    player sees and the data map is what the car drives; keeping them separate is
    the whole point of the two-image format, and sharing one would let a pretty
    texture quietly change collision.
    """
    grid_h, grid_w = codes.shape
    vis_img = Image.fromarray(codes.astype(np.uint8)).resize((size, size), Image.Resampling.NEAREST)
    vis = np.array(vis_img)
    h, w = vis.shape
    out = np.zeros((h, w, 3), dtype=np.uint8)

    def base(code):
        return np.array(RGB_TUPLE[code], dtype=np.float32)

    def grain(y, x, salt=0):
        scale = max(1, size // grid_w // 2)
        gx = x // scale
        gy = y // scale
        v = ((gx * 73856093) ^ (gy * 19349663) ^ (salt * 83492791)) & 0xFF
        return (v & 0x3F) / 255.0

    for y in range(h):
        for x in range(w):
            c = int(vis[y, x])
            n = grain(y, x)
            n2 = grain(y // 4, x // 4, salt=7)
            b = base(c)
            if c in (TARMAC, TARMAC_WORN):
                out[y, x] = np.clip(b * (1.0 - 0.10 - n * 0.10 - n2 * 0.06), 0, 255)
            elif c in (KERB_WHITE, KERB_RED):
                blk = ((x * grid_w // size) + (y * grid_h // size)) % 2
                b = base(KERB_WHITE if blk == 0 else KERB_RED)
                out[y, x] = np.clip(b * (1.0 - n * 0.12), 0, 255)
            elif c in (GRASS,):
                out[y, x] = np.clip(b * (1.0 - 0.14 - n * 0.22 - n2 * 0.10), 0, 255)
            elif c in (GRAVEL, SAND, SCENERY):
                out[y, x] = np.clip(b * (1.0 - 0.14 - n * 0.24), 0, 255)
            elif c == WALL:
                edge = any(
                    0 <= y + dy < h and 0 <= x + dx < w
                    and int(vis[y + dy, x + dx]) in
                    (TARMAC, TARMAC_WORN, KERB_WHITE, KERB_RED, BOOST, GATE,
                     START_LINE, TUNNEL, BRIDGE)
                    for dy, dx in ((-1, 0), (1, 0), (0, -1), (0, 1))
                )
                lit = 0.22 if edge else n * 0.08
                out[y, x] = np.clip(b + (255 - b) * lit, 0, 255)
            elif c == START_LINE:
                cell_x = x * grid_w // size
                cell_y = y * grid_h // size
                out[y, x] = ((245, 245, 245)
                             if (cell_x + cell_y) % 2 else (28, 28, 32))
            elif c == BOOST:
                cell_x = x * grid_w // size
                cell_y = y * grid_h // size
                band = (cell_y + cell_x) % 2
                out[y, x] = (255, 240, 200) if band else np.clip(b * (1 - n * 0.1), 0, 255)
            elif c == OIL:
                t = 0.5 + 0.5 * ((x * 0.3 + y * 0.2) % 6) / 6.0
                out[y, x] = (18 + 42 * t, 14 + 16 * t, 26 + 54 * t)
            elif c == GATE:
                out[y, x] = np.clip(b + (255 - b) * (0.15 + n * 0.15), 0, 255)
            elif c == VOID:
                out[y, x] = (0, 0, 0)
            else:
                out[y, x] = np.clip(b * (1.0 - n * 0.10), 0, 255)
    return out.astype(np.uint8)


def write_outputs(spec: dict, outdir: str) -> dict:
    svg_text, centre = build_svg(spec)
    stem = os.path.join(outdir, spec["name"].replace(" ", "_"))

    with open(stem + ".svg", "w") as f:
        f.write(svg_text)

    # Supersample, then majority-vote per cell.
    fine = rasterise(svg_text, DATA_PX * SUPERSAMPLE)
    codes = resolve_cells(fine, DATA_PX * SUPERSAMPLE)

    # Data image: flat exact palette colours, DATA_PX px per cell.
    dat = np.repeat(np.repeat(codes, DATA_PX, axis=0), DATA_PX, axis=1)
    # Index by code through a lookup table rather than a boolean mask per code:
    # the data image must be *exactly* palette colours, and building it from the
    # code map by table guarantees that rather than hoping the raster was exact.
    lut = np.zeros((16, 3), dtype=np.uint8)
    for code, rgb in RGB_TUPLE.items():
        lut[code] = rgb
    flat = lut[dat]
    Image.fromarray(flat, "RGB").save(stem + ".data.png")

    # Visual image: painted from the code map, 1024x1024.
    vis = paint_visual(codes, SIZE)
    vis_path = stem + ".visual.png"
    gen_path = stem + ".visual.gen.png"
    Image.fromarray(vis, "RGB").save(vis_path)

    # Compress the current visual or visual.gen png to a 500kb jpeg for streaming
    src_png = gen_path if os.path.exists(gen_path) else vis_path
    im = Image.open(src_png).convert("RGB")
    jpeg_bytes = compress_1024_jpeg_500kb(im)
    jpg_path = stem + ".jpg"
    with open(jpg_path, "wb") as f:
        f.write(jpeg_bytes)

    with open(stem + ".palette.json", "w") as f:
        json.dump({"cell_px": DATA_PX, "grid": GRID,
                   "visual_size": SIZE,
                   "visual_px_per_cell": SIZE // GRID,
                   "palette": {str(k): list(v) for k, v in RGB_TUPLE.items()}}, f)

    # Centreline sidecar, in cell units as the compiler expects.
    with open(stem + ".line.json", "w") as f:
        json.dump({
            "cell_px": DATA_PX,
            "grid": GRID,
            "levels": 1,
            "road_cells": ROAD_HALF_CELLS * 2,
            "source": "svg",
            "samples": [[round(x, 3), round(y, 3), 0] for x, y in centre],
        }, f)

    counts: dict[str, int] = {}
    for code, rgb in RGB_TUPLE.items():
        counts[RGB[code]] = int((codes == code).sum())
    return {
        "name": spec["name"],
        "svg": stem + ".svg",
        "data": stem + ".data.png",
        "visual": vis_path,
        "jpeg": jpg_path,
        "line": stem + ".line.json",
        "centreline_samples": len(centre),
        "counts": counts,
    }


# --- The circuits -------------------------------------------------------------

#: Four circuits, spanning about 10 to 20 seconds a lap.
#:
#: # How the sizes were chosen
#:
#: The target is a lap the player finishes in 10-20 seconds. Everything below
#: follows from two measured constants:
#:
#: * **Pace.** The reference driver laps the previous 96-cell oval (a
#:   `straight_half` 20 / `radius` 16 stadium, so a 180.5-cell centreline, 5,776
#:   world units) in 1,261 ticks. That is 4.58 world units per tick, and it is the
#:   number every perimeter here is divided by.
#: * **Room.** From a centre at `GRID / 2`, the widest point of the ring is
#:   `straight_half + radius` for a stadium, or `half_x + radius` for a rounded
#:   rectangle, plus the whole cross-section out to the barrier:
#:
#:       straight_half + radius + BORDER_CELLS <= GRID / 2 - 1
#:
#:   which at `GRID` 80 and `BORDER_CELLS` 7.1 is `<= 31.9`. The *road* only has
#:   to stay off the image border -- that is what `compile_circuit.validate`
#:   checks -- so the real bound is `+ ROAD_HALF_CELLS` and is a full 4 cells
#:   looser; the barrier of the largest circuit does reach the outermost column.
#:   Conflating the two is what made the previous 96-cell sizing arithmetic look
#:   tighter than it was.
#:
#: Perimeter in cells, then:
#:
#: | circuit | shape | geometry | centreline | world units | ticks | seconds |
#: | :--- | :--- | :--- | ---: | ---: | ---: | ---: |
#: | Hells Bells | stadium | 10 / 7 | 84 | 2,687 | 592 | 9.9 |
#: | Copper Gorge | stadium | 14 / 9 | 112 | 3,602 | 798 | 13.3 |
#: | Longbow | rounded rect | 25 x 11 / 7 | 132 | 4,223 | 907 | 15.1 |
#: | Amber Mesa | rounded rect | 28 x 14 / 7 | 156 | 4,991 | 1,185 | 19.8 |
#:
#: Measured, not extrapolated. These are the `dev` figures from
#: `playtest --calibrate` on exactly these geometries -- the fastest lap the
#: reference driver manages across the Garage's six tuning presets, which is the
#: "reference pace" the 20-second target is stated in. Two reasons the numbers do
#: not follow from the perimeter arithmetic above:
#:
#: * **Lap time is not proportional to distance.** Corner speed depends on radius,
#:   not on arc length, so a rounded rectangle has to be *bigger* than a stadium of
#:   the same lap time. Longbow's 132-cell centreline laps 50% slower than
#:   Copper Gorge's 112 despite being only 17% longer.
#: * **Tuning matters.** The default-tune lap is 25-35% slower again (Hells Bells:
#:   592 swept, 743 on defaults, which is the `gold` medal target). The table is
#:   the swept figure because that is what the design target refers to;
#:   `par_calibration.json` carries all four tiers.
#:
#: The four are not evenly spaced, deliberately: an easy stage at ten seconds
#: and a showcase at twenty is the range the design asks for, and the two in
#: between are placed where the shapes change character rather than where the
#: arithmetic is neat. Copper Gorge is the same oval as Hells Bells and only
# exists to be a slightly longer oval; Longbow is where the circuit stops being
# two corners and starts being four.
#:
#: The rounded rectangles exist because a stadium at a fixed perimeter is mostly
#: empty: its bounding box is `2 * (straight_half + radius)` on a side and it
#: only fills that box at the four arc extremes, so the easy stage would sit in a
#: small ring adrift in the middle of a lot of grass -- the "drowning in the
#: space" complaint again, in a different costume. A rounded rectangle spends the
#: same lap length on four straights and four corners spread across the grid, so
#: the two large circuits use the space they have.
#:
#: # Why these four and not more
#:
#: Four is what the budget buys. `build_atlas` bakes one 4bpp world texture per
#: circuit, 320x320 texels = 51 KB each, and the PSX static-RAM gate leaves under
#: 300 KB of headroom over the rest of the game; five circuits would not fit. A
#: tenth of a second per circuit is not worth failing the gate over.
#:
#: # Two earlier failures, for whoever moves these numbers next
#:
#: Attempts at 15/8 and then 12/7 (at the original 48-cell grid) both failed
#: validation with the *same* pair of errors: a road cell on the border, plus a
#: phantom "second piece of road". Both were one bug -- the ring ran off the edge
#: of the grid, so `find_regions` clipped it into two fragments at the wrap.
#: Neither error named the cause, which is why it read as two problems. The
#: `BORDER_CELLS` constant exists so the next person does the arithmetic once.
#: Gates per lap. Six because `test_super_stages_are_real_circuits` wants at
#: least six checkpoints, and `LapTimer` needs enough of them that a single lap
#: cannot pass through every one in one frame -- the plausibility guard in
#: `timing.rs` is what stops a teleport being read as a lap, and its threshold is
#: a fraction of the lap, so the gate *count* is what sets that margin.
GATES = 6


def standard_marks(boost_at: tuple[int, ...] = (1, 4),
                   oil_at: tuple[int, ...] = ()) -> list[tuple]:
    """Gates evenly spaced round the lap, plus boost pads and oil slicks.

    Gates sit at fractions `i / (GATES + 1)` for `i` in `1..=GATES`.
    Boost pads sit at `(i - 0.5) / (GATES + 1)`.
    Oil slicks sit at `(i - 0.25) / (GATES + 1)` as high-risk hazards.
    """
    step = 1.0 / (GATES + 1)
    across = ROAD_HALF_CELLS + KERB_CELLS
    marks: list[tuple] = [(0.0, START_LINE, across, 1.2)]
    for i in range(1, GATES + 1):
        marks.append((i * step, GATE, across, 0.7))
    for i in boost_at:
        assert 1 <= i <= GATES, f"boost interval {i} is not between two gates"
        marks.append(((i - 0.5) * step, BOOST, ROAD_HALF_CELLS, 1.0))
    for i in oil_at:
        assert 1 <= i <= GATES, f"oil interval {i} is not between two gates"
        marks.append(((i - 0.25) * step, OIL, 1.5, 0.8))
    return marks


CIRCUITS = [
    # --- BRONZE CUP (Tracks 1 - 6) ---
    {
        "name": "Cascade Falls",
        "shape": "polygon",
        "vertices": [
            (18.0, 20.0),
            (62.0, 20.0),
            (62.0, 40.0),
            (50.0, 40.0),
            (45.0, 52.0),
            (37.0, 52.0),
            (32.0, 40.0),
            (18.0, 40.0),
        ],
        "radii": [5.0, 5.0, 4.0, 3.5, 3.5, 3.5, 4.0, 5.0],
        "shortcuts": ["M 50.0 40.0 L 32.0 40.0"],
        "runoff": GRAVEL,
        "marks": standard_marks((1, 4), (3,)),
        "note": "Short technical circuit featuring an aggressive chicane, hairpin, oil hazard, and chicane bypass shortcut.",
    },
    {
        "name": "Copper Gorge",
        "shape": "polygon",
        "vertices": [
            (18.0, 16.0),
            (62.0, 16.0),
            (62.0, 38.0),
            (46.0, 38.0),
            (46.0, 62.0),
            (18.0, 62.0),
            (18.0, 38.0),
            (30.0, 28.0),
        ],
        "radii": [5.0, 5.0, 4.0, 4.0, 5.0, 5.0, 4.0, 4.0],
        "shortcuts": ["M 46.0 38.0 L 18.0 38.0"],
        "runoff": SAND,
        "marks": standard_marks((2, 5), (4,)),
        "note": "Canyon switchback circuit with sweeping bends, tight dogleg, oil slick, and waist-line canyon shortcut.",
    },
    {
        "name": "Longbow",
        "shape": "polygon",
        "vertices": [
            (16.0, 16.0),
            (64.0, 16.0),
            (64.0, 62.0),
            (48.0, 62.0),
            (44.0, 48.0),
            (36.0, 48.0),
            (32.0, 62.0),
            (16.0, 62.0),
        ],
        "radii": [5.0, 6.0, 6.0, 4.0, 3.5, 3.5, 4.0, 5.0],
        "shortcuts": ["M 48.0 62.0 L 32.0 62.0"],
        "runoff": GRASS,
        "marks": standard_marks((1, 3, 5), (2, 6)),
        "note": "High-speed circuit with long straights, sweeping carousel, technical chicane with bypass, boost pads and oil slicks.",
    },
    {
        "name": "Amber Mesa",
        "shape": "polygon",
        "vertices": [
            (18.0, 16.0),
            (62.0, 16.0),
            (62.0, 36.0),
            (48.0, 46.0),
            (62.0, 54.0),
            (62.0, 64.0),
            (36.0, 64.0),
            (18.0, 46.0),
        ],
        "radii": [5.0, 5.0, 4.0, 4.0, 4.0, 5.0, 5.0, 5.0],
        "shortcuts": ["M 62.0 36.0 L 62.0 54.0"],
        "runoff": SAND,
        "marks": standard_marks((1, 4, 6), (2, 5)),
        "note": "Grand Prix championship circuit with high-speed straights, sweeping carousel, technical chicane, chicane bypass shortcut, and boost zones.",
    },
    {
        "name": "Emerald Glen",
        "shape": "polygon",
        "vertices": [
            (18.0, 18.0),
            (62.0, 18.0),
            (62.0, 36.0),
            (50.0, 36.0),
            (44.0, 50.0),
            (58.0, 62.0),
            (22.0, 62.0),
            (18.0, 40.0),
        ],
        "radii": [5.0, 5.0, 4.0, 3.5, 3.5, 5.0, 5.0, 4.5],
        "shortcuts": ["M 50.0 36.0 L 58.0 62.0"],
        "runoff": GRASS,
        "marks": standard_marks((1, 4), (3,)),
        "note": "Rolling meadow valley featuring an S-chicane, sweeping carousel, and hillside bypass shortcut.",
    },
    {
        "name": "Sunburst Valley",
        "shape": "polygon",
        "vertices": [
            (20.0, 18.0),
            (60.0, 18.0),
            (64.0, 38.0),
            (52.0, 38.0),
            (46.0, 52.0),
            (56.0, 62.0),
            (20.0, 62.0),
            (16.0, 38.0),
        ],
        "radii": [5.0, 5.0, 4.5, 3.5, 3.5, 4.5, 5.0, 4.5],
        "shortcuts": ["M 64.0 38.0 L 56.0 62.0"],
        "runoff": GRAVEL,
        "marks": standard_marks((2, 5), (1, 4)),
        "note": "Sun-drenched valley circuit with flowing sweepers, chicane challenge, and scenic shortcut.",
    },

    # --- SILVER CUP (Tracks 7 - 12) ---
    {
        "name": "Whispering Pines",
        "shape": "polygon",
        "vertices": [
            (16.0, 18.0),
            (64.0, 18.0),
            (64.0, 36.0),
            (50.0, 46.0),
            (64.0, 56.0),
            (48.0, 62.0),
            (16.0, 62.0),
            (24.0, 40.0),
        ],
        "radii": [5.0, 5.0, 4.0, 3.5, 4.0, 5.0, 5.0, 4.0],
        "shortcuts": ["M 64.0 36.0 L 48.0 62.0"],
        "runoff": GRASS,
        "marks": standard_marks((2, 5), (3,)),
        "note": "Dense pine forest speedway with a sharp inward chicane pocket, hairpin curve, and pine pass shortcut.",
    },
    {
        "name": "Ironstone Quarry",
        "shape": "polygon",
        "vertices": [
            (18.0, 16.0),
            (62.0, 16.0),
            (62.0, 34.0),
            (46.0, 34.0),
            (46.0, 48.0),
            (62.0, 48.0),
            (62.0, 64.0),
            (18.0, 64.0),
        ],
        "radii": [4.5, 4.5, 4.0, 3.5, 3.5, 4.0, 4.5, 4.5],
        "shortcuts": ["M 46.0 34.0 L 46.0 48.0"],
        "runoff": GRAVEL,
        "marks": standard_marks((1, 4), (2, 5)),
        "note": "Industrial quarry circuit carved into stone terraces with dual dogleg chicanes and high-risk chute.",
    },
    {
        "name": "Silverstone Mist",
        "shape": "polygon",
        "vertices": [
            (16.0, 20.0),
            (48.0, 20.0),
            (62.0, 32.0),
            (62.0, 58.0),
            (38.0, 58.0),
            (30.0, 46.0),
            (22.0, 58.0),
            (16.0, 42.0),
        ],
        "radii": [5.0, 4.5, 5.0, 5.0, 4.0, 3.5, 4.0, 4.5],
        "shortcuts": ["M 38.0 58.0 L 22.0 58.0"],
        "runoff": SAND,
        "marks": standard_marks((2, 4), (3,)) ,
        "note": "Misty coastal course with long sweeping curve, tightening hairpin, and beach road bypass.",
    },
    {
        "name": "Falcon Crest",
        "shape": "polygon",
        "vertices": [
            (18.0, 18.0),
            (62.0, 18.0),
            (62.0, 42.0),
            (50.0, 42.0),
            (50.0, 56.0),
            (40.0, 64.0),
            (18.0, 64.0),
            (18.0, 38.0),
        ],
        "radii": [5.0, 5.0, 4.0, 3.5, 4.0, 5.0, 5.0, 4.5],
        "shortcuts": ["M 50.0 42.0 L 40.0 64.0"],
        "runoff": GRAVEL,
        "marks": standard_marks((1, 5), (3, 6)),
        "note": "Ridge-top circuit with asymmetric corners, descending hairpin, and crestline shortcut.",
    },
    {
        "name": "Willow Creek",
        "shape": "polygon",
        "vertices": [
            (20.0, 18.0),
            (60.0, 18.0),
            (60.0, 36.0),
            (46.0, 42.0),
            (60.0, 50.0),
            (60.0, 62.0),
            (20.0, 62.0),
            (20.0, 40.0),
        ],
        "radii": [4.5, 4.5, 4.0, 3.5, 4.0, 4.5, 4.5, 4.0],
        "shortcuts": ["M 60.0 36.0 L 60.0 50.0"],
        "runoff": GRASS,
        "marks": standard_marks((2, 4), (1,)),
        "note": "Creek bend track with serpentine S-curves, technical mid-sector chicane, and creek crossing shortcut.",
    },
    {
        "name": "Cobalt Run",
        "shape": "polygon",
        "vertices": [
            (16.0, 22.0),
            (64.0, 22.0),
            (64.0, 44.0),
            (52.0, 44.0),
            (46.0, 62.0),
            (32.0, 62.0),
            (24.0, 44.0),
            (16.0, 44.0),
        ],
        "radii": [5.0, 5.0, 4.0, 3.5, 4.5, 4.5, 3.5, 4.0],
        "shortcuts": ["M 52.0 44.0 L 24.0 44.0"],
        "runoff": GRAVEL,
        "marks": standard_marks((1, 3, 5), (4,)),
        "note": "Urban industrial ring with rapid chicanes, high-speed straight, and warehouse shortcut.",
    },

    # --- GOLD CUP (Tracks 13 - 18) ---
    {
        "name": "Dragon's Spine",
        "shape": "polygon",
        "vertices": [
            (18.0, 16.0),
            (62.0, 16.0),
            (62.0, 32.0),
            (48.0, 38.0),
            (62.0, 46.0),
            (62.0, 62.0),
            (38.0, 62.0),
            (18.0, 44.0),
        ],
        "radii": [5.0, 5.0, 4.0, 3.5, 4.0, 5.0, 5.0, 4.5],
        "shortcuts": ["M 62.0 32.0 L 62.0 46.0"],
        "runoff": SAND,
        "marks": standard_marks((2, 5), (3, 6)),
        "note": "Mountain spine with dramatic chicanes, double kinks, oil slicks, and dragon's ridge shortcut.",
    },
    {
        "name": "Neon Boulevard",
        "shape": "polygon",
        "vertices": [
            (18.0, 18.0),
            (62.0, 18.0),
            (62.0, 36.0),
            (42.0, 36.0),
            (42.0, 50.0),
            (62.0, 50.0),
            (62.0, 64.0),
            (18.0, 64.0),
        ],
        "radii": [4.5, 4.5, 4.0, 3.5, 3.5, 4.0, 4.5, 4.5],
        "shortcuts": ["M 62.0 36.0 L 62.0 50.0"],
        "runoff": GRAVEL,
        "marks": standard_marks((1, 3), (2, 5)),
        "note": "Broad neon-lit boulevard into tight city-block 90-degree corners with back-alley shortcut.",
    },
    {
        "name": "Redwood Pass",
        "shape": "polygon",
        "vertices": [
            (16.0, 18.0),
            (64.0, 18.0),
            (64.0, 40.0),
            (50.0, 48.0),
            (64.0, 58.0),
            (36.0, 64.0),
            (16.0, 64.0),
            (16.0, 38.0),
        ],
        "radii": [5.0, 5.0, 4.0, 3.5, 4.5, 5.0, 5.0, 4.5],
        "shortcuts": ["M 64.0 40.0 L 36.0 64.0"],
        "runoff": GRASS,
        "marks": standard_marks((1, 4), (2, 5)),
        "note": "Forest high-speed sweep with technical chicane obstacle and logging road shortcut.",
    },
    {
        "name": "Thunder Basin",
        "shape": "polygon",
        "vertices": [
            (20.0, 16.0),
            (60.0, 16.0),
            (64.0, 38.0),
            (48.0, 38.0),
            (44.0, 54.0),
            (56.0, 62.0),
            (20.0, 62.0),
            (16.0, 38.0),
        ],
        "radii": [5.0, 5.0, 4.5, 3.5, 3.5, 4.5, 5.0, 4.5],
        "shortcuts": ["M 48.0 38.0 L 56.0 62.0"],
        "runoff": SAND,
        "marks": standard_marks((2, 5), (3,)),
        "note": "Bowl-shaped canyon with banked double hairpins, oil hazard, and basin jump shortcut.",
    },
    {
        "name": "Canyon Mirage",
        "shape": "polygon",
        "vertices": [
            (18.0, 20.0),
            (62.0, 20.0),
            (62.0, 42.0),
            (46.0, 42.0),
            (46.0, 54.0),
            (60.0, 62.0),
            (28.0, 62.0),
            (18.0, 46.0),
        ],
        "radii": [5.0, 5.0, 4.0, 3.5, 3.5, 4.5, 5.0, 4.5],
        "shortcuts": ["M 46.0 42.0 L 28.0 62.0"],
        "runoff": SAND,
        "marks": standard_marks((1, 4), (2, 5)),
        "note": "Deceptive desert canyon with switchback S-curves, blinding apexes, and mirage canyon shortcut.",
    },
    {
        "name": "Obsidian Loop",
        "shape": "polygon",
        "vertices": [
            (16.0, 16.0),
            (64.0, 16.0),
            (64.0, 38.0),
            (50.0, 46.0),
            (38.0, 46.0),
            (52.0, 62.0),
            (16.0, 62.0),
            (16.0, 38.0),
        ],
        "radii": [5.0, 5.0, 4.0, 3.5, 3.5, 5.0, 5.0, 4.5],
        "shortcuts": ["M 64.0 38.0 L 52.0 62.0"],
        "runoff": GRAVEL,
        "marks": standard_marks((1, 3, 5), (2, 4)),
        "note": "Dark volcanic speedway with long straights, carousel chicane, and lava chute shortcut.",
    },

    # --- PLATINUM CUP (Tracks 19 - 24) ---
    {
        "name": "Apex Predator",
        "shape": "polygon",
        "vertices": [
            (16.0, 16.0),
            (64.0, 16.0),
            (64.0, 36.0),
            (48.0, 42.0),
            (64.0, 48.0),
            (64.0, 64.0),
            (32.0, 64.0),
            (16.0, 44.0),
        ],
        "radii": [5.0, 5.0, 4.0, 3.5, 4.0, 5.0, 5.0, 4.5],
        "shortcuts": ["M 64.0 36.0 L 64.0 48.0"],
        "runoff": GRAVEL,
        "marks": standard_marks((1, 4, 6), (2, 5)),
        "note": "Championship showcase with aggressive double chicane, triple boost pads, and predator bypass.",
    },
    {
        "name": "Viper's Tail",
        "shape": "polygon",
        "vertices": [
            (20.0, 18.0),
            (60.0, 18.0),
            (60.0, 34.0),
            (42.0, 40.0),
            (58.0, 48.0),
            (58.0, 64.0),
            (20.0, 64.0),
            (20.0, 40.0),
        ],
        "radii": [4.5, 4.5, 4.0, 3.5, 4.0, 4.5, 4.5, 4.0],
        "shortcuts": ["M 60.0 34.0 L 58.0 48.0"],
        "runoff": SAND,
        "marks": standard_marks((2, 5), (3, 6)),
        "note": "Coiled serpentine track demanding precise drift angles with sudden chicane and venom shortcut.",
    },
    {
        "name": "Frostpeak Summit",
        "shape": "polygon",
        "vertices": [
            (18.0, 18.0),
            (62.0, 18.0),
            (62.0, 38.0),
            (48.0, 46.0),
            (62.0, 56.0),
            (44.0, 64.0),
            (18.0, 64.0),
            (18.0, 40.0),
        ],
        "radii": [5.0, 5.0, 4.0, 3.5, 4.0, 4.5, 5.0, 4.5],
        "shortcuts": ["M 48.0 46.0 L 44.0 64.0"],
        "runoff": GRAVEL,
        "marks": standard_marks((1, 3, 5), (2, 4)),
        "note": "High alpine ridge with sharp hairpins, slick gravel runoffs, and mountain ridge shortcut.",
    },
    {
        "name": "Golden Horizon",
        "shape": "polygon",
        "vertices": [
            (16.0, 20.0),
            (64.0, 20.0),
            (64.0, 40.0),
            (50.0, 40.0),
            (46.0, 56.0),
            (36.0, 56.0),
            (30.0, 40.0),
            (16.0, 40.0),
        ],
        "radii": [5.0, 5.0, 4.0, 3.5, 3.5, 3.5, 4.0, 5.0],
        "shortcuts": ["M 50.0 40.0 L 30.0 40.0"],
        "runoff": SAND,
        "marks": standard_marks((1, 4), (2, 5)),
        "note": "Sunset speedway with long straights, tight carousel chicane, and horizon straight shortcut.",
    },
    {
        "name": "Phantom Ravine",
        "shape": "polygon",
        "vertices": [
            (18.0, 16.0),
            (62.0, 16.0),
            (62.0, 36.0),
            (46.0, 36.0),
            (46.0, 50.0),
            (60.0, 58.0),
            (32.0, 64.0),
            (18.0, 44.0),
        ],
        "radii": [5.0, 5.0, 4.0, 3.5, 3.5, 4.0, 4.5, 4.5],
        "shortcuts": ["M 46.0 36.0 L 60.0 58.0"],
        "runoff": GRAVEL,
        "marks": standard_marks((2, 4, 6), (1, 3)),
        "note": "Intense cliffside ravine circuit with double hairpins, oil traps, and ravine bridge shortcut.",
    },
    {
        "name": "Grand Finale",
        "shape": "polygon",
        "vertices": [
            (16.0, 16.0),
            (64.0, 16.0),
            (64.0, 36.0),
            (46.0, 44.0),
            (62.0, 52.0),
            (62.0, 64.0),
            (34.0, 64.0),
            (16.0, 48.0),
        ],
        "radii": [5.0, 5.0, 4.0, 3.5, 4.0, 5.0, 5.0, 4.5],
        "shortcuts": ["M 64.0 36.0 L 62.0 52.0"],
        "runoff": GRASS,
        "marks": standard_marks((1, 3, 5), (2, 4, 6)),
        "note": "The ultimate grand championship test combining every feature: long boost straights, technical chicanes, hairpin switchback, and championship bypass shortcut.",
    },
]


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--out", default=None,
                    help="output directory (default: <repo>/tracks)")
    ap.add_argument("--only", default=None, help="generate just this circuit")
    args = ap.parse_args()

    root = os.path.dirname(os.path.dirname(os.path.dirname(
        os.path.abspath(__file__))))
    outdir = args.out or os.path.join(root, "tracks")
    os.makedirs(outdir, exist_ok=True)

    for spec in CIRCUITS:
        if args.only and args.only.lower() not in spec["name"].lower():
            continue
        info = write_outputs(spec, outdir)
        road = info["counts"][RGB[TARMAC]] + info["counts"][RGB[TARMAC_WORN]]
        print(f"{info['name']:16} {info['centreline_samples']:6} centreline samples  "
              f"road {road:5} cells  -> {os.path.basename(info['svg'])}")
        print(f"{'':16} + .data.png + .visual.png + .line.json + .palette.json + .jpg (100kb streaming)")
    print(f"\nwrote to {outdir}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())