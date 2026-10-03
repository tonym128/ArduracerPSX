//! In-Game Arcade Racing HUD.
//!
//! Renders the analogue tacho + gear indicator, lap counter, checkpoint tracker,
//! current lap timer, live delta split, and a dynamic minimap with the full
//! circuit outline plus position blips (GAME.md §TASK-604).

use crate::ui::font::{draw_char, draw_text};
use arduracer_core::{AiRacer, LapTimer, TrackDef, TrackTile, VehicleState, NITRO_MAX_TICKS};
use psx_gpu as gpu;

pub const HUD_MARGIN: u16 = 8;
/// Width of the minimap panel in pixels.
pub const MINIMAP_SIZE: u16 = 52;
/// Inner drawing area of the minimap panel.
const MINIMAP_INNER: i32 = (MINIMAP_SIZE - 8) as i32;

/// Formats `MM:SS.ccc` into a fixed 8 character buffer (no allocation).
fn format_time(buf: &mut [u8; 8], ticks: u32) {
    let (m, s, cs) = LapTimer::<1>::format_ticks(ticks);
    let digits = [
        b'0' + (m / 10) as u8 % 10,
        b'0' + (m % 10) as u8,
        b':',
        b'0' + (s / 10) as u8,
        b'0' + (s % 10) as u8,
        b'.',
        b'0' + (cs / 10) as u8,
        b'0' + (cs % 10) as u8,
    ];
    buf.copy_from_slice(&digits);
}

/// Formats a signed second delta as `+0.45` / `-0.12` into a 6 char buffer.
fn format_delta(buf: &mut [u8; 6], delta_ticks: i32) {
    let sign = if delta_ticks < 0 { b'-' } else { b'+' };
    let abs = delta_ticks.unsigned_abs();
    let whole = (abs / 60) % 100;
    let frac = ((abs % 60) * 100) / 60;
    buf.copy_from_slice(&[
        sign,
        b'0' + (whole / 10) as u8,
        b'0' + (whole % 10) as u8,
        b'.',
        b'0' + (frac / 10) as u8,
        b'0' + (frac % 10) as u8,
    ]);
}

/// Draws a NUL/space padded byte buffer without allocating (the game is
/// `#![no_std]` with no global allocator).
fn blit(buf: &[u8], x: u16, y: u16, color: (u8, u8, u8), scale: u16) {
    let mut cur_x = x;
    for &b in buf.iter() {
        if b != b' ' {
            draw_char(cur_x, y, b, color, scale);
        }
        cur_x += 6 * scale;
    }
}

