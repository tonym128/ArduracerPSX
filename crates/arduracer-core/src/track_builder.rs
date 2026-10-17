//! TrackMania-Style Modular Track Builder & Custom Track Serialization.
//!
//! Provides a 20x20 modular block grid (80x80 tile world = 2560x2560 units)
//! supporting 12 snapped TrackMania block types:
//! Start Grid, Straight, 90° Turn, Hairpin, Chicane, Dirt Section, Ice Bobsleigh,
//! Jump Ramp, Bridge Overpass, Free-Wheel (Engine-Off), Reactor Boost, and Checkpoint Gate.
//!
//! Compact serialization packs a complete custom track (metadata, 64 placed blocks,
//! author validation time, and CRC checksum) into exactly 256 bytes, enabling
//! 4 custom tracks to fit inside a single standard PS1 memory card block.

use crate::math::{Vec2, ANGLE_180, ANGLE_90};
use crate::save::crc16;
use crate::timing::{CheckpointGate, ParTimes};
use crate::track::{TrackDef, TrackTile, MAX_TRACK_CHECKPOINTS, TILE_SIZE};

/// Magic header for serialized custom tracks ("TMTR" = TrackMania Track).
pub const CUSTOM_TRACK_MAGIC: [u8; 4] = *b"TMTR";
pub const CUSTOM_TRACK_VERSION: u8 = 1;

/// Grid dimensions for custom tracks:
/// 20x20 macro blocks = 80x80 physics tiles = 2560x2560 world units.
pub const CUSTOM_GRID_MACRO_DIM: usize = 20;
pub const CUSTOM_GRID_TILE_DIM: usize = 80;
pub const BLOCK_TILE_SIZE: usize = 4;
pub const TOTAL_CUSTOM_TILES: usize = CUSTOM_GRID_TILE_DIM * CUSTOM_GRID_TILE_DIM; // 6,400 tiles

/// Maximum number of placed blocks in one custom track.
pub const MAX_CUSTOM_BLOCKS: usize = 64;

/// Serialized binary size of one custom track definition.
pub const CUSTOM_TRACK_SERIALIZED_SIZE: usize = 256;

/// Total custom track slots stored on the memory card.
pub const TOTAL_CUSTOM_TRACK_SLOTS: usize = 4;

/// The 12 modular TrackMania block types.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
#[repr(u8)]
pub enum TrackBlockKind {
    #[default]
    StartGrid = 0,
    Straight = 1,
    Turn90 = 2,
    Hairpin = 3,
    Chicane = 4,
    DirtSection = 5,
    IceBobsleigh = 6,
    JumpRamp = 7,
    BridgeOverpass = 8,
    FreeWheel = 9,
    ReactorBoost = 10,
    CheckpointGate = 11,
}

impl TrackBlockKind {
    pub const COUNT: usize = 12;

    pub const fn from_u8(v: u8) -> Self {
        match v {
            0 => Self::StartGrid,
            1 => Self::Straight,
            2 => Self::Turn90,
            3 => Self::Hairpin,
            4 => Self::Chicane,
            5 => Self::DirtSection,
            6 => Self::IceBobsleigh,
            7 => Self::JumpRamp,
            8 => Self::BridgeOverpass,
            9 => Self::FreeWheel,
            10 => Self::ReactorBoost,
            11 => Self::CheckpointGate,
            _ => Self::Straight,
        }
    }

    pub const fn as_u8(self) -> u8 {
        self as u8
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::StartGrid => "START/FINISH",
            Self::Straight => "STRAIGHT",
            Self::Turn90 => "90 DEG TURN",
            Self::Hairpin => "HAIRPIN",
            Self::Chicane => "CHICANE",
            Self::DirtSection => "DIRT RALLY",
            Self::IceBobsleigh => "ICE BOBSLEIGH",
            Self::JumpRamp => "JUMP RAMP",
            Self::BridgeOverpass => "BRIDGE PASS",
            Self::FreeWheel => "FREE-WHEEL RED",
            Self::ReactorBoost => "REACTOR BOOST",
            Self::CheckpointGate => "CHECKPOINT",
        }
    }

    pub const fn is_checkpoint_or_start(self) -> bool {
        matches!(self, Self::StartGrid | Self::CheckpointGate)
    }
}

