"""Level-format constants: the palette, the grid, and the road cross-section.

These are the level format, and they are asserted against the pipeline rather
than restated from it. `assert_agrees_with_pipeline()` compares every number
here with the live `tools/track_cook/generate_svg_circuit.py` and
`compile_circuit.py` at import time, because drifting from them is a *silent*
corruption rather than a compile error: `load_data` matches data pixels by exact
RGB equality, so a one-step palette drift turns every cell into "unknown" and
rejects the whole circuit with a message that names a pixel rather than a cause.
"""

from __future__ import annotations

import os
import re
import sys

# --- Grid ---------------------------------------------------------------------

#: Cells per side. One cell is one `TrackTile` byte and one square of the data
#: image. The engine addresses at most `MAX_TRACK_DIM` cells a side.
GRID = 80

#: World units per cell. Mirrors `TILE_SIZE` in `arduracer_core::track`.
WORLD_PER_CELL = 32

#: Raster resolution per cell for the data image. 3 is the smallest that
#: survives majority-vote resolution of the ~0.7-cell kerb fringe.
DATA_PX = 3

#: Master visual image size, streamed to the disc as 500 KB blocks.
SIZE = 1024

#: The data raster is taken at this multiple of the cell resolution and
#: majority-voted back down. Without it every anti-aliased edge pixel is
#: ambiguous and the data image is illegal.
SUPERSAMPLE = 3

# --- Palette ------------------------------------------------------------------

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

CODE_NAME = {
    VOID: "VOID",
    TARMAC: "TARMAC",
    TARMAC_WORN: "TARMAC_WORN",
    KERB_WHITE: "KERB_WHITE",
    KERB_RED: "KERB_RED",
    GRASS: "GRASS",
    GRAVEL: "GRAVEL",
    SAND: "SAND",
    OIL: "OIL",
    BOOST: "BOOST",
    START_LINE: "START_LINE",
    GATE: "GATE",
    WALL: "WALL",
    TUNNEL: "TUNNEL",
    BRIDGE: "BRIDGE",
    SCENERY: "SCENERY",
}

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

NAME_CODE = {v: k for k, v in CODE_NAME.items()}
RGB_CODE = {v.upper(): k for k, v in RGB.items()}

RGB_TUPLE = {
    code: (int(hexv[1:3], 16), int(hexv[3:5], 16), int(hexv[5:7], 16))
    for code, hexv in RGB.items()
}

# --- Road cross-section, in cells ---------------------------------------------
#
# Half-widths, because the road is drawn as a *stroke* on the centreline and a
# stroke is centred. The bands are stacked outside each other and painted back
# to front: SVG has no z-buffer, so paint order is the format.

#: Default tarmac half-width in world units, i.e. three lanes. A level may
#: override it with `Level.road_half_cells`, but it must stay a whole number of
#: cells -- see `road_half_cells_legal`, and the assertion `build_atlas` makes.
#:
#: This is the *default for a new level*, not the floor: `DEFAULT_ROAD_HALF_CELLS`
#: is the same number in cells, and the floor is `min_half_width()`.
ROAD_HALF_WORLD = 96

KERB_CELLS = 0.7           # rumble band outside the tarmac
RUNOFF_CELLS = 2.4         # gravel band outside the kerb
WALL_CELLS = 1.0           # barrier band outside the runoff

INFIELD = GRASS
OUTSIDE = GRASS

#: Road half-widths a level may use, in cells. The engine floors `half_width` at
#: `MIN_ROAD_HALF_WIDTH` (one lane) and `build_atlas` requires a whole number of
#: cells, so the legal set is integral.
#:
#: 1 is one lane -- a single-car technical circuit, a go-kart track. 2 is two
#: lanes, enough to pass. 3 is the width the shipped circuits were designed at
#: and what every one of them uses. 4 is a wide, flowing circuit.
#:
#: All four are publishable. They were not always: 1 and 2 were offered by every
#: chooser and then refused downstream, because the floor lived in a *test*
#: constant (`MIN_HALF_WIDTH` in `tools/test_game_logic`) that encoded a taste
#: decision about the shipped 24 rather than a limit of the engine. That floor is
#: now the engine's own one-lane floor, so the widths this module offers and the
#: widths the game accepts are the same set.
ROAD_HALF_CHOICES = (1.0, 2.0, 3.0, 4.0)

#: Human-readable name for each road half-width, used by the width choosers.
#:
#: Lanes are the vocabulary the rest of the game uses -- `TrackDef::lane_count`,
#: the AI's swerve budget, the starting grid -- so the editor should speak it
#: too, rather than making an author work out that "2 cells" means one lane.
ROAD_HALF_LABELS = {
    1.0: "1 lane  (2 cells)",
    2.0: "2 lanes  (4 cells)",
    3.0: "3 lanes  (6 cells)",
    4.0: "4 lanes  (8 cells)",
}

