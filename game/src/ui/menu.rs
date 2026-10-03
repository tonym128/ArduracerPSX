//! Main Menu Screen.
//!
//! Provides selection between Time Attack, Grand Prix, Tuning Garage, and Records.

use crate::ui::font::draw_text;
use psx_gpu as gpu;
use psx_pad::{button, PadState};

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum MenuItem {
    TimeTrial = 0,
    GrandPrix = 1,
    TuningGarage = 2,
    Records = 3,
}

pub struct MainMenu {
    pub selected_idx: u8,
    pub prev_up: bool,
    pub prev_down: bool,
}

impl MainMenu {
    pub const fn new() -> Self {
        MainMenu {
            selected_idx: 0,
            prev_up: false,
            prev_down: false,
        }
    }

    /// Handles menu navigation and confirms selection.
    pub fn update(&mut self, pad: &PadState) -> Option<MenuItem> {
        let b = pad.buttons;
        let up = b.is_held(button::UP);
        let down = b.is_held(button::DOWN);

        if up && !self.prev_up {
            if self.selected_idx > 0 {
                self.selected_idx -= 1;
            } else {
                self.selected_idx = 3;
            }
        } else if down && !self.prev_down {
            if self.selected_idx < 3 {
                self.selected_idx += 1;
            } else {
                self.selected_idx = 0;
            }
        }

        self.prev_up = up;
        self.prev_down = down;

        if b.is_held(button::CROSS) || b.is_held(button::START) {
            match self.selected_idx {
                0 => Some(MenuItem::TimeTrial),
                1 => Some(MenuItem::GrandPrix),
                2 => Some(MenuItem::TuningGarage),
                3 => Some(MenuItem::Records),
                _ => None,
            }
        } else {
            None
        }
    }

    /// Renders the main menu screen.
    pub fn render(&self, draw_y: i16) {
        let base_y = draw_y as u16;

        // Background panel
        gpu::fill_rect(20, base_y + 20, 280, 200, 20, 22, 30);
        gpu::fill_rect(22, base_y + 22, 276, 196, 28, 32, 42);

        // Header
        draw_text(110, base_y + 36, "MAIN MENU", (255, 220, 0), 2);
        gpu::fill_rect(40, base_y + 58, 240, 2, 220, 40, 60);

        let items = [
            "1. TIME TRIAL",
            "2. GRAND PRIX",
            "3. TUNING GARAGE",
            "4. RECORDS & MEDALS",
        ];

        for (idx, label) in items.iter().enumerate() {
            let y = base_y + 78 + (idx as u16) * 30;
            let is_sel = (idx as u8) == self.selected_idx;

            if is_sel {
                // Highlight box
                gpu::fill_rect(50, y - 4, 220, 22, 220, 30, 50);
                draw_text(60, y, label, (255, 255, 255), 1);
                // Cursor arrow
                draw_text(245, y, "<", (255, 240, 0), 1);
            } else {
                gpu::fill_rect(50, y - 4, 220, 22, 35, 40, 50);
                draw_text(60, y, label, (180, 190, 205), 1);
            }
        }

        // Footer instructions
        draw_text(
            52,
            base_y + 196,
            "D-PAD: SELECT   CROSS: CONFIRM",
            (120, 140, 160),
            1,
        );
    }
}
