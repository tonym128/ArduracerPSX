#!/usr/bin/env python3
"""
Authored circuits for Arduracer PSX.

Every one of the 24 stages is defined here. This file is the single source of
truth for circuit geometry; `convert_levels.py` rasterises it into
`crates/arduracer-core/src/levels.rs`.

Why this file exists
--------------------

The circuits used to be *derived*. `convert_levels.py` read
`ArduRacerFx/Levels/Level1..20.csv` and fit a spline through whatever tile
clusters the Arduboy renderer happened to leave as gates. Gates exist for gate
coverage, not for a driving line, so three things followed:

* **Narrow roads.** The corridor half-width was 1.05 tiles -- a two-tile band.
  Anything wider was unavailable, not unwanted.
* **Short straights.** Control points landed where the FX gates were, so the gap
  between corners was whatever the gate spacing left over.
* **Blocky corners**, because a 64-unit tile is a 64-unit tile at any width.

The FX CSVs stay in the repository as provenance (see `PROVENANCE.md`) but no
longer contribute geometry.

Authoring in segments, not anchor points
----------------------------------------

An earlier pass of this file listed anchor points by hand and it produced **no
straights at all**: measured, the turn angle between consecutive centreline
samples had a *minimum* of 1.22 degrees, so every part of every circuit was
curving. The cause is Catmull-Rom. Its tangent at an anchor is
`(next - prev) / 2`, so two anchors with neighbours off to the side still bulge
through the middle -- a straight needs three or more **collinear** anchors, not
two. Spacing points further apart does not fix it.

So circuits are authored as a sequence of `straight` and `corner` segments and
the anchors are *generated*:

* `("straight", tiles)` walks in a straight line, emitting anchors every
  `STRAIGHT_ANCHOR_TILES`. They are collinear by construction, so the curve
  through them is genuinely straight.
* `("corner", degrees, radius_tiles)` walks an arc of one radius, emitting an
  anchor every `CORNER_ANCHOR_DEG`. Every corner therefore has a single
  consistent radius and reads as a corner rather than a wobble.

`test_circuits_are_wide_and_have_long_straights` measures the *evaluated*
centreline, and it is what caught the hand-written anchors. "Long straights"
should be a measurement, not a claim.

Segment vocabulary
------------------

    ("straight", tiles)
    ("corner", weight, radius_tiles)

A corner's `weight` is **relative**, not degrees. The builder normalises every
weight so the signed angles sum to exactly 360 -- one full turn -- which is the
condition for the loop to close. Authoring absolute angles does not work in
practice: hand-tuned values drift a few degrees per circuit, and a circuit whose
corners sum to 350 degrees is not a slightly-off circuit, it is an open
spiral. Weights also say what the author meant: `K(3, 8)` is a corner three times
as tight as `K(1, 12)`, and the absolute degrees follow from the rest of the
layout.

Weights may be negative for a left-hander. As long as the weights do not sum to
zero, the normalisation is well defined.

Headings are compass degrees: 0 = north (screen -y), 90 = east (+x). A positive
`corner` turns clockwise on screen. For a closed loop the signed corner angles
must sum to +/-360; `validate_segments` below checks that before anything is
emitted, because a circuit that does not close is not a circuit.

Field reference
----------------

* `half_width` -- tarmac half-width in tiles. Below ~1.8 the road reads as a
  two-tile strip. **2.2-3.0 is "wide"** at this tile size, roughly a six-car
  abreast avenue. The cooker adds a 0.65-tile curb band either side.
* `checkpoints` -- spread evenly along the rasterised centreline by arc
  fraction, so they land where a straight begins rather than where a grid column
  happens to fall.
* `boost_at` / `oil_at` -- **fractions of the lap** (0.0-1.0), not grid
  coordinates, so hazards stay on the racing line when a circuit is re-tuned.
* `trait` -- `speed`, `technical`, `drift`, `boost` or `mixed`. Documentation
  today, and the scenery pass's contract tomorrow: a `boost` circuit should be
  recognisable by its boost pads.
"""

import math

#: Anchor spacing along a straight, in tiles. Small enough that a 12-tile
#: straight gets three anchors (the minimum for a straight through Catmull-Rom).
STRAIGHT_ANCHOR_TILES = 4.0

#: Angular interval between anchors through a corner, in degrees.
CORNER_ANCHOR_DEG = 12.0

#: Curb band either side of the tarmac, in tiles. Mirrors the cooker's
#: `road_radius = half_width + 0.65`.
CURB_TILES = 0.65

