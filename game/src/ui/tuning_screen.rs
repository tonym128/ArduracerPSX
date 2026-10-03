//! Car Tuning Garage UI Screen for PlayStation 1.
//!
//! Provides interactive 5-slider tuning allocating a 20-point performance budget
//! across Top Speed, Acceleration, Handling/Grip, Drift Stability, and Gearing,
//! complete with real-time stats and a rotating 3D turntable vehicle preview.

use crate::ui::font::{draw_char, draw_text};
use arduracer_core::math;
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
    pub prev_exit: bool,
    pub prev_reset: bool,
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
            prev_exit: true, // Edge-trigger: must release before exiting
            prev_reset: false,
            preview_angle: 0,
        }
    }

    /// Updates tuning navigation and adjustments. Returns true when player presses CIRCLE/START to exit.
    pub fn update(&mut self, pad: &PadState) -> bool {
        self.preview_angle = (self.preview_angle + 24) % 4096;

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

        // Presets via TRIANGLE (edge-triggered)
        let reset_held = b.is_held(button::TRIANGLE);
        if reset_held && !self.prev_reset {
            self.tuning = CarTuning::default();
        }
        self.prev_reset = reset_held;

        self.prev_up = up;
        self.prev_down = down;
        self.prev_left = left;
        self.prev_right = right;

        let exit_held = b.is_held(button::CIRCLE) || b.is_held(button::START);
        let exited = exit_held && !self.prev_exit;
        self.prev_exit = exit_held;
        exited
    }

    /// Renders the tuning garage interface.
    pub fn render(&self, _draw_y: i16) {
        // Background
        gpu::draw_rect_flat(10, 10, 300, 220, 16, 20, 28);
        gpu::draw_rect_flat(12, 12, 296, 216, 25, 30, 38);

        // Header
        draw_text(90, 18, "TUNING GARAGE", (255, 220, 0), 2);
        gpu::draw_rect_flat(30, 36, 260, 2, 220, 40, 60);

        // Points budget summary
        let total_points = self.tuning.top_speed
            + self.tuning.acceleration
            + self.tuning.handling
            + self.tuning.drift_stability
            + self.tuning.gearing;
        let remaining = 20u8.saturating_sub(total_points);

        draw_text(24, 46, "POINTS REMAINING: ", (0, 220, 255), 1);
        let rem_char = b'0' + remaining.min(9);
        draw_char(132, 46, rem_char, (255, 255, 255), 1);

        // Sliders
        let sliders = [
            ("TOP SPEED", self.tuning.top_speed),
            ("ACCELERATION", self.tuning.acceleration),
            ("HANDLING", self.tuning.handling),
            ("DRIFT GRIP", self.tuning.drift_stability),
            ("GEARING RATIO", self.tuning.gearing),
        ];

        for (idx, (name, val)) in sliders.iter().enumerate() {
            let y = 64 + (idx as i16) * 26;
            let is_sel = (idx as u8) == self.selected_slider;

            if is_sel {
                gpu::draw_rect_flat(18, y - 2, 180, 22, 220, 30, 50);
            }

            draw_text(
                22,
                (y + 2) as u16,
                name,
                if is_sel {
                    (255, 255, 255)
                } else {
                    (180, 190, 200)
                },
                1,
            );

            // Slider gauge: 10 notches
            let gauge_x = 105i16;
            let gauge_y = y + 4;
            gpu::draw_rect_flat(gauge_x, gauge_y, 80, 10, 20, 20, 30);
            let fill_w = (*val as u16) * 8;
            if fill_w > 0 {
                gpu::draw_rect_flat(gauge_x + 1, gauge_y + 1, fill_w - 1, 8, 255, 200, 0);
            }
        }

        // Vehicle Preview Turntable (Top Right panel)
        gpu::draw_rect_flat(210, 64, 86, 86, 14, 16, 22);
        gpu::draw_rect_flat(212, 66, 82, 82, 30, 36, 48);
        draw_text(224, 70, "CAR 01", (0, 220, 255), 1);

        // Rotating preview car on turntable
        let pv_cx = 253i16;
        let pv_cy = 115i16;
        let cos_a = math::cos(self.preview_angle).raw() as i32;
        let sin_a = math::sin(self.preview_angle).raw() as i32;

        let tf = |u: i32, v: i32| -> (i16, i16) {
            let x = pv_cx as i32 + ((u * cos_a + v * sin_a) >> 12);
            let y = pv_cy as i32 + ((u * sin_a - v * cos_a) >> 12);
            (x as i16, y as i16)
        };

        // Rotating turntable shadow
        let p_sh = [tf(-6, 12), tf(6, 12), tf(-7, -12), tf(7, -12)];
        let p_sh_offset = [
            (p_sh[0].0 + 2, p_sh[0].1 + 2),
            (p_sh[1].0 + 2, p_sh[1].1 + 2),
            (p_sh[2].0 + 2, p_sh[2].1 + 2),
            (p_sh[3].0 + 2, p_sh[3].1 + 2),
        ];
        gpu::draw_quad_flat(p_sh_offset, 16, 20, 28);

        // Rotating car body
        let p_body = [tf(-6, 12), tf(6, 12), tf(-7, -12), tf(7, -12)];
        gpu::draw_quad_flat(p_body, 220, 25, 45);

        // Rotating windshield glass
        let p_glass = [tf(-4, 4), tf(4, 4), tf(-5, -4), tf(5, -4)];
        gpu::draw_quad_flat(p_glass, 25, 40, 60);

        // Headlights
        let hl_l = tf(-4, 11);
        let hl_r = tf(4, 11);
        gpu::draw_line_mono(hl_l.0, hl_l.1, hl_l.0 + 1, hl_l.1, 255, 240, 150);
        gpu::draw_line_mono(hl_r.0, hl_r.1, hl_r.0 + 1, hl_r.1, 255, 240, 150);

        // Instructions Footer
        draw_text(
            24,
            200,
            "L/R: TUNE  TRI: RESET  CIR/STA: SAVE & EXIT",
            (130, 150, 170),
            1,
        );
    }
}
