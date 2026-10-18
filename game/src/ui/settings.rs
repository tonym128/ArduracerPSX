//! Graphics settings screen: circuit texture format, framerate, view and
//! CD-audio.
//!
//! Rendering only. All input logic lives in `settings_input.rs` so
//! `tools/test_ui` can drive it without a GPU.

use crate::gpu::tracktex::{Framerate, TextureFormat};
use crate::ui::font::draw_text;
use crate::ui::settings_input::{
    SettingsChoice, SettingsInput, SettingsMenu, SettingsValues, ROW_COUNT,
};
use arduracer_core::save::CdAudioMode;
use psx_gpu as gpu;

/// Maps the persisted integers onto the enums the renderer uses.
pub fn values_to_state(v: SettingsValues) -> (TextureFormat, Framerate) {
    (
        TextureFormat::from_saved(v.texture_format),
        Framerate::from_saved(v.framerate),
    )
}

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

/// Maps the enums back into the persisted integers.
pub fn state_to_values(
    f: TextureFormat,
    r: Framerate,
    camera_view: u8,
    cd_audio: u8,
    debug_metrics: u8,
    input_profile: u8,
    split_mode: u8,
) -> SettingsValues {
    SettingsValues {
        texture_format: f.to_saved(),
        framerate: r.to_saved(),
        camera_view,
        cd_audio,
        debug_metrics,
        input_profile,
        split_mode,
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

        gpu::draw_rect_flat(60, 34, 200, 170, 8, 10, 14);
        gpu::draw_rect_flat(62, 36, 196, 166, 16, 20, 30);
        gpu::draw_rect_flat(64, 38, 192, 2, 220, 40, 60);
        draw_text(112, 42, "SETTINGS", (200, 210, 235), 1);

        let view_label = match self.values().camera_view {
            0 => "BEHIND",
            1 => "OVERHEAD",
            _ => "ROTATING",
        };

        // Seven rows, value between `<` and `>`. Spacing is 15 px.
        let rows: [(&str, &str, u16); ROW_COUNT as usize] = [
            ("TEXTURE", fmt.label(), 56),
            ("FRAMERATE", fps.label(), 71),
            ("VIEW", view_label, 86),
            ("CD AUDIO", cd_audio_label(self.values().cd_audio), 101),
            (
                "METRICS",
                debug_metrics_label(self.values().debug_metrics),
                116,
            ),
            (
                "CONTROLS",
                controls_label(self.values().input_profile),
                131,
            ),
            (
                "2P SPLIT",
                split_label(self.values().split_mode),
                146,
            ),
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
            160,
            "UP/DOWN ROW   LEFT/RIGHT CHANGE",
            (120, 135, 160),
            1,
        );
        draw_text(72, 174, "CROSS: BACK", (120, 135, 160), 1);
    }
}

/// The CD-audio row's value text.
///
/// Spelled ON/OFF rather than ENABLED/DISABLED so the label and its value fit
/// between the `<` and `>` arrows without overlapping them.
impl SettingsValues {
    /// The decoded disc-audio mode behind the raw menu byte.
    pub const fn cd_audio_mode(&self) -> CdAudioMode {
        CdAudioMode::from_saved(self.cd_audio)
    }
}

/// How the setting names itself in the menu.
///
/// Three states, not two, and the middle one is named rather than "ON" because
/// the choice is no longer a flag: Redbook CD-DA holds the whole drive for as
/// long as it plays, and XA costs about 58% of that. See
/// [`arduracer_core::save::CdAudioMode`].
pub const fn cd_audio_label(v: u8) -> &'static str {
    match v {
        0 => "OFF",
        2 => "XA",
        _ => "CD-DA",
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
