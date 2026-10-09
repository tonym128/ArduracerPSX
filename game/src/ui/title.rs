//! Title Screen with 90s Arcade Presentation for PlayStation 1.
//!
//! Features animated retro synthwave speed stripes, bold neon typography,
//! and an edge-triggered pulsing PRESS START prompt.

use crate::ui::font::draw_text;
pub use crate::ui::title_input::TitleInput;
use psx_gpu as gpu;
use psx_pad::{button, PadState};

pub struct TitleScreen {
    pub input: TitleInput,
}

impl Default for TitleScreen {
    fn default() -> Self {
        Self::new()
    }
}

impl TitleScreen {
    pub const fn new() -> Self {
        TitleScreen {
            input: TitleInput::new(),
        }
    }

    pub fn timer(&self) -> u32 {
        self.input.timer
    }

    /// Updates title screen state, returning true on fresh edge-triggered START or CROSS.
    pub fn update(&mut self, pad: &PadState) -> bool {
        let start_held = pad.buttons.is_held(button::START);
        let cross_held = pad.buttons.is_held(button::CROSS);
        self.input.update(start_held, cross_held)
    }

    /// Renders title screen elements.
    pub fn render(&self) {
        let timer = self.input.timer;
        // 1. Dark synthwave gradient background stripes
        for line in 0..12i16 {
            let offset = ((timer * 2 + (line as u32) * 20) % 240) as i16;
            let shade = (30 + line * 3) as u8;
            gpu::draw_rect_flat(0, offset, 320, 4, 15, shade, shade + 30);
        }

        // 2. Main Title Banner Shadow & Header
        gpu::draw_rect_flat(44, 44, 232, 42, 10, 10, 20);
        gpu::draw_rect_flat(40, 40, 240, 40, 220, 25, 45); // Crimson Red
        gpu::draw_rect_flat(42, 42, 236, 36, 30, 30, 40);

        // Bold Title text: "ARDURACER PSX" (2x scale)
        draw_text(60, 50, "ARDURACER PSX", (255, 230, 0), 2);

        // Subtitle: "HIGH-OCTANE OVERHEAD RACING"
        draw_text(52, 92, "HIGH-OCTANE OVERHEAD RACING", (0, 220, 255), 1);

        // 3. Pulsing "PRESS START TO RACE" (blinks every 30 frames)
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
    }
}
