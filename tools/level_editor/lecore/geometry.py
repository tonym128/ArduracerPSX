"""Centreline geometry: fillets, splines, arc-length, and the mark bands.

Two shape modes, because they answer different requests and only one of them
was already in the format:

* ``poly`` -- a closed polygon with a circular fillet per corner, emitted as
  ``M``/``A``/``L``/``Z``. This is byte-for-byte the grammar
  `generate_svg_circuit.py` already writes, so a level opened from a shipped
  circuit and saved again produces the same drawing.
* ``spline`` -- a closed centripetal Catmull-Rom through the same nodes,
  emitted as ``M``/``C``/``Z``. Genuinely smooth: curvature is continuous, not
  merely tangent-continuous, so a fast corner reads as a corner rather than a
  join.

Both walk the geometry in the same direction and emit the *same* dense sample
list, because `compile_circuit` places gates, the start line and boost pads by
**index into that list**. A centreline that runs the other way round, or a
sample list that disagrees with the drawn path, puts every pad in the runoff
while every image still validates -- the failure the pipeline's own comments
keep coming back to.
"""

from __future__ import annotations

import math
from typing import Iterable, Sequence

Point = tuple[float, float]

#: Sample spacing along the centreline, in cells. Matches the generator: fine
#: enough that `transverse_band` squares marks up to the local tangent rather
#: than to the chord across a long curve.
DEFAULT_STEP = 0.25

#: Samples per spline span used to fit each cubic. Kept for callers that fit
#: by interior samples; the tangent fit below needs no samples to be correct.
SPLINE_FIT_SAMPLES = 8

#: Node tangent magnitude ceiling, as a multiple of the shorter adjacent chord.
#: The centripetal spline's own derivative grows without bound as one adjacent
#: chord grows and the other shrinks, and `c1 = p1 + m1/3` then throws a control
#: point clean across the circuit -- the emitted path stops looking like the
#: shape it was drawn from. Capping at the shorter chord is what keeps a road
#: drawn with three tight corners and one long straight a road and not a
#: bowtie. One means "a tangent up to a full chord long", which is already
#: looser than the classic 1/3.
TANGENT_CAP = 1.0


# --- Small vector helpers ------------------------------------------------------


def _sub(a: Point, b: Point) -> Point:
    return (a[0] - b[0], a[1] - b[1])


def _add(a: Point, b: Point) -> Point:
    return (a[0] + b[0], a[1] + b[1])


def _mul(a: Point, k: float) -> Point:
    return (a[0] * k, a[1] * k)


def _len(a: Point) -> float:
    return math.hypot(a[0], a[1])


def _norm(a: Point) -> Point:
    m = _len(a)
    return (a[0] / m, a[1] / m) if m > 1e-12 else (0.0, 0.0)


def _dist(a: Point, b: Point) -> float:
    return math.hypot(a[0] - b[0], a[1] - b[1])


def dedupe(points: Iterable[Point], eps: float = 1e-6) -> list[Point]:
    """Drops consecutive duplicates, including across the closing seam.

    A duplicate vertex has no defined edge direction, and `fillet_geometry`
    divides by it. The generator asserts instead of handling it; an editor that
    deletes a node while its neighbour is selected produces exactly this a few
    times an hour, and an assertion is the wrong answer to a keystroke.
    """
    out: list[Point] = []
    for p in points:
        if not out or _dist(out[-1], p) > eps:
            out.append(p)
    while len(out) > 1 and _dist(out[0], out[-1]) <= eps:
        out.pop()
    return out


# --- Fillet polygon (the format's existing shape mode) -------------------------


