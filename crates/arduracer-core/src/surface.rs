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
    pub fn grip_factor(self) -> Fixed {
        match self {
            SurfaceType::Tarmac => Fixed::ONE,
            SurfaceType::Curb => Fixed::from_raw(3481), // ~85% grip
            SurfaceType::OffRoad => Fixed::from_raw(1433), // ~35% speed
            SurfaceType::OilSlick => Fixed::from_raw(409), // ~10% grip (spin)
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
    /// Tarmac holds the car on line; oil slicks and drifts let it slide.
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
                    Fixed::from_raw(3640) // 0.89
                } else {
                    Fixed::from_raw(3641) // 0.89 (grip, slide only slightly less)
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
