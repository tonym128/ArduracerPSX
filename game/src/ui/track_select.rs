//! Track & Cup Selection Carousel for PlayStation 1.
//!
//! Allows browsing tracks partitioned across cups with par times,
//! track stats, real track names, and edge-triggered confirmation.

use crate::ui::font::{draw_char, draw_text};
use arduracer_core::levels::{GameMode, ALL_CUPS};
use arduracer_core::{TrackDef, ALL_TRACKS};
use psx_gpu as gpu;
use psx_pad::{button, PadState};

/// Draws `value` as two decimal digits at `x`, advancing by one glyph width.
///
/// Hardware-free apart from the two `draw_char` calls, so the digit arithmetic
/// is separated from the drawing.
fn draw_count(mut x: u16, y: u16, value: u8, colour: (u8, u8, u8)) {
    let clamped = value.min(99);
    draw_char(x, y, b'0' + clamped / 10, colour, 1);
    x += 8;
    draw_char(x, y, b'0' + clamped % 10, colour, 1);
}

pub struct TrackSelectScreen {
    pub mode: GameMode,
    pub selected_track_idx: usize,
    pub is_records_mode: bool,
    pub prev_left: bool,
    pub prev_right: bool,
    pub prev_up: bool,
    pub prev_down: bool,
    pub prev_confirm: bool,
    pub prev_cancel: bool,
    pub prev_select: bool,
    pub open_table: bool,
}

impl Default for TrackSelectScreen {
    fn default() -> Self {
        Self::new()
    }
}

impl TrackSelectScreen {
    pub const fn new() -> Self {
        TrackSelectScreen {
            mode: GameMode::GrandPrix,
            selected_track_idx: 0,
            is_records_mode: false,
            // All armed: every one of these confirms or navigates on the main
            // menu immediately before this screen, so a button still held on
            // entry must be released before it can act (TASK-1217).
            prev_left: true,
            prev_right: true,
            prev_up: true,
            prev_down: true,
            prev_confirm: true,
            prev_cancel: true,
            prev_select: true,
            open_table: false,
        }
    }

    /// Adopts the buttons held right now as the edge baseline.
    ///
    /// Sets the mode and records browsing flag, selecting the first track in
    /// the mode's track range.
    pub fn arm_for_entry(&mut self, mode: GameMode, is_records_mode: bool) {
        self.mode = mode;
        self.is_records_mode = is_records_mode;
        let (start, _) = self.track_range();
        self.arm_for_entry_with_track(mode, is_records_mode, start);
    }

    /// Adopts the buttons held right now as the edge baseline and focuses
    /// on `track_idx` (clamped to the mode's track range).
    pub fn arm_for_entry_with_track(
        &mut self,
        mode: GameMode,
        is_records_mode: bool,
        track_idx: usize,
    ) {
        self.mode = mode;
        self.is_records_mode = is_records_mode;
        let (start, end) = self.track_range();
        if track_idx >= start && track_idx < end {
            self.selected_track_idx = track_idx;
        } else {
            self.selected_track_idx = start;
        }
        self.prev_left = true;
        self.prev_right = true;
        self.prev_up = true;
        self.prev_down = true;
        self.prev_confirm = true;
        self.prev_cancel = true;
        self.prev_select = true;
        self.open_table = false;
    }

    /// Consumes and returns whether a switch to the Stage Table was requested.
    pub fn check_open_table(&mut self) -> bool {
        let v = self.open_table;
        self.open_table = false;
        v
    }

    /// Returns the half-open range `(start, end)` of track indices available.
    pub fn track_range(&self) -> (usize, usize) {
        if self.is_records_mode {
            (0, ALL_TRACKS.len())
        } else {
            match self.mode {
                GameMode::Mania => (25, ALL_TRACKS.len()),
                GameMode::GrandPrix | GameMode::FreeRoam => (0, 25),
            }
        }
    }

    fn jump_cup(&mut self, forward: bool) {
        let (start, end) = self.track_range();
        let mut valid = [0usize; 16];
        let mut count = 0;
        for (i, cup) in ALL_CUPS.iter().enumerate() {
            if let Some(&first) = cup.track_indices.first() {
                if first >= start && first < end {
                    valid[count] = i;
                    count += 1;
                }
            }
        }
        if count <= 1 {
            let total = end.saturating_sub(start);
            if total > 0 {
                let offset = (self.selected_track_idx - start + (total / 2).max(1)) % total;
                self.selected_track_idx = start + offset;
            }
            return;
        }

        let cur_cup_idx = ALL_CUPS
            .iter()
            .position(|c| c.track_indices.contains(&self.selected_track_idx))
            .unwrap_or(0);

        let cur_pos = valid[..count]
            .iter()
            .position(|&idx| idx == cur_cup_idx)
            .unwrap_or(0);
        let next_pos = if forward {
            (cur_pos + 1) % count
        } else if cur_pos > 0 {
            cur_pos - 1
        } else {
            count - 1
        };

        let target_cup = &ALL_CUPS[valid[next_pos]];
        if let Some(&first) = target_cup.track_indices.first() {
            self.selected_track_idx = first;
        }
    }

