//! Title Screen with 90s Arcade Presentation.
//!
//! Features animated retro speed stripes, bold neon typography, and a pulsing
//! PRESS START prompt.

use crate::ui::font::draw_text;
use psx_gpu as gpu;
use psx_pad::{button, PadState};

pub struct TitleScreen {
    pub timer: u32,
}

impl TitleScreen {
    pub const fn new() -> Self {
        TitleScreen { timer: 0 }
    }

    /// Updates title screen state, returning true if player presses START or CROSS.
    pub fn update(&mut self, pad: &PadState) -> bool {
        self.timer = self.timer.wrapping_add(1);
        pad.buttons.is_held(button::START) || pad.buttons.is_held(button::CROSS)
    }

    /// Renders title screen elements.
    pub fn render(&self, draw_y: i16) {
        let base_y = draw_y as u16;

        // 1. Dark synthwave gradient background stripes
        for line in 0..12 {
            let offset = ((self.timer * 2 + (line as u32) * 20) % 240) as u16;
            let shade = (30 + line * 3) as u8;
            gpu::fill_rect(0, base_y + offset, 320, 4, 15, shade, shade + 30);
        }

        // 2. Main Title Banner Shadow & Header
        gpu::fill_rect(44, base_y + 44, 232, 42, 10, 10, 20);
        gpu::fill_rect(40, base_y + 40, 240, 40, 220, 25, 45); // Crimson Red
        gpu::fill_rect(42, base_y + 42, 236, 36, 30, 30, 40);

        // Bold Title text: "ARDURACER PSX" (2x scale)
        draw_text(60, base_y + 50, "ARDURACER PSX", (255, 230, 0), 2);

        // Subtitle: "HIGH-OCTANE OVERHEAD ARCADE RACING"
        draw_text(
            52,
            base_y + 92,
            "HIGH-OCTANE OVERHEAD RACING",
            (0, 220, 255),
            1,
        );

        // 3. Pulsing "PRESS START TO RACE" (blinks every 30 frames)
        if (self.timer / 30) % 2 == 0 {
            draw_text(88, base_y + 160, "PRESS START TO RACE", (255, 255, 255), 1);
        }

        // 4. Copyright & build banner
        draw_text(
            68,
            base_y + 215,
            "(C) 2026 ARDURACER TEAM & PSOXIDE",
            (120, 120, 140),
            1,
        );
    }
}
