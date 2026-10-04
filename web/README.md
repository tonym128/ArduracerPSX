# Arduracer PSX Web Arcade

An authentic browser-based arcade frontend for **Arduracer PSX**, running on real-time WebAssembly via **PSoXide** (60 FPS top-down motorsport, MIPS R3000 emulation, GTE math, SPU audio, CD-DA soundtrack, and MDEC FMV playback).

## Architecture

- **`index.html`**: Arcade bezel with Sony Trinitron HR monitor simulation, CRT shader filters (scanlines/glow), live DualShock controller HUD, on-screen virtual touch controls, 24 circuits guide, and manual.
- **`app.js`**: Companion controller bridge handling keyboard, touch input, HTML5 Gamepad API polling, and postMessage communication with PSoXide wasm player.
- **`player/`**: PSoXide WebAssembly runtime (`frontend.wasm`, WebGL display canvas, WebAudio stereo mixer).
- **`roms/`**: Bootable PS1 master disc images:
  - `arduracer.cue` + `arduracer.bin` (Red Book CD with CD-DA tracks & MDEC video, 185 MB)
  - `arduracer.iso` (Mode 1 data track ISO, 2 MB)
  - `arduracer.exe` (MIPS bare-metal executable, 304 KB)

## Running Locally

To run the web arcade locally with HTTP 206 Partial Content (Range request) streaming for the 185 MB CD image:

```bash
make web
```

Or invoke the server directly:

```bash
python3 tools/serve_web.py 8080 web
```

Then navigate to `http://localhost:8080`.