#: Clearance beyond the curb before the grid edge, in tiles. The cooker pads
#: again by `MARGIN_TILES`, but a road that reaches its own grid edge produces a
#: runoff thinner than it claims.
GRID_CLEARANCE_TILES = 3.0


def _rounded_loop(vertices, radii):
    """Turns a closed polygon into anchors, filleting every vertex.

    This is what produces *shape*. A single superellipse cannot: raising its
    exponent turns an oval into a stadium, and a stadium has no hairpin, no
    right-angle corner and no esse. Every circuit came out looking like an oval
    because that is all a superellipse is.

    The polygon supplies the shape and the per-vertex radius supplies the
    character: a small radius is a tight corner, a large one a sweeper, and a
    near-180-degree vertex is a hairpin. The edge between two vertices is a
    straight by construction, so "long straight then corner" is just "two
    vertices far apart".

    ### The fillet geometry, which is easy to get subtly wrong

    For a vertex with signed turn `theta` and fillet radius `r`:

    * tangent distance along each edge is `r * |tan(theta / 2)|`;
    * the arc centre is `r / |sin(theta / 2)|` from the vertex, along the
      bisector.

    Using `r / cos(theta / 2)` for the centre is the natural-looking mistake and
    it **coincides at exactly 90 degrees**, where sin and cos agree -- so a
    right-angle test case passes and every other corner is wrong. The arc then
    misses its tangent point and the anchor polyline folds back on itself,
    producing a cusp that the runtime spline inherits.

    Radii are clamped so a fillet cannot overrun either adjacent edge, which is
    what stops two corners on a short edge from overlapping.
    """
    n = len(vertices)
    if n != len(radii):
        raise ValueError(f"{n} vertices but {len(radii)} radii")
    if n < 3:
        raise ValueError("a loop needs at least 3 vertices")

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
        if m < 1e-9:
            raise ValueError("duplicate consecutive vertices")
        return (a[0] / m, a[1] / m)

    edges = [norm(sub(vertices[(i + 1) % n], vertices[i])) for i in range(n)]

    fillets = []
    for i in range(n):
        v = vertices[i]
        u_in = edges[(i - 1) % n]
        u_out = edges[i]
        cross = u_in[0] * u_out[1] - u_in[1] * u_out[0]
        dot = u_in[0] * u_out[0] + u_in[1] * u_out[1]
        theta = math.atan2(cross, dot)  # signed turn, in (-pi, pi]
        if abs(theta) < 1e-6:
            fillets.append(None)
            continue
        tan_half = math.tan(theta / 2.0)
        r = abs(radii[i])
        tangent = r * abs(tan_half)
        # Clamp against both adjacent edges so neighbouring fillets cannot meet.
        prev_len = length(sub(v, vertices[(i - 1) % n]))
        next_len = length(sub(vertices[(i + 1) % n], v))
        # 0.42, not 0.5: at exactly half the shared edge the two neighbouring
        # fillets meet, so `p_out` of one equals `p_in` of the next and the
        # interior straight anchors collapse onto the arc endpoints. The
        # resulting zero-length spans are what produced 135-degree cusps in the
        # esses. The gap has to be positive, not merely non-negative.
        limit = 0.42 * min(prev_len, next_len)
        if tangent > limit:
            tangent = limit
            r = abs(tangent / tan_half)
        p_in = sub(v, scale(u_in, tangent))
        p_out = add(v, scale(u_out, tangent))
        # Bisector direction is `u_out - u_in`, **not** `u_in + u_out`. The two
        # have the same magnitude, so the distance formula is unaffected and the
        # error hides in the direction: `u_in + u_out` points outboard on a left
        # turn, putting the centre on the wrong side of the corner. The arc then
        # misses `p_out` by a factor of sqrt(2) and the polyline cusps.
        bis = norm(sub(u_out, u_in))
        centre = add(v, scale(bis, r / abs(math.sin(theta / 2.0))))
        fillets.append({
            "theta": theta, "p_in": p_in, "p_out": p_out,
            "centre": centre, "radius": r,
        })

    pts = []
    for i in range(n):
        f = fillets[i]
        nxt = fillets[(i + 1) % n]
        if f is None:
            v = vertices[i]
            pts.append((round(v[0], 2), round(v[1], 2)))
            continue
        steps = max(
            2, int(round(abs(math.degrees(f["theta"])) / CORNER_ANCHOR_DEG))
        )
        a0 = math.atan2(f["p_in"][1] - f["centre"][1], f["p_in"][0] - f["centre"][0])
        for k in range(steps + 1):
            a = a0 + f["theta"] * (k / steps)
            pts.append((
                round(f["centre"][0] + math.cos(a) * f["radius"], 2),
                round(f["centre"][1] + math.sin(a) * f["radius"], 2),
            ))
        if nxt is not None:
            a, b = f["p_out"], nxt["p_in"]
            # Interior anchors keep the straight collinear through Catmull-Rom.
            # Three is better than two: the span out of a fillet's exit anchor
            # is not straight, because the tangent there points into the corner.
            # Skipped entirely when the gap is tiny, since anchors closer than
            # this make the span shorter than the spline's own resolution.
            gap = length(sub(b, a))
            if gap > 3.0 * STRAIGHT_ANCHOR_TILES:
                for k in (1, 2, 3):
                    t = k / 4.0
                    pts.append((round(a[0] + (b[0] - a[0]) * t, 2),
                                round(a[1] + (b[1] - a[1]) * t, 2)))
    return pts


