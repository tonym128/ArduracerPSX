> **Superseded in part.** The level authoring model changed after this document
> was written: circuits are now **colour-coded images**, not tile grids or
> parametric shapes. See [`LEVEL-FORMAT.md`](LEVEL-FORMAT.md) for the decision,
> the palette, and the memory constraint that forces coarse-physics/author-fine.
> Batch A below is kept for its validation work, which survives; its superellipse
> circuit authoring does not.

# STAGE ATLAS — authored circuits, streaming scenery, and props

Programme plan for making the 24 circuits look and play like real stages rather
than smoothed Arduboy artwork. Follows `OVERHAUL.md`; this document covers the
work that is *new* rather than the ten steps that were already scoped.

Ordered. Each batch has a done-condition that can be checked headlessly, because
"looks good" is not one.

---

## What is wrong today

The 24 circuits are not authored. `tools/track_cook/convert_levels.py` reads
`ArduRacerFx/Levels/Level1..20.csv` and fits a spline through whatever tile
clusters the FX renderer happened to leave as gates. Four circuits
(`SUPER_STAGES`) *are* authored, and they are visibly better: wider, longer,
fewer blocky corners.

Three consequences follow from deriving geometry rather than authoring it:

- **Roads are narrow.** `CORRIDOR_HALF_WIDTH = 1.05` tiles is a two-tile-wide
  band. Anything wider was not available, not unwanted.
- **Corners are short.** Control points sit where FX gates were, which are placed
  for *gate coverage*, not for a driving line. Straights between corners are
  therefore whatever gap the gate spacing left over.
- **Nothing is streamed.** The whole grid is one flat `&'static [TrackTile]`,
  baked into `levels.rs`. A large atlas cannot be expressed, and 24 circuits'
  worth of tiles is resident whether or not it is on screen.

## Hard ceilings this work has to lift first

| Limit | Now | Needed | Why |
| :--- | ---: | ---: | :--- |
| `MAX_TRACK_DIM` | 40 | 64 | A 64x64 grid is the smallest that reads as an "atlas" rather than a circuit. 64 tiles = 4096 world units per side. |
| `MAX_ROUTE_SAMPLES` | 192 | 288 | 8 samples per span caps authored centrelines at 24 control points. Smooth corners on a large circuit need more. Costs ~1 KB per `Route`; there are 6 live (player + 5 rivals). |
| `MAX_TRACK_CHECKPOINTS` | 16 | 24 | Longer circuits need proportionally more split points to keep a straight from being one long blind gate. |
| Arc accumulator | `u16` world units | `u32` | Verified safe to a 96x96 grid (24,576 units vs the 65,535 ceiling), but the margin is only 2.6x and a future grid change would saturate silently. `u16` saturating makes every gate collapse onto one arc, which fails every lap with no error. |

`MAX_TILE_INDEX` is derived from `MAX_TRACK_DIM` and must move with it.

---

## Batch A — author all 24 circuits

Replace the FX-derived geometry with authored data for every circuit, not just
the four super stages.

**New file `tools/track_cook/circuits.py`**, holding 24 entries:

```python
CIRCUITS = [
    {
        "name": "Sunset Ridgeway",
        "grid": (64, 48),
        # Control points in tile coordinates, ordered along the racing line.
        # Denser where the corner is tight; sparse along straights.
        "centre": [(8, 10), (24, 8), (40, 12), (52, 22), (46, 34), (28, 38), (12, 32), (5, 20)],
        "half_width": 2.4,            # wide: ~4.8 tiles of tarmac
        "checkpoints": 10,
        "boost_at": (0.30, 0.72),
        "oil_at": (),
        "trait": "mixed",
    },
    ...
]
```

- `half_width` is the road half-width in tiles. `2.0`-`3.0` is "wide" at this
  tile size, versus the current 1.05.
- `centre` control points are spaced by *intent*: 12-20 tiles along a straight,
  4-8 through a corner. That is what produces long straights and smooth corners
  from the same spline.
- `trait` tags a circuit so Batch E can verify scenery and hazards land where
  they should: `boost`, `drift`, `technical`, `speed`.

`convert_levels.py` consumes `circuits.py` and stops reading
`ArduRacerFx/Levels/*.csv` for geometry. The directory stays in the repository
as provenance (see `tools/track_cook/PROVENANCE.md`).

**Done when:** all 24 circuits are authored, `test_centreline_stays_on_the_road`
passes on all of them, all 24 are playable 5 player + 5 AI laps, and the par table
is recalibrated.

