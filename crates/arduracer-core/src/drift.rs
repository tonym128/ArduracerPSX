//! Drift physics, slip angle dynamics, and drift-boost charging.
//!
//! Provides the arcade driving feel where sustained high-speed cornering
//! breaks lateral traction into a controlled slide, rewarding skilled
//! counter-steering with a mini-turbo boost on drift exit.

use crate::math::{self, Fixed};
use crate::tuning::CarTuning;

/// Maximum duration (in 60Hz ticks) of an oil-slick or oversteer spin-out (~1 second).
pub const SPINOUT_TICKS: u16 = 60;
/// Ticks of sustained drift needed to earn Level 1 Mini-Turbo (Blue sparks).
pub const DRIFT_BOOST_LEVEL1_TICKS: u16 = 45;
/// Ticks of sustained drift needed to earn Level 2 Super-Turbo (Orange sparks).
pub const DRIFT_BOOST_LEVEL2_TICKS: u16 = 90;

/// Drift and traction state of the vehicle.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum DriftState {
    /// Full grip, normal rolling traction.
    #[default]
    Grip,
    /// Vehicle is actively sliding sideways.
    Drifting {
        /// Number of consecutive 60Hz ticks the drift has been maintained.
        ticks: u16,
        /// Lateral slip angle relative to vehicle heading (-2048..2048).
        slip_angle: i16,
        /// Direction of drift: -1 for left slide, +1 for right slide.
        direction: i8,
        /// Earned boost tier (0: none, 1: mini-turbo, 2: super-turbo).
        boost_tier: u8,
    },
    /// Vehicle has lost complete control and is spinning uncontrollably.
    SpinOut { remaining_ticks: u16 },
}

impl DriftState {
    /// Whether the car is currently sideways in a drift.
    pub fn is_drifting(&self) -> bool {
        matches!(self, DriftState::Drifting { .. })
    }

    /// Whether the car is spinning out from an oil slick or crash.
    pub fn is_spinning(&self) -> bool {
        matches!(self, DriftState::SpinOut { .. })
    }

    /// Current boost tier available upon releasing drift.
    pub fn boost_tier(&self) -> u8 {
        match self {
            DriftState::Drifting { boost_tier, .. } => *boost_tier,
            _ => 0,
        }
    }

    /// Initiates a controlled drift in a given direction (-1 for left, +1 for right).
    pub fn initiate_drift(&mut self, direction: i8) {
        if !self.is_spinning() {
            *self = DriftState::Drifting {
                ticks: 0,
                slip_angle: (direction as i16) * 256,
                direction,
                boost_tier: 0,
            };
        }
    }

    /// Triggers an uncontrolled spin-out (e.g. hitting an oil slick or wall).
    pub fn trigger_spinout(&mut self) {
        *self = DriftState::SpinOut {
            remaining_ticks: SPINOUT_TICKS,
        };
    }

    /// Advances the drift simulation by one 60Hz tick.
    /// Returns any released boost impulse when exiting a successful drift.
    pub fn tick(&mut self, steer_input: Fixed, car_speed: Fixed, tuning: &CarTuning) -> Fixed {
        match *self {
            DriftState::Grip => Fixed::ZERO,
            DriftState::SpinOut {
                ref mut remaining_ticks,
            } => {
                if *remaining_ticks > 0 {
                    *remaining_ticks -= 1;
                }
                if *remaining_ticks == 0 {
                    *self = DriftState::Grip;
                }
                Fixed::ZERO
            }
            DriftState::Drifting {
                ref mut ticks,
                ref mut slip_angle,
                direction,
                ref mut boost_tier,
            } => {
                // Low speed cancels drift
                if car_speed < Fixed::from_raw(2000) {
                    *self = DriftState::Grip;
                    return Fixed::ZERO;
                }

                *ticks = ticks.saturating_add(1);

                // Counter-steering dynamics:
                // If sliding right (direction > 0) and steering left (steer_input < 0), player is counter-steering!
                let is_counter_steering = (direction > 0 && steer_input < Fixed::ZERO)
                    || (direction < 0 && steer_input > Fixed::ZERO);

                // Modulate slip angle based on counter-steering and tuning
                let stability_scale = CarTuning::scale_factor(tuning.drift_stability).raw();
                if is_counter_steering {
                    // Counter-steering tightens the drift and stabilizes the angle
                    let recovery = ((32 * stability_scale) >> math::FP_SHIFT) as i16;
                    if direction > 0 {
                        *slip_angle = (*slip_angle - recovery).max(128);
                    } else {
                        *slip_angle = (*slip_angle + recovery).min(-128);
                    }
                } else if steer_input.raw() != 0 {
                    // Hard cornering deepens the drift angle
                    let deepening = 24;
                    if direction > 0 {
                        *slip_angle = (*slip_angle + deepening).min(640);
                    } else {
                        *slip_angle = (*slip_angle - deepening).max(-640);
                    }
                }

                // Over-drifting spin-out threshold
                if slip_angle.abs() >= 640 {
                    self.trigger_spinout();
                    return Fixed::ZERO;
                }

                // Boost charging tiers based on drift duration
                if *ticks >= DRIFT_BOOST_LEVEL2_TICKS {
                    *boost_tier = 2; // Super-Turbo
                } else if *ticks >= DRIFT_BOOST_LEVEL1_TICKS {
                    *boost_tier = 1; // Mini-Turbo
                }

                Fixed::ZERO
            }
        }
    }

    /// Releases the drift and returns the resulting forward boost impulse.
    pub fn release_drift(&mut self) -> Fixed {
        if let DriftState::Drifting { boost_tier, .. } = *self {
            let boost_impulse = match boost_tier {
                2 => Fixed::from_raw(4000), // Super Turbo
                1 => Fixed::from_raw(2000), // Mini Turbo
                _ => Fixed::ZERO,
            };
            *self = DriftState::Grip;
            boost_impulse
        } else {
            Fixed::ZERO
        }
    }
}
