"""Procedural circuit generation.

Every circuit is a **radial function**: `r(theta) = R * (1 + sum a_k sin(k*theta
+ phi_k))`. That is a deliberate choice with one property worth more than
anything else here -- a radial curve cannot self-intersect as long as `r > 0`,
so the generated centreline is always a single closed ring. `validate` rejects
"a second piece of road" as a hard error, and the ways a random walker
generates one are numerous: a loop that fails to close, a chicane that folds
back through the corner before it, a spline whose clamped tangent overshoots.
Each of those produces a shape that *looks* like a lap and is not drivable, and
the failure is invisible until the compiler names a cell.

What the harmonics buy is character. One low harmonic is an oval; two or three
give the four-distinct-corners layout `rounded_rect` exists for; a localised
Gaussian spike pulls a hairpin out of the side. Difficulty controls how much
total amplitude the harmonics are allowed, which is what makes an easy circuit
flowing and a hard one technical -- not the node count, which changes nothing a
driver feels.

Reproducibility is not a nicety: a generated circuit is a level, and a level
somebody spent twenty minutes on has to come back the same shape tomorrow. Every
level here is a pure function of its seed.
"""

from __future__ import annotations

import math
import random
from dataclasses import dataclass, field

from . import constants as C
from . import geometry as G
from .model import Level, Mark, Node, safe_stem

#: Difficulty presets. `amp` is the total harmonic amplitude and sets the
#: character of the ring, `corners` is how many real corners it has, `nodes` the
#: control-point count and `perimeter` the target centreline length in cells.
#
# The perimeter range is taken from the measured lap table in
# `generate_svg_circuit.py`: 84 cells laps in about ten seconds, 156 in about
# twenty, and past that the largest circuit stops fitting the grid. Nothing is
# generated that would not fit.
DIFFICULTIES = {
    "easy": dict(amp=0.10, harmonics=(2, 3), nodes=10, corners=5,
                 perimeter=96, hairpin=0.0),
    "medium": dict(amp=0.20, harmonics=(2, 3, 5), nodes=14, corners=6,
                   perimeter=132, hairpin=0.3),
    "hard": dict(amp=0.30, harmonics=(2, 3, 5, 7), nodes=20, corners=9,
                 perimeter=168, hairpin=0.7),
    "wild": dict(amp=0.40, harmonics=(3, 5, 7, 11), nodes=28, corners=12,
                 perimeter=196, hairpin=1.0),
}

#: How much of each edge a corner's fillet may consume, as a fraction of the
#: shorter adjacent edge. The compiler clamps a fillet at 0.48, and a generator
#: that always produces that maximum leaves a straight of 4% of an edge -- an
#: edge of twelve cells with a half-cell of road on it, which is not a straight.
#: 0.34 leaves two thirds of every edge as road a driver can put a car down on,
#: and still rounds the corner enough that the join is tangent-continuous.
CORNER_SPAN = 0.34

#: Cells between inserted nodes along a spline's straight runs. Four cells is
#: about a car length and a half, and is short enough that a 96-node route can
#: follow it without a control point dropping onto its neighbour's tile centre.
STRAIGHT_NODE_SPACING = 4.0

#: Nodes a spline needs along a straight for the Catmull-Rom tangents to come
#: out parallel to it. At an interior node the tangent is taken from its two
#: neighbours, so four collinear nodes give two interior nodes that are exactly
#: straight and only the ends of the run bend. Three is the minimum that gives
#: one straight interior node, which is too short to satisfy the straight test.
MIN_STRAIGHT_NODES = 5

