//! Real-time Ghost Car Playback and Rendering.
//!
//! Reconstructs sub-tick interpolated vehicle positions and renders the
//! translucent ghost rival on the racetrack.

use crate::ghost_recorder::LapGhostRecorder;
use crate::gpu::{render_car, Camera};
use arduracer_core::ghost::{sample_author_ghost, GhostPlayer, FLAG_BOOSTING, FLAG_DRIFTING};
use arduracer_core::{TrackDef, VehicleState};

/// Evaluates and renders both the circuit's **Dev Platinum Author Ghost** (Gold)
/// and the player's **Personal Best Ghost** (Cyan) when active (`NO1-025`).
pub fn render_active_ghost(
    recorder: &LapGhostRecorder,
    track: &TrackDef,
    camera: &Camera,
    current_lap_ticks: u32,
) {
    if current_lap_ticks == 0 {
        return;
    }

    // 1. Render Dev Platinum Author Ghost (shimmering gold phantom) on every circuit
    if let Some(author) = sample_author_ghost(track, current_lap_ticks) {
        let author_vehicle = VehicleState {
            position: author.position,
            visual_angle: author.heading,
            heading: author.heading,
            is_drifting: (author.flags & FLAG_DRIFTING) != 0,
            boost_ticks: if (author.flags & FLAG_BOOSTING) != 0 {
                4
            } else {
                0
            },
            ..VehicleState::default()
        };
        render_car(&author_vehicle, camera, true, (255, 210, 45));
    }

    // 2. Render Personal Best Ghost (translucent cyan phantom) if recorded
    if !recorder.has_ghost || recorder.best_frame_count == 0 {
        return;
    }

    let frames = &recorder.best_frames[..recorder.best_frame_count];
    let total_ticks = if recorder.best_lap_ticks > 0 {
        recorder.best_lap_ticks
    } else {
        recorder.recorder.lap_time_ticks
    };
    let player = GhostPlayer::new(frames, total_ticks);
    if player.is_exhausted(current_lap_ticks) {
        return;
    }
    let sample = player.sample_at_tick(current_lap_ticks);

    let ghost_vehicle = VehicleState {
        position: sample.position,
        visual_angle: sample.heading,
        heading: sample.heading,
        is_drifting: (sample.flags & FLAG_DRIFTING) != 0,
        boost_ticks: if (sample.flags & FLAG_BOOSTING) != 0 {
            4
        } else {
            0
        },
        ..VehicleState::default()
    };

    // Render as translucent cyan phantom car (is_ghost = true)
    render_car(&ghost_vehicle, camera, true, (100, 180, 255));
}