**Status: closed.** All 24 are authored and validated, `make ci` is green, and the
measured geometry is real: road half-widths are 2.2-3.0 tiles (140-192 px)
against the old derived 1.05 (67 px), grids are 40x40 to 56x43 before padding
against the old 18x18-38x38, and laps are ~12,300 world units against ~6,600.
All 24 are playable 5 player + 5 AI laps with a recalibrated par table.

### The five stranded circuits: diagnosed, and the diagnosis was not what it looked like

The symptom was five circuits (Willow Bend, Chrome Basin, Longshadow Flats,
Harbourmaster, Aurora Vault) where `AiRacer` never finished -- 400,000 ticks,
`current_lap` stuck at 1, one checkpoint ever cleared. Three things turned out to
be wrong with the obvious reading:

1. **"Peak speed never rose above 3" was a units mistake.** `speed.to_int()` on a
   Q20.12 value of ~12,400 raw is 3 *world units per tick*, which is essentially
   top speed (3.42). The cars were not stalled at all; they were driving at full
   pace.
2. **The gates were not being missed.** At 1x1 a gate looks far too small for a
   2.8-tile road, and the car does pass wide of a gate centre. But
   `every_rival_finishes_every_circuit_within_the_playtest_budget` passes on all
   24 with 1x1 gates, so in normal play the AI does hit them. The misses only
   occurred in the *corrupted-index* scenario below.
3. **Square gates spanning the corridor make it worse, not better.** Tried as a
   stopgap: a `2 * half_width` square centred on each gate. It trades one failure
   for three real regressions -- the square's corners land in the runoff so
   "route node sits on-road" fails, and both the start/finish scoring and the par
   floor break. A square is the wrong primitive; a gate defined as a *segment* on
   the centreline cannot have off-road corners, which is precisely why step 3
   wants `arc_pos`/`half_width` rather than a tile rectangle. Reverted.

What it actually is: the failing test deliberately sets `target_gate_idx =
u8::MAX`, which reduces modulo `route_len()` and lands the rival's target
*part-way round the circuit*. The rival then steers across the infield, leaves
the tarmac, and never recovers -- because `OffRoad` does not count as "wedged",
so the reverse-and-respawn path never triggers. Measured: on **19 of 24 circuits
the corruption costs nothing** (tick counts identical to an uncorrupted run); on
those five it never recovers.

So the test was asserting something stronger than it claimed and only passed
before by accident of circuit size. It now asserts the property it is actually
named for -- a corrupt index cannot escape the route, checked through behaviour
(no panic, rival stays inside the grid, index reduction does not overflow) --
and the real AI gap is filed as **TASK-1410**.

## Batch B — steps 3 and 4, now that geometry is authored

These are `OVERHAUL.md` steps 3 and 4. They were blocked because the centreline
came from FX gates; that is resolved by Batch A and by the merged
`fix/owned-levels-and-route` basis fix, so they can proceed.

- **Step 3:** `CheckpointGate { arc_pos: u32, half_width: u16, heading: u16 }`,
  centreline-relative rather than tile-relative.
- **Step 4:** lap validation as signed arc crossing in ascending order inside a
  direction cone, replacing tile-coverage scoring with no direction check.

**Done when:** a forward clean lap validates, a reversed lap does not, a lap with
a skipped gate does not — all asserted as tests, on all 24 circuits.

## Batch C — step 5, render the road as a ribbon

This is the batch that actually makes it *look* right. Tiles at 64 units will
always stair-step; nothing about authoring fixes that.

Build a triangle strip along the centreline at +/- `half_width`, banded into
tarmac / curb / shoulder, and demote the tile grid to a surface lookup for
physics and AI only. ~200 quads per circuit against up to 4,096 tile primitives.

**Done when:** no stair-stepping on any corner at gameplay zoom, and the
per-frame primitive count is independent of grid size.

## Batch D — large atlas with streaming

Split the grid into regions and stream them from CD rather than baking the whole
atlas into `levels.rs`.

**Done when:** a circuit's resident tile memory is bounded by the streaming
window, not by grid size, and a circuit larger than `MAX_TRACK_DIM` can be
expressed as several windows.

## Batch E — 3D scenery and props

Tunnels, bridges, palm trees, rocks, fences, spectators. Each needs a prop
system with depth sorting against the ribbon; PSX has no hardware z-buffer in the
path currently used, so painter's order from the projected `y` is the mechanism
(matching what the vehicles already do).

**Done when:** props occlude correctly against cars and the ribbon, and each
circuit's `trait` is visibly exercised by its scenery.

## Later — damage and collisions, then personas

Per request, after the above:

- **Damage and collisions** — a damage model, panel deformation or a simpler
  speed/impact-driven performance loss, and collision consequences beyond a
  bounce. This is the first change that touches `VehicleState`'s invariants, so it
  wants its own branch and a calibration pass.
- **Character personas and in-race chatter** — driver personalities affecting AI
  lines and aggression, plus line calls. Needs a text system wide enough for
  speech bubbles and an audio path for voice lines.

---

## Verification

Every batch ends with `make ci` green plus `make calibrate-tracks`, because the
par table is derived from measured laps and moves with geometry and physics. The
non-negotiable gates are the existing ones:

- `test_centreline_stays_on_the_road` on all 24 circuits
- `test_runtime_curve_matches_float_reference`
- the wall tests, which are non-vacuous by construction since the fix in
  `309a64d` (a wall must exist, the car must reach it, it must spin)
- all 24 circuits playable, 5 player + 5 AI laps
- RAM budget

Add per batch:

- **A:** road half-width and straight-length bounds as assertions, so "wide" and
  "long" are checked rather than asserted.
- **B:** forward/reversed/skipped-gate lap tests.
- **C:** ribbon primitive count independent of grid size.
- **D:** resident tile memory bounded by the window.
- **E:** prop depth ordering.


---

## Image pipeline: state of play

**Built:** `generate_circuit_images.py` (24 circuits, two images each, 768x768
visual / 144x144 data at 48x48 cells), `compile_circuit.py` (validation +
`surface.bin`/`texture.bin`/`manifest.json`), `build_atlas.py` (runs validation
over all 24 and emits `levels.rs`, refusing to emit if any circuit fails).

All 24 circuits generate, validate and compile. `levels.rs` is built entirely from
the colour images; the old parametric cooker is gone.

**Not yet playtestable.** `tools/test_game_logic` is 45/49. Four failures, all
diagnosed to a known cause but not fixed:

1. **`Hairpin Ridge`: centreline sample 1-2 sits on `Barrier`.** This is at the
   *start line*, not spline overshoot. `wall_the_edge` stamps `WALL` on every
   non-road cell adjacent to road, and the start lands close enough to the road's
   outer edge that the centreline clips it. The hairpins compound it: a 2.6-cell
   radius is barely wider than the 2.5-cell road, so the inside of the turn has
   no room.
2. **`The Esses` / `Right Angles`: arc wraps twice in a lap, and hinted vs full
   `nearest` scans disagree.** The centreline passes close to itself, so `arc`
   is ambiguous at the crossing. This is the same self-approach that
   `TASK-1410` (AI off-road recovery) also runs into.
3. **`Track 4` route node 4 sits off-road** -- same root cause as 1.
4. **Par times are placeholders.** Not calibrated; the atlas is new geometry.

**Also unresolved:** the AI does not complete laps on the tighter circuits
(`Right Angles`: stuck 30,264 ticks at gate 6). Not yet established whether that
is the geometry, the centreline, or the gate positions.

### The two lessons worth keeping

* **Tile corners, not centres.** The first emitter wrote `cx * TILE_SIZE`, which
  is half a tile off the road. On a 2.5-cell road that is enough to land on a
  barrier. It must be `cx * TILE_SIZE + TILE_SIZE // 2`.
* **Decimate the centreline to the route reservoir, but aim near the ceiling.**
  Targeting 480 samples (of 768) let the spline cut straight across the
  hairpins. Targeting ~700 fixed nothing on its own -- which is how the start-line
  problem above became visible instead of being masked by overshoot elsewhere.

### Root cause of all of it: the circuits do not close

Measured across all 24: **0 of 24 are closed loops.** The seam gap between the
last and first control point is 1,775 to 2,983 world units -- the walker ends
where it started *only by accident of the grid*, and the gap is larger than most
of the circuit's radius.

The centreline is fitted as a **closed** Catmull-Rom, so it must connect the last
point back to the first. With a seam that large, the spline has to jump it, and
everything downstream follows from that one fact:

| Symptom | Why |
| :--- | :--- |
| arc wraps twice per lap | the seam jump is read as forward progress, then re-wrapped |
| hinted vs full `nearest()` disagree | two places on the route are near-coincident across the seam |
| coincident centreline samples | the spline passes through the same point twice while jumping it |
| centreline on `Barrier` at sample 1-2 | the tangent at control point 0 is `(next - prev)/2` with `prev` on the far side of the circuit, so it points off the track |
| AI stuck for 30k ticks | it is trying to drive a jump that is not there |

Sample 1-2 of `Hairpin Ridge` is at y=2,400 on a straight whose control points run
x = 288, 352, 416, 544. The runtime spline puts sample 2 at x=128 -- *behind* the
first control point, off the end of the straight. That is the seam, not
overshoot, and no amount of resampling or radius tuning will move it.