    /// Handles carousel navigation. Returns:
    /// - (Some(track_idx), false) if confirmed with CROSS or START
    /// - (None, true) if cancelled with CIRCLE
    /// - (None, false) if still browsing
    pub fn update(&mut self, pad: &PadState) -> (Option<usize>, bool) {
        let b = pad.buttons;
        let left = b.is_held(button::LEFT);
        let right = b.is_held(button::RIGHT);
        let up = b.is_held(button::UP);
        let down = b.is_held(button::DOWN);

        let (start, end) = self.track_range();
        let total = end.saturating_sub(start);

        // Previous / Next Track
        if total > 0 {
            if right && !self.prev_right {
                if self.selected_track_idx + 1 < end {
                    self.selected_track_idx += 1;
                } else {
                    self.selected_track_idx = start;
                }
            } else if left && !self.prev_left {
                if self.selected_track_idx > start {
                    self.selected_track_idx -= 1;
                } else {
                    self.selected_track_idx = end - 1;
                }
            }
        }

        // Cup Jump
        if down && !self.prev_down {
            self.jump_cup(true);
        } else if up && !self.prev_up {
            self.jump_cup(false);
        }

        self.prev_left = left;
        self.prev_right = right;
        self.prev_up = up;
        self.prev_down = down;

        let confirm_held = b.is_held(button::CROSS) || b.is_held(button::START);
        let cancel_held = b.is_held(button::CIRCLE);
        let select_held = b.is_held(button::SELECT);

        let confirmed = confirm_held && !self.prev_confirm;
        let cancelled = cancel_held && !self.prev_cancel;
        if select_held && !self.prev_select {
            self.open_table = true;
        }

        self.prev_confirm = confirm_held;
        self.prev_cancel = cancel_held;
        self.prev_select = select_held;

        if confirmed {
            (Some(self.selected_track_idx), false)
        } else if cancelled {
            (None, true)
        } else {
            (None, false)
        }
    }

