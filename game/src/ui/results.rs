//! Race Results and Medal Ceremony Screen.
//!
//! Displays race completion times, split records, earned medals,
//! Grand Prix championship standings, and retry/continue prompts.

use crate::ui::font::{draw_char, draw_text};
use arduracer_core::championship::{ChampionshipSession, POINTS_TABLE};
use arduracer_core::timing::{LapTimer, Medal};
use arduracer_core::TrackDef;
use psx_gpu as gpu;
use psx_pad::{button, PadState};

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ResultsAction {
    Continue,
    RestartRace,
    WatchReplay,
    ExitToMenu,
}

pub struct ResultsScreen {
    pub best_lap_ticks: u32,
    pub total_race_ticks: u32,
    pub medal: Medal,
    pub next_medal: Medal,
    pub next_medal_target_ticks: u32,
    pub ticks_to_next_medal: u32,
    pub rank: u8,
    pub points: u8,
    pub is_grand_prix: bool,
    pub is_2p_duel: bool,
    pub p1_won: bool,
    /// False when the player reached results without completing a validated lap.
    pub valid_lap_recorded: bool,
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
        let effective_best = if best_lap_ticks != 0 && best_lap_ticks != u32::MAX {
            best_lap_ticks
        } else {
            total_race_ticks
        };
        let medal = track.par_times.evaluate_medal(effective_best);
        let (next_medal, next_medal_target_ticks) = if medal == Medal::DevPlatinum {
            (Medal::None, 0)
        } else {
            track.par_times.next_medal_target(effective_best)
        };
        let safe_rank = rank.clamp(1, 6);
        let points = POINTS_TABLE[(safe_rank - 1) as usize];
        ResultsScreen {
            best_lap_ticks: effective_best,
            total_race_ticks,
            medal,
            next_medal,
            next_medal_target_ticks,
            ticks_to_next_medal: effective_best.saturating_sub(next_medal_target_ticks),
            rank: safe_rank,
            points,
            is_grand_prix,
            is_2p_duel: false,
            p1_won: true,
            valid_lap_recorded: true,
            anim_timer: 0,
            prev_buttons: 0,
        }
    }

    pub fn new_2p(
        best_lap_ticks: u32,
        total_race_ticks: u32,
        track: &TrackDef,
        p1_won: bool,
    ) -> Self {
        let effective_best = if best_lap_ticks != 0 && best_lap_ticks != u32::MAX {
            best_lap_ticks
        } else {
            total_race_ticks
        };
        let medal = track.par_times.evaluate_medal(effective_best);
        let (next_medal, next_medal_target_ticks) = if medal == Medal::DevPlatinum {
            (Medal::None, 0)
        } else {
            track.par_times.next_medal_target(effective_best)
        };
        ResultsScreen {
            best_lap_ticks: effective_best,
            total_race_ticks,
            medal,
            next_medal,
            next_medal_target_ticks,
            ticks_to_next_medal: effective_best.saturating_sub(next_medal_target_ticks),
            rank: if p1_won { 1 } else { 2 },
            points: if p1_won { 10 } else { 6 },
            is_grand_prix: false,
            is_2p_duel: true,
            p1_won,
            valid_lap_recorded: true,
            anim_timer: 0,
            prev_buttons: 0,
        }
    }

    fn format_ticks(buf: &mut [u8; 8], ticks: u32) {
        let (minutes, seconds, hundredths) = LapTimer::<1>::format_ticks(ticks);
        buf.copy_from_slice(&[
            b'0' + (minutes / 10) as u8 % 10,
            b'0' + (minutes % 10) as u8,
            b':',
            b'0' + (seconds / 10) as u8,
            b'0' + (seconds % 10) as u8,
            b'.',
            b'0' + (hundredths / 10) as u8,
            b'0' + (hundredths % 10) as u8,
        ]);
    }

    fn format_delta(buf: &mut [u8; 6], ticks: u32) {
        let seconds = ticks / 60;
        let hundredths = ((ticks % 60) * 100) / 60;
        buf.copy_from_slice(&[
            b'+',
            b'0' + ((seconds / 10) % 10) as u8,
            b'0' + (seconds % 10) as u8,
            b'.',
            b'0' + (hundredths / 10) as u8,
            b'0' + (hundredths % 10) as u8,
        ]);
    }

    fn blit(buf: &[u8], x: u16, y: u16, color: (u8, u8, u8)) {
        for (idx, &byte) in buf.iter().enumerate() {
            if byte != b' ' {
                draw_char(x + idx as u16 * 6, y, byte, color, 1);
            }
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

        if b.pressed_since(prev, button::SQUARE) {
            Some(ResultsAction::RestartRace)
        } else if b.pressed_since(prev, button::TRIANGLE) {
            Some(ResultsAction::WatchReplay)
        } else if b.pressed_since(prev, button::CIRCLE) {
            Some(ResultsAction::ExitToMenu)
        } else if (b.bits() & !prev.bits()) != 0 {
            Some(ResultsAction::Continue)
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
        if self.is_2p_duel {
            if self.p1_won {
                draw_text(84, 24, "PLAYER 1 WINS!", (255, 215, 0), 2);
            } else {
                draw_text(84, 24, "PLAYER 2 WINS!", (0, 220, 255), 2);
            }
        } else {
            draw_text(84, 24, "RACE FINISHED!", (255, 220, 0), 2);
        }
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
        draw_text(
            40,
            94,
            if self.valid_lap_recorded {
                "BEST LAP: "
            } else {
                "NO VALID LAP"
            },
            if self.valid_lap_recorded {
                (0, 220, 255)
            } else {
                (255, 110, 80)
            },
            1,
        );
        if !self.valid_lap_recorded {
            // Spell out the actual validation rules: order is flexible, direction is not,
            // and recovery does not invalidate the running lap.
            draw_text(
                40,
                104,
                "CLEAR EVERY GATE, IN ANY ORDER, FORWARD",
                (255, 190, 110),
                1,
            );
            draw_text(
                40,
                124,
                "CHECKPOINT RESPAWNS KEEP THE LAP LIVE",
                (170, 190, 210),
                1,
            );
        } else if self.best_lap_ticks == 0 || self.best_lap_ticks == u32::MAX {
            draw_text(120, 94, "--.-- S", (255, 255, 255), 1);
        } else {
            let best_sec = self.best_lap_ticks / 60;
            let best_cs = ((self.best_lap_ticks % 60) * 100) / 60;

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
        }

        if !self.valid_lap_recorded {
            // The validation guidance above replaces medal pace for an unscored run.
        } else if self.next_medal == Medal::None {
            draw_text(40, 104, "ALL MEDALS EARNED", (0, 220, 255), 1);
        } else {
            let (label, color) = match self.next_medal {
                Medal::Bronze => ("BRONZE", (205, 127, 50)),
                Medal::Silver => ("SILVER", (200, 200, 210)),
                Medal::Gold => ("GOLD", (255, 215, 0)),
                Medal::DevPlatinum => ("PLAT", (0, 220, 255)),
                Medal::None => ("", (150, 160, 170)),
            };
            draw_text(40, 104, "NEXT", (150, 165, 190), 1);
            draw_text(70, 104, label, color, 1);
            let mut target_buf = [b' '; 8];
            Self::format_ticks(&mut target_buf, self.next_medal_target_ticks);
            Self::blit(&target_buf, 110, 104, color);
            draw_text(164, 104, "NEED", (150, 165, 190), 1);
            let mut gap_buf = [b' '; 6];
            Self::format_delta(&mut gap_buf, self.ticks_to_next_medal);
            Self::blit(&gap_buf, 190, 104, color);
        }

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
        draw_text(85, 163, "PRESS ANY KEY TO CONTINUE", (255, 255, 255), 1);

        draw_text(
            42,
            186,
            "SQUARE: RETRY   TRIANGLE: REPLAY",
            (130, 150, 170),
            1,
        );
        let exit_label = if self.is_2p_duel {
            "CIRCLE: MAIN MENU"
        } else {
            "CIRCLE: STAGE SELECT"
        };
        let exit_x = if self.is_2p_duel { 112 } else { 96 };
        draw_text(exit_x, 198, exit_label, (130, 150, 170), 1);
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

        if !self.valid_lap_recorded {
            draw_text(lx as u16 + 8, 132, "NO VALID LAP", (255, 110, 80), 1);
            draw_text(
                lx as u16 + 8,
                142,
                "ALL GATES, ANY ORDER",
                (255, 190, 110),
                1,
            );
            draw_text(
                lx as u16 + 8,
                152,
                "FORWARD ONLY; RESPAWN OK",
                (170, 190, 210),
                1,
            );
        }

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
        draw_text(lx as u16 + 8, 86, "BEST LAP:", (140, 160, 185), 1);
        if self.best_lap_ticks == 0 || self.best_lap_ticks == u32::MAX {
            draw_text(lx as u16 + 8, 98, "--.-- S", (255, 255, 255), 1);
        } else {
            let best_sec = self.best_lap_ticks / 60;
            let best_cs = ((self.best_lap_ticks % 60) * 100) / 60;
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
        }

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
        draw_text(85, 171, "PRESS ANY KEY TO CONTINUE", (255, 255, 255), 1);

        draw_text(
            42,
            190,
            "SQUARE: RETRY   TRIANGLE: REPLAY",
            (130, 150, 170),
            1,
        );
        draw_text(112, 202, "CIRCLE: MAIN MENU", (130, 150, 170), 1);
    }
}
