"""SVG emission and re-import, in the exact format the game pipeline reads.

Emission mirrors `generate_svg_circuit.build_svg` element for element: same
paint order, same palette, same stroke widths derived from the road half-width,
same `<polygon>` bands for the features. The only difference is that the road
width and the centreline come from an edited `Level` instead of a hard-coded
circuit table, which is the whole point.

Re-import is the other half of "open any level in the game". The SVG is a
*drawing*, so the centreline has to be recovered from it rather than read out
of it -- there are no ids, no groups and no transforms anywhere in the format,
because the format's entire contract is "exact palette colours, in document
order". Recovery is lossy by construction, so the recovered level is marked
`imported` in its descriptors and the first thing the editor does with it is
let you nudge a node and see what it did.
"""

from __future__ import annotations

import math
import os
import re
import xml.etree.ElementTree as ET
from typing import Sequence

from . import constants as C
from . import geometry as G
from .model import Level, Mark, Node

SVG_NS = "http://www.w3.org/2000/svg"

#: How far a recovered node may sit from the line it replaced before the
#: simplifier keeps an extra one. Small enough that a 3-cell corner survives,
#: large enough that the 0.25-cell sample noise along a straight does not.
SIMPLIFY_EPS = 0.55

_NUM = re.compile(r"[-+]?(?:\d+\.?\d*|\.\d+)(?:[eE][-+]?\d+)?")


# --- Path parsing -------------------------------------------------------------


def _tokenize_path(d: str) -> list[tuple[str, list[float]]]:
    """Splits an SVG `d` into (command, numbers) pairs, handling implicit repeats.

    `L 1 2 3 4` is two line segments, not one, and hand-written SVG -- and
    every exporter ever written -- relies on that. Splitting on the command
    letter alone silently drops the second segment, which for a track SVG
    means half the circuit vanishing.
    """
    out: list[tuple[str, list[float]]] = []
    i = 0
    n = len(d)
    cmd = ""
    while i < n:
        ch = d[i]
        if ch in "MmLlHhVvCcSsQqTtAaZz":
            cmd = ch
            i += 1
            if cmd in "Zz":
                out.append((cmd, []))
                continue
        elif ch in " ,\t\r\n":
            i += 1
            continue
        m = _NUM.match(d, i)
        if not m:
            i += 1
            continue
        i = m.end()
        # Sweep flags and large-arc flags are single digits, not numbers;
        # `_NUM` would swallow "01" as one token. Split them back out.
        if cmd in "Aa":
            start = m.start()
            raw = m.group(0)
            if len(raw.lstrip("+-")) > 1 and "." not in raw and "e" not in raw.lower():
                vals = [float(c) for c in raw]
            else:
                vals = [float(raw)]
            del start
        else:
            vals = [float(m.group(0))]
        if not out or out[-1][0] != cmd:
            out.append((cmd, []))
        out[-1][1].extend(vals)
    return out