def fillet_geometry(
    nodes: Sequence[Point],
    radii: Sequence[float],
    min_corner_radius_cells: float | None = None,
) -> tuple[list[dict], list[float]]:
    """Per-vertex fillet solution for a closed filleted polygon.

    Returns the per-vertex records and the incoming edge lengths. Kept separate
    from path emission so both the `poly` writer and the radius estimator can
    use it.

    `min_corner_radius_cells` is the smallest fillet radius the *road* can be
    drawn on: the road half-width plus its kerb, below which the inside edge of
    the corner folds through the centre of curvature. Pass it whenever the road
    width is known -- see the clamp below. `None` means "unknown", which is the
    pre-existing behaviour.
    """
    pts = dedupe(nodes)
    n = len(pts)
    if n < 3:
        raise ValueError(f"a centreline needs at least 3 nodes, got {n}")

    edges: list[Point] = []
    lengths: list[float] = []
    for i in range(n):
        seg = _sub(pts[(i + 1) % n], pts[i])
        L = _len(seg)
        lengths.append(L)
        edges.append(_mul(seg, 1.0 / L) if L > 1e-12 else (0.0, 0.0))

    fillets: list[dict] = []
    for i in range(n):
        p = pts[i]
        u_in = edges[(i - 1) % n]
        u_out = edges[i]
        cross = u_in[0] * u_out[1] - u_in[1] * u_out[0]
        dot = u_in[0] * u_out[0] + u_in[1] * u_out[1]
        theta = math.atan2(cross, dot)
        r = float(radii[i]) if i < len(radii) else 0.0

        if abs(theta) < 1e-4 or r <= 1e-6:
            # Collinear, or the corner was deliberately square. No arc; the
            # two straight runs simply meet at the vertex.
            fillets.append(
                {
                    "active": False,
                    "p_start": p,
                    "p_end": p,
                    "r": 0.0,
                    "sweep": 1 if cross > 0 else 0,
                    "centre": p,
                    "a_start": 0.0,
                    "d_ang": 0.0,
                    "vertex": p,
                    "theta": theta,
                }
            )
            continue

        tan_half = math.tan(abs(theta) / 2.0)
        t_dist = r * tan_half
        # Clamp to under half the shorter adjoining edge. An unclamped fillet
        # runs past the corner's neighbour, which produces a cusp -- the
        # geometry silently looks like a lap while containing a kink the car
        # hits. Clamping is the same rule the generator uses, for the same
        # reason.
        max_t = min(lengths[(i - 1) % n], lengths[i]) * 0.48
        # ...and to a radius the road can actually be drawn on.
        #
        # A fillet of radius `r` sweeps an arc whose *inner* edge has radius
        # `r - (road_half + kerb)`. When that goes negative the inner edge folds
        # through the centre of curvature: the road outline self-intersects and
        # the car hits a cusp it can see coming. Nothing enforced the lower
        # bound, and the edge-length clamp above only makes it *more* likely on a
        # short edge -- so on a narrow road, where the radius needed is already
        # small, corners folded through their own insides.
        #
        # `min_corner_radius_cells` is None for callers that do not know the road
        # width (SVG import, before the width is recovered), which keeps the old
        # behaviour rather than guessing a width.
        if min_corner_radius_cells is not None:
            r_min = min_corner_radius_cells
            if r < r_min:
                r = r_min
                t_dist = r * tan_half
        if t_dist > max_t:
            t_dist = max_t
            r = t_dist / max(1e-4, tan_half)

        p_start = _sub(p, _mul(u_in, t_dist))
        p_end = _add(p, _mul(u_out, t_dist))
        if cross > 0:
            n_in = (-u_in[1], u_in[0])
            sweep = 1
        else:
            n_in = (u_in[1], -u_in[0])
            sweep = 0
        centre = _add(p_start, _mul(n_in, r))
        a_start = math.atan2(p_start[1] - centre[1], p_start[0] - centre[0])
        a_end = math.atan2(p_end[1] - centre[1], p_end[0] - centre[0])
        d_ang = (
            (a_end - a_start) % (2 * math.pi)
            if sweep == 1
            else -((a_start - a_end) % (2 * math.pi))
        )
        fillets.append(
            {
                "active": True,
                "p_start": p_start,
                "p_end": p_end,
                "r": r,
                "sweep": sweep,
                "centre": centre,
                "a_start": a_start,
                "d_ang": d_ang,
                "vertex": p,
                "theta": theta,
            }
        )
    return fillets, lengths


def _arc_samples(f: dict, step: float, out: list[Point]) -> None:
    if not f["active"]:
        return
    arc_len = abs(f["d_ang"]) * f["r"]
    n = max(1, int(round(arc_len / step)))
    cx, cy = f["centre"]
    r = f["r"]
    a0 = f["a_start"]
    da = f["d_ang"]
    for k in range(n):
        a = a0 + da * (k / n)
        out.append((cx + r * math.cos(a), cy + r * math.sin(a)))


