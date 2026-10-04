# Arduracer PSX 🏎️💨

> **The High-Octane 60 FPS Overhead Arcade Racer for the Sony PlayStation 1 (PSX)**  
> *Spiritual successor to ArduRacer FX (Arduboy), engineered in bare-metal `#![no_std]` Rust using the PSoXide SDK.*

---

## 1. Executive Summary & Vision

**Arduracer PSX** takes the pinpoint time-trial precision, car tuning, and track-mastery gameplay of the acclaimed Arduboy title **ArduRacer FX** and catapults it onto 32-bit Sony PlayStation hardware.

Where the Arduboy original ran on an 8-bit ATmega32U4 with a 128×64 monochrome OLED screen, **Arduracer PSX** unleashes the full power of the PS1:
- **Locked 60 FPS Racing**: Razor-sharp, butter-smooth 60 Hz NTSC (50 Hz PAL) physics and rendering for flawless arcade responsiveness.
- **2D/2.5D Overhead Neo-Arcade Presentation**: Rich multi-layer parallax scrolling, crisp 32-bit color palettes, dynamic trackside objects, hardware semi-transparency for smoke and sparks, dynamic tire skidmark buffers in VRAM, and GTE-driven camera tilt and zoom.
- **Pure Arcade Driving Dynamics**: High-speed cornering, weight transfer, drift slides, surface grip mechanics (tarmac, curbs, dirt, grass, oil slicks), and aerodynamic drafting/slipstreaming.
- **Thumping Red Book CD-DA Soundtrack**: High-energy 90s Eurobeat, synthwave, and big-beat techno streamed straight off the CD-ROM disc, complemented by hardware SPU engine synthesis, tire screech, and environmental reverb.
- **Deep Replayability**:
  - **Time Trial Mode**: 20 classic ArduRacer FX circuits completely remastered + 4 brand-new PSX Super Stages. Beat Bronze, Silver, Gold, and Dev Platinum times.
  - **Deterministic Ghost Car**: Race against your own personal best ghost or developer record ghosts in real-time.
  - **Grand Prix Cup Mode**: Compete against a full 6-car grid of aggressive, personality-driven AI racers across 4 championship cups.
  - **Car Tuning & Garage**: Comprehensive performance tuning (Top Speed, Acceleration, Downforce, Drift Factor, Gearing) and custom liveries.
  - **Recovery**: if the car is wedged, spun, or facing a wall it cannot drive out
  of, the recovery button drops it back on the nearest route node facing down the
  racing line, clearing all momentum, drift and boost state. Bound to `L1` in
  every layout.

- **DualShock Force Feedback**: Feel every curb rattle, drift threshold, and engine redline through dual-motor analog vibration.
  - **PlayStation Memory Card Support**: 1-block save file with a custom 16×16 animated BIOS icon saving best laps, trophy cabinets, ghost telemetry, and custom tuning profiles.

---

## 2. Learnings from Plattypus & PSOxide

The architecture of Arduracer PSX is built on the battle-tested lessons learned from developing **Plattypus** and the **PSoXide** SDK:

### 2.1 Pure `#![no_std]` Core Separation (`arduracer-core`)
- **Lesson**: Testing game logic inside an emulator is slow and blind; testing a *mirror* of the code in the host harness leads to silent drift and false green test runs.
- **Solution**: All vehicle physics, collision detection, track definitions, waypoint graphs, lap timers, AI steering decisions, and save data serialization live in `crates/arduracer-core` with `#![no_std]` and zero hardware dependencies.
- **Host Testing**: The host test harness (`tools/test_game_logic`) imports `arduracer-core` directly. 100% of physics ticks, replay serialization, and rank evaluations are verified natively on the host workstation with `cargo test` in milliseconds.

