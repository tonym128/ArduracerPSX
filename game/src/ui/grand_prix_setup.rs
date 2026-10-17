//! Grand Prix Setup & Difficulty Selection Screen.
//!
//! Allows the player to select Cup (Bronze, Silver, Gold, Platinum),
//! Difficulty (Easy, Medium, Hard), or Resume an in-progress Championship
//! saved to the PlayStation memory card.

use crate::ui::font::draw_text;
use arduracer_core::championship::{ChampionshipSession, Difficulty};
use arduracer_core::levels::{CupDef, GameMode, GRAND_PRIX_CUPS, MANIA_CUPS};
use psx_gpu as gpu;
use psx_pad::{button, PadState};

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum GrandPrixSetupAction {
    StartNew(u8, Difficulty), // (cup_idx, difficulty)
    Resume,
    Back,
}

pub struct GrandPrixSetupScreen {
    pub mode: GameMode,
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
            mode: GameMode::GrandPrix,
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

    pub fn arm_for_entry(&mut self, resume_available: bool, mode: GameMode) {
        self.mode = mode;
        self.resume_available = resume_available && mode == GameMode::GrandPrix;
        self.selected_row = if self.resume_available { 0 } else { 1 };
        self.selected_cup = 0;
        self.anim_timer = 0;
        self.prev_up = true;
        self.prev_down = true;
        self.prev_left = false;
        self.prev_right = false;
        self.prev_confirm = true;
        self.prev_cancel = true;
    }

    pub fn active_cups(&self) -> &'static [CupDef] {
        match self.mode {
            GameMode::GrandPrix => &GRAND_PRIX_CUPS,
            GameMode::Mania => &MANIA_CUPS,
            GameMode::FreeRoam => &GRAND_PRIX_CUPS,
        }
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

        let num_cups = self.active_cups().len();

