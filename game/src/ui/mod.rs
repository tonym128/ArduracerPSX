//! User Interface Subsystem for Arduracer PSX.
//!
//! Organizes arcade menus, tuning screens, track carousels, and ceremony results.

pub mod font;
pub mod menu;
pub mod pause;
pub mod results;
pub mod title;
pub mod track_select;
pub mod tuning_screen;

pub use menu::{MainMenu, MenuItem};
pub use pause::{PauseChoice, PauseMenu};
pub use results::ResultsScreen;
pub use title::TitleScreen;
pub use track_select::TrackSelectScreen;
pub use tuning_screen::TuningScreen;