### 2.2 Strict Hardware & Memory Budget Discipline
- **Main RAM (2 MB total, ~2,001,152 bytes link region)**:
  - Code & Static Data (`.text`, `.rodata`): Kept under 450 KB.
  - Game State & Entities (`.bss`): Pre-allocated fixed arenas. Zero dynamic heap allocation (`alloc` is forbidden in the game loop; no fragmentation).
  - Replay & Ghost Telemetry: Compressed circular buffer (position, heading, speed packed into 4-byte frames at 30Hz/60Hz, under 32 KB per race).
- **Instruction Cache (I-Cache: 4 KiB)**:
  - The hot inner physics and GPU packet generation loops must fit tightly within the 4 KiB direct-mapped I-cache. Monolithic functions and deep virtual dispatch are eliminated in favor of compact, inlineable functions.
- **VRAM Partitioning (1024×512 16-bit)**:
  - Double-buffered display: Frame 0 at `(0, 0) - (320, 240)`, Frame 1 at `(0, 240) - (320, 480)`.
  - Tile Atlas & Backgrounds: Page-aligned 16-color (4-bit CLUT) and 256-color (8-bit CLUT) texture pages.
  - Car Sprites: 64 rotation angles packed with dedicated CLUT palettes for player and rival colors.
  - Dynamic Skidmark & Particle Scratchpad: Offscreen VRAM scratch region for persistent tire marks blitted to the track.
- **SPU Audio RAM (512 KB)**:
  - Dedicated VAG sound sample bank: Engine loops (idle, mid, redline), tire squeal (variable pitch loop), turbo blow-off, curb rumble, metal impact crunch, checkpoint chime, countdown beeps.
  - Maximum sample footprint < 200 KB, leaving generous headroom for SPU reverb work area.

### 2.3 GTE & GPU Pipeline Optimization
- **GTE (Geometry Transformation Engine)**: Utilized for fast fixed-point matrix transforms, screen coordinate projections, track rotation, and dynamic camera zoom/tilt.
- **GPU DMA Packet Batching**: Double-buffered Ordering Tables (OT) with DMA Channel 2 chain-linking for maximum fill-rate and minimal CPU stall.
- **Fast 2D/2.5D Sprite Batching**: Utilizing GPU `SPRT` (untextured/textured sprites) and `POLY_FT4` textured quads for high-performance track blitting and particle rendering.

---

## 3. Game Mechanics & Simulation

### 3.1 Vehicle Dynamics & Physics Model
The driving model is engineered for **instant pick-up-and-play arcade fun with an ultra-high skill ceiling**:

1. **Coordinate System & Units**:
   - Fixed-point arithmetic (`i32` with 12-bit fractional part `Q20.12` or `Q16.16`) ensures 100% deterministic simulation across MIPS R3000A and host x86_64.
   - Heading stored as 16-bit angular BAMs (Binary Angle Measurement) or 0–4095 angle units (matching PSX GTE trig tables).
2. **Acceleration, Drag & Top Speed**:
   - Forward thrust governed by car engine curve and tuning.
   - Aerodynamic drag modeled as proportional to velocity squared ($F_{\text{drag}} = C_d \cdot v^2$).
   - Rolling resistance and engine braking when off-throttle.
3. **Drifting & Lateral Grip**:
   - Two friction regimes: **Grip Mode** and **Slip/Drift Mode**.
   - Initiating drift: Hard counter-steering, tapping brake while turning, or lifting off throttle into sharp corners.
   - Dynamic Slip Angle: Visual car angle decouples from velocity vector during drifts, accompanied by tire smoke particles and skidmarks.
   - Counter-steering control: Skillful counter-steer maintains drift speed without spinning out.
4. **Surface Interactions**:
   - **Tarmac (Racing Line)**: Optimal friction ($1.0\times$), crisp turn-in.
   - **Curb / Kerb**: High vibration (triggers DualShock small motor), slight grip reduction ($0.85\times$), audio rumble.
   - **Off-Road (Grass / Gravel)**: Severe speed penalty ($0.35\times$ top speed, high drag), tire kick-up particles.
   - **Oil Slicks / Water Hazards**: Instant loss of lateral friction ($0.1\times$ grip), involuntary spin.
   - **Boost Pads**: Instant impulse acceleration exceeding standard top speed, nitro flame exhaust.

