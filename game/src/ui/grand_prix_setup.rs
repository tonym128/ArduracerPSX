//! Grand Prix Setup & Difficulty Selection Screen.
//!
//! Allows the player to select Cup (Bronze, Silver, Gold, Platinum),
//! Difficulty (Easy, Medium, Hard), or Resume an in-progress Championship
//! saved to the PlayStation memory card.

use crate::ui::font::draw_text;
use arduracer_core::championship::{ChampionshipSession, Difficulty, CUP_COUNT, CUP_NAMES};
use psx_gpu as gpu;
use psx_pad::{button, PadState};

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum GrandPrixSetupAction {
    StartNew(u8, Difficulty), // (cup_idx, difficulty)
    Resume,
    Back,
}

pub struct GrandPrixSetupScreen {
    pub selected_cup: u8,
    pub selected_difficulty: Difficulty,
    pub resume_available: bool,
    pub selected_row: u8, // 0 = Resume (if available), 1 = Cup, 2 = Difficulty, 3 = Start Race
    pub anim_timer: u32,
    pub prev_up: bool,
    pub prev_down: bool,
    pub prev_left: bool,
    pub prev_right: bool,
    pub prev_confirm: bool,
    pub prev_cancel: bool,
}

impl GrandPrixSetupScreen {
    pub fn new(resume_available: bool) -> Self {
        GrandPrixSetupScreen {
            selected_cup: 0,
            selected_difficulty: Difficulty::Medium,
            resume_available,
            selected_row: if resume_available { 0 } else { 1 },
            anim_timer: 0,
            prev_up: true,
            prev_down: true,
            prev_left: false,
            prev_right: false,
            prev_confirm: true,
            prev_cancel: true,
        }
    }

    pub fn arm_for_entry(&mut self, resume_available: bool) {
        self.resume_available = resume_available;
        self.selected_row = if resume_available { 0 } else { 1 };
        self.anim_timer = 0;
        self.prev_up = true;
        self.prev_down = true;
        self.prev_left = false;
        self.prev_right = false;
        self.prev_confirm = true;
        self.prev_cancel = true;
    }

    pub fn update(&mut self, pad: &PadState) -> Option<GrandPrixSetupAction> {
        self.anim_timer = self.anim_timer.wrapping_add(1);

        let b = pad.buttons;
        let up = b.is_held(button::UP);
        let down = b.is_held(button::DOWN);
        let left = b.is_held(button::LEFT);
        let right = b.is_held(button::RIGHT);
        let confirm_held = b.is_held(button::CROSS) || b.is_held(button::START);
        let cancel_held = b.is_held(button::CIRCLE);

        let min_row = if self.resume_available { 0 } else { 1 };

        // Vertical navigation
        if up && !self.prev_up {
            if self.selected_row > min_row {
                self.selected_row -= 1;
            } else {
                self.selected_row = 3;
            }
        } else if down && !self.prev_down {
            if self.selected_row < 3 {
                self.selected_row += 1;
            } else {
                self.selected_row = min_row;
            }
        }

        // Horizontal navigation
        if left && !self.prev_left {
            if self.selected_row == 1 {
                // Cup
                if self.selected_cup > 0 {
                    self.selected_cup -= 1;
                } else {
                    self.selected_cup = (CUP_COUNT - 1) as u8;
                }
            } else if self.selected_row == 2 {
                // Difficulty
                self.selected_difficulty = match self.selected_difficulty {
                    Difficulty::Easy => Difficulty::Hard,
                    Difficulty::Medium => Difficulty::Easy,
                    Difficulty::Hard => Difficulty::Medium,
                };
            }
        } else if right && !self.prev_right {
            if self.selected_row == 1 {
                // Cup
                if (self.selected_cup as usize) < CUP_COUNT - 1 {
                    self.selected_cup += 1;
                } else {
                    self.selected_cup = 0;
                }
            } else if self.selected_row == 2 {
                // Difficulty
                self.selected_difficulty = match self.selected_difficulty {
                    Difficulty::Easy => Difficulty::Medium,
                    Difficulty::Medium => Difficulty::Hard,
                    Difficulty::Hard => Difficulty::Easy,
                };
            }
        }

        self.prev_up = up;
        self.prev_down = down;
        self.prev_left = left;
        self.prev_right = right;

        let confirmed = confirm_held && !self.prev_confirm;
        self.prev_confirm = confirm_held;

        let cancelled = cancel_held && !self.prev_cancel;
        self.prev_cancel = cancel_held;

        if cancelled {
            return Some(GrandPrixSetupAction::Back);
        }

        if confirmed {
            if self.selected_row == 0 && self.resume_available {
                return Some(GrandPrixSetupAction::Resume);
            } else if self.selected_row == 3 || self.selected_row == 1 || self.selected_row == 2 {
                return Some(GrandPrixSetupAction::StartNew(
                    self.selected_cup,
                    self.selected_difficulty,
                ));
            }
        }

        None
    }

