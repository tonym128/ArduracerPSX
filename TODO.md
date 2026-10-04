# TODO.md: Arduracer PSX Roadmap & Task Tracker

> **Tracking parallel development streams, worktree assignments, and review milestones.**  
> *Targeting rock-solid 60 FPS overhead racing on Sony PlayStation 1 hardware.*

---

## Progress Overview

- [x] **Phase 0: Workspace, Toolchain & Scaffolding**
- [x] **Phase 1: Pure `#![no_std]` Core Simulation (`arduracer-core`)**
- [x] **Phase 2: Track Pipeline & Level Importer (20 FX Tracks + Super Stages)**
- [x] **Phase 3: PSX Hardware Rendering Engine (`game/src/gpu`)**
- [x] **Phase 4: SPU & CD-DA Audio Engine**
- [x] **Phase 5: Input & DualShock Force Feedback Engine**
- [x] **Phase 6: UI, HUD, Garage & Game Loop**
- [x] **Phase 7: Ghost Car & Memory Card System**
- [x] **Phase 8: AI Opponents & Grand Prix Mode**
- [x] **Phase 9: FMV Cinematics & Disc Mastering**
- [x] **Phase 10: Multi-Perspective Review & Performance Audit**

### Verification Gates (all green)

| Gate | Command | Result |
| :--- | :--- | :--- |
| Formatting | `make fmt-check` | clean |
| Lints (`-D warnings`) | `make clippy` | clean |
| Game-logic suite | `make test` | **40 / 40** |
| Circuit playability | `make playtest` | **24 / 24** (player + 5 AI rivals, 5 laps each) |
| MIPS build + RAM budget | `make exe` | 307 KB / 2 MB static (**14.6 %**) |
| Disc mastering | `make disc` | 185 MB BIN + CUE, 7 tracks (Plattypus 15 FPS FMV + full CD-DA soundtrack) |
| Video Playback Gate | `make ci && make ci-disc` | **PASS** (15 FPS BS v2, 5 sectors/frame, 6-slot FIFO ring, DMA2 VRAM upload) |

See [`REVIEW.md`](REVIEW.md) for the TASK-1001 four-perspective audit, the full
defect log, and the documented deviations.

---

## Phase 0: Workspace, Toolchain & Scaffolding

- [x] **TASK-001**: Configure root workspace structure and build system.
  - **Worktree**: `wt-scaffold`
  - **Files**: `Makefile`, `rust-toolchain.toml`, `.cargo/config.toml`, `.gitignore`
  - **Specs**:
    - Support targets: `make test`, `make exe`, `make disc`, `make ci`, `make fmt-check`, `make clippy`.
    - PSoXide toolchain pinning matching PlayStation MIPS cross-compilation (`mipsel-sony-psx`).
    - CI gate checking RAM budget (< 2,097,152 bytes) on `arduracer.exe`.
  - **Review Perspective**: Principal Architect, Senior Systems Engineer.

- [x] **TASK-002**: Scaffold `crates/arduracer-core` and host test suite.
  - **Worktree**: `wt-scaffold`
  - **Files**: `crates/arduracer-core/Cargo.toml`, `crates/arduracer-core/src/lib.rs`, `tools/test_game_logic/Cargo.toml`, `tools/test_game_logic/src/main.rs`
  - **Specs**:
    - Pure `#![no_std]` library crate with `extern crate alloc;` only when feature enabled, default none.
    - Zero platform-specific dependencies.
    - Smoke test in `tools/test_game_logic` that verifies fast host test execution.
  - **Review Perspective**: Principal Architect, Verification/QA Lead.

- [x] **TASK-003**: Scaffold PSX target application crate.
  - **Worktree**: `wt-scaffold`
  - **Files**: `game/Cargo.toml`, `game/build.rs`, `game/src/main.rs`
  - **Specs**:
    - `#![no_std]` `#![no_main]` entrypoint linking against in-tree `psoxide`.
    - Minimal PlayStation boot routine displaying black screen and blinking GPU status.
  - **Review Perspective**: Senior Systems Engineer.

