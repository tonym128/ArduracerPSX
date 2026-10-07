//! Track definitions, tile surfaces, and track geometry.
//!
//! A [`TrackDef`] is a static, `no_std`-friendly description of one circuit:
//! a tile grid of [`TrackTile`] surfaces, a start/finish gate, an ordered
//! checkpoint array, and the par times used for medal evaluation.
//!
//! Tile *surfaces* (rather than raw level-art indices) are baked by
//! `tools/track_cook/build_atlas.py` so that collision, rendering, audio and
//! the AI all agree on what a tile means without duplicating lookup tables.

use crate::ai::heading_towards;
use crate::math::{Fixed, Vec2};
use crate::surface::SurfaceType;
use crate::timing::{CheckpointGate, ParTimes};

/// Maximum checkpoints per track.
/// Checkpoint slots per circuit. Longer circuits need proportionally more split
/// points so a straight is not one long blind gate.
pub const MAX_TRACK_CHECKPOINTS: usize = 24;
/// Maximum dimension of a track grid, in cells.
///
/// 96 rather than 64. Cells are half the world size they used to be (see
/// [`TILE_SIZE`]), so this is the same 3,072-unit world at twice the resolution.
/// `MAX_TILE_INDEX` follows.
///
/// The ceiling is RAM, not taste. A cell is one byte as a `TrackTile`, so 96x96
/// costs 9,216 bytes a circuit -- 221 KB for all 24 resident, inside the
/// ~347 KB the PSX build has free. The next step down, 192x192, is 884 KB and
/// does not fit; see the note on [`TILE_SIZE`] for why 16-unit cells are the
/// floor rather than the ceiling.
pub const MAX_TRACK_DIM: usize = 96;
/// Largest cell index the `u8` cell API can represent.
///
/// [`TrackDef::tile_x_of`] / [`TrackDef::tile_y_of`] saturate here, so the
/// largest possible circuit is still addressable. `tile_at` rejects anything
/// wider than the grid it belongs to and answers `Barrier`.
pub const MAX_TILE_INDEX: u8 = (MAX_TRACK_DIM - 1) as u8;
/// World-space size of one surface cell.
///
/// 32, halved from the 64 it was when a "tile" was also the thing the renderer
/// drew a square for. The world is unchanged at 96 x 32 = 3,072 units a side;
/// the cell just carries half as much area, so the collision surface is four
/// times finer and the road edge is no longer a 64-unit staircase.
///
/// # Why this is 32 and not smaller
///
/// One world unit per cell -- true per-pixel physics -- is 3072 x 3072 cells.
/// At four bits per code (the palette has exactly 16 entries) that is 4.7 MB
/// for a *single* circuit. The machine has 2 MB of RAM, of which the build
/// gate allows 999 KB and currently uses ~652 KB. It is not close: even one
/// circuit at one byte per cell is 9.4 MB.
///
/// So the data is coarse and the *rendering* is per-pixel. That is the split
/// `LEVEL-FORMAT.md` argues for, and the arithmetic above is why it is forced
/// rather than merely tidy. The visual side has no such limit -- a texture in
/// VRAM is sampled per screen pixel -- so appearance is fine-grained even where
/// collision is not.
pub const TILE_SIZE: i32 = 32;

/// Integer square root (floor) of a `u64`. No floats: the core is `no_std`
/// Q20.12 arithmetic only.
fn isqrt_u64(n: u64) -> u64 {
    if n < 2 {
        return n;
    }
    // Newton's method seeded with a power-of-two upper bound.
    let mut x = n;
    let mut y = x.div_ceil(2);
    while y < x {
        x = y;
        y = (x + n / x) / 2;
    }
    x
}