def _line_samples(a: Point, b: Point, step: float, out: list[Point]) -> None:
    seg = _dist(a, b)
    n = max(1, int(round(seg / step)))
    for k in range(n):
        t = k / n
        out.append((a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t))


def poly_path(
    nodes: Sequence[Point],
    radii: Sequence[float],
    step: float = DEFAULT_STEP,
    min_corner_radius_cells: float | None = None,
) -> tuple[str, list[Point]]:
    """Closed filleted polygon -> (SVG `d`, dense centreline samples).

    The `d` is exactly the grammar the shipped circuits use: `M`, one `A` per
    active fillet, `L` to the next fillet's start, `Z`.

    `min_corner_radius_cells` is forwarded to [`fillet_geometry`]; see there for
    why a corner tighter than the road is not a corner.
    """
    pts = dedupe(nodes)
    fillets, _ = fillet_geometry(
        pts,
        list(radii) + [0.0] * max(0, len(pts) - len(radii)),
        min_corner_radius_cells,
    )
    n = len(pts)

    cmds = [f"M {fillets[0]['p_start'][0]:.4f} {fillets[0]['p_start'][1]:.4f}"]
    samples: list[Point] = []

    for i in range(n):
        f = fillets[i]
        if f["active"]:
            cmds.append(
                f"A {f['r']:.4f} {f['r']:.4f} 0 0 {f['sweep']} "
                f"{f['p_end'][0]:.4f} {f['p_end'][1]:.4f}"
            )
        nxt = fillets[(i + 1) % n]
        cmds.append(f"L {nxt['p_start'][0]:.4f} {nxt['p_start'][1]:.4f}")

        _arc_samples(f, step, samples)
        _line_samples(f["p_end"], nxt["p_start"], step, samples)

    cmds.append("Z")
    return " ".join(cmds), samples


# --- Closed centripetal Catmull-Rom (the smooth mode) -------------------------


def _catmull_rom(p0: Point, p1: Point, p2: Point, p3: Point, t: float,
                 alpha: float = 0.5) -> Point:
    """One span of a centripetal Catmull-Rom spline (Barry-Goldman form).

    Centripetal rather than uniform because uniform Catmull-Rom overshoots and
    forms cusps wherever nodes bunch up -- and node spacing here is whatever
    the person drawing chose, so bunching is guaranteed rather than possible.
    A cusp looks like a pinched road and drives like a wall.
    """
    def knot(ti: float, pa: Point, pb: Point) -> float:
        return ti + _dist(pa, pb) ** alpha

    t0 = 0.0
    t1 = knot(t0, p0, p1)
    t2 = knot(t1, p1, p2)
    t3 = knot(t2, p2, p3)
    t = t1 + (t2 - t1) * t

    def mix(a: Point, b: Point, ta: float, tb: float) -> Point:
        if abs(tb - ta) < 1e-12:
            return a
        k = (t - ta) / (tb - ta)
        return (a[0] + (b[0] - a[0]) * k, a[1] + (b[1] - a[1]) * k)

    a1 = mix(p0, p1, t0, t1)
    a2 = mix(p1, p2, t1, t2)
    a3 = mix(p2, p3, t2, t3)
    b1 = mix(a1, a2, t0, t2)
    b2 = mix(a2, a3, t1, t3)
    return mix(b1, b2, t1, t2)


def _bezier(p0: Point, c1: Point, c2: Point, p1: Point, t: float) -> Point:
    u = 1.0 - t
    a = u * u * u
    b = 3 * u * u * t
    c = 3 * u * t * t
    d = t * t * t
    return (
        a * p0[0] + b * c1[0] + c * c2[0] + d * p1[0],
        a * p0[1] + b * c1[1] + c * c2[1] + d * p1[1],
    )