---

## Phase 1: Pure `#![no_std]` Core Simulation (`arduracer-core`)

- [x] **TASK-101**: Deterministic fixed-point math & vector module.
  - **Worktree**: `wt-physics-core`
  - **Files**: `crates/arduracer-core/src/math.rs`
  - **Specs**:
    - `Fixed` type with `Q20.12` format (or `Q16.16`), basic arithmetic, saturating adds/multiplies.
    - Binary Angular Measurement (BAMs: 0–4095 angle units representing 0–360° for GTE compatibility).
    - Lookup table / GTE-compatible trigonometric functions (`sin`, `cos`).
    - 2D fixed-point vector math (`Vec2`: dot product, magnitude, normalization, reflection).
  - **Review Perspective**: Senior Systems Engineer, QA Lead (fuzz arithmetic bounds).

- [x] **TASK-102**: Vehicle dynamics & acceleration engine.
  - **Worktree**: `wt-physics-core`
  - **Files**: `crates/arduracer-core/src/vehicle.rs`
  - **Specs**:
    - Vehicle state: `pos`, `vel`, `heading`, `steering_angle`, `engine_rpm`, `current_gear`.
    - Engine torque curve, transmission gear ratios, and aerodynamic drag ($F_d \propto v^2$).
    - Rolling resistance and engine braking on throttle lift-off.
  - **Review Perspective**: Game Designer, Principal Architect.

- [x] **TASK-103**: Advanced drift & lateral traction physics.
  - **Worktree**: `wt-physics-core`
  - **Files**: `crates/arduracer-core/src/drift.rs`
  - **Specs**:
    - Dual friction regime: Grip state vs Lateral slip/drift state.
    - Slip angle calculation: decoupling of vehicle heading from velocity vector.
    - Counter-steering recovery dynamics and drift speed penalty calculation.
    - Handbrake impulse mechanics.
  - **Review Perspective**: Game Designer (ensure high-octane arcade responsiveness).

- [x] **TASK-104**: Surface friction & hazard interaction.
  - **Worktree**: `wt-physics-core`
  - **Files**: `crates/arduracer-core/src/surface.rs`
  - **Specs**:
    - Surface types: Tarmac ($1.0\times$), Curb ($0.85\times$ grip + rumble flag), Grass/Sand ($0.35\times$ max speed + high drag), Oil Slick ($0.1\times$ grip, forced spin), Speed Boost Pad (instant forward impulse).
  - **Review Perspective**: Game Designer, QA Lead.

- [x] **TASK-105**: Checkpoint gate array & anti-cheat lap timing.
  - **Worktree**: `wt-physics-core`
  - **Files**: `crates/arduracer-core/src/timing.rs`
  - **Specs**:
    - Checkpoint line intersection math (segment-to-segment crossing detection).
    - Sequential gate verification: must pass checkpoint $N$ before $N+1$.
    - Lap completion trigger on Start/Finish line crossing.
    - Lap timing with 60 Hz tick counter (formatted as `MM:SS.ccc`).
    - Real-time delta-split calculation against target or personal best.
  - **Review Perspective**: QA Lead (test reverse crossings, skip attempts, boundary bugs).

- [x] **TASK-106**: Car tuning parameter matrix.
  - **Worktree**: `wt-physics-core`
  - **Files**: `crates/arduracer-core/src/tuning.rs`
  - **Specs**:
    - 5 tuning sliders: Top Speed, Acceleration, Handling/Grip, Drift Stability, Gearing.
    - 20-point allocation budget, 10% modulation formula matching ArduRacer FX heritage.
    - Validation and default preset initialization.
  - **Review Perspective**: Game Designer.

