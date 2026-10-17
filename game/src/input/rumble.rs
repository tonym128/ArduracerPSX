//! Dual-motor force feedback rumble driver.
//!
//! Controls DualShock haptic feedback motors:
//! - Small Motor: High-frequency on/off eccentric weight for curb chatter and rev-limit vibration.
//! - Large Motor: Variable-speed (0–255) low-frequency weighted rotor for collision impacts and turbo impulses.

use arduracer_core::SurfaceType;

pub struct RumbleDriver {
    pub small_motor: bool,
    pub large_motor: u8,
    pub impact_ticks: u8,
    pub boost_ticks: u8,
}

impl Default for RumbleDriver {
    fn default() -> Self {
        Self::new()
    }
}

impl RumbleDriver {
    pub const fn new() -> Self {
        RumbleDriver {
            small_motor: false,
            large_motor: 0,
            impact_ticks: 0,
            boost_ticks: 0,
        }
    }

    /// Triggers a strong barrier crash impact vibration.
    pub fn trigger_impact(&mut self, intensity: u8) {
        self.large_motor = intensity.max(self.large_motor);
        self.impact_ticks = 12;
    }

    /// Triggers a turbo boost launch kick.
    pub fn trigger_boost(&mut self) {
        self.large_motor = 220;
        self.boost_ticks = 10;
        self.small_motor = true;
    }

    /// Updates rumble motors for the current 60Hz tick based on vehicle dynamics, speed, tire scrub, and surface.
    pub fn tick(
        &mut self,
        surface: SurfaceType,
        speed: arduracer_core::Fixed,
        is_drifting: bool,
        tire_scrub: arduracer_core::Fixed,
        rpm: u16,
    ) {
        let mut target_small = false;
        let mut target_large: u8 = 0;
        let abs_speed = speed.abs();
        let is_moving = abs_speed > arduracer_core::Fixed::from_raw(200);
        let speed_scale = (abs_speed.raw().clamp(0, 14_000) * 256) / 14_000;

        // 1. Surface tactile rumble (only when rolling)
        if is_moving {
            match surface {
                SurfaceType::Curb => {
                    // High-frequency rumble strip chatter
                    target_small = true;
                    target_large = ((70 * speed_scale) >> 8).clamp(35, 70) as u8;
                }
                SurfaceType::OffRoad => {
                    // Low-frequency rough terrain drag
                    target_large = ((110 * speed_scale) >> 8).clamp(45, 110) as u8;
                }
                SurfaceType::OilSlick => {
                    // Smooth slippery loss of traction
                    target_small = false;
                }
                _ => {}
            }
        }

        // 2. High-speed drift slide vibration and tire scrub warning
        if is_drifting {
            target_small = true;
            target_large = target_large.max(90);
        } else if tire_scrub > arduracer_core::Fixed::from_raw(3000) {
            // Tactile warning as tires fight for grip near adhesion limit
            target_small = true;
            if tire_scrub > arduracer_core::Fixed::ONE {
                // Outward washout slide
                target_large = target_large.max(60);
            }
        }

        // 3. Engine rev-limiter redline buzz (> 8000 RPM)
        if rpm > 8000 {
            target_small = true;
        }

        // 4. Decay one-shot effects (impacts, boost)
        if self.impact_ticks > 0 {
            self.impact_ticks -= 1;
            target_large = target_large.max(self.large_motor.saturating_sub(15));
        }
        if self.boost_ticks > 0 {
            self.boost_ticks -= 1;
            target_small = true;
            target_large = target_large.max(180);
        }

        self.small_motor = target_small;
        self.large_motor = target_large;
    }

    /// Returns the raw actuator bytes to send to DualShock port 1: (small_on, large_speed).
    pub fn actuator_bytes(&self) -> (bool, u8) {
        (self.small_motor, self.large_motor)
    }
}
