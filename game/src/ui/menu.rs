//! Main Menu Screen for PlayStation 1.
//!
//! Provides edge-triggered selection between Time Trial, Grand Prix,
//! Tuning Garage, and Records & Medals with smooth arcade styling.

use crate::ui::font::draw_text;
use psx_gpu as gpu;
use psx_pad::{button, PadState};

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum MenuItem {
    GrandPrix = 0,
    TimeTrial = 1,
    ManiaPrix = 2,
    ManiaTrials = 3,
    CityFreeDrive = 4,
    SplitScreen2P = 5,
    TuningGarage = 6,
    Records = 7,
    Settings = 8,
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
    /// Clears the edge baseline so a button held on the previous screen does
    /// not immediately act on this one.
    pub fn arm_for_entry(&mut self) {
        self.prev_up = true;
        self.prev_down = true;
        self.prev_confirm = true;
    }

    pub fn update(&mut self, pad: &PadState) -> Option<MenuItem> {
        self.anim_timer = self.anim_timer.wrapping_add(1);

        let b = pad.buttons;
        let up = b.is_held(button::UP);
        let down = b.is_held(button::DOWN);

        if up && !self.prev_up {
            if self.selected_idx > 0 {
                self.selected_idx -= 1;
            } else {
                self.selected_idx = 8;
            }
        } else if down && !self.prev_down {
            if self.selected_idx < 8 {
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
                0 => Some(MenuItem::GrandPrix),
                1 => Some(MenuItem::TimeTrial),
                2 => Some(MenuItem::ManiaPrix),
                3 => Some(MenuItem::ManiaTrials),
                4 => Some(MenuItem::CityFreeDrive),
                5 => Some(MenuItem::SplitScreen2P),
                6 => Some(MenuItem::TuningGarage),
                7 => Some(MenuItem::Records),
                8 => Some(MenuItem::Settings),
                _ => None,
            }
        } else {
            None
        }
    }

    /// Renders the main menu screen.
    pub fn render(&self) {
        // Outer border & shadow
        gpu::draw_rect_flat(18, 10, 284, 220, 12, 14, 20);
        gpu::draw_rect_flat(20, 12, 280, 216, 24, 28, 38);
        gpu::draw_rect_flat(22, 14, 276, 212, 32, 36, 48);

        // Header Title
        draw_text(110, 18, "MAIN MENU", (255, 225, 30), 2);
        gpu::draw_rect_flat(40, 34, 240, 2, 220, 40, 60);

        let items = [
            "1. GRAND PRIX",
            "2. GP TIME TRIAL",
            "3. MANIA PRIX",
            "4. MANIA TRIALS",
            "5. FREE ROAM",
            "6. 2P SPLIT SCREEN",
            "7. TUNING GARAGE",
            "8. STAGE CHALLENGES",
            "9. SETTINGS",
        ];

        for (idx, label) in items.iter().enumerate() {
            let y = 39 + (idx as i16) * 18;
            let is_sel = (idx as u8) == self.selected_idx;

            if is_sel {
                // Pulsing highlight box
                let pulse = (self.anim_timer / 15).is_multiple_of(2);
                let bg_color = if pulse {
                    (220, 35, 55) // Bright crimson
                } else {
                    (180, 25, 45) // Deep crimson
                };
                gpu::draw_rect_flat(44, y - 2, 232, 16, bg_color.0, bg_color.1, bg_color.2);
                draw_text(52, y as u16, label, (255, 255, 255), 1);
                // Pulsing cursor arrows
                draw_text(252, y as u16, "<<", (255, 235, 40), 1);
            } else {
                gpu::draw_rect_flat(44, y - 2, 232, 16, 40, 45, 58);
                draw_text(52, y as u16, label, (180, 190, 205), 1);
            }
        }

        // Footer instructions
        draw_text(
            52,
            206,
            "D-PAD: SELECT   CROSS: CONFIRM",
            (140, 155, 175),
            1,
        );
    }
}
