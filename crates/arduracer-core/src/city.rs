//! City definitions, spatial chunking, and multi-race urban layout.
//!
//! A city is an unbounded, arbitrary-sized world map (e.g. 512x512, 1024x1024,
//! or larger) designed to be streamed from CD-ROM. Unlike single isolated tracks,
//! a city contains multiple districts and multiple authored circuits/races
//! (`CityRace`) interconnected by streets and avenues.
//!
//! Memory discipline:
//! The entire city map is partitioned into spatial chunks (e.g. 32x32 cells).
//! Collision tiles and 8-bit visual textures are streamed in real-time by the
//! engine's CD streamer ([`crate::streaming::CityStreamer`]), ensuring only a
//! small working set is resident in PSX RAM and VRAM at any given moment.

use crate::math::{Fixed, Vec2};
use crate::timing::{CheckpointGate, ParTimes};
use crate::track::{TrackTile, TILE_SIZE};

/// Default spatial chunk dimension in surface cells (32x32 cells).
/// At 32 world units per cell, one chunk covers 1024 x 1024 world units.
pub const DEFAULT_CHUNK_DIM: usize = 32;

/// Number of surface cells in one standard chunk (32 x 32 = 1024).
pub const CHUNK_CELLS: usize = DEFAULT_CHUNK_DIM * DEFAULT_CHUNK_DIM;

/// Default visual texels per surface cell (8 texels per cell = 256x256 texels per chunk).
/// Exactly matches one 256x256 texture page in PSX VRAM.
pub const DEFAULT_TEXELS_PER_CELL: usize = 8;

/// Number of 8-bit visual texels in one standard chunk (256 x 256 = 65536).
pub const CHUNK_TEXELS: usize =
    (DEFAULT_CHUNK_DIM * DEFAULT_TEXELS_PER_CELL) * (DEFAULT_CHUNK_DIM * DEFAULT_TEXELS_PER_CELL);

/// A race / circuit situated within a city.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CityRace {
    /// Name of this race.
    pub name: &'static str,
    /// District / neighbourhood of the city where the race takes place.
    pub district: &'static str,
    /// Exact spawn position in city world coordinates.
    pub start_pos: Vec2,
    /// Initial vehicle heading in BAMs (0 = North, 1024 = East).
    pub start_heading: u16,
    /// Start / finish gate.
    pub start_gate: CheckpointGate,
    /// Par times for medal evaluation.
    pub par_times: ParTimes,
    /// Checkpoints in driving order.
    pub checkpoints: &'static [CheckpointGate],
    /// Authored racing line control points.
    pub route: &'static [Vec2],
    /// Half-width of the racing corridor in world units.
    pub half_width: u16,
}

/// Metadata and layout definition for a full-sized city.
#[derive(Clone, Debug)]
pub struct CityDef {
    /// Display name of the city (e.g. "Neo Yokohama", "Metro City").
    pub name: &'static str,
    /// Total width of the city in surface cells.
    pub width_cells: u32,
    /// Total height of the city in surface cells.
    pub height_cells: u32,
    /// Dimension of each chunk in cells (e.g. 32).
    pub chunk_dim: usize,
    /// Texels per cell for the 8-bit visual texture (e.g. 8).
    pub texels_per_cell: usize,
    /// Races and circuits authored inside this city.
    pub races: &'static [CityRace],
    /// 256-entry 8-bit colour palette (CLUT) for the city's visual textures.
    pub clut: [(u8, u8, u8); 256],
}

impl CityDef {
    /// Creates a new city definition.
    pub const fn new(
        name: &'static str,
        width_cells: u32,
        height_cells: u32,
        chunk_dim: usize,
        texels_per_cell: usize,
        races: &'static [CityRace],
        clut: [(u8, u8, u8); 256],
    ) -> Self {
        Self {
            name,
            width_cells,
            height_cells,
            chunk_dim,
            texels_per_cell,
            races,
            clut,
        }
    }

    /// Number of chunk columns across the city.
    #[inline]
    pub fn chunks_x(&self) -> usize {
        (self.width_cells as usize).div_ceil(self.chunk_dim)
    }

    /// Number of chunk rows down the city.
    #[inline]
    pub fn chunks_y(&self) -> usize {
        (self.height_cells as usize).div_ceil(self.chunk_dim)
    }

    /// Total number of chunks in the entire city.
    #[inline]
    pub fn total_chunks(&self) -> usize {
        self.chunks_x() * self.chunks_y()
    }

    /// Total city width in world units.
    #[inline]
    pub fn world_width(&self) -> i32 {
        self.width_cells as i32 * TILE_SIZE
    }

    /// Total city height in world units.
    #[inline]
    pub fn world_height(&self) -> i32 {
        self.height_cells as i32 * TILE_SIZE
    }

    /// World-space width of a single chunk.
    #[inline]
    pub fn chunk_world_size(&self) -> i32 {
        self.chunk_dim as i32 * TILE_SIZE
    }

