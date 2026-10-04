# OVERHAUL — Track, Camera and Race-Start Overhaul

Planning document. Each step below is a proposal with a defined "done" condition.

**Status:** steps 7, 8 and 9 are implemented and verified. Step 1 is implemented
(`arduracer_core::Route`, with the centreline *derived* by fitting a spline
through each circuit's already-ordered gate centres -- no level-pipeline change
needed, and it matches the AI's own gate-to-gate line by construction).

**Steps 3 and 4 are blocked on step 2, and this was measured, not assumed.**
Migrating lap validation to centreline-relative, direction-aware arc crossings
works -- a prototype scored forward laps and correctly refused reverse ones on
all 24 circuits -- but `tools/playtest`'s reference driver then **stalled on every
hairpin**, because a spline through gate centres overshoots there and stops being
a line the car can drive. The centreline is the wrong validation line until the
*road* is drivable: a curve can be geometrically correct and still run off the
inside of a corner. Step 2 (authored runoff margin, and a road-quality pass on the
tight circuits) has to land first. Enabling arc validation before then would
reject legitimate laps on exactly the tracks where driving is hardest.

Consequence for the plan: the reference driver is the canary for this. When step
2 lands, `make playtest` must be re-run before step 4 is enabled.

**The level pipeline regenerates byte-identically.** Verified while starting
step 1: `python3 tools/track_cook/convert_levels.py` rewrites `levels.rs` with an
empty diff, and its inputs are in the repository -- `ArduRacerFx/Levels/` is 46
tracked files, not a gitignored local checkout. So the pipeline is reproducible
from a fresh clone, which makes step 1 tractable: change the cooker, regenerate,
commit the result. It also retires the first half of audit finding #5, which
claimed `levels.rs` could not be regenerated.

The goal is the set of features that separate this PSX version from the Arduboy
original: **bigger circuits with real runoff**, **smooth corners instead of
blocks**, **checkpoint gates spanning the full track width**, **speed-reactive
camera**, and **a countdown to the start**. Along the way three confirmed
defects get closed (gate width, reverse-lap scoring, dead camera zoom).

Ordering matters. Steps 1-4 are one coherent data-model change and should not be
split. Steps 5-6 consume it. Steps 7-9 are independent of the geometry work and
can ship first if a smaller slice is wanted. Step 10 is mandatory.

---

## Current state (measured, not assumed)

Three findings drive this plan.

**1. `Camera::zoom` is computed every frame and then discarded.**
`Camera::update` (`game/src/gpu/camera.rs:40`) derives a speed-based `zoom`
correctly, but `world_to_screen` (`camera.rs:59`) maps world space to screen space
one-to-one:

```rust
let rel_x = (world_pos.x - self.pos.x).raw() / FP_ONE;
let screen_x = (SCREEN_W / 2) + (rel_x as i16);
```

There is no `zoom` term. Every consumer — `car_renderer.rs:103`,
`particles.rs:104`, `skidmarks.rs:95` — goes through that function. Speed-zoom is
therefore a **dead scaffold**, not a tuning problem: step 7 is mostly wiring.

**2. The centrelines are discarded at cook time.**
`tools/track_cook/convert_levels.py` fits closed Catmull-Rom splines
(`catmull_rom`, line 310) through authored control points, rasterises them into
64-unit tiles, and emits only `TrackDef { tiles, start_gate, checkpoints }`. The
emitted gates are hardcoded 1x1 tiles (line 429:
`# Gates are 1x1, matching CheckpointGate { width: 1, height: 1 }`).

So the smooth route **already exists** in the cook script and is thrown away.
Blocky corners are a *renderer* artifact, not a geometry-authoring one. That is
why step 1 is an enabler rather than a rewrite.

**3. Circuits are small.** Track grids run from 10x10 tiles (100) to 30x30 (900);
the entire 24-track tile set is **6,424 bytes** as `u8`. `render_track`
(`game/src/gpu/tile_blitter.rs:98`) walks the visible tile window and issues one
primitive per tile.

---

## The ten steps

### 1. Emit centrelines into `TrackDef`

Add `route: &[Vec2]` (8-16 control points) and `half_width: u16` per track to
`TrackDef` (`crates/arduracer-core/src/track.rs`). Change
`convert_levels.py` to write them rather than discarding them after
rasterisation. Evaluate the spline **at runtime** rather than baking a dense
polyline, so this costs effectively no ROM.

*Files:* `crates/arduracer-core/src/track.rs`,
`tools/track_cook/convert_levels.py` (regenerates `levels.rs`).

*Done when:* every `TrackDef` exposes a closed, non-self-intersecting centreline
and `cargo run -p test_game_logic` confirms closure for all 24 circuits.

*Implemented, differently to plan.* Rather than emitting control points from the
cooker, `Route::from_track` fits the spline at runtime from the ordered gates the
level data already contains. That is why no cooker change, and no `levels.rs`
regeneration, is needed.

Two traps this hit, both now guarded by tests: arc positions are **world units in
a `u16`**, so accumulating them in Q20.12 saturates at 65535 -- every circuit
longer than 16 units then reports a full lap as 65535 and all gates collapse onto
one arc, which silently fails every lap. And `nearest()` must be given a **full
scan** when deriving gate arcs: chaining the search hint put a start/finish line at
the wrong arc on Arduboy Oval (669 instead of 1284).

### 2. Author an explicit runoff margin

Give every circuit a margin of at least 6 tiles of drivable `OffRoad` beyond the
widest point of the corridor, stored as `margin_tiles` rather than left to
whatever whitespace the rasteriser happened to produce.

Then retire the world-bounds hard clamp. `Vehicle::collide_with_track`
(`vehicle.rs:376`) currently clamps against `track.world_width() /
world_height()` — an invisible wall at the tile-grid edge, which is a large part
of why the track reads as ending.

*Done when:* no reachable driving surface is within N tiles of the grid edge, and
the outermost row of tiles is never referenced as a wall.

### 3. Redefine gates as centreline-relative

Replace `CheckpointGate { x, y, width: 1, height: 1 }` (`timing.rs:18`) with a
model that references the route rather than the grid:

```rust
pub struct CheckpointGate {
    pub arc_pos: u16,    // distance along the centreline
    pub half_width: u16, // spans the full road width by construction
    pub heading: u16,    // crossing direction
}
```

Gates then span the road at any track scale, and the authoring data stops
depending on tile coordinates.

*Done when:* every gate's `half_width` >= the corridor half-width at its
`arc_pos`, asserted for all 24 circuits.

### 4. Rewrite lap validation as plane crossing  *(blocked on step 2)*

`LapTimer` scores a lap from `contains_tile` coverage with no ordering and no
direction check — which is why **driving backwards through every gate scores a
lap on all 24 circuits** (audit item #15).

Replace it with signed progress along the centreline: a lap requires crossing
each gate in ascending `arc_pos` order, inside a direction cone. This closes
audit items #5 and #15 together, and makes the wider gates meaningful.

*Done when:* a forward clean lap validates, a reversed lap does not, and a lap
with skipped gates does not.

*Blocked, with evidence.* A prototype of this passed: walking a circuit's
centreline scores laps on all 24 tracks, and driving the same centreline backwards
scores nothing on any of them -- which is the defect closed. But feeding the real
physics through it, `make playtest` completed **zero** laps on the hairpins and
serpentines (Twin Hairpin, The Serpent, Metropolis 10) and only recovered the
oval-style circuits after the reference driver was changed to follow the centreline
with a lookahead. The cause is that a spline through gate centres overshoots on
tight turns and leaves the road, so it cannot be the validation line until the
road is drivable. Do step 2 first.

### 5. Render the road as a ribbon, not tiles

Build a triangle-strip mesh along the centreline (+/- `half_width`), coloured in
tarmac/curb bands, and demote the tile grid to a **surface lookup** used only by
physics and AI. A ribbon is roughly 200 quads for a whole circuit against up to
900 tile primitives today, and removes the blocky corners.

*Open decision — see Risks.* Tile-based collision means the visual edge can
disagree with the physical edge by up to half a tile.

*Done when:* no visible stair-stepping on any corner at gameplay zoom, and the
primitive count per frame drops.

### 6. Derive the minimap from the centreline

The static minimap outline is currently redrawn every frame, up to 900
primitives on a 30x30 track (audit item #16) — and that gets worse as grids grow.
One polyline stroke replaces it and scales for free.

*Done when:* minimap primitive count is independent of grid size.

### 7. Clamp the camera, then apply zoom  *(done)*

With step 2's margin in place, clamp `Camera::pos` so the view rectangle cannot
leave the authored bounds. **Then** give `world_to_screen` a real `zoom` term —
today the field is written and discarded. Keep the on-screen scale roughly
constant as circuits grow, or the new size reads as a tiny car in an empty field.

*Done when:* the viewport never shows outside the authored bounds, and screen
scale visibly changes between standstill and top speed.

*Implemented.* `world_to_screen` now applies zoom; `tile_blitter` draws through
the same transform (one `k()` helper scales positions, tile size and every
decorative sub-rect) and takes its tile window from
`Camera::visible_half_extents`; `clamp_to_bounds` keeps the view inside the
circuit, centring on axes where the circuit is narrower than the view. Zoom
ranges 1.0 at rest to ~0.78 at top speed.

### 8. Make look-ahead heading-based  *(done)*

`camera.rs:42` leads by `target_velocity * 0.39`, which collapses toward zero when
the car is slow or pinned against a wall. Lead by **heading** with a
speed-scaled magnitude, so the car always sits behind screen centre and the
driver sees where they are going. This is the "car drifts back on screen" feel,
and needs no new data once step 7 lands.

*Done when:* the car's screen position is stable at zero speed and leans into
corners without jitter.

*Implemented.* Lead is velocity/heading derived and scales from ~10 world units at
rest to ~72 at top speed, so the car is never dead centre and always sits behind
screen centre in the direction of travel.

### 9. Add a real start sequence  *(done)*

`LapTimer::start()` arms the clock the instant the race begins; there is no
countdown anywhere. Add a pre-race state: 3-2-1-GO with throttle and brake
locked, AI held on the grid, and `timer.start()` fired on GO. The existing doc
comment on `timing.rs` already references a countdown light that was never built.

Add the audio (beeps per light, engine idle) from the existing SPU path.

*Done when:* the clock starts on GO, not on crossing the line; no input during
the countdown moves the car.

*Implemented.* `StartSequence` in `arduracer-core::timing`: grid, three lights,
GO, racing. `reset_race` no longer calls `timer.start()`; the race loop arms the
clock on the one-shot `just_started()` edge, zeroes driver input until GO, holds
the AI on the grid, and `render_start_lights` draws the rig above the car. The
audio cues are still outstanding -- the lights are visual only.

### 10. Re-verify end to end

`make playtest` drives all 24 circuits, so it catches broken geometry — but
**every par time must be re-measured** (`make calibrate-tracks`), because faster,
larger circuits invalidate every published metric in `GAME.md` and `REVIEW.md`.

New tests: spline closure, margin sufficiency, gate span >= road width,
direction-enforced lap validation, countdown state transitions, zoom monotonicity.
Then the RAM budget gate.

*Done when:* `make ci` is green, par times are re-measured and documented, and no
document claims a metric that no longer holds.

---

## Risks and open decisions

**Collision smoothness (step 5).** Either accept a half-tile mismatch between the
smooth visual edge and the tile-based physical edge, or store a per-tile signed
distance-to-centreline band and interpolate at runtime. Recommendation: pay for
the band. It is the difference between "smooth" and "smooth-looking but catches on
invisible edges". Cost is roughly one byte per tile plus a little per-tick work.

**Par times are published content.** Step 10 changes every medal target. This is a
visible content change, not a refactor, and should be reviewed as such.

**ROM headroom is adequate but not generous.** The executable is 364 KB against a
500 KB target. Growing all 24 grids to 48x48 adds about **49 KB** of tile data
(6,424 bytes -> ~55 KB). Step 1's runtime spline evaluation is what keeps this
from being a problem: baking dense polylines instead would add ~20 KB of
coordinates for no benefit.

**Playtest coverage stays honest.** `playtest` currently passes `&[]` for
neighbour positions, so AI obstacle avoidance has no coverage at all. The larger
circuits will change AI racing lines, which makes that gap more expensive to
leave.

---

## Suggested slicing

- **Quick win (steps 7, 8, 9):** camera zoom, heading look-ahead, countdown.
  Independent of geometry, immediately visible, and closes two confirmed defects
  plus one missing feature.
- **Coherent change (steps 1-6):** the data model, then its consumers. Do not
  split — a centreline that nothing renders is dead weight, and gates that span a
  width nothing knows about are not gates.
- **Always last (step 10):** recalibration and verification.

---

## Related open work

Car-to-car collision is still missing entirely (`collide_with_track` takes no car
argument), and it interacts with steps 2 and 5: bigger circuits with more runoff
give AI more room to race each other, and ribbon geometry gives a surface normal
to compute contact from. It is out of scope here, but it will be cheaper to add
after step 5 than before.