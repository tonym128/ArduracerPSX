//! Track & Cup Selection Carousel for PlayStation 1.
//!
//! Allows browsing all 24 tracks partitioned across 4 cups with par times,
//! track stats, real track names, and edge-triggered confirmation.

use crate::ui::font::{draw_char, draw_text};
use arduracer_core::{TrackDef, ALL_TRACKS};
use psx_gpu as gpu;
use psx_pad::{button, PadState};

pub struct TrackSelectScreen {
    pub selected_track_idx: usize,
    pub prev_left: bool,
    pub prev_right: bool,
    pub prev_up: bool,
    pub prev_down: bool,
    pub prev_confirm: bool,
    pub prev_cancel: bool,
}

impl Default for TrackSelectScreen {
    fn default() -> Self {
        Self::new()
    }
}

impl TrackSelectScreen {
    pub const fn new() -> Self {
        TrackSelectScreen {
            selected_track_idx: 0,
            prev_left: false,
            prev_right: false,
            prev_up: false,
            prev_down: false,
            prev_confirm: true, // Edge-trigger: must release before confirming
            prev_cancel: true,
        }
    }

    /// Handles carousel navigation. Returns:
    /// - (Some(track_idx), false) if confirmed with CROSS or START
    /// - (None, true) if cancelled with CIRCLE
    /// - (None, false) if still browsing
    pub fn update(&mut self, pad: &PadState) -> (Option<usize>, bool) {
        let b = pad.buttons;
        let left = b.is_held(button::LEFT);
        let right = b.is_held(button::RIGHT);
        let up = b.is_held(button::UP);
        let down = b.is_held(button::DOWN);

        // Previous / Next Track
        if right && !self.prev_right {
            self.selected_track_idx = (self.selected_track_idx + 1) % ALL_TRACKS.len();
        } else if left && !self.prev_left {
            if self.selected_track_idx > 0 {
                self.selected_track_idx -= 1;
            } else {
                self.selected_track_idx = ALL_TRACKS.len() - 1;
            }
        }

        // Cup Jump (+/- 6 tracks)
        if down && !self.prev_down {
            self.selected_track_idx = (self.selected_track_idx + 6) % ALL_TRACKS.len();
        } else if up && !self.prev_up {
            if self.selected_track_idx >= 6 {
                self.selected_track_idx -= 6;
            } else {
                self.selected_track_idx = ALL_TRACKS.len() - (6 - self.selected_track_idx);
            }
        }

        self.prev_left = left;
        self.prev_right = right;
        self.prev_up = up;
        self.prev_down = down;

        let confirm_held = b.is_held(button::CROSS) || b.is_held(button::START);
        let cancel_held = b.is_held(button::CIRCLE);

        let confirmed = confirm_held && !self.prev_confirm;
        let cancelled = cancel_held && !self.prev_cancel;

        self.prev_confirm = confirm_held;
        self.prev_cancel = cancel_held;

        if confirmed {
            (Some(self.selected_track_idx), false)
        } else if cancelled {
            (None, true)
        } else {
            (None, false)
        }
    }

    /// Renders the track selection screen.
    pub fn render(&self, _draw_y: i16) {
        let track: &'static TrackDef = ALL_TRACKS[self.selected_track_idx];

        // Background panel
        gpu::draw_rect_flat(10, 10, 300, 220, 16, 20, 28);
        gpu::draw_rect_flat(12, 12, 296, 216, 26, 32, 44);

        // Header: "SELECT CIRCUIT"
        draw_text(90, 20, "SELECT CIRCUIT", (255, 225, 30), 2);
        gpu::draw_rect_flat(30, 38, 260, 2, 220, 40, 60);

        // Cup classification
        let cup_name = match self.selected_track_idx / 6 {
            0 => "BRONZE CUP (AMATEUR)",
            1 => "SILVER CUP (PRO)",
            2 => "GOLD CUP (CHAMPION)",
            _ => "PLATINUM CUP (SUPER SPEEDWAYS)",
        };
        draw_text(50, 46, cup_name, (0, 220, 255), 1);

        // Track Number & Name Box
        gpu::draw_rect_flat(24, 62, 272, 34, 35, 42, 55);
        gpu::draw_rect_flat(26, 64, 268, 30, 20, 25, 35);

        // Track Index Number: e.g. "TRACK 01 / 24"
        let track_num = (self.selected_track_idx + 1) as u8;
        let num_str = [
            b'T',
            b'R',
            b'A',
            b'C',
            b'K',
            b' ',
            b'0' + (track_num / 10),
            b'0' + (track_num % 10),
            b' ',
            b'/',
            b' ',
            b'2',
            b'4',
            0,
        ];
        let mut cur_x = 34u16;
        for &ch in num_str.iter() {
            if ch == 0 {
                break;
            }
            draw_char(cur_x, 74, ch, (255, 240, 50), 1);
            cur_x += 7;
        }

        // Real track name
        draw_text(130, 74, track.name, (255, 255, 255), 1);

        // Target Par Times panel
        let pt = &track.par_times;
        let par_y = 106i16;
        draw_text(24, par_y as u16, "TARGET PAR TIMES:", (255, 255, 255), 1);

        let bronze_sec = pt.bronze_ticks / 60;
        let silver_sec = pt.silver_ticks / 60;
        let gold_sec = pt.gold_ticks / 60;
        let dev_sec = pt.dev_platinum_ticks / 60;

        let medals = [
            ("BRONZE: ", bronze_sec, (205, 127, 50)),
            ("SILVER: ", silver_sec, (192, 192, 192)),
            ("GOLD:   ", gold_sec, (255, 215, 0)),
            ("DEV PLAT:", dev_sec, (100, 220, 255)),
        ];

        for (idx, (label, sec, col)) in medals.iter().enumerate() {
            let my = par_y + 16 + (idx as i16) * 16;
            draw_text(30, my as u16, label, *col, 1);

            let s_ten = b'0' + ((sec / 10) % 10) as u8;
            let s_one = b'0' + (sec % 10) as u8;
            draw_char(110, my as u16, s_ten, (255, 255, 255), 1);
            draw_char(117, my as u16, s_one, (255, 255, 255), 1);
            draw_char(124, my as u16, b'S', (160, 160, 160), 1);
        }

        // Track Dimensions & Checkpoints Box (Right Panel)
        gpu::draw_rect_flat(170, par_y, 126, 76, 30, 36, 48);
        draw_text(178, (par_y + 8) as u16, "GATES: ", (0, 220, 255), 1);
        draw_char(
            234,
            (par_y + 8) as u16,
            b'0' + track.checkpoint_count,
            (255, 255, 255),
            1,
        );

        draw_text(178, (par_y + 26) as u16, "LAPS:  5", (0, 220, 255), 1);
        draw_text(178, (par_y + 44) as u16, "SPEED: HIGH", (255, 200, 40), 1);

        // Navigation Footer
        draw_text(
            24,
            202,
            "L/R: TRACK  U/D: CUP  CROSS: RACE  O: BACK",
            (130, 150, 170),
            1,
        );
    }
}
