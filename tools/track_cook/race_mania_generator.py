#!/usr/bin/env python3
"""
2D Race Mania Track Generator for ArduracerPSX.

Generates dense, authentic race tracks inspired by classic top-down racers
(Race Mania, Micro Machines, GeneRally, Super Cars II).

Unlike radial generators that produce circular/oval tracks, this generator
routes through the central infield using:
1. Macro-Lattice Cycle Routing with Infield Ingress.
2. Classic Racing Motifs:
   - Long Main Straights with heavy braking zones.
   - Tight 180° Hairpin complexes for drifting.
   - Rapid S-bend Chicanes.
   - Sweeping Carousels (high-speed arcs).
   - Infield switchbacks utilizing the center of the 80x80 grid.
3. Strict Clearance Enforcement (parallel lane separation >= 10 cells)
   to ensure zero self-intersections and clean compilation in `compile_circuit.py`.
"""
from __future__ import annotations

import math
import random
from dataclasses import dataclass, field
from typing import Any

# Standard ArduracerPSX constants
GRID = 80
ROAD_HALF_CELLS = 3.0
BORDER_CELLS = 7.1  # Road + Kerb + Runoff + Barrier = 3.0 + 0.7 + 2.4 + 1.0
SAFE_MIN = 14.0
SAFE_MAX = 66.0
MIN_LANE_SEP = 10.0  # Minimum separation between non-adjacent track centrelines


def point_segment_distance(px: float, py: float, x1: float, y1: float, x2: float, y2: float) -> float:
    """Euclidean distance from point (px, py) to line segment (x1, y1)-(x2, y2)."""
    dx, dy = x2 - x1, y2 - y1
    l2 = dx * dx + dy * dy
    if l2 < 1e-9:
        return math.hypot(px - x1, py - y1)
    t = max(0.0, min(1.0, ((px - x1) * dx + (py - y1) * dy) / l2))
    proj_x = x1 + t * dx
    proj_y = y1 + t * dy
    return math.hypot(px - proj_x, py - proj_y)


def segment_segment_distance(p1: tuple[float, float], p2: tuple[float, float],
                             q1: tuple[float, float], q2: tuple[float, float]) -> float:
    """Minimum Euclidean distance between two line segments."""
    # Check for intersection
    def ccw(a, b, c):
        return (c[1] - a[1]) * (b[0] - a[0]) > (b[1] - a[1]) * (c[0] - a[0])

    intersect = (ccw(p1, q1, q2) != ccw(p2, q1, q2)) and (ccw(p1, p2, q1) != ccw(p1, p2, q2))
    if intersect:
        return 0.0

    return min(
        point_segment_distance(p1[0], p1[1], q1[0], q1[1], q2[0], q2[1]),
        point_segment_distance(p2[0], p2[1], q1[0], q1[1], q2[0], q2[1]),
        point_segment_distance(q1[0], q1[1], p1[0], p1[1], p2[0], p2[1]),
        point_segment_distance(q2[0], q2[1], p1[0], p1[1], p2[0], p2[1]),
    )


def validate_clearance(vertices: list[tuple[float, float]], min_sep: float = MIN_LANE_SEP) -> tuple[bool, str]:
    """Validates that no non-adjacent track segments come closer than `min_sep`."""
    n = len(vertices)
    for i in range(n):
        p1 = vertices[i]
        p2 = vertices[(i + 1) % n]
        # Check against non-adjacent segments
        for j in range(i + 2, n):
            if i == 0 and j == n - 1:
                continue  # Adjacent at loop closure
            q1 = vertices[j]
            q2 = vertices[(j + 1) % n]
            dist = segment_segment_distance(p1, p2, q1, q2)
            if dist < min_sep:
                return False, f"Segments ({i}->{i+1}) and ({j}->{j+1}) clearance too close: {dist:.2f} < {min_sep}"

    # Also check bounds
    for i, (x, y) in enumerate(vertices):
        if not (SAFE_MIN - 1.0 <= x <= SAFE_MAX + 1.0 and SAFE_MIN - 1.0 <= y <= SAFE_MAX + 1.0):
            return False, f"Vertex {i} at ({x:.1f}, {y:.1f}) exceeds safe bounds [{SAFE_MIN}, {SAFE_MAX}]"

    return True, "OK"


# ------------------------------------------------------------------------------
# Race Track Archetype Blueprints
# Each blueprint defines an authentic racing layout with infield routing.
# ------------------------------------------------------------------------------

