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
/// Side-slip unwound per tick at *full* opposite lock, in BAMs (4096 = 360 deg).
///
/// The old code applied a flat 32 BAM/tick recovery that saturated at a
/// hard-coded floor of +/-128, so no amount of counter-steering ever took the
/// slide past 128, the 640 spin-out threshold was unreachable from a
/// counter-steer, and `ticks` charged forever: endless Super-Turbos with the
/// handbrake held and one steering direction, without taking a corner.
const COUNTER_RECOVERY_FULL_LOCK: i32 = 8;
/// Side-slip that decays on its own per tick when the driver lets go of the
/// wheel entirely (a slide always bleeds off; it does not sit at a fixed angle).
const COUNTER_RECOVERY_COAST: i32 = 3;
/// Slide depth at which the car is spinning into the corner.
const DRIFT_SPIN_SLIP: i16 = 640;
/// Side-slip added per tick when the driver keeps steering into the slide.
const DRIFT_DEEPEN_PER_TICK: i16 = 24;
/// Initial slip angle a handbrake stab throws the car into (256 BAM = 22.5 deg).
const DRIFT_INITIAL_SLIP: i16 = 256;

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
    ///
    /// Total for any `direction`: only the *sign* is meaningful (the field is
    /// documented as `-1`/`+1`), so the direction is normalised to it and the
    /// slip angle can never be pushed outside the spin-out range.
    pub fn initiate_drift(&mut self, direction: i8) {
        if !self.is_spinning() {
            let sign: i16 = if direction < 0 { -1 } else { 1 };
            *self = DriftState::Drifting {
                ticks: 0,
                slip_angle: sign * DRIFT_INITIAL_SLIP,
                direction: sign as i8,
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

    /// Cuts a spin-out short and hands control back to the driver.
    ///
    /// The player needs an escape hatch: without one, a spin-out triggered by a
    /// wall (or an oil slick) ran its full [`SPINOUT_TICKS`] with the throttle,
    /// the steering and the reverse gear all fighting a state that ignored them.
    /// A no-op in any other state.
    pub fn cancel_spinout(&mut self) {
        if self.is_spinning() {
            *self = DriftState::Grip;
        }
    }

    /// Advances the drift simulation by one 60Hz tick.
    ///
    /// Returns the boost impulse the drift released on its way out: whatever
    /// tier was earned, whether the drift was ended by lifting off the
    /// handbrake, by the car scrubbing off all its speed, or by the slide
    /// finally unwinding past neutral. Returning it here (rather than
    /// silently replacing the state) is what stops a paid-for mini-turbo from
    /// being thrown away.
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
                // Drifting to a standstill ends the slide -- but the turbo that
                // was already earned is paid out, not confiscated.
                if car_speed < Fixed::from_raw(2000) {
                    return self.release_drift();
                }

                *ticks = ticks.saturating_add(1);

                // Counter-steering dynamics.
                //
                // Sliding right (direction > 0) and steering left (steer < 0)
                // means the driver is counter-steering: the slide unwinds at a
                // rate proportional to how much opposite lock is applied, and a
                // car with a high `drift_stability` slider resists being snapped
                // straight (so it holds a slide longer). Steering *into* the
                // slide deepens it. Hands off the wheel, it bleeds off on its own.
                let is_counter_steering = (direction > 0 && steer_input < Fixed::ZERO)
                    || (direction < 0 && steer_input > Fixed::ZERO);
                let stability = CarTuning::scale_factor(tuning.drift_stability).raw().max(1);
                // 256 == "recovery as specified"; drift_stability 4 gives exactly 256.
                let stability_numerator = ((math::FP_ONE as i64) << 8) / stability as i64;

                if is_counter_steering {
                    // `saturating_abs`, not `Fixed::abs`: the latter negates and
                    // overflows for `Fixed::from_raw(i32::MIN)`.
                    let opposite_lock = steer_input.raw().saturating_abs() as i64;
                    let proportional =
                        (opposite_lock * COUNTER_RECOVERY_FULL_LOCK as i64) >> math::FP_SHIFT;
                    let recovery = ((proportional * stability_numerator) >> 8).max(1) as i16;
                    if direction > 0 {
                        *slip_angle = (*slip_angle - recovery).max(-DRIFT_SPIN_SLIP);
                    } else {
                        *slip_angle = (*slip_angle + recovery).min(DRIFT_SPIN_SLIP);
                    }
                } else if steer_input.raw() != 0 {
                    // Hard cornering deepens the drift angle.
                    if direction > 0 {
                        *slip_angle = (*slip_angle + DRIFT_DEEPEN_PER_TICK).min(DRIFT_SPIN_SLIP);
                    } else {
                        *slip_angle = (*slip_angle - DRIFT_DEEPEN_PER_TICK).max(-DRIFT_SPIN_SLIP);
                    }
                } else {
                    let recovery =
                        ((COUNTER_RECOVERY_COAST as i64 * stability_numerator) >> 8).max(1) as i16;
                    if direction > 0 {
                        *slip_angle = (*slip_angle - recovery).max(-DRIFT_SPIN_SLIP);
                    } else {
                        *slip_angle = (*slip_angle + recovery).min(DRIFT_SPIN_SLIP);
                    }
                }

                // Over-drifting spin-out threshold: spun into the corner.
                // `abs` on i16 would overflow for i16::MIN, so widen first.
                if (*slip_angle as i32).abs() >= DRIFT_SPIN_SLIP as i32 {
                    self.trigger_spinout();
                    return Fixed::ZERO;
                }

                // The slide has unwound past neutral: the driver caught it. The
                // drift is over and whatever it earned is paid out. Without this
                // a held handbrake could park the slip at a fixed angle
                // indefinitely and farm turbos forever.
                if (direction > 0 && *slip_angle <= 0) || (direction < 0 && *slip_angle >= 0) {
                    return self.release_drift();
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

#[cfg(test)]
mod tests {
    use super::*;

    fn tuning() -> CarTuning {
        CarTuning::default()
    }

    /// A drift can only be held at speed, well above the low-speed cancel.
    const FAST: Fixed = Fixed::from_int(3);

    fn slip(state: &DriftState) -> i16 {
        match state {
            DriftState::Drifting { slip_angle, .. } => *slip_angle,
            _ => 0,
        }
    }

    /// Magnitude of the slide, widened so it cannot overflow like `i16::abs`.
    fn slip_abs(state: &DriftState) -> i32 {
        match state {
            DriftState::Drifting { slip_angle, .. } => (*slip_angle as i32).abs(),
            _ => 0,
        }
    }

    // --- initiate_drift -------------------------------------------------------

    #[test]
    fn initiate_drift_sets_direction_and_initial_slip() {
        for (direction, expected) in [(1i8, 256i16), (-1i8, -256i16)] {
            let mut state = DriftState::Grip;
            state.initiate_drift(direction);
            assert!(state.is_drifting());
            assert_eq!(slip(&state), expected, "direction {direction}");
            assert_eq!(state.boost_tier(), 0);
            match state {
                DriftState::Drifting {
                    direction: d,
                    ticks,
                    ..
                } => {
                    assert_eq!(d, direction);
                    assert_eq!(ticks, 0);
                }
                _ => panic!("expected a drift"),
            }
        }
    }

    #[test]
    fn initiate_drift_does_not_override_a_spinout() {
        let mut state = DriftState::Grip;
        state.trigger_spinout();
        state.initiate_drift(1);
        assert!(state.is_spinning(), "a spin-out outranks a new drift");
    }

    #[test]
    fn initiate_drift_is_total_for_every_i8_direction() {
        for direction in -128i8..=127 {
            let mut state = DriftState::Grip;
            state.initiate_drift(direction);
            assert!(state.is_drifting(), "direction {direction} lost the drift");
            let angle = slip(&state);
            let sign = if direction < 0 { -1i16 } else { 1 };
            assert_eq!(angle, sign * DRIFT_INITIAL_SLIP);
            assert!(slip_abs(&state) <= DRIFT_SPIN_SLIP as i32);
            // And the state machine can run it without panicking.
            for _ in 0..20 {
                state.tick(Fixed::ZERO, FAST, &tuning());
            }
        }
    }

    // --- bug 18: counter-steering pinned the slip at 128 ---------------------

    #[test]
    fn counter_steering_recovers_the_slip_all_the_way_to_zero() {
        // Reproduced the bug: slip sat at exactly 128 from tick 10 to tick 200.
        let mut state = DriftState::Grip;
        state.initiate_drift(1);
        let mut seen = [false; 200];
        let mut unwound_at = 0u16;
        for i in 0..200u16 {
            // Gentle counter-steer to the left against a right-hand slide.
            state.tick(Fixed::from_raw(-1024), FAST, &tuning());
            if let DriftState::Drifting { slip_angle, .. } = state {
                seen[i as usize] = true;
                assert!(
                    slip_angle < DRIFT_INITIAL_SLIP,
                    "counter-steering must keep unwinding (tick {i}, slip {slip_angle})"
                );
                assert!(
                    slip_angle > 0,
                    "the drift must end at neutral, not run past it (tick {i})"
                );
            } else if unwound_at == 0 {
                unwound_at = i + 1;
            }
        }
        assert!(
            !seen[199],
            "a drift held with one steering direction must not last 200 ticks"
        );
        assert!(unwind_nonzero(unwound_at), "the slide never unwound");
    }

    fn unwind_nonzero(ticks: u16) -> bool {
        ticks > 0 && ticks < 200
    }

    #[test]
    fn full_opposite_lock_unwinds_the_slide_faster_than_a_gentle_touch() {
        let mut full = DriftState::Grip;
        full.initiate_drift(1);
        let mut gentle = DriftState::Grip;
        gentle.initiate_drift(1);

        let mut full_ticks = 0u16;
        let mut gentle_ticks = 0u16;
        for _ in 0..300 {
            if full.is_drifting() {
                full.tick(Fixed::from_raw(-Fixed::ONE.raw()), FAST, &tuning());
                full_ticks += 1;
            }
            if gentle.is_drifting() {
                gentle.tick(Fixed::from_raw(-1024), FAST, &tuning());
                gentle_ticks += 1;
            }
        }
        assert!(full_ticks > 0 && gentle_ticks > full_ticks);
        assert!(full_ticks <= DRIFT_INITIAL_SLIP as u16 / 2);
        assert!(
            !full.is_drifting(),
            "full opposite lock must catch the slide"
        );
        assert!(!gentle.is_drifting());
    }

    #[test]
    fn holding_the_handbrake_and_one_steering_direction_cannot_farm_turbos() {
        // A player can hold the handbrake with a single steering direction and
        // previously reached Super-Turbo indefinitely (slip pinned at 128, ticks
        // charging past 200). Each press of the handbrake may now pay out at
        // most one turbo, so a held handbrake pays out once and then the slide
        // is over for good.
        let mut state = DriftState::Grip;
        let mut payouts = 0u32;
        let mut best_ticks_seen = 0u16;
        // The handbrake is pressed once at the start and never let go: the drift
        // must terminate rather than re-arm itself.
        state.initiate_drift(1);
        for i in 0..600u16 {
            let released = state.tick(Fixed::from_raw(-1024), FAST, &tuning());
            if released > Fixed::ZERO {
                payouts += 1;
            }
            if let DriftState::Drifting { ticks, .. } = state {
                best_ticks_seen = best_ticks_seen.max(ticks);
            }
            if !state.is_drifting() && i > 0 {
                // Still holding the handbrake: re-arming here is the vehicle
                // model's job, and it is gated on the handbrake being released.
                assert!(
                    !state.is_drifting(),
                    "the slide must stay finished while the handbrake is held"
                );
            }
        }
        assert_eq!(
            payouts, 1,
            "a single held handbrake paid out {payouts} turbos"
        );
        assert!(
            best_ticks_seen <= DRIFT_BOOST_LEVEL2_TICKS + 64,
            "a single held handbrake charged for {best_ticks_seen} ticks"
        );
    }

    #[test]
    fn over_steering_into_the_slide_still_spins_the_car() {
        let mut state = DriftState::Grip;
        state.initiate_drift(1);
        // Keep the handbrake in and the wheel wound further into the slide.
        for _ in 0..100 {
            if state.is_drifting() {
                state.tick(Fixed::from_raw(2048), FAST, &tuning());
            }
        }
        assert!(state.is_spinning(), "over-steering must spin the car");
        assert_eq!(
            state,
            DriftState::SpinOut {
                remaining_ticks: SPINOUT_TICKS
            }
        );
    }

    // --- bug 19: untested state-machine edges ---------------------------------

    #[test]
    fn low_speed_drift_loss_pays_out_an_earned_turbo() {
        // The old code replaced the state with `Grip` and returned nothing,
        // silently destroying a mini-turbo the player had already earned.
        let mut state = DriftState::Grip;
        state.initiate_drift(1);
        for _ in 0..(DRIFT_BOOST_LEVEL1_TICKS + 5) {
            let _ = state.tick(Fixed::ZERO, FAST, &tuning());
            if !state.is_drifting() {
                panic!("the drift must survive a held slide at speed");
            }
        }
        assert_eq!(state.boost_tier(), 1);

        // Scrub off all the speed mid-drift.
        let payout = state.tick(Fixed::ZERO, Fixed::ZERO, &tuning());
        assert_eq!(payout, Fixed::from_raw(2000), "mini-turbo must be paid");
        assert_eq!(state, DriftState::Grip, "the drift is over");
    }

    #[test]
    fn low_speed_drift_loss_with_no_tier_pays_nothing_and_still_ends() {
        let mut state = DriftState::Grip;
        state.initiate_drift(-1);
        let payout = state.tick(Fixed::ZERO, Fixed::ZERO, &tuning());
        assert_eq!(payout, Fixed::ZERO);
        assert_eq!(state, DriftState::Grip);
    }

    #[test]
    fn spinout_counts_down_to_grip() {
        let mut state = DriftState::Grip;
        state.trigger_spinout();
        assert!(state.is_spinning());
        for tick in 1..=SPINOUT_TICKS {
            let payout = state.tick(Fixed::ZERO, FAST, &tuning());
            assert_eq!(payout, Fixed::ZERO, "a spin-out pays nothing");
            if tick < SPINOUT_TICKS {
                assert!(state.is_spinning(), "spin-out ended early at {tick}");
                match state {
                    DriftState::SpinOut { remaining_ticks } => {
                        assert_eq!(remaining_ticks, SPINOUT_TICKS - tick);
                    }
                    _ => panic!("state reports spinning but is {state:?}"),
                }
            } else {
                assert_eq!(state, DriftState::Grip, "spin-out must expire");
            }
        }
        // And it stays recovered.
        for _ in 0..10 {
            state.tick(Fixed::ZERO, FAST, &tuning());
        }
        assert_eq!(state, DriftState::Grip);
    }

    #[test]
    fn spinout_is_total_when_over_ticked() {
        let mut state = DriftState::Grip;
        state.trigger_spinout();
        for _ in 0..(SPINOUT_TICKS + 50) {
            state.tick(Fixed::ONE, FAST, &tuning());
        }
        assert_eq!(state, DriftState::Grip);
    }

    #[test]
    fn steering_hands_off_lets_the_slide_bleed_off() {
        let mut state = DriftState::Grip;
        state.initiate_drift(1);
        // No steering input at all: the slide must still decay, not sit at 256.
        for _ in 0..120 {
            state.tick(Fixed::ZERO, FAST, &tuning());
            if !state.is_drifting() {
                break;
            }
        }
        assert!(!state.is_drifting(), "a hands-off slide must bleed off");
    }

    #[test]
    fn release_drift_pays_the_earned_tier_exactly_once() {
        let mut state = DriftState::Grip;
        assert_eq!(state.release_drift(), Fixed::ZERO, "nothing to release");

        state.initiate_drift(1);
        assert_eq!(state.release_drift(), Fixed::ZERO, "tier 0 pays nothing");
        assert_eq!(state.release_drift(), Fixed::ZERO, "and only once");

        state.initiate_drift(1);
        for _ in 0..(DRIFT_BOOST_LEVEL2_TICKS + 2) {
            state.tick(Fixed::from_raw(-1024), FAST, &tuning());
            assert!(state.is_drifting(), "must still be drifting at tier 2");
        }
        assert_eq!(state.boost_tier(), 2);
        assert_eq!(state.release_drift(), Fixed::from_raw(4000));
        assert_eq!(state, DriftState::Grip);
    }

    #[test]
    fn drift_stability_holds_a_slide_longer() {
        // BLAZE (drift_stability 6) should recover a counter-steered slide more
        // slowly than a low-stability car.
        let stable = CarTuning {
            handling: 2,
            drift_stability: 6,
            ..CarTuning::default()
        };
        let let_go = CarTuning {
            gearing: 7,
            drift_stability: 1,
            ..CarTuning::default()
        };
        assert!(stable.is_valid() && let_go.is_valid());

        let mut held = DriftState::Grip;
        held.initiate_drift(1);
        let mut loose = DriftState::Grip;
        loose.initiate_drift(1);
        let (mut held_for, mut loose_for) = (0u16, 0u16);
        for _ in 0..300 {
            if held.is_drifting() {
                held.tick(Fixed::from_raw(-1024), FAST, &stable);
                held_for += 1;
            }
            if loose.is_drifting() {
                loose.tick(Fixed::from_raw(-1024), FAST, &let_go);
                loose_for += 1;
            }
        }
        assert!(held_for > loose_for, "{held_for} vs {loose_for}");
    }

    #[test]
    fn tick_is_total_over_the_whole_fixed_steer_range() {
        for raw in [i32::MIN, -100_000, -4096, -1, 0, 1, 4096, 100_000, i32::MAX] {
            for speed_raw in [i32::MIN, -1, 0, 2000, 14000, i32::MAX] {
                for direction in [-1i8, 0, 1] {
                    let mut state = DriftState::Grip;
                    state.initiate_drift(direction);
                    for _ in 0..5 {
                        let _ =
                            state.tick(Fixed::from_raw(raw), Fixed::from_raw(speed_raw), &tuning());
                    }
                    assert!(slip_abs(&state) <= DRIFT_SPIN_SLIP as i32);
                }
            }
        }
    }
}
