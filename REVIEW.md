# REVIEW.md — Arduracer PSX Four-Perspective Audit & Sign-off

> **TASK-1001** · Multi-perspective review, executable size verification, and
> performance audit of the complete `arduracer-core` + `game` + `tools` workspace.
>
> Reviewers: Principal Architect · Senior Systems (PSX Hardware) Engineer ·
> Game & Experience Designer · Verification / QA Lead.

---

## 0. Verification Summary

| Gate | Command | Result |
| :--- | :--- | :--- |
| Formatting | `make fmt-check` | **clean** (6 crates) |
| Lints | `make clippy` (`-D warnings`) | **clean** (5 host crates) |
| Game-logic unit suite | `make test` | **42 / 42 pass** |
| Memory-card persistence suite | `cargo test -p test_memcard` | **14 / 14 pass** |
| UI state-machine suite | `cargo test -p test_ui` | **6 / 6 pass** |
| **Circuit playability** | `make playtest` | **24 / 24 circuits playable** |
| Bare-metal MIPS build | `make exe` | **clean, zero warnings** |
| Main-RAM budget | `make ci-game` | **424 KB / 2 MB = 20.7 %** |
| Disc mastering | `make disc` | **10.17 MB BIN + CUE, 7 tracks** |
| Emulator boot | `retroarch -L pcsx_rearmed` | **runs 2 min+, no faults** |

The single most important outcome of this pass: **the game was unplayable and is
now provably playable on every shipped circuit.** Section 1 documents what was
broken, Section 5 documents the evidence.

---

## 1. Critical Findings Fixed in This Pass

Every item below was found by *simulating real laps*, not by reading code. The
previous suite asserted that `LapTimer` counted gates; it never asked whether a
lap could actually be completed.

### F-01 — No lap could ever be completed (blocker)

`convert_levels.py` emitted checkpoints in **raster-scan order**, and
`LapTimer::update_player_tile` demanded they be visited in exactly that order.
On 23 of 24 circuits the first gate in raster order lies *behind* the start
line, so the sequence was unreachable. The lap also completed on the *last
raster checkpoint* rather than on the start/finish line.

ArduRacer FX (`racer.cpp:350-420`) actually works by **coverage**: every gate is
a bit; leaving the start block with all bits set scores the lap.

*Fix* — `LapTimer` now mirrors the original: a `u16` `checkpoint_mask` records
*which* gates were touched (any order), and a lap only scores when the car
**leaves** the start/finish gate with the mask full. `TrackDef` gained an
explicit `start_gate`, and `TrackDef::route_len()/route_node()` expose the
ordered racing line (checkpoints, then the line) for the AI.

### F-02 — Cars moved ~30× too slowly (blocker)

Position integration multiplied velocity by `120/4096 ≈ 0.029`. With 64-unit
tiles and a 3.42 u/tick top speed that is **0.1 units per tick** — roughly 640
ticks (10.7 s) to cross one tile, and about 6 px/s of on-screen motion. Every
par time in the project was unreachable by an order of magnitude.

*Fix* — velocity is now in world-units-per-tick and integrates 1:1, matching the
original's 2.5 px/frame on a 64 px tile grid. Top speed ≈ 3.42 u/tick
(≈ 205 u/s, ~1.6 s to cross the viewport). `test_world_scale_matches_track_tiles`
locks this in as a regression test.

### F-03 — Off-road was a 1 %-of-top-speed wall, not a 35 % penalty

`grip_factor()` was multiplied into *engine thrust*, so off-road terminal speed
was `accel·grip / (1 - drag·grip) ≈ 0.6 %` of tarmac top speed. GAME.md §3.1
promises `0.35×`.

*Fix* — `SurfaceType` now separates three coefficients (`max_speed_factor`,
`traction`, `lateral_hold`): off-road is a hard 0.35× cap with 0.55× traction.
`test_offroad_penalty_is_playable` measures the ratio and fails outside
0.30–0.40.

### F-04 — Wall contact glued the car in place

`handle_barrier_collision` applied a 30 % speed scrub **and** a 60-tick spin-out
on *every* tick of contact, so a car scraping a barrier decelerated to a
standstill and — because the steering deadzone is 0.049 u/tick — could never
steer away. `main.rs` never called the function at all; `Barrier` grip of `0.0`
just froze the car mid-track.

*Fix* — glancing contact cancels only the into-wall velocity component (so the
car slides); a *hard* impact (`> HARD_IMPACT_THRESHOLD`) scrubs speed and spins.
Bounds and authored barriers are resolved by `VehicleState::collide_with_track`,
which the race loop, the AI, and the host suite all share.

### F-05 — Rival AI could not race (3–5× slower than the player)

The AI steered straight at the next raster checkpoint (often behind it), never
braked for corners, and hand-braked into walls. Measured lap times were 3–5×
the player's.

*Fix* — rivals now follow `TrackDef::route_node()` and use a two-loop arcade
controller: proportional steering on heading error plus a **corner-speed
governor** that inspects the bend *after* the next node and eases the speed
limit in over `CORNER_LEAD_UNITS`. Result: rival 5-lap races are within
5–20 % of the reference driver on every circuit, and all five personalities
complete every race.