/// A single cell of a track's tile grid.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum TrackTile {
    /// Optimal high-grip racing surface.
    Tarmac = 0,
    /// Rumble curb: slight grip loss plus DualShock/audio rumble.
    Curb = 1,
    /// Drivable terrain outside the racing surface (grass, gravel, sand).
    OffRoad = 2,
    /// Oil slick hazard: near-zero lateral grip.
    OilSlick = 3,
    /// Speed booster strip granting an instant forward impulse.
    BoostPad = 4,
    /// Solid wall: blocks the car and triggers a barrier bounce.
    Barrier = 5,
    /// Start / finish line (also a lap-completion gate).
    StartFinish = 6,
    /// Checkpoint gate tile.
    Checkpoint = 7,
}

impl TrackTile {
    /// Physical surface type used by the vehicle dynamics and haptics.
    pub const fn surface(self) -> SurfaceType {
        match self {
            TrackTile::Tarmac | TrackTile::StartFinish | TrackTile::Checkpoint => {
                SurfaceType::Tarmac
            }
            TrackTile::Curb => SurfaceType::Curb,
            TrackTile::OffRoad => SurfaceType::OffRoad,
            TrackTile::OilSlick => SurfaceType::OilSlick,
            TrackTile::BoostPad => SurfaceType::BoostPad,
            TrackTile::Barrier => SurfaceType::Barrier,
        }
    }

    /// Whether this tile is part of the drivable racing surface.
    pub const fn is_road(self) -> bool {
        matches!(
            self,
            TrackTile::Tarmac | TrackTile::Curb | TrackTile::StartFinish | TrackTile::Checkpoint
        )
    }

    /// Whether this tile blocks the car outright.
    pub const fn is_solid(self) -> bool {
        matches!(self, TrackTile::Barrier)
    }
}

/// A complete racetrack definition.
#[derive(Clone, Debug)]
pub struct TrackDef {
    /// Display name shown in the track select carousel.
    pub name: &'static str,
    /// Grid width in tiles.
    pub width: u8,
    /// Grid height in tiles.
    pub height: u8,
    /// Exact world-space spawn position (centre of the start tile).
    pub start_pos: Vec2,
    /// Initial vehicle heading in BAMs (0 = North, 1024 = East).
    pub start_heading: u16,
    /// The start / finish gate. Leaving this tile with every checkpoint cleared
    /// completes a lap.
    pub start_gate: CheckpointGate,
    /// Target times used for Bronze / Silver / Gold / Dev Platinum medals.
    pub par_times: ParTimes,
    /// Number of active entries in `checkpoints`.
    pub checkpoint_count: u8,
    /// Checkpoint gates in *driving* order (tours may be traversed either way).
    pub checkpoints: [CheckpointGate; MAX_TRACK_CHECKPOINTS],
    /// Row-major tile grid, `height * width` entries.
    pub tiles: &'static [TrackTile],
    /// Authored racing-line control points in world space, in order. The core
    /// builds [`Route`](crate::Route) from these rather than from the checkpoint
    /// gates, so the AI follows the centreline instead of a gate-to-gate chord.
    /// Empty means "derive a fallback from the gates" (test fixtures only).
    pub route: &'static [Vec2],
    /// Half-width of the drivable corridor in world units. Runoff beyond it is
    /// open ground. Zero means "unknown", which the fallback treats as a whole
    /// tile wide.
    pub half_width: u16,
}

impl TrackDef {
    /// Whether the grid dimensions fit inside the `u8` tile API and the
    /// [`MAX_TRACK_DIM`] ceiling. Never enforced by the accessors (they stay
    /// total either way) but it is what every cooker and level integrity check
    /// wants to assert.
    #[inline]
    pub fn dimensions_are_valid(&self) -> bool {
        self.width >= 1
            && self.height >= 1
            && (self.width as usize) <= MAX_TRACK_DIM
            && (self.height as usize) <= MAX_TRACK_DIM
    }

    /// Total world-space width of the circuit in world units.
    pub const fn world_width(&self) -> i32 {
        self.width as i32 * TILE_SIZE
    }

    /// Total world-space height of the circuit in world units.
    pub const fn world_height(&self) -> i32 {
        self.height as i32 * TILE_SIZE
    }

