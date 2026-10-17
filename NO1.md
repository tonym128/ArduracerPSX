# NO1.md — Master Blueprint: Making Arduracer PSX the #1 Arcade TrackMania-Style Racer Ever

> **Worktree**: `.worktrees/no1` (Branch: `no1`)  
> **Goal**: Transform **Arduracer PSX** from a solid bare-metal PS1 prototype into the **undisputed #1 arcade TrackMania-style racing game ever built**, combining *TrackMania Nations / Turbo*'s "one-more-try" flow-state perfection, stunt blocks, and split-time obsession with *Ridge Racer Type 4* / *Rollcage* / *Micro Machines V3* PS1 hardware mastery.

---

## Executive Audit Summary: Where We Stand Today

A deep line-by-line audit of `crates/arduracer-core`, `game/src`, `tools/`, `tracks/`, and `assets/` revealed both the strengths of our architecture and the exact gaps holding the game back from #1 status:

1. **`GameMode::Mania` is Currently Only a Menu Label**: Tracks `25..=30` (`MANIA CUP`) use the exact same 5-lap loop and flat 6-surface physics as Grand Prix (`crates/arduracer-core/src/levels.rs:14581`). There are no TrackMania blocks (Engine-Off Free-Wheeling, Directional Super Reactor Boosters, Jump Ramps, Ice/Dirt/Sand surfaces, or 2-layer Bridge/Tunnel crossovers), no 1-frame Instant Restart vs. Flying/Standing Checkpoint Respawn, and no Stunt/Platform/Sprint modes.
2. **Inaccurate Split Deltas & Respawn Flaws**: `LapTimer::delta_ticks()` (`crates/arduracer-core/src/timing.rs:711-728`) uses a linear fraction estimate (`best * next_checkpoint_index() / total`) that sawtooths between gates instead of recording real per-checkpoint split times. Worse, `TrackDef::respawn_point(from)` (`crates/arduracer-core/src/track.rs:620-642`) teleports to the *spatially nearest* route node (which can teleport a car *forward* across a hairpin wall) instead of the **last cleared checkpoint in driving order**.
3. **Critical Physics, AI & Input Bugs**:
   - **0.25s Nitro Recharge Bug (`crates/arduracer-core/src/vehicle.rs:594`)**: `self.nitro_charge + NITRO_RECHARGE_TICKS` adds `+6` per tick instead of `+1` every 6 ticks, refilling the entire 90-tick nitro bar in **0.25 seconds** instead of the intended 6 seconds.
   - **AI Checkpoint Bitmask Overflow (`crates/arduracer-core/src/ai.rs:205, 735, 740`)**: `AiRacer::checkpoint_mask` is `u16` while `MAX_TRACK_CHECKPOINTS` is `24` (`track.rs:46`); any track with `>16` checkpoints overflows `1 << i`.
   - **Bowtie Particle & Skidmark Quads (`game/src/gpu/particles.rs:37-43`, `skidmarks.rs:25-31`)**: Vertices are submitted in perimeter order `(TL, TR, BR, BL)` instead of Z-order `(TL, TR, BL, BR)`, turning every smoke puff and skidmark into a self-intersecting bowtie triangle wedge.
   - **Hardcoded `-Y` Smoke Direction (`game/src/gpu/effects_sim.rs:144-165`)**: `emit_smoke` ignores car heading and always spawns/drifts smoke toward world-north (`-Y`).
   - **Stationary Endless Rumble (`game/src/input/rumble.rs:51-57`)**: Stopping (`speed == 0`) on a `Curb` or `OffRoad` tile vibrates the DualShock motors forever.
   - **Unreachable Control Profiles & `R2` Conflict (`game/src/main.rs:404, 1179`, `input/mapping.rs:11-39`)**: `ModernTriggers` and `DualAnalog` exist in code but cannot be selected in any menu, and `R2` is hardcoded to toggle debug metrics (colliding with `ModernTriggers` throttle).
   - **Broken Tuning Garage Slots (`game/src/ui/tuning_screen.rs:49, 264`, `tuning_input.rs:300-304`)**: Slot indicator is hardcoded to `"1/3"` and pressing `L1`/`R1` changes the index without loading saved slot parameters (`TuningMenu::load_slots` is dead code).
4. **Massive Underutilization of PlayStation 1 Hardware**:
   - **GTE (COP2): 0% Used**: Every rotation and Mode 7 perspective divide runs in software on the R3000A CPU.
   - **GPU: Immediate FIFO Only, 0% Gouraud, 0% 3D/Sprites**: Cars are ~10 flat-shaded 2D polygons (`car_renderer.rs`), the world has zero 3D roadside props/arches/bridges, and `Tim4`/`Tim8` leave 75%/50% of their VRAM cache pages empty (`texlayout.rs:55-128`).
   - **SPU: 67% Voices Unused, >93% SPU RAM Unused, 0% Reverb, 0% Stereo Pan**: Only 8 of 24 SPU voices are used; the entire soundbank is 15.5 KB of Python-synthesized sine/LFSR waves (`wav2vag.py`); hardware SPU Reverb is off; all audio is mono (`set_volume(vol, vol)`).
   - **Memory Card: 95.3% Unused (7,804 of 8,192 Bytes Zero-Padded)**: `SaveData` uses only 388 bytes (`save.rs:83`), yet ghosts are RAM-only and lost on reboot (`ghost.rs:8-19`), and no Dev Platinum / Author ghosts ship on disc.

---

## The Ordered Action List (Work Item by Work Item)

We will execute these items **one by one** inside `.worktrees/no1`, verifying each item with `make test`, `make playtest`, `cargo fmt --check`, and `cargo clippy -D warnings`.

---

### Phase 1: Critical Bug Fixes & Core Simulation Integrity (Items `NO1-001` – `NO1-009`)

