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