    /// Largest legal world coordinate of the circuit, one unit *inside* the
    /// outer wall.
    ///
    /// The last tile spans `[width*TILE_SIZE - TILE_SIZE, width*TILE_SIZE)`, so
    /// its centre is `width*TILE_SIZE - TILE_SIZE/2` and its last legal integer
    /// coordinate is `width*TILE_SIZE - 1`. Clamping a car to
    /// `world_width()` instead parks it one unit past the grid, where
    /// `tile_at` reports `Barrier`: no traction, no speed, and a wall nobody
    /// can escape.
    #[inline]
    pub fn max_inside_x(&self) -> i32 {
        (self.world_width() - 1).max(0)
    }

    /// See [`TrackDef::max_inside_x`] for the vertical axis.
    #[inline]
    pub fn max_inside_y(&self) -> i32 {
        (self.world_height() - 1).max(0)
    }

    /// The outermost world coordinate on each axis that is still on a drivable
    /// tile.
    ///
    /// [`Self::max_inside_x`] and [`Self::max_inside_y`] only keep a car inside
    /// the grid, which is not the same as keeping it somewhere it can drive: the
    /// circuits carry a solid wall band around the outside, so the last row and
    /// column of the grid are `Barrier`. Clamping a car there handed it a tile
    /// with zero traction and zero top speed -- a car that could never move
    /// again. Walks inward from the edge to the last non-solid coordinate.
    #[inline]
    pub fn max_drivable_x(&self) -> i32 {
        for step in 1..=self.width.max(1) {
            let tx = self.width - step;
            if !self.tile_at(tx, 0).is_solid() {
                return (tx as i32) * TILE_SIZE + (TILE_SIZE - 1);
            }
        }
        0
    }

    /// See [`Self::max_drivable_x`] for the vertical axis.
    #[inline]
    pub fn max_drivable_y(&self) -> i32 {
        for step in 1..=self.height.max(1) {
            let ty = self.height - step;
            if !self.tile_at(0, ty).is_solid() {
                return (ty as i32) * TILE_SIZE + (TILE_SIZE - 1);
            }
        }
        0
    }

    /// Returns the raw tile at a grid coordinate, or `Barrier` when out of bounds.
    #[inline]
    pub fn tile_at(&self, tx: u8, ty: u8) -> TrackTile {
        if tx >= self.width || ty >= self.height {
            return TrackTile::Barrier;
        }
        let idx = (ty as usize) * (self.width as usize) + (tx as usize);
        match self.tiles.get(idx) {
            Some(&t) => t,
            None => TrackTile::Barrier,
        }
    }

    /// Gets the surface type for a tile coordinate.
    #[inline]
    pub fn surface_at(&self, tx: u8, ty: u8) -> SurfaceType {
        self.tile_at(tx, ty).surface()
    }

    /// Whether a tile coordinate lies inside the drivable racing surface.
    #[inline]
    pub fn is_road_at(&self, tx: u8, ty: u8) -> bool {
        self.tile_at(tx, ty).is_road()
    }

    /// Whether a car should *aim* to be on this tile: the racing surface plus
    /// the drivable hazards that sit on top of it.
    ///
    /// [`TrackDef::is_road_at`] excludes boost pads and oil slicks even though
    /// both are perfectly drivable, so navigation that only accepts road tiles
    /// treats a boost strip as a hole in the circuit.
    #[inline]
    pub fn is_line_tile(&self, tx: u8, ty: u8) -> bool {
        matches!(
            self.tile_at(tx, ty),
            TrackTile::Tarmac
                | TrackTile::Curb
                | TrackTile::StartFinish
                | TrackTile::Checkpoint
                | TrackTile::BoostPad
                | TrackTile::OilSlick
        )
    }