- [x] **`NO1-001` — Fix Nitro Recharge Rate & Overdrive Balance (`crates/arduracer-core/src/vehicle.rs`)**
  - **Problem**: `vehicle.rs:122-125, 594` adds `NITRO_RECHARGE_TICKS` (`+6`) *every tick* when off-nitro, refilling the 90-tick meter in 15 ticks (0.25s) instead of the documented ~6 seconds.
  - **Action**: Replace the per-tick `+6` addition with a sub-tick accumulator (`nitro_recharge_sub: u8`) that increments by `1` every `4` ticks passively, and awards **active recharge bonuses** for high-skill driving: `+1` charge per tick while drifting (`is_drifting`), airborne, or slipstreaming behind a rival.
- [x] **`NO1-002` — Fix `AiRacer::checkpoint_mask` `u16` Shift Overflow (`crates/arduracer-core/src/ai.rs`)**
  - **Problem**: `AiRacer::checkpoint_mask` (`ai.rs:205`) is `u16`, whereas `MAX_TRACK_CHECKPOINTS = 24` (`track.rs:46`, used by `Titan Circuit`). `1 << i` at `ai.rs:735, 740` overflows for `i >= 16`.
  - **Action**: Widen `AiRacer::checkpoint_mask` from `u16` to `u32` (or `u64` matching `LapTimer::checkpoint_mask`) and add a unit test running AI racers across 24-checkpoint tracks.
- [x] **`NO1-003` — Fix Particle & Skidmark Bowtie Quad Vertex Order (`game/src/gpu/particles.rs`, `skidmarks.rs`)**
  - **Problem**: `particles::render` (`particles.rs:37-43`) and `skidmarks::render` (`skidmarks.rs:25-31`) pass vertices in perimeter order `(TL, TR, BR, BL)` to `draw_quad_flat_blended`, which expects Z-order `(TL, TR, BL, BR)`, rendering corrupted bowtie triangles.
  - **Action**: Swap the last two vertices in both functions to Z-order `(TL, TR, BL, BR)` and scale particle/skid sizes by camera zoom / Mode 7 depth (`camera.m7_scale_q12`).
- [x] **`NO1-004` — Fix World-Space `-Y` Smoke Emission & Enable Rival Particles/Skidmarks (`game/src/gpu/effects_sim.rs`, `main.rs`)**
  - **Problem**: `ParticleSystem::emit_smoke` (`effects_sim.rs:144-165`) spawns smoke at `pos.y - SMOKE_DRIFT * 2` drifting in `-Y` regardless of car heading, and only `self.player` emits smoke/sparks/skidmarks (`main.rs:1568-1590`).
  - **Action**: Pass `heading` and rear-axle wheel offsets into `emit_smoke` / `emit_sparks` so smoke billows from the rear tires opposite the car's velocity vector, and wire all 5 AI rivals into `ParticleSystem` and `SkidBuffer`.
- [x] **`NO1-005` — Fix Stationary Curb/Off-Road Infinite Rumble Bug (`game/src/input/rumble.rs`)**
  - **Problem**: `RumbleDriver::tick` (`rumble.rs:51-57`) activates DualShock motors on `Curb` and `OffRoad` even when `car.speed == 0`.
  - **Action**: Gate surface rumble on `car.speed.abs() > STEER_MIN_SPEED` and modulate motor intensity proportionally to speed.
- [x] **`NO1-006` — Fix Tuning Garage Slot Display (`"1/3"`) & Dead Slot Loading (`game/src/ui/tuning_screen.rs`, `tuning_input.rs`, `main.rs`)**
  - **Problem**: `tuning_screen.rs:264` hardcodes `"1/3"`, and pressing `L1`/`R1` in `tuning_input.rs:300-304` changes `self.menu.slot` without loading or saving `save_data.tuning_slots[slot]` (`TuningMenu::load_slots` is dead code).
  - **Action**: Wire `TuningMenu::load_slots` on garage entry, synchronize slot changes (`1/3`, `2/3`, `3/3`) with `save_data.tuning_slots`, and add named preset archetypes (e.g., *Balanced*, *Grip King*, *Drift Spec*, *Top Speed*).
- [x] **`NO1-007` — Fix XA Music Playing Through Pause Menu & Distinct UI SFX (`game/src/main.rs`, `game/src/audio/mod.rs`)**
  - **Problem**: Pausing a race (`main.rs:1035-1037, 1152-1164`) pauses `cdda` but forgets to pause/resume `xa` music; `AudioSystem::tick_ui` (`audio/mod.rs:195-201`) ignores `UiSound::Confirm` and `UiSound::Back`, playing `play_ui_move()` for everything.
  - **Action**: Pause/resume `self.audio.xa` alongside `cdda`, and route `UiSound::Move`, `UiSound::Confirm`, and `UiSound::Back` to distinct SPU pitches/samples.
- [x] **`NO1-008` — Fix Orphaned Platinum Cup Circuits (`Frost Cascade`, `Lunar Gorge`, `Onyx Run`) (`tools/track_cook/generate_svg_circuit.py`)**
  - **Problem**: Circuits 21, 22, and 23 exist in `tracks/` and `CANONICAL_CIRCUIT_ORDER` (`build_atlas.py:241-263`), but their definitions are missing from `generate_svg_circuit.py:CIRCUITS`, making them non-reproducible when running `make circuits`.
  - **Action**: Restore canonical generator definitions for `Frost_Cascade`, `Lunar_Gorge`, and `Onyx_Run` in `generate_svg_circuit.py` and unify `levels::TOTAL_TRACKS` documentation across the repo.
- [x] **`NO1-009` — Auto-Detect PAL/NTSC Video Mode & Camera Bounds Clamping (`game/src/main.rs`, `game/src/gpu/camera.rs`)**
  - **Problem**: `gpu::init(VideoMode::Ntsc, ...)` (`main.rs:340`) hardcodes NTSC even when `is_pal_mode()` is true; `Camera::update` skips `clamp_to_bounds` in `mode7` (`camera.rs:329`) and `rotating` (`camera.rs:357`) modes.
  - **Action**: Pass the detected `VideoMode` into `gpu::init` and apply boundary clamping consistently across all camera modes.