def _cr_tangent(nodes: Sequence[Point], j: int, eps: float = 1e-3) -> Point:
    """The spline's tangent at `nodes[j]`, in units of cells per unit parameter.

    Central difference across the join rather than one-sided, because the node
    is where the two adjoining spans have to agree; measuring on one side only
    and using it for both leaves a tangent discontinuity that a stroke join
    turns into a visible crease exactly at the corner you were drawing.
    """
    n = len(nodes)
    before = _catmull_rom(
        nodes[(j - 1) % n], nodes[j], nodes[(j + 1) % n], nodes[(j + 2) % n],
        1.0 - eps,
    )
    after = _catmull_rom(
        nodes[j], nodes[(j + 1) % n], nodes[(j + 2) % n], nodes[(j + 3) % n],
        eps,
    )
    return ((after[0] - before[0]) / (2 * eps), (after[1] - before[1]) / (2 * eps))


def spline_path(
    nodes: Sequence[Point],
    step: float = DEFAULT_STEP,
    fit_samples: int = SPLINE_FIT_SAMPLES,
) -> tuple[str, list[Point]]:
    """Closed centripetal Catmull-Rom -> (SVG `d` of cubics, samples).

    Each span becomes one cubic Hermite built from the spline's own clamped
    tangents: `c1 = p1 + m1/3`, `c2 = p2 - m2/3`. Tangents are computed once
    per node and clamped to a ceiling derived from *both* adjacent chords, so
    the same clamped vector feeds the span before and the span after -- a
    per-span clamp would break the very continuity the mode exists to provide.

    The samples are then taken from the emitted Beziers rather than from the
    spline they were fitted to, so `d` and the centreline sidecar describe the
    same curve to the last decimal. The pipeline's own comments are full of
    levels whose drawing and sidecar disagreed: every image validated and every
    boost pad sat in the grass.
    """
    pts = dedupe(nodes)
    n = len(pts)
    if n < 3:
        raise ValueError(f"a centreline needs at least 3 nodes, got {n}")
    if n == 3:
        # Three spans of one third each: every cubic is exact, and the
        # interior-fit solve is singular (the two interior samples coincide
        # with the endpoints). Fall back to a circle through the three nodes.
        return _triangle_path(pts, step)

    tangents: list[Point] = []
    for j in range(n):
        t = _cr_tangent(pts, j)
        chord = min(
            _dist(pts[j], pts[(j - 1) % n]),
            _dist(pts[j], pts[(j + 1) % n]),
        )
        cap = chord * TANGENT_CAP
        m = _len(t)
        tangents.append(_mul(t, cap / m) if m > 1e-12 else (0.0, 0.0))

    cmds = [f"M {pts[0][0]:.4f} {pts[0][1]:.4f}"]
    samples: list[Point] = []
    third = 1.0 / 3.0

    for i in range(n):
        p1 = pts[i]
        p2 = pts[(i + 1) % n]
        c1 = _add(p1, _mul(tangents[i], third))
        c2 = _sub(p2, _mul(tangents[(i + 1) % n], third))
        cmds.append(
            f"C {c1[0]:.4f} {c1[1]:.4f} {c2[0]:.4f} {c2[1]:.4f} "
            f"{p2[0]:.4f} {p2[1]:.4f}"
        )
        _bezier_samples(p1, c1, c2, p2, step, samples)

    cmds.append("Z")
    return " ".join(cmds), samples


