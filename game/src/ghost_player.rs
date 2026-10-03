//! Real-time Ghost Car Playback and Rendering.
//!
//! Reconstructs sub-tick interpolated vehicle positions and renders the
//! translucent ghost rival on the racetrack.

use crate::ghost_recorder::LapGhostRecorder;
use crate::gpu::{render_car, Camera};
use arduracer_core::ghost::GhostPlayer;
use arduracer_core::VehicleState;

/// Evaluates and renders the ghost vehicle if telemetry is available for the current circuit.
pub fn render_active_ghost(
    recorder: &LapGhostRecorder,
    camera: &Camera,
    current_lap_ticks: u32,
    draw_y: i16,
) {
    if !recorder.has_ghost || recorder.best_frame_count == 0 {
        return;
    }

    let frames = &recorder.best_frames[..recorder.best_frame_count];
    let player = GhostPlayer::new(frames, recorder.recorder.lap_time_ticks);
    let sample = player.sample_at_tick(current_lap_ticks);

    let mut ghost_vehicle = VehicleState::default();
    ghost_vehicle.position = sample.position;
    ghost_vehicle.visual_angle = sample.heading;
    ghost_vehicle.heading = sample.heading;

    // Render as translucent cyan phantom car (is_ghost = true)
    render_car(&ghost_vehicle, camera, draw_y, true, (100, 160, 230));
}
