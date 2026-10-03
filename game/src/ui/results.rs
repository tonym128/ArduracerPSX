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
        }
    }

    /// Handles screen input. Returns:
    /// - (true, false) for Retry / Continue
    /// - (false, true) for Exit to Menu
    pub fn update(&mut self, pad: &PadState) -> (bool, bool) {
        self.anim_timer = self.anim_timer.wrapping_add(1);
        let b = pad.buttons;
        let cont = b.is_held(button::CROSS) || b.is_held(button::START);
        let exit = b.is_held(button::CIRCLE);
        (cont, exit)
    }

    /// Renders the race results screen.
    pub fn render(&self, draw_y: i16) {
        let base_y = draw_y as u16;

        // Background panel
        gpu::fill_rect(20, base_y + 16, 280, 208, 16, 20, 28);
        gpu::fill_rect(22, base_y + 18, 276, 204, 24, 28, 40);

        // Header
        draw_text(90, base_y + 24, "RACE FINISHED!", (255, 220, 0), 2);
        gpu::fill_rect(40, base_y + 44, 240, 2, 220, 40, 60);

        // Medal Banner
        let (medal_str, medal_col) = match self.medal {
            Medal::DevPlatinum => ("DEV PLATINUM MEDAL!", (0, 220, 255)),
            Medal::Gold => ("GOLD MEDAL EARNED!", (255, 215, 0)),
            Medal::Silver => ("SILVER MEDAL EARNED!", (200, 200, 210)),
            Medal::Bronze => ("BRONZE MEDAL EARNED!", (205, 127, 50)),
            Medal::None => ("NO MEDAL - KEEP PRACTICING!", (150, 160, 170)),
        };

        // Pulsing medal box
        gpu::fill_rect(36, base_y + 54, 248, 26, 35, 40, 55);
        draw_text(48, base_y + 62, medal_str, medal_col, 1);

        // Best Lap Time Display
        let best_sec = self.best_lap_ticks / 60;
        let best_cs = ((self.best_lap_ticks % 60) * 100) / 60;

        draw_text(40, base_y + 96, "BEST LAP: ", (0, 220, 255), 1);
        let m_sec_ten = b'0' + ((best_sec / 10) % 10) as u8;
        let m_sec_one = b'0' + (best_sec % 10) as u8;
        let cs_ten = b'0' + ((best_cs / 10) % 10) as u8;
        let cs_one = b'0' + (best_cs % 10) as u8;

        draw_char(120, base_y + 96, m_sec_ten, (255, 255, 255), 1);
        draw_char(127, base_y + 96, m_sec_one, (255, 255, 255), 1);
        draw_char(134, base_y + 96, b'.', (255, 255, 255), 1);
        draw_char(141, base_y + 96, cs_ten, (255, 255, 255), 1);
        draw_char(148, base_y + 96, cs_one, (255, 255, 255), 1);
        draw_char(158, base_y + 96, b'S', (180, 180, 180), 1);

        // Total Race Time Display
        let tot_sec = self.total_race_ticks / 60;
        let tot_min = tot_sec / 60;
        let tot_sec_rem = tot_sec % 60;

        draw_text(40, base_y + 118, "TOTAL TIME: ", (0, 220, 255), 1);
        let min_char = b'0' + (tot_min.min(9) as u8);
        let sec_ten = b'0' + ((tot_sec_rem / 10) % 10) as u8;
        let sec_one = b'0' + (tot_sec_rem % 10) as u8;

        draw_char(130, base_y + 118, min_char, (255, 255, 255), 1);
        draw_char(137, base_y + 118, b':', (255, 255, 255), 1);
        draw_char(144, base_y + 118, sec_ten, (255, 255, 255), 1);
        draw_char(151, base_y + 118, sec_one, (255, 255, 255), 1);

        // Position & Championship Points Display
        let (pos_str, pos_col) = match self.rank {
            1 => ("POSITION: 1ST  (+10 PTS)", (255, 215, 0)),
            2 => ("POSITION: 2ND  (+6 PTS)", (210, 210, 220)),
            3 => ("POSITION: 3RD  (+4 PTS)", (205, 127, 50)),
            4 => ("POSITION: 4TH  (+3 PTS)", (180, 200, 220)),
            5 => ("POSITION: 5TH  (+2 PTS)", (160, 180, 200)),
            _ => ("POSITION: 6TH  (+1 PT)", (140, 150, 170)),
        };
        draw_text(40, base_y + 138, pos_str, pos_col, 1);

        // Action Buttons
        gpu::fill_rect(40, base_y + 158, 240, 28, 220, 25, 45);
        draw_text(
            60,
            base_y + 166,
            "CROSS: NEXT RACE / RETRY",
            (255, 255, 255),
            1,
        );

        draw_text(76, base_y + 196, "CIRCLE: MAIN MENU", (130, 150, 170), 1);
    }
}