_PREFIX = [
    "Amber", "Azure", "Basalt", "Cobalt", "Copper", "Crimson", "Emerald",
    "Frost", "Golden", "Granite", "Ivory", "Jade", "Lunar", "Obsidian",
    "Onyx", "Ruby", "Rust", "Sable", "Scarlet", "Silver", "Solar", "Umber",
    "Verdant", "Violet",
]
_SUFFIX = [
    "Bend", "Cascade", "Circuit", "Crest", "Dale", "Flats", "Forge", "Gap",
    "Gorge", "Grotto", "Hairpin", "Hollow", "Landing", "Loop", "Maze",
    "Narrows", "Pass", "Ridge", "Run", "Sweep", "Track", "Vale", "Whorl",
    "Yard",
]
_INFIELD = (C.GRASS, C.GRASS, C.GRASS, C.GRAVEL, C.SAND)
_RUNOFF = (C.GRASS, C.GRASS, C.GRAVEL, C.GRAVEL, C.SAND)


@dataclass
class GenParams:
    """Everything a seed needs. Two `GenParams` with the same seed agree."""
    seed: int = 0
    difficulty: str = "medium"
    name: str = ""
    mode: str = "spline"
    road_half_cells: float = 3.0
    runoff: int | None = None
    outside: int | None = None
    infield: int | None = None
    gates: int = C.GATES
    boost_pads: int = 3
    oil_slicks: int = 1
    jitter: float = 0.35          # node spacing noise, 0 = perfectly even
    steps: dict = field(default_factory=dict)

    def resolved(self) -> dict:
        if self.difficulty not in DIFFICULTIES:
            raise ValueError(
                f"unknown difficulty {self.difficulty!r}; expected one of "
                f"{', '.join(DIFFICULTIES)}"
            )
        base = dict(DIFFICULTIES[self.difficulty])
        base.update(self.steps)
        return base


def make_name(rng: random.Random) -> str:
    return f"{rng.choice(_PREFIX)} {rng.choice(_SUFFIX)}"


def _radial_shape(rng: random.Random, cfg: dict) -> list[tuple[int, float, float]]:
    """Amplitudes and phases for the harmonic sum.

    Amplitudes are drawn so their total is `amp`, not so each is `amp`: four
    harmonics each at the ceiling produce a star with four spikes of the same
    depth, which is a recognisable shape and reads as generated.
    """
    ks = list(cfg["harmonics"])
    weights = [rng.random() + 0.35 for _ in ks]
    total = sum(weights)
    amps = [cfg["amp"] * w / total for w in weights]
    phases = [rng.uniform(0.0, 2 * math.pi) for _ in ks]
    return list(zip(ks, amps, phases))


def _radial_points(
    rng: random.Random, cfg: dict, shape: list[tuple[int, float, float]],
    n: int, centre: tuple[float, float], reach: float, jitter: float,
) -> list[tuple[float, float]]:
    """Sample the radial function at `n` jittered angles.

    `r` is floored at 15% of the base radius. A harmonic sum whose value
    approaches zero makes the curve double back through the origin's
    neighbourhood; the ring is still formally simple, but the resulting hairpin
    is narrower than the road and the compiler rejects it as a second piece of
    road with a cell coordinate rather than a cause.
    """
    hairpin_at = rng.randrange(n) if rng.random() < cfg["hairpin"] else None
    hairpin_w = rng.uniform(0.10, 0.20)
    hairpin_d = rng.uniform(0.35, 0.55)

    angles: list[float] = []
    for i in range(n):
        a = 2 * math.pi * i / n
        a += rng.uniform(-0.5, 0.5) * jitter * (2 * math.pi / n)
        angles.append(a)

    pts: list[tuple[float, float]] = []
    for a in angles:
        m = 1.0
        for k, amp, phase in shape:
            m += amp * math.sin(k * a + phase)
        if hairpin_at is not None:
            d = abs(((a - angles[hairpin_at] + math.pi) % (2 * math.pi)) - math.pi)
            m -= hairpin_d * math.exp(-((d / hairpin_w) ** 2))
        r = reach * max(0.15, m)
        pts.append((centre[0] + r * math.cos(a), centre[1] + r * math.sin(a)))

    # Scale so the drawn ring fits rather than trusting the sample to have
    # landed inside the reach by luck: a jittered ring can overshoot by a
    # corner.
    xs = [p[0] for p in pts]
    ys = [p[1] for p in pts]
    worst = max(
        max(abs(x - centre[0]) for x in xs),
        max(abs(y - centre[1]) for y in ys),
    )
    if worst > 1e-9:
        k = min(1.0, reach / worst)
        if k < 1.0:
            pts = [(centre[0] + (x - centre[0]) * k,
                    centre[1] + (y - centre[1]) * k) for x, y in pts]
    return pts