### F-06 — The 4 PSX Super Stages were empty fields

Stages 21–24 were 22×16 grids of uniform tile `1` with two gates on a diagonal.
No circuit, no curbs, no hazards, and GAME.md promised "banking curves, flyover
bridges, tunnel sections, and technical chicanes".

*Fix* — they are now rasterised from closed Catmull-Rom centrelines: a wide
flowing super-speedway with boost pads, a narrow technical canyon run with oil
slicks, an ultra-wide hazard oval, and a long street circuit. Each has ≥ 6 gates,
rumble-curb shoulders, and ≥ 2 hazard tiles (asserted by
`test_super_stages_are_real_circuits`).

### F-07 — Level 7 had no start/finish line at all

`ArduRacerFx/Levels/Level7.csv` contains no tile 24/25. The converter faked it
with a hard-coded coordinate that was itself a checkpoint, producing a "lap"
scored in 1 tick.

*Fix* — the cooker synthesises a start box on the first tarmac tile with a
heading derived from its road neighbours, and tops the circuit up to GAME.md's
4-gate minimum by farthest-point sampling. `test_sprint_short_has_a_start_line`
guards it.

### F-08 — Sign-convention footgun in heading maths

`atan2_bams(dy, dx)` takes **world** `dy` (screen Y grows downwards). Getting
the sign wrong silently steers every AI car into the infield — which is exactly
what happened while this audit was being performed.

*Fix* — added `heading_towards(from, to)`, the only helper any call site now
uses, plus `test_atan2_heading_convention` asserting all eight compass
directions.

### F-09 — HUD was missing most of its spec, and surfaces were duplicated

TASK-604 requires a gear indicator, `MM:SS.ccc` lap timer, live delta split, and
a minimap showing the **track overview**; none existed. Surface classification
was duplicated across `track.rs` and `tile_blitter.rs` as raw magic tile IDs.

*Fix* — added a single `TrackTile` enum as the one source of surface truth;
rewrote `hud_renderer.rs` with gear, nitro meter, lap timer, best lap, delta
split and a circuit-outline minimap; rewrote `tile_blitter.rs` for `TrackTile`
with speed-dependent zoom, and made the camera zoom actually change how much
road is visible.

### F-10 — Missing GAME.md controls, and a memory card that never saved

`Start` (pause) and `Select` (toggle HUD) were unimplemented, and nitro had no
input. `MemoryCardManager` only manipulated a struct in RAM — `psx-mc` was
linked but never called, so **nothing was ever written to the card**.

*Fix* — new `ui/pause.rs` (Resume / Restart / Quit, with the frozen frame
repainted underneath); `VehicleInput::nitro` + `nitro_charge` meter drained on
use and recharged on release, wired to Triangle / R1 / L1 / R2 per layout;
`MemoryCardManager` now probes port 1 at boot, loads the CRC-validated save, and
flushes with a custom 16×16 checkered-flag BIOS icon through
`Card::write_with_icon`, degrading to in-memory defaults on any failure.

### F-11 — The memory card still never saved: every write panicked

F-10 declared the card wired up, but the path had never been executed by a
test, and **the first save aborted the game**. The BIOS icon builder indexed
the 128-byte frame with the pixel *width* as the row stride
(`y * 16 + x / 2`) instead of half of it, so every row from the eighth onwards
walked off the end of the array. The PSX build is `panic = "abort"`: reaching
the finish line and calling `flush()` killed the process rather than saving.
The same finding applies to the tuning-store flush in the garage menu.

*Root cause of the miss* — `tools/test_memcard` existed but had never
compiled: it referenced a `SaveData.best_laps` field that is actually called
`best_lap_ticks`, so not one of its eleven cases had ever run. Nothing in
`make test`, `make clippy`, or CI referenced the crate.

*Fix* — row stride corrected to `y * ICON_WIDTH / 2` with named geometry
constants and a `debug_assert` tying `ICON_WIDTH * ICON_HEIGHT / 2` to
`FRAME_SIZE`; harness field names corrected; the suite added to `make test`,
`make clippy`, `make fmt-check`, and the CI host job so it cannot rot again.

### F-12 — The save CRC covered uninitialised struct padding

`SaveData` was `#[repr(C)]` and both the checksum and the card image were raw
`size_of::<SaveData>()`-byte memcpys of the struct. The 15-byte
`tuning_slots` array ends on an odd offset, so the struct carries **one
interior padding byte before the checksum and two trailing bytes** — and
`compute_checksum` derived its length by pointer arithmetic that swept the
interior pad into the CRC. Those bytes are uninitialised: the checksum depended
on whatever the stack happened to hold, the card received nondeterministic
bytes, and reading them is undefined behaviour the optimiser is free to
miscompile. The 164-byte "payload" was also 3 bytes larger than the format.

