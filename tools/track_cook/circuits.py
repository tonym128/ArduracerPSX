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


def _superellipse(rx, ry, exponent, phase, anchors):
    """Samples a closed superellipse as evenly spaced anchors.

    `x = rx * cos(t)^(2/e)`, `y = ry * sin(t)^(2/e)`. The exponent `e` is the
    whole design knob:

    * `e = 2` is a plain ellipse -- no straights at all, everything is a corner.
    * `e` around 4 is a fast sweepers' circuit.
    * `e` around 8 is a stadium: long straights joined by tight, constant-radius
      corners.

    Sampling an analytic curve rather than filleting a polygon is deliberate.
    Two earlier approaches both produced cusps, where the anchor polyline folded
    back on itself and the runtime spline inherited a 140-degree kink:

    * **Fillet a closed polygon.** The arc centre is `r / sin(theta/2)` from the
      vertex. Getting that wrong is invisible at exactly 90 degrees, where
      `sin` and `cos` agree, and wrong everywhere else.
    * **Straights plus corner angles.** Normalising the turns to sum to 360
      closes the *angles* but not the *positions*: a circuit can turn exactly
      once and still spiral away, because the straight lengths and radii do not
      balance.

    A closed analytic curve cannot have either problem. It closes by
    construction, it is smooth by construction, and where the straights are is a
    direct consequence of the exponent.
    """
    power = 2.0 / exponent

    def at(t):
        c, sn = math.cos(t), math.sin(t)
        return (rx * math.copysign(abs(c) ** power, c),
                ry * math.copysign(abs(sn) ** power, sn))

    # Sample densely, then pick anchors at **equal arc-length** intervals.
    #
    # Sampling by curve parameter instead is what a naive implementation does,
    # and it is wrong for a superellipse: at exponent 9 most of the parameter
    # range maps to the four corners and very little to the straights, so the
    # anchors bunch up on the corners and spread out along the straights --
    # which is exactly backwards. The crowding also made two centreline samples
    # nearly coincident, so `Route::nearest` reported a different arc for a
    # hinted and a full scan at the same point.
    dense = 2048
    raw = [at(phase + 2.0 * math.pi * i / dense) for i in range(dense + 1)]
    cum = [0.0]
    for i in range(1, len(raw)):
        cum.append(cum[-1] + math.hypot(raw[i][0] - raw[i - 1][0],
                                        raw[i][1] - raw[i - 1][1]))
    total = cum[-1]
    pts = []
    seg = 0
    for i in range(anchors):
        target = total * i / anchors
        while seg < dense and cum[seg + 1] < target:
            seg += 1
        span = cum[seg + 1] - cum[seg]
        t = (target - cum[seg]) / span if span > 1e-9 else 0.0
        a, b = raw[seg], raw[seg + 1]
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
