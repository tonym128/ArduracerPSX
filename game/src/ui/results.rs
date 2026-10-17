//! Race Results and Medal Ceremony Screen.
//!
//! Displays race completion times, split records, earned medals,
//! Grand Prix championship standings, and retry/continue prompts.

use crate::ui::font::{draw_char, draw_text};
use arduracer_core::championship::{ChampionshipSession, POINTS_TABLE};
use arduracer_core::timing::Medal;
use arduracer_core::TrackDef;
use psx_gpu as gpu;
use psx_pad::{button, PadState};

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ResultsAction {
    Continue,
    RestartRace,
    ExitToMenu,
}

pub struct ResultsScreen {
    pub best_lap_ticks: u32,
    pub total_race_ticks: u32,
    pub medal: Medal,
    pub rank: u8,
    pub points: u8,
    pub is_grand_prix: bool,
    pub anim_timer: u32,
    pub prev_buttons: u16,
}

impl ResultsScreen {
    pub fn new(
        best_lap_ticks: u32,
        total_race_ticks: u32,
        track: &TrackDef,
        rank: u8,
        is_grand_prix: bool,
    ) -> Self {
        let medal = track.par_times.evaluate_medal(best_lap_ticks);
        let safe_rank = rank.clamp(1, 6);
        let points = POINTS_TABLE[(safe_rank - 1) as usize];
        ResultsScreen {
            best_lap_ticks,
            total_race_ticks,
            medal,
            rank: safe_rank,
            points,
            is_grand_prix,
            anim_timer: 0,
            prev_buttons: 0,
        }
    }

    /// Handles screen input.
    pub fn update(&mut self, pad: &PadState) -> Option<ResultsAction> {
        self.anim_timer = self.anim_timer.wrapping_add(1);
        let b = pad.buttons;
        let prev = psx_pad::ButtonState::from_bits(self.prev_buttons);
        self.prev_buttons = b.bits();

        // 30-frame (~0.5s) initial lockout so player does not accidentally
        // skip the results screen with inputs held across the finish line.
        let can_input = self.anim_timer > 30;
        if !can_input {
            return None;
        }

        if b.pressed_since(prev, button::CROSS) || b.pressed_since(prev, button::START) {
            Some(ResultsAction::Continue)
        } else if b.pressed_since(prev, button::SQUARE) {
            Some(ResultsAction::RestartRace)
        } else if b.pressed_since(prev, button::CIRCLE) {
            Some(ResultsAction::ExitToMenu)
        } else {
            None
        }
    }

    /// Renders the race results screen with optional championship standings.
    pub fn render(&self, champ: Option<&ChampionshipSession>) {
        if self.is_grand_prix {
            if let Some(c) = champ {
                self.render_grand_prix(c);
                return;
            }
        }
        self.render_standard();
    }