---

### Phase 2: True TrackMania Gameplay, Timing, Respawn & Mania Modes (Items `NO1-010` – `NO1-017`)

- [x] **`NO1-010` — Real Per-Checkpoint Split Times & Live Split Delta HUD (`crates/arduracer-core/src/timing.rs`, `game/src/gpu/hud_renderer.rs`)**
  - **Problem**: `LapTimer::delta_ticks()` (`timing.rs:711-728`) fabricates expected split time as `best * next_checkpoint_index() / active_checkpoint_count()`, which is inaccurate and climbs every frame between gates.
  - **Action**: Record exact checkpoint crossing timestamps `current_splits: [u32; MAX_TRACK_CHECKPOINTS]`, `best_splits: [u32; MAX_TRACK_CHECKPOINTS]`, and `last_split_delta: Option<i32>` in `LapTimer`. When crossing a gate, flash a center-top TrackMania split banner (`CP 4/10  00:18.42  -00.31s` in green/blue for ahead, red for behind) with a two-tone split chime.
- [x] **`NO1-011` — Ordered & Direction-Aware Checkpoint Validation via `Route` (`crates/arduracer-core/src/timing.rs`, `route.rs`, `build_atlas.py`)**
  - **Problem**: Checkpoints in `levels.rs` are emitted as `1×1` tiles (`build_atlas.py:553-574`) and `LapTimer` allows touching checkpoints in *any* order via bitmask (`timing.rs:528-537`), never calling `Route::crossed` (`route.rs:300`).
  - **Action**: Emit real road-spanning gate widths/orientations in `build_atlas.py`, enforce sequential checkpoint order (or explicit multi-branch checkpoint groups for split routes), and validate forward crossing direction via `Route::crossed`.
- [x] **`NO1-012` — TrackMania Dual Respawn System: Last-Checkpoint Respawn vs. 1-Frame Instant Restart (`crates/arduracer-core/src/timing.rs`, `vehicle.rs`, `game/src/main.rs`)**
  - **Problem**: Pressing `L1` calls `TrackDef::respawn_point(from)` (`track.rs:620-642`), which searches for the *spatially nearest* route node and can teleport the player *forward* across a hairpin wall. There is also no single-button 1-frame Instant Track Restart.
  - **Action**:
    1. **Tap `L1` (Checkpoint Respawn)**: Teleport strictly to the **last cleared `CheckpointGate`** (or `start_gate` if no gate cleared yet on the current lap), aligned with the gate's forward tangent. Support both **Standing Respawn** (double-tap `L1`) and **TrackMania Flying Respawn** (single-tap `L1`: restores pre-crash forward speed minus a 1.0s penalty).
    2. **Hold `L1` (0.4s) or Press `Select + L1` (Instant Restart)**: Reset the entire race/time-trial to the starting grid in **1 frame** (zero CD loading, reusing the already-resident `RAM_BLOCKS` and `ACTIVE_TILES`).
- [x] **`NO1-013` — Signature TrackMania Block Types in `TrackTile` & `SurfaceType` (`crates/arduracer-core/src/track.rs`, `surface.rs`, `vehicle.rs`, `compile_circuit.py`)**
  - **Problem**: `compile_circuit.py:310-346` collapses 16 authoring codes into 6 basic surfaces.
  - **Action**: Expand `TrackTile` and `SurfaceType` to support genuine TrackMania puzzle/stunt blocks:
    - **`EngineOff` (Free-Wheeling Red Block)**: Disables engine throttle (`free_wheeling = true`) while maintaining full tarmac grip until the car crosses the next Checkpoint, BoostPad, or `EngineReset` block.
    - **`SuperBoostPad` (Reactor / Tier-2 Directional Booster)**: Grants a stacked `+30%` overdrive top speed (`SUPER_BOOST_TOP_SPEED_BONUS`) and directional impulse along the pad's orientation.
    - **`NoSteer` / `Fragile` Block**: Temporarily locks steering or triggers a spin if wall contact exceeds a low threshold until the next gate.
- [x] **`NO1-014` — Distinct Mania Game Modes: Sprint (1-Lap), Platform (Zero-Respawn), Stunt & Knockout (`crates/arduracer-core/src/championship.rs`, `levels.rs`, `game/src/ui/`)**
  - **Problem**: `GameMode::Mania` runs the exact same 5-lap race format as `GrandPrix`.
  - **Action**: Give `GameMode::Mania` authentic TrackMania sub-modes:
    - **Mania Sprint / Time Attack**: 1-lap point-to-point or flying-lap hotseat mode with instant restart and live medal target tracker on HUD (`NEXT MEDAL: GOLD 00:42.10 (-0.45s)`).
    - **Mania Platform Mode**: Medal awarded strictly by **Respawn Count** (0 respawns = Dev Platinum, ≤1 = Gold, ≤3 = Silver, Finish = Bronze) across hazard-heavy obstacle courses.
    - **Mania Knockout (Elimination)**: 6-car race where the last-place car is eliminated at the end of every lap until 1 winner remains.
- [x] **`NO1-015` — Aerodynamic Slipstreaming (Drafting) & Slingshot Overtakes (`crates/arduracer-core/src/vehicle.rs`, `main.rs`)**
  - **Problem**: `GAME.md:15` promises aerodynamic drafting/slipstreaming, but `vehicle.rs` has no slipstream check.
  - **Action**: When a car trails within `32..160` world units directly behind another car's wake cone (`±18°`) at `>50%` top speed, build a `draft_meter` (`0..60` ticks) that reduces aerodynamic drag (`DRAG_RETAIN` boosted from `4062` to `4084`), raises top speed by `+8%`, and emits subtle white wind-streak particles off the leading car's spoiler.
