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

---

## Post-Audit Fix Log (Phase 11 audit, 2026-10)

Found by a full source review of `crates/arduracer-core`, `game/src`, `tools/` and
CI, cross-checked against a working baseline. Twenty issues were raised; the
thirteen below the "Fixed" heading are merged and covered by tests. **Everything
in this section is still open** — none of it has been started. Full analysis with
reproduction commands is in [`AUDIT-2026-10.md`](AUDIT-2026-10.md).

Verification state at the time of writing — `make fmt-check`, `make clippy`,
`make test` (149 core + 42 logic + 15 memcard + 13 UI + 14 playtest),
`make playtest` (24/24), `make ci-game` (541,220 B statics = 27 % of the RAM
region) and `make ci-disc` all pass.

### Fixed and merged

- [x] **FIX-17**: `Fixed::mul` / `Fixed::div` truncated the `i64` intermediate with a
  bare `as i32`, so any product or quotient above `i32::MAX` became a large
  *negative* number. Now saturating, matching `Add`/`Sub`.
  `crates/arduracer-core/src/math.rs`.
- [x] **FIX-18**: `Vec2::length()` went through that wrapping multiply, so every
  separation of ~800 units or more reported `0.00` (`vec(1500,0)` reported
  `390.96`). It is the AI's only range measurement, and the two 30×30 circuits
  are 1920 units wide, so the AI never braked for corners on the largest tracks.
  Magnitude now accumulated in `u64`.
- [x] **FIX-19**: Par times were unreachable on all 24 circuits — bronze on
  TRACK_01 needed 1740 units in 509 ticks against a 1792-unit circuit — so
  `evaluate_medal` always returned `None` and no medal could be earned. Par is
  regenerated from the measured reference driver; `make playtest` now fails on
  drift.
- [x] **FIX-20**: A 1-tick lap satisfied the scoring guard, scoring Dev Platinum
  and persisting it. Minimum plausible lap floor added, plus field sanitisation on
  save load (`active_tuning_slot`, `max_unlocked_level`, volumes, medals,
  undriveable lap times, tuning sliders).
- [x] **FIX-21**: `checkpoint_mask` was cleared only on a *scored* lap, so gates
  touched on a failed attempt carried into the next attempt and a lap could be
  scored with no circuit driven. Cleared on every start-line crossing.
- [x] **FIX-22**: Wall contact was a death sentence — 60 ticks of spin with all
  lateral velocity preserved and reverse disabled, re-triggering on expiry (569
  of 600 ticks stuck in reproduction). Spin now applies friction, keeps steering
  authority, is cancellable by braking, and cannot re-arm while the car is still
  charging a barrier.
- [x] **FIX-23**: `compute_standings` ranked on `gate_idx * 100_000`, so a larger
  index won regardless of position and the player was displayed 6th while leading.
  Now ranks on flag → laps → distance to next gate → index.
- [x] **FIX-24**: AI beelined between gate centres and lapped 27 % under the
  physical floor of the circuit. Replaced with road-following racing line, stuck
  detection and reverse/rescue recovery.
- [x] **FIX-25**: No `Barrier` tile existed in any of the 24 grids, so
  `TrackTile::is_solid`, `SurfaceType::is_solid` and the whole interior-wall branch
  of `collide_with_track` were unreachable. Walls authored in the cooker for the
  Super Stages, with minimum-centreline-distance and flood-fill reachability
  validation.
- [x] **FIX-26**: `cargo test` on the core crate ran **0 tests** and exited 0,
  printing "tests passed" having executed nothing. 149 in-crate tests added;
  `no_std` now applied via `cfg_attr(not(test))` so the host harness works while
  `mipsel-sony-psx` keeps its freestanding guarantee.
- [x] **FIX-27**: `test_game_logic` printed every failure as the literal text
  `Any { .. }` via `{:?}` on `Box<dyn Any>`, so 42 identical content-free lines
  were all CI ever reported. Payload is now downcast.
