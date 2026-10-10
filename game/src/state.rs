//! Game State Machine for Arduracer PSX.
//!
//! Manages flow between Title Screen, Main Menu, Track Select Carousel,
//! Tuning Garage, Active Racing, and Race Results Ceremonies.

use crate::ui::{
    CitySelectScreen, GrandPrixSetupScreen, MainMenu, ResultsScreen, TitleScreen,
    TrackSelectScreen, TuningScreen, VictoryScreen,
};
use arduracer_core::championship::ChampionshipSession;
use arduracer_core::tuning::CarTuning;

#[derive(Default, Copy, Clone, Debug, PartialEq, Eq)]
pub enum GameState {
    #[default]
    Title,
    MainMenu,
    GrandPrixSetup,
    TrackSelect,
    CitySelect,
    Garage,
    Racing,
    Results,
    Victory,
}

pub struct StateManager {
    pub current: GameState,
    pub title: TitleScreen,
    pub menu: MainMenu,
    pub gp_setup: GrandPrixSetupScreen,
    pub track_select: TrackSelectScreen,
    pub city_select: CitySelectScreen,
    pub garage: TuningScreen,
    pub results: Option<ResultsScreen>,
    pub victory: Option<VictoryScreen>,
    pub championship: Option<ChampionshipSession>,
    pub champ_stage_backup: Option<ChampionshipSession>,
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
            gp_setup: GrandPrixSetupScreen::new(false),
            track_select: TrackSelectScreen::new(),
            city_select: CitySelectScreen::new(),
            garage: TuningScreen::new(CarTuning::default()),
            results: None,
            victory: None,
            championship: None,
            champ_stage_backup: None,
        }
    }
}