TRACK_ARCHETYPES = [
    {
        "archetype": "suzuka_ribbon",
        "name_template": "{prefix} GP",
        "description": "Grand Prix folded ribbon: long straight, sweeper, tight infield hairpin, and S-chicane.",
        "base_vertices": [
            (16.0, 16.0),  # Start of main straight
            (64.0, 16.0),  # End of main straight into Turn 1
            (64.0, 38.0),  # Turn 1 sweeper
            (40.0, 38.0),  # Dive into infield
            (40.0, 52.0),  # Infield hairpin
            (54.0, 52.0),  # Infield dogleg
            (54.0, 64.0),  # South outer entry
            (16.0, 64.0),  # Bottom straight
        ],
        "radii": [5.0, 5.0, 4.0, 4.0, 4.0, 4.0, 4.5, 5.0],
        "shortcuts": ["M 64.0 38.0 L 54.0 52.0"],
    },
    {
        "archetype": "infield_serpent",
        "name_template": "{prefix} Serpent",
        "description": "Rallycross switchback: double hairpin slalom weaving back and forth across the infield.",
        "base_vertices": [
            (16.0, 16.0),  # Start
            (64.0, 16.0),  # Top blast
            (64.0, 28.0),  # Turn 1
            (30.0, 28.0),  # Hairpin 1 back west
            (30.0, 42.0),  # Down
            (62.0, 42.0),  # Hairpin 2 back east
            (62.0, 62.0),  # South outer
            (16.0, 62.0),  # Bottom straight
            (16.0, 36.0),  # West return
        ],
        "radii": [5.0, 5.0, 4.0, 4.0, 4.0, 4.0, 5.0, 5.0, 4.5],
        "shortcuts": ["M 30.0 28.0 L 30.0 42.0"],
    },
    {
        "archetype": "hammerhead_pro",
        "name_template": "{prefix} Hammerhead",
        "description": "Technical airfield circuit with hammerhead loop, high-speed perimeter, and chicane bypass.",
        "base_vertices": [
            (16.0, 18.0),  # Main straight
            (62.0, 18.0),
            (62.0, 36.0),
            (48.0, 36.0),  # Infield entry
            (48.0, 48.0),  # Hammerhead neck
            (62.0, 48.0),  # Hammerhead right lobe
            (62.0, 62.0),  # South corner
            (34.0, 62.0),  # West return
            (34.0, 48.0),  # Hammerhead left lobe
            (20.0, 48.0),
            (16.0, 34.0),
        ],
        "radii": [5.0, 5.0, 4.0, 3.5, 3.5, 4.5, 5.0, 4.0, 3.5, 4.0, 4.5],
        "shortcuts": ["M 48.0 36.0 L 34.0 62.0"],
    },
    {
        "archetype": "monza_parabolica",
        "name_template": "{prefix} Autodromo",
        "description": "High-speed temple: twin blast straights connected by a double chicane and wide parabolica.",
        "base_vertices": [
            (16.0, 16.0),  # Main straight
            (64.0, 16.0),  # High-speed braking zone
            (64.0, 32.0),  # Prima Variante (Chicane 1)
            (52.0, 32.0),
            (52.0, 46.0),  # Variante della Roggia
            (64.0, 46.0),
            (64.0, 64.0),  # Parabolica entry
            (16.0, 64.0),  # Long back straight
            (16.0, 40.0),  # Curva Grande
        ],
        "radii": [5.0, 4.5, 3.5, 3.5, 3.5, 4.0, 6.0, 5.0, 5.0],
        "shortcuts": ["M 64.0 32.0 L 64.0 46.0"],
    },
    {
        "archetype": "stadium_technical",
        "name_template": "{prefix} Arena",
        "description": "Stadium karting layout: fast sweeping outer ring wrapping around an intricate infield maze.",
        "base_vertices": [
            (16.0, 16.0),
            (64.0, 16.0),
            (64.0, 64.0),
            (50.0, 64.0),
            (50.0, 36.0),  # Infield dive
            (36.0, 36.0),  # Infield hairpin
            (36.0, 52.0),  # Infield south
            (26.0, 52.0),  # Hairpin west
            (26.0, 32.0),  # North loop
            (16.0, 32.0),
        ],
        "radii": [5.0, 5.0, 5.0, 4.0, 4.0, 4.0, 4.0, 4.0, 4.5, 4.5],
        "shortcuts": ["M 50.0 64.0 L 26.0 52.0"],
    },
    {
        "archetype": "twin_loops",
        "name_template": "{prefix} Twin",
        "description": "Hourglass twin circuit: two broad sweeping loops joined by a narrow technical neck.",
        "base_vertices": [
            (16.0, 18.0),
            (42.0, 18.0),
            (42.0, 34.0),  # Neck
            (62.0, 34.0),  # East lobe
            (62.0, 62.0),
            (36.0, 62.0),
            (36.0, 46.0),  # Return neck
            (16.0, 46.0),  # West lobe
        ],
        "radii": [5.0, 4.5, 4.0, 4.5, 5.0, 4.5, 4.0, 5.0],
        "shortcuts": ["M 42.0 34.0 L 36.0 46.0"],
    },
]


