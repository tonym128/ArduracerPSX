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

/// World units of drivable road one racing lane occupies.
///
/// A car body is 14 units wide (`contact::CAR_BODY_RADIUS`), so a lane is
/// roughly four and a half car widths: one car with room to place it anywhere
/// across the corridor without a wheel crossing the white line, which is what
/// makes a *lane* rather than a groove. Two cells wide, so every legal road
/// half-width is a whole number of cells and the collision grid can represent
/// it exactly.
///
/// This is the unit the width floor, the AI's swerve budget and the starting
/// grid are all expressed in, so a circuit can be authored as "one lane" or
/// "two lanes" and every one of those readers agrees what that means.
pub const LANE_WIDTH: i32 = 2 * TILE_SIZE;

/// Half the width of the narrowest publishable road: one lane.
///
/// A road narrower than this is narrower than a single car can be placed in
/// without a wheel hanging over the kerb, so it is a decoration, not a circuit.
pub const MIN_ROAD_HALF_WIDTH: i32 = LANE_WIDTH / 2;

/// Widest publishable road half-width, in world units.
///
/// Half the grid, less the outer barrier band, rounded down to a whole cell.
/// A road wider than this cannot have its kerbs and barrier inside the grid, so
/// the circuit runs off the edge of the map rather than being wide.
pub const MAX_ROAD_HALF_WIDTH: i32 = (MAX_TRACK_DIM as i32 / 2 - 1) * TILE_SIZE;

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
pub const MAX_TRACK_DIM: usize = 240;
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

use crate::math::isqrt_u64;

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
    /// Decodes a grid byte read from the disc into a variant.
    ///
    /// The discriminants are consecutive from 0, so this is a bounds check and
    /// a cast. It exists because the byte comes off a disc the cooker wrote, and
    /// a bare `transmute` here would turn one bad byte into an invalid enum
    /// discriminant -- undefined behaviour in everything downstream, including
    /// the `match` in [`surface`](Self::surface). Anything unrecognised becomes
    /// [`Barrier`](Self::Barrier), which is what `tile_at` already returns for
    /// an out-of-bounds coordinate, so a corrupt grid fails visibly instead of
    /// unpredictably.
    pub const fn from_disc_byte(b: u8) -> Self {
        match b {
            0 => Self::Tarmac,
            1 => Self::Curb,
            2 => Self::OffRoad,
            3 => Self::OilSlick,
            4 => Self::BoostPad,
            5 => Self::Barrier,
            6 => Self::StartFinish,
            7 => Self::Checkpoint,
            _ => Self::Barrier,
        }
    }

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
    ///
    ///
    /// `None` means the grid has not been loaded. That is the shipping state:
    /// the grids live on the disc and are installed into a shared buffer at
    /// level load, and this field is repointed at that buffer once (see
    /// `game::level_grid`). Keeping it optional rather than always pointing
    /// somewhere is what makes "not loaded" representable -- a stale grid from
    /// the previously-loaded level is far worse than none.
    ///
    /// `Some` is what the host test tools use, and what a level looks like
    /// before its data moves off the executable.
    pub tiles: Option<&'static [TrackTile]>,
    /// Authored racing-line control points in world space, in order. The core
    /// builds [`Route`](crate::Route) from these rather than from the checkpoint
    /// gates, so the AI follows the centreline instead of a gate-to-gate chord.
    /// Empty means "derive a fallback from the gates" (test fixtures only).
    pub route: &'static [Vec2],
    /// Half-width of the drivable corridor in world units. Runoff beyond it is
    /// open ground. Zero means "unknown", which [`Self::road_half_width`] treats
    /// as a single lane -- see that method for why the floor exists.
    pub half_width: u16,
}

/// How wide the car is, in world units, for anything that has to fit a car
/// across a corridor.
///
/// Half the drawn body plus a margin: the drawn body is 14 units wide and the
/// wheels reach 18, and a rival that puts its aim point within half a body of
/// the road edge is aiming at the kerb rather than at the racing line.
pub const CAR_HALF_WIDTH: i32 = 9;

