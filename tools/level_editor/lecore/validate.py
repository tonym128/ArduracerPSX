"""Validation, mirroring the rules `compile_circuit.validate` enforces.

Every rule here has a counterpart in the compiler, and the ones that are hard
errors there are hard errors here. That is the whole job of this module: the
editor's feedback loop is *seconds* and `make atlas` is minutes, so a level that
would be rejected by the compiler has to be rejected here first, naming the
same cell.

The rules that the compiler checks and this module does not are the ones that
can only be checked on a file on disk -- exact palette RGB, cell alignment --
because the editor *builds* the data image from a code map rather than
rasterising a PNG and reading it back, so it cannot make those mistakes.
"""

from __future__ import annotations

import math
from dataclasses import dataclass, field

import numpy as np

from . import constants as C
from . import geometry as G
from .model import Level

#: Anything the car can stand on, runoff included. Used for connectivity.
SURFACE = frozenset({C.TARMAC, C.TARMAC_WORN, C.KERB_WHITE, C.KERB_RED,
                     C.GRASS, C.GRAVEL, C.SAND, C.OIL, C.BOOST,
                     C.START_LINE, C.GATE, C.TUNNEL, C.BRIDGE, C.SCENERY})

#: Road and track features only -- runoff deliberately excluded. A closed ring
#: of road necessarily encloses an infield, which is a separate component of
#: drivable cells and is correct rather than a stray island.
ROAD = frozenset({C.TARMAC, C.TARMAC_WORN, C.KERB_WHITE, C.KERB_RED,
                  C.OIL, C.BOOST, C.START_LINE, C.GATE, C.TUNNEL, C.BRIDGE})

#: Road as `TrackDef::is_road_at` means it: the racing surface, and *not* the
#: hazards. Boost pads and oil slicks are perfectly drivable -- `is_line_tile`
#: exists precisely because navigation that only accepts road tiles treats a
#: boost strip as a hole in the circuit -- but a checkpoint gate must not have
#: one next to it, because a lap-completion test drives the whole lap one tile
#: off the gate to stand in for an ordinary racing line and requires the tile it
#: lands on to be road. See `a_racing_line_off_the_authored_gates_still_
#: completes_a_lap` in `timing.rs`.
STRICT_ROAD = frozenset({C.TARMAC, C.TARMAC_WORN, C.KERB_WHITE, C.KERB_RED,
                         C.START_LINE, C.GATE, C.TUNNEL, C.BRIDGE})

_HAZARDS = ((C.BOOST, "BOOST"), (C.OIL, "OIL"),
            (C.START_LINE, "START_LINE"), (C.GATE, "GATE"))

#: The nine tiles a racing line can plausibly occupy relative to a gate.
GATE_NEIGHBOURS = tuple(
    (dx, dy) for dy in (-1, 0, 1) for dx in (-1, 0, 1)
)


def gate_cells(level: Level, centreline: list) -> list[tuple[int, int]]:
    """The gate cells the *compiler* will emit, not the ones the drawing shows.

    `compile_circuit.emit_rust` places gates by **index** into the centreline
    at `i * len / (gates + 1)` and rounds to a whole tile. It does not read the
    gate polygons out of the SVG at all -- the drawing and the compiled gate
    list are two separate statements about where the checkpoints are, and they
    agree only because both walk the same centreline in the same direction.
    Recomputing the compiler's answer here means the editor can check the thing
    that will actually be tested.
    """
    n = len(centreline)
    if n < 4:
        return []
    gates = max(1, min(C.MAX_GATES, len(level.gate_marks())))
    out = []
    for k in range(1, gates + 1):
        i = int(n * k / (gates + 1)) % n
        out.append((round(centreline[i][0]), round(centreline[i][1])))
    return out


