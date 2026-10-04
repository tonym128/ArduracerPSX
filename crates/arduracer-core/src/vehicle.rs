//! Vehicle state, dynamics, and drift simulation.
//!
//! Units: `speed` and `velocity` are expressed in **world units per 60 Hz tick**.
//! A track tile is [`crate::track::TILE_SIZE`] (64) world units, so a car at top
//! speed (`BASE_TOP_SPEED` = 3.42 u/tick, ~205 u/s) crosses one tile in ~19 ticks
//! and the 320x240 viewport in ~1.6 s - matching the 90s arcade pace ArduRacer FX
//! ran at on its 640x640 px level space.

use crate::drift::DriftState;
use crate::math::{self, Fixed, Vec2, FP_ONE};
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
/// Hard ceiling on reverse speed, ~14% of `BASE_TOP_SPEED`.
const REVERSE_SPEED_LIMIT: Fixed = Fixed::from_raw(2000);
/// Turbo impulse applied per tick while a boost is active.
const BOOST_THRUST: Fixed = Fixed::from_raw(80);
/// Extra top speed headroom while a boost is active.
const BOOST_TOP_SPEED_BONUS: Fixed = Fixed::from_raw(2000);
/// Angular units of heading change per tick at full steering lock.
pub const BASE_TURN_RATE: u16 = 70;
/// Below this speed the steering input is ignored (prevents jitter at rest).
const STEER_MIN_SPEED: Fixed = Fixed::from_raw(200);
/// Steering authority at top speed, as a fraction of [`BASE_TURN_RATE`].
///
/// Steering used to be the same 70 BAM/tick (6.15 deg/tick) at 3 u/tick as it
/// was at 0.1 u/tick, which makes the tightest circle the car can describe
/// scale with speed -- at top speed that was a 32-unit radius, a third of a
/// tile. Real cars lose steering authority with speed (understeer), and that
/// falloff is what puts a floor under the achievable corner radius. 0.25 puts
/// the full-lock radius at the top speed at ~128 units, i.e. two tiles, which is
/// the tightest corner any shipped circuit actually has.
const TOP_SPEED_STEER_AUTHORITY: Fixed = Fixed::from_raw(3072);
/// Steering authority left while the car is spinning out.
const SPINOUT_STEER_AUTHORITY: Fixed = Fixed::from_raw(2048);
/// Steering authority while drifting, i.e. how much the handbrake costs the
/// driver in the direction they are trying to point the car.
const DRIFT_STEER_AUTHORITY: Fixed = Fixed::from_raw(3072);
/// Side-slip the tyres can hold as a fraction of the forward speed when the car
/// is barely rolling, and at top speed (the tangent of the maximum slip angle).
///
/// Before the friction circle, `lateral_hold` was a flat 0.889/tick applied to
/// whatever the rotating heading produced, so full lock at speed settled into a
/// permanent ~39 degree skid -- identical on a 10x10 and a 30x30 circuit,
/// because nothing scaled with speed. The car could not hold a racing line.
const MAX_SLIP_RATIO_SLOW: Fixed = Fixed::from_raw(2048);
const MAX_SLIP_RATIO_FAST: Fixed = Fixed::from_raw(1024);
/// Floor on the slip allowance, so a car crawling at 0 speed is not pinned to
/// zero sideways velocity and cannot be nudged out of a wall contact.
const LATERAL_CAP_FLOOR: Fixed = Fixed::from_raw(48);
/// Lateral velocity retained per tick during a spin-out. The old value was a
/// flat `ZERO`, i.e. a spinning car had *no* lateral friction at all: it slid
/// along the wall for the full second with no way to scrub speed, no way to
/// turn away, and a fresh 60-tick spin started the tick the old one expired.
const SPINOUT_LATERAL_HOLD: Fixed = Fixed::from_raw(2048);
/// Boost pads re-arm only after leaving the pad, preventing per-tick re-triggering.
const BOOST_PAD_COOLDOWN_TICKS: u16 = 12;
/// Into-wall speed (world units/tick) above which a barrier hit counts as a crash.
pub const HARD_IMPACT_THRESHOLD: Fixed = Fixed::from_raw(900);
/// Speed retained after a hard impact (30% scrub).
const HARD_IMPACT_SCRUB: Fixed = Fixed::from_raw(2867);
/// Full nitro meter: 1.5 s of continuous boost, recharging in ~6 s.
pub const NITRO_MAX_TICKS: u16 = 90;
pub const NITRO_RECHARGE_TICKS: u16 = 6;
/// Per-tick nitro thrust (well above the engine's own push).
const NITRO_THRUST: Fixed = Fixed::from_raw(150);
/// Minimum speed for the handbrake to break traction.
const HANDBRAKE_MIN_SPEED: Fixed = Fixed::from_raw(3000);
/// Speed below which braking will cancel a spin-out, giving the driver a way
/// out of a wall they are wedged against.
const SPINOUT_CANCEL_SPEED: Fixed = Fixed::from_int(1);
/// Speed above which driving onto an oil slick breaks the back end.
const OIL_SLICK_MIN_SPEED: Fixed = Fixed::from_int(2);

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
    /// Latched once a drift starts; cleared when the handbrake is released.
    ///
    /// Without this the vehicle re-entered a drift on the very next tick
    /// whenever the handbrake was still held, so a drift that ended by itself
    /// (the slide unwound, or the car scrubbed to a stop) immediately started
    /// again and paid out another turbo, forever.
    pub drift_latched: bool,
    /// Ticks before a wall hit can spin the car out again.
    ///
    /// This is what turns a wall from a life sentence into a penalty: without a
    /// rearm delay the tick after `SPINOUT_TICKS` expired began a fresh
    /// `SPINOUT_TICKS`, and a car wedged against a barrier never recovered.
    pub spinout_cooldown: u16,
    /// Latched once an oil slick has spun the car; cleared on leaving the slick.
    ///
    /// A slick is an area, not an event: re-arming it as soon as the previous
    /// spin-out expired would mean a car parked on one could never drive out.
    pub oil_slick_latch: bool,
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
            drift_latched: false,
            spinout_cooldown: 0,
            oil_slick_latch: false,
            tuning: CarTuning::default(),
        }
    }
}

