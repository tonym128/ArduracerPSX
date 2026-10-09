//! In-Game Arcade Racing HUD for PlayStation 1.
//!
//! Renders the tachometer rev bar, nitro gauge, digital speedometer, gear indicator,
//! lap counter, checkpoint tracker, lap timer, live delta split, and the minimap
//! with full circuit outline and racer blips.
//!
//! The minimap's static half -- the circuit outline -- is *baked into a VRAM
//! texture once per track load* by [`bake_minimap`] and blitted as a single sprite
//! each frame. It used to be rebuilt from scratch every frame, one GP0 rectangle
//! per tile across the whole grid: 1,444 rectangles and ~7,200 GP0 words on the
//! padded 38x38 circuits, about 43 % of the 16.67 ms frame budget, to redraw an
//! image that cannot change while the circuit does not (TASK-1202). The moving
//! half -- the racer blips -- stays as primitives drawn on top.

use crate::gpu::texlayout::{minimap_colour, minimap_tile_rect, minimap_world_pos, MINIMAP_SIZE};
use crate::gpu::texpipe::TextureSlot;
use crate::ui::font::{draw_char, draw_text};
use arduracer_core::{AiRacer, LapTimer, TrackDef, VehicleState, NITRO_MAX_TICKS};
use psx_gpu as gpu;
use psx_gpu::material::BlendMode;

/// Bakes the circuit outline into `slot` and uploads it.
///
/// Call once per track load, never per frame: the upload is a DMA transfer and
/// the image is static for the whole race.
pub fn bake_minimap(slot: &TextureSlot, track: &TrackDef) {
    slot.begin_compose();
    for ty in 0..track.height {
        for tx in 0..track.width {
            let tile = track.tile_at(tx, ty);
            let (r, g, b) = minimap_colour(tile);
            let (px, py, pw, ph) = minimap_tile_rect(tx, ty, track.width, track.height);
            slot.fill_rect(px, py, pw, ph, (r, g, b));
        }
    }
    slot.upload();
}

pub const HUD_MARGIN: u16 = 8;

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
    minimap_texture: &TextureSlot,
) {
    // ------------------------------------------------- 1. Lap / Checkpoint Panel
    gpu::draw_rect_flat(8, 6, 74, 38, 14, 16, 24);
    gpu::draw_rect_flat(9, 7, 72, 36, 22, 26, 36);

    // Laps boxes (top-left)
    for lap_idx in 0..5u16 {
        let x = 13 + lap_idx * 12;
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
    let cp_count = (timer.total_checkpoints as u16).min(7);
    for cp_idx in 0..cp_count {
        let x = 13 + cp_idx * 9;
        let is_passed = (cp_idx as u32) < cleared;
        let (cr, cg, cb) = if is_passed {
            (0, 220, 255)
        } else {
            (45, 48, 58)
        };
        gpu::draw_rect_flat(x as i16, 23, 7, 4, cr, cg, cb);
    }
    draw_text(13, 31, "LAPS", (150, 150, 165), 1);

    // Best lap record readout
    if timer.best_lap_ticks != u32::MAX {
        gpu::draw_rect_flat(8, 46, 74, 18, 14, 16, 24);
        gpu::draw_rect_flat(9, 47, 72, 16, 22, 26, 36);
        let mut best_buf = [b' '; 8];
        format_time(&mut best_buf, timer.best_lap_ticks);
        draw_text(12, 51, "BEST", (150, 150, 165), 1);
        blit(&best_buf, 38, 51, (200, 215, 240), 1);
    }

    // ------------------------------------------------ 2. Race Position & Lap Timer
    let has_best = timer.best_lap_ticks != u32::MAX;
    let center_h = if has_best { 48 } else { 38 };
    gpu::draw_rect_flat(108, 6, 104, center_h, 14, 16, 24);
    gpu::draw_rect_flat(109, 7, 102, center_h - 2, 22, 26, 36);

    let (rank_str, rank_col) = match rank {
        1 => ("1ST", (255, 215, 0)),
        2 => ("2ND", (220, 225, 235)),
        3 => ("3RD", (210, 130, 50)),
        4 => ("4TH", (180, 200, 220)),
        5 => ("5TH", (160, 180, 200)),
        _ => ("6TH", (140, 150, 170)),
    };
    draw_text(142, 9, rank_str, rank_col, 2);

    let mut time_buf = [b' '; 8];
    format_time(&mut time_buf, timer.current_lap_ticks);
    blit(&time_buf, 112, 25, (245, 245, 250), 2);

    if has_best {
        let delta = timer.delta_ticks();
        let mut delta_buf = [b' '; 6];
        format_delta(&mut delta_buf, delta);
        let delta_col = if delta < 0 {
            (60, 240, 90) // Ahead of pace: vivid green
        } else {
            (245, 60, 60) // Behind pace: bright red
        };
        blit(&delta_buf, 142, 40, delta_col, 1);
    }

    // ------------------------------------------------ 3. Gauges & Telemetry Cluster
    gpu::draw_rect_flat(216, 6, 96, 44, 14, 16, 24);
    gpu::draw_rect_flat(217, 7, 94, 42, 22, 26, 36);

    // Tachometer border and backing
    gpu::draw_rect_flat(220, 10, 88, 8, 14, 16, 24);
    let rpm_ratio = ((player.engine_rpm as i32 - 1000) * 86) / 7000;
    let fill_w = rpm_ratio.clamp(0, 86) as u16;
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
        gpu::draw_rect_flat(221, 11, fill_w, 6, bar.0, bar.1, bar.2);
    }

    // Nitro charge bar directly beneath the rev bar
    gpu::draw_rect_flat(220, 20, 88, 4, 14, 16, 24);
    let nitro_w = ((player.nitro_charge as i32 * 86) / NITRO_MAX_TICKS as i32).clamp(0, 86) as u16;
    if nitro_w > 0 {
        let (nr, ng, nb) = if player.nitro_charge > NITRO_MAX_TICKS / 3 {
            (255, 150, 40) // Full / active charge
        } else {
            (80, 110, 140) // Low charge
        };
        gpu::draw_rect_flat(221, 21, nitro_w, 2, nr, ng, nb);
    }

    // Digital Speedometer (approx. MPH based on forward speed)
    let speed_mph = ((player.speed.raw() * 145) / 14000).clamp(0, 199) as u16;
    let mut speed_buf = [b' '; 7];
    format_speed(&mut speed_buf, speed_mph);
    blit(&speed_buf, 220, 28, (240, 240, 250), 1);

    // Gear indicator (right of the speedo)
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
    draw_text(278, 30, "G", (150, 150, 165), 1);
    blit(&gear_buf, 288, 26, gear_col, 2);

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

    // The circuit outline is already in VRAM; one sprite packet replaces the
    // per-tile rectangle walk.
    minimap_texture.blit(map_x + 4, map_y + 4, BlendMode::Opaque);

    // Rival blips
    for rival in rivals {
        let rx = minimap_world_pos(rival.state.position.x, track.width);
        let ry = minimap_world_pos(rival.state.position.y, track.height);
        let (rc, gc, bc) = rival.profile.color;
        gpu::draw_rect_flat(map_x + 4 + rx - 1, map_y + 4 + ry - 1, 2, 2, rc, gc, bc);
    }

    // Player position blip (bright golden dot)
    let px = minimap_world_pos(player.position.x, track.width);
    let py = minimap_world_pos(player.position.y, track.height);
    gpu::draw_rect_flat(map_x + 4 + px - 1, map_y + 4 + py - 1, 3, 3, 255, 230, 0);
}