#: Half-width the shipped circuits were authored at, in cells. The default for a
#: new level, and the width the level editor's generate dialog opens at.
DEFAULT_ROAD_HALF_CELLS = 3.0

#: Narrowest road the game will accept, in world units, and the matching
#: straight and lap-length floors. These are `MIN_HALF_WIDTH`,
#: `MIN_STRAIGHT_SAMPLES` and `MIN_LAP` in `tools/test_game_logic`, and
#: `make test` fails on any track below them. Read out of that file rather than
#: restated here -- see `engine_floors`.
#: Fallback only if `tools/test_game_logic` cannot be found. One lane, which is
#: `MIN_ROAD_HALF_WIDTH` in `arduracer_core::track` -- the engine's own floor,
#: and the number this must agree with.
DEFAULT_MIN_HALF_WIDTH = 32

#: The other two floors `test_circuits_are_wide_and_have_long_straights`
#: enforces, read from the same place. A circuit below any of them compiles,
#: packs and renders perfectly and then fails `make test`.
_FLOOR_RE = re.compile(
    r"const\s+(MIN_HALF_WIDTH|MIN_STRAIGHT_SAMPLES|MIN_LAP)\s*:\s*u\d+\s*=\s*([\d_]+)"
)

_DEFAULT_FLOORS = {
    "MIN_HALF_WIDTH": DEFAULT_MIN_HALF_WIDTH,
    "MIN_STRAIGHT_SAMPLES": 12,
    "MIN_LAP": 2500,
}


def engine_floors() -> dict[str, int]:
    """The gameplay floors `make test` enforces on every track.

    Read out of `tools/test_game_logic` rather than restated, for the reason
    `min_half_width` gives: these are design decisions about how a PSX racetrack
    should feel, they live in one place, and a second copy is a third thing to
    remember when they change.
    """
    floors = dict(_DEFAULT_FLOORS)
    path = os.path.join(REPO_ROOT, "tools", "test_game_logic", "src", "main.rs")
    try:
        with open(path, encoding="utf-8") as f:
            text = f.read()
    except OSError:
        return floors
    for name, value in _FLOOR_RE.findall(text):
        floors[name] = int(value.replace("_", ""))
    return floors


def min_half_width() -> int:
    """The engine's road-width floor, in world units, read from its test."""
    return engine_floors()["MIN_HALF_WIDTH"]


def publishable_road_halves() -> tuple[float, ...]:
    """Road half-widths in cells that `make test` will accept."""
    floor = min_half_width() / WORLD_PER_CELL
    legal = tuple(
        v for v in ROAD_HALF_CHOICES
        if v * WORLD_PER_CELL >= min_half_width() - 1e-9
    )
    # The `or ROAD_HALF_CHOICES` this used to fall back to was a lie in the one
    # case it could fire: a floor above the widest choice reports *every* width
    # as publishable, when in fact none of them is. Say so instead.
    return legal or ()


def road_half_label(cells: float) -> str:
    """Chooser label for a road half-width, in lanes.

    Falls back to cells for a width outside [`ROAD_HALF_CHOICES`], which is
    legal to author (`road_half_cells_legal` only demands a whole number of
    cells) but has no lane name.
    """
    return ROAD_HALF_LABELS.get(round(float(cells), 3), f"{cells:g}-cell half-width")


#: Kerb dash period, in cells, for the red/white alternation.
KERB_DASH = 2.0

# --- Checkpoints --------------------------------------------------------------

#: Checkpoint gates per lap. `test_super_stages_are_real_circuits` wants at
#: least six, and `LapTimer`'s anti-teleport guard needs enough that one lap
#: cannot pass through every gate in a single frame.
GATES = 6

#: Ceiling on gates the compiled `TrackDef` can hold (`MAX_TRACK_CHECKPOINTS`).
MAX_GATES = 6

# --- Feature marks ------------------------------------------------------------

#: kind -> (palette code, default half-thickness in cells, spans kerbs too)
MARK_KINDS = {
    "start": (START_LINE, 1.2, True),
    "gate": (GATE, 0.7, True),
    "boost": (BOOST, 1.0, False),
    "oil": (OIL, 0.8, False),
}

#: Boost pads and checkpoints the game reacts to. `oil` is scenery that is also
#: a hazard; it is placed the same way and compiles to `TrackTile::OilSlick`.
PLACEABLE_MARKS = ("gate", "boost", "oil")


# --- Paths --------------------------------------------------------------------

