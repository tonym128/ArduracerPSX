//! Car Tuning Garage UI Screen.
//!
//! Provides interactive 5-slider tuning allocating a 20-point performance budget
//! across Top Speed, Acceleration, Handling/Grip, Drift Stability, and Gearing,
//! complete with real-time stats and visual vehicle preview.

use crate::ui::font::{draw_char, draw_text};
use arduracer_core::tuning::CarTuning;
use psx_gpu as gpu;
use psx_pad::{button, PadState};

pub struct TuningScreen {
    pub tuning: CarTuning,
    pub selected_slider: u8, // 0..4
    pub prev_up: bool,
    pub prev_down: bool,
    pub prev_left: bool,
    pub prev_right: bool,
    pub preview_angle: u16,
}

impl TuningScreen {
    pub fn new(initial: CarTuning) -> Self {
        TuningScreen {
            tuning: initial,
            selected_slider: 0,
            prev_up: false,
            prev_down: false,
            prev_left: false,
            prev_right: false,
            preview_angle: 0,
        }
    }

    /// Updates tuning navigation and adjustments. Returns true when player presses CIRCLE/START to exit.
    pub fn update(&mut self, pad: &PadState) -> bool {
        self.preview_angle = (self.preview_angle + 32) % 4096;

        let b = pad.buttons;
        let up = b.is_held(button::UP);
        let down = b.is_held(button::DOWN);
        let left = b.is_held(button::LEFT);
        let right = b.is_held(button::RIGHT);

        // Slider selection
        if up && !self.prev_up {
            if self.selected_slider > 0 {
                self.selected_slider -= 1;
            } else {
                self.selected_slider = 4;
            }
        } else if down && !self.prev_down {
            if self.selected_slider < 4 {
                self.selected_slider += 1;
            } else {
                self.selected_slider = 0;
            }
        }

        // Point adjustment
        let total_points = self.tuning.top_speed
            + self.tuning.acceleration
            + self.tuning.handling
            + self.tuning.drift_stability
            + self.tuning.gearing;

        if right && !self.prev_right && total_points < 20 {
            match self.selected_slider {
                0 => self.tuning.top_speed = (self.tuning.top_speed + 1).min(10),
                1 => self.tuning.acceleration = (self.tuning.acceleration + 1).min(10),
                2 => self.tuning.handling = (self.tuning.handling + 1).min(10),
                3 => self.tuning.drift_stability = (self.tuning.drift_stability + 1).min(10),
                4 => self.tuning.gearing = (self.tuning.gearing + 1).min(10),
                _ => {}
            }
        } else if left && !self.prev_left {
            match self.selected_slider {
                0 => self.tuning.top_speed = self.tuning.top_speed.saturating_sub(1),
                1 => self.tuning.acceleration = self.tuning.acceleration.saturating_sub(1),
                2 => self.tuning.handling = self.tuning.handling.saturating_sub(1),
                3 => self.tuning.drift_stability = self.tuning.drift_stability.saturating_sub(1),
                4 => self.tuning.gearing = self.tuning.gearing.saturating_sub(1),
                _ => {}
            }
        }

        // Presets via TRIANGLE
        if b.is_held(button::TRIANGLE) {
            self.tuning = CarTuning::default();
        }

        self.prev_up = up;
        self.prev_down = down;
        self.prev_left = left;
        self.prev_right = right;

        b.is_held(button::CIRCLE) || b.is_held(button::START)
    }

    /// Renders the tuning garage interface.
    pub fn render(&self, draw_y: i16) {
        let base_y = draw_y as u16;

        // Background
        gpu::fill_rect(10, base_y + 10, 300, 220, 18, 22, 28);
        gpu::fill_rect(12, base_y + 12, 296, 216, 25, 30, 38);

        // Header
        draw_text(90, base_y + 18, "TUNING GARAGE", (255, 220, 0), 2);
        gpu::fill_rect(30, base_y + 36, 260, 2, 220, 40, 60);

        // Points budget summary
        let total_points = self.tuning.top_speed
            + self.tuning.acceleration
            + self.tuning.handling
            + self.tuning.drift_stability
            + self.tuning.gearing;
        let remaining = 20u8.saturating_sub(total_points);

        draw_text(24, base_y + 46, "POINTS REMAINING: ", (0, 220, 255), 1);
        let rem_char = b'0' + remaining.min(9);
        draw_char(132, base_y + 46, rem_char, (255, 255, 255), 1);

        // Sliders
        let sliders = [
            ("TOP SPEED", self.tuning.top_speed),
            ("ACCELERATION", self.tuning.acceleration),
            ("HANDLING", self.tuning.handling),
            ("DRIFT GRIP", self.tuning.drift_stability),
            ("GEARING RATIO", self.tuning.gearing),
        ];

        for (idx, (name, val)) in sliders.iter().enumerate() {
            let y = base_y + 64 + (idx as u16) * 26;
            let is_sel = (idx as u8) == self.selected_slider;

            if is_sel {
                gpu::fill_rect(18, y - 2, 180, 22, 220, 30, 50);
            }

            draw_text(
                22,
                y + 2,
                name,
                if is_sel {
                    (255, 255, 255)
                } else {
                    (180, 190, 200)
                },
                1,
            );

            // Slider gauge: 10 notches
            let gauge_x = 105u16;
            let gauge_y = y + 4;
            gpu::fill_rect(gauge_x, gauge_y, 80, 10, 20, 20, 30);
            let fill_w = (*val as u16) * 8;
            if fill_w > 0 {
                gpu::fill_rect(gauge_x + 1, gauge_y + 1, fill_w - 1, 8, 255, 200, 0);
            }
        }

        // Vehicle Preview Turntable (Top Right panel)
        gpu::fill_rect(210, base_y + 64, 86, 86, 15, 18, 24);
        gpu::fill_rect(212, base_y + 66, 82, 82, 35, 40, 50);
        draw_text(224, base_y + 70, "CAR 01", (0, 220, 255), 1);

        // Simple rotating preview car chassis
        let pv_cx = 253u16;
        let pv_cy = base_y + 115;
        gpu::fill_rect(pv_cx - 8, pv_cy - 14, 16, 28, 220, 25, 45); // Red body
        gpu::fill_rect(pv_cx - 4, pv_cy - 6, 8, 12, 30, 45, 65); // Windshield
        gpu::fill_rect(pv_cx - 3, pv_cy - 12, 6, 3, 255, 240, 150); // Lights

        // Instructions Footer
        draw_text(
            24,
            base_y + 200,
            "L/R: TUNE   TRI: RESET   CIR/STA: EXIT",
            (130, 150, 170),
            1,
        );
    }
}
