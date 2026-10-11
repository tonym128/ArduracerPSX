"""Repairs the editor knows how to make automatically.

Some constraints are geometry and some are rasterisation, and only the second
kind can be checked without drawing the thing. A boost pad that sits one tile
beside a checkpoint gate is a perfectly good boost pad and a failing lap test,
and the difference between them is decided by the majority vote over a 3px-per-
cell raster rather than by anything visible in the drawing. So the level is
rasterised, the offending feature is moved *along the centreline* -- never
across the road, which would put it in the gravel -- and the raster is redone.

Moving along the lap rather than sideways is the point. A one-cell nudge
sideways is enough to leave the racing line on a wide straight and not enough
on a tight corner, so a sideways repair is a repair that works most of the time
and fails silently the rest. Sliding a pad forward or back along the road
always works, because the pad stays on the tarmac by construction, and it is
also what a track designer would do -- a boost pad is a feature of a *place* on
a circuit, not of a square metre of paint.
"""

from __future__ import annotations

from dataclasses import dataclass, field

from . import geometry as G
from . import validate as V
from .model import Level, Mark

#: Rasterisations to try before giving up. Each pass costs about half a second
#: for an 80-cell circuit, and a circuit this broken is fixed on the first.
MAX_PASSES = 8

#: Fraction of the lap to slide a feature per attempt. Large enough to clear a
#: hazard's own width in one step on a typical lap, small enough that a pad does
#: not jump past the next feature.
SLIDE_STEP = 0.02


@dataclass
class RepairReport:
    moved: int = 0
    passes: int = 0
    remaining: list[str] = field(default_factory=list)

    @property
    def ok(self) -> bool:
        return not self.remaining


def _offending_hazards(level: Level, grid, centreline) -> list[tuple[Mark, int, int]]:
    """Hazards sitting in a gate's nine-tile neighbourhood."""
    gates = V.gate_cells(level, centreline)
    if not gates:
        return []
    h, w = grid.shape
    out = []
    for mark in level.marks:
        if mark.kind not in ("boost", "oil"):
            continue
        try:
            at, tangent = G.point_at_fraction(centreline, mark.frac)
        except ValueError:
            continue
        mx, my = round(at[0] + -tangent[1] * mark.offset), \
            round(at[1] + tangent[0] * mark.offset)
        for gx, gy in gates:
            if abs(mx - gx) <= 1 and abs(my - gy) <= 1:
                out.append((mark, gx, gy))
                break
    return out


def separate_hazards(level: Level) -> RepairReport:
    """Slides boost pads and oil slicks off the gates, until they are clear."""
    from .raster import compile_level

    report = RepairReport()
    for _ in range(MAX_PASSES):
        result = compile_level(level, write=False)
        report.passes += 1
        centreline = level.centreline()
        offenders = _offending_hazards(level, result.codes, centreline)
        if not offenders:
            report.remaining = list(result.report.errors)
            return report
        moved_any = False
        for mark, _gx, _gy in offenders:
            if _slide(level, centreline, mark):
                moved_any = True
                report.moved += 1
        if not moved_any:
            report.remaining = list(result.report.errors)
            return report
    result = compile_level(level, write=False)
    report.remaining = list(result.report.errors)
    return report


def _slide(level: Level, centreline, mark: Mark) -> bool:
    """Moves one hazard along the lap, away from the nearest gate.

    Searches forward then backward, nearest-first, and stops at the first
    fraction whose cell is clear. Nearest-first rather than always forward so a
    pad does not accumulate a systematic drift in one direction, which over a
    few generations would bunch every boost pad into the second half of the lap.
    """
    gates = [m.frac % 1.0 for m in level.marks if m.kind == "gate"]
    here = mark.frac % 1.0
    best_dir = 1.0
    if gates:
        gaps = [((here - g + 0.5) % 1.0) - 0.5 for g in gates]   # signed
        nearest = min(gaps, key=abs)
        best_dir = 1.0 if nearest >= 0 else -1.0

    for step in range(1, 60):
        for direction in (best_dir, -best_dir):
            frac = (here + direction * SLIDE_STEP * step) % 1.0
            if frac == mark.frac % 1.0:
                continue
            try:
                at, tangent = G.point_at_fraction(centreline, frac)
            except ValueError:
                continue
            mx = round(at[0] + -tangent[1] * mark.offset)
            my = round(at[1] + tangent[0] * mark.offset)
            if all(abs(mx - gx) > 1 or abs(my - gy) > 1
                   for gx, gy in V.gate_cells(level, centreline)):
                mark.frac = frac
                return True
    return False


def auto_repair(level: Level) -> RepairReport:
    """Every automatic fix, in the order they are cheapest to verify."""
    from .raster import compile_level

    total = RepairReport()
    if level.scale_to_fit():
        total.moved += 1
    sep = separate_hazards(level)
    total.moved += sep.moved
    total.passes += sep.passes

    result = compile_level(level, write=False)
    total.remaining = list(result.report.errors)
    return total