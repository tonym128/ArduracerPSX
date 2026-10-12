# AGENT.md: Multi-Agent Parallel Engineering & Review Protocol

> **Operational manual for autonomous agents collaborating on Arduracer PSX.**  
> *Setting the standard for high-concurrency development, strict peer-review gates, Git worktree isolation, and zero-defect hardware engineering.*

---

## 1. Core Principles

1. **Never Commit Directly to Main**: All feature development, refactoring, and bug fixes occur in dedicated branches via **Git Worktrees**.
2. **Strict Multi-Perspective Review**: No branch is merged into `main` without passing an exhaustive multi-perspective peer review (Principal Architect, Senior Systems Engineer, Game Designer, QA/Verification).
3. **Pure Core Testing Standard**: The host test suite MUST test the real game logic in `crates/arduracer-core`, NEVER a mirror or duplicate implementation.
4. **Hardware Discipline**: Target PlayStation 1 hardware constraints (2 MB Main RAM, 4 KiB I-Cache, 1 MB VRAM, 512 KB SPU RAM, 33.8688 MHz MIPS R3000A CPU). Every PR must respect memory budgets and maintain locked 60 FPS performance.
5. **Zero Lints, Zero Panics**: Code targeting the PSX must be `#![no_std]` with zero panics (`unwrap`, `expect`, `panic!`, `todo!`, `unimplemented!` are forbidden in `game/src/`). All crates must pass `cargo fmt --check` and `cargo clippy -D warnings`.

---

## 2. Architecture & Layer Boundaries

```mermaid
graph TD
    subgraph Host Development & Verification
        TC[Track & Tile Assets<br/>TMX / CSV / PNG] -->|tools/track_cook| TRK[Binary Track Chunks<br/>.trk]
        AUD[Audio Assets<br/>WAV / MP3] -->|tools/audio_cook| VAG[SPU ADPCM .vag<br/>CD-DA .cdda]
        VID[Cinematics<br/>MP4] -->|tools/encode_mdec| STR[MDEC Stream<br/>.STR]
        
        COOK[tools/track_cook/build_atlas.py<br/>Authored circuit images in tracks/*.png<br/>Validates gates & surfaces]
        CORE[crates/arduracer-core<br/>Pure #![no_std] Game Engine<br/>Physics, Collision, Timing, AI, Saves]
        TEST[tools/test_game_logic<br/>Host Test Suite<br/>Sweeps, Fuzzers, Bit-flip Verification]
        PLAY[tools/playtest<br/>Circuit Playability Verifier<br/>Simulates real laps on all 24 tracks]
        CAL[tools/track_cook/par_calibration.json<br/>Measured Medal Targets]
        COOK -->|levels.rs| CORE
        TEST -->|Direct Dependency| CORE
        PLAY -->|Direct Dependency| CORE
        PLAY -->|--calibrate| CAL
        CAL -->|read by cooker| COOK
    end

    subgraph PSX Target Hardware
        GAME[game/src<br/>Bare-Metal PSX Application<br/>MIPS R3000A #![no_std]]
        GAME -->|Direct Dependency| CORE
        GAME --> GTE[GTE Coprocessor 2<br/>Fixed-point Matrix & 2.5D Tilt]
        GAME --> GPU[GPU DMA & Ordering Tables<br/>Double-buffered 60 FPS]
        GAME --> SPU[SPU Audio Driver<br/>Dynamic RPM & SFX]
        GAME --> PAD[DualShock Controller Driver<br/>Analog & Dual-Motor Rumble]
        GAME --> MC[Memory Card Driver<br/>1-Block Save with BIOS Icon]
    end

    subgraph Disc Mastering
        GAME --> EXE[dist/arduracer.exe]
        TRK --> ISO[mkisopsx Masterer]
        VAG --> ISO
        STR --> ISO
        EXE --> ISO
        ISO --> DISC[dist/arduracer.bin + .cue<br/>Bootable PS1 Disc]
    end
```

### Module Responsibilities:
- **`crates/arduracer-core`**:
  - Pure `#![no_std]` library.
  - Zero platform dependencies (compiles for both `x86_64-unknown-linux-gnu` and `mipsel-sony-psx`).
  - Contains:
    - Vehicle dynamics, acceleration curves, friction/drift physics, fixed-point math (`Q20.12`).
    - Collision detection against track tiles, borders, and rival cars.
    - Checkpoint gate array and lap timer logic with anti-cheat state machine.
    - Deterministic ghost car recording and delta-compression replay serializer.
    - Memory card save struct (`SaveData`), 16-bit CRC/checksums, and serialization.
    - AI waypoint pathfinding and overtaking decision logic.