    fn render_standard(&self) {
        // Background card and shadow
        gpu::draw_rect_flat(18, 14, 284, 212, 10, 12, 18);
        gpu::draw_rect_flat(20, 16, 280, 208, 18, 22, 32);
        gpu::draw_rect_flat(22, 18, 276, 204, 26, 32, 46);

        // Header banner
        draw_text(84, 24, "RACE FINISHED!", (255, 220, 0), 2);
        gpu::draw_rect_flat(36, 44, 248, 2, 220, 40, 60);

        // Medal Banner
        let (medal_str, medal_col) = match self.medal {
            Medal::DevPlatinum => ("DEV PLATINUM MEDAL!", (0, 220, 255)),
            Medal::Gold => ("GOLD MEDAL EARNED!", (255, 215, 0)),
            Medal::Silver => ("SILVER MEDAL EARNED!", (200, 200, 210)),
            Medal::Bronze => ("BRONZE MEDAL EARNED!", (205, 127, 50)),
            Medal::None => ("NO MEDAL - KEEP PRACTICING!", (150, 160, 170)),
        };

        // Highlighted medal badge
        gpu::draw_rect_flat(36, 54, 248, 26, 36, 44, 62);
        draw_text(48, 62, medal_str, medal_col, 1);

        // Best Lap Time Display
        let best_sec = self.best_lap_ticks / 60;
        let best_cs = ((self.best_lap_ticks % 60) * 100) / 60;

        draw_text(40, 94, "BEST LAP: ", (0, 220, 255), 1);
        let m_sec_ten = b'0' + ((best_sec / 10) % 10) as u8;
        let m_sec_one = b'0' + (best_sec % 10) as u8;
        let cs_ten = b'0' + ((best_cs / 10) % 10) as u8;
        let cs_one = b'0' + (best_cs % 10) as u8;

        draw_char(120, 94, m_sec_ten, (255, 255, 255), 1);
        draw_char(127, 94, m_sec_one, (255, 255, 255), 1);
        draw_char(134, 94, b'.', (255, 255, 255), 1);
        draw_char(141, 94, cs_ten, (255, 255, 255), 1);
        draw_char(148, 94, cs_one, (255, 255, 255), 1);
        draw_char(158, 94, b'S', (180, 180, 180), 1);

        // Total Race Time Display
        let tot_sec = self.total_race_ticks / 60;
        let tot_min = tot_sec / 60;
        let tot_sec_rem = tot_sec % 60;

        draw_text(40, 114, "TOTAL TIME: ", (0, 220, 255), 1);
        let min_char = b'0' + (tot_min.min(9) as u8);
        let sec_ten = b'0' + ((tot_sec_rem / 10) % 10) as u8;
        let sec_one = b'0' + (tot_sec_rem % 10) as u8;

        draw_char(130, 114, min_char, (255, 255, 255), 1);
        draw_char(137, 114, b':', (255, 255, 255), 1);
        draw_char(144, 114, sec_ten, (255, 255, 255), 1);
        draw_char(151, 114, sec_one, (255, 255, 255), 1);

        // Position & Points Display
        let (pos_str, pos_col) = match self.rank {
            1 => ("POSITION: 1ST  (+10 PTS)", (255, 215, 0)),
            2 => ("POSITION: 2ND  (+6 PTS)", (210, 210, 220)),
            3 => ("POSITION: 3RD  (+4 PTS)", (205, 127, 50)),
            4 => ("POSITION: 4TH  (+3 PTS)", (180, 200, 220)),
            5 => ("POSITION: 5TH  (+2 PTS)", (160, 180, 200)),
            _ => ("POSITION: 6TH  (+1 PT)", (140, 150, 170)),
        };
        draw_text(40, 134, pos_str, pos_col, 1);

        // Action Buttons
        let prompt_pulse = (self.anim_timer >> 3) & 1 == 0;
        let btn_col = if prompt_pulse {
            (230, 35, 55)
        } else {
            (190, 25, 45)
        };
        gpu::draw_rect_flat(40, 156, 240, 22, btn_col.0, btn_col.1, btn_col.2);
        draw_text(
            52,
            163,
            "CROSS: CONTINUE   SQUARE: RETRY",
            (255, 255, 255),
            1,
        );

        draw_text(76, 190, "CIRCLE: MAIN MENU", (130, 150, 170), 1);
    }