- [x] **TASK-107**: Replay & Ghost telemetry compression serializer.
  - **Worktree**: `wt-physics-core`
  - **Files**: `crates/arduracer-core/src/ghost.rs`
  - **Specs**:
    - Compact 4-byte frame serialization: `x_delta` (10 bits), `y_delta` (10 bits), `heading` (8 bits), `flags` (4 bits: brake, drift, boost).
    - Circular buffer storing up to 5 minutes of 30Hz/60Hz race telemetry in < 32 KB.
    - Deterministic interpolation for playback.
  - **Review Perspective**: Senior Systems Engineer (RAM footprint), QA Lead.

- [x] **TASK-108**: Memory Card `SaveData` format & 16-bit CRC checksum.
  - **Worktree**: `wt-physics-core`
  - **Files**: `crates/arduracer-core/src/save.rs`
  - **Specs**:
    - Fixed wire format fitting within 1 PSX Memory Card block (8,192 bytes).
    - Best lap times for all 24 tracks, unlocked cup trophies, 3 tuning presets.
    - Full 4-D fuzz sweep and bit-flip corruption detection test in host suite.
  - **Review Perspective**: Principal Architect, QA Lead.

---

## Phase 2: Track Pipeline & Level Importer

- [x] **TASK-201**: Binary track format definition.
  - **Worktree**: `wt-track-pipeline`
  - **Files**: `crates/arduracer-core/src/track.rs`
  - **Specs**:
    - Chunk-based 16×16 spatial partitioning for fast rendering and collision lookups.
    - Track header: dimensions, grid start coordinates, starting heading, par times (Bronze, Silver, Gold, Dev).
    - Checkpoint gate definitions (coordinates, orientation, width).
    - Waypoint graph array for AI navigation.
  - **Review Perspective**: Principal Architect, Senior Systems Engineer.

- [x] **TASK-202**: Port 20 legacy ArduRacer FX levels from CSV/TMX.
  - **Worktree**: `wt-track-pipeline`
  - **Files**: `tools/track_cook/convert_levels.py`, `crates/arduracer-core/src/levels.rs`
  - **Specs**:
    - Convert all 20 levels in `ArduRacerFx/Levels/*.csv` and `*.tmx` to `.trk` binaries / static definitions.
    - Preserve original track geometry, start positions, and checkpoint gates.
    - Retain and verify original dev par times.
  - **Review Perspective**: Game Designer.

- [x] **TASK-203**: Create 4 PSX-exclusive Grand Prix Super-Speedways.
  - **Worktree**: `wt-track-pipeline`
  - **Files**: `crates/arduracer-core/src/levels.rs`
  - **Specs**:
    - Wide roads, high-speed banking curves, flyover bridges, tunnel sections, and technical chicanes.
  - **Review Perspective**: Game Designer.

- [x] **TASK-204**: Automated track verification test suite.
  - **Worktree**: `wt-track-pipeline`
  - **Files**: `tools/test_game_logic/src/main.rs`
  - **Specs**:
    - Validates every track: start position is on road, checkpoint sequence forms an unbroken loop, AI waypoints are reachable, and no dead ends exist.
  - **Review Perspective**: QA Lead.

---

## Phase 3: PSX Hardware Rendering Engine (`game/src/gpu`)

- [x] **TASK-301**: Double-buffered 320×240 @ 60 FPS display engine.
  - **Worktree**: `wt-gpu-renderer`
  - **Files**: `game/src/gpu/display.rs`
  - **Specs**:
    - Frame 0 at `(0, 0)`, Frame 1 at `(0, 240)`.
    - VSync synchronization targeting 60 Hz NTSC (50 Hz PAL).
    - Double-buffered Ordering Table (OT) allocation and DMA Channel 2 dispatch.
  - **Review Perspective**: Senior Systems Engineer.

- [x] **TASK-302**: High-performance track tile blitter.
  - **Worktree**: `wt-gpu-renderer`
  - **Files**: `game/src/gpu/tile_blitter.rs`
  - **Specs**:
    - Viewport frustum culling: only submit visible 16×16 tiles.
    - GPU `SPRT_16` or textured quads `POLY_FT4` batching.
    - Texture page caching to minimize VRAM page switches.
  - **Review Perspective**: Senior Systems Engineer (I-Cache and DMA efficiency).