def _corner_nodes(pts: list[tuple[float, float]], corners: int,
                  rng: random.Random) -> list[tuple[float, float]]:
    """The subset of ring nodes that are corners.

    Spread by index with jitter rather than sampled at random, because two
    corners landing next to each other produce an edge shorter than the fillets
    at either end -- the compiler clamps them and the join cusps. Spreading
    guarantees a minimum angular gap, which is what keeps the edges long enough
    to be straights.

    `rng` is accepted for symmetry with the other shape helpers and to keep the
    call site readable; the spread is deterministic given the ring, because a
    corner count that moved with the seed would make two circuits with the same
    shape have different numbers of corners and no way to reason about either.
    """
    n = len(pts)
    if corners >= n:
        return list(pts)
    stride = n / corners
    picks: list[int] = []
    taken: set[int] = set()
    for k in range(corners):
        centre = int(k * stride + stride / 2)
        span = max(1, int(stride * 0.3))
        pick = None
        for delta in range(span):
            for cand in (centre - delta, centre + delta):
                if 0 <= cand < n and cand not in taken:
                    pick = cand
                    break
            if pick is not None:
                break
        if pick is None:
            pick = centre
        taken.add(pick)
        picks.append(pick)
    del rng
    return [pts[i] for i in sorted(taken)]