### The fix

The circuit *descriptions* have to form closed loops. Two conditions:

1. **The turn must close**: signed corner angles sum to +/-360.
2. **The displacement must close**: the straights and arcs must return to the
   start, which is a second, independent constraint.

Condition 1 alone is not enough, and assuming it was is what produced 24 open
spirals. The reliable construction is a **closed polygon of corner vertices,
filleted** -- a closed polygon cannot fail to close, and the edges become straights
and the vertices become corners. That is what the earlier superellipse work threw
away for being too uniform; the answer is not "a smooth analytic curve" but "an
explicit closed outline".

`generate_circuit_images.py` should therefore validate, at generation time:
turn sum +/-360 *and* seam gap below one control-point step. A circuit that fails
must not produce an image, because a broken loop is invisible in the picture and
fatal in the physics.

### Next

1. Re-author the 24 circuits as closed, filleted polygons. The images do not need
   regenerating for this -- only the walk does.
2. Add the two closure assertions to the generator so a non-closing circuit fails
   loudly instead of quietly producing a broken map.
3. Recalibrate par, then `make playtest`.

---

# HANDOVER — 2026-10-05

Everything above this line is history. The closure fix described in "Root cause
of all of it" is **done**: all 24 circuits are closed, filleted polygons and
`assert_closed()` gates them at generation time. This section is the current
state and the work that remains.

Branch: `feat/atlas-and-authored-circuits` in worktree `.worktrees/wt-atlas`.
`main` is untouched. Tip: `dcfa17c`.

| Commit | What |
| :--- | :--- |
| `a56ef9d` | Circuits authored as closed polygons; closure asserted at generation |
| `184fa5f` | Fixed a double-applied fit transform in `render()` |
| `1dda059` | Longer straights; stopped repeating the route's first control point |
| `432d4d4` | Replaced all ten serpentines with closed rings |
| `5a7e9d5` | Restored Longbow's fillet radius to 13.0 |
| `dcfa17c` | Recalibrated par from measured reference laps |

## Current test state

- **Core: 155 passed, 3 failed.**
- **Game logic: 46/49 — STALE.** Last measured *before* the serpentine
  replacement. Re-run it before trusting any number below.

Failing core tests:
- `ai::tests::every_rival_drives_a_real_racing_line_on_every_circuit` — Longbow
- `ai::tests::every_rival_finishes_every_circuit_within_the_playtest_budget`
- `ai::tests::the_look_ahead_point_lies_on_a_drivable_tile`

## Do these three things, in this order

### 1. Root-cause Longbow. Do not retune shapes until you have.

This is the blocker for everything else. Until it is understood, every geometry
change moves failures around for reasons nobody can see.

The facts, all verified:

- `HEAD~1` circuits -> Longbow **passes**. Current circuits -> Longbow **fails**.
- The generator line is **byte-identical** in both:
  `_ring(4, 36, 24, 0.0, 4.2), 13.0`
- The emitted `TrackDef`'s start position, heading, half-width and route point
  count **all match** between the two.
- The failure is **deterministic** across three runs.
- It fails when the test runs **in isolation**, and `drive_one_lap` builds a
  fresh `AiRacer` per call, so there is no shared state to blame.

So Longbow's own definition and its own emitted data are the same in both cases
and the result still differs. Something outside Longbow is responsible.

**Start here:** diff the whole of `crates/arduracer-core/src/levels.rs` between
`HEAD~1` and now, looking at circuit **ordering** and at any **shared/static
arena or packing**. A neighbour's data changing Longbow's behaviour has the exact
signature of a packing bug.

**Caveat on my own evidence:** my block-extraction diff grabbed ~1 MB instead of
Longbow's block, so its regex for the next circuit boundary is wrong. The values
it printed are consistent, but "identical" there is weaker than it sounds.
Re-derive it properly rather than trusting that diff.

### 2. Wire par times into the atlas pipeline

**`build_atlas.py` never reads `par_calibration.json`** — not one reference. The
numbers `dcfa17c` measured cannot reach `levels.rs` through the atlas pipeline,
so the emitted par times are stale or absent. This is a known missing link, not
a mystery, and a strong candidate for the failing AI lap-budget test.

Four circuits reported **no measurement** during calibration — Foundry Spiral,
Rattlesnake Pass, Cinder Bowl, Switchback. The written JSON has plausible values
for all 24 (nothing below 100), so the `->1` in the log looks like a display
artifact, but that was **not confirmed**. Check those four before trusting them.

### 3. Then the remaining gates

