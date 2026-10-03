//! In-Game Arcade Racing HUD for PlayStation 1.
//!
//! Renders the tachometer rev bar, nitro gauge, digital speedometer, gear indicator,
//! lap counter, checkpoint tracker, lap timer, live delta split, and dynamic
//! minimap with full circuit outline and racer blips.

use crate::ui::font::{draw_char, draw_text};
use arduracer_core::{AiRacer, LapTimer, TrackDef, TrackTile, VehicleState, NITRO_MAX_TICKS};
use psx_gpu as gpu;

pub const HUD_MARGIN: u16 = 8;
/// Width of the minimap panel in pixels.
pub const MINIMAP_SIZE: u16 = 56;
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

/// Formats integer speed into a 7 char buffer `142 MPH`.
fn format_speed(buf: &mut [u8; 7], mph: u16) {
    let h = (mph / 100) as u8;
    let t = ((mph / 10) % 10) as u8;
    let o = (mph % 10) as u8;
    buf[0] = if h > 0 { b'0' + h } else { b' ' };
    buf[1] = if h > 0 || t > 0 { b'0' + t } else { b' ' };
    buf[2] = b'0' + o;
    buf[3] = b' ';
    buf[4] = b'M';
    buf[5] = b'P';
    buf[6] = b'H';
}

