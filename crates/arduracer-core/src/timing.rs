//! Checkpoint tracking, lap timer, and target par times.
//!
//! Operates on a 60 Hz fixed tick rate matching PSX NTSC 60 FPS display.
//!
//! Lap validation reproduces ArduRacer FX exactly (see `racer.cpp`): a lap only
//! counts when the car *leaves* the start/finish block having already touched
//! every checkpoint. Checkpoints may be taken in any order, so cutting a corner
//! short simply leaves that checkpoint uncounted for the lap - it cannot be
//! skipped, but the player is never forced into a rigid gate sequence either.
//! Gate order-independence is a deliberate gameplay choice (TODO.md FIX-01:
//! raster-scan gate order made 23 of 24 circuits uncompletable), and it is why
//! *every* start/finish crossing resets the gate progress rather than only a
//! scoring one: with coverage semantics, keeping the mask across a crossing
//! would let a car bank gates over several aborted attempts and score a lap it
//! never drove.
//!
//! Independently of gate coverage, a completed lap is only accepted as a *lap
//! time* when it is physically plausible -- see [`MIN_PLAUSIBLE_LAP_TICKS`] and
//! [`ParTimes::min_plausible_lap_ticks`]. The lap itself always counts (the lap
//! counter advances exactly once per completed circuit); only the time is
//! discarded, so a cheat cannot write itself to the memory card.

use crate::math::{Fixed, Vec2};
use crate::track::{TrackDef, TrackTile};
use crate::vehicle::VehicleState;

/// Ticks per second (60 Hz NTSC).
pub const TICKS_PER_SECOND: u32 = 60;
/// Default timed laps in an arcade time trial.
pub const TOTAL_LAPS: u8 = 5;

/// Most gates a single [`LapTimer`] can track, set by the width of its bitmask.
///
/// [`crate::track::MAX_TRACK_CHECKPOINTS`] is 16, so every shipped circuit fits with
/// room to spare; a wider timer (or a `total_checkpoints` field poked by a
/// caller) is clamped to this count instead of aliasing gates against each
/// other or indexing past the end of the gate array.
pub const MAX_TRACKED_CHECKPOINTS: usize = 64;

/// Slack, in tiles, allowed around every gate when deciding whether the car
/// touched it.
///
/// Every gate on every shipped circuit -- checkpoints and the start/finish
/// alike -- is a single 1x1 tile, 32x32 world units, while the road it spans is
/// 12 tiles wide at the line and wider through the corners. Testing the car's
/// tile for exact equality with a one-tile rectangle therefore made a gate
/// register only if the car happened to occupy that one tile out of the dozen
/// or so it was legally allowed to drive across: taking a wide line, running
/// wide over a curb or getting pushed off line was enough to silently drop the
/// gate. Worst of all, a start/finish crossing that missed its one tile never
/// latched `on_start_gate`, so `left_start` never fired and the lap simply
/// never counted no matter how many times the line was driven.
///
/// One tile of slack is what the rivals already run on
/// ([`crate::ai::AiRacer`] registers gates with `contains_tile_or_nearby(tx,
/// ty, 1)`), which is why AI cars never miss a gate and the player always
/// could. Sharing the value keeps the two from drifting apart again, and keeps
/// the anti-cheat materially tighter than simply painting every gate across the
/// full width of the road (TODO.md TASK-1101), which is the real fix but a
/// track-data change.
pub const GATE_TOLERANCE_TILES: u8 = 1;

/// Shortest lap time that may be recorded, in ticks, for a timer that was not
/// told the track's par times (half a second).
///
/// This is an absolute backstop, deliberately loose: a `LapTimer` built with
/// [`LapTimer::new`] has no idea what circuit it is timing, so the tight,
/// data-derived bound lives in [`ParTimes::min_plausible_lap_ticks`] and is
/// applied by [`LapTimer::with_par_times`].
///
/// The derivation is the fastest circuit in the game. `TRACK_01` ("Arduboy
/// Oval") is a 10x10 tile (640x640 world unit) oval whose fastest *verified*
/// lap is 323 ticks with the best swept tuning (`make playtest`); its narrowest
/// possible route is therefore far longer than 30 ticks' worth of travel at
/// `BASE_TOP_SPEED` (3.42 units/tick = 205 units/s, i.e. 205 world units in
/// 30 ticks -- a third of the width of the track). A "lap" of 0, 1 or 2 ticks
/// is the exploit this floor exists to kill: it used to score `Medal::DevPlatinum`
/// and be persisted to the memory card.
pub const MIN_PLAUSIBLE_LAP_TICKS: u32 = 30;

/// Checkpoint gate representation on the track.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct CheckpointGate {
    pub x: u8,
    pub y: u8,
    pub width: u8,
    pub height: u8,
}

impl CheckpointGate {
    /// Tests if a given tile coordinate (tx, ty) falls within this checkpoint gate.
    #[inline]
    pub fn contains_tile(&self, tx: u8, ty: u8) -> bool {
        tx >= self.x
            && tx < self.x.saturating_add(self.width)
            && ty >= self.y
            && ty < self.y.saturating_add(self.height)
    }

    /// Whether this gate has any area at all (inactive padding gates do not).
    #[inline]
    pub fn is_active(&self) -> bool {
        self.width > 0 && self.height > 0
    }

    /// Tests if a given tile coordinate (tx, ty) falls within this checkpoint gate,
    /// or within `tolerance` tiles around its boundaries.
    #[inline]
    pub fn contains_tile_or_nearby(&self, tx: u8, ty: u8, tolerance: u8) -> bool {
        if !self.is_active() {
            return false;
        }
        let min_x = self.x.saturating_sub(tolerance);
        let max_x = self.x.saturating_add(self.width).saturating_add(tolerance);
        let min_y = self.y.saturating_sub(tolerance);
        let max_y = self.y.saturating_add(self.height).saturating_add(tolerance);
        tx >= min_x && tx < max_x && ty >= min_y && ty < max_y
    }
}

/// Target par times for a track in 60Hz ticks.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct ParTimes {
    pub bronze_ticks: u32,
    pub silver_ticks: u32,
    pub gold_ticks: u32,
    pub dev_platinum_ticks: u32,
}

/// Medal earned based on best lap time.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum Medal {
    None = 0,
    Bronze = 1,
    Silver = 2,
    Gold = 3,
    DevPlatinum = 4,
}

impl ParTimes {
    /// Evaluates which medal is earned by a given lap time (in ticks).
    pub fn evaluate_medal(&self, lap_ticks: u32) -> Medal {
        if lap_ticks == 0 {
            return Medal::None;
        }
        if lap_ticks <= self.dev_platinum_ticks {
            Medal::DevPlatinum
        } else if lap_ticks <= self.gold_ticks {
            Medal::Gold
        } else if lap_ticks <= self.silver_ticks {
            Medal::Silver
        } else if lap_ticks <= self.bronze_ticks {
            Medal::Bronze
        } else {
            Medal::None
        }
    }