    /// Renders the track selection screen with minimap, personal records, and level passed status.
    pub fn render(&self, save: Option<&arduracer_core::SaveData>) {
        let track: &'static TrackDef =
            ALL_TRACKS[self.selected_track_idx.min(ALL_TRACKS.len() - 1)];

        // Background panel
        gpu::draw_rect_flat(10, 10, 300, 220, 16, 20, 28);
        gpu::draw_rect_flat(12, 12, 296, 216, 26, 32, 44);

        // Header: "GRAND PRIX TRIALS", "MANIA TRIALS", or "RECORDS & MEDALS"
        let (header_title, header_x) = if self.is_records_mode {
            ("RECORDS & MEDALS", 80)
        } else if self.mode == GameMode::Mania {
            ("MANIA TRIALS", 96)
        } else {
            ("GRAND PRIX TRIALS", 76)
        };
        draw_text(header_x, 20, header_title, (255, 225, 30), 2);
        gpu::draw_rect_flat(30, 38, 260, 2, 220, 40, 60);

        // Cup classification
        let cup = ALL_CUPS
            .iter()
            .find(|c| c.track_indices.contains(&self.selected_track_idx));
        let cup_name = match cup {
            Some(c) if c.name == "BRONZE CUP" => "BRONZE CUP (AMATEUR)",
            Some(c) if c.name == "SILVER CUP" => "SILVER CUP (PRO)",
            Some(c) if c.name == "GOLD CUP" => "GOLD CUP (CHAMPION)",
            Some(c) if c.name == "PLATINUM CUP" => "PLATINUM CUP (SUPER SPEEDWAYS)",
            Some(c) if c.name == "DIAMOND CUP" => "DIAMOND CUP (TITAN CIRCUIT)",
            Some(c) if c.name == "MANIA CUP" => "MANIA CUP (ARCADE RACING)",
            Some(c) => c.name,
            None => "CIRCUIT",
        };
        draw_text(50, 46, cup_name, (0, 220, 255), 1);

        // Track Number & Name Box
        gpu::draw_rect_flat(20, 62, 280, 34, 35, 42, 55);
        gpu::draw_rect_flat(22, 64, 276, 30, 20, 25, 35);

        // Track Index Number: e.g. "TRACK 01 / 25", "TRACK 01 / 06"
        let (start, end) = self.track_range();
        let cur_in_mode = (self.selected_track_idx.saturating_sub(start) + 1) as u8;
        let tot_in_mode = (end.saturating_sub(start)) as u8;
        let num_str = [
            b'T',
            b'R',
            b'A',
            b'C',
            b'K',
            b' ',
            b'0' + ((cur_in_mode / 10) % 10),
            b'0' + (cur_in_mode % 10),
            b' ',
            b'/',
            b' ',
            b'0' + ((tot_in_mode / 10) % 10),
            b'0' + (tot_in_mode % 10),
            0,
        ];
        let mut cur_x = 30u16;
        for &ch in num_str.iter() {
            if ch == 0 {
                break;
            }
            draw_char(cur_x, 74, ch, (255, 240, 50), 1);
            cur_x += 7;
        }

        // Real track name
        draw_text(128, 74, track.name, (255, 255, 255), 1);

        // Left Column: Minimap
        crate::gpu::draw_ui_minimap(20, 102, 68, 68, track);
        draw_text(22, 174, "GATES:", (0, 220, 255), 1);
        draw_count(62, 174, track.checkpoint_count, (255, 255, 255));
        draw_text(22, 185, "LAPS: 5", (0, 220, 255), 1);

        // Center Column: Target Par Times
        let pt = &track.par_times;
        gpu::draw_rect_flat(92, 102, 100, 92, 30, 36, 48);
        draw_text(96, 106, "TARGET PARS:", (255, 225, 30), 1);

        let bronze_sec = pt.bronze_ticks / 60;
        let silver_sec = pt.silver_ticks / 60;
        let gold_sec = pt.gold_ticks / 60;
        let dev_sec = pt.dev_platinum_ticks / 60;

        let medals = [
            ("BRONZE: ", bronze_sec, (205, 127, 50)),
            ("SILVER: ", silver_sec, (192, 192, 192)),
            ("GOLD:   ", gold_sec, (255, 215, 0)),
            ("DEV:    ", dev_sec, (100, 220, 255)),
        ];

        for (idx, (label, sec, col)) in medals.iter().enumerate() {
            let my = 120 + (idx as i16) * 16;
            draw_text(96, my as u16, label, *col, 1);

            let s_ten = b'0' + ((sec / 10) % 10) as u8;
            let s_one = b'0' + (sec % 10) as u8;
            draw_char(158, my as u16, s_ten, (255, 255, 255), 1);
            draw_char(165, my as u16, s_one, (255, 255, 255), 1);
            draw_char(172, my as u16, b'S', (160, 160, 160), 1);
        }

        // Right Column: Personal Records & Level Passed
        gpu::draw_rect_flat(196, 102, 104, 92, 30, 36, 48);
        draw_text(200, 106, "YOUR RECORD:", (0, 220, 255), 1);

        // Personal Best record readout
        let best_ticks = save
            .and_then(|s| s.best_lap_ticks.get(self.selected_track_idx))
            .copied()
            .unwrap_or(arduracer_core::timing::NO_BEST_LAP);
        draw_text(200, 118, "BEST:", (180, 200, 220), 1);
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
            let mut cur_x = 238u16;
            for &ch in time_str.iter() {
                if ch == 0 {
                    break;
                }
                draw_char(cur_x, 118, ch, (255, 230, 40), 1);
                cur_x += 6;
            }
        } else {
            draw_text(238, 118, "--:--.--", (140, 150, 160), 1);
        }

        // Earned Medal readout
        let medal_code = save
            .and_then(|s| s.medals_earned.get(self.selected_track_idx))
            .copied()
            .unwrap_or(0);
        draw_text(200, 134, "MEDAL:", (180, 200, 220), 1);
        let (medal_str, medal_col) = match medal_code {
            1 => ("BRONZE", (205, 127, 50)),
            2 => ("SILVER", (192, 192, 192)),
            3 => ("GOLD", (255, 215, 0)),
            4 => ("DEV PLAT", (100, 220, 255)),
            _ => ("NONE", (140, 150, 160)),
        };
        draw_text(244, 134, medal_str, medal_col, 1);

        // Level Passed Status
        draw_text(200, 152, "LEVEL:", (180, 200, 220), 1);
        let is_passed = medal_code > 0
            || (best_ticks != arduracer_core::timing::NO_BEST_LAP && best_ticks != u32::MAX);
        if is_passed {
            gpu::draw_rect_flat(200, 164, 96, 14, 25, 75, 35);
            draw_text(224, 167, "PASSED", (100, 255, 120), 1);
        } else {
            gpu::draw_rect_flat(200, 164, 96, 14, 45, 48, 56);
            draw_text(210, 167, "NOT PASSED", (170, 180, 195), 1);
        }

        // Navigation Footer
        let footer_text = if self.is_records_mode {
            "L/R: TRACK  U/D: CUP  SEL: TABLE  CROSS/O: RET"
        } else {
            "L/R: TRACK  U/D: CUP  SEL: TABLE  CROSS: RACE  O: BACK"
        };
        draw_text(16, 202, footer_text, (130, 150, 170), 1);
    }
}