    /// Converts a world position into the containing tile column.
    ///
    /// Total for every input, which is why `pub` fields without a constructor
    /// are safe to pass around: negative positions clamp to `0` and anything
    /// past the largest representable circuit saturates at [`MAX_TILE_INDEX`].
    ///
    /// The returned index is *not* clamped against a particular track's
    /// `width`; it can legitimately be `>= width` for a position beyond the
    /// grid, and [`TrackDef::tile_at`] then reports `Barrier`. Callers that
    /// want an on-track tile must clamp themselves (as the vehicle and the AI
    /// do with `.min(width - 1)`).
    #[inline]
    pub fn tile_x_of(world_x: Fixed) -> u8 {
        Self::tile_index_of(world_x)
    }

    /// Converts a world position into the containing tile row.
    ///
    /// See [`TrackDef::tile_x_of`]: total, saturating, and not clamped against
    /// any particular track's `height`.
    #[inline]
    pub fn tile_y_of(world_y: Fixed) -> u8 {
        Self::tile_index_of(world_y)
    }

    /// Shared world-unit -> tile-index mapping (total, saturating, never wraps).
    #[inline]
    fn tile_index_of(world: Fixed) -> u8 {
        // `to_int` is an arithmetic shift, so this is at worst -524288 and at
        // best 524287: the division cannot overflow.
        let units = world.to_int();
        if units <= 0 {
            return 0;
        }
        let tile = units / TILE_SIZE;
        if tile >= MAX_TRACK_DIM as i32 {
            MAX_TILE_INDEX
        } else {
            tile as u8
        }
    }

    /// World-space centre of a tile centre coordinate.
    #[inline]
    pub fn tile_centre(tile: u8) -> Fixed {
        Fixed::from_int(tile as i32 * TILE_SIZE + TILE_SIZE / 2)
    }

    /// Number of checkpoints that actually exist, clamped to the array.
    ///
    /// [`TrackDef::checkpoint_count`] is a `pub u8` with no constructor, so a
    /// hand-built `TrackDef` can claim more gates than the fixed-size
    /// [`TrackDef::checkpoints`] array holds. Every accessor goes through this
    /// so such a value degrades to "all 16" instead of indexing past the end.
    #[inline]
    pub fn active_checkpoint_count(&self) -> usize {
        (self.checkpoint_count as usize).min(MAX_TRACK_CHECKPOINTS)
    }

    /// Active checkpoint gates as a slice.
    ///
    /// Total: the length is clamped by [`TrackDef::active_checkpoint_count`],
    /// so this can never slice out of bounds for any `checkpoint_count`.
    #[inline]
    pub fn checkpoint_slice(&self) -> &[CheckpointGate] {
        let count = self.active_checkpoint_count();
        &self.checkpoints[..count]
    }

    /// Number of nodes in a full circuit: every checkpoint plus the start/finish.
    ///
    /// AI navigation walks `0 ..= route_len() - 1`, wrapping back to 0, so the
    /// start/finish is part of the racing line rather than a shortcut the drivers
    /// skip. Always at least 1, so `% route_len()` can never divide by zero.
    #[inline]
    pub fn route_len(&self) -> usize {
        self.active_checkpoint_count() + 1
    }

    /// The `idx`-th node of the circuit (checkpoints in driving order, then the
    /// start/finish gate). `idx` wraps modulo [`TrackDef::route_len`].
    ///
    /// Total: `idx >= active_checkpoint_count()` (which includes every `idx`
    /// past the end of the circuit) resolves to the start/finish gate, and the
    /// lookup into `checkpoints` goes through `get` rather than an index.
    #[inline]
    pub fn route_node(&self, idx: usize) -> CheckpointGate {
        if idx >= self.active_checkpoint_count() {
            return self.start_gate;
        }
        match self.checkpoints.get(idx) {
            Some(&gate) => gate,
            None => self.start_gate,
        }
    }

    /// Centre point of a checkpoint gate in world space.
    #[inline]
    pub fn gate_centre(gate: &CheckpointGate) -> Vec2 {
        Vec2 {
            x: Fixed::from_int(gate.x as i32 * TILE_SIZE + (gate.width as i32 * TILE_SIZE) / 2),
            y: Fixed::from_int(gate.y as i32 * TILE_SIZE + (gate.height as i32 * TILE_SIZE) / 2),
        }
    }