- [x] **TASK-303**: 64-direction vehicle sprite renderer.
  - **Worktree**: `wt-gpu-renderer`
  - **Files**: `game/src/gpu/car_renderer.rs`
  - **Specs**:
    - 64 rotation frames mapped to BAMs angle.
    - CLUT palette swapping for player car and rival team liveries.
    - Dynamic vehicle shadow projected onto track surface.
  - **Review Perspective**: Game Designer, Senior Systems Engineer.

- [x] **TASK-304**: Dynamic camera controller.
  - **Worktree**: `wt-gpu-renderer`
  - **Files**: `game/src/gpu/camera.rs`
  - **Specs**:
    - Smooth tracking with velocity-dependent forward look-ahead.
    - Dynamic zoom: widen field-of-view as car speed approaches maximum.
    - GTE-assisted banking roll/tilt when drifting through corners.
  - **Review Perspective**: Game Designer.

- [x] **TASK-305**: Semi-transparent particle engine.
  - **Worktree**: `wt-gpu-renderer`
  - **Files**: `game/src/gpu/particles.rs`
  - **Specs**:
    - Additive blending (`GPU_BLEND_ADD`) for sparks on wall contact, turbo flame exhaust.
    - Billowing tire smoke particles during hard drift slides.
    - Fixed pre-allocated particle arena (zero dynamic allocations).
  - **Review Perspective**: Senior Systems Engineer, Game Designer.

- [x] **TASK-306**: Persistent VRAM skidmark buffer.
  - **Worktree**: `wt-gpu-renderer`
  - **Files**: `game/src/gpu/skidmarks.rs`
  - **Specs**:
    - Real-time stamping of black rubber skidmarks into offscreen VRAM scratch region during drift.
  - **Review Perspective**: Senior Systems Engineer.

---

## Phase 4: SPU & CD-DA Audio Engine

- [x] **TASK-401**: SPU driver & sample bank loader.
  - **Worktree**: `wt-audio-spu`
  - **Files**: `game/src/audio/spu.rs`
  - **Specs**:
    - Initialize SPU hardware, configure voice channels (0–23).
    - Load packed VAG ADPCM sound effects into SPU RAM (< 200 KB).
  - **Review Perspective**: Senior Systems Engineer.

- [x] **TASK-402**: Dynamic engine RPM pitch synthesizer.
  - **Worktree**: `wt-audio-spu`
  - **Files**: `game/src/audio/engine_sound.rs`
  - **Specs**:
    - Real-time modulation of SPU voice pitch register based on car engine RPM and gear shifts.
    - Smooth transition between idle hum, acceleration howl, and redline limiter cut-off.
  - **Review Perspective**: Game Designer, Senior Systems Engineer.

- [x] **TASK-403**: Tire screech & interactive sound effects.
  - **Worktree**: `wt-audio-spu`
  - **Files**: `game/src/audio/sfx.rs`
  - **Specs**:
    - Variable pitch/volume tire squeal based on lateral slip angle.
    - Wall impact crunch with velocity-proportional volume.
    - Checkpoint tone, countdown beeps, curb vibration sound.
  - **Review Perspective**: Game Designer.

- [x] **TASK-404**: Red Book CD-DA soundtrack controller.
  - **Worktree**: `wt-audio-spu`
  - **Files**: `game/src/audio/cdda.rs`
  - **Specs**:
    - CD-ROM controller commands to play, pause, seek, and loop CD-DA audio tracks.
    - Automatic track selection per cup / menu screen.
  - **Review Perspective**: Senior Systems Engineer.

---

## Phase 5: Input & DualShock Force Feedback Engine

- [x] **TASK-501**: Controller driver (Digital & DualShock Analog).
  - **Worktree**: `wt-input-dualshock`
  - **Files**: `game/src/input/pad.rs`
  - **Specs**:
    - Auto-detect controller type (Standard Digital, DualShock Analog SCPH-1200).
    - Left stick analog steering with configurable deadzone and non-linear response curve.
    - Fallback to D-Pad with progressive steering smoothing.
  - **Review Perspective**: Game Designer.