    /// Shortest lap time the anti-cheat accepts on this circuit, in ticks.
    ///
    /// Derived from the measured data rather than guessed: `tools/playtest`
    /// drives every circuit with a reference driver and writes
    /// `dev_platinum_ticks` from the *fastest lap it could actually complete*
    /// across the tuning sweep, so `dev_platinum_ticks` is an upper bound on
    /// what driving can achieve. Half of it is therefore already unreachable:
    /// covering the circuit in less time than the fastest verified lap at
    /// twice the pace is not a driving feat, it is a missing gate or a lap that
    /// never left the start box. Gold (the default-tune reference lap) is the
    /// next target down and would leave no room at all for a real player.
    ///
    /// Never tighter than the circuit-independent
    /// [`MIN_PLAUSIBLE_LAP_TICKS`], so a circuit with an unset (zero) par table
    /// still gets a usable floor instead of rejecting every lap.
    pub fn min_plausible_lap_ticks(&self) -> u32 {
        let from_par = self.dev_platinum_ticks / 2;
        if from_par > MIN_PLAUSIBLE_LAP_TICKS {
            from_par
        } else {
            MIN_PLAUSIBLE_LAP_TICKS
        }
    }
}

/// Ticks each countdown light stays lit, at 60 Hz.
pub const COUNTDOWN_LIGHT_TICKS: u16 = 36;
/// Number of lights before GO (three, as in an arcade start rig).
pub const COUNTDOWN_LIGHTS: u8 = 3;
/// Ticks the GO state is held after the last light, so the player sees it.
pub const COUNTDOWN_GO_TICKS: u16 = 30;

/// Which light is showing, and whether the race is live.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum StartPhase {
    /// Grid is forming; the first light has not come on.
    Grid,
    /// Lit light count, 1..=3, still counting down.
    Lit(u8),
    /// Lights out, racing has begun.
    Go,
    /// The hold after GO has elapsed; the player is fully in control.
    Racing,
}

/// Pre-race start sequence: three lights, then GO, then racing.
///
/// The lap clock used to arm the instant a race was loaded, so a standing start
/// was indistinguishable from a flying one and the reaction-time cost of a launch
/// fell on the player. This exists so the clock starts on GO.
///
/// Pure and host-testable: it holds no engine state, so the race loop drives it
/// and can gate input on [`StartSequence::accepts_input`].
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct StartSequence {
    elapsed: u16,
}

impl StartSequence {
    pub const fn new() -> Self {
        StartSequence { elapsed: 0 }
    }

    /// Advances one 60 Hz tick.
    pub fn tick(&mut self) {
        self.elapsed = self.elapsed.saturating_add(1);
    }

    /// Ticks elapsed since the grid.
    pub fn elapsed(&self) -> u16 {
        self.elapsed
    }

    /// Tick at which the lights go out and the race begins.
    ///
    /// The grid holds for one light period *before* the first light, so this is
    /// `lights + 1` periods, not `lights`. Derived once here because getting it
    /// wrong silently costs the player the last light.
    pub const fn lights_out_tick() -> u16 {
        COUNTDOWN_LIGHT_TICKS * (COUNTDOWN_LIGHTS as u16 + 1)
    }

    /// Current light state.
    pub fn phase(&self) -> StartPhase {
        let out = Self::lights_out_tick();
        if self.elapsed < COUNTDOWN_LIGHT_TICKS {
            StartPhase::Grid
        } else if self.elapsed < out {
            // One light per period after the grid hold.
            let lit = (self.elapsed / COUNTDOWN_LIGHT_TICKS) as u8;
            StartPhase::Lit(lit.min(COUNTDOWN_LIGHTS))
        } else if self.elapsed < out + COUNTDOWN_GO_TICKS {
            StartPhase::Go
        } else {
            StartPhase::Racing
        }
    }

    /// True once the player may apply throttle or steering.
    ///
    /// Locked through the grid *and* all three lights: releasing on light two
    /// would hand a launch away before the start.
    pub fn accepts_input(&self) -> bool {
        matches!(self.phase(), StartPhase::Go | StartPhase::Racing)
    }

    /// True on the tick the lights go out. Call once per frame and use it to arm
    /// the lap clock, so the clock starts on GO rather than at load.
    pub fn just_started(&self) -> bool {
        self.elapsed == Self::lights_out_tick()
    }
}

/// Real-time lap timer with ArduRacer FX style anti-cheat checkpoint validation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LapTimer<const MAX_CHECKPOINTS: usize> {
    pub current_lap: u8,
    pub current_lap_ticks: u32,
    /// Best lap recorded so far, or [`NO_BEST_LAP`] if none is plausible yet.
    ///
    /// Prefer [`LapTimer::best_lap_ticks`] (an `Option`) or
    /// [`LapTimer::has_best_lap`]: a raw `u32::MAX` reaching a formatter is how
    /// the results screen ends up rendering 71,582,788 seconds.
    pub best_lap_ticks: u32,
    /// Ticks taken by the last lap that completed every gate, implausible or not.
    ///
    /// Only meaningful on a tick where [`LapTimer::update_player_tile`] returned
    /// `true`.
    pub last_completed_lap_ticks: u32,
    /// Gates declared by the track. Clamped to the gate array and to
    /// [`MAX_TRACKED_CHECKPOINTS`]; use [`LapTimer::active_checkpoint_count`]
    /// rather than trusting this field blindly.
    pub total_checkpoints: u8,
    pub checkpoints: [CheckpointGate; MAX_CHECKPOINTS],
    /// Start / finish gate. Leaving it with every checkpoint cleared scores a lap.
    pub start_gate: CheckpointGate,
    /// Bit `i` set once checkpoint `i` has been touched during the current lap.
    ///
    /// Cleared by *every* start/finish crossing, so the ring is progress within
    /// one attempt and can never be banked across attempts.
    pub checkpoint_mask: u64,
    /// True while the car is inside the start/finish block.
    pub on_start_gate: bool,
    /// Lap timing arms once the car first leaves the grid.
    pub is_running: bool,
    pub is_finished: bool,
    /// Shortest completed lap accepted as a lap time (see
    /// [`LapTimer::with_par_times`]). Private: it is an anti-cheat bound, not
    /// game state for a caller to tune.
    min_lap_ticks: u32,
}

/// Sentinel stored in [`LapTimer::best_lap_ticks`] while no lap time has been
/// recorded. Read it through [`LapTimer::best_lap_ticks`] instead.
pub const NO_BEST_LAP: u32 = u32::MAX;

/// Longest lap the plausibility floor is allowed to be pushed to, in ticks
/// (60 s). The longest shipped circuit's reference lap is 38 s, so this only
/// ever clamps a nonsense bound handed in by a caller -- without it, passing
/// `u32::MAX` would silently switch the anti-cheat off.
const MAX_PLAUSIBLE_LAP_TICKS: u32 = 3_600;

impl<const MAX_CHECKPOINTS: usize> LapTimer<MAX_CHECKPOINTS> {
    /// Builds a timer from a track's start gate and checkpoint list.
    ///
    /// Without par times only the circuit-independent
    /// [`MIN_PLAUSIBLE_LAP_TICKS`] backstop applies; pass the track's par times
    /// to [`LapTimer::with_par_times`] for the tight, per-circuit bound.
    pub fn new(checkpoints: &[CheckpointGate], start_gate: CheckpointGate) -> Self {
        Self::with_min_lap_ticks(checkpoints, start_gate, MIN_PLAUSIBLE_LAP_TICKS)
    }

    /// Builds a timer that rejects laps faster than half of the circuit's Dev
    /// Platinum target -- the fastest lap `tools/playtest` can actually drive.
    pub fn with_par_times(
        checkpoints: &[CheckpointGate],
        start_gate: CheckpointGate,
        par_times: ParTimes,
    ) -> Self {
        Self::with_min_lap_ticks(checkpoints, start_gate, par_times.min_plausible_lap_ticks())
    }