def _arc_points(p0: G.Point, rx: float, ry: float, large: int, sweep: int,
                p1: G.Point, step: float) -> list[G.Point]:
    """Endpoint-parameterised elliptical arc -> polyline (SVG F.6.5).

    Written out rather than approximated by a circle because the shipped
    circuits use `rx == ry` arcs only by coincidence of the fillet maths, and a
    parser that assumes circularity would quietly mis-parse any SVG written by
    anything else -- including one exported out of this editor by a future
    version.
    """
    if math.hypot(p1[0] - p0[0], p1[1] - p0[1]) < 1e-9:
        return []
    rx, ry = abs(rx), abs(ry)
    if rx < 1e-9 or ry < 1e-9:
        return [p1]

    phi = math.radians(0.0)
    dx2 = (p0[0] - p1[0]) / 2.0
    dy2 = (p0[1] - p1[1]) / 2.0
    x1p = math.cos(phi) * dx2 + math.sin(phi) * dy2
    y1p = -math.sin(phi) * dx2 + math.cos(phi) * dy2

    lam = (x1p * x1p) / (rx * rx) + (y1p * y1p) / (ry * ry)
    if lam > 1.0:
        # SVG F.6.6: scale the radii up rather than draw nothing.
        s = math.sqrt(lam)
        rx *= s
        ry *= s

    num = rx * rx * ry * ry - rx * rx * y1p * y1p - ry * ry * x1p * x1p
    den = rx * rx * y1p * y1p + ry * ry * x1p * x1p
    if den < 1e-12:
        return [p1]
    coef = math.sqrt(max(0.0, num / den))
    if large == sweep:
        coef = -coef
    cxp = coef * rx * y1p / ry
    cyp = -coef * ry * x1p / rx
    cx = math.cos(phi) * cxp - math.sin(phi) * cyp + (p0[0] + p1[0]) / 2.0
    cy = math.sin(phi) * cxp + math.cos(phi) * cyp + (p0[1] + p1[1]) / 2.0

    def angle(ux, uy, vx, vy):
        dot = ux * vx + uy * vy
        n = math.hypot(ux, uy) * math.hypot(vx, vy)
        a = math.acos(max(-1.0, min(1.0, dot / n))) if n > 1e-12 else 0.0
        return -a if (ux * vy - uy * vx) < 0 else a

    theta1 = angle(1, 0, (x1p - cxp) / rx, (y1p - cyp) / ry)
    dtheta = angle((x1p - cxp) / rx, (y1p - cyp) / ry,
                   (-x1p - cxp) / rx, (-y1p - cyp) / ry)
    if not sweep and dtheta > 0:
        dtheta -= 2 * math.pi
    elif sweep and dtheta < 0:
        dtheta += 2 * math.pi

    arc_len = abs(dtheta) * max(rx, ry)
    n_pts = max(2, min(256, int(arc_len / max(1e-3, step)) + 2))
    out = []
    for k in range(1, n_pts):
        a = theta1 + dtheta * (k / n_pts)
        out.append((
            cx + rx * math.cos(a) * math.cos(phi) - ry * math.sin(a) * math.sin(phi),
            cy + rx * math.cos(a) * math.sin(phi) + ry * math.sin(a) * math.cos(phi),
        ))
    out.append(p1)
    return out


def _cubic(p0: G.Point, c1: G.Point, c2: G.Point, p1: G.Point, t: float) -> G.Point:
    u = 1.0 - t
    a = u * u * u
    b = 3 * u * u * t
    c = 3 * u * t * t
    d = t * t * t
    return (a * p0[0] + b * c1[0] + c * c2[0] + d * p1[0],
            a * p0[1] + b * c1[1] + c * c2[1] + d * p1[1])


