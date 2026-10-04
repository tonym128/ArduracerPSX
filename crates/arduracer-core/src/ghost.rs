//! Deterministic Ghost Car recording, playback, and telemetry compression.
//!
//! Telemetry is encoded into 6-byte frames sampled at 30 Hz, so a lap costs
//! `MAX_FRAMES * 2` ticks of wall clock and 6 bytes per 30 Hz sample: a
//! 60-second lap is 10,800 bytes (10.5 KiB) at the shipped `MAX_FRAMES` of 1800
//! (`MAX_GHOST_FRAMES` in `game/src/ghost_recorder.rs`).
//!
//! **That does not fit a PlayStation 1 Memory Card block, which is 8 KiB.**
//! [`GHOST_MAGIC`] and [`GHOST_VERSION`] exist for a `to_bytes`/`from_bytes`
//! serializer that has not been written: there is no ghost blob on the card
//! today. [`crate::save::SaveData::to_block_bytes`] writes only
//! [`crate::save::PAYLOAD_SIZE`] (161) bytes and zero-fills the rest of the
//! 8 KiB block, and [`crate::save::SaveData`] has no ghost field at all -- a
//! recorded ghost lives in RAM for the length of the session only.
//!
//! Shipping a ghost on the card therefore needs its own work, not a tweak to
//! the numbers above: either a smaller frame (delta-coded, ~2-3 bytes) or a
//! second block, plus the serializer, a compression ratio that survives
//! quantisation, and the migration story for an existing card image.

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
///
/// The frame buffer is fixed, so a lap longer than
/// `MAX_FRAMES * GHOST_SAMPLE_INTERVAL_TICKS` ticks (60 s at the shipped
/// `MAX_FRAMES` of 1800) cannot be stored in full. Recording then *truncates*
/// rather than overwriting or growing, and says so:
/// [`GhostRecorder::is_truncated`] latches once a sample has been dropped, so
/// the race loop can refuse to promote an incomplete ghost to the best-lap
/// replay instead of shipping one that stops dead mid-race.
#[derive(Clone, Debug)]
pub struct GhostRecorder<const MAX_FRAMES: usize> {
    pub frames: [GhostFrame; MAX_FRAMES],
    pub frame_count: usize,
    pub track_id: u8,
    /// Ticks recorded so far, or the final time passed to
    /// [`GhostRecorder::finish`]. Saturating: a race left running cannot wrap.
    pub lap_time_ticks: u32,
    pub is_recording: bool,
    /// True once a sample was dropped because the frame buffer was full.
    /// Latched for the whole run, cleared by [`GhostRecorder::start`].
    pub is_truncated: bool,
}

impl<const MAX_FRAMES: usize> GhostRecorder<MAX_FRAMES> {
    pub fn new(track_id: u8) -> Self {
        GhostRecorder {
            frames: [GhostFrame::default(); MAX_FRAMES],
            frame_count: 0,
            track_id,
            lap_time_ticks: 0,
            is_recording: false,
            is_truncated: false,
        }
    }

    /// Starts recording a new lap.
    pub fn start(&mut self) {
        self.frame_count = 0;
        self.lap_time_ticks = 0;
        self.is_recording = true;
        self.is_truncated = false;
    }

    /// Samples the vehicle state every 2 ticks (30 Hz).
    ///
    /// Frame `k` holds the state at tick `2k`: the first `record_tick` of a run
    /// writes frame 0, so `sample_at_tick(0)` is the state the ghost was in at
    /// the start of the lap. The tick counter is therefore read *before* it is
    /// incremented -- doing it the other way round put every ghost 2 ticks
    /// (33 ms) ahead of the player for the whole race.
    pub fn record_tick(&mut self, pos: Vec2, heading: u16, flags: u8) {
        if !self.is_recording {
            return;
        }
        let tick = self.lap_time_ticks;
        self.lap_time_ticks = self.lap_time_ticks.saturating_add(1);

        if !tick.is_multiple_of(GHOST_SAMPLE_INTERVAL_TICKS as u32) {
            return;
        }
        if self.frame_count < MAX_FRAMES {
            self.frames[self.frame_count] = GhostFrame::encode(pos, heading, flags);
            self.frame_count += 1;
        } else {
            // Out of buffer: this lap is longer than the recorder can hold. Flag
            // it instead of dropping telemetry silently.
            self.is_truncated = true;
        }
    }

