//! Deterministic Ghost Car recording, playback, and telemetry compression.
//!
//! Encodes vehicle telemetry into compact 6-byte frames sampled at 30 Hz: a
//! 60-second lap is under 11 KB, which still fits in one PlayStation 1 Memory
//! Card block alongside the rest of the save.

use crate::math::{self, Fixed, Vec2};

pub const GHOST_MAGIC: [u8; 8] = *b"PSXGHOST";
pub const GHOST_VERSION: u16 = 1;
pub const GHOST_SAMPLE_INTERVAL_TICKS: u8 = 2; // 60Hz / 2 = 30Hz recording

pub const FLAG_BRAKING: u8 = 1 << 0;
pub const FLAG_DRIFTING: u8 = 1 << 1;
pub const FLAG_BOOSTING: u8 = 1 << 2;
pub const FLAG_SKIDMARK: u8 = 1 << 3;

/// World units are stored as quarter-units in a `u16`: 0.25-unit precision
/// (a car is ~24 units long, so this is well under a pixel) across worlds up to
/// 16383 units, which is 256x256 tiles. The largest shipped circuit is 30x30
/// tiles = 1920 units.
///
/// The previous encoding shifted the raw Q20.12 value right by 4 into an `i16`,
/// which only spanned +/-128 world units and wrapped modulo 256 -- every track
/// in the game is at least 640 units across, so every recorded lap replayed as
/// garbage. Truncating a *world* coordinate into 16 signed bits is the bug this
/// constant exists to prevent; see `ghost_positions_survive_real_circuits`.
const POS_QUARTER_UNITS: i32 = 4;
/// Q20.12 raw units per stored quarter-unit (4096 / 4).
const POS_RAW_PER_QUARTER: i32 = math::FP_ONE / POS_QUARTER_UNITS;
/// Largest value representable in the stored `u16`.
const POS_MAX_QUARTERS: i32 = u16::MAX as i32;

/// A single compressed telemetry keyframe (6 bytes).
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
#[repr(C, packed)]
pub struct GhostFrame {
    /// World position X in quarter-units (see [`POS_QUARTER_UNITS`]).
    pub pos_x: u16,
    /// World position Y in quarter-units (see [`POS_QUARTER_UNITS`]).
    pub pos_y: u16,
    /// Vehicle heading angle (0..4096 mapped to 0..255).
    pub heading_byte: u8,
    /// Vehicle status flags (drift, brake, boost, skidmark).
    pub flags: u8,
}

/// Compresses one world axis, saturating rather than wrapping.
///
/// A car is clamped to the track bounds, so positions are non-negative; a
/// negative or out-of-range value can only mean corrupt telemetry, and
/// saturating keeps it from becoming a wild teleport on playback.
fn encode_axis(value: Fixed) -> u16 {
    let quarters = (value.raw() / POS_RAW_PER_QUARTER).clamp(0, POS_MAX_QUARTERS);
    quarters as u16
}

/// Expands one stored axis back to world units.
fn decode_axis(quarters: u16) -> Fixed {
    // u16::MAX * 1024 = 67,108,864, comfortably inside i32.
    Fixed::from_raw((quarters as i32) * POS_RAW_PER_QUARTER)
}

impl GhostFrame {
    /// Encodes a world position and heading into a compressed GhostFrame.
    pub fn encode(pos: Vec2, heading: u16, flags: u8) -> Self {
        GhostFrame {
            pos_x: encode_axis(pos.x),
            pos_y: encode_axis(pos.y),
            heading_byte: ((heading & 0x0FFF) >> 4) as u8,
            flags,
        }
    }

    /// Decodes the compressed frame back into fixed-point world coordinates.
    pub fn decode_position(&self) -> Vec2 {
        Vec2 {
            x: decode_axis(self.pos_x),
            y: decode_axis(self.pos_y),
        }
    }

    /// Decodes the vehicle heading angle back to 0..4096 BAMs.
    pub fn decode_heading(&self) -> u16 {
        (self.heading_byte as u16) << 4
    }
}

/// Telemetry recorder capturing a race run at 30 Hz.
#[derive(Clone, Debug)]
pub struct GhostRecorder<const MAX_FRAMES: usize> {
    pub frames: [GhostFrame; MAX_FRAMES],
    pub frame_count: usize,
    pub track_id: u8,
    pub lap_time_ticks: u32,
    pub is_recording: bool,
    tick_counter: u8,
}

