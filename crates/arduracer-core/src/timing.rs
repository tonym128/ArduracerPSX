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
/// Default timed laps in an arcade time trial and tournament race.
pub const TOTAL_LAPS: u8 = 3;

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

/// Half-width, in tiles, of the halo [`CheckpointGate::touches_tile`] adds around
/// every gate.
///
/// One tile absorbs the error a car accumulates tracking the centreline on a
/// straight; the measured miss at hairpins is more like two. Scored across the
/// whole track by the circuit suite: 1 is too tight, 3 wedges rivals on folded
/// corners (measured on Copper Gorge) -- 2 is where every rival laps.
pub const TILE_TOUCH_HALO: i32 = 2;

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

    /// Forgiving touch test: `contains_tile` plus a one-tile halo.
    ///
    /// Every gate is a thin line across the centreline, and rivals/reference
    /// cars follow that centreline to within roughly a cell -- measured on The
    /// Esses and Longbow, where a car missed the gate line by exactly a column
    /// or row and so the lap could never close. Scoring the crossing with a
    /// one-tile halo makes a one-cell miss a hit, while keeping the gate's
    /// *geometry* (for route length and disjointness) a strict one-cell line.
    #[inline]
    pub fn touches_tile(&self, tx: u8, ty: u8) -> bool {
        tx.wrapping_add(TILE_TOUCH_HALO as u8 + 1) >= self.x
            && tx <= self.x.saturating_add(self.width) + TILE_TOUCH_HALO as u8
            && ty.wrapping_add(TILE_TOUCH_HALO as u8 + 1) >= self.y
            && ty <= self.y.saturating_add(self.height) + TILE_TOUCH_HALO as u8
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
        let from_par = self.dev_platinum_ticks / 4;
        from_par.clamp(MIN_PLAUSIBLE_LAP_TICKS, MAX_PLAUSIBLE_LAP_TICKS)
    }

    /// Returns the next unachieved `(Medal, target_ticks)` for a given personal
    /// best lap time (`0` or `NO_BEST_LAP` if no lap recorded yet).
    pub fn next_medal_target(&self, best_ticks: u32) -> (Medal, u32) {
        if best_ticks == 0 || best_ticks == NO_BEST_LAP || best_ticks > self.bronze_ticks {
            (Medal::Bronze, self.bronze_ticks)
        } else if best_ticks > self.silver_ticks {
            (Medal::Silver, self.silver_ticks)
        } else if best_ticks > self.gold_ticks {
            (Medal::Gold, self.gold_ticks)
        } else {
            (Medal::DevPlatinum, self.dev_platinum_ticks)
        }
    }
}

/// Ticks each countdown light stays lit, at 60 Hz.
pub const COUNTDOWN_LIGHT_TICKS: u16 = 36;
/// Number of lights before GO (three, as in an arcade start rig).
pub const COUNTDOWN_LIGHTS: u8 = 3;
/// Ticks the GO state is held after the last light, so the player sees it.
pub const COUNTDOWN_GO_TICKS: u16 = 30;

/// Lower RPM boundary of the Perfect Launch rev-matching sweet spot.
pub const LAUNCH_SWEET_SPOT_MIN_RPM: u16 = 6_200;
/// Upper RPM boundary of the Perfect Launch rev-matching sweet spot.
pub const LAUNCH_SWEET_SPOT_MAX_RPM: u16 = 7_600;

/// Outcome of the grid rev-matching launch mini-game when the lights go out.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum LaunchOutcome {
    /// Standard standing launch (low/moderate RPM).
    Normal,
    /// Sweet-spot launch (`6,200..=7,600` RPM): instant 35-tick Turbo Start!
    TurboBoost,
    /// Over-revved redline (`> 7,600` RPM): brief wheelspin burnout.
    Wheelspin,
}

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

    /// Evaluates the player's engine RPM at the instant the start lights go out.
    pub const fn evaluate_launch(rpm: u16) -> LaunchOutcome {
        if rpm >= LAUNCH_SWEET_SPOT_MIN_RPM && rpm <= LAUNCH_SWEET_SPOT_MAX_RPM {
            LaunchOutcome::TurboBoost
        } else if rpm > LAUNCH_SWEET_SPOT_MAX_RPM {
            LaunchOutcome::Wheelspin
        } else {
            LaunchOutcome::Normal
        }
    }
}