- [x] **`NO1-016` — Perfect Launch (Grid Rev-Matching) Mini-Game (`crates/arduracer-core/src/timing.rs`, `vehicle.rs`, `game/src/gpu/hud_renderer.rs`)**
  - **Problem**: `StartSequence` (`timing.rs:283`) locks all input until `StartPhase::Go`, so every race start is identical.
  - **Action**: Allow revving the engine (`input.throttle`) during the 3-2-1 countdown lights! Holding RPM inside the sweet spot (`6,500..7,500 RPM`, highlighted in green on the HUD tachometer) when `GO!` triggers a **Turbo Start** (`boost_ticks = 35`); holding full throttle at `8,500 RPM` redline causes **Wheelspin Burnout** (0.4s traction loss + tire smoke).
- [x] **`NO1-017` — TrackMania Medal Progress Bar & Live Pace Target on HUD (`game/src/gpu/hud_renderer.rs`)**
  - **Problem**: Players in Time Trial / Mania Trials cannot see the Bronze/Silver/Gold/Dev Platinum target times during a run.
  - **Action**: Render a compact top-right **Target Medal Badge** showing the next unachieved medal icon, target time, and live pace bar so the player always knows what time they are chasing.

---

### Phase 3: 3D Elevation, Jumps, Bridges & Multi-Surface Physics (Items `NO1-018` – `NO1-023`)

- [x] **`NO1-018` — Vertical `Z` Axis, Jump Ramps & Airtime Physics (`crates/arduracer-core/src/vehicle.rs`, `surface.rs`, `track.rs`)**
  - **Problem**: `VehicleState` (`vehicle.rs:153`) is strictly 2D `(x, y)` with zero elevation or jumping.
  - **Action**: Add `z: Fixed`, `vz: Fixed`, and `airborne_ticks: u16` to `VehicleState`. Driving over a `JumpRamp` tile imparts `vz = speed * RAMP_LAUNCH_SLOPE`. While `z > 0` (`is_airborne()`):
    - Gravity pulls `vz -= GRAVITY_PER_TICK`.
    - Steering authority is reduced to subtle mid-air yaw control (`AIR_STEER_AUTHORITY = 512`), surface drag/hazards (`OffRoad`, `OilSlick`, ground cars) beneath the car are bypassed, and the car casts an offset drop-shadow that separates proportionally to `z`.
    - Clean flat landings (`|slip_angle| < 128`) grant a TrackMania **Landing Suspension Bounce Boost** (`+500` speed impulse + spark/smoke puff); crooked landings scrub speed.
- [x] **`NO1-019` — Two-Layer Bridge / Overpass & Tunnel Crossover Physics (`crates/arduracer-core/src/track.rs`, `vehicle.rs`, `compile_circuit.py`)**
  - **Problem**: `TUNNEL` (code 13) and `BRIDGE` (code 14) collapse to flat `TrackTile::Tarmac` (`compile_circuit.py:338-339`), preventing real figure-8 crossovers (`Metro 8` had to be uncrossed).
  - **Action**: Preserve `TrackTile::Tunnel` and `TrackTile::Bridge` in `compile_circuit.py`. Track `elevation_layer: u8` (`0` = ground/tunnel, `1` = bridge deck) on `VehicleState` based on entry ramp/approach direction so cars on the bridge deck (`layer == 1`) pass cleanly over cars and barriers in the tunnel underpass (`layer == 0`), and re-author `Metro 8` + 2 new Mania tracks with real over-under figure-8 crossovers!
- [x] **`NO1-020` — True Multi-Surface Physics: Dirt, Gravel, Sand & Ice (`crates/arduracer-core/src/surface.rs`, `track.rs`, `compile_circuit.py`)**
  - **Problem**: `GRASS` (5), `GRAVEL` (6), and `SAND` (7) all compile into a single `TrackTile::OffRoad` with `0.35×` max speed (`surface.rs:69`), making dirt/gravel racing impossible, and there is no `Ice` surface even on `Frostpeak Summit` and `Frost Cascade`.
  - **Action**: Split `OffRoad` into distinct physics surfaces:
    - **`Dirt` / `Gravel` (TrackMania Dirt)**: `max_speed_factor = 0.88`, `grip_factor = 0.68`, `lateral_hold = 0.80` — fast, slidey rally surface where smooth counter-steer drifting is faster than tarmac grip!
    - **`Grass` / `Sand` (Runoff)**: Keeps off-road penalty (`0.55×` for Grass, `0.40×` for deep Sand) with coloured rooster-tail particles (green grass blades, tan sand spray).
    - **`Ice` (TrackMania Ice / Bobsleigh)**: `max_speed_factor = 1.02`, `grip_factor = 0.30`, `lateral_hold = 0.96` — near-zero tire scrub when sliding sideways, requiring early rotation and throttle commitment on `Frostpeak Summit` and `Frost Cascade`.
- [x] **`NO1-021` — Wall-Ride & Barrier Grazing Physics (`crates/arduracer-core/src/vehicle.rs`)**
  - **Problem**: Any barrier contact above `HARD_IMPACT_THRESHOLD = 900` (`vehicle.rs:28, 1003`) triggers a punishing 60-tick `SpinOut`, even on shallow glancing touches.
  - **Action**: Scale impact severity by `sin(incidence_angle)` (`into_wall / speed`). Shallow glancing contacts (`< 25°` incidence) scrape along the wall with metallic sparks, audio grind, and mild speed friction (enabling TrackMania high-speed wall-riding!) while only head-on/steep impacts (`> 35°`) trigger a spin-out.
- [x] **`NO1-022` — Mass-Weighted Car-to-Car Contact & Rival Bump Personalities (`crates/arduracer-core/src/contact.rs`, `tuning.rs`)**
  - **Problem**: `contact::collide_cars` (`contact.rs:21, 152`) assumes equal mass (`50/50` separation and impulse) for every vehicle.
  - **Action**: Derive vehicle mass from `CarTuning` (`top_speed + drift_stability - handling`), so heavy brawlers like `TITAN` displace lighter agile cars more strongly in door-to-door contact.
