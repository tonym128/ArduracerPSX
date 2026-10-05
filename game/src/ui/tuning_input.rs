//! Tuning garage input state machine and point-budget rules.
//!
//! Deliberately free of hardware dependencies: it takes held/held-last button
//! state as plain booleans and returns what the garage should do. The `psx-pad`
//! adapter and the GPU drawing live in `tuning_screen.rs`, and `tools/test_ui`
//! exercises this file directly.
//!
//! Why the split: this logic used to inline `0..=10` as literal slider bounds
//! while `CarTuning::is_valid` -- the gate `memcard::store_tuning` applies --
//! requires `MIN_SLIDER..=MAX_SLIDER` (1..=7). A player who pushed a slider
//! past 7, or down to 0, built a setup the garage would accept, the exit path
//! would happily "save", and the card write would then *drop* it: `flush()`
//! still ran, `store_tuning` never set `is_dirty`, and so `flush` returned
//! early with no error banner anywhere. The tuning was lost silently.
//!
//! Owning the bounds here as the same exported constants `is_valid` uses makes
//! the two impossible to disagree, and enforcing the budget on *both* edges
//! keeps every reachable state inside `is_valid` by construction.

use arduracer_core::tuning::{CarTuning, MAX_SLIDER, MIN_SLIDER, TOTAL_POINTS};

/// The five tuning axes, in the order the garage displays them.
pub const SLIDER_COUNT: u8 = 5;

/// A single slider's identifier, so callers need not index raw offsets.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Slider {
    TopSpeed = 0,
    Acceleration = 1,
    Handling = 2,
    DriftStability = 3,
    Gearing = 4,
}

impl Slider {
    const ALL: [Slider; SLIDER_COUNT as usize] = [
        Slider::TopSpeed,
        Slider::Acceleration,
        Slider::Handling,
        Slider::DriftStability,
        Slider::Gearing,
    ];

    pub const fn index(self) -> u8 {
        self as u8
    }

    pub const fn count() -> u8 {
        SLIDER_COUNT
    }

    /// Reads the slider's current value out of a setup.
    pub const fn value_of(self, tuning: &CarTuning) -> u8 {
        match self {
            Slider::TopSpeed => tuning.top_speed,
            Slider::Acceleration => tuning.acceleration,
            Slider::Handling => tuning.handling,
            Slider::DriftStability => tuning.drift_stability,
            Slider::Gearing => tuning.gearing,
        }
    }

    /// Writes the slider's value into a setup.
    pub fn set_value(self, tuning: &mut CarTuning, value: u8) {
        match self {
            Slider::TopSpeed => tuning.top_speed = value,
            Slider::Acceleration => tuning.acceleration = value,
            Slider::Handling => tuning.handling = value,
            Slider::DriftStability => tuning.drift_stability = value,
            Slider::Gearing => tuning.gearing = value,
        }
    }

    /// The next slider, wrapping at the ends.
    pub const fn next(self) -> Slider {
        Self::ALL[(self.index() as usize + 1) % SLIDER_COUNT as usize]
    }

    /// The previous slider, wrapping at the ends.
    pub const fn prev(self) -> Slider {
        Self::ALL[(self.index() as usize + SLIDER_COUNT as usize - 1) % SLIDER_COUNT as usize]
    }
}

/// Why a requested adjustment did not happen.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Reject {
    /// The slider is already at `MAX_SLIDER`.
    AtMax,
    /// The slider is already at `MIN_SLIDER`.
    AtMin,
    /// The whole 20-point budget is already spent.
    NoPointsLeft,
    /// The player tried to leave with points unallocated. Distinct from
    /// `NoPointsLeft`: this is a refusal to *save*, not to adjust.
    Unbalanced,
}

/// Number of garage presets a player can choose between.
pub const SLOT_COUNT: usize = arduracer_core::save::TOTAL_TUNING_SLOTS;

/// Buttons the garage cares about, for one frame.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct TuningInput {
    pub up: bool,
    pub down: bool,
    pub left: bool,
    pub right: bool,
    pub cross: bool,
    pub circle: bool,
    pub start: bool,
    pub triangle: bool,
    /// Shoulder buttons cycle presets. Separate from the D-pad because the
    /// sliders already own up/down/left/right.
    pub l1: bool,
    pub r1: bool,
}

/// Everything one garage frame needs to know.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct TuningFrame {
    /// The player asked to save and leave.
    pub exited: bool,
    /// A `Triangle` press is waiting on a confirmation.
    pub reset_requested: bool,
    /// A pending reset was confirmed by `Cross` this frame.
    pub reset_confirmed: bool,
    /// A pending reset was cancelled by `Circle` this frame.
    pub reset_cancelled: bool,
    /// An adjustment was refused, for the on-screen banner.
    pub rejected: Option<Reject>,
}

/// Garage selection, pending-confirmation and edge-detection state.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct TuningMenu {
    pub tuning: CarTuning,
    pub selected: Slider,
    /// A `Triangle` press is armed and awaiting `Cross`.
    pub reset_pending: bool,
    /// Which of the three presets is loaded and will be saved.
    pub slot: usize,
    /// Whether the current slot has been written since the last load.
    pub slot_dirty: bool,
    /// Buttons held on the previous frame; the sole edge-detection state.
    prev: TuningInput,
}

impl TuningMenu {
    pub const fn new(tuning: CarTuning) -> Self {
        // Cross/Circle/Start start armed so they must be released before they
        // can act: all three navigate or confirm on the screens leading here.
        TuningMenu {
            tuning,
            selected: Slider::TopSpeed,
            reset_pending: false,
            slot: 0,
            slot_dirty: false,
            prev: TuningInput {
                up: false,
                down: false,
                left: false,
                right: false,
                cross: true,
                circle: true,
                start: true,
                triangle: false,
                l1: true,
                r1: true,
            },
        }
    }

