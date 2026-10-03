//! In-Race Pause Overlay (DualShock `Start`).
//!
//! GAME.md §7 assigns `Start` to "Pause Game Menu" and `Select` to
//! "Toggle In-Game Minimap / HUD". This module owns both so the race loop stays
//! a single, readable `match` arm.

use crate::ui::font::draw_text;
use psx_gpu as gpu;
use psx_pad::{button, PadState};

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PauseChoice {
    /// Nothing changed this frame.
    None,
    Resume,
    RestartRace,
    QuitToMenu,
}

pub struct PauseMenu {
    pub selected_idx: u8,
    pub prev_up: bool,
    pub prev_down: bool,
    /// Button mask from the previous frame, for edge detection.
    pub prev_buttons: u16,
    /// Set when `Select` is pressed; the race loop consumes and clears it.
    pub toggle_hud_request: bool,
}

const ITEM_COUNT: u8 = 3;

impl PauseMenu {
    pub const fn new() -> Self {
        PauseMenu {
            selected_idx: 0,
            prev_up: false,
            prev_down: false,
            prev_buttons: 0,
            toggle_hud_request: false,
        }
    }

    /// Handles navigation and confirms a selection.
    pub fn update(&mut self, pad: &PadState) -> PauseChoice {
        let b = pad.buttons;
        let prev = psx_pad::ButtonState::from_bits(self.prev_buttons);
        self.prev_buttons = b.bits();

        if b.pressed_since(prev, button::SELECT) {
            self.toggle_hud_request = true;
        }

        let up = b.is_held(button::UP);
        let down = b.is_held(button::DOWN);
        if up && !self.prev_up {
            self.selected_idx = if self.selected_idx == 0 {
                ITEM_COUNT - 1
            } else {
                self.selected_idx - 1
            };
        } else if down && !self.prev_down {
            self.selected_idx = if self.selected_idx + 1 >= ITEM_COUNT {
                0
            } else {
                self.selected_idx + 1
            };
        }
        self.prev_up = up;
        self.prev_down = down;

        if b.pressed_since(prev, button::START) || b.pressed_since(prev, button::CIRCLE) {
            PauseChoice::Resume
        } else if b.pressed_since(prev, button::CROSS) {
            match self.selected_idx {
                0 => PauseChoice::Resume,
                1 => PauseChoice::RestartRace,
                _ => PauseChoice::QuitToMenu,
            }
        } else {
            PauseChoice::None
        }
    }

    /// Clears the one-shot HUD toggle request.
    pub fn take_toggle_hud(&mut self) -> bool {
        let v = self.toggle_hud_request;
        self.toggle_hud_request = false;
        v
    }

    /// Renders the dimmed pause panel.
    pub fn render(&self, _draw_y: i16) {
        // Drop shadow and panel backing
        gpu::draw_rect_flat(68, 54, 184, 132, 8, 10, 14);
        gpu::draw_rect_flat(70, 56, 180, 128, 16, 20, 30);
        gpu::draw_rect_flat(72, 58, 176, 124, 26, 30, 42);
        gpu::draw_rect_flat(72, 58, 176, 2, 220, 40, 60);

        draw_text(102, 68, "PAUSED", (255, 220, 0), 2);

        let items = ["RESUME", "RESTART RACE", "QUIT TO MENU"];
        for (idx, label) in items.iter().enumerate() {
            let y = 100 + (idx as u16) * 22;
            let selected = (idx as u8) == self.selected_idx;
            if selected {
                gpu::draw_rect_flat(82, (y - 3) as i16, 156, 16, 200, 30, 50);
                draw_text(100, y, label, (255, 255, 255), 1);
                draw_text(88, y, ">", (255, 240, 0), 1);
            } else {
                draw_text(100, y, label, (150, 165, 185), 1);
            }
        }

        draw_text(80, 168, "CROSS SELECT   START RESUME", (110, 130, 155), 1);
    }
}