    /// Approximate centre-line length of the circuit in world units, used for
    /// pace balancing and lap-time sanity checks.
    ///
    /// This sums the *Manhattan* distance between consecutive route-node
    /// centres, so it deliberately over-estimates a real racing line (a car
    /// cuts the corner instead of driving the right-angle dog-leg). It is
    /// therefore a safe numerator for a lap-time floor but not a safe measure
    /// of how far a car actually has to travel -- see
    /// [`TrackDef::min_route_length`] for that.
    pub fn route_length(&self) -> i32 {
        if self.active_checkpoint_count() == 0 {
            return self.world_width() + self.world_height();
        }
        let mut total = 0i32;
        let start = Self::gate_centre(&self.start_gate);
        let mut prev = start;
        for gate in self.checkpoint_slice() {
            let p = Self::gate_centre(gate);
            total += (p.x - prev.x).to_int().abs() + (p.y - prev.y).to_int().abs();
            prev = p;
        }
        total += (prev.x - start.x).to_int().abs() + (prev.y - start.y).to_int().abs();
        total
    }

    /// Lower bound on the distance any car must travel to complete one circuit.
    ///
    /// For each consecutive pair of route nodes this is the distance between
    /// their gate *rectangles* (zero when they overlap or touch), so a car
    /// clipping the nearest corner of every gate still cannot do better.
    /// Dividing this by a car's top speed gives the physically achievable
    /// lap-time floor: anything faster means the driver skipped part of the
    /// circuit.
    ///
    /// Note this is a *bound*, not a prediction: a real racing line is longer,
    /// because a car cannot drive a straight line between two corners. See
    /// [`TrackDef::route_length`] for the deliberately pessimistic (Manhattan)
    /// figure used for pace balancing.
    ///
    /// Total for any `TrackDef`: the walk is bounded by [`Self::route_len`],
    /// gate coordinates are `u8` so the squares fit an `i64`, and the addition
    /// saturates.
    pub fn min_route_length(&self) -> i32 {
        let n = self.route_len();
        let mut total = 0i32;
        for idx in 0..n {
            let a = self.route_node(idx);
            let b = self.route_node((idx + 1) % n);
            // Axis-separated gap between the two gate rectangles, in world units.
            let dx = (a.x as i32 - (b.x as i32 + b.width as i32))
                .max(b.x as i32 - (a.x as i32 + a.width as i32))
                .max(0)
                * TILE_SIZE;
            let dy = (a.y as i32 - (b.y as i32 + b.height as i32))
                .max(b.y as i32 - (a.y as i32 + a.height as i32))
                .max(0)
                * TILE_SIZE;
            let gap = isqrt_u64(dx as u64 * dx as u64 + dy as u64 * dy as u64);
            total = total.saturating_add(gap.min(i32::MAX as u64) as i32);
        }
        total
    }

