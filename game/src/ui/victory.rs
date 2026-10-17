//! Championship Victory Celebration Screen.
//!
//! Shown when completing the final stage of a Grand Prix Cup.
//! Renders victory fanfare banner, trophy presentation, final cup leaderboard,
//! and celebration particle effects.

use crate::ui::font::draw_text;
use arduracer_core::championship::ChampionshipSession;
use psx_gpu as gpu;
use psx_pad::{button, PadState};

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

        // 60-frame (~1.0s) initial lockout so player can enjoy victory fanfare
        let can_input = self.anim_timer > 60;
        can_input
            && (b.pressed_since(prev, button::CROSS)
                || b.pressed_since(prev, button::START)
                || b.pressed_since(prev, button::CIRCLE))
    }

    pub fn render(&self, session: &ChampionshipSession) {
        // Outer box and drop shadow
        gpu::draw_rect_flat(14, 10, 292, 220, 12, 10, 20);
        gpu::draw_rect_flat(16, 12, 288, 216, 25, 20, 45);
        gpu::draw_rect_flat(18, 14, 284, 212, 35, 28, 65);

        // Header Title Banner
        let pulse = (self.anim_timer >> 3) & 1 == 0;
        let title_col = if pulse { (255, 235, 60) } else { (255, 215, 0) };
        draw_text(60, 20, "CHAMPIONSHIP VICTORY!", title_col, 1);
        gpu::draw_rect_flat(30, 32, 260, 2, 220, 40, 60);

        // Cup and Player standing banner
        let is_champion = self.player_rank == 1;
        let (banner_txt, banner_col) = if is_champion {
            ("1ST PLACE - CUP CHAMPION!", (255, 215, 0))
        } else if self.player_rank == 2 {
            ("2ND PLACE - SILVER PODIUM!", (210, 210, 225))
        } else if self.player_rank == 3 {
            ("3RD PLACE - BRONZE PODIUM!", (205, 127, 50))
        } else {
            ("CONGRATULATIONS ON COMPLETION!", (180, 200, 220))
        };

        gpu::draw_rect_flat(30, 38, 260, 18, 50, 40, 80);
        draw_text(48, 43, banner_txt, banner_col, 1);

        // Trophy graphic (composed with flat rectangles)
        let tx = 38i16;
        let ty = 66i16;
        let trophy_col = if is_champion {
            (255, 215, 0) // Gold
        } else if self.player_rank == 2 {
            (200, 205, 220) // Silver
        } else if self.player_rank == 3 {
            (205, 127, 50) // Bronze
        } else {
            (160, 170, 185)
        };
        // Cup bowl
        gpu::draw_rect_flat(tx + 6, ty, 20, 14, trophy_col.0, trophy_col.1, trophy_col.2);
        gpu::draw_rect_flat(
            tx + 10,
            ty + 14,
            12,
            6,
            trophy_col.0,
            trophy_col.1,
            trophy_col.2,
        );
        // Handles
        gpu::draw_rect_flat(
            tx + 2,
            ty + 2,
            4,
            8,
            trophy_col.0,
            trophy_col.1,
            trophy_col.2,
        );
        gpu::draw_rect_flat(
            tx + 26,
            ty + 2,
            4,
            8,
            trophy_col.0,
            trophy_col.1,
            trophy_col.2,
        );
        // Stem and base
        gpu::draw_rect_flat(
            tx + 14,
            ty + 20,
            4,
            10,
            trophy_col.0,
            trophy_col.1,
            trophy_col.2,
        );
        gpu::draw_rect_flat(tx + 8, ty + 30, 16, 6, 80, 85, 95);

        // Cup Name label
        draw_text(
            (tx - 4) as u16,
            (ty + 42) as u16,
            self.cup_name,
            (255, 255, 255),
            1,
        );

        // Confetti / Celebration particle burst effect
        for p in 0..16 {
            let offset_x = ((self.anim_timer.wrapping_mul(7 + p) + p * 37) % 270) as i16 + 25;
            let offset_y = ((self.anim_timer.wrapping_mul(5 + p * 3) + p * 19) % 180) as i16 + 20;
            let col = match p % 4 {
                0 => (255, 60, 60),
                1 => (255, 220, 0),
                2 => (0, 220, 255),
                _ => (120, 255, 120),
            };
            gpu::draw_rect_flat(offset_x, offset_y, 3, 3, col.0, col.1, col.2);
        }

        // Leaderboard table (Right side)
        let lx = 105i16;
        let ly = 62i16;
        gpu::draw_rect_flat(lx, ly, 185, 114, 25, 28, 42);
        gpu::draw_rect_flat(lx + 2, ly + 2, 181, 110, 18, 20, 30);
        draw_text(
            lx as u16 + 10,
            ly as u16 + 6,
            "FINAL CHAMPIONSHIP STANDINGS",
            (0, 220, 255),
            1,
        );

        let board = session.sorted_leaderboard();
        for (i, comp) in board.iter().enumerate() {
            let row_y = ly as u16 + 22 + (i as u16) * 14;
            let is_p = comp.is_player;

            if is_p {
                gpu::draw_rect_flat(lx + 4, (row_y - 2) as i16, 177, 12, 180, 25, 45);
            }

            let rank_char = b'1' + (i as u8);
            crate::ui::font::draw_char(lx as u16 + 8, row_y, rank_char, (255, 255, 255), 1);
            draw_text(lx as u16 + 18, row_y, ".", (255, 255, 255), 1);

            let name_col = if is_p { (255, 255, 255) } else { comp.color };
            draw_text(lx as u16 + 28, row_y, comp.name, name_col, 1);

            // Points
            let pts = comp.total_points;
            let pts_ten = b'0' + ((pts / 10) % 10) as u8;
            let pts_one = b'0' + (pts % 10) as u8;
            crate::ui::font::draw_char(lx as u16 + 130, row_y, pts_ten, (255, 220, 0), 1);
            crate::ui::font::draw_char(lx as u16 + 136, row_y, pts_one, (255, 220, 0), 1);
            draw_text(lx as u16 + 146, row_y, "PTS", (180, 190, 205), 1);
        }

        // Return to Menu prompt
        let prompt_col = if pulse {
            (255, 255, 255)
        } else {
            (180, 200, 220)
        };
        gpu::draw_rect_flat(40, 188, 240, 24, 200, 30, 50);
        draw_text(60, 196, "PRESS CROSS TO CONTINUE", prompt_col, 1);
    }
}
