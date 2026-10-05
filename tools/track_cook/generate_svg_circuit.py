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

#: Cells per side. Matches `MAX_TRACK_DIM` 64 with room to spare; the cell is the
#: atom of both physics and authoring, so a 48-cell square is 3,072 world units at
#: `TILE_SIZE` 64 -- a stage-sized circuit rather than a lap of a car park.
GRID = 48

#: Raster resolution per cell for the *data* image. 3 is the smallest that
#: survives majority-vote resolution of the kerb fringe, which is ~0.7 cells.
DATA_PX = 3

#: Raster resolution per cell for the *visual* image. Painted, so it wants room.
VISUAL_PX = 16

#: Raster is taken at this multiple of the data resolution, then majority-voted
#: down. 4x4 = 16 samples per cell is ample to resolve a 0.7-cell fringe; the
#: default 3 px/cell with no supersampling leaves edges ambiguous.
SUPERSAMPLE = 4

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
# The spec asks for a 2-3 cell drivable width. These are half-widths, because
# the road is drawn as a *stroke* on the centreline and a stroke is centred.
# 1.25 cells of half-width gives 2.5 cells across, mid-range.

ROAD_HALF_CELLS = 1.25     # tarmac half-width
KERB_CELLS = 0.7           # rumble band outside the tarmac
RUNOFF_CELLS = 3.2         # gravel band outside the kerb
WALL_CELLS = 1.0           # barrier band outside the runoff

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


def build_svg(spec: dict) -> tuple[str, list[tuple[float, float]]]:
    """Renders one circuit spec to an SVG document, and returns its centreline.

    Drawn back to front: ground, runoff, kerb, road, racing line, markings. The
    order is the whole trick -- SVG has no z-buffer beyond document order, so a
    kerb painted before the road is simply painted over.
    """
    cx = cy = GRID / 2.0
    straight_half = spec["straight_half"]
    radius = spec["radius"]
    centre = stadium_centreline(cx, cy, straight_half, radius)
    path = stadium_path(cx, cy, straight_half, radius)

    road_hw = ROAD_HALF_CELLS
    kerb_hw = road_hw + KERB_CELLS
    runoff_hw = kerb_hw + RUNOFF_CELLS
    wall_hw = runoff_hw + WALL_CELLS

    out = svg_header(GRID * VISUAL_PX)
    out.append(f'<!-- {spec["name"]}: authored circuit, cell units. -->')
    out.append(f'<!-- centreline: stadium, straight_half={straight_half}, '
               f'radius={radius} -->')

    # Ground. Everything outside the circuit is grass; the infield is grass too,
    # which is what gives the ring its surroundings.
    out.append(f'<rect x="0" y="0" width="{GRID}" height="{GRID}" '
               f'fill="{RGB[OUTSIDE]}"/>')

    # Barrier, then runoff over its inner half, so the barrier reads as a rim.
    #
    # `RGB[WALL]` and `RGB[TARMAC]` cannot be told apart by the snapping in
    # `snap_to_codes`, which votes a blend of the two to whichever is nearer by a
    # weighted RGB distance. Wall (42,46,56) sits close to tarmac (60,62,68), and
    # a 1-cell barrier is *mostly* anti-aliased edge, so its cells resolved to
    # tarmac. A barrier that reports as road is worse than no barrier: it becomes
    # a drivable cell on the circuit's outer edge, and `find_regions` then counts
    # it as part of the ring. Painted last it would be worse still -- it must be
    # *wider* than the runoff so its outer band survives, which is why it goes
    # first and the runoff covers only its inner half.
    out.append(f'<path d="{path}" fill="none" stroke="{RGB[WALL]}" '
               f'stroke-width="{2 * wall_hw:.4f}" stroke-linecap="butt"/>')
    runoff_code = int(spec["runoff"])
    out.append(f'<path d="{path}" fill="none" stroke="{RGB[runoff_code]}" '
               f'stroke-width="{2 * runoff_hw:.4f}" stroke-linecap="butt"/>')

    # Kerb as two dashes of opposite phase. Alternation is a property of the
    # *sequence along the track*, so it cannot be decided per pixel; drawing it
    # as two complementary dash patterns on the same path expresses the sequence
    # directly in the document, which is the whole reason to be drawing SVG.
    # A wide, solid kerb band first, so the rumble always reads as continuous.
    dash = 2.0
    out.append(f'<path d="{path}" fill="none" stroke="{RGB[KERB_WHITE]}" '
               f'stroke-width="{2 * kerb_hw:.4f}" stroke-linecap="butt"/>')
    # Then the red half of the alternation as dashes over it.
    #
    # This is why the kerb is *two* passes and not two dashed strokes. Two
    # complementary dashed strokes leave gaps: `stroke-dasharray` restarts on
    # every subpath, and where a gap from one phase happens to land on the other
    # phase's gap the kerb disappears entirely for a cell. `find_regions` is
    # 4-connected, so a one-cell gap in the ring splits the circuit into pieces
    # and validation reports "a second piece of road" -- an error that names the
    # symptom rather than the cause, and that reads like a geometry bug when it
    # is a paint-order bug. Solid underneath, dashed on top, the kerb is
    # continuous by construction and the phase cannot open a hole in it.
    out.append(
        f'<path d="{path}" fill="none" stroke="{RGB[KERB_RED]}" '
        f'stroke-width="{2 * kerb_hw:.4f}" stroke-linecap="butt" '
        f'stroke-dasharray="{dash:.2f} {dash:.2f}" '
        f'stroke-dashoffset="0.00"/>'
    )

    # Road.
    out.append(f'<path d="{path}" fill="none" stroke="{RGB[TARMAC]}" '
               f'stroke-width="{2 * road_hw:.4f}" stroke-linecap="butt"/>')

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


