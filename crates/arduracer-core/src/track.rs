//! Track definitions, tilemaps, and track geometry.

use crate::math::Vec2;
use crate::surface::SurfaceType;
use crate::timing::{CheckpointGate, ParTimes};

/// Maximum checkpoints per track.
pub const MAX_TRACK_CHECKPOINTS: usize = 16;
/// Maximum dimension of a track grid (up to 32x32 tiles).
pub const MAX_TRACK_DIM: usize = 32;

/// A complete racetrack definition.
#[derive(Clone, Debug)]
pub struct TrackDef {
    pub name: &'static str,
    pub width: u8,
    pub height: u8,
    pub start_pos: Vec2,
    pub start_heading: u16,
    pub par_times: ParTimes,
    pub checkpoint_count: u8,
    pub checkpoints: [CheckpointGate; MAX_TRACK_CHECKPOINTS],
    pub tiles: &'static [u8],
}

impl TrackDef {
    /// Gets the surface type for a tile coordinate.
    pub fn surface_at(&self, tx: u8, ty: u8) -> SurfaceType {
        if tx >= self.width || ty >= self.height {
            return SurfaceType::Barrier;
        }
        let idx = (ty as usize) * (self.width as usize) + (tx as usize);
        if idx >= self.tiles.len() {
            return SurfaceType::Barrier;
        }
        match self.tiles[idx] {
            20 => SurfaceType::OffRoad,      // Grass / Dirt border
            21..=23 => SurfaceType::Barrier, // Solid Barriers / Walls / Water
            _ => SurfaceType::Tarmac, // Road surfaces, start lines (24, 25), checkpoints (26, 27)
        }
    }
}