/// A single placed block on the 20x20 macro grid.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub struct PlacedBlock {
    /// Macro X coordinate (0..19).
    pub x: u8,
    /// Macro Y coordinate (0..19).
    pub y: u8,
    /// Block kind.
    pub kind: TrackBlockKind,
    /// Orientation: 0 = North (0°), 1 = East (90°), 2 = South (180°), 3 = West (270°).
    pub orientation: u8,
}

impl PlacedBlock {
    pub const fn new(x: u8, y: u8, kind: TrackBlockKind, orientation: u8) -> Self {
        Self {
            x,
            y,
            kind,
            orientation: orientation & 0x03,
        }
    }

    /// Packs into 3 bytes: `[x, y, (kind & 0x0F) | ((orientation & 0x03) << 4)]`.
    pub const fn pack(self) -> [u8; 3] {
        [
            self.x,
            self.y,
            (self.kind as u8 & 0x0F) | ((self.orientation & 0x03) << 4),
        ]
    }

    /// Unpacks from 3 bytes.
    pub const fn unpack(bytes: [u8; 3]) -> Self {
        Self {
            x: bytes[0],
            y: bytes[1],
            kind: TrackBlockKind::from_u8(bytes[2] & 0x0F),
            orientation: (bytes[2] >> 4) & 0x03,
        }
    }
}

/// Full custom track definition stored in memory card slot.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct CustomTrackDef {
    /// Track name (up to 16 ASCII characters, zero-padded).
    pub name: [u8; 16],
    /// Author validation lap time in ticks (0 = unvalidated draft).
    pub author_time_ticks: u32,
    /// Number of active placed blocks (1..=64).
    pub block_count: u8,
    /// Array of placed modular blocks.
    pub blocks: [PlacedBlock; MAX_CUSTOM_BLOCKS],
}

impl Default for CustomTrackDef {
    fn default() -> Self {
        let mut name = [0u8; 16];
        let default_name = b"CUSTOM ARENA";
        let mut i = 0;
        while i < default_name.len() && i < 16 {
            name[i] = default_name[i];
            i += 1;
        }

        Self {
            name,
            author_time_ticks: 0,
            block_count: 0,
            blocks: [PlacedBlock::default(); MAX_CUSTOM_BLOCKS],
        }
    }
}

/// Error returned when track compilation or validation fails.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum TrackBuildError {
    EmptyTrack,
    NoStartGrid,
    MultipleStartGrids,
    OutOfBounds,
    DuplicatePosition,
    CorruptData,
}

impl CustomTrackDef {
    /// Creates a new empty custom track with a given name.
    pub fn new(name_str: &str) -> Self {
        let mut name = [0u8; 16];
        let bytes = name_str.as_bytes();
        let len = bytes.len().min(16);
        name[..len].copy_from_slice(&bytes[..len]);

        Self {
            name,
            author_time_ticks: 0,
            block_count: 0,
            blocks: [PlacedBlock::default(); MAX_CUSTOM_BLOCKS],
        }
    }

    /// Track display name as string slice.
    pub fn name_str(&self) -> &str {
        let len = self.name.iter().position(|&b| b == 0).unwrap_or(16);
        core::str::from_utf8(&self.name[..len]).unwrap_or("CUSTOM")
    }

    /// Sets the track name.
    pub fn set_name(&mut self, name_str: &str) {
        self.name = [0u8; 16];
        let bytes = name_str.as_bytes();
        let len = bytes.len().min(16);
        self.name[..len].copy_from_slice(&bytes[..len]);
    }

