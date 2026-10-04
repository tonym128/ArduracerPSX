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
use arduracer_core::VehicleState;

/// 60 seconds of telemetry at 30 Hz. Good laps are 15-25 s, so this leaves
/// room for a slow lap before the recording truncates.
pub const MAX_GHOST_FRAMES: usize = 1800;

pub struct LapGhostRecorder {
    pub recorder: GhostRecorder<MAX_GHOST_FRAMES>,
    pub best_frames: [GhostFrame; MAX_GHOST_FRAMES],
    pub best_frame_count: usize,
    pub has_ghost: bool,
}

impl LapGhostRecorder {
    pub fn new(track_id: u8) -> Self {
        LapGhostRecorder {
            recorder: GhostRecorder::new(track_id),
            best_frames: [GhostFrame::default(); MAX_GHOST_FRAMES],
            best_frame_count: 0,
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
        if player.boost_ticks > 0 {
            flags |= FLAG_BOOSTING;
        }
        if player.speed < arduracer_core::Fixed::from_int(1) && player.engine_rpm < 1500 {
            flags |= FLAG_BRAKING;
        }

        self.recorder
            .record_tick(player.position, player.visual_angle, flags);
    }

    /// Completes the recorded lap. If it beats the current best, updates the ghost.
    pub fn finish_lap(&mut self, lap_ticks: u32, is_new_record: bool) {
        self.recorder.finish(lap_ticks);
        if is_new_record && self.recorder.frame_count > 0 {
            let count = self.recorder.frame_count.min(MAX_GHOST_FRAMES);
            self.best_frames[..count].copy_from_slice(&self.recorder.frames[..count]);
            self.best_frame_count = count;
            self.has_ghost = true;
        }
    }

    /// Clears recorded ghost data.
    pub fn reset(&mut self, track_id: u8) {
        self.recorder = GhostRecorder::new(track_id);
        self.best_frame_count = 0;
        self.has_ghost = false;
    }
}
