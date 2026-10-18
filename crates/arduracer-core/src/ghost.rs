//! Deterministic Ghost Car recording, playback, and telemetry compression.
//!
//! Telemetry is encoded into 6-byte frames sampled at 30 Hz, so a lap costs
//! `MAX_FRAMES * 2` ticks of wall clock and 6 bytes per 30 Hz sample: a
//! 60-second lap is 10,800 bytes (10.5 KiB) at the shipped `MAX_FRAMES` of 1800
//! (`MAX_GHOST_FRAMES` in `game/src/ghost_recorder.rs`).
//!
//! Position, heading, and flags are delta-compressed into the free tail of the
//! save file's single 8 KiB card block. A typical one-minute lap fits in that
//! space; encoding refuses clips that exceed the one-block data capacity.

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

    /// Computes the live time gap (in 60Hz ticks) between the player at `player_pos`
    /// and this recorded ghost at `current_tick` (`NO1-026`).
    ///
    /// Negative values mean the player is AHEAD of the ghost's pace; positive
    /// values mean the player is BEHIND the ghost.
    pub fn time_gap_ticks(&self, player_pos: Vec2, current_tick: u32) -> Option<i32> {
        if self.frames.is_empty() {
            return None;
        }
        let n = self.frames.len();
        let center_idx = ((current_tick / (GHOST_SAMPLE_INTERVAL_TICKS as u32)) as usize)
            .min(n.saturating_sub(1));
        let radius = 150usize.min(n / 2).max(1);
        let start = center_idx.saturating_sub(radius);
        let end = (center_idx + radius).min(n - 1);

        let mut best_idx = center_idx;
        let mut best_dist_sq = i64::MAX;
        for idx in start..=end {
            let p = self.frames[idx].decode_position();
            let dx = (p.x.raw() - player_pos.x.raw()) >> 8;
            let dy = (p.y.raw() - player_pos.y.raw()) >> 8;
            let d2 = (dx as i64) * (dx as i64) + (dy as i64) * (dy as i64);
            if d2 < best_dist_sq {
                best_dist_sq = d2;
                best_idx = idx;
            }
        }
        let ghost_tick_at_pos = (best_idx as i32) * (GHOST_SAMPLE_INTERVAL_TICKS as i32);
        Some((current_tick as i32) - ghost_tick_at_pos)
    }
}

/// Header size of a delta-compressed ghost stream (`NO1-024`):
/// `magic(8) + version(2) + track_id(1) + lap_time_ticks(4) + frame_count(2) + stream_len(2) = 19`.
pub const GHOST_HEADER_SIZE: usize = 19;
/// Trailing CRC-16 checksum size in bytes.
pub const GHOST_CRC_SIZE: usize = 2;
/// Sentinel first byte (`-128i8`) indicating a 7-byte absolute keyframe in a compressed stream.
pub const KEYFRAME_TAG: u8 = 0x80;

/// Exact encoded size of a ghost clip, including its header and CRC.
pub fn compressed_ghost_encoded_len(frames: &[GhostFrame]) -> Option<usize> {
    if frames.len() > u16::MAX as usize {
        return None;
    }
    let mut stream_len = 0usize;
    let mut prev = GhostFrame::default();
    for (idx, &frame) in frames.iter().enumerate() {
        let dx = frame.pos_x as i32 - prev.pos_x as i32;
        let dy = frame.pos_y as i32 - prev.pos_y as i32;
        let dh = frame.heading_byte.wrapping_sub(prev.heading_byte) as i8;
        let delta = idx > 0
            && (-127..=127).contains(&dx)
            && (-127..=127).contains(&dy)
            && (-8..=7).contains(&dh)
            && frame.flags & 0xF0 == 0;
        stream_len = stream_len.checked_add(if delta { 3 } else { 7 })?;
        prev = frame;
    }
    if stream_len > u16::MAX as usize {
        return None;
    }
    GHOST_HEADER_SIZE
        .checked_add(stream_len)?
        .checked_add(GHOST_CRC_SIZE)
}

