//! Main Menu Screen for PlayStation 1.
//!
//! Provides edge-triggered selection between Time Trial, Grand Prix,
//! Tuning Garage, and Records & Medals with smooth arcade styling.

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
    pub prev_confirm: bool,
    pub anim_timer: u32,
}

impl Default for MainMenu {
    fn default() -> Self {
        Self::new()
    }
}

impl MainMenu {
    pub const fn new() -> Self {
        MainMenu {
            selected_idx: 0,
            prev_up: false,
            prev_down: false,
            prev_confirm: true, // Edge-trigger: must release before confirming
            anim_timer: 0,
        }
    }

    /// Handles menu navigation and confirms selection on fresh press.
    pub fn update(&mut self, pad: &PadState) -> Option<MenuItem> {
        self.anim_timer = self.anim_timer.wrapping_add(1);

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

        let confirm_held = b.is_held(button::CROSS) || b.is_held(button::START);
        let confirmed = confirm_held && !self.prev_confirm;
        self.prev_confirm = confirm_held;

        if confirmed {
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
    pub fn render(&self, _draw_y: i16) {
        // Outer border & shadow
        gpu::draw_rect_flat(18, 18, 284, 204, 12, 14, 20);
        gpu::draw_rect_flat(20, 20, 280, 200, 24, 28, 38);
        gpu::draw_rect_flat(22, 22, 276, 196, 32, 36, 48);

        // Header Title
        draw_text(110, 36, "MAIN MENU", (255, 225, 30), 2);
        gpu::draw_rect_flat(40, 58, 240, 2, 220, 40, 60);

        let items = [
            "1. TIME TRIAL",
            "2. GRAND PRIX",
            "3. TUNING GARAGE",
            "4. RECORDS & MEDALS",
        ];

        for (idx, label) in items.iter().enumerate() {
            let y = 78 + (idx as i16) * 30;
            let is_sel = (idx as u8) == self.selected_idx;

            if is_sel {
                // Pulsing highlight box
                let pulse = (self.anim_timer / 15).is_multiple_of(2);
                let bg_color = if pulse {
                    (220, 35, 55) // Bright crimson
                } else {
                    (180, 25, 45) // Deep crimson
                };
                gpu::draw_rect_flat(50, y - 4, 220, 22, bg_color.0, bg_color.1, bg_color.2);
                draw_text(60, y as u16, label, (255, 255, 255), 1);
                // Pulsing cursor arrows
                draw_text(240, y as u16, "<<", (255, 235, 40), 1);
            } else {
                gpu::draw_rect_flat(50, y - 4, 220, 22, 40, 45, 58);
                draw_text(60, y as u16, label, (180, 190, 205), 1);
            }
        }

        // Footer instructions
        draw_text(
            52,
            196,
            "D-PAD: SELECT   CROSS: CONFIRM",
            (140, 155, 175),
            1,
        );
    }
}
