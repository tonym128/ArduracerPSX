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
"""

import json
import math
import os
import sys

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
    path = os.path.join(find_repo_root(), "tools", "track_cook", "par_calibration.json")
    if not os.path.exists(path):
        raise SystemExit(
            "par_calibration.json missing. Generate it with:\n"
            "  cargo run --manifest-path tools/playtest/Cargo.toml --release -- --calibrate"
        )
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
    cur = os.path.abspath(os.path.dirname(__file__))
    while cur != "/":
        if os.path.exists(os.path.join(cur, "ArduRacerFx")):
            return cur
        cur = os.path.dirname(cur)
    return os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))


def load_level_csv(level_idx: int):
    root = find_repo_root()
    path = os.path.join(root, f"ArduRacerFx/Levels/Level{level_idx}.csv")
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

    forward = two_opt(start, tour_from_start(start, checkpoints))
    backward = list(reversed(forward))
    if forward_alignment(backward) > forward_alignment(forward):
        return backward
    return forward


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


def paint_corridor(tiles, w, h, start, checkpoints, half_width=1.05, shoulder=0.55):
    """Repaints a track's surface into a continuous, readable racing ribbon.

    The ArduRacer FX tile art is only implicitly connected: the road band passes
    through corner tiles whose indices fall outside the tarmac predicate, so the
    naive mask reads as a mottled scatter rather than a circuit. Rasterising a
    band along the *route* (start + ordered checkpoints) guarantees an unbroken
    racing surface with a rumble-curb shoulder, which is what the player actually
    needs to read at 60 Hz. FX tarmac adjacent to the ribbon is preserved.
    """
    pts = [(float(start[0]), float(start[1]))] + [
        (float(c[0]), float(c[1])) for c in checkpoints
    ]
    poly = catmull_rom(pts, samples_per_span=12)

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
    w, h = stage["w"], stage["h"]
    hw = stage["half_width"]
    poly = catmull_rom(stage["centre"], samples_per_span=16)

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

    hazards = 0
    for frac in stage.get("boost_at", ()):
        idx = int(n_poly * frac) % n_poly
        px, py = poly[idx]
        hazards += stamp_near(
            min(w - 1, max(0, int(px))), min(h - 1, max(0, int(py))), T_BOOST, 0
        )

    for frac in stage.get("oil_at", ()):
        idx = int(n_poly * frac) % n_poly
        px, py = poly[idx]
        hazards += stamp_near(
            min(w - 1, max(0, int(px))), min(h - 1, max(0, int(py))), T_OIL, 0
        )

    if hazards < 2:
        raise RuntimeError(
            f"{stage['name']}: only {hazards} hazard tile(s) placed, need >= 2"
        )

    start = (start[0], start[1])
    # Initial heading: follow the centreline away from the start tile.
    nx, ny = poly[3]
    start_heading = heading_from_vector(nx - start_pt[0], -(ny - start_pt[1]))

    return {
        "name": stage["name"],
        "w": w,
        "h": h,
        "tiles": tiles,
        "start": start,
        "start_heading": start_heading,
        "checkpoints": cps,
        "par": (stage["bronze_cs"], stage["silver_cs"], stage["gold_cs"], stage["dev_cs"]),
        "poly": poly,
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
    if not (10 <= w <= 32 and 10 <= h <= 32):
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
    dupes = len(set(checkpoints)) != len(checkpoints)
    if dupes:
        errors.append("duplicate checkpoint tiles")
    if errors:
        print(f"  !! track {idx} ({name}): " + "; ".join(errors), file=sys.stderr)
    return errors


def reachable_road_fraction(tiles, w, h, start):
    """Fraction of drivable tiles reachable from the start without leaving the
    level bounds (all terrain is drivable in FX, so this should be 100%)."""
    drivable = sum(1 for t in tiles if t != T_BARRIER)
    return drivable


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
        start_heading = heading_from_fx(stype)
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
            start_heading = 1024
        elif sy + 1 < h and tiles[(sy + 1) * w + sx] == T_TARMAC:
            start_heading = 0
        else:
            start_heading = 2048
        tiles[sy * w + sx] = T_START

    raw_cps = synthesize_gates(tiles, w, h, start, raw_cps)
    checkpoints = order_checkpoints(start, raw_cps, start_heading)
    paint_corridor(tiles, w, h, start, checkpoints)
    # Re-stamp the gates: the corridor pass turns them into plain tarmac.
    tiles[start[1] * w + start[0]] = T_START
    for (cx, cy) in checkpoints:
        tiles[cy * w + cx] = T_CHECKPOINT
    par = calibration[name]

    validate(idx, name, w, h, tiles, start, checkpoints)
    emit_track_const(code, idx, name, w, h, tiles, start, start_heading, checkpoints, par)


def emit_track_const(code, idx, name, w, h, tiles, start, start_heading, checkpoints, par):
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
    for cidx in range(16):
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
    code.append("};")
    code.append("")


def generate_rust_code():
    calibration = load_par_calibration()
    code = []
    code.append("//! Racetrack definitions for Arduracer PSX.")
    code.append("//!")
    code.append("//! Generated by `tools/track_cook/convert_levels.py` - do not edit by hand.")
    code.append("//! 20 remastered ArduRacer FX circuits + 4 PSX Grand Prix Super Stages.")
    code.append("")
    code.append("use crate::math::{Fixed, Vec2};")
    code.append("use crate::timing::{CheckpointGate, ParTimes};")
    code.append("use crate::track::{TrackDef, TrackTile};")
    code.append("")
    code.append("/// Padding entry for circuits with fewer than 16 checkpoint gates.")
    code.append("const EMPTY_GATE: CheckpointGate = CheckpointGate {")
    code.append("    x: 0,")
    code.append("    y: 0,")
    code.append("    width: 0,")
    code.append("    height: 0,")
    code.append("};")
    code.append("")

    all_tracks = []
    failures = []

    for i in range(1, 21):
        rows = load_level_csv(i)
        h = len(rows)
        w = len(rows[0])
        name = TRACK_NAMES[i - 1]
        before = len(code)
        emit_fx_track(code, i, name, rows, calibration)
        if len(code) == before:
            failures.append(i)
        all_tracks.append(f"TRACK_{i:02d}")

    for offset, stage in enumerate(SUPER_STAGES):
        idx = 21 + offset
        built = build_super_stage(stage)
        cps = order_checkpoints(built["start"], built["checkpoints"], built["start_heading"])
        if not validate(idx, built["name"], built["w"], built["h"], built["tiles"],
                        built["start"], cps):
            emit_track_const(code, idx, built["name"], built["w"], built["h"],
                             built["tiles"], built["start"], built["start_heading"],
                             cps, calibration[built["name"]])
        else:
            failures.append(idx)
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

    if failures:
        raise SystemExit(f"track validation failed for tracks: {failures}")

    return "\n".join(code)


if __name__ == "__main__":
    out_path = os.path.join(find_repo_root(), "crates", "arduracer-core", "src", "levels.rs")
    print(f"Generating {out_path}...")
    rust_code = generate_rust_code()
    with open(out_path, "w") as f:
        f.write(rust_code)
    print("Done! 24 tracks generated successfully.")
