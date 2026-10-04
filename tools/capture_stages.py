#!/usr/bin/env python3
"""
Automated headless screenshot capture harness for all 24 Arduracer PSX stages.

Runs the PSoXide headless frontend, navigates the track select menu via scheduled
controller pulses, lets the race start, and captures pixel-accurate 320x240 PNGs
for visual auditing.
"""

import os
import subprocess
import sys
import time

TRACKS = [
    (0, "Arduboy Oval", "arduboy_oval"),
    (1, "Twin Hairpin", "twin_hairpin"),
    (2, "The Serpent", "the_serpent"),
    (3, "Canyon Chicane", "canyon_chicane"),
    (4, "Switchback Pass", "switchback_pass"),
    (5, "Grand Ring", "grand_ring"),
    (6, "Sprint Short", "sprint_short"),
    (7, "Octagon Speedway", "octagon_speedway"),
    (8, "Devil's Elbow", "devils_elbow"),
    (9, "Metropolis 10", "metropolis_10"),
    (10, "Forest Expressway", "forest_expressway"),
    (11, "Coastal Link", "coastal_link"),
    (12, "Alpine Drift", "alpine_drift"),
    (13, "Industrial Yard", "industrial_yard"),
    (14, "Nightway Circuit", "nightway_circuit"),
    (15, "Harbor Slalom", "harbor_slalom"),
    (16, "Mountain Gauntlet", "mountain_gauntlet"),
    (17, "Super Speedway", "super_speedway"),
    (18, "Endurance Colosseum", "endurance_colosseum"),
    (19, "Championship Final", "championship_final"),
    (20, "Neo Tokyo Expressway", "neo_tokyo_expressway"),
    (21, "Canyon Drift Apex", "canyon_drift_apex"),
    (22, "Cyber Circuit 2097", "cyber_circuit_2097"),
    (23, "Monaco GP Classic", "monaco_gp_classic"),
]

EMULATOR = "./PSoXide-emulator/target/release/frontend"
CUE_PATH = "dist/arduracer.cue"
OUTPUT_DIR = "screenshots"
TEMP_PPM = "/tmp/arduracer_stage_capture.ppm"


def capture_stage(idx: int, name: str, slug: str):
    stage_num = idx + 1
    png_path = os.path.join(OUTPUT_DIR, f"stage_{stage_num:02d}_{slug}.png")

    # Generate press sequence
    # Tick 50: skip FMV intro -> MainMenu
    # Tick 70: select TimeTrial -> TrackSelect
    # Tick 80+: pulse Right N times with stride 4, then Cross
    if idx == 0:
        press = "50:start:4,70:cross:4,80:cross:4"
    else:
        rights = [f"{80 + 4 * i}:right:2" for i in range(idx)]
        confirm_tick = 80 + 4 * idx
        press = f"50:start:4,70:cross:4,{','.join(rights)},{confirm_tick}:cross:4"

    # Step budget: base 36M + 1.6M per menu pulse
    steps = 36_000_000 + idx * 1_600_000

    print(f"[{stage_num:02d}/24] Capturing '{name}' (steps={steps})...", flush=True)

    if os.path.exists(TEMP_PPM):
        os.remove(TEMP_PPM)

    cmd = [
        EMULATOR,
        "launch",
        "--path",
        CUE_PATH,
        "--steps",
        str(steps),
        "--press",
        press,
        "--dump-display",
        TEMP_PPM,
    ]

    t0 = time.time()
    res = subprocess.run(cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    if res.returncode != 0:
        print(f"Error running emulator for stage {stage_num}:", file=sys.stderr)
        print(res.stderr, file=sys.stderr)
        return False

    if not os.path.exists(TEMP_PPM):
        print(f"PPM dump was not created for stage {stage_num}!", file=sys.stderr)
        return False

    # Convert to PNG
    conv = subprocess.run(
        ["ffmpeg", "-y", "-i", TEMP_PPM, png_path],
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    if conv.returncode != 0:
        print(f"ffmpeg conversion failed for stage {stage_num}:", file=sys.stderr)
        print(conv.stderr.decode("utf-8", errors="replace"), file=sys.stderr)
        return False

    if os.path.exists(TEMP_PPM):
        os.remove(TEMP_PPM)

    elapsed = time.time() - t0
    file_size = os.path.getsize(png_path)
    print(
        f"       -> Saved {png_path} ({file_size} bytes, took {elapsed:.2f}s)",
        flush=True,
    )
    return True


def main():
    os.makedirs(OUTPUT_DIR, exist_ok=True)
    success = 0
    total = len(TRACKS)

    for idx, name, slug in TRACKS:
        if capture_stage(idx, name, slug):
            success += 1
        else:
            print(f"FAILED to capture stage {idx + 1}: {name}", file=sys.stderr)

    print(f"\nCompleted capture: {success}/{total} stages captured successfully.")
    if success != total:
        sys.exit(1)


if __name__ == "__main__":
    main()
