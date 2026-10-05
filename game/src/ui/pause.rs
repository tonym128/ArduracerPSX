//! In-Race Pause Overlay (DualShock `Start`).
//!
//! GAME.md §7 assigns `Start` to "Pause Game Menu" and `Select` to
//! "Toggle In-Game Minimap / HUD". This module owns the drawing and the
//! `psx-pad` adapter; the input state machine lives in [`pause_input`] so it can
//! be exercised on the host by `tools/test_ui`.

use crate::ui::font::draw_text;
pub use crate::ui::pause_input::PauseMenu;
use crate::ui::pause_input::{PauseFrame, PauseInput};
use psx_gpu as gpu;
use psx_pad::{button, PadState};

pub use crate::ui::pause_input::PauseChoice;

impl PauseMenu {
    /// Extracts the pause-relevant buttons from a polled pad.
    ///
    /// Public so the race loop can hand the same snapshot to
    /// [`PauseMenu::sync_edges`] after a transition that skipped frames.
    pub fn input_from_pad(pad: &PadState) -> PauseInput {
        let b = pad.buttons;
        PauseInput {
            start: b.is_held(button::START),
            select: b.is_held(button::SELECT),
            cross: b.is_held(button::CROSS),
            circle: b.is_held(button::CIRCLE),
            up: b.is_held(button::UP),
            down: b.is_held(button::DOWN),
        }
    }

    /// Reads this frame's pause-relevant buttons off the pad and runs the state
    /// machine.
    pub fn update_from_pad(&mut self, pad: &PadState) -> PauseFrame {
        self.update(Self::input_from_pad(pad))
    }

    /// Renders the dimmed pause panel.
    pub fn render(&self) {
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