impl<const MAX_FRAMES: usize> GhostRecorder<MAX_FRAMES> {
    pub fn new(track_id: u8) -> Self {
        GhostRecorder {
            frames: [GhostFrame::default(); MAX_FRAMES],
            frame_count: 0,
            track_id,
            lap_time_ticks: 0,
            is_recording: false,
            tick_counter: 0,
        }
    }

    /// Starts recording a new lap.
    pub fn start(&mut self) {
        self.frame_count = 0;
        self.lap_time_ticks = 0;
        self.is_recording = true;
        self.tick_counter = 0;
    }

    /// Samples the vehicle state every 2 ticks (30 Hz).
    pub fn record_tick(&mut self, pos: Vec2, heading: u16, flags: u8) {
        if !self.is_recording {
            return;
        }
        self.lap_time_ticks = self.lap_time_ticks.saturating_add(1);
        self.tick_counter += 1;

        if self.tick_counter >= GHOST_SAMPLE_INTERVAL_TICKS {
            self.tick_counter = 0;
            if self.frame_count < MAX_FRAMES {
                self.frames[self.frame_count] = GhostFrame::encode(pos, heading, flags);
                self.frame_count += 1;
            }
        }
    }

    /// Finishes recording and locks the telemetry.
    pub fn finish(&mut self, final_lap_ticks: u32) {
        self.is_recording = false;
        self.lap_time_ticks = final_lap_ticks;
    }
}

/// Interpolated playback state for a ghost car.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct GhostPlaybackState {
    pub position: Vec2,
    pub heading: u16,
    pub flags: u8,
}

/// Telemetry player performing real-time smooth playback with linear interpolation.
pub struct GhostPlayer<'a> {
    pub frames: &'a [GhostFrame],
    pub total_ticks: u32,
}

impl<'a> GhostPlayer<'a> {
    pub fn new(frames: &'a [GhostFrame], total_ticks: u32) -> Self {
        GhostPlayer {
            frames,
            total_ticks,
        }
    }

    /// Evaluates the interpolated position and heading at an exact 60Hz tick.
    pub fn sample_at_tick(&self, current_tick: u32) -> GhostPlaybackState {
        if self.frames.is_empty() {
            return GhostPlaybackState::default();
        }

        // Each keyframe is spaced by GHOST_SAMPLE_INTERVAL_TICKS (2 ticks)
        let frame_idx = (current_tick / (GHOST_SAMPLE_INTERVAL_TICKS as u32)) as usize;
        let sub_tick = current_tick % (GHOST_SAMPLE_INTERVAL_TICKS as u32);

        if frame_idx >= self.frames.len() - 1 {
            let last = self.frames[self.frames.len() - 1];
            return GhostPlaybackState {
                position: last.decode_position(),
                heading: last.decode_heading(),
                flags: last.flags,
            };
        }

        let f0 = self.frames[frame_idx];
        let f1 = self.frames[frame_idx + 1];

        let p0 = f0.decode_position();
        let p1 = f1.decode_position();

        // Sub-tick interpolation factor (0 = at f0, 2048 = halfway between f0 and f1)
        let t = if sub_tick == 1 { math::FP_HALF } else { 0 };

        let lerp_x =
            p0.x.raw() + (((p1.x.raw() - p0.x.raw()) as i64 * t as i64) >> math::FP_SHIFT) as i32;
        let lerp_y =
            p0.y.raw() + (((p1.y.raw() - p0.y.raw()) as i64 * t as i64) >> math::FP_SHIFT) as i32;

        let h0 = f0.decode_heading();
        let h1 = f1.decode_heading();
        // Shortest angular difference
        let diff = (h1 as i32 - h0 as i32 + 2048).rem_euclid(4096) - 2048;
        let lerp_h = ((h0 as i32 + ((diff * t) >> math::FP_SHIFT)) & 0x0FFF) as u16;

        GhostPlaybackState {
            position: Vec2::new(Fixed::from_raw(lerp_x), Fixed::from_raw(lerp_y)),
            heading: lerp_h,
            flags: f0.flags,
        }
    }
}
