# Arduracer PSX Web Arcade

An authentic browser-based arcade frontend for **Arduracer PSX**, running on real-time WebAssembly via **PSoXide** (60 FPS top-down motorsport, MIPS R3000 emulation, GTE math, SPU audio, CD-DA soundtrack, and MDEC FMV playback).

## Architecture

- **`index.html`**: Arcade bezel with Sony Trinitron HR monitor simulation, CRT shader filters (scanlines/glow), live DualShock controller HUD, on-screen virtual touch controls, 24 circuits guide, and manual.
- **`app.js`**: Companion controller bridge handling keyboard, touch input, HTML5 Gamepad API polling, and postMessage communication with PSoXide wasm player.
- **`player/`**: PSoXide WebAssembly runtime (`frontend.wasm`, WebGL display canvas, WebAudio stereo mixer).
- **`sw.js`**: Service Worker with HTTP 206 Range request slicing for cached disc streaming and offline PWA support.
- **`rom-cache.js`**: Cache manager tracking CacheStorage, download progress, and automatic cache flushing when new ROM versions are deployed.
- **`roms/`**: Bootable PS1 master disc images:
  - `arduracer.cue` + `arduracer.bin` (Red Book CD with CD-DA tracks & MDEC video, 185 MB)
  - `arduracer.iso` (Mode 1 data track ISO, 2 MB)
  - `arduracer.exe` (MIPS bare-metal executable, 304 KB)
  - `manifest.json` (SHA256 checksums, byte sizes, and build IDs used for automated cache invalidation)

## ROM Caching & Invalidation Architecture

To prevent redownloading the 185 MB disc image on every session:
1. **Persistent Caching**: Disc images and emulator assets are cached in the browser's persistent `CacheStorage` API (`arduracer-roms-v1`).
2. **Byte-Range Slicing**: `sw.js` intercepts `Range: bytes=start-end` requests from PSoXide. When an image is cached, the Service Worker slices the cached Blob and returns HTTP 206 Partial Content, delivering zero-latency disc reads.
3. **Automated Invalidation & Flush**: On startup, `rom-cache.js` queries `roms/manifest.json`. If the remote `build_id` or checksums do not match the cached version (a new image has been released), the outdated cache is automatically flushed, ensuring users never run stale builds.
4. **Manual Controls**: Users can inspect cached files, pre-cache discs for offline play, check for updates, or manually flush the cache via the **💾 Cache** button on the arcade toolbar.

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