    pub fn render(&self, saved_champ: Option<&ChampionshipSession>) {
        // Outer box and drop shadow
        gpu::draw_rect_flat(18, 14, 284, 212, 10, 12, 18);
        gpu::draw_rect_flat(20, 16, 280, 208, 18, 22, 32);
        gpu::draw_rect_flat(22, 18, 276, 204, 26, 32, 46);

        // Header Title
        draw_text(66, 24, "GRAND PRIX CHAMPIONSHIP", (255, 220, 0), 1);
        gpu::draw_rect_flat(36, 38, 248, 2, 220, 40, 60);

        let pulse = (self.anim_timer >> 3) & 1 == 0;
        let hl_bg = if pulse { (220, 35, 55) } else { (180, 25, 45) };

        let mut y = 48i16;

        // 0. Resume Championship (if available)
        if self.resume_available {
            let is_sel = self.selected_row == 0;
            if is_sel {
                gpu::draw_rect_flat(36, y - 2, 248, 22, hl_bg.0, hl_bg.1, hl_bg.2);
                draw_text(44, y as u16 + 4, "> RESUME SAVED CUP", (255, 255, 255), 1);
            } else {
                gpu::draw_rect_flat(36, y - 2, 248, 22, 35, 40, 55);
                draw_text(44, y as u16 + 4, "  RESUME SAVED CUP", (180, 210, 240), 1);
            }
            if let Some(sc) = saved_champ {
                let cup_str = sc.cup_name();
                let _stage_num = sc.current_stage + 1;
                draw_text(180, y as u16 + 4, cup_str, (255, 215, 0), 1);
            }
            y += 26;
        }

        // Section header
        draw_text(40, y as u16, "--- NEW CHAMPIONSHIP ---", (120, 140, 165), 1);
        y += 18;

        // 1. Select Cup
        {
            let is_sel = self.selected_row == 1;
            if is_sel {
                gpu::draw_rect_flat(36, y - 2, 248, 22, hl_bg.0, hl_bg.1, hl_bg.2);
                draw_text(44, y as u16 + 4, "> CUP:", (255, 255, 255), 1);
            } else {
                gpu::draw_rect_flat(36, y - 2, 248, 22, 35, 40, 55);
                draw_text(44, y as u16 + 4, "  CUP:", (180, 190, 205), 1);
            }
            let cup_name = CUP_NAMES[(self.selected_cup as usize).min(CUP_COUNT - 1)];
            let cup_col = match self.selected_cup {
                0 => (205, 127, 50),
                1 => (200, 200, 210),
                2 => (255, 215, 0),
                _ => (0, 220, 255),
            };
            draw_text(110, y as u16 + 4, "<", (255, 235, 40), 1);
            draw_text(124, y as u16 + 4, cup_name, cup_col, 1);
            draw_text(244, y as u16 + 4, ">", (255, 235, 40), 1);
            y += 26;
        }

        // 2. Select Difficulty
        {
            let is_sel = self.selected_row == 2;
            if is_sel {
                gpu::draw_rect_flat(36, y - 2, 248, 22, hl_bg.0, hl_bg.1, hl_bg.2);
                draw_text(44, y as u16 + 4, "> DIFFICULTY:", (255, 255, 255), 1);
            } else {
                gpu::draw_rect_flat(36, y - 2, 248, 22, 35, 40, 55);
                draw_text(44, y as u16 + 4, "  DIFFICULTY:", (180, 190, 205), 1);
            }
            let diff_name = self.selected_difficulty.name();
            let diff_col = match self.selected_difficulty {
                Difficulty::Easy => (80, 220, 100),
                Difficulty::Medium => (255, 215, 0),
                Difficulty::Hard => (255, 60, 60),
            };
            draw_text(150, y as u16 + 4, "<", (255, 235, 40), 1);
            draw_text(164, y as u16 + 4, diff_name, diff_col, 1);
            draw_text(224, y as u16 + 4, ">", (255, 235, 40), 1);
            y += 28;
        }

        // 3. Start Race Button
        {
            let is_sel = self.selected_row == 3;
            if is_sel {
                gpu::draw_rect_flat(50, y, 220, 24, hl_bg.0, hl_bg.1, hl_bg.2);
                draw_text(
                    80,
                    y as u16 + 7,
                    ">> START GRAND PRIX <<",
                    (255, 255, 255),
                    1,
                );
            } else {
                gpu::draw_rect_flat(50, y, 220, 24, 45, 55, 75);
                draw_text(
                    80,
                    y as u16 + 7,
                    "   START GRAND PRIX   ",
                    (200, 210, 230),
                    1,
                );
            }
        }

        // Instructions Footer
        draw_text(
            44,
            198,
            "D-PAD: CHANGE   CROSS: CONFIRM   O: BACK",
            (140, 155, 175),
            1,
        );
    }
}
