//! In-Race Pause Overlay (DualShock `Start`).
//!
//! GAME.md §7 assigns `Start` to "Pause Game Menu" and `Select` to
//! "Toggle In-Game Minimap / HUD". This module owns the drawing and the
//! `psx-pad` adapter; the input state machine lives in [`pause_input`] so it can
//! be exercised on the host by `tools/test_ui`.

use crate::ui::font::draw_text;
pub use crate::ui::pause_input::PauseMenu;
use crate::ui::pause_input::{PauseFrame, PauseInput, SettingsValues};
use crate::ui::settings::values_to_state;
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
            left: b.is_held(button::LEFT),
            right: b.is_held(button::RIGHT),
        }
    }

    /// Reads this frame's pause-relevant buttons off the pad and runs the state
    /// machine.
    pub fn update_from_pad(&mut self, pad: &PadState) -> PauseFrame {
        self.update(Self::input_from_pad(pad))
    }

    /// Reads this frame's buttons and runs the machine against `values`.
    pub fn update_from_pad_with_values(
        &mut self,
        pad: &PadState,
        values: SettingsValues,
    ) -> PauseFrame {
        self.update_with_values(Self::input_from_pad(pad), values)
    }

    /// Renders the dimmed pause panel.
    pub fn render(&self) {
        // Panel grown to hold nine rows plus the footer hint.
        gpu::draw_rect_flat(60, 28, 200, 186, 8, 10, 14);
        gpu::draw_rect_flat(62, 30, 196, 182, 16, 20, 30);
        gpu::draw_rect_flat(64, 32, 192, 178, 26, 30, 42);
        gpu::draw_rect_flat(64, 32, 192, 2, 220, 40, 60);

        draw_text(102, 38, "PAUSED", (255, 220, 0), 2);

        // The settings rows carry their value inline between < and >; the
        // action rows are plain labels. Values come from the menu's
        // working copy, so an in-progress edit is visible before it is applied.
        let (fmt, fps) = values_to_state(self.values());
        let view_label = match self.values().camera_view {
            0 => "BEHIND",
            1 => "OVERHEAD",
            _ => "ROTATING",
        };
        let rows: [(&str, Option<&str>); 9] = [
            ("RESUME", None),
            ("RESTART RACE", None),
            ("TEXTURE", Some(fmt.label())),
            ("FRAMERATE", Some(fps.label())),
            ("VIEW", Some(view_label)),
            (
                "CD AUDIO",
                Some(crate::ui::settings::cd_audio_label(self.values().cd_audio)),
            ),
            (
                "METRICS",
                Some(crate::ui::settings::debug_metrics_label(
                    self.values().debug_metrics,
                )),
            ),
            (
                "CONTROLS",
                Some(crate::ui::settings::controls_label(
                    self.values().input_profile,
                )),
            ),
            ("QUIT TO MENU", None),
        ];
        for (idx, (label, value)) in rows.iter().enumerate() {
            // 14 px vertical spacing so nine rows sit cleanly inside the frame.
            let y = 60 + (idx as u16) * 14;
            let selected = (idx as u8) == self.selected_idx;
            let color = if selected {
                (255, 255, 255)
            } else {
                (150, 165, 185)
            };
            if selected {
                gpu::draw_rect_flat(74, (y - 3) as i16, 172, 14, 200, 30, 50);
                draw_text(82, y, ">", (255, 240, 0), 1);
            }
            draw_text(92, y, label, color, 1);
            if let Some(v) = value {
                if selected {
                    draw_text(168, y, "<", (255, 240, 0), 1);
                    draw_text(238, y, ">", (255, 240, 0), 1);
                }
                draw_text(176, y, v, color, 1);
            }
        }

        draw_text(
            70,
            192,
            "L/R CHANGE   CROSS SELECT   START RESUME",
            (110, 130, 155),
            1,
        );
    }
}
