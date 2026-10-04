//! Vehicle state, dynamics, and drift simulation.
//!
//! Units: `speed` and `velocity` are expressed in **world units per 60 Hz tick**.
//! A track tile is [`crate::track::TILE_SIZE`] (64) world units, so a car at top
//! speed (`BASE_TOP_SPEED` = 3.42 u/tick, ~205 u/s) crosses one tile in ~19 ticks
//! and the 320x240 viewport in ~1.6 s - matching the 90s arcade pace ArduRacer FX
//! ran at on its 640x640 px level space.

use crate::drift::DriftState;
use crate::math::{self, Fixed, Vec2};
use crate::surface::SurfaceType;
use crate::track::{TrackDef, TILE_SIZE};
use crate::tuning::CarTuning;

/// Unladen top speed in world units per tick (~3.42 u/tick == ~205 u/s).
pub const BASE_TOP_SPEED: Fixed = Fixed::from_raw(14000);
/// Engine thrust per tick at full throttle and default tuning (~0.065 u/tick^2).
///
/// With `DRAG_RETAIN` this reaches ~92% of `BASE_TOP_SPEED` in ~108 ticks (1.8 s):
/// punchy enough for arcade corner-exit traction without being untameable, and it
/// saturates above `BASE_TOP_SPEED` so the top-speed clamp is what limits the car.
const BASE_ACCEL: Fixed = Fixed::from_raw(268);
/// Per-tick retention of forward speed from aerodynamic drag (~0.85%/tick).
const DRAG_RETAIN: Fixed = Fixed::from_raw(4062);
/// Foot-brake deceleration in world units per tick (~2.6 u/s^2, ~1.3 s to stop
/// from top speed - firm enough to make braking points matter).
const BRAKE_FORCE: Fixed = Fixed::from_raw(180);
/// Reverse gear thrust, ~40% of forward thrust.
const REVERSE_SCALE: Fixed = Fixed::from_raw(1638);
/// Turbo impulse applied per tick while a boost is active.
const BOOST_THRUST: Fixed = Fixed::from_raw(80);
/// Extra top speed headroom while a boost is active.
const BOOST_TOP_SPEED_BONUS: Fixed = Fixed::from_raw(2000);
/// Angular units of heading change per tick at full steering lock.
pub const BASE_TURN_RATE: u16 = 70;
/// Below this speed the steering input is ignored (prevents jitter at rest).
const STEER_MIN_SPEED: Fixed = Fixed::from_raw(200);
/// Boost pads re-arm only after leaving the pad, preventing per-tick re-triggering.
const BOOST_PAD_COOLDOWN_TICKS: u16 = 12;
/// Into-wall speed (world units/tick) above which a barrier hit counts as a crash.
pub const HARD_IMPACT_THRESHOLD: Fixed = Fixed::from_raw(900);
/// Full nitro meter: 1.5 s of continuous boost, recharging in ~6 s.
pub const NITRO_MAX_TICKS: u16 = 90;
pub const NITRO_RECHARGE_TICKS: u16 = 6;
/// Per-tick nitro thrust (well above the engine's own push).
const NITRO_THRUST: Fixed = Fixed::from_raw(150);

/// Vehicle input commands for a single simulation frame.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct VehicleInput {
    /// Throttle applied: 0 (none) to FP_ONE (full gas).
    pub throttle: Fixed,
    /// Brake applied: 0 (none) to FP_ONE (full brake). Becomes reverse at rest.
    pub brake: Fixed,
    /// Steering input: -FP_ONE (full left) to +FP_ONE (full right).
    pub steer: Fixed,
    /// Handbrake flag (triggers drift initiation).
    pub handbrake: bool,
    /// Nitro held: spends the boost meter for a sustained speed surge
    /// (GAME.md §7 "R1 / R2: Upshift (Manual mode) / Nitro").
    pub nitro: bool,
}

/// Dynamic physical state of a racing car.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct VehicleState {
    /// Position in fixed-point world coordinates.
    pub position: Vec2,
    /// Velocity vector in world units per tick.
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
    /// Nitro meter: 0..=NITRO_MAX_TICKS of accumulated charge. Drains while
    /// `VehicleInput::nitro` is held and refills when it is not.
    pub nitro_charge: u16,
    /// Cooldown before a boost pad can fire again.
    pub boost_pad_cooldown: u16,
    /// Current engine RPM (1000..8000 scale).
    pub engine_rpm: u16,
    /// Current gear (1..5).
    pub gear: u8,
    /// True while reversing under brake input.
    pub is_reversing: bool,
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
            nitro_charge: NITRO_MAX_TICKS,
            boost_pad_cooldown: 0,
            engine_rpm: 1000,
            gear: 1,
            is_reversing: false,
            tuning: CarTuning::default(),
        }
    }
}

