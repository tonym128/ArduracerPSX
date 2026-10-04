//! Track surface properties and hazard types.
//!
//! Each surface exposes three independent coefficients so that the vehicle model
//! stays predictable:
//!
//! * [`SurfaceType::max_speed_factor`] – hard cap on forward speed (arcade
//!   "off-road penalty", per GAME.md §3.1).
//! * [`SurfaceType::traction`] – fraction of engine thrust actually delivered.
//! * [`SurfaceType::lateral_hold`] – how quickly sideways velocity is scrubbed.
//!
//! Keeping them separate avoids the classic bug where multiplying engine thrust
//! and drag together makes off-road speed collapse to a fraction of a percent of
//! top speed (an unusable wall rather than a penalty).

use crate::math::Fixed;

/// Surface type classifications on the racetrack.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum SurfaceType {
    /// Optimal high-grip racing surface.
    Tarmac = 0,
    /// Textured rumble curb. Induces vibration and slight grip reduction.
    Curb = 1,
    /// Off-road terrain (grass, gravel, sand) causing severe drag.
    OffRoad = 2,
    /// Slippery hazard causing immediate loss of lateral grip.
    OilSlick = 3,
    /// Speed booster strip giving instantaneous forward acceleration impulse.
    BoostPad = 4,
    /// Solid wall or track barrier (causes collision bounce and scrape sparks).
    Barrier = 5,
}

impl SurfaceType {
    /// Friction coefficient applied to vehicle velocity.
    ///
    /// Per-tick retention of *sideways* velocity on this surface, i.e. how
    /// hard the tyres scrub a slide off. The vehicle model now folds this into
    /// its speed-dependent grip circle (see
    /// [`crate::vehicle::VehicleState::tick`]): it is an upper bound on the
    /// bare per-tick retention, so a surface can never be grippier than its
    /// coefficient allows, and the two are no longer redundant.
    pub fn grip_factor(self) -> Fixed {
        match self {
            SurfaceType::Tarmac => Fixed::ONE,
            SurfaceType::Curb => Fixed::from_raw(3641), // ~0.89
            SurfaceType::OffRoad => Fixed::from_raw(3482), // ~0.85
            SurfaceType::OilSlick => Fixed::from_raw(2048), // 0.50 - instant slide
            SurfaceType::BoostPad => Fixed::ONE,
            SurfaceType::Barrier => Fixed::ZERO,
        }
    }

    /// Hard ceiling on forward speed as a fraction of the car's top speed.
    ///
    /// GAME.md §3.1: tarmac `1.0x`, curb `~0.97x`, off-road `0.35x`, boost `1.0x`.
    pub fn max_speed_factor(self) -> Fixed {
        match self {
            SurfaceType::Tarmac | SurfaceType::BoostPad => Fixed::ONE,
            SurfaceType::Curb => Fixed::from_raw(3969), // ~0.97
            SurfaceType::OffRoad => Fixed::from_raw(1433), // ~0.35
            SurfaceType::OilSlick => Fixed::ONE,
            SurfaceType::Barrier => Fixed::ZERO,
        }
    }

    /// Fraction of engine thrust that actually reaches the tarmac.
    pub fn traction(self) -> Fixed {
        match self {
            SurfaceType::Tarmac | SurfaceType::BoostPad => Fixed::ONE,
            SurfaceType::Curb => Fixed::from_raw(3539), // ~0.86
            SurfaceType::OffRoad => Fixed::from_raw(2253), // ~0.55
            SurfaceType::OilSlick => Fixed::from_raw(3277), // ~0.80
            SurfaceType::Barrier => Fixed::ZERO,
        }
    }

    /// Per-tick retention of sideways velocity while cornering (0..FP_ONE).
    ///
    /// Tarmac holds the car on line; oil slicks and drifts let it slide. The
    /// drifting branch used to return 3640 against 3641 for the gripping
    /// branch -- a 1/4096 difference, so "sliding" changed nothing on tarmac and
    /// the doc comment was a lie. Drifting on tarmac now genuinely lets the
    /// back end go (0.62/tick), off-road slides less (0.72) than the dry tarmac
    /// drift does, and an oil slick is hopeless either way.
    ///
    /// This is the per-tick *scrub* only; how much side force the tyres can
    /// hold at all is [`SurfaceType::grip_factor`], applied by the vehicle's
    /// friction circle.
    pub fn lateral_hold(self, drifting: bool) -> Fixed {
        match self {
            SurfaceType::OilSlick => Fixed::from_raw(2048), // 0.50 - instant slide
            SurfaceType::Barrier => Fixed::ZERO,
            SurfaceType::OffRoad => {
                if drifting {
                    Fixed::from_raw(2944) // 0.72
                } else {
                    Fixed::from_raw(3482) // 0.85
                }
            }
            _ => {
                if drifting {
                    Fixed::from_raw(2540) // 0.62 - the back end has let go
                } else {
                    Fixed::from_raw(3604) // 0.88 - on line
                }
            }
        }
    }

    /// Whether this surface triggers DualShock small-motor curb vibration.
    pub fn triggers_curb_rumble(self) -> bool {
        matches!(self, SurfaceType::Curb)
    }

