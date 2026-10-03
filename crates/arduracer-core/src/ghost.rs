//! Deterministic Ghost Car recording, playback, and telemetry compression.
//!
//! Encodes vehicle telemetry into compact 6-byte frames sampled at 30 Hz,
//! fitting an entire 30-second time trial run within 5.4 KB for easy storage
//! on a PlayStation 1 Memory Card.

use crate::math::{self, Fixed, Vec2};

pub const GHOST_MAGIC: [u8; 8] = *b"PSXGHOST";
pub const GHOST_VERSION: u16 = 1;
pub const GHOST_SAMPLE_INTERVAL_TICKS: u8 = 2; // 60Hz / 2 = 30Hz recording

pub const FLAG_BRAKING: u8 = 1 << 0;
pub const FLAG_DRIFTING: u8 = 1 << 1;
pub const FLAG_BOOSTING: u8 = 1 << 2;
pub const FLAG_SKIDMARK: u8 = 1 << 3;

/// A single compressed telemetry keyframe (6 bytes).
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
#[repr(C, packed)]
pub struct GhostFrame {
    /// World position X (raw fixed point Q20.12 shifted right by 4 bits).
    pub pos_x: i16,
    /// World position Y (raw fixed point Q20.12 shifted right by 4 bits).
    pub pos_y: i16,
    /// Vehicle heading angle (0..4096 mapped to 0..255).
    pub heading_byte: u8,
    /// Vehicle status flags (drift, brake, boost, skidmark).
    pub flags: u8,
}

impl GhostFrame {
    /// Encodes a world position and heading into a compressed GhostFrame.
    pub fn encode(pos: Vec2, heading: u16, flags: u8) -> Self {
        GhostFrame {
            pos_x: (pos.x.raw() >> 4) as i16,
            pos_y: (pos.y.raw() >> 4) as i16,
            heading_byte: ((heading & 0x0FFF) >> 4) as u8,
            flags,
        }
    }

    /// Decodes the compressed frame back into fixed-point world coordinates.
    pub fn decode_position(&self) -> Vec2 {
        Vec2 {
            x: Fixed::from_raw((self.pos_x as i32) << 4),
            y: Fixed::from_raw((self.pos_y as i32) << 4),
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