/// Real-time lap timer with ArduRacer FX style anti-cheat checkpoint validation
/// and TrackMania per-checkpoint split timing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LapTimer<const MAX_CHECKPOINTS: usize> {
    pub current_lap: u8,
    pub target_laps: u8,
    pub current_lap_ticks: u32,
    /// Total elapsed race ticks across all laps, excluding countdown and pause time.
    pub total_race_ticks: u32,
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
    /// Exact `current_lap_ticks` recorded when checkpoint `i` was cleared on this lap.
    pub current_splits: [u32; MAX_CHECKPOINTS],
    /// Exact per-checkpoint split times from the fastest plausible lap (`best_lap_ticks`).
    pub best_splits: [u32; MAX_CHECKPOINTS],
    /// Forward speed recorded when checkpoint `i` was cleared on this lap (for Flying Respawn).
    pub current_split_speeds: [Fixed; MAX_CHECKPOINTS],
    /// Index of the most recently cleared checkpoint on this lap (`None` before any gate).
    pub last_cleared_checkpoint: Option<u8>,
    /// Elapsed lap ticks at the moment `last_cleared_checkpoint` was cleared.
    pub last_split_ticks: u32,
    /// Signed split delta (in ticks) at `last_cleared_checkpoint` against `best_splits`.
    pub last_split_delta: Option<i32>,
    /// Countdown ticks (`120` = 2.0s) for displaying the center-top TrackMania split banner.
    pub split_banner_ticks: u8,
    /// Number of checkpoint respawns used during this session (used by Mania Platform mode).
    pub respawn_count: u16,
    /// True while the car is inside the start/finish block.
    pub on_start_gate: bool,
    /// Lap timing arms once the car first leaves the grid.
    pub is_running: bool,
    pub is_finished: bool,
    /// Most recent failed start/finish validation, retained until the next attempt.
    pub validity_warning: LapValidityWarning,
    /// Brief HUD duration for explaining the failed crossing.
    pub validity_warning_ticks: u8,
    /// Shortest completed lap accepted as a lap time (see
    /// [`LapTimer::with_par_times`]). Private: it is an anti-cheat bound, not
    /// game state for a caller to tune.
    min_lap_ticks: u32,
}

/// Sentinel stored in [`LapTimer::best_lap_ticks`] while no lap time has been
/// recorded. Read it through [`LapTimer::best_lap_ticks`] instead.
pub const NO_BEST_LAP: u32 = u32::MAX;

/// Feedback shown after a start/finish crossing fails to validate the lap.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum LapValidityWarning {
    #[default]
    None,
    MissingCheckpoints,
    WrongDirection,
}

/// Longest lap the plausibility floor is allowed to be pushed to, in ticks
/// (60 s). The longest shipped circuit's reference lap is 38 s, so this only
/// ever clamps a nonsense bound handed in by a caller -- without it, passing
/// `u32::MAX` would silently switch the anti-cheat off.
pub const MAX_PLAUSIBLE_LAP_TICKS: u32 = 10_800;