def _triangle_path(pts: list[Point], step: float) -> tuple[str, list[Point]]:
    """Circumscribed circle through three nodes, as four cubics.

    A three-node spline is underdetermined; the circle is the only answer that
    closes and stays smooth, and it is what a triangle's "round the corners"
    means to a driver anyway.
    """
    (x1, y1), (x2, y2), (x3, y3) = pts
    d = 2 * (x1 * (y2 - y3) + x2 * (y3 - y1) + x3 * (y1 - y2))
    if abs(d) < 1e-9:
        return poly_path(pts, [0.0, 0.0, 0.0], step)
    s1 = x1 * x1 + y1 * y1
    s2 = x2 * x2 + y2 * y2
    s3 = x3 * x3 + y3 * y3
    ux = (s1 * (y2 - y3) + s2 * (y3 - y1) + s3 * (y1 - y2)) / d
    uy = (s1 * (x3 - x2) + s2 * (x1 - x3) + s3 * (x2 - x1)) / d
    cx, cy = ux, uy
    r = _dist((cx, cy), (x1, y1))
    a0 = math.atan2(y1 - cy, x1 - cx)
    da = math.atan2(y3 - y1, x3 - x1) - math.atan2(y2 - y1, x2 - x1)
    while da <= -math.pi:
        da += 2 * math.pi
    while da > math.pi:
        da -= 2 * math.pi

    k = 4.0 / 3.0 * math.tan(da / 8.0)
    cmds: list[str] = []
    samples: list[Point] = []
    for i in range(4):
        a = a0 + da * (i / 4.0)
        p = (cx + r * math.cos(a), cy + r * math.sin(a))
        b = a + da / 4.0
        tan = (-math.sin(b), math.cos(b))
        nxt = (cx + r * math.cos(b), cy + r * math.sin(b))
        c1 = (p[0] + k * r * tan[0], p[1] + k * r * tan[1])
        c2 = (nxt[0] - k * r * tan[0], nxt[1] - k * r * tan[1])
        cmds.append(
            f"{'M' if i == 0 else 'C'} {p[0]:.4f} {p[1]:.4f} "
            f"{c1[0]:.4f} {c1[1]:.4f} {c2[0]:.4f} {c2[1]:.4f} "
            f"{nxt[0]:.4f} {nxt[1]:.4f}"
        )
        _bezier_samples(p, c1, c2, nxt, step, samples)
    cmds.append("Z")
    return " ".join(cmds), samples


def _bezier_samples(p0: Point, c1: Point, c2: Point, p1: Point,
                    step: float, out: list[Point]) -> None:
    """Sample a span at no more than `step` cells apart, and finely enough to
    follow it.

    Two bounds, and the second is the one that matters. The flatness bound
    stops the polyline faceting the curve, but a long gentle arc is flat *and*
    long: a thirteen-cell span between two mild corners has a flatness of about
    one cell, so flatness alone samples it five times. That is a legible curve
    and a useless centreline -- the sidecar, the gates, the boost pads and the
    route control points are all placed by *index* into this list, so a
    four-times-coarser list puts four times the error on each of them, and a
    boost pad placed at fraction 0.21 lands a third of a cell from where it
    was asked to land. Chord length is what bounds the error, so both apply.
    """
    flat = _bezier_flatness(p0, c1, c2, p1)
    chord = _dist(p0, p1)
    n = max(int(math.ceil(flat / step)), int(math.ceil(chord / step))) + 1
    n = max(2, min(96, n))
    for k in range(n):
        out.append(_bezier(p0, c1, c2, p1, k / n))


def _bezier_flatness(p0: Point, c1: Point, c2: Point, p1: Point) -> float:
    """Upper bound on how far the curve strays from its chord.

    A cubic lies inside the convex hull of its four control points, so the
    larger perpendicular distance of `c1` and `c2` from the chord bounds the
    whole span. Cheaper than solving for the stationary point and, unlike the
    usual `max(|p0-c1|, |c1-c2|, |c2-p1|)` heuristic, it does not call a gentle
    arc 40 samples long because its chord happens to be short.
    """
    dx, dy = p1[0] - p0[0], p1[1] - p0[1]
    m = math.hypot(dx, dy)
    if m < 1e-12:
        return max(_dist(p0, c1), _dist(p0, c2))
    return max(
        abs(dy * (c1[0] - p0[0]) - dx * (c1[1] - p0[1])) / m,
        abs(dy * (c2[0] - p0[0]) - dx * (c2[1] - p0[1])) / m,
    )


def shape_path(
    nodes: Sequence[Point], radii: Sequence[float], mode: str,
    step: float = DEFAULT_STEP,
    min_corner_radius_cells: float | None = None,
) -> tuple[str, list[Point]]:
    """Dispatch for the two shape modes. The only place `mode` is branched on.

    The sample list is closed before it is returned. Leaving the ring open --
    which is what the generator does -- makes the last walk's length vanish
    from every arc-length sum, so a mark placed at fraction 0.99 lands short
    and a resampled route is a fraction of a cell short of closing. The
    compiler pops the trailing duplicate back off (`emit_rust`).

    `min_corner_radius_cells` applies to `poly` mode only: a spline has no
    fillets, and its curvature is already whatever the control points make it.
    """
    if mode == "spline":
        d, samples = spline_path(nodes, step)
    elif mode == "poly":
        d, samples = poly_path(nodes, radii, step, min_corner_radius_cells)
    else:
        raise ValueError(
            f"unknown centreline mode {mode!r} (expected 'poly' or 'spline')"
        )
    if samples:
        samples = samples + [samples[0]]
    return d, samples