- [x] **`NO1-023` — AI Spline Racing-Line Upgrade, Surface Awareness & Jump/Bridge Navigation (`crates/arduracer-core/src/ai.rs`)**
  - **Problem**: `AiRacer::racing_line_point` (`ai.rs:342-442`) uses a greedy 10-tile Manhattan walk that can cut across hairpins when a gate index wraps or on complex crossovers.
  - **Action**: Hybridize `AiRacer` lookahead with `Route` spline arc progression (`route.rs`), teach AI personalities to seek `Dirt`/`SuperBoostPad`/`JumpRamp` shortcuts, and respect `EngineOff` momentum lines.

---

### Phase 4: Ghosts, Dev Platinum Records, Replays & Memory Card Compression (Items `NO1-024` – `NO1-027`)

- [x] **`NO1-024` — Delta-Compressed Ghost Serializer in the 7,804 Unused Memory Card Bytes (`crates/arduracer-core/src/ghost.rs`, `save.rs`)**
  - **Problem**: `SaveData` uses only `388` of `8,192` bytes in its 1-block Memory Card save (`save.rs:83`), leaving **7,804 bytes zeroed**, while `ghost.rs` stores uncompressed 6-byte frames (`10,800` bytes) in RAM only and discards them on power-off.
  - **Action**: Implement a deterministic delta-encoded ghost stream (`GhostCompressed`: 15 Hz sampling with `(dx_q2: i8, dy_q2: i8, dheading_flags: u8)` = 3 bytes/sample, ~1,350 bytes for a 30s lap) and store the player's **Personal Best Ghost** (up to 5 slots or the most recent/favorite PB laps) inside the unused `7,400` bytes of the existing single 8 KiB Memory Card block (protected by CRC-16).
- [x] **`NO1-025` — Pre-Baked Dev Platinum (Author) & Gold Ghosts on Disc (`tools/playtest/`, `assets/`, `crates/arduracer-core/src/ghost.rs`)**
  - **Problem**: In Time Trial / Mania Trials, there is no ghost until the player finishes a lap, and no Author/Dev Platinum ghost to study for optimal lines.
  - **Action**: Extend `tools/playtest --calibrate` to record the reference driver's optimal Dev Platinum and Gold laps across all 31 circuits, pack them into `assets/GHOSTS.BIN` (~150 KB total on CD), and stream the circuit's **Dev Platinum Ghost** + **Personal Best Ghost** into RAM on track load so players can race **two simultaneous translucent ghosts** (Cyan = Personal Best, Gold = Dev Author)!
- [x] **`NO1-026` — Ghost Checkpoint Split Comparison & Distance Gap Indicator (`crates/arduracer-core/src/ghost.rs`, `game/src/gpu/hud_renderer.rs`)**
  - **Problem**: `GhostPlayer` only interpolates `(x, y, heading)` and provides no telemetry comparison.
  - **Action**: Compute live distance/time gap to the active Ghost (`GHOST +0.18s`) and display a floating distance tag or split delta comparison at every checkpoint.
- [x] **`NO1-027` — Post-Race Action Replay Theater with Broadcast TV Cameras (`game/src/main.rs`, `gpu/camera.rs`, `ui/results.rs`)**
  - **Problem**: After finishing a lap or race (`ResultsScreen`), players cannot watch a replay of their run.
  - **Action**: Add a `"WATCH REPLAY (TRIANGLE)"` option on `ResultsScreen` that replays the recorded `GhostRecorder` buffer using dynamic **Trackside Broadcast TV Cameras** (auto-switching to the nearest checkpoint camera with zoom-tracking as the car flies past), Mode 7 chase cam, or Helicopter cam!

---

### Phase 5: PSX GTE/GPU Hardware Engine & 3D/2.5D Visual Overhaul (Items `NO1-028` – `NO1-036`)

- [x] **`NO1-028` — Hardware GTE (COP2) Acceleration for Mode 7, 3D Meshes & Camera Transforms (`game/src/gpu/camera.rs`, `tracktex.rs`, `car_geometry.rs`)**
  - **Problem**: The PS1's Geometry Transformation Engine (COP2) is **0% used**; all rotations and `1/z` perspective divides run in software on the CPU.
  - **Action**: Load camera rotation/translation matrices into GTE registers and use hardware `RTPS`/`RTPT` (15–23 cycles for 3 vertices with perspective divide!), `NCLIP` backface culling, and `DPCS` depth-cueing fog for Mode 7 tile vertices, 3D cars, and roadside props.
- [x] **`NO1-029` — GPU Ordering Table (OT) & DMA Channel 2 Linked-List Rendering (`game/src/gpu/mod.rs`, `main.rs`)**
  - **Problem**: Every polygon is pushed one word at a time by the CPU to the GP0 FIFO, and depth sorting uses CPU insertion sorts (`main.rs:603`, `tracktex.rs:2678`).
  - **Action**: Implement a double-buffered **512-bucket Z-Ordering Table (OT)** in static RAM submitted via **DMA Channel 2 linked-list mode (`D2_CHCR = 0x01000401`)**, sorting ground tiles, skidmarks, shadows, 3D roadside props, bridges, cars, and particles in $O(1)$ per primitive while the CPU runs physics/audio in parallel!
- [x] **`NO1-030` — Upgrade Cars to Gouraud-Shaded 3D Low-Poly Meshes (Mode 7) & Rich Multi-Part Top-Down Geometry (`game/src/gpu/car_renderer.rs`)**
  - **Problem**: Cars are ~10 flat-shaded 2D polygons that look completely flat in Mode 7 chase view (`car_renderer.rs:60-200`), front wheels don't steer (`line 101`), twin stripes only draw one stripe (`line 125`), and brake lights never illuminate (`line 171`).
  - **Action**:
    1. Build compact **GTE-transformed, Gouraud-shaded 3D low-poly car meshes** (~28–36 triangles per car: sculpted wedge nose, raised cockpit canopy, sidepods, elevated rear wing with endplates, 4 wheel arches) for Mode 7 chase view and Replay TV view, plus enhanced top-down geometry for Overhead view.
    2. Animate **front-wheel steering angles** (`rot + steer_visual`), render true **twin racing stripes**, illuminate **bright red brake lights + additive glow** when braking, tilt the car body with **pitch (acceleration/braking/jumps) and roll (cornering G-force)**, and render distinct silhouettes per tuning archetype!