/// Encodes a slice of [`GhostFrame`]s into a compact, lossless delta-coded byte stream
/// protected by a trailing CCITT-16 CRC (`NO1-024`).
///
/// Each 30 Hz sample is encoded either as:
/// - **3-byte Delta Packet** `[dx: i8, dy: i8, (dh_nibble << 4) | (flags & 0x0F)]` when
///   quarter-unit deltas fit in `-127..=127` and heading delta fits in `-8..=7`
/// - **7-byte Absolute Keyframe** `[0x80, x_lo, x_hi, y_lo, y_hi, heading_byte, flags]`
///   for frame 0 or any teleport/respawn/spinout exceeding delta range.
///
/// A full 60-second lap (1,800 frames) typically compresses from 10,800 bytes to around
/// 5,425 bytes. It shares the save file's 7,532-byte tail with up to 103 bytes of exact
/// split metadata; encoding refuses a clip that cannot fit.
pub fn encode_compressed_ghost(
    track_id: u8,
    lap_time_ticks: u32,
    frames: &[GhostFrame],
    out: &mut [u8],
) -> Option<usize> {
    let required_len = compressed_ghost_encoded_len(frames)?;
    if out.len() < required_len {
        return None;
    }

    let mut pos = GHOST_HEADER_SIZE;
    let mut prev = GhostFrame::default();

    for (idx, &f) in frames.iter().enumerate() {
        let p_x = f.pos_x;
        let p_y = f.pos_y;
        let dx = (p_x as i32) - (prev.pos_x as i32);
        let dy = (p_y as i32) - (prev.pos_y as i32);
        let dh = (f.heading_byte.wrapping_sub(prev.heading_byte)) as i8;

        let can_delta = idx > 0
            && (-127..=127).contains(&dx)
            && (-127..=127).contains(&dy)
            && (-8..=7).contains(&dh)
            && (f.flags & 0xF0) == 0;

        if can_delta {
            if pos + 3 + GHOST_CRC_SIZE > out.len() {
                return None;
            }
            out[pos] = (dx as i8) as u8;
            out[pos + 1] = (dy as i8) as u8;
            out[pos + 2] = (((dh as u8) & 0x0F) << 4) | (f.flags & 0x0F);
            pos += 3;
        } else {
            if pos + 7 + GHOST_CRC_SIZE > out.len() {
                return None;
            }
            let xb = p_x.to_le_bytes();
            let yb = p_y.to_le_bytes();
            out[pos] = KEYFRAME_TAG;
            out[pos + 1] = xb[0];
            out[pos + 2] = xb[1];
            out[pos + 3] = yb[0];
            out[pos + 4] = yb[1];
            out[pos + 5] = f.heading_byte;
            out[pos + 6] = f.flags;
            pos += 7;
        }
        prev = f;
    }

    let stream_len = pos - GHOST_HEADER_SIZE;
    if stream_len > u16::MAX as usize {
        return None;
    }

    out[0..8].copy_from_slice(&GHOST_MAGIC);
    out[8..10].copy_from_slice(&GHOST_VERSION.to_le_bytes());
    out[10] = track_id;
    out[11..15].copy_from_slice(&lap_time_ticks.to_le_bytes());
    out[15..17].copy_from_slice(&(frames.len() as u16).to_le_bytes());
    out[17..19].copy_from_slice(&(stream_len as u16).to_le_bytes());

    let crc = crate::save::crc16(&out[..pos]);
    out[pos..pos + 2].copy_from_slice(&crc.to_le_bytes());
    Some(pos + 2)
}

