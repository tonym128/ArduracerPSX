//! Pause menu input state machine (DualShock `Start`).
//!
//! Deliberately free of hardware dependencies: it takes held/held-last button
//! state as plain booleans and returns what the race loop should do. The
//! `psx-pad` adapter and the GPU drawing live in `pause.rs`, and
//! `tools/test_ui` exercises this file directly.
//!
//! Why the split: this logic used to read and write a shared `prev_buttons`
//! mask that the race loop also wrote to. Because the loop updated the mask
//! before handing the pad to [`PauseMenu::update`], the menu always compared
//! the current frame against itself, so no button ever looked *newly pressed*.
//! Entering the pause menu was unrecoverable without a power cycle. Owning all
//! of the edge state here makes that class of bug impossible: there is exactly
//! one writer, and the caller cannot reach in and clobber it.

/// What the player chose from the pause menu.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum PauseChoice {
    /// Nothing was confirmed this frame.
    #[default]
    None,
    Resume,
    RestartRace,
    QuitToMenu,
}

/// Buttons the pause menu cares about, for one frame.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct PauseInput {
    pub start: bool,
    pub select: bool,
    pub cross: bool,
    pub circle: bool,
    pub up: bool,
    pub down: bool,
}

/// Everything one racing frame needs to know about pause input.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct PauseFrame {
    /// `Start` was newly pressed: toggle the pause veil.
    pub start_pressed: bool,
    /// `Select` was newly pressed: toggle the HUD.
    pub select_pressed: bool,
    /// A menu entry was confirmed. Only meaningful on frames where the menu was
    /// *already* open -- the frame that opens it must not also confirm.
    pub choice: PauseChoice,
}

/// Menu highlight and edge-detection state.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct PauseMenu {
    pub selected_idx: u8,
    /// Buttons held on the previous frame; the sole edge-detection state.
    prev: PauseInput,
}

const ITEM_COUNT: u8 = 3;

impl Default for PauseMenu {
    fn default() -> Self {
        Self::new()
    }
}

impl PauseMenu {
    pub const fn new() -> Self {
        PauseMenu {
            selected_idx: 0,
            prev: PauseInput {
                start: false,
                select: false,
                cross: false,
                circle: false,
                up: false,
                down: false,
            },
        }
    }

    /// Number of entries in the menu.
    pub const fn item_count() -> u8 {
        ITEM_COUNT
    }

    /// Handles navigation and confirms a selection.
    ///
    /// Call exactly once per rendered frame, whether or not the menu is open:
    /// skipping a frame leaves a stale [`PauseInput`] behind and the next press
    /// reads as a hold instead of an edge.
    pub fn update(&mut self, input: PauseInput) -> PauseFrame {
        let prev = self.prev;
        self.prev = input;

        // Edge detect against the previous frame, not against `input` itself:
        // a button held for many frames is one press, not one per frame.
        let up_pressed = input.up && !prev.up;
        let down_pressed = input.down && !prev.down;
        let start_pressed = input.start && !prev.start;
        let select_pressed = input.select && !prev.select;
        let cross_pressed = input.cross && !prev.cross;
        let circle_pressed = input.circle && !prev.circle;

        if up_pressed {
            self.selected_idx = if self.selected_idx == 0 {
                ITEM_COUNT - 1
            } else {
                self.selected_idx - 1
            };
        } else if down_pressed {
            self.selected_idx = if self.selected_idx + 1 >= ITEM_COUNT {
                0
            } else {
                self.selected_idx + 1
            };
        }

        let choice = if start_pressed || circle_pressed {
            PauseChoice::Resume
        } else if cross_pressed {
            match self.selected_idx {
                0 => PauseChoice::Resume,
                1 => PauseChoice::RestartRace,
                _ => PauseChoice::QuitToMenu,
            }
        } else {
            PauseChoice::None
        };

        PauseFrame {
            start_pressed,
            select_pressed,
            choice,
        }
    }

    /// Adopts `input` as the edge baseline without acting on it.
    ///
    /// Call on transitions that skip frames -- reloading a track, leaving the
    /// race -- so a button the player is still holding across the change is not
    /// seen as a fresh press. (Resetting the baseline to "nothing held" would
    /// do the opposite: the first frame back would read as a press and confirm
    /// whatever the highlight happened to be on.)
    pub fn sync_edges(&mut self, input: PauseInput) {
        self.prev = input;
    }

    /// Arms the menu for a race that is about to start.
    ///
    /// `Start` is a confirm button on the title, main menu and track select
    /// screens in sequence, so a player who holds it while navigating loads the
    /// race with the button still down. The baseline starts at "nothing held",
    /// so the very first racing frame reads that as a fresh press and the pause
    /// veil opens on frame 1 with the music stopped (TASK-1209).
    ///
    /// There is no pad to sample here, so treat *every* button as held: the
    /// player then has to release before any of them can re-trigger. That is
    /// the same contract `MainMenu` and `TrackSelectScreen` use for
    /// `prev_confirm`.
    pub fn arm_for_race_start(&mut self) {
        self.prev = PauseInput {
            start: true,
            select: true,
            cross: true,
            circle: true,
            up: true,
            down: true,
        };
    }
}
