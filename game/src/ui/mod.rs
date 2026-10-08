//! User Interface Subsystem for Arduracer PSX.
//!
//! Organizes arcade menus, tuning screens, track carousels, and ceremony results.

pub mod city_select;
pub mod font;
pub mod menu;
pub mod pause;
pub mod pause_input;
pub mod results;
pub mod title;
pub mod track_select;
pub mod tuning_input;
pub mod tuning_screen;

pub use city_select::CitySelectScreen;
pub use menu::{MainMenu, MenuItem};
pub use pause::PauseMenu;
pub use pause_input::{PauseChoice, PauseFrame, PauseInput};
pub use results::ResultsScreen;
pub use title::TitleScreen;
pub use track_select::TrackSelectScreen;
pub use tuning_input::{Reject, TuningInput, TuningMenu};
pub use tuning_screen::TuningScreen;