    /// Builds a timer with an explicit plausibility floor, never tighter than
    /// [`MIN_PLAUSIBLE_LAP_TICKS`] and never looser than 60 seconds.
    pub fn with_min_lap_ticks(
        checkpoints: &[CheckpointGate],
        start_gate: CheckpointGate,
        min_lap_ticks: u32,
    ) -> Self {
        let count = checkpoints
            .len()
            .min(MAX_CHECKPOINTS)
            .min(MAX_TRACKED_CHECKPOINTS);
        let mut gates = [CheckpointGate::default(); MAX_CHECKPOINTS];
        gates[..count].copy_from_slice(&checkpoints[..count]);

        LapTimer {
            current_lap: 1,
            current_lap_ticks: 0,
            best_lap_ticks: NO_BEST_LAP,
            last_completed_lap_ticks: 0,
            total_checkpoints: count as u8,
            checkpoints: gates,
            start_gate,
            checkpoint_mask: 0,
            // Unverified until a tile feed says so; `start` keeps that state.
            on_start_gate: false,
            is_running: false,
            is_finished: false,
            min_lap_ticks: min_lap_ticks.clamp(MIN_PLAUSIBLE_LAP_TICKS, MAX_PLAUSIBLE_LAP_TICKS),
        }
    }

    /// Bit for gate `i`. `i` must be below [`MAX_TRACKED_CHECKPOINTS`]; every
    /// caller goes through [`LapTimer::active_checkpoint_count`], which clamps.
    #[inline]
    fn gate_bit(i: usize) -> u64 {
        1u64 << i
    }

    /// Gates this timer will actually track: the declared count, clipped to the
    /// gate array and to the width of [`LapTimer::checkpoint_mask`].
    ///
    /// `total_checkpoints` is a public field, so a caller can set it to anything;
    /// clamping here is what keeps the gate loop in bounds and stops gate 16
    /// aliasing gate 0 in a mask too narrow for it.
    #[inline]
    pub fn active_checkpoint_count(&self) -> usize {
        (self.total_checkpoints as usize)
            .min(MAX_CHECKPOINTS)
            .min(MAX_TRACKED_CHECKPOINTS)
    }

    /// Rewrites an out-of-range `total_checkpoints` in place, so the HUD and any
    /// other reader see the count that is really being tracked.
    fn clamp_checkpoint_count(&mut self) {
        let active = self.active_checkpoint_count() as u8;
        if self.total_checkpoints != active {
            self.total_checkpoints = active;
        }
    }

    /// Number of checkpoints already cleared this lap.
    #[inline]
    pub fn checkpoints_cleared(&self) -> u32 {
        self.checkpoint_mask.count_ones()
    }

    /// Whether every checkpoint has been touched during the current lap.
    #[inline]
    pub fn all_checkpoints_cleared(&self) -> bool {
        let total = self.active_checkpoint_count();
        total > 0 && self.checkpoints_cleared() >= total as u32
    }

    /// Index of the next gate the car has to reach, in driving order.
    ///
    /// This is the length of the *contiguous cleared prefix* of the gate array,
    /// not the raw number of gates touched: gates may be taken in any order, so a
    /// count says nothing about where the car is. A driver who has banked gates 3
    /// and 7 is still physically on the way to gate 0. Equals
    /// `active_checkpoint_count()` once the whole ring is cleared, which is the
    /// index of the start/finish node in `TrackDef::route_node` order.
    pub fn next_checkpoint_index(&self) -> u32 {
        let total = self.active_checkpoint_count();
        let mut i = 0;
        while i < total && self.checkpoint_mask & Self::gate_bit(i) != 0 {
            i += 1;
        }
        i as u32
    }

    /// Shortest completed lap accepted as a lap time, in ticks.
    #[inline]
    pub fn min_plausible_lap_ticks(&self) -> u32 {
        self.min_lap_ticks
    }

    /// Best recorded lap time, or `None` while the sentinel is in place.
    ///
    /// The safe accessor: consumers cannot accidentally format the sentinel as a
    /// 32-year lap time.
    #[inline]
    pub fn best_lap_ticks(&self) -> Option<u32> {
        if self.best_lap_ticks == NO_BEST_LAP {
            None
        } else {
            Some(self.best_lap_ticks)
        }
    }

    /// Whether a plausible lap has been recorded, i.e. whether a best-lap
    /// readout and a delta split should be shown at all.
    #[inline]
    pub fn has_best_lap(&self) -> bool {
        self.best_lap_ticks != NO_BEST_LAP
    }

    /// Advances the timer by one 60Hz tick.
    pub fn tick(&mut self) {
        if self.is_running && !self.is_finished {
            self.current_lap_ticks = self.current_lap_ticks.saturating_add(1);
        }
    }

    /// Starts timing (called when the countdown light turns GREEN).
    pub fn start(&mut self) {
        self.is_running = true;
        self.is_finished = false;
        self.current_lap = 1;
        self.current_lap_ticks = 0;
        self.checkpoint_mask = 0;
        // The grid sits inside the start/finish block, but the timer is not told
        // that: it starts *outside* it and only records the car as being on the
        // line once a tile feed says so. That makes the roll off the grid a
        // non-event rather than a lap crossing, so every crossing this state
        // machine ever reports really is a return to the line -- and no gate
        // progress can survive one.
        self.on_start_gate = false;
    }

    /// Feeds the player's tile position into the gate state machine.
    /// Returns true when a lap was just scored.
    pub fn update_player_tile(&mut self, tx: u8, ty: u8) -> bool {
        if self.is_finished {
            return false;
        }
        // `total_checkpoints` is public: re-clamp before trusting it.
        self.clamp_checkpoint_count();

        let inside_start = self.start_gate.is_active() && self.start_gate.contains_tile(tx, ty);
        let left_start = self.on_start_gate && !inside_start;

        // A start/finish crossing ends the attempt. Under coverage-only gates the
        // mask *is* the lap's progress, so carrying it across a crossing is what
        // let a car touch two gates, cut over the line, touch the other two and
        // score a lap with no circuit driven. It is dropped whether or not the
        // lap is scored -- an aborted attempt banks nothing.
        //
        // The drop happens before this tile's gates are registered, not after: a
        // gate at the tile the car is *leaving* the line for has, by definition,
        // been touched after it left the line, so it belongs to the new lap.
        let completed = left_start && self.all_checkpoints_cleared();
        if left_start {
            self.checkpoint_mask = 0;
        }

        // Register checkpoint touches (order independent, anti-cheat by coverage).
        for i in 0..self.active_checkpoint_count() {
            let bit = Self::gate_bit(i);
            if self.checkpoint_mask & bit != 0 {
                continue;
            }
            if self.checkpoints[i].contains_tile(tx, ty) {
                self.checkpoint_mask |= bit;
            }
        }

        self.on_start_gate = inside_start;

        if !(left_start && self.is_running && completed) {
            return false;
        }

        let lap_ticks = self.current_lap_ticks;
        self.last_completed_lap_ticks = lap_ticks;
        // A lap that could not physically be driven is dropped here and never
        // reaches `best_lap_ticks`, the medal table or the memory card. The lap
        // still counts below: the driver did complete the circuit, and silently
        // withholding a lap would corrupt the lap counter instead.
        if lap_ticks >= self.min_lap_ticks && lap_ticks < self.best_lap_ticks {
            self.best_lap_ticks = lap_ticks;
        }

        if self.current_lap >= TOTAL_LAPS {
            self.is_finished = true;
            self.is_running = false;
        } else {
            self.current_lap += 1;
            self.current_lap_ticks = 0;
        }
        true
    }

