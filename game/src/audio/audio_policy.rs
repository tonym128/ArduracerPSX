//! Audio policy: which voices are allowed to sound, and when.
//!
//! Deliberately free of hardware dependencies: it decides *what* should be
//! audible and returns that decision, while `engine_audio.rs`, `sfx.rs` and
//! `spu.rs` own the SPU register writes. `tools/test_audio` exercises this
//! file directly.
//!
//! Why the split: the engine voice is a continuous loop that is keyed on at
//! boot and whose volume register is only ever rewritten from
//! `AudioSystem::tick`. `tick` is reached from the race arm of the state match
//! and nowhere else, so every other screen -- and the pause veil, which
//! `continue`s before the tick -- inherited whatever volume the last racing
//! frame happened to leave behind. That is why the engine droned through the
//! results screen and under the menu music.
//!
//! Owning the decision here rather than sprinkling `silence()` calls across the
//! state machine makes the failure mode a no-op instead of a drone: a screen
//! that forgets to opt back in stays quiet rather than buzzing.

/// A UI sound cue derived from a button press.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum UiSound {
    /// The highlight moved (D-pad in any direction).
    Move,
    /// An entry was confirmed.
    Confirm,
    /// The screen was dismissed without confirming.
    Back,
}

/// The button bits the policy needs, mirroring `psx_pad::button`.
pub const BTN_UP: u16 = 1 << 0;
pub const BTN_DOWN: u16 = 1 << 1;
pub const BTN_LEFT: u16 = 1 << 2;
pub const BTN_RIGHT: u16 = 1 << 3;
pub const BTN_CROSS: u16 = 1 << 4;
pub const BTN_CIRCLE: u16 = 1 << 5;
pub const BTN_START: u16 = 1 << 8;
pub const BTN_SELECT: u16 = 1 << 9;

const DIRECTION_MASK: u16 = BTN_UP | BTN_DOWN | BTN_LEFT | BTN_RIGHT;
const CONFIRM_MASK: u16 = BTN_CROSS | BTN_START;
const CANCEL_MASK: u16 = BTN_CIRCLE;

/// Which voices the game currently wants to hear.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct AudioPolicy {
    /// Set while a stage is loaded, cleared on leaving it.
    in_race: bool,
    /// Set while the pause veil is up.
    paused: bool,
    /// Last frame's raw button mask; the sole edge-detection state.
    prev_buttons: u16,
}

impl AudioPolicy {
    pub const fn new() -> Self {
        AudioPolicy {
            // Silent until a race starts: nothing is audible during boot, the
            // intro, or the title screen.
            in_race: false,
            paused: false,
            prev_buttons: 0,
        }
    }

    /// Whether the engine synth and race SFX may sound this frame.
    ///
    /// The pause veil vetoes independently of the race state, so a stage that
    /// loads while the veil is up stays silent.
    pub const fn racing_voices(&self) -> bool {
        self.in_race && !self.paused
    }

    /// Arms the race voices, e.g. when a stage loads or the race restarts.
    pub fn enter_race(&mut self) {
        self.in_race = true;
    }

    /// Disarms the race voices. Called for the pause veil, the results screen,
    /// and every menu.
    pub fn silence_race_voices(&mut self) {
        self.in_race = false;
    }

    /// Applies the pause veil. Pausing silences the race voices immediately;
    /// unpausing re-arms them.
    pub fn set_paused(&mut self, paused: bool) {
        self.paused = paused;
    }

    /// Feeds one frame of button state and returns the UI cue to play.
    ///
    /// Fires at most one cue per frame, checked in descending priority: a
    /// confirm or cancel wins over a move, because on the frame the player
    /// presses CROSS the highlight did not also move. Held buttons do not
    /// retrigger -- a cue needs a genuine press edge.
    pub fn ui_sound(&mut self, buttons: u16) -> Option<UiSound> {
        let pressed = buttons & !self.prev_buttons;
        self.prev_buttons = buttons;

        if pressed & CONFIRM_MASK != 0 {
            Some(UiSound::Confirm)
        } else if pressed & CANCEL_MASK != 0 {
            Some(UiSound::Back)
        } else if pressed & DIRECTION_MASK != 0 {
            Some(UiSound::Move)
        } else {
            None
        }
    }

    /// Adopts the buttons held right now as already-seen.
    ///
    /// Used after a transition that skipped frames (a restart, say) so the same
    /// press is not re-read as a fresh cue against the new screen.
    pub fn sync_edges(&mut self, buttons: u16) {
        self.prev_buttons = buttons;
    }
}