- [x] **FIX-28**: The `game` crate was absent from the clippy job, so the largest
  and most hardware-risky code had no lint coverage; playtest par drift never
  reached `all_errors`; the RAM gate compared `.exe` file size against 2 MB, which
  can never fire and cannot see `.bss` at all. All three gates now real.

### Critical — graphics and frame pacing

- [ ] **TASK-1201**: Frame synchronisation is absent, so double-buffering provides
  no protection. The frame loop is a bare `wait_vblank(); fb.swap()`, and
  `FrameBuffer::swap()` writes its three `GP0(02h/10h)` words through raw
  `write_gp0` with no `wait_cmd_ready()` — unlike every other GP0 writer in the
  SDK. If the 256-word command FIFO is full those words are dropped and the draw
  area stays pointed at the previous buffer; nothing also prevents frame N+1 being
  submitted while VBlank flips to it, collapsing the double buffer to a 1-deep
  queue. Use the SDK's deferred swap (`begin_deferred_swap` +
  `queue_gp1_at_vblank` + `draw_done`) and switch the six full-screen clears from
  a rasterised `draw_rect_flat` (76,800 px/frame, ~90 % overdrawn) to
  `FrameBuffer::clear()` / `GP0(02h)`.
  **Files**: `game/src/main.rs`, `game/src/video.rs`.
  **Verify**: needs a hardware or emulator pass; cannot be confirmed headless.

- [ ] **TASK-1202**: The HUD minimap rebuilds a *static* image every frame,
  walking the entire tile grid with no culling and no caching — 2,216 GP0 packets
  / 11,080 GP0 words per frame on a 30×30 circuit, ~7.2 ms of the 16.67 ms frame
  (**~43 %**). The circuit does not change during a race. Pre-render once per
  track load, upload to VRAM, blit as a single `GP0(68h)` sprite; keep the moving
  rival/player blips as separate primitives drawn on top. Depends on TASK-1204.
  **File**: `game/src/gpu/hud_renderer.rs:232`.

- [ ] **TASK-1203**: The cockpit canopy quad is degenerate. `make_quad` negates
  `l_rear` internally, but the glass passes `l_rear = -4` where every other call
  site passes a positive value, which cancels the negation and puts all four
  vertices on the same row. Measured twice-area: **0** at 0°, 90°, 180° and 270°.
  The "Tinted Cockpit Canopy" feature does not exist on screen. One-character sign
  fix, and add a test asserting non-zero area at all cardinal and diagonal
  headings.
  **File**: `game/src/gpu/car_renderer.rs:179`.

- [ ] **TASK-1204**: No texture pipeline exists at all: zero texture pages, CLUTs
  or textured primitives in `game/src`, so VRAM sits 71 % idle. Prerequisite for
  TASK-1202. Respect the 1 MB budget, 256 px / 64 px page alignment and 15-bit
  BGR555 packing, and document the VRAM accounting.
  **Files**: `game/src/gpu/*`, `game/src/video.rs`.

### High — rendering correctness

- [ ] **TASK-1205**: Zero semi-transparency in the renderer — `semi_transparent` is
  hard-wired `false` in every polygon opcode. The ghost car is fully opaque solid
  blue, smoke "fades" in 3 discrete opaque greys, and skidmarks step once from one
  colour to another. Also: dithering is never enabled (`gpu::init()`'s `GP1(01h)`
  clears `GPUSTAT` bit 9), and quantising the palette through the 5-bit VRAM path
  shows `SPEEDWAY.road` and `SPEEDWAY.road2` collapsing to the **identical** word
  `0x14A6`, so on tracks 1–6 the "dark asphalt" underlay of every timing gate is
  byte-identical to the surrounding tarmac. TODO TASK-305 previously claimed
  additive blending shipped; it did not.
  **Files**: `game/src/gpu/{car_renderer,particles,skidmarks,tile_blitter}.rs`.