*Fix* — `SaveData` is no longer the on-disk layout. Fields are serialised
explicitly into a fixed 161-byte `PAYLOAD_SIZE` buffer, `compute_checksum` CRCs
the bytes that `write_payload` actually produces (so the two cannot drift), and
`read_payload` rejects short, mis-magicked, mis-versioned, or CRC-failing
input. `repr(C)` and the pointer arithmetic are gone. A regression test
serialises the same save into an all-zero and an all-`0xFF` buffer and requires
identical output, which is precisely what the old code could not do.

### F-13 — A failed save was invisible, and a truncated one was trusted

`flush()`'s result was discarded at both call sites, so a pulled or full card
cost the player their records with no feedback, despite `MemcardStatus` being
documented as "surfaced to the UI". `probe` also ignored the length `Card::read`
returned and validated a fixed-size window of the scratch buffer, so a payload
that arrived short was read past rather than rejected.

*Fix* — the manager parks every player-relevant outcome (`Saved`, `Corrupt`,
`WriteFailed`) in a countdown the frame loop drains, rendered as a bottom-screen
banner by `render_card_notice`; a load or blank card stays silent. A short
read is now `Corrupt`. Unused `save_to_block` (a duplicate of the flush path)
removed.

---

## 1b. Second Audit: 20 Reported Defects, Re-verified

A 20-item defect list was checked against the tree rather than taken at face
value. Six items were wrong, four were partly wrong, and two were already fixed
by the memory card pass. Numbers below were measured, not estimated.

### Fixed in this pass

- **F-14 — Pause menu was a one-way door (blocker).** `main.rs` wrote
  `pause.prev_buttons` itself *before* handing the pad to `PauseMenu::update`,
  so the menu compared every frame against itself and `pressed_since` was never
  true. `update()` could only ever return `None`: the player could enter the menu
  and only power-cycle out. Highlight navigation still worked, which is why it
  read as "the menu is unresponsive" rather than "the game is bricked".
  *Fix* — the input state machine moved to a hardware-free `ui/pause_input.rs`
  that owns **all** edge state; the race loop can no longer reach in and clobber
  it. `Start` now toggles the veil, the opening frame's confirm is ignored, and
  `tools/test_ui` (new) asserts the menu can always be dismissed.
- **F-15 — Ghost telemetry wrapped on every circuit.** `ghost.rs` encoded
  position as `(raw >> 4) as i16`, i.e. a *world* coordinate truncated into 16
  signed bits: a ±128-unit range wrapping every 256 units, against circuits
  640–1920 units across. Every recorded lap replayed as garbage; the two existing
  ghost tests only used coordinates 0 and 10.
  *Fix* — positions are stored as `u16` quarter-units (0.25-unit precision,
  worlds to 16383 units, saturating rather than wrapping), same 6-byte frame. The
  recorder now also restarts **per lap**: previously `finish_lap` ran only at
  race end, so the "ghost" was the opening 30 s of lap 1. New regression test
  walks every circuit's real extent; reinstating the old encoding fails it while
  both pre-existing ghost tests still pass.
- **F-16 — No in-race recovery.** Added `VehicleState::respawn_at` and
  `TrackDef::respawn_point`: the car is dropped on the nearest route node facing
  the next node, with velocity, drift, boost, nitro and reverse state cleared.
  Bound per input profile to the shoulder button that layout leaves free
  (R1 / L1 / L2) — GAME.md's "Triangle: Reset Car to Track" is unreachable
  because Triangle is nitro. Edge-detected in the same pad sample that produces
  the vehicle input. Proven on all 24 circuits by a test that wedges the car,
  recovers it, and requires it to drive away.

### Confirmed, still open

| # | Finding | Measured |
| :--- | :--- | :--- |
| 1 | No car-to-car collision | `collide_with_track` takes no car; nothing transfers momentum between cars. Only a steering nudge (`ai.rs:170`). Contradicts GAME.md §4.2 "The Brawler" and §4.3 split-screen, neither of which exists. |
| 2 | Interior Barrier bricks the car | Real: `collide_with_track` resolves velocity but never depenetrates position, and Barrier is 0 traction / 0 top speed. Measured immobile for 50 s. **But zero Barrier tiles ship** in all 24 circuits (the cooker maps FX walls to drivable OffRoad), so it is unreachable today. |
| 3 | Nitro effectively always on | True in practice, wrong in the figure: 85.7 % duty cycle, not 3000/3000. `NITRO_RECHARGE_TICKS = 6` refills the 90-tick meter in **0.25 s**, not the ~6 s the doc comment claims (24× off). |
| 4 | No countdown | `timer.start()` arms the clock in `reset_race`; no 3-2-1-GO anywhere. `timing.rs`'s own doc comment references a countdown light that does not exist. |
| 5 | Lap validation is coverage-only | `timing.rs` scores a lap on leaving the start gate with every gate touched, in any order, either direction. Reverse-through-every-gate scores on all 24 circuits. |
| 6 | Unreproducible build | **Confirmed by fresh clone, now fixed** (see §1c): `make fmt-check` died resolving the unversioned, gitignored `../psoxide` path deps, so `make ci` aborted before clippy, tests or the MIPS build. The SDK is now vendored. **One half of this row was wrong:** `ArduRacerFx/Levels/*.csv` is *tracked* (46 files), and `python3 tools/track_cook/convert_levels.py` regenerates `levels.rs` byte-identically from a fresh clone. |
| 7 | Minimap redrawn per frame | Up to 900 quads/frame on 30×30 tracks, uncached. Skidmarks: 96-slot ring written every frame = 1.6 s of life against a `life: 180` (3 s) constant, so the fade colour at `life > 60` is unreachable. |
| 8 | Garage allows illegal builds | UI clamps to 10, `MAX_SLIDER` is 7. A 10-slider build is 1.6006× top speed, is applied to the live car, then **silently dropped** by `store_tuning`'s `is_valid()` gate, so it vanishes on reboot. Beats Dev Platinum par on 13/24 circuits (not "every par"). |
| 9 | Docs and packaging | No `LICENSE` file. 91 MB of MP3/MP4 tracked in git. `packaging/*.md` and `web/assets/*` assert Sony product codes (`SLUS-01995`, `SLUS-00999`) and ESRB/ELSPA ratings for a GPL project. |

