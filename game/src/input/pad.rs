//! PlayStation Controller Driver (Digital & DualShock Analog).
//!
//! Polls the SIO0 hardware, handles deadzone calibration, non-linear steering
//! response curves, progressive D-pad steering smoothing, and translates button
//! states into deterministic [`arduracer_core::VehicleInput`].

use crate::input::mapping::InputProfile;
use arduracer_core::{Fixed, VehicleInput};
use psx_pad::{button, PadMode, PadState, STICK_CENTER};

pub const STEERING_DEADZONE: i16 = 14; // ~11% deadband around stick center

pub struct ControllerDriver {
    pub profile: InputProfile,
    pub is_analog_mode: bool,
    /// Digital D-Pad steering accumulator for smooth ramp-up (-4096..+4096).
    pub dpad_steer_acc: i32,
    /// True on the frame the recovery button goes down, set by [`Self::poll`]
    /// from the same pad sample that produced the vehicle input.
    pub respawn_pressed: bool,
    /// True when the recovery button is double-tapped within 20 frames (Standing Respawn).
    pub standing_respawn_pressed: bool,
    /// True on the frame the recovery button has been held for 24 frames (Instant Track Restart).
    pub instant_restart_triggered: bool,
    /// Button state from the previous poll, for recovery edge detection.
    prev_buttons: u16,
    /// Countdown window (in frames) for detecting a double-tap on L1.
    double_tap_window: u8,
    /// Consecutive frames L1 has been held down.
    hold_frames: u8,
}

impl ControllerDriver {
    pub const fn new(profile: InputProfile) -> Self {
        ControllerDriver {
            profile,
            is_analog_mode: false,
            dpad_steer_acc: 0,
            respawn_pressed: false,
            standing_respawn_pressed: false,
            instant_restart_triggered: false,
            prev_buttons: 0,
            double_tap_window: 0,
            hold_frames: 0,
        }
    }

    /// The recovery button for the active layout.
    ///
    /// Uniform across every layout: **L1**. GAME.md §7 still gives Triangle
    /// "Reset Car to Track", but Triangle is a face button that the classic
    /// layout spends on nitro, so recovery lives on the left bumper. L1 is held
    /// to nothing else in any profile, which is what makes it safe to bind it
    /// globally instead of hunting for whichever button each layout happens to
    /// leave spare.
    const fn recovery_button(_profile: InputProfile) -> u16 {
        button::L1
    }

    /// Generates a [`VehicleInput`] from a pad sample the caller already has.
    ///
    /// The frame loop polls the pad once for the UI and needs the *same* sample
    /// for the vehicle. Polling again inside the driver cost a second full SIO0
    /// transaction per frame and, worse, meant the UI snapshot and the
    /// `VehicleInput` came from different instants: a `Start` tap could open the
    /// pause menu without ever reaching the car (TASK-1214).
    pub fn poll_from(&mut self, pad: &PadState) -> VehicleInput {
        let pad = *pad;
        self.is_analog_mode = pad.mode == PadMode::Analog;

        let mut throttle = Fixed::ZERO;
        let mut brake = Fixed::ZERO;
        let mut handbrake = false;

        let b = pad.buttons;

        // Recovery edge, taken from this same sample so it cannot disagree with
        // the vehicle input built below it.
        let prev = psx_pad::ButtonState::from_bits(self.prev_buttons);
        let recovery_btn = Self::recovery_button(self.profile);
        let just_pressed = b.pressed_since(prev, recovery_btn);
        let is_held = b.is_held(recovery_btn);
        self.respawn_pressed = just_pressed;
        self.standing_respawn_pressed = just_pressed && self.double_tap_window > 0;
        if just_pressed {
            self.double_tap_window = 20;
            self.hold_frames = 1;
        } else {
            self.double_tap_window = self.double_tap_window.saturating_sub(1);
            if is_held {
                self.hold_frames = self.hold_frames.saturating_add(1);
            } else {
                self.hold_frames = 0;
            }
        }
        self.instant_restart_triggered = self.hold_frames == 24;
        self.prev_buttons = b.bits();

        // Nitro is **R1** in every layout, mirroring L1 for recovery.
        //
        // This also clears up two conflicts the per-layout bindings carried:
        // Modern Triggers had nitro on `R1 || R2` while R2 was also throttle, so
        // holding the accelerator fired nitro every frame; and Dual Analog had
        // nitro on L1, which recovery now owns.
        let nitro = b.is_held(button::R1);

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
                // R1 is nitro everywhere, so the dual-analog handbrake moves to
                // L2 -- which that layout no longer uses for recovery.
                if b.is_held(button::L2) {
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

        // Digital D-Pad Steering: Instant tap bite + progressive ramp-up while held
        // Enables precision rhythmic tapping for cornering (Micro Machines / Super Sprint feel)
        let b = pad.buttons;
        let left = b.is_held(button::LEFT);
        let right = b.is_held(button::RIGHT);

        const TAP_INITIAL_BITE: i32 = 1600; // ~39% steering lock on first tap frame
        const RAMP_RATE: i32 = 400; // Ramp to full 100% lock over ~6 frames if held
        const CENTER_DECAY: i32 = 1600; // Fast centering decay on release

        if left && !right {
            if self.dpad_steer_acc >= 0 {
                // Immediate crisp tap bite
                self.dpad_steer_acc = -TAP_INITIAL_BITE;
            } else {
                // Progressive ramp toward full lock
                self.dpad_steer_acc = (self.dpad_steer_acc - RAMP_RATE).max(-4096);
            }
        } else if right && !left {
            if self.dpad_steer_acc <= 0 {
                // Immediate crisp tap bite
                self.dpad_steer_acc = TAP_INITIAL_BITE;
            } else {
                // Progressive ramp toward full lock
                self.dpad_steer_acc = (self.dpad_steer_acc + RAMP_RATE).min(4096);
            }
        } else {
            // Rapid self-centering spring decay towards 0
            if self.dpad_steer_acc > 0 {
                self.dpad_steer_acc = (self.dpad_steer_acc - CENTER_DECAY).max(0);
            } else if self.dpad_steer_acc < 0 {
                self.dpad_steer_acc = (self.dpad_steer_acc + CENTER_DECAY).min(0);
            }
        }

        Fixed::from_raw(self.dpad_steer_acc)
    }
}
