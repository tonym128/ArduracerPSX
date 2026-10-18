//! Championship Victory Celebration Screen (`NO1-050`).
//!
//! Shown when completing the final stage of a Grand Prix or Mania Cup.
//! Renders:
//! - 3D GTE Rotating Championship Trophy with metallic lighting and backface culling
//! - 3-tier metallic Olympic podium steps (1st Gold, 2nd Silver, 3rd Bronze)
//! - Winning car showcase atop the 1st place gold podium with livery & aerodynamic details
//! - Multi-color particle fireworks celebration with gravity-curved sparkling trails
//! - Championship telemetry and sorted points leaderboard panel

use crate::ui::font::draw_text;
use arduracer_core::championship::ChampionshipSession;
use arduracer_core::math::{cos, sin};
use psx_gpu as gpu;
use psx_gte::math::{Mat3I16, Vec3I16, Vec3I32};
use psx_gte::scene;
use psx_pad::{button, PadState};

/// Precomputed 8-direction unit vectors for octagonal trophy rings in Q12 (4096 = 1.0).
const OCT_DIR: [(i16, i16); 8] = [
    (4096, 0),
    (2896, 2896),
    (0, 4096),
    (-2896, 2896),
    (-4096, 0),
    (-2896, -2896),
    (0, -4096),
    (2896, -2896),
];

/// Precomputed 8-direction radial spark vectors for fireworks.
const SPARK_DIR: [(i8, i8); 8] = [
    (4, 0),
    (3, 3),
    (0, 4),
    (-3, 3),
    (-4, 0),
    (-3, -3),
    (0, -4),
    (3, -3),
];

pub struct VictoryScreen {
    pub cup_name: &'static str,
    pub player_rank: u8,
    pub anim_timer: u32,
    pub prev_buttons: u16,
}

impl VictoryScreen {
    pub fn new(session: &ChampionshipSession) -> Self {
        VictoryScreen {
            cup_name: session.cup_name(),
            player_rank: session.player_leaderboard_rank(),
            anim_timer: 0,
            prev_buttons: 0,
        }
    }

    pub fn update(&mut self, pad: &PadState) -> bool {
        self.anim_timer = self.anim_timer.wrapping_add(1);
        let b = pad.buttons;
        let prev = psx_pad::ButtonState::from_bits(self.prev_buttons);
        self.prev_buttons = b.bits();

        let can_input = self.anim_timer > 60;
        can_input
            && (b.pressed_since(prev, button::CROSS)
                || b.pressed_since(prev, button::START)
                || b.pressed_since(prev, button::CIRCLE))
    }

