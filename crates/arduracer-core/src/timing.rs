//! Checkpoint tracking, lap timer, and target par times.
//!
//! Operates on a 60 Hz fixed tick rate matching PSX NTSC 60 FPS display.
//!
//! Lap validation reproduces ArduRacer FX exactly (see `racer.cpp`): a lap only
//! counts when the car *leaves* the start/finish block having already touched
//! every checkpoint. Checkpoints may be taken in any order, so cutting a corner
//! short simply leaves that checkpoint uncounted for the lap - it cannot be
//! skipped, but the player is never forced into a rigid gate sequence either.

/// Ticks per second (60 Hz NTSC).
pub const TICKS_PER_SECOND: u32 = 60;
/// Default timed laps in an arcade time trial.
pub const TOTAL_LAPS: u8 = 5;

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
}

/// Real-time lap timer with ArduRacer FX style anti-cheat checkpoint validation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LapTimer<const MAX_CHECKPOINTS: usize> {
    pub current_lap: u8,
    pub current_lap_ticks: u32,
    pub best_lap_ticks: u32,
    pub last_completed_lap_ticks: u32,
    pub total_checkpoints: u8,
    pub checkpoints: [CheckpointGate; MAX_CHECKPOINTS],
    /// Start / finish gate. Leaving it with every checkpoint cleared scores a lap.
    pub start_gate: CheckpointGate,
    /// Bit `i` set once checkpoint `i` has been touched during the current lap.
    pub checkpoint_mask: u16,
    /// True while the car is inside the start/finish block.
    pub on_start_gate: bool,
    /// Lap timing arms once the car first leaves the grid.
    pub is_running: bool,
    pub is_finished: bool,
}

impl<const MAX_CHECKPOINTS: usize> LapTimer<MAX_CHECKPOINTS> {
    /// Builds a timer from a track's start gate and checkpoint list.
    pub fn new(checkpoints: &[CheckpointGate], start_gate: CheckpointGate) -> Self {
        let count = checkpoints.len().min(MAX_CHECKPOINTS);
        let mut gates = [CheckpointGate::default(); MAX_CHECKPOINTS];
        gates[..count].copy_from_slice(&checkpoints[..count]);

        LapTimer {
            current_lap: 1,
            current_lap_ticks: 0,
            best_lap_ticks: u32::MAX,
            last_completed_lap_ticks: 0,
            total_checkpoints: count as u8,
            checkpoints: gates,
            start_gate,
            checkpoint_mask: 0,
            on_start_gate: true,
            is_running: false,
            is_finished: false,
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
        self.total_checkpoints > 0 && self.checkpoints_cleared() >= self.total_checkpoints as u32
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
        // The grid places the car on the start/finish block, so the clock only
        // arms once it drives off it.
        self.on_start_gate = self.start_gate.is_active();
    }

    /// Feeds the player's tile position into the gate state machine.
    /// Returns true when a lap was just scored.
    pub fn update_player_tile(&mut self, tx: u8, ty: u8) -> bool {
        if self.is_finished {
            return false;
        }

        let inside_start = self.start_gate.is_active() && self.start_gate.contains_tile(tx, ty);

        // Register checkpoint touches (order independent, anti-cheat by coverage).
        for i in 0..self.total_checkpoints as usize {
            if self.checkpoint_mask & (1 << i) != 0 {
                continue;
            }
            if self.checkpoints[i].contains_tile(tx, ty) {
                self.checkpoint_mask |= 1 << i;
            }
        }

        // Crossing the start/finish line: score the lap if every gate was cleared.
        let left_start = self.on_start_gate && !inside_start;
        self.on_start_gate = inside_start;

        if left_start && self.is_running && self.all_checkpoints_cleared() {
            self.checkpoint_mask = 0;
            self.last_completed_lap_ticks = self.current_lap_ticks;
            if self.current_lap_ticks > 0 && self.current_lap_ticks < self.best_lap_ticks {
                self.best_lap_ticks = self.current_lap_ticks;
            }

            if self.current_lap >= TOTAL_LAPS {
                self.is_finished = true;
                self.is_running = false;
            } else {
                self.current_lap += 1;
                self.current_lap_ticks = 0;
            }
            return true;
        }

        false
    }

    /// Live delta against the player's best lap, in ticks.
    ///
    /// Positive means the current lap is slower than the reference pace (red),
    /// negative means it is ahead (green). The reference pace is prorated by how
    /// much of the checkpoint ring has been cleared, which is what an arcade
    /// split indicator shows. Returns 0 when no reference lap exists yet.
    pub fn delta_ticks(&self) -> i32 {
        if self.best_lap_ticks == u32::MAX || self.total_checkpoints == 0 {
            return 0;
        }
        let progress = self.checkpoints_cleared() as u64;
        let total = self.total_checkpoints as u64;
        // Ticks at which the reference run is expected to have cleared `progress`
        // gates, plus the partial gate the car is currently inside.
        let expected = ((self.best_lap_ticks as u64 * progress) / total) as i32;
        self.current_lap_ticks as i32 - expected
    }

    /// Index of the next route node (checkpoints in driving order, then the
    /// start/finish gate), for HUD and rival standings.
    pub fn route_node_index(&self, track_route_len: usize) -> usize {
        if track_route_len == 0 {
            0
        } else {
            self.checkpoints_cleared() as usize % track_route_len
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
