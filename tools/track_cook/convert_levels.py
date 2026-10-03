#!/usr/bin/env python3
"""
Track Converter for Arduracer PSX.
Compiles 20 legacy ArduRacer FX levels + 4 PSX Grand Prix Super Stages
into pure `#![no_std]` Rust static definitions for arduracer-core.
"""

import os
import sys

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
    "Neo Tokyo Expressway", "Canyon Drift Apex", "Cyber Circuit 2097", "Monaco GP Classic"
]

def cs_to_ticks(cs: int) -> int:
    # 60 ticks per 100 centiseconds
    return (cs * 60) // 100

def find_repo_root():
    cur = os.path.abspath(os.path.dirname(__file__))
    while cur != "/":
        if os.path.exists(os.path.join(cur, "ArduRacerFx")):
            return cur
        cur = os.path.dirname(cur)
    return "/home/tonym/Projects/ArduracerPSX"

def load_level_csv(level_idx: int):
    root = find_repo_root()
    path = os.path.join(root, f"ArduRacerFx/Levels/Level{level_idx}.csv")
    with open(path) as f:
        rows = [list(map(int, line.strip().split(","))) for line in f if line.strip()]
    return rows

def generate_rust_code():
    code = []
    code.append("//! Automatically generated racetrack definitions for Arduracer PSX.")
    code.append("//! Remastered from ArduRacer FX with PSX Grand Prix Super Stages.")
    code.append("")
    code.append("use crate::math::{Fixed, Vec2};")
    code.append("use crate::timing::{CheckpointGate, ParTimes};")
    code.append("use crate::track::TrackDef;")
    code.append("")

    all_tracks = []

    # 1. Process 20 FX levels
    for i in range(1, 21):
        rows = load_level_csv(i)
        h = len(rows)
        w = len(rows[0])
        name = TRACK_NAMES[i - 1]
        
        # Find start and checkpoints
        starts = []
        checkpoints = []
        flat_tiles = []
        for y, row in enumerate(rows):
            for x, val in enumerate(row):
                flat_tiles.append(val)
                if val in (24, 25):
                    starts.append((x, y, val))
                elif val in (26, 27):
                    checkpoints.append((x, y))

        # Fallback for Level 7 where start was placed adjacent to checkpoint
        if not starts:
            if i == 7:
                starts = [(6, 1, 25)]
            else:
                starts = [(1, 1, 24)]

        sx, sy, stype = starts[0]
        # Heading: 24 horizontal -> 1024 (East); 25 vertical -> 0 (North)
        s_heading = 1024 if stype == 24 else 0

        par_dev, par_silver, par_bronze = PAR_TIMES_CS[i - 1]
        par_gold = (par_dev + par_silver) // 2

        var_tiles = f"TRACK_{i:02d}_TILES"
        var_track = f"TRACK_{i:02d}"

        code.append(f"const {var_tiles}: [u8; {len(flat_tiles)}] = [")
        for chunk_idx in range(0, len(flat_tiles), 16):
            chunk = flat_tiles[chunk_idx:chunk_idx + 16]
            code.append("    " + ", ".join(map(str, chunk)) + ",")
        code.append("];")
        code.append("")

        code.append(f"pub const {var_track}: TrackDef = TrackDef {{")
        code.append(f'    name: "{name}",')
        code.append(f"    width: {w},")
        code.append(f"    height: {h},")
        # World coordinates: (tx * 64 + 32) * FP_ONE
        start_x_raw = (sx * 64 + 32) * 4096
        start_y_raw = (sy * 64 + 32) * 4096
        code.append(f"    start_pos: Vec2 {{ x: Fixed({start_x_raw}), y: Fixed({start_y_raw}) }},")
        code.append(f"    start_heading: {s_heading},")
        code.append(f"    par_times: ParTimes {{")
        code.append(f"        bronze_ticks: {cs_to_ticks(par_bronze)},")
        code.append(f"        silver_ticks: {cs_to_ticks(par_silver)},")
        code.append(f"        gold_ticks: {cs_to_ticks(par_gold)},")
        code.append(f"        dev_platinum_ticks: {cs_to_ticks(par_dev)},")
        code.append(f"    }},")

        # Checkpoints
        code.append(f"    checkpoint_count: {min(len(checkpoints), 16)},")
        code.append("    checkpoints: [")
        for cp_idx in range(16):
            if cp_idx < len(checkpoints):
                cpx, cpy = checkpoints[cp_idx]
                code.append(f"        CheckpointGate {{ x: {cpx}, y: {cpy}, width: 1, height: 1 }},")
            else:
                code.append("        CheckpointGate { x: 0, y: 0, width: 0, height: 0 },")
        code.append("    ],")
        code.append(f"    tiles: &{var_tiles},")
        code.append("};")
        code.append("")
        all_tracks.append(var_track)

    # 2. Add 4 PSX Super Stages (21..24)
    super_stages = [
        (21, "Neo Tokyo Expressway", 16, 16, 3200, 2600, 2200, 1900),
        (22, "Canyon Drift Apex", 16, 16, 3400, 2800, 2400, 2050),
        (23, "Cyber Circuit 2097", 16, 16, 3600, 3000, 2500, 2150),
        (24, "Monaco GP Classic", 16, 16, 4000, 3400, 2800, 2350),
    ]

    for idx, name, w, h, b_cs, s_cs, g_cs, d_cs in super_stages:
        tiles = [1] * (w * h)
        # Add border walls
        for y in range(h):
            for x in range(w):
                if x == 0 or x == w - 1 or y == 0 or y == h - 1:
                    tiles[y * w + x] = 20 # Offroad border
        tiles[1 * w + 1] = 24 # Start line
        tiles[8 * w + 8] = 26 # Mid Checkpoint
        tiles[12 * w + 12] = 27 # Final Checkpoint

        var_tiles = f"TRACK_{idx:02d}_TILES"
        var_track = f"TRACK_{idx:02d}"

        code.append(f"const {var_tiles}: [u8; {len(tiles)}] = [")
        for chunk_idx in range(0, len(tiles), 16):
            chunk = tiles[chunk_idx:chunk_idx + 16]
            code.append("    " + ", ".join(map(str, chunk)) + ",")
        code.append("];")
        code.append("")

        start_x_raw = (1 * 64 + 32) * 4096
        start_y_raw = (1 * 64 + 32) * 4096
        code.append(f"pub const {var_track}: TrackDef = TrackDef {{")
        code.append(f'    name: "{name}",')
        code.append(f"    width: {w},")
        code.append(f"    height: {h},")
        code.append(f"    start_pos: Vec2 {{ x: Fixed({start_x_raw}), y: Fixed({start_y_raw}) }},")
        code.append(f"    start_heading: 1024,")
        code.append(f"    par_times: ParTimes {{")
        code.append(f"        bronze_ticks: {cs_to_ticks(b_cs)},")
        code.append(f"        silver_ticks: {cs_to_ticks(s_cs)},")
        code.append(f"        gold_ticks: {cs_to_ticks(g_cs)},")
        code.append(f"        dev_platinum_ticks: {cs_to_ticks(d_cs)},")
        code.append(f"    }},")
        code.append("    checkpoint_count: 2,")
        code.append("    checkpoints: [")
        code.append("        CheckpointGate { x: 8, y: 8, width: 1, height: 1 },")
        code.append("        CheckpointGate { x: 12, y: 12, width: 1, height: 1 },")
        for _ in range(14):
            code.append("        CheckpointGate { x: 0, y: 0, width: 0, height: 0 },")
        code.append("    ],")
        code.append(f"    tiles: &{var_tiles},")
        code.append("};")
        code.append("")
        all_tracks.append(var_track)

    code.append(f"/// All {len(all_tracks)} official tracks in Arduracer PSX.")
    code.append(f"pub const ALL_TRACKS: [&TrackDef; {len(all_tracks)}] = [")
    for trk in all_tracks:
        code.append(f"    &{trk},")
    code.append("];")
    code.append("")

    return "\n".join(code)

if __name__ == "__main__":
    out_path = "crates/arduracer-core/src/levels.rs"
    print(f"Generating {out_path}...")
    rust_code = generate_rust_code()
    with open(out_path, "w") as f:
        f.write(rust_code)
    print("Done! 24 tracks generated successfully.")