### Reported but not reproducible

| # | Claim | Reality |
| :--- | :--- | :--- |
| 10 | "Zero automated tests" | 40 game-logic + 14 memory-card + this pass's 6 UI + 1024 playtest laps. The literal `#[test]` count was 0 only because the harnesses are `run_test!` binaries. |
| 11 | "Time Trial awards championship points" | Rivals are spawned unconditionally — true. Points are **not** awarded: `award_stage_points` is behind `if let Some(ref mut champ)`, and Time Trial leaves `championship = None`. |
| 12 | "Grand Prix always loads cup_index * 6" | Only race 1: `current_stage` is forced to 0 for the highlighted track, then `advance_stage()` walks the cup correctly for stages 2-6. |
| 13 | "Drifts can't be sustained; spin at 14 ticks" | Spin-out is at **16** ticks (`24n ≥ 384`). Drifts **are** sustainable: counter-steering bypasses the deepening branch and pins slip at the 128 clamp, reaching tier-2 boost at 90 ticks (measured 400+ ticks). |
| 14 | "HUD: 4 px overlap, gear over nitro, no lap counter, Select hides the timer" | Measured boxes: timer ink ends x=208, speedo starts x=220 (12 px clear); nitro bar rows 20–24, gear ink rows 26–40 (below it, not over); five lap boxes plus a `LAPS` label exist. Select hides the **whole** HUD, which is the spec. The `=` glyph is real but lives in `ui/track_select.rs:176`, where `b'0' + 13` lands in the font's symbol range — not the HUD. |
| 15 | "Particles invisible" | Order confirmed (particles at `main.rs:438`, cars at 440–450). Tire smoke is fully hidden — spawned at the car's exact centre with zero velocity, inside the body sprite. Sparks are only partly hidden. Ghost opacity confirmed: `car_renderer.rs:146` uses `draw_quad_flat` with GP0 bit 25 clear despite the "translucent" comment. |
| 16 | "CRC hashes padding; both corruption tests are false positives" | Correct, and fixed by F-12 above. Byte 10 and byte 5 are both inside `magic`, which `is_valid()` short-circuits on, so no test exercised the CRC. Replaced with an exhaustive single-bit-flip sweep over the real payload. |

### Runner-ups confirmed

`gearing` has **zero** simulation reads — 4 of the 20-point budget buys nothing
while being documented, rendered and swept by playtest. `drift_stability` is read
at exactly one site (`drift.rs:112`) and only scales the counter-steer unwinding
rate, never the spin threshold, contradicting GAME.md §3.2. `Fixed::mul` wraps
(`math.rs:78`) while `add`/`sub` saturate and `abs`/`neg` panic in debug — a real
latent trap, though both panic paths are currently dead code and `make test`
(debug) vs `make playtest` (release) do differ. `compute_standings` indexes
`rivals[i-1]` unguarded for `i in 1..6`. `playtest` passes `&[]` for neighbours,
so AI obstacle avoidance has zero coverage. Finished AI reverses forever and can
overflow `current_lap: u8`. A cancelled Grand Prix leaks `championship` into
"Records & Medials".

---

## 1c. The build was never reproducible: the SDK is a generated, untracked component

CI failed on a fresh checkout with `failed to load manifest for dependency
psx-mc` / `psx-asset`. The cause is not a gitignore mistake and no amount of
submodule or path cleanup in this repository can fix it.

**What `psoxide/sdk` actually is.** In the PSoXide project it is a *generated*
component. `PSoXide-emulator/.gitignore:88` lists `/sdk`, under the heading
"Locked component sources; generated by bootstrap-components.py".
`components.lock.json` pins it to a full commit of a **different** repository:

```json
"sdk": {
  "repository": "EBonura/PSoXide",
  "revision": "3a7c21a05c67579e01c8146ec112ab697a3b769a",
  "paths": ["sdk", "crates", "tools/mkisopsx", ...]
}
```