/// Duration (in 60 Hz ticks) the TrackMania split banner stays visible after crossing a gate.
pub const SPLIT_BANNER_DURATION_TICKS: u8 = 120;

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
            target_laps: TOTAL_LAPS,
            current_lap_ticks: 0,
            total_race_ticks: 0,
            best_lap_ticks: NO_BEST_LAP,
            last_completed_lap_ticks: 0,
            total_checkpoints: count as u8,
            checkpoints: gates,
            start_gate,
            checkpoint_mask: 0,
            current_splits: [0; MAX_CHECKPOINTS],
            best_splits: [0; MAX_CHECKPOINTS],
            current_split_speeds: [Fixed::ZERO; MAX_CHECKPOINTS],
            last_cleared_checkpoint: None,
            last_split_ticks: 0,
            last_split_delta: None,
            split_banner_ticks: 0,
            respawn_count: 0,
            // Unverified until a tile feed says so; `start` keeps that state.
            on_start_gate: false,
            is_running: false,
            is_finished: false,
            validity_warning: LapValidityWarning::None,
            validity_warning_ticks: 0,
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

    /// Loads a trusted Personal Best reference and its exact checkpoint splits.
    /// Returns false for an impossible time or split data outside this circuit.
    pub fn load_best_lap_reference(&mut self, lap_ticks: u32, splits: &[u32]) -> bool {
        if lap_ticks < self.min_lap_ticks
            || lap_ticks == NO_BEST_LAP
            || splits.len() > self.active_checkpoint_count()
            || splits.iter().any(|&split| split > lap_ticks)
        {
            return false;
        }
        self.best_lap_ticks = lap_ticks;
        self.best_splits.fill(0);
        self.best_splits[..splits.len()].copy_from_slice(splits);
        true
    }

    /// Whether a plausible lap has been recorded, i.e. whether a best-lap
    /// readout and a delta split should be shown at all.
    #[inline]
    pub fn has_best_lap(&self) -> bool {
        self.best_lap_ticks != NO_BEST_LAP
    }

    /// Configures the number of laps required to finish the session (e.g. `1` for
    /// point-to-point Mania Sprint trials, `TOTAL_LAPS` for standard circuits).
    pub fn with_target_laps(mut self, target_laps: u8) -> Self {
        self.target_laps = target_laps.max(1);
        self
    }

    /// Whether the timer is currently on the final lap of the race.
    #[inline]
    pub fn is_final_lap(&self) -> bool {
        self.target_laps > 1 && self.current_lap >= self.target_laps
    }

    /// Advances the timer by one 60Hz tick.
    pub fn tick(&mut self) {
        if self.is_running && !self.is_finished {
            self.current_lap_ticks = self.current_lap_ticks.saturating_add(1);
            self.total_race_ticks = self.total_race_ticks.saturating_add(1);
        }
        self.split_banner_ticks = self.split_banner_ticks.saturating_sub(1);
        self.validity_warning_ticks = self.validity_warning_ticks.saturating_sub(1);
    }

    /// Starts timing (called when the countdown light turns GREEN).
    pub fn start(&mut self) {
        self.is_running = true;
        self.is_finished = false;
        self.current_lap = 1;
        self.current_lap_ticks = 0;
        self.total_race_ticks = 0;
        self.checkpoint_mask = 0;
        self.current_splits = [0; MAX_CHECKPOINTS];
        self.current_split_speeds = [Fixed::ZERO; MAX_CHECKPOINTS];
        self.last_cleared_checkpoint = None;
        self.last_split_ticks = 0;
        self.last_split_delta = None;
        self.split_banner_ticks = 0;
        self.respawn_count = 0;
        // The grid sits inside the start/finish block, but the timer is not told
        // that: it starts *outside* it and only records the car as being on the
        // line once a tile feed says so. That makes the roll off the grid a
        // non-event rather than a lap crossing, so every crossing this state
        // machine ever reports really is a return to the line -- and no gate
        // progress can survive one.
        self.on_start_gate = false;
    }

    /// Records the split time and entry speed when checkpoint `i` is newly cleared.
    fn record_checkpoint_split(&mut self, i: usize, speed: Fixed) {
        if i >= MAX_CHECKPOINTS {
            return;
        }
        self.current_splits[i] = self.current_lap_ticks;
        self.current_split_speeds[i] = speed;
        self.last_cleared_checkpoint = Some(i as u8);
        self.last_split_ticks = self.current_lap_ticks;
        self.last_split_delta = if self.has_best_lap() && self.best_splits[i] > 0 {
            let delta = self.current_lap_ticks as i64 - self.best_splits[i] as i64;
            Some(delta.clamp(i32::MIN as i64, i32::MAX as i64) as i32)
        } else {
            None
        };
        self.split_banner_ticks = SPLIT_BANNER_DURATION_TICKS;
    }

    /// Active checkpoint split banner data `(checkpoint_index, split_ticks, split_delta_ticks)`
    /// while the post-checkpoint banner timer (`split_banner_ticks`) is still counting down.
    #[inline]
    pub fn checkpoint_split_banner(&self) -> Option<(u8, u32, Option<i32>)> {
        if self.split_banner_ticks > 0 {
            self.last_cleared_checkpoint
                .map(|cp| (cp, self.last_split_ticks, self.last_split_delta))
        } else {
            None
        }
    }

    /// Records a checkpoint respawn while keeping the lap timer running, and returns
    /// `(last_cleared_checkpoint, entry_speed_at_checkpoint)` for standing or flying respawn.
    pub fn record_respawn(&mut self) -> (Option<u8>, Fixed) {
        self.respawn_count = self.respawn_count.saturating_add(1);
        let speed = match self.last_cleared_checkpoint {
            Some(idx) if (idx as usize) < MAX_CHECKPOINTS => {
                self.current_split_speeds[idx as usize]
            }
            _ => Fixed::ZERO,
        };
        (self.last_cleared_checkpoint, speed)
    }

    /// Evaluates the Mania Platform mode medal based on respawn count (fewer respawns = higher medal).
    pub const fn evaluate_platform_medal(&self) -> Medal {
        if !self.is_finished {
            return Medal::None;
        }
        match self.respawn_count {
            0 => Medal::DevPlatinum,
            1 => Medal::Gold,
            2 | 3 => Medal::Silver,
            4..=6 => Medal::Bronze,
            _ => Medal::None,
        }
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
        let finished_splits = self.current_splits;
        if left_start {
            self.checkpoint_mask = 0;
            self.current_splits = [0; MAX_CHECKPOINTS];
            self.current_split_speeds = [Fixed::ZERO; MAX_CHECKPOINTS];
            self.last_cleared_checkpoint = None;
            self.last_split_delta = None;
        }

        // Register checkpoint touches (order independent, anti-cheat by coverage).
        for i in 0..self.active_checkpoint_count() {
            let bit = Self::gate_bit(i);
            if self.checkpoint_mask & bit != 0 {
                continue;
            }
            if self.checkpoints[i].contains_tile(tx, ty) {
                self.checkpoint_mask |= bit;
                self.record_checkpoint_split(i, Fixed::ZERO);
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
            self.best_splits = finished_splits;
        }

        if self.current_lap >= self.target_laps.max(1) {
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
                if (sc - pt).shorter_than(Fixed::from_int(48)) {
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

        let total_gates = self.active_checkpoint_count();
        let left_start = self.on_start_gate && !inside_start;
        let completed = left_start
            && self.all_checkpoints_cleared()
            && track.is_forward_crossing(total_gates, player.velocity);
        let finished_splits = self.current_splits;
        if left_start {
            self.validity_warning = if !self.all_checkpoints_cleared() {
                LapValidityWarning::MissingCheckpoints
            } else if !track.is_forward_crossing(total_gates, player.velocity) {
                LapValidityWarning::WrongDirection
            } else {
                LapValidityWarning::None
            };
            self.validity_warning_ticks = if self.validity_warning == LapValidityWarning::None {
                0
            } else {
                180
            };
            self.checkpoint_mask = 0;
            self.current_splits = [0; MAX_CHECKPOINTS];
            self.current_split_speeds = [Fixed::ZERO; MAX_CHECKPOINTS];
            self.last_cleared_checkpoint = None;
            self.last_split_delta = None;
        }

        // Register checkpoint touches
        let sample_pts = [pos, front, rear, left, right];
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
                if (gate_centre - pt).shorter_than(Fixed::from_int(48)) {
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

            if touched && track.is_forward_crossing(i, player.velocity) {
                self.checkpoint_mask |= bit;
                self.record_checkpoint_split(i, player.speed.max(Fixed::ZERO));
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
            self.best_splits = finished_splits;
        }

        if self.current_lap >= self.target_laps.max(1) {
            self.is_finished = true;
            self.is_running = false;
        } else {
            self.current_lap += 1;
            self.current_lap_ticks = 0;
        }
        true
    }

    /// Most recent exact checkpoint delta against the player's best lap, in ticks.
    /// Positive means the current split is slower (red); negative means it is
    /// ahead (green). The value holds between gate crossings rather than
    /// estimating time from route progress. Returns 0 until a comparable split
    /// has been recorded.
    pub fn delta_ticks(&self) -> i32 {
        self.last_split_delta.unwrap_or(0)
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
    use crate::track::{TrackDef, MAX_TRACK_CHECKPOINTS, MIN_ROAD_HALF_WIDTH, TILE_SIZE};

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
    fn total_race_time_accumulates_across_laps_and_resets_for_a_new_race() {
        let mut t = four_gate_timer::<4>().with_target_laps(2);
        full_lap(&mut t, 60);
        assert_eq!(t.current_lap_ticks, 0);
        assert_eq!(t.total_race_ticks, 240);

        full_lap(&mut t, 90);
        assert!(t.is_finished);
        assert_eq!(t.last_completed_lap_ticks, 360);
        assert_eq!(t.total_race_ticks, 600);

        t.start();
        assert_eq!(t.current_lap_ticks, 0);
        assert_eq!(t.total_race_ticks, 0);
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
        assert_eq!(bound, track.par_times.dev_platinum_ticks / 4);
        assert!(bound > MIN_PLAUSIBLE_LAP_TICKS);

        // A lap at a fraction of the fastest verified reference lap: 3.4 units/tick of
        // top speed cannot cover the circuit in this time.
        let per_gate = bound / 8;
        for g in track.checkpoint_slice() {
            drive_to(&mut t, *g, per_gate);
        }
        t.update_player_tile(track.start_gate.x, track.start_gate.y);
        // Step clear of the start/finish to close the lap. Gates span the road
        // (a 1x1 gate on a 2-3 cell road is a target a car drives straight past),
        // so "clear" means past the gate's own width, not a fixed four tiles.
        let past = track.start_gate.x.wrapping_add(track.start_gate.width + 2);
        assert!(
            t.update_player_tile(past, track.start_gate.y),
            "leaving the start gate must arm the lap"
        );
        assert_eq!(
            t.best_lap_ticks(),
            None,
            "a lap faster than the anti-cheat bound is rejected"
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
        let past = track.start_gate.x.wrapping_add(track.start_gate.width + 2);
        assert!(
            timer.update_player_tile(past, track.start_gate.y),
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

    #[test]
    fn alleyway_chicanes_admits_fast_player_twenty_four_second_lap() {
        let track = ALL_TRACKS
            .iter()
            .find(|t| t.name == "Alleyway Chicanes")
            .expect("Alleyway Chicanes track exists");
        let mut t = LapTimer::<16>::with_par_times(
            track.checkpoint_slice(),
            track.start_gate,
            track.par_times,
        );
        t.start();

        // 24 seconds at 60 Hz = 1440 ticks.
        // Old anti-cheat had floor = 3878 / 2 = 1939 ticks (32.3s), which rejected 1440 ticks.
        // New floor is 3878 / 4 = 969 ticks (16.15s), so 1440 ticks is accepted.
        let ticks_24s = 24 * 60;
        let per_gate = ticks_24s / track.checkpoint_count as u32;
        for g in track.checkpoint_slice() {
            drive_to(&mut t, *g, per_gate);
        }
        t.update_player_tile(track.start_gate.x, track.start_gate.y);
        let past = track.start_gate.x.wrapping_add(track.start_gate.width + 2);
        assert!(
            t.update_player_tile(past, track.start_gate.y),
            "crossing finish line must score"
        );
        let best = t
            .best_lap_ticks()
            .expect("24s lap must not be rejected by anti-cheat floor");
        assert_eq!(best, ticks_24s);
        assert_eq!(track.par_times.evaluate_medal(best), Medal::DevPlatinum);
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

    // ---- Exact split comparison: no route-progress interpolation ----

    #[test]
    fn delta_holds_the_last_exact_gate_comparison_until_the_next_gate() {
        let mut t = four_gate_timer::<4>();
        full_lap(&mut t, 150);
        assert_eq!(t.best_lap_ticks(), Some(600));

        // A gate crossing compares its actual timestamp to that gate's PB split,
        // even if legacy order-independent checkpoint coverage allows a gate
        // outside the contiguous route prefix to be cleared first.
        for _ in 0..100 {
            t.tick();
        }
        t.update_player_tile(RING[3].x, RING[3].y);
        assert_eq!(t.next_checkpoint_index(), 0);
        assert_eq!(t.current_splits[3], 100);
        assert_eq!(t.delta_ticks(), -500);

        // Elapsed time between gates does not invent a comparison value.
        for _ in 0..50 {
            t.tick();
        }
        assert_eq!(t.delta_ticks(), -500);

        // The next actual gate crossing replaces it with that exact split delta.
        t.update_player_tile(RING[0].x, RING[0].y);
        assert_eq!(t.next_checkpoint_index(), 1);
        assert_eq!(t.delta_ticks(), 150 - 150);
    }

    #[test]
    fn loaded_personal_best_uses_persisted_exact_checkpoint_splits() {
        let mut t = four_gate_timer::<4>();
        assert!(t.load_best_lap_reference(600, &[150, 300, 450, 600]));
        assert_eq!(t.best_lap_ticks(), Some(600));

        drive_to(&mut t, RING[0], 180);
        assert_eq!(t.last_split_delta, Some(30));
        assert_eq!(t.delta_ticks(), 30);

        let mut fresh = four_gate_timer::<4>();
        assert!(!fresh.load_best_lap_reference(600, &[150, 300, 450, 600, 750]));
        assert_eq!(fresh.best_lap_ticks(), None);
    }

    #[test]
    fn delta_remains_the_exact_split_when_the_lap_counter_saturates() {
        let mut t = four_gate_timer::<4>();
        full_lap(&mut t, 150);
        t.current_lap_ticks = 150;
        t.update_player_tile(RING[0].x, RING[0].y);
        assert_eq!(t.delta_ticks(), 0);

        // A race left running for years cannot change a checkpoint comparison.
        t.current_lap_ticks = u32::MAX;
        assert_eq!(t.delta_ticks(), 0);
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
        // The premise of the two tests below, asserted so the reason they exist
        // cannot rot: each gate is a single 1x1 tile, so it only covers
        // `1 / road_width` of the line the player is allowed to drive across.
        //
        // Which is only a *gap* if the road is wider than the gate. A one-lane
        // circuit is exactly one tile wide, so its gate spans the whole
        // corridor and there is nothing for the tolerance below to bridge --
        // it is a second, redundant safety net rather than the primary
        // mechanism. Those circuits are covered by
        // `a_one_lane_circuit_completes_a_lap_on_its_only_line`, which drives
        // the narrow case end to end instead of assuming it away here.
        let mut narrowest = usize::MAX;
        let mut one_lane_circuits = 0usize;
        for track in ALL_TRACKS.iter() {
            for gate in track
                .checkpoint_slice()
                .iter()
                .copied()
                .chain(std::iter::once(track.start_gate))
            {
                let width = road_width_at(track, gate);
                if width <= 1 {
                    one_lane_circuits += 1;
                }
                narrowest = narrowest.min(width);
                assert!(
                    width >= 1,
                    "{}: gate at ({},{}) sits on no road at all",
                    track.name,
                    gate.x,
                    gate.y
                );
            }
        }
        // Every circuit is either wider than its gates, or exactly one tile wide
        // and relying on the road-wide check. Nothing in between is a case this
        // reasoning covers, so a road two tiles wide is fine and a gate on no
        // road at all is not.
        assert!(
            narrowest >= 1,
            "every gate must sit on at least one tile of road"
        );
        // Whether or not the shipped fleet currently contains a narrow circuit,
        // the one-lane path is covered by
        // `a_one_lane_circuit_completes_a_lap_on_its_only_line`, which drives it
        // end to end. This test only reports how many circuits rely on it.
        let _ = one_lane_circuits;
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

    /// The [`LINE_OFFSETS`] that are on the road at *every* gate of the lap.
    ///
    /// Not all nine are legal everywhere, and they never were: a gate at a corner
    /// has road on one side and barrier on the other, so `(1,-1)` is a
    /// corner-cutting line on one circuit and a drive into the wall on the next.
    /// The tests below used to `assert!` that all nine were road, which is only
    /// true of circuits wide enough that every gate has three cells of road around
    /// it -- and it failed outright the moment a narrower circuit was cooked in,
    /// reporting a *legal* corner-cutting line as a defect.
    ///
    /// The filter is on the whole lap, not per gate, and that matters: a per-gate
    /// filter silently produces a lap where the car is on the centreline at one
    /// checkpoint and two tiles wide of it at the next. That is not a racing line,
    /// it is a teleport, and it can skip a gate on the way -- which is exactly what
    /// the first version of this did.
    ///
    /// A one-lane circuit offers the along-road offsets only, which is the point:
    /// it still has to complete a lap, on the one line it has.
    fn legal_line_offsets(track: &TrackDef) -> impl Iterator<Item = (i16, i16)> + '_ {
        LINE_OFFSETS.into_iter().filter(move |&(dx, dy)| {
            track
                .checkpoint_slice()
                .iter()
                .copied()
                .chain(std::iter::once(track.start_gate))
                .all(|g| {
                    let (tx, ty) = offset_tile(g, dx, dy);
                    track.is_road_at(tx, ty)
                })
        })
    }

    #[test]
    fn a_racing_line_off_the_authored_gates_still_completes_a_lap() {
        // The reported defect, end to end on every shipped circuit: drive the
        // whole lap one tile off the authored gate tiles -- an ordinary racing
        // line, and on a wide road a perfectly ordinary one -- and the lap
        // must still count.
        for track in ALL_TRACKS.iter() {
            let mut drove_any = false;
            for (dx, dy) in legal_line_offsets(track) {
                drove_any = true;
                let mut t = LapTimer::<16>::new(track.checkpoint_slice(), track.start_gate);
                t.start();

                for g in track.checkpoint_slice() {
                    let (tx, ty) = offset_tile(*g, dx, dy);
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
            assert!(
                drove_any,
                "{}: the authored gate line itself is not on the road, so there \
                 is no line at all to test",
                track.name
            );
        }
    }

    /// A road exactly one lane wide: the narrowest circuit the engine will
    /// accept, and the case that used to have no representation at all.
    ///
    /// The gate-detection tests above are about a car being *off the centre of
    /// the authored gate tile*. On a one-lane road there is no such place to be
    /// -- the road *is* two tiles wide, one lane -- so the question becomes the
    /// stronger one: is the road-wide gate still catchable at all, when the
    /// tolerance box drawn around it is three tiles wide and the road is two?
    ///
    /// The start line is deliberately placed *past* the last checkpoint rather
    /// than on it, which is how a real circuit is laid out and what the
    /// `LapTimer` mask arithmetic needs: the timer clears `checkpoint_mask` on
    /// leaving the start gate, so a start gate sitting on the first checkpoint
    /// takes that checkpoint and then immediately forgets it. On a wide road the
    /// two can be far enough apart for that not to bite; on a one-lane road they
    /// are on the same line, and it is fatal.
    fn one_lane_track() -> (TrackDef, [CheckpointGate; MAX_TRACK_CHECKPOINTS]) {
        let mut gates = [CheckpointGate::default(); MAX_TRACK_CHECKPOINTS];
        let mut tiles = [TrackTile::OffRoad; 100];
        // A 10x10 grid whose road is exactly two cells wide (rows 5 and 6),
        // which is one lane at 32 world units a cell.
        for x in 0..10usize {
            tiles[5 * 10 + x] = TrackTile::Tarmac;
            tiles[6 * 10 + x] = TrackTile::Tarmac;
        }
        for (i, slot) in gates.iter_mut().enumerate().take(4) {
            *slot = CheckpointGate {
                x: 1 + i as u8,
                y: 5,
                width: 1,
                height: 1,
            };
        }
        (
            TrackDef {
                name: "ONE LANE",
                width: 10,
                height: 10,
                start_pos: Vec2::new(Fixed::from_int(96), Fixed::from_int(96)),
                start_heading: 0,
                start_gate: CheckpointGate {
                    x: 8,
                    y: 5,
                    width: 1,
                    height: 1,
                },
                par_times: ParTimes::default(),
                checkpoint_count: 4,
                checkpoints: gates,
                tiles: Some(Box::leak(Box::new(tiles))),
                route: &[],
                // One lane, exactly: the narrowest legal road.
                half_width: MIN_ROAD_HALF_WIDTH as u16,
            },
            gates,
        )
    }

    #[test]
    fn a_one_lane_circuit_completes_a_lap_on_its_only_line() {
        let (track, gates) = one_lane_track();
        assert_eq!(track.lane_count(), 1);
        assert_eq!(track.road_half_width(), MIN_ROAD_HALF_WIDTH);
        // The tolerance box around a gate is three tiles across; the road is two.
        // So on a one-lane circuit the gate is catchable *only* because the
        // road-wide check exists, and this is the test that says so.
        const _: () = assert!(
            MIN_ROAD_HALF_WIDTH * 2 < TILE_SIZE * 3,
            "the one-lane road must be narrower than a gate's tolerance box for \
             this test to mean anything"
        );

        // The active slice, as `TrackDef::checkpoint_slice` hands it over: the
        // full array carries 24 slots of which 4 are active.
        let active = &gates[..track.active_checkpoint_count()];
        let mut t = LapTimer::<16>::new(active, track.start_gate);
        t.start();
        for g in gates {
            let car = VehicleState::new(
                Vec2::new(TrackDef::tile_centre(g.x), TrackDef::tile_centre(g.y)),
                0,
                crate::tuning::CarTuning::default(),
            );
            t.update_player_on_track(&car, &track);
        }
        assert!(
            t.all_checkpoints_cleared(),
            "every checkpoint on a one-lane road must be catchable"
        );

        // Reaching the line must not disturb the mask: the timer only clears it
        // on the way *out* of the line.
        let car_on_line = VehicleState::new(
            Vec2::new(
                TrackDef::tile_centre(track.start_gate.x),
                TrackDef::tile_centre(track.start_gate.y),
            ),
            0,
            crate::tuning::CarTuning::default(),
        );
        t.update_player_on_track(&car_on_line, &track);
        assert!(t.on_start_gate, "the one-lane start line must register");
        assert!(
            t.all_checkpoints_cleared(),
            "reaching the start line must not wipe the checkpoints already taken"
        );
        assert_eq!(t.current_lap, 1);

        // Now clear of the line: that is the crossing, and it scores.
        let car_past_line = VehicleState::new(
            Vec2::new(
                TrackDef::tile_centre(track.start_gate.x.saturating_sub(4)),
                TrackDef::tile_centre(track.start_gate.y),
            ),
            0,
            crate::tuning::CarTuning::default(),
        );
        assert!(
            t.update_player_on_track(&car_past_line, &track),
            "a lap on a one-lane road must score"
        );
        assert_eq!(t.current_lap, 2);
    }

    /// The start line must be able to sit *on* a one-lane road alongside a
    /// checkpoint without one destroying the other.
    ///
    /// `LapTimer` clears `checkpoint_mask` the moment the car leaves the start
    /// gate, so a start gate drawn *on* the first checkpoint takes that
    /// checkpoint and then immediately forgets it. Invisible on a wide road --
    /// the two are far apart laterally so the 1-tile tolerance box around the
    /// line does not reach the gate -- and fatal on a one-lane road, where they
    /// are on the same line and every metre apart.
    #[test]
    fn a_one_lane_start_line_beyond_the_last_checkpoint_keeps_the_mask() {
        let (track, gates) = one_lane_track();
        let last = gates[(track.active_checkpoint_count() - 1).min(MAX_TRACK_CHECKPOINTS - 1)];
        assert!(
            track.start_gate.x > last.x,
            "the fixture must put the line past the checkpoints for this to \
             test anything"
        );
        assert!(
            track.start_gate.x - last.x > 1,
            "and by more than the gate tolerance, or the line's own box would \
             reach back to the last checkpoint"
        );
    }

    #[test]
    fn crossing_the_start_finish_off_centre_scores_the_lap() {
        // The start/finish symptom in isolation. The line tile covers a small
        // fraction of a road that is many tiles wide, so a car anywhere else
        // across the road used to drive a full lap that scored nothing.
        //
        // On a road only one or two lanes wide the gate tile *is* the road, so
        // "off centre" means "on the one line there is". Both are driven below.
        for track in ALL_TRACKS.iter() {
            let gate = track.start_gate;
            for (dx, dy) in LINE_OFFSETS {
                let (sx, sy) = offset_tile(gate, dx, dy);
                if !track.is_road_at(sx, sy) {
                    // Not a line this road offers. Covered by the one-lane test
                    // above and by the wide circuits' other eight offsets.
                    continue;
                }

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
        let track = ALL_TRACKS[0];
        let mut t = LapTimer::<16>::new(track.checkpoint_slice(), track.start_gate);
        t.start();

        if track.name == "Cascade Falls" {
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
        } else {
            let sy = track.start_gate.y;
            let mut sf_x = Vec::new();
            for x in 0..track.width {
                if track.tile_at(x, sy) == TrackTile::StartFinish {
                    sf_x.push(x);
                }
            }
            if let (Some(&min_x), Some(&max_x)) = (sf_x.first(), sf_x.last()) {
                let car = VehicleState::new(
                    Vec2::new(TrackDef::tile_centre(min_x), TrackDef::tile_centre(sy)),
                    0,
                    crate::tuning::CarTuning::default(),
                );
                t.update_player_on_track(&car, track);
                assert!(t.on_start_gate, "left edge of start line must be detected");

                let car2 = VehicleState::new(
                    Vec2::new(TrackDef::tile_centre(max_x), TrackDef::tile_centre(sy)),
                    0,
                    crate::tuning::CarTuning::default(),
                );
                t.update_player_on_track(&car2, track);
                assert!(t.on_start_gate, "right edge of start line must be detected");
            }
        }
    }

    #[test]
    fn vehicle_nose_and_flank_sampling_registers_gate() {
        // Test that a car whose center is outside the gate tile still registers
        // when its nose or flanks clip the gate.
        let track = ALL_TRACKS[0];
        if track.name != "Cascade Falls" {
            return;
        }
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

    #[test]
    fn checkpoint_splits_record_and_compute_exact_deltas() {
        let mut t = four_gate_timer::<4>();
        // Lap 1: 200 ticks per gate -> splits at [200, 400, 600, 800], lap = 800
        full_lap(&mut t, 200);
        assert_eq!(t.best_lap_ticks(), Some(800));
        assert_eq!(t.best_splits, [200, 400, 600, 800]);

        // Lap 2: reach gate 0 in 180 ticks (-20 delta), gate 1 in +230 ticks (+10 delta)
        drive_to(&mut t, RING[0], 180);
        assert_eq!(t.current_splits[0], 180);
        assert_eq!(t.last_cleared_checkpoint, Some(0));
        assert_eq!(t.last_split_delta, Some(-20));
        assert_eq!(t.checkpoint_split_banner(), Some((0, 180, Some(-20))));

        drive_to(&mut t, RING[1], 230);
        assert_eq!(t.current_splits[1], 410);
        assert_eq!(t.last_cleared_checkpoint, Some(1));
        assert_eq!(t.last_split_delta, Some(10));
        assert_eq!(t.checkpoint_split_banner(), Some((1, 410, Some(10))));

        // Banner expires after SPLIT_BANNER_DURATION_TICKS ticks
        for _ in 0..SPLIT_BANNER_DURATION_TICKS {
            t.tick();
        }
        assert_eq!(t.checkpoint_split_banner(), None);
    }

    #[test]
    fn respawn_uses_the_most_recently_cleared_gate_and_keeps_the_clock_running() {
        let mut t = four_gate_timer::<4>();
        t.start();
        drive_to(&mut t, RING[3], 40);

        let before_respawn = t.current_lap_ticks;
        assert_eq!(t.record_respawn().0, Some(3));
        assert_eq!(t.respawn_count, 1);
        t.tick();
        assert_eq!(t.current_lap_ticks, before_respawn + 1);
    }

    #[test]
    fn target_laps_and_platform_medal_evaluation() {
        let mut t = four_gate_timer::<4>().with_target_laps(1);
        t.start();
        assert_eq!(t.record_respawn().0, None);
        full_lap(&mut t, 200);
        assert!(t.is_finished, "1-lap sprint finishes after 1 lap");
        assert_eq!(t.respawn_count, 1);
        assert_eq!(t.evaluate_platform_medal(), Medal::Gold);
    }

    #[test]
    fn launch_rpm_evaluation_and_next_medal_target() {
        assert_eq!(StartSequence::evaluate_launch(4_000), LaunchOutcome::Normal);
        assert_eq!(
            StartSequence::evaluate_launch(LAUNCH_SWEET_SPOT_MIN_RPM),
            LaunchOutcome::TurboBoost
        );
        assert_eq!(
            StartSequence::evaluate_launch(7_000),
            LaunchOutcome::TurboBoost
        );
        assert_eq!(
            StartSequence::evaluate_launch(LAUNCH_SWEET_SPOT_MAX_RPM + 100),
            LaunchOutcome::Wheelspin
        );

        let par = ParTimes {
            bronze_ticks: 3_000,
            silver_ticks: 2_400,
            gold_ticks: 1_800,
            dev_platinum_ticks: 1_500,
        };
        assert_eq!(par.next_medal_target(0), (Medal::Bronze, 3_000));
        assert_eq!(par.next_medal_target(2_800), (Medal::Silver, 2_400));
        assert_eq!(par.next_medal_target(2_200), (Medal::Gold, 1_800));
        assert_eq!(par.next_medal_target(1_700), (Medal::DevPlatinum, 1_500));
        assert_eq!(par.next_medal_target(1_400), (Medal::DevPlatinum, 1_500));
    }
}