    /// Whether this track has been validated by driving a complete clean lap.
    pub fn is_validated(&self) -> bool {
        self.author_time_ticks > 0
    }

    /// Adds or updates a block on the macro grid.
    pub fn place_block(
        &mut self,
        x: u8,
        y: u8,
        kind: TrackBlockKind,
        orientation: u8,
    ) -> Result<(), TrackBuildError> {
        if x as usize >= CUSTOM_GRID_MACRO_DIM || y as usize >= CUSTOM_GRID_MACRO_DIM {
            return Err(TrackBuildError::OutOfBounds);
        }

        // Check if a block already exists at this (x, y) coordinate: replace it.
        for i in 0..self.block_count as usize {
            if self.blocks[i].x == x && self.blocks[i].y == y {
                self.blocks[i] = PlacedBlock::new(x, y, kind, orientation);
                // Changing track geometry invalidates existing author time.
                self.author_time_ticks = 0;
                return Ok(());
            }
        }

        // Add new block if under capacity.
        if (self.block_count as usize) < MAX_CUSTOM_BLOCKS {
            let idx = self.block_count as usize;
            self.blocks[idx] = PlacedBlock::new(x, y, kind, orientation);
            self.block_count += 1;
            self.author_time_ticks = 0;
            Ok(())
        } else {
            Err(TrackBuildError::DuplicatePosition)
        }
    }

    /// Removes any block placed at (x, y).
    pub fn remove_block(&mut self, x: u8, y: u8) -> bool {
        let count = self.block_count as usize;
        for i in 0..count {
            if self.blocks[i].x == x && self.blocks[i].y == y {
                for j in i..count - 1 {
                    self.blocks[j] = self.blocks[j + 1];
                }
                self.blocks[count - 1] = PlacedBlock::default();
                self.block_count -= 1;
                self.author_time_ticks = 0;
                return true;
            }
        }
        false
    }

    /// Serializes this track into exactly 256 bytes.
    pub fn serialize(&self, out: &mut [u8; CUSTOM_TRACK_SERIALIZED_SIZE]) {
        *out = [0u8; CUSTOM_TRACK_SERIALIZED_SIZE];

        out[0..4].copy_from_slice(&CUSTOM_TRACK_MAGIC);
        out[4] = CUSTOM_TRACK_VERSION;
        out[5..21].copy_from_slice(&self.name);
        out[21..25].copy_from_slice(&self.author_time_ticks.to_le_bytes());
        out[25] = self.block_count;

        let mut offset = 28;
        for i in 0..self.block_count.min(MAX_CUSTOM_BLOCKS as u8) as usize {
            let packed = self.blocks[i].pack();
            out[offset..offset + 3].copy_from_slice(&packed);
            offset += 3;
        }

        // CRC16 covers bytes 0..26 and 28..256.
        let mut crc_buf = [0u8; 254];
        crc_buf[0..26].copy_from_slice(&out[0..26]);
        crc_buf[26..254].copy_from_slice(&out[28..256]);
        let checksum = crc16(&crc_buf);
        out[26..28].copy_from_slice(&checksum.to_le_bytes());
    }

    /// Deserializes a track from 256 bytes, validating magic, version, and CRC.
    pub fn deserialize(bytes: &[u8; CUSTOM_TRACK_SERIALIZED_SIZE]) -> Option<Self> {
        if bytes[0..4] != CUSTOM_TRACK_MAGIC || bytes[4] != CUSTOM_TRACK_VERSION {
            return None;
        }

        let stored_crc = u16::from_le_bytes([bytes[26], bytes[27]]);
        let mut crc_buf = [0u8; 254];
        crc_buf[0..26].copy_from_slice(&bytes[0..26]);
        crc_buf[26..254].copy_from_slice(&bytes[28..256]);
        if crc16(&crc_buf) != stored_crc {
            return None;
        }

        let mut name = [0u8; 16];
        name.copy_from_slice(&bytes[5..21]);
        let author_time_ticks = u32::from_le_bytes([bytes[21], bytes[22], bytes[23], bytes[24]]);
        let block_count = bytes[25].min(MAX_CUSTOM_BLOCKS as u8);

        let mut blocks = [PlacedBlock::default(); MAX_CUSTOM_BLOCKS];
        let mut offset = 28;
        for block in blocks.iter_mut().take(block_count as usize) {
            *block = PlacedBlock::unpack([bytes[offset], bytes[offset + 1], bytes[offset + 2]]);
            offset += 3;
        }

        Some(Self {
            name,
            author_time_ticks,
            block_count,
            blocks,
        })
    }