    /// Texel dimension of a single chunk (e.g. 32 * 8 = 256 texels).
    #[inline]
    pub fn chunk_texel_size(&self) -> usize {
        self.chunk_dim * self.texels_per_cell
    }

    /// Unique linear ID for a chunk coordinate `(cx, cy)`.
    #[inline]
    pub fn chunk_id(&self, cx: usize, cy: usize) -> u32 {
        (cy * self.chunks_x() + cx) as u32
    }

    /// Maps a linear chunk ID back to chunk coordinates `(cx, cy)`.
    #[inline]
    pub fn chunk_coords(&self, chunk_id: u32) -> (usize, usize) {
        let id = chunk_id as usize;
        let cx = id % self.chunks_x();
        let cy = id / self.chunks_x();
        (cx, cy)
    }

    /// Converts a world-space coordinate to the containing chunk coordinate `(cx, cy)`.
    #[inline]
    pub fn world_to_chunk(&self, x: Fixed, y: Fixed) -> (usize, usize) {
        let ux = x.to_int().max(0);
        let uy = y.to_int().max(0);
        let chunk_world = self.chunk_world_size().max(1);
        let cx = (ux / chunk_world) as usize;
        let cy = (uy / chunk_world) as usize;
        (
            cx.min(self.chunks_x().saturating_sub(1)),
            cy.min(self.chunks_y().saturating_sub(1)),
        )
    }

    /// Converts a world-space coordinate to the containing tile cell coordinate `(tx, ty)`.
    #[inline]
    pub fn world_to_cell(&self, x: Fixed, y: Fixed) -> (u32, u32) {
        let ux = x.to_int().max(0);
        let uy = y.to_int().max(0);
        let tx = (ux / TILE_SIZE) as u32;
        let ty = (uy / TILE_SIZE) as u32;
        (
            tx.min(self.width_cells.saturating_sub(1)),
            ty.min(self.height_cells.saturating_sub(1)),
        )
    }

    /// Returns the cell offset `(lx, ly)` inside its containing chunk.
    #[inline]
    pub fn cell_to_chunk_local(&self, tx: u32, ty: u32) -> (usize, usize) {
        (
            (tx as usize) % self.chunk_dim,
            (ty as usize) % self.chunk_dim,
        )
    }
}

/// A spatial city chunk containing collision/surface tiles.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct CityChunkData {
    /// Chunk X coordinate.
    pub cx: u16,
    /// Chunk Y coordinate.
    pub cy: u16,
    /// Surface cells: exactly 1024 entries (`CHUNK_CELLS`).
    pub tiles: [TrackTile; CHUNK_CELLS],
    /// Visual 8bpp data slice (points to resident sector or buffer).
    pub visual_8bpp: &'static [u8],
}

impl CityChunkData {
    /// Builds an empty chunk filled with barrier tiles.
    pub const fn empty(cx: u16, cy: u16) -> Self {
        Self {
            cx,
            cy,
            tiles: [TrackTile::Barrier; CHUNK_CELLS],
            visual_8bpp: &[],
        }
    }

    /// Reads the tile at chunk-local coordinate `(lx, ly)`.
    #[inline]
    pub fn tile_at(&self, lx: usize, ly: usize, chunk_dim: usize) -> TrackTile {
        if lx < chunk_dim && ly < chunk_dim {
            let idx = ly * chunk_dim + lx;
            if idx < self.tiles.len() {
                self.tiles[idx]
            } else {
                TrackTile::Barrier
            }
        } else {
            TrackTile::Barrier
        }
    }

    /// Sets the tile at chunk-local coordinate `(lx, ly)`.
    #[inline]
    pub fn set_tile(&mut self, lx: usize, ly: usize, chunk_dim: usize, tile: TrackTile) {
        if lx < chunk_dim && ly < chunk_dim {
            let idx = ly * chunk_dim + lx;
            if idx < self.tiles.len() {
                self.tiles[idx] = tile;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn city_layout_and_chunk_maths() {
        let city = CityDef::new(
            "Metropolis",
            1024,
            1024,
            DEFAULT_CHUNK_DIM,
            DEFAULT_TEXELS_PER_CELL,
            &[],
            [(0, 0, 0); 256],
        );

        assert_eq!(city.chunks_x(), 32);
        assert_eq!(city.chunks_y(), 32);
        assert_eq!(city.total_chunks(), 1024);
        assert_eq!(city.world_width(), 1024 * 32);
        assert_eq!(city.chunk_world_size(), 1024);
        assert_eq!(city.chunk_texel_size(), 256);

        let origin = city.world_to_chunk(Fixed::ZERO, Fixed::ZERO);
        assert_eq!(origin, (0, 0));

        let pt = city.world_to_chunk(Fixed::from_int(1024), Fixed::from_int(2048));
        assert_eq!(pt, (1, 2));

        let id = city.chunk_id(1, 2);
        assert_eq!(id, 2 * 32 + 1);
        assert_eq!(city.chunk_coords(id), (1, 2));
    }
}