impl VehicleState {
    /// Creates a car parked at `start_pos` facing `start_heading`.
    pub fn new(start_pos: Vec2, start_heading: u16, tuning: CarTuning) -> Self {
        VehicleState {
            position: start_pos,
            heading: start_heading,
            visual_angle: start_heading,
            tuning,
            ..VehicleState::default()
        }
    }

    /// Unit forward vector for the current heading (0 = North / -Y).
    #[inline]
    pub fn forward_dir(&self) -> Vec2 {
        Vec2::new(math::sin(self.heading), -math::cos(self.heading))
    }

    /// Puts the car back on the racing surface at `pos`, facing `heading`.
    ///
    /// This is the recovery path for a car that is stuck, spun, or off in a
    /// corner it cannot drive out of. Everything that could keep a car pinned is
    /// cleared: velocity, drift state, boost and nitro charge, reverse flag and
    /// the visual drift angle. `heading` is snapped to whole units so the car
    /// never resumes sideways.
    ///
    /// The caller picks the spot -- [`TrackDef::respawn_point`] chooses the
    /// nearest route node facing down the racing line.
    pub fn respawn_at(&mut self, pos: Vec2, heading: u16) {
        self.position = pos;
        self.heading = heading & 0x0FFF;
        self.visual_angle = self.heading;
        self.velocity = Vec2::ZERO;
        self.speed = Fixed::ZERO;
        self.is_drifting = false;
        self.drift = DriftState::Grip;
        self.boost_ticks = 0;
        self.nitro_charge = NITRO_MAX_TICKS;
        self.boost_pad_cooldown = 0;
        self.is_reversing = false;
        self.gear = 1;
        self.engine_rpm = 1000;
    }

    /// Unit right-hand lateral vector for the current heading.
    #[inline]
    pub fn right_dir(&self) -> Vec2 {
        Vec2::new(math::cos(self.heading), math::sin(self.heading))
    }

    /// Signed forward component of velocity.
    #[inline]
    pub fn forward_speed(&self) -> Fixed {
        self.velocity.dot(self.forward_dir())
    }

    /// Resolves a collision against a solid track barrier with the given surface
    /// normal pointing *away* from the wall.
    ///
    /// Light contact cancels only the into-wall velocity component so the car
    /// slides along the barrier instead of sticking to it; a hard impact also
    /// scrubs speed and kicks the car into a spin-out (GAME.md §5.2 wall sparks).
    pub fn handle_barrier_collision(&mut self, normal: Vec2) {
        let v_dot_n = self.velocity.dot(normal);
        // Only resolve when actually moving into the wall.
        if v_dot_n >= Fixed::ZERO {
            return;
        }

        let impact = -v_dot_n;
        // Restitution coefficient 0.35 (35% bounce) cancels the penetration.
        let restitution = Fixed::from_raw(1433);
        let impulse_mag = (Fixed::ONE + restitution) * impact;
        self.velocity = self.velocity + normal.scale(impulse_mag);

        // Hard head-on hits scrub momentum and spin the car; brushing a wall
        // while scraping along it does not, or the car would be glued in place.
        if impact > HARD_IMPACT_THRESHOLD {
            self.velocity = self.velocity.scale(Fixed::from_raw(2867)); // 30% scrub
            self.speed = self.velocity.length();
            if !self.drift.is_spinning() {
                self.drift.trigger_spinout();
            }
        }
    }