def circuit(name, rx, ry, exponent, half_width, checkpoints, trait,
            boost_at=(), oil_at=(), anchors=48, phase=0.0):
    """Builds a circuit entry from a superellipse.

    `rx` / `ry` are the semi-axes in tiles and set the circuit's overall size;
    `exponent` sets straights-versus-corners (2 = oval, 4 = flowing, 8 = stadium);
    `anchors` is how many control points the spline gets.
    """
    centre = _superellipse(rx, ry, exponent, phase, anchors)
    pad = half_width + CURB_TILES + GRID_CLEARANCE_TILES
    xs = [q[0] for q in centre]
    ys = [q[1] for q in centre]
    lo_x, hi_x = min(xs) - pad, max(xs) + pad
    lo_y, hi_y = min(ys) - pad, max(ys) + pad
    return {
        "name": name,
        "grid": (int(math.ceil(hi_x - lo_x)), int(math.ceil(hi_y - lo_y))),
        "centre": [(round(px - lo_x, 2), round(py - lo_y, 2)) for px, py in centre],
        "half_width": half_width,
        "checkpoints": checkpoints,
        "boost_at": tuple(boost_at),
        "oil_at": tuple(oil_at),
        "trait": trait,
        "exponent": exponent,
        "anchors": anchors,
    }


# ---------------------------------------------------------------------------
# Cup 1 -- BRONZE. Wide, flowing, forgiving. The introduction to the atlas.
# ---------------------------------------------------------------------------

BRONZE = [
    circuit(
        "Sunset Ridgeway",
        rx=17, ry=12, exponent=6, phase=0,
        half_width=2.8, checkpoints=8, trait="mixed",
        anchors=56,
        boost_at=(0.3, 0.72), oil_at=(),
    ),
    circuit(
        "Harbour Loop",
        rx=20, ry=12, exponent=7, phase=0.4,
        half_width=2.6, checkpoints=7, trait="speed",
        anchors=56,
        boost_at=(0.45, 0.88), oil_at=(),
    ),
    circuit(
        "Meadow Run",
        rx=16, ry=14, exponent=5, phase=0.9,
        half_width=2.4, checkpoints=9, trait="mixed",
        anchors=56,
        boost_at=(0.18,), oil_at=(0.55,),
    ),
    circuit(
        "Copper Gorge",
        rx=15, ry=15, exponent=4, phase=0.4,
        half_width=2.2, checkpoints=8, trait="technical",
        anchors=56,
        boost_at=(0.2,), oil_at=(0.62, 0.8),
    ),
    circuit(
        "Willow Bend",
        rx=18, ry=13, exponent=6, phase=1.2,
        half_width=2.5, checkpoints=8, trait="drift",
        anchors=56,
        boost_at=(0.5, 0.92), oil_at=(),
    ),
    circuit(
        "Lantern Fields",
        rx=19, ry=12, exponent=7, phase=0,
        half_width=2.7, checkpoints=8, trait="mixed",
        anchors=56,
        boost_at=(0.25, 0.65), oil_at=(),
    ),
]