- [x] **TASK-502**: Dual-motor force feedback rumble driver.
  - **Worktree**: `wt-input-dualshock`
  - **Files**: `game/src/input/rumble.rs`
  - **Specs**:
    - Small motor (high frequency): curb rumble strip chatter, wheel slippage warning.
    - Large motor (low frequency): barrier collisions, heavy off-road shudder.
  - **Review Perspective**: Game Designer.

---

## Phase 6: UI, HUD, Garage & Game Loop

- [x] **TASK-601**: Title screen & Main Menu.
  - **Worktree**: `wt-ui-hud`
  - **Files**: `game/src/ui/title.rs`, `game/src/ui/menu.rs`
  - **Specs**:
    - Animated logo with 90s arcade presentation.
    - Mode select: Time Trial, Grand Prix, Tuning Garage, Options, Records.
  - **Review Perspective**: Game Designer.

- [x] **TASK-602**: Car Tuning Garage UI.
  - **Worktree**: `wt-ui-hud`
  - **Files**: `game/src/ui/tuning_screen.rs`
  - **Specs**:
    - Interactive 5-slider tuning interface with visual car rotation preview.
    - Real-time stat changes (Top Speed, Accel, Grip, Drift, Gearing).
    - Save setup to Memory Card slot.
  - **Review Perspective**: Game Designer.

- [x] **TASK-603**: Track Select & Target Times display.
  - **Worktree**: `wt-ui-hud`
  - **Files**: `game/src/ui/track_select.rs`
  - **Specs**:
    - Track preview map, par times (Bronze, Silver, Gold, Dev Platinum), personal best lap time.
  - **Review Perspective**: Game Designer.

- [x] **TASK-604**: In-Game Racing HUD.
  - **Worktree**: `wt-ui-hud`
  - **Files**: `game/src/ui/hud.rs`
  - **Specs**:
    - Analog tachometer / digital speedometer with gear indicator.
    - Lap counter (`Lap 2/5`), checkpoint tracker, current lap timer (`00:23.450`).
    - Real-time delta split (`-0.12` green / `+0.45` red).
    - Dynamic minimap with track overview and position blips.
  - **Review Perspective**: Game Designer.

- [x] **TASK-605**: Race Debriefing & Trophy Podium.
  - **Worktree**: `wt-ui-hud`
  - **Files**: `game/src/ui/debrief.rs`
  - **Specs**:
    - Lap time breakdown, medal award fanfare, trophy unlock animations.
  - **Review Perspective**: Game Designer.

---

## Phase 7: Ghost Car & Memory Card System

- [x] **TASK-701**: Real-time Ghost Car rendering.
  - **Worktree**: `wt-ghost-memcard`
  - **Files**: `game/src/ghost_player.rs`
  - **Specs**:
    - Semi-transparent additive ghost vehicle rendering alongside player.
    - Smooth interpolation between 30Hz telemetry keyframes.
  - **Review Perspective**: Senior Systems Engineer, Game Designer.

- [x] **TASK-702**: PlayStation Memory Card driver.
  - **Worktree**: `wt-ghost-memcard`
  - **Files**: `game/src/memcard.rs`
  - **Specs**:
    - 1-block save file (`BAS-ARDURACER-01`).
    - 16×16 16-color 3-frame animated BIOS icon.
    - Safe load/save routines with corrupt save warnings and clean format prompts.
  - **Review Perspective**: Principal Architect, QA Lead.

---

## Phase 8: AI Opponents & Grand Prix Mode

- [x] **TASK-801**: Waypoint navigation & racing line AI.
  - **Worktree**: `wt-ai-opponents`
  - **Files**: `crates/arduracer-core/src/ai.rs`
  - **Specs**:
    - AI vehicles follow curved waypoint graphs with speed adjustments for corner curvature.
    - Dynamic obstacle avoidance and overtaking decision tree.
  - **Review Perspective**: Game Designer, QA Lead.