impl TrackDef {
    /// Half-width of the drivable corridor in world units, with a floor.
    ///
    /// [`TrackDef::half_width`] is authored data and nothing constructs a
    /// `TrackDef`, so it can be zero -- hand-written test fixtures set it that
    /// way, and a cooker that forgot the field would emit zero for a real
    /// circuit. Zero used to mean "as wide as a whole tile" in the field's own
    /// doc comment while no such fallback existed anywhere, so every reader
    /// quietly invented its own behaviour for the same circuit. This is the one
    /// reader, and it reads a zero as the narrowest legal road.
    ///
    /// The floor is [`MIN_ROAD_HALF_WIDTH`], one lane. Reading the field as a
    /// floor rather than a value is what lets a circuit be one or two lanes
    /// wide: a caller asks how much room there is, and a narrow circuit answers
    /// "less", instead of every caller assuming the width of the old wide roads.
    #[inline]
    pub fn road_half_width(&self) -> i32 {
        let authored = self.half_width as i32;
        if authored < MIN_ROAD_HALF_WIDTH {
            MIN_ROAD_HALF_WIDTH
        } else {
            authored
        }
    }

    /// How many lanes wide the corridor is, rounded down.
    ///
    /// The whole design vocabulary is lanes: `lane_count() == 1` is a single
    /// file technical circuit, `2` leaves room to pass. Fractional lanes are
    /// floored because a road cannot be *partly* a lane -- a car either fits
    /// across the corridor or it does not.
    #[inline]
    pub fn lane_count(&self) -> u8 {
        let lanes = self.road_half_width() * 2 / LANE_WIDTH;
        if lanes < 1 {
            1
        } else {
            lanes.min(u8::MAX as i32) as u8
        }
    }

    /// How far off the centreline a car may be placed and still be fully on the
    /// racing surface, in world units.
    ///
    /// This is the budget every lateral move in the game is clamped to: the
    /// AI's swerve, the starting grid's stagger. Clamping to it rather than to
    /// [`Self::road_half_width`] is the difference between a rival dodging into
    /// the kerb and a rival dodging down the road.
    ///
    /// Floors at zero, so a corridor too narrow to hold a car reports "there is
    /// no room" rather than a negative budget that callers would have to
    /// special-case.
    #[inline]
    pub fn lateral_room(&self) -> i32 {
        let room = self.road_half_width() - CAR_HALF_WIDTH;
        if room < 0 {
            0
        } else {
            room
        }
    }

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

    /// Whether the authored road half-width is one the engine can represent.
    ///
    /// Three checks, all of which have been violated by authoring rather than by
    /// a code defect, and none of which the compiler can catch: `half_width` is
    /// a world-unit count, so a fractional cell width compiles to a road a
    /// fraction of a cell narrower than the one that was rasterised; a width
    /// under [`MIN_ROAD_HALF_WIDTH`] is narrower than a car can sit in; and one
    /// over [`MAX_ROAD_HALF_WIDTH`] has its kerbs off the edge of the grid.
    ///
    /// Zero is *not* an error here. It is the documented "unknown" value that
    /// [`Self::road_half_width`] floors, and the hand-written test fixtures set
    /// it. Use [`Self::road_width_is_legal`] to decide whether a *cooked* circuit
    /// is publishable.
    #[inline]
    pub fn road_width_is_legal(&self) -> bool {
        self.half_width == 0 || self.half_width_is_legal()
    }

    /// See [`Self::road_width_is_legal`], for a width that is known to be set.
    #[inline]
    pub fn half_width_is_legal(&self) -> bool {
        let hw = self.half_width as i32;
        hw % TILE_SIZE == 0 && (MIN_ROAD_HALF_WIDTH..=MAX_ROAD_HALF_WIDTH).contains(&hw)
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

    /// The loaded grid, or an empty slice when none is installed.
    ///
    /// Empty rather than a fallback fill: every reader already treats a missing
    /// cell as [`Barrier`](TrackTile::Barrier), so an unloaded level reads as
    /// solid wall everywhere instead of as plausible terrain.
    #[inline]
    pub fn tile_grid(&self) -> &'static [TrackTile] {
        // Host tests read the same `LEVELS.BIN` the console does, installed once
        // on first use. Without this every test that reads terrain sees an empty
        // grid -- i.e. solid wall everywhere -- and fails for the wrong reason.
        #[cfg(test)]
        crate::test_grids::ensure_installed();
        self.tiles.unwrap_or(&[])
    }

