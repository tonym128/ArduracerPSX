//! Autonomous AI Racing Opponents and Navigation Logic.
//!
//! Rivals follow the track's ordered checkpoint route, brake for corner severity,
//! drift through tight apexes when their personality allows, and use the checkpoint
//! *mask* (not a rigid index) for lap progress - exactly like the player.

use crate::ai_profiles::AiProfile;
use crate::math::{Fixed, Vec2};
use crate::track::{TrackDef, TILE_SIZE};
use crate::tuning::CarTuning;
use crate::vehicle::{VehicleInput, VehicleState, BASE_TOP_SPEED, NITRO_MAX_TICKS};

/// Approximates atan2 in Binary Angular Measurement (0..4096 BAMs).
///
/// Convention: `dy` is the **world** Y delta (screen Y grows *downwards*), `dx`
/// the world X delta. The result is a heading for [`VehicleState`]'s forward
/// vector `(sin h, -cos h)`, so 0 = North (-Y), 1024 = East (+X).
/// Prefer [`heading_towards`] rather than calling this with hand-signed deltas.
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

/// Wraps an angular error into the shortest signed range (-2048..2048).
#[inline]
pub fn angle_error(from: u16, to: u16) -> i32 {
    (to as i32 - from as i32 + 2048).rem_euclid(4096) - 2048
}

/// Heading that points from `from` towards `to` in world space.
///
/// Removes the screen-space Y inversion footgun from every steering site.
#[inline]
pub fn heading_towards(from: Vec2, to: Vec2) -> u16 {
    atan2_bams((to.y - from.y).to_int(), (to.x - from.x).to_int())
}

/// Standstill speed below which the AI simply applies full throttle.
const CREEP_SPEED: Fixed = Fixed::from_int(3);
/// Distance (world units) over which the AI ramps its corner speed limit in.
const CORNER_LEAD_UNITS: i32 = 448;

/// An active AI rival racer on the grid.
#[derive(Clone, Debug)]
pub struct AiRacer {
    pub state: VehicleState,
    pub profile: AiProfile,
    /// Index of the next route node the rival is chasing.
    pub target_gate_idx: u8,
    /// Checkpoints touched this lap (mirrors `LapTimer::checkpoint_mask`).
    pub checkpoint_mask: u16,
    pub current_lap: u8,
    pub is_finished: bool,
    /// Set once the rival has left the grid (mirrors the lap timer's arming).
    pub left_start_gate: bool,
}

impl AiRacer {
    pub fn new(start_pos: Vec2, start_heading: u16, profile: AiProfile) -> Self {
        let state = VehicleState::new(start_pos, start_heading, profile.tuning);
        AiRacer {
            state,
            profile,
            target_gate_idx: 0,
            checkpoint_mask: 0,
            current_lap: 1,
            is_finished: false,
            left_start_gate: false,
        }
    }

    /// Places the rival on the grid at a world-space offset from the pole position.
    pub fn offset_from_pole(
        &mut self,
        start_pos: Vec2,
        start_heading: u16,
        forward: i32,
        lateral: i32,
    ) {
        let fx = crate::math::cos(start_heading);
        let fy = crate::math::sin(start_heading);
        let lx = -crate::math::sin(start_heading);
        let ly = crate::math::cos(start_heading);
        self.state.position = start_pos
            + Vec2::new(
                fx * Fixed::from_int(forward) + lx * Fixed::from_int(lateral),
                fy * Fixed::from_int(forward) + ly * Fixed::from_int(lateral),
            );
        self.state.heading = start_heading;
        self.state.visual_angle = start_heading;
        self.state.velocity = Vec2::ZERO;
        self.state.speed = Fixed::ZERO;
    }

    fn target_position(&self, track: &TrackDef) -> Vec2 {
        TrackDef::gate_centre(&track.route_node(self.target_gate_idx as usize))
    }

    /// Corner severity between the current target and the one after it, in BAMs.
    fn upcoming_corner(&self, track: &TrackDef) -> i32 {
        let n = track.route_len();
        let a = TrackDef::gate_centre(&track.route_node(self.target_gate_idx as usize % n));
        let b = TrackDef::gate_centre(&track.route_node((self.target_gate_idx as usize + 1) % n));
        let heading_a = heading_towards(a, b);
        let heading_here = heading_towards(self.state.position, a);
        angle_error(heading_here, heading_a).abs()
    }