/// Picks the shortest push that takes a car out of a solid tile and lands it on
/// a non-solid tile inside the grid.
///
/// The old code reflected the velocity but never moved `self.position`, so a car
/// whose centre ended a tick inside a solid tile stayed inside forever and
/// re-collided every tick. Its face selection (`local_x + local_y <= TILE_SIZE`)
/// was also wrong across most of the tile: it claimed the whole upper-left
/// triangle *including the exact centre* (local 32,32) for the left face, so a
/// car entering through a block's top-left corner was shoved sideways along it
/// rather than back out the way it came.
///
/// Returns `(landing position, unit normal)`, or `None` only if the car is
/// enclosed on all four sides -- which cannot happen, because at least one face
/// always has to reach the grid edge or a non-solid tile. Total: every loop is
/// bounded by [`crate::track::MAX_TRACK_DIM`] and no index can go out of bounds
/// (`tile_at` answers `Barrier` for anything past the grid).
fn resolve_solid_exit(track: &TrackDef, from: Vec2, velocity: Vec2) -> Option<(Vec2, Vec2)> {
    let tx = TrackDef::tile_x_of(from.x).min(track.width.saturating_sub(1));
    let ty = TrackDef::tile_y_of(from.y).min(track.height.saturating_sub(1));
    let local_x = from.x.to_int() - (tx as i32 * TILE_SIZE);
    let local_y = from.y.to_int() - (ty as i32 * TILE_SIZE);

    // The four faces, cheapest exit first. A candidate is only accepted if it
    // actually lands the car somewhere it can drive.
    let faces = [
        (local_x, Vec2::new(-Fixed::ONE, Fixed::ZERO), -1i32, 0i32),
        (
            (TILE_SIZE - local_x).max(0),
            Vec2::new(Fixed::ONE, Fixed::ZERO),
            1,
            0,
        ),
        (local_y, Vec2::new(Fixed::ZERO, -Fixed::ONE), 0, -1),
        (
            (TILE_SIZE - local_y).max(0),
            Vec2::new(Fixed::ZERO, Fixed::ONE),
            0,
            1,
        ),
    ];

    let mut best: Option<(i32, Fixed, Vec2, Vec2)> = None;
    for (depth, normal, step_x, step_y) in faces {
        if depth < 0 {
            continue;
        }
        // +1 unit clears the tile, then keep walking while the run of solid
        // tiles continues, so a block several tiles thick is escaped in one go
        // instead of ping-ponging between neighbouring blocks.
        let mut push = depth + 1;
        let mut cursor_x = tx as i32;
        let mut cursor_y = ty as i32;
        let mut steps = 0usize;
        while steps < crate::track::MAX_TRACK_DIM {
            let next_x = cursor_x + step_x;
            let next_y = cursor_y + step_y;
            let in_grid = next_x >= 0
                && next_y >= 0
                && (next_x as u8) < track.width
                && (next_y as u8) < track.height;
            if !in_grid || !track.tile_at(next_x as u8, next_y as u8).is_solid() {
                break;
            }
            // The block is thicker than one tile: keep going.
            push += TILE_SIZE;
            cursor_x = next_x;
            cursor_y = next_y;
            steps += 1;
        }
        let landed = from + normal.scale(Fixed::from_int(push));
        let ltx = TrackDef::tile_x_of(landed.x);
        let lty = TrackDef::tile_y_of(landed.y);
        if ltx >= track.width || lty >= track.height {
            continue;
        }
        if track.tile_at(ltx, lty).is_solid() {
            continue;
        }
        // Shortest push wins; ties break towards the face the car is moving out
        // through, so it keeps its momentum instead of reversing.
        let through = velocity.dot(normal);
        let better = match best {
            None => true,
            Some((best_push, best_through, _, _)) => {
                push < best_push || (push == best_push && through > best_through)
            }
        };
        if better {
            best = Some((push, through, landed, normal));
        }
    }

    best.map(|(_, _, landed, normal)| (landed, normal))
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
        self.drift_latched = false;
        self.spinout_cooldown = 0;
        self.oil_slick_latch = false;
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
    /// `normal` is normalised before use: the impulse is scaled by the normal's
    /// length otherwise, so a caller passing `Vec2::new(Fixed::from_int(5),
    /// Fixed::ZERO)` bounced 12% harder than one passing a unit axis. A
    /// zero-length normal has no direction to resolve against and is ignored.
    ///
    /// Light contact cancels only the into-wall velocity component so the car
    /// slides along the barrier instead of sticking to it; a hard impact also
    /// scrubs speed and kicks the car into a spin-out (GAME.md §5.2 wall sparks)
    /// -- but only while [`VehicleState::spinout_cooldown`] is clear, so a car
    /// wedged against a wall cannot start a new spin-out on the tick the last one
    /// expired.
    ///
    /// `self.speed` is always resynchronised with `|self.velocity|`, so an
    /// external caller cannot leave `speed` feeding stale data into the engine
    /// RPM, the drift gate and the steering gate.
    pub fn handle_barrier_collision(&mut self, normal: Vec2) {
        let len = normal.length();
        if len.raw() <= 0 {
            // Degenerate normal: no axis to resolve against, and dividing by it
            // would blow the scale factor up to i32::MAX. `speed` is still
            // resynchronised so the field cannot go stale on this path either.
            self.speed = self.velocity.length();
            return;
        }
        // Normalise per component in 64 bits: scaling the vector by a Q20.12
        // reciprocal would round a 5x normal down to 4095/4096 and leak the
        // caller's length into the impulse.
        let unit = Vec2::new(
            Fixed::from_raw(((normal.x.raw() as i64 * FP_ONE as i64) / len.raw() as i64) as i32),
            Fixed::from_raw(((normal.y.raw() as i64 * FP_ONE as i64) / len.raw() as i64) as i32),
        );

        let v_dot_n = self.velocity.dot(unit);
        // Only resolve when actually moving into the wall.
        if v_dot_n >= Fixed::ZERO {
            self.speed = self.velocity.length();
            return;
        }

        let impact = -v_dot_n;
        // Restitution coefficient 0.35 (35% bounce) cancels the penetration.
        let restitution = Fixed::from_raw(1433);
        let impulse_mag = (Fixed::ONE + restitution) * impact;
        self.velocity = self.velocity + unit.scale(impulse_mag);

        // Hard head-on hits scrub momentum and spin the car; brushing a wall
        // while scraping along it does not, or the car would be glued in place.
        if impact > HARD_IMPACT_THRESHOLD {
            self.velocity = self.velocity.scale(HARD_IMPACT_SCRUB); // 30% scrub
            if self.spinout_cooldown == 0 && !self.drift.is_spinning() {
                self.spinout_cooldown = crate::drift::SPINOUT_TICKS;
                self.drift.trigger_spinout();
            }
        }

        self.speed = self.velocity.length();
    }

    /// Advances the vehicle physics simulation by one 60Hz tick.
    pub fn tick(&mut self, input: VehicleInput, surface: SurfaceType) {
        if self.boost_pad_cooldown > 0 {
            self.boost_pad_cooldown -= 1;
        }
        if self.spinout_cooldown > 0 {
            self.spinout_cooldown -= 1;
        }

        // Steering authority falls off with speed (understeer), and is reduced
        // while spinning or drifting. Without this the tightest circle the car
        // could describe scaled with speed, so at top speed full lock was a
        // 32-unit radius and the car could not hold a racing line.
        let speed_fraction = (self.speed / BASE_TOP_SPEED).clamp(Fixed::ZERO, Fixed::ONE);
        let authority = if self.drift.is_spinning() {
            SPINOUT_STEER_AUTHORITY
        } else if self.drift.is_drifting() {
            DRIFT_STEER_AUTHORITY
        } else {
            Fixed::ONE - (Fixed::ONE - TOP_SPEED_STEER_AUTHORITY) * speed_fraction
        };

        // 1. Steering -------------------------------------------------------------
        let scaled_turn = self.tuning.scaled_turn_rate(BASE_TURN_RATE);
        if input.steer.raw() != 0 && self.speed > STEER_MIN_SPEED {
            // (turn * steer * authority) in Q12 x3, kept in 64 bits.
            let steer_delta =
                (((scaled_turn as i64 * input.steer.raw() as i64 * authority.raw() as i64)
                    >> math::FP_SHIFT)
                    >> math::FP_SHIFT) as i32;
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
                // Apply the thrust first, then clamp. Sampling the reverse speed
                // before adding the impulse let the car overshoot the cap by a
                // full tick of thrust every time the clamp engaged.
                let mut trial = self.velocity - forward_dir.scale(reverse);
                let backward = -trial.dot(forward_dir);
                if backward > REVERSE_SPEED_LIMIT {
                    trial = trial + forward_dir.scale(backward - REVERSE_SPEED_LIMIT);
                }
                self.velocity = trial;
            }
        }

        // 4. Drift state machine -------------------------------------------------
        // The handbrake must be *released* between drifts. A drift that ends by
        // itself (the slide unwound, or the car scrubbed to a stop) must not
        // immediately re-arm while the button is still down.
        if !input.handbrake {
            self.drift_latched = false;
            if self.drift.is_drifting() {
                // Releasing the handbrake releases the mini-turbo boost.
                let boost_impulse = self.drift.release_drift();
                if boost_impulse > Fixed::ZERO {
                    self.boost_ticks = 30; // 0.5s of turbo boost
                    self.velocity = self.velocity + forward_dir.scale(boost_impulse);
                }
            }
        } else if !self.drift_latched
            && !self.drift.is_drifting()
            && !self.drift.is_spinning()
            && self.speed > HANDBRAKE_MIN_SPEED
        {
            let drift_dir = if input.steer < Fixed::ZERO { -1 } else { 1 };
            self.drift.initiate_drift(drift_dir);
            self.drift_latched = true;
        }

        // An oil slick has almost no lateral grip, so crossing one fast breaks
        // the back end (GAME.md §5.2, `track.rs` `OilSlick`). One spin per
        // crossing: the latch clears only once the car is off the slick again.
        if surface != SurfaceType::OilSlick {
            self.oil_slick_latch = false;
        } else if !self.oil_slick_latch
            && self.speed > OIL_SLICK_MIN_SPEED
            && !self.drift.is_spinning()
        {
            self.oil_slick_latch = true;
            self.spinout_cooldown = crate::drift::SPINOUT_TICKS;
            self.drift.trigger_spinout();
        }

        // A spin-out is a penalty, never a sentence: braking cancels it as soon
        // as the car is slow enough, so the driver always has a way out of a
        // wall they are wedged against.
        if self.drift.is_spinning()
            && input.brake > Fixed::ZERO
            && self.speed < SPINOUT_CANCEL_SPEED
        {
            self.drift.cancel_spinout();
        }

        // `drift.tick` pays out a turbo the drift earned if it ended by itself.
        let banked_boost = self.drift.tick(input.steer, self.speed, &self.tuning);
        if banked_boost > Fixed::ZERO {
            self.boost_ticks = 30;
            self.velocity = self.velocity + forward_dir.scale(banked_boost);
        }
        self.is_drifting = self.drift.is_drifting();
        if self.drift.is_spinning() {
            self.is_reversing = false;
        }

        // 5. Body-frame decomposition, drag and lateral traction ----------------
        let forward_speed = self.velocity.dot(forward_dir);
        let lateral_speed = self.velocity.dot(right_dir);

        let lateral_hold = if self.drift.is_spinning() {
            // A spin-out scrubs sideways speed at half grip. The old value was a
            // flat zero, which let a spinning car slide along a wall for a full
            // second with no lateral friction at all.
            surface.lateral_hold(false) * SPINOUT_LATERAL_HOLD
        } else {
            surface.lateral_hold(self.is_drifting)
        };

        // Tyre friction circle: the tyres can only hold so much side load, and
        // the faster the car goes the less of it there is per unit of speed.
        // This is the cornering limit -- without it the rotating heading alone
        // decides the slide and the car skids permanently off its nose.
        let damped_forward = forward_speed * DRAG_RETAIN;
        let slip_cap_ratio = (MAX_SLIP_RATIO_SLOW
            + (MAX_SLIP_RATIO_FAST - MAX_SLIP_RATIO_SLOW) * speed_fraction)
            * surface.grip_factor();
        // Measured against the *damped* forward component, so the realised slip
        // angle is the cap rather than the cap divided by the drag ratio.
        let lateral_cap = damped_forward.abs() * slip_cap_ratio + LATERAL_CAP_FLOOR;

        let damped_lateral = (lateral_speed * lateral_hold).clamp(-lateral_cap, lateral_cap);

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
        }
        // Always measured from the velocity, never assumed: `length()` saturates
        // for absurd inputs, and assuming the clamp landed exactly on
        // `effective_top` desynchronised `speed` from `|velocity|`.
        self.speed = self.velocity.length();

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
    ///
    /// Total for any position, including one far outside the grid: interior
    /// solids get the car pushed *out* of the tile (not just bounced), and the
    /// outer bound is the last legal coordinate inside the grid rather than
    /// `world_width()`. Clamping to `world_width()` used to park the car on tile
    /// index `width`, which `tile_at` rejects as out of bounds and answers
    /// `Barrier` -- zero traction, zero top speed and a car that can never move
    /// again.
    pub fn collide_with_track(&mut self, track: &TrackDef) -> bool {
        let max_x = track.max_inside_x();
        let max_y = track.max_inside_y();
        let mut hit = false;

        // Solid interior walls (authored into the PSX Super Stages).
        let tx = TrackDef::tile_x_of(self.position.x).min(track.width.saturating_sub(1));
        let ty = TrackDef::tile_y_of(self.position.y).min(track.height.saturating_sub(1));
        if track.tile_at(tx, ty).is_solid() {
            hit = true;
            if let Some((landed, normal)) = resolve_solid_exit(track, self.position, self.velocity)
            {
                self.position = landed;
                self.handle_barrier_collision(normal);
            }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::drift::SPINOUT_TICKS;
    use crate::levels::ALL_TRACKS;
    use crate::timing::{CheckpointGate, ParTimes};
    use crate::track::TrackTile;

    fn flat(throttle: Fixed, steer: Fixed) -> VehicleInput {
        VehicleInput {
            throttle,
            steer,
            ..VehicleInput::default()
        }
    }

    /// All-tarmac grid with a solid block parked in the middle of the road.
    fn walled_track() -> TrackDef {
        // 10x10, ring road on the perimeter, one Barrier at (4..6, 4..6).
        let mut tiles = [TrackTile::Tarmac; 100];
        for ty in 4..6usize {
            for tx in 4..6usize {
                tiles[ty * 10 + tx] = TrackTile::Barrier;
            }
        }
        let gates = [
            CheckpointGate {
                x: 1,
                y: 1,
                width: 1,
                height: 1,
            },
            CheckpointGate {
                x: 8,
                y: 1,
                width: 1,
                height: 1,
            },
            CheckpointGate {
                x: 8,
                y: 8,
                width: 1,
                height: 1,
            },
            CheckpointGate {
                x: 1,
                y: 8,
                width: 1,
                height: 1,
            },
            CheckpointGate {
                x: 1,
                y: 4,
                width: 1,
                height: 1,
            },
            CheckpointGate {
                x: 8,
                y: 4,
                width: 1,
                height: 1,
            },
            CheckpointGate {
                x: 1,
                y: 5,
                width: 1,
                height: 1,
            },
            CheckpointGate {
                x: 8,
                y: 5,
                width: 1,
                height: 1,
            },
            CheckpointGate {
                x: 4,
                y: 1,
                width: 1,
                height: 1,
            },
            CheckpointGate {
                x: 5,
                y: 1,
                width: 1,
                height: 1,
            },
            CheckpointGate {
                x: 4,
                y: 8,
                width: 1,
                height: 1,
            },
            CheckpointGate {
                x: 5,
                y: 8,
                width: 1,
                height: 1,
            },
            CheckpointGate {
                x: 1,
                y: 2,
                width: 1,
                height: 1,
            },
            CheckpointGate {
                x: 8,
                y: 2,
                width: 1,
                height: 1,
            },
            CheckpointGate {
                x: 1,
                y: 7,
                width: 1,
                height: 1,
            },
            CheckpointGate {
                x: 8,
                y: 7,
                width: 1,
                height: 1,
            },
        ];
        TrackDef {
            name: "WALLED",
            width: 10,
            height: 10,
            start_pos: Vec2::new(Fixed::from_int(96), Fixed::from_int(96)),
            start_heading: 0,
            start_gate: CheckpointGate {
                x: 1,
                y: 1,
                width: 1,
                height: 1,
            },
            par_times: ParTimes::default(),
            checkpoint_count: 8,
            checkpoints: gates,
            tiles: Box::leak(Box::new(tiles)),
        }
    }

    fn solid_at(track: &TrackDef, tx: u8, ty: u8) -> bool {
        track.tile_at(tx, ty).is_solid()
    }

    // ========================================================================
    // bug 16: cornering force limit
    // ========================================================================

    #[test]
    fn full_lock_cornering_saturates_the_slip_angle() {
        // Sustained full lock at full throttle used to settle into a permanent
        // ~38.8 degree skid (|lateral| / |forward| == 0.804): `lateral_hold` was
        // a flat 3641/4096 per tick no matter the speed or the steering angle,
        // and the rotating heading kept feeding sideways velocity faster than
        // the tyres could scrub it. Nothing about that scaled with speed, so it
        // was identical on a 10x10 and a 30x30 circuit.
        let mut car = VehicleState::default();
        let input = flat(Fixed::ONE, Fixed::ONE);
        let mut worst = Fixed::ZERO;
        for _ in 0..600 {
            car.tick(input, SurfaceType::Tarmac);
            let forward = car.velocity.dot(car.forward_dir());
            if forward > Fixed::ZERO {
                let slip = car.velocity.dot(car.right_dir()).abs() / forward;
                worst = worst.max(slip);
            }
        }
        // 2048/4096 == 0.5 == 26.6 degrees: the car is riding the edge of its
        // tyres and no further. Measured after the fix: 0.27 at half lock.
        assert!(
            worst <= Fixed::from_raw(2048),
            "full lock produced a {:.1} degree slide (slip ratio {:?})",
            (worst.raw() as f64).atan().to_degrees(),
            worst.raw()
        );
    }

    #[test]
    fn steering_authority_falls_off_with_speed() {
        // Same steering input at two speeds: the fast car must turn less per
        // tick, otherwise the tightest circle the car can describe scales with
        // speed (full lock at 3.4 u/tick was a 32-unit radius, half a tile).
        let rate_at = |speed: Fixed| -> i32 {
            let mut car = VehicleState {
                // heading 0 points north, so travel is along -Y.
                velocity: Vec2::new(Fixed::ZERO, -speed),
                speed,
                ..VehicleState::default()
            };
            car.tick(flat(Fixed::ONE, Fixed::ONE), SurfaceType::Tarmac);
            crate::ai::angle_error(0, car.heading)
        };
        let crawl = rate_at(Fixed::from_raw(250));
        let fast = rate_at(BASE_TOP_SPEED);
        assert!(crawl > 0, "steering must work at a crawl");
        assert!(
            fast * 5 <= crawl * 4,
            "authority did not fall off with speed: {crawl} BAM/tick crawling vs {fast} at top speed"
        );
        assert!(fast > 0, "steering must not vanish entirely at top speed");
    }

    #[test]
    fn a_corner_has_a_floor_on_its_radius_at_top_speed() {
        // The full-lock radius at the top speed must be at least a tile wide, or
        // the car can pivot on the spot and the racing line is meaningless.
        let mut car = VehicleState::default();
        for _ in 0..240 {
            car.tick(flat(Fixed::ONE, Fixed::ONE), SurfaceType::Tarmac);
        }
        assert!(car.speed > Fixed::ONE, "the car must be moving");
        let mut previous = car.position;
        let mut distance = Fixed::ZERO;
        for _ in 0..240 {
            car.tick(flat(Fixed::ONE, Fixed::ONE), SurfaceType::Tarmac);
            distance = distance + (car.position - previous).length();
            previous = car.position;
        }
        // 240 ticks at full lock is 240 * 70 BAM of heading; at 4096 BAM per
        // revolution that is 4.1 revolutions, so the distance covered is the
        // circumference times (turn / 4096).
        let turns_raw = 240i64 * BASE_TURN_RATE as i64;
        let circumference = distance.raw() as i64 * 4096 / turns_raw;
        // r = C / 2*pi, in Q20.12: r_raw = C_raw * 4096 / (2*pi*4096).
        let radius = (circumference * 4096 / 25_133) as i32;
        assert!(
            radius >= TILE_SIZE,
            "full lock at top speed describes a {radius}-unit radius ({distance:?} over {turns_raw} BAM)"
        );
    }

    #[test]
    fn a_sliding_car_scrubs_sideways_speed_faster_than_a_gripping_one() {
        // Bug 17: `lateral_hold(true)` returned 3640 against `lateral_hold(false)`
        // = 3641, so a handbrake drift changed nothing on tarmac and the doc
        // comment claiming "oil slicks and drifts let it slide" was false.
        let initial = Vec2::new(Fixed::from_raw(2048), -Fixed::from_int(3));
        let mk = || VehicleState {
            velocity: initial,
            speed: initial.length(),
            ..VehicleState::default()
        };
        let mut grip = mk();
        let mut slide = mk();
        // Heading 0 points north, so `right` is +X and the car is already moving
        // half a unit sideways at 3 u/tick forward -- fast enough that the
        // friction circle is not the binding constraint.
        grip.tick(flat(Fixed::ZERO, Fixed::ZERO), SurfaceType::Tarmac);
        slide.tick(
            VehicleInput {
                handbrake: true,
                ..VehicleInput::default()
            },
            SurfaceType::Tarmac,
        );
        assert!(slide.is_drifting, "the handbrake must break traction");
        assert_ne!(slide.visual_angle, slide.heading, "the tail must be out");
        assert!(
            slide.velocity.dot(slide.right_dir()) < grip.velocity.dot(grip.right_dir()),
            "drifting ({:?}) scrubbed no more sideways speed than gripping ({:?})",
            slide.velocity.dot(slide.right_dir()),
            grip.velocity.dot(grip.right_dir())
        );
        // A drifting car on tarmac must let go more than off-road grip, which is
        // the ordering the doc comment promises.
        assert!(
            SurfaceType::Tarmac.lateral_hold(true) < SurfaceType::Tarmac.lateral_hold(false),
            "drifting must change lateral grip on tarmac"
        );
        assert!(
            SurfaceType::OffRoad.lateral_hold(true) > SurfaceType::Tarmac.lateral_hold(true),
            "off-road should still be more slidey than tarmac"
        );
    }

    #[test]
    fn an_oil_slick_breaks_the_back_end() {
        // `track.rs` documents OilSlick as "immediate loss of lateral grip /
        // spin" but nothing implemented it: the only caller of
        // `DriftState::trigger_spinout` was the wall.
        let mut car = VehicleState {
            velocity: Vec2::new(Fixed::ZERO, -Fixed::from_int(3)),
            speed: Fixed::from_int(3),
            ..VehicleState::default()
        };
        car.tick(flat(Fixed::ONE, Fixed::ZERO), SurfaceType::OilSlick);
        assert!(
            car.drift.is_spinning(),
            "crossing an oil slick at speed must spin the car"
        );
        // And it is a single spin, not one every tick.
        let mut spins = 1;
        let mut was_spinning = true;
        for _ in 0..(SPINOUT_TICKS + 10) {
            car.tick(flat(Fixed::ONE, Fixed::ZERO), SurfaceType::OilSlick);
            let spinning = car.drift.is_spinning();
            if spinning && !was_spinning {
                spins += 1;
            }
            was_spinning = spinning;
        }
        assert_eq!(spins, 1, "the slick re-triggered {spins} times");
    }

    // ========================================================================
    // bug 1: wall contact must be a penalty, not a death sentence
    // ========================================================================

    /// Drives into a wall at speed on `track` and returns how many ticks the car
    /// spent unable to recover without pressing respawn.
    fn ticks_trapped_against_wall(track: &TrackDef) -> u32 {
        let mut car = VehicleState::default();
        let input = VehicleInput {
            throttle: Fixed::ONE,
            nitro: true,
            ..VehicleInput::default()
        };
        let mut trapped = 0u32;
        for _ in 0..600 {
            car.tick_on_track(input, track);
            // "Trapped" = not in control and not moving away from the wall.
            if car.drift.is_spinning() {
                trapped += 1;
            }
        }
        trapped
    }

    #[test]
    fn a_wall_hit_is_always_recoverable_without_respawning() {
        // Reproduced the bug: 569 of 600 ticks stuck in SpinOut against the east
        // wall, because the spin-out disabled reverse and the steering, gave the
        // car no lateral friction at all, and the tick after the 60-tick window
        // expired began a fresh 60-tick spin.
        let track = ALL_TRACKS[0];
        let trapped = ticks_trapped_against_wall(track);
        assert!(
            trapped < SPINOUT_TICKS as u32 * 2,
            "{trapped}/600 ticks were spent spinning against the wall"
        );
    }

    #[test]
    fn a_wedged_car_can_reverse_out_of_a_wall() {
        // The player-facing escape hatch: brake, and the spin-out is cancelled
        // and the car backs off, with no respawn.
        let track = ALL_TRACKS[0];
        let mut car = VehicleState {
            position: Vec2::new(Fixed::from_int(560), Fixed::from_int(320)),
            heading: 1024, // due east
            velocity: Vec2::new(Fixed::from_int(3), Fixed::ZERO),
            speed: Fixed::from_int(3),
            ..VehicleState::default()
        };
        let gas = flat(Fixed::ONE, Fixed::ZERO);
        for _ in 0..240 {
            car.tick_on_track(gas, track);
        }
        assert!(car.drift.is_spinning(), "the wall must have spun the car");

        // Now brake: reverse cancels the spin and the car moves back down the road.
        let brake = VehicleInput {
            brake: Fixed::ONE,
            ..VehicleInput::default()
        };
        let mut cancelled_at = 0u32;
        let mut reversing_seen = false;
        for tick in 1..=120u32 {
            car.tick_on_track(brake, track);
            if !car.drift.is_spinning() {
                if cancelled_at == 0 {
                    cancelled_at = tick;
                }
                if car.is_reversing {
                    reversing_seen = true;
                }
            }
        }
        assert!(
            cancelled_at > 0 && cancelled_at <= SPINOUT_TICKS as u32,
            "reverse never cancelled the spin-out (still spinning after 120 ticks)"
        );
        assert!(
            reversing_seen,
            "the car never engaged reverse once the spin-out was cancelled"
        );
        assert!(
            car.position.x.to_int() < 620,
            "the car never backed away from the wall: x={} (wall is at {})",
            car.position.x.to_int(),
            track.max_inside_x()
        );
    }

    #[test]
    fn the_drivable_circuit_never_traps_the_car_on_every_track() {
        // Every wall, on every shipped circuit, from every edge.
        for track in ALL_TRACKS.iter() {
            for (heading, start) in [
                (
                    1024u16,
                    Vec2::new(Fixed::from_int(32), Fixed::from_int(320)),
                ), // east
                (
                    2048u16,
                    Vec2::new(Fixed::from_int(320), Fixed::from_int(32)),
                ), // south
                (0u16, Vec2::new(Fixed::from_int(320), Fixed::from_int(608))), // north
                (
                    3072u16,
                    Vec2::new(Fixed::from_int(608), Fixed::from_int(320)),
                ), // west
            ] {
                // Point the car at the wall it is heading for.
                let dir = Vec2::new(math::cos(heading), math::sin(heading));
                let mut car = VehicleState {
                    position: start,
                    heading,
                    velocity: dir.scale(Fixed::from_int(3)),
                    speed: Fixed::from_int(3),
                    ..VehicleState::default()
                };
                // Holding the throttle into the wall means crashing again and
                // again; what must not happen is one unbroken spin-out that the
                // driver cannot influence, because the next one used to start on
                // the very tick the last one expired.
                let gas = flat(Fixed::ONE, Fixed::ZERO);
                let mut run = 0u32;
                let mut longest_run = 0u32;
                for _ in 0..600 {
                    car.tick_on_track(gas, track);
                    if car.drift.is_spinning() {
                        run += 1;
                        longest_run = longest_run.max(run);
                    } else {
                        run = 0;
                    }
                }
                assert!(
                    longest_run <= SPINOUT_TICKS as u32 + 4,
                    "{}: an unbroken {longest_run}-tick SpinOut (heading {heading})",
                    track.name
                );
            }
        }
    }

    #[test]
    fn a_car_against_a_wall_keeps_some_steering_authority() {
        // While spinning the car must still respond to the wheel, otherwise the
        // player cannot even point it away from the barrier.
        let mut car = VehicleState::default();
        car.drift.trigger_spinout();
        car.speed = Fixed::from_int(2);
        car.velocity = car.forward_dir().scale(Fixed::from_int(2));
        let start = car.heading;
        car.tick(flat(Fixed::ZERO, Fixed::ONE), SurfaceType::Tarmac);
        assert!(
            car.heading != start,
            "a spinning car must still be steerable"
        );
    }

    #[test]
    fn a_spinning_car_still_scrubs_sideways_speed() {
        // `lateral_hold` used to be a flat ZERO while spinning: the car was a
        // frictionless puck for the whole 60 ticks.
        let mut car = VehicleState {
            velocity: Vec2::new(Fixed::from_int(2), Fixed::from_int(2)),
            speed: Vec2::new(Fixed::from_int(2), Fixed::from_int(2)).length(),
            ..VehicleState::default()
        };
        car.drift.trigger_spinout();
        for _ in 0..10 {
            car.tick(flat(Fixed::ZERO, Fixed::ZERO), SurfaceType::Tarmac);
        }
        let lateral = car.velocity.dot(car.right_dir()).abs();
        assert!(
            lateral < Fixed::from_int(2),
            "a spinning car kept all its sideways speed ({lateral:?})"
        );
        assert!(
            car.velocity.length() < Vec2::new(Fixed::from_int(2), Fixed::from_int(2)).length(),
            "a spin-out must scrub speed"
        );
    }

    // ========================================================================
    // bug 13: the right / bottom world wall
    // ========================================================================

    #[test]
    fn the_outer_bounds_clamp_keeps_the_car_on_a_valid_tile() {
        // `world_width()` is 640 for a 10-wide track, so clamping to it parked the
        // car on tile index 10 -- which `tile_at` rejects as out of bounds and
        // answers `Barrier` for. Traction 0, top speed 0, dead car.
        for track in ALL_TRACKS.iter() {
            let max_x = track.max_inside_x();
            let max_y = track.max_inside_y();
            assert!(max_x < track.world_width());
            assert!(max_y < track.world_height());

            for (x, y) in [
                (max_x + 400, max_y - 1),
                (max_x, max_y),
                (-400, max_y + 400),
                (max_x + 400, max_y + 400),
            ] {
                let mut car = VehicleState {
                    position: Vec2::new(Fixed::from_int(x), Fixed::from_int(y)),
                    velocity: Vec2::new(Fixed::from_int(4), Fixed::from_int(4)),
                    speed: Fixed::from_int(4),
                    ..VehicleState::default()
                };
                car.collide_with_track(track);
                let tx = TrackDef::tile_x_of(car.position.x);
                let ty = TrackDef::tile_y_of(car.position.y);
                assert!(
                    tx < track.width && ty < track.height,
                    "{}: clamped to tile ({tx},{ty}) outside the {w}x{h} grid",
                    track.name,
                    w = track.width,
                    h = track.height
                );
                assert!(
                    !solid_at(track, tx, ty),
                    "{}: clamped onto a solid tile ({tx},{ty})",
                    track.name
                );
                assert!(
                    track.surface_at(tx, ty).max_speed_factor() > Fixed::ZERO,
                    "{}: clamped onto an undrivable surface at ({tx},{ty})",
                    track.name
                );
            }
        }
    }

    #[test]
    fn a_car_pinned_against_the_east_wall_can_still_accelerate() {
        // Reproduced the bug: 5 s of full throttle plus nitro against the east
        // wall left the car at speed 0.0000, because it was sitting on a Barrier
        // tile with zero traction and zero top speed.
        let track = ALL_TRACKS[0];
        let mut car = VehicleState {
            position: Vec2::new(Fixed::from_int(630), Fixed::from_int(320)),
            heading: 1024,
            velocity: Vec2::new(Fixed::from_int(3), Fixed::ZERO),
            speed: Fixed::from_int(3),
            ..VehicleState::default()
        };
        let gas = VehicleInput {
            throttle: Fixed::ONE,
            nitro: true,
            ..VehicleInput::default()
        };
        for _ in 0..300 {
            car.tick_on_track(gas, track);
        }
        assert!(
            car.speed > Fixed::ZERO,
            "the car against the east wall has no speed at all"
        );
        // And it is on a drivable tile.
        let tx = TrackDef::tile_x_of(car.position.x);
        assert!(tx < track.width, "clamped onto tile {tx}");
        assert!(!solid_at(track, tx, TrackDef::tile_y_of(car.position.y)));
    }

    #[test]
    fn the_outer_bound_is_the_last_legal_coordinate_of_the_grid() {
        for track in ALL_TRACKS.iter() {
            for units in [0, 1, 63, 64, track.world_width() - 1] {
                let tx = TrackDef::tile_x_of(Fixed::from_int(units));
                assert!(tx < track.width, "{}: x={units} -> {tx}", track.name);
            }
            for units in [0, 1, 63, 64, track.world_height() - 1] {
                let ty = TrackDef::tile_y_of(Fixed::from_int(units));
                assert!(ty < track.height, "{}: y={units} -> {ty}", track.name);
            }
        }
    }

    // ========================================================================
    // bug 14: interior solids
    // ========================================================================

    #[test]
    fn a_solid_tile_pushes_the_car_out_of_it() {
        // The old code reflected the velocity but never moved the position, so
        // a car whose centre ended a tick inside a solid tile stayed inside
        // forever and re-collided every tick.
        let track = walled_track();
        for (tx, ty) in [(4u8, 4u8), (5, 5), (4, 5), (5, 4)] {
            assert!(solid_at(&track, tx, ty), "test setup: ({tx},{ty})");
            // Drop the car just inside the top-left of the block.
            let inside = Vec2::new(
                Fixed::from_int(tx as i32 * TILE_SIZE + 4),
                Fixed::from_int(ty as i32 * TILE_SIZE + 4),
            );
            let mut car = VehicleState {
                position: inside,
                velocity: Vec2::new(Fixed::from_int(1), Fixed::from_int(1)),
                speed: Fixed::from_int(1),
                ..VehicleState::default()
            };
            car.collide_with_track(&track);
            let nx = TrackDef::tile_x_of(car.position.x);
            let ny = TrackDef::tile_y_of(car.position.y);
            assert!(
                !solid_at(&track, nx, ny),
                "({tx},{ty}): car is still inside a solid tile at ({nx},{ny})"
            );
        }
    }

    #[test]
    fn a_solid_tile_pushes_the_car_out_through_the_nearest_face() {
        let track = walled_track();
        // A car whose centre sits 4 units inside the LEFT edge of the block at
        // (4,4) must exit west; 4 units inside the TOP edge must exit north.
        let cases = [
            (4i32, 4i32, 4i32, 20i32, -1i32, 0i32), // left face -> (-1, 0)
            (4, 4, 60, 4, 0, -1),                   // top face  -> ( 0,-1)
            (5, 5, 4, 60, 0, 1),                    // right block, bottom face
            // Equidistant from all four faces of the block's corner tile: the
            // tie breaks towards the face the car is moving out through.
            (4, 4, 60, 60, 0, -1),
        ];
        for (tx, ty, lx, ly, nx_expected, ny_expected) in cases {
            let mut car = VehicleState {
                position: Vec2::new(
                    Fixed::from_int(tx * TILE_SIZE + lx),
                    Fixed::from_int(ty * TILE_SIZE + ly),
                ),
                velocity: Vec2::new(
                    Fixed::ZERO,
                    if ny_expected < 0 {
                        -Fixed::ONE
                    } else {
                        Fixed::ZERO
                    },
                ),
                speed: Fixed::ONE,
                ..VehicleState::default()
            };
            let before = car.position;
            car.collide_with_track(&track);
            let dx = (car.position.x - before.x).to_int();
            let dy = (car.position.y - before.y).to_int();
            let moved_x = dx * nx_expected;
            let moved_y = dy * ny_expected;
            assert!(
                (moved_x > 0 && moved_y >= 0) || (moved_y > 0 && moved_x >= 0),
                "({tx},{ty})+({lx},{ly}) exited by ({dx},{dy}), expected a push out of ({nx_expected},{ny_expected})"
            );
            let nx = TrackDef::tile_x_of(car.position.x);
            let ny = TrackDef::tile_y_of(car.position.y);
            assert!(
                !solid_at(&track, nx, ny),
                "({tx},{ty})+({lx},{ly}) ended inside a solid tile ({nx},{ny})"
            );
        }
    }

    #[test]
    fn the_exact_tile_centre_still_exits_a_solid_tile() {
        // The old `local_x + local_y <= TILE_SIZE` test claimed the whole
        // upper-left triangle *including* (32, 32) for the left face, so the
        // exact centre was pushed left rather than out of the nearest face.
        let track = walled_track();
        for tx in 4i32..=5 {
            for ty in 4i32..=5 {
                let mut car = VehicleState {
                    position: Vec2::new(
                        Fixed::from_int(tx * TILE_SIZE + TILE_SIZE / 2),
                        Fixed::from_int(ty * TILE_SIZE + TILE_SIZE / 2),
                    ),
                    velocity: Vec2::ZERO,
                    speed: Fixed::ZERO,
                    ..VehicleState::default()
                };
                car.collide_with_track(&track);
                let nx = TrackDef::tile_x_of(car.position.x);
                let ny = TrackDef::tile_y_of(car.position.y);
                assert!(
                    !solid_at(&track, nx, ny),
                    "tile centre ({tx},{ty}) did not escape: ended at ({nx},{ny})"
                );
            }
        }
    }

    #[test]
    fn interior_walls_do_not_trap_the_car_in_a_push_out_loop() {
        // A block hard against the right edge of the grid cannot be exited to the
        // right; the resolution must not ping-pong against the outer bound.
        let mut tiles = [TrackTile::Tarmac; 100];
        for ty in 3..5usize {
            for tx in 8..10usize {
                tiles[ty * 10 + tx] = TrackTile::Barrier;
            }
        }
        let gates = [CheckpointGate {
            x: 1,
            y: 1,
            width: 1,
            height: 1,
        }; 16];
        let track = TrackDef {
            name: "EDGEBLOCK",
            width: 10,
            height: 10,
            start_pos: Vec2::new(Fixed::from_int(96), Fixed::from_int(96)),
            start_heading: 0,
            start_gate: CheckpointGate {
                x: 1,
                y: 1,
                width: 1,
                height: 1,
            },
            par_times: ParTimes::default(),
            checkpoint_count: 4,
            checkpoints: gates,
            tiles: Box::leak(Box::new(tiles)),
        };
        let mut car = VehicleState {
            position: Vec2::new(
                Fixed::from_int(9 * TILE_SIZE + 32),
                Fixed::from_int(3 * TILE_SIZE + 32),
            ),
            velocity: Vec2::new(Fixed::from_int(2), Fixed::ZERO),
            speed: Fixed::from_int(2),
            ..VehicleState::default()
        };
        let gas = flat(Fixed::ONE, Fixed::ZERO);
        for _ in 0..600 {
            car.tick_on_track(gas, &track);
            let tx = TrackDef::tile_x_of(car.position.x);
            let ty = TrackDef::tile_y_of(car.position.y);
            assert!(
                tx < track.width && ty < track.height && !solid_at(&track, tx, ty),
                "the car got stuck at tile ({tx},{ty})"
            );
        }
    }

    #[test]
    fn collide_with_track_is_total_for_a_position_far_off_the_grid() {
        let track = walled_track();
        for (x, y) in [
            (0i32, 0i32),
            (-1, -1),
            (i32::MAX, i32::MAX),
            (i32::MIN, i32::MIN),
            (5_000_000, -5_000_000),
        ] {
            let mut car = VehicleState {
                position: Vec2::new(
                    Fixed::from_raw(x.saturating_mul(4096)),
                    Fixed::from_raw(y.saturating_mul(4096)),
                ),
                velocity: Vec2::new(Fixed::from_int(1), Fixed::from_int(1)),
                speed: Fixed::from_int(1),
                ..VehicleState::default()
            };
            car.collide_with_track(&track);
            let tx = TrackDef::tile_x_of(car.position.x);
            let ty = TrackDef::tile_y_of(car.position.y);
            assert!(
                tx < track.width && ty < track.height,
                "({x},{y}) clamped outside the grid to ({tx},{ty})"
            );
        }
    }

    // ========================================================================
    // bug 15: handle_barrier_collision
    // ========================================================================

    #[test]
    fn barrier_collision_always_resyncs_speed() {
        // `speed` is a separate field from `|velocity|` and feeds the engine RPM,
        // the drift gate and the steering gate. A `pub` caller must not be able
        // to desynchronise it.
        for (velocity, normal) in [
            (
                Vec2::new(Fixed::from_int(-4), Fixed::from_int(2)),
                Vec2::new(Fixed::ONE, Fixed::ZERO),
            ),
            (
                Vec2::new(Fixed::from_int(-4), Fixed::ZERO),
                Vec2::new(Fixed::ONE, Fixed::ZERO),
            ),
            (
                Vec2::new(Fixed::from_int(4), Fixed::ZERO),
                Vec2::new(Fixed::ONE, Fixed::ZERO),
            ),
            (
                Vec2::new(Fixed::ZERO, Fixed::from_int(-1)),
                Vec2::new(Fixed::ZERO, Fixed::ONE),
            ),
            (
                Vec2::new(Fixed::from_int(-1), Fixed::from_int(-1)),
                Vec2::new(Fixed::ONE, Fixed::ONE),
            ),
            (
                Vec2::new(Fixed::from_int(-1), Fixed::from_int(-1)),
                Vec2::new(Fixed::from_raw(4096), Fixed::from_raw(4096)),
            ),
        ] {
            let mut car = VehicleState {
                velocity,
                speed: Fixed::from_int(99), // deliberately wrong
                ..VehicleState::default()
            };
            car.handle_barrier_collision(normal);
            let expected = car.velocity.length();
            assert_eq!(
                car.speed, expected,
                "speed {expected:?} out of sync with |velocity| after normal {normal:?}"
            );
        }
    }

    #[test]
    fn barrier_collision_normalises_the_normal() {
        // A non-unit normal used to scale the impulse by its own length, so a
        // diagonal normal bounced twice as hard as an axis one.
        let unit = Vec2::new(Fixed::ONE, Fixed::ZERO);
        let scaled = Vec2::new(Fixed::from_int(5), Fixed::ZERO);
        let mut a = VehicleState {
            velocity: Vec2::new(Fixed::from_int(-3), Fixed::from_int(1)),
            speed: Fixed::ZERO,
            ..VehicleState::default()
        };
        let mut b = a;
        a.handle_barrier_collision(unit);
        b.handle_barrier_collision(scaled);
        assert_eq!(
            a.velocity, b.velocity,
            "the normal's length leaked into the impulse"
        );
    }

    #[test]
    fn barrier_collision_ignores_a_degenerate_normal() {
        for normal in [Vec2::ZERO, Vec2::new(Fixed::ZERO, Fixed::ZERO)] {
            let mut car = VehicleState {
                velocity: Vec2::new(Fixed::from_int(-3), Fixed::ZERO),
                speed: Fixed::from_int(-3),
                ..VehicleState::default()
            };
            car.handle_barrier_collision(normal);
            assert_eq!(car.velocity, Vec2::new(Fixed::from_int(-3), Fixed::ZERO));
            assert_eq!(car.speed, Fixed::from_int(3));
        }
    }

    #[test]
    fn a_hard_hit_scrubs_speed_but_a_glance_does_not() {
        // The normal is +Y, so both cars are travelling north into it; the
        // glancing one only clips it at 300 raw (well under HARD_IMPACT_THRESHOLD
        // = 900), the hard one arrives at 4 u/tick.
        let mut glancing = VehicleState {
            velocity: Vec2::new(Fixed::from_int(2), Fixed::from_raw(-300)),
            speed: Fixed::from_int(2),
            ..VehicleState::default()
        };
        let mut hard = VehicleState {
            velocity: Vec2::new(Fixed::ZERO, Fixed::from_int(-4)),
            speed: Fixed::from_int(4),
            ..VehicleState::default()
        };
        glancing.handle_barrier_collision(Vec2::new(Fixed::ZERO, Fixed::ONE));
        hard.handle_barrier_collision(Vec2::new(Fixed::ZERO, Fixed::ONE));
        assert!(!glancing.drift.is_spinning(), "a glance must not spin");
        assert!(hard.drift.is_spinning(), "a head-on hit must spin");
        assert!(hard.speed < glancing.speed);
    }

    // ========================================================================
    // drift gating / latch
    // ========================================================================

    #[test]
    fn a_held_handbrake_cannot_re_arm_a_drift_that_ended_by_itself() {
        let mut car = VehicleState {
            velocity: Vec2::new(Fixed::ZERO, -Fixed::from_int(3)),
            speed: Fixed::from_int(3),
            ..VehicleState::default()
        };
        // The cheapest possible sustained drift: stab the handbrake into a slide
        // and then hold it on a light counter-steer, never letting go.
        let stab = VehicleInput {
            throttle: Fixed::ONE,
            steer: Fixed::from_raw(600),
            handbrake: true,
            ..VehicleInput::default()
        };
        let held = VehicleInput {
            throttle: Fixed::ONE,
            steer: Fixed::from_raw(-600),
            handbrake: true,
            ..VehicleInput::default()
        };
        car.tick(stab, SurfaceType::Tarmac);
        assert!(car.is_drifting);
        let mut payouts = 0u32;
        let mut previous_boost = 0u16;
        for _ in 0..400 {
            car.tick(held, SurfaceType::Tarmac);
            if car.boost_ticks > previous_boost {
                payouts += 1;
            }
            previous_boost = car.boost_ticks;
        }
        assert_eq!(payouts, 1, "a held handbrake paid out {payouts} turbos");
    }

    #[test]
    fn releasing_the_handbrake_re_arms_the_drift() {
        let mut car = VehicleState {
            velocity: Vec2::new(Fixed::ZERO, -Fixed::from_int(3)),
            speed: Fixed::from_int(3),
            ..VehicleState::default()
        };
        let held = VehicleInput {
            throttle: Fixed::ONE,
            steer: Fixed::ONE,
            handbrake: true,
            ..VehicleInput::default()
        };
        car.tick(held, SurfaceType::Tarmac);
        assert!(car.is_drifting);
        for _ in 0..200 {
            car.tick(held, SurfaceType::Tarmac);
            if !car.is_drifting {
                break;
            }
        }
        assert!(
            !car.is_drifting,
            "over-steering into the slide must end the drift"
        );
        // Wait out the spin-out the over-steering caused.
        for _ in 0..(crate::drift::SPINOUT_TICKS + 2) {
            car.tick(VehicleInput::default(), SurfaceType::Tarmac);
        }
        assert!(!car.drift.is_spinning());
        // Release and press again: a fresh drift is allowed.
        car.tick(VehicleInput::default(), SurfaceType::Tarmac);
        assert!(
            !car.drift_latched,
            "releasing the handbrake clears the latch"
        );
        car.tick(held, SurfaceType::Tarmac);
        assert!(
            car.is_drifting,
            "a fresh handbrake press must start a new drift"
        );
    }

    #[test]
    fn respawn_clears_every_state_that_could_pin_the_car() {
        let mut car = VehicleState {
            is_drifting: true,
            drift_latched: true,
            spinout_cooldown: 40,
            oil_slick_latch: true,
            boost_ticks: 20,
            nitro_charge: 0,
            is_reversing: true,
            gear: 5,
            ..VehicleState::default()
        };
        car.drift.trigger_spinout();
        car.respawn_at(Vec2::ZERO, 4095);
        assert!(!car.drift_latched);
        assert_eq!(car.spinout_cooldown, 0);
        assert!(!car.oil_slick_latch);
        assert_eq!(car.boost_ticks, 0);
        assert_eq!(car.nitro_charge, NITRO_MAX_TICKS);
        assert_eq!(car.drift, DriftState::Grip);
        assert_eq!(car.heading, 4095 & 0x0FFF);
    }

    #[test]
    fn tick_is_total_over_the_whole_fixed_input_range() {
        for steer_raw in [i32::MIN, -100_000, -4096, 0, 4096, i32::MAX] {
            for throttle_raw in [i32::MIN, 0, 4096, i32::MAX] {
                for brake_raw in [0i32, 2048, i32::MAX] {
                    for speed_raw in [i32::MIN, -1, 0, 14000, i32::MAX] {
                        let mut car = VehicleState {
                            velocity: Vec2::new(
                                Fixed::from_raw(speed_raw),
                                Fixed::from_raw(speed_raw / 2),
                            ),
                            speed: Fixed::from_raw(speed_raw),
                            boost_ticks: 3,
                            ..VehicleState::default()
                        };
                        car.tick(
                            VehicleInput {
                                throttle: Fixed::from_raw(throttle_raw),
                                brake: Fixed::from_raw(brake_raw),
                                steer: Fixed::from_raw(steer_raw),
                                handbrake: true,
                                nitro: true,
                            },
                            SurfaceType::Tarmac,
                        );
                        // The speed field must always agree with the velocity.
                        assert_eq!(car.speed, car.velocity.length());
                    }
                }
            }
        }
    }
}