### 3.2 Car Tuning System (Expanded from FX)
Retaining the beloved 10% point-allocation mechanic from ArduRacer FX while adding depth:

| Parameter | Default | Min / Max | Gameplay Effect |
| :--- | :---: | :---: | :--- |
| **Top Speed** | 4 | 1 – 7 | Increases maximum terminal velocity down straights. |
| **Acceleration** | 4 | 1 – 7 | Faster 0–100 km/h sprint, better recovery from slow hairpins. |
| **Handling / Grip** | 4 | 1 – 7 | Sharper turn-in, higher lateral G-forces before breaking into a slide. |
| **Drift Stability** | 4 | 1 – 7 | Easier to maintain sustained high-speed drifts without losing momentum. |
| **Gearing / Boost** | 4 | 1 – 7 | Trade-off between rapid low-end punch and high-end overdrive speed. |

Total points pool: 20 points to allocate freely across 5 categories.

### 3.3 Checkpoints & Timing
- **Multi-point Checkpoint Gate Array**: Each track features 4–12 sequential checkpoint gates across the course.
- **Anti-Cheat Validation**: Checkpoints must be cleared in strict order; cutting across off-road sections to skip gates invalidates the lap time.
- **Timing Precision**: 60 ticks per second yields 16.66 ms resolution. Displays formatted as `MM:SS.ccc` (e.g., `01:23.450`).
- **Delta Splits**: Real-time HUD indicator showing `+0.25` (red, behind best lap) or `-0.14` (green, ahead of best lap) at each checkpoint.

---

## 4. Game Modes

### 4.1 Time Trial (The Ultimate Challenge)
- **Target Times**:
  - **Bronze**: Achievable by a clean run with default tuning.
  - **Silver**: Requires good cornering lines and slight tuning optimization.
  - **Gold**: Requires mastering drift mechanics and finding optimal racing lines.
  - **Dev Platinum**: The creator's absolute best time with perfect tuning and extreme shortcut mastery.
- **Ghost Car**:
  - Real-time translucent silhouette of your personal best lap or the Dev record.
  - Instant visual feedback on braking points, apex lines, and drift angles.

### 4.2 Grand Prix (Arcade Cup)
- 4 Championship Cups:
  - **Bronze Cup**: Tracks 1–5 (Beginner-friendly circuits, wide roads).
  - **Silver Cup**: Tracks 6–10 (Technical turns, chicane complexes).
  - **Gold Cup**: Tracks 11–15 (Narrow mountain passes, hazardous hazards).
  - **Platinum Cup**: Tracks 16–20 + Super Stages (High-speed hyper-tracks, elevation leaps).
- **Rival AI System**:
  - 5 distinctive AI competitors with custom car liveries and driving personalities:
    - *The Speeder*: Blistering straight-line speed, conservative in corners.
    - *The Drifter*: Slides aggressively around every apex, dangerous on tight circuits.
    - *The Tactician*: Follows the perfect mathematical racing line, difficult to pass.
    - *The Brawler*: Aggressive block-passing, defends position with door-to-door contact.
    - *The Rookie*: Prone to overcooking corners and spinning on oil slicks.

### 4.3 2-Player Split-Screen Mode
- Head-to-head racing on a single PlayStation console.
- Dual-viewport rendering (Horizontal split 320×120 × 2 or Vertical split 160×240 × 2).
- Pure competitive fun with slipstream battles and collision shoving.

---

## 5. Visuals & Presentation

### 5.1 Dynamic Camera & Perspective
- **Overhead 2.5D Camera**:
  - Dynamic Zoom: Smooth camera zoom-out as vehicle speed increases, providing greater forward visibility at high speeds.
  - Camera Lead & Heading Tracking: Camera softly anticipates turns by panning slightly forward into the car's heading direction.
  - Corner Banking Tilt: Subtle GTE 3D perspective roll/tilt during high-G cornering drifts, giving an exhilarating sense of speed and momentum.