- [x] **`NO1-031` — 3D Roadside Props, TrackMania Checkpoint Arches, Start Gantry & Bridges (`game/src/gpu/`, `crates/arduracer-core/src/track.rs`)**
  - **Problem**: The world outside the ground plane is completely empty (`0` 3D props, `0` billboards; start lights are 4 flat 2D HUD boxes at `main.rs:1918`).
  - **Action**: Add a lightweight GTE-projected **3D Prop & Structure Renderer** placed along circuit edges and gates:
    - **3D Start/Finish Gantry Truss** with overhead 3-2-1-GO light pods and animated checkered banner.
    - **TrackMania Holographic Checkpoint Arches** spanning every `CheckpointGate` (turning from cyan to glowing emerald green as you blast through them!).
    - **Biome Roadside Props**: Stadium floodlight towers & grandstands (`Mania Cup`), palm trees & coastal barriers (`Bronze/City`), pine trees & rock spires (`Silver/Gold`), snowbanks & neon cyber-pylons (`Platinum/Diamond`), and 3D tunnel portals / bridge guardrails.
- [x] **`NO1-032` — Wire Biome Sky Palettes, Horizon Parallax Mountains & Corner-Banking Camera Roll (`game/src/gpu/palette.rs`, `tracktex.rs`, `camera.rs`)**
  - **Problem**: `PALETTES` in `gpu/palette.rs:81-130` is dead code; `draw_m7_backdrop` (`tracktex.rs:2817`) draws the same 5 grey-blue flat bands on every track; Mode 7 has no horizon mountain/city silhouette and no camera roll when drifting.
  - **Action**:
    1. Wire `gpu/palette.rs` + cup biome themes to render **Gouraud-shaded sky gradients** (Sunset Orange, Neon Cyber Purple, Alpine Dusk, Stadium Floodlight Night) and a **scrolling parallax horizon silhouette** (mountains, city skyline, stadium roof) that shifts horizontally with `camera.follow_heading`.
    2. Add subtle **Corner-Banking Camera Roll (`±4°`)** and **FOV Speed Warp** during high-speed drifts and Super Boosts!
- [x] **`NO1-033` — Dynamic Night / Tunnel Lighting & Additive Headlight Cones (`game/src/gpu/car_renderer.rs`, `tracktex.rs`)**
  - **Problem**: `GAME.md §5.2` promises translucent headlight cones for night and tunnel stages, but cars only draw 2px yellow lines (`car_renderer.rs:176`).
  - **Action**: On Night/Cyber circuits and inside `TrackTile::Tunnel` cells, dim ambient ground vertex shading and project **Gouraud-faded additive (`BlendMode::Add`) dual headlight cones** ahead of all 6 cars, plus glowing neon curb strips and taillight trails!
- [x] **`NO1-034` — Expand `Tim4` (256-Slot) and `Tim8` (128-Slot) VRAM Tile Cache (`game/src/gpu/texlayout.rs`, `tracktex.rs`)**
  - **Problem**: `VRAM_SLOT_COUNT` (`texlayout.rs:55`) is hardcoded to `64` (sized for 15bpp), leaving **75% of VRAM empty in `Tim4`** and **50% empty in `Tim8`**.
  - **Action**: Dynamically pack `256` tile slots in `Tim4` (4 tiles per 64-word TPage slot) and `128` tile slots in `Tim8` (2 tiles per 64-word TPage slot), holding an entire 1024×1024 circuit (`256` tiles) resident in VRAM with **zero mid-race CD tile misses**!
- [x] **`NO1-035` — VRAM Bitmap Font Sheet & Smooth Sub-Tile Minimap Blips (`game/src/ui/font.rs`, `gpu/hud_renderer.rs`, `gpu/texlayout.rs`)**
  - **Problem**: `font::draw_char` (`font.rs:79-139`) submits dozens of `draw_rect_flat` GP0 packets per character, and `render_hud` (`hud_renderer.rs:344`) quantizes minimap car dots to coarse integer tiles instead of calling `texlayout::minimap_world_pos`.
  - **Action**: Upload `FONT_5X7` (plus lowercase and arcade icons/medals) once at boot as a tiny 4bpp sprite sheet in VRAM so every glyph renders in a **single `GP0(74h)` 8×8 sprite command** (cutting UI GP0 command volume by ~85%), and wire `texlayout::minimap_world_pos` for smooth sub-pixel minimap dots with directional player arrows.
- [x] **`NO1-036` — Arcade Screen Transitions, Speed Lines & Loading Banner (`game/src/main.rs`, `gpu/hud_renderer.rs`)**
  - **Problem**: Screen changes snap instantly (`main.rs:496`) and CD track loads freeze the screen without a loading overlay.
  - **Action**: Add fast 8-frame Gouraud shutter/fade transitions between menus, display a styled `"LOADING CIRCUIT..."` track preview banner before synchronous CD loads, and render radial additive **Speed Lines** at the screen edges during Nitro / Super Boosts.

---

### Phase 6: SPU Audio, Multi-Layer Engine Synthesis, Spatial Mixing & Announcer (Items `NO1-037` – `NO1-041`)