/// Decodes a delta-compressed ghost stream created by [`encode_compressed_ghost`],
/// verifying magic, version, bounds, and CCITT-16 CRC (`NO1-024`).
///
/// Returns `Some((track_id, lap_time_ticks, frame_count))` on success.
pub fn decode_compressed_ghost(
    bytes: &[u8],
    out_frames: &mut [GhostFrame],
) -> Option<(u8, u32, usize)> {
    if bytes.len() < GHOST_HEADER_SIZE + GHOST_CRC_SIZE {
        return None;
    }
    if bytes[0..8] != GHOST_MAGIC {
        return None;
    }
    let version = u16::from_le_bytes([bytes[8], bytes[9]]);
    if version != GHOST_VERSION {
        return None;
    }
    let track_id = bytes[10];
    let lap_time_ticks = u32::from_le_bytes([bytes[11], bytes[12], bytes[13], bytes[14]]);
    let frame_count = u16::from_le_bytes([bytes[15], bytes[16]]) as usize;
    let stream_len = u16::from_le_bytes([bytes[17], bytes[18]]) as usize;

    let crc_offset = GHOST_HEADER_SIZE + stream_len;
    if crc_offset + GHOST_CRC_SIZE > bytes.len() || frame_count > out_frames.len() {
        return None;
    }

    let expected_crc = u16::from_le_bytes([bytes[crc_offset], bytes[crc_offset + 1]]);
    let actual_crc = crate::save::crc16(&bytes[..crc_offset]);
    if expected_crc != actual_crc {
        return None;
    }

    let mut pos = GHOST_HEADER_SIZE;
    let end = crc_offset;
    let mut prev = GhostFrame::default();

    for slot in out_frames.iter_mut().take(frame_count) {
        if pos >= end {
            return None;
        }
        let b0 = bytes[pos];
        let frame = if b0 == KEYFRAME_TAG {
            if pos + 7 > end {
                return None;
            }
            let pos_x = u16::from_le_bytes([bytes[pos + 1], bytes[pos + 2]]);
            let pos_y = u16::from_le_bytes([bytes[pos + 3], bytes[pos + 4]]);
            let heading_byte = bytes[pos + 5];
            let flags = bytes[pos + 6];
            pos += 7;
            GhostFrame {
                pos_x,
                pos_y,
                heading_byte,
                flags,
            }
        } else {
            if pos + 3 > end {
                return None;
            }
            let dx = (b0 as i8) as i32;
            let dy = (bytes[pos + 1] as i8) as i32;
            let b2 = bytes[pos + 2];
            let raw_dh = (b2 >> 4) & 0x0F;
            let dh: i8 = if raw_dh >= 8 {
                (raw_dh as i8) - 16
            } else {
                raw_dh as i8
            };
            let flags = b2 & 0x0F;
            pos += 3;
            let pos_x = (prev.pos_x as i32 + dx).clamp(0, POS_MAX_QUARTERS) as u16;
            let pos_y = (prev.pos_y as i32 + dy).clamp(0, POS_MAX_QUARTERS) as u16;
            let heading_byte = prev.heading_byte.wrapping_add(dh as u8);
            GhostFrame {
                pos_x,
                pos_y,
                heading_byte,
                flags,
            }
        };
        *slot = frame;
        prev = frame;
    }

    if pos != end {
        return None;
    }

    Some((track_id, lap_time_ticks, frame_count))
}

/// Evaluates the deterministic **Dev Platinum (Author) Ghost** on `track` at `current_tick`
/// (`NO1-025`).
///
/// Evaluates the circuit's closed Catmull-Rom racing line (`track.route`) paced to
/// `track.par_times.dev_platinum_ticks` with zero `.bss` RAM overhead, so every circuit
/// always has an Author Ghost to chase from Lap 1.
pub fn sample_author_ghost(
    track: &crate::track::TrackDef,
    current_tick: u32,
) -> Option<GhostPlaybackState> {
    let target_ticks = track.par_times.dev_platinum_ticks;
    if track.checkpoint_count == 0 || target_ticks == 0 || current_tick > target_ticks {
        return None;
    }

    let n = if !track.route.is_empty() {
        track.route.len()
    } else {
        track.route_len()
    };
    if n < 3 {
        return None;
    }

    let node_at = |idx: usize| -> Vec2 {
        if !track.route.is_empty() {
            track.route[idx % n]
        } else {
            crate::track::TrackDef::gate_centre(&track.route_node(idx % n))
        }
    };

    // Locate the route control point closest to the start line so tick 0 begins at start_pos.
    let mut start_idx = 0usize;
    let mut best_d2 = i64::MAX;
    for i in 0..n {
        let p = node_at(i);
        let dx = (p.x.raw() - track.start_pos.x.raw()) >> 8;
        let dy = (p.y.raw() - track.start_pos.y.raw()) >> 8;
        let d2 = (dx as i64) * (dx as i64) + (dy as i64) * (dy as i64);
        if d2 < best_d2 {
            best_d2 = d2;
            start_idx = i;
        }
    }

    let eval_at = |tick: u32| -> Vec2 {
        let clamped = tick.min(target_ticks);
        // Total progress in Q12 spans: `clamped * n * 4096 / target_ticks`
        let prog_q12 =
            ((clamped as u64) * (n as u64) * (math::FP_ONE as u64)) / (target_ticks.max(1) as u64);
        let span_offset = ((prog_q12 >> math::FP_SHIFT) as usize) % n;
        let t = Fixed::from_raw((prog_q12 & ((math::FP_ONE - 1) as u64)) as i32);

        let idx = (start_idx + span_offset) % n;
        let p0 = node_at((idx + n - 1) % n);
        let p1 = node_at(idx);
        let p2 = node_at((idx + 1) % n);
        let p3 = node_at((idx + 2) % n);
        crate::route::catmull_rom(p0, p1, p2, p3, t)
    };

    let pos = eval_at(current_tick);
    let ahead_pos = eval_at((current_tick + 3).min(target_ticks));
    let far_pos = eval_at((current_tick + 8).min(target_ticks));

    let heading = if pos == ahead_pos {
        track.start_heading
    } else {
        crate::ai::heading_towards(pos, ahead_pos)
    };
    let far_heading = if ahead_pos == far_pos {
        heading
    } else {
        crate::ai::heading_towards(ahead_pos, far_pos)
    };

    let curve_bam = crate::ai::angle_error(heading, far_heading).abs();
    let flags = if curve_bam > 96 {
        FLAG_DRIFTING
    } else if curve_bam < 24 {
        FLAG_BOOSTING
    } else {
        0
    };

    Some(GhostPlaybackState {
        position: pos,
        heading,
        flags,
    })
}