    /// Points still unallocated.
    pub fn points_remaining(&self) -> u8 {
        TOTAL_POINTS.saturating_sub(self.tuning.total_points())
    }

    /// Applies one point to `slider`, respecting both the slider bounds and the
    /// shared budget.
    ///
    /// Returns the refusal reason so the UI can say *why* nothing happened,
    /// rather than silently ignoring the press.
    pub fn increment(&mut self, slider: Slider) -> Result<(), Reject> {
        let current = slider.value_of(&self.tuning);
        if current >= MAX_SLIDER {
            return Err(Reject::AtMax);
        }
        if self.tuning.total_points() >= TOTAL_POINTS {
            return Err(Reject::NoPointsLeft);
        }
        slider.set_value(&mut self.tuning, current + 1);
        Ok(())
    }

    /// Removes one point from `slider`, respecting the slider floor.
    ///
    /// No budget guard here, deliberately: the budget is a *target*, not a
    /// ceiling the player must stay under. Freeing points before re-spending
    /// them is the whole mechanic ("POINTS REMAINING" counts down as the setup
    /// is filled in), and `is_valid` wants the total to land exactly on
    /// `TOTAL_POINTS` -- so the intermediate states below budget are legitimate
    /// to pass through. [`Self::can_save`] is what gates the exit.
    pub fn decrement(&mut self, slider: Slider) -> Result<(), Reject> {
        let current = slider.value_of(&self.tuning);
        if current <= MIN_SLIDER {
            return Err(Reject::AtMin);
        }
        slider.set_value(&mut self.tuning, current - 1);
        Ok(())
    }

    /// Moves to another preset, wrapping at the ends.
    ///
    /// `active_tuning_slot` was serialised and checksummed but never written, so
    /// the garage could only ever load and save preset 0 -- the other two were
    /// unreachable (TASK-1217).
    pub fn cycle_slot(&mut self, forward: bool) {
        if forward {
            self.slot = (self.slot + 1) % SLOT_COUNT;
        } else {
            self.slot = (self.slot + SLOT_COUNT - 1) % SLOT_COUNT;
        }
    }

    /// Adopts the setup stored in `slot` and selects it.
    pub fn load_slot(&mut self, slot: usize, tuning: CarTuning) {
        self.slot = slot.min(SLOT_COUNT - 1);
        self.tuning = tuning;
        self.slot_dirty = false;
    }

    /// Records that the exit path should persist the current slot.
    pub fn mark_dirty(&mut self) {
        self.slot_dirty = true;
    }

    /// Whether the setup may be written to the card.
    ///
    /// This is `CarTuning::is_valid` by another name, stated where the decision
    /// belongs. The garage lets the player leave the screen with an unbalanced
    /// budget; what it must not do is hand that setup to `store_tuning`, which
    /// would drop it without a word.
    pub fn can_save(&self) -> bool {
        self.tuning.is_valid()
    }

    /// Handles one frame of garage input.
    ///
    /// `Cross` and `Circle` are edge-triggered with an armed baseline, because
    /// `Cross` confirms the reset prompt and exits the screen, and both are
    /// used to navigate the screens immediately before this one.
    pub fn update(&mut self, input: TuningInput) -> TuningFrame {
        let prev = self.prev;
        self.prev = input;

        let edge = |now: bool, before: bool| now && !before;
        let up = edge(input.up, prev.up);
        let down = edge(input.down, prev.down);
        let left = edge(input.left, prev.left);
        let right = edge(input.right, prev.right);
        let triangle = edge(input.triangle, prev.triangle);
        let l1 = edge(input.l1, prev.l1);
        let r1 = edge(input.r1, prev.r1);
        let cross = edge(input.cross, prev.cross);
        let circle = edge(input.circle, prev.circle);
        let start = edge(input.start, prev.start);

        let mut frame = TuningFrame::default();

        if self.reset_pending {
            // While the prompt is up, only Cross (confirm) and Circle (cancel)
            // mean anything; the D-pad must not move sliders underneath it.
            if cross {
                self.tuning = CarTuning::default();
                self.reset_pending = false;
                frame.reset_confirmed = true;
            } else if circle {
                self.reset_pending = false;
                frame.reset_cancelled = true;
            } else if start {
                frame.exited = true;
            }
            return frame;
        }

        if up {
            self.selected = self.selected.prev();
        } else if down {
            self.selected = self.selected.next();
        }

        if right {
            frame.rejected = self.increment(self.selected).err();
        } else if left {
            frame.rejected = self.decrement(self.selected).err();
        }

        // Triangle arms a confirmation instead of wiping the setup outright.
        if r1 {
            self.cycle_slot(true);
        } else if l1 {
            self.cycle_slot(false);
        }

        if triangle {
            self.reset_pending = true;
            frame.reset_requested = true;
        }

        if circle || start {
            // Refuse to leave with the budget unbalanced, and say why. The card
            // write would reject this setup anyway; catching it here turns a
            // silent discard into a visible explanation.
            if self.can_save() {
                frame.exited = true;
            } else {
                frame.rejected = Some(Reject::Unbalanced);
            }
        }

        frame
    }

    /// Adopts `input` as the edge baseline without acting on it.
    ///
    /// Call when entering the garage so a `Cross` or `Start` still held from
    /// the main menu does not immediately save-and-exit.
    pub fn sync_edges(&mut self, input: TuningInput) {
        self.prev = input;
    }
}