    /// Validates the track layout before a test drive or race can start.
    pub fn validate(&self) -> Result<(), TrackBuildError> {
        if self.block_count < 3 {
            return Err(TrackBuildError::EmptyTrack);
        }

        let mut start_count = 0;
        for i in 0..self.block_count as usize {
            let b = &self.blocks[i];
            if b.x as usize >= CUSTOM_GRID_MACRO_DIM || b.y as usize >= CUSTOM_GRID_MACRO_DIM {
                return Err(TrackBuildError::OutOfBounds);
            }
            if b.kind == TrackBlockKind::StartGrid {
                start_count += 1;
            }
        }

        if start_count == 0 {
            return Err(TrackBuildError::NoStartGrid);
        }
        if start_count > 1 {
            return Err(TrackBuildError::MultipleStartGrids);
        }

        Ok(())
    }

    /// Compiles this custom track into an 80x80 tile grid and metadata.
    pub fn compile(
        &self,
        grid_out: &mut [TrackTile],
    ) -> Result<CompiledCustomTrack, TrackBuildError> {
        self.validate()?;

        if grid_out.len() < TOTAL_CUSTOM_TILES {
            return Err(TrackBuildError::OutOfBounds);
        }

        // Fill background with OffRoad; perimeter borders with Barrier.
        for tile in grid_out.iter_mut() {
            *tile = TrackTile::OffRoad;
        }
        for x in 0..CUSTOM_GRID_TILE_DIM {
            grid_out[x] = TrackTile::Barrier;
            grid_out[(CUSTOM_GRID_TILE_DIM - 1) * CUSTOM_GRID_TILE_DIM + x] = TrackTile::Barrier;
        }
        for y in 0..CUSTOM_GRID_TILE_DIM {
            grid_out[y * CUSTOM_GRID_TILE_DIM] = TrackTile::Barrier;
            grid_out[y * CUSTOM_GRID_TILE_DIM + (CUSTOM_GRID_TILE_DIM - 1)] = TrackTile::Barrier;
        }

        let mut start_pos = Vec2::ZERO;
        let mut start_heading = 0u16;
        let mut start_gate = CheckpointGate::default();
        let mut checkpoints = [CheckpointGate::default(); MAX_TRACK_CHECKPOINTS];
        let mut checkpoint_count = 0u8;

        let mut route_points = [Vec2::ZERO; MAX_CUSTOM_BLOCKS];
        let mut route_count = 0usize;

        // First pass: locate StartGrid for start position & heading.
        for i in 0..self.block_count as usize {
            let b = &self.blocks[i];
            if b.kind == TrackBlockKind::StartGrid {
                let tx = b.x as usize * BLOCK_TILE_SIZE;
                let ty = b.y as usize * BLOCK_TILE_SIZE;

                let wx = (tx as i32 + 2) * TILE_SIZE;
                let wy = (ty as i32 + 2) * TILE_SIZE;
                start_pos = Vec2::new(crate::math::Fixed::from_int(wx), crate::math::Fixed::from_int(wy));

                start_heading = match b.orientation & 0x03 {
                    0 => 0,                            // North
                    1 => ANGLE_90,                     // East
                    2 => ANGLE_180,                    // South
                    3 => ANGLE_90 * 3,                 // West
                    _ => 0,
                };

                start_gate = CheckpointGate {
                    x: tx as u8,
                    y: ty as u8,
                    width: BLOCK_TILE_SIZE as u8,
                    height: BLOCK_TILE_SIZE as u8,
                };
                break;
            }
        }

        // Second pass: rasterize every modular block into the 4x4 subgrid.
        for i in 0..self.block_count as usize {
            let b = &self.blocks[i];
            let tx = b.x as usize * BLOCK_TILE_SIZE;
            let ty = b.y as usize * BLOCK_TILE_SIZE;

            rasterize_block(b.kind, b.orientation, tx, ty, grid_out);

            // Collect checkpoints.
            if b.kind == TrackBlockKind::CheckpointGate
                && (checkpoint_count as usize) < MAX_TRACK_CHECKPOINTS
            {
                checkpoints[checkpoint_count as usize] = CheckpointGate {
                    x: tx as u8,
                    y: ty as u8,
                    width: BLOCK_TILE_SIZE as u8,
                    height: BLOCK_TILE_SIZE as u8,
                };
                checkpoint_count += 1;
            }

            // Collect route waypoint.
            if route_count < MAX_CUSTOM_BLOCKS {
                let wx = (tx as i32 + 2) * TILE_SIZE;
                let wy = (ty as i32 + 2) * TILE_SIZE;
                route_points[route_count] = Vec2::new(crate::math::Fixed::from_int(wx), crate::math::Fixed::from_int(wy));
                route_count += 1;
            }
        }

        // Par times: derived from Author validation time or reasonable defaults.
        let dev = if self.author_time_ticks > 0 {
            self.author_time_ticks
        } else {
            1800 // 30s default
        };
        let gold = dev + (dev / 7).max(60);
        let silver = dev + (dev / 3).max(120);
        let bronze = dev + (dev / 2).max(180);
        let par_times = ParTimes {
            bronze_ticks: bronze,
            silver_ticks: silver,
            gold_ticks: gold,
            dev_platinum_ticks: dev,
        };

        Ok(CompiledCustomTrack {
            width: CUSTOM_GRID_TILE_DIM as u8,
            height: CUSTOM_GRID_TILE_DIM as u8,
            start_pos,
            start_heading,
            start_gate,
            par_times,
            checkpoint_count,
            checkpoints,
            route_points,
            route_count,
        })
    }
}

