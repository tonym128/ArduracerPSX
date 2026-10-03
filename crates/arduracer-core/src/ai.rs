//! Autonomous AI Racing Opponents and Navigation Logic.
//!
//! Controls AI rival vehicles following checkpoint racing lines with corner
//! deceleration, drift execution, and dynamic collision avoidance.

use crate::ai_profiles::AiProfile;
use crate::math::{Fixed, Vec2};
use crate::track::TrackDef;
use crate::vehicle::{VehicleInput, VehicleState};

/// Approximates atan2 in Binary Angular Measurement (0..4096 BAMs).
pub fn atan2_bams(dy: i32, dx: i32) -> u16 {
    if dx == 0 && dy == 0 {
        return 0;
    }
    let abs_y = dy.abs();
    let abs_x = dx.abs();

    // Octant ratio calculation (Q12) using i64 to prevent overflow
    let angle_octant = if abs_x >= abs_y {
        let ratio = ((abs_y as i64 * 4096) / (abs_x as i64).max(1)) as i32;
        (ratio * 512) / 4096
    } else {
        let ratio = ((abs_x as i64 * 4096) / (abs_y as i64).max(1)) as i32;
        1024 - (ratio * 512) / 4096
    };

    let base_angle = if dx >= 0 && dy >= 0 {
        // Quadrant 1 (0..90 deg -> 1024..2048 in screen space where Y is down)
        1024 + angle_octant
    } else if dx < 0 && dy >= 0 {
        // Quadrant 2
        2048 + (1024 - angle_octant)
    } else if dx < 0 && dy < 0 {
        // Quadrant 3
        3072 + angle_octant
    } else {
        // Quadrant 4
        1024 - angle_octant
    };

    (base_angle & 0x0FFF) as u16
}

/// An active AI rival racer on the grid.
#[derive(Clone, Debug)]
pub struct AiRacer {
    pub state: VehicleState,
    pub profile: AiProfile,
    pub target_gate_idx: u8,
    pub current_lap: u8,
    pub is_finished: bool,
}

impl AiRacer {
    pub fn new(start_pos: Vec2, start_heading: u16, profile: AiProfile) -> Self {
        let state = VehicleState::new(start_pos, start_heading, profile.tuning);
        AiRacer {
            state,
            profile,
            target_gate_idx: 0,
            current_lap: 1,
            is_finished: false,
        }
    }

    /// Evaluates navigation and obstacle avoidance to generate vehicle input.
    pub fn compute_input(&mut self, track: &TrackDef, other_positions: &[Vec2]) -> VehicleInput {
        if self.is_finished {
            return VehicleInput {
                throttle: Fixed::ZERO,
                brake: Fixed::ONE,
                steer: Fixed::ZERO,
                handbrake: false,
            };
        }

        let gate_count = track.checkpoint_count.max(1) as usize;
        let current_gate = &track.checkpoints[self.target_gate_idx as usize % gate_count];

        // Target waypoint is the center of the current checkpoint gate
        let target_x = ((current_gate.x as i32 * 64) + (current_gate.width as i32 * 32)) * 4096;
        let target_y = ((current_gate.y as i32 * 64) + (current_gate.height as i32 * 32)) * 4096;

        let diff_x = target_x - self.state.position.x.raw();
        let diff_y = target_y - self.state.position.y.raw();

        let desired_heading = atan2_bams(diff_y, diff_x);

        // Compute shortest angular error (-2048..+2048)
        let mut angle_err =
            (desired_heading as i32 - self.state.heading as i32 + 2048).rem_euclid(4096) - 2048;

        // Dynamic obstacle avoidance: check distance to other cars
        for other_pos in other_positions {
            let ox = (other_pos.x - self.state.position.x).raw() / 4096;
            let oy = (other_pos.y - self.state.position.y).raw() / 4096;
            let dist_sq = ox * ox + oy * oy;
            if dist_sq > 0 && dist_sq < 36 * 36 {
                // If car is ahead, steer slightly around
                if ox.abs() < 16 {
                    if ox >= 0 {
                        angle_err -= 256;
                    } else {
                        angle_err += 256;
                    }
                }
            }
        }

        // Steer proportionally to error
        let steer_raw = (angle_err * 6).clamp(-4096, 4096);
        let steer = Fixed::from_raw(steer_raw);

        // Throttle & braking logic based on turn severity and current speed
        let abs_err = angle_err.abs();
        let (throttle, brake, handbrake) = if self.state.speed < Fixed::from_int(3) {
            // Low speed / standstill: Always apply full throttle to get moving and steer
            (Fixed::ONE, Fixed::ZERO, false)
        } else if abs_err < 400 {
            // Straight stretch: Wide Open Throttle
            (Fixed::ONE, Fixed::ZERO, false)
        } else if abs_err < 800 {
            // Mild turn: Maintain high throttle
            (Fixed::from_raw(3200), Fixed::ZERO, false)
        } else if abs_err < 1400 {
            // Sharp corner: Tap handbrake to drift if personality allows
            let can_drift = self.profile.drift_tendency >= 4;
            (Fixed::from_raw(2048), Fixed::ZERO, can_drift)
        } else {
            // Hairpin turn at high speed: Moderate brake and turn in
            (Fixed::from_raw(1024), Fixed::from_raw(1024), true)
        };

        VehicleInput {
            throttle,
            brake,
            steer,
            handbrake,
        }
    }

    /// Advances the AI vehicle simulation by one tick.
    pub fn tick(&mut self, track: &TrackDef, other_positions: &[Vec2]) {
        let tx = (self.state.position.x.to_int() / 64) as u8;
        let ty = (self.state.position.y.to_int() / 64) as u8;
        let surface = track.surface_at(tx, ty);

        let input = self.compute_input(track, other_positions);
        self.state.tick(input, surface);

        // Checkpoint gate crossing detection
        let gate_count = track.checkpoint_count.max(1) as usize;
        let current_gate = &track.checkpoints[self.target_gate_idx as usize % gate_count];
        if current_gate.contains_tile(tx, ty) {
            let next_idx = (self.target_gate_idx + 1) % (track.checkpoint_count.max(1));
            if self.target_gate_idx > 0 && next_idx == 0 {
                self.current_lap += 1;
                if self.current_lap > 5 {
                    self.is_finished = true;
                }
            }
            self.target_gate_idx = next_idx;
        }
    }
}
