//! Host-testable input state machine for the graphics settings.
//!
//! Deliberately split from rendering, exactly like `pause_input.rs`: the
//! button-edge ownership and wrapping rules are the part that can be wrong in
//! a way no screenshot catches, so they live where `tools/test_ui` can drive
//! them directly.
//!
//! Two independent settings are edited: the circuit texture format and the
//! framerate. Up/Down picks the row, Left/Right cycles that row's value, Cross
//! confirms and exits. Both wrap, because a 2-row menu is too small to warrant
//! open-ended behaviour.

/// Rows, in display order.
pub const ROW_COUNT: u8 = 3;
const ROW_TEXTURE: u8 = 0;
const ROW_FRAMERATE: u8 = 1;
const ROW_VIEW: u8 = 2;

/// Raw button state for one frame.
#[derive(Copy, Clone, Default)]
pub struct SettingsInput {
    pub cross: bool,
    pub circle: bool,
    pub triangle: bool,
    pub up: bool,
    pub down: bool,
    pub left: bool,
    pub right: bool,
}

// The value type lives in `pause_input.rs` so both menus share one definition
// even when `tools/test_ui` includes the two files by path in either order.
pub use super::pause_input::SettingsValues;

/// What the caller must do after an update.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum SettingsChoice {
    /// Nothing happened; keep drawing the screen.
    None,
    /// A value changed and must be applied and persisted.
    Changed(SettingsValues),
    /// The screen should close.
    Exit,
}

/// Highest legal `texture_format` value; see `TextureFormat::from_saved`.
const MAX_TEXTURE_FORMAT: u8 = 3;

pub struct SettingsMenu {
    pub selected_row: u8,
    values: SettingsValues,
    prev: SettingsInput,
}

impl SettingsMenu {
    /// Builds the screen over a starting set of values.
    pub const fn new(values: SettingsValues) -> Self {
        Self {
            selected_row: 0,
            values,
            prev: SettingsInput {
                cross: false,
                circle: false,
                triangle: false,
                up: false,
                down: false,
                left: false,
                right: false,
            },
        }
    }

    /// The values currently on screen.
    pub const fn values(&self) -> SettingsValues {
        self.values
    }

    /// Advances the machine by one frame of input.
    pub fn update(&mut self, input: SettingsInput) -> SettingsChoice {
        let up = input.up && !self.prev.up;
        let down = input.down && !self.prev.down;
        let left = input.left && !self.prev.left;
        let right = input.right && !self.prev.right;
        let cross = input.cross && !self.prev.cross;
        let circle = input.circle && !self.prev.circle;
        let triangle = input.triangle && !self.prev.triangle;

        self.prev = input;

        // Row navigation is only meaningful with more than one row, and this
        // screen is exactly the case where that is easy to forget: a stale
        // selection would edit the wrong setting with no visible error.
        if up {
            self.selected_row = wrap(self.selected_row, ROW_COUNT, true);
        } else if down {
            self.selected_row = wrap(self.selected_row, ROW_COUNT, false);
        }

        let mut changed = false;
        if left {
            changed |= self.cycle(-1);
        } else if right {
            changed |= self.cycle(1);
        }

        // Cross and Circle both exit, matching the pause menu's convention that
        // the right-hand face button backs out.
        if cross || circle || triangle {
            if changed {
                SettingsChoice::Changed(self.values)
            } else {
                SettingsChoice::Exit
            }
        } else if changed {
            SettingsChoice::Changed(self.values)
        } else {
            SettingsChoice::None
        }
    }

    /// Steps the selected row's value, wrapping in both directions.
    fn cycle(&mut self, dir: i32) -> bool {
        let span = match self.selected_row {
            ROW_TEXTURE => MAX_TEXTURE_FORMAT as i32 + 1,
            ROW_FRAMERATE => 2,
            ROW_VIEW => 3,
            _ => return false,
        };
        let cur = match self.selected_row {
            ROW_TEXTURE => self.values.texture_format as i32,
            ROW_FRAMERATE => self.values.framerate as i32,
            ROW_VIEW => self.values.camera_view as i32,
            _ => return false,
        };
        let next = (cur + dir).rem_euclid(span) as u8;
        match self.selected_row {
            ROW_TEXTURE => self.values.texture_format = next,
            ROW_FRAMERATE => self.values.framerate = next,
            ROW_VIEW => self.values.camera_view = next,
            _ => return false,
        }
        next != cur as u8
    }

    /// Forces every button to read as *held*.
    ///
    /// Called when the screen is entered. Arming as released would be exactly
    /// backwards: the edge detector compares against `prev`, so a button the
    /// player is still holding to leave the previous screen would read as
    /// freshly pressed and immediately change a setting or close this one.
    /// Arming as held means the player must release before it can act again,
    /// which is the same contract `PauseMenu::arm_for_race_start` uses.
    pub fn arm_for_entry(&mut self) {
        self.prev = SettingsInput {
            cross: true,
            circle: true,
            triangle: true,
            up: true,
            down: true,
            left: true,
            right: true,
        };
    }

    /// Replaces the on-screen values without reporting a change.
    pub fn set_values(&mut self, values: SettingsValues) {
        self.values = values;
    }
}

/// Wraps an index within `count`, where `up` selects the decrementing direction.
fn wrap(idx: u8, count: u8, up: bool) -> u8 {
    if count == 0 {
        return 0;
    }
    if up {
        if idx == 0 {
            count - 1
        } else {
            idx - 1
        }
    } else if idx + 1 >= count {
        0
    } else {
        idx + 1
    }
}