- [x] **TASK-802**: 5 Distinct Rival Personalities.
  - **Worktree**: `wt-ai-opponents`
  - **Files**: `crates/arduracer-core/src/ai_profiles.rs`
  - **Specs**:
    - The Speeder, The Drifter, The Tactician, The Brawler, The Rookie.
  - **Review Perspective**: Game Designer.

- [x] **TASK-803**: 4 Championship Cups & Points Standings.
  - **Worktree**: `wt-ai-opponents`
  - **Files**: `crates/arduracer-core/src/championship.rs`
  - **Specs**:
    - 5 stages per cup, F1/arcade points table (10, 6, 4, 3, 2, 1).
  - **Review Perspective**: Game Designer.

---

## Phase 9: FMV Cinematics & Disc Mastering

- [x] **TASK-901**: MDEC Full-Motion Video Attract Intro.
  - **Worktree**: `wt-disc-master`
  - **Files**: `game/src/video.rs`, `tools/fmv_cook/cook_intro_str.py` (cooks `assets/INTRO.STR` & `assets/INTRO.ADPCM`)
  - **Specs**:
    - 320×240 @ 15 fps Version-2 MDEC STR video streaming via CD-ROM double speed (150 sectors/s) and MDEC DMA0/DMA1.
    - Encoded from `AssetSource/ArduracerPSX Intro.mp4` with `psxavenc` (148 frames, 10 sectors/frame, 3,031,040 bytes).
    - Synchronized SPU ADPCM audio (16 kHz mono on Voice 5) uploaded to SPU RAM at boot.
    - Runs immediately on boot (after console Sony logo) before Title Screen / Main Menu.
    - Edge-triggered button skip (`START` / `CROSS` / `CIRCLE`) stops intro voice and jumps directly to Main Menu.
  - **Review Perspective**: Senior Systems Engineer.

- [x] **TASK-902**: Complete CUE/BIN Disc Mastering.
  - **Worktree**: `wt-disc-master`
  - **Files**: `Makefile`, `dist/arduracer.cue`, `dist/arduracer.bin`
  - **Specs**:
    - Track 1 data + Tracks 2–7 CD-DA audio tracks.
  - **Review Perspective**: Senior Systems Engineer.

- [x] **TASK-903**: Packaging & Manual Artwork.
  - **Worktree**: `wt-disc-master`
  - **Files**: `packaging/`
  - **Specs**:
    - Jewel case inserts (NTSC-U / PAL), disc surface art, instruction manual.
  - **Review Perspective**: Game Designer.

---

## Phase 11: Track, Camera & Race-Start Overhaul

- [ ] **TASK-1101**: Ten-step overhaul plan for bigger circuits with runoff,
  smooth (non-blocky) corners, full-width checkpoint gates, speed-reactive
  camera, and a race-start countdown. Planning only -- see **`OVERHAUL.md`** for
  the ordered steps, measured findings that motivate them, and open decisions.

---

## Phase 10: Multi-Perspective Review & Performance Audit

- [x] **TASK-1001**: Comprehensive Four-Perspective Audit & Sign-off.
  - **Worktree**: `wt-perf-audit`
  - **Files**: `REVIEW.md`
  - **Specs**:
    - Detailed evaluation across Principal Architect, Senior Systems Engineer, Game Designer, and QA Lead.
    - Executable size verification (< 2 MB).
    - 60.00 FPS performance lock certified in DuckStation / real hardware.
  - **Review Perspective**: All Reviewers.
  - **Outcome**: Approved with one condition — the human DuckStation / real-hardware
    pass could not be performed in a headless environment (see REVIEW.md §6.5).
    Executable verified at 424 KB static (20.7 % of 2 MB). Ten blocking defects
    found and fixed (uncompletable laps, 30× physics scale error, off-road wall,
    glued barrier collision, unracable AI, empty Super Stages, missing Level 7
    start line, heading sign-convention bug, incomplete HUD, unsaved Memory Card).