    /// Feeds the player's physical state and track information into the gate state machine.
    /// Returns true when a lap was just scored.
    ///
    /// This resolves the single-tile sampling limitations by:
    /// - Testing multiple sample points on the car body (center, nose, rear, and flanks)
    ///   so touching or clipping a line/checkpoint with any part of the vehicle registers immediately.
    /// - Recognizing `TrackTile::StartFinish` and `TrackTile::Checkpoint` across the entire
    ///   authored road width (which is typically 6-12 tiles wide).
    /// - Proximity checking in world units to gate centres (< 48 units), matching AI semantics.
    pub fn update_player_on_track(&mut self, player: &VehicleState, track: &TrackDef) -> bool {
        if self.is_finished {
            return false;
        }
        self.clamp_checkpoint_count();

        let pos = player.position;
        let fwd = player.forward_dir();
        // Lateral perpendicular to forward: (-fwd.y, fwd.x)
        let lat = Vec2::new(-fwd.y, fwd.x);

        // Key sample points across the vehicle body:
        // Car is ~24 units long (12 front, 12 rear) and ~14 units wide (7 left, 7 right).
        let front = pos + fwd.scale(Fixed::from_int(12));
        let rear = pos - fwd.scale(Fixed::from_int(12));
        let left = pos + lat.scale(Fixed::from_int(7));
        let right = pos - lat.scale(Fixed::from_int(7));

        let check_point_inside_start = |pt: Vec2| -> bool {
            let tx = TrackDef::tile_x_of(pt.x);
            let ty = TrackDef::tile_y_of(pt.y);

            // 1. Explicit TrackTile::StartFinish on the racing surface
            if track.tile_at(tx, ty) == TrackTile::StartFinish {
                return true;
            }

            // 2. Authored start_gate rectangle + tolerance
            if self.start_gate.is_active()
                && self
                    .start_gate
                    .contains_tile_or_nearby(tx, ty, GATE_TOLERANCE_TILES)
            {
                return true;
            }

            // 3. Proximity to start_gate centre in world space (< 48 units)
            if self.start_gate.is_active() {
                let sc = TrackDef::gate_centre(&self.start_gate);
                if (sc - pt).length() < Fixed::from_int(48) {
                    return true;
                }
            }

            false
        };

        let inside_start = check_point_inside_start(pos)
            || check_point_inside_start(front)
            || check_point_inside_start(rear)
            || check_point_inside_start(left)
            || check_point_inside_start(right);

        let left_start = self.on_start_gate && !inside_start;
        let completed = left_start && self.all_checkpoints_cleared();
        if left_start {
            self.checkpoint_mask = 0;
        }

        // Register checkpoint touches
        let sample_pts = [pos, front, rear, left, right];
        let total_gates = self.active_checkpoint_count();
        for i in 0..total_gates {
            let bit = Self::gate_bit(i);
            if self.checkpoint_mask & bit != 0 {
                continue;
            }
            let gate = self.checkpoints[i];
            if !gate.is_active() {
                continue;
            }
            let gate_centre = TrackDef::gate_centre(&gate);

            let mut touched = false;
            for &pt in &sample_pts {
                let tx = TrackDef::tile_x_of(pt.x);
                let ty = TrackDef::tile_y_of(pt.y);

                // a. CheckpointGate tile bounds + tolerance
                if gate.contains_tile_or_nearby(tx, ty, GATE_TOLERANCE_TILES) {
                    touched = true;
                    break;
                }

                // b. Proximity in world units (< 48 units = 1.5 tiles)
                if (gate_centre - pt).length() < Fixed::from_int(48) {
                    touched = true;
                    break;
                }

                // c. Track surface is TrackTile::Checkpoint within gate vicinity (8 tiles)
                if track.tile_at(tx, ty) == TrackTile::Checkpoint {
                    let dx = (gate.x as i32 - tx as i32).abs();
                    let dy = (gate.y as i32 - ty as i32).abs();
                    if dx + dy <= 8 {
                        touched = true;
                        break;
                    }
                }
            }

            if touched {
                self.checkpoint_mask |= bit;
            }
        }

        self.on_start_gate = inside_start;

        if !(left_start && self.is_running && completed) {
            return false;
        }

        let lap_ticks = self.current_lap_ticks;
        self.last_completed_lap_ticks = lap_ticks;
        if lap_ticks >= self.min_lap_ticks && lap_ticks < self.best_lap_ticks {
            self.best_lap_ticks = lap_ticks;
        }

        if self.current_lap >= TOTAL_LAPS {
            self.is_finished = true;
            self.is_running = false;
        } else {
            self.current_lap += 1;
            self.current_lap_ticks = 0;
        }
        true
    }

    /// Live delta against the player's best lap, in ticks.
    ///
    /// Positive means the current lap is slower than the reference pace (red),
    /// negative means it is ahead (green). The reference pace is prorated by how
    /// far along the *driving order* the car is
    /// ([`LapTimer::next_checkpoint_index`] / gate count), which is what an
    /// arcade split indicator shows and, unlike a raw gate count, only ever
    /// moves forward as the car advances around the circuit. Returns 0 when no
    /// reference lap exists yet.
    pub fn delta_ticks(&self) -> i32 {
        let best = match self.best_lap_ticks() {
            Some(t) => t,
            None => return 0,
        };
        let total = self.active_checkpoint_count() as u64;
        if total == 0 {
            return 0;
        }
        // Ticks at which the reference run is expected to have reached the
        // gate the car is currently heading for.
        let progress = self.next_checkpoint_index() as u64;
        let expected = best as u64 * progress / total;
        // i64 throughout: `current_lap_ticks` is a saturating u32 counter, so it
        // can legitimately exceed `i32::MAX` on a race left running.
        let delta = self.current_lap_ticks as i64 - expected as i64;
        delta.clamp(i32::MIN as i64, i32::MAX as i64) as i32
    }

    /// Route index of the gate the car must reach next: the checkpoints in
    /// driving order, then the start/finish gate at
    /// `TrackDef::route_len() - 1`.
    ///
    /// Returns the *next* gate, not how many gates have been touched, so it
    /// agrees with the `target_gate_idx` rivals report to `compute_standings`
    /// even when gates were taken out of order. Clamped into
    /// `0 ..= track_route_len - 1` so a caller indexing a route array cannot go
    /// out of bounds on a mismatched length.
    pub fn route_node_index(&self, track_route_len: usize) -> usize {
        if track_route_len == 0 {
            return 0;
        }
        let next = self.next_checkpoint_index() as usize;
        if next >= track_route_len {
            track_route_len - 1
        } else {
            next
        }
    }