    /// Bytes a grid of this level's dimensions occupies on the disc.
    #[inline]
    pub fn grid_bytes(&self) -> usize {
        self.width as usize * self.height as usize
    }

    /// Returns the raw tile at a grid coordinate, or `Barrier` when out of bounds
    /// or when no grid is loaded.
    #[inline]
    pub fn tile_at(&self, tx: u8, ty: u8) -> TrackTile {
        if tx >= self.width || ty >= self.height {
            return TrackTile::Barrier;
        }
        let idx = (ty as usize) * (self.width as usize) + (tx as usize);
        match self.tile_grid().get(idx) {
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
            tiles: Some(&TILES),
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
            (8_191, 127),  // 255 tiles
            (16_383, 255), // 511 tiles -- one below the wrap
            (16_384, 0),   // 512 tiles -> wrapped to 0
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

    // --- road width, in lanes ------------------------------------------------

    #[test]
    fn a_lane_is_a_whole_number_of_cells() {
        // Everything downstream -- the collision grid, the cooker, the editor --
        // quantises the road to cells, so a lane that is not a whole number of
        // cells would be a half-cell road that no two of them agree on.
        assert_eq!(LANE_WIDTH % TILE_SIZE, 0);
        assert_eq!(MIN_ROAD_HALF_WIDTH % TILE_SIZE, 0);
        assert_eq!(MIN_ROAD_HALF_WIDTH * 2, LANE_WIDTH);
    }

    #[test]
    fn road_half_width_floors_at_one_lane() {
        // Zero is the "unknown" value the field documents, and every reader has
        // to agree on what it means. It means the narrowest legal road.
        for zero in [0u16, 1, 16, (MIN_ROAD_HALF_WIDTH - 1) as u16] {
            let mut track = scratch_track(4);
            track.half_width = zero;
            assert_eq!(
                track.road_half_width(),
                MIN_ROAD_HALF_WIDTH,
                "half_width {zero}"
            );
        }
        // ...and a real width is passed through untouched, one lane and up.
        for lanes in 1..=4u8 {
            let mut track = scratch_track(4);
            track.half_width = (LANE_WIDTH * lanes as i32 / 2) as u16;
            assert_eq!(track.road_half_width(), LANE_WIDTH * lanes as i32 / 2);
            assert_eq!(track.lane_count(), lanes);
        }
    }

    #[test]
    fn lane_count_is_never_below_one() {
        for zero in [0u16, 8, (MIN_ROAD_HALF_WIDTH - 1) as u16] {
            let mut track = scratch_track(4);
            track.half_width = zero;
            assert_eq!(track.lane_count(), 1, "half_width {zero}");
        }
    }

    #[test]
    fn lateral_room_leaves_the_car_on_the_road() {
        // The point of the accessor: whatever the road is, a swerve clamped to
        // this budget leaves the car on the racing surface.
        for lanes in 1..=4u8 {
            let mut track = scratch_track(4);
            let half = LANE_WIDTH * lanes as i32 / 2;
            track.half_width = half as u16;
            let room = track.lateral_room();
            assert_eq!(room, half - CAR_HALF_WIDTH, "{lanes} lanes");
            assert!(
                room + CAR_HALF_WIDTH <= half,
                "{lanes} lanes: the car overhangs the road"
            );
            // A one-lane road is the tight case and must still be positive.
            assert!(room > 0, "{lanes} lanes has no room to manoeuvre");
        }
    }

    #[test]
    fn lateral_room_never_goes_negative() {
        // A road narrower than a car is a room of zero, not a negative budget
        // every caller would have to special-case.
        let mut track = scratch_track(4);
        track.half_width = (CAR_HALF_WIDTH as u16) / 2;
        assert_eq!(track.road_half_width(), MIN_ROAD_HALF_WIDTH);
        assert!(track.lateral_room() >= 0);
    }

    #[test]
    fn half_width_legality_rejects_what_the_cooker_cannot_represent() {
        let mut track = scratch_track(4);
        // Whole cells only.
        for good in [
            MIN_ROAD_HALF_WIDTH as u16,
            (LANE_WIDTH * 3 / 2) as u16,
            MAX_ROAD_HALF_WIDTH as u16,
        ] {
            track.half_width = good;
            assert!(track.half_width_is_legal(), "{good} should be legal");
            assert!(track.road_width_is_legal());
        }
        for bad in [
            1u16,                                     // sub-cell
            (MIN_ROAD_HALF_WIDTH + 1) as u16,         // sub-cell, just above the floor
            (MIN_ROAD_HALF_WIDTH - 1) as u16,         // narrower than one lane
            (MAX_ROAD_HALF_WIDTH + TILE_SIZE) as u16, // wider than the grid
            u16::MAX,
        ] {
            track.half_width = bad;
            assert!(!track.half_width_is_legal(), "{bad} should be rejected");
            // ...but zero stays acceptable through the unknown-value door.
            assert_eq!(track.road_width_is_legal(), bad == 0);
        }
    }

    #[test]
    fn every_shipped_track_is_a_legal_road_width() {
        for track in ALL_TRACKS.iter() {
            assert!(
                track.road_width_is_legal(),
                "{}: half_width {} is not a whole-cell width in [{}, {}]",
                track.name,
                track.half_width,
                MIN_ROAD_HALF_WIDTH,
                MAX_ROAD_HALF_WIDTH,
            );
            assert!(
                track.lane_count() >= 1,
                "{}: reports {} lanes",
                track.name,
                track.lane_count()
            );
            assert!(
                track.lateral_room() > 0,
                "{}: no room to manoeuvre",
                track.name
            );
        }
    }

    #[test]
    fn every_shipped_track_is_at_least_as_wide_as_one_lane() {
        // The one bound that has to hold for every published circuit, whatever
        // its design intent: you cannot sell a circuit a car cannot drive.
        for track in ALL_TRACKS.iter() {
            assert!(
                track.half_width >= MIN_ROAD_HALF_WIDTH as u16,
                "{}: {} lanes is below the one-lane floor",
                track.name,
                track.road_half_width() / LANE_WIDTH * 2,
            );
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
    }

    #[test]
    fn disc_bytes_round_trip_through_every_variant() {
        // The cooker writes the discriminant straight into the asset, so this
        // mapping is a wire format between the disc and the engine. If a
        // variant is ever renumbered without updating this, a loaded grid
        // silently turns into different terrain.
        let all = [
            TrackTile::Tarmac,
            TrackTile::Curb,
            TrackTile::OffRoad,
            TrackTile::OilSlick,
            TrackTile::BoostPad,
            TrackTile::Barrier,
            TrackTile::StartFinish,
            TrackTile::Checkpoint,
        ];
        for tile in all {
            assert_eq!(TrackTile::from_disc_byte(tile as u8), tile, "{tile:?}");
        }
        // Discriminants are consecutive from 0, which is what makes the decode
        // a bounds check rather than a lookup.
        assert_eq!(all.len(), 8);
    }

    #[test]
    fn an_unmapped_disc_byte_becomes_barrier_not_an_invalid_variant() {
        // A corrupt or truncated asset must fail as solid wall. Reading it as a
        // raw discriminant instead would be undefined behaviour in every
        // `match` downstream.
        for byte in [8u8, 9, 200, 255] {
            assert_eq!(TrackTile::from_disc_byte(byte), TrackTile::Barrier);
        }
    }

    #[test]
    fn an_unloaded_grid_reads_as_barrier_everywhere() {
        // `tiles: None` is the shipping state between loads. It must not read
        // as the previous level's terrain, and must not panic.
        let mut t = scratch_track(4);
        t.tiles = None;
        assert_eq!(t.tile_grid().len(), 0);
        assert_eq!(t.tile_at(0, 0), TrackTile::Barrier);
        assert_eq!(t.tile_at(2, 3), TrackTile::Barrier);
        // Out of bounds is unchanged by any of this.
        assert_eq!(t.tile_at(9, 9), TrackTile::Barrier);
    }

    // Boost pads and oil slicks are drivable but are not `is_road`, which is
    // why navigation needs this predicate at all.
    #[test]
    fn boost_and_oil_tiles_are_line_tiles_but_not_road() {
        let mut pad = scratch_track(4);
        pad.tiles = Some(Box::leak(Box::new([TrackTile::BoostPad; 100])));
        assert!(!pad.is_road_at(0, 0));
        assert!(pad.is_line_tile(0, 0));
        let mut slick = scratch_track(4);
        slick.tiles = Some(Box::leak(Box::new([TrackTile::OilSlick; 100])));
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