def _with_straight_nodes(
    corners: list[tuple[float, float]],
    road_half_cells: float = C.DEFAULT_ROAD_HALF_CELLS,
) -> list[tuple[float, float]]:
    """Corners plus collinear nodes along each edge, for spline mode.

    A spline has no straight sections of its own. Curvature is continuous, so
    even the flattest part of a smooth ring turns a little every sample, and the
    runtime's straight test measures exactly that turn. The way to get a straight
    out of a Catmull-Rom is to put nodes *on* it, evenly and collinear: at an
    interior node the tangent comes from the two neighbours, and collinear
    neighbours give a tangent along the line. Four or more inserted nodes per
    edge leaves the middle of it exactly straight and bends only where the
    corners are, which is also where the circuit should be bending.

    The spacing floors at one road width. A fixed four cells is two full road
    widths on a two-cell circuit, which means the spline's curvature between
    control points is high *relative to the corridor it has to fit in* -- a
    corner that is comfortable at six cells wide pinches the inside edge at
    two, and the road rasterises as a lens rather than a road.
    """
    spacing = max(STRAIGHT_NODE_SPACING, 2.0 * road_half_cells)
    n = len(corners)
    out: list[tuple[float, float]] = []
    for i in range(n):
        a = corners[i]
        b = corners[(i + 1) % n]
        out.append(a)
        length = math.dist(a, b)
        extra = max(int(length // spacing), MIN_STRAIGHT_NODES - 2)
        for k in range(1, extra + 1):
            t = k / (extra + 1)
            out.append((a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t))
    return out


def _natural_radii(pts: list[tuple[float, float]]) -> list[float]:
    """Corner radii for a ring of corner nodes joined by straight edges.

    For a node that turns through `theta`, a fillet of radius `r` touches the
    adjacent edges at `r * tan(theta/2)` from the node. Solving for the `r`
    that puts that touch at `CORNER_SPAN` of the shorter edge reproduces the
    drawn corner and, more importantly, *leaves the rest of the edge straight*.
    See `CORNER_SPAN` for why the compiler's own 0.48 clamp is the wrong target.
    """
    n = len(pts)
    out: list[float] = []
    for i in range(n):
        p, a, b = pts[i], pts[(i - 1) % n], pts[(i + 1) % n]
        u = (p[0] - a[0], p[1] - a[1])
        v = (b[0] - p[0], b[1] - p[1])
        lu, lv = math.hypot(*u), math.hypot(*v)
        if lu < 1e-9 or lv < 1e-9:
            out.append(0.0)
            continue
        u = (u[0] / lu, u[1] / lu)
        v = (v[0] / lv, v[1] / lv)
        cross = u[0] * v[1] - u[1] * v[0]
        dot = max(-1.0, min(1.0, u[0] * v[0] + u[1] * v[1]))
        theta = abs(math.atan2(cross, dot))
        if theta < 1e-3:
            out.append(0.0)
            continue
        t_dist = min(lu, lv) * CORNER_SPAN
        out.append(t_dist / max(1e-3, math.tan(theta / 2.0)))
    return out


def _fit_perimeter(
    pts: list[tuple[float, float]],
    centre: tuple[float, float],
    reach: float,
    target: float,
    road_half_cells: float = C.DEFAULT_ROAD_HALF_CELLS,
) -> list[tuple[float, float]]:
    """Uniformly scale the ring so its centreline is `target` cells long.

    Lap time is set by perimeter *and* by corner radius, and they pull against
    each other: the same lap length as a fast oval is a completely different
    circuit as a technical one, which is why the shape comes from the harmonics
    and only the size comes from here. Scaling is capped at `reach`, so a
    target perimeter the grid cannot hold yields the biggest circuit that fits
    rather than one that runs off the map.
    """
    length = sum(
        math.dist(pts[i], pts[(i + 1) % len(pts)]) for i in range(len(pts))
    )
    if length < 1e-6:
        return pts
    want = min(1.0, target / length)
    if abs(want - 1.0) < 1e-6:
        return pts
    return [
        (centre[0] + (x - centre[0]) * want, centre[1] + (y - centre[1]) * want)
        for x, y in pts
    ]


def _fit_centreline(pts, radii, mode, centre, reach, target,
                    road_half_cells: float = C.DEFAULT_ROAD_HALF_CELLS):
    """Scale the node ring so its *drawn centreline* is `target` cells long.

    Two passes, because the mapping from nodes to length depends on the mode:
    fillets cut the corners off a polygon (shorter), while a spline rounds
    them (longer). Two is enough -- the first pass gets within a few per cent
    and the second lands it.

    Scaling can go either way. The ring this starts from is drawn at whatever
    the harmonics produced, which is usually *smaller* than the difficulty's
    target perimeter, and clamping the scale to 1.0 would leave every hard
    circuit the same size as an easy one. Growing it is fine as long as the
    result still clears the grid, so the growth is applied first and any
    overflow scaled back off afterwards -- which means a target the grid cannot
    hold yields the biggest circuit that fits, rather than one that runs off
    the map or one that quietly ignores the difficulty setting.
    """
    for _ in range(2):
        length = _centreline_length(pts, radii, mode, road_half_cells)
        if length < 1e-6:
            return pts
        want = target / length
        if abs(want - 1.0) < 5e-4:
            break
        pts = [(centre[0] + (x - centre[0]) * want,
                centre[1] + (y - centre[1]) * want) for x, y in pts]

    worst = max(
        max(abs(x - centre[0]), abs(y - centre[1])) for x, y in pts
    )
    if worst > reach and worst > 1e-9:
        k = reach / worst
        pts = [(centre[0] + (x - centre[0]) * k,
                centre[1] + (y - centre[1]) * k) for x, y in pts]
    return pts


def _centreline_length(pts, radii, mode, road_half_cells: float = C.DEFAULT_ROAD_HALF_CELLS) -> float:
    try:
        min_r = road_half_cells + C.KERB_CELLS
        return G.path_length(G.shape_path(pts, radii, mode, G.DEFAULT_STEP, min_r)[1])
    except (ValueError, ZeroDivisionError):
        return 0.0


def generate(params: GenParams | None = None, *, avoid: set[str] | None = None) -> Level:
    """A complete, valid circuit. Pure function of `params.seed`."""
    p = params or GenParams()
    rng = random.Random(p.seed)
    cfg = p.resolved()
    avoid = avoid or set()

    centre = (C.GRID / 2.0, C.GRID / 2.0)
    reach = C.max_extent(p.road_half_cells)
    shape = _radial_shape(rng, cfg)
    pts = _radial_points(rng, cfg, shape, cfg["nodes"], centre, reach, p.jitter)

    name = p.name or make_name(rng)
    tries = 0
    while safe_stem(name) in avoid and tries < 40:
        name = make_name(rng)
        tries += 1

    if p.mode == "race_mania":
        import sys
        import os
        here = os.path.dirname(os.path.abspath(__file__))
        track_cook_dir = os.path.join(os.path.dirname(os.path.dirname(here)), "track_cook")
        if track_cook_dir not in sys.path:
            sys.path.insert(0, track_cook_dir)
        import race_mania_generator as rmg

        spec = rmg.generate_race_mania_track(seed=p.seed, name=name)
        placed = spec["vertices"]
        radii = spec["radii"]
        level = Level(
            name=name,
            nodes=[Node(x, y, r) for (x, y), r in zip(placed, radii)],
            mode="poly",
            road_half_cells=p.road_half_cells,
            runoff=spec.get("runoff", rng.choice(_RUNOFF) if p.runoff is None else p.runoff),
            infield=rng.choice(_INFIELD) if p.infield is None else p.infield,
            outside=rng.choice(_INFIELD) if p.outside is None else p.outside,
            meta={
                "generated": True,
                "seed": p.seed,
                "difficulty": p.difficulty,
                "generator": "race_mania",
                "editor": "level_editor",
            },
        )
        level.clamp_to_grid(1.0)
        _lay_out_marks(level, p, rng)
        return level

    # Corners first, then straight runs between them. Both modes need a real
    # straight -- `test_circuits_are_wide_and_have_long_straights` fails below
    # 24 route samples, and a ring built purely out of smooth curvature has no
    # straight anywhere in it, however good it looks.
    corners = _corner_nodes(pts, int(cfg["corners"]), rng)
    if len(corners) < 3:
        corners = pts
    if p.mode == "spline":
        placed = _with_straight_nodes(corners, p.road_half_cells)
        radii = [0.0] * len(placed)
    else:
        placed = corners
        radii = _natural_radii(placed)

    # Sized *after* the corners are chosen. Choosing them replaces arcs with
    # chords, so a ring scaled to its target perimeter beforehand comes out
    # short -- and `test_circuits_are_wide_and_have_long_straights` has a
    # `MIN_LAP` floor as well as a straight floor, so "a bit under the target"
    # is a failing circuit rather than a small one.
    placed = _fit_centreline(placed, radii, p.mode, centre, reach,
                             float(cfg["perimeter"]), p.road_half_cells)

    level = Level(
        name=name,
        nodes=[Node(x, y, r) for (x, y), r in zip(placed, radii)],
        mode=p.mode,
        road_half_cells=p.road_half_cells,
        runoff=rng.choice(_RUNOFF) if p.runoff is None else p.runoff,
        infield=rng.choice(_INFIELD) if p.infield is None else p.infield,
        outside=rng.choice(_INFIELD) if p.outside is None else p.outside,
        meta={
            "generated": True,
            "seed": p.seed,
            "difficulty": p.difficulty,
            "editor": "level_editor",
        },
    )
    level.clamp_to_grid(1.0)
    _lay_out_marks(level, p, rng)
    return level


def _lay_out_marks(level: Level, p: GenParams, rng: random.Random) -> None:
    """Start line, gates, boost pads and slicks, spread round the lap.

    Pads and slicks are placed at random points *between* gates rather than at
    the sector midpoints `standard_marks` uses. A generator that produces the
    same pad at the same fraction every time produces circuits that differ only
    in outline, and the sector midpoint is exactly where a driver brakes.
    """
    gates = max(1, min(C.MAX_GATES, int(p.gates)))
    marks = [Mark(kind="start", frac=0.0)]
    fracs = [i / (gates + 1) for i in range(1, gates + 1)]
    for f in fracs:
        marks.append(Mark(kind="gate", frac=f))

    sectors = len(fracs) + 1
    # Keep hazards clear of the gate cells by more than the one-cell exclusion
    # box `validate_gates` uses. As a fraction of the lap, 0.015 of a ~78-cell
    # lap is 1.2 cells -- *inside* the box, so on a narrow road a pad was
    # generated onto a gate and then failed validation, and on a wide road it
    # was only just clear. 0.03 puts it two cells out at any road width.
    clearance = 0.03
    # Lateral boost offsets are a fraction of the road's half-width, not an
    # absolute cell count. At a fixed ±0.55 they meant "two pads side by side"
    # on a six-cell road and "a pad hanging off the tarmac into the kerb" on a
    # two-cell one, because the pad's own half-width is already the full road.
    side = 0.45 * p.road_half_cells
    for k in range(int(p.boost_pads)):
        sec = rng.randrange(sectors)
        lo = fracs[sec - 1] if sec > 0 else 0.0
        hi = fracs[sec] if sec < len(fracs) else 1.0
        if hi - lo < 0.04 + 2 * clearance:
            continue
        marks.append(Mark(
            kind="boost",
            frac=rng.uniform(lo + clearance, hi - clearance),
            offset=rng.choice((-side, 0.0, side)) if rng.random() < 0.4 else 0.0,
        ))
    for k in range(int(p.oil_slicks)):
        sec = rng.randrange(sectors)
        lo = fracs[sec - 1] if sec > 0 else 0.0
        hi = fracs[sec] if sec < len(fracs) else 1.0
        if hi - lo < 0.04 + 2 * clearance:
            continue
        marks.append(Mark(kind="oil", frac=rng.uniform(lo + clearance, hi - clearance)))

    level.marks = marks
    level.meta["boost_pads"] = sum(1 for m in marks if m.kind == "boost")
    level.meta["oil_slicks"] = sum(1 for m in marks if m.kind == "oil")


def generate_batch(
    count: int,
    *,
    seed: int = 0,
    store: str | None = None,
    difficulty: str | None = None,
    difficulties: list[str] | None = None,
    road_widths: list[float] | None = None,
    modes: list[str] | None = None,
    prefix: str = "",
    outdir: str | None = None,
    publish: bool = False,
    on_progress=None,
) -> list[Level]:
    """`count` distinct circuits, as one pure function of `seed`.

    `difficulty=None` cycles through the presets instead of fixing one, which
    is what makes a batch worth having: four identical-difficulty circuits with
    different outlines are four times the work for whoever has to learn them.
    """
    from .project import LevelStore

    pool = difficulties or [difficulty] if difficulty else list(DIFFICULTIES)
    widths = road_widths or [3.0]
    mode_pool = modes or ["spline", "spline", "poly"]
    avoid: set[str] = set()
    if store:
        avoid = {lv.stem for lv in LevelStore(store).levels()}

    out: list[Level] = []
    for i in range(count):
        rng = random.Random((seed * 1_000_003) ^ (i * 0x9E3779B1))
        diff = pool[i % len(pool)]
        params = GenParams(
            seed=(seed * 1_000_003 + i) & 0xFFFFFFFF,
            difficulty=diff,
            road_half_cells=widths[i % len(widths)],
            mode=mode_pool[i % len(mode_pool)],
            name=f"{prefix}{make_name(rng)}" if prefix else "",
        )
        level = generate(params, avoid=avoid)
        if store:
            LevelStore(store).save(level)
            avoid.add(level.stem)
        if publish:
            from .raster import compile_level
            compile_level(level, outdir or C.TRACKS_DIR)
        out.append(level)
        if on_progress:
            on_progress(i + 1, count, level)
    return out