- [ ] **TASK-1206**: Particles are effectively invisible. They are rendered
  *before* the cars (`main.rs:183` vs `:185`/`:193`, and `:473` vs `:476`), so the
  opaque body quad paints over them; smoke is emitted at the car's exact centre
  with zero velocity so it never drifts; `max_life` is written in three places and
  read in none; the skidmark ring is 96 slots stamped with `life: 180`, so every
  mark is overwritten at ~53 % of its nominal life and the fade branch is a
  single-frame flash. There is also no depth sorting anywhere: cars are drawn in
  array order with the player unconditionally last, so overlapping cars
  interpenetrate.
  **Files**: `game/src/gpu/{particles,skidmarks}.rs`, `game/src/main.rs`.

### High — audio and input

- [ ] **TASK-1207**: Three SPU defects in `game/src/audio/`. (a) Curb rumble is
  never audible: the curb branch writes volume/pitch on `VOICE_SKID`, which is
  keyed on only in the *drift* branch, and the `else` immediately `key_off`s it.
  (b) `if hit_wall { play_crash() }` has no edge detector and `hit_wall` is true
  every frame of a sustained scrape, so a 0.25 s sample restarts 60×/s. (c) There
  is no `key_off(VOICE_ENGINE)` anywhere, so the engine drones at the last RPM
  through every menu and the pause veil. Also worth folding in: engine pitch
  reaches 3.25× on a 22.05 kHz sample, 2.4× past the SPU's Nyquist limit, and
  `INTRO_ADDR` overlaps the checkpoint chime by 2,288 bytes with no compile-time
  guard.
  **Files**: `game/src/audio/{sfx,spu,engine_audio}.rs`.

- [ ] **TASK-1208**: Force feedback is entirely inert. `actuator_bytes()` has no
  callers and the vendored `psx-pad` exposes no actuator/motor write path — the
  only 8-byte transaction is analog-enable. The DualShock motor command (`0x4D`,
  8 bytes) has to be added to the SDK before `RumbleDriver` can drive anything,
  and a stop-motors path is needed for pause/menu exit. `GAME.md` §1 and §7 list
  rumble as a shipped headline feature.
  **Files**: `game/src/input/rumble.rs`, `game/src/input/mod.rs`,
  `psoxide/sdk/crates/psx-pad/src/lib.rs`.

### Critical — state machine and data loss

- [ ] **TASK-1209**: The pause menu opens itself on frame 1 of a race. `load_track`
  never calls `pause.sync_edges()`, and `PauseMenu::new()` initialises
  `prev.start = false`, while `Start` is a confirm button on three consecutive
  screens — so holding `Start` from the title through Main Menu and Track Select
  loads the race already paused, music stopped.
  **Files**: `game/src/main.rs`, `game/src/ui/pause_input.rs`.

- [ ] **TASK-1210**: The garage silently discards tuning setups. The sliders allow
  **0–10** but `CarTuning::is_valid()` requires **1–7** and `total_points() == 20`,
  so `memcard.rs` drops the setup; `flush()` still runs but `store_tuning` never set
  `is_dirty`, so it returns early with no error banner and no message. One
  `Triangle` press also wipes the whole setup with no confirmation or undo, and
  "POINTS REMAINING" wraps at 9 while the value ranges 0–20. Use the exported
  `MIN_SLIDER`/`MAX_SLIDER` as the bounds and surface any rejection.
  **Files**: `game/src/ui/tuning_screen.rs`, `game/src/memcard.rs`.

### High — missing screens and features

- [ ] **TASK-1211**: No options screen exists. `sound_volume`, `music_volume`,
  `rumble_enabled` and `max_unlocked_level` are serialised and checksummed but have
  **zero readers** in `game/src`. `InputProfile::ModernTriggers` and `DualAnalog`
  are unreachable (`mapping.rs` is dead documentation), and `ModernTriggers` binds
  R2 to throttle *and* nitro, so it is unplayable as written. Specified in
  `GAME.md` §8.
  **Files**: `game/src/state.rs`, `game/src/ui/*`, `game/src/input/*`.

- [ ] **TASK-1212**: "RECORDS & MEDALS" is a lie — `main.rs:235` opens Track Select
  and then starts a race. `SaveData::medals_earned` is written by `update_best_lap`
  and never read anywhere. Now that par times are reachable (FIX-19) the medals
  that get awarded have nowhere to be displayed. Needs a real per-track
  best-lap/medal screen using the new `LapTimer::best_lap_ticks() -> Option<u32>`
  accessor, rendering `--` when there is none.
  **Files**: `game/src/main.rs`, new `game/src/ui/records.rs`.

