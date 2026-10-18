# Track It — Arduracer PSX Upgrade Roadmap

**Goal:** Make Arduracer PSX an outstanding arcade time-attack racer with TrackMania's fast retry loop, readable tracks, precise timing, expressive driving, and a strong PS1 identity.

This is a living, ordered backlog for work in the `trackit` worktree. Recommendations are grounded in the current project tree and its October 2026 audit; they are not a claim that every advertised feature is finished. We will take items one at a time, verify each against PS1 constraints, and update this file as decisions and implementation results land.

## Product priorities

1. **Trust:** laps, medals, splits, standings, saves, and replays report the truth.
2. **Flow:** restart and checkpoint recovery are immediate; retries keep the player racing.
3. **Feel:** steering, grip, drift, collisions, surfaces, and feedback are learnable and rewarding at speed.
4. **Readability:** the next gate, route, hazard, speed, and pace are clear without stopping to parse the screen.
5. **Identity:** memorable circuits, cars, sound, music, presentation, and progression.
6. **Hardware discipline:** target stable 60 Hz NTSC / 50 Hz PAL and measure PS1 GPU/SPU/VRAM/RAM/CD budgets.

## Phase 0 — Make the current game trustworthy (do first)

These issues can invalidate lap results or make the game appear broken; resolve them before adding modes or polish.

- [x] **T-001 Fixed-point correctness.** Fix `Fixed::mul`/`div` overflow and truncation, define divide-by-zero behavior, and replace `Vec2::length()` with a safe wide-integer implementation. Cover boundaries in the real core crate.
- [x] **T-002 Valid lap rules.** Reject implausibly short recorded laps, require every checkpoint on each attempt, reset progress after every start-line departure, and reject clearly reverse checkpoint/finish travel. Checkpoints intentionally remain order-independent: strict ordering previously made 23 of 24 original circuits impossible to complete; preserve that compatibility unless a separately validated route-order system replaces it.
- [x] **T-003 Reachable medal targets.** Re-measure medal pars against achievable laps after physics changes. Make par drift fail the normal playability gate; recalibrate intentionally.
- [x] **T-004 Recoverable collisions.** Prevent permanent spin-outs, preserve useful player control during recovery, correct position after interior-wall hits, and make all track edges escapable without manual reset.
- [x] **T-005 Correct race positions.** Rank player and rivals with one progress metric (lap, ordered gate, route distance); handle start/finish and ties consistently.
- [x] **T-006 Credible rival pace.** Give AI route-aware lines and surface awareness; prevent shortcuts, impossible speeds, and laps faster than track length allows. Make personalities distinct but fair.
- [x] **T-007 Reliable state transitions.** Fix held-button edge baselines at race/screen entry so confirm cannot immediately pause or skip; make pause/restart/quit deterministic.
- [x] **T-008 Results and progression integrity.** Show cumulative race time, preserve cup progress through podiums, make Records open records, and never silently discard invalid tuning/save data.
- [x] **T-009 Frame pacing and command safety.** Synchronize drawing and VBlank-aware buffer swaps. Prevent scanout of an actively drawn buffer and dropped GPU FIFO writes.
- [ ] **T-010 Measure frame time.** Profile representative busy scenes and every track at target video modes; record worst-case frame budget, not just average speed.
- [ ] **T-011 Test production logic.** Add meaningful tests beside `arduracer-core` logic or directly exercise production code. Prevent CI success with zero tests and improve failure reports.
- [ ] **T-012 Honest quality gates.** Lint the PSX game target, gate par drift, and align documented verification with actual CI.

## Phase 1 — TrackMania retry and time-attack loop

