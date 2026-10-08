//! Real-time CD streaming engine for full-sized cities and visual data.
//!
//! # Architecture
//!
//! A full-sized city (e.g. 1024x1024 cells, 32,768 x 32,768 world units) is orders
//! of magnitude larger than PSX main RAM (2 MB total, < 1 MB static budget) and VRAM
//! (1 MB). Consequently, the world cannot be resident all at once.
//!
//! This module implements the real-time CD streaming system:
//!
//! 1. **Disc Sector Layout**: World chunks are packed into 2048-byte CD sectors
//!    conforming to the PSX CD-ROM streaming architecture (`psx-pack` `PSOXWPAK`).
//! 2. **Fixed Resident Working Set**: The [`CityStreamer`] maintains a bounded LRU
//!    cache of resident slots (up to [`MAX_RESIDENT_SLOTS`] slots), consuming a
//!    strictly fixed, bounded RAM footprint (e.g. ~16 KB for surface grids).
//! 3. **Lookahead Prefetching**: Predicts required chunks along the vehicle's heading
//!    and velocity vector, streaming them off the disc before the vehicle crosses
//!    chunk boundaries.
//! 4. **8-Bit Visual Texture Streaming**: Concurrently streams 8-bit colour texture
//!    data for each active chunk, enabling smooth, uninterrupted presentation in
//!    real-time without hitching.

use crate::city::{CityChunkData, CityDef};
use crate::math::{Fixed, Vec2};
use crate::surface::SurfaceType;
use crate::track::TrackTile;

/// Standard CD sector size in user data bytes (Mode 2 Form 1).
pub const CD_SECTOR_BYTES: usize = 2048;

/// Maximum number of chunks resident in RAM at any given time.
pub const MAX_RESIDENT_SLOTS: usize = 16;

/// Default resident cache capacity in chunks (e.g. 12 or 16 chunks = ~4x4 working set).
pub const DEFAULT_RESIDENT_CAPACITY: usize = 12;

/// Streaming error types.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum StreamError {
    /// Requested chunk is not present in the CD pack.
    ChunkNotFound,
    /// Sector read failed or corrupted checksum.
    ReadError,
    /// Resident cache is completely full of pinned chunks.
    CacheSaturated,
}

/// Abstract CD stream source (allows testing on host and execution on PSX hardware).
pub trait CdStreamSource {
    /// Fetches a chunk's data from the CD stream.
    fn read_chunk(&mut self, chunk_id: u32) -> Result<CityChunkData, StreamError>;
    /// Whether a chunk exists on the disc.
    fn has_chunk(&self, chunk_id: u32) -> bool;
    /// Total sectors read during the session.
    fn sectors_read(&self) -> u64;
    /// Total bytes transferred from the disc.
    fn bytes_transferred(&self) -> u64;
}

/// Maximum chunks supported by the host memory-simulated CD stream source.
#[cfg(test)]
pub const MAX_SIM_CHUNKS: usize = 128;

/// In-memory simulated CD stream source for testing and host simulation.
#[cfg(test)]
#[derive(Clone, Debug)]
pub struct MemoryCdStreamSource {
    chunks: std::vec::Vec<(u32, CityChunkData)>,
    sectors_read_count: u64,
    bytes_transferred_count: u64,
}

#[cfg(test)]
impl Default for MemoryCdStreamSource {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
impl MemoryCdStreamSource {
    /// Creates a new empty simulated CD stream source.
    pub fn new() -> Self {
        Self {
            chunks: std::vec::Vec::new(),
            sectors_read_count: 0,
            bytes_transferred_count: 0,
        }
    }

    /// Stores a chunk into the simulated CD disc pack.
    pub fn insert_chunk(&mut self, chunk_id: u32, chunk: CityChunkData) {
        if let Some(pos) = self.chunks.iter().position(|(id, _)| *id == chunk_id) {
            self.chunks[pos] = (chunk_id, chunk);
        } else {
            self.chunks.push((chunk_id, chunk));
        }
    }
}

#[cfg(test)]
impl CdStreamSource for MemoryCdStreamSource {
    fn read_chunk(&mut self, chunk_id: u32) -> Result<CityChunkData, StreamError> {
        if let Some((_, chunk)) = self.chunks.iter().find(|(id, _)| *id == chunk_id) {
            let chunk_data_bytes = chunk.tiles.len() + chunk.visual_8bpp.len();
            let sectors = chunk_data_bytes.div_ceil(CD_SECTOR_BYTES);
            self.sectors_read_count += sectors as u64;
            self.bytes_transferred_count += chunk_data_bytes as u64;
            Ok(*chunk)
        } else {
            Err(StreamError::ChunkNotFound)
        }
    }

    fn has_chunk(&self, chunk_id: u32) -> bool {
        self.chunks.iter().any(|(id, _)| *id == chunk_id)
    }

    fn sectors_read(&self) -> u64 {
        self.sectors_read_count
    }