    /// Formats a tick count into MM:SS.ccc string parts: (minutes, seconds, hundredths).
    pub fn format_ticks(ticks: u32) -> (u32, u32, u32) {
        let total_seconds = ticks / TICKS_PER_SECOND;
        let hundredths = ((ticks % TICKS_PER_SECOND) * 100) / TICKS_PER_SECOND;
        let minutes = total_seconds / 60;
        let seconds = total_seconds % 60;
        (minutes, seconds, hundredths)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::levels::ALL_TRACKS;
    use crate::track::TrackDef;

    /// Single-tile gate, the shape every shipped circuit uses.
    const fn gate(x: u8, y: u8) -> CheckpointGate {
        CheckpointGate {
            x,
            y,
            width: 1,
            height: 1,
        }
    }

    const START: CheckpointGate = CheckpointGate {
        x: 0,
        y: 0,
        width: 1,
        height: 1,
    };

    /// The four gates of the test circuit, four tiles apart.
    ///
    /// The spacing is load-bearing. Gates are matched with a one-tile tolerance
    /// ([`GATE_TOLERANCE_TILES`]), so a fixture that packed them a single tile
    /// apart would have a car standing on one gate register its neighbours, and
    /// the coverage tests would be measuring the tolerance rather than coverage.
    /// Real circuits do not come close: on every shipped track the closest two
    /// gates are tens of tiles apart, and the first gate sits well clear of the
    /// start/finish.
    const RING: [CheckpointGate; 4] = [gate(4, 4), gate(8, 4), gate(12, 4), gate(16, 4)];

    /// A square 4-gate circuit: start (0,0), gates [`RING`].
    fn four_gate_timer<const N: usize>() -> LapTimer<N> {
        let mut t = LapTimer::<N>::new(&RING, START);
        t.start();
        t
    }

    /// Puts the car on `gate` after `ticks` of driving.
    fn drive_to<const N: usize>(t: &mut LapTimer<N>, g: CheckpointGate, ticks: u32) {
        for _ in 0..ticks {
            t.tick();
        }
        t.update_player_tile(g.x, g.y);
    }

    /// Runs one full in-order lap of `t` at `ticks_per_gate` ticks per gate.
    fn full_lap<const N: usize>(t: &mut LapTimer<N>, ticks_per_gate: u32) {
        for g in RING {
            drive_to(t, g, ticks_per_gate);
        }
        // Enter the line, then leave it: leaving is the crossing.
        t.update_player_tile(START.x, START.y);
        assert!(t.update_player_tile(9, 9), "a full lap must score");
    }

    /// Touches every gate and crosses the line one tick later: a "lap" that took
    /// a single tick, which is exactly what used to be recorded as a best lap and
    /// medal-evaluated as Dev Platinum.
    fn one_tick_lap<const N: usize>(t: &mut LapTimer<N>) {
        for g in RING {
            drive_to(t, g, 0);
        }
        t.update_player_tile(START.x, START.y);
        t.tick();
        assert!(t.update_player_tile(9, 9), "the lap is scored");
        assert_eq!(t.last_completed_lap_ticks, 1, "one tick of a lap");
    }

    // ---- Bug 1: a 1-tick lap scored Dev Platinum and reached the card ----

    #[test]
    fn one_tick_lap_is_not_recorded_as_a_best_lap() {
        // The end-to-end exploit: a real lap, then a "lap" completed one tick
        // later. It used to set best_lap_ticks = 1, evaluate_medal(1) answered
        // Dev Platinum, and SaveData::update_best_lap persisted it.
        assert_eq!(
            ALL_TRACKS[0].par_times.evaluate_medal(1),
            Medal::DevPlatinum,
            "one tick is the fastest possible 'lap' under the medal table"
        );

        let mut t = four_gate_timer::<4>();
        full_lap(&mut t, 150);
        assert_eq!(t.best_lap_ticks(), Some(600));
        assert_eq!(t.current_lap, 2);

        one_tick_lap(&mut t);
        assert_eq!(
            t.best_lap_ticks(),
            Some(600),
            "an undriveable lap must not replace a real one"
        );
        assert_eq!(t.current_lap, 3, "the lap itself still counts");
        assert_ne!(
            t.best_lap_ticks, NO_BEST_LAP,
            "the raw field holds the real time, never the sentinel"
        );
        assert_eq!(
            t.last_completed_lap_ticks, 1,
            "the lap time itself is still reported"
        );
    }

    #[test]
    fn degenerate_laps_score_no_medal_on_any_timer() {
        // With no reference lap at all, a 1-tick lap must still leave the safe
        // accessor empty, so no consumer can turn the sentinel into a record.
        let mut t = four_gate_timer::<4>();
        for _ in 0..3 {
            one_tick_lap(&mut t);
            assert_eq!(t.best_lap_ticks(), None);
            assert!(!t.has_best_lap());
        }
        // ... and a lap just over the floor is still accepted, or the anti-cheat
        // would throw away every lap on a slow circuit.
        let mut t = four_gate_timer::<4>();
        full_lap(&mut t, MIN_PLAUSIBLE_LAP_TICKS / 4 + 1);
        let recorded = match t.best_lap_ticks() {
            Some(ticks) => ticks,
            None => panic!("a lap at the floor was rejected"),
        };
        assert!(recorded >= MIN_PLAUSIBLE_LAP_TICKS);
    }

    #[test]
    fn par_derived_floor_rejects_a_lap_no_car_could_drive() {
        let track = &ALL_TRACKS[0];
        let mut t = LapTimer::<16>::with_par_times(
            track.checkpoint_slice(),
            track.start_gate,
            track.par_times,
        );
        t.start();

        let bound = track.par_times.min_plausible_lap_ticks();
        assert_eq!(bound, track.par_times.dev_platinum_ticks / 2);
        assert!(bound > MIN_PLAUSIBLE_LAP_TICKS);

        // A lap at half the fastest verified reference lap: 3.4 units/tick of
        // top speed cannot cover the circuit in this time.
        let per_gate = bound / 8;
        for g in track.checkpoint_slice() {
            drive_to(&mut t, *g, per_gate);
        }
        t.update_player_tile(track.start_gate.x, track.start_gate.y);
        assert!(t.update_player_tile(track.start_gate.x.wrapping_add(4), track.start_gate.y));
        assert_eq!(
            t.best_lap_ticks(),
            None,
            "a lap faster than half of Dev Platinum is a cheat"
        );
        assert_eq!(t.current_lap, 2, "the lap still counts");

        // The measured reference lap clears the bound with 2x headroom.
        full_reference_lap(&mut t, track);
        let recorded = match t.best_lap_ticks() {
            Some(ticks) => ticks,
            None => panic!("the measured reference lap was rejected as a cheat"),
        };
        assert!(recorded >= bound && recorded <= track.par_times.dev_platinum_ticks);
        assert_eq!(t.current_lap, 3);
    }

    /// Drives `timer` around `track` in gate order, spending
    /// `dev_platinum_ticks / gate_count` ticks per gate.
    fn full_reference_lap<const N: usize>(timer: &mut LapTimer<N>, track: &crate::track::TrackDef) {
        let per_gate = track.par_times.dev_platinum_ticks / track.checkpoint_count as u32;
        for g in track.checkpoint_slice() {
            drive_to(timer, *g, per_gate);
        }
        timer.update_player_tile(track.start_gate.x, track.start_gate.y);
        assert!(
            timer.update_player_tile(track.start_gate.x.wrapping_add(4), track.start_gate.y),
            "the reference lap must score"
        );
    }

    #[test]
    fn every_shipped_track_admits_its_measured_reference_lap() {
        // `dev_platinum_ticks` is written by tools/playtest from the fastest lap
        // its reference driver could actually complete (see
        // tools/track_cook/par_calibration.json), so a bound at or under half of
        // it cannot reject a real player on any shipped circuit.
        for track in ALL_TRACKS.iter() {
            let min = track.par_times.min_plausible_lap_ticks();
            assert!(
                min <= track.par_times.dev_platinum_ticks,
                "{}: floor {} would reject its own reference lap {}",
                track.name,
                min,
                track.par_times.dev_platinum_ticks
            );
            assert!(
                min >= MIN_PLAUSIBLE_LAP_TICKS,
                "{}: floor must never drop below the absolute floor",
                track.name
            );
            // Gold, the default-tune reference pace, must stay comfortably
            // earnable above the floor.
            assert!(min * 2 <= track.par_times.gold_ticks, "{}", track.name);

            // And the timer built from the par table agrees.
            let t = LapTimer::<16>::with_par_times(
                track.checkpoint_slice(),
                track.start_gate,
                track.par_times,
            );
            assert_eq!(t.min_plausible_lap_ticks(), min, "{}", track.name);
        }
    }

    // ---- Bug 2: gate progress survived a failed attempt ----

    #[test]
    fn gate_progress_does_not_survive_a_failed_attempt() {
        // Exploit: bank two gates, cut over the line (no lap), bank the other
        // two, cross again => a lap with no circuit driven.
        let mut t = four_gate_timer::<4>();
        drive_to(&mut t, RING[0], 20);
        drive_to(&mut t, RING[1], 20);
        t.update_player_tile(START.x, START.y);
        assert!(!t.update_player_tile(9, 9), "a partial ring must not score");
        assert_eq!(
            t.checkpoints_cleared(),
            0,
            "the abandoned attempt must bank nothing"
        );

        drive_to(&mut t, RING[2], 20);
        drive_to(&mut t, RING[3], 20);
        assert_eq!(t.checkpoints_cleared(), 2);
        t.update_player_tile(START.x, START.y);
        assert!(
            !t.update_player_tile(9, 9),
            "the second half of a lap may not be stitched onto the first"
        );
        assert_eq!(t.current_lap, 1, "no lap was completed");
        assert_eq!(t.best_lap_ticks(), None);
    }

    #[test]
    fn a_gate_on_the_leaving_tile_belongs_to_the_new_lap() {
        // The car crosses the line straight onto a gate: the gate is touched
        // *after* the line, so it must count towards the lap that follows.
        // (tools/test_game_logic #27 walks a route exactly like this.)
        let mut t = four_gate_timer::<4>();
        t.update_player_tile(START.x, START.y);
        for g in RING {
            drive_to(&mut t, g, 10);
        }
        // Enter the line, then leave it straight onto the first gate.
        t.update_player_tile(START.x, START.y);
        t.update_player_tile(RING[0].x, RING[0].y);
        assert!(!t.update_player_tile(START.x, START.y), "no lap yet");
        assert_eq!(t.checkpoints_cleared(), 1, "gate 1 counts for the new lap");
    }

    #[test]
    fn a_lap_needs_every_gate_but_may_take_them_in_any_order() {
        // Coverage, not sequence: TODO.md FIX-01 replaced raster-scan ordering
        // because it made 23 of 24 circuits uncompletable.
        let mut t = four_gate_timer::<4>();
        for g in [RING[3], RING[1], RING[0], RING[2]] {
            drive_to(&mut t, g, 60);
        }
        assert!(t.all_checkpoints_cleared());
        t.update_player_tile(START.x, START.y);
        assert!(t.update_player_tile(9, 9), "out-of-order lap scores");
        assert_eq!(t.best_lap_ticks(), Some(240));
        assert_eq!(t.checkpoints_cleared(), 0, "the next lap starts empty");
    }

    // ---- Bug 3: 1 << i on a u16 mask ----

    #[test]
    fn masks_never_alias_beyond_sixteen_gates() {
        // 20 gates: bit 16..19 have no representation in a u16 mask, where
        // `1 << i` panics in debug and aliases bit 0 in release.
        let mut gates = [CheckpointGate::default(); 20];
        for (i, g) in gates.iter_mut().enumerate() {
            // Four tiles apart: closer than the one-tile tolerance and the gates
            // would register each other (see `RING`).
            *g = gate((i as u8) * 4 + 4, 4);
        }

        let mut t = LapTimer::<32>::new(&gates, START);
        t.start();
        assert_eq!(t.total_checkpoints, 20);

        for g in gates.iter() {
            t.update_player_tile(g.x, g.y);
        }
        assert_eq!(t.checkpoints_cleared(), 20, "gate 16 must not alias gate 0");
        assert!(t.all_checkpoints_cleared());
        // Each gate is distinct: bank only the high half and nothing counts.
        let mut half = LapTimer::<32>::new(&gates, START);
        half.start();
        for g in gates.iter().skip(16) {
            half.update_player_tile(g.x, g.y);
        }
        assert_eq!(half.checkpoints_cleared(), 4);
        assert!(!half.all_checkpoints_cleared());
    }

    #[test]
    fn an_out_of_range_gate_count_is_clamped_not_trusted() {
        // `total_checkpoints` is public. A caller (or corrupt data) setting it
        // past the gate array must not index out of bounds or panic.
        let mut t = four_gate_timer::<4>();
        t.total_checkpoints = 200;
        // No gate of a 4-gate circuit is on these tiles, so nothing can score.
        for x in 0..255u8 {
            t.update_player_tile(x, 200);
        }
        assert_eq!(
            t.total_checkpoints, 4,
            "the count is rewritten to what is really tracked"
        );
        assert_eq!(t.active_checkpoint_count(), 4);
        assert_eq!(t.checkpoints_cleared(), 0);

        // A timer whose array is narrower than its declared count clamps to the
        // array, and the inactive padding gates can never be touched.
        let mut wide = LapTimer::<2>::new(&[RING[0], RING[1], RING[2]], START);
        assert_eq!(wide.total_checkpoints, 2);
        wide.total_checkpoints = 255;
        wide.start();
        wide.update_player_tile(3, 1);
        assert_eq!(wide.total_checkpoints, 2);
        assert_eq!(wide.checkpoints_cleared(), 0);
    }

    // ---- Bug 4: route_node_index returned the gate count ----

    #[test]
    fn route_node_index_is_the_next_gate_not_the_gate_count() {
        let mut t = four_gate_timer::<4>();
        let route_len = 5;
        assert_eq!(t.route_node_index(route_len), 0, "gate 0 first");

        // Gate 3 alone: the car is still heading for gate 0 even though one
        // gate is banked. The old code answered 1 here.
        drive_to(&mut t, RING[3], 10);
        assert_eq!(t.checkpoints_cleared(), 1);
        assert_eq!(t.next_checkpoint_index(), 0);
        assert_eq!(t.route_node_index(route_len), 0);

        // A contiguous prefix advances the index with the car.
        drive_to(&mut t, RING[0], 10);
        assert_eq!(t.route_node_index(route_len), 1);
        drive_to(&mut t, RING[1], 10);
        assert_eq!(t.route_node_index(route_len), 2);

        // The whole ring banked: the next node is the start/finish, the last
        // index of the route, not 4 % 5 of a count.
        drive_to(&mut t, RING[2], 10);
        assert_eq!(t.route_node_index(route_len), 4);
        assert_eq!(t.next_checkpoint_index(), 4);

        // Defensive: the result always indexes a route array of that length.
        assert_eq!(t.route_node_index(0), 0);
        assert_eq!(t.route_node_index(1), 0);
        assert!(t.route_node_index(3) < 3);
    }

    // ---- Bug 5: the u32::MAX sentinel leaked to the UI ----

    #[test]
    fn best_lap_accessors_hide_the_sentinel() {
        let mut t = four_gate_timer::<4>();
        assert_eq!(t.best_lap_ticks(), None);
        assert!(!t.has_best_lap());
        assert_eq!(t.delta_ticks(), 0, "no reference pace without a lap");

        full_lap(&mut t, 150);
        assert_eq!(t.best_lap_ticks(), Some(600));
        assert!(t.has_best_lap());

        // The safe path can never hand a formatter the sentinel: that is how the
        // results screen rendered 71,582,788 seconds.
        if let Some(ticks) = t.best_lap_ticks() {
            let (m, _, _) = LapTimer::<4>::format_ticks(ticks);
            assert!(m < 60, "a lap time must not be a 32-year clock");
        } else {
            panic!("best lap accessor lost a recorded lap");
        }
    }

    // ---- Bug 6: the delta prorated by gate count ----

    #[test]
    fn delta_follows_route_position_not_raw_gate_count() {
        let mut t = four_gate_timer::<4>();
        full_lap(&mut t, 150);
        assert_eq!(t.best_lap_ticks(), Some(600));

        // One gate banked, but it is the *last* gate: the car has made no route
        // progress at all, so the whole lap is still ahead of it and the split
        // must read as the elapsed time, not as half a lap's worth of credit.
        for _ in 0..100 {
            t.tick();
        }
        t.update_player_tile(RING[3].x, RING[3].y);
        assert_eq!(t.next_checkpoint_index(), 0);
        assert_eq!(
            t.delta_ticks(),
            100,
            "an out-of-order gate must not pre-credit route progress"
        );

        // In order, the split behaves like a real sector split.
        for _ in 0..50 {
            t.tick();
        }
        t.update_player_tile(RING[0].x, RING[0].y);
        assert_eq!(t.next_checkpoint_index(), 1);
        assert_eq!(t.delta_ticks(), 150 - 150);
    }

    #[test]
    fn delta_survives_a_saturated_lap_counter() {
        let mut t = four_gate_timer::<4>();
        full_lap(&mut t, 150);
        // A race left running for years: the counter saturates instead of
        // wrapping into a negative (or panicking) delta.
        t.current_lap_ticks = u32::MAX;
        let d = t.delta_ticks();
        assert_eq!(d, i32::MAX, "clamped, not wrapped");
    }

    #[test]
    fn test_checkpoint_gate_nearby() {
        let gate = CheckpointGate {
            x: 10,
            y: 10,
            width: 1,
            height: 1,
        };
        assert!(gate.contains_tile(10, 10));
        assert!(!gate.contains_tile(9, 10));
        assert!(gate.contains_tile_or_nearby(10, 10, 1));
        assert!(gate.contains_tile_or_nearby(9, 10, 1));
        assert!(gate.contains_tile_or_nearby(11, 10, 1));
        assert!(gate.contains_tile_or_nearby(10, 9, 1));
        assert!(gate.contains_tile_or_nearby(10, 11, 1));
        assert!(!gate.contains_tile_or_nearby(8, 10, 1));
        assert!(!gate.contains_tile_or_nearby(10, 12, 1));
    }

    // ---- Bug 7: a one-tile gate cannot span a twelve-tile road ----

    /// Tiles of drivable road contiguous with `gate` along the wider of the two
    /// axes, i.e. how wide the road the gate sits on actually is.
    fn road_width_at(track: &TrackDef, gate: CheckpointGate) -> usize {
        let mut row = 0usize;
        for dx in 0..track.width as i32 {
            let tx = gate.x as i32 + dx;
            if (0..=255).contains(&tx) && track.is_road_at(tx as u8, gate.y) {
                row += 1;
            }
        }
        let mut col = 0usize;
        for dy in 0..track.height as i32 {
            let ty = gate.y as i32 + dy;
            if (0..=255).contains(&ty) && track.is_road_at(gate.x, ty as u8) {
                col += 1;
            }
        }
        row.max(col)
    }

    #[test]
    fn every_gate_is_far_narrower_than_the_road_it_sits_on() {
        // The premise of the three tests below, asserted so the reason they
        // exist cannot rot: each gate is a single 1x1 tile, so it only covers
        // `1 / road_width` of the line the player is allowed to drive across.
        let mut narrowest = usize::MAX;
        for track in ALL_TRACKS.iter() {
            for gate in track
                .checkpoint_slice()
                .iter()
                .copied()
                .chain(std::iter::once(track.start_gate))
            {
                let width = road_width_at(track, gate);
                narrowest = narrowest.min(width);
                assert!(
                    width > 1,
                    "{}: gate at ({},{}) spans a {}-tile road, so this test \
                     cannot demonstrate the defect",
                    track.name,
                    gate.x,
                    gate.y,
                    width
                );
            }
        }
        assert!(
            narrowest >= 2,
            "at least one gate must sit on a road wider than itself"
        );
    }

    /// A line tile `dx`/`dy` away from a gate tile, saturating at the grid edge
    /// so an offset near a border stays a legal `u8`.
    fn offset_tile(g: CheckpointGate, dx: i16, dy: i16) -> (u8, u8) {
        let tx = (g.x as i16 + dx).clamp(0, 255) as u8;
        let ty = (g.y as i16 + dy).clamp(0, 255) as u8;
        (tx, ty)
    }

    /// Every lateral offset a racing line can plausibly be off a one-tile gate,
    /// including dead centre.
    const LINE_OFFSETS: [(i16, i16); 9] = [
        (-1, -1),
        (-1, 0),
        (-1, 1),
        (0, -1),
        (0, 0),
        (0, 1),
        (1, -1),
        (1, 0),
        (1, 1),
    ];

    #[test]
    fn a_racing_line_off_the_authored_gates_still_completes_a_lap() {
        // The reported defect, end to end on every shipped circuit: drive the
        // whole lap one tile off the authored gate tiles -- an ordinary racing
        // line, and on a 12-tile road a perfectly ordinary one -- and the lap
        // must still count.
        for track in ALL_TRACKS.iter() {
            for (dx, dy) in LINE_OFFSETS {
                let mut t = LapTimer::<16>::new(track.checkpoint_slice(), track.start_gate);
                t.start();

                for g in track.checkpoint_slice() {
                    let (tx, ty) = offset_tile(*g, dx, dy);
                    assert!(
                        track.is_road_at(tx, ty),
                        "{}: the offset line ({tx},{ty}) is off the road, so \
                         driving it is not a legal line",
                        track.name
                    );
                    for _ in 0..40 {
                        t.tick();
                    }
                    let car = VehicleState::new(
                        Vec2::new(TrackDef::tile_centre(tx), TrackDef::tile_centre(ty)),
                        0,
                        crate::tuning::CarTuning::default(),
                    );
                    t.update_player_on_track(&car, track);
                }
                assert!(
                    t.all_checkpoints_cleared(),
                    "{}: line offset ({dx},{dy}) missed a checkpoint",
                    track.name
                );

                // Enter the line, then drive clear of it. Clearing the line by
                // a whole tile is not "not crossing it".
                let (sx, sy) = offset_tile(track.start_gate, dx, dy);
                let car_on_line = VehicleState::new(
                    Vec2::new(TrackDef::tile_centre(sx), TrackDef::tile_centre(sy)),
                    0,
                    crate::tuning::CarTuning::default(),
                );
                t.update_player_on_track(&car_on_line, track);
                assert!(t.on_start_gate, "{}: the line never registered", track.name);
                for _ in 0..40 {
                    t.tick();
                }
                // Drive far enough clear of the line and its road ribbon
                let clear_x = if sx + 8 < track.width {
                    sx + 8
                } else {
                    sx.saturating_sub(8)
                };
                let car_past_line = VehicleState::new(
                    Vec2::new(TrackDef::tile_centre(clear_x), TrackDef::tile_centre(sy)),
                    0,
                    crate::tuning::CarTuning::default(),
                );
                assert!(
                    t.update_player_on_track(&car_past_line, track),
                    "{}: crossing the line on a ({dx},{dy}) line must score a lap",
                    track.name
                );
                assert_eq!(t.current_lap, 2, "{}", track.name);
            }
        }
    }

    #[test]
    fn crossing_the_start_finish_off_centre_scores_the_lap() {
        // The start/finish symptom in isolation. The line tile covers a small
        // fraction of a road that is many tiles wide, so a car anywhere else
        // across the road used to drive a full lap that scored nothing.
        for track in ALL_TRACKS.iter() {
            let gate = track.start_gate;
            for (dx, dy) in LINE_OFFSETS {
                let (sx, sy) = offset_tile(gate, dx, dy);
                assert!(
                    track.is_road_at(sx, sy),
                    "{}: ({sx},{sy}) is off the road",
                    track.name
                );

                let mut t = LapTimer::<16>::new(track.checkpoint_slice(), gate);
                t.start();
                for g in track.checkpoint_slice() {
                    let car = VehicleState::new(
                        Vec2::new(TrackDef::tile_centre(g.x), TrackDef::tile_centre(g.y)),
                        0,
                        crate::tuning::CarTuning::default(),
                    );
                    t.update_player_on_track(&car, track);
                }
                assert!(t.all_checkpoints_cleared(), "{}", track.name);

                let car_on_line = VehicleState::new(
                    Vec2::new(TrackDef::tile_centre(sx), TrackDef::tile_centre(sy)),
                    0,
                    crate::tuning::CarTuning::default(),
                );
                t.update_player_on_track(&car_on_line, track);
                assert!(
                    t.on_start_gate,
                    "{}: crossing the line at ({sx},{sy}) (a ({dx},{dy}) line) \
                     did not register as being on the line",
                    track.name
                );
                let clear_x = if sx + 8 < track.width {
                    sx + 8
                } else {
                    sx.saturating_sub(8)
                };
                let car_past_line = VehicleState::new(
                    Vec2::new(TrackDef::tile_centre(clear_x), TrackDef::tile_centre(sy)),
                    0,
                    crate::tuning::CarTuning::default(),
                );
                assert!(
                    t.update_player_on_track(&car_past_line, track),
                    "{}",
                    track.name
                );
                assert_eq!(t.current_lap, 2, "{}", track.name);
            }
        }
    }

    #[test]
    fn gate_tolerance_does_not_swallow_the_start_finish_crossing() {
        // The other side of the tolerance: it must make the line catchable
        // without making it sticky. A car that has genuinely driven away from
        // the line has left it, however close it was a moment ago.
        let mut t = LapTimer::<4>::new(&[gate(1, 1), gate(2, 1), gate(3, 1), gate(4, 1)], START);
        t.start();
        for g in [gate(1, 1), gate(2, 1), gate(3, 1), gate(4, 1)] {
            drive_to(&mut t, g, 40);
        }
        t.update_player_tile(START.x, START.y);
        assert!(t.on_start_gate);
        // Already outside the line: the crossing must register on this very
        // tick, not be deferred until the car is further away still.
        assert!(
            t.update_player_tile(START.x + 3, START.y),
            "a car clear of the line has crossed it"
        );
        assert_eq!(t.current_lap, 2);
    }

    #[test]
    fn start_finish_ribbon_detected_across_full_road_width() {
        // Cascade Falls start/finish line spans x: 14..=21 at y: 24..=25 (16 tiles total).
        // The authored start_gate is only at (18, 25).
        // Cars driving at the extreme edges (e.g. x=14 and x=21) must be detected on the gate.
        let track = ALL_TRACKS[0]; // Cascade Falls
        let mut t = LapTimer::<16>::new(track.checkpoint_slice(), track.start_gate);
        t.start();

        // Far-left edge of the road ribbon (x=14, y=24)
        assert_eq!(track.tile_at(14, 24), TrackTile::StartFinish);
        let left_car = VehicleState::new(
            Vec2::new(TrackDef::tile_centre(14), TrackDef::tile_centre(24)),
            0,
            crate::tuning::CarTuning::default(),
        );
        t.update_player_on_track(&left_car, track);
        assert!(t.on_start_gate, "left edge of start line must be detected");

        // Step off the line to reset
        let clear_car = VehicleState::new(
            Vec2::new(TrackDef::tile_centre(14), TrackDef::tile_centre(10)),
            0,
            crate::tuning::CarTuning::default(),
        );
        t.update_player_on_track(&clear_car, track);
        assert!(!t.on_start_gate);

        // Far-right edge of the road ribbon (x=21, y=24)
        assert_eq!(track.tile_at(21, 24), TrackTile::StartFinish);
        let right_car = VehicleState::new(
            Vec2::new(TrackDef::tile_centre(21), TrackDef::tile_centre(24)),
            0,
            crate::tuning::CarTuning::default(),
        );
        t.update_player_on_track(&right_car, track);
        assert!(t.on_start_gate, "right edge of start line must be detected");
    }

    #[test]
    fn vehicle_nose_and_flank_sampling_registers_gate() {
        // Test that a car whose center is outside the gate tile still registers
        // when its nose or flanks clip the gate.
        let track = ALL_TRACKS[0];
        let mut t = LapTimer::<16>::new(track.checkpoint_slice(), track.start_gate);
        t.start();

        // Start line ribbon starts at y=24 (world y: [24*32, 25*32) = [768, 800)).
        // Position car at y=810 (tile 25, center y = 816), heading north (heading 0 BAMs).
        // Center is at world y = 810 (tile 25, which on x=18 is start line).
        // Let's place center at tile y = 26 (outside the start line, y = 26 * 32 + 10 = 842).
        // Center tile is y=26, which is tarmac (NOT start line: 842 / 32 = 26).
        assert_ne!(track.tile_at(18, 26), TrackTile::StartFinish);

        // Nose is +12 units forward (towards -Y, heading North = 0).
        // Position car center at y = 808 (tile 25). Nose reaches y = 808 - 12 = 796 (tile 24, which is StartFinish).
        // Or position center at y = 835 (tile 26). Heading 0 (North): fwd is (0, -1).
        // Front is (x, 835 - 12 = 823), still in tile 25.
        // If center is at world y = 840 (tile 26), with front at 840 - 12 = 828 (tile 25).
        // Tile (18, 25) is StartFinish! Tile (18, 26) is Tarmac!
        let car_nose_on_line = VehicleState::new(
            Vec2::new(TrackDef::tile_centre(18), Fixed::from_int(840)),
            0, // Heading North: fwd is (0, -1)
            crate::tuning::CarTuning::default(),
        );
        // Center position is tile 26:
        assert_eq!(TrackDef::tile_y_of(car_nose_on_line.position.y), 26);
        assert_eq!(track.tile_at(18, 26), TrackTile::Tarmac);

        t.update_player_on_track(&car_nose_on_line, track);
        assert!(
            t.on_start_gate,
            "vehicle nose touching the start line must register even when center is on tarmac"
        );
    }
}