    /// Evaluates navigation and obstacle avoidance to generate vehicle input.
    ///
    /// Two-loop arcade controller: a proportional steering loop on the heading
    /// error to the next route node, plus a corner-speed governor that inspects
    /// how hard the road bends *after* that node so rivals brake before the apex
    /// instead of stamping on the brakes mid-corner.
    pub fn compute_input(&mut self, track: &TrackDef, other_positions: &[Vec2]) -> VehicleInput {
        if self.is_finished {
            return VehicleInput {
                throttle: Fixed::ZERO,
                brake: Fixed::ONE,
                steer: Fixed::ZERO,
                handbrake: false,
                nitro: false,
            };
        }

        let target = self.target_position(track);
        let desired_heading = heading_towards(self.state.position, target);
        let dist = (target - self.state.position).length();

        let mut angle_err = angle_error(self.state.heading, desired_heading);
        let abs_err = angle_err.abs();

        // Dynamic obstacle avoidance: nudge around cars in the near field.
        for other_pos in other_positions {
            let ox = (other_pos.x - self.state.position.x).to_int();
            let oy = (other_pos.y - self.state.position.y).to_int();
            let dist_sq = ox * ox + oy * oy;
            if dist_sq == 0 || dist_sq > 30 * 30 {
                continue;
            }
            // Only react to cars roughly alongside/ahead on the racing line.
            if ox.abs() < 14 && oy.abs() < 20 {
                angle_err -= if ox >= 0 { 220 } else { -220 };
            }
        }

        // Steering: proportional, saturating at full lock.
        let steer = Fixed::from_raw((angle_err * 6).clamp(-Fixed::ONE.raw(), Fixed::ONE.raw()));

        // --- Corner-speed governor ---------------------------------------------
        let corner = self.upcoming_corner(track);
        let tightness = corner.clamp(0, 2048);
        // A full-lock corner caps the rival at ~38% of its top speed.
        let corner_cap = Fixed::from_raw(4096 - (tightness as i64 * 2539 / 2048) as i32);

        // Ease the restriction in with distance so rivals brake *before* the
        // corner rather than on entry.
        let lead = dist.to_int().clamp(0, CORNER_LEAD_UNITS);
        let blend = (lead as i64 * 4096) / CORNER_LEAD_UNITS as i64;
        let top = BASE_TOP_SPEED * CarTuning::scale_factor(self.state.tuning.top_speed);
        let mut limit = top - (top - corner_cap) * Fixed::from_raw(blend as i32);

        // Personality: aggression brakes later, and therefore carries more speed.
        limit = limit * Fixed::from_raw(4096 + (self.profile.aggression as i32 - 5) * 130);

        let (throttle, brake, handbrake) = if self.state.speed < CREEP_SPEED {
            // Standing start: always get moving.
            (Fixed::ONE, Fixed::ZERO, false)
        } else if self.state.speed > limit + Fixed::from_raw(400) {
            // Well over the corner speed: brake hard.
            (Fixed::ZERO, Fixed::from_raw(3600), false)
        } else if self.state.speed > limit {
            // Slightly over: lift and coast into the apex.
            (Fixed::from_raw(600), Fixed::from_raw(1100), false)
        } else if angle_err.abs() > 700 && dist.to_int() < 220 && self.profile.drift_tendency >= 7 {
            // Only true apex hunters (BLAZE) hang the tail out; a brief stab.
            (Fixed::from_raw(2600), Fixed::ZERO, true)
        } else {
            // On the limit: wide-open throttle.
            (Fixed::ONE, Fixed::ZERO, false)
        };

        VehicleInput {
            throttle,
            brake,
            steer,
            handbrake,
            // Rivals cash their nitro on the straights, not mid-corner.
            nitro: self.state.nitro_charge > NITRO_MAX_TICKS / 3 && abs_err < 200,
        }
    }

    /// Registers gate touches for the current lap.
    fn update_route_progress(&mut self, track: &TrackDef, tx: i32, ty: i32) {
        let route = track.route_len();
        let checkpoint_count = track.checkpoint_count as usize;
        let tile = (
            tx.clamp(0, (track.width as i32) - 1) as u8,
            ty.clamp(0, (track.height as i32) - 1) as u8,
        );

        // Start/finish arming.
        if !track.start_gate.contains_tile(tile.0, tile.1) {
            self.left_start_gate = true;
        }

        // Advance along the racing line when the current node is reached.
        let node = self.target_gate_idx as usize % route;
        if track.route_node(node).contains_tile(tile.0, tile.1) {
            let next = node + 1;
            if next >= route {
                // Wrapped past the start/finish: a full circuit is done.
                self.on_lap_complete();
                self.target_gate_idx = 0;
            } else {
                self.target_gate_idx = next as u8;
            }
        }

        // Track checkpoint coverage so rivals can be ranked against the player.
        for i in 0..checkpoint_count {
            if self.checkpoint_mask & (1 << i) != 0 {
                continue;
            }
            if track.checkpoints[i].contains_tile(tile.0, tile.1) {
                self.checkpoint_mask |= 1 << i;
            }
        }
    }

    fn on_lap_complete(&mut self) {
        if self.left_start_gate {
            self.current_lap += 1;
            if self.current_lap > 5 {
                self.is_finished = true;
            }
        }
        self.checkpoint_mask = 0;
    }

    /// Advances the AI vehicle simulation by one tick.
    pub fn tick(&mut self, track: &TrackDef, other_positions: &[Vec2]) {
        let input = self.compute_input(track, other_positions);
        self.state.tick_on_track(input, track);

        let tx = self.state.position.x.to_int() / TILE_SIZE;
        let ty = self.state.position.y.to_int() / TILE_SIZE;
        self.update_route_progress(track, tx, ty);
    }

    /// Number of checkpoints the rival has cleared this lap (for HUD + standings).
    #[inline]
    pub fn checkpoints_cleared(&self) -> u32 {
        self.checkpoint_mask.count_ones()
    }
}
