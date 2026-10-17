#!/usr/bin/env python3
"""Drives the headless emulator to a race on one specific texture format.

Navigating the real settings menu beats seeding a `.mcd`: the card layer talks
to SIO0 through the BIOS card protocol, and a hand-built card image is not
accepted, so every run silently booted with the default format.

Emits the `--press` tape `frontend launch` wants:

    tick:button:duration,tick:button:duration,...

and optionally runs the capture and converts the PPM to PNG. The number of Right
presses on the TEXTURE row is what selects the format:

    0 = AUTO   1 = SVG   2 = QOI   3 = JPEG   4 = TIM   5 = TIM4   6 = TIM8
"""

import os
import subprocess
import sys

FRONTEND = "/home/tonym/Projects/ArduracerPSX/PSoXide-emulator/target/release/frontend"

# Format byte -> Right presses on the TEXTURE row. The menu cycles
# AUTO -> SVG -> QOI -> JPEG -> TIM -> TIM4 -> TIM8 byte-linearly.
FORMAT_RIGHT_PRESSES = {0: 0, 1: 1, 2: 2, 3: 3, 4: 4, 5: 5, 6: 6}


def tape(rights: int, fmt: int) -> str:
    p = []
    # `start` skips the FMV; a *second* press is needed to leave the title.
    p.append("50:start:4")
    p.append("300:cross:4")           # title -> main menu (selection on TIME TRIAL)
    t = 320
    for _ in range(5):                # down to SETTINGS
        p.append(f"{t}:down:2")
        t += 4
    p.append(f"{t}:cross:4")          # enter the settings screen
    t += 20
    for _ in range(rights):           # cycle TEXTURE
        p.append(f"{t}:right:2")
        t += 4
    p.append(f"{t}:cross:4")          # leave settings (applies + persists)
    t += 20
    for _ in range(5):                # back up to TIME TRIAL
        p.append(f"{t}:up:2")
        t += 4
    p.append(f"{t}:cross:4")          # time trial
    t += 20
    p.append(f"{t}:cross:4")          # track select -> start the race
    return ",".join(p)


def run(root: str, fmt: int, out_png: str, steps: int = 900_000_000):
    cue = os.path.join(root, "dist", "arduracer.cue")
    ppm = out_png + ".ppm"
    press = tape(FORMAT_RIGHT_PRESSES[fmt], fmt)
    cmd = [
        FRONTEND, "launch",
        "--path", cue,
        "--steps", str(steps),
        "--press", press,
        "--dump-display", ppm,
    ]
    res = subprocess.run(cmd, capture_output=True, text=True, timeout=3600)
    log = res.stdout + res.stderr
    for line in log.splitlines():
        if any(k in line for k in
               ("SETTINGS", "graphics:", "TRACKTEX] circuit", "port1-polls", "PLACEHOLDER",
                "TIM4", "codec=")):
            print("   ", line.strip())
    if not os.path.exists(ppm):
        print("    no PPM dumped")
        return False
    subprocess.run(["ffmpeg", "-y", "-i", ppm, out_png],
                   capture_output=True)
    os.remove(ppm)
    print(f"    -> {out_png} ({os.path.getsize(out_png)} bytes)")
    return True


if __name__ == "__main__":
    root = sys.argv[1] if len(sys.argv) > 1 else "."
    outdir = sys.argv[2] if len(sys.argv) > 2 else "/tmp/opencode/shots"
    fmts = [int(x) for x in sys.argv[3].split(",")] if len(sys.argv) > 3 else [5]
    steps = int(sys.argv[4]) if len(sys.argv) > 4 else 900_000_000
    os.makedirs(outdir, exist_ok=True)
    names = {0: "auto", 1: "svg", 2: "qoi", 3: "jpeg", 4: "tim", 5: "tim4"}
    for f in fmts:
        print(f"[{names[f]}] format byte {f}")
        run(root, f, os.path.join(outdir, f"race_{names[f]}.png"), steps)