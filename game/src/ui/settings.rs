//! Settings screen: view, music, metrics, controls and 2P split.
//!
//! Rendering only. All input logic lives in `settings_input.rs` so
//! `tools/test_ui` can drive it without a GPU.

use crate::ui::font::draw_text;
use crate::ui::settings_input::{
    SettingsChoice, SettingsInput, SettingsMenu, SettingsValues, ROW_COUNT,
};
use psx_gpu as gpu;

pub fn controls_label(profile: u8) -> &'static str {
    match profile {
        1 => "TRIGGERS",
        2 => "DUAL STICK",
        _ => "CLASSIC",
    }
}

pub const fn split_label(mode: u8) -> &'static str {
    match mode {
        1 => "VERTICAL",
        2 => "DIAGONAL",
        _ => "HORIZONTAL",
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
        let sel = self.selected_row();

        gpu::draw_rect_flat(60, 34, 200, 144, 8, 10, 14);
        gpu::draw_rect_flat(62, 36, 196, 140, 16, 20, 30);
        gpu::draw_rect_flat(64, 38, 192, 2, 220, 40, 60);
        draw_text(112, 42, "SETTINGS", (200, 210, 235), 1);

        let view_label = match self.values().camera_view {
            0 => "BEHIND",
            1 => "OVERHEAD",
            _ => "ROTATING",
        };

        // Five rows, value between `<` and `>`. Spacing is 15 px.
        let rows: [(&str, &str, u16); ROW_COUNT as usize] = [
            ("VIEW", view_label, 56),
            ("MUSIC", music_label(self.values().music), 71),
            (
                "METRICS",
                debug_metrics_label(self.values().debug_metrics),
                86,
            ),
            ("CONTROLS", controls_label(self.values().input_profile), 101),
            ("2P SPLIT", split_label(self.values().split_mode), 116),
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
                draw_text(168, *y, "<", color, 1);
                draw_text(238, *y, ">", color, 1);
            }
            draw_text(176, *y, value, color, 1);
        }

        draw_text(
            72,
            134,
            "UP/DOWN ROW   LEFT/RIGHT CHANGE",
            (120, 135, 160),
            1,
        );
        draw_text(72, 148, "CROSS: BACK", (120, 135, 160), 1);
    }
}

/// The music row's value text.
pub const fn music_label(on: bool) -> &'static str {
    if on {
        "ON"
    } else {
        "OFF"
    }
}

/// How the debug metrics setting names itself in the menu.
pub const fn debug_metrics_label(v: u8) -> &'static str {
    match v {
        1 => "FPS",
        2 => "FPS GRAPH",
        3 => "ALL",
        _ => "OFF",
    }
}
