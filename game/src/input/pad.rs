//! PlayStation Controller Driver (Digital & DualShock Analog).
//!
//! Polls the SIO0 hardware, handles deadzone calibration, non-linear steering
//! response curves, progressive D-pad steering smoothing, and translates button
//! states into deterministic [`arduracer_core::VehicleInput`].

use crate::input::mapping::InputProfile;
use arduracer_core::{Fixed, VehicleInput};
use psx_pad::{button, poll_port1, PadMode, STICK_CENTER};

pub const STEERING_DEADZONE: i16 = 14; // ~11% deadband around stick center

pub struct ControllerDriver {
    pub profile: InputProfile,
    pub is_analog_mode: bool,
    /// Digital D-Pad steering accumulator for smooth ramp-up (-4096..+4096).
    pub dpad_steer_acc: i32,
    /// True while the configured nitro button is held.
    pub nitro_held: bool,
    /// True on the frame the recovery button goes down, set by [`Self::poll`]
    /// from the same pad sample that produced the vehicle input.
    pub respawn_pressed: bool,
    /// Button state from the previous poll, for recovery edge detection.
    prev_buttons: u16,
}

impl ControllerDriver {
    pub const fn new(profile: InputProfile) -> Self {
        ControllerDriver {
            profile,
            is_analog_mode: false,
            dpad_steer_acc: 0,
            nitro_held: false,
            respawn_pressed: false,
            prev_buttons: 0,
        }
    }

    /// The recovery button for the active layout.
    ///
    /// GAME.md §7 gives Triangle "Reset Car to Track", but Triangle is nitro in
    /// the classic layout, so recovery takes the shoulder button that layout
    /// leaves free. Kept per-profile for the same reason nitro is.
    fn recovery_button(profile: InputProfile) -> u16 {
        match profile {
            InputProfile::ClassicArcade => button::R1,
            InputProfile::ModernTriggers => button::L1,
            InputProfile::DualAnalog => button::L2,
        }
    }

    /// Polls controller hardware on port 1 and generates a VehicleInput frame.
    pub fn poll(&mut self) -> VehicleInput {
        let pad = poll_port1();
        self.is_analog_mode = pad.mode == PadMode::Analog;

        let mut throttle = Fixed::ZERO;
        let mut brake = Fixed::ZERO;
        let mut handbrake = false;

        let b = pad.buttons;

        // Recovery edge, taken from this same sample so it cannot disagree with
        // the vehicle input built below it.
        let prev = psx_pad::ButtonState::from_bits(self.prev_buttons);
        self.respawn_pressed = b.pressed_since(prev, Self::recovery_button(self.profile));
        self.prev_buttons = b.bits();

        // GAME.md §7: nitro lives on Triangle / R1 / L1 depending on layout.
        let nitro = match self.profile {
            InputProfile::ClassicArcade => b.is_held(button::TRIANGLE),
            InputProfile::ModernTriggers => b.is_held(button::R1) || b.is_held(button::R2),
            InputProfile::DualAnalog => b.is_held(button::L1),
        };
        self.nitro_held = nitro;

        match self.profile {
            InputProfile::ClassicArcade => {
                if b.is_held(button::CROSS) {
                    throttle = Fixed::ONE;
                }
                if b.is_held(button::SQUARE) {
                    brake = Fixed::ONE;
                }
                if b.is_held(button::CIRCLE) {
                    handbrake = true;
                }
            }
            InputProfile::ModernTriggers => {
                if b.is_held(button::R2) {
                    throttle = Fixed::ONE;
                }
                if b.is_held(button::L2) {
                    brake = Fixed::ONE;
                }
                if b.is_held(button::SQUARE) {
                    handbrake = true;
                }
            }
            InputProfile::DualAnalog => {
                if self.is_analog_mode {
                    // Right stick Y axis (0 = full up / throttle, 255 = full down / brake)
                    let ry = pad.sticks.right_y as i16 - STICK_CENTER as i16;
                    if ry < -STEERING_DEADZONE {
                        let mag = ((-ry - STEERING_DEADZONE) as i32 * 4096)
                            / (127 - STEERING_DEADZONE as i32);
                        throttle = Fixed::from_raw(mag.clamp(0, 4096));
                    } else if ry > STEERING_DEADZONE {
                        let mag = ((ry - STEERING_DEADZONE) as i32 * 4096)
                            / (127 - STEERING_DEADZONE as i32);
                        brake = Fixed::from_raw(mag.clamp(0, 4096));
                    }
                } else {
                    if b.is_held(button::CROSS) {
                        throttle = Fixed::ONE;
                    }
                    if b.is_held(button::SQUARE) {
                        brake = Fixed::ONE;
                    }
                }
                if b.is_held(button::R1) {
                    handbrake = true;
                }
            }
        }

        let steer = self.compute_steering(&pad);

        VehicleInput {
            throttle,
            brake,
            steer,
            handbrake,
            nitro,
        }
    }

    /// Computes steering with analog priority and progressive digital smoothing fallback.
    fn compute_steering(&mut self, pad: &psx_pad::PadState) -> Fixed {
        if self.is_analog_mode {
            let lx = pad.sticks.left_x as i16 - STICK_CENTER as i16;
            if lx.abs() > STEERING_DEADZONE {
                let sign = if lx > 0 { 1 } else { -1 };
                let mag = (lx.abs() - STEERING_DEADZONE) as i32;
                let max_range = (127 - STEERING_DEADZONE) as i32;
                // Non-linear cubic response curve: 40% linear + 60% cubic
                // Provides precision around center with full lock at maximum deflection
                let norm = (mag * 1024) / max_range; // 0..1024
                let norm_cubed = (norm * norm * norm) / (1024 * 1024);
                let curve = (norm * 40 + norm_cubed * 60) / 100;
                let raw_steer = (curve * 4096 * sign) / 1024;
                return Fixed::from_raw(raw_steer.clamp(-4096, 4096));
            }
        }

        // Digital D-Pad Steering: Smooth ramp-up over 8 frames (~130ms)
        let b = pad.buttons;
        let left = b.is_held(button::LEFT);
        let right = b.is_held(button::RIGHT);

        if left && !right {
            // Ramp toward -4096
            self.dpad_steer_acc = (self.dpad_steer_acc - 640).max(-4096);
        } else if right && !left {
            // Ramp toward +4096
            self.dpad_steer_acc = (self.dpad_steer_acc + 640).min(4096);
        } else {
            // Self-centering spring decay towards 0
            if self.dpad_steer_acc > 0 {
                self.dpad_steer_acc = (self.dpad_steer_acc - 1024).max(0);
            } else if self.dpad_steer_acc < 0 {
                self.dpad_steer_acc = (self.dpad_steer_acc + 1024).min(0);
            }
        }

        Fixed::from_raw(self.dpad_steer_acc)
    }
}