- Game logic 46/49: route node 4 off-road, an arc wrapping twice per lap, and
  the longest-straight floor (24 samples, circuit reported 17).
- Re-derive the straight/width/lap floors — the user has approved treating them
  as calibration artifacts of the deleted FX circuits, not design intent.
- Measure static headroom against the 999,424-byte budget (old circuits used
  678,436).
- `make playtest`, then `fmt-check`, `clippy`, `test`, `ci`.

## TRAP: `make calibrate-tracks` destroys this branch

Do not run the target as written. Its second step runs
`tools/track_cook/convert_levels.py`, which **writes**
`crates/arduracer-core/src/levels.rs` from the 20 legacy `ArduRacerFx/Levels/*.csv`
files and calls itself "the single source of truth" for that file.

`ArduRacerFx/` **is present** in this worktree, so `require-fx-levels` passes and
that step *will* run, replacing all 24 image-derived circuits with the legacy CSV
pipeline this branch exists to retire.

Run only the first step instead:

```
cargo run --manifest-path tools/playtest/Cargo.toml --release -- --calibrate
```

That writes `tools/track_cook/par_calibration.json` and touches nothing else.
Better still, make `convert_levels.py` refuse to run when the atlas pipeline owns
`levels.rs`, so this cannot happen to the next person.

## Pitfalls already paid for — do not rediscover these

1. **Serpentines are not circuits.** An out-and-back zigzag has *zero net turn*.
   It is an S, and it fails the +/-360 check no matter how it is mirrored. These
   are now rings. Do not reintroduce one.
2. **A smaller fillet radius is a *tighter* corner.** I changed Longbow
   13.0 -> 9.0, called it "loosening", and it inverted the meaning. That change
   is a plausible cause of the Longbow failure.
3. **Never transform coordinates twice.** `render()` densified into cell space
   and then let the stamping loop apply `scale`/`dx`/`dy` again. Everything was
   painted ~2.5x too big and clipped off the grid; `Right Angles` came out as an
   L-shape. Apply the fit exactly once, where the conversion happens.
4. **Size sample steps in cell space, after the fit.** Straight anchors spaced in
   authoring units were 9.8 cells apart against a 2.5-cell-wide road, so
   stamping left holes and split the ring in two.
5. **`SCENERY` (15) must not be in the validator's `ROAD` set.** Scenery lives in
   the infield, so counting it as road made all 24 circuits look like they had a
   stray island.
6. **Do not repeat the route's first control point.** The walker closes the
   polyline by repeating point 0; carrying that into `resample_uniform` puts
   control point 95 exactly on point 0, a zero-length span, and Catmull-Rom takes
   its tangent from `(next - prev) / 2`.
7. **Bulk `str.replace(..., 1)` on shared substrings is unsafe** when editing
   circuit definitions — several `_ring(...)` calls share text like `0.0, 3.0)`.
   Verify each landed on the intended circuit.
8. **A Python straight-length predictor over-estimated ~3.5x** against the real
   spline, because the route is a Catmull-Rom through 96 arc-length-uniform
   controls sampled 8x per span. Use the Rust test as the oracle, not a model.
9. `add_walls()` in `generate_circuit_images.py` is **dead code** — it computes a
   `ring` and returns `out` unmodified. `wall_the_edge()` is what actually walls.

## Context you will need

- Read `LEVEL-FORMAT.md` for the two-image format and the validation rules.
- `tools/track_cook/generate_circuit_images.py` — the polygon builders `_ring`,
  `_rect`, `CIRCUITS`, `poly_walk`, `assert_closed`, `fit_scale`, `render`,
  `compose`, `add_walls`, `wall_the_edge`.
- `tools/track_cook/compile_circuit.py` — `ROAD`/`SURFACE` sets, `load_data`,
  `downsample`, `find_regions`, `resample_uniform`, `emit_rust`.
- `tools/track_cook/build_atlas.py` — validates all 24 and writes `levels.rs`.
- The AI lap tests are in `crates/arduracer-core/src/ai.rs` around line 620-665;
  `drive_one_lap` builds a fresh racer and gives it 60,000 ticks for two laps.
- Ceilings already lifted: `MAX_TRACK_DIM=64`, `MAX_TRACK_CHECKPOINTS=24`,
  `MAX_ROUTE_SAMPLES=768`, route arcs widened to `u32`.
- Grid is 48x48 cells, `SIZE=768`, `CELL=16`; the PNGs are 3 px per cell.

Iteration is slow: each shape change needs `generate_circuit_images.py`, then
`build_atlas.py`, then a `cargo test` rebuild. Budget accordingly and prefer
batching changes.
