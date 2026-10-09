//! City Free Drive Selection Screen for Arduracer PSX.
//!
//! Allows browsing the available 10 km^2 photoreal streaming cities
//! (Cape Town, Melbourne, etc.) and launching free urban roam.

use crate::ui::font::draw_text;
use arduracer_core::{TrackDef, TRACK_CAPETOWN, TRACK_MELBOURNE};
use psx_gpu as gpu;
use psx_pad::{button, PadState};

pub struct CityInfo {
    pub name: &'static str,
    pub country: &'static str,
    pub landmarks: &'static str,
    pub track: &'static TrackDef,
    pub track_id: usize, // 99 for Cape Town, 98 for Melbourne
}

pub static CITIES: [CityInfo; 2] = [
    CityInfo {
        name: "CAPE TOWN",
        country: "SOUTH AFRICA (ATLANTIC COAST)",
        landmarks: "GREEN POINT & V&A WATERFRONT",
        track: &TRACK_CAPETOWN,
        track_id: 99,
    },
    CityInfo {
        name: "MELBOURNE",
        country: "AUSTRALIA (VICTORIA)",
        landmarks: "ALBERT PARK GP & ST KILDA",
        track: &TRACK_MELBOURNE,
        track_id: 98,
    },
];

pub struct CitySelectScreen {
    pub selected_idx: usize,
    pub prev_left: bool,
    pub prev_right: bool,
    pub prev_confirm: bool,
    pub prev_cancel: bool,
    pub anim_timer: u32,
}

impl Default for CitySelectScreen {
    fn default() -> Self {
        Self::new()
    }
}

impl CitySelectScreen {
    pub const fn new() -> Self {
        CitySelectScreen {
            selected_idx: 0,
            prev_left: true,
            prev_right: true,
            prev_confirm: true,
            prev_cancel: true,
            anim_timer: 0,
        }
    }

    /// Arms button edges when entering from the main menu so held presses don't leak.
    pub fn arm_for_entry(&mut self) {
        self.prev_left = true;
        self.prev_right = true;
        self.prev_confirm = true;
        self.prev_cancel = true;
    }

    /// Handles city selection navigation. Returns `(Option<&'static CityInfo>, bool)`:
    /// - `(Some(city), false)` on confirmation (CROSS / START)
    /// - `(None, true)` on cancel (TRIANGLE)
    /// - `(None, false)` while navigating
    pub fn update(&mut self, pad: &PadState) -> (Option<&'static CityInfo>, bool) {
        self.anim_timer = self.anim_timer.wrapping_add(1);

        let b = pad.buttons;
        let left = b.is_held(button::LEFT);
        let right = b.is_held(button::RIGHT);

        let total_cities = CITIES.len();

        if left && !self.prev_left {
            if self.selected_idx > 0 {
                self.selected_idx -= 1;
            } else {
                self.selected_idx = total_cities.saturating_sub(1);
            }
        } else if right && !self.prev_right {
            if self.selected_idx + 1 < total_cities {
                self.selected_idx += 1;
            } else {
                self.selected_idx = 0;
            }
        }

        self.prev_left = left;
        self.prev_right = right;

        let confirm_held = b.is_held(button::CROSS) || b.is_held(button::START);
        let confirmed = confirm_held && !self.prev_confirm;
        self.prev_confirm = confirm_held;

        let cancel_held = b.is_held(button::TRIANGLE);
        let cancelled = cancel_held && !self.prev_cancel;
        self.prev_cancel = cancel_held;

        if confirmed {
            (Some(&CITIES[self.selected_idx]), false)
        } else if cancelled {
            (None, true)
        } else {
            (None, false)
        }
    }

    /// Renders the city selection carousel and satellite map metadata.
    pub fn render(&self) {
        // Outer panel styling
        gpu::draw_rect_flat(14, 14, 292, 212, 12, 14, 20);
        gpu::draw_rect_flat(16, 16, 288, 208, 24, 28, 38);
        gpu::draw_rect_flat(18, 18, 284, 204, 32, 36, 48);

        // Header Title
        draw_text(64, 26, "CITY FREE DRIVE - 10KM ROAM", (255, 225, 30), 1);
        gpu::draw_rect_flat(28, 38, 264, 2, 0, 200, 255);

        if self.selected_idx >= CITIES.len() {
            return;
        }

        let city = &CITIES[self.selected_idx];

        // 1. City index indicator: "< CITY 1 / 2 >"
        let city_num = (self.selected_idx as u8) + 1;
        let mut idx_buf = [b' '; 14];
        idx_buf[0] = b'<';
        idx_buf[1] = b' ';
        idx_buf[2] = b'C';
        idx_buf[3] = b'I';
        idx_buf[4] = b'T';
        idx_buf[5] = b'Y';
        idx_buf[6] = b' ';
        idx_buf[7] = b'0' + city_num;
        idx_buf[8] = b' ';
        idx_buf[9] = b'/';
        idx_buf[10] = b' ';
        idx_buf[11] = b'0' + (CITIES.len() as u8);
        idx_buf[12] = b' ';
        idx_buf[13] = b'>';

        let mut ix = 110u16;
        for &ch in &idx_buf {
            crate::ui::font::draw_char(ix, 46, ch, (180, 200, 220), 1);
            ix += 6;
        }

        // 2. City Name Header (2x scale bold neon)
        let name_x = match self.selected_idx {
            0 => 96, // "CAPE TOWN"
            _ => 96, // "MELBOURNE"
        };
        draw_text(name_x, 60, city.name, (255, 235, 40), 2);

        // 3. Info Card Panel
        gpu::draw_rect_flat(30, 84, 260, 104, 20, 24, 32);
        gpu::draw_rect_flat(32, 86, 256, 100, 28, 34, 46);

        // Country info
        draw_text(40, 94, "COUNTRY:", (160, 180, 200), 1);
        draw_text(94, 94, city.country, (255, 255, 255), 1);

        // Landmarks info
        draw_text(40, 110, "DISTRICT:", (160, 180, 200), 1);
        draw_text(94, 110, city.landmarks, (0, 220, 255), 1);

        // Grid & Area info
        draw_text(
            40,
            128,
            "AREA: 10 KM^2 (3.16 km x 3.16 km)",
            (180, 190, 210),
            1,
        );
        draw_text(
            40,
            144,
            "GRID: 3x3 STREAMING JPEG BLOCKS",
            (255, 200, 50),
            1,
        );
        draw_text(40, 160, "VIEW: PHOTOREAL SATELLITE ORTHO", (60, 240, 90), 1);

        // Footer Navigation Hints
        draw_text(
            34,
            198,
            "[LEFT/RIGHT] BROWSE  [CROSS] DRIVE  [TRIANGLE] BACK",
            (140, 160, 180),
            1,
        );
    }
}