/// Renders the complete in-game HUD overlay.
pub fn render_hud<const N: usize>(
    player: &VehicleState,
    timer: &LapTimer<N>,
    track: &TrackDef,
    rank: u8,
    rivals: &[AiRacer],
    draw_y: i16,
) {
    let base_y = draw_y as u16;

    // ---------------------------------------------------------------- 1. Tacho
    // Left-to-right rev bar driven by engine RPM, coloured by rev band.
    gpu::fill_rect(206, base_y + 10, 106, 10, 18, 18, 28);
    let rpm_ratio = ((player.engine_rpm as i32 - 1000) * 100) / 7000;
    let fill_w = rpm_ratio.clamp(0, 100) as u16;
    let bar = if player.boost_ticks > 0 {
        (0, 210, 255) // Neon cyan while boosting
    } else if player.engine_rpm > 6500 {
        (255, 40, 40) // Redline
    } else if player.engine_rpm > 4500 {
        (255, 200, 30) // Upper mid
    } else {
        (40, 220, 60) // Cruising
    };
    if fill_w > 0 {
        gpu::fill_rect(209, base_y + 12, fill_w, 6, bar.0, bar.1, bar.2);
    }

    // Nitro meter directly beneath the rev bar.
    gpu::fill_rect(206, base_y + 22, 106, 6, 18, 18, 28);
    let nitro_w =
        ((player.nitro_charge as i32 * 102) / NITRO_MAX_TICKS as i32).clamp(0, 102) as u16;
    if nitro_w > 0 {
        let c = if player.nitro_charge > NITRO_MAX_TICKS / 3 {
            (255, 150, 40)
        } else {
            (90, 110, 140)
        };
        gpu::fill_rect(208, base_y + 23, nitro_w, 4, c.0, c.1, c.2);
    }

    // Gear indicator (large, right of the tacho).
    let gear_char = if player.is_reversing {
        b'R'
    } else {
        b'0' + player.gear.clamp(1, 5)
    };
    let gear_col = if player.is_reversing {
        (255, 160, 40)
    } else {
        (240, 240, 250)
    };
    let mut gear_buf = [b' '; 1];
    gear_buf[0] = gear_char;
    blit(&gear_buf, 290, base_y + 24, gear_col, 2);
    draw_text(284, base_y + 40, "GEAR", (150, 150, 165), 1);

    // ------------------------------------------------- 2. Lap / checkpoint HUD
    for lap_idx in 0..5u16 {
        let x = 12 + lap_idx * 12;
        let is_current = lap_idx + 1 == timer.current_lap as u16;
        let is_done = lap_idx + 1 < timer.current_lap as u16;
        let (lr, lg, lb) = if is_current {
            (255, 220, 0)
        } else if is_done {
            (40, 200, 60)
        } else {
            (60, 60, 70)
        };
        gpu::fill_rect(x, base_y + 10, 9, 9, lr, lg, lb);
    }

    let cleared = timer.checkpoints_cleared();
    for cp_idx in 0..(timer.total_checkpoints as u16) {
        let x = 12 + cp_idx * 9;
        let is_passed = (cp_idx as u32) < cleared;
        let (cr, cg, cb) = if is_passed {
            (0, 220, 255)
        } else {
            (50, 50, 60)
        };
        gpu::fill_rect(x, base_y + 24, 7, 4, cr, cg, cb);
    }
    draw_text(12, base_y + 32, "LAPS", (150, 150, 165), 1);

    // Race position (top centre).
    let (rank_str, rank_col) = match rank {
        1 => ("1ST", (255, 215, 0)),
        2 => ("2ND", (215, 215, 225)),
        3 => ("3RD", (205, 127, 50)),
        4 => ("4TH", (180, 200, 220)),
        5 => ("5TH", (160, 180, 200)),
        _ => ("6TH", (140, 150, 170)),
    };
    draw_text(120, base_y + 10, rank_str, rank_col, 2);

    // ------------------------------------------------ 3. Lap timer + delta
    let mut time_buf = [b' '; 8];
    format_time(&mut time_buf, timer.current_lap_ticks);
    blit(&time_buf, 96, base_y + 30, (245, 245, 250), 2);

    let delta = timer.delta_ticks();
    let mut delta_buf = [b' '; 6];
    format_delta(&mut delta_buf, delta);
    let delta_col = if delta < 0 {
        (60, 235, 90)
    } else {
        (240, 70, 70)
    };
    if timer.best_lap_ticks != u32::MAX {
        blit(&delta_buf, 120, base_y + 48, delta_col, 1);
    }

    // Best lap readout.
    if timer.best_lap_ticks != u32::MAX {
        let mut best_buf = [b' '; 8];
        format_time(&mut best_buf, timer.best_lap_ticks);
        draw_text(12, base_y + 44, "BEST", (150, 150, 165), 1);
        blit(&best_buf, 12, base_y + 52, (200, 210, 235), 1);
    }

    // ------------------------------------------------------- 4. Minimap panel
    let map_x = 10u16;
    let map_y = base_y + 170;
    gpu::fill_rect(map_x, map_y, MINIMAP_SIZE, MINIMAP_SIZE, 12, 15, 20);

    let step_x = MINIMAP_INNER / (track.width as i32).max(1);
    let step_y = MINIMAP_INNER / (track.height as i32).max(1);

    // Circuit outline so the player can read the layout at a glance.
    for ty in 0..track.height as i32 {
        for tx in 0..track.width as i32 {
            let tile = track.tile_at(tx as u8, ty as u8);
            let (r, g, b) = match tile {
                TrackTile::StartFinish => (255, 255, 255),
                TrackTile::Checkpoint => (0, 200, 255),
                TrackTile::Curb => (180, 180, 190),
                TrackTile::BoostPad => (255, 150, 0),
                _ if tile.is_road() => (110, 118, 132),
                TrackTile::Barrier => (40, 40, 48),
                _ => (28, 52, 32),
            };
            let px = map_x + 4 + (tx * step_x) as u16;
            let py = map_y + 4 + (ty * step_y) as u16;
            let w = (step_x.max(1) + 1) as u16;
            let h = (step_y.max(1) + 1) as u16;
            gpu::fill_rect(px, py, w, h, r, g, b);
        }
    }

    // Rival blips, then the player blip on top.
    for rival in rivals {
        let r_tx = TrackDef::tile_x_of(rival.state.position.x) as i32;
        let r_ty = TrackDef::tile_y_of(rival.state.position.y) as i32;
        let (rc, gc, bc) = rival.profile.color;
        gpu::fill_rect(
            map_x + 4 + (r_tx * step_x) as u16,
            map_y + 4 + (r_ty * step_y) as u16,
            2,
            2,
            rc,
            gc,
            bc,
        );
    }
    let p_tx = TrackDef::tile_x_of(player.position.x) as i32;
    let p_ty = TrackDef::tile_y_of(player.position.y) as i32;
    gpu::fill_rect(
        map_x + 3 + (p_tx * step_x) as u16,
        map_y + 3 + (p_ty * step_y) as u16,
        3,
        3,
        255,
        230,
        0,
    );
}