# --- Arc length, resampling, simplification ------------------------------------


def path_length(points: Sequence[Point]) -> float:
    total = 0.0
    for i in range(len(points)):
        total += _dist(points[i], points[(i + 1) % len(points)])
    return total


def resample_closed(points: Sequence[Point], n: int) -> list[Point]:
    """`n` points at equal arc length around a closed polyline.

    Index-based decimation is not arc-length uniform, and index-based marks
    bunch up on the tight corners and stretch on the straights -- so gates
    cluster where the lap is hardest and the start line lands at an arbitrary
    point of the grid.
    """
    pts = dedupe(points)
    if len(pts) < 3 or n < 3:
        return list(pts)
    segs: list[float] = []
    for i in range(len(pts)):
        segs.append(_dist(pts[i], pts[(i + 1) % len(pts)]))
    total = sum(segs)
    if total <= 0:
        return list(pts)

    out: list[Point] = []
    target = 0.0
    acc = 0.0
    i = 0
    step_len = total / n
    for _ in range(n):
        while i < len(segs) - 1 and acc + segs[i] < target:
            acc += segs[i]
            i += 1
        t = (target - acc) / segs[i] if segs[i] > 1e-12 else 0.0
        a, b = pts[i], pts[(i + 1) % len(pts)]
        out.append((a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t))
        target += step_len
    return out


def point_at_fraction(points: Sequence[Point], frac: float) -> tuple[Point, Point]:
    """The point at `frac` round the lap, and the unit tangent there.

    Returns a tangent rather than a chord to the next sample so a mark squares
    up to the local direction of travel. Assuming `x` or `y` is what puts a
    boost pad in the runoff on any curve, and it still validates, because the
    pad happens to touch drivable surface.
    """
    n = len(points)
    if n < 2:
        raise ValueError("need at least two centreline samples")
    f = (frac % 1.0) * n
    i = int(f) % n
    t = f - int(f)
    a = points[i]
    b = points[(i + 1) % n]
    at = (a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t)
    return at, _norm(_sub(b, a))


def _rdp(points: Sequence[Point], eps: float) -> list[int]:
    """Indices kept by Ramer-Douglas-Peucker on an *open* polyline."""
    n = len(points)
    if n < 3:
        return list(range(n))
    keep = [False] * n
    keep[0] = keep[n - 1] = True
    stack: list[tuple[int, int]] = [(0, n - 1)]
    worst = 0
    while stack:
        lo, hi = stack.pop()
        if hi <= lo + 1:
            continue
        ax, ay = points[lo]
        bx, by = points[hi]
        dx, dy = bx - ax, by - ay
        m = math.hypot(dx, dy)
        worst_d = 0.0
        for k in range(lo + 1, hi):
            px, py = points[k]
            if m < 1e-12:
                d = math.hypot(px - ax, py - ay)
            else:
                d = abs(dy * (px - ax) - dx * (py - ay)) / m
            if d > worst_d:
                worst_d, worst = d, k
        if worst_d > eps and worst > 0:
            keep[worst] = True
            stack.append((lo, worst))
            stack.append((worst, hi))
    return [i for i in range(n) if keep[i]]