def flatten_path(d: str, step: float = G.DEFAULT_STEP) -> list[G.Point]:
    """Any SVG path -> a dense polyline, sampled every `step` cells.

    Supports the whole grammar, not just what the generator emits, because the
    editor's own spline mode writes `C` and a future import of an SVG from
    anywhere else will use `S`, `T` or `Q`.

    The cursor is threaded through explicitly rather than captured in a
    closure: `cur` is reassigned on every command, and a closure over it is the
    kind of thing that reads as obviously-correct scoping right up until a
    `Q` command lands in the wrong place.
    """
    pts: list[G.Point] = []
    cur: G.Point = (0.0, 0.0)
    start: G.Point = (0.0, 0.0)
    prev_cubic_ctrl: G.Point | None = None
    prev_quad_ctrl: G.Point | None = None

    def push(p: G.Point) -> None:
        if not pts or math.dist(pts[-1], p) > 1e-7:
            pts.append(p)

    def walk_line(a: G.Point, b: G.Point) -> None:
        n = max(1, int(math.dist(a, b) / step))
        for k in range(n):
            t = k / n
            push((a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t))

    def walk_cubic(a: G.Point, c1: G.Point, c2: G.Point, b: G.Point) -> None:
        flat = max(_perp(a, c1, b), _perp(a, c2, b),
                   math.dist(a, c1), math.dist(c2, b))
        n = max(2, min(256, int(flat / step) + 2))
        for k in range(n):
            push(_cubic(a, c1, c2, b, k / n))

    def rel(x: float, y: float) -> G.Point:
        return (cur[0] + x, cur[1] + y) if is_rel[0] else (x, y)

    is_rel = [False]
    for cmd, args in cmds_of(d):
        is_rel[0] = cmd.islower()
        C_ = cmd.upper()

        if C_ == "M":
            for k in range(0, len(args) - 1, 2):
                p = rel(args[k], args[k + 1])
                if k == 0:
                    cur, start = p, p
                    push(p)
                else:
                    walk_line(cur, p)
                    cur = p
        elif C_ == "L":
            for k in range(0, len(args) - 1, 2):
                p = rel(args[k], args[k + 1])
                walk_line(cur, p)
                cur = p
        elif C_ == "H":
            for x in args:
                p = rel(x, cur[1])
                walk_line(cur, p)
                cur = p
        elif C_ == "V":
            for y in args:
                p = rel(cur[0], y)
                walk_line(cur, p)
                cur = p
        elif C_ == "C":
            for k in range(0, len(args) - 5, 6):
                v = args[k:k + 6]
                c1 = rel(v[0], v[1])
                c2 = rel(v[2], v[3])
                p = rel(v[4], v[5])
                walk_cubic(cur, c1, c2, p)
                cur, prev_cubic_ctrl = p, c2
        elif C_ == "S":
            for k in range(0, len(args) - 3, 4):
                v = args[k:k + 4]
                c1 = (2 * cur[0] - prev_cubic_ctrl[0],
                      2 * cur[1] - prev_cubic_ctrl[1]) if prev_cubic_ctrl else cur
                c2 = rel(v[0], v[1])
                p = rel(v[2], v[3])
                walk_cubic(cur, c1, c2, p)
                cur, prev_cubic_ctrl = p, c2
        elif C_ == "Q":
            for k in range(0, len(args) - 3, 4):
                v = args[k:k + 4]
                q = rel(v[0], v[1])
                p = rel(v[2], v[3])
                walk_cubic(cur, _quad_ctrl(cur, q), _mirror(q, p), p)
                cur, prev_quad_ctrl = p, q
        elif C_ == "T":
            for k in range(0, len(args) - 1, 2):
                q = (2 * cur[0] - prev_quad_ctrl[0],
                     2 * cur[1] - prev_quad_ctrl[1]) if prev_quad_ctrl else cur
                p = rel(args[k], args[k + 1])
                walk_cubic(cur, _quad_ctrl(cur, q), _mirror(q, p), p)
                cur, prev_quad_ctrl = p, q
        elif C_ == "A":
            for k in range(0, len(args) - 6, 7):
                rx, ry, _rot, large, sweep, x, y = args[k:k + 7]
                p = rel(x, y)
                for q in _arc_points(cur, rx, ry, int(large), int(sweep), p, step):
                    push(q)
                cur = p
        elif C_ == "Z":
            if math.dist(cur, start) > 1e-7:
                walk_line(cur, start)
            cur = start

    if len(pts) > 1 and math.dist(pts[0], pts[-1]) > 1e-7:
        pts.append(pts[0])
    return pts


def cmds_of(d: str) -> list[tuple[str, list[float]]]:
    return _tokenize_path(d)


def _perp(a: G.Point, b: G.Point, c: G.Point) -> float:
    dx, dy = c[0] - a[0], c[1] - a[1]
    m = math.hypot(dx, dy)
    if m < 1e-12:
        return math.dist(a, b)
    return abs(dy * (b[0] - a[0]) - dx * (b[1] - a[1])) / m


def _quad_ctrl(p0: G.Point, q: G.Point) -> G.Point:
    return (p0[0] + 2.0 / 3.0 * (q[0] - p0[0]), p0[1] + 2.0 / 3.0 * (q[1] - p0[1]))


def _mirror(q: G.Point, p: G.Point) -> G.Point:
    return (2 * p[0] - q[0], 2 * p[1] - q[1])


# --- Emission -----------------------------------------------------------------


