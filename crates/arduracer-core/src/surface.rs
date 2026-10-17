//! Track surface properties and hazard types.
//!
//! Each surface exposes three independent coefficients so that the vehicle model
//! stays predictable:
//!
//! * [`SurfaceType::max_speed_factor`] – hard cap on forward speed (arcade
//!   "off-road penalty", per GAME.md §3.1).
//! * [`SurfaceType::traction`] – fraction of engine thrust actually delivered.
//! * [`SurfaceType::lateral_hold`] – how quickly sideways velocity is scrubbed
//!   off a car that is on line.
//!
//! Keeping them separate avoids the classic bug where multiplying engine thrust
//! and drag together makes off-road speed collapse to a fraction of a percent of
//! top speed (an unusable wall rather than a penalty).
//!
//! A *drift* deliberately appears nowhere on this list. Side load while sliding is
//! the vehicle model's business (it generates the slide rather than scrubbing it),
//! and [`SurfaceType::grip_factor`] is what decides how dear a slide is on a given
//! surface. A coefficient that only ever applied while drifting was a way for a
//! slide and a grip to end up with the same physics.

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

    /// Per-tick retention of sideways velocity for a car on line (0..FP_ONE).
    ///
    /// Tarmac holds the car on line; an oil slick lets it slide. This is the
    /// per-tick *scrub* only; how much side force the tyres can hold at all is
    /// [`SurfaceType::grip_factor`], applied by the vehicle's friction circle.
    ///
    /// There is deliberately no "drifting" branch any more. It used to return 2540
    /// against 3604 for the gripping case, and the doc comment called that "the
    /// back end has let go" -- but retention is how much sideways velocity
    /// *survives*, so the lower number scrubbed a slide away nearly three times
    /// faster than gripping did. A handbrake stab therefore killed its own slide:
    /// 1.4-7.5 degrees of real side-slip while the car was drawn at 25-40. Side
    /// load while sliding is now the vehicle's business (it generates the slide
    /// rather than scrubbing it), and a surface decides what a slide *costs*
    /// through [`SurfaceType::grip_factor`] instead.
    pub fn lateral_hold(self) -> Fixed {
        match self {
            SurfaceType::OilSlick => Fixed::from_raw(2048), // 0.50 - instant slide
            SurfaceType::Barrier => Fixed::ZERO,
            // Off-road is loose but not frictionless: it scrubs sideways speed
            // faster than tarmac does, which is what makes running wide there feel
            // like running on a different surface rather than a different handling
            // model.
            SurfaceType::OffRoad => Fixed::from_raw(3482), // 0.85
            _ => Fixed::from_raw(3604),                    // 0.88 - on line
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
            assert!(surface.lateral_hold() >= Fixed::ZERO, "{surface:?}");
            assert!(surface.lateral_hold() <= Fixed::ONE, "{surface:?}");
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
        assert_eq!(SurfaceType::Barrier.lateral_hold(), Fixed::ZERO);
        assert!(SurfaceType::Barrier.is_solid());
        assert_eq!(SurfaceType::Tarmac.max_speed_factor(), Fixed::ONE);
        assert_eq!(SurfaceType::Tarmac.grip_factor(), Fixed::ONE);
    }

    /// Reproduced: the "drifting" branch returned 2540 against 3604 for the
    /// gripping case, and its doc comment called the *lower* number "the back end
    /// has let go". Retention is how much sideways velocity survives a tick, so
    /// the branch scrubbed a slide away nearly three times faster than gripping
    /// did -- the handbrake cancelled the slide it was supposed to cause, and a
    /// tarmac drift ended up with the same physics as a tarmac grip.
    ///
    /// There is no branch to assert any more. What replaces it has to live in the
    /// vehicle model, which is where this test's successor lives: `lateral_hold`
    /// is now a single number per surface and says only what a car *on line* does.
    #[test]
    fn there_is_no_longer_a_drift_only_lateral_hold() {
        // Off-road still scrubs more than tarmac, which is what keeps running wide
        // there feeling like a different surface rather than a different car.
        assert!(SurfaceType::OffRoad.lateral_hold() < SurfaceType::Tarmac.lateral_hold());
        // And the oil slick is still the one surface that lets a car on line go.
        assert!(SurfaceType::OilSlick.lateral_hold() < SurfaceType::OffRoad.lateral_hold());
    }

    /// The documented order: tarmac grips hardest, an oil slick has almost none,
    /// and a barrier has none at all.
    #[test]
    fn the_documented_grip_ordering_holds() {
        assert!(SurfaceType::Tarmac.grip_factor() > SurfaceType::Curb.grip_factor());
        assert!(SurfaceType::Curb.grip_factor() > SurfaceType::OffRoad.grip_factor());
        assert!(SurfaceType::OffRoad.grip_factor() > SurfaceType::OilSlick.grip_factor());
        assert!(SurfaceType::OilSlick.grip_factor() > SurfaceType::Barrier.grip_factor());

        assert!(SurfaceType::OilSlick.lateral_hold() < SurfaceType::Tarmac.lateral_hold());
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
