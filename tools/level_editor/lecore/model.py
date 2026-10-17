"""The level model: what an `.ardlvl` file holds.

Deliberately *not* the SVG. The SVG is a derived, game-consumable artefact and
is regenerated on every save; the nodes are the thing a person edits. Keeping
them separate is what makes an edited level round-trip losslessly -- a closed
ring's nodes survive a save/load/compile/save cycle, while the 600-sample
polyline the rasteriser produces never becomes the source of truth.

The sidecar descriptors (`meta`) exist because SVG has nowhere to put them and
the format has no room for them: level name, difficulty, author, tags, lap
notes. They are written to `tracks/<Name>.level.json` alongside the SVG, which
is where `build_atlas.py` picks up per-level road width, and they travel with
the level into the external store and out of it again.
"""

from __future__ import annotations

import json
import math
import os
import re
import unicodedata
from dataclasses import dataclass, field, replace

from . import constants as C
from . import geometry as G

SCHEMA = "arduracer.level/1"

_MODE_POLY = "poly"
_MODE_SPLINE = "spline"


def safe_stem(name: str) -> str:
    """Filename stem for a level name.

    Names come from people and from the random generator, and a name is also a
    Rust identifier on the way into `levels.rs`
    (`TRACK_` + ``"".join(c if c.isalnum() else "_" for c in n).upper()``).
    Two names that collapse to the same stem here would silently overwrite each
    other's files in `tracks/`, and two that collapse to the same identifier
    would not compile at all -- so the normalisation happens once, at the point
    of naming, instead of at the point of failure.
    """
    ascii_name = unicodedata.normalize("NFKD", name).encode("ascii", "ignore").decode()
    cleaned = re.sub(r"[^A-Za-z0-9_]+", "_", ascii_name).strip("_")
    cleaned = re.sub(r"_+", "_", cleaned)
    if not cleaned:
        cleaned = "Untitled"
    if cleaned[0].isdigit():
        cleaned = "L" + cleaned
    return cleaned


def display_name(stem: str) -> str:
    """The inverse of `safe_stem`, for `TrackDef.name`."""
    return stem.replace("_", " ").title()


@dataclass
class Node:
    """One centreline node: a position, and in `poly` mode a fillet radius."""
    x: float
    y: float
    r: float = 0.0

    def as_tuple(self) -> G.Point:
        return (self.x, self.y)


@dataclass
class Mark:
    """A road feature placed at a fraction of the lap.

    `frac` is the position; `offset` is the sideways slide across the road, in
    cells, which is how a left-right pair of boost pads is expressed without
    two marks fighting over the same square metre of tarmac.
    """
    kind: str                       # start | gate | boost | oil
    frac: float = 0.0
    half_len: float | None = None   # None -> derived from kind and road width
    half_thick: float | None = None
    offset: float = 0.0

    @property
    def code(self) -> int:
        return C.MARK_KINDS[self.kind][0]

    def to_json(self) -> dict:
        return {
            "kind": self.kind,
            "frac": round(self.frac % 1.0, 6),
            "half_len": self.half_len,
            "half_thick": self.half_thick,
            "offset": round(self.offset, 4),
        }

    @staticmethod
    def from_json(d: dict) -> "Mark":
        kind = d.get("kind", "gate")
        if kind not in C.MARK_KINDS:
            raise ValueError(f"unknown mark kind {kind!r}")
        return Mark(
            kind=kind,
            frac=float(d.get("frac", 0.0)),
            half_len=None if d.get("half_len") is None else float(d["half_len"]),
            half_thick=None if d.get("half_thick") is None else float(d["half_thick"]),
            offset=float(d.get("offset", 0.0)),
        )