/// Computes the live time gap (in 60Hz ticks) between the player at `player_pos`
/// and the circuit's Dev Platinum Author Ghost at `current_tick` (`NO1-026`).
pub fn author_ghost_time_gap_ticks(
    track: &crate::track::TrackDef,
    player_pos: Vec2,
    current_tick: u32,
) -> Option<i32> {
    let target_ticks = track.par_times.dev_platinum_ticks;
    if track.checkpoint_count == 0 || target_ticks == 0 {
        return None;
    }
    let n = if !track.route.is_empty() {
        track.route.len()
    } else {
        track.route_len()
    };
    if n < 3 {
        return None;
    }
    let node_at = |idx: usize| -> Vec2 {
        if !track.route.is_empty() {
            track.route[idx % n]
        } else {
            crate::track::TrackDef::gate_centre(&track.route_node(idx % n))
        }
    };

    let mut start_idx = 0usize;
    let mut best_start_d2 = i64::MAX;
    let mut player_idx = 0usize;
    let mut best_player_d2 = i64::MAX;
    for i in 0..n {
        let p = node_at(i);
        let sdx = (p.x.raw() - track.start_pos.x.raw()) >> 8;
        let sdy = (p.y.raw() - track.start_pos.y.raw()) >> 8;
        let sd2 = (sdx as i64) * (sdx as i64) + (sdy as i64) * (sdy as i64);
        if sd2 < best_start_d2 {
            best_start_d2 = sd2;
            start_idx = i;
        }

        let pdx = (p.x.raw() - player_pos.x.raw()) >> 8;
        let pdy = (p.y.raw() - player_pos.y.raw()) >> 8;
        let pd2 = (pdx as i64) * (pdx as i64) + (pdy as i64) * (pdy as i64);
        if pd2 < best_player_d2 {
            best_player_d2 = pd2;
            player_idx = i;
        }
    }

    let spans_completed = (player_idx + n - start_idx) % n;
    let expected_author_tick =
        ((spans_completed as u64) * (target_ticks as u64) / (n as u64)) as i32;
    Some((current_tick as i32) - expected_author_tick)
}