- [ ] **TASK-1213**: Results and championship presentation. "TOTAL TIME" shows the
  last lap because `LapTimer` has no cumulative field; there is no cup standings
  or podium and `sorted_leaderboard()` is never called from game code, so after six
  stages the player's cup progress is discarded with no ceremony. Also: the pause
  menu has three items, no settings/controls entry, and `QUIT TO MENU` destroys a
  live Grand Prix with no confirmation.
  **Files**: `game/src/ui/{results,pause}.rs`, `game/src/main.rs`.

- [ ] **TASK-1214**: The pad is polled twice per racing frame — `main.rs:199` and
  again inside `ControllerDriver::update` — two full SIO0 transactions per frame,
  and the UI snapshot and `VehicleInput` come from different instants, so a `Start`
  tap can open the pause menu without reaching the vehicle.
  **Files**: `game/src/main.rs`, `game/src/input/pad.rs`.

### Medium — remaining smaller defects

- [ ] **TASK-1215**: `gearing` is completely inert. It appears only in the struct,
  `is_valid`, `total_points`, save (de)serialisation and the AI profiles; no
  physics, audio or HUD code reads it. Five of the 20 allocatable tuning points buy
  nothing while the Garage shows the slider. Implement it in `vehicle.rs` or remove
  it and rebalance to four sliders.
  **Files**: `crates/arduracer-core/src/{tuning,vehicle}.rs`.

- [ ] **TASK-1216**: Dead GPU state. `Camera::zoom` is computed every frame and read
  by nothing — `world_to_screen` never applies it — and the `draw_y` /
  `_draw_offset_y` parameter is threaded through 15 call sites that all discard it,
  which is a trap for any future renderer that does honour it. `REVIEW.md:146` and
  `:438` both assert the zoom works; they are wrong. Either implement both or
  delete them and correct the documents. Related: `world_to_screen` narrows
  `i32 → i16` with no clamp and culls *after* truncating.
  **Files**: `game/src/gpu/camera.rs`, `REVIEW.md`.

- [ ] **TASK-1217**: Smaller UI defects. Track Select's confirm/cancel edge baseline
  is never reset on entry, so a held `Cross` confirms on frame 1; gate count renders
  as `:`, `<`, `=` on the four circuits with 10/12/13 gates; no race-start
  countdown (the clock arms on frame 1); no controller-disconnected state, so the
  title blinks forever with no pad; memory card writes clobber the existing save
  with no slot picker or confirmation; `active_tuning_slot` is never written so only
  slot 0 is reachable; the ghost is never persisted and `GHOST_MAGIC`/`GHOST_VERSION`
  have no serializer, with the module doc still claiming 11 KB fits in an 8 KB card
  block; `VideoStorage` reserves 262.5 KiB of BSS for ~59 KiB of need (13.4 % of
  usable RAM, recorded as 128 KB in `REVIEW.md:495`).
  **Files**: `game/src/ui/*`, `game/src/memcard.rs`, `game/src/ghost_*.rs`,
  `game/src/video.rs`.

### Not code — documentation corrections

- [ ] **TASK-1218**: `REVIEW.md` and `TODO.md` make three claims the code
  contradicts: the speed-dependent camera zoom (`REVIEW.md:146` and the `[x]`
  VRAM sign-off at `:502`), curb rumble audio (`:530`, "curb chatter reuses the
  skid voice"), and the FMV decode buffer size (`:495` says 128 KB; the static is
  an actual 262.5 KiB). TODO TASK-305 marks additive blending as `[x]` when no
  `BlendMode` is used anywhere. The "Verification Gates" table at the top of
  `TODO.md` still reports the game-logic suite as 40/40; it is now 149 core tests
  plus 42/15/13/14 in the host tools, and the static footprint is 27 % rather than
  14.6 %.
  **Files**: `REVIEW.md`, `TODO.md`.
