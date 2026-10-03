//! Checkpoint tracking, lap timer, and target par times.
//!
//! Operates on a 60 Hz fixed tick rate matching PSX NTSC 60 FPS display.

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
    pub fn contains_tile(&self, tx: u8, ty: u8) -> bool {
        tx >= self.x && tx < self.x + self.width && ty >= self.y && ty < self.y + self.height
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

/// Real-time lap timer state machine with anti-cheat sequential checkpoint validation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LapTimer<const MAX_CHECKPOINTS: usize> {
    pub current_lap: u8,
    pub current_lap_ticks: u32,
    pub best_lap_ticks: u32,
    pub last_completed_lap_ticks: u32,
    pub next_checkpoint_idx: u8,
    pub total_checkpoints: u8,
    pub checkpoints: [CheckpointGate; MAX_CHECKPOINTS],
    pub is_running: bool,
    pub is_finished: bool,
}

impl<const MAX_CHECKPOINTS: usize> LapTimer<MAX_CHECKPOINTS> {
    pub fn new(checkpoints: &[CheckpointGate]) -> Self {
        let count = checkpoints.len().min(MAX_CHECKPOINTS);
        let mut gates = [CheckpointGate::default(); MAX_CHECKPOINTS];
        gates[..count].copy_from_slice(&checkpoints[..count]);

        LapTimer {
            current_lap: 1,
            current_lap_ticks: 0,
            best_lap_ticks: u32::MAX,
            last_completed_lap_ticks: 0,
            next_checkpoint_idx: 0,
            total_checkpoints: count as u8,
            checkpoints: gates,
            is_running: false,
            is_finished: false,
        }
    }

    /// Advances the timer by one 60Hz tick.
    pub fn tick(&mut self) {
        if self.is_running && !self.is_finished {
            self.current_lap_ticks = self.current_lap_ticks.saturating_add(1);
        }
    }

    /// Starts timing (e.g. when countdown light turns GREEN).
    pub fn start(&mut self) {
        self.is_running = true;
        self.is_finished = false;
        self.current_lap = 1;
        self.current_lap_ticks = 0;
        self.next_checkpoint_idx = 0;
    }

    /// Checks player tile against checkpoint gates and advances lap when all gates are cleared.
    /// Returns true if a lap was just completed.
    pub fn update_player_tile(&mut self, tx: u8, ty: u8) -> bool {
        if !self.is_running || self.is_finished || self.total_checkpoints == 0 {
            return false;
        }

        // Check if player stepped into the expected next checkpoint
        let target = self.checkpoints[self.next_checkpoint_idx as usize];
        if target.contains_tile(tx, ty) {
            self.next_checkpoint_idx += 1;

            // If player cleared the final checkpoint (Start/Finish line)
            if self.next_checkpoint_idx >= self.total_checkpoints {
                self.next_checkpoint_idx = 0;
                self.last_completed_lap_ticks = self.current_lap_ticks;

                if self.current_lap_ticks < self.best_lap_ticks {
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
        }
        false
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