def _stroke(
    d: str, colour: str, width: float, cap: str = "butt", extra: str = ""
) -> str:
    join = ' stroke-linejoin="round"' if cap == "round" else ""
    dash = f" {extra}" if extra else ""
    return (
        f'<path d="{d}" fill="none" stroke="{colour}" '
        f'stroke-width="{width:.4f}" stroke-linecap="{cap}"{join}{dash}/>'
    )


def build_svg(level: Level) -> tuple[str, list[G.Point]]:
    """Renders a level to an SVG document, and returns its centreline.

    Drawn back to front: ground, barrier, runoff, kerb, tarmac, markings. The
    order is the whole trick -- SVG has no z-buffer beyond document order, so a
    kerb painted before the road is simply painted over.
    """
    path, centre = level.path_and_centre()
    hw = level.half_widths()

    out: list[str] = [
        f'<svg xmlns="{SVG_NS}" width="{C.SIZE}" height="{C.SIZE}" '
        f'viewBox="0 0 {C.GRID} {C.GRID}" shape-rendering="geometricPrecision">',
        f'<!-- {level.name}: authored in the level editor, cell units. -->',
        f'<!-- centreline: {level.mode}, {len(level.nodes)} nodes, '
        f'road half-width {level.road_half_cells:g} cells -->',
    ]
    meta_note = level.meta.get("notes")
    if meta_note:
        note = str(meta_note).replace("--", "-").replace("\n", " ").strip()
        out.append(f"<!-- {note} -->")

    out.append(
        f'<rect x="0" y="0" width="{C.GRID}" height="{C.GRID}" '
        f'fill="{C.RGB[level.outside]}"/>'
    )

    shortcuts = [s for s in level.shortcuts if s.strip()]

    out.append(_stroke(path, C.RGB[C.WALL], 2 * hw["wall"]))
    for sc in shortcuts:
        out.append(_stroke(sc, C.RGB[C.WALL], 2 * hw["wall"], cap="round"))

    out.append(_stroke(path, C.RGB[level.runoff], 2 * hw["runoff"]))
    for sc in shortcuts:
        out.append(_stroke(sc, C.RGB[level.runoff], 2 * hw["runoff"], cap="round"))

    out.append(_stroke(path, C.RGB[C.KERB_WHITE], 2 * hw["kerb"]))
    for sc in shortcuts:
        out.append(_stroke(sc, C.RGB[C.KERB_WHITE], 2 * hw["kerb"], cap="round"))

    out.append(_stroke(
        path, C.RGB[C.KERB_RED], 2 * hw["kerb"],
        extra=f'stroke-dasharray="{C.KERB_DASH:.2f} {C.KERB_DASH:.2f}" '
              f'stroke-dashoffset="0.00"',
    ))

    out.append(_stroke(path, C.RGB[C.TARMAC], 2 * hw["road"]))
    for sc in shortcuts:
        out.append(_stroke(sc, C.RGB[C.TARMAC], 2 * hw["road"], cap="round"))

    for mark in level.sorted_marks():
        at, tangent = G.point_at_fraction(centre, mark.frac)
        pts = G.band_polygon(
            at, tangent,
            level.mark_half_len(mark),
            level.mark_half_thick(mark),
            mark.offset,
        )
        out.append(
            f'<polygon points="{pts}" fill="{C.RGB[mark.code]}" stroke="none"/>'
        )

    out.append("</svg>")
    return "\n".join(out), centre


def save_svg(level: Level, path: str) -> str:
    text, _ = build_svg(level)
    os.makedirs(os.path.dirname(os.path.abspath(path)) or ".", exist_ok=True)
    tmp = f"{path}.tmp"
    with open(tmp, "w", encoding="utf-8") as f:
        f.write(text + "\n")
    os.replace(tmp, path)
    return path


# --- Re-import ----------------------------------------------------------------


def _strip_ns(tag: str) -> str:
    return tag.rsplit("}", 1)[-1]


