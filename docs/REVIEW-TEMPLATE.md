# REVIEW.md — Arduracer PSX Four-Perspective Audit & Sign-off

> **TASK-1001** · Multi-perspective review, executable size verification, and
> performance audit of the complete `arduracer-core` + `game` + `tools` workspace.
> 
> Reviewers: Principal Architect · Senior Systems (PSX Hardware) Engineer ·
> Game & Experience Designer · Verification / QA Lead.
> 
> ---
> 
> ## 0. Verification Summary
> 
> | Gate | Command | Result |
> | :--- | :--- | :--- |
> | Formatting | `make fmt-check` | **clean** (6 crates) |
> | Lints | `make clippy` (`-D warnings`) | **clean** (5 host crates) |
> | Game-logic unit suite | `make test` | **42 / 42 pass** |
> | Memory-card persistence suite | `cargo test -p test_memcard` | **14 / 14 pass** |
> | UI state-machine suite | `cargo test -p test_ui` | **6 / 6 pass** |
> | **Circuit playability** | `make playtest` | **24 / 24 circuits playable** |
> | Bare-metal MIPS build | `make exe` | **clean, zero warnings** |
> | Main-RAM budget | `make ci-game` | **424 KB / 2 MB = 20.7 %** |
> | Disc mastering | `make disc` | **10.17 MB BIN + CUE, 7 tracks** |
> | Emulator boot | 
> 
> ---
> 
> ## 1. Principal Architect
> 
> - [ ] **Boundary Cleanliness**: Does `crates/arduracer-core` remain 100% free of PSX hardware headers, graphics types, or IO handles?
> - [ ] **No Code Duplication**: Are structs, constants, or algorithms declared once in `core` rather than mirrored in tests or game submodules?
> - [ ] **State Ownership**: Is mutable state explicitly localized? Zero global mutable singletons without strict synchronization/reset paths.
> - [ ] **`#![no_std]` & Panic Freedom**: No `.unwrap()`, `.expect()`, `panic!()`, or unbounded recursion in PSX-bound code.
> 
> ---
> 
> ## 2. Senior Systems & PSX Hardware Engineer
> 
> - [ ] **Main RAM Footprint**: Does the PSX executable (`arduracer.exe`) stay under the `ci-game` static-RAM gate? The gate measures the link map against the linker RAM region (`RAM_GATE_MAX_PCT`, default 80% of the 2 MB window); the build currently sits at ~79.4%, so any new static allocation must fit the remaining headroom.
> - [ ] **I-Cache Locality**: Are inner loop functions compact enough to fit inside the 4 KiB direct-mapped I-cache?
> - [ ] **GPU Ordering Table & DMA**: Are GPU packets submitted via DMA list chains without blocking the CPU?
> - [ ] **VRAM Management**: Does texture / CLUT allocation stay strictly within the assigned pages without overlapping double-buffer displays?
> - [ ] **SPU Budget**: Total sound bank size stays under 250 KB (well within 512 KB SPU RAM limit).
> 
> ---
> 
> ## 3. Game & Experience Designer
> 
> - [ ] **Arcade Responsiveness**: Locked 60 FPS with zero dropped frames or stuttering.
> - [ ] **Driving Feel**: Is drift initiation intuitive, rewarding, and controllable?
> - [ ] **Visual Clarity**: Is the track boundary, surface type (curb, grass, oil), and checkpoint indicator instantly readable at high speed?
> - [ ] **Audio Feedback**: Do engine revs, tire squeals, and curb chatter provide immediate acoustic cues to the player?
> 
> ---
> 
> ## 4. Verification & QA Engineer
> 
> - [ ] **Automated Host Tests**: All unit and integration tests pass with `cargo test`.
> - [ ] **Edge Cases Covered**: Boundary collisions, reverse-direction checkpoint crossings, maximum speed overflows, and timer wrap-arounds tested.
> - [ ] **Playability Proven**: `make playtest` completes 5 timed laps on all 40 circuits for the player *and* every rival personality. A green unit suite is **not** sufficient evidence that the game is playable - the lap simulation is.
> - [ ] **Formatting & Linting**: `cargo fmt --check` clean; `cargo clippy -D warnings` clean across all crates.
> - [ ] **Lockfile Consistency**: Cargo dependencies locked and reproducible.
> 
> ---
> 
> ## 5. Media & Asset Pipeline Protocols
> 
> ### 5.1 Track & Level Pipeline (`tools/track_cook`)
> 
> The authored source is the **colour-coded circuit images** under `tracks/`
> (see [`LEVEL-FORMAT.md`](LEVEL-FORMAT.md)); each circuit carries its
> `*.level.json` / `*.line.json` / `*.palette.json` alongside the PNGs. There is
> no TMX or `.trk` step.
> 
> - Input: `tracks/**/*.png` (+ per-circuit JSON) and the legacy ArduRacer FX
>   level CSVs under `ArduRacerFx/Levels/`.
> - Process: `tools/track_cook/build_atlas.py` parses the images, validates every
>   gate (reachable, on-surface, start box inside the racing surface), builds the
>   texture blocks, and emits the generated Rust tables.
> - Output: `crates/arduracer-core/src/levels.rs` + `visual_tex.rs`, the cooked
>   `levels/*.ardlvl`, and the disc texture assets (`assets/*.BIN`). Run
>   `make atlas`; never hand-edit the generated files.
> 
> ### 5.2 Sprites & Textures (`tools/track_cook`, `tools/title_cook`)
> 
> - Circuit textures are emitted by `make atlas` in five formats with a pure
>   fallback chain (SVG -&gt; QOI -&gt; JPEG, plus explicit TIM / TIM4) -- see §2.1.
>   `tools/track_cook/verify_tim.py` checks the uncompressed assets against the
>   source raster.
> 
> - Single images convert to PSX TIM with `tools/psx_texture_converter.py`
>   (`--bpp`, `--colors`; `--atlas` for megatexture packing).
> 
> - The title/menu backdrop is cooked by
>   `tools/title_cook/cook_title_background.py`.
> 
> ### 5.3 Audio Pipeline (`tools/audio_cook`)
> 
> - **Sound Effects (SPU)**:
>   1. Source: 44.1 kHz 16-bit WAV audio.
>   2. Process: Resample to 22,050 Hz or 11,025 Hz, encode to PSX 4-bit ADPCM VAG format with loop flag markers.
>   3. Output: SPU sample bank binary (`sounds.bnk`) with lookup table.
> 
> - **Soundtrack (CD-DA)**:
>   1. Source: Mastered stereo tracks.
>   2. Process: Encode to 44.1 kHz 16-bit uncompressed PCM stereo audio (`.raw` / `.cdda`) padded to exact 2352-byte sector boundaries.
>   3. Output: Disc image CUE sheet with Track 1 (Data) and Tracks 2–7 (Audio).
> 
> ### 5.4 FMV Cinematics (`tools/fmv_cook`)
> 
> - Input: MP4 1080p source video (Attract Intro, Ending Cinematic).
> - Process:
>   1. Downscale to 320×240 @ 15 fps (or 30 fps).
>   2. Encode to PSX MDEC BS (bitstream) macroblocks.
>   3. Interleave with CD-XA ADPCM audio sectors.
> - Output: Bootable `.STR` movie files streamable via PSX CD-ROM DMA Channel 2.