    fn bytes_transferred(&self) -> u64 {
        self.bytes_transferred_count
    }
}

/// A resident cache slot holding a loaded chunk.
#[derive(Copy, Clone, Debug)]
struct ResidentSlot {
    chunk_id: u32,
    chunk: CityChunkData,
    last_access: u64,
    pinned: bool,
}

/// Real-time streaming manager for unbounded cities (pure `core`, `#![no_std]`-safe).
pub struct CityStreamer<S: CdStreamSource> {
    city: CityDef,
    source: S,
    slots: [Option<ResidentSlot>; MAX_RESIDENT_SLOTS],
    capacity: usize,
    access_clock: u64,
    cache_hits: u64,
    cache_misses: u64,
    last_center_chunk: (usize, usize),
}

impl<S: CdStreamSource> CityStreamer<S> {
    /// Creates a streamer for the given city and CD source with a bounded slot capacity.
    pub fn new(city: CityDef, source: S, capacity: usize) -> Self {
        let cap = capacity.min(MAX_RESIDENT_SLOTS);
        Self {
            city,
            source,
            slots: [None; MAX_RESIDENT_SLOTS],
            capacity: cap,
            access_clock: 0,
            cache_hits: 0,
            cache_misses: 0,
            last_center_chunk: (usize::MAX, usize::MAX),
        }
    }

    /// Access to the underlying city definition.
    pub fn city(&self) -> &CityDef {
        &self.city
    }

    /// Number of chunks currently resident in RAM.
    pub fn resident_count(&self) -> usize {
        let mut count = 0;
        for slot in &self.slots {
            if slot.is_some() {
                count += 1;
            }
        }
        count
    }

    /// Cache hit count.
    pub fn cache_hits(&self) -> u64 {
        self.cache_hits
    }

    /// Cache miss count (required reading from CD).
    pub fn cache_misses(&self) -> u64 {
        self.cache_misses
    }

    /// Total sectors read by the CD streaming source.
    pub fn sectors_streamed(&self) -> u64 {
        self.source.sectors_read()
    }

    /// Total bytes transferred by the CD streaming source.
    pub fn bytes_streamed(&self) -> u64 {
        self.source.bytes_transferred()
    }

    /// Updates the streaming working set based on vehicle position, heading, and speed.
    pub fn update(&mut self, pos: Vec2, heading: u16, speed: Fixed) {
        self.access_clock += 1;
        let (cx, cy) = self.city.world_to_chunk(pos.x, pos.y);
        self.last_center_chunk = (cx, cy);

        // Fixed buffer of needed chunks
        let mut needed_chunks = [(0usize, 0usize); 16];
        let mut needed_count = 0usize;

        let min_x = cx.saturating_sub(1);
        let max_x = (cx + 1).min(self.city.chunks_x().saturating_sub(1));
        let min_y = cy.saturating_sub(1);
        let max_y = (cy + 1).min(self.city.chunks_y().saturating_sub(1));

        for y in min_y..=max_y {
            for x in min_x..=max_x {
                if needed_count < needed_chunks.len() {
                    needed_chunks[needed_count] = (x, y);
                    needed_count += 1;
                }
            }
        }

        // Lookahead prefetching along vehicle velocity
        if speed > Fixed::from_int(1) {
            let fwd_x = crate::math::sin(heading);
            let fwd_y = -crate::math::cos(heading);

            let lookahead_steps = if speed > Fixed::from_int(2) { 2 } else { 1 };
            for step in 1..=lookahead_steps {
                let px = (cx as i32 + (fwd_x.to_int() * step))
                    .clamp(0, self.city.chunks_x() as i32 - 1) as usize;
                let py = (cy as i32 + (fwd_y.to_int() * step))
                    .clamp(0, self.city.chunks_y() as i32 - 1) as usize;

                let already_included = needed_chunks[..needed_count].contains(&(px, py));
                if !already_included && needed_count < needed_chunks.len() {
                    needed_chunks[needed_count] = (px, py);
                    needed_count += 1;
                }
            }
        }

        // Unpin all slots
        for s in self.slots.iter_mut().flatten() {
            s.pinned = false;
        }

        // Ensure all needed chunks are resident
        for &(nx, ny) in &needed_chunks[..needed_count] {
            let chunk_id = self.city.chunk_id(nx, ny);
            self.ensure_chunk_resident(chunk_id);
        }
    }