    fn render_grand_prix(&self, champ: &ChampionshipSession) {
        // Outer card and border
        gpu::draw_rect_flat(14, 10, 292, 220, 10, 12, 18);
        gpu::draw_rect_flat(16, 12, 288, 216, 18, 22, 32);
        gpu::draw_rect_flat(18, 14, 284, 212, 26, 32, 46);

        // Header: Cup Name & Stage Number
        let cup_title = champ.cup_name();
        draw_text(30, 18, cup_title, (255, 220, 0), 1);

        let stage_num = champ.current_stage + 1;
        draw_text(160, 18, "STAGE ", (0, 220, 255), 1);
        let stage_char = b'0' + stage_num;
        draw_char(200, 18, stage_char, (255, 255, 255), 1);
        draw_text(208, 18, "/6", (180, 190, 205), 1);

        gpu::draw_rect_flat(26, 30, 268, 2, 220, 40, 60);

        // Left column (Race Summary)
        let lx = 26i16;
        gpu::draw_rect_flat(lx, 36, 126, 122, 32, 38, 54);
        gpu::draw_rect_flat(lx + 2, 38, 122, 118, 22, 26, 38);

        draw_text(lx as u16 + 8, 42, "RACE RESULT", (0, 220, 255), 1);

        let (pos_str, pos_col) = match self.rank {
            1 => ("1ST PLACE", (255, 215, 0)),
            2 => ("2ND PLACE", (210, 210, 220)),
            3 => ("3RD PLACE", (205, 127, 50)),
            4 => ("4TH PLACE", (180, 200, 220)),
            5 => ("5TH PLACE", (160, 180, 200)),
            _ => ("6TH PLACE", (140, 150, 170)),
        };
        draw_text(lx as u16 + 8, 56, pos_str, pos_col, 1);

        // Points earned
        draw_text(lx as u16 + 8, 70, "+", (255, 235, 40), 1);
        let pts_ten = b'0' + ((self.points / 10) % 10);
        let pts_one = b'0' + (self.points % 10);
        if self.points >= 10 {
            draw_char(lx as u16 + 16, 70, pts_ten, (255, 235, 40), 1);
            draw_char(lx as u16 + 23, 70, pts_one, (255, 235, 40), 1);
            draw_text(lx as u16 + 32, 70, "POINTS", (255, 235, 40), 1);
        } else {
            draw_char(lx as u16 + 16, 70, pts_one, (255, 235, 40), 1);
            draw_text(lx as u16 + 25, 70, "POINTS", (255, 235, 40), 1);
        }

        // Best lap
        let best_sec = self.best_lap_ticks / 60;
        let best_cs = ((self.best_lap_ticks % 60) * 100) / 60;
        draw_text(lx as u16 + 8, 86, "BEST LAP:", (140, 160, 185), 1);
        let m_sec_ten = b'0' + ((best_sec / 10) % 10) as u8;
        let m_sec_one = b'0' + (best_sec % 10) as u8;
        let cs_ten = b'0' + ((best_cs / 10) % 10) as u8;
        let cs_one = b'0' + (best_cs % 10) as u8;
        draw_char(lx as u16 + 8, 98, m_sec_ten, (255, 255, 255), 1);
        draw_char(lx as u16 + 15, 98, m_sec_one, (255, 255, 255), 1);
        draw_char(lx as u16 + 22, 98, b'.', (255, 255, 255), 1);
        draw_char(lx as u16 + 29, 98, cs_ten, (255, 255, 255), 1);
        draw_char(lx as u16 + 36, 98, cs_one, (255, 255, 255), 1);
        draw_char(lx as u16 + 46, 98, b'S', (180, 180, 180), 1);

        // Grid feedback for next race
        draw_text(lx as u16 + 8, 114, "NEXT GRID:", (140, 160, 185), 1);
        // Find player position in champ.grid_positions
        let mut p_grid = 1u8;
        for (g_idx, &c_idx) in champ.grid_positions.iter().enumerate() {
            if c_idx == 0 {
                p_grid = (g_idx + 1) as u8;
                break;
            }
        }
        let grid_char = b'0' + p_grid;
        draw_char(lx as u16 + 8, 126, grid_char, (255, 220, 0), 1);
        let suffix = match p_grid {
            1 => "ST (POLE)",
            2 => "ND",
            3 => "RD",
            _ => "TH",
        };
        draw_text(lx as u16 + 16, 126, suffix, (255, 220, 0), 1);

        // Right column (Championship Standings Leaderboard)
        let rx = 158i16;
        gpu::draw_rect_flat(rx, 36, 136, 122, 32, 38, 54);
        gpu::draw_rect_flat(rx + 2, 38, 132, 118, 22, 26, 38);

        draw_text(rx as u16 + 6, 42, "CUP STANDINGS", (0, 220, 255), 1);

        let board = champ.sorted_leaderboard();
        for (i, comp) in board.iter().enumerate() {
            let row_y = 56u16 + (i as u16) * 16;
            let is_p = comp.is_player;

            if is_p {
                gpu::draw_rect_flat(rx + 4, (row_y - 2) as i16, 128, 14, 180, 25, 45);
            }

            let rank_char = b'1' + (i as u8);
            draw_char(rx as u16 + 6, row_y, rank_char, (255, 255, 255), 1);
            draw_text(rx as u16 + 13, row_y, ".", (255, 255, 255), 1);

            let name_col = if is_p { (255, 255, 255) } else { comp.color };
            draw_text(rx as u16 + 21, row_y, comp.name, name_col, 1);

            let pts = comp.total_points;
            let p_ten = b'0' + ((pts / 10) % 10) as u8;
            let p_one = b'0' + (pts % 10) as u8;
            draw_char(rx as u16 + 90, row_y, p_ten, (255, 220, 0), 1);
            draw_char(rx as u16 + 97, row_y, p_one, (255, 220, 0), 1);
            draw_text(rx as u16 + 106, row_y, "P", (160, 175, 195), 1);
        }

        // Action Buttons Footer
        let prompt_pulse = (self.anim_timer >> 3) & 1 == 0;
        let btn_col = if prompt_pulse {
            (230, 35, 55)
        } else {
            (190, 25, 45)
        };
        gpu::draw_rect_flat(26, 164, 268, 22, btn_col.0, btn_col.1, btn_col.2);
        draw_text(
            34,
            171,
            "CROSS: NEXT RACE   SQUARE: RETRY",
            (255, 255, 255),
            1,
        );

        draw_text(90, 194, "CIRCLE: MAIN MENU", (130, 150, 170), 1);
    }
}
