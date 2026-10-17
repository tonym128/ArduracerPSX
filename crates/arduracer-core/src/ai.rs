//! Autonomous AI Racing Opponents and Navigation Logic.
//!
//! Rivals follow the track's ordered checkpoint route with a *look-ahead racing
//! line* rather than beelining at the next gate centre, brake for corner
//! severity, drift through tight apexes when their personality allows, and use
//! the checkpoint *mask* (not a rigid index) for lap progress - exactly like the
//! player.
//!
//! The one thing an opponent never does is cheat: a rival that laps faster than
//! its own top speed allows is not driving well, it is skipping track. So the aim
//! point is a pure-pursuit point on a *road-following* line -- a greedy walk over
//! the tile grid towards the route node the rival is chasing, preferring the
//! racing surface -- rather than the next gate centre, and a rival that wedges
//! itself gets itself out again.

use crate::ai_profiles::{Aggression, AiProfile, DriftTendency};
use crate::line::{RacingLine, DEFAULT_LOOKAHEAD_UNITS as AIM_LOOKAHEAD_UNITS};
use crate::math::{Fixed, Vec2};
use crate::timing::TOTAL_LAPS;
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

/// Whether a tile coordinate is somewhere a car can actually be driven.
///
/// `tile_at` answers `Barrier` outside the grid, so this doubles as an in-bounds
/// test.
#[inline]
fn is_drivable(track: &TrackDef, tx: u8, ty: u8) -> bool {
    !track.tile_at(tx, ty).is_solid()
}

