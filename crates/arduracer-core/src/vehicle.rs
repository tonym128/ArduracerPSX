//! Vehicle state, dynamics, and drift simulation.

use crate::drift::DriftState;
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
    /// Detailed drift state machine (boost tiers, slip angle).
    pub drift: DriftState,
    /// Remaining ticks of turbo boost (e.g. from drift release or boost pad).
    pub boost_ticks: u16,
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
            drift: DriftState::Grip,
            boost_ticks: 0,
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
            drift: DriftState::Grip,
            boost_ticks: 0,
            engine_rpm: 1000,
            gear: 1,
            tuning,
        }
    }

    /// Handles elastic/inelastic collision against a solid track barrier with surface normal.
    pub fn handle_barrier_collision(&mut self, normal: Vec2) {
        let v_dot_n = self.velocity.dot(normal);
        // Only bounce if heading into the wall
        if v_dot_n < Fixed::ZERO {
            // Restitution coefficient 0.35 (35% bounce)
            let restitution = Fixed::from_raw(1433);
            let impulse_mag = -(Fixed::ONE + restitution) * v_dot_n;
            self.velocity = self.velocity + normal.scale(impulse_mag);
            // Glancing speed penalty (30% speed scrub)
            self.velocity = self.velocity.scale(Fixed::from_raw(2867));
            self.speed = self.velocity.length();
            self.drift.trigger_spinout();
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

        // 2. Acceleration / Engine thrust & Boost
        let base_accel = Fixed::from_raw(60); // Base forward push per tick
        let accel = self.tuning.scaled_acceleration(base_accel);

        let forward_dir = Vec2::new(math::sin(self.heading), -math::cos(self.heading));

        if input.throttle > Fixed::ZERO {
            let thrust = accel * input.throttle;
            self.velocity = self.velocity + forward_dir.scale(thrust);
        }

        // Apply active turbo boost impulse
        if self.boost_ticks > 0 {
            self.boost_ticks -= 1;
            let boost_thrust = Fixed::from_raw(80);
            self.velocity = self.velocity + forward_dir.scale(boost_thrust);
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

        // 4. Drift & Lateral Traction Dynamics
        let grip = surface.grip_factor();

        // Handle drift initiation and release
        if input.handbrake && !self.drift.is_drifting() && self.speed > Fixed::from_raw(3000) {
            let drift_dir = if input.steer < Fixed::ZERO { -1 } else { 1 };
            self.drift.initiate_drift(drift_dir);
        } else if !input.handbrake && self.drift.is_drifting() {
            // Releasing handbrake releases mini-turbo boost!
            let boost_impulse = self.drift.release_drift();
            if boost_impulse > Fixed::ZERO {
                self.boost_ticks = 30; // 0.5s of turbo boost
                self.velocity = self.velocity + forward_dir.scale(boost_impulse);
            }
        }

        self.drift.tick(input.steer, self.speed, &self.tuning);
        self.is_drifting = self.drift.is_drifting();

        let lateral_friction = if self.is_drifting || surface == SurfaceType::OilSlick {
            Fixed::from_raw(3700) // Lower lateral hold -> slide
        } else {
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
        // Allow temporary exceedance when under turbo boost
        let effective_top = if self.boost_ticks > 0 {
            max_speed + Fixed::from_raw(3000)
        } else {
            max_speed
        };

        let speed_mag = self.velocity.length();
        if speed_mag > effective_top {
            let scale_down = effective_top / speed_mag;
            self.velocity = self.velocity.scale(scale_down);
            self.speed = effective_top;
        } else {
            self.speed = speed_mag;
        }

        // 6. Integrate Position
        self.position = self.position + self.velocity.scale(Fixed::from_raw(120));

        // 7. Visual Angle Smoothing (drift slip angle visual representation)
        match self.drift {
            DriftState::Drifting { slip_angle, .. } => {
                let visual = (self.heading as i32 + (slip_angle as i32)) & 0x0FFF;
                self.visual_angle = visual as u16;
            }
            DriftState::SpinOut { remaining_ticks } => {
                // Wild spin animation
                let spin_offset = remaining_ticks * 128;
                self.visual_angle = self.heading.wrapping_add(spin_offset) & 0x0FFF;
            }
            DriftState::Grip => {
                self.visual_angle = self.heading;
            }
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
