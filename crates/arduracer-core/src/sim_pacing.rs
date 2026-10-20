//! Fixed-timestep pacing: how many 60 Hz simulation ticks a rendered frame
//! runs.
//!
//! Time is counted in *units*: one tick costs [`TICK_COST`] units, and a VBlank
//! is worth 5 units on NTSC (60 Hz, one tick) or 6 on PAL (50 Hz, 1.2 ticks).
//!
//! The pacer used to run every tick owed, so a frame that missed its deadline
//! made the next one run an extra tick: about 3 ms more physics, AI and lap
//! logic on a frame already behind. Measured in a race, late frames came in
//! streaks of up to 8 for exactly that reason. Now a frame that follows a late
//! one runs only the ticks an on-time frame would; the rest is carried and paid
//! back one tick at a time, only in frames following one that finished with
//! [`REPAY_SLACK_LINES`] to spare. Once [`MAX_CARRY_TICKS`] are carried the
//! frames are late as a rule, so every frame repays one; only what is owed
//! beyond that is dropped.

/// Units one simulation tick costs.
pub const TICK_COST: u32 = 5;

/// Most ticks of owed time carried into later frames.
pub const MAX_CARRY_TICKS: u32 = 3;

/// Scanlines the last frame must have finished before its deadline for this
/// one to repay a carried tick. One tick of physics, AI and lap logic is about
/// 3 ms in a race, roughly 50 scanlines.
pub const REPAY_SLACK_LINES: u32 = 64;

/// Most VBlanks one call accounts for. A longer gap (loading, a pause) says
/// nothing about the race.
const MAX_ELAPSED_VBLANKS: u32 = 4;

/// The fixed-timestep accumulator.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SimPacer {
    units: u32,
}

impl SimPacer {
    pub const fn new() -> Self {
        Self { units: 0 }
    }

    /// Drops any owed time (a race restart, a resume from pause).
    pub fn reset(&mut self) {
        self.units = 0;
    }

    /// Units of owed time not yet run.
    pub const fn carried_units(&self) -> u32 {
        self.units
    }