- [ ] **T-013 Instant restart.** Reset car, clock, splits, ghost comparison, and countdown in one input/frame without reloading the track from CD.
- [ ] **T-014 Checkpoint respawn.** Respawn at the last cleared in-order checkpoint with a useful heading and defined penalty/rule; never choose a spatially nearby point on the wrong side of a hairpin.
- [ ] **T-015 Exact split timing.** Store actual checkpoint crossing times and compare them to PB/author ghosts; remove estimated linear deltas.
- [ ] **T-016 Split feedback.** Show gate, delta, gained/lost time, and next target with a brief color/chime cue that stays legible and does not obscure the road.
- [ ] **T-017 Countdown and launch.** Make starts repeatable and start the clock at the actual start; decide whether rev/launch skill fits the arcade model.
- [ ] **T-018 Medal pace display.** Show next-medal target and current pace in time trial; clearly explain the award at finish.
- [ ] **T-019 Ghosts that teach.** Persist a compact PB ghost, compare at splits, and render it distinct/translucent without blocking the player. Add author ghosts only with fair, reproducible lines.
- [ ] **T-020 Records and replay.** Provide PB, medal, best splits, ghost availability, and deterministic replay playback. Add camera choices after playback is reliable.
- [ ] **T-021 Failure clarity.** Explain invalid laps (order, direction, respawn rule) immediately and make retry obvious.

## Phase 2 — Driving, controls, and AI

- [ ] **T-022 Physics design pass.** Set handling targets and tune acceleration, braking, top speed, grip, lateral slip, drift recovery, collisions, and boost as one model. Compare oval, hairpin, chicane, and mixed-surface laps.
- [ ] **T-023 Tuning that matters.** Make each garage slider measurably affect the car, clamp UI values to core ranges, and verify presets/slots save and reload exactly.
- [ ] **T-024 Control setup.** Expose steering/throttle profiles, calibration/deadzone, remapping where hardware allows, and controller status. Resolve button conflicts and analog re-arming.
- [ ] **T-025 Haptics.** Wire and verify DualShock motors end to end (including stop on pause/menu/disconnect), or remove unsupported rumble claims. Make intensity speed-aware and configurable.
- [ ] **T-026 Surface identity.** Make road, curb, grass, gravel, sand, oil, boost, wall, bridge, and tunnel meaningfully distinct in handling and feedback; only advertise surfaces runtime truly supports.
- [ ] **T-027 Drafting and car contact.** Prototype slipstream/contact only if it improves fair racing. Bound bonuses and test jams and edge cases.
- [ ] **T-028 AI racecraft.** Add route following, corner speed, passing, recovery, and personality tuning. Benchmark medal pace and remove hidden speed advantages.
- [ ] **T-029 Onboarding.** Teach braking, drift, grip, checkpoints, and restart/respawn through concise lessons/tips; provide approachable first tracks and mastery targets.

## Phase 3 — Track quality and content

- [ ] **T-030 Track audit.** Review every course for route readability, corner variety, safe margins, gate placement, shortcuts, difficulty curve, and visual identity. Remove filler/duplicates.
- [ ] **T-031 Signature moments.** Give courses memorable, fair features: blind crest, slalom, hairpin, speed section, crossover, surface change, or scenic set piece.
- [ ] **T-032 Barriers and dressing.** Populate collision geometry and roadside props intentionally; make art and collision agree; telegraph hazards and avoid invisible trap walls.
- [ ] **T-033 Authoring pipeline.** Keep editable source, validation, generated assets, and runtime data synchronized. Reject invalid starts, gates, and route connections.
- [ ] **T-034 Custom tracks.** Improve the editor: place/rotate pieces, instant test, reachability validation, compact save/share, and clear limits. Prefer a usable desktop/browser builder if in-game editing is too constrained.
- [ ] **T-035 Distinct modes.** Make Mania/time attack differ from Grand Prix in lap format, medals, restart, and progression. Add sprint/platform/stunt/knockout only when tracks support them well.
- [ ] **T-036 Progression and rewards.** Unlock cups clearly, award medals/records visibly, allow course replay, and make results lead naturally to retry/next race.

## Phase 4 — Visuals and rendering

