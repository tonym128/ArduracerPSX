//! Game State Machine for Arduracer PSX.
//!
//! Manages flow between Title Screen, Main Menu, Track Select Carousel,
//! Tuning Garage, Active Racing, and Race Results Ceremonies.

use crate::ui::{MainMenu, ResultsScreen, TitleScreen, TrackSelectScreen, TuningScreen};
use arduracer_core::championship::ChampionshipSession;
use arduracer_core::tuning::CarTuning;

#[derive(Default)]
pub enum GameState {
    #[default]
    Title,
    MainMenu,
    TrackSelect,
    Garage,
    Racing,
    Results,
}

pub struct StateManager {
    pub current: GameState,
    pub title: TitleScreen,
    pub menu: MainMenu,
    pub track_select: TrackSelectScreen,
    pub garage: TuningScreen,
    pub results: Option<ResultsScreen>,
    pub championship: Option<ChampionshipSession>,
}

impl Default for StateManager {
    fn default() -> Self {
        Self::new()
    }
}

impl StateManager {
    pub fn new() -> Self {
        StateManager {
            current: GameState::Title,
            title: TitleScreen::new(),
            menu: MainMenu::new(),
            track_select: TrackSelectScreen::new(),
            garage: TuningScreen::new(CarTuning::default()),
            results: None,
            championship: None,
        }
    }
}
