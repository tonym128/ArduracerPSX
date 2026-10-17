//! Stage Table Interface for all GP Trials and Mania Trials.
//!
//! Provides a comprehensive tabular overview of all stages, clearance status,
//! medals/levels earned, and personal best times, with real-time minimap preview
//! and one-click challenge launching.

use crate::ui::font::{draw_char, draw_text};
use arduracer_core::levels::{ALL_CUPS, ALL_TRACKS};
use psx_gpu as gpu;
use psx_pad::{button, PadState};

#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum StageFilterTab {
    #[default]
    All = 0,
    GrandPrix = 1,
    Mania = 2,
}

impl StageFilterTab {
    pub fn name(self) -> &'static str {
        match self {
            StageFilterTab::All => "ALL STAGES (31)",
            StageFilterTab::GrandPrix => "GP TRIALS (25)",
            StageFilterTab::Mania => "MANIA TRIALS (6)",
        }
    }

    pub fn next(self) -> Self {
        match self {
            StageFilterTab::All => StageFilterTab::GrandPrix,
            StageFilterTab::GrandPrix => StageFilterTab::Mania,
            StageFilterTab::Mania => StageFilterTab::All,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            StageFilterTab::All => StageFilterTab::Mania,
            StageFilterTab::GrandPrix => StageFilterTab::All,
            StageFilterTab::Mania => StageFilterTab::GrandPrix,
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum StageTableAction {
    None,
    SelectStage(usize),
    Back,
    SwitchToCarousel,
}

pub struct StageTableScreen {
    pub tab: StageFilterTab,
    pub selected_row: usize,
    pub scroll_top: usize,
    pub anim_timer: u32,
    pub prev_up: bool,
    pub prev_down: bool,
    pub prev_left: bool,
    pub prev_right: bool,
    pub prev_confirm: bool,
    pub prev_cancel: bool,
    pub prev_select: bool,
}

impl Default for StageTableScreen {
    fn default() -> Self {
        Self::new()
    }
}

impl StageTableScreen {
    pub const fn new() -> Self {
        StageTableScreen {
            tab: StageFilterTab::All,
            selected_row: 0,
            scroll_top: 0,
            anim_timer: 0,
            prev_up: true,
            prev_down: true,
            prev_left: true,
            prev_right: true,
            prev_confirm: true,
            prev_cancel: true,
            prev_select: true,
        }
    }

    pub fn arm_for_entry(&mut self, initial_tab: StageFilterTab) {
        self.tab = initial_tab;
        self.selected_row = 0;
        self.scroll_top = 0;
        self.anim_timer = 0;
        self.prev_up = true;
        self.prev_down = true;
        self.prev_left = true;
        self.prev_right = true;
        self.prev_confirm = true;
        self.prev_cancel = true;
        self.prev_select = true;
    }

    /// Returns the track indices corresponding to the active tab.
    pub fn active_tracks(&self) -> &'static [usize] {
        static ALL_INDICES: [usize; 31] = [
            0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23,
            24, 25, 26, 27, 28, 29, 30,
        ];
        static GP_INDICES: [usize; 25] = [
            0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23,
            24,
        ];
        static MANIA_INDICES: [usize; 6] = [25, 26, 27, 28, 29, 30];

        match self.tab {
            StageFilterTab::All => &ALL_INDICES,
            StageFilterTab::GrandPrix => &GP_INDICES,
            StageFilterTab::Mania => &MANIA_INDICES,
        }
    }

    pub fn update(&mut self, pad: &PadState) -> StageTableAction {
        self.anim_timer = self.anim_timer.wrapping_add(1);

        let b = pad.buttons;
        let up = b.is_held(button::UP);
        let down = b.is_held(button::DOWN);
        let left = b.is_held(button::LEFT) || b.is_held(button::L1);
        let right = b.is_held(button::RIGHT) || b.is_held(button::R1);
        let confirm_held = b.is_held(button::CROSS) || b.is_held(button::START);
        let cancel_held = b.is_held(button::CIRCLE);
        let select_held = b.is_held(button::SELECT);

        let tracks = self.active_tracks();
        let total = tracks.len();

        // Vertical row navigation
        if down && !self.prev_down {
            if total > 0 && self.selected_row + 1 < total {
                self.selected_row += 1;
                if self.selected_row >= self.scroll_top + 7 {
                    self.scroll_top = self.selected_row - 6;
                }
            } else {
                self.selected_row = 0;
                self.scroll_top = 0;
            }
        } else if up && !self.prev_up {
            if self.selected_row > 0 {
                self.selected_row -= 1;
                if self.selected_row < self.scroll_top {
                    self.scroll_top = self.selected_row;
                }
            } else if total > 0 {
                self.selected_row = total - 1;
                self.scroll_top = self.selected_row.saturating_sub(6);
            }
        }

        // Horizontal tab switching
        if right && !self.prev_right {
            self.tab = self.tab.next();
            self.selected_row = 0;
            self.scroll_top = 0;
        } else if left && !self.prev_left {
            self.tab = self.tab.prev();
            self.selected_row = 0;
            self.scroll_top = 0;
        }

        self.prev_up = up;
        self.prev_down = down;
        self.prev_left = left;
        self.prev_right = right;

        let confirmed = confirm_held && !self.prev_confirm;
        let cancelled = cancel_held && !self.prev_cancel;
        let selected = select_held && !self.prev_select;

        self.prev_confirm = confirm_held;
        self.prev_cancel = cancel_held;
        self.prev_select = select_held;

        if confirmed {
            if let Some(&track_idx) = tracks.get(self.selected_row) {
                return StageTableAction::SelectStage(track_idx);
            }
        } else if cancelled {
            return StageTableAction::Back;
        } else if selected {
            return StageTableAction::SwitchToCarousel;
        }

        StageTableAction::None
    }

    pub fn render(&self, save: Option<&arduracer_core::SaveData>) {
        // Outer box and drop shadow
        gpu::draw_rect_flat(10, 8, 300, 224, 12, 14, 20);
        gpu::draw_rect_flat(12, 10, 296, 220, 20, 25, 36);
        gpu::draw_rect_flat(14, 12, 292, 216, 28, 34, 48);

        // Header Title
        draw_text(60, 16, "STAGE CHALLENGES TABLE", (255, 225, 30), 1);

        // Tab Selector Bar
        gpu::draw_rect_flat(20, 30, 280, 16, 35, 42, 58);
        draw_text(26, 34, "<", (255, 235, 40), 1);
        let tab_name = self.tab.name();
        draw_text(80, 34, tab_name, (0, 220, 255), 1);
        draw_text(284, 34, ">", (255, 235, 40), 1);

        let tracks = self.active_tracks();
        let total = tracks.len();

        // Left Table: Columns: # (x=20), NAME (x=38), STATUS (x=116), MEDAL (x=156)
        gpu::draw_rect_flat(18, 50, 178, 148, 18, 22, 32);
        gpu::draw_rect_flat(18, 50, 178, 14, 28, 34, 48);
        draw_text(20, 53, "#", (200, 210, 230), 1);
        draw_text(38, 53, "STAGE NAME", (200, 210, 230), 1);
        draw_text(118, 53, "STATUS", (200, 210, 230), 1);
        draw_text(160, 53, "LVL", (200, 210, 230), 1);

        let pulse = (self.anim_timer >> 3) & 1 == 0;
        let hl_col = if pulse { (220, 35, 55) } else { (180, 25, 45) };

        let visible_count = 7.min(total.saturating_sub(self.scroll_top));
        for i in 0..visible_count {
            let row_idx = self.scroll_top + i;
            let track_idx = tracks[row_idx];
            let track = ALL_TRACKS[track_idx.min(ALL_TRACKS.len() - 1)];

            let ry = 66 + (i as i16) * 18;
            let is_sel = row_idx == self.selected_row;

            if is_sel {
                gpu::draw_rect_flat(19, ry - 1, 176, 17, hl_col.0, hl_col.1, hl_col.2);
            } else if (i % 2) == 1 {
                gpu::draw_rect_flat(19, ry - 1, 176, 17, 24, 28, 40);
            }

            // Stage number (01..31)
            let stage_num = (track_idx + 1) as u8;
            let num_ten = b'0' + stage_num / 10;
            let num_one = b'0' + stage_num % 10;
            let num_col = if is_sel {
                (255, 255, 255)
            } else {
                (255, 230, 40)
            };
            draw_char(20, ry as u16 + 3, num_ten, num_col, 1);
            draw_char(27, ry as u16 + 3, num_one, num_col, 1);

            // Stage Name (truncated to 11 chars if needed)
            let mut name_buf = [0u8; 12];
            let mut n_len = 0;
            for &b in track.name.as_bytes() {
                if n_len >= 11 {
                    break;
                }
                name_buf[n_len] = b;
                n_len += 1;
            }
            let mut nx = 38u16;
            for &ch in &name_buf[..n_len] {
                draw_char(nx, ry as u16 + 3, ch, (255, 255, 255), 1);
                nx += 7;
            }

            // Record & Status
            let best_ticks = save
                .and_then(|s| s.best_lap_ticks.get(track_idx))
                .copied()
                .unwrap_or(arduracer_core::timing::NO_BEST_LAP);
            let medal = save
                .and_then(|s| s.medals_earned.get(track_idx))
                .copied()
                .unwrap_or(0);
            let passed = medal > 0
                || (best_ticks != arduracer_core::timing::NO_BEST_LAP && best_ticks != u32::MAX);

            if passed {
                draw_text(118, ry as u16 + 3, "PASS", (100, 255, 120), 1);
            } else {
                draw_text(118, ry as u16 + 3, "----", (130, 140, 155), 1);
            }

            // Level / Medal
            let (m_str, m_col) = match medal {
                1 => ("BRNZ", (205, 127, 50)),
                2 => ("SILV", (192, 192, 192)),
                3 => ("GOLD", (255, 215, 0)),
                4 => ("PLAT", (100, 220, 255)),
                _ => (" -- ", (130, 140, 155)),
            };
            draw_text(158, ry as u16 + 3, m_str, m_col, 1);
        }

        // Scroll indicators
        if self.scroll_top > 0 {
            draw_text(188, 53, "^", (255, 235, 40), 1);
        }
        if self.scroll_top + 7 < total {
            draw_text(188, 186, "v", (255, 235, 40), 1);
        }

        // Right Panel: Highlighted Stage Details & Vector Minimap
        let sel_track_idx = tracks
            .get(self.selected_row)
            .copied()
            .unwrap_or(0)
            .min(ALL_TRACKS.len() - 1);
        let sel_track = ALL_TRACKS[sel_track_idx];

        gpu::draw_rect_flat(200, 50, 100, 148, 24, 28, 40);
        gpu::draw_rect_flat(202, 52, 96, 144, 16, 20, 28);

        // Cup Name
        let cup = ALL_CUPS
            .iter()
            .find(|c| c.track_indices.contains(&sel_track_idx));
        let cup_str = cup.map(|c| c.name).unwrap_or("CIRCUIT");
        draw_text(206, 56, cup_str, (0, 220, 255), 1);

        // Minimap
        crate::gpu::draw_ui_minimap(206, 68, 88, 56, sel_track);

        // Personal Best Record
        let sel_best_ticks = save
            .and_then(|s| s.best_lap_ticks.get(sel_track_idx))
            .copied()
            .unwrap_or(arduracer_core::timing::NO_BEST_LAP);

        draw_text(206, 128, "BEST:", (180, 200, 220), 1);
        if sel_best_ticks != arduracer_core::timing::NO_BEST_LAP && sel_best_ticks != u32::MAX {
            let (m, s, cs) = arduracer_core::LapTimer::<1>::format_ticks(sel_best_ticks);
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
            let mut tx = 242u16;
            for &ch in time_str.iter() {
                if ch == 0 {
                    break;
                }
                draw_char(tx, 128, ch, (255, 230, 40), 1);
                tx += 6;
            }
        } else {
            draw_text(242, 128, "--:--.--", (140, 150, 160), 1);
        }

        // Target Gold Par Time
        let gold_sec = sel_track.par_times.gold_ticks / 60;
        draw_text(206, 140, "GOLD:", (255, 215, 0), 1);
        let g_ten = b'0' + ((gold_sec / 10) % 10) as u8;
        let g_one = b'0' + (gold_sec % 10) as u8;
        draw_char(242, 140, g_ten, (255, 255, 255), 1);
        draw_char(249, 140, g_one, (255, 255, 255), 1);
        draw_char(256, 140, b'S', (160, 160, 160), 1);

        // Level Passed Status Box
        let sel_medal = save
            .and_then(|s| s.medals_earned.get(sel_track_idx))
            .copied()
            .unwrap_or(0);
        let is_passed = sel_medal > 0
            || (sel_best_ticks != arduracer_core::timing::NO_BEST_LAP
                && sel_best_ticks != u32::MAX);

        draw_text(206, 154, "LEVEL:", (180, 200, 220), 1);
        if is_passed {
            gpu::draw_rect_flat(206, 166, 88, 15, 25, 75, 35);
            draw_text(224, 170, "PASSED", (100, 255, 120), 1);
        } else {
            gpu::draw_rect_flat(206, 166, 88, 15, 45, 48, 56);
            draw_text(212, 170, "NOT PASSED", (170, 180, 195), 1);
        }

        // Gates and Laps
        draw_text(206, 184, "GATES:", (0, 220, 255), 1);
        let clamped_gates = sel_track.checkpoint_count.min(99);
        draw_char(252, 184, b'0' + clamped_gates / 10, (255, 255, 255), 1);
        draw_char(259, 184, b'0' + clamped_gates % 10, (255, 255, 255), 1);

        // Footer Instructions
        draw_text(
            18,
            204,
            "D-PAD: NAVIGATE  CROSS: RACE  O: BACK  SEL: VIEW",
            (140, 155, 175),
            1,
        );
    }
}