    /// Finishes recording and locks the telemetry.
    ///
    /// The clip is complete only while [`GhostRecorder::is_truncated`] is false.
    /// A truncated clip still holds every frame it did capture, so it plays back
    /// correctly up to its last tick and nowhere further.
    pub fn finish(&mut self, final_lap_ticks: u32) {
        self.is_recording = false;
        self.lap_time_ticks = final_lap_ticks;
    }

    /// Ticks of telemetry this recorder can hold at the sample interval.
    pub const fn capacity_ticks(&self) -> u32 {
        (MAX_FRAMES * (GHOST_SAMPLE_INTERVAL_TICKS as usize)) as u32
    }

    /// Whether this clip is longer than [`GhostRecorder::capacity_ticks`] and
    /// therefore incomplete.
    pub fn is_complete(&self) -> bool {
        !self.is_truncated
    }

    /// Ticks of playback this clip actually covers: one sample interval per
    /// stored frame, capped at the recorded lap time.
    pub fn recorded_ticks(&self) -> u32 {
        let frame_ticks = (self.frame_count as u32) * (GHOST_SAMPLE_INTERVAL_TICKS as u32);
        frame_ticks.min(self.lap_time_ticks)
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
///
/// `total_ticks` is the lap time the clip claims to cover. It is checked against
/// the frame count on construction ([`GhostPlayer::is_consistent`]) because a
/// truncated recorder still reports the *full* lap time: without the check the
/// two disagree silently and the ghost appears to stop early. Playback itself
/// is bounded by the frames, so the mismatch is a reporting problem, never an
/// out-of-bounds one.
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

    /// Ticks of playback the frames can actually cover: one sample interval per
    /// stored frame, capped at the claimed lap time.
    pub fn recorded_ticks(&self) -> u32 {
        let frame_ticks = (self.frames.len() as u32) * (GHOST_SAMPLE_INTERVAL_TICKS as u32);
        frame_ticks.min(self.total_ticks)
    }

    /// Whether `total_ticks` and the frame count describe the same clip.
    ///
    /// `false` means the recorder ran out of frames before the lap ended (see
    /// [`GhostRecorder::is_truncated`]): playback stops at
    /// [`GhostPlayer::recorded_ticks`], so the ghost has finished its lap and
    /// should be drawn no further rather than held at its last frame forever.
    pub fn is_consistent(&self) -> bool {
        self.recorded_ticks() == self.total_ticks
    }

    /// Whether the clip has run out of telemetry by `current_tick`.
    ///
    /// Callers must honour this: past the end of the frames
    /// [`GhostPlayer::sample_at_tick`] deliberately holds the last frame, so a
    /// ghost that ignored this would sit on the track for the rest of the race
    /// pretending to be a rival.
    pub fn is_exhausted(&self, current_tick: u32) -> bool {
        // An empty clip has no telemetry at any tick, so it is exhausted from
        // the start rather than after one interval.
        self.frames.is_empty() || current_tick > self.recorded_ticks()
    }

    /// Evaluates the interpolated position and heading at an exact 60Hz tick.
    ///
    /// Past the end of the clip the last frame is held: interpolating past the
    /// final keyframe would extrapolate into the scenery, and returning the
    /// default would teleport the ghost to the world origin. Use
    /// [`GhostPlayer::is_exhausted`] to stop drawing it.
    pub fn sample_at_tick(&self, current_tick: u32) -> GhostPlaybackState {
        if self.frames.is_empty() {
            return GhostPlaybackState::default();
        }

        // Each keyframe is spaced by GHOST_SAMPLE_INTERVAL_TICKS (2 ticks)
        let frame_idx = (current_tick / (GHOST_SAMPLE_INTERVAL_TICKS as u32)) as usize;
        let sub_tick = current_tick % (GHOST_SAMPLE_INTERVAL_TICKS as u32);

        if frame_idx >= self.frames.len() - 1 {
            let last = match self.frames.last() {
                Some(&f) => f,
                None => return GhostPlaybackState::default(),
            };
            return GhostPlaybackState {
                position: last.decode_position(),
                heading: last.decode_heading(),
                flags: last.flags,
            };
        }

        let (f0, f1) = match (self.frames.get(frame_idx), self.frames.get(frame_idx + 1)) {
            (Some(&a), Some(&b)) => (a, b),
            _ => return GhostPlaybackState::default(),
        };

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

#[cfg(test)]
mod tests {
    use super::*;

    /// Drives `count` ticks through the recorder, putting the car at `start + tick`.
    fn record_ramp<const N: usize>(
        rec: &mut GhostRecorder<N>,
        start: i32,
        count: u32,
        heading: u16,
    ) {
        for tick in 0..count {
            rec.record_tick(
                Vec2::new(Fixed::from_int(start + tick as i32), Fixed::ZERO),
                heading,
                0,
            );
        }
    }

    // ---- Bug 10: record_tick sampled one interval late ----

    #[test]
    fn frame_zero_is_the_state_at_tick_zero() {
        // A 1 unit/tick ramp: frame k must hold the position at tick 2k, so
        // tick 0 is the grid, not the state 33 ms later.
        let mut rec = GhostRecorder::<8>::new(1);
        rec.start();
        record_ramp(&mut rec, 100, 6, 0);

        assert_eq!(rec.frame_count, 3, "6 ticks at 30 Hz is 3 frames");
        let frames = &rec.frames[..rec.frame_count];
        assert_eq!(frames[0].decode_position().x.to_int(), 100, "tick 0");
        assert_eq!(frames[1].decode_position().x.to_int(), 102, "tick 2");
        assert_eq!(frames[2].decode_position().x.to_int(), 104, "tick 4");

        let player = GhostPlayer::new(frames, 6);
        assert_eq!(player.sample_at_tick(0).position.x.to_int(), 100);
        assert_eq!(player.sample_at_tick(1).position.x.to_int(), 101);
        assert_eq!(player.sample_at_tick(2).position.x.to_int(), 102);
        assert_eq!(player.sample_at_tick(5).position.x.to_int(), 104);
    }

    #[test]
    fn heading_is_sampled_on_the_same_tick_as_position() {
        let mut rec = GhostRecorder::<4>::new(1);
        rec.start();
        for tick in 0..4u16 {
            rec.record_tick(
                Vec2::new(Fixed::from_int(tick as i32), Fixed::ZERO),
                tick * 1024,
                0,
            );
        }
        let frames = &rec.frames[..rec.frame_count];
        assert_eq!(frames[0].decode_heading(), 0);
        assert_eq!(frames[1].decode_heading(), 2048);
        let player = GhostPlayer::new(frames, 4);
        assert_eq!(player.sample_at_tick(0).heading, 0);
        assert_eq!(player.sample_at_tick(2).heading, 2048);
    }

    #[test]
    fn sample_rate_and_frame_count_are_unchanged() {
        // 60 Hz input, 30 Hz sampling: a 60-tick lap is 30 frames.
        let mut rec = GhostRecorder::<300>::new(1);
        rec.start();
        record_ramp(&mut rec, 0, 60, 0);
        rec.finish(60);
        assert_eq!(rec.frame_count, 30);
        assert_eq!(rec.lap_time_ticks, 60);
        assert!(rec.is_complete());
    }

    // ---- Bug 8: silent truncation, then a permanent freeze ----

    #[test]
    fn an_over_long_lap_is_flagged_as_truncated() {
        let mut rec = GhostRecorder::<4>::new(1);
        rec.start();
        assert!(!rec.is_truncated);
        assert_eq!(rec.capacity_ticks(), 8, "4 frames at 2 ticks each");

        // Exactly the capacity: still complete.
        record_ramp(&mut rec, 0, 8, 0);
        assert_eq!(rec.frame_count, 4);
        assert!(rec.is_complete(), "a lap at capacity is not truncated");

        // One sample interval past it: telemetry is dropped, and says so.
        record_ramp(&mut rec, 0, 2, 0);
        assert_eq!(rec.frame_count, 4, "the buffer must not overflow");
        assert!(rec.is_truncated);
        assert!(!rec.is_complete());
        // The clock kept running, which is exactly why the flag is needed.
        assert_eq!(rec.lap_time_ticks, 10);
        rec.finish(10);
        assert!(rec.is_truncated, "finish() must not launder the flag");
        assert_eq!(rec.recorded_ticks(), 8, "playback covers the frames only");

        // Restarting clears the flag.
        rec.start();
        assert!(!rec.is_truncated);
    }

    #[test]
    fn player_reports_a_truncated_clip_as_inconsistent() {
        let mut rec = GhostRecorder::<2>::new(1);
        rec.start();
        record_ramp(&mut rec, 0, 40, 0);
        rec.finish(40);
        assert!(rec.is_truncated);
        let frames = &rec.frames[..rec.frame_count];

        let player = GhostPlayer::new(frames, rec.lap_time_ticks);
        assert!(!player.is_consistent(), "40 claimed ticks vs 2 frames");
        assert_eq!(player.recorded_ticks(), 4);
        assert!(player.is_exhausted(5), "playback ended at tick 4");
        assert!(!player.is_exhausted(4));

        // Past the end the last frame is held rather than teleporting, but the
        // caller is told, so the ghost can be dropped instead of sliding on.
        let held = player.sample_at_tick(4_000);
        assert_eq!(held.position.x.to_int(), 2, "the last frame, held");
        assert_eq!(player.sample_at_tick(4_000), held, "and held forever");
    }

    #[test]
    fn a_complete_clip_is_consistent_and_not_exhausted_early() {
        let mut rec = GhostRecorder::<30>::new(1);
        rec.start();
        record_ramp(&mut rec, 0, 60, 0);
        rec.finish(60);
        let frames = &rec.frames[..rec.frame_count];
        let player = GhostPlayer::new(frames, rec.lap_time_ticks);
        assert!(player.is_consistent());
        assert!(!player.is_exhausted(60));
        // One tick past the line the ghost has finished its lap.
        assert!(player.is_exhausted(61));
    }

    #[test]
    fn an_empty_or_absurd_clip_is_handled_without_panicking() {
        let player = GhostPlayer::new(&[], 1_000);
        assert_eq!(player.sample_at_tick(0), GhostPlaybackState::default());
        assert_eq!(
            player.sample_at_tick(u32::MAX),
            GhostPlaybackState::default()
        );
        assert!(player.is_exhausted(0));
        assert!(!player.is_consistent());

        let frames = [
            GhostFrame::encode(Vec2::new(Fixed::from_int(5), Fixed::ZERO), 0, 0),
            GhostFrame::encode(Vec2::new(Fixed::from_int(9), Fixed::ZERO), 0, 0),
        ];
        let huge = GhostPlayer::new(&frames, u32::MAX);
        assert_eq!(huge.recorded_ticks(), 4, "capped by the frame count");
        assert!(!huge.is_consistent());
        // Every tick is in range, including the last representable one.
        let _ = huge.sample_at_tick(u32::MAX);
    }

    #[test]
    fn the_claim_in_the_module_doc_is_the_real_size() {
        // 6-byte frames at 30 Hz for a 60-second lap is 10,800 bytes, which does
        // not fit one 8 KiB card block. The doc comment now says so; this keeps
        // the arithmetic honest if the frame or sample rate ever changes.
        let frames_for_60s = 60 * 60 / GHOST_SAMPLE_INTERVAL_TICKS as u32;
        assert_eq!(frames_for_60s, 1_800);
        assert!(
            frames_for_60s * 6 > 8192,
            "60 s of ghost does not fit a block"
        );
    }
}
