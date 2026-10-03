//! Telemetry Recorder for Ghost Replays.
//!
//! Captures 30Hz compressed keyframes during Time Trials, promoting the fastest
//! clean lap to the active track ghost.

use arduracer_core::ghost::{
    GhostFrame, GhostRecorder, FLAG_BOOSTING, FLAG_BRAKING, FLAG_DRIFTING,
};
use arduracer_core::VehicleState;

pub const MAX_GHOST_FRAMES: usize = 900; // 30 seconds at 30 Hz

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