/// Output of compiling a custom track into executable engine state.
#[derive(Copy, Clone, Debug)]
pub struct CompiledCustomTrack {
    pub width: u8,
    pub height: u8,
    pub start_pos: Vec2,
    pub start_heading: u16,
    pub start_gate: CheckpointGate,
    pub par_times: ParTimes,
    pub checkpoint_count: u8,
    pub checkpoints: [CheckpointGate; MAX_TRACK_CHECKPOINTS],
    pub route_points: [Vec2; MAX_CUSTOM_BLOCKS],
    pub route_count: usize,
}

impl CompiledCustomTrack {
    /// Builds a static-lifetime compatible `TrackDef`.
    pub fn to_track_def(
        &self,
        name: &'static str,
        tiles: &'static [TrackTile],
        route: &'static [Vec2],
    ) -> TrackDef {
        TrackDef {
            name,
            width: self.width,
            height: self.height,
            start_pos: self.start_pos,
            start_heading: self.start_heading,
            start_gate: self.start_gate,
            par_times: self.par_times,
            checkpoint_count: self.checkpoint_count,
            checkpoints: self.checkpoints,
            tiles: Some(tiles),
            route,
            half_width: (2 * TILE_SIZE) as u16,
        }
    }
}

/// Rasterizes a 4x4 modular TrackMania block into the 80x80 tile grid.
fn rasterize_block(
    kind: TrackBlockKind,
    orientation: u8,
    tx: usize,
    ty: usize,
    grid: &mut [TrackTile],
) {
    let mut pattern = [TrackTile::OffRoad; 16];

    match kind {
        TrackBlockKind::Straight => {
            // Base orientation (North/South): col 0 curb, col 1..2 tarmac, col 3 curb.
            for r in 0..4 {
                pattern[r * 4] = TrackTile::Curb;
                pattern[r * 4 + 1] = TrackTile::Tarmac;
                pattern[r * 4 + 2] = TrackTile::Tarmac;
                pattern[r * 4 + 3] = TrackTile::Curb;
            }
        }
        TrackBlockKind::StartGrid => {
            for r in 0..4 {
                pattern[r * 4] = TrackTile::Curb;
                pattern[r * 4 + 1] = TrackTile::StartFinish;
                pattern[r * 4 + 2] = TrackTile::StartFinish;
                pattern[r * 4 + 3] = TrackTile::Curb;
            }
        }
        TrackBlockKind::CheckpointGate => {
            for r in 0..4 {
                pattern[r * 4] = TrackTile::Curb;
                pattern[r * 4 + 1] = TrackTile::Checkpoint;
                pattern[r * 4 + 2] = TrackTile::Checkpoint;
                pattern[r * 4 + 3] = TrackTile::Curb;
            }
        }
        TrackBlockKind::DirtSection => {
            for r in 0..4 {
                pattern[r * 4] = TrackTile::Dirt;
                pattern[r * 4 + 1] = TrackTile::Dirt;
                pattern[r * 4 + 2] = TrackTile::Dirt;
                pattern[r * 4 + 3] = TrackTile::Dirt;
            }
        }
        TrackBlockKind::IceBobsleigh => {
            for r in 0..4 {
                pattern[r * 4] = TrackTile::Curb;
                pattern[r * 4 + 1] = TrackTile::Ice;
                pattern[r * 4 + 2] = TrackTile::Ice;
                pattern[r * 4 + 3] = TrackTile::Curb;
            }
        }
        TrackBlockKind::JumpRamp => {
            for r in 0..4 {
                pattern[r * 4] = TrackTile::Curb;
                pattern[r * 4 + 1] = if r == 1 || r == 2 {
                    TrackTile::JumpRamp
                } else {
                    TrackTile::Tarmac
                };
                pattern[r * 4 + 2] = if r == 1 || r == 2 {
                    TrackTile::JumpRamp
                } else {
                    TrackTile::Tarmac
                };
                pattern[r * 4 + 3] = TrackTile::Curb;
            }
        }
        TrackBlockKind::BridgeOverpass => {
            for r in 0..4 {
                pattern[r * 4] = TrackTile::Curb;
                pattern[r * 4 + 1] = TrackTile::Bridge;
                pattern[r * 4 + 2] = TrackTile::Bridge;
                pattern[r * 4 + 3] = TrackTile::Curb;
            }
        }
        TrackBlockKind::FreeWheel => {
            for r in 0..4 {
                pattern[r * 4] = TrackTile::Curb;
                pattern[r * 4 + 1] = if r == 1 || r == 2 {
                    TrackTile::EngineOff
                } else {
                    TrackTile::Tarmac
                };
                pattern[r * 4 + 2] = if r == 1 || r == 2 {
                    TrackTile::EngineOff
                } else {
                    TrackTile::Tarmac
                };
                pattern[r * 4 + 3] = TrackTile::Curb;
            }
        }
        TrackBlockKind::ReactorBoost => {
            for r in 0..4 {
                pattern[r * 4] = TrackTile::Curb;
                pattern[r * 4 + 1] = if r == 1 || r == 2 {
                    TrackTile::SuperBoostPad
                } else {
                    TrackTile::Tarmac
                };
                pattern[r * 4 + 2] = if r == 1 || r == 2 {
                    TrackTile::SuperBoostPad
                } else {
                    TrackTile::Tarmac
                };
                pattern[r * 4 + 3] = TrackTile::Curb;
            }
        }
        TrackBlockKind::Turn90 => {
            // Turn entering from North (cols 1..2 at row 0) and exiting to East (rows 1..2 at col 3).
            pattern[0] = TrackTile::Curb;
            pattern[1] = TrackTile::Tarmac;
            pattern[2] = TrackTile::Tarmac;
            pattern[3] = TrackTile::Curb;

            pattern[4] = TrackTile::Curb;
            pattern[5] = TrackTile::Tarmac;
            pattern[6] = TrackTile::Tarmac;
            pattern[7] = TrackTile::Tarmac;

            pattern[8] = TrackTile::Curb;
            pattern[9] = TrackTile::Curb;
            pattern[10] = TrackTile::Tarmac;
            pattern[11] = TrackTile::Tarmac;

            pattern[12] = TrackTile::OffRoad;
            pattern[13] = TrackTile::Curb;
            pattern[14] = TrackTile::Curb;
            pattern[15] = TrackTile::Curb;
        }
        TrackBlockKind::Hairpin => {
            // 180° hairpin entering North cols 0..1, looping through row 2..3, exiting North cols 2..3.
            pattern[0] = TrackTile::Tarmac;
            pattern[1] = TrackTile::Curb;
            pattern[2] = TrackTile::Curb;
            pattern[3] = TrackTile::Tarmac;

            pattern[4] = TrackTile::Tarmac;
            pattern[5] = TrackTile::Barrier;
            pattern[6] = TrackTile::Barrier;
            pattern[7] = TrackTile::Tarmac;

            pattern[8] = TrackTile::Tarmac;
            pattern[9] = TrackTile::Tarmac;
            pattern[10] = TrackTile::Tarmac;
            pattern[11] = TrackTile::Tarmac;

            pattern[12] = TrackTile::Curb;
            pattern[13] = TrackTile::Curb;
            pattern[14] = TrackTile::Curb;
            pattern[15] = TrackTile::Curb;
        }
        TrackBlockKind::Chicane => {
            // Quick S-bend chicane.
            pattern[0] = TrackTile::Curb;
            pattern[1] = TrackTile::Tarmac;
            pattern[2] = TrackTile::Tarmac;
            pattern[3] = TrackTile::Curb;

            pattern[4] = TrackTile::Curb;
            pattern[5] = TrackTile::Curb;
            pattern[6] = TrackTile::Tarmac;
            pattern[7] = TrackTile::Tarmac;

            pattern[8] = TrackTile::Tarmac;
            pattern[9] = TrackTile::Tarmac;
            pattern[10] = TrackTile::Curb;
            pattern[11] = TrackTile::Curb;

            pattern[12] = TrackTile::Curb;
            pattern[13] = TrackTile::Tarmac;
            pattern[14] = TrackTile::Tarmac;
            pattern[15] = TrackTile::Curb;
        }
    }

    // Apply rotation (0..3) to the 4x4 subgrid and write to destination.
    let rot = orientation & 0x03;
    for r in 0..4 {
        for c in 0..4 {
            let (src_r, src_c) = match rot {
                0 => (r, c),             // 0°
                1 => (3 - c, r),         // 90° clockwise
                2 => (3 - r, 3 - c),     // 180°
                3 => (c, 3 - r),         // 270° clockwise
                _ => (r, c),
            };

            let tile = pattern[src_r * 4 + src_c];
            let dst_x = tx + c;
            let dst_y = ty + r;

            if dst_x < CUSTOM_GRID_TILE_DIM && dst_y < CUSTOM_GRID_TILE_DIM {
                grid[dst_y * CUSTOM_GRID_TILE_DIM + dst_x] = tile;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placed_block_packing_round_trips() {
        for kind_idx in 0..TrackBlockKind::COUNT {
            let kind = TrackBlockKind::from_u8(kind_idx as u8);
            for rot in 0..4 {
                let block = PlacedBlock::new(12, 18, kind, rot);
                let packed = block.pack();
                let unpacked = PlacedBlock::unpack(packed);
                assert_eq!(block, unpacked);
            }
        }
    }

    #[test]
    fn custom_track_serialization_round_trips_cleanly() {
        let mut track = CustomTrackDef::new("NEO TOKYO 2026");
        assert!(track
            .place_block(2, 2, TrackBlockKind::StartGrid, 0)
            .is_ok());
        assert!(track.place_block(2, 3, TrackBlockKind::Straight, 0).is_ok());
        assert!(track
            .place_block(2, 4, TrackBlockKind::CheckpointGate, 0)
            .is_ok());
        assert!(track.place_block(2, 5, TrackBlockKind::Turn90, 1).is_ok());
        assert!(track
            .place_block(3, 5, TrackBlockKind::ReactorBoost, 1)
            .is_ok());
        track.author_time_ticks = 1420;

        let mut buf = [0u8; CUSTOM_TRACK_SERIALIZED_SIZE];
        track.serialize(&mut buf);

        let restored = CustomTrackDef::deserialize(&buf).expect("deserialize valid track");
        assert_eq!(restored.name_str(), "NEO TOKYO 2026");
        assert_eq!(restored.author_time_ticks, 1420);
        assert_eq!(restored.block_count, 5);
        assert_eq!(restored.blocks[0], track.blocks[0]);
        assert_eq!(restored.blocks[3], track.blocks[3]);
        assert_eq!(restored.blocks[4], track.blocks[4]);
    }

    #[test]
    fn corrupted_custom_track_is_rejected() {
        let mut track = CustomTrackDef::new("ARENA X");
        track
            .place_block(5, 5, TrackBlockKind::StartGrid, 0)
            .unwrap();
        track.place_block(5, 6, TrackBlockKind::Straight, 0).unwrap();
        track.place_block(5, 7, TrackBlockKind::Turn90, 0).unwrap();

        let mut buf = [0u8; CUSTOM_TRACK_SERIALIZED_SIZE];
        track.serialize(&mut buf);

        // Corrupt one data byte inside the block array
        buf[35] ^= 0xFF;
        assert!(CustomTrackDef::deserialize(&buf).is_none());
    }

    #[test]
    fn track_compilation_produces_valid_engine_def() {
        let mut track = CustomTrackDef::new("STADIUM LOOP");
        track
            .place_block(5, 5, TrackBlockKind::StartGrid, 0)
            .unwrap();
        track.place_block(5, 6, TrackBlockKind::Straight, 0).unwrap();
        track
            .place_block(5, 7, TrackBlockKind::CheckpointGate, 0)
            .unwrap();
        track.place_block(5, 8, TrackBlockKind::Turn90, 0).unwrap();
        track.author_time_ticks = 1200;

        let mut grid = [TrackTile::OffRoad; TOTAL_CUSTOM_TILES];
        let compiled = track.compile(&mut grid).expect("compiles cleanly");

        assert_eq!(compiled.width, 80);
        assert_eq!(compiled.height, 80);
        assert_eq!(compiled.checkpoint_count, 1);
        assert_eq!(compiled.par_times.dev_platinum_ticks, 1200);
        assert!(compiled.par_times.gold_ticks > 1200);
        assert!(compiled.par_times.silver_ticks > compiled.par_times.gold_ticks);
        assert!(compiled.par_times.bronze_ticks > compiled.par_times.silver_ticks);

        // Verify start grid is StartFinish tiles
        let start_tile_x = 5 * BLOCK_TILE_SIZE + 1;
        let start_tile_y = 5 * BLOCK_TILE_SIZE + 1;
        assert_eq!(
            grid[start_tile_y * 80 + start_tile_x],
            TrackTile::StartFinish
        );
    }
}