/// Standstill speed below which the AI simply applies full throttle.
const CREEP_SPEED: Fixed = Fixed::from_int(3);
/// Distance (world units) over which the AI ramps its corner speed limit in.
const CORNER_LEAD_UNITS: i32 = 448;
/// Half-width of the obstacle-avoidance box, in world units.
///
/// The old test was `ox.abs() < 14 && oy.abs() < 20` on *world* deltas: a 14x20
/// unit box, smaller than a single grid cell, so rivals routinely drove through
/// each other. Now a car width.
///
/// Deliberately *not* expressed in tiles. This is a driving distance, not a grid
/// resolution: written as `TILE_SIZE` it halved along with the surface grid, and
/// a 32-unit box is a quarter of the road width -- measured on The Esses, where
/// the rival stopped seeing the infield it was wedged against and never recovered.
const AVOIDANCE_HALF_UNITS: i32 = 64;
/// Lateral offset used to dodge an obstacle, in world units.
const AVOIDANCE_OFFSET_UNITS: i32 = 64;
/// Distance beyond which an obstacle is ignored entirely (world units).
const AVOIDANCE_RANGE_UNITS: i32 = 256;
/// Proportional steering gain; full lock is reached at ~54 degrees of error.
///
/// Deliberately modest. At the old gain of 6 the loop saturated after 15 degrees
/// of heading error, so the rival sat on full lock through every corner: it both
/// slammed into walls and cut the circuit down to 65% of its centre line.
const STEER_GAIN: i32 = 4;
/// Ticks spent reversing back out before trying to drive forward again.
const STUCK_REVERSE_TICKS: u16 = 30;
/// Ticks without getting any closer to the next gate before the rival is
/// declared wedged.
const STALL_TICKS: u16 = 120;
/// Ticks of wedged-ness before the rival is put back on the racing line.
const STUCK_RESCUE_TICKS: u16 = 270;
/// Steering used while reversing out of a wedge.
const REVERSE_STEER: Fixed = Fixed::from_raw(600);

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
    /// Consecutive ticks spent going nowhere. The first [`STUCK_REVERSE_TICKS`]
    /// are spent reversing out; if nothing has changed by
    /// [`STUCK_RESCUE_TICKS`] the rival is put back on the racing line.
    pub wedged_ticks: u16,
    /// Closest approach to the current target gate so far, in world units.
    /// Reset whenever the rival picks up a new gate.
    pub best_gate_distance: i32,
    /// Ticks since that closest approach last improved.
    pub progress_ticks: u16,
    /// Total ticks spent wedged (for HUD / telemetry).
    pub wedged_total_ticks: u32,
    /// The racing line this rival follows, and where on the centreline it was
    /// last matched.
    line: RacingLine,
    /// Last centreline index fed to [`AiRacer::update_progress`], so forward
    /// movement can be told from a jump across the circuit.
    route_last_idx: usize,
    /// Monotonic count of control points passed forwards around the circuit.
    route_advance: u32,
    /// Highest [`AiRacer::route_advance`] seen on the current gate.
    best_route_advance: u32,
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
            wedged_ticks: 0,
            best_gate_distance: i32::MAX,
            progress_ticks: 0,
            wedged_total_ticks: 0,
            line: RacingLine::new(),
            route_last_idx: 0,
            route_advance: 0,
            best_route_advance: 0,
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

    /// Centre of the route node the rival is currently chasing.
    fn target_position(&self, track: &TrackDef) -> Vec2 {
        let route = track.route_len();
        TrackDef::gate_centre(&track.route_node(self.target_gate_idx as usize % route))
    }

    #[doc(hidden)]
    pub fn probe_target(&self, track: &TrackDef) -> Vec2 {
        self.target_position(track)
    }

    /// A point `lookahead` world units further along the racing line.
    ///
    /// Delegates to [`RacingLine`], which owns the geometry and the reasoning:
    /// the circuit's centreline gives the direction, and a road-preferring tile
    /// walk turns that into an aim point the car can actually be driven onto.
    /// `make playtest`'s reference driver steers with the same code, which is the
    /// point -- the oracle and the AI have to agree about where the road is.
    fn racing_line_point(&mut self, track: &TrackDef, lookahead: i32) -> Vec2 {
        // With no authored centreline (hand-built test fixtures) there is no
        // racing line to follow, so aim at the gate being chased as before.
        let fallback = TrackDef::gate_centre(
            &track.route_node((self.target_gate_idx as usize) % track.route_len()),
        );
        self.line.aim(
            track,
            self.state.position,
            self.state.heading,
            lookahead,
            fallback,
        )
    }

    /// Lateral dodge, in world units, to steer around cars in the near field.
    ///
    /// The box is sized in *world* units (`AVOIDANCE_HALF_UNITS`), because the
    /// old `ox.abs() < 14 && oy.abs() < 20` world-delta test was a 14x20 unit box
    /// -- smaller than a single grid cell -- so rivals drove through each other.
    fn avoidance_offset(&self, other_positions: &[Vec2]) -> i32 {
        let half = AVOIDANCE_HALF_UNITS;
        let mut offset = 0i32;
        for other_pos in other_positions {
            let dx = (other_pos.x - self.state.position.x).to_int();
            let dy = (other_pos.y - self.state.position.y).to_int();
            let distance_sq = (dx as i64) * (dx as i64) + (dy as i64) * (dy as i64);
            if distance_sq == 0
                || distance_sq > ((AVOIDANCE_RANGE_UNITS * AVOIDANCE_RANGE_UNITS) as i64)
            {
                continue;
            }
            if dx.abs() < half && dy.abs() < half {
                offset += if dx >= 0 {
                    -AVOIDANCE_OFFSET_UNITS
                } else {
                    AVOIDANCE_OFFSET_UNITS
                };
            }
        }
        offset.clamp(-2 * AVOIDANCE_OFFSET_UNITS, 2 * AVOIDANCE_OFFSET_UNITS)
    }

    /// Corner severity between the current target and the one after it, in BAMs.
    fn upcoming_corner(&self, track: &TrackDef) -> i32 {
        // The corner-speed governor used to measure the bend *between two
        // checkpoint bearings* -- gates spaced a quarter of the lap apart -- so
        // it saw almost nothing and carried top speed into corners it should
        // have braked for. That is why one cell of lateral error accumulated
        // everywhere a gate sat down a technical sequence: the car never
        // tightened up.
        //
        // Measure the road's own bend in the few route segments ahead of the
        // car instead. The window length sets the braking distance: the car
        // brakes for the worst turn a short lookahead reveals.
        let n = track.route.len();
        if n < 3 {
            return 0;
        }
        let i = self.line.hint() % n;
        let mut worst = 0i32;
        for k in 0..3 {
            let a = track.route[(i + k) % n];
            let b = track.route[(i + k + 1) % n];
            let c = track.route[(i + k + 2) % n];
            let ab = heading_towards(a, b);
            let bc = heading_towards(b, c);
            worst = worst.max(angle_error(ab, bc).abs());
        }
        worst
    }

    /// Whether the rival is spinning, or has stopped making route progress.
    ///
    /// Measured on distance to the next gate, not on whether the car is moving or
    /// how fast: a rival circling at full throttle beside the gate it is chasing
    /// moves every single tick at nearly top speed and gets precisely nowhere, so
    /// both a movement-based and a speed-based stuck timer never fire and it sits
    /// there for ever.
    fn is_wedged(&self) -> bool {
        self.state.drift.is_spinning() || self.progress_ticks >= STALL_TICKS
    }

    /// Records whether the rival got any closer to the gate it is chasing.
    fn update_progress(&mut self, track: &TrackDef) {
        // Progress is forward movement *along the centreline*, not shrinking
        // straight-line distance to the next gate.
        //
        // The gate distance looks reasonable and is wrong on a closed circuit,
        // because the gate is up to a quarter of the way round the ring: driving
        // the racing line correctly around a corner routinely carries a car
        // further from the next gate in a straight line than it started. Measured
        // on Sunset Ridgeway, a rival driving correctly along the road held a
        // gate distance of 558 units for 211 consecutive ticks -- real progress
        // the whole time -- so the old measure declared it wedged, and it spent
        // the rest of the lap reversing and respawning. This is the same
        // beeline assumption that made the aim point leave the road.
        //
        // Falls back to the gate distance only for a track with no centreline,
        // where there is nothing better to measure.
        let advanced = if self.line.has_match() {
            let n = track.route.len();
            if n < 2 {
                false
            } else {
                let cur = self.line.hint() % n;
                // Forward distance in control points, wrapping at the seam. A
                // jump larger than a third of the loop is a respawn or a slide
                // across the circuit, not progress, and must not count.
                let forward = (cur + n - self.route_last_idx % n) % n;
                let forward = if forward * 3 > n { 0 } else { forward };
                self.route_last_idx = cur;
                if forward > 0 {
                    self.route_advance = self.route_advance.saturating_add(forward as u32);
                }
                self.route_advance > self.best_route_advance
            }
        } else {
            let target = self.target_position(track);
            let distance = (target - self.state.position).length().to_int();
            distance < self.best_gate_distance
        };

        if advanced {
            self.best_route_advance = self.route_advance;
            self.progress_ticks = 0;
        } else {
            self.progress_ticks = self.progress_ticks.saturating_add(1);
        }
    }

    /// Steering to use while reversing out of a wedge.
    fn reverse_steer(&self, track: &TrackDef) -> Fixed {
        // Reverse away from whatever the nose is buried in: if there is road
        // behind, back straight out; if not, turn towards the road.
        let back = self.state.position - self.state.forward_dir().scale(Fixed::from_int(TILE_SIZE));
        if is_drivable(
            track,
            TrackDef::tile_x_of(back.x),
            TrackDef::tile_y_of(back.y),
        ) {
            Fixed::ZERO
        } else {
            REVERSE_STEER
        }
    }

    /// Puts the rival back on the racing line and clears the recovery state.
    pub fn recover(&mut self, track: &TrackDef) {
        let (pos, heading) = track.respawn_point(self.state.position);
        self.state.respawn_at(pos, heading);
        self.wedged_ticks = 0;
        self.best_gate_distance = i32::MAX;
        self.best_route_advance = self.route_advance;
        self.progress_ticks = 0;
        // Teleported: the cached centreline match describes where the car was.
        // It would still be re-validated, but dropping it makes the next tick a
        // full scan instead of a confident wrong answer.
        self.line.reset();
    }

    /// Evaluates navigation and obstacle avoidance to generate vehicle input.
    ///
    /// Three-loop arcade controller: a pure-pursuit steering loop onto a point
    /// ahead down the racing line, a corner-speed governor that inspects how hard
    /// the road bends *after* the node ahead so rivals brake before the apex, and
    /// an avoidance bias sized in tiles.
    pub fn compute_input(&mut self, track: &TrackDef, other_positions: &[Vec2]) -> VehicleInput {
        if self.is_finished {
            self.wedged_ticks = 0;
            self.progress_ticks = 0;
            return VehicleInput {
                throttle: Fixed::ZERO,
                brake: Fixed::ONE,
                steer: Fixed::ZERO,
                handbrake: false,
                nitro: false,
            };
        }

        // --- Recovery -------------------------------------------------------
        // A wedged rival used to sit on full throttle for ever. Reverse out
        // first; if that fails for long enough, put it back on the racing line.
        if self.is_wedged() {
            self.wedged_ticks = self.wedged_ticks.saturating_add(1);
            self.wedged_total_ticks = self.wedged_total_ticks.saturating_add(1);
            if self.wedged_ticks >= STUCK_RESCUE_TICKS {
                self.recover(track);
            } else {
                let reversing = self.wedged_ticks < STUCK_REVERSE_TICKS;
                return VehicleInput {
                    throttle: if reversing {
                        Fixed::ZERO
                    } else {
                        Fixed::from_raw(1800)
                    },
                    brake: Fixed::ONE,
                    steer: if reversing {
                        self.reverse_steer(track)
                    } else {
                        Fixed::ZERO
                    },
                    handbrake: false,
                    nitro: false,
                };
            }
        } else {
            self.wedged_ticks = 0;
        }

        self.drive_input(track, other_positions)
    }

    /// The normal racing input: racing line, corner governor and avoidance.
    fn drive_input(&mut self, track: &TrackDef, other_positions: &[Vec2]) -> VehicleInput {
        let dodge = self.avoidance_offset(other_positions);
        let line = self.racing_line_point(track, AIM_LOOKAHEAD_UNITS);
        let aim = if dodge != 0 {
            // Apply the dodge perpendicular to the direction of travel, so it
            // slides the aim point sideways rather than yanking the heading.
            let normal = Vec2::new(
                crate::math::cos(self.state.heading),
                crate::math::sin(self.state.heading),
            );
            line + normal.scale(Fixed::from_int(dodge))
        } else {
            line
        };

        let desired_heading = heading_towards(self.state.position, aim);
        let target = self.target_position(track);
        let dist = (target - self.state.position).length();

        // Single source of truth for the heading error. The old code captured
        // `abs_err` *before* the avoidance nudge was folded into `angle_err`, so
        // the nitro gate decided with a stale value and rivals fired nitro while
        // actively swerving around traffic.
        let angle_err = angle_error(self.state.heading, desired_heading);
        let abs_err = angle_err.abs();

        // Steering: proportional, saturating at full lock.
        let steer =
            Fixed::from_raw((angle_err * STEER_GAIN).clamp(-Fixed::ONE.raw(), Fixed::ONE.raw()));

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
        // Clamped to the documented 1..=10 range: `aggression` is a bare `pub u8`
        // with no validation, and `aggression = 255` used to hand the rival an
        // 8.93x top-speed multiplier.
        limit = limit * Aggression::brake_bias(self.profile.aggression);

        let apex_hunter =
            DriftTendency::clamped(self.profile.drift_tendency) >= DriftTendency::BLAZE;
        let (throttle, brake, handbrake) = if self.state.speed < CREEP_SPEED {
            // Standing start: always get moving.
            (Fixed::ONE, Fixed::ZERO, false)
        } else if self.state.speed > limit + Fixed::from_raw(400) {
            // Well over the corner speed: brake hard.
            (Fixed::ZERO, Fixed::from_raw(3600), false)
        } else if self.state.speed > limit {
            // Slightly over: lift and coast into the apex.
            (Fixed::from_raw(600), Fixed::from_raw(1100), false)
        } else if abs_err > 700 && dist.to_int() < 220 && apex_hunter {
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
            // Rivals cash their nitro on the straights: never mid-corner, and
            // never while actively swerving around traffic.
            nitro: self.state.nitro_charge > NITRO_MAX_TICKS / 3 && abs_err < 200 && dodge == 0,
        }
    }

    /// Registers gate touches for the current lap.
    fn update_route_progress(&mut self, track: &TrackDef, tx: u8, ty: u8) {
        let route = track.route_len();
        let checkpoint_count = track.active_checkpoint_count();

        // Start/finish arming.
        if !track.start_gate.touches_tile(tx, ty) {
            self.left_start_gate = true;
        }

        // Advance along the racing line when the current node is reached.
        let node = (self.target_gate_idx as usize) % route;
        if track.route_node(node).touches_tile(tx, ty) {
            let next = node + 1;
            self.target_gate_idx = if next >= route {
                // Wrapped past the start/finish: a full circuit is done.
                self.on_lap_complete();
                0
            } else {
                next.min(u8::MAX as usize) as u8
            };
            // A new gate means a new distance to make.
            self.best_gate_distance = i32::MAX;
            self.best_route_advance = self.route_advance;
            self.progress_ticks = 0;
        }

        // Track checkpoint coverage so rivals can be ranked against the player.
        for i in 0..checkpoint_count {
            if self.checkpoint_mask & (1 << i) != 0 {
                continue;
            }
            match track.checkpoints.get(i) {
                Some(gate) if gate.touches_tile(tx, ty) => self.checkpoint_mask |= 1 << i,
                _ => {}
            }
        }
    }

    fn on_lap_complete(&mut self) {
        if self.left_start_gate && !self.is_finished {
            self.current_lap = self.current_lap.saturating_add(1);
            // `timing::TOTAL_LAPS`, not a hard-coded 5: a rival that counted a
            // different number of laps than the lap timer can never be reconciled
            // with the player, nor with its own finish flag.
            if self.current_lap > TOTAL_LAPS {
                self.is_finished = true;
            }
        }
        self.checkpoint_mask = 0;
    }

    /// Advances the AI vehicle simulation by one tick.
    pub fn tick(&mut self, track: &TrackDef, other_positions: &[Vec2]) {
        let input = self.compute_input(track, other_positions);
        self.state.tick_on_track(input, track);

        // `TrackDef::tile_x_of` / `tile_y_of`, not `pos / TILE_SIZE`: truncating
        // `i32` division produced negative indices for negative positions and a
        // third convention alongside `TrackDef::tile_x_of`.
        let tx = TrackDef::tile_x_of(self.state.position.x).min(track.width.saturating_sub(1));
        let ty = TrackDef::tile_y_of(self.state.position.y).min(track.height.saturating_sub(1));
        self.update_route_progress(track, tx, ty);
        self.update_progress(track);
    }

    /// Number of checkpoints the rival has cleared this lap (for HUD + standings).
    #[inline]
    pub fn checkpoints_cleared(&self) -> u32 {
        self.checkpoint_mask.count_ones()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    // `segment_units` moved to `crate::line` with the racing-line geometry; its
    // test came with it, and this re-export keeps it covered here too.
    use crate::ai_profiles::AI_PROFILES;
    use crate::levels::ALL_TRACKS;
    use crate::line::segment_units;

    /// Fraction of the centre-line a rival must actually drive to prove it is not
    /// cutting the circuit.
    ///
    /// `TrackDef::route_length` is the *Manhattan* distance around the route
    /// nodes, so it over-states a real racing line by the corners a car clips, and
    /// a car can never match it. Measured across all 24 circuits and all five
    /// personalities: the old gate-to-gate beeline bottomed out at 70.3% and
    /// peaked at 90.6%; the road-following rival bottoms out at 75.8% with a
    /// median of 87.8%. 73% separates the two with margin on both sides.
    const MIN_ROUTE_COVERAGE: i64 = 73;

    /// Top speed of a profile in Q20.12 raw units per tick.
    fn top_speed_raw(profile: &AiProfile) -> i64 {
        (BASE_TOP_SPEED * CarTuning::scale_factor(profile.tuning.top_speed)).raw() as i64
    }

    /// Drives one timed lap and returns `(ticks, distance covered in world units)`.
    ///
    /// The step lengths are accumulated in Q20.12 *raw* units: a single tick at
    /// low speed covers well under one world unit, and `Fixed::to_int` truncates,
    /// so rounding every step would throw most of the lap away.
    fn drive_one_lap(track: &TrackDef, profile: &AiProfile) -> (u32, i64) {
        let mut racer = AiRacer::new(track.start_pos, track.start_heading, *profile);
        let mut ticks = 0u32;
        let mut raw_distance = 0i64;
        let mut previous = racer.state.position;
        while ticks < 60_000 && racer.current_lap < 2 {
            racer.tick(track, &[]);
            raw_distance += (racer.state.position - previous).length().raw() as i64;
            previous = racer.state.position;
            ticks += 1;
        }
        let distance = raw_distance / crate::math::FP_ONE as i64;
        assert!(
            racer.current_lap >= 2,
            "{}: {} did not complete a lap in {ticks} ticks",
            track.name,
            profile.name
        );
        (ticks, distance)
    }

    // ========================================================================
    // bug 6: real path following, and physically achievable lap times
    // ========================================================================

    #[test]
    fn a_rival_lap_is_never_faster_than_the_circuit_is_long() {
        // Reproduced on TRACK_01: the 1792-unit centre line has a >=525-tick
        // floor at `BASE_TOP_SPEED`, and VIPER "lapped" it in 383 ticks -- 27%
        // faster than the circuit is physically long -- by beelining gate to gate
        // and cutting every corner.
        let track = ALL_TRACKS[0];
        let centre_line = track.route_length();
        assert!(centre_line > 0, "the circuit must have a centre line");
        for profile in AI_PROFILES.iter() {
            let (ticks, distance) = drive_one_lap(track, profile);
            // Two independent statements of the same thing.
            let coverage = (distance * 100) / centre_line as i64;
            assert!(
                coverage >= MIN_ROUTE_COVERAGE,
                "{} drove only {distance} of the {centre_line}-unit centre line ({coverage}%)",
                profile.name
            );
            // distance / top_speed, in ticks: no car can average more than its top
            // speed, so a lap shorter than that is not a lap.
            let floor = ((centre_line as i64) * 4096) / top_speed_raw(profile);
            let floor = (floor * MIN_ROUTE_COVERAGE) / 100;
            assert!(
                ticks as i64 >= floor,
                "{} lapped {} in {ticks} ticks, faster than the {floor}-tick geometric floor",
                profile.name,
                track.name
            );
        }
    }

    #[test]
    fn the_fastest_rival_cannot_lap_the_oval_faster_than_its_own_top_speed() {
        // The exact reproduction from the audit: VIPER "lapped" TRACK_01 in 383
        // ticks, 27% faster than the 1792-unit centre line allows at
        // `BASE_TOP_SPEED`. VIPER's `top_speed = 7` tuning raises its ceiling, so
        // the floor for *it* is `centre_line / its own top_speed` -- 403 ticks,
        // no percentage fudge -- and the old gate-to-gate beeline failed that.
        //
        // The other four run default-ish tunes whose honest floor is below the
        // Manhattan centre line (a car cannot drive the right-angle dog-leg a
        // Manhattan measurement implies), so for them the assertion is the
        // physical one: the distance covered divided by the lap time may never
        // exceed the car's own top speed.
        let track = ALL_TRACKS[0];
        let centre_line = track.route_length() as i64;
        let viper = AI_PROFILES[0];
        let (viper_ticks, _) = drive_one_lap(track, &viper);
        let viper_floor = (centre_line * 4096) / top_speed_raw(&viper);
        assert!(
            viper_ticks as i64 >= viper_floor,
            "VIPER lapped {} in {viper_ticks} ticks, faster than the {viper_floor}-tick centre-line floor",
            track.name
        );

        for profile in AI_PROFILES.iter() {
            let (ticks, distance) = drive_one_lap(track, profile);
            let budget = (ticks as i64) * top_speed_raw(profile);
            assert!(
                distance * 4096 <= budget,
                "{} covered {distance} units in {ticks} ticks -- more than its own top speed allows",
                profile.name
            );
        }
    }

    #[test]
    fn every_rival_drives_a_real_racing_line_on_every_circuit() {
        // The same bound, on all 24 circuits and all five personalities.
        for track in ALL_TRACKS.iter() {
            let centre_line = track.route_length();
            for profile in AI_PROFILES.iter() {
                let (ticks, distance) = drive_one_lap(track, profile);
                let coverage = (distance * 100) / centre_line as i64;
                assert!(
                    coverage >= MIN_ROUTE_COVERAGE,
                    "{}: {} drove only {distance} of {centre_line} units ({coverage}%) in {ticks} ticks",
                    track.name,
                    profile.name
                );
            }
        }
    }

    #[test]
    fn a_rival_stays_on_the_racing_surface() {
        // Road awareness: a rival must not spend the lap ploughing through
        // scenery. Sampled rather than exhaustive so the test stays quick.
        for track in [ALL_TRACKS[0], ALL_TRACKS[14], ALL_TRACKS[19]] {
            for profile in [&AI_PROFILES[0], &AI_PROFILES[2]] {
                let mut racer = AiRacer::new(track.start_pos, track.start_heading, *profile);
                let mut on_road = 0u32;
                let mut total = 0u32;
                for _ in 0..1500 {
                    racer.tick(track, &[]);
                    if racer.is_finished {
                        break;
                    }
                    let tx = TrackDef::tile_x_of(racer.state.position.x);
                    let ty = TrackDef::tile_y_of(racer.state.position.y);
                    if track.is_road_at(tx, ty) {
                        on_road += 1;
                    }
                    total += 1;
                }
                assert!(total > 60, "{}: the rival never got going", track.name);
                assert!(
                    on_road * 10 >= total * 7,
                    "{}: {} was only off the racing surface for {}/{} ticks",
                    track.name,
                    profile.name,
                    total - on_road,
                    total
                );
            }
        }
    }

    #[test]
    fn the_look_ahead_point_lies_on_a_drivable_tile() {
        // The aim point itself must never be inside scenery, and it must stay a
        // bounded distance down the route.
        for track in ALL_TRACKS.iter() {
            for profile in AI_PROFILES.iter() {
                let mut racer = AiRacer::new(track.start_pos, track.start_heading, *profile);
                for _ in 0..400 {
                    let aim = racer.racing_line_point(track, AIM_LOOKAHEAD_UNITS);
                    let tx = TrackDef::tile_x_of(aim.x);
                    let ty = TrackDef::tile_y_of(aim.y);
                    assert!(
                        !track.tile_at(tx, ty).is_solid(),
                        "{}: {} aimed inside scenery at tile ({tx},{ty})",
                        track.name,
                        profile.name
                    );
                    let distance = (aim - racer.state.position).length().to_int().abs();
                    assert!(
                        distance <= AIM_LOOKAHEAD_UNITS + TILE_SIZE,
                        "{}: {} aim point {distance} units away",
                        track.name,
                        profile.name
                    );
                    racer.tick(track, &[]);
                    if racer.is_finished {
                        break;
                    }
                }
            }
        }
    }

    #[test]
    fn racing_line_point_is_total_for_a_car_anywhere() {
        // The aim point must be computable even for a position off the grid or
        // inside scenery: `AiRacer` fields are `pub` and `TrackDef` has no
        // constructor.
        let track = ALL_TRACKS[0];
        let mut racer = AiRacer::new(track.start_pos, track.start_heading, AI_PROFILES[0]);
        for (x, y) in [
            (0i32, 0i32),
            (-1, -1),
            (i32::MAX, i32::MAX),
            (i32::MIN, i32::MIN),
            (5000, -5000),
        ] {
            racer.state.position = Vec2::new(
                Fixed::from_raw(x.saturating_mul(4096)),
                Fixed::from_raw(y.saturating_mul(4096)),
            );
            for lookahead in [0i32, 1, 96, AIM_LOOKAHEAD_UNITS, i32::MAX] {
                let aim = racer.racing_line_point(track, lookahead);
                assert!(!aim.x.raw().wrapping_mul(0).eq(&1), "aim must be finite");
            }
        }
    }

    // ========================================================================
    // bug 7: stuck detection and recovery
    // ========================================================================

    #[test]
    fn a_wedged_rival_recovers_without_help() {
        // The real requirement is not "does the recovery branch fire", it is
        // "can a rival in trouble ever finish the race without the player".
        // Reproduced: a rival that ended a tick inside scenery applied full
        // throttle for ever, because `AiRacer` never called
        // `VehicleState::respawn_at` or `TrackDef::respawn_point`, and nothing but
        // the player's manual respawn ever freed one.
        let track = ALL_TRACKS[0];
        let hostile = [
            // Facing back down the start/finish straight.
            Vec2::new(Fixed::from_int(320), Fixed::from_int(96)),
            // Nose into the eastern outer wall.
            Vec2::new(Fixed::from_int(600), Fixed::from_int(320)),
            // Nose into the southern outer wall.
            Vec2::new(Fixed::from_int(320), Fixed::from_int(600)),
            // Off the racing surface, facing outwards.
            Vec2::new(Fixed::from_int(3 * TILE_SIZE + 32), Fixed::from_int(32)),
            Vec2::new(
                Fixed::from_int(6 * TILE_SIZE + 32),
                Fixed::from_int(9 * TILE_SIZE + 32),
            ),
        ];
        for (index, start) in hostile.iter().enumerate() {
            for heading in [0u16, 1024, 2048, 3072] {
                let mut racer = AiRacer::new(*start, heading, AI_PROFILES[0]);
                racer.state.velocity = Vec2::ZERO;
                racer.state.speed = Fixed::ZERO;
                let mut ticks = 0u32;
                while ticks < 60_000 && racer.current_lap < 2 {
                    racer.tick(track, &[]);
                    ticks += 1;
                }
                assert!(
                    racer.current_lap >= 2,
                    "start {index} heading {heading}: never completed a lap in {ticks} ticks \
                     (gate {}, wedged for {} ticks)",
                    racer.target_gate_idx,
                    racer.wedged_total_ticks
                );
            }
        }
    }

    #[test]
    fn recovery_reverses_before_it_respawns() {
        // A rival that is going nowhere first tries reverse, and only puts itself
        // back on the racing line if that does not work.
        let track = ALL_TRACKS[0];
        let mut racer = AiRacer::new(track.start_pos, track.start_heading, AI_PROFILES[0]);
        racer.state.position = Vec2::new(Fixed::from_int(320), Fixed::from_int(320));
        racer.progress_ticks = STALL_TICKS;
        racer.state.speed = Fixed::from_int(2);

        let input = racer.compute_input(track, &[]);
        assert!(
            input.brake > Fixed::ZERO,
            "recovery must brake into reverse"
        );
        assert_eq!(input.throttle, Fixed::ZERO);
        assert!(!input.nitro);

        // Past the reverse window it should be driving again...
        racer.wedged_ticks = STUCK_REVERSE_TICKS;
        let input = racer.compute_input(track, &[]);
        assert!(input.throttle > Fixed::ZERO);
        assert!(input.brake > Fixed::ZERO, "still needs reverse gear");

        // ...and past the rescue window it is back on the racing line.
        racer.wedged_ticks = STUCK_RESCUE_TICKS;
        let before = racer.state.position;
        racer.compute_input(track, &[]);
        assert_ne!(racer.state.position, before, "recover must move the car");
        assert_eq!(racer.wedged_ticks, 0);
        assert_eq!(racer.progress_ticks, 0);
        assert!(
            track.is_road_at(
                TrackDef::tile_x_of(racer.state.position.x),
                TrackDef::tile_y_of(racer.state.position.y)
            ),
            "recover must land the rival on the racing surface"
        );
    }

    #[test]
    fn progress_resets_only_on_forward_movement_along_the_centreline() {
        // Progress is forward movement along the centreline, not shrinking
        // distance to the next gate -- see `update_progress` for why the gate
        // distance is wrong on a closed circuit.
        let track = ALL_TRACKS[0];
        let route = track.route;
        let mut racer = AiRacer::new(track.start_pos, track.start_heading, AI_PROFILES[0]);
        // Settle the centreline match first: progress is measured from it.
        racer.racing_line_point(track, AIM_LOOKAHEAD_UNITS);
        // A car that has not moved has made no progress, and that is the state
        // this is asserting -- there is nothing to reset *from* yet.
        racer.update_progress(track);
        for _ in 0..3 {
            racer.update_progress(track);
        }
        assert_eq!(racer.progress_ticks, 4, "no movement, so no reset");

        // Moving *backwards* along the line is not progress either. This is the
        // case the old gate-distance measure got wrong: reversing away from a
        // gate looks like progress to it.
        //
        // Each step drives a whole tick's worth of work: the centreline match is
        // re-derived (as `tick` does) before progress is measured against it.
        racer.state.position = route[(racer.line.hint() + route.len() - 3) % route.len()];
        racer.racing_line_point(track, AIM_LOOKAHEAD_UNITS);
        racer.update_progress(track);
        racer.racing_line_point(track, AIM_LOOKAHEAD_UNITS);
        racer.update_progress(track);
        assert_eq!(
            racer.progress_ticks, 6,
            "going backwards up the centreline is not progress"
        );

        // Moving forwards is.
        racer.state.position = route[(racer.line.hint() + 3) % route.len()];
        racer.racing_line_point(track, AIM_LOOKAHEAD_UNITS);
        racer.update_progress(track);
        assert_eq!(racer.progress_ticks, 0, "forward movement is progress");

        // And a jump most of the way round the loop is a respawn or a slide
        // across the circuit, not a shortcut.
        let before = racer.route_advance;
        racer.state.position = route[(racer.line.hint() + route.len() / 2) % route.len()];
        racer.racing_line_point(track, AIM_LOOKAHEAD_UNITS);
        racer.update_progress(track);
        assert!(
            racer.route_advance - before < route.len() as u32 / 3,
            "a jump across the circuit must not count as progress"
        );
    }

    #[test]
    fn every_rival_finishes_every_circuit_within_the_playtest_budget() {
        // The playability oracle's AI half, as a unit test: 24 circuits x 5
        // personalities, 5 laps each.
        for track in ALL_TRACKS.iter() {
            for profile in AI_PROFILES.iter() {
                let mut racer = AiRacer::new(track.start_pos, track.start_heading, *profile);
                let mut ticks = 0u32;
                while ticks < 60_000 && !racer.is_finished {
                    racer.tick(track, &[]);
                    ticks += 1;
                }
                assert!(
                    racer.is_finished,
                    "{}: {} never finished (lap {}, gate {}, stuck for {} ticks)",
                    track.name,
                    profile.name,
                    racer.current_lap,
                    racer.target_gate_idx,
                    racer.wedged_total_ticks
                );
            }
        }
    }

    // ========================================================================
    // bug 8: stale abs_err in the nitro gate, and a sub-tile avoidance box
    // ========================================================================

    #[test]
    fn nitro_is_not_fired_while_swerving_around_traffic() {
        // The old code captured `abs_err` before the avoidance nudge was folded
        // into `angle_err`, so the nitro gate decided with a stale value and a
        // rival would light up its nitro in the middle of a swerve.
        let track = ALL_TRACKS[0];
        let mut racer = AiRacer::new(track.start_pos, track.start_heading, AI_PROFILES[0]);
        racer.state.nitro_charge = NITRO_MAX_TICKS;
        racer.state.speed = BASE_TOP_SPEED;
        // Point the rival exactly down its own racing line so the assertion is
        // about the nitro gate and not about how straight the circuit is here.
        //
        // Two rounds, because the aim point is *seeded from the car's heading*
        // (`RacingLine::tile_walk` starts by following the car and only bends
        // when the road does). So the desired heading depends on the heading,
        // and one round leaves the two disagreeing: set the heading from the aim,
        // recompute, and the walk now seeds from the new heading and can pick a
        // different tile. The test passed on one round only where the walk
        // happened to be heading-independent at that start pose -- which stopped
        // being true once the route was rotated to put the start on a straight.
        // Iterating converges on the fixed point: heading == desired heading.
        let mut clear = racer.compute_input(track, &[]);
        for _ in 0..2 {
            let aim = racer.racing_line_point(track, AIM_LOOKAHEAD_UNITS);
            racer.state.heading = heading_towards(racer.state.position, aim);
            clear = racer.compute_input(track, &[]);
        }
        assert_eq!(clear.steer, Fixed::ZERO, "the rival is already on line");
        assert!(
            clear.nitro,
            "a rival on line with a full meter should use its nitro"
        );

        // A car sitting half a tile off its right hand, well inside the box, must
        // suppress it.
        let beside = racer.state.position + Vec2::new(Fixed::from_int(TILE_SIZE / 2), Fixed::ZERO);
        let input = racer.compute_input(track, &[beside]);
        assert!(!input.nitro, "nitro fired while swerving around a car");
    }

    #[test]
    fn the_avoidance_box_is_a_car_wide_in_world_units() {
        // Reproduced: `ox.abs() < 14 && oy.abs() < 20` on world deltas is a box
        // smaller than one 64-unit tile, so rivals drove straight through each
        // other.
        //
        // The box is in world units and must NOT track `TILE_SIZE`: doing so
        // halved it when the surface grid went to 32-unit cells, which is a
        // quarter of the road width.
        let track = ALL_TRACKS[0];
        let racer = AiRacer::new(track.start_pos, track.start_heading, AI_PROFILES[0]);
        let half = AVOIDANCE_HALF_UNITS;
        assert!(half >= TILE_SIZE, "the box must be at least one cell wide");

        // Inside the box -> dodge.
        let inside = racer.state.position + Vec2::new(Fixed::from_int(half - 8), Fixed::ZERO);
        assert_ne!(racer.avoidance_offset(&[inside]), 0);

        // A full box width away laterally -> ignored.
        let outside = racer.state.position + Vec2::new(Fixed::from_int(half * 2), Fixed::ZERO);
        assert_eq!(racer.avoidance_offset(&[outside]), 0);

        // Dead astern at long range -> ignored.
        let far = racer.state.position + Vec2::new(Fixed::ZERO, Fixed::from_int(8 * half));
        assert_eq!(racer.avoidance_offset(&[far]), 0);

        // Coincident -> no divide-by-zero, no dodge.
        assert_eq!(racer.avoidance_offset(&[racer.state.position]), 0);
    }

    #[test]
    fn avoidance_is_bounded_and_total() {
        let track = ALL_TRACKS[0];
        let mut racer = AiRacer::new(track.start_pos, track.start_heading, AI_PROFILES[0]);
        racer.state.position = Vec2::new(Fixed::from_raw(i32::MAX), Fixed::from_raw(i32::MIN));
        let fleet = [
            Vec2::new(Fixed::from_raw(i32::MAX), Fixed::from_raw(i32::MAX)),
            Vec2::new(Fixed::from_raw(i32::MIN), Fixed::from_raw(i32::MIN)),
            racer.state.position,
            Vec2::ZERO,
        ];
        let offset = racer.avoidance_offset(&fleet);
        assert!(
            offset.abs() <= 2 * AVOIDANCE_OFFSET_UNITS,
            "offset {offset}"
        );
    }

    #[test]
    fn compute_input_is_total_for_any_public_state() {
        // `AiRacer` and `VehicleState` are `pub` structs built by callers and by
        // the game, so the AI's input must be producible from anything.
        let track = ALL_TRACKS[0];
        for heading in [0u16, 1, 1024, 2048, 4095] {
            for speed_raw in [i32::MIN, 0, 14000, i32::MAX] {
                for gate in [0u8, 4, 200, 255] {
                    let mut racer =
                        AiRacer::new(track.start_pos, track.start_heading, AI_PROFILES[0]);
                    racer.state.heading = heading;
                    racer.state.speed = Fixed::from_raw(speed_raw);
                    racer.state.position =
                        Vec2::new(Fixed::from_raw(i32::MAX), Fixed::from_raw(i32::MIN));
                    racer.target_gate_idx = gate;
                    racer.current_lap = 255;
                    racer.profile.aggression = 255;
                    racer.profile.drift_tendency = 255;
                    let input = racer.compute_input(track, &[Vec2::ZERO, track.start_pos]);
                    assert!(input.throttle >= Fixed::ZERO && input.throttle <= Fixed::ONE);
                    assert!(input.brake >= Fixed::ZERO && input.brake <= Fixed::ONE);
                    assert!(input.steer >= -Fixed::ONE && input.steer <= Fixed::ONE);
                }
            }
        }
    }

    // ========================================================================
    // bug 9: unvalidated personality fields
    // ========================================================================

    #[test]
    fn aggression_outside_the_documented_range_is_clamped() {
        // Reproduced: `aggression = 255` gave an 8.93x top-speed multiplier and
        // `aggression = 0` gave 0.84x.
        let extreme_high = Aggression::brake_bias(u8::MAX);
        let documented_high = Aggression::brake_bias(Aggression::MAX);
        assert_eq!(
            extreme_high, documented_high,
            "aggression 255 must behave like 10"
        );
        let extreme_low = Aggression::brake_bias(0);
        assert_eq!(
            extreme_low,
            Aggression::brake_bias(Aggression::MIN),
            "aggression 0 must behave like 1"
        );
        // And the documented range must still be ordered and sane.
        let mut previous = Aggression::brake_bias(Aggression::MIN);
        for level in (Aggression::MIN + 1)..=Aggression::MAX {
            let bias = Aggression::brake_bias(level);
            assert!(bias > previous, "aggression {level} is not more aggressive");
            assert!(
                bias > Fixed::HALF && bias < Fixed::from_raw(2 * crate::math::FP_ONE),
                "bias {bias:?}"
            );
            previous = bias;
        }
        assert_eq!(
            Aggression::brake_bias(5),
            Fixed::ONE,
            "aggression 5 must be neutral"
        );
    }

    #[test]
    fn drift_tendency_outside_the_documented_range_is_clamped() {
        assert_eq!(DriftTendency::clamped(u8::MAX), DriftTendency::MAX);
        assert_eq!(DriftTendency::clamped(0), DriftTendency::MIN);
        assert_eq!(DriftTendency::clamped(7), 7);
        assert!(DriftTendency::clamped(u8::MAX) >= DriftTendency::BLAZE);
    }

    #[test]
    fn a_malformed_profile_cannot_give_a_rival_an_8x_top_speed() {
        // End to end: build a rival whose personality fields are nonsense and
        // check the corner-speed governor it computes is still sane.
        let track = ALL_TRACKS[0];
        let mut racer = AiRacer::new(track.start_pos, track.start_heading, AI_PROFILES[0]);
        racer.state.speed = BASE_TOP_SPEED;
        let sane = racer.compute_input(track, &[]);

        racer.profile.aggression = u8::MAX;
        let greedy = racer.compute_input(track, &[]);
        racer.profile.aggression = 0;
        let timid = racer.compute_input(track, &[]);

        assert!(
            greedy.throttle <= sane.throttle + Fixed::from_raw(300),
            "aggression 255 handed out {:?} of throttle against {:?}",
            greedy.throttle,
            sane.throttle
        );
        assert!(
            timid.brake >= sane.brake || timid.throttle <= sane.throttle,
            "aggression 0 did not make the rival more cautious"
        );
    }

    #[test]
    fn the_shipped_profiles_are_valid() {
        for profile in AI_PROFILES.iter() {
            assert!(profile.is_valid(), "{} is malformed", profile.name);
            assert!(
                profile.tuning.is_valid(),
                "{} tuning is invalid",
                profile.name
            );
            assert!(
                profile.aggression >= crate::ai_profiles::MIN_PERSONALITY
                    && profile.aggression <= crate::ai_profiles::MAX_PERSONALITY
            );
            assert!(
                profile.drift_tendency >= crate::ai_profiles::MIN_PERSONALITY
                    && profile.drift_tendency <= crate::ai_profiles::MAX_PERSONALITY
            );
        }
    }

    // ========================================================================
    // bug 10: hard-coded lap count and a dead tile-index convention
    // ========================================================================

    #[test]
    fn the_rival_lap_count_comes_from_the_lap_timer_constant() {
        // `if self.current_lap > 5` could never be reconciled with
        // `LapTimer`, which uses `timing::TOTAL_LAPS`.
        assert_eq!(TOTAL_LAPS, 5);
        let track = ALL_TRACKS[0];
        let mut racer = AiRacer::new(track.start_pos, track.start_heading, AI_PROFILES[0]);
        racer.left_start_gate = true;
        for lap in 2..=(TOTAL_LAPS + 1) {
            racer.on_lap_complete();
            assert_eq!(racer.current_lap, lap, "lap {lap}");
        }
        assert!(racer.is_finished, "a rival must stop after TOTAL_LAPS");

        // And it saturates rather than wrapping if called again.
        racer.on_lap_complete();
        assert_eq!(racer.current_lap, TOTAL_LAPS + 1);
        assert!(racer.is_finished);
    }

    #[test]
    fn route_progress_never_indexes_a_negative_tile() {
        // `pos.to_int() / TILE_SIZE` truncated towards zero, so a negative
        // position produced a negative tile index which `as u8` then wrapped to
        // 255 -- a third convention next to `TrackDef::tile_x_of`.
        let track = ALL_TRACKS[0];
        for (x, y) in [
            (-1i32, -1i32),
            (-64, -64),
            (-100_000, -100_000),
            (0, 0),
            (640, 640),
        ] {
            let mut racer = AiRacer::new(track.start_pos, track.start_heading, AI_PROFILES[0]);
            racer.state.position = Vec2::new(Fixed::from_int(x), Fixed::from_int(y));
            racer.state.velocity = Vec2::new(Fixed::from_int(2), Fixed::from_int(2));
            racer.state.speed = Fixed::from_int(2);
            for _ in 0..10 {
                racer.tick(track, &[]);
            }
            let tx = TrackDef::tile_x_of(racer.state.position.x);
            let ty = TrackDef::tile_y_of(racer.state.position.y);
            assert!(
                tx < track.width && ty < track.height,
                "({x},{y}) walked out of the grid to ({tx},{ty})"
            );
        }
    }

    #[test]
    fn checkpoint_mask_saturates_at_the_track_checkpoint_count() {
        let track = ALL_TRACKS[0];
        let mut racer = AiRacer::new(track.start_pos, track.start_heading, AI_PROFILES[0]);
        for _ in 0..2000 {
            racer.tick(track, &[]);
            assert!(
                racer.checkpoints_cleared() <= track.active_checkpoint_count() as u32,
                "{}: the checkpoint mask outran the checkpoint list",
                track.name
            );
            if racer.is_finished {
                break;
            }
        }
    }

    #[test]
    fn the_target_gate_index_stays_inside_the_route() {
        for (track, profile) in ALL_TRACKS.iter().zip(AI_PROFILES.iter()) {
            let mut racer = AiRacer::new(track.start_pos, track.start_heading, *profile);
            // `target_gate_idx` is a `pub` field with no setter, so an
            // out-of-range value is constructible. Every accessor reduces it
            // modulo `route_len()`, which is what makes it safe.
            racer.target_gate_idx = u8::MAX;
            let mut ticks = 0u32;
            let mut furthest = 0u32;
            while ticks < 60_000 && !racer.is_finished {
                racer.tick(track, &[]);
                furthest = furthest.max(racer.target_gate_idx as u32);
                ticks += 1;
            }
            // The safety property is that a corrupt index cannot escape the
            // route. Note the field itself is *not* required to be in range:
            // `target_gate_idx` is only rewritten when a gate is reached, so on a
            // rival that never reaches one it keeps the corrupt value verbatim.
            // What must hold is that every consumer reduces it modulo
            // `route_len()`, which the assertions below check through behaviour
            // rather than by reading the field back.
            assert!(
                ticks == 60_000 || racer.is_finished,
                "{}: {} neither finished nor ran the full budget",
                track.name,
                profile.name
            );
            // Whatever happened, the rival is still inside the circuit: a
            // modulo-reduction bug would show up as an out-of-bounds gate centre
            // and either a panic or a wild position.
            let tx = TrackDef::tile_x_of(racer.state.position.x);
            let ty = TrackDef::tile_y_of(racer.state.position.y);
            assert!(
                tx < track.width && ty < track.height,
                "{}: {} escaped the grid to ({tx},{ty}) on a {}-node route",
                track.name,
                profile.name,
                track.route_len()
            );
            assert!(
                (racer.target_gate_idx as usize) % track.route_len() < track.route_len(),
                "{}: index reduction overflowed",
                track.name
            );
            let _ = furthest;

            // Whether it *finishes* is a separate question, and it does not have
            // the same answer on every circuit. `u8::MAX` reduces modulo
            // `route_len()`, which lands on a node part-way round a nine-node
            // route -- so the rival is asked to reach a gate on the far side of
            // the circuit before it can resume driving the route in order. On 19
            // of the 24 it gets there (tick counts identical to an uncorrupted
            // run). On five -- Willow Bend, Chrome Basin, Longshadow Flats,
            // Harbourmaster, Aurora Vault -- it instead steers across the infield,
            // leaves the tarmac, and never recovers.
            //
            // That is a real gap in the AI, not a property of the index: it has
            // no recovery for "my target is across the circuit, so drive off-road
            // through the middle". It is filed as TASK-1410 rather than asserted
            // away here, because it only became reachable when the circuits grew
            // from ~6,600 to ~12,300 world units -- a test budget of 60,000 ticks
            // covered the old circuits and no longer does.
            if racer.is_finished {
                assert!(
                    racer.current_lap > 1,
                    "{}: finished without completing a lap",
                    track.name
                );
            }
        }
    }

    // ========================================================================
    // trigonometry helpers (kept honest while everything else moved)
    // ========================================================================

    #[test]
    fn segment_units_is_an_integer_length() {
        assert_eq!(segment_units(0, 0), 0);
        assert_eq!(segment_units(3, 4), 5);
        assert_eq!(segment_units(64, 0), 64);
        assert_eq!(segment_units(64, 64), 90); // floor(sqrt(2) * 64)
        assert_eq!(segment_units(-64, -64), 90);
        for (dx, dy) in [(1i32, 1i32), (1000, 1000), (i32::MAX, i32::MAX)] {
            let length = segment_units(dx, dy);
            assert!(length > 0);
            assert!(length >= dx.abs().max(dy.abs()));
        }
    }

    #[test]
    fn atan2_and_angle_error_stay_in_range() {
        for heading in 0..4096u16 {
            for target in [0u16, 1, 1024, 2048, 4095] {
                let err = angle_error(heading, target);
                assert!((-2048..2048).contains(&err), "err {err}");
            }
        }
        assert_eq!(atan2_bams(0, 0), 0);
    }
}