def _polygon_centre(points_attr: str) -> G.Point | None:
    nums = [float(v) for v in _NUM.findall(points_attr)]
    if len(nums) < 6:
        return None
    pts = list(zip(nums[0::2], nums[1::2]))
    return (sum(p[0] for p in pts) / len(pts), sum(p[1] for p in pts) / len(pts))


def load_svg(path: str, name: str | None = None) -> Level:
    """Recovers an editable `Level` from a game-format circuit SVG.

    Three things come back: the centreline (flattened, simplified, and given a
    best-fit fillet radius per node), the road half-width (read off the tarmac
    stroke rather than guessed, because a road authored at 2 cells and reopened
    at 3 is a *different level*, not a rounded one), and the feature marks
    (matched to the lap by their position along the recovered centreline).
    """
    tree = ET.parse(path)
    root = tree.getroot()

    outside = C.OUTSIDE
    runoff = C.GRASS
    road_half = 3.0
    ring_d: str | None = None
    rings: list[tuple[float, str, str]] = []   # (width, cap, d)
    polygons: list[tuple[str, G.Point]] = []

    for el in root.iter():
        tag = _strip_ns(el.tag)
        if tag == "rect":
            fill = (el.get("fill") or "").upper()
            if fill in C.RGB_CODE:
                outside = C.RGB_CODE[fill]
        elif tag == "path":
            d = el.get("d") or ""
            stroke = (el.get("stroke") or "").upper()
            cap = el.get("stroke-linecap") or "butt"
            try:
                w = float(el.get("stroke-width") or 0.0)
            except ValueError:
                w = 0.0
            if stroke == C.RGB[C.TARMAC].upper():
                rings.append((w, cap, d))
                if d.rstrip().endswith("Z") and ring_d is None:
                    ring_d = d
            elif stroke == C.RGB[C.WALL].upper() and w > 0:
                runoff = _infer_runoff(root, w)
        elif tag == "polygon":
            fill = (el.get("fill") or "").upper()
            centre_pt = _polygon_centre(el.get("points") or "")
            if centre_pt and fill in C.RGB_CODE:
                polygons.append((fill, centre_pt))

    if ring_d is None and rings:
        ring_d = max(rings, key=lambda r: r[0])[2]
    if ring_d is None:
        raise ValueError(
            f"{os.path.basename(path)}: no tarmac path found. This does not look "
            f"like a game-format circuit SVG -- the level format paints the road "
            f"as a {C.RGB[C.TARMAC]} stroke on the centreline, and nothing else "
            f"in the pipeline emits that by accident."
        )

    if rings:
        road_half = max(rings, key=lambda r: r[0])[0] / 2.0
    if not C.road_half_cells_legal(road_half):
        # A road width that is not a whole number of cells cannot be expressed
        # to the engine at all. Round rather than refuse: the file is still
        # worth opening, and the validator will say so if the difference matters.
        road_half = round(road_half)

    centreline = flatten_path(ring_d, G.DEFAULT_STEP)
    if len(centreline) < 8:
        raise ValueError(f"{os.path.basename(path)}: centreline is degenerate")

    closed = centreline + [centreline[0]]
    nodes_pts = G.simplify_closed(closed, SIMPLIFY_EPS)
    radii = estimate_radii(nodes_pts)

    level = Level(
        name=name or _svg_name(root) or os.path.splitext(os.path.basename(path))[0],
        nodes=[Node(x, y, r) for (x, y), r in zip(nodes_pts, radii)],
        mode=_detect_mode(ring_d),
        road_half_cells=road_half,
        runoff=runoff,
        outside=outside,
        infield=outside,
    )
    level.meta["source_svg"] = os.path.abspath(path)
    level.meta["imported"] = True
    level.meta["notes"] = _svg_note(root)
    level.marks = _recover_marks(centreline, polygons)
    if not level.marks:
        level.add_standard_marks()
    return level


def _detect_mode(d: str) -> str:
    """Which of the two modes a `d` was written in.

    Both parse identically, so the mode only decides how the nodes behave when
    edited afterwards. Getting it right means reopening a spline circuit and
    dragging a node moves the curve instead of moving a fillet handle.
    """
    return "spline" if "C" in d or "c" in d else "poly"