- **`game/src/`**:
  - The bare-metal PlayStation 1 runtime using `psoxide`.
  - Responsible strictly for hardware interfacing:
    - VRAM texture uploading and double-buffer allocation.
    - GPU Ordering Table (OT) packet generation and DMA Channel 2 dispatch.
    - GTE matrix multiplication and camera perspective transformation.
    - SPU voice allocation, pitch register modulation, and ADPCM streaming.
    - CD-ROM controller commands (CD-DA audio track playback, sector streaming).
    - DualShock polling, deadzone calibration, and rumble motor actuation.
- **`tools/test_game_logic/`**:
  - Host test harness testing `crates/arduracer-core`.
  - Runs in milliseconds under standard `cargo test` on developer workstations and CI.
- **`tools/playtest/`**:
  - **Circuit playability verifier.** Simulates a full 5-lap race on all 24
    circuits with a reference driver *and* all 5 rival personalities, then
    asserts per-circuit geometry, gate validity, route ordering and medal
    reachability. This is the gate that catches "the game builds but is not
    actually playable" defects; run it with `make playtest`.
  - `--render` dumps every circuit as ASCII for geometry review.
  - `--calibrate` re-measures reference laps and rewrites
    `tools/track_cook/par_calibration.json` (`make atlas-and-calibrate`, which
    then folds the table into `levels.rs`).
- **`tools/`**:
  - Asset cooking scripts and CLI tools (track compiler, VAG audio compiler, MDEC movie encoder, disc masterer).
  - `tools/track_cook/build_atlas.py` is the **single source of truth** for
    `crates/arduracer-core/src/levels.rs` and `visual_tex.rs`, compiled from the
    authored circuit images in `tracks/`. Never hand-edit those files: run
    `make atlas`, which validates every circuit first and refuses to emit on an
    unreachable gate, an off-road gate, or a start box outside the racing
    surface. `make circuits` redraws the images from
    `tools/track_cook/generate_svg_circuit.py`.

### 2.1 Circuit Texture Codecs (SVG, QOI, JPEG, TIM)

`make atlas` emits **four** disc assets per circuit, and the runtime picks per
circuit. `gpu::tracktex` streams one 1024x1024 compressed block at a time into a
single 500 KB RAM buffer, then decodes 64x16 MCU rows from it into VRAM.

| Asset | Codec | Flag | Decoder | Notes |
| --- | --- | --- | --- | --- |
| `assets/SVGTRACKS.BIN` | `ARSV`: SVG raster + code map + per-surface detail | `svg-track-texture` | `arduracer_core::svgtex` | Preferred. Straight vector edges, synthesised detail. |
| `assets/TRACKS.BIN` | Baseline JPEG, `DRI=4` | always | `arduracer_core::jpeg` | Always present; last-resort fallback. |
| `assets/QOITRACKS.BIN` | Segmented QOI | `qoi-track-stream` | `arduracer_core::qoi` | Lossless copy of the SVG raster, no detail pass. |
| `assets/TIMTRACKS.BIN` | Uncompressed 15bpp, DMA'd | `tim-track-stream` | none | No decode at all. Explicitly selected; never chosen by `Auto`. |

`codec_for_circuit` steps down SVG -> QOI -> JPEG. Every step down is a pure
fallback: the feature off, the disc missing the asset, or the circuit's table
entry empty all fall through, so no combination of flags can remove content. The
three 10 km^2 city maps are JPEG-only, since their art is generated procedurally
at cook time.

### 2.2 The TIM texture path (uncompressed, DMA to VRAM)

TIM is the one format with **no decoder**. A 64x64 15bpp tile is 8192 bytes =
exactly 4 CD sectors, so `upload_tile_dma` reads those four sectors straight
into the shared 8 KB staging buffer and hands them to the GPU's block-mode DMA.
No header, no entropy coding, no inverse DCT.

