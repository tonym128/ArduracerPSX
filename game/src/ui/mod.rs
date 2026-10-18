//! User Interface Subsystem for Arduracer PSX.
//!
//! Organizes arcade menus, tuning screens, track carousels, and ceremony results.

pub mod city_select;
pub mod font;
pub mod grand_prix_setup;
pub mod menu;
pub mod pause;
pub mod pause_input;
pub mod results;
pub mod settings;
pub mod settings_input;
pub mod stage_table;
pub mod title;
mod title_background;
pub mod title_input;
pub mod track_editor;
pub mod track_select;
pub mod tuning_input;
pub mod tuning_screen;
pub mod victory;

pub use city_select::{CityInfo, CitySelectScreen, CITIES};
pub use grand_prix_setup::{GrandPrixSetupAction, GrandPrixSetupScreen};
pub use menu::{MainMenu, MenuItem};
pub use pause::PauseMenu;
pub use pause_input::{PauseChoice, PauseFrame, PauseInput};
pub use results::{ResultsAction, ResultsScreen};
pub use settings::SettingsScreen;
pub use settings_input::{SettingsChoice, SettingsInput, SettingsMenu, SettingsValues};
pub use stage_table::{StageFilterTab, StageTableAction, StageTableScreen};
pub use title::TitleScreen;
pub use title_input::TitleInput;
pub use track_editor::{TrackEditorAction, TrackEditorScreen};
pub use track_select::TrackSelectScreen;
pub use tuning_input::{Reject, TuningInput, TuningMenu};
pub use tuning_screen::TuningScreen;
pub use victory::VictoryScreen;
