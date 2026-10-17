//! Pause menu input state machine (DualShock `Start`).
//!
//! Deliberately free of hardware dependencies: it takes held/held-last button
//! state as plain booleans and returns what the race loop should do. The
//! `psx-pad` adapter and the GPU drawing live in `pause.rs`, and
//! `tools/test_ui` exercises this file directly.
//!
//! It also carries the two inline graphics rows (texture format, framerate)
//! so the player can change them mid-race without leaving the pause menu; the
//! same [`SettingsValues`] type is shared with the standalone settings screen.
//!
//! Why the split: this logic used to read and write a shared `prev_buttons`
//! mask that the race loop also wrote to. Because the loop updated the mask
//! before handing the pad to [`PauseMenu::update`], the menu always compared
//! the current frame against itself, so no button ever looked *newly pressed*.
//! Entering the pause menu was unrecoverable without a power cycle. Owning all
//! of the edge state here makes that class of bug impossible: there is exactly
//! one writer, and the caller cannot reach in and clobber it.

/// The graphics values both menus edit.
///
/// Defined here rather than in `settings_input.rs` because `pause_input.rs` is
/// the file `tools/test_ui` includes first, and a re-export would otherwise
/// give the two include paths two distinct types for the same struct.
#[derive(Copy, Clone, PartialEq, Eq, Debug, Default)]
pub struct SettingsValues {
    /// 0 = auto, 1 = SVG, 2 = QOI, 3 = JPEG, 4 = TIM, 5 = TIM4, 6 = TIM8. Totality lives in
    /// `arduracer_core::texture_format::TextureFormat::from_saved`.
    pub texture_format: u8,
    /// 0 = 60 FPS, 1 = 30 FPS.
    pub framerate: u8,
    /// 0 = Behind (Mode 7 Chase), 1 = Overhead (Top-down).
    pub camera_view: u8,
    /// 0 = Off, 1 = CD-DA, 2 = XA.
    ///
    /// Not merely a mute. The TIM texture streamer and CD-DA both want the one
    /// physical drive, and `cd_fs::read_sectors` pauses playback around every
    /// read, so with music on and TIM selected the player hears the drive
    /// changing hands several times a second. Turning the music off releases
    /// the drive, which is the only fix -- nothing on the audio side can hide
    /// it.
    pub cd_audio: u8,
    /// 0 = Off, 1 = Simple FPS, 2 = Simple Graphing FPS, 3 = All Metrics.
    pub debug_metrics: u8,
    /// 0 = Classic Arcade, 1 = Modern Triggers, 2 = Dual Stick.
    pub input_profile: u8,
}

/// What the player chose from the pause menu.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum PauseChoice {
    /// Nothing was confirmed this frame.
    #[default]
    None,
    Resume,
    RestartRace,
    /// Graphics settings changed while paused; the caller applies and persists.
    Graphics(SettingsValues),
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
    /// Shoulder and D-pad left/right, used only by the inline value rows.
    pub left: bool,
    pub right: bool,
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
    /// Working copy of the graphics settings, committed by reporting a
    /// [`PauseChoice::Graphics`].
    values: SettingsValues,
}

/// Entries, in display order. The settings rows sit between the race
/// actions and the quit, so the destructive option stays last.
const ITEM_RESUME: u8 = 0;
const ITEM_RESTART: u8 = 1;
const ITEM_TEXTURE: u8 = 2;
const ITEM_FRAMERATE: u8 = 3;
const ITEM_VIEW: u8 = 4;
const ITEM_CD_AUDIO: u8 = 5;
const ITEM_METRICS: u8 = 6;
const ITEM_CONTROLS: u8 = 7;
const ITEM_QUIT: u8 = 8;
const ITEM_COUNT: u8 = 9;

/// Highest legal `texture_format` value; see `TextureFormat::from_saved`.
pub const MAX_TEXTURE_FORMAT: u8 = 6;

/// Whether `v` names a selectable texture format.
///
/// Both menus need this: they cycle the byte directly rather than going through
/// the enum, so without a shared ceiling each would keep its own stale copy of
/// the list. It already went wrong once -- the ceiling was 3 when TIM took byte
/// 4 -- and the symptom is a format that saves fine and then cannot be reached
/// again, or a cycle that skips one.
pub const fn texture_format_is_selectable(v: u8) -> bool {
    v <= MAX_TEXTURE_FORMAT
}

impl Default for PauseMenu {
    fn default() -> Self {
        Self::new()
    }
}

