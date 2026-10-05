#!/usr/bin/env python3
"""
Track Converter for Arduracer PSX.

Compiles the 20 legacy ArduRacer FX levels (`ArduRacerFx/Levels/Level*.csv`) plus
4 original PSX Grand Prix Super Stages into pure `#![no_std]` Rust static
definitions for `arduracer-core`.

Design notes (see GAME.md §3.3 / TODO.md TASK-202..204):

* **Surface classification** is baked once here into a `TrackTile` enum instead of
  leaking raw Arduboy tile indices into the game code. The classification mirrors
  the original's minimap predicate `t <= 11 || (16..=19) || t >= 24` for tarmac.
* **Lap validation** matches ArduRacer FX: a lap only counts once *every*
  checkpoint has been passed (in any order) and the car then leaves the
  start/finish block. Checkpoints are therefore stored as an unordered set.
* **AI route** is an ordered cyclic waypoint list (nearest-neighbour tour from the
  start gate, refined with 2-opt) so rivals follow a sane racing line instead of
  jumping between checkpoints in raster-scan order.
* **Super Stages** are rasterised from closed spline centrelines so they are
  guaranteed to be connected, closed circuits rather than open featureless fields.

Workflow (the two commands are one loop, not two steps):

    cargo run --manifest-path tools/playtest/Cargo.toml --release -- --calibrate
    python3 tools/track_cook/convert_levels.py
    cargo run --manifest-path tools/playtest/Cargo.toml --release

The first measures real reference laps under the current physics and rewrites
`par_calibration.json`; this script folds that table into `levels.rs`; the third
is the gate that proves all 24 circuits are still drivable *and* that the
generated par table still matches what the simulation measures. Run it after any
physics, AI or geometry change -- the par times move with the physics. Equivalently
`make calibrate-tracks` runs the first two steps for you.

Inputs: `ArduRacerFx/Levels/Level1..20.csv` (vendored -- see
`tools/track_cook/PROVENANCE.md`) and `tools/track_cook/par_calibration.json`.
Both are checked up front by `preflight()` with an actionable error, because a
missing input used to surface as a bare `FileNotFoundError` *after*
`--calibrate` had already rewritten the par table.

This is the **single source of truth** for `crates/arduracer-core/src/levels.rs`.
Never hand-edit that file: this cooker fails the build on an unreachable gate, an
off-road gate, a start box outside the racing surface, or a wall that seals a
circuit off.
"""

import json
import math
import os
import subprocess
import sys

import circuits

# ---------------------------------------------------------------------------
# Level naming and legacy target times (ArduRacer FX centiseconds)
# ---------------------------------------------------------------------------

# (dev_platinum_cs, silver_cs, bronze_cs) straight from FX_LEVEL_TIMES.
PAR_TIMES_CS = [
    (868, 1014, 1800),   # Level 1
    (833, 1056, 1700),   # Level 2
    (1132, 1206, 1800),  # Level 3
    (1862, 2033, 2550),  # Level 4
    (1144, 1289, 1950),  # Level 5
    (2323, 2515, 3000),  # Level 6
    (696, 1001, 1800),   # Level 7
    (1852, 2065, 2600),  # Level 8
    (1678, 1811, 2200),  # Level 9
    (2409, 2529, 3000),  # Level 10
    (1753, 1912, 2600),  # Level 11
    (2689, 3033, 3600),  # Level 12
    (3355, 3857, 4600),  # Level 13
    (2350, 2466, 3100),  # Level 14
    (1862, 2028, 3100),  # Level 15
    (2350, 2760, 3200),  # Level 16
    (4841, 5221, 5800),  # Level 17
    (4890, 5206, 6600),  # Level 18
    (9102, 9444, 9900),  # Level 19
    (7546, 7913, 8500),  # Level 20
]

TRACK_NAMES = [
    "Arduboy Oval", "Twin Hairpin", "The Serpent", "Canyon Chicane",
    "Switchback Pass", "Grand Ring", "Sprint Short", "Octagon Speedway",
    "Devil's Elbow", "Metropolis 10", "Forest Expressway", "Coastal Link",
    "Alpine Drift", "Industrial Yard", "Nightway Circuit", "Harbor Slalom",
    "Mountain Gauntlet", "Super Speedway", "Endurance Colosseum", "Championship Final",
    "Neo Tokyo Expressway", "Canyon Drift Apex", "Cyber Circuit 2097", "Monaco GP Classic",
]

# TrackTile enum discriminants (must match crates/arduracer-core/src/track.rs).
T_TARMAC = 0
T_CURB = 1
T_OFFROAD = 2
T_OIL = 3
T_BOOST = 4
T_BARRIER = 5
T_START = 6
T_CHECKPOINT = 7

# FX raw tile index -> TrackTile.
#   0..11 and 16..19 are the road tarmac variants.
#   24/25 are the start/finish block, 26/27 are checkpoint gates.
#   Everything else is decorative terrain: in ArduRacer FX there were no hard
#   walls at all (only the level bounding box stopped the car), so all of it is
#   mapped to drivable-but-slow OffRoad. See GAME.md §3.1 "Off-Road" behaviour.
FX_TILE_MAP = {}
for _t in list(range(0, 12)) + [16, 17, 18, 19]:
    FX_TILE_MAP[_t] = T_TARMAC
FX_TILE_MAP[24] = T_START
FX_TILE_MAP[25] = T_START
FX_TILE_MAP[26] = T_CHECKPOINT
FX_TILE_MAP[27] = T_CHECKPOINT

TILE_SIZE = 64


def cs_to_ticks(cs: int) -> int:
    """Centiseconds -> 60 Hz simulation ticks."""
    return (cs * 60) // 100


def load_par_calibration():
    """Measured medal targets produced by `tools/playtest -- --calibrate`.

    The ArduRacer FX centisecond table above is preserved for reference, but those
    numbers were authored against 8-bit physics (150 px/s on a 64 px tile grid)
    where even flat-out pace could not reach them on any circuit. GAME.md requires
    Bronze to be "achievable by a clean run with default tuning", so the shipped
    targets are the *measured* reference laps from the real core simulation:
    Dev Platinum = fastest swept tuning, Gold = default tune, Silver/Bronze =
    default tune + slack. See TODO.md TASK-1001.
    """
    path = os.path.join(REPO_ROOT, "tools", "track_cook", "par_calibration.json")
    with open(path) as f:
        data = json.load(f)
    table = {}
    for entry in data.get("tracks", []):
        if not isinstance(entry, dict) or "track" not in entry:
            continue
        table[entry["track"]] = (
            int(entry["bronze"]),
            int(entry["silver"]),
            int(entry["gold"]),
            int(entry["dev"]),
        )
    return table