    /// Where to drop a stuck car: the nearest route node to `from`, facing along
    /// the racing line toward the node after it.
    ///
    /// Used by the in-race recovery. Dropping on the node *centre* guarantees
    /// the car lands on the racing surface rather than inside scenery, and facing
    /// the next node means it points down the track rather than back across it.
    pub fn respawn_point(&self, from: Vec2) -> (Vec2, u16) {
        let n = self.route_len();
        if n == 0 {
            return (Self::gate_centre(&self.start_gate), 0);
        }

        let mut best_idx = 0usize;
        let mut best_dist = i32::MAX;
        for idx in 0..n {
            let p = Self::gate_centre(&self.route_node(idx));
            let dx = (p.x - from.x).to_int();
            let dy = (p.y - from.y).to_int();
            let dist = dx * dx + dy * dy;
            if dist < best_dist {
                best_dist = dist;
                best_idx = idx;
            }
        }

        let here = Self::gate_centre(&self.route_node(best_idx));
        let next = Self::gate_centre(&self.route_node((best_idx + 1) % n));
        (here, heading_towards(here, next))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::levels::ALL_TRACKS;

    /// A 10x10 all-tarmac grid with the full gate array filled, and a
    /// hand-editable live count.
    ///
    /// The gates are *generated* across the capacity rather than hand-written.
    /// A literal list silently under-fills when `MAX_TRACK_CHECKPOINTS` grows:
    /// the array still has the right length, but the tail is `default()` and
    /// therefore inactive, so any test asserting every route node resolves fails
    /// for a reason that has nothing to do with what it is testing.
    fn scratch_track(checkpoint_count: u8) -> TrackDef {
        const GRID: usize = 10;
        let mut gates = [CheckpointGate::default(); MAX_TRACK_CHECKPOINTS];
        for (i, slot) in gates.iter_mut().enumerate() {
            // Walk the grid perimeter so every gate is distinct and on tarmac.
            let edge = GRID - 2;
            let (x, y) = match i % 4 {
                0 => (i % (GRID - 1), 1),
                1 => (edge, i % (GRID - 1)),
                2 => (edge - (i % edge), edge),
                _ => (1, edge - (i % edge)),
            };
            *slot = CheckpointGate {
                x: x as u8,
                y: y as u8,
                width: 1,
                height: 1,
            };
        }
        static TILES: [TrackTile; 100] = [TrackTile::Tarmac; 100];
        TrackDef {
            name: "SCRATCH",
            width: 10,
            height: 10,
            start_pos: Vec2::new(Fixed::from_int(96), Fixed::from_int(96)),
            start_heading: 0,
            start_gate: CheckpointGate {
                x: 1,
                y: 1,
                width: 1,
                height: 1,
            },
            par_times: ParTimes::default(),
            checkpoint_count,
            checkpoints: gates,
            tiles: &TILES,
            // No authored centreline: exercises the gate-derived fallback.
            route: &[],
            half_width: 0,
        }
    }

    // --- bug 11: checkpoint_slice / route_node above the array capacity -------

    #[test]
    fn checkpoint_count_above_capacity_cannot_panic() {
        // `checkpoint_count` is a pub u8 with no constructor, so it can exceed
        // the array. Asked for well past capacity so the test keeps meaning
        // something as `MAX_TRACK_CHECKPOINTS` grows.
        let track = scratch_track(MAX_TRACK_CHECKPOINTS as u8 + 4);
        assert_eq!(track.active_checkpoint_count(), MAX_TRACK_CHECKPOINTS);
        assert_eq!(track.checkpoint_slice().len(), MAX_TRACK_CHECKPOINTS);
        assert_eq!(track.route_len(), MAX_TRACK_CHECKPOINTS + 1);
        // And every route index, including far past the end, resolves safely.
        for idx in 0..64 {
            let node = track.route_node(idx);
            assert!(node.is_active(), "route node {idx} came back empty");
        }
        assert_eq!(track.route_node(MAX_TRACK_CHECKPOINTS), track.start_gate);
        assert_eq!(
            track.route_node(MAX_TRACK_CHECKPOINTS + 5),
            track.start_gate
        );
    }

    #[test]
    fn checkpoint_count_at_and_below_capacity_is_unchanged() {
        for count in 0..=(MAX_TRACK_CHECKPOINTS as u8) {
            let track = scratch_track(count);
            assert_eq!(track.active_checkpoint_count(), count as usize);
            assert_eq!(track.checkpoint_slice().len(), count as usize);
            assert_eq!(track.route_len(), count as usize + 1);
            for idx in 0..count as usize {
                assert_eq!(track.route_node(idx), track.checkpoints[idx]);
            }
        }
    }

    #[test]
    fn route_node_is_total_for_every_usize() {
        let track = scratch_track(4);
        let count = track.active_checkpoint_count();
        // Every index at or past the checkpoint count is the start/finish line,
        // however absurd the index is.
        for idx in [
            count,
            count + 1,
            count + 2,
            MAX_TRACK_CHECKPOINTS,
            16,
            17,
            255,
            256,
            4096,
            1 << 20,
            usize::MAX / 2,
            usize::MAX,
        ] {
            assert_eq!(track.route_node(idx), track.start_gate, "idx {idx}");
        }
        // And a count that overruns the array still resolves every index.
        let over = scratch_track(MAX_TRACK_CHECKPOINTS as u8 + 4);
        for idx in 0..(MAX_TRACK_CHECKPOINTS * 2) {
            let node = over.route_node(idx);
            assert!(node.is_active(), "route node {idx} came back empty");
        }
    }

    #[test]
    fn route_len_is_never_zero_so_modulo_is_safe() {
        for count in [0u8, 1, 16, 20, 24, 255] {
            assert!(scratch_track(count).route_len() >= 1);
        }
    }

    // --- bug 12: tile_x_of / tile_y_of must be total and saturating ---------

    #[test]
    fn tile_index_never_wraps_for_positive_coordinates() {
        // Everything inside the largest representable circuit (32 tiles) is an
        // exact division.
        for tile in 0..MAX_TILE_INDEX {
            for unit in [
                tile as i32 * TILE_SIZE,
                tile as i32 * TILE_SIZE + TILE_SIZE - 1,
            ] {
                assert_eq!(TrackDef::tile_x_of(Fixed::from_int(unit)), tile, "{unit}");
                assert_eq!(TrackDef::tile_y_of(Fixed::from_int(unit)), tile, "{unit}");
            }
        }
        // Past the largest circuit the index saturates at the ceiling.
        // `(v / TILE_SIZE) as u8` used to wrap mod 256 instead, so these all
        // came back as tile 0 (or some arbitrary low tile).
        // Coordinates past the largest circuit saturate. The first entry is
        // computed from `MAX_TRACK_DIM` rather than written down, so raising the
        // grid ceiling cannot leave this probing a legal coordinate and quietly
        // asserting the wrong thing.
        let one_past = (MAX_TRACK_DIM as i32 + 1) * TILE_SIZE;
        for (units, wrapped_before) in [
            (one_past, 0), // one past the ceiling
            (4_096, 64),   // 64 tiles
            (8_191, 127),  // 127 tiles
            (16_383, 255), // 255 tiles -- one below the wrap
            (16_384, 0),   // 256 tiles -> wrapped to 0
            (100_000, 26), // 1562 tiles -> wrapped
            (524_287, 31), // 8191 tiles, the largest world
        ] {
            assert_eq!(
                TrackDef::tile_x_of(Fixed::from_int(units)),
                MAX_TILE_INDEX,
                "tile_x_of({units}) used to report tile {wrapped_before}"
            );
            assert_eq!(TrackDef::tile_y_of(Fixed::from_int(units)), MAX_TILE_INDEX);
        }
        assert_eq!(TrackDef::tile_y_of(Fixed::from_raw(i32::MIN)), 0);
        assert_eq!(
            TrackDef::tile_y_of(Fixed::from_raw(i32::MAX)),
            MAX_TILE_INDEX
        );
    }

    #[test]
    fn tile_index_is_monotonic_across_the_whole_legal_world() {
        let mut previous = 0u8;
        let mut units = 0i32;
        while units <= MAX_TILE_INDEX as i32 * TILE_SIZE {
            let tx = TrackDef::tile_x_of(Fixed::from_int(units));
            assert!(tx >= previous, "tile_x_of went backwards at {units}");
            previous = tx;
            units += 7;
        }
        // A full sweep never exceeds the ceiling.
        units = 0;
        while units < 200_000 {
            assert!(TrackDef::tile_y_of(Fixed::from_int(units)) <= MAX_TILE_INDEX);
            units += 311;
        }
    }

    #[test]
    fn tile_index_handles_negatives_and_zero() {
        for units in [i32::MIN, -100_000, -32, -1, 0] {
            assert_eq!(TrackDef::tile_x_of(Fixed::from_int(units)), 0, "{units}");
            assert_eq!(TrackDef::tile_y_of(Fixed::from_int(units)), 0, "{units}");
        }
        // Boundary is `TILE_SIZE`, not a literal: this test asserted 63/64 when a
        // cell was 64 units, and would have passed a cell size that was silently
        // wrong by being written against the old constant's value.
        assert_eq!(TrackDef::tile_x_of(Fixed::from_int(TILE_SIZE - 1)), 0);
        assert_eq!(TrackDef::tile_x_of(Fixed::from_int(TILE_SIZE)), 1);
    }

    #[test]
    fn tile_index_matches_the_tile_centres() {
        for tile in 0..=MAX_TILE_INDEX {
            let centre = TrackDef::tile_centre(tile);
            assert_eq!(TrackDef::tile_x_of(centre), tile);
            assert_eq!(TrackDef::tile_y_of(centre), tile);
        }
    }

    #[test]
    fn every_shipped_track_is_inside_the_tile_index_range() {
        for track in ALL_TRACKS.iter() {
            assert!(
                track.dimensions_are_valid(),
                "{}: bad dimensions",
                track.name
            );
            // Every legal coordinate of the grid maps back inside the grid.
            for units in [0, 1, track.world_width() / 2, track.world_width() - 1] {
                let tx = TrackDef::tile_x_of(Fixed::from_int(units));
                assert!(tx < track.width, "{}: x={units} -> tile {tx}", track.name);
            }
            for units in [0, 1, track.world_height() / 2, track.world_height() - 1] {
                let ty = TrackDef::tile_y_of(Fixed::from_int(units));
                assert!(ty < track.height, "{}: y={units} -> tile {ty}", track.name);
            }
            assert!(track.max_inside_x() < track.world_width());
            assert!(track.max_inside_y() < track.world_height());
            assert!(track.max_inside_x() >= 0);
        }
    }

    // --- route_length / min_route_length -------------------------------------

    #[test]
    fn min_route_length_is_a_true_lower_bound_on_the_circuit() {
        for track in ALL_TRACKS.iter() {
            let min = track.min_route_length();
            assert!(min > 0, "{}: zero-length lower bound", track.name);
            assert!(
                min <= track.route_length(),
                "{}: min {} exceeds manhattan {}",
                track.name,
                min,
                track.route_length()
            );
        }
    }

    #[test]
    fn line_tiles_cover_the_racing_surface_and_the_hazards_on_it() {
        for track in ALL_TRACKS.iter() {
            for ty in 0..track.height {
                for tx in 0..track.width {
                    let tile = track.tile_at(tx, ty);
                    if tile.is_road() {
                        assert!(
                            track.is_line_tile(tx, ty),
                            "{}: ({tx},{ty}) {:?} is road but not a line tile",
                            track.name,
                            tile
                        );
                    }
                    if tile.is_solid() {
                        assert!(
                            !track.is_line_tile(tx, ty),
                            "{}: ({tx},{ty}) is scenery but claims to be a line tile",
                            track.name
                        );
                    }
                }
            }
        }
        // Boost pads and oil slicks are drivable but are not `is_road`, which is
        // why navigation needs this predicate at all.
        let mut pad = scratch_track(4);
        pad.tiles = Box::leak(Box::new([TrackTile::BoostPad; 100]));
        assert!(!pad.is_road_at(0, 0));
        assert!(pad.is_line_tile(0, 0));
        let mut slick = scratch_track(4);
        slick.tiles = Box::leak(Box::new([TrackTile::OilSlick; 100]));
        assert!(!slick.is_road_at(0, 0));
        assert!(slick.is_line_tile(0, 0));
    }

    #[test]
    fn isqrt_is_floored_and_correct() {
        for n in [0u64, 1, 2, 3, 4, 8, 9, 15, 16, 255, 4096, 65535] {
            let r = isqrt_u64(n);
            assert!(r * r <= n, "isqrt({n}) = {r} overshoots");
            assert!((r + 1) * (r + 1) > n, "isqrt({n}) = {r} undershoots");
        }
    }
}