REPO_ROOT = os.path.abspath(
    os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..", "..")
)
TRACK_COOK = os.path.join(REPO_ROOT, "tools", "track_cook")
TRACKS_DIR = os.path.join(REPO_ROOT, "tracks")

#: Levels authored in the editor live outside the game tree by default. The
#: point of an external store is that a hundred generated circuits do not become
#: a hundred entries in `levels.rs` until one is explicitly published.
DEFAULT_STORE = os.path.join(REPO_ROOT, "levels")


def track_cook_module(name: str):
    """Imports a `tools/track_cook` module, adding the directory to `sys.path`.

    Reused rather than reimplemented: `paint_visual`, `resolve_cells` and
    `compress_1024_jpeg_500kb` are the ones the shipped circuits were baked
    with, and a second implementation of them would be a second set of answers
    to "what does a kerb look like at 3 px per cell".
    """
    if TRACK_COOK not in sys.path:
        sys.path.insert(0, TRACK_COOK)
    import importlib

    return importlib.import_module(name)


def road_half_cells_legal(cells: float) -> bool:
    """Whether `cells` is a road half-width the engine can be told about.

    `build_atlas` asserts `ROAD_HALF_WORLD % TILE_SIZE == 0` because `half_width`
    is a world-unit count: a fractional cell width compiles to a road a fraction
    of a cell narrower than the one that was drawn, and nothing else fails.
    """
    world = cells * WORLD_PER_CELL
    return abs(world - round(world)) < 1e-9


def border_cells(road_half: float) -> float:
    """Centreline-to-barrier extent for a given road half-width."""
    return road_half + KERB_CELLS + RUNOFF_CELLS + WALL_CELLS


def max_extent(road_half: float) -> float:
    """Largest centreline half-extent from the grid centre that still validates.

    `compile_circuit.validate` only rejects *road* on the image border -- runoff
    and barrier are allowed to reach the edge, and on the biggest circuits they
    do. The bound below keeps the whole cross-section inside the grid, which is
    stricter than the compiler and never surprises anybody.
    """
    return GRID / 2.0 - 1.0 - border_cells(road_half)


def assert_agrees_with_pipeline() -> None:
    """Fails loudly if this module has drifted from the compiler it feeds.

    Cheap (two module imports, no rasterising) and worth running on every
    launch. The failure it prevents -- a level that validates in the editor and
    is rejected by `make atlas` with a pixel coordinate -- is otherwise a very
    slow round trip.
    """
    gen = track_cook_module("generate_svg_circuit")
    cc = track_cook_module("compile_circuit")

    checks = [
        ("GRID", GRID, gen.GRID),
        ("WORLD_PER_CELL", WORLD_PER_CELL, gen.WORLD_PER_CELL),
        ("DATA_PX", DATA_PX, gen.DATA_PX),
        ("SIZE", SIZE, gen.SIZE),
        ("SUPERSAMPLE", SUPERSAMPLE, gen.SUPERSAMPLE),
        ("KERB_CELLS", KERB_CELLS, gen.KERB_CELLS),
        ("RUNOFF_CELLS", RUNOFF_CELLS, gen.RUNOFF_CELLS),
        ("WALL_CELLS", WALL_CELLS, gen.WALL_CELLS),
        ("ROAD_HALF_WORLD", ROAD_HALF_WORLD, gen.ROAD_HALF_WORLD),
        ("TILE_SIZE", WORLD_PER_CELL, cc.TILE_SIZE),
    ]
    for label, ours, theirs in checks:
        if ours != theirs:
            raise AssertionError(
                f"level editor {label}={ours} but tools/track_cook says {theirs}; "
                f"a level drawn here would be the wrong size or the wrong colours "
                f"and make atlas would reject it with a pixel coordinate"
            )
    for code, hexv in RGB.items():
        if gen.RGB.get(code) != hexv:
            raise AssertionError(
                f"palette entry {CODE_NAME[code]} is {hexv} here but "
                f"{gen.RGB.get(code)} in generate_svg_circuit.py; load_data matches "
                f"data pixels by exact RGB, so every cell would read as unknown"
            )
    if GRID > cc.MAX_TRACK_DIM:
        raise AssertionError(
            f"the editor authors {GRID} cells a side but the engine addresses at "
            f"most {cc.MAX_TRACK_DIM}"
        )


def palette_report() -> list[dict]:
    """The palette as data, for the GUI's colour picker and the docs."""
    return [
        {
            "code": code,
            "name": CODE_NAME[code],
            "hex": RGB[code],
            "runoff_ok": code in (GRASS, GRAVEL, SAND),
        }
        for code in sorted(CODE_NAME)
    ]