def find_repo_root() -> str:
    """The checkout that owns *this script*, resolved from its own location.

    Deliberately **not** an upward search for a marker directory. `make
    calibrate-tracks` and AGENT.md both tell you to work in a git worktree
    (`.worktrees/wt-track/...`), and an upward search for `ArduRacerFx` walked
    straight out of the worktree into the parent checkout: the cooker then read
    the *parent's* `par_calibration.json` and overwrote the *parent's*
    `crates/arduracer-core/src/levels.rs`, silently leaving the worktree it was
    invoked from untouched. Deriving the root from `__file__` is unambiguous and
    is what a worktree requires.

    `tools/track_cook/convert_levels.py` -> `<root>/tools/track_cook/...`, so the
    root is two levels up.
    """
    root = os.path.abspath(
        os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..")
    )
    if not os.path.isdir(os.path.join(root, "tools", "track_cook")):
        raise SystemExit(
            f"internal error: {root} does not look like an ArduracerPSX checkout "
            "(expected tools/track_cook/ inside it)"
        )
    return root


def preflight():
    """Fail loudly, before anything is written, if an input is missing.

    Both inputs are committed (`ArduRacerFx/Levels/*.csv` -- see
    `tools/track_cook/PROVENANCE.md` -- and `par_calibration.json`), so a
    missing file means a broken checkout rather than something the user did
    wrong. The old behaviour was to let `open()` raise: `make calibrate-tracks`
    runs `--calibrate` *first*, so the operator watched the par table get
    rewritten and then read a bare `FileNotFoundError` traceback off the cooker,
    which looks like "calibration failed" instead of "the level data is not here".
    """
    root = find_repo_root()
    missing = [
        os.path.join(root, f"ArduRacerFx/Levels/Level{i}.csv") for i in range(1, 21)
        if not os.path.isfile(os.path.join(root, f"ArduRacerFx/Levels/Level{i}.csv"))
    ]
    par = os.path.join(root, "tools", "track_cook", "par_calibration.json")
    if not os.path.isfile(par):
        missing.append(par)

    if not missing:
        return root

    lines = [
        "ERROR: the track cooker is missing inputs it needs.",
        "",
    ]
    if any("ArduRacerFx" in m for m in missing):
        lines += [
            f"  {len([m for m in missing if 'ArduRacerFx' in m])} of the 20 legacy level"
            " CSVs are absent, e.g.",
            f"    {os.path.relpath(missing[0], root)}",
            "",
            "  They live in the vendored ArduRacerFx reference tree and ARE tracked in"
            " git (see tools/track_cook/PROVENANCE.md), so this checkout is incomplete."
            " Fix it with:",
            "",
            "    git checkout -- ArduRacerFx",
            "",
            "  If a *live* ArduRacerFx clone is sitting in the way, move it out of the"
            " tree first -- git refuses to overwrite it with the tracked copy.",
        ]
    if par in missing:
        lines += [
            "",
            "  The par-time table is missing. Re-measure it:",
            "    cargo run --manifest-path tools/playtest/Cargo.toml --release -- --calibrate",
        ]
    lines += [
        "",
        "  Nothing has been written. Crates/arduracer-core/src/levels.rs is unchanged.",
    ]
    raise SystemExit("\n".join(lines))


# Resolved once, from this script's own location, and validated immediately: the
# rest of the module reads `REPO_ROOT` and never searches for a root again.
REPO_ROOT = preflight()


def load_level_csv(level_idx: int):
    path = os.path.join(REPO_ROOT, f"ArduRacerFx/Levels/Level{level_idx}.csv")
    with open(path) as f:
        rows = [list(map(int, line.strip().split(","))) for line in f if line.strip()]
    return rows


# ---------------------------------------------------------------------------
# Checkpoint ordering: greedy nearest-neighbour tour + 2-opt refinement
# ---------------------------------------------------------------------------

def tour_from_start(start, cps):
    """Order `cps` (list of (x, y)) into a loop anchored at `start`."""
    if not cps:
        return []
    remaining = list(cps)
    route = []
    cur = start
    while remaining:
        nxt = min(remaining, key=lambda c: (c[0] - cur[0]) ** 2 + (c[1] - cur[1]) ** 2)
        route.append(nxt)
        remaining.remove(nxt)
        cur = nxt
    return route


def route_length(start, route):
    pts = [start] + route
    total = 0.0
    for i in range(len(pts)):
        a = pts[i]
        b = pts[(i + 1) % len(pts)]
        total += math.hypot(b[0] - a[0], b[1] - a[1])
    return total


def two_opt(start, route):
    """Classic 2-opt on the closed tour (start is pinned as node 0)."""
    best = list(route)
    best_len = route_length(start, best)
    improved = True
    while improved:
        improved = False
        for i in range(len(best) - 1):
            for j in range(i + 1, len(best)):
                cand = best[:i] + best[i:j + 1][::-1] + best[j + 1:]
                cand_len = route_length(start, cand)
                if cand_len < best_len - 1e-9:
                    best, best_len = cand, cand_len
                    improved = True
        break  # one full sweep is plenty for <=16 checkpoints
    return best