impl PauseMenu {
    pub const fn new() -> Self {
        PauseMenu {
            selected_idx: 0,
            values: SettingsValues {
                texture_format: 0,
                framerate: 0,
                camera_view: 0,
                cd_audio: 1,
                debug_metrics: 0,
                input_profile: 0,
            },
            prev: PauseInput {
                start: false,
                select: false,
                cross: false,
                circle: false,
                up: false,
                down: false,
                left: false,
                right: false,
            },
        }
    }

    /// Number of entries in the menu.
    pub const fn item_count() -> u8 {
        ITEM_COUNT
    }

    /// The settings copy the menu is currently editing, for rendering.
    pub const fn values(&self) -> SettingsValues {
        self.values
    }

    /// Steps the selected row's value, wrapping. Only the settings rows
    /// have values; the action rows report `false`.
    fn cycle_value(&mut self, dir: i32) -> bool {
        let (cur, span) = match self.selected_idx {
            ITEM_TEXTURE => (
                self.values.texture_format as i32,
                MAX_TEXTURE_FORMAT as i32 + 1,
            ),
            ITEM_FRAMERATE => (self.values.framerate as i32, 2),
            ITEM_VIEW => (self.values.camera_view as i32, 3),
            ITEM_CD_AUDIO => (self.values.cd_audio as i32, 3),
            ITEM_METRICS => (self.values.debug_metrics as i32, 4),
            ITEM_CONTROLS => (self.values.input_profile as i32, 3),
            _ => return false,
        };
        let next = (cur + dir).rem_euclid(span) as u8;
        match self.selected_idx {
            ITEM_TEXTURE => self.values.texture_format = next,
            ITEM_FRAMERATE => self.values.framerate = next,
            ITEM_VIEW => self.values.camera_view = next,
            ITEM_CD_AUDIO => self.values.cd_audio = next,
            ITEM_METRICS => self.values.debug_metrics = next,
            ITEM_CONTROLS => self.values.input_profile = next,
            _ => return false,
        }
        next as i32 != cur
    }

    /// Handles navigation and confirms a selection.
    ///
    /// Call exactly once per rendered frame, whether or not the menu is open:
    /// skipping a frame leaves a stale [`PauseInput`] behind and the next press
    /// reads as a hold instead of an edge.
    pub fn update(&mut self, input: PauseInput) -> PauseFrame {
        self.update_with_values(input, self.values)
    }

    /// Handles navigation, inline value editing, and confirmation.
    ///
    /// `values` are the settings currently in force; the menu edits a copy and
    /// reports a [`PauseChoice::Graphics`] only when a value actually moved, so
    /// holding a shoulder button does not rewrite the save on every frame.
    pub fn update_with_values(&mut self, input: PauseInput, values: SettingsValues) -> PauseFrame {
        let prev = self.prev;
        self.prev = input;
        self.values = values;

        // Edge detect against the previous frame, not against `input` itself:
        // a button held for many frames is one press, not one per frame.
        let up_pressed = input.up && !prev.up;
        let down_pressed = input.down && !prev.down;
        let start_pressed = input.start && !prev.start;
        let select_pressed = input.select && !prev.select;
        let cross_pressed = input.cross && !prev.cross;
        let circle_pressed = input.circle && !prev.circle;
        let left_pressed = input.left && !prev.left;
        let right_pressed = input.right && !prev.right;

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

        // Inline value rows. Left/Right edit in place and never activate the row,
        // so a player can adjust a setting without leaving the race menu.
        let mut changed = false;
        if left_pressed || right_pressed {
            let dir: i32 = if right_pressed { 1 } else { -1 };
            changed = self.cycle_value(dir);
        }

        let choice = if start_pressed || circle_pressed {
            PauseChoice::Resume
        } else if cross_pressed {
            match self.selected_idx {
                ITEM_RESUME => PauseChoice::Resume,
                ITEM_RESTART => PauseChoice::RestartRace,
                ITEM_TEXTURE | ITEM_FRAMERATE | ITEM_VIEW | ITEM_CD_AUDIO | ITEM_METRICS => {
                    if changed {
                        PauseChoice::Graphics(self.values)
                    } else {
                        PauseChoice::None
                    }
                }
                ITEM_QUIT => PauseChoice::QuitToMenu,
                _ => PauseChoice::None,
            }
        } else if changed {
            PauseChoice::Graphics(self.values)
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
            left: true,
            right: true,
        };
    }
}