    /// Advances the vehicle physics simulation by one 60Hz tick.
    pub fn tick(&mut self, input: VehicleInput, surface: SurfaceType) {
        if self.boost_pad_cooldown > 0 {
            self.boost_pad_cooldown -= 1;
        }

        // 1. Steering -------------------------------------------------------------
        let scaled_turn = self.tuning.scaled_turn_rate(BASE_TURN_RATE);
        if input.steer.raw() != 0 && self.speed > STEER_MIN_SPEED {
            let steer_delta =
                ((scaled_turn as i64 * input.steer.raw() as i64) >> math::FP_SHIFT) as i32;
            self.heading = ((self.heading as i32 + steer_delta) & 0x0FFF) as u16;
        }

        // 2. Engine thrust -------------------------------------------------------
        let forward_dir = self.forward_dir();
        let right_dir = self.right_dir();
        let traction = surface.traction();

        if input.throttle > Fixed::ZERO && traction > Fixed::ZERO {
            let accel = self.tuning.scaled_acceleration(BASE_ACCEL);
            self.velocity = self.velocity + forward_dir.scale(accel * input.throttle * traction);
        }

        if self.boost_ticks > 0 {
            self.boost_ticks -= 1;
            self.velocity = self.velocity + forward_dir.scale(BOOST_THRUST);
        }

        // 2b. Nitro ----------------------------------------------------------------
        // Held nitro spends the meter and keeps the car above its normal ceiling
        // for as long as the charge lasts.
        let nitro_active = input.nitro && self.nitro_charge > 0 && traction > Fixed::ZERO;
        if nitro_active {
            self.nitro_charge -= 1;
            self.boost_ticks = self.boost_ticks.max(2);
            self.velocity = self.velocity + forward_dir.scale(NITRO_THRUST);
        } else if self.nitro_charge < NITRO_MAX_TICKS {
            self.nitro_charge = (self.nitro_charge + NITRO_RECHARGE_TICKS).min(NITRO_MAX_TICKS);
        }

        // 3. Braking & reverse ---------------------------------------------------
        self.is_reversing = false;
        if input.brake > Fixed::ZERO {
            let speed_along_body = self.velocity.dot(forward_dir);
            if speed_along_body > Fixed::ZERO {
                // Moving forwards: scrub speed along the direction of travel.
                let decel = BRAKE_FORCE * input.brake;
                self.velocity = self.velocity - forward_dir.scale(decel);
                if self.velocity.dot(forward_dir) < Fixed::ZERO {
                    self.velocity =
                        self.velocity - forward_dir.scale(self.velocity.dot(forward_dir));
                }
            } else {
                // At a standstill (or already rolling backwards): reverse gear.
                self.is_reversing = true;
                let accel = self.tuning.scaled_acceleration(BASE_ACCEL);
                let reverse = REVERSE_SCALE * accel * input.brake * traction;
                let reverse_speed = -(self.velocity.dot(forward_dir));
                let limit = Fixed::from_raw(2000);
                if reverse_speed < limit {
                    self.velocity = self.velocity - forward_dir.scale(reverse);
                }
            }
        }

        // 4. Drift state machine -------------------------------------------------
        if input.handbrake && !self.drift.is_drifting() && self.speed > Fixed::from_raw(3000) {
            let drift_dir = if input.steer < Fixed::ZERO { -1 } else { 1 };
            self.drift.initiate_drift(drift_dir);
        } else if !input.handbrake && self.drift.is_drifting() {
            // Releasing the handbrake releases the mini-turbo boost.
            let boost_impulse = self.drift.release_drift();
            if boost_impulse > Fixed::ZERO {
                self.boost_ticks = 30; // 0.5s of turbo boost
                self.velocity = self.velocity + forward_dir.scale(boost_impulse);
            }
        }

        self.drift.tick(input.steer, self.speed, &self.tuning);
        self.is_drifting = self.drift.is_drifting();
        if self.drift.is_spinning() {
            self.is_reversing = false;
        }

        // 5. Body-frame decomposition, drag and lateral traction ----------------
        let forward_speed = self.velocity.dot(forward_dir);
        let lateral_speed = self.velocity.dot(right_dir);

        let lateral_hold = if self.drift.is_spinning() {
            Fixed::ZERO
        } else {
            surface.lateral_hold(self.is_drifting)
        };

        let damped_forward = forward_speed * DRAG_RETAIN;
        let damped_lateral = lateral_speed * lateral_hold;

        self.velocity = forward_dir.scale(damped_forward) + right_dir.scale(damped_lateral);

        // 6. Surface speed cap & absolute top-speed clamp -------------------------
        let top_speed = self.tuning.scaled_top_speed(BASE_TOP_SPEED);
        let surface_cap = top_speed * surface.max_speed_factor();
        let effective_top = if self.boost_ticks > 0 {
            surface_cap + BOOST_TOP_SPEED_BONUS
        } else {
            surface_cap
        };

        let speed_mag = self.velocity.length();
        if speed_mag > effective_top {
            let scale_down = effective_top / speed_mag;
            self.velocity = self.velocity.scale(scale_down);
            self.speed = effective_top;
        } else {
            self.speed = speed_mag;
        }

        // 7. Boost pads ----------------------------------------------------------
        if surface.is_boost_pad()
            && self.boost_pad_cooldown == 0
            && self.boost_ticks == 0
            && !self.is_reversing
            && !nitro_active
        {
            self.boost_ticks = 45;
            self.boost_pad_cooldown = BOOST_PAD_COOLDOWN_TICKS;
        }

        // 8. Integrate position --------------------------------------------------
        self.position = self.position + self.velocity;

        // 9. Visual angle smoothing (drift slip angle representation) ------------
        match self.drift {
            DriftState::Drifting { slip_angle, .. } => {
                self.visual_angle = ((self.heading as i32 + slip_angle as i32) & 0x0FFF) as u16;
            }
            DriftState::SpinOut { remaining_ticks } => {
                let spin_offset = remaining_ticks * 128;
                self.visual_angle = self.heading.wrapping_add(spin_offset) & 0x0FFF;
            }
            DriftState::Grip => {
                self.visual_angle = self.heading;
            }
        }

        // 10. Engine RPM & gear for audio / HUD ----------------------------------
        let ratio = (self.speed.raw() as i64 * 8000) / (BASE_TOP_SPEED.raw() as i64).max(1);
        let target_rpm = (1000 + ratio).clamp(0, 8000) as u16;
        self.engine_rpm = target_rpm;
        self.gear = match target_rpm {
            0..=2200 => 1,
            2201..=3800 => 2,
            3801..=5400 => 3,
            5401..=7000 => 4,
            _ => 5,
        };
    }