@dataclass
class Level:
    """One circuit: centreline, cross-section, features, descriptors."""

    name: str = "Untitled"
    nodes: list[Node] = field(default_factory=list)
    mode: str = _MODE_SPLINE
    road_half_cells: float = 3.0
    kerb_cells: float = C.KERB_CELLS
    runoff_cells: float = C.RUNOFF_CELLS
    wall_cells: float = C.WALL_CELLS
    runoff: int = C.GRASS
    infield: int = C.GRASS
    outside: int = C.GRASS
    marks: list[Mark] = field(default_factory=list)
    shortcuts: list[str] = field(default_factory=list)
    meta: dict = field(default_factory=dict)

    # --- identity -----------------------------------------------------------

    @property
    def stem(self) -> str:
        return safe_stem(self.name)

    def display_name(self) -> str:
        return display_name(self.stem)

    # --- geometry -----------------------------------------------------------

    def points(self) -> list[G.Point]:
        return [n.as_tuple() for n in self.nodes]

    def radii(self) -> list[float]:
        return [n.r for n in self.nodes]

    def min_corner_radius_cells(self) -> float:
        """Smallest fillet radius the road can be drawn on, in cells.

        A corner of radius `r` sweeps an arc whose inside edge sits `r` minus the
        half-width of road-and-kerb from the centre of curvature. Once that
        passes through zero the inside edge folds back on itself: the outline
        self-intersects and the car meets a cusp.

        Floored at the kerb width so a level with the kerb turned off still gets
        a positive radius rather than a divide-by-zero downstream.
        """
        return max(
            self.road_half_cells + self.kerb_cells,
            self.kerb_cells,
            0.5,
        )

    def path_and_centre(self) -> tuple[str, list[G.Point]]:
        """The centreline as an SVG `d`, and the dense samples it produces."""
        return G.shape_path(
            self.points(),
            self.radii(),
            self.mode,
            G.DEFAULT_STEP,
            self.min_corner_radius_cells(),
        )

    def centreline(self) -> list[G.Point]:
        return self.path_and_centre()[1]

    def centreline_length(self) -> float:
        return G.path_length(self.centreline())

    def bbox(self) -> tuple[float, float, float, float]:
        return G.bbox(self.points())

    def border_cells(self) -> float:
        return self.road_half_cells + self.kerb_cells + self.runoff_cells + self.wall_cells

    @property
    def half_width_world(self) -> int:
        """Road half-width converted to world units, rounded to integer."""
        return int(round(self.road_half_cells * C.WORLD_PER_CELL))

    # --- cross-section ------------------------------------------------------

    def half_widths(self) -> dict[str, float]:
        """Half-width of each painted band, outermost last.

        Returned rather than precomputed on the model because every one of them
        moves when the road half-width does, and a cached set would need
        invalidating on a change the UI makes sixty times a second.
        """
        road = self.road_half_cells
        kerb = road + self.kerb_cells
        runoff = kerb + self.runoff_cells
        return {
            "road": road,
            "kerb": kerb,
            "runoff": runoff,
            "wall": runoff + self.wall_cells,
        }

    def mark_half_len(self, mark: Mark) -> float:
        """How far across the road a mark reaches.

        Gates and the start line span road plus kerb, because they are the
        *lap* logic: `timing.rs` looks for `TrackTile::Checkpoint` within eight
        tiles of the gate centre, and a gate that stopped at the kerb edge
        would sit off the racing line. Boost pads span the tarmac only -- a pad
        over the kerb triggers on a car that has already run wide, which reads
        as the game helping you off-track.
        """
        if mark.half_len is not None:
            return mark.half_len
        code, _, spans_kerb = C.MARK_KINDS[mark.kind]
        hw = self.half_widths()
        if mark.kind == "oil":
            # Narrower than the full road, because a slick across every lane is
            # a wall rather than a hazard. The `1.5` is an absolute cell count,
            # so it was written for the three-lane circuits and meant "half the
            # road". On a one-lane road it means the opposite: three cells across
            # a two-cell road, spilling half a cell onto each kerb -- and because
            # the kerb is drivable the hazard check does not catch it, so the
            # slick is simply painted in the wrong place.
            return min(1.5, hw["road"])
        return hw["kerb"] if spans_kerb else hw["road"]

    def mark_half_thick(self, mark: Mark) -> float:
        if mark.half_thick is not None:
            return mark.half_thick
        return C.MARK_KINDS[mark.kind][1]

    # --- marks --------------------------------------------------------------

    def sorted_marks(self) -> list[Mark]:
        return sorted(self.marks, key=lambda m: (m.frac % 1.0, m.kind))

    def start_mark(self) -> Mark:
        starts = [m for m in self.marks if m.kind == "start"]
        if starts:
            return min(starts, key=lambda m: m.frac % 1.0)
        return Mark(kind="start", frac=0.0)

    def gate_marks(self) -> list[Mark]:
        return sorted(
            (m for m in self.marks if m.kind == "gate"), key=lambda m: m.frac % 1.0
        )

    def add_standard_marks(self, gates: int = C.GATES) -> None:
        """Replace the feature set with a standard lap: start, gates, boosts.

        Gates at `i / (gates + 1)`, boost pads at `(i - 0.5) / (gates + 1)` so
        each pad sits in the middle of a sector rather than on a gate a driver
        is already braking for. Same layout as `standard_marks` in the
        generator, and the same reason for the half-cell offset.
        """
        gates = max(1, min(C.MAX_GATES, int(gates)))
        step = 1.0 / (gates + 1)
        marks = [Mark(kind="start", frac=0.0)]
        for i in range(1, gates + 1):
            marks.append(Mark(kind="gate", frac=i * step))
        for i in range(1, gates + 1):
            if i % 2 == 1:
                marks.append(Mark(kind="boost", frac=(i - 0.5) * step))
        self.marks = marks

    def nudge_marks(self) -> None:
        """Re-spread gates round the lap, keeping boosts and oil where they are.

        Dragging a node changes the lap length, so a gate left where it was is
        no longer a seventh of the way round. This is what the "re-space gates"
        action does.
        """
        gates = sorted((m for m in self.marks if m.kind == "gate"),
                       key=lambda m: m.frac % 1.0)
        if not gates:
            return
        n = len(gates)
        for i, m in enumerate(gates):
            m.frac = (i + 1) / (n + 1)

    # --- node editing -------------------------------------------------------

    def node_at(self, x: float, y: float, radius: float) -> int | None:
        """Index of the node within `radius` cells, or `None`.

        Hit radius is in cells so it grows with the zoom, which is the only way
        a handle stays clickable at 8 px/cell and stoppable at 0.5.
        """
        best, best_d = None, radius
        for i, n in enumerate(self.nodes):
            d = math.hypot(n.x - x, n.y - y)
            if d <= best_d:
                best, best_d = i, d
        return best

    def insert_node_on_edge(self, a: int, b: int, t: float = 0.5) -> int:
        """Inserts a node between `a` and `b`, returning its index."""
        pa, pb = self.nodes[a], self.nodes[b]
        node = Node(
            x=pa.x + (pb.x - pa.x) * t,
            y=pa.y + (pb.y - pa.y) * t,
            r=pa.r + (pb.r - pa.r) * t,
        )
        self.nodes.insert(b, node)
        return b

    def clamp_to_grid(self, margin: float = 0.0) -> bool:
        """Pulls nodes back inside `[margin, GRID - margin]`. True if moved."""
        lo, hi = margin, C.GRID - margin
        moved = False
        for n in self.nodes:
            nx = min(hi, max(lo, n.x))
            ny = min(hi, max(lo, n.y))
            if nx != n.x or ny != n.y:
                n.x, n.y = nx, ny
                moved = True
        return moved

    def scale_to_fit(self, road_half: float | None = None) -> bool:
        """Uniformly scales the centreline about the grid centre to fit.

        Fitting is on the *nodes*, not the drawn road, so a level that is
        dragged outside the grid comes back the same shape at a different
        scale rather than being clipped -- clipping a ring turns it into a
        shape the car can leave, and `validate` reports it as a second piece of
        road with a cell coordinate and no idea why.
        """
        if len(self.nodes) < 3:
            return False
        cx = cy = C.GRID / 2.0
        reach = max(
            C.max_extent(road_half if road_half is not None else self.road_half_cells),
            1.0,
        )
        worst = 0.0
        for n in self.nodes:
            worst = max(worst, abs(n.x - cx), abs(n.y - cy))
        if worst <= 1e-9 or worst <= reach:
            return False
        k = reach / worst
        for n in self.nodes:
            n.x = cx + (n.x - cx) * k
            n.y = cy + (n.y - cy) * k
        return True

    # --- serialisation ------------------------------------------------------

    def to_json(self) -> dict:
        return {
            "schema": SCHEMA,
            "name": self.name,
            "mode": self.mode,
            "nodes": [
                {"x": round(n.x, 4), "y": round(n.y, 4), "r": round(n.r, 4)}
                for n in self.nodes
            ],
            "road_half_cells": self.road_half_cells,
            "kerb_cells": self.kerb_cells,
            "runoff_cells": self.runoff_cells,
            "wall_cells": self.wall_cells,
            "runoff": self.runoff,
            "infield": self.infield,
            "outside": self.outside,
            "marks": [m.to_json() for m in self.sorted_marks()],
            "shortcuts": list(self.shortcuts),
            "meta": dict(self.meta),
        }

    @staticmethod
    def from_json(d: dict) -> "Level":
        schema = d.get("schema")
        if schema != SCHEMA:
            raise ValueError(
                f"not a level file: schema is {schema!r}, expected {SCHEMA!r}"
            )
        nodes = [
            Node(float(n["x"]), float(n["y"]), float(n.get("r", 0.0)))
            for n in d.get("nodes", [])
        ]
        if len(nodes) < 3:
            raise ValueError(
                f"level {d.get('name', '?')!r} has {len(nodes)} nodes; a centreline "
                f"needs at least 3"
            )
        mode = d.get("mode", _MODE_SPLINE)
        if mode not in (_MODE_POLY, _MODE_SPLINE):
            raise ValueError(f"unknown centreline mode {mode!r}")
        level = Level(
            name=d.get("name", "Untitled"),
            nodes=nodes,
            mode=mode,
            road_half_cells=float(d.get("road_half_cells", 3.0)),
            kerb_cells=float(d.get("kerb_cells", C.KERB_CELLS)),
            runoff_cells=float(d.get("runoff_cells", C.RUNOFF_CELLS)),
            wall_cells=float(d.get("wall_cells", C.WALL_CELLS)),
            runoff=int(d.get("runoff", C.GRASS)),
            infield=int(d.get("infield", C.GRASS)),
            outside=int(d.get("outside", C.GRASS)),
            marks=[Mark.from_json(m) for m in d.get("marks", [])],
            shortcuts=list(d.get("shortcuts", [])),
            meta=dict(d.get("meta", {})),
        )
        level.meta.setdefault("schema", schema)
        return level

    def save(self, path: str) -> str:
        """Writes the `.ardlvl` JSON atomically.

        Atomic because the alternative is a half-written file standing in for a
        level someone spent twenty minutes on, discovered at the next load. The
        write goes to a sibling temp file and is renamed into place, which is
        atomic on the same filesystem.
        """
        os.makedirs(os.path.dirname(os.path.abspath(path)) or ".", exist_ok=True)
        tmp = f"{path}.tmp"
        payload = json.dumps(self.to_json(), indent=2, ensure_ascii=False)
        with open(tmp, "w", encoding="utf-8") as f:
            f.write(payload + "\n")
        os.replace(tmp, path)
        return path

    @staticmethod
    def load(path: str) -> "Level":
        with open(path, encoding="utf-8") as f:
            return Level.from_json(json.load(f))

    # --- construction helpers ----------------------------------------------

    def copy(self) -> "Level":
        return replace(
            self,
            nodes=[replace(n) for n in self.nodes],
            marks=[replace(m) for m in self.marks],
            shortcuts=list(self.shortcuts),
            meta=dict(self.meta),
        )

    def rename(self, name: str) -> None:
        self.name = name

    def descriptor(self) -> dict:
        """The sidecar `build_atlas.py` and any external tool read.

        Written next to the SVG as `tracks/<Stem>.level.json`. Everything here
        is something the compiler needs and the SVG cannot express: the road
        half-width (which becomes `TrackDef.half_width`), and the free-form
        descriptors that have nowhere to go in a document whose only contract is
        "exact palette colours and document order".
        """
        centre = self.centreline()
        return {
            "schema": SCHEMA,
            "name": self.name,
            "stem": self.stem,
            "grid": C.GRID,
            "mode": self.mode,
            "road_half_cells": self.road_half_cells,
            "half_width_world": int(round(self.road_half_cells * C.WORLD_PER_CELL)),
            "kerb_cells": self.kerb_cells,
            "runoff_cells": self.runoff_cells,
            "wall_cells": self.wall_cells,
            "runoff": C.CODE_NAME[self.runoff],
            "runoff_hex": C.RGB[self.runoff],
            "gates": len(self.gate_marks()),
            "checkpoints": len(self.gate_marks()),
            "centreline_samples": len(centre),
            "centreline_length_cells": round(self.centreline_length(), 3),
            "nodes": len(self.nodes),
            "marks": [m.to_json() for m in self.sorted_marks()],
            "source": "level_editor",
            "meta": dict(self.meta),
        }