/// Draws a NUL/space padded byte buffer without allocating.
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
    _draw_y: i16,
) {
    // ---------------------------------------------------------------- 1. Tacho & Speed
    // Tachometer border and backing
    gpu::draw_rect_flat(206, 10, 106, 10, 18, 18, 28);
    let rpm_ratio = ((player.engine_rpm as i32 - 1000) * 100) / 7000;
    let fill_w = rpm_ratio.clamp(0, 100) as u16;
    let bar = if player.boost_ticks > 0 {
        (0, 220, 255) // Neon cyan while boosting
    } else if player.engine_rpm > 6500 {
        (255, 40, 40) // Redline warning
    } else if player.engine_rpm > 4500 {
        (255, 200, 30) // Power band
    } else {
        (40, 220, 70) // Cruising
    };
    if fill_w > 0 {
        gpu::draw_rect_flat(209, 12, fill_w, 6, bar.0, bar.1, bar.2);
    }

    // Nitro charge bar directly beneath the rev bar
    gpu::draw_rect_flat(206, 22, 106, 6, 18, 18, 28);
    let nitro_w =
        ((player.nitro_charge as i32 * 102) / NITRO_MAX_TICKS as i32).clamp(0, 102) as u16;
    if nitro_w > 0 {
        let (nr, ng, nb) = if player.nitro_charge > NITRO_MAX_TICKS / 3 {
            (255, 150, 40) // Full / active charge
        } else {
            (80, 110, 140) // Low charge
        };
        gpu::draw_rect_flat(208, 23, nitro_w, 4, nr, ng, nb);
    }

    // Digital Speedometer (approx. MPH based on forward speed)
    let speed_mph = ((player.speed.raw() as i32 * 145) / 14000).clamp(0, 199) as u16;
    let mut speed_buf = [b' '; 7];
    format_speed(&mut speed_buf, speed_mph);
    blit(&speed_buf, 206, 32, (240, 240, 250), 1);

    // Gear indicator (large bold, right of the speedo)
    let gear_char = if player.is_reversing {
        b'R'
    } else {
        b'0' + player.gear.clamp(1, 5)
    };
    let gear_col = if player.is_reversing {
        (255, 160, 40)
    } else {
        (255, 225, 40)
    };
    let mut gear_buf = [b' '; 1];
    gear_buf[0] = gear_char;
    blit(&gear_buf, 290, 24, gear_col, 2);
    draw_text(284, 40, "GEAR", (150, 150, 165), 1);

    // ------------------------------------------------- 2. Lap / checkpoint HUD
    // Laps boxes (top-left)
    for lap_idx in 0..5u16 {
        let x = 12 + lap_idx * 12;
        let is_current = lap_idx + 1 == timer.current_lap as u16;
        let is_done = lap_idx + 1 < timer.current_lap as u16;
        let (lr, lg, lb) = if is_current {
            (255, 220, 0)
        } else if is_done {
            (40, 210, 70)
        } else {
            (50, 52, 62)
        };
        gpu::draw_rect_flat(x as i16, 10, 9, 9, lr, lg, lb);
    }

    // Checkpoint progress dots
    let cleared = timer.checkpoints_cleared();
    for cp_idx in 0..(timer.total_checkpoints as u16) {
        let x = 12 + cp_idx * 9;
        let is_passed = (cp_idx as u32) < cleared;
        let (cr, cg, cb) = if is_passed {
            (0, 220, 255)
        } else {
            (45, 48, 58)
        };
        gpu::draw_rect_flat(x as i16, 24, 7, 4, cr, cg, cb);
    }
    draw_text(12, 32, "LAPS", (150, 150, 165), 1);

    // Race position (top centre, bold 2x)
    let (rank_str, rank_col) = match rank {
        1 => ("1ST", (255, 215, 0)),
        2 => ("2ND", (220, 225, 235)),
        3 => ("3RD", (210, 130, 50)),
        4 => ("4TH", (180, 200, 220)),
        5 => ("5TH", (160, 180, 200)),
        _ => ("6TH", (140, 150, 170)),
    };
    draw_text(140, 10, rank_str, rank_col, 2);

    // ------------------------------------------------ 3. Lap timer + delta
    let mut time_buf = [b' '; 8];
    format_time(&mut time_buf, timer.current_lap_ticks);
    blit(&time_buf, 116, 30, (245, 245, 250), 2);

    let delta = timer.delta_ticks();
    let mut delta_buf = [b' '; 6];
    format_delta(&mut delta_buf, delta);
    let delta_col = if delta < 0 {
        (60, 240, 90) // Ahead of pace: vivid green
    } else {
        (245, 60, 60) // Behind pace: bright red
    };
    if timer.best_lap_ticks != u32::MAX {
        blit(&delta_buf, 140, 48, delta_col, 1);
    }

    // Best lap record readout
    if timer.best_lap_ticks != u32::MAX {
        let mut best_buf = [b' '; 8];
        format_time(&mut best_buf, timer.best_lap_ticks);
        draw_text(12, 44, "BEST", (150, 150, 165), 1);
        blit(&best_buf, 12, 52, (200, 215, 240), 1);
    }

    // ------------------------------------------------------- 4. Minimap panel
    let map_x = 10i16;
    let map_y = 174i16;
    gpu::draw_rect_flat(map_x, map_y, MINIMAP_SIZE, MINIMAP_SIZE, 14, 16, 24);
    gpu::draw_rect_flat(
        map_x + 1,
        map_y + 1,
        MINIMAP_SIZE - 2,
        MINIMAP_SIZE - 2,
        22,
        26,
        36,
    );

    let step_x = MINIMAP_INNER / (track.width as i32).max(1);
    let step_y = MINIMAP_INNER / (track.height as i32).max(1);

    // Circuit track outline
    for ty in 0..track.height as i32 {
        for tx in 0..track.width as i32 {
            let tile = track.tile_at(tx as u8, ty as u8);
            let (r, g, b) = match tile {
                TrackTile::StartFinish => (255, 255, 255),
                TrackTile::Checkpoint => (0, 210, 255),
                TrackTile::Curb => (180, 185, 195),
                TrackTile::BoostPad => (255, 160, 20),
                _ if tile.is_road() => (100, 110, 125),
                TrackTile::Barrier => (35, 35, 45),
                _ => (25, 45, 28),
            };
            let px = map_x + 4 + (tx * step_x) as i16;
            let py = map_y + 4 + (ty * step_y) as i16;
            let w = (step_x.max(1) + 1) as u16;
            let h = (step_y.max(1) + 1) as u16;
            gpu::draw_rect_flat(px, py, w, h, r, g, b);
        }
    }

    // Rival blips
    for rival in rivals {
        let r_tx = TrackDef::tile_x_of(rival.state.position.x) as i32;
        let r_ty = TrackDef::tile_y_of(rival.state.position.y) as i32;
        let (rc, gc, bc) = rival.profile.color;
        gpu::draw_rect_flat(
            map_x + 4 + (r_tx * step_x) as i16,
            map_y + 4 + (r_ty * step_y) as i16,
            2,
            2,
            rc,
            gc,
            bc,
        );
    }

    // Player position blip (bright golden dot)
    let p_tx = TrackDef::tile_x_of(player.position.x) as i32;
    let p_ty = TrackDef::tile_y_of(player.position.y) as i32;
    gpu::draw_rect_flat(
        map_x + 3 + (p_tx * step_x) as i16,
        map_y + 3 + (p_ty * step_y) as i16,
        3,
        3,
        255,
        230,
        0,
    );
}