    /// Resolves the vehicle against the track: solid barriers bounce the car back
    /// onto the circuit and the world bounding box acts as the outer wall.
    ///
    /// Mirrors ArduRacer FX, which only ever collided with the level bounds; the
    /// FX tile art had no hard walls, so the bounding box is the real "barrier".
    pub fn collide_with_track(&mut self, track: &TrackDef) -> bool {
        let max_x = track.world_width();
        let max_y = track.world_height();
        let mut hit = false;

        // Solid interior walls (authored into the PSX Super Stages).
        let tx = TrackDef::tile_x_of(self.position.x).min(track.width.saturating_sub(1));
        let ty = TrackDef::tile_y_of(self.position.y).min(track.height.saturating_sub(1));
        let tile = track.tile_at(tx, ty);
        if tile.is_solid() {
            hit = true;
            // Push back along whichever face of the tile we entered through.
            let local_x = self.position.x.to_int() - (tx as i32 * TILE_SIZE);
            let local_y = self.position.y.to_int() - (ty as i32 * TILE_SIZE);
            let normal = if local_x + local_y <= TILE_SIZE {
                Vec2::new(-Fixed::ONE, Fixed::ZERO)
            } else if local_x + local_y >= TILE_SIZE * 2 {
                Vec2::new(Fixed::ONE, Fixed::ZERO)
            } else if local_y <= local_x {
                Vec2::new(Fixed::ZERO, -Fixed::ONE)
            } else {
                Vec2::new(Fixed::ZERO, Fixed::ONE)
            };
            self.handle_barrier_collision(normal);
        }

        // Outer world bounds (the circuit perimeter).
        if self.position.x.to_int() < 0 {
            self.position.x = Fixed::ZERO;
            self.handle_barrier_collision(Vec2::new(Fixed::ONE, Fixed::ZERO));
            hit = true;
        } else if self.position.x.to_int() > max_x {
            self.position.x = Fixed::from_int(max_x);
            self.handle_barrier_collision(Vec2::new(-Fixed::ONE, Fixed::ZERO));
            hit = true;
        }

        if self.position.y.to_int() < 0 {
            self.position.y = Fixed::ZERO;
            self.handle_barrier_collision(Vec2::new(Fixed::ZERO, Fixed::ONE));
            hit = true;
        } else if self.position.y.to_int() > max_y {
            self.position.y = Fixed::from_int(max_y);
            self.handle_barrier_collision(Vec2::new(Fixed::ZERO, -Fixed::ONE));
            hit = true;
        }

        if hit {
            self.speed = self.velocity.length();
        }
        hit
    }

    /// Convenience wrapper: run one full tick including track collision and
    /// return the surface the car ended the tick on.
    pub fn tick_on_track(&mut self, input: VehicleInput, track: &TrackDef) -> SurfaceType {
        let tx = TrackDef::tile_x_of(self.position.x).min(track.width.saturating_sub(1));
        let ty = TrackDef::tile_y_of(self.position.y).min(track.height.saturating_sub(1));
        let surface = track.surface_at(tx, ty);
        self.tick(input, surface);
        self.collide_with_track(track);
        let tx = TrackDef::tile_x_of(self.position.x).min(track.width.saturating_sub(1));
        let ty = TrackDef::tile_y_of(self.position.y).min(track.height.saturating_sub(1));
        track.surface_at(tx, ty)
    }
}
