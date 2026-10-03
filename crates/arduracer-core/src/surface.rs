//! Track surface properties and hazard types.

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
}