def generate_race_mania_track(
    seed: int,
    archetype_idx: int | None = None,
    name: str = "",
    runoff: int = 5,  # GRASS default
    grid: int = GRID,
) -> dict[str, Any]:
    """Generates a complete circuit specification with authentic 2D Race Mania features."""
    rng = random.Random(seed)

    if archetype_idx is None:
        archetype_idx = rng.randrange(len(TRACK_ARCHETYPES))
    template = TRACK_ARCHETYPES[archetype_idx % len(TRACK_ARCHETYPES)]

    # Perturb vertices slightly based on seed while preserving clearances
    base_v = template["base_vertices"]
    v_out: list[tuple[float, float]] = []

    # Jitter vertices within safe bounds (up to +/- 2.5 cells)
    for x, y in base_v:
        jx = x + rng.uniform(-2.0, 2.0)
        jy = y + rng.uniform(-2.0, 2.0)
        jx = max(SAFE_MIN, min(SAFE_MAX, jx))
        jy = max(SAFE_MIN, min(SAFE_MAX, jy))
        v_out.append((round(jx, 1), round(jy, 1)))

    # Compute default corner radii if not provided
    radii = template.get("radii")
    if radii is None or len(radii) != len(v_out):
        radii = [round(rng.uniform(3.8, 5.2), 1) for _ in v_out]

    # Validate clearance
    ok, reason = validate_clearance(v_out)
    if not ok:
        # Fall back to un-jittered base vertices if jitter caused clearance issue
        v_out = [(round(x, 1), round(y, 1)) for x, y in base_v]
        radii = template.get("radii", [4.5] * len(v_out))

    # Naming
    if not name:
        prefixes = ["Apex", "Thunder", "Cobalt", "Ironstone", "Crimson", "Emerald", "Falcon", "Obsidian"]
        pfx = rng.choice(prefixes)
        name = template["name_template"].format(prefix=pfx)

    # Standard marks (start line, 6 gates, 2-3 boost pads, 1-2 oil slicks)
    boosts = (rng.choice([1, 2]), rng.choice([4, 5]))
    oils = (rng.choice([3, 6]),)

    # Calculate marks using standard gate spacing
    step = 1.0 / 7.0
    marks = [(0.0, 10, ROAD_HALF_CELLS + 0.7, 1.2)]  # Start Line
    for i in range(1, 7):
        marks.append((i * step, 11, ROAD_HALF_CELLS + 0.7, 0.7))  # Gates
    for b in boosts:
        marks.append(((b - 0.5) * step, 9, ROAD_HALF_CELLS, 1.0))  # Boost
    for o in oils:
        marks.append(((o - 0.25) * step, 8, 1.5, 0.8))  # Oil

    shortcuts = template.get("shortcuts", [])

    return {
        "name": name,
        "shape": "polygon",
        "grid": grid,
        "vertices": v_out,
        "radii": radii,
        "shortcuts": shortcuts,
        "runoff": runoff,
        "marks": marks,
        "note": f"{template['description']} (Seed {seed})",
    }


# ------------------------------------------------------------------------------
# The Dedicated 6-Track Mania Cup Set
# ------------------------------------------------------------------------------