- [x] **`NO1-037` — Rich Soundbank Upgrade & Multi-Voice SPU Allocation (`tools/audio_cook/wav2vag.py`, `game/src/audio/spu.rs`, `soundbank.rs`)**
  - **Problem**: Only 8 of 24 SPU voices and 15.5 KB of 512 KB SPU RAM are used; all 6 SFX are basic sine/LFSR buzzes (`wav2vag.py:143-240`).
  - **Action**: Expand the SPU soundbank (up to ~180 KB of the 512 KB SPU RAM, still well within the 250 KB budget) and allocate all **24 SPU hardware voices**:
    - **Voices 0–2 (Player Multi-Layer Engine)**: Crossfade **Idle Thrum (`V0`)**, **Mid-Range Induction Growl (`V1`)**, and **High-RPM Redline Wail (`V2`)** with a **Turbo Blow-Off / Gear-Shift Pop (`V3`)** on automatic upshifts!
    - **Voices 4–8 (5 AI Rival Engines)**: Give each of the 5 AI rivals their own SPU voice playing distance-attenuated, **Doppler pitch-shifted, stereo-panned** engine loops as they pass the player!
    - **Voices 9–15 (Surface & Physics Loops)**: Dedicated samples for **Tire Screech (`V9`)**, **Sawtooth Kerb Brap (`V10`)**, **Gravel/Dirt Crunch (`V11`)**, **Sand/Grass Brush (`V12`)**, **Ice Scrape (`V13`)**, **Wall Scrape Sparks (`V14`)**, and **Slipstream Wind Whoosh (`V15`)**.
    - **Voices 16–23 (One-Shot Race Events, UI & Announcer)**: **3-2-1 Countdown Beeps + GO Horn (`V16`)**, **Checkpoint Split Chime (`V17`)**, **Reactor Boost Ignition (`V18`)**, **Jump Launch / Landing Thud (`V19`)**, **Heavy Crash Crunch (`V20`)**, **UI Move / Confirm / Back (`V21`)**, **Medal Fanfare (`V22`)**, and **Arcade Announcer Voice Clips (`V23`: *"THREE! TWO! ONE! GO!"*, *"CHECKPOINT!"*, *"FINAL LAP!"*, *"NEW RECORD!"*, *"PLATINUM MEDAL!"*)**.
- [x] **`NO1-038` — Hardware SPU Reverb for Tunnels, Canyons & Stadiums (`game/src/audio/spu.rs`, `main.rs`)**
  - **Problem**: Hardware SPU Reverb is completely unconfigured (`spu.rs`).
  - **Action**: Configure the SPU Reverb work area at top of SPU RAM (`0x7C000..0x7FFFF`, Studio/Hall/Space Echo preset), enable `EON` reverb bits on engine, tire, and crash voices, and dynamically boost reverb wet depth (`vLOUT`/`vROUT`) when the player drives inside a `Tunnel` or canyon stage!
- [x] **`NO1-039` — Real-Time Stereo Panning & Doppler Pitch Shift (`game/src/audio/sfx.rs`, `engine_audio.rs`)**
  - **Problem**: Every `set_volume(vol, vol)` call in `game/src/audio/` is hardcoded mono center.
  - **Action**: Pan collisions, rival engines, off-center kerb rumbles, and checkpoint chimes across Left/Right channels based on screen-space X offset from the camera center (`pan = (screen_x - 160) / 160`).
- [x] **`NO1-040` — Master / SFX / Music Volume Sliders & Pause/Results Audio Ducking (`game/src/ui/settings.rs`, `audio/mod.rs`)**
  - **Problem**: `SaveData` already stores `sound_volume: u8` and `music_volume: u8` (`0..=8`, `save.rs:163-164`), **but `SettingsScreen` (`ui/settings.rs`) doesn't expose them** and `spu::init` hardcodes `Volume::MAX`!
  - **Action**: Add `SFX VOLUME` and `MUSIC VOLUME` sliders in `SettingsScreen` & `PauseMenu`, scale SPU main volume and CD/XA mixer registers (`SPU_VOL_CD`), and smoothly duck music volume by `-6 dB` when paused or on the results screen.
- [x] **`NO1-041` — In-Race Music Track Selector & Dynamic Final-Lap Tempo/Intensity (`game/src/audio/cdda.rs`, `xa.rs`, `ui/pause.rs`)**
  - **Problem**: Music track is fixed per cup with no way to skip/select tracks in-game.
  - **Action**: Add a `BGP MUSIC TRACK` selector in `PauseMenu` / `SettingsScreen` (with on-screen Track Title toast banner `"♪ NOW PLAYING: ASPHALT OVERDRIVE"` at race start).

---

### Phase 7: Controls, DualShock Haptics, neGcon & 2-Player Split-Screen (Items `NO1-042` – `NO1-045`)

- [ ] **`NO1-042` — Expose `InputProfile` (`ClassicArcade`, `ModernTriggers`, `DualAnalog`) & Fix `R2` Conflict (`game/src/ui/settings.rs`, `pause.rs`, `main.rs`, `crates/arduracer-core/src/save.rs`)**
  - **Problem**: `ModernTriggers` and `DualAnalog` in `input/mapping.rs:11-39` are completely unreachable because `main.rs:404` hardcodes `ClassicArcade`, and `R2` is hardcoded to cycle debug metrics (`main.rs:1179`).
  - **Action**:
    1. Move the debug profiler toggle from `R2` to **`Select + R2`** (or `L3`) so `R2` is free for `ModernTriggers` throttle.
    2. Add `CONTROLS: CLASSIC / TRIGGERS / DUAL STICK` to `SettingsScreen` and `PauseMenu`, and persist the chosen `InputProfile` in `SaveData`.
- [ ] **`NO1-043` — Left Analog Stick Menu Navigation & Hot-Plug Analog Re-Arm (`game/src/input/mod.rs`, `ui/*`)**
  - **Problem**: Menus only check digital D-Pad buttons (`PadButtons::UP/DOWN/LEFT/RIGHT`), ignoring the DualShock left stick, and `enable_analog_port1()` is only called once at boot.
  - **Action**: Synthesize edge-triggered menu navigation events when the left stick crosses `±64` with auto-repeat hysteresis, and re-enable DualShock analog mode if a controller hot-plugs.