### 5.2 Hardware Particle & Visual Effects
- **Tire Skidmarks**: Real-time persistent skidmark buffer drawn to VRAM when slipping/drifting.
- **Smoke & Sparks**: Semi-transparent additive blending (`GPU_BLEND_ADD`) for billowing tire smoke, turbo backfire flames, and metallic sparks on wall scraping.
- **Dynamic Headlight Cones**: 16-color translucent lighting masks for night and tunnel stages.
- **Speed Lines**: Subtle radial distortion streaks at maximum top speed / nitro boost.

---

## 6. Audio Architecture

### 6.1 Red Book CD-DA Title & Race Tracks
Uncompressed 44.1 kHz 16-bit stereo Red Book CD-DA audio mastered onto disc tracks:
- **Track 1**: PSX Data Track (Game executable, assets, videos).
- **Track 2**: Title / Menu Theme (*"Neon Overdrive"* - Synthwave / Future Funk).
- **Track 3**: Cup 1 Theme (*"Asphalt Adrenaline"* - 90s Eurobeat).
- **Track 4**: Cup 2 Theme (*"Night Drift City"* - D&B / Jungle Breakbeat).
- **Track 5**: Cup 3 Theme (*"Canyon Rush"* - High-energy Arcade Rock/Techno).
- **Track 6**: Cup 4 Theme (*"Apex Predator"* - Speed Metal / Hard Trance).
- **Track 7**: Victory / Podium Fanfare.

### 6.2 SPU Hardware Sound Effects
- Low-latency SPU ADPCM sound effects:
  - Real-time engine RPM synthesizer with variable pitch modulation matching car velocity and gear shifts.
  - Modulated tire screech loop that changes pitch and volume based on slip angle.
  - Dual-tone checkpoint chimes and lap record fanfare.
  - Impact crunches with directional stereo panning.

---

## 7. Controls & DualShock

| Button | Primary Action | Secondary / Menu Action |
| :--- | :--- | :--- |
| **Left Stick / D-Pad** | Analog / Digital Steering | Menu Navigation |
| **Cross ($\times$)** | Throttle / Accelerate | Confirm / Select |
| **Square ($\Box$)** | Foot Brake / Reverse | Back / Cancel |
| **Circle ($\bigcirc$)** | Handbrake / Drift Initiate | Resume (pause menu) |
| **R1** | Boost / Nitro (all layouts) | Next Tab |
| **L1** | Recovery: Reset Car to Track (all layouts) | Previous Tab |
| **Start** | Pause Game Menu | - |
| **Select** | Toggle In-Game Minimap / HUD | - |

- **Recovery**: if the car is wedged, spun, or facing a wall it cannot drive out
  of, the recovery button drops it back on the nearest route node facing down the
  racing line, clearing all momentum, drift and boost state. Bound to `L1` in
  every layout.

- **DualShock Force Feedback**:
  - Small high-speed motor: Engine revs, curb vibrations, wheel slippage warning.
  - Large low-speed motor: Heavy crashes, wall impacts, rough off-road shudder.

---

## 8. Memory Card Specification

- **File Header**: `BAS-ARDURACER-01`
- **Capacity**: Exactly 1 Memory Card Block (8 KB).
- **Icon**: Custom 16×16 16-color animated 3-frame BIOS icon (spinning checkered flag / flashing race car).
- **Contents**:
  - Lap records (Best times for all 24 tracks with checksum).
  - Trophy & Cup progress (Unlocked tracks, dev medals earned).
  - Tuning setups (3 custom save slots for car tuning presets).
  - Best Ghost lap telemetry (Compact delta-encoded track positions).
  - Player preferences (Audio volumes, DualShock vibration toggle, screen calibration).