MANIA_CUP_TRACKS = [
    # 1. Metro 8 (Figure-8 Crossover) - Stresses Traffic Awareness & Brawling
    {
        "name": "Metro 8",
        "shape": "polygon",
        "grid": 80,
        "road_half_cells": 3.0,
        "vertices": [
            (18.0, 18.0),
            (44.0, 18.0),
            (44.0, 32.0),
            (62.0, 32.0),
            (62.0, 62.0),
            (36.0, 62.0),
            (36.0, 48.0),
            (18.0, 48.0),
        ],
        "radii": [5.0, 4.5, 4.0, 4.5, 5.0, 5.0, 4.0, 5.0],
        "shortcuts": ["M 44.0 32.0 L 36.0 48.0"],
        "runoff": 6,  # GRAVEL
        "marks": [
            (0.0, 10, 3.7, 1.2),
            (1 / 7, 11, 3.7, 0.7),
            (2 / 7, 11, 3.7, 0.7),
            (3 / 7, 11, 3.7, 0.7),
            (4 / 7, 11, 3.7, 0.7),
            (5 / 7, 11, 3.7, 0.7),
            (6 / 7, 11, 3.7, 0.7),
            (0.5 / 7, 9, 3.0, 1.0),
            (3.5 / 7, 9, 3.0, 1.0),
            (2.75 / 7, 8, 1.5, 0.8),
        ],
        "note": "Figure-8 crossover circuit with central crossover neck and bypass chute. Stresses traffic timing and rival brawling.",
    },
    # 2. Speedway Boulevard (Super-wide Top Speed) - Stresses Top Speed & Long Gearing (VIPER setup)
    {
        "name": "Speedway Boulevard",
        "shape": "polygon",
        "grid": 80,
        "road_half_cells": 4.0,
        "vertices": [
            (16.0, 18.0),
            (64.0, 18.0),
            (64.0, 62.0),
            (16.0, 62.0),
        ],
        "radii": [7.0, 7.0, 7.0, 7.0],
        "shortcuts": [],
        "runoff": 5,  # GRASS
        "marks": [
            (0.0, 10, 4.7, 1.2),
            (1 / 7, 11, 4.7, 0.7),
            (2 / 7, 11, 4.7, 0.7),
            (3 / 7, 11, 4.7, 0.7),
            (4 / 7, 11, 4.7, 0.7),
            (5 / 7, 11, 4.7, 0.7),
            (6 / 7, 11, 4.7, 0.7),
            (0.5 / 7, 9, 4.0, 1.0),
            (2.5 / 7, 9, 4.0, 1.0),
            (4.5 / 7, 9, 4.0, 1.0),
        ],
        "note": "Super-wide multi-lane dragway stressing maximum top speed, slipstreaming, and long gearing.",
    },
    # 3. Serpent Canyon (Hairpin Drift & Counter-Steer) - Stresses Drift Stability & Handling (BLAZE setup)
    {
        "name": "Serpent Canyon",
        "shape": "polygon",
        "grid": 80,
        "road_half_cells": 3.0,
        "vertices": [
            (16.0, 16.0),
            (64.0, 16.0),
            (64.0, 32.0),
            (28.0, 32.0),
            (28.0, 48.0),
            (64.0, 48.0),
            (64.0, 64.0),
            (16.0, 64.0),
            (16.0, 36.0),
        ],
        "radii": [5.0, 5.0, 4.5, 4.5, 4.5, 4.5, 5.0, 5.0, 5.0],
        "shortcuts": [],
        "runoff": 7,  # SAND
        "marks": [
            (0.0, 10, 3.7, 1.2),
            (0.2287, 11, 3.7, 0.7),
            (0.3340, 11, 3.7, 0.7),
            (0.4231, 11, 3.7, 0.7),
            (0.5121, 11, 3.7, 0.7),
            (0.6174, 11, 3.7, 0.7),
            (0.7540, 11, 3.7, 0.7),
            (0.092, 9, 3.0, 1.0),
            (0.697, 9, 3.0, 1.0),
        ],
        "note": "Technical switchback circuit with sequential 180-degree hairpins stressing drift stability and counter-steering.",
    },
    # 4. Alleyway Chicanes (Narrow Urban Maze) - Stresses Handling & Short Gearing (RAZOR setup)
    {
        "name": "Alleyway Chicanes",
        "shape": "polygon",
        "grid": 80,
        "road_half_cells": 2.0,
        "vertices": [
            (16.0, 16.0),
            (64.0, 16.0),
            (64.0, 36.0),
            (48.0, 36.0),
            (48.0, 48.0),
            (64.0, 48.0),
            (64.0, 64.0),
            (34.0, 64.0),
            (34.0, 48.0),
            (16.0, 48.0),
        ],
        "radii": [4.0, 4.0, 3.5, 3.5, 3.5, 3.5, 4.0, 3.5, 3.5, 4.0],
        "shortcuts": ["M 48.0 36.0 L 34.0 64.0"],
        "runoff": 6,  # GRAVEL
        "marks": [
            (0.0, 10, 2.7, 1.2),
            (1 / 7, 11, 2.7, 0.7),
            (2 / 7, 11, 2.7, 0.7),
            (3 / 7, 11, 2.7, 0.7),
            (4 / 7, 11, 2.7, 0.7),
            (5 / 7, 11, 2.7, 0.7),
            (6 / 7, 11, 2.7, 0.7),
            (1.5 / 7, 9, 2.0, 1.0),
            (4.5 / 7, 9, 2.0, 1.0),
            (3.75 / 7, 8, 1.5, 0.8),
        ],
        "note": "Tight urban street circuit with narrow corridors, chicanes, and alleyway shortcut stressing handling.",
    },
    # 5. Oil Rig Slipway (Low-Grip Hazard Management) - Stresses Throttle Control & Low Traction
    {
        "name": "Oil Rig Slipway",
        "shape": "polygon",
        "grid": 80,
        "road_half_cells": 3.0,
        "vertices": [
            (16.0, 18.0),
            (42.0, 18.0),
            (42.0, 34.0),
            (62.0, 34.0),
            (62.0, 62.0),
            (36.0, 62.0),
            (36.0, 46.0),
            (16.0, 46.0),
        ],
        "radii": [5.0, 4.5, 4.0, 4.5, 5.0, 4.5, 4.0, 5.0],
        "shortcuts": [],
        "runoff": 7,  # SAND
        "marks": [
            (0.0, 10, 3.7, 1.2),
            (1 / 7, 11, 3.7, 0.7),
            (2 / 7, 11, 3.7, 0.7),
            (3 / 7, 11, 3.7, 0.7),
            (4 / 7, 11, 3.7, 0.7),
            (5 / 7, 11, 3.7, 0.7),
            (6 / 7, 11, 3.7, 0.7),
            (1.5 / 7, 9, 3.0, 1.0),
            (0.75 / 7, 8, 1.5, 0.8),
            (2.75 / 7, 8, 1.5, 0.8),
            (4.75 / 7, 8, 1.5, 0.8),
        ],
        "note": "Slick industrial dockside track with multiple oil hazards on apexes stressing throttle discipline.",
    },
    # 6. Apex Arena (Complete Grand Championship) - Stresses Balanced Complete Tuning
    {
        "name": "Apex Arena",
        "shape": "polygon",
        "grid": 80,
        "road_half_cells": 3.0,
        "vertices": [
            (16.0, 16.0),
            (64.0, 16.0),
            (64.0, 64.0),
            (50.0, 64.0),
            (50.0, 36.0),
            (36.0, 36.0),
            (36.0, 52.0),
            (26.0, 52.0),
            (26.0, 32.0),
            (16.0, 32.0),
        ],
        "radii": [5.0, 5.0, 5.0, 4.0, 4.0, 4.0, 4.0, 4.0, 4.5, 4.5],
        "shortcuts": ["M 50.0 64.0 L 26.0 52.0"],
        "runoff": 5,  # GRASS
        "marks": [
            (0.0, 10, 3.7, 1.2),
            (1 / 7, 11, 3.7, 0.7),
            (2 / 7, 11, 3.7, 0.7),
            (3 / 7, 11, 3.7, 0.7),
            (4 / 7, 11, 3.7, 0.7),
            (5 / 7, 11, 3.7, 0.7),
            (6 / 7, 11, 3.7, 0.7),
            (0.5 / 7, 9, 3.0, 1.0),
            (3.5 / 7, 9, 3.0, 1.0),
            (2.75 / 7, 8, 1.5, 0.8),
            (5.75 / 7, 8, 1.5, 0.8),
        ],
        "note": "Championship stadium test combining wide straights, technical chicanes, hairpins, and shortcuts.",
    },
]


