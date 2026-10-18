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

use crate::gpu::texlayout::{
    minimap_colour, minimap_tile_rect, minimap_world_pos, MINIMAP_INNER, MINIMAP_SIZE,
};
use crate::gpu::texpipe::TextureSlot;
use crate::ui::font::{draw_char, draw_text};
use arduracer_core::math::{cos, sin};
use arduracer_core::{
    AiRacer, LapTimer, Medal, TrackDef, TrackTile, VehicleState, FP_SHIFT,
    LAUNCH_SWEET_SPOT_MAX_RPM, LAUNCH_SWEET_SPOT_MIN_RPM, NITRO_MAX_TICKS,
};
use psx_gpu as gpu;
use psx_gpu::material::BlendMode;

/// Bakes the circuit outline into `slot` and uploads it.
///
/// Call once per track load, never per frame: the upload is a DMA transfer and
/// the image is static for the whole race.
///
/// Multi-pass layered rasterisation guarantees road continuity:
/// 1. Background terrain (OffRoad) fills the inner minimap box.
/// 2. Barriers and non-drivable bounds are drawn.
/// 3. Drivable roads, curbs, and speed/hazard pads are stamped on top.
/// 4. Start/finish and checkpoint gates are stamped with highest priority.
pub fn bake_minimap(slot: &TextureSlot, track: &TrackDef) {
    slot.begin_compose();
    let bg = minimap_colour(TrackTile::OffRoad);
    slot.fill_rect(0, 0, MINIMAP_INNER, MINIMAP_INNER, bg);

    let tiles = track.tile_grid();
    if tiles.is_empty() {
        slot.upload();
        return;
    }

    let w = track.width;
    let h = track.height;

    // Pass 1: Barriers and boundary walls
    for ty in 0..h {
        let row_idx = ty as usize * w as usize;
        for tx in 0..w {
            if let Some(&tile) = tiles.get(row_idx + tx as usize) {
                if tile == TrackTile::Barrier {
                    let (r, g, b) = minimap_colour(tile);
                    let (px, py, pw, ph) = minimap_tile_rect(tx, ty, w, h);
                    slot.fill_rect(px, py, pw, ph, (r, g, b));
                }
            }
        }
    }

    // Pass 2: Drivable road surface, curbs, and hazards (drawn on top of barriers/terrain)
    for ty in 0..h {
        let row_idx = ty as usize * w as usize;
        for tx in 0..w {
            if let Some(&tile) = tiles.get(row_idx + tx as usize) {
                if tile.is_road()
                    || tile == TrackTile::Curb
                    || tile == TrackTile::BoostPad
                    || tile == TrackTile::SuperBoostPad
                    || tile == TrackTile::EngineOff
                    || tile == TrackTile::EngineReset
                    || tile == TrackTile::OilSlick
                {
                    let (r, g, b) = minimap_colour(tile);
                    let (px, py, pw, ph) = minimap_tile_rect(tx, ty, w, h);
                    slot.fill_rect(px, py, pw, ph, (r, g, b));
                }
            }
        }
    }

    // Pass 3: Start/finish and checkpoint gates (highest visual priority)
    for ty in 0..h {
        let row_idx = ty as usize * w as usize;
        for tx in 0..w {
            if let Some(&tile) = tiles.get(row_idx + tx as usize) {
                if tile == TrackTile::StartFinish || tile == TrackTile::Checkpoint {
                    let (r, g, b) = minimap_colour(tile);
                    let (px, py, pw, ph) = minimap_tile_rect(tx, ty, w, h);
                    slot.fill_rect(px, py, pw, ph, (r, g, b));
                }
            }
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
    let is_freeroam = track.checkpoint_count == 0;

    // High-speed radial speed lines during Nitro or Super Boost (NO1-036)
    if player.boost_ticks > 0 || player.super_boost_ticks > 0 || player.speed.raw() > 14_000 {
        render_speed_lines(
            player.speed.raw(),
            player.boost_ticks,
            player.super_boost_ticks,
            timer.current_lap_ticks,
        );
    }

    // ------------------------------------------------- 1. Lap / Checkpoint Panel
    if is_freeroam {
        gpu::draw_rect_flat(8, 6, 88, 38, 14, 16, 24);
        gpu::draw_rect_flat(9, 7, 86, 36, 22, 26, 36);
        draw_text(13, 11, track.name, (255, 225, 40), 1);
        draw_text(13, 23, "FREE ROAM", (140, 210, 255), 1);
    } else {
        gpu::draw_rect_flat(8, 6, 74, 38, 14, 16, 24);
        gpu::draw_rect_flat(9, 7, 72, 36, 22, 26, 36);

        // Laps boxes (top-left, scaled to target_laps up to 5)
        let lap_boxes = (timer.target_laps.clamp(1, 5)) as u16;
        for lap_idx in 0..lap_boxes {
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
        draw_text(
            13,
            31,
            if timer.target_laps == 1 {
                "SPRINT"
            } else {
                "LAPS"
            },
            (150, 150, 165),
            1,
        );

        // Best lap record readout + Next Target Medal pace indicator (NO1-017)
        let (target_medal, target_ticks) = track.par_times.next_medal_target(timer.best_lap_ticks);
        let (medal_label, medal_col) = match target_medal {
            Medal::DevPlatinum => ("PLAT", (80, 245, 255)),
            Medal::Gold => ("GOLD", (255, 215, 0)),
            Medal::Silver => ("SLVR", (200, 210, 225)),
            _ => ("BRNZ", (210, 130, 50)),
        };
        if timer.best_lap_ticks != u32::MAX {
            gpu::draw_rect_flat(8, 46, 84, 38, 14, 16, 24);
            gpu::draw_rect_flat(9, 47, 82, 36, 22, 26, 36);
            let mut best_buf = [b' '; 8];
            format_time(&mut best_buf, timer.best_lap_ticks);
            draw_text(12, 50, "BEST", (150, 150, 165), 1);
            blit(&best_buf, 40, 50, (200, 215, 240), 1);

            let mut targ_buf = [b' '; 8];
            format_time(&mut targ_buf, target_ticks);
            draw_text(12, 61, medal_label, medal_col, 1);
            blit(&targ_buf, 40, 61, medal_col, 1);

            if let Some(gap) = arduracer_core::author_ghost_time_gap_ticks(
                track,
                player.position,
                timer.current_lap_ticks,
            ) {
                let mut g_buf = [b' '; 6];
                format_delta(&mut g_buf, gap);
                let g_col = if gap <= 0 {
                    (60, 245, 110)
                } else {
                    (255, 150, 55)
                };
                draw_text(12, 72, "GHST", (150, 165, 190), 1);
                blit(&g_buf, 46, 72, g_col, 1);
            }
        } else if target_ticks > 0 {
            gpu::draw_rect_flat(8, 46, 84, 28, 14, 16, 24);
            gpu::draw_rect_flat(9, 47, 82, 26, 22, 26, 36);
            let mut targ_buf = [b' '; 8];
            format_time(&mut targ_buf, target_ticks);
            draw_text(12, 50, medal_label, medal_col, 1);
            blit(&targ_buf, 40, 50, medal_col, 1);

            if let Some(gap) = arduracer_core::author_ghost_time_gap_ticks(
                track,
                player.position,
                timer.current_lap_ticks,
            ) {
                let mut g_buf = [b' '; 6];
                format_delta(&mut g_buf, gap);
                let g_col = if gap <= 0 {
                    (60, 245, 110)
                } else {
                    (255, 150, 55)
                };
                draw_text(12, 61, "GHST", (150, 165, 190), 1);
                blit(&g_buf, 46, 61, g_col, 1);
            }
        }
    }

    // ------------------------------------------------ 2. Race Position & Lap Timer
    if is_freeroam {
        gpu::draw_rect_flat(108, 6, 104, 38, 14, 16, 24);
        gpu::draw_rect_flat(109, 7, 102, 36, 22, 26, 36);
        draw_text(138, 9, "CRUISING", (255, 215, 0), 1);
        let mut time_buf = [b' '; 8];
        format_time(&mut time_buf, timer.current_lap_ticks);
        blit(&time_buf, 112, 23, (245, 245, 250), 2);
    } else {
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

        // TrackMania Checkpoint Split Banner (NO1-010)
        if let Some((cp_idx, split_ticks, split_delta)) = timer.checkpoint_split_banner() {
            let banner_y = (8 + center_h as i16) + 2;
            gpu::draw_rect_flat(104, banner_y, 112, 22, 10, 14, 22);
            gpu::draw_rect_flat(105, banner_y + 1, 110, 20, 24, 30, 44);
            let cp_num = cp_idx.saturating_add(1);
            let cp_label = [
                b'C',
                b'P',
                if cp_num >= 10 {
                    b'0' + (cp_num / 10)
                } else {
                    b'0' + cp_num
                },
                if cp_num >= 10 {
                    b'0' + (cp_num % 10)
                } else {
                    b' '
                },
            ];
            blit(&cp_label, 109, (banner_y + 3) as u16, (0, 220, 255), 1);
            let mut split_buf = [b' '; 8];
            format_time(&mut split_buf, split_ticks);
            blit(&split_buf, 138, (banner_y + 3) as u16, (245, 245, 250), 1);

            if let Some(d) = split_delta {
                let mut d_buf = [b' '; 6];
                format_delta(&mut d_buf, d);
                let d_col = if d <= 0 { (50, 245, 95) } else { (255, 65, 65) };
                blit(&d_buf, 142, (banner_y + 12) as u16, d_col, 1);
            } else {
                draw_text(
                    136,
                    (banner_y + 12) as u16,
                    "SPLIT TIME",
                    (150, 165, 190),
                    1,
                );
            }
        }
    }

    // ------------------------------------------------ 3. Gauges & Telemetry Cluster
    gpu::draw_rect_flat(216, 6, 96, 44, 14, 16, 24);
    gpu::draw_rect_flat(217, 7, 94, 42, 22, 26, 36);

    // Tachometer border and backing + Launch Sweet-Spot markers (NO1-016)
    gpu::draw_rect_flat(220, 10, 88, 8, 14, 16, 24);
    let sweet_min_x = 221 + (((LAUNCH_SWEET_SPOT_MIN_RPM as i32 - 1000) * 86) / 7000).clamp(0, 86);
    let sweet_max_x = 221 + (((LAUNCH_SWEET_SPOT_MAX_RPM as i32 - 1000) * 86) / 7000).clamp(0, 86);
    gpu::draw_rect_flat(
        sweet_min_x as i16,
        10,
        (sweet_max_x - sweet_min_x).max(2) as u16,
        8,
        24,
        68,
        52,
    );
    let rpm_ratio = ((player.engine_rpm as i32 - 1000) * 86) / 7000;
    let fill_w = rpm_ratio.clamp(0, 86) as u16;
    let bar = if player.super_boost_ticks > 0 {
        (255, 60, 40) // Crimson-orange while on TrackMania Super Boost
    } else if player.boost_ticks > 0 {
        (0, 220, 255) // Neon cyan while boosting
    } else if player.engine_rpm > LAUNCH_SWEET_SPOT_MAX_RPM {
        (255, 40, 40) // Redline warning
    } else if player.engine_rpm >= LAUNCH_SWEET_SPOT_MIN_RPM {
        (60, 245, 110) // Launch sweet spot / peak power band
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
        let (nr, ng, nb) = if player.is_boost_disabled() {
            (180, 40, 30) // Red while depleted lockout is active
        } else if player.nitro_charge > NITRO_MAX_TICKS / 3 {
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
    let gear_char = if player.free_wheeling {
        b'N'
    } else if player.is_reversing {
        b'R'
    } else {
        b'0' + player.gear.clamp(1, 5)
    };
    let gear_col = if player.free_wheeling {
        (255, 80, 150)
    } else if player.is_reversing {
        (255, 160, 40)
    } else {
        (255, 225, 40)
    };
    let mut gear_buf = [b' '; 1];
    gear_buf[0] = gear_char;
    draw_text(278, 30, "G", (150, 150, 165), 1);
    blit(&gear_buf, 288, 26, gear_col, 2);

    // Status badge below gauges (Super Boost, Airtime, Free-Wheeling, Slipstream, Wheelspin)
    let status_badge: Option<(&str, (u8, u8, u8))> = if player.super_boost_ticks > 0 {
        Some(("SUPER BOOST!", (255, 70, 40)))
    } else if player.is_airborne() {
        Some(("AIRTIME!", (255, 225, 45)))
    } else if player.free_wheeling {
        Some(("ENGINE OFF", (255, 80, 150)))
    } else if player.is_drafting() {
        Some(("SLIPSTREAM", (80, 235, 255)))
    } else if player.wheelspin_ticks > 0 {
        Some(("WHEELSPIN!", (255, 150, 40)))
    } else if player.boost_ticks > 0 {
        Some(("TURBO BOOST", (0, 220, 255)))
    } else {
        None
    };
    if let Some((badge_text, badge_col)) = status_badge {
        gpu::draw_rect_flat(216, 52, 96, 14, 14, 16, 24);
        gpu::draw_rect_flat(217, 53, 94, 12, 22, 26, 36);
        draw_text(224, 55, badge_text, badge_col, 1);
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

    // The circuit outline is already in VRAM; one sprite packet replaces the
    // per-tile rectangle walk.
    // Only the composed window, not the whole slot: the slot is 64px square but
    // the map is 48, and blitting the slot drew the untouched border as an
    // opaque black rectangle past the panel and off the bottom of the screen.
    minimap_texture.blit_region(
        map_x + 4,
        map_y + 4,
        MINIMAP_INNER as u16,
        MINIMAP_INNER as u16,
        BlendMode::Opaque,
    );

    // Rival blips: smooth continuous world positioning (NO1-035)
    if !is_freeroam {
        for rival in rivals {
            let r_mx = map_x + 4 + minimap_world_pos(rival.state.position.x, track.width);
            let r_my = map_y + 4 + minimap_world_pos(rival.state.position.y, track.height);
            let (rc, gc, bc) = rival.profile.color;
            gpu::draw_rect_flat(r_mx - 1, r_my - 1, 3, 3, rc, gc, bc);
        }
    }

    // Player vehicle blip: oriented directional chevron arrow (NO1-035)
    let p_mx = map_x + 4 + minimap_world_pos(player.position.x, track.width);
    let p_my = map_y + 4 + minimap_world_pos(player.position.y, track.height);

    let h = player.heading;
    let tip_dx = (sin(h).raw() * 4) >> FP_SHIFT;
    let tip_dy = -(cos(h).raw() * 4) >> FP_SHIFT;
    let wing_l_dx = (sin(h.wrapping_add(1500)).raw() * 3) >> FP_SHIFT;
    let wing_l_dy = -(cos(h.wrapping_add(1500)).raw() * 3) >> FP_SHIFT;
    let wing_r_dx = (sin(h.wrapping_sub(1500)).raw() * 3) >> FP_SHIFT;
    let wing_r_dy = -(cos(h.wrapping_sub(1500)).raw() * 3) >> FP_SHIFT;

    let tip = (p_mx + tip_dx as i16, p_my + tip_dy as i16);
    let wing_l = (p_mx + wing_l_dx as i16, p_my + wing_l_dy as i16);
    let wing_r = (p_mx + wing_r_dx as i16, p_my + wing_r_dy as i16);

    gpu::draw_tri_flat([tip, wing_l, wing_r], 255, 230, 30);
    gpu::draw_line_mono(wing_l.0, wing_l.1, wing_r.0, wing_r.1, 180, 140, 20);
}

/// Renders a standalone vector minimap for UI screens (track select, championship, stage table).
///
/// Draws the track's route outline, start/finish line, and checkpoint gates inside the
/// specified `(x, y, w, h)` bounding box. Runs entirely via hardware GP0 lines and rects,
/// requiring no resident level grid or CD-ROM streaming.
pub fn draw_ui_minimap(x: i16, y: i16, w: u16, h: u16, track: &TrackDef) {
    if w < 10 || h < 10 {
        return;
    }
    // Background and border
    gpu::draw_rect_flat(x, y, w, h, 14, 16, 24);
    gpu::draw_rect_flat(x + 1, y + 1, w - 2, h - 2, 28, 34, 48);
    gpu::draw_rect_flat(x + 2, y + 2, w - 4, h - 4, 10, 12, 18);

    let inner_x = x + 4;
    let inner_y = y + 4;
    let inner_w = (w - 8) as i32;
    let inner_h = (h - 8) as i32;

    let total_w = track.width as i32 * arduracer_core::TILE_SIZE;
    let total_h = track.height as i32 * arduracer_core::TILE_SIZE;
    if total_w <= 0 || total_h <= 0 {
        return;
    }

    // 1. Draw route line
    if track.route.len() >= 2 {
        for i in 0..track.route.len() {
            let next_i = if i + 1 < track.route.len() { i + 1 } else { 0 };
            let p0 = track.route[i];
            let p1 = track.route[next_i];

            let x0 = inner_x + ((p0.x.to_int().clamp(0, total_w) * inner_w) / total_w) as i16;
            let y0 = inner_y + ((p0.y.to_int().clamp(0, total_h) * inner_h) / total_h) as i16;
            let x1 = inner_x + ((p1.x.to_int().clamp(0, total_w) * inner_w) / total_w) as i16;
            let y1 = inner_y + ((p1.y.to_int().clamp(0, total_h) * inner_h) / total_h) as i16;

            // Base road line (light silver-blue)
            gpu::draw_line_mono(x0, y0, x1, y1, 200, 210, 230);
            // 1-pixel offset for slight thickness
            gpu::draw_line_mono(x0 + 1, y0, x1 + 1, y1, 140, 150, 170);
        }
    } else {
        // Fallback: connect checkpoints if route is absent
        let cps = track.checkpoint_slice();
        if !cps.is_empty() {
            for i in 0..cps.len() {
                let next_i = if i + 1 < cps.len() { i + 1 } else { 0 };
                let c0 = cps[i];
                let c1 = cps[next_i];
                let x0 = inner_x + ((c0.x as i32 * inner_w) / track.width as i32) as i16;
                let y0 = inner_y + ((c0.y as i32 * inner_h) / track.height as i32) as i16;
                let x1 = inner_x + ((c1.x as i32 * inner_w) / track.width as i32) as i16;
                let y1 = inner_y + ((c1.y as i32 * inner_h) / track.height as i32) as i16;
                gpu::draw_line_mono(x0, y0, x1, y1, 180, 190, 210);
            }
        }
    }

    // 2. Checkpoints (cyan dots)
    let cps = track.checkpoint_slice();
    for cp in cps {
        let cx = inner_x + ((cp.x as i32 * inner_w) / track.width as i32) as i16;
        let cy = inner_y + ((cp.y as i32 * inner_h) / track.height as i32) as i16;
        gpu::draw_rect_flat(cx, cy, 2, 2, 0, 210, 255);
    }

    // 3. Start / Finish line (bright gold marker)
    let sx = inner_x + ((track.start_gate.x as i32 * inner_w) / track.width as i32) as i16;
    let sy = inner_y + ((track.start_gate.y as i32 * inner_h) / track.height as i32) as i16;
    gpu::draw_rect_flat(sx - 1, sy - 1, 3, 3, 255, 230, 40);
}

/// Renders the Broadcast TV Action Replay Theater letterbox HUD overlay (`NO1-027`).
pub fn render_replay_hud(track_name: &str, replay_tick: u32, total_ticks: u32, camera_mode: u8) {
    // Top cinema letterbox bar
    gpu::draw_rect_flat(0, 0, 320, 24, 10, 12, 18);
    gpu::draw_rect_flat(0, 24, 320, 2, 220, 40, 60);

    // Blinking REC/PLAY dot
    if ((replay_tick >> 4) & 1) == 0 {
        gpu::draw_rect_flat(10, 8, 8, 8, 255, 50, 50);
    }
    draw_text(24, 8, "ACTION REPLAY", (255, 220, 40), 1);
    draw_text(120, 8, track_name, (200, 215, 235), 1);

    let mut cur_buf = [b' '; 8];
    format_time(&mut cur_buf, replay_tick);
    blit(&cur_buf, 260, 8, (0, 225, 255), 1);

    // Bottom cinema letterbox bar
    gpu::draw_rect_flat(0, 214, 320, 2, 220, 40, 60);
    gpu::draw_rect_flat(0, 216, 320, 24, 10, 12, 18);

    // Timeline progress bar
    let bar_w = if total_ticks > 0 {
        ((replay_tick.min(total_ticks) as u64 * 300) / (total_ticks as u64)) as u16
    } else {
        0
    };
    gpu::draw_rect_flat(10, 219, 300, 3, 30, 36, 50);
    if bar_w > 0 {
        gpu::draw_rect_flat(10, 219, bar_w, 3, 0, 220, 255);
    }

    let cam_label = match camera_mode {
        0 => "CAM: BROADCAST TV",
        1 => "CAM: MODE 7 CHASE",
        _ => "CAM: HELICOPTER",
    };
    draw_text(10, 226, cam_label, (80, 235, 255), 1);
    draw_text(132, 226, "TRIANGLE: CAM", (255, 220, 40), 1);
    draw_text(224, 226, "CROSS: RESULTS", (220, 225, 235), 1);
}

/// Renders high-speed radial motion streaks during Nitro, Super Boost, or extreme velocity (`NO1-036`).
pub fn render_speed_lines(speed_raw: i32, nitro_ticks: u16, super_boost_ticks: u16, ticks: u32) {
    let intensity = if super_boost_ticks > 0 {
        3
    } else if nitro_ticks > 0 {
        2
    } else if speed_raw > 14_000 {
        1
    } else {
        0
    };
    if intensity == 0 {
        return;
    }

    let col = match intensity {
        3 => (255, 60, 40),   // Reactor Super Boost: fiery crimson
        2 => (50, 220, 255),  // Nitro Boost: cyan/electric
        _ => (160, 180, 210), // High Speed: white/blue streaks
    };

    let cx = 160i16;
    let cy = 130i16;

    // 12 pseudo-random radial lines shooting to screen edges
    const ANGLES_Q8: [i32; 12] = [15, 42, 75, 110, 145, 178, 205, 238, 270, 305, 335, 355];
    for (i, &ang) in ANGLES_Q8.iter().enumerate() {
        let t_seed = (ticks.wrapping_mul(7) + (i as u32 * 53)) & 31;
        let r0 = 55 + t_seed as i32;
        let r1 = r0 + 40 + (intensity * 25);

        let h_bam = ((ang + (ticks as i32 * 3)) & 0x0FFF) as u16;
        let s = sin(h_bam).raw();
        let c = cos(h_bam).raw();

        let x0 = cx + ((s * r0) >> FP_SHIFT) as i16;
        let y0 = cy - ((c * r0) >> FP_SHIFT) as i16;
        let x1 = cx + ((s * r1) >> FP_SHIFT) as i16;
        let y1 = cy - ((c * r1) >> FP_SHIFT) as i16;

        if (0..320).contains(&x1) && (0..240).contains(&y1) {
            gpu::draw_line_mono_blended((x0, y0), (x1, y1), (col.0, col.1, col.2), BlendMode::Add);
        }
    }
}

/// Renders the fullscreen "LOADING CIRCUIT..." overlay with track metadata and progress bar (`NO1-036`).
pub fn render_loading_screen(track_name: &str, author: &str, progress_pct: u8) {
    // Backdrop
    gpu::draw_rect_flat(0, 0, 320, 240, 12, 14, 22);

    // Header banner
    gpu::draw_rect_flat(0, 70, 320, 4, 0, 210, 255);
    gpu::draw_rect_flat(0, 74, 320, 48, 20, 24, 38);
    gpu::draw_rect_flat(0, 122, 320, 2, 0, 150, 200);

    draw_text(
        (160i32 - ((track_name.len() as i32 * 6 * 2) / 2)).max(8) as u16,
        82,
        track_name,
        (255, 255, 255),
        2,
    );
    if !author.is_empty() {
        draw_text(
            (160i32 - ((author.len() as i32 * 6) / 2)).max(8) as u16,
            106,
            author,
            (160, 215, 255),
            1,
        );
    }

    // Subtitle
    draw_text(
        160 - (18 * 6 / 2),
        140,
        "LOADING CIRCUIT...",
        (0, 210, 255),
        1,
    );

    // Progress bar frame
    let bar_x = 60i16;
    let bar_y = 160i16;
    let bar_w = 200u16;
    let bar_h = 10u16;
    gpu::draw_rect_flat(bar_x - 1, bar_y - 1, bar_w + 2, bar_h + 2, 40, 48, 68);
    gpu::draw_rect_flat(bar_x, bar_y, bar_w, bar_h, 16, 20, 30);

    let fill_w = ((bar_w as u32 * progress_pct.min(100) as u32) / 100) as u16;
    if fill_w > 0 {
        gpu::draw_rect_flat(
            bar_x + 1,
            bar_y + 1,
            fill_w.saturating_sub(2),
            bar_h - 2,
            0,
            220,
            255,
        );
    }
}

/// Renders the on-screen music track title toast notification banner (`NO1-041`).
pub fn render_music_toast(title: &str, toast_ticks: u16) {
    if toast_ticks == 0 {
        return;
    }
    let w = ((title.len() as u16 * 6) + 70).min(300);
    let x = (160u16).saturating_sub(w / 2);
    let y = 6u16;

    // Translucent card with cyan bottom accent
    gpu::draw_rect_flat(x as i16 - 1, y as i16 - 1, w + 2, 18, 12, 16, 24);
    gpu::draw_rect_flat(x as i16, y as i16, w, 16, 20, 26, 38);
    gpu::draw_rect_flat(x as i16, y as i16 + 15, w, 1, 0, 220, 255);

    draw_text(x + 6, y + 4, "MUSIC:", (255, 220, 40), 1);
    draw_text(x + 48, y + 4, title, (220, 235, 255), 1);
}

/// Renders a streamlined top-bar HUD overlay for 2-Player Split-Screen mode (`NO1-045`).
pub fn render_split_hud(
    player: &VehicleState,
    timer: &LapTimer<16>,
    player_num: u8,
    badge_color: (u8, u8, u8),
) {
    // Top bar card
    gpu::draw_rect_flat(6, 4, 308, 22, 14, 16, 24);
    gpu::draw_rect_flat(7, 5, 306, 20, 22, 26, 36);

    // Player Badge (e.g. "1P" or "2P")
    gpu::draw_rect_flat(9, 7, 24, 16, badge_color.0, badge_color.1, badge_color.2);
    let p_str = if player_num == 1 { "1P" } else { "2P" };
    draw_text(13, 11, p_str, (255, 255, 255), 1);

    // Lap counter: "LAP X/Y"
    draw_text(38, 11, "LAP", (150, 165, 190), 1);
    let cur_lap_char = b'0' + timer.current_lap.min(9);
    let tot_lap_char = b'0' + timer.target_laps.min(9);
    let lap_buf = [cur_lap_char, b'/', tot_lap_char];
    blit(&lap_buf, 62, 11, (255, 220, 40), 1);

    // Current Lap Time: "MM:SS.CC"
    let mut time_buf = [b' '; 8];
    format_time(&mut time_buf, timer.current_lap_ticks);
    blit(&time_buf, 92, 11, (245, 245, 250), 1);

    // Speedometer (MPH)
    let speed_mph = ((player.speed.raw() * 145) / 14000).clamp(0, 199) as u16;
    let mut speed_buf = [b' '; 7];
    format_speed(&mut speed_buf, speed_mph);
    blit(&speed_buf, 154, 11, (240, 240, 250), 1);

    // Mini Tachometer & Nitro Bar
    gpu::draw_rect_flat(218, 8, 90, 6, 14, 16, 24);
    let rpm_ratio = ((player.engine_rpm as i32 - 1000) * 88) / 7000;
    let fill_w = rpm_ratio.clamp(0, 88) as u16;
    let bar_col = if player.boost_ticks > 0 {
        (0, 220, 255)
    } else if player.engine_rpm > 5500 {
        (255, 40, 40)
    } else {
        (40, 220, 70)
    };
    if fill_w > 0 {
        gpu::draw_rect_flat(219, 9, fill_w, 4, bar_col.0, bar_col.1, bar_col.2);
    }
    gpu::draw_rect_flat(218, 16, 90, 4, 14, 16, 24);
    let nitro_w = ((player.nitro_charge as i32 * 88) / NITRO_MAX_TICKS as i32).clamp(0, 88) as u16;
    if nitro_w > 0 {
        let (nr, ng, nb) = if player.is_boost_disabled() {
            (180, 40, 30)
        } else {
            (255, 150, 40)
        };
        gpu::draw_rect_flat(219, 17, nitro_w, 2, nr, ng, nb);
    }
}

