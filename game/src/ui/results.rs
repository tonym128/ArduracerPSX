//! Race Results and Medal Ceremony Screen.
//!
//! Displays race completion times, split records, earned medals, and
//! retry/continue prompts.

use crate::ui::font::{draw_char, draw_text};
use arduracer_core::championship::POINTS_TABLE;
use arduracer_core::timing::Medal;
use arduracer_core::TrackDef;
use psx_gpu as gpu;
use psx_pad::{button, PadState};

pub struct ResultsScreen {
    pub best_lap_ticks: u32,
    pub total_race_ticks: u32,
    pub medal: Medal,
    pub rank: u8,
    pub points: u8,
    pub anim_timer: u32,
    pub prev_buttons: u16,
}

impl ResultsScreen {
    pub fn new(best_lap_ticks: u32, total_race_ticks: u32, track: &TrackDef, rank: u8) -> Self {
        let medal = track.par_times.evaluate_medal(best_lap_ticks);
        let safe_rank = rank.max(1).min(6);
        let points = POINTS_TABLE[(safe_rank - 1) as usize];
        ResultsScreen {
            best_lap_ticks,
            total_race_ticks,
            medal,
            rank: safe_rank,
            points,
            anim_timer: 0,
            prev_buttons: 0xFFFF,
        }
    }

    /// Handles screen input. Returns:
    /// - (true, false) for Retry / Continue
    /// - (false, true) for Exit to Menu
    pub fn update(&mut self, pad: &PadState) -> (bool, bool) {
        self.anim_timer = self.anim_timer.wrapping_add(1);
        let b = pad.buttons;
        let prev = psx_pad::ButtonState::from_bits(self.prev_buttons);
        self.prev_buttons = b.bits();

        // 30-frame (~0.5s) initial lockout so player does not accidentally
        // skip the results screen with inputs held across the finish line.
        let can_input = self.anim_timer > 30;
        let cont = can_input
            && (b.pressed_since(prev, button::CROSS) || b.pressed_since(prev, button::START));
        let exit = can_input && b.pressed_since(prev, button::CIRCLE);
        (cont, exit)
    }

    /// Renders the race results screen with screen-relative coordinates.
    pub fn render(&self, _draw_y: i16) {
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

        // Position & Championship Points Display
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
        gpu::draw_rect_flat(40, 156, 240, 24, btn_col.0, btn_col.1, btn_col.2);
        draw_text(56, 164, "CROSS: NEXT RACE / RETRY", (255, 255, 255), 1);

        draw_text(76, 192, "CIRCLE: MAIN MENU", (130, 150, 170), 1);
    }
}