**Why it is not in the `Auto` walk.** A 1024x1024 15bpp block is 2,097,152
bytes — twice the machine's main RAM. It therefore cannot be streamed as a
block at all, and `RamBlock`'s 512 KB buffer is not involved. Instead each VRAM
tile miss becomes its own 4-sector read. On a single-block track layout that is
1024 short reads where SVG/QOI/JPEG do one 500 KB read to serve 256 tiles, and
`cd_fs::read_sectors` pauses and resumes CD-DA playback around every one of them.
That is audible, so TIM is a player choice (`TextureFormat::Tim`) rather than a
default, and the **CD AUDIO** setting exists alongside it for exactly this case.

**Why the tile grid is arithmetic and not a table.** A block is 256 tiles laid
out row-major, four sectors each, so a tile's LBA is
`block_lba + (tile_y * 16 + tile_x) * 4`. No per-tile index is shipped or
streamed, which is what makes a 2 MB uncompressed block affordable at all.
`visual_tex::CIRCUIT_TIM_SECTORS` carries only the per-circuit base.

**Why DMA uses BS = 16 words and not one row.** The GPU command FIFO is 64 bytes,
so a block size wider than 16 words can wedge real hardware (CL2 probe,
2026-07-31). But block mode advances MADR by *BS words after each block*, so
laying BS-word blocks back to back walks a contiguous address range for any BS.
`psx_vram::dma_block_params` therefore clamps BS to the FIFO width and lets BA
carry the count: a 64x64 tile is BS = 16, BA = 128, moving the same 2048 words as
the row-shaped BS = 32 / BA = 64 that used to be refused outright. It also falls
back to the GP0 FIFO path if the channel does not report done, which is the
documented CL2 failure mode.

`tools/track_cook/verify_tim.py` checks the on-disc bytes against the source
raster. This exists because the path has no decoder: a swapped channel or a
host-endian leak ships a disc that boots, renders, and is subtly wrong everywhere,
and nothing in the game can report it.

### 2.3 The SVG texture path (`ARSV`)

`.visual.png` is painted from the 80x80 code map
(`generate_svg_circuit.paint_visual`), which quantises geometry to 12.8 px cells
and adds a per-pixel grain hash. That grain is what makes the old images
expensive: it was ~325 KB of a 396 KB QOI block, as incompressible noise.

`ARSV` instead stores the SVG rasterised at the full 1024 visual resolution,
which is smoother *and* about four times cheaper, because the extra pixels are
smooth runs rather than noise. It stores that raster in a segmented-QOI payload
(the decoder core is shared with `qoi::decode_segment_rgb`, not reimplemented)
plus the 80x80 code map packed one nibble per cell.

The detail comes back at decode time instead. `svgtex::detail_delta` picks a
function from the texel's surface code and evaluates it per texel: tarmac
aggregate, grass clumps, gravel speckle, sand ripple, oil banding, boost
chevrons, start-line checks, wall hatching, tunnel ribs, bridge planks, scenery.
Deliberately integer-only, since the R3000A has no FPU, and deliberately
evaluated per texel rather than per cell -- quantising detail to the cell would
reintroduce exactly the blockiness the SVG raster exists to remove.

**Why QOI is segmented.** Stock QOI is a single linear stream with no random
access, so it cannot be sliced into MCU rows the way the VRAM streamer needs.
`tools/track_cook/qoi_codec.py` therefore cuts each image into 1024 independently
decodable 64x16 segments with a `u32` offset table, restarting QOI state per
segment. That reproduces the JPEG restart-interval contract exactly — same
partition, same order, `index = (tile_y * 4 + row) * 16 + tile_x` — so the two
codecs are interchangeable call sites.

**Why some circuits have no QOI block.** QOI is lossless, so the only way to
force an over-budget image to fit is to discard detail first. For the one
circuit whose art currently exceeds the budget (`Amber_Mesa.visual.gen.png`),
JPEG reaches 486 KB at RMSE 3.7 while forcing QOI to fit needs 1-bit
quantisation at RMSE 39. `compress_1024_qoi_500kb` returns `None` in that case
rather than shipping a much worse image to save a decode win, and the runtime
falls back automatically. Adding a circuit is therefore always safe: if it fits,
it gets QOI; if not, it keeps JPEG.

Changing the codec requires **no** change to `RamBlock`'s size: both codecs
share the one 512 KB `block_data` buffer and the same 1024-entry offset array.
A second per-codec buffer would cost another 512 KB of the 2 MB main RAM and
would blow the `ci-game` static-RAM ceiling.

---

## 3. Git Worktree & Parallel Agent Workflow

To allow multiple autonomous agents to work concurrently on physics, graphics, audio, UI, tracks, and tools without branch collisions or git lock contention:

### 3.1 Worktree Directory Structure
All worktrees reside in `.worktrees/`:
```
/home/tonym/Projects/ArduracerPSX/
├── .worktrees/
│   ├── wt-physics-core/    # Agent A: Vehicle dynamics & drift math
│   ├── wt-gpu-renderer/    # Agent B: GPU ordering table & tile blitter
│   ├── wt-track-pipeline/  # Agent C: TMX importer & Arduracer FX track conversion
│   ├── wt-audio-spu/       # Agent D: SPU sound driver & engine pitch modulation
│   ├── wt-ui-screens/      # Agent E: Tuning garage & championship menus
│   └── wt-ai-opponents/    # Agent F: Grid AI & waypoint racing
```

### 3.2 Creating an Isolated Worktree
When launching a task:
```bash
ROOT="/home/tonym/Projects/ArduracerPSX"
NAME="wt-feature-name"
BRANCH="feat/feature-name"

# 1. Add worktree on a new branch branched from main
git worktree add "$ROOT/.worktrees/$NAME" -b "$BRANCH" main

# 2. Link vendored psoxide (if submodule/vendored folder is used)
if [ -d "$ROOT/psoxide" ]; then
    rmdir "$ROOT/.worktrees/$NAME/psoxide" 2>/dev/null || true
    ln -s "$ROOT/psoxide" "$ROOT/.worktrees/$NAME/psoxide"
fi
```

### 3.3 Building & Verifying Inside a Worktree
- **Host Tests** (fast iteration):
  ```bash
  cd "$ROOT/.worktrees/$NAME"
  make test        # unit suite + host harness
  make playtest    # simulate real laps on every circuit
  ```
- **PSX Target Compilation** (bare-metal cross-build):
  ```bash
  cd "$ROOT/.worktrees/$NAME/game"
  cargo build --release
  ```
- **Format & Lint Check**:
  ```bash
  cd "$ROOT/.worktrees/$NAME"
  cargo fmt --all -- --check
  cargo clippy --all-targets -- -D warnings
  ```

### 3.4 Merging & Cleaning Up
Once the stringent review passes (see §4):
```bash
cd "$ROOT"
git merge --no-ff "feat/feature-name" -m "feat(scope): Description of changes"
git worktree remove --force "$ROOT/.worktrees/$NAME"
git branch -d "feat/feature-name"
```

---

## 4. Stringent Multi-Perspective Review Process

Before ANY worktree branch is merged into `main`, it must undergo a structured review from **four distinct engineering perspectives**. The review findings must be documented in the PR / merge summary.

```
       ┌────────────────────────────────────────────────────────┐
       │             Branch Verification & Tests Pass           │
       └───────────────────────────┬────────────────────────────┘
                                   │
       ┌───────────────────────────┴────────────────────────────┐
       │             FOUR-PERSPECTIVE REVIEW AUDIT             │
       ├────────────────────────────────────────────────────────┤
       │ 1. Principal Architect       - Modularity & Boundaries │
       │ 2. Senior Systems Engineer   - PSX Hardware & Budgets  │
       │ 3. Game Experience Designer  - Feel, Pacing & Visuals  │
       │ 4. Verification / QA Lead    - Test Suites & Coverage  │
       └───────────────────────────┬────────────────────────────┘
                                   │ (All 4 Approved)
       ┌───────────────────────────▼────────────────────────────┐
       │              Merge Approved into `main`                │
       └────────────────────────────────────────────────────────┘
```

### 4.1 Review Perspective Checklists

#### 1. Principal Architect
- [ ] **Boundary Cleanliness**: Does `crates/arduracer-core` remain 100% free of PSX hardware headers, graphics types, or IO handles?
- [ ] **No Code Duplication**: Are structs, constants, or algorithms declared once in `core` rather than mirrored in tests or game submodules?
- [ ] **State Ownership**: Is mutable state explicitly localized? Zero global mutable singletons without strict synchronization/reset paths.
- [ ] **`#![no_std]` & Panic Freedom**: No `.unwrap()`, `.expect()`, `panic!()`, or unbounded recursion in PSX-bound code.