def write_race_mania_track(spec: dict[str, Any], outdir: str) -> dict[str, Any]:
    """Renders and writes all track assets into `outdir` using `generate_svg_circuit`."""
    import generate_svg_circuit as gen
    import compile_circuit as cc

    # Validate before writing
    svg_text, centre = gen.build_svg(spec)
    fine = gen.rasterise(svg_text, gen.DATA_PX * gen.SUPERSAMPLE, spec.get("grid", GRID))
    codes = gen.resolve_cells(fine, gen.DATA_PX * gen.SUPERSAMPLE)
    errors = []
    cc.validate(codes, 1, errors)
    if errors:
        raise ValueError(f"Circuit validation failed for {spec['name']}: {errors}")

    return gen.write_outputs(spec, outdir)


def main() -> int:
    import argparse
    import os

    ap = argparse.ArgumentParser(
        description="2D Race Mania Track Generator for ArduracerPSX",
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    ap.add_argument("--list-archetypes", action="store_true", help="List available track archetypes")
    ap.add_argument("--seed", type=int, default=42, help="Random seed (default: 42)")
    ap.add_argument("--archetype", type=int, default=None, help="Archetype index (0 to 5)")
    ap.add_argument("--name", type=str, default="", help="Custom circuit name")
    ap.add_argument("--runoff", type=str, default="grass", choices=["grass", "gravel", "sand"],
                    help="Runoff surface type")
    ap.add_argument("--write", action="store_true", help="Write circuit assets to tracks/ directory")
    ap.add_argument("--batch", type=int, default=0, help="Generate N distinct Race Mania circuits")
    ap.add_argument("--mania-cup", action="store_true", help="Generate all 6 Mania Cup curated circuits")
    ap.add_argument("--out", type=str, default=None, help="Output directory (default: <repo>/tracks)")

    args = ap.parse_args()

    if args.list_archetypes:
        print("Available Race Mania Track Archetypes:")
        for i, a in enumerate(TRACK_ARCHETYPES):
            print(f"  [{i}] {a['archetype']:20} - {a['description']}")
        return 0

    root = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
    outdir = args.out or os.path.join(root, "tracks")

    if args.mania_cup:
        print(f"Generating 6 curated Mania Cup circuits into {outdir}...")
        for i, spec in enumerate(MANIA_CUP_TRACKS):
            ok, msg = validate_clearance(spec["vertices"])
            road_w = spec.get("road_half_cells", 3.0) * 2
            sc_count = len(spec.get("shortcuts", []))
            sc_str = f"with {sc_count} shortcut" if sc_count else "no shortcuts"
            if args.write:
                info = write_race_mania_track(spec, outdir)
                print(f"  [{i+1}/6] Wrote {info['name']:20} ({road_w:.1f} cells wide, {sc_str}) -> {os.path.basename(info['svg'])}")
            else:
                print(f"  [{i+1}/6] Preview {spec['name']:20} ({road_w:.1f} cells wide, {sc_str}) - {spec['note']}")
        if not args.write:
            print("\nRun with --mania-cup --write to compile and save all 6 tracks into tracks/")
        return 0

    runoff_map = {"grass": 5, "gravel": 6, "sand": 7}
    runoff_code = runoff_map[args.runoff.lower()]

    if args.batch > 0:
        print(f"Generating {args.batch} Race Mania circuits into {outdir}...")
        for i in range(args.batch):
            s = (args.seed * 1_000_003 + i * 997) & 0xFFFFFFFF
            arch_idx = i % len(TRACK_ARCHETYPES)
            spec = generate_race_mania_track(seed=s, archetype_idx=arch_idx, runoff=runoff_code)
            ok, msg = validate_clearance(spec["vertices"])
            assert ok, f"Batch track {i} failed clearance: {msg}"
            if args.write:
                info = write_race_mania_track(spec, outdir)
                print(f"  [{i+1}/{args.batch}] Wrote {info['name']} ({len(spec['vertices'])} vertices)")
            else:
                print(f"  [{i+1}/{args.batch}] Preview {spec['name']} ({len(spec['vertices'])} vertices) - OK")
        return 0

    spec = generate_race_mania_track(
        seed=args.seed,
        archetype_idx=args.archetype,
        name=args.name,
        runoff=runoff_code,
    )
    ok, msg = validate_clearance(spec["vertices"])
    print(f"Circuit: {spec['name']}")
    print(f"Archetype: {spec['note']}")
    print(f"Vertices ({len(spec['vertices'])}): {spec['vertices']}")
    print(f"Clearance Status: {msg}")

    if args.write:
        info = write_race_mania_track(spec, outdir)
        print(f"\nSuccessfully wrote circuit assets to {outdir}:")
        print(f"  SVG:      {os.path.basename(info['svg'])}")
        print(f"  Data:     {os.path.basename(info['data'])}")
        print(f"  Visual:   {os.path.basename(info['visual'])}")
        print(f"  Line:     {os.path.basename(info['line'])}")
        print(f"  JPEG:     {os.path.basename(info['jpeg'])}")
    else:
        print("\nRun with --write to generate full SVG, visual PNG, and level data into tracks/")

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
