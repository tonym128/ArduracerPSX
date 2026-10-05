//! Car Tuning configuration and parameter scaling.
//!
//! Preserves the classic ArduRacer FX 10% point-allocation mechanic,
//! extending it to 5 tuning axes on PSX hardware.

use crate::math::{Fixed, FP_ONE};

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

    /// How the final-drive ratio shapes engine thrust across the rev range.
    ///
    /// A short ratio (low `gearing`) is punchy low down and gutless at the top;
    /// a long ratio (high `gearing`) is the reverse. Modelled as a straight line
    /// through the midpoint of the speed range, scaled by how far `gearing` sits
    /// from the default:
    ///
    /// ```text
    /// multiplier(speed_fraction) = 1 + (ratio - 1) * (2 * speed_fraction - 1)
    /// ```
    ///
    /// where `ratio` is `scale_factor(gearing)`. At the default of 4, `ratio` is
    /// exactly 1.0, so the multiplier is exactly 1.0 at every speed and the
    /// default car drives precisely as it did before gearing did anything. That
    /// matters: making the default non-neutral would have silently retuned
    /// every lap time in the game.
    ///
    /// `speed_fraction` is the car's speed as a fraction of its own top speed.
    pub fn gear_thrust_factor(&self, speed_fraction: Fixed) -> Fixed {
        let ratio_excess = Self::scale_factor(self.gearing) - Fixed::ONE;
        // -1 at a standstill, +1 at top speed.
        let bias = speed_fraction
            .clamp(Fixed::ZERO, Fixed::ONE)
            .scale(Fixed::from_int(2))
            - Fixed::ONE;
        // Clamped so a hostile or corrupt tuning value cannot invert thrust or
        // make it explode.
        let factor = Fixed::ONE + ratio_excess * bias;
        factor.clamp(Fixed::from_raw(FP_ONE / 2), Fixed::from_int(2))
    }
}

#[cfg(test)]
mod tests {
    use super::{CarTuning, DEFAULT_SLIDER, MAX_SLIDER, MIN_SLIDER};
    use crate::math::Fixed;

    fn tuning_with_gearing(gearing: u8) -> CarTuning {
        CarTuning {
            gearing,
            ..CarTuning::default()
        }
    }

    /// The load-bearing property. `gearing` was inert for the whole project
    /// (TASK-1215): it appeared in the struct, `is_valid`, `total_points`, the
    /// save format, the AI profiles and the Garage slider, and nothing read it.
    /// Fixing it must not silently retune the default car, or every lap time in
    /// the game would shift underneath the player.
    #[test]
    fn the_default_car_is_unaffected_by_the_gearing_slider() {
        for speed_fraction in [0, 1, 2, 3, 4] {
            let fraction = Fixed::from_raw(Fixed::ONE.raw() / 4 * speed_fraction);
            assert_eq!(
                tuning_with_gearing(DEFAULT_SLIDER).gear_thrust_factor(fraction),
                Fixed::ONE,
                "default gearing changed thrust at speed fraction {speed_fraction}/4"
            );
        }
    }

    #[test]
    fn a_short_ratio_pulls_harder_low_down_and_tapers_at_the_top() {
        let short = tuning_with_gearing(MIN_SLIDER);
        let low = short.gear_thrust_factor(Fixed::ZERO);
        let high = short.gear_thrust_factor(Fixed::ONE);
        assert!(
            low > Fixed::ONE,
            "a short ratio must pull harder than neutral off the corner: {low:?}"
        );
        assert!(
            high < Fixed::ONE,
            "a short ratio must taper at the top: {high:?}"
        );
    }

    #[test]
    fn a_long_ratio_is_lazy_low_down_and_stronger_at_the_top() {
        let long = tuning_with_gearing(MAX_SLIDER);
        assert!(
            long.gear_thrust_factor(Fixed::ZERO) < Fixed::ONE,
            "a long ratio must be lazy off the line"
        );
        assert!(
            long.gear_thrust_factor(Fixed::ONE) > Fixed::ONE,
            "a long ratio must carry better at the top"
        );
    }

    #[test]
    fn gearing_has_an_effect_at_all() {
        // The regression test for the original bug: any slider value must change
        // something the physics reads.
        let short = tuning_with_gearing(MIN_SLIDER);
        let long = tuning_with_gearing(MAX_SLIDER);
        assert_ne!(
            short.gear_thrust_factor(Fixed::ZERO),
            long.gear_thrust_factor(Fixed::ZERO),
            "the gearing slider still does nothing"
        );
    }

    #[test]
    fn the_torque_curve_pivots_at_mid_range() {
        // Both directions of gearing must cross over at the middle, where the
        // ratio stops mattering.
        for gearing in MIN_SLIDER..=MAX_SLIDER {
            let mid = tuning_with_gearing(gearing)
                .gear_thrust_factor(Fixed::from_raw(Fixed::ONE.raw() / 2));
            assert_eq!(
                mid,
                Fixed::ONE,
                "gearing {gearing} did not pivot at mid range"
            );
        }
    }

    #[test]
    fn the_curve_is_monotonic_across_the_speed_range() {
        for gearing in MIN_SLIDER..=MAX_SLIDER {
            let tuning = tuning_with_gearing(gearing);
            let mut previous = tuning.gear_thrust_factor(Fixed::ZERO);
            for step in 1..=8 {
                let fraction = Fixed::from_raw((Fixed::ONE.raw() / 8) * step);
                let current = tuning.gear_thrust_factor(fraction);
                let rising = gearing > DEFAULT_SLIDER;
                if rising {
                    assert!(
                        current >= previous,
                        "gearing {gearing}: thrust fell as speed rose ({previous:?} -> {current:?})"
                    );
                } else if gearing < DEFAULT_SLIDER {
                    assert!(
                        current <= previous,
                        "gearing {gearing}: thrust rose as speed rose ({previous:?} -> {current:?})"
                    );
                }
                previous = current;
            }
        }
    }

    #[test]
    fn thrust_is_always_positive_and_bounded() {
        // A hostile save value must not be able to invert thrust or explode it.
        for gearing in 0..=u8::MAX {
            let tuning = tuning_with_gearing(gearing);
            for step in 0..=8 {
                let fraction = Fixed::from_raw((Fixed::ONE.raw() / 8) * step);
                let factor = tuning.gear_thrust_factor(fraction);
                assert!(
                    factor > Fixed::ZERO,
                    "gearing {gearing} produced non-positive thrust at step {step}"
                );
                assert!(
                    factor <= Fixed::from_int(2),
                    "gearing {gearing} produced runaway thrust {factor:?}"
                );
            }
        }
    }

    #[test]
    fn out_of_range_speed_fractions_are_clamped_not_wrapped() {
        let short = tuning_with_gearing(MIN_SLIDER);
        assert_eq!(
            short.gear_thrust_factor(Fixed::from_int(-5)),
            short.gear_thrust_factor(Fixed::ZERO),
            "a negative speed fraction changed the curve"
        );
        assert_eq!(
            short.gear_thrust_factor(Fixed::from_int(5)),
            short.gear_thrust_factor(Fixed::ONE),
            "an absurd speed fraction changed the curve"
        );
    }
}