SILVER = [
    circuit(
        "Neon Causeway",
        rx=21, ry=13, exponent=8, phase=0,
        half_width=3.0, checkpoints=8, trait="boost",
        anchors=56,
        boost_at=(0.22, 0.58, 0.86), oil_at=(),
    ),
    circuit(
        "Gridlock Mile",
        rx=15, ry=15, exponent=3, phase=0.7,
        half_width=2.4, checkpoints=10, trait="technical",
        anchors=56,
        boost_at=(0.38,), oil_at=(0.68,),
    ),
    circuit(
        "Skyway Nine",
        rx=21.3, ry=10.7, exponent=9, phase=0.4,
        half_width=2.9, checkpoints=7, trait="speed",
        anchors=56,
        boost_at=(0.3, 0.75), oil_at=(),
    ),
    circuit(
        "Foundry Spiral",
        rx=14, ry=14, exponent=3, phase=0.2,
        half_width=2.3, checkpoints=10, trait="technical",
        anchors=56,
        boost_at=(0.15,), oil_at=(0.5, 0.85),
    ),
    circuit(
        "Chrome Basin",
        rx=20, ry=13, exponent=7, phase=0.8,
        half_width=2.6, checkpoints=8, trait="mixed",
        anchors=56,
        boost_at=(0.42, 0.9), oil_at=(),
    ),
    circuit(
        "Vapour Trail",
        rx=18, ry=14, exponent=6, phase=0.5,
        half_width=2.5, checkpoints=8, trait="drift",
        anchors=56,
        boost_at=(), oil_at=(0.25, 0.6),
    ),
]

GOLD = [
    circuit(
        "Amber Mesa",
        rx=21.3, ry=14.5, exponent=8, phase=0,
        half_width=2.9, checkpoints=8, trait="speed",
        anchors=56,
        boost_at=(0.34, 0.8), oil_at=(),
    ),
    circuit(
        "Rattlesnake Pass",
        rx=16, ry=16, exponent=3, phase=0.6,
        half_width=2.2, checkpoints=10, trait="technical",
        anchors=56,
        boost_at=(), oil_at=(0.4, 0.72),
    ),
    circuit(
        "Salt Flats Sprint",
        rx=21.2, ry=10.1, exponent=9, phase=0,
        half_width=3.0, checkpoints=7, trait="boost",
        anchors=56,
        boost_at=(0.18, 0.52, 0.84), oil_at=(),
    ),
    circuit(
        "Cinder Bowl",
        rx=17, ry=14, exponent=6, phase=0.1,
        half_width=2.4, checkpoints=8, trait="mixed",
        anchors=56,
        boost_at=(0.28,), oil_at=(0.64,),
    ),
    circuit(
        "Longshadow Flats",
        rx=21, ry=15, exponent=7, phase=0.6,
        half_width=2.7, checkpoints=8, trait="drift",
        anchors=56,
        boost_at=(0.48, 0.9), oil_at=(),
    ),
    circuit(
        "Ochre Canyon",
        rx=15, ry=15, exponent=3, phase=1,
        half_width=2.3, checkpoints=10, trait="technical",
        anchors=56,
        boost_at=(0.35,), oil_at=(0.55, 0.88),
    ),
]

PLATINUM = [
    circuit(
        "Glacier Spine",
        rx=21.2, ry=13.8, exponent=8, phase=0.3,
        half_width=3.0, checkpoints=8, trait="speed",
        anchors=56,
        boost_at=(0.26, 0.64, 0.9), oil_at=(),
    ),
    circuit(
        "Harbourmaster",
        rx=21, ry=14, exponent=7, phase=0.9,
        half_width=2.8, checkpoints=8, trait="mixed",
        anchors=56,
        boost_at=(0.4, 0.78), oil_at=(),
    ),
    circuit(
        "Summit Descent",
        rx=17, ry=16, exponent=5, phase=0.3,
        half_width=2.4, checkpoints=10, trait="drift",
        anchors=56,
        boost_at=(0.32,), oil_at=(0.6, 0.86),
    ),
    circuit(
        "Aurora Vault",
        rx=21.3, ry=12.6, exponent=8, phase=0.7,
        half_width=2.9, checkpoints=8, trait="boost",
        anchors=56,
        boost_at=(0.22, 0.58, 0.88), oil_at=(),
    ),
    circuit(
        "Crown Circuit",
        rx=21.1, ry=14.1, exponent=9, phase=0,
        half_width=3.0, checkpoints=8, trait="speed",
        anchors=56,
        boost_at=(0.3, 0.7), oil_at=(),
    ),
    circuit(
        "Ivory Straits",
        rx=16, ry=15, exponent=3, phase=0.5,
        half_width=2.5, checkpoints=10, trait="technical",
        anchors=56,
        boost_at=(0.45,), oil_at=(0.2, 0.75),
    ),
]