    /// Whether this surface triggers off-road heavy vibration and slowdown.
    pub fn is_offroad(self) -> bool {
        matches!(self, SurfaceType::OffRoad)
    }

    /// Whether this surface is impassable solid barrier.
    pub fn is_solid(self) -> bool {
        matches!(self, SurfaceType::Barrier)
    }

    /// Whether driving over this surface instantly grants a forward impulse.
    pub fn is_boost_pad(self) -> bool {
        matches!(self, SurfaceType::BoostPad)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every surface must hand the vehicle a coherent set of coefficients: a
    /// grippy surface may not be slower than a slippery one, and nothing may
    /// divide by zero downstream.
    #[test]
    fn surface_coefficients_are_ordered_and_bounded() {
        for surface in [
            SurfaceType::Tarmac,
            SurfaceType::Curb,
            SurfaceType::OffRoad,
            SurfaceType::OilSlick,
            SurfaceType::BoostPad,
            SurfaceType::Barrier,
        ] {
            assert!(surface.lateral_hold(false) >= Fixed::ZERO, "{surface:?}");
            assert!(surface.lateral_hold(false) <= Fixed::ONE, "{surface:?}");
            assert!(surface.lateral_hold(true) >= Fixed::ZERO, "{surface:?}");
            assert!(surface.lateral_hold(true) <= Fixed::ONE, "{surface:?}");
            assert!(surface.grip_factor() >= Fixed::ZERO, "{surface:?}");
            assert!(surface.grip_factor() <= Fixed::ONE, "{surface:?}");
            assert!(surface.traction() >= Fixed::ZERO, "{surface:?}");
            assert!(surface.traction() <= Fixed::ONE, "{surface:?}");
            assert!(surface.max_speed_factor() >= Fixed::ZERO, "{surface:?}");
            assert!(surface.max_speed_factor() <= Fixed::ONE, "{surface:?}");
        }
        assert_eq!(SurfaceType::Barrier.max_speed_factor(), Fixed::ZERO);
        assert_eq!(SurfaceType::Barrier.traction(), Fixed::ZERO);
        assert_eq!(SurfaceType::Barrier.grip_factor(), Fixed::ZERO);
        assert_eq!(SurfaceType::Barrier.lateral_hold(false), Fixed::ZERO);
        assert!(SurfaceType::Barrier.is_solid());
        assert_eq!(SurfaceType::Tarmac.max_speed_factor(), Fixed::ONE);
        assert_eq!(SurfaceType::Tarmac.grip_factor(), Fixed::ONE);
    }

    /// Reproduced: the drifting branch returned 3640 against a gripping 3641 -- a
    /// 1/4096 difference -- so "oil slicks and drifts let it slide" was not true
    /// of tarmac at all.
    #[test]
    fn drifting_really_does_change_grip_on_tarmac() {
        let gripping = SurfaceType::Tarmac.lateral_hold(false);
        let sliding = SurfaceType::Tarmac.lateral_hold(true);
        assert!(
            sliding + Fixed::from_raw(500) < gripping,
            "tarmac drift ({}) is indistinguishable from tarmac grip ({})",
            sliding.raw(),
            gripping.raw()
        );
        assert!(
            sliding < Fixed::from_raw(2816),
            "a tarmac drift must be a real slide, not a rounding error"
        );
    }

    /// The documented order: tarmac grips hardest, an oil slick has almost none,
    /// and a barrier has none at all.
    #[test]
    fn the_documented_grip_ordering_holds() {
        assert!(SurfaceType::Tarmac.grip_factor() > SurfaceType::Curb.grip_factor());
        assert!(SurfaceType::Curb.grip_factor() > SurfaceType::OffRoad.grip_factor());
        assert!(SurfaceType::OffRoad.grip_factor() > SurfaceType::OilSlick.grip_factor());
        assert!(SurfaceType::OilSlick.grip_factor() > SurfaceType::Barrier.grip_factor());

        assert!(
            SurfaceType::OilSlick.lateral_hold(false) < SurfaceType::Tarmac.lateral_hold(false)
        );
        assert!(
            SurfaceType::Tarmac.lateral_hold(true) < SurfaceType::Tarmac.lateral_hold(false),
            "gripping must hold more than sliding"
        );
        // Off-road is drivable and grippier than an oil slick, but the vehicle
        // model has to be able to tell them apart.
        assert!(SurfaceType::OffRoad.traction() > Fixed::ZERO);
        assert!(SurfaceType::OffRoad.traction() < Fixed::ONE);
    }

    /// Off-road is a penalty, not a wall: the vehicle model reads `traction` and
    /// `max_speed_factor` from here.
    #[test]
    fn the_offroad_penalty_is_playable() {
        assert!(SurfaceType::OffRoad.max_speed_factor() > Fixed::ZERO);
        assert!(SurfaceType::OffRoad.max_speed_factor() < Fixed::HALF);
        assert!(SurfaceType::OffRoad.traction() > Fixed::HALF);
        assert!(SurfaceType::OffRoad.is_offroad());
        assert!(SurfaceType::Curb.triggers_curb_rumble());
        assert!(!SurfaceType::Tarmac.triggers_curb_rumble());
        assert!(SurfaceType::BoostPad.is_boost_pad());
        assert!(!SurfaceType::Tarmac.is_boost_pad());
    }
}