- [ ] **`NO1-044` — Namco neGcon (`0x23`) & Jogcon (`0xE3`) Controller Support (`psoxide/sdk/crates/psx-pad`, `game/src/input/pad.rs`)**
  - **Problem**: Only standard Digital (`0x41`) and DualShock (`0x73`/`0x53`) pads are decoded.
  - **Action**: Decode controller ID `0x23` (**Namco neGcon**: 8-bit twist axis for 1:1 precision steering + analog `I` throttle and analog `II`/`L` progressive braking) and `0xE3` (**Jogcon** steering dial)—the gold standard of PS1 arcade racers (*Ridge Racer Type 4*, *WipEout 3*)!
- [ ] **`NO1-045` — 2-Player Head-to-Head Split-Screen Mode (`game/src/main.rs`, `gpu/camera.rs`, `input/mod.rs`, `ui/menu.rs`)**
  - **Problem**: Documented in `GAME.md §4.3` (`lines 135-139`), but completely unimplemented in `game/src/`.
  - **Action**: Poll Controller Port 2 (`psx_pad::poll_port2()`), add `"2P SPLIT SCREEN"` to `MainMenu`, and render dual horizontal viewports (`320×118` Top P1 + `320×118` Bottom P2 via hardware `GP0(E3h)`/`GP0(E4h)` drawing area clipping and `GP0(E5h)` drawing offset) so two players can battle head-to-head on one PlayStation!

---

### Phase 8: TrackMania Track Builder, Custom Tracks, FMV Cinematics & Web Arcade (Items `NO1-046` – `NO1-050`)

- [ ] **`NO1-046` — In-Game TrackMania Track Builder & Custom Track Memory Card Slot (`crates/arduracer-core/src/track_builder.rs`, `game/src/ui/track_editor.rs`, `save.rs`)**
  - **Problem**: No track editor exists in-game or in `tools/` (`tools/level_editor/` referenced in `build_atlas.py:194` does not exist). What makes *TrackMania* eternal is building, testing, and validating your own tracks in seconds!
  - **Action**:
    1. Build an **In-Game Grid Track Builder (`TRACK EDITOR` in Main Menu)**: Place and rotate modular snapped TrackMania blocks (Start Grid, Straight, 90° Turn, Hairpin, Chicane, Dirt Section, Ice Bobsleigh, Jump Ramp, Bridge Overpass, Free-Wheel Red Block, Reactor Boost, Checkpoint Gate) on an 80×80 grid with instant `"TEST DRIVE & VALIDATE"` (setting your own Author Platinum time!).
    2. Serialize compact custom track definitions (~256 bytes of block commands or RLE nibbles) directly into the Memory Card block so players can save and share up to **4 Custom Tracks** on a single 1-block save!
- [ ] **`NO1-047` — Interactive Web Track Studio & Shareable Track Codes (`web/`, `tools/track_cook/`)**
  - **Problem**: `web/index.html:375-399` is a static wrapper with an outdated 24-track list and no track authoring tools.
  - **Action**: Update `web/index.html` to reflect all 31 circuits + 3 cities, and add an interactive **Browser TrackMania Canvas Builder** tab that exports shareable URL codes, `.svg`/`.data.png` circuits, or injects custom tracks straight into the WebAssembly emulator's Virtual Memory Card!
- [ ] **`NO1-048` — Upgrade MDEC Video to 24bpp True-Color & Locked `.STR` Audio Sync (`game/src/video.rs`, `tools/fmv_cook/cook_intro_str.py`)**
  - **Problem**: `video.rs:529` decodes MDEC in 15bpp (`DECODE_15BPP`) with visible 5-bit banding and runs `INTRO.ADPCM` open-loop on SPU Voice 5 (`main.rs:344-358`).
  - **Action**: Upgrade `video.rs` to 24-bit MDEC true-color output (`DECODE_24BPP` + `GP1(08h)` 24bpp display mode) and pace frame presentation against the hardware audio clock for drift-free lip/beat sync.
- [ ] **`NO1-049` — Title Screen Attract Mode Loop (FMV + Live AI Autopilot Demo Races) (`game/src/ui/title.rs`, `autopilot.rs`, `main.rs`)**
  - **Problem**: Leaving `TitleScreen` idle never replays `INTRO.STR` or demonstrates gameplay, even though `autopilot::generate_autopilot_input` (`game/src/autopilot.rs:35-111`) is already implemented!
  - **Action**: After 20 seconds of inactivity on `TitleScreen`, enter **Attract Mode**: alternate between replaying `INTRO.STR` and running a 30-second live **AI Autopilot Demo Race** on a showcase circuit with dynamic Broadcast TV cameras and an `"ARDURACER PSX — PRESS START"` arcade overlay!
- [ ] **`NO1-050` — Championship Podium / Victory Ceremony & Outro `.STR` FMV (`game/src/ui/victory.rs`, `tools/fmv_cook/`)**
  - **Problem**: Winning a Grand Prix or Mania Cup (`VictoryScreen`, `ui/victory.rs`) shows a flat 2D text screen with 16 falling 3×3 rectangles.
  - **Action**: Render a **3D GTE Rotating Trophy & Winning Car Showcase Podium** with gold/silver/bronze podium steps, particle fireworks, cup summary telemetry, and an optional mastered `VICTORY.STR` celebration sequence!

---

## Execution Protocol in `.worktrees/no1`

For every single item `NO1-XXX` we tackle:
1. Implement the change cleanly following `AGENT.md` rules (`#![no_std]`, zero panics/unwraps in `game/src/`, strict separation between `crates/arduracer-core` and `game/src/`).
2. Add or update unit/integration tests in `crates/arduracer-core` and `tools/test_*`.
3. Run the verification gates inside `.worktrees/no1`:
   - `make test`
   - `make playtest`
   - `cargo fmt --all -- --check`
   - `cargo clippy --all-targets -- -D warnings`
   - `cd game && cargo build --release` (verifying static RAM / `.bss` budget)
4. Mark `[x]` on the completed item in `.worktrees/no1/NO1.md` and commit to the `no1` branch.