- [ ] **T-037 Fix visual defects.** Correct degenerate canopy geometry and particle/skid quad winding; render effects where visible and sort overlapping cars/effects by depth.
- [ ] **T-038 Stable renderer.** Use measured buffer synchronization and an ordering strategy suited to PS1. Verify semi-transparency/blending on target before relying on it.
- [ ] **T-039 Camera readability.** Tune follow distance, look-ahead, speed zoom, bounds, shake/roll, and PAL/NTSC behavior. Keep gates/apexes visible without discomfort.
- [ ] **T-040 Road legibility.** Improve contrast between road, runoff, curbs, hazards, gates, and background after PS1 15-bit conversion; check palettes in motion.
- [ ] **T-041 Car identity.** Give cars distinct silhouettes/palettes; animate steering/braking cues where economical; keep the player easy to track.
- [ ] **T-042 Trackside art.** Add reusable era-appropriate props: gantry, gate markers, grandstands, signs, trees/rocks, tunnel portals, bridges. Prefer strong silhouettes over clutter.
- [ ] **T-043 Useful effects.** Make smoke, skid marks, sparks, boost, dust, and collision flashes visible and performant; communicate grip and speed.
- [ ] **T-044 HUD/minimap budget.** Cache static map geometry or avoid rebuilding it each frame. Prioritize position, lap, timer, split, next gate, speed/boost, and race state.
- [ ] **T-045 Texture quality.** Compare codecs/palettes in motion; prevent streaming stutter or audible CD-DA interruption; test fallbacks.
- [ ] **T-046 Transitions/loading.** Add brief consistent transitions and useful track previews without masking stalls or breaking audio/frame state.

## Phase 5 — Sound, music, and video

- [ ] **T-047 Audio state correctness.** Stop/pause voices and music on screen changes; edge-trigger crashes; correctly start curb/skid sounds; audit voice key-on/off.
- [ ] **T-048 Driving mix.** Tune engine to speed/RPM; add distinct tire/surface/collision/checkpoint/boost cues; keep effects intelligible beneath music.
- [ ] **T-049 Music presentation.** Audit CD-DA/XA selection, loops, transitions, and pause behavior. Expose volume settings that affect output.
- [ ] **T-050 Sound asset quality.** Replace placeholder/test tones with authored or properly licensed arcade masters; cook to PS1-friendly ADPCM and document provenance.
- [ ] **T-051 Spatial mix.** Use stereo pan/reverb only where APIs/hardware support it; test voice limits and speaker/headphone mix.
- [ ] **T-052 Video/attract mode.** Audit intro/victory content, interleaved audio, and sync. Add attract mode only if it returns cleanly to title and input/audio state.
- [ ] **T-053 Announcer.** Consider concise optional countdown/checkpoint/record calls after the soundscape is stable; avoid repetitive chatter.

## Phase 6 — Saves, menus, accessibility, and polish

- [ ] **T-054 Save/settings integrity.** Expose persisted settings, validate ranges, report card absent/full/write errors, and protect records from accidental overwrite/corruption.
- [ ] **T-055 Complete menus.** Implement functional Records, Settings, Controls, race setup, and results. Every selectable label must work as promised.
- [ ] **T-056 Readability/accessibility.** Provide color-safe pace cues, separate music/SFX levels, alternate controls where feasible, and text readable at CRT/240p distance.
- [ ] **T-057 Two-player scope.** Assess split-screen against frame/GPU budgets and controller availability; defer unless it can remain smooth and readable.
- [ ] **T-058 Packaging/docs.** Keep manual/web feature claims accurate; document controls/modes and reproducible build/disc instructions.

## Phase 7 — Measured excellence

- [ ] **T-059 Human track playtest.** Combine simulation with human sessions. Record confusion, unfair hazards, retries, medal completion, and controller feel.
- [ ] **T-060 Hardware matrix.** Verify NTSC/PAL, real PS1 or validated emulator behavior, RAM/VRAM/SPU budgets, CD streaming, audio continuity, and worst-case render scenes.
- [ ] **T-061 Release pass.** Fix crashes, softlocks, false records, broken menus, missing assets, and major audio/visual defects; smoke-test final disc and web builds.

## Suggested execution order

1. T-001–T-012: trustworthy simulation, scoring, rendering pace, verification.
2. T-013–T-021: restart/respawn, exact splits, medals, records, ghosts.
3. T-022–T-029: handling, controls, surfaces, fair AI.
4. T-030–T-036: tracks and distinct progression/modes.
5. T-037–T-053: graphics, assets, sound, music, video.
6. T-054–T-061: usability, scope choices, release validation.

## Working rules

