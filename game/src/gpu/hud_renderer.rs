//! In-Game Arcade Racing HUD.
//!
//! Renders speed bar, tachometer, lap counter, checkpoint progress,
//! digital timer, and dynamic corner minimap.

use arduracer_core::{LapTimer, TrackDef, VehicleState};
use psx_gpu as gpu;

pub const HUD_MARGIN: u16 = 8;

/// Renders the complete in-game HUD overlay.
pub fn render_hud<const N: usize>(
    player: &VehicleState,
    timer: &LapTimer<N>,
    track: &TrackDef,
    draw_y: i16,
) {
    let base_y = draw_y as u16;

    // 1. Tachometer / Speed Bar (Top Right)
    // Background bar
    gpu::fill_rect(220, base_y + 10, 90, 8, 20, 20, 30);
    // Speed fill
    let speed_ratio = (player.speed.raw() as i64 * 88) / 14000;
    let fill_w = (speed_ratio.max(0).min(88)) as u16;

    let (r, g, b) = if player.boost_ticks > 0 {
        (0, 200, 255) // Neon Cyan on boost
    } else if player.engine_rpm > 6500 {
        (255, 40, 40) // Redline
    } else if player.engine_rpm > 4500 {
        (255, 200, 30) // High revs (yellow)
    } else {
        (40, 220, 60) // Cruising (green)
    };
    if fill_w > 0 {
        gpu::fill_rect(221, base_y + 11, fill_w, 6, r, g, b);
    }

    // 2. Lap Indicator & Checkpoint Tracker (Top Left)
    // Lap boxes (1 to 5)
    for lap_idx in 0..5 {
        let x = 12 + (lap_idx as u16) * 12;
        let is_current = (lap_idx + 1) == (timer.current_lap as u16);
        let is_done = (lap_idx + 1) < (timer.current_lap as u16);

        let (lr, lg, lb) = if is_current {
            (255, 220, 0) // Yellow active
        } else if is_done {
            (40, 200, 60) // Green cleared
        } else {
            (60, 60, 70) // Inactive gray
        };
        gpu::fill_rect(x, base_y + 10, 8, 8, lr, lg, lb);
    }

    // Checkpoint ticks
    for cp_idx in 0..(timer.total_checkpoints as u16) {
        let x = 12 + (cp_idx as u16) * 8;
        let is_passed = cp_idx < (timer.next_checkpoint_idx as u16);
        let (cr, cg, cb) = if is_passed {
            (0, 220, 255) // Passed cyan
        } else {
            (50, 50, 60) // Pending
        };
        gpu::fill_rect(x, base_y + 22, 6, 4, cr, cg, cb);
    }

    // 3. Corner Minimap (Bottom Left: 48x48)
    let map_x = 10u16;
    let map_y = base_y + 180;
    gpu::fill_rect(map_x, map_y, 48, 48, 15, 18, 24);
    // Draw border
    gpu::fill_rect(map_x, map_y, 48, 1, 80, 85, 95);
    gpu::fill_rect(map_x, map_y + 47, 48, 1, 80, 85, 95);
    gpu::fill_rect(map_x, map_y, 1, 48, 80, 85, 95);
    gpu::fill_rect(map_x + 47, map_y, 1, 48, 80, 85, 95);

    // Player position blip on minimap
    let p_tx = player.position.x.to_int() / 64;
    let p_ty = player.position.y.to_int() / 64;
    let blip_x = map_x
        + (((p_tx as i32 * 44) / (track.width as i32).max(1))
            .max(2)
            .min(44)) as u16;
    let blip_y = map_y
        + (((p_ty as i32 * 44) / (track.height as i32).max(1))
            .max(2)
            .min(44)) as u16;
    gpu::fill_rect(blip_x, blip_y, 3, 3, 255, 230, 0); // Blinking yellow player dot
}