    /// The ticks to run this frame, `elapsed_vblanks` after the last call.
    ///
    /// `frame_vblanks` is the frame-rate lock (2 at 30 FPS) and `slack_lines`
    /// how long before its deadline the last frame's work finished (0 if it
    /// was late). Always at least one tick.
    pub fn ticks(
        &mut self,
        elapsed_vblanks: u32,
        units_per_vblank: u32,
        frame_vblanks: u32,
        slack_lines: u32,
    ) -> u32 {
        if elapsed_vblanks == 0 {
            return 1;
        }
        let elapsed = if elapsed_vblanks > MAX_ELAPSED_VBLANKS {
            MAX_ELAPSED_VBLANKS
        } else {
            elapsed_vblanks
        };
        // Carried from earlier frames, before this one's time is added.
        let full = self.units >= MAX_CARRY_TICKS * TICK_COST;
        self.units = self.units.saturating_add(elapsed * units_per_vblank);
        let frame_vblanks = if frame_vblanks == 0 { 1 } else { frame_vblanks };
        // What an on-time frame runs, rounded up: 2 on NTSC, 3 on PAL.
        let on_time = (frame_vblanks * units_per_vblank).div_ceil(TICK_COST);
        // Repay a carried tick after a frame with room for it -- or whenever
        // the carry is full, which means the frames are late as a rule rather
        // than now and then (60 FPS mode on a scene that cannot hold 60).
        // Without that the carry is dropped every frame and the race runs in
        // slow motion: measured at ~0.6x in 60 FPS mode at ~39 FPS.
        let cap = if slack_lines >= REPAY_SLACK_LINES || full {
            on_time + 1
        } else {
            on_time
        };
        let owed = self.units / TICK_COST;
        let ticks = if owed < cap { owed } else { cap };
        self.units -= ticks * TICK_COST;
        let max_carry = MAX_CARRY_TICKS * TICK_COST + (TICK_COST - 1);
        if self.units > max_carry {
            self.units = max_carry;
        }
        if ticks == 0 {
            1
        } else {
            ticks
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NTSC: u32 = 5;
    const PAL: u32 = 6;
    const ROOMY: u32 = REPAY_SLACK_LINES;

    #[test]
    fn on_time_ntsc_frames_run_two_ticks_and_carry_nothing() {
        let mut p = SimPacer::new();
        for _ in 0..100 {
            assert_eq!(p.ticks(2, NTSC, 2, ROOMY), 2);
            assert_eq!(p.carried_units(), 0);
        }
    }

    #[test]
    fn a_late_frame_does_not_make_the_next_one_heavier() {
        let mut p = SimPacer::new();
        assert_eq!(p.ticks(3, NTSC, 2, 0), 2);
        assert_eq!(p.carried_units(), TICK_COST);
    }

    #[test]
    fn carried_ticks_are_repaid_one_per_frame_with_slack() {
        let mut p = SimPacer::new();
        p.ticks(3, NTSC, 2, 0);
        p.ticks(3, NTSC, 2, 0);
        assert_eq!(p.carried_units(), 2 * TICK_COST);
        // No slack: nothing repaid.
        assert_eq!(p.ticks(2, NTSC, 2, ROOMY - 1), 2);
        assert_eq!(p.ticks(2, NTSC, 2, ROOMY), 3);
        assert_eq!(p.ticks(2, NTSC, 2, ROOMY), 3);
        assert_eq!(p.ticks(2, NTSC, 2, ROOMY), 2);
        assert_eq!(p.carried_units(), 0);
    }

    #[test]
    fn carry_is_bounded() {
        let mut p = SimPacer::new();
        for _ in 0..50 {
            assert!(p.ticks(4, NTSC, 2, 0) <= 3);
            assert!(p.carried_units() < (MAX_CARRY_TICKS + 1) * TICK_COST);
        }
        let mut repaid = 0;
        for _ in 0..10 {
            repaid += p.ticks(2, NTSC, 2, ROOMY) - 2;
        }
        assert!(repaid <= MAX_CARRY_TICKS);
        assert!(p.carried_units() < TICK_COST);
    }

    #[test]
    fn steady_overload_keeps_real_time() {
        // 60 FPS mode where frames take 1.5 VBlanks on average: the race must
        // not fall into slow motion.
        let mut p = SimPacer::new();
        let (mut ticks, mut vblanks) = (0, 0);
        for i in 0..600 {
            let elapsed = 1 + (i & 1);
            ticks += p.ticks(elapsed, NTSC, 1, 0);
            vblanks += elapsed;
        }
        assert!(
            vblanks - ticks <= MAX_CARRY_TICKS + 1,
            "{ticks} of {vblanks}"
        );
    }

    #[test]
    fn pal_on_time_frames_average_sixty_hertz() {
        let mut p = SimPacer::new();
        let ticks: u32 = (0..50).map(|_| p.ticks(2, PAL, 2, ROOMY)).sum();
        // 50 frames of 2 PAL VBlanks are 2 s, 120 ticks.
        assert_eq!(ticks, 120);
    }

    #[test]
    fn real_time_is_kept_when_every_frame_has_slack() {
        let mut p = SimPacer::new();
        let mut ticks = 0;
        let mut vblanks = 0;
        for i in 0..300 {
            let elapsed = if i % 7 == 0 { 3 } else { 2 };
            ticks += p.ticks(elapsed, NTSC, 2, ROOMY);
            vblanks += elapsed;
        }
        assert!(vblanks - ticks <= MAX_CARRY_TICKS);
    }

    #[test]
    fn sixty_fps_lock_runs_one_tick() {
        let mut p = SimPacer::new();
        assert_eq!(p.ticks(1, NTSC, 1, 0), 1);
        assert_eq!(p.ticks(2, NTSC, 1, 0), 1);
        assert_eq!(p.ticks(1, NTSC, 1, ROOMY), 2);
    }

    #[test]
    fn always_runs_at_least_one_tick() {
        let mut p = SimPacer::new();
        assert_eq!(p.ticks(0, NTSC, 2, 0), 1);
        assert_eq!(p.carried_units(), 0);
    }
}