- Work one numbered item at a time in `trackit`; split only when implementation requires it.
- Before coding, inspect the current branch and reproduce/confirm the finding. Audit notes may have been fixed elsewhere or may need fresh evidence.
- Prefer small reviewable changes with a player-facing outcome and measurable acceptance criteria.
- Keep production logic in the real shared core; do not test a copied implementation.
- Run tests/builds when requested for an item or needed to verify an agreed change; record exactly what ran and the result.
- Update this file as work completes; add material findings discovered during inspection.
- Preserve stable 60 Hz NTSC / 50 Hz PAL behavior and explicit PS1 RAM, VRAM, SPU, CD, and GPU budgets.

## Initial evidence

- `AUDIT-2026-10.md`: findings on arithmetic, lap validation, AI standings, spin-out recovery, frame sync, minimap cost, effects/audio, menus, and test coverage.
- `NO1.md`: broad prior TrackMania-style feature blueprint. Its checked boxes are historical status, not proof those items exist in current `main`.
- `GAME.md`: product direction and intended feature claims; keep claims aligned with implementation.

## Progress log

- **T-001 complete (already present on branch base):** `math.rs` uses saturating wide intermediates for fixed multiplication/division and a 64-bit integer square root for vector length. `cargo test --manifest-path crates/arduracer-core/Cargo.toml` passed (298 core tests, including arithmetic and distance edge cases). No code change was needed for this item.
- **T-002 complete (already present on branch base):** the production timer requires all gates within one lap attempt, clears progress at every start-line departure, applies a track-derived plausible-time floor to records, and checks route-relative travel at gates/finish. Checkpoint order remains intentionally flexible for shipped-track compatibility. The all-track playtest completed three laps on every listed course with five rivals; it exited nonzero only because 23 par tables differ slightly from the current calibration (tracked under T-003).
- **T-003 complete:** recalibrated all 40 authored circuits in `tools/track_cook/par_calibration.json` and regenerated `levels.rs`/disc level data with `make atlas-and-calibrate` (the initial atlas subprocess needed escalation in this environment). `make verify-levels` now passes all courses, reference laps, five AI rivals, and par checks. Differences were small, up to 10 ticks, but CI correctly treated any drift as failure.
- **T-004 complete (already present on branch base):** wall impacts resolve velocity against a normalized normal, glancing contacts scrape instead of spin, hard impacts have a bounded spin-out cooldown, spin-outs scrub lateral velocity and retain reverse/control, and collision correction covers interior and outer boundaries. The core suite passed recovery regressions including `a_wall_hit_is_always_recoverable_without_respawning`, `interior_walls_do_not_trap_the_car_in_a_push_out_loop`, and `collide_with_track_is_total_for_a_position_far_off_the_grid`.
- **T-005 complete (already present on branch base):** championship standings compare finish state, completed laps, and distance to the actual next route node, including start/finish. Rival-count edge cases and ties are bounded and deterministic; the core suite includes start-grid, route-distance, lap-priority, and start/finish regressions.
- **T-006 complete (already present on branch base):** rivals use cached route-aware racing lines, bounded corner/surface speeds, and explicit stuck recovery. `make verify-levels` passed all 40 authored circuits with all five rival profiles completing the required races; the core has full-circuit line and recovery regressions.
- **T-007 complete (already present on branch base):** menu screens arm confirm edges on entry, race loading arms pause input against a held Start, and restart synchronizes the held-button baseline. `cargo test --manifest-path tools/test_ui/Cargo.toml` passed all 226 UI tests, including held-confirm/entry and pause edge regressions.
- **T-008 complete:** added `LapTimer::total_race_ticks`, which advances only while the race is live and resets when a new race starts. Single-player and two-player results now receive this cumulative value instead of the final lap's resettable counter. Core regression confirms two-lap accumulation and reset; all 299 core tests passed, and `make exe` built the PSX executable. Cup points/progression, Records routing, and tuning validation were already implemented on the base branch.
- **T-009 complete (already present on branch base):** the frame loop arms a draw-done marker, waits for the queued display flip to be applied at VBlank before selecting the next draw target, and closes every visible/early-exit frame before queuing its flip. FrameBuffer GP0 writes wait for command readiness. UI tests cover frame closure on pause and other early exits; the current PSX build passed under `make exe`.