/// Euclidean distance in world units between the player and a ghost vehicle (`NO1-026`).
pub fn ghost_distance_units(player_pos: Vec2, ghost_pos: Vec2) -> i32 {
    (player_pos - ghost_pos).length().to_int()
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

    #[test]
    fn delta_compressed_ghost_round_trips_losslessly_and_fits_memory_card_block() {
        // Simulate a full 60-second lap (1,800 frames at 30 Hz) with realistic motion,
        // drifting/boosting flags, and a mid-lap respawn jump.
        let mut frames = [GhostFrame::default(); 1800];
        let mut x = 300 * 4;
        let mut y = 400 * 4;
        let mut h: u8 = 0;
        for (i, slot) in frames.iter_mut().enumerate() {
            if i == 900 {
                // Simulate a large teleport/respawn that forces a 7-byte keyframe
                x += 800;
                y -= 600;
                h = h.wrapping_add(90);
            } else {
                x = (x + 12).clamp(0, 60000);
                y = (y + ((i as i32 % 5) - 2) * 8).clamp(0, 60000);
                h = h.wrapping_add(1);
            }
            *slot = GhostFrame {
                pos_x: x as u16,
                pos_y: y as u16,
                heading_byte: h,
                flags: (i as u8) & 0x0F,
            };
        }

        // A typical lap fits in the save file tail after its profile data.
        let mut compressed =
            [0u8; crate::save::CARD_FILE_DATA_CAPACITY - crate::save::PAYLOAD_SIZE];
        let written = encode_compressed_ghost(7, 3600, &frames, &mut compressed)
            .expect("60s delta-coded ghost must fit in remaining memory card block space");
        assert!(
            written < 5600,
            "1800 delta frames should be ~5430 bytes, got {written}"
        );

        let mut decoded = [GhostFrame::default(); 1800];
        let (track_id, lap_ticks, count) =
            decode_compressed_ghost(&compressed[..written], &mut decoded)
                .expect("valid compressed ghost must decode");
        assert_eq!(track_id, 7);
        assert_eq!(lap_ticks, 3600);
        assert_eq!(count, 1800);
        assert_eq!(&decoded[..count], &frames[..]);

        // Any single-bit flip in the compressed stream must be caught by CRC-16.
        compressed[written / 2] ^= 0x04;
        assert!(
            decode_compressed_ghost(&compressed[..written], &mut decoded).is_none(),
            "corrupted compressed ghost must be rejected by CRC-16"
        );
    }

    #[test]
    fn author_ghost_samples_valid_positions_across_shipped_circuits() {
        for track in crate::levels::ALL_TRACKS.iter() {
            let dev_ticks = track.par_times.dev_platinum_ticks;
            assert!(dev_ticks > 0);
            let start_sample =
                sample_author_ghost(track, 0).expect("Author ghost must sample at tick 0");
            let mid_sample = sample_author_ghost(track, dev_ticks / 2)
                .expect("Author ghost must sample at mid-lap");
            assert_ne!(
                start_sample.position, mid_sample.position,
                "{}: Author ghost must move around the circuit",
                track.name
            );
            assert!(
                sample_author_ghost(track, dev_ticks + 1).is_none(),
                "{}: Author ghost must finish after dev_platinum_ticks",
                track.name
            );
        }
    }

    #[test]
    fn ghost_time_gap_reports_ahead_and_behind() {
        let mut rec = GhostRecorder::<64>::new(0);
        rec.start();
        for tick in 0..60u32 {
            rec.record_tick(
                Vec2::new(Fixed::from_int((tick * 4) as i32), Fixed::from_int(100)),
                1024,
                0,
            );
        }
        rec.finish(60);
        let player = GhostPlayer::new(&rec.frames[..rec.frame_count], 60);

        // At tick 20, the ghost is at x = 80.
        // If the player is already at x = 120 (where the ghost is at tick 30),
        // the player is 10 ticks AHEAD (-10).
        let gap_ahead = player
            .time_gap_ticks(Vec2::new(Fixed::from_int(120), Fixed::from_int(100)), 20)
            .unwrap();
        assert!(
            gap_ahead < 0,
            "Player ahead of ghost position must report negative time gap, got {gap_ahead}"
        );

        // If at tick 30 the player is only at x = 80 (ghost's tick 20 position),
        // the player is 10 ticks BEHIND (+10).
        let gap_behind = player
            .time_gap_ticks(Vec2::new(Fixed::from_int(80), Fixed::from_int(100)), 30)
            .unwrap();
        assert!(
            gap_behind > 0,
            "Player behind ghost position must report positive time gap, got {gap_behind}"
        );
    }
}