    #[inline(never)]
    pub fn render(&self, session: &ChampionshipSession) {
        // 1. Stadium Backdrop
        gpu::draw_rect_flat(6, 6, 308, 228, 10, 12, 18);
        gpu::draw_rect_flat(8, 8, 304, 224, 18, 22, 34);

        // 2. Header Title Banner & Sub-Banner
        let pulse = (self.anim_timer >> 3).is_multiple_of(2);
        let title_col = if pulse { (255, 240, 70) } else { (255, 215, 0) };
        draw_text(66, 12, "CHAMPIONSHIP VICTORY!", title_col, 1);
        gpu::draw_rect_flat(30, 23, 260, 2, 220, 40, 60);

        let is_champion = self.player_rank == 1;
        let (banner_txt, banner_col) = match self.player_rank {
            1 => ("1ST PLACE - CUP CHAMPION!", (255, 215, 0)),
            2 => ("2ND PLACE - SILVER PODIUM!", (210, 215, 230)),
            3 => ("3RD PLACE - BRONZE PODIUM!", (215, 135, 60)),
            _ => ("CONGRATULATIONS ON COMPLETION!", (180, 200, 225)),
        };

        gpu::draw_rect_flat(30, 27, 260, 12, 36, 42, 64);
        draw_text(52, 29, banner_txt, banner_col, 1);

        // 3. Multi-Color Particle Fireworks Celebration across the Sky
        for b in 0..4 {
            let bx = match b {
                0 => 55,
                1 => 255,
                2 => 145,
                _ => 205,
            };
            let by = match b {
                0 => 46,
                1 => 42,
                2 => 38,
                _ => 52,
            };
            let phase = (b as u32) * 32;
            let age = (self.anim_timer.wrapping_add(128).wrapping_sub(phase)) & 127;
            if age < 40 {
                let t = age as i16;
                let col = match b {
                    0 => (255, 220, 40),
                    1 => (255, 60, 90),
                    2 => (0, 220, 255),
                    _ => (60, 240, 100),
                };
                for &(dx_raw, dy_raw) in &SPARK_DIR {
                    let dx = dx_raw as i16;
                    let dy = dy_raw as i16;
                    let sx = bx + (dx * t) / 4;
                    let sy = by + (dy * t) / 4 + (t * t) / 36;
                    if (10..310).contains(&sx) && (10..220).contains(&sy) {
                        gpu::draw_rect_flat(sx, sy, 2, 2, col.0, col.1, col.2);
                    }
                }
            }
        }

        // 4. 3D GTE Rotating Championship Cup Trophy
        let (base_r, base_g, base_b) = match self.player_rank {
            1 => (255u16, 215u16, 0u16),
            2 => (210u16, 215u16, 230u16),
            3 => (215u16, 130u16, 50u16),
            _ => (160u16, 175u16, 195u16),
        };

        let yaw = self.anim_timer.wrapping_mul(14) as u16;
        let cos_y = cos(yaw).raw() as i32;
        let sin_y = sin(yaw).raw() as i32;
        let sin_p = 990i32;
        let cos_p = 3974i32;

        let rot = Mat3I16 {
            m: [
                [cos_y as i16, 0, sin_y as i16],
                [
                    ((sin_p * sin_y) >> 12) as i16,
                    cos_p as i16,
                    ((-sin_p * cos_y) >> 12) as i16,
                ],
                [
                    ((-cos_p * sin_y) >> 12) as i16,
                    sin_p as i16,
                    ((cos_p * cos_y) >> 12) as i16,
                ],
            ],
        };

        scene::load_rotation(&rot);
        scene::load_translation(Vec3I32::new(0, 0, 270));
        scene::set_screen_offset(110 << 16, 72 << 16);
        scene::set_projection_plane(140);

        let mut p_top = [(0i16, 0i16); 8];
        let mut p_bot = [(0i16, 0i16); 8];
        for k in 0..8 {
            let xt = ((22 * OCT_DIR[k].0 as i32) >> 12) as i16;
            let zt = ((22 * OCT_DIR[k].1 as i32) >> 12) as i16;
            let pt = scene::project_vertex_scheduled(Vec3I16::new(xt, -20, zt));
            p_top[k] = (pt.sx, pt.sy);

            let xb = ((10 * OCT_DIR[k].0 as i32) >> 12) as i16;
            let zb = ((10 * OCT_DIR[k].1 as i32) >> 12) as i16;
            let pb = scene::project_vertex_scheduled(Vec3I16::new(xb, 10, zb));
            p_bot[k] = (pb.sx, pb.sy);
        }

        for k in 0..8 {
            let n = (k + 1) & 7;
            let cp = (p_top[n].0 as i32 - p_top[k].0 as i32)
                * (p_bot[k].1 as i32 - p_top[k].1 as i32)
                - (p_top[n].1 as i32 - p_top[k].1 as i32) * (p_bot[k].0 as i32 - p_top[k].0 as i32);
            if cp > 0 {
                let shade = if (k & 1) == 0 { 240 } else { 180 };
                let r = ((base_r * shade) >> 8) as u8;
                let g = ((base_g * shade) >> 8) as u8;
                let b = ((base_b * shade) >> 8) as u8;
                gpu::draw_quad_flat([p_top[k], p_top[n], p_bot[k], p_bot[n]], r, g, b);
            }
        }

        let hr = ((base_r * 220) >> 8) as u8;
        let hg = ((base_g * 220) >> 8) as u8;
        let hb = ((base_b * 220) >> 8) as u8;
        for s in [-1i16, 1i16] {
            let h0 = scene::project_vertex_scheduled(Vec3I16::new(s * 18, -16, 0));
            let h1 = scene::project_vertex_scheduled(Vec3I16::new(s * 28, -6, 0));
            let h2 = scene::project_vertex_scheduled(Vec3I16::new(s * 10, 4, 0));
            gpu::draw_line_mono(h0.sx, h0.sy, h1.sx, h1.sy, hr, hg, hb);
            gpu::draw_line_mono(h1.sx, h1.sy, h2.sx, h2.sy, hr, hg, hb);
        }
        gpu::draw_rect_flat(100, 84, 20, 6, 80, 85, 95);

        // 5. 3-Tier Metallic Olympic Podium Steps
        let board = session.sorted_leaderboard();
        gpu::draw_rect_flat(12, 186, 194, 6, 12, 14, 20);

        // Silver (Left)
        gpu::draw_rect_flat(16, 152, 56, 5, 235, 240, 252);
        gpu::draw_rect_flat(16, 157, 56, 29, 180, 190, 205);
        draw_text(40, 166, "2", (255, 255, 255), 1);
        if board.len() > 1 {
            draw_text(24, 176, board[1].name, (220, 230, 245), 1);
        }

        // Bronze (Right)
        gpu::draw_rect_flat(148, 162, 56, 5, 235, 155, 80);
        gpu::draw_rect_flat(148, 167, 56, 19, 180, 105, 40);
        draw_text(172, 170, "3", (255, 220, 170), 1);
        if board.len() > 2 {
            draw_text(156, 176, board[2].name, (240, 175, 110), 1);
        }

        // Gold (Center)
        gpu::draw_rect_flat(74, 136, 72, 6, 255, 225, 70);
        gpu::draw_rect_flat(74, 142, 72, 44, 220, 175, 15);
        draw_text(106, 156, "1", (255, 255, 255), 1);
        if !board.is_empty() {
            draw_text(82, 174, board[0].name, (255, 245, 140), 1);
        }

        // 6. Winning Car Showcase atop Gold Podium
        let car_col = if !board.is_empty() {
            board[0].color
        } else {
            (220, 30, 45)
        };
        let cx = 110i16;
        let cy = 127i16;
        gpu::draw_rect_flat(cx - 16, cy + 5, 32, 3, 140, 105, 10);
        gpu::draw_rect_flat(cx - 15, cy - 3, 4, 4, 20, 20, 25);
        gpu::draw_rect_flat(cx + 11, cy - 3, 4, 4, 20, 20, 25);
        gpu::draw_rect_flat(cx - 15, cy + 2, 4, 4, 20, 20, 25);
        gpu::draw_rect_flat(cx + 11, cy + 2, 4, 4, 20, 20, 25);
        gpu::draw_rect_flat(cx - 13, cy - 6, 26, 10, car_col.0, car_col.1, car_col.2);
        gpu::draw_rect_flat(cx - 10, cy + 4, 20, 4, car_col.0, car_col.1, car_col.2);
        gpu::draw_rect_flat(cx - 1, cy - 6, 2, 14, 255, 255, 255);
        gpu::draw_rect_flat(cx - 6, cy - 3, 12, 6, 30, 50, 80);
        gpu::draw_rect_flat(cx - 4, cy - 2, 8, 4, 50, 110, 160);
        gpu::draw_rect_flat(cx - 15, cy - 9, 30, 2, car_col.0, car_col.1, car_col.2);

        // 7. Championship Telemetry & Sorted Leaderboard Panel (Right)
        let lx = 208i16;
        let ly = 42i16;
        gpu::draw_rect_flat(lx, ly, 104, 150, 20, 25, 38);
        draw_text(lx as u16 + 8, ly as u16 + 6, "STANDINGS", (0, 220, 255), 1);
        draw_text(
            lx as u16 + 8,
            ly as u16 + 18,
            self.cup_name,
            (255, 215, 0),
            1,
        );

        for (i, comp) in board.iter().enumerate() {
            let row_y = ly as u16 + 32 + (i as u16) * 14;
            if comp.is_player {
                gpu::draw_rect_flat(lx + 2, (row_y - 2) as i16, 100, 12, 160, 30, 45);
            }
            let r_col = if comp.is_player {
                (255, 255, 255)
            } else {
                comp.color
            };
            crate::ui::font::draw_char(lx as u16 + 6, row_y, b'1' + i as u8, (255, 220, 0), 1);
            draw_text(lx as u16 + 12, row_y, ".", (255, 220, 0), 1);
            let max_l = comp.name.len().min(5);
            draw_text(lx as u16 + 18, row_y, &comp.name[..max_l], r_col, 1);

            let pts = comp.total_points;
            crate::ui::font::draw_char(
                lx as u16 + 56,
                row_y,
                b'0' + ((pts / 10) % 10) as u8,
                (255, 225, 40),
                1,
            );
            crate::ui::font::draw_char(
                lx as u16 + 62,
                row_y,
                b'0' + (pts % 10) as u8,
                (255, 225, 40),
                1,
            );
            draw_text(lx as u16 + 70, row_y, "PTS", (160, 175, 195), 1);
        }

        draw_text(
            lx as u16 + 8,
            ly as u16 + 122,
            session.difficulty.name(),
            (170, 185, 205),
            1,
        );
        let badge = if is_champion {
            "GOLD CUP"
        } else if self.player_rank == 2 {
            "SILVER CUP"
        } else if self.player_rank == 3 {
            "BRONZE CUP"
        } else {
            "FINISHER"
        };
        draw_text(lx as u16 + 8, ly as u16 + 135, badge, banner_col, 1);

        // 8. Return to Menu Prompt
        let prompt_col = if pulse {
            (255, 255, 255)
        } else {
            (180, 205, 230)
        };
        gpu::draw_rect_flat(36, 198, 248, 24, 200, 30, 50);
        gpu::draw_rect_flat(38, 200, 244, 20, 28, 32, 46);
        draw_text(62, 206, "PRESS CROSS TO CONTINUE", prompt_col, 1);
    }
}
