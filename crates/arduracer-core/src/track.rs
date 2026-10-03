//! Track definitions, tile surfaces, and track geometry.
//!
//! A [`TrackDef`] is a static, `no_std`-friendly description of one circuit:
//! a tile grid of [`TrackTile`] surfaces, a start/finish gate, an ordered
//! checkpoint array, and the par times used for medal evaluation.
//!
//! Tile *surfaces* (rather than raw level-art indices) are baked by
//! `tools/track_cook/convert_levels.py` so that collision, rendering, audio and
//! the AI all agree on what a tile means without duplicating lookup tables.

use crate::math::{Fixed, Vec2};
use crate::surface::SurfaceType;
use crate::timing::{CheckpointGate, ParTimes};

/// Maximum checkpoints per track.
pub const MAX_TRACK_CHECKPOINTS: usize = 16;
/// Maximum dimension of a track grid (up to 32x32 tiles).
pub const MAX_TRACK_DIM: usize = 32;
/// World-space size of a single track tile.
pub const TILE_SIZE: i32 = 64;

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
}

impl TrackDef {
    /// Total world-space width of the circuit in world units.
    pub const fn world_width(&self) -> i32 {
        self.width as i32 * TILE_SIZE
    }

    /// Total world-space height of the circuit in world units.
    pub const fn world_height(&self) -> i32 {
        self.height as i32 * TILE_SIZE
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

    /// Converts a world position into the containing tile column.
    #[inline]
    pub fn tile_x_of(world_x: Fixed) -> u8 {
        // Negative positions saturate to 0 so callers never see wrapped u8s.
        let v = world_x.to_int();
        if v <= 0 {
            0
        } else {
            (v / TILE_SIZE) as u8
        }
    }

    /// Converts a world position into the containing tile row.
    #[inline]
    pub fn tile_y_of(world_y: Fixed) -> u8 {
        let v = world_y.to_int();
        if v <= 0 {
            0
        } else {
            (v / TILE_SIZE) as u8
        }
    }

    /// World-space centre of a tile centre coordinate.
    #[inline]
    pub fn tile_centre(tile: u8) -> Fixed {
        Fixed::from_int(tile as i32 * TILE_SIZE + TILE_SIZE / 2)
    }

    /// Active checkpoint gates as a slice.
    #[inline]
    pub fn checkpoint_slice(&self) -> &[CheckpointGate] {
        &self.checkpoints[..self.checkpoint_count as usize]
    }

    /// Number of nodes in a full circuit: every checkpoint plus the start/finish.
    ///
    /// AI navigation walks `0 ..= route_len() - 1`, wrapping back to 0, so the
    /// start/finish is part of the racing line rather than a shortcut the drivers
    /// skip.
    #[inline]
    pub fn route_len(&self) -> usize {
        self.checkpoint_count as usize + 1
    }

    /// The `idx`-th node of the circuit (checkpoints in driving order, then the
    /// start/finish gate). `idx` wraps modulo [`TrackDef::route_len`].
    #[inline]
    pub fn route_node(&self, idx: usize) -> CheckpointGate {
        let n = self.checkpoint_count as usize;
        if idx >= n {
            self.start_gate
        } else {
            self.checkpoints[idx]
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
    pub fn route_length(&self) -> i32 {
        if self.checkpoint_count == 0 {
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
}