def simplify_closed(points: Sequence[Point], eps: float) -> list[Point]:
    """Few nodes that still draw the same shape.

    Split at the two furthest-apart samples so each half is an open polyline,
    which is what RDP needs; running RDP on a closed loop without the split
    keeps whichever pair the seed scan happened to find and may lose the
    opposite side entirely.
    """
    pts = dedupe(points)
    n = len(pts)
    if n < 8:
        return list(pts)
    a, b = 0, 0
    best = -1.0
    for i in range(n):
        for j in range(i + 1, n):
            d = _dist(pts[i], pts[j])
            if d > best:
                best, a, b = d, i, j
    half_a = _rdp(pts[a : b + 1], eps)
    half_b = _rdp(pts[b:] + pts[: a + 1], eps)
    picked = [a + k for k in half_a] + [(b + k) % n for k in half_b[1:-1]]
    picked = sorted(set(picked))
    if len(picked) < 3:
        picked = list(range(0, n, max(1, n // 12)))
    return [pts[i] for i in picked]


def smooth_closed(points: Sequence[Point], iterations: int = 1,
                  strength: float = 0.5) -> list[Point]:
    """Laplacian relaxation over the node ring -- "tweak this line".

    Each node moves toward the midpoint of its neighbours, which is the
    cheapest thing that turns a jittery hand-drawn ring into a drivable one
    without shrinking it the way repeated freehand smoothing does: the ring's
    centroid is unchanged, because the operator is a rotation of the
    coordinates and every row sums to zero.
    """
    pts: list[Point] = [(float(p[0]), float(p[1])) for p in points]
    n = len(pts)
    if n < 3:
        return pts
    k = max(0.0, min(1.0, strength))
    for _ in range(iterations):
        out = []
        for i in range(n):
            a = pts[(i - 1) % n]
            b = pts[(i + 1) % n]
            out.append((
                pts[i][0] + ((a[0] + b[0]) / 2 - pts[i][0]) * k,
                pts[i][1] + ((a[1] + b[1]) / 2 - pts[i][1]) * k,
            ))
        pts = out
    return pts


def offset_closed(points: Sequence[Point], d: float) -> list[Point]:
    """The centreline displaced `d` cells along its left normal.

    Used for measuring where a mark lands across the road and for the editor's
    width readout. Not used to *draw* the road: a stroked path is exact and
    free, whereas a displaced polyline pinches at every corner.
    """
    n = len(points)
    out: list[Point] = []
    for i in range(n):
        a = points[(i - 1) % n]
        b = points[(i + 1) % n]
        t = _norm(_sub(b, a))
        out.append((points[i][0] - t[1] * d, points[i][1] + t[0] * d))
    return out


def bbox(points: Sequence[Point]) -> tuple[float, float, float, float]:
    xs = [p[0] for p in points]
    ys = [p[1] for p in points]
    return (min(xs), min(ys), max(xs), max(ys))


# --- Marks --------------------------------------------------------------------


def band_points(
    at: Point, tangent: Point, half_len: float, half_thick: float,
    offset: float = 0.0
) -> list[Point]:
    """Four corners of a band squared up to the road at `at`.

    The long axis is the road's *normal*, so the band crosses the road at right
    angles; `offset` slides it sideways, which is how two boost pads on one
    straight are expressed as a left-right pair rather than a row of three.
    """
    tx, ty = tangent
    nx, ny = -ty, tx
    cx = at[0] + nx * offset
    cy = at[1] + ny * offset
    corners = []
    for a, b in ((-1, -1), (1, -1), (1, 1), (-1, 1)):
        corners.append((
            cx + nx * half_len * a + tx * half_thick * b,
            cy + ny * half_len * a + ty * half_thick * b,
        ))
    return corners


def band_polygon(
    at: Point, tangent: Point, half_len: float, half_thick: float,
    offset: float = 0.0, precision: int = 4
) -> str:
    """`points` attribute for a `<polygon>` band."""
    pts = band_points(at, tangent, half_len, half_thick, offset)
    return " ".join(f"{x:.{precision}f},{y:.{precision}f}" for x, y in pts)


def nearest_fraction(points: Sequence[Point], p: Point) -> tuple[float, float]:
    """Where on the lap `p` lands, as a fraction, and how far away it is.

    Snapped to the nearest sample rather than interpolated: a mark placed by
    clicking the road should land on a whole sample index, because that is what
    the sidecar and the emitted polygon agree about.
    """
    best_i, best_d = 0, float("inf")
    for i, q in enumerate(points):
        d = (q[0] - p[0]) ** 2 + (q[1] - p[1]) ** 2
        if d < best_d:
            best_d, best_i = d, i
    return best_i / len(points), math.sqrt(best_d)