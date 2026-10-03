//! Car Tuning configuration and parameter scaling.
//!
//! Preserves the classic ArduRacer FX 10% point-allocation mechanic,
//! extending it to 5 tuning axes on PSX hardware.

use crate::math::Fixed;

/// The maximum value for any individual tuning slider.
pub const MAX_SLIDER: u8 = 7;
/// The minimum value for any individual tuning slider.
pub const MIN_SLIDER: u8 = 1;
/// Default value for each tuning slider.
pub const DEFAULT_SLIDER: u8 = 4;
/// Total points to allocate across all 5 sliders.
pub const TOTAL_POINTS: u8 = 20;

/// Tuning settings for a race car.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct CarTuning {
    pub top_speed: u8,
    pub acceleration: u8,
    pub handling: u8,
    pub drift_stability: u8,
    pub gearing: u8,
}

impl Default for CarTuning {
    fn default() -> Self {
        CarTuning {
            top_speed: DEFAULT_SLIDER,
            acceleration: DEFAULT_SLIDER,
            handling: DEFAULT_SLIDER,
            drift_stability: DEFAULT_SLIDER,
            gearing: DEFAULT_SLIDER,
        }
    }
}

impl CarTuning {
    /// Checks if the allocated points sum to the required total.
    pub fn is_valid(&self) -> bool {
        self.top_speed >= MIN_SLIDER
            && self.top_speed <= MAX_SLIDER
            && self.acceleration >= MIN_SLIDER
            && self.acceleration <= MAX_SLIDER
            && self.handling >= MIN_SLIDER
            && self.handling <= MAX_SLIDER
            && self.drift_stability >= MIN_SLIDER
            && self.drift_stability <= MAX_SLIDER
            && self.gearing >= MIN_SLIDER
            && self.gearing <= MAX_SLIDER
            && (self.total_points() == TOTAL_POINTS)
    }

    /// Sum of all allocated tuning points.
    pub fn total_points(&self) -> u8 {
        self.top_speed + self.acceleration + self.handling + self.drift_stability + self.gearing
    }

    /// Computes the fixed-point scaling factor for a slider value relative to default (4).
    /// Value 4 = 1.0 (4096).
    /// Each step above/below changes value by 10% (410 in Q20.12).
    pub fn scale_factor(slider_value: u8) -> Fixed {
        let diff = (slider_value as i32) - (DEFAULT_SLIDER as i32);
        // 10% of 4096 is 409.6 -> 410
        let delta = diff * 410;
        Fixed::from_raw(crate::math::FP_ONE + delta)
    }

    /// Scaled top speed in world units per second.
    pub fn scaled_top_speed(&self, base_speed: Fixed) -> Fixed {
        base_speed * Self::scale_factor(self.top_speed)
    }

    /// Scaled acceleration.
    pub fn scaled_acceleration(&self, base_accel: Fixed) -> Fixed {
        base_accel * Self::scale_factor(self.acceleration)
    }

    /// Scaled handling/turn rate.
    pub fn scaled_turn_rate(&self, base_turn: u16) -> u16 {
        let factor = Self::scale_factor(self.handling).raw();
        ((base_turn as i64 * factor as i64) >> crate::math::FP_SHIFT) as u16
    }
}