def paint_visual(codes: np.ndarray, px: int) -> np.ndarray:
    """Paints the appearance image from the code map. Display only.

    Deliberately does *not* read the SVG raster. The visual image is what the
    player sees and the data map is what the car drives; keeping them separate is
    the whole point of the two-image format, and sharing one would let a pretty
    texture quietly change collision.
    """
    vis = np.repeat(np.repeat(codes, px, axis=0), px, axis=1)
    h, w = vis.shape
    out = np.zeros((h, w, 3), dtype=np.uint8)

    def base(code):
        return np.array(RGB_TUPLE[code], dtype=np.float32)

    def grain(y, x, salt=0):
        v = ((x * 73856093) ^ (y * 19349663) ^ (salt * 83492791)) & 0xFF
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
                # Blocks along the direction of travel, not the 1-cell fringe the
                # data map carries: the map says "kerb here", the visual says
                # "red and white in 2-cell blocks".
                #
                # The block size is one *cell*, not two pixels. The previous 2-px
                # checker averaged out at display scale into a flat pink, because
                # a 2-px alternating pair has a mean colour and the eye sees the
                # mean. At `VISUAL_PX` per cell a 1-cell block is 16 px -- four
                # times the display resolution of the whole 320x240 frame, so the
                # alternation survives to the screen.
                blk = ((x // px) + (y // px)) % 2
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
                out[y, x] = (245, 245, 245) if ((x // 6) + (y // 6)) % 2 else (28, 28, 32)
            elif c == BOOST:
                band = (y + x // 3) % 6
                out[y, x] = (255, 240, 200) if band < 2 else np.clip(b * (1 - n * 0.1), 0, 255)
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
    dat = np.repeat(np.repeat(codes, DATA_PX, axis=0), DATA_PX, axis=1)
    # Index by code through a lookup table rather than a boolean mask per code:
    # the data image must be *exactly* palette colours, and building it from the
    # code map by table guarantees that rather than hoping the raster was exact.
    lut = np.zeros((16, 3), dtype=np.uint8)
    for code, rgb in RGB_TUPLE.items():
        lut[code] = rgb
    flat = lut[dat]
    Image.fromarray(flat, "RGB").save(stem + ".data.png")

    # Visual image: painted from the code map, VISUAL_PX px per cell.
    vis = paint_visual(codes, VISUAL_PX)
    Image.fromarray(vis, "RGB").save(stem + ".visual.png")

    with open(stem + ".palette.json", "w") as f:
        json.dump({"cell_px": DATA_PX, "grid": GRID,
                   "visual_px_per_cell": VISUAL_PX,
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
        "visual": stem + ".visual.png",
        "line": stem + ".line.json",
        "centreline_samples": len(centre),
        "counts": counts,
    }


# --- The circuits -------------------------------------------------------------

#: One oval to start with. `straight_half` and `radius` are in cells; the outer
#: radius including kerb, runoff and wall is what has to clear the grid border,
#: because `compile_circuit.validate` rejects road on the border.
#
#: Sizing a stadium so the whole cross-section fits inside the grid.
#:
#: The widest point of the ring is `straight_half + radius + wall_hw`: the arc
#: segments bow out by their full radius *beyond* the straight's endpoints, so
#: the horizontal extent is driven by the sum of all three, not by the straight
#: length. From a centre at `GRID / 2`, that must leave the outermost cell inside
#: the grid:
#:
#:     straight_half + radius + wall_hw <= GRID / 2 - 1
#:
#: with `wall_hw = road_hw + KERB_CELLS + RUNOFF_CELLS + WALL` = 1.25 + 0.7 + 3.2 + 1.0 =
#: 6.15 cells. `straight_half` 9, `radius` 7 gives 22.15, so the ring spans
#: columns 1.85..46.15 and rows 10.85..37.15. The *road* stops much earlier, at
#: `straight_half + radius + road_hw` = 17.25, i.e. columns 6.75..41.25, which is
#: what `compile_circuit.validate` actually checks.
#:
#: Two earlier attempts (15/8, then 12/7) both failed validation, and with the
#: *same* pair of errors each time: a road cell on the border, plus a phantom
#: "second piece of road". Both are one bug. The ring ran off the edge of the
#: grid, so `find_regions` clipped it into two fragments where it met the border
#: and the wrap-around. Neither error named the cause, which is why the first
#: attempt looked like two unrelated problems.
CIRCUITS = [
    {
        "name": "Hells Bells",
        "straight_half": 9.0,
        "radius": 7.0,
        "runoff": GRAVEL,
        # (fraction of lap, code, half-length across road, half-thickness along)
        "marks": [
            (0.00, START_LINE, ROAD_HALF_CELLS + KERB_CELLS, 0.6),
            (0.17, GATE, ROAD_HALF_CELLS + KERB_CELLS, 0.35),
            (0.34, GATE, ROAD_HALF_CELLS + KERB_CELLS, 0.35),
            (0.50, GATE, ROAD_HALF_CELLS + KERB_CELLS, 0.35),
            (0.66, GATE, ROAD_HALF_CELLS + KERB_CELLS, 0.35),
            (0.83, GATE, ROAD_HALF_CELLS + KERB_CELLS, 0.35),
            (0.10, BOOST, ROAD_HALF_CELLS, 0.5),
            (0.60, BOOST, ROAD_HALF_CELLS, 0.5),
        ],
        "note": "Hand-authored SVG oval. Baseline for the image pipeline.",
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
        print(f"{'':16} + .data.png + .visual.png + .line.json + .palette.json")
    print(f"\nwrote to {outdir}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())