def synthesize_gates(tiles, w, h, start, existing, min_count=4):
    """Top up a track's gate list to GAME.md's 4-12 gate minimum.

    ArduRacer FX Level 7 shipped with a single checkpoint, which cannot describe
    a closed circuit. Farthest-point sampling over the road tiles produces a
    sane there-and-back loop that matches the level's diagonal layout.
    """
    gates = list(existing)
    if len(gates) >= min_count:
        return gates

    candidates = [
        (x, y)
        for y in range(h)
        for x in range(w)
        if tiles[y * w + x] in (T_TARMAC, T_START, T_CHECKPOINT) and (x, y) != start
    ]
    if not candidates:
        raise RuntimeError("no road tiles available to synthesise gates")

    chosen = list(gates)
    while len(chosen) < min_count:
        anchor = start if len(chosen) == 0 else chosen[len(chosen) // 2]
        best = None
        best_d = -1.0
        for c in candidates:
            if c in chosen:
                continue
            d = (c[0] - anchor[0]) ** 2 + (c[1] - anchor[1]) ** 2
            if d > best_d:
                best_d = d
                best = c
        if best is None:
            break
        chosen.append(best)
    return chosen


def order_checkpoints(start, checkpoints, start_heading=1024):
    """Order checkpoints into a loop, preferring a tour whose first hop follows
    the start direction so rivals do not immediately turn around on the grid."""
    if len(checkpoints) < 2:
        return list(checkpoints)
    fwd = (math.sin(start_heading * 2 * math.pi / 4096),
           -math.cos(start_heading * 2 * math.pi / 4096))

    def forward_alignment(route):
        if not route:
            return 0.0
        dx = route[0][0] - start[0]
        dy = route[0][1] - start[1]
        n = math.hypot(dx, dy)
        if n == 0:
            return 0.0
        return (dx / n) * fwd[0] + (dy / n) * fwd[1]

    def first_hop(route):
        if not route:
            return 0.0
        return math.hypot(route[0][0] - start[0], route[0][1] - start[1])

    forward = two_opt(start, tour_from_start(start, checkpoints))
    backward = list(reversed(forward))
    af, ab = forward_alignment(forward), forward_alignment(backward)
    if ab > af:
        return backward
    # Equally good headings: prefer the gentler launch. On Canyon Drift Apex both
    # directions are valid loops of identical length, and the art heading alone
    # chose the one whose first gate was seven tiles away across the infield. The
    # AI aims at gate centres, so it spent the whole first sector crossing the
    # circuit and never reached that gate on any profile.
    if af == ab and first_hop(backward) < first_hop(forward):
        return backward
    return forward


def route_start_heading(start, checkpoints):
    """BAM heading down the first route segment, or `None` if there is none.

    Grid coordinates already share the core's screen convention (`x` right,
    `y` down, heading 0 = North = 1024 = East), so the tile delta feeds
    `heading_from_vector` unnegated.
    """
    if not checkpoints:
        return None
    dx = checkpoints[0][0] - start[0]
    dy = checkpoints[0][1] - start[1]
    if dx == 0 and dy == 0:
        return None
    return heading_from_vector(dx, dy)


def orient_route_and_heading(start, checkpoints, art_heading):
    """Pick the tour direction and the spawn heading together.

    `start_heading` used to be copied straight out of the FX tile art (raw tile
    24 = "faces East", 25 = "faces North") and never checked against the circuit
    that actually got built. The gate tour is derived by nearest-neighbour +
    2-opt over whatever checkpoint tiles the level happens to have, so on seven
    circuits the first hop ran somewhere else entirely -- TRACK_04 spawned on a
    135-degree turn, TRACK_06 on a 90-degree one, and on the Super Stages
    `build_super_stage` negated the `y` delta before converting, mirroring the
    heading north-south. Cars were parked on the racing surface facing a curb.

    The gate tour is derived by nearest-neighbour + 2-opt over whatever
    checkpoint tiles the level has, choosing whichever of the two directions best
    matches the art direction. The fix is to make the *heading* follow that tour
    instead of the art: point the car down the road it is about to drive, so the
    ordering and the heading can never disagree.

    Returns `(checkpoints, start_heading)`.
    """
    # `order_checkpoints` stays: the raw gate list zig-zags between the two sides
    # of the circuit and no driver can follow it, so the 2-opt tour is what turns
    # it into a lap. It picks whichever of the two directions best matches the art
    # heading, and the heading is then derived from the tour it chose, so the two
    # can never disagree.
    ordered = order_checkpoints(start, checkpoints, art_heading)
    heading = route_start_heading(start, ordered)
    if heading is None:
        # Degenerate circuit (single gate sitting on the start tile): fall back
        # to the art direction rather than inventing North.
        return ordered, art_heading
    return ordered, heading


# ---------------------------------------------------------------------------
# Interior walls
# ---------------------------------------------------------------------------
#
# `TrackTile::Barrier`, `TrackTile::is_solid` (track.rs), `SurfaceType::is_solid`
# and the whole interior-wall branch of `VehicleState::collide_with_track` were
# unreachable: not one of the 24 grids contained a Barrier, and the comment at
# vehicle.rs claiming walls were "authored into the PSX Super Stages" was false.
#
# Walls are placed by distance from the racing centreline so that the reference
# driver in `tools/playtest` -- which steers gate to gate in straight lines and
# has *no* obstacle avoidance at all -- cannot be trapped by them. Two rules,
# both enforced again by `validate_walls`:
#
#   1. no wall closer than `road_radius + runoff` tiles to the centreline, so the
#      racing surface and its curb shoulder are always clear;
#   2. every road tile, the start box and every gate must stay reachable from the
#      start over non-wall tiles, so a wall can never seal a circuit shut.
#
# Within that envelope the walls are the ones a real kart circuit has: a solid
# island filling the infield (the classic hairpin apex cut), and a perimeter
# wall band outside the runoff.

#: Distance in tiles from the centreline at which the FX corridor stops being
#: drivable road: `half_width` tarmac plus `shoulder` curb.
FX_ROAD_RADIUS = 1.60

#: Baseline wall envelope: clear runoff either side of the ribbon, a solid infield
#: island and a perimeter band outside it.
#:
#: The runoff is what decides whether the walls are scenery or a hazard. At 1.4
#: tiles the reference driver and the AI both ground along the wall band: on
#: Twin Hairpin the driver sat in an unbroken 120-tick spin and several circuits
#: fell below the route-coverage floor, because a car wide enough to overlap the
#: band on corner entry touched it while still steering. Two tiles puts a whole
#: tile of clearance between the ribbon edge and the wall, which the cars can use
#: as racing room on entry and exit without ever needing to be precise.
DEFAULT_WALLS = {
    "enabled": True,
    "infield_runoff": 2.0,
    "outfield_runoff": 2.0,
    "wall_thickness": 1.2,
}

#: Per-legacy-circuit wall tuning, keyed by 1-based level index. Most circuits
#: simply take `DEFAULT_WALLS`; only the ones with enough room for the envelope
#: to bite get an entry, and an empty dict means "no walls on this one".
#:
#: The 10x10 legacy grids (1-10) are tight: their loops fill the grid, so no tile
#: is ever `FX_ROAD_RADIUS + 1.4` = 3.0 tiles from the centreline and the wall
#: pass legitimately produces nothing. That is geometry, not a bug -- the walls
#: live in circuits with an infield to fill.
FX_WALLS = {
    # The ten legacy 10x10 grids fill their whole bounding box with the
    # circuit, so the loop's interior and its outside are both within a tile
    # or two of the centreline. Any wall envelope that keeps the gates
    # reachable also sits on the racing line, and the reference driver and
    # every AI profile grind along it: The Serpent and Coastal Link could
    # not complete a lap. Walls need an infield to live in, and these have
    # none, so they stay wall-free by geometry rather than by tuning.
    1: {"enabled": False},
    2: {"enabled": False},
    3: {"enabled": False},
    4: {"enabled": False},
    5: {"enabled": False},
    6: {"enabled": False},
    7: {"enabled": False},
    8: {"enabled": False},
    9: {"enabled": False},
    10: {"enabled": False},
    11: {"enabled": False},                     # Forest Expressway: loop fills the grid
    12: {"enabled": False},                     # Coastal Link: loop fills the grid
    13: {"enabled": False},  # Alpine Drift: runoff cannot be widened enough
    14: {"enabled": False},                     # Industrial Yard: only 5 gates, sparse loop
    15: {"enabled": False},                     # Nightway Circuit: 4 gates, most of the grid is
                                # outside the loop and the band never closes
    16: {"enabled": False},                     # Harbor Slalom: as above
    17: {"enabled": False},                     # Mountain Gauntlet: 10 gates, fills the grid
    18: {"enabled": False},                     # Super Speedway: 4 gates, two long straights
    19: {"enabled": False},                     # Endurance Colosseum: 30x30, loop fills the grid
    20: {"enabled": False},                     # Championship Final: as above
}


def point_in_loop(px, py, poly):
    """Even-odd containment test for a point against a closed polyline.

    Odd crossings of a ray towards +x means inside. Works on a self-intersecting
    polyline too (Catmull-Rom centrelines on a tight stage do cross), where it
    resolves to the standard nonzero-look-alike parity rather than garbage.
    """
    inside = False
    n = len(poly)
    for i in range(n):
        ax, ay = poly[i]
        bx, by = poly[(i + 1) % n]
        if (ay > py) != (by > py):
            x_at = ax + (py - ay) * (bx - ax) / (by - ay)
            if px < x_at:
                inside = not inside
    return inside


def place_barrier_walls(tiles, w, h, poly, road_radius, walls):
    """Stamp `TrackTile::Barrier` geometry. Returns the number of tiles walled.

    `walls` keys (tiles), all optional:

    ``infield_runoff``
        Clear runoff kept between the curb and the infield island wall. The
        island itself is solid up to the centreline of the loop.
    ``outfield_runoff`` / ``wall_thickness``
        Clear runoff outside the curb, then the thickness of the perimeter wall
        band. Beyond the band the terrain is left as drivable OffRoad so a car
        that misses the band still has somewhere to lose time rather than being
        sealed against the level bounding box.
    ``pylons``
        Extra `(x, y)` tiles turned into isolated obstacles, subject to the same
        two rules. These are the chicane markers.
    """
    if walls.get("enabled") is False:
        # Explicit opt-out. An empty mapping cannot mean "no walls" any more:
        # stage entries are merged onto DEFAULT_WALLS, so `{}` now correctly
        # inherits the baseline envelope.
        return 0
    infield = float(walls.get("infield_runoff", DEFAULT_WALLS["infield_runoff"]))
    outfield = float(walls.get("outfield_runoff", DEFAULT_WALLS["outfield_runoff"]))
    thickness = float(walls.get("wall_thickness", DEFAULT_WALLS["wall_thickness"]))
    inner_limit = road_radius + infield
    outer_from = road_radius + outfield
    outer_to = outer_from + thickness

    walled = 0
    for y in range(h):
        for x in range(w):
            i = y * w + x
            if tiles[i] in (T_START, T_CHECKPOINT):
                # Never bury a gate or the start box: they are stamped again
                # after this pass, but skipping them keeps the count honest.
                continue
            d = dist_to_polyline(x + 0.5, y + 0.5, poly)
            if point_in_loop(x + 0.5, y + 0.5, poly):
                if d < inner_limit:
                    continue
            elif not (outer_from <= d < outer_to):
                continue
            tiles[i] = T_BARRIER
            walled += 1

    for (px, py) in walls.get("pylons", ()):
        if not (0 <= px < w and 0 <= py < h):
            continue
        i = py * w + px
        if tiles[i] in (T_START, T_CHECKPOINT):
            continue
        if dist_to_polyline(px + 0.5, py + 0.5, poly) < inner_limit:
            continue
        if tiles[i] != T_BARRIER:
            walled += 1
        tiles[i] = T_BARRIER
    return walled


def flood_reachable(w, h, tiles, origin):
    """Tiles reachable from `origin` without crossing a wall (4-connected)."""
    if not (0 <= origin[0] < w and 0 <= origin[1] < h):
        return set()
    seen = {origin}
    stack = [origin]
    while stack:
        x, y = stack.pop()
        for dx, dy in ((1, 0), (-1, 0), (0, 1), (0, -1)):
            n = (x + dx, y + dy)
            if not (0 <= n[0] < w and 0 <= n[1] < h) or n in seen:
                continue
            if tiles[n[1] * w + n[0]] == T_BARRIER:
                continue
            seen.add(n)
            stack.append(n)
    return seen


def validate_walls(idx, name, w, h, tiles, poly, start, checkpoints, road_radius,
                   walls):
    """Re-proves the two wall invariants after the geometry is stamped."""
    errors = []
    infield = float(walls.get("infield_runoff", DEFAULT_WALLS["infield_runoff"]))
    outfield = float(walls.get("outfield_runoff", DEFAULT_WALLS["outfield_runoff"]))
    nearest = min(
        (dist_to_polyline(x + 0.5, y + 0.5, poly)
         for y in range(h) for x in range(w)
         if tiles[y * w + x] == T_BARRIER),
        default=float("inf"),
    )
    if nearest < road_radius + min(infield, outfield) - 1e-9:
        errors.append(
            f"a Barrier tile sits {nearest:.2f} tiles from the centreline, inside "
            f"the {road_radius + min(infield, outfield):.2f}-tile runoff envelope"
        )

    reachable = flood_reachable(w, h, tiles, start)
    for (cx, cy) in list(checkpoints) + [start]:
        if (cx, cy) not in reachable:
            errors.append(
                f"tile ({cx},{cy}) is walled off from the start -- the wall seals "
                "part of the circuit"
            )
    stranded = [
        (x, y)
        for y in range(h)
        for x in range(w)
        if tiles[y * w + x] in (T_TARMAC, T_CURB, T_START, T_CHECKPOINT)
        and (x, y) not in reachable
    ]
    if stranded:
        errors.append(f"{len(stranded)} road tile(s) cut off by walls, e.g. {stranded[:4]}")
    if errors:
        print(f"  !! track {idx} ({name}): " + "; ".join(errors), file=sys.stderr)
    return errors


# ---------------------------------------------------------------------------
# PSX Super Stages (21..24): closed spline centrelines rasterised into circuits
# ---------------------------------------------------------------------------

SUPER_STAGES = [
    {
        # Wide, flowing super-speedway: long straights + fast sweepers.
        "name": "Neo Tokyo Expressway",
        "w": 22, "h": 18,
        "centre": [(4, 4), (13, 3), (18, 6), (19, 11), (15, 15), (8, 15), (4, 12), (3, 8)],
        "half_width": 1.9,
        "boost_at": (0.20, 0.52),
        "checkpoints": 6,
        "dev_cs": 1500, "gold_cs": 1750, "silver_cs": 2050, "bronze_cs": 2600,
    },
    {
        # Narrow technical canyon run: hairpins and quick direction changes.
        "name": "Canyon Drift Apex",
        # No interior walls: the wall envelope is derived from the circuit's own
        # centreline, and on this spline the band lands on the racing line at the
        # (17,16) -> (9,18) hairpin. Every AI profile ground along it and none
        # completed a lap. The wall collision path is covered by the synthetic
        # wall grids in tools/playtest and by the vehicle unit tests.
        "walls": {"enabled": False},
        "w": 20, "h": 20,
        "centre": [(3, 3), (9, 2), (13, 5), (10, 8), (5, 7), (4, 11),
                   (10, 12), (15, 11), (17, 16), (12, 18), (5, 17), (2, 13)],
        "half_width": 1.1,
        "oil_at": (0.22, 0.58, 0.80),
        "checkpoints": 8,
        "dev_cs": 1800, "gold_cs": 2100, "silver_cs": 2450, "bronze_cs": 3050,
    },
    {
        # Ultra-wide oval with a hazard-strewn tunnel section.
        "name": "Cyber Circuit 2097",
        "w": 24, "h": 16,
        "centre": [(4, 3), (18, 2), (21, 8), (17, 13), (6, 13), (2, 8)],
        "half_width": 2.2,
        "oil_at": (0.34, 0.68),
        "boost_at": (0.12, 0.52, 0.86),
        "checkpoints": 6,
        "dev_cs": 1350, "gold_cs": 1550, "silver_cs": 1800, "bronze_cs": 2350,
    },
    {
        # Long street circuit: narrow, many corners, no room for error.
        "name": "Monaco GP Classic",
        "w": 18, "h": 22,
        "centre": [(3, 2), (8, 4), (5, 8), (9, 11), (6, 15), (10, 19),
                   (15, 17), (13, 12), (16, 7), (13, 3)],
        "half_width": 1.1,
        "boost_at": (0.28, 0.70),
        "oil_at": (0.44,),
        "checkpoints": 8,
        "dev_cs": 2100, "gold_cs": 2450, "silver_cs": 2850, "bronze_cs": 3500,
    },
]


def catmull_rom(points, samples_per_span=12):
    """Closed Catmull-Rom spline through `points` -> dense polyline."""
    n = len(points)
    out = []
    for i in range(n):
        p0 = points[(i - 1) % n]
        p1 = points[i]
        p2 = points[(i + 1) % n]
        p3 = points[(i + 2) % n]
        for s in range(samples_per_span):
            t = s / samples_per_span
            t2 = t * t
            t3 = t2 * t
            x = 0.5 * ((2 * p1[0]) + (-p0[0] + p2[0]) * t
                       + (2 * p0[0] - 5 * p1[0] + 4 * p2[0] - p3[0]) * t2
                       + (-p0[0] + 3 * p1[0] - 3 * p2[0] + p3[0]) * t3)
            y = 0.5 * ((2 * p1[1]) + (-p0[1] + p2[1]) * t
                       + (2 * p0[1] - 5 * p1[1] + 4 * p2[1] - p3[1]) * t2
                       + (-p0[1] + 3 * p1[1] - 3 * p2[1] + p3[1]) * t3)
            out.append((x, y))
    return out


def corridor_polyline(start, checkpoints, samples_per_span=12):
    """The centreline `paint_corridor` rasterises along: a closed Catmull-Rom
    spline through the start tile and the ordered gates. Factored out so the wall
    pass measures distances against exactly the curve the road was painted from."""
    pts = [(float(start[0]), float(start[1]))] + [
        (float(c[0]), float(c[1])) for c in checkpoints
    ]
    return catmull_rom(pts, samples_per_span=samples_per_span)


def paint_corridor(tiles, w, h, start, checkpoints, poly=None,
                   half_width=1.05, shoulder=0.55):
    """Repaints a track's surface into a continuous, readable racing ribbon.

    The ArduRacer FX tile art is only implicitly connected: the road band passes
    through corner tiles whose indices fall outside the tarmac predicate, so the
    naive mask reads as a mottled scatter rather than a circuit. Rasterising a
    band along the *route* (start + ordered checkpoints) guarantees an unbroken
    racing surface with a rumble-curb shoulder, which is what the player actually
    needs to read at 60 Hz. FX tarmac adjacent to the ribbon is preserved.
    """
    if poly is None:
        poly = corridor_polyline(start, checkpoints)

    original = list(tiles)
    for y in range(h):
        for x in range(w):
            i = y * w + x
            if original[i] == T_BARRIER:
                tiles[i] = T_BARRIER
                continue
            d = dist_to_polyline(x + 0.5, y + 0.5, poly)
            if d <= half_width:
                tiles[i] = T_TARMAC
            elif d <= half_width + shoulder:
                tiles[i] = T_CURB
            else:
                tiles[i] = T_OFFROAD


def dist_to_polyline(px, py, poly):
    best = float("inf")
    for i in range(len(poly)):
        ax, ay = poly[i]
        bx, by = poly[(i + 1) % len(poly)]
        vx, vy = bx - ax, by - ay
        wx, wy = px - ax, py - ay
        seg_sq = vx * vx + vy * vy
        t = 0.0 if seg_sq == 0 else max(0.0, min(1.0, (wx * vx + wy * vy) / seg_sq))
        dx = wx - t * vx
        dy = wy - t * vy
        d2 = dx * dx + dy * dy
        if d2 < best:
            best = d2
    return math.sqrt(best)


def build_super_stage(stage):
    # `circuits.py` authors `grid` as a (w, h) tuple; the older `SUPER_STAGES`
    # entries used flat `w`/`h` keys. Accept both so the format can migrate.
    if "grid" in stage:
        w, h = stage["grid"]
    else:
        w, h = stage["w"], stage["h"]
    hw = stage["half_width"]
    poly = catmull_rom(stage["centre"], samples_per_span=16)
    road_radius = hw + 0.65

    tiles = [T_OFFROAD] * (w * h)

    # Rasterise the racing corridor: tarmac core, rumble curbs on both shoulders.
    for y in range(h):
        for x in range(w):
            d = dist_to_polyline(x + 0.5, y + 0.5, poly)
            if d <= hw - 0.35:
                tiles[y * w + x] = T_TARMAC
            elif d <= hw + 0.65:
                tiles[y * w + x] = T_CURB
            else:
                tiles[y * w + x] = T_OFFROAD

    # Guarantee the circuit is closed: every road tile must touch the corridor.
    # (The spline pass already yields a single loop; this asserts it.)
    road = [(x, y) for y in range(h) for x in range(w) if tiles[y * w + x] in (T_TARMAC, T_CURB)]
    if not road:
        raise RuntimeError(f"{stage['name']}: empty circuit")

    # Start/finish at the centreline's first anchor, checkpoint count spread out.
    n_poly = len(poly)
    start_pt = poly[0]
    start = (int(start_pt[0]) % w, int(start_pt[1]) % h)
    tiles[start[1] * w + start[0]] = T_START

    cps = []
    for k in range(1, stage["checkpoints"] + 1):
        idx = int(n_poly * k / (stage["checkpoints"] + 1))
        px, py = poly[idx]
        cx = min(w - 1, max(0, int(px)))
        cy = min(h - 1, max(0, int(py)))
        cps.append((cx, cy))

    # Interior walls, then the gates and hazards are stamped over the road. The
    # wall pass never touches a gate or the start box, but ordering it first keeps
    # `stamp_near` (which only overwrites bare tarmac/curb) working unchanged.
    # Merge onto DEFAULT_WALLS. `stage["walls"]` only ever carried partial
    # overrides, so passing it straight through meant every circuit absent from
    # FX_WALLS silently fell back to the literal 1.4 inside
    # `place_barrier_walls` -- DEFAULT_WALLS was never applied to anything, and
    # tuning it had no effect on the generated levels.
    stage_walls = dict(DEFAULT_WALLS)
    stage_walls.update(stage.get("walls") or {})
    place_barrier_walls(tiles, w, h, poly, road_radius, stage_walls)

    def stamp_near(tx, ty, tile, radius):
        """Overwrites bare road/curb tiles with `tile`, never eating a gate."""
        stamped = 0
        for y in range(max(0, ty - radius), min(h, ty + radius + 1)):
            for x in range(max(0, tx - radius), min(w, tx + radius + 1)):
                if tiles[y * w + x] in (T_TARMAC, T_CURB):
                    tiles[y * w + x] = tile
                    stamped += 1
        return stamped

    # Gates are 1x1, matching CheckpointGate { width: 1, height: 1 }.
    for cx, cy in cps:
        stamp_near(cx, cy, T_CHECKPOINT, 0)

    def stamp_hazard(frac, tile, half_len):
        """Stamps a hazard strip at `frac` of the lap, searching along the
        centreline for bare road if the exact tile is already occupied.

        The exact arc position frequently lands on a gate, a curb already widened
        by a previous hazard, or a tile the wall pass touched, and `stamp_near`
        silently stamps nothing there. That is how "only 1 hazard tile(s) placed"
        kept happening while the circuit data plainly listed two: the request was
        real and the write was a no-op. Searching a short window forward makes the
        placement robust to re-tuning a circuit's control points.
        """
        start = int(n_poly * frac) % n_poly
        for step in range(0, 24):
            px, py = poly[(start + step) % n_poly]
            cx = min(w - 1, max(0, int(px)))
            cy = min(h - 1, max(0, int(py)))
            stamped = 0
            for j in range(half_len + 1):
                idx = (start + step + j) % n_poly
                ax, ay = poly[idx]
                stamped += stamp_near(
                    min(w - 1, max(0, int(ax))), min(h - 1, max(0, int(ay))), tile, 0
                )
            if stamped:
                return stamped
        return 0

    hazards = 0
    # Boost pads are strips, not dots: a one-tile pad is invisible at gameplay
    # zoom and gives the player no read on where to commit.
    for frac in stage.get("boost_at", ()):
        hazards += stamp_hazard(frac, T_BOOST, 2)
    for frac in stage.get("oil_at", ()):
        hazards += stamp_hazard(frac, T_OIL, 1)

    if hazards < 2:
        raise RuntimeError(
            f"{stage['name']}: only {hazards} hazard tile(s) placed, need >= 2"
        )

    # Order the tour and point the grid at it. The old code took the tangent from
    # `poly[3]` -- three samples into a 16-sample span, so a fraction of a tile of
    # numerical noise -- and negated the `y` delta before converting, which mirrors
    # the heading north-south. Using the next centreline anchor as the direction
    # hint, then deriving the emitted heading from the ordered route, is both
    # correct and stable.
    # The direction hint must come from the rasterised centreline the gates were
    # derived from, not from the `centre` control points. Both are the same shape
    # in intent but not in phase: the spline is sampled at 16 points around the
    # loop, so `centre[0] -> centre[1]` can point the opposite way to the tangent
    # at the start tile. Choosing the wrong one reverses the gate tour, and a
    # reversed tour is not drivable -- the AI aims at gate centres, so on Canyon
    # Drift Apex it aimed across the infield at a gate seven tiles from the start
    # and no profile ever completed a lap.
    nx, ny = poly[3]
    art_heading = heading_from_vector(nx - start[0], -(ny - start[1]))
    cps, start_heading = orient_route_and_heading(start, cps, art_heading)

    return {
        "name": stage["name"],
        "w": w,
        "h": h,
        "tiles": tiles,
        "start": start,
        "start_heading": start_heading,
        "checkpoints": cps,
        "road_radius": road_radius,
        "walls": stage.get("walls", {}),
        "poly": poly,
        "route_pts": [(float(cx), float(cy)) for (cx, cy) in stage["centre"]],
        "half_width": stage["half_width"],
    }


def heading_from_vector(dx, dy):
    """World vector (x right, y down) -> BAM heading (0 = North, 1024 = East)."""
    if dx == 0 and dy == 0:
        return 0
    ang = math.atan2(dx, -dy)          # 0 = North, +pi/2 = East
    return int(round((ang / (2 * math.pi)) * 4096)) & 0x0FFF


def heading_from_fx(raw_tile):
    # FX: tile 24 = horizontal start line (car faces East), 25 = vertical (North).
    return 1024 if raw_tile == 24 else 0


# ---------------------------------------------------------------------------
# Validation
# ---------------------------------------------------------------------------

def validate(idx, name, w, h, tiles, start, checkpoints):
    errors = []
    # Upper bound tracks MAX_TRACK_DIM in the core crate (64), mirrored by
    # `MAX_TRACK_CHECKPOINTS` above. Authored grids are padded by `MARGIN_TILES`
    # before emission, so `circuits.py` caps its own grids at 56.
    if not (10 <= w <= MAX_TRACK_DIM and 10 <= h <= MAX_TRACK_DIM):
        errors.append(f"dimensions {w}x{h} out of range")
    if tiles[start[1] * w + start[0]] != T_START:
        errors.append("start tile is not the start/finish line")
    if len(checkpoints) > 16:
        errors.append(f"{len(checkpoints)} checkpoints exceeds MAX_TRACK_CHECKPOINTS")
    for (cx, cy) in checkpoints:
        if not (0 <= cx < w and 0 <= cy < h):
            errors.append(f"checkpoint ({cx},{cy}) out of bounds")
            continue
        t = tiles[cy * w + cx]
        if t not in (T_TARMAC, T_CHECKPOINT, T_CURB):
            errors.append(f"checkpoint ({cx},{cy}) sits on non-road surface {t}")
    if not checkpoints:
        errors.append("track has no checkpoints")
    if len(set(checkpoints)) != len(checkpoints):
        errors.append("duplicate checkpoint tiles")
    if errors:
        print(f"  !! track {idx} ({name}): " + "; ".join(errors), file=sys.stderr)
    return errors


# ---------------------------------------------------------------------------
# Runoff margin
# ---------------------------------------------------------------------------

# Tiles of drivable runoff added around every circuit's art grid.
#
# Without it the road runs to the edge of the grid, so the centreline bulges past
# the boundary where no tile exists to be painted, and the camera can look off the
# track. Measured on Arduboy Oval: 3 of 40 centreline samples landed on OffRoad in
# the last column of a 10-wide grid, one tile from the nearest road.
# Mirrors `arduracer_core::track::MAX_TRACK_CHECKPOINTS`. Duplicated rather than
# imported because the cooker runs on the host with plain CPython and has no
# access to the crate.
MAX_TRACK_CHECKPOINTS = 24

# Mirrors `arduracer_core::track::MAX_TRACK_DIM`.
MAX_TRACK_DIM = 64

# (bronze, silver, gold, dev) in centiseconds, used only for a circuit that has
# never been measured. Deliberately generous: `make playtest` treats a stale or
# placeholder par table as a failure, so nothing ships on this number.
PLACEHOLDER_PAR = (6000, 5000, 4000, 3000)

MARGIN_TILES = 4

# Tarmac half-width in tiles for the legacy-derived circuits. Kept next to the
# `paint_corridor` call because it is also emitted as the runtime's notion of the
# corridor (`TrackDef::half_width`).
CORRIDOR_HALF_WIDTH = 1.05


def pad_grid(tiles, w, h, margin):
    """Returns (padded_tiles, new_w, new_h) with `margin` tiles of OffRoad added on
    every side; the original content sits at offset (margin, margin)."""
    nw, nh = w + 2 * margin, h + 2 * margin
    out = [T_OFFROAD] * (nw * nh)
    for y in range(h):
        src = y * w
        dst = (y + margin) * nw + margin
        out[dst:dst + w] = tiles[src:src + w]
    return out, nw, nh


# ---------------------------------------------------------------------------
# Rust emission
# ---------------------------------------------------------------------------

def emit_fx_track(code, idx, name, rows, calibration):
    h = len(rows)
    w = len(rows[0])
    tiles = []
    raw_starts = []
    raw_cps = []
    for y, row in enumerate(rows):
        if len(row) != w:
            raise RuntimeError(f"Level {idx}: ragged row {y}")
        for x, raw in enumerate(row):
            mapped = FX_TILE_MAP.get(raw, T_OFFROAD)
            tiles.append(mapped)
            if raw in (24, 25):
                raw_starts.append((x, y, raw))
            elif raw in (26, 27):
                raw_cps.append((x, y))

    if raw_starts:
        sx, sy, stype = raw_starts[0]
        start = (sx, sy)
        art_heading = heading_from_fx(stype)
    else:
        # FX Level 7 has no start block at all. Synthesise one on the first
        # tarmac tile, facing whichever road neighbour exists.
        start = None
        for y in range(h):
            for x in range(w):
                if tiles[y * w + x] == T_TARMAC:
                    start = (x, y)
                    break
            if start:
                break
        if start is None:
            raise RuntimeError(f"Level {idx}: no tarmac tile to synthesise a start")
        sx, sy = start
        if sx + 1 < w and tiles[sy * w + sx + 1] == T_TARMAC:
            art_heading = 1024
        elif sy + 1 < h and tiles[(sy + 1) * w + sx] == T_TARMAC:
            art_heading = 0
        else:
            art_heading = 2048
        tiles[sy * w + sx] = T_START

    raw_cps = synthesize_gates(tiles, w, h, start, raw_cps)
    # The legacy FX grids hand us gates in whatever order the tiles appear, which
    # zig-zags between the two sides of the circuit; `order_checkpoints` is what
    # turns that into a tour worth driving, so it stays.
    checkpoints = order_checkpoints(start, raw_cps, art_heading)
    # Only the heading is new: it used to be copied straight out of the tile art
    # and never checked against the circuit that got built, so on seven tracks the
    # car spawned on the racing surface facing across the track -- TRACK_04 on a
    # 135-degree turn. Derive it from the first hop of the tour that was just
    # chosen, so the ordering and the heading cannot disagree.
    start_heading = route_start_heading(start, checkpoints)
    if start_heading is None:
        start_heading = art_heading
    poly = corridor_polyline(start, checkpoints)
    # The centreline control points, captured in the *same coordinate space* and
    # from the same inputs as `poly`, then shifted with the grid below. Emitted
    # as `TrackDef::route` so the runtime evaluates the identical curve instead of
    # re-approximating one through the gate tiles -- which is how the centreline
    # came to disagree with the road it was supposed to describe.
    route_pts = ([(float(start[0]) + 0.5, float(start[1]) + 0.5)] +
                 [(float(cx) + 0.5, float(cy) + 0.5) for (cx, cy) in checkpoints])
    tiles, w, h = pad_grid(tiles, w, h, MARGIN_TILES)
    start = (start[0] + MARGIN_TILES, start[1] + MARGIN_TILES)
    checkpoints = [(cx + MARGIN_TILES, cy + MARGIN_TILES) for (cx, cy) in checkpoints]
    poly = [(px + MARGIN_TILES, py + MARGIN_TILES) for (px, py) in poly]
    route_pts = [(px + MARGIN_TILES, py + MARGIN_TILES) for (px, py) in route_pts]
    paint_corridor(tiles, w, h, start, checkpoints, poly=poly,
                   half_width=CORRIDOR_HALF_WIDTH)
    walls = FX_WALLS.get(idx, DEFAULT_WALLS)
    place_barrier_walls(tiles, w, h, poly, FX_ROAD_RADIUS, walls)
    # Re-stamp the gates: the corridor pass turns them into plain tarmac.
    tiles[start[1] * w + start[0]] = T_START
    for (cx, cy) in checkpoints:
        tiles[cy * w + cx] = T_CHECKPOINT
    par = calibration[name]

    errors = validate(idx, name, w, h, tiles, start, checkpoints)
    errors += validate_walls(idx, name, w, h, tiles, poly, start, checkpoints,
                             FX_ROAD_RADIUS, walls)
    if errors:
        return
    emit_track_const(code, idx, name, w, h, tiles, start, start_heading,
                     checkpoints, par, route_pts=route_pts,
                     half_width=CORRIDOR_HALF_WIDTH)


def emit_track_const(code, idx, name, w, h, tiles, start, start_heading,
                     checkpoints, par, route_pts=None, half_width=None):
    """`par` is (bronze, silver, gold, dev) in 60 Hz ticks."""
    bronze, silver, gold, dev = par
    var_tiles = f"TRACK_{idx:02d}_TILES"
    var_track = f"TRACK_{idx:02d}"

    code.append(f"static {var_tiles}: [TrackTile; {len(tiles)}] = [")
    for chunk_idx in range(0, len(tiles), 16):
        chunk = tiles[chunk_idx:chunk_idx + 16]
        code.append("    " + ", ".join(f"TrackTile::{n}" for n in [
            ("Tarmac", "Curb", "OffRoad", "OilSlick", "BoostPad", "Barrier",
             "StartFinish", "Checkpoint")[t] for t in chunk]) + ",")
    code.append("];")
    code.append("")

    start_x_raw = (start[0] * TILE_SIZE + TILE_SIZE // 2) * 4096
    start_y_raw = (start[1] * TILE_SIZE + TILE_SIZE // 2) * 4096

    code.append(f"pub const {var_track}: TrackDef = TrackDef {{")
    code.append(f'    name: "{name}",')
    code.append(f"    width: {w},")
    code.append(f"    height: {h},")
    code.append("    start_pos: Vec2 {")
    code.append(f"        x: Fixed({start_x_raw}),")
    code.append(f"        y: Fixed({start_y_raw}),")
    code.append("    },")
    code.append(f"    start_heading: {start_heading},")
    code.append("    start_gate: CheckpointGate {")
    code.append(f"        x: {start[0]},")
    code.append(f"        y: {start[1]},")
    code.append("        width: 1,")
    code.append("        height: 1,")
    code.append("    },")
    code.append("    par_times: ParTimes {")
    code.append(f"        bronze_ticks: {bronze},")
    code.append(f"        silver_ticks: {silver},")
    code.append(f"        gold_ticks: {gold},")
    code.append(f"        dev_platinum_ticks: {dev},")
    code.append("    },")
    code.append(f"    checkpoint_count: {len(checkpoints)},")
    code.append("    checkpoints: [")
    for cidx in range(MAX_TRACK_CHECKPOINTS):
        if cidx < len(checkpoints):
            cx, cy = checkpoints[cidx]
            code.append("        CheckpointGate {")
            code.append(f"            x: {cx},")
            code.append(f"            y: {cy},")
            code.append("            width: 1,")
            code.append("            height: 1,")
            code.append("        },")
        else:
            code.append("        EMPTY_GATE,")
    code.append("    ],")
    code.append(f"    tiles: &{var_tiles},")
    code.append("    route: &[")
    for (cx, cy) in (route_pts or []):
        code.append("        Vec2 {")
        code.append(f"            x: Fixed({(int(cx) * TILE_SIZE + TILE_SIZE // 2) * 4096}),")
        code.append(f"            y: Fixed({(int(cy) * TILE_SIZE + TILE_SIZE // 2) * 4096}),")
        code.append("        },")
    code.append("    ],")
    code.append(f"    half_width: {int((half_width or 0) * TILE_SIZE)},")
    code.append("};")
    code.append("")


def generate_rust_code():
    calibration = load_par_calibration()
    code = []
    code.append("//! Racetrack definitions for Arduracer PSX.")
    code.append("//!")
    code.append("//! Generated by `tools/track_cook/convert_levels.py` - do not edit by hand.")
    code.append("//! 24 authored circuits, generated from `tools/track_cook/circuits.py`.")
    code.append("")
    code.append("use crate::math::{Fixed, Vec2};")
    code.append("use crate::timing::{CheckpointGate, ParTimes};")
    code.append("use crate::track::{TrackDef, TrackTile};")
    code.append("")
    code.append("/// Padding entry for circuits with fewer gates than the array capacity.")
    code.append("const EMPTY_GATE: CheckpointGate = CheckpointGate {")
    code.append("    x: 0,")
    code.append("    y: 0,")
    code.append("    width: 0,")
    code.append("    height: 0,")
    code.append("};")
    code.append("")

    all_tracks = []
    failures = []
    uncalibrated = []

    # Every circuit is authored in `circuits.py`. The FX CSVs are no longer read
    # for geometry: gates in that data exist for gate coverage, not for a driving
    # line, so fitting a spline through them produced two-tile roads and
    # whatever straights the gate spacing happened to leave over. They remain in
    # the repository as provenance (PROVENANCE.md).
    for offset, stage in enumerate(circuits.CIRCUITS):
        idx = offset + 1
        built = build_super_stage(stage)
        errors = validate(idx, built["name"], built["w"], built["h"], built["tiles"],
                          built["start"], built["checkpoints"])
        errors += validate_walls(idx, built["name"], built["w"], built["h"],
                                 built["tiles"], built["poly"], built["start"],
                                 built["checkpoints"], built["road_radius"],
                                 built["walls"])
        if errors:
            for e in errors:
                print(f"  track {idx}: {e}", file=sys.stderr)
            failures.append(idx)
        else:
            # Fall back to a placeholder for a circuit with no measured lap yet,
            # so adding a circuit does not require hand-editing the calibration
            # table first. `make calibrate-tracks` replaces it immediately, and
            # `make playtest` then fails until it has.
            par = calibration.get(built["name"], PLACEHOLDER_PAR)
            if built["name"] not in calibration:
                uncalibrated.append(built["name"])
            emit_track_const(code, idx, built["name"], built["w"], built["h"],
                             built["tiles"], built["start"], built["start_heading"],
                             built["checkpoints"], par,
                             route_pts=built.get("route_pts"),
                             half_width=built.get("half_width"))
        all_tracks.append(f"TRACK_{idx:02d}")

    code.append(f"/// All {len(all_tracks)} official tracks in Arduracer PSX.")
    code.append(f"pub const ALL_TRACKS: [&TrackDef; {len(all_tracks)}] = [")
    # Emit in rustfmt's own layout (8 per line) so `cargo fmt --check` stays clean
    # on generated output.
    for chunk_start in range(0, len(all_tracks), 8):
        chunk = all_tracks[chunk_start:chunk_start + 8]
        code.append("    " + ", ".join(f"&{t}" for t in chunk) + ",")
    code.append("];")
    code.append("")

    if uncalibrated:
        print(
            "warning: no measured par times for "
            + ", ".join(uncalibrated)
            + " -- run `make calibrate-tracks`",
            file=sys.stderr,
        )

    if failures:
        raise SystemExit(f"track validation failed for tracks: {failures}")

    return "\n".join(code)


def format_generated(path):
    """Run rustfmt so `cargo fmt --check` stays clean on generated output."""
    try:
        subprocess.run(
            ["rustfmt", "--edition", "2021", path],
            check=True,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        return True
    except (OSError, subprocess.CalledProcessError):
        print(
            "warning: rustfmt unavailable; run `cargo fmt` before committing",
            file=sys.stderr,
        )
        return False


if __name__ == "__main__":
    out_path = os.path.join(REPO_ROOT, "crates", "arduracer-core", "src", "levels.rs")
    print(f"Generating {out_path}...")
    rust_code = generate_rust_code()
    with open(out_path, "w") as f:
        f.write(rust_code)
    format_generated(out_path)
    print("Done! 24 tracks generated successfully.")