        // Horizontal navigation
        if left && !self.prev_left {
            if self.selected_row == 1 {
                // Cup
                if self.selected_cup > 0 {
                    self.selected_cup -= 1;
                } else {
                    self.selected_cup = (num_cups.saturating_sub(1)) as u8;
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
                if (self.selected_cup as usize) < num_cups.saturating_sub(1) {
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
                let global_cup = match self.mode {
                    GameMode::GrandPrix => self.selected_cup,
                    GameMode::Mania => (GRAND_PRIX_CUPS.len() as u8) + self.selected_cup,
                    GameMode::FreeRoam => self.selected_cup,
                };
                return Some(GrandPrixSetupAction::StartNew(
                    global_cup,
                    self.selected_difficulty,
                ));
            }
        }

        None
    }

    pub fn render(
        &self,
        saved_champ: Option<&ChampionshipSession>,
        save: Option<&arduracer_core::SaveData>,
    ) {
        // Outer box and drop shadow
        gpu::draw_rect_flat(14, 12, 292, 216, 10, 12, 18);
        gpu::draw_rect_flat(16, 14, 288, 212, 18, 22, 32);
        gpu::draw_rect_flat(18, 16, 284, 208, 26, 32, 46);

        // Header Title
        let header_str = match self.mode {
            GameMode::Mania => "MANIA PRIX CHAMPIONSHIP",
            _ => "GRAND PRIX CHAMPIONSHIP",
        };
        draw_text(60, 22, header_str, (255, 220, 0), 1);
        gpu::draw_rect_flat(30, 34, 260, 2, 220, 40, 60);

        let pulse = (self.anim_timer >> 3) & 1 == 0;
        let hl_bg = if pulse { (220, 35, 55) } else { (180, 25, 45) };

        // Left Column: Navigation Options
        let mut y = 42i16;

        // 0. Resume Championship (if available)
        if self.resume_available {
            let is_sel = self.selected_row == 0;
            if is_sel {
                gpu::draw_rect_flat(24, y - 2, 146, 22, hl_bg.0, hl_bg.1, hl_bg.2);
                draw_text(28, y as u16 + 4, "> RESUME CUP", (255, 255, 255), 1);
            } else {
                gpu::draw_rect_flat(24, y - 2, 146, 22, 35, 40, 55);
                draw_text(28, y as u16 + 4, "  RESUME CUP", (180, 210, 240), 1);
            }
            if let Some(sc) = saved_champ {
                let cup_str = sc.cup_name();
                draw_text(104, y as u16 + 4, cup_str, (255, 215, 0), 1);
            }
            y += 26;
        }

        // Section header
        draw_text(26, y as u16, "-- CHAMPIONSHIP --", (120, 140, 165), 1);
        y += 16;

        // 1. Select Cup
        {
            let is_sel = self.selected_row == 1;
            if is_sel {
                gpu::draw_rect_flat(24, y - 2, 146, 22, hl_bg.0, hl_bg.1, hl_bg.2);
                draw_text(28, y as u16 + 4, "> CUP:", (255, 255, 255), 1);
            } else {
                gpu::draw_rect_flat(24, y - 2, 146, 22, 35, 40, 55);
                draw_text(28, y as u16 + 4, "  CUP:", (180, 190, 205), 1);
            }
            let cups = self.active_cups();
            let cup_name = cups
                .get(self.selected_cup as usize)
                .map(|c| c.name)
                .unwrap_or("CUP");
            let cup_col = match self.selected_cup {
                0 => (205, 127, 50),
                1 => (200, 200, 210),
                2 => (255, 215, 0),
                _ => (0, 220, 255),
            };
            draw_text(74, y as u16 + 4, "<", (255, 235, 40), 1);
            draw_text(84, y as u16 + 4, cup_name, cup_col, 1);
            draw_text(160, y as u16 + 4, ">", (255, 235, 40), 1);
            y += 26;
        }

        // 2. Select Difficulty
        {
            let is_sel = self.selected_row == 2;
            if is_sel {
                gpu::draw_rect_flat(24, y - 2, 146, 22, hl_bg.0, hl_bg.1, hl_bg.2);
                draw_text(28, y as u16 + 4, "> DIFF:", (255, 255, 255), 1);
            } else {
                gpu::draw_rect_flat(24, y - 2, 146, 22, 35, 40, 55);
                draw_text(28, y as u16 + 4, "  DIFF:", (180, 190, 205), 1);
            }
            let diff_name = self.selected_difficulty.name();
            let diff_col = match self.selected_difficulty {
                Difficulty::Easy => (80, 220, 100),
                Difficulty::Medium => (255, 215, 0),
                Difficulty::Hard => (255, 60, 60),
            };
            draw_text(84, y as u16 + 4, "<", (255, 235, 40), 1);
            draw_text(96, y as u16 + 4, diff_name, diff_col, 1);
            draw_text(152, y as u16 + 4, ">", (255, 235, 40), 1);
            y += 28;
        }

        // 3. Start Race Button
        {
            let is_sel = self.selected_row == 3;
            let (btn_str, btn_unsel) = match self.mode {
                GameMode::Mania => (">> START MANIA <<", "   START MANIA   "),
                _ => (">> START RACE <<", "   START RACE   "),
            };
            if is_sel {
                gpu::draw_rect_flat(24, y, 146, 24, hl_bg.0, hl_bg.1, hl_bg.2);
                draw_text(36, y as u16 + 7, btn_str, (255, 255, 255), 1);
            } else {
                gpu::draw_rect_flat(24, y, 146, 24, 45, 55, 75);
                draw_text(36, y as u16 + 7, btn_unsel, (200, 210, 230), 1);
            }
        }

        // Right Column: Minimap, Records, Par Times & Level Passed
        gpu::draw_rect_flat(178, 40, 122, 156, 30, 36, 48);
        let preview_track_idx = if self.selected_row == 0 && self.resume_available {
            saved_champ.map(|c| c.current_track_idx()).unwrap_or(0)
        } else {
            let cups = self.active_cups();
            let cup = cups.get(self.selected_cup as usize);
            cup.and_then(|c| c.track_indices.first().copied())
                .unwrap_or(0)
        };
        let track = arduracer_core::levels::ALL_TRACKS
            [preview_track_idx.min(arduracer_core::levels::ALL_TRACKS.len() - 1)];

        draw_text(182, 44, "STAGE PREVIEW:", (255, 225, 30), 1);
        draw_text(182, 54, track.name, (255, 255, 255), 1);

        // Minimap
        crate::gpu::draw_ui_minimap(182, 66, 62, 54, track);

        // Par Times
        let gold_sec = track.par_times.gold_ticks / 60;
        draw_text(248, 70, "GOLD:", (255, 215, 0), 1);
        let g_str = [
            b'0' + ((gold_sec / 10) % 10) as u8,
            b'0' + (gold_sec % 10) as u8,
            b'S',
            0,
        ];
        let mut gx = 250u16;
        for &ch in g_str.iter() {
            if ch == 0 {
                break;
            }
            crate::ui::font::draw_char(gx, 82, ch, (255, 255, 255), 1);
            gx += 7;
        }

        // Personal Best
        let best_ticks = save
            .and_then(|s| s.best_lap_ticks.get(preview_track_idx))
            .copied()
            .unwrap_or(arduracer_core::timing::NO_BEST_LAP);
        draw_text(182, 124, "BEST:", (180, 200, 220), 1);
        if best_ticks != arduracer_core::timing::NO_BEST_LAP && best_ticks != u32::MAX {
            let (m, s, cs) = arduracer_core::LapTimer::<1>::format_ticks(best_ticks);
            let time_str = [
                b'0' + ((m / 10) % 10) as u8,
                b'0' + (m % 10) as u8,
                b':',
                b'0' + ((s / 10) % 10) as u8,
                b'0' + (s % 10) as u8,
                b'.',
                b'0' + ((cs / 10) % 10) as u8,
                b'0' + (cs % 10) as u8,
                0,
            ];
            let mut cur_x = 224u16;
            for &ch in time_str.iter() {
                if ch == 0 {
                    break;
                }
                crate::ui::font::draw_char(cur_x, 124, ch, (255, 230, 40), 1);
                cur_x += 6;
            }
        } else {
            draw_text(224, 124, "--:--.--", (140, 150, 160), 1);
        }

        // Medal
        let medal_code = save
            .and_then(|s| s.medals_earned.get(preview_track_idx))
            .copied()
            .unwrap_or(0);
        draw_text(182, 138, "MEDAL:", (180, 200, 220), 1);
        let (medal_str, medal_col) = match medal_code {
            1 => ("BRONZE", (205, 127, 50)),
            2 => ("SILVER", (192, 192, 192)),
            3 => ("GOLD", (255, 215, 0)),
            4 => ("DEV PLAT", (100, 220, 255)),
            _ => ("NONE", (140, 150, 160)),
        };
        draw_text(228, 138, medal_str, medal_col, 1);

        // Level Passed Status
        draw_text(182, 154, "LEVEL:", (180, 200, 220), 1);
        let is_passed = medal_code > 0
            || (best_ticks != arduracer_core::timing::NO_BEST_LAP && best_ticks != u32::MAX);
        if is_passed {
            gpu::draw_rect_flat(182, 166, 114, 16, 25, 75, 35);
            draw_text(216, 170, "PASSED", (100, 255, 120), 1);
        } else {
            gpu::draw_rect_flat(182, 166, 114, 16, 45, 48, 56);
            draw_text(202, 170, "NOT PASSED", (170, 180, 195), 1);
        }

        // Instructions Footer
        draw_text(
            24,
            202,
            "D-PAD: CHANGE   CROSS: CONFIRM   O: BACK",
            (140, 155, 175),
            1,
        );
    }
}
