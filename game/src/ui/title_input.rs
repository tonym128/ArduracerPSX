//! Title screen input state machine (Start / Cross edge trigger).
//!
//! Deliberately free of hardware dependencies: takes held/pressed button
//! state as plain booleans and returns whether the player confirmed entry
//! to the main menu. Tested host-side in `tools/test_ui`.

/// Input and edge-detection state for the title / attract screen.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct TitleInput {
    pub timer: u32,
    /// Whether Start or Cross was held on the previous frame.
    /// Initialized to `true` to require button release before triggering,
    /// preventing a button held across video playback from immediately
    /// dismissing the title screen.
    pub prev_held: bool,
}

impl Default for TitleInput {
    fn default() -> Self {
        Self::new()
    }
}

impl TitleInput {
    pub const fn new() -> Self {
        TitleInput {
            timer: 0,
            prev_held: true,
        }
    }

    /// Updates title screen state given raw boolean hold states for START and CROSS.
    /// Returns true on fresh edge-triggered press.
    pub fn update(&mut self, start_held: bool, cross_held: bool) -> bool {
        self.timer = self.timer.wrapping_add(1);
        let held = start_held || cross_held;
        let pressed = held && !self.prev_held;
        self.prev_held = held;
        pressed
    }
}