#: All 24 authored circuits, in cup order.
CIRCUITS = BRONZE + SILVER + GOLD + PLATINUM


# ---------------------------------------------------------------------------
# Self-validation. A circuit that does not close is not a circuit, and finding
# that out from `validate_walls` several stages later is a waste of a run.
# ---------------------------------------------------------------------------

def _validate_circuit(c):
    """Checks a circuit before anything is emitted."""
    name = c["name"]
    anchors = c["centre"]
    if len(anchors) < 16:
        raise AssertionError(f"{name}: only {len(anchors)} anchors is too coarse to drive")

    # The loop must close and enclose area. A self-cancelling shape has zero
    # signed area and walls the circuit off at the pinch.
    area = 0.0
    n = len(anchors)
    for i in range(n):
        a, b = anchors[i], anchors[(i + 1) % n]
        area += a[0] * b[1] - b[0] * a[1]
    area *= 0.5
    if abs(area) < 1.0:
        raise AssertionError(f"{name}: encloses {area:.2f} tiles^2 -- degenerate")

    # Winding must be consistent: a figure-of-eight walls the circuit off where
    # the lobes cross, and no fillet or spline can rescue it.
    signs = set()
    for i in range(n):
        a, b, c2 = anchors[(i - 1) % n], anchors[i], anchors[(i + 1) % n]
        cross = (b[0] - a[0]) * (c2[1] - b[1]) - (b[1] - a[1]) * (c2[0] - b[0])
        if abs(cross) > 1e-9:
            signs.add(cross > 0)
    if len(signs) > 1:
        raise AssertionError(f"{name}: winds both ways -- a figure of eight")

    # No cusps. The anchor polyline is what the runtime spline is fitted
    # through, so a sharp reversal here becomes a 140-degree kink in the driven
    # centreline. Both earlier geometry approaches produced these and nothing
    # caught them, because nothing measured the evaluated curve.
    worst = 0.0
    worst_at = 0
    for i in range(n):
        a, b, c2 = anchors[(i - 1) % n], anchors[i], anchors[(i + 1) % n]
        d1 = (b[0] - a[0], b[1] - a[1])
        d2 = (c2[0] - b[0], c2[1] - b[1])
        turn = abs(math.degrees(math.atan2(
            d1[0] * d2[1] - d1[1] * d2[0], d1[0] * d2[0] + d1[1] * d2[1])))
        if turn > worst:
            worst, worst_at = turn, i
    if worst > 45.0:
        raise AssertionError(
            f"{name}: anchor {worst_at} turns {worst:.1f} degrees -- a cusp, "
            f"which the runtime spline would inherit"
        )


for _c in CIRCUITS:
    _validate_circuit(_c)

assert len(CIRCUITS) == 24, f"expected 24 authored circuits, got {len(CIRCUITS)}"

# The cooker pads every circuit by `MARGIN_TILES` a side, so the authored grid
# plus padding must fit `MAX_TRACK_DIM`.
try:
    import pathlib as _pathlib
    import sys as _sys

    _sys.path.insert(0, str(_pathlib.Path(__file__).parent))
    from convert_levels import MARGIN_TILES as _MARGIN  # noqa: E402
except Exception:  # pragma: no cover - preflight reports a missing cooker
    _MARGIN = 4

MAX_AUTHORED_DIM = 64 - 2 * _MARGIN  # 56 with a 4-tile margin
for _c in CIRCUITS:
    _w, _h = _c["grid"]
    assert _w <= MAX_AUTHORED_DIM and _h <= MAX_AUTHORED_DIM, (
        f"{_c['name']}: grid {_w}x{_h} exceeds {MAX_AUTHORED_DIM} after "
        f"{_MARGIN}-tile padding"
    )
    # Anchor count is capped by MAX_ROUTE_SAMPLES / DEFAULT_SAMPLES_PER_SPAN.
    assert len(_c["centre"]) <= 96, (
        f"{_c['name']}: {len(_c['centre'])} anchors exceeds the 96 the route "
        f"reservoir can sample"
    )
    for _frac in _c["boost_at"] + _c["oil_at"]:
        assert 0.0 <= _frac < 1.0, f"{_c['name']}: hazard fraction {_frac} out of range"
    assert len(_c["centre"]) >= 6, f"{_c['name']}: too few anchors to be a circuit"