    /// Ensures a specific chunk is loaded in the resident cache and pins it.
    fn ensure_chunk_resident(&mut self, chunk_id: u32) {
        // Check if already in cache
        for s in self.slots.iter_mut().flatten() {
            if s.chunk_id == chunk_id {
                s.last_access = self.access_clock;
                s.pinned = true;
                self.cache_hits += 1;
                return;
            }
        }

        // Cache miss: load chunk from CD source
        self.cache_misses += 1;
        if let Ok(chunk) = self.source.read_chunk(chunk_id) {
            let new_slot = ResidentSlot {
                chunk_id,
                chunk,
                last_access: self.access_clock,
                pinned: true,
            };

            // Look for empty slot within capacity
            for i in 0..self.capacity {
                if self.slots[i].is_none() {
                    self.slots[i] = Some(new_slot);
                    return;
                }
            }

            // Evict least-recently-used non-pinned chunk
            let mut lru_idx = None;
            let mut oldest_access = u64::MAX;

            for i in 0..self.capacity {
                if let Some(s) = &self.slots[i] {
                    if !s.pinned && s.last_access < oldest_access {
                        oldest_access = s.last_access;
                        lru_idx = Some(i);
                    }
                }
            }

            if let Some(idx) = lru_idx {
                self.slots[idx] = Some(new_slot);
            }
        }
    }

    /// Real-time collision query: returns the surface tile at world position `(x, y)`.
    pub fn tile_at(&mut self, x: Fixed, y: Fixed) -> TrackTile {
        let (cx, cy) = self.city.world_to_chunk(x, y);
        let chunk_id = self.city.chunk_id(cx, cy);

        for s in self.slots.iter_mut().flatten() {
            if s.chunk_id == chunk_id {
                s.last_access = self.access_clock;
                self.cache_hits += 1;
                let (tx, ty) = self.city.world_to_cell(x, y);
                let (lx, ly) = self.city.cell_to_chunk_local(tx, ty);
                return s.chunk.tile_at(lx, ly, self.city.chunk_dim);
            }
        }

        // Immediate on-demand stream if missed
        self.cache_misses += 1;
        if let Ok(chunk) = self.source.read_chunk(chunk_id) {
            let (tx, ty) = self.city.world_to_cell(x, y);
            let (lx, ly) = self.city.cell_to_chunk_local(tx, ty);
            let tile = chunk.tile_at(lx, ly, self.city.chunk_dim);

            let new_slot = ResidentSlot {
                chunk_id,
                chunk,
                last_access: self.access_clock,
                pinned: false,
            };

            for i in 0..self.capacity {
                if self.slots[i].is_none() {
                    self.slots[i] = Some(new_slot);
                    return tile;
                }
            }

            tile
        } else {
            TrackTile::Barrier
        }
    }

    /// Real-time physics query: surface type at world position `(x, y)`.
    #[inline]
    pub fn surface_at(&mut self, x: Fixed, y: Fixed) -> SurfaceType {
        self.tile_at(x, y).surface()
    }

    /// Real-time query: whether the position blocks the car outright.
    #[inline]
    pub fn is_solid(&mut self, x: Fixed, y: Fixed) -> bool {
        self.tile_at(x, y).is_solid()
    }

    /// Real-time query: whether the position is part of the drivable racing surface.
    #[inline]
    pub fn is_road(&mut self, x: Fixed, y: Fixed) -> bool {
        self.tile_at(x, y).is_road()
    }

    /// Real-time visual query: retrieves the 8-bit colour texel buffer for chunk `(cx, cy)`.
    pub fn visual_chunk_at(&self, cx: usize, cy: usize) -> Option<&'static [u8]> {
        let chunk_id = self.city.chunk_id(cx, cy);
        for s in self.slots.iter().flatten() {
            if s.chunk_id == chunk_id {
                return Some(s.chunk.visual_8bpp);
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn city_streaming_maintains_bounded_residency() {
        let city = CityDef::new(
            "MegaCity",
            1024,
            1024,
            32,
            8,
            -378600000,
            1449300000,
            -378050000,
            1449950000,
            &[],
            [(0, 0, 0); 256],
        );

        let mut cd = MemoryCdStreamSource::new();

        for cy in 0..8 {
            for cx in 0..8 {
                let id = city.chunk_id(cx, cy);
                let mut chunk = CityChunkData::empty(cx as u16, cy as u16);
                for x in 0..32 {
                    chunk.set_tile(x, 16, 32, TrackTile::Tarmac);
                }
                cd.insert_chunk(id, chunk);
            }
        }

        let capacity = 12;
        let mut streamer = CityStreamer::new(city, cd, capacity);

        let mut x = 500i32;
        while x < 3500 {
            let pos = Vec2::new(Fixed::from_int(x), Fixed::from_int(512));
            streamer.update(pos, 1024, Fixed::from_int(3));

            let road_pos_y = Fixed::from_int(512);
            assert!(streamer.is_road(Fixed::from_int(x), road_pos_y));

            assert!(
                streamer.resident_count() <= capacity,
                "Resident chunks ({}) exceeded capacity ({})!",
                streamer.resident_count(),
                capacity
            );

            x += 200;
        }

        assert!(streamer.sectors_streamed() > 0);
        assert!(streamer.bytes_streamed() > 0);
        assert!(streamer.cache_hits() > 0);
    }
}
