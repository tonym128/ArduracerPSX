//! Graphics settings screen: circuit texture format and framerate.
//!
//! Rendering only. All input logic lives in `settings_input.rs` so
//! `tools/test_ui` can drive it without a GPU.

use crate::gpu::tracktex::{Framerate, TextureFormat};
use crate::ui::font::draw_text;
use crate::ui::settings_input::{
    SettingsChoice, SettingsInput, SettingsMenu, SettingsValues, ROW_COUNT,
};
use psx_gpu as gpu;

/// Maps the persisted integers onto the enums the renderer uses.
pub fn values_to_state(v: SettingsValues) -> (TextureFormat, Framerate) {
    (
        TextureFormat::from_saved(v.texture_format),
        Framerate::from_saved(v.framerate),
    )
}

/// Maps the enums back into the persisted integers.
pub fn state_to_values(f: TextureFormat, r: Framerate, camera_view: u8) -> SettingsValues {
    SettingsValues {
        texture_format: f.to_saved(),
        framerate: r.to_saved(),
        camera_view,
    }
}

pub struct SettingsScreen {
    menu: SettingsMenu,
}

impl SettingsScreen {
    pub const fn new(values: SettingsValues) -> Self {
        Self {
            menu: SettingsMenu::new(values),
        }
    }

    /// Advances the screen by one frame and reports what the caller must do.
    pub fn update(&mut self, input: SettingsInput) -> SettingsChoice {
        self.menu.update(input)
    }

    /// Values currently on screen.
    pub fn values(&self) -> SettingsValues {
        self.menu.values()
    }

    /// Replaces the values on screen without reporting a change, used to reseed
    /// the screen from the settings actually in force on entry.
    pub fn set_values(&mut self, values: SettingsValues) {
        self.menu.set_values(values);
    }

    /// Which row is highlighted.
    pub fn selected_row(&self) -> u8 {
        self.menu.selected_row
    }

    /// Clears the edge baseline on entry.
    pub fn arm_for_entry(&mut self) {
        self.menu.arm_for_entry();
    }

    pub fn render(&self) {
        let (fmt, fps) = values_to_state(self.values());
        let sel = self.selected_row();

        gpu::draw_rect_flat(60, 48, 200, 142, 8, 10, 14);
        gpu::draw_rect_flat(62, 50, 196, 138, 16, 20, 30);
        gpu::draw_rect_flat(64, 52, 192, 2, 220, 40, 60);
        draw_text(112, 56, "GRAPHICS", (200, 210, 235), 1);

        let view_label = match self.values().camera_view {
            0 => "BEHIND",
            1 => "OVERHEAD",
            _ => "ROTATING",
        };

        // Three rows, value between `<` and `>`.
        let rows: [(&str, &str, u16); ROW_COUNT as usize] = [
            ("TEXTURE", fmt.label(), 84),
            ("FRAMERATE", fps.label(), 108),
            ("VIEW", view_label, 132),
        ];
        for (i, (label, value, y)) in rows.iter().enumerate() {
            let selected = i as u8 == sel;
            let color = if selected {
                (255, 220, 110)
            } else {
                (150, 160, 185)
            };
            draw_text(72, *y, label, color, 1);
            if selected {
                draw_text(196, *y, "<", color, 1);
                draw_text(232, *y, ">", color, 1);
            }
            draw_text(204, *y, value, color, 1);
        }

        draw_text(
            72,
            160,
            "UP/DOWN ROW   LEFT/RIGHT CHANGE",
            (120, 135, 160),
            1,
        );
        draw_text(72, 172, "CROSS: BACK", (120, 135, 160), 1);
    }
}
