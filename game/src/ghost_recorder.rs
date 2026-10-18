//! Telemetry Recorder for Ghost Replays.
//!
//! Captures 30Hz compressed keyframes during a race, promoting the fastest
//! completed lap to the active track ghost.
//!
//! The race loop calls [`LapGhostRecorder::finish_lap`] and
//! [`LapGhostRecorder::start_lap`] at every lap boundary, so the ghost is a
//! whole lap rather than the opening seconds of the first one.

use arduracer_core::ghost::{
    GhostFrame, GhostRecorder, FLAG_BOOSTING, FLAG_BRAKING, FLAG_DRIFTING,
};
use arduracer_core::track::MAX_TRACK_CHECKPOINTS;
use arduracer_core::VehicleState;

/// 60 seconds of telemetry at 30 Hz. Good laps are 15-25 s, so this leaves
/// room for a slow lap before the recording truncates.
pub const MAX_GHOST_FRAMES: usize = 1800;

pub struct LapGhostRecorder {
    pub recorder: GhostRecorder<MAX_GHOST_FRAMES>,
    pub best_frames: [GhostFrame; MAX_GHOST_FRAMES],
    pub best_frame_count: usize,
    pub best_lap_ticks: u32,
    pub best_splits: [u32; MAX_TRACK_CHECKPOINTS],
    pub best_split_count: usize,
    pub has_ghost: bool,
}

impl LapGhostRecorder {
    pub fn new(track_id: u8) -> Self {
        LapGhostRecorder {
            recorder: GhostRecorder::new(track_id),
            best_frames: [GhostFrame::default(); MAX_GHOST_FRAMES],
            best_frame_count: 0,
            best_lap_ticks: 0,
            best_splits: [0; MAX_TRACK_CHECKPOINTS],
            best_split_count: 0,
            has_ghost: false,
        }
    }

    /// Begins recording a fresh lap.
    pub fn start_lap(&mut self) {
        self.recorder.start();
    }

    /// Records current frame telemetry during 60Hz game loop.
    pub fn record_tick(&mut self, player: &VehicleState) {
        let mut flags = 0u8;
        if player.is_drifting {
            flags |= FLAG_DRIFTING;
        }
        if player.boost_ticks > 0 || player.super_boost_ticks > 0 {
            flags |= FLAG_BOOSTING;
        }
        if player.speed < arduracer_core::Fixed::from_int(1) && player.engine_rpm < 1500 {
            flags |= FLAG_BRAKING;
        }

        self.recorder
            .record_tick(player.position, player.visual_angle, flags);
    }

    /// Completes the recorded lap. If it beats the current best (or no ghost exists yet),
    /// promotes the lap to `best_frames`.
    pub fn finish_lap(&mut self, lap_ticks: u32, is_new_record: bool, splits: &[u32]) {
        self.recorder.finish(lap_ticks);
        if (is_new_record || !self.has_ghost)
            && self.recorder.frame_count > 0
            && lap_ticks > 0
            && !self.recorder.is_truncated
            && splits.len() <= MAX_TRACK_CHECKPOINTS
        {
            let count = self.recorder.frame_count.min(MAX_GHOST_FRAMES);
            self.best_frames[..count].copy_from_slice(&self.recorder.frames[..count]);
            self.best_frame_count = count;
            self.best_lap_ticks = lap_ticks;
            self.best_splits.fill(0);
            self.best_splits[..splits.len()].copy_from_slice(splits);
            self.best_split_count = splits.len();
            self.has_ghost = true;
        }
    }

    /// Returns the best available telemetry clip `(frames, total_ticks)` for replay playback.
    pub fn active_replay_clip(&self) -> (&[GhostFrame], u32) {
        if self.has_ghost && self.best_frame_count > 0 {
            (
                &self.best_frames[..self.best_frame_count],
                self.best_lap_ticks.max(1),
            )
        } else {
            let count = self.recorder.frame_count.min(MAX_GHOST_FRAMES);
            (
                &self.recorder.frames[..count],
                self.recorder.lap_time_ticks.max(1),
            )
        }
    }

    /// Encodes the active Personal Best ghost into the unused tail of an 8 KB Memory Card block (`NO1-024`).
    pub fn save_to_card_block(&self, block: &mut [u8; 8192]) -> bool {
        if self.has_ghost && self.best_frame_count > 0 {
            arduracer_core::SaveData::write_ghost_to_block(
                block,
                self.recorder.track_id,
                self.best_lap_ticks,
                &self.best_frames[..self.best_frame_count],
                &self.best_splits[..self.best_split_count],
            )
            .is_some()
        } else {
            false
        }
    }

    /// Restores a Personal Best ghost from an 8 KB Memory Card block if it matches `expected_track_id` (`NO1-024`).
    pub fn load_from_card_block(&mut self, block: &[u8; 8192], expected_track_id: u8) -> bool {
        if let Some((track_id, lap_ticks, count)) =
            arduracer_core::SaveData::read_ghost_from_block(block, &mut self.best_frames)
        {
            if track_id == expected_track_id && count > 0 && lap_ticks > 0 {
                self.best_frame_count = count.min(MAX_GHOST_FRAMES);
                self.best_lap_ticks = lap_ticks;
                self.best_splits.fill(0);
                self.best_split_count = arduracer_core::SaveData::read_ghost_splits_from_block(
                    block,
                    &mut self.best_splits,
                )
                .unwrap_or(0);
                self.has_ghost = true;
                return true;
            }
        }
        false
    }

    /// Clears recorded ghost data.
    pub fn reset(&mut self, track_id: u8) {
        self.recorder = GhostRecorder::new(track_id);
        self.best_frame_count = 0;
        self.best_lap_ticks = 0;
        self.best_splits.fill(0);
        self.best_split_count = 0;
        self.has_ghost = false;
    }
}