---

## Post-Audit Fix Log (Phase 10 findings)

All items below were regressions that made the game unplayable and were caught by
simulating real laps rather than by reading code. Each now has a host regression
test in `tools/test_game_logic` and/or `tools/playtest`.

- [x] **FIX-01**: Lap scoring replaced with ArduRacer FX coverage semantics
  (checkpoint bitmask + start/finish exit) in `crates/arduracer-core/src/timing.rs`.
  Previously checkpoints were demanded in raster-scan order, so no lap on 23 of
  24 circuits could ever complete.
- [x] **FIX-02**: World scale corrected in `crates/arduracer-core/src/vehicle.rs`
  (velocity is now world-units-per-tick and integrates 1:1). The previous
  `velocity × 120/4096` integrator needed ~640 ticks per 64-unit tile.
- [x] **FIX-03**: Surface model split into `max_speed_factor` / `traction` /
  `lateral_hold` in `crates/arduracer-core/src/surface.rs`, restoring GAME.md's
  0.35× off-road penalty (it was effectively 0.6 %, i.e. a wall).
- [x] **FIX-04**: `VehicleState::collide_with_track` resolves authored barriers and
  the level bounding box; glancing contact slides, hard impacts scrub and spin.
- [x] **FIX-05**: AI rewritten around `TrackDef::route_node()` with a corner-speed
  governor; rival lap times went from 3–5× the player's to within 5–20 %.
- [x] **FIX-06**: Super Stages 21–24 rebuilt as closed Catmull-Rom circuits with
  curbs, boost pads and oil slicks.
- [x] **FIX-07**: Level 7 start/finish synthesised; all circuits now meet GAME.md's
  4–12 checkpoint minimum.
- [x] **FIX-08**: `heading_towards(from, to)` helper added to remove the
  screen-space Y sign footgun that silently mis-steered every AI car.
- [x] **FIX-09**: `TrackTile` enum replaces duplicated magic-ID surface lookups;
  `hud_renderer.rs` completed (gear, nitro meter, lap timer, best lap, delta
  split, circuit-outline minimap); `tile_blitter.rs` rewritten with
  speed-dependent zoom.
- [x] **FIX-10**: Added `Start` pause menu, `Select` HUD toggle, nitro input +
  `nitro_charge` meter, and real `psx-mc` Memory Card load/save with a custom
  16×16 BIOS icon.
- [x] **FIX-11**: Par times re-derived from measured reference laps via
  `make calibrate-tracks` (see REVIEW.md §6.1 for why the FX table was not
  retained verbatim).
- [x] **FIX-12**: New `tools/playtest` playability verifier added to `make ci-host`.

---

## Post-Audit Fix Log (Memory Card pass)

Found by making the memory card path testable for the first time; see REVIEW.md
§1 F-11 to F-13. FIX-10 above claimed the card was wired up — it was not, and
the save had never executed even once.

- [x] **FIX-13**: `tools/test_memcard` host suite made to compile (it referenced a
  `SaveData.best_laps` field that does not exist, so all eleven cases had never
  run) and wired into `make test`, `make clippy`, `make fmt-check` and CI.
- [x] **FIX-14**: BIOS icon builder indexed the 128-byte frame with the pixel
  width as row stride, so every row from the eighth overflowed the buffer. Under
  `panic = "abort"` **every save crashed the game**; fixed with named geometry
  constants and a `debug_assert` tying the icon size to `FRAME_SIZE`.
- [x] **FIX-15**: `SaveData` is no longer memcpy'd to the card. Explicit 161-byte
  serialisation (`write_payload` / `read_payload`) removes the interior and
  trailing struct padding that the CRC was covering and the card was storing.
- [x] **FIX-16**: Card outcomes are surfaced instead of discarded — `Saved`,
  `Corrupt` and `WriteFailed` queue a HUD banner for 150 frames — and a payload
  that arrives short is rejected instead of read past.