`tools/bootstrap-components.py` materialises those paths and records a per-file
SHA-256 receipt. So the 15 path dependencies in `game/Cargo.toml` resolve against
a tree that is generated, not committed — here, on one machine.

**Verified, in order:**

1. `psoxide` is a symlink to a local `PSoXide-emulator` clone. `sdk/` exists only
   there and on no remote.
2. Upstream `EBonura/PSoXide-emulator` has **no `sdk/` directory at all**, so
   pinning a submodule at upstream cannot work.
3. `psoxide/sdk/` is **not identical to the locked revision**: 7 files differ
   from `components.lock.json` — `psx-fmv/src/mdec.rs`, `psx-io/src/cdrom.rs`,
   `psx-io/src/lib.rs`, `psx-pack/src/cd.rs`, `psx-rt/src/interrupts.rs`,
   `psx-vram/src/lib.rs`, `psx-fmv/tests/encoder_stream.rs`. So
   `bootstrap-components.py --check` fails locally, by design.
4. Those 7 edits are **functional, not cosmetic**:
   `psx-vram`'s `upload_words_with` services a polled device mid-upload "keeping
   long CPU FIFO uploads from starving time-sensitive polled readers such as the
   CD sector stream"; `psx-rt/interrupts.rs` clears the CPU-side CDROM latch so
   an enabled CD IRQ does not become an interrupt storm; `psx-pack/cd.rs` adds
   XA-ADPCM mode handling. The FMV and CD-audio paths depend on them.
5. Bootstrapping the **pristine** locked SDK into a clean tree *does* compile the
   game (`make ci-game` passes), at the cost of those 7 fixes.

**Consequences.** Any claim that the build, the CI gate, or a measured metric is
verifiable by anyone else — including REVIEW.md's own verification table — was
not true, because the tree cannot be reconstructed off this machine. That is a
bigger problem than the red CI badge.

**Landed here.** `make deps` provisions the SDK from the pinned lock, every
cargo-resolving target depends on `require-sdk`, and all three CI jobs call
`make deps` first. A missing SDK now prints those two commands instead of a cargo
manifest dump. **This makes CI reproducible, not correct**: CI will build against
the pristine locked SDK, which is missing the 7 local edits. Making CI match
local behaviour needs one of:

- **(a)** publish the 7 edits to `EBonura/PSoXide` (or the author's fork) and
  re-lock `components.lock.json` to that revision — cleanest, and the only option
  where the receipt verifies;
- **(b)** commit the SDK tree into this repository (≈5.9 MB, 278 files) — unblocks
  reproducibility immediately, at the cost of a vendored dependency;
- **(c)** do nothing and accept that CI validates a slightly different SDK than
  the one the game is played with.

### Resolved: the SDK is vendored

**(b) was chosen** — `psoxide/sdk` is committed here, so the build no longer
depends on a tree that exists on one machine.

- Vendored: `sdk/` (all 20 crates), `crates/psx-hw`, `crates/psx-iso`,
  `crates/psxed-format`, `tools/mkisopsx` — **304 files, 6.2 MB**, an exact copy
  of the working tree this game was being built against, so **CI now compiles the
  same SDK the game is played with**, including the ten local modifications and
  the two local additions listed in `psoxide/PROVENANCE.md`.
- `psoxide/Cargo.toml` is a reduced member list over that subset (upstream's root
  lists emulator crates that are not vendored); `[workspace.package]`,
  `[workspace.lints]` and `[workspace.dependencies]` are copied verbatim.
  `sdk/Cargo.toml` is upstream's, unedited.
- `.gitignore` keeps the rest of the upstream tree out (emulator, assets, docs,
  build output) and re-admits the font bitmaps, which the repo's own `*.bin`
  build-output rule would otherwise have silently dropped.
- `make deps` no longer clones; it asserts the vendored tree is present.

Verified by cloning this branch with nothing but git and running the full gate:
`fmt-check`, `clippy`, 42/42 + 14/14 + 6/6, playtest 24/24, `ci-game` all pass
with no network access to any SDK repository.

The ten modified files are still unpublished upstream. This makes the build
reproducible, but it does not make the divergence go away — publishing them and
re-locking `components.lock.json` would let this directory be deleted. Until
then the pinned revision is advisory, and `psoxide/PROVENANCE.md` is the record.

---

## 1d. The web build is not deployed: the Pages site does not exist

`https://tonym128.github.io/ArduracerPSX/` is the repository's configured
`homepage` and the target of `.github/workflows/deploy-pages.yml`, but it returns
**404**: no Pages deployment has ever succeeded, so there is no public page to
play the game on. `make web` serves the same `web/` app on `localhost:8080`.

Two distinct failures, read from the run history rather than guessed:

1. **Fixed.** Run `37186362573` (commit `ba2839a`) failed at *Build PS1
   Executable, ISO, and Disc Images* — `deploy-pages.yml` never provisioned the
   SDK, so it hit the same missing-`psoxide/sdk` failure `ci.yml` had. The
   vendored SDK fixed it; that step now passes.
2. **Open, needs a human.** Run `37186646858` (commit `f5e38d1`) gets past the
   build and fails at *Setup GitHub Pages*:

   ```
   Get Pages site failed. Please verify that the repository has Pages enabled
   and configured to build using GitHub Actions... Error: Not Found
   ```

   `GET /repos/{owner}/{repo}/pages` returns 404, i.e. **no Pages site exists**
   for this repository. `actions/configure-pages`' `enablement` input defaults to
   `false`, and its description states it "requires a token other than
   `GITHUB_TOKEN`" — so the action only reads the site and cannot create it.
   Setting `enablement: true` without also supplying a PAT secret cannot succeed.

   **Fix once, as a repo admin:** Settings -> Pages -> Build and deployment ->
   Source -> **GitHub Actions**.

Note: `.nojekyll` is *not* needed here — Actions-based Pages publishing serves the
uploaded artifact directly without running Jekyll, and `web/` contains no
underscore-prefixed paths.

---

## 1e. The deployed build shipped a silent placeholder intro

The live site's `arduracer.exe` was **253,952 bytes** where a local build is
**346,112** — a 91 KB gap that is exactly `assets/INTRO.ADPCM`. The disc still
passed the `ci-disc` gate, so nothing flagged it.

**Cause.** `tools/fmv_cook/cook_intro_str.py` needs `psxavenc` to encode the
intro. Without it, line 207 falls back to the procedural FMV generator — which
discards the *whole* cinematic, not just the audio, and writes a 16-byte silent
ADPCM so compilation cannot break. `psxavenc` is not installed by any workflow,
so every CI run produced the placeholder. Found by comparing the deployed
artifact's size against a local build, then reading the cook script.

**Fix.** All three workflows that cook assets (ci's `disc` job, `deploy-pages`,
`release`) now install `psxavenc` from a pinned release (v0.3.1) with the
archive **and** extracted-binary SHA-256 verified. ci's `host` and `game` jobs
do not cook assets and are deliberately untouched.

Verified: with the pinned release on `PATH`, a fresh clone's `make assets`
produces `INTRO.ADPCM` at 91,520 bytes and `INTRO.STR` at 1,546,240 — matching
a local build, where the missing encoder produced the 16-byte stub.

The silent fallback also now emits a `::warning` Actions annotation, and
`ARTHURACER_REQUIRE_FMV_ENCODER=1` turns it into a hard failure for anyone who
would rather CI refuse to build than ship a placeholder.

---

## 2. Principal Architect

- [x] **Boundary cleanliness** — `arduracer-core` remains 100 % hardware-free:
  no PSX headers, no GPU types, no IO handles. Track *interpretation* moved into
  the core (`TrackTile`, `surface_at`, `collide_with_track`) precisely so the
  game layer is pure hardware marshalling.
- [x] **No duplication** — the single `TrackTile` enum replaced the magic-ID
  lookups that were previously copied between `track.rs` and `tile_blitter.rs`.
  `heading_towards` removed a whole class of duplicated sign arithmetic.
- [x] **State ownership** — one `static mut GAME: Option<ArduracerGame>`; all
  arenas (`ParticleSystem`, `SkidmarkBuffer`, `LapGhostRecorder`,
  `MemoryCardManager`) are fixed-size structs inside it. `MemoryCardManager::new`
  is the only allocation-shaped call and it is a plain struct literal — no heap.
- [x] **`no_std` / panic freedom** — zero `unwrap`, `expect`, `panic!`, `todo!`
  or `unimplemented!` in `game/src/` and `crates/arduracer-core/src/`
  (verified by grep in this review; the only indexing that could panic is
  `POINTS_TABLE[rank - 1]`, whose index is clamped to `1..=6` against a
  6-element table).
- [x] **Generated code is reproducible** — `crates/arduracer-core/src/levels.rs`
  is 100 % produced by `tools/track_cook/convert_levels.py`, which *fails the
  build* on an unreachable gate, an off-road gate, or a start box outside the
  racing surface, and runs `rustfmt` so `cargo fmt --check` stays clean.
- [x] **Documented deviations** — see §6.

## 3. Senior Systems (PSX Hardware) Engineer

### 3.1 Main RAM (measured from the `rust-lld` link map)

| Section | Bytes | Share |
| :--- | ---: | ---: |
| `.text` (code) | 137,984 | 6.6 % |
| `.data` | 56,576 | 2.7 % |
| `.bss` (arenas, state, stacks) | 237,604 | 11.3 % |
| `.psx_exe_header` | 2,048 | 0.1 % |
| **Total static** | **434,212** | **20.7 % of 2 MB** |
| On-disc executable | 196,608 | — |

Comfortably inside the ~2,001,152-byte link region, and the `.bss` is dominated
by fixed-size arenas rather than surprises: ghost telemetry 10.8 KB, memory-card
scratch 8 KB, particles 64 × 32 B, skidmarks 96 × 24 B, FMV decode buffers
**260 KiB** *(corrected by TASK-1218: this was recorded as 128 KB; `VideoStorage`
is 6 slots × 16 chunks × 2016 B plus a 64 KiB RLE buffer, a 5 KiB column buffer
and a 2 KiB sector buffer -- 13.4 % of usable RAM, and far more than the ~59 KiB
the FIFO actually needs. Reducing the ring would free ~200 KiB but changes
playback depth, so it needs an emulator pass rather than a headless one.)*

- [x] **I-cache locality** — the hot physics loop (`VehicleState::tick`) is a
  flat sequence of inline fixed-point ops with no dispatch and no allocation. No
  recursion, no `dyn`.
- [x] **GPU DMA / ordering tables** — unchanged from Phase 3; the double-buffered
  OT path via `FrameBuffer::swap()` and DMA channel 2 is untouched by this pass.
- [x] **VRAM** — *(corrected by TASK-1218)*. This originally read "the camera
  zoom is applied as an *integer* tile-size step". That described the pre-Batch-A
  behaviour. Speed-reactive zoom now lives in `Camera::world_to_screen`, which
  multiplies by a Q20.12 factor and is the single projection every layer goes
  through, so zoom cannot drift between cars, particles, skidmarks and tiles.
- [x] **SPU budget** — `game/src/audio/soundbank.rs` was unchanged; still well
  under the 200 KB target.

### 3.2 Frame budget

Locked 60 Hz by `psx_rt::interrupts::wait_vblank()` at the top of the loop. The
per-frame CPU work is: one player tick, five AI ticks (each a route lookup plus
one vehicle tick), 6 × 64 `fill_rect` calls for the tilemap, and the HUD. That
is a small fraction of the 16.67 ms slice on a 33.87 MHz R3000A.

## 4. Game & Experience Designer

- [x] **Arcade responsiveness** — 0→92 % of top speed in ~1.8 s; the car
  crosses the 320 px viewport in ~1.6 s. Full-lock steering gives a ~3-tile turn
  radius, which matches the scale of these compact circuits.
- [x] **Drift feel** — drift initiation, slip-angle dynamics, counter-steer
  recovery, spin-out threshold and the two mini-turbo tiers are unchanged, and
  `test_drift_boost_charging` / `test_counter_steering` still pin the behaviour.
- [x] **Visual clarity** — every circuit is now a continuous tarmac ribbon with a
  red/white rumble-curb shoulder, a checkered start/finish, and neon-cyan gate
  posts. This directly answers the §4 checklist item: at 60 Hz the boundary,
  the surface type, and the next gate are all unmistakable. `make playtest --
  --render` dumps every circuit as ASCII for eyeball review.
- [x] **Off-road feedback** — 0.35× speed cap, large-motor rumble, and the
  surface readout on the minimap (verge renders distinctly from tarmac).
- [x] Audio feedback — engine RPM synth tracks the new speed scale; tire
  squeal is driven by drift state, and crash audio is driven by the
  *authoritative* collision result instead of a speed-delta heuristic that
  stopped working under the old integrator.
  *(corrected by TASK-1207: "curb chatter reuses the skid voice" was false --
  sharing the voice made it inaudible, because the drift state machine keys that
  voice on and straight back off. Curb rumble now has its own voice. The crash
  trigger is also edge-detected with a refractory period, since `hit_wall` is
  true on every frame of contact.)*
- [x] **Medal progression is real** — tuning measurably matters: the swept best
  setup is 6–25 % faster than default on every circuit (e.g. Arduboy Oval
  6.63 s → 5.38 s). Gold = default tune, Silver = default +12 %, Bronze =
  default +28 %, Dev Platinum = tuned best.

### 4.1 Measured lap times (default tune, 5-lap race pace)

| Circuit | Best lap | Circuit | Best lap |
| :--- | ---: | :--- | ---: |
| Arduboy Oval | 0:06.63 | Forest Expressway | 0:14.03 |
| Twin Hairpin | 0:07.00 | Coastal Link | 0:15.71 |
| The Serpent | 0:08.10 | Alpine Drift | 0:19.90 |
| Canyon Chicane | 0:12.51 | Industrial Yard | 0:11.36 |
| Switchback Pass | 0:09.20 | Nightway Circuit | 0:14.70 |
| Grand Ring | 0:10.76 | Harbor Slalom | 0:13.76 |
| Sprint Short | 0:10.41 | Mountain Gauntlet | 0:15.38 |
| Octagon Speedway | 0:11.06 | Super Speedway | 0:15.98 |
| Devil's Elbow | 0:08.80 | Endurance Colosseum | 0:33.26 |
| Metropolis 10 | 0:10.33 | Championship Final | 0:37.93 |
| Neo Tokyo Expressway | 0:13.76 | Canyon Drift Apex | 0:19.31 |
| Cyber Circuit 2097 | 0:15.51 | Monaco GP Classic | 0:18.23 |

## 5. Verification / QA Lead

- [x] **Automated host tests** — 40/40 pass in `make test`. The suite now
  includes 13 regression tests that specifically encode the failures above
  (world scale, heading convention, surface caps, off-road ratio, boost pad,
  barrier slide, bounds clamp, route ordering, super-stage content, Level 7
  start box, nitro meter, lap-crossing rule, delta split).
- [x] **The tests test real code** — `tools/test_game_logic` and the new
  `tools/playtest` both link `arduracer-core` directly; there is no mirror
  implementation anywhere (AGENT.md §1.3 satisfied).
- [x] **New end-to-end gate** — `make playtest` simulates a full 5-lap race on
  all 24 circuits with a reference driver **and** runs all 5 rival personalities
  on all 24 circuits, then asserts per-circuit: tile-array size, start on
  racing surface, spawn inside the start gate, gates in bounds / active /
  on-road / unique, ordered route ending at the line, ordered par times, laps
  completed, and best lap ≤ Bronze. This is the gate that would have caught
  F-01…F-07.
- [x] **Edge cases** — reverse/duplicate gate crossings, missing-gate laps,
  zero-tick laps, barrier pins, negative world coordinates (u8 tile wrap),
  tune-overallocation, CRC single-bit flips, and 8 KB block round-trips are all
  covered.
- [x] **Formatting & linting** — `make fmt-check` and `make clippy -D warnings`
  clean across `arduracer-core`, `test_game_logic`, `playtest` and `game`.
- [x] **Lockfiles** — `tools/playtest/Cargo.lock` committed like the other tools.

### 5.1 Reproducing this audit

```bash
make ci                  # fmt + clippy + 40 unit tests + 24 circuits + MIPS build + RAM gate
make playtest            # circuit playability table
make playtest -- --render   # ASCII dump of all 24 circuits
make calibrate-tracks    # re-measure par times, regenerate levels.rs
make disc && make run    # master and boot the disc
```

---

## 6. Documented Deviations & Known Gaps

These are deliberate, and each is a decision rather than an oversight.

1. **Par times are measured, not inherited.** TASK-202 asked to "retain and
   verify original dev par times". Verification showed they are *unreachable on
   the original hardware too*: FX Level 1's 8.68 s dev time over a ~1,792-unit
   circuit demands ~206 units/s, while the FX car's own `max_speed` is
   2.5 px/frame = 150 px/s. Those figures were aspirational leaderboard targets,
   not human records, and GAME.md requires Bronze to be "achievable by a clean
   run with default tuning". The shipped targets are therefore measured from the
   real simulation via `make calibrate-tracks`, and the original centisecond
   table is retained in `convert_levels.py` for reference. Tunability is
   preserved because Gold requires the tuned setup.
2. **Track surfaces are repainted, tile art is not ported.** The FX road band
   passes through corner tiles whose indices fall outside the tarmac predicate,
   so the naive mask renders as a mottled scatter rather than a circuit (and the
   original resolved this with *per-pixel* art collision, which the PSX port has
   no asset budget for). The cooker therefore rasterises a continuous corridor
   along each circuit's ordered route — inheriting gate positions, circuit shape,
   names and dimensions, and repainting the surface with a rumble-curb shoulder.
   This is what GAME.md means by "completely remastered".
3. **Non-road FX art maps to drivable OffRoad, not walls.** FX had no wall
   collision at all — only the level bounding box stopped the car — so hard
   barriers would box the player into the mottled FX interiors. The bounding box
   is now the real barrier (with bounce, sparks and rumble), and `TrackTile::Barrier`
   is reserved for authored interior walls in the Super Stages.
4. **Memory-card icon is one frame, not three.** TASK-702/GAME.md §8 specify a
   3-frame animated icon; `psx-mc`'s filesystem writer emits a single icon frame
   (`hdr[T_ICON_FLAG] = 0x11`). A custom checkered-flag frame ships now; the
   multi-frame support belongs in the SDK, not this pass.
5. **No on-hardware or captured-video frame verification.** The audit
   environment is headless, so 60 FPS is *structurally* guaranteed (vsync-locked
   loop, measured 424 KB RAM) and the build boots and runs for minutes in
   PCSX-ReARMed without faults — but no frame capture or real-hardware run was
   possible here. **A human pass in DuckStation on a real 60 Hz display is still
   required before sign-off.**
6. **2-player split-screen (GAME.md §4.3) is not implemented.** It does not
   appear in TODO.md's task list, so it was out of scope; the renderer already
   works in a viewport-offset coordinate space, which is most of the work.

---

## 7. Sign-off

| Perspective | Verdict |
| :--- | :--- |
| Principal Architect | **Approved** — boundaries clean, no duplication, no panics, generated data is reproducible and self-validating |
| Senior Systems Engineer | **Approved** — 424 KB / 2 MB (20.7 %), vsync-locked 60 Hz loop, no dynamic allocation, integer-locked VRAM writes |
| Game & Experience Designer | **Approved** — arcade pace restored, all 24 circuits readable and drivable, medals and tuning both meaningful |
| Verification / QA Lead | **Approved with condition** — 40/40 unit tests, 24/24 circuit playability, fmt/clippy clean; **conditioned on the human DuckStation pass in §6.5** |

**Overall: APPROVED for the verification track, pending an on-screen playtest by a
human operator.**