#### 2. Senior Systems & PSX Hardware Engineer
- [ ] **Main RAM Footprint**: Does the PSX executable (`arduracer.exe`) remain well under the 2 MB limit (ideal: < 500 KB total)?
- [ ] **I-Cache Locality**: Are inner loop functions compact enough to fit inside the 4 KiB direct-mapped I-cache?
- [ ] **GPU Ordering Table & DMA**: Are GPU packets submitted via DMA list chains without blocking the CPU?
- [ ] **VRAM Management**: Does texture / CLUT allocation stay strictly within the assigned pages without overlapping double-buffer displays?
- [ ] **SPU Budget**: Total sound bank size stays under 250 KB (well within 512 KB SPU RAM limit).

#### 3. Game & Experience Designer
- [ ] **Arcade Responsiveness**: Locked 60 FPS with zero dropped frames or stuttering.
- [ ] **Driving Feel**: Is drift initiation intuitive, rewarding, and controllable?
- [ ] **Visual Clarity**: Is the track boundary, surface type (curb, grass, oil), and checkpoint indicator instantly readable at high speed?
- [ ] **Audio Feedback**: Do engine revs, tire squeals, and curb chatter provide immediate acoustic cues to the player?

#### 4. Verification & QA Engineer
- [ ] **Automated Host Tests**: All unit and integration tests pass with `cargo test`.
- [ ] **Edge Cases Covered**: Boundary collisions, reverse-direction checkpoint crossings, maximum speed overflows, and timer wrap-arounds tested.
- [ ] **Playability Proven**: `make playtest` completes 5 timed laps on all 24 circuits for the player *and* every rival personality. A green unit suite is **not** sufficient evidence that the game is playable - the lap simulation is.
- [ ] **Formatting & Linting**: `cargo fmt --check` clean; `cargo clippy -D warnings` clean across all crates.
- [ ] **Lockfile Consistency**: Cargo dependencies locked and reproducible.

---

## 5. Media & Asset Pipeline Protocols

### 5.1 Track & Level Pipeline (`tools/track_cook`)
- Input: TMX (Tiled JSON/XML) and CSV files from `ArduRacerFx/Levels/`.
- Process:
  1. Parse tile IDs, checkpoint polyline triggers, start grid coordinates, and surface property flags.
  2. Optimize tile arrays into 16×16 spatial chunks for fast camera culling and minimal VRAM texture swaps.
  3. Generate waypoint graph nodes with curvature metadata for AI pathing.
- Output: Compact binary `.trk` file loaded directly into PSX RAM or streamed off disc.

### 5.2 Sprites & Textures (`tools/texture_cook`)
- Input: High-resolution PNG sprite sheets and tile maps.
- Process:
  1. Quantize to 16-color (4-bit CLUT) or 256-color (8-bit CLUT) optimized palettes.
  2. Generate 64-directional vehicle rotation frames with dynamic lighting highlights.
  3. Pack into Sony PSX TIM file format with page coordinates and CLUT VRAM coordinates.
- Output: Compressed `.tim` binaries ready for DMA transfer into VRAM.

### 5.3 Audio Pipeline (`tools/audio_cook`)
- **Sound Effects (SPU)**:
  1. Source: 44.1 kHz 16-bit WAV audio.
  2. Process: Resample to 22,050 Hz or 11,025 Hz, encode to PSX 4-bit ADPCM VAG format with loop flag markers.
  3. Output: SPU sample bank binary (`sounds.bnk`) with lookup table.
- **Soundtrack (CD-DA)**:
  1. Source: Mastered stereo tracks.
  2. Process: Encode to 44.1 kHz 16-bit uncompressed PCM stereo audio (`.raw` / `.cdda`) padded to exact 2352-byte sector boundaries.
  3. Output: Disc image CUE sheet with Track 1 (Data) and Tracks 2–7 (Audio).

### 5.4 FMV Cinematics (`tools/encode_mdec`)
- Input: MP4 1080p source video (Attract Intro, Ending Cinematic).
- Process:
  1. Downscale to 320×240 @ 15 fps (or 30 fps).
  2. Encode to PSX MDEC BS (bitstream) macroblocks.
  3. Interleave with CD-XA ADPCM audio sectors.
- Output: Bootable `.STR` movie files streamable via PSX CD-ROM DMA Channel 2.

---

## 6. Subagent Tasking Checklist

When invoking a subagent for a task:
1. Specify the exact branch name and worktree path.
2. Outline the exact requirements, constraints, and target files.
3. Require that the subagent executes the local test suite and linter before reporting back.
4. Require the subagent to produce a structured review summary addressing the 4 review perspectives.