def validate_gates(level: Level, grid: np.ndarray, centreline: list) -> Report:
    """Every tile a racing line can take around a gate must be road."""
    r = Report()
    cells = gate_cells(level, centreline)
    if not cells:
        return r
    h, w = grid.shape
    for (gx, gy) in cells:
        for dx, dy in GATE_NEIGHBOURS:
            x, y = gx + dx, gy + dy
            if not (0 <= x < w and 0 <= y < h):
                continue
            code = int(grid[y, x])
            if code in STRICT_ROAD:
                continue
            if code in (C.BOOST, C.OIL):
                r.errors.append(
                    f"the {C.CODE_NAME[code]} at ({x},{y}) is on the racing "
                    f"line beside the gate at ({gx},{gy}). `is_road_at` "
                    f"deliberately excludes hazards, and the lap test drives one "
                    f"tile off every gate, so a boost pad or oil slick next to a "
                    f"gate fails `make test`. Move it at least two tiles along "
                    f"the lap."
                )
                break
    return r


@dataclass
class Report:
    """Validation outcome. `errors` are compiler-fatal; `warnings` are judgement."""
    errors: list[str] = field(default_factory=list)
    warnings: list[str] = field(default_factory=list)
    info: dict = field(default_factory=dict)

    @property
    def ok(self) -> bool:
        return not self.errors

    def merge(self, other: "Report") -> "Report":
        self.errors.extend(other.errors)
        self.warnings.extend(other.warnings)
        self.info.update(other.info)
        return self

    def summary(self) -> str:
        if self.errors:
            return f"{len(self.errors)} error(s), {len(self.warnings)} warning(s)"
        if self.warnings:
            return f"valid, with {len(self.warnings)} warning(s)"
        return "valid"


def _components(grid: np.ndarray, of_interest: frozenset[int]) -> list[list[tuple[int, int]]]:
    """4-connected components of cells whose code is of interest.

    Written iteratively with an explicit stack. A recursive flood fill over an
    80x80 grid is fine until someone raises `GRID` to the engine's `MAX_TRACK_DIM`
    of 240, where Python's recursion limit turns a routine validation into a
    `RecursionError` that looks like a crash.
    """
    h, w = grid.shape
    seen = np.zeros((h, w), dtype=bool)
    mask = np.isin(grid, tuple(of_interest))
    regions: list[list[tuple[int, int]]] = []
    for y in range(h):
        for x in range(w):
            if seen[y, x] or not mask[y, x]:
                continue
            seen[y, x] = True
            stack = [(x, y)]
            comp: list[tuple[int, int]] = []
            while stack:
                cx, cy = stack.pop()
                comp.append((cx, cy))
                for dx, dy in ((1, 0), (-1, 0), (0, 1), (0, -1)):
                    nx, ny = cx + dx, cy + dy
                    if 0 <= nx < w and 0 <= ny < h and not seen[ny, nx] \
                            and mask[ny, nx]:
                        seen[ny, nx] = True
                        stack.append((nx, ny))
            regions.append(comp)
    return regions