def _infer_runoff(root: ET.Element, wall_width: float) -> int:
    """The palette colour of the band just inside the barrier.

    The runoff is the widest stroke that is not the barrier and not the
    tarmac. Identifying it geometrically rather than by paint order means an
    SVG with the bands reordered still imports.
    """
    widths: list[tuple[float, int]] = []
    for el in root.iter():
        if _strip_ns(el.tag) != "path":
            continue
        stroke = (el.get("stroke") or "").upper()
        code = C.RGB_CODE.get(stroke)
        if code is None or code in (C.WALL, C.TARMAC, C.KERB_WHITE, C.KERB_RED):
            continue
        try:
            w = float(el.get("stroke-width") or 0.0)
        except ValueError:
            continue
        widths.append((w, code))
    if not widths:
        return C.GRASS
    return max(widths)[1]


def _svg_name(root: ET.Element) -> str:
    return ""


def _svg_note(root: ET.Element) -> str:
    for el in root:
        if not isinstance(el.tag, str) or not _strip_ns(el.tag).startswith(
            ("Comment", "comment")
        ):
            continue
    return ""


def estimate_radii(points: Sequence[G.Point]) -> list[float]:
    """Best-fit fillet radius per node, from local turn and adjacent chord length.

    Recovered rather than exact, because the nodes came from a *drawing* of a
    curve: a fillet polygon sampled and simplified back down loses the radius it
    was drawn with. The estimate uses half the shorter adjacent chord as the
    tangent distance, which is where a fillet sits when the neighbouring corners
    are not fighting it for the same edge.
    """
    n = len(points)
    out: list[float] = []
    for i in range(n):
        p = points[i]
        a = points[(i - 1) % n]
        b = points[(i + 1) % n]
        u_in = _unit((p[0] - a[0], p[1] - a[1]))
        u_out = _unit((b[0] - p[0], b[1] - p[1]))
        cross = u_in[0] * u_out[1] - u_in[1] * u_out[0]
        dot = max(-1.0, min(1.0, u_in[0] * u_out[0] + u_in[1] * u_out[1]))
        theta = abs(math.atan2(cross, dot))
        if theta < 1e-3:
            out.append(0.0)
            continue
        chord = min(math.dist(p, a), math.dist(p, b))
        t_dist = chord * 0.5
        out.append(t_dist / max(1e-3, math.tan(theta / 2.0)))
    return out


def _unit(v: G.Point) -> G.Point:
    m = math.hypot(v[0], v[1])
    return (v[0] / m, v[1] / m) if m > 1e-12 else (0.0, 0.0)


def _recover_marks(
    centreline: Sequence[G.Point], polygons: list[tuple[str, G.Point]]
) -> list[Mark]:
    """Turn drawn feature polygons back into marks on the lap.

    Matched by position along the centreline rather than by absolute
    coordinates, so a mark stays the same mark after the level is moved or
    scaled. A polygon that lands more than half a road-width off the centreline
    was drawn on a curb or in the runoff and is dropped: keeping it would put a
    boost pad in the gravel with a warning nobody reads.
    """
    if not centreline:
        return []
    marks: list[Mark] = []
    for fill, pt in polygons:
        code = C.RGB_CODE.get(fill)
        kind = _MARK_KIND_BY_CODE.get(code) if code is not None else None
        if kind is None:
            continue
        frac, dist = G.nearest_fraction(centreline, pt)
        if dist > 8.0:
            continue
        at, tangent = G.point_at_fraction(centreline, frac)
        lateral = (pt[0] - at[0]) * -tangent[1] + (pt[1] - at[1]) * tangent[0]
        marks.append(Mark(kind=kind, frac=frac, offset=round(lateral, 3)))
    return marks


_MARK_KIND_BY_CODE = {
    C.START_LINE: "start",
    C.GATE: "gate",
    C.BOOST: "boost",
    C.OIL: "oil",
}