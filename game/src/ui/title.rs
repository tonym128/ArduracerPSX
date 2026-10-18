//! PS1 start screen with the cooked ArduracerPSX logo and an edge-triggered
//! PRESS START prompt.

use crate::ui::font::draw_text;
use crate::ui::title_background;
pub use crate::ui::title_input::TitleInput;
use psx_gpu as gpu;
use psx_gpu::material::BlendMode;
use psx_pad::{button, PadState};

pub struct TitleScreen {
    pub input: TitleInput,
    white_fade_frames: u8,
}

impl Default for TitleScreen {
    fn default() -> Self {
        Self::new()
    }
}

impl TitleScreen {
    pub fn new() -> Self {
        title_background::upload();
        TitleScreen {
            input: TitleInput::new(),
            white_fade_frames: 0,
        }
    }

    pub fn timer(&self) -> u32 {
        self.input.timer
    }

    /// Updates title screen state, returning true on fresh edge-triggered START or CROSS.
    pub fn update(&mut self, pad: &PadState) -> bool {
        self.white_fade_frames = self.white_fade_frames.saturating_sub(1);
        let start_held = pad.buttons.is_held(button::START);
        let cross_held = pad.buttons.is_held(button::CROSS);
        self.input.update(start_held, cross_held)
    }

    /// Starts the white flash/fade sequence used when an intro movie ends.
    pub fn start_white_fade(&mut self) {
        self.white_fade_frames = 8;
    }

    /// Renders title screen elements.
    pub fn render(&self) {
        let timer = self.input.timer;
        title_background::draw();

        // Pulsing "PRESS START TO RACE" (blinks every 30 frames)
        if (timer / 30).is_multiple_of(2) {
            draw_text(88, 160, "PRESS START TO RACE", (255, 255, 255), 1);
        }

        // 4. Copyright & build banner
        draw_text(
            68,
            215,
            "(C) 2026 ARDURACER TEAM & PSOXIDE",
            (120, 120, 140),
            1,
        );

        // Repeated quarter-add white overlays provide a stepped fade on the
        // PS1 GPU. The base scene is redrawn each frame, so these layers do not
        // accumulate differently across the two framebuffers.
        let white_layers = self.white_fade_frames.div_ceil(2);
        for _ in 0..white_layers {
            gpu::draw_rect_flat_blended(0, 0, 320, 240, 255, 255, 255, BlendMode::AddQuarter);
        }
    }
}