def validate_grid(grid: np.ndarray) -> Report:
    """The compiler's rules, on a resolved cell grid rather than a PNG."""
    r = Report()
    h, w = grid.shape

    if int((grid < 0).sum()):
        r.errors.append("the data image contains cells that match no palette code")

    if not np.isin(grid, tuple(SURFACE)).any():
        r.errors.append("no drivable surface at all")
        return r

    road_regions = _components(grid, ROAD)
    if not road_regions:
        r.errors.append("no road surface at all (only runoff)")
        return r
    road_regions.sort(key=len, reverse=True)
    if len(road_regions) > 1:
        second = sorted(road_regions[1])[0]
        r.errors.append(
            f"a second piece of road of {len(road_regions[1])} cells at cell "
            f"({second[0]},{second[1]}) -- the circuit must be one closed ring, "
            f"not several. Usually two nodes dragged onto each other, or a road "
            f"that has folded back through itself."
        )
    main = road_regions[0]
    r.info["road_cells"] = len(main)
    r.info["road_components"] = len(road_regions)

    # Road width: the run of drivable cells to the right of each road cell.
    main_set = set(main)
    widths = []
    for (cx, cy) in sorted(main)[:: max(1, len(main) // 60)]:
        widths.append(
            sum(1 for k in (1, 2, 3, 4, 5)
                if (cx + k, cy) in main_set or _in_surface(grid, cx + k, cy))
        )
    if widths:
        med = sorted(widths)[len(widths) // 2]
        r.info["median_road_width_cells"] = med
        if med > 8:
            r.warnings.append(
                f"road is ~{med} cells wide; the design says 2 to 4, and wider "
                f"reads as a car park"
            )

    # Hazards must sit on something drivable.
    for code, name in _HAZARDS:
        for (cx, cy) in main:
            if grid[cy, cx] != code:
                continue
            if not any(
                _in_surface(grid, cx + dx, cy + dy)
                for dy in (-1, 0, 1) for dx in (-1, 0, 1)
                if not (dx == 0 and dy == 0)
            ):
                r.errors.append(
                    f"{name} at cell ({cx},{cy}) is not surrounded by drivable "
                    f"surface, so it is unreachable -- the mark is off the road"
                )
                break   # one report per kind; they share a cause

    # Road may not sit on the border. Runoff may, and on the biggest circuits
    # it does -- that is what gives the circuit its surroundings.
    for x in range(w):
        if grid[0, x] in ROAD or grid[h - 1, x] in ROAD:
            r.errors.append(
                f"road cell on the top/bottom border at column {x} -- the circuit "
                f"runs off the map"
            )
            break
    for y in range(h):
        if grid[y, 0] in ROAD or grid[y, w - 1] in ROAD:
            r.errors.append(f"road cell on the left/right border at row {y}")
            break

    return r


def _in_surface(grid: np.ndarray, x: int, y: int) -> bool:
    if 0 <= y < grid.shape[0] and 0 <= x < grid.shape[1]:
        return int(grid[y, x]) in SURFACE
    return False


def validate_level(level: Level, grid: np.ndarray | None = None) -> Report:
    """Model-level checks, plus the grid checks when a raster is available."""
    r = Report()

    if len(level.nodes) < 3:
        r.errors.append(
            f"a centreline needs at least 3 nodes; this one has {len(level.nodes)}"
        )
        return r

    if not C.road_half_cells_legal(level.road_half_cells):
        cells = level.road_half_cells
        world = cells * C.WORLD_PER_CELL
        r.errors.append(
            f"road half-width {cells:g} cells is {world:g} world units, which is "
            f"not a whole number of {C.WORLD_PER_CELL}-unit cells. "
            f"`build_atlas` asserts this and `TrackDef.half_width` could not say "
            f"it anyway; pick one of "
            f"{', '.join(f'{v:g}' for v in C.ROAD_HALF_CHOICES)}."
        )
    elif level.road_half_cells * C.WORLD_PER_CELL < C.min_half_width():
        # The editor will draw a road this narrow, and it will validate, rasterise
        # and pack into ARSV perfectly. `make test` is what refuses: the engine's
        # road-width floor lives in `tools/test_game_logic` as `MIN_HALF_WIDTH`,
        # and a track below it fails the suite. Said here so it is found while
        # drawing rather than by a failing build with a level name in it.
        floor = C.min_half_width()
        r.errors.append(
            f"road is {int(level.road_half_cells * 2)} cells wide, below the "
            f"game's floor of {floor / C.WORLD_PER_CELL:g}-cell half-width "
            f"({floor} world units, `MIN_HALF_WIDTH` in tools/test_game_logic). "
            f"It draws and rasterises fine; `make test` fails on it. Publishable "
            f"widths are "
            f"{', '.join(f'{int(v * 2)} cells' for v in C.publishable_road_halves())}."
        )
    elif level.road_half_cells not in C.ROAD_HALF_CHOICES:
        r.warnings.append(
            f"road half-width {level.road_half_cells:g} cells is unusual; the "
            f"shipped circuits use 3"
        )

    reach = C.max_extent(level.road_half_cells)
    cx = cy = C.GRID / 2.0
    worst = 0.0
    worst_at = (cx, cy)
    for n in level.nodes:
        d = max(abs(n.x - cx), abs(n.y - cy))
        if d > worst:
            worst, worst_at = d, (n.x, n.y)
    if worst > reach:
        over = worst - reach
        r.warnings.append(
            f"the centreline reaches {worst:.1f} cells from the middle but the "
            f"cross-section needs {reach:.1f}; {over:.1f} cells of the barrier "
            f"run off the grid near ({worst_at[0]:.1f}, {worst_at[1]:.1f}). "
            f"Fit-to-grid will scale it back in."
        )

    n_nodes = len(level.nodes)
    shortest = min(
        (
            ((level.nodes[(i + 1) % n_nodes].x - n.x) ** 2
             + (level.nodes[(i + 1) % n_nodes].y - n.y) ** 2) ** 0.5
            for i, n in enumerate(level.nodes)
        ),
        default=0.0,
    )
    if shortest < 0.75:
        r.warnings.append(
            f"two nodes are {shortest:.2f} cells apart. The fillet clamps to under "
            f"half the shorter edge, so the corner between them is not the one "
            f"you drew -- drag them apart."
        )

    # The two floors `test_circuits_are_wide_and_have_long_straights` enforces
    # besides the road width. Both are properties of the *shape* and both are
    # invisible in a drawing: a lap can look like a lap and still be under the
    # minimum, and a ring can be all gentle curvature with no straight anywhere
    # in it. Checking them here turns a red `make test` naming a track into a
    # message on the level you are looking at.
    floors = C.engine_floors()
    try:
        centreline = level.centreline()
    except (ValueError, ZeroDivisionError):
        centreline = []
    if centreline:
        lap = int(G.path_length(centreline) * C.WORLD_PER_CELL)
        if lap < floors["MIN_LAP"]:
            r.errors.append(
                f"lap is only {lap} world units; `make test` requires "
                f"{floors['MIN_LAP']} (`MIN_LAP`). A circuit shorter than that "
                f"is a hairpin, not a lap -- spread the nodes out or add some."
            )
        straight = longest_straight_samples(centreline)
        r.info["longest_straight_samples"] = straight
        if straight < floors["MIN_STRAIGHT_SAMPLES"]:
            r.errors.append(
                f"the longest straight is {straight} route samples; `make test` "
                f"requires {floors['MIN_STRAIGHT_SAMPLES']} "
                f"(`MIN_STRAIGHT_SAMPLES`). A circuit made only of smooth "
                f"curvature has no straight anywhere in it however good it "
                f"looks -- in polygon mode make some nodes square (radius 0) so "
                f"the edge between them runs straight; in spline mode, add nodes "
                f"in a straight line along one edge."
            )
        r.warnings.append(
            f"two nodes are {shortest:.2f} cells apart. The fillet clamps to under "
            f"half the shorter edge, so the corner between them is not the one "
            f"you drew -- drag them apart."
        )

    gates = len(level.gate_marks())
    if gates < C.GATES:
        r.warnings.append(
            f"{gates} checkpoint gate(s). `test_super_stages_are_real_circuits` "
            f"wants {C.GATES}, and `LapTimer`'s anti-teleport guard needs enough "
            f"of them that one lap cannot pass through every gate in a frame."
        )
    if not any(m.kind == "start" for m in level.marks):
        r.warnings.append(
            "no start/finish line. The lap logic keys off it, so the first "
            "crossing will not count as finishing a lap."
        )
    if grid is not None:
        r.merge(validate_grid(grid))
        if centreline:
            r.merge(validate_gates(level, grid, centreline))
    return r


def code_histogram(grid: np.ndarray) -> dict[str, int]:
    """Cells per palette entry, for the editor's readout."""
    counts = np.bincount(grid.ravel().astype(np.int64), minlength=16)
    return {
        C.CODE_NAME[c]: int(counts[c])
        for c in range(16) if int(counts[c]) > 0
    }


# --- the straight --------------------------------------------------------------

#: Tangent turn per route sample, in degrees, below which the runtime calls a
#: stretch "straight". Same number and same reasoning as `longest_straight` in
#: `tools/test_game_logic`: tight on purpose, because a looser tolerance counts
#: an ordinary corner as a straight and the measurement stops meaning anything.
STRAIGHT_TOLERANCE_DEG = 0.75

#: Control points the compiler puts in a route (`build_atlas.ROUTE_NODES`).
ROUTE_NODES = 96

#: Samples the runtime takes per route span (`DEFAULT_SAMPLES_PER_SPAN`).
SAMPLES_PER_SPAN = 8


def longest_straight_samples(centreline, route_nodes: int = ROUTE_NODES,
                             samples_per_span: int = SAMPLES_PER_SPAN,
                             tolerance_deg: float = STRAIGHT_TOLERANCE_DEG) -> int:
    """Longest run of route samples with no meaningful turn, in samples.

    Mirrors `longest_straight` in `tools/test_game_logic`, on the same resampled
    route the compiler emits, because `test_circuits_are_wide_and_have_long_
    straights` fails on anything below 24 and a level editor that lets you draw
    one is an editor that hands you a red build.

    Measuring on the *centreline* rather than on the drawn road is deliberate
    and is also why this can be computed before rasterising. Curvature is
    continuous, not piecewise: a spline through a smooth ring has no zero-curvature
    stretch anywhere, which is a real constraint on how a circuit may be drawn
    and not something a picture can show you.
    """
    pts = list(centreline)
    n = len(pts)
    if n < 4:
        return 0
    ctrl = _resample_route(pts, route_nodes)

    best = 0
    run = 1
    prev = None
    for i in range(len(ctrl) * samples_per_span):
        a = ctrl[i // samples_per_span]
        b = ctrl[(i // samples_per_span + 1) % len(ctrl)]
        t = (i % samples_per_span) / samples_per_span
        pa = (a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t)
        a2 = ctrl[(i // samples_per_span + 1) % len(ctrl)]
        b2 = ctrl[(i // samples_per_span + 2) % len(ctrl)]
        t2 = ((i % samples_per_span) + 1) / samples_per_span
        pb = (a2[0] + (b2[0] - a2[0]) * t2, a2[1] + (b2[1] - a2[1]) * t2)
        dx, dy = pb[0] - pa[0], pb[1] - pa[1]
        if dx == 0 and dy == 0:
            continue
        direction = (dx, dy)
        if prev is not None:
            cross = prev[0] * direction[1] - prev[1] * direction[0]
            dot = prev[0] * direction[0] + prev[1] * direction[1]
            turn = math.degrees(math.atan2(cross, dot))
            turn = abs(turn) if turn <= 180 else abs(360 - turn)
            if turn <= tolerance_deg:
                run += 1
                best = max(best, run)
            else:
                run = 1
        prev = direction
    return best


def _resample_route(points, route_nodes: int):
    """Even arc-length resample, standing in for `compile_circuit.control_points`.

    `control_points` additionally drops control points closer than a cell,
    because two that share a tile centre give the runtime's Catmull-Rom a
    zero-length span and throw the tangent at the seam. This resample keeps
    the spacing uniform instead, which over-counts slightly on a very long
    lap -- erring toward accepting a circuit the test would accept.
    """
    from . import geometry as G

    pts = list(points)
    if len(pts) > 1 and G._dist(pts[0], pts[-1]) < 1e-9:
        pts = pts[:-1]
    return G.resample_closed(pts, min(route_nodes, max(3, len(pts) // 2)))
