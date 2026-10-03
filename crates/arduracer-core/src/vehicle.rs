//! Vehicle state, dynamics, and drift simulation.

use crate::math::{self, Fixed, Vec2};
use crate::surface::SurfaceType;
use crate::tuning::CarTuning;

/// Vehicle input commands for a single simulation frame.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct VehicleInput {
    /// Throttle applied: 0 (none) to FP_ONE (full gas).
    pub throttle: Fixed,
    /// Brake applied: 0 (none) to FP_ONE (full brake).
    pub brake: Fixed,
    /// Steering input: -FP_ONE (full left) to +FP_ONE (full right).
    pub steer: Fixed,
    /// Handbrake flag (triggers drift initiation).
    pub handbrake: bool,
}

/// Dynamic physical state of a racing car.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct VehicleState {
    /// Position in fixed-point world coordinates.
    pub position: Vec2,
    /// Velocity vector in world coordinates.
    pub velocity: Vec2,
    /// Orientation angle (0..4096 = 0..360 deg).
    pub heading: u16,
    /// Visual rendering angle (differs from heading during drift).
    pub visual_angle: u16,
    /// Current speed magnitude (Fixed).
    pub speed: Fixed,
    /// Whether the car is currently in a drift slide.
    pub is_drifting: bool,
    /// Current engine RPM (0..8000 scale).
    pub engine_rpm: u16,
    /// Current gear (1..5).
    pub gear: u8,
    /// Car tuning profile.
    pub tuning: CarTuning,
}

impl Default for VehicleState {
    fn default() -> Self {
        VehicleState {
            position: Vec2::ZERO,
            velocity: Vec2::ZERO,
            heading: 0,
            visual_angle: 0,
            speed: Fixed::ZERO,
            is_drifting: false,
            engine_rpm: 1000,
            gear: 1,
            tuning: CarTuning::default(),
        }
    }
}

impl VehicleState {
    pub fn new(start_pos: Vec2, start_heading: u16, tuning: CarTuning) -> Self {
        VehicleState {
            position: start_pos,
            velocity: Vec2::ZERO,
            heading: start_heading,
            visual_angle: start_heading,
            speed: Fixed::ZERO,
            is_drifting: false,
            engine_rpm: 1000,
            gear: 1,
            tuning,
        }
    }

    /// Advances the vehicle physics simulation by one 60Hz tick.
    pub fn tick(&mut self, input: VehicleInput, surface: SurfaceType) {
        // 1. Steering calculation
        let base_turn_rate: u16 = 70; // angular units per tick at full lock (~6.1 deg/tick)
        let scaled_turn = self.tuning.scaled_turn_rate(base_turn_rate);

        if input.steer.raw() != 0 && self.speed > Fixed::from_raw(200) {
            let steer_delta =
                ((scaled_turn as i64 * input.steer.raw() as i64) >> math::FP_SHIFT) as i32;
            let new_heading = (self.heading as i32 + steer_delta) & 0x0FFF;
            self.heading = new_heading as u16;
        }

        // 2. Acceleration / Engine thrust
        let base_accel = Fixed::from_raw(60); // Base forward push per tick
        let accel = self.tuning.scaled_acceleration(base_accel);

        let forward_dir = Vec2::new(math::sin(self.heading), -math::cos(self.heading));

        if input.throttle > Fixed::ZERO {
            let thrust = accel * input.throttle;
            self.velocity = self.velocity + forward_dir.scale(thrust);
        }

        // 3. Braking & Reverse
        if input.brake > Fixed::ZERO {
            let brake_force = Fixed::from_raw(120) * input.brake;
            let current_speed = self.velocity.length();
            if current_speed > brake_force {
                let decel_ratio = (current_speed - brake_force) / current_speed;
                self.velocity = self.velocity.scale(decel_ratio);
            } else {
                self.velocity = Vec2::ZERO;
            }
        }

        // 4. Drift & Lateral Grip Dynamics
        let grip = surface.grip_factor();
        let is_handbrake = input.handbrake;

        let lateral_friction = if is_handbrake || surface == SurfaceType::OilSlick {
            self.is_drifting = true;
            Fixed::from_raw(3700) // Lower lateral hold -> slide
        } else {
            self.is_drifting = false;
            Fixed::from_raw(3950) // High lateral grip
        };

        // Decompose velocity into forward and lateral components
        let forward_speed = self.velocity.dot(forward_dir);
        let right_dir = Vec2::new(math::cos(self.heading), math::sin(self.heading));
        let lateral_speed = self.velocity.dot(right_dir);

        // Apply friction and surface damping
        let damped_forward = forward_speed * Fixed::from_raw(4080) * grip;
        let damped_lateral = lateral_speed * lateral_friction * grip;

        self.velocity = forward_dir.scale(damped_forward) + right_dir.scale(damped_lateral);

        // 5. Terminal Velocity Clamp
        let base_top_speed = Fixed::from_raw(14000); // Top speed in Q20.12
        let max_speed = self.tuning.scaled_top_speed(base_top_speed);
        let speed_mag = self.velocity.length();

        if speed_mag > max_speed {
            let scale_down = max_speed / speed_mag;
            self.velocity = self.velocity.scale(scale_down);
            self.speed = max_speed;
        } else {
            self.speed = speed_mag;
        }

        // 6. Integrate Position
        self.position = self.position + self.velocity.scale(Fixed::from_raw(120));

        // 7. Visual Angle Smoothing (drift slip angle visual representation)
        if self.is_drifting {
            // Visual angle leads or trails slightly
            self.visual_angle = self.heading.wrapping_add(128);
        } else {
            self.visual_angle = self.heading;
        }

        // 8. Dynamic Engine RPM & Gear calculation for audio
        let speed_ratio = (self.speed.raw() as i64 * 8000) / (base_top_speed.raw() as i64).max(1);
        let target_rpm = (1000 + speed_ratio).min(8000) as u16;
        self.engine_rpm = target_rpm;
        self.gear = match target_rpm {
            0..=2200 => 1,
            2201..=3800 => 2,
            3801..=5400 => 3,
            5401..=7000 => 4,
            _ => 5,
        };
    }
}
