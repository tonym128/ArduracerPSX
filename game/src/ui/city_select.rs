//! City Selection Screen for Free Urban Driving in Arduracer PSX.
//!
//! Allows browsing the 7 real-world cities (Cape Town, Melbourne, London,
//! Sydney, New York, Tokyo, Singapore) with OpenStreetMap GPS coordinates,
//! district descriptions, and satellite map parameters.

use crate::ui::font::draw_text;
use arduracer_core::city_data::ALL_CITIES;
use arduracer_core::CityDef;
use psx_gpu as gpu;
use psx_pad::{button, PadState};

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

    /// Handles city selection navigation. Returns `(Option<usize>, bool)`:
    /// - `(Some(city_idx), false)` on confirmation (CROSS / START)
    /// - `(None, true)` on cancel (TRIANGLE)
    /// - `(None, false)` while navigating
    pub fn update(&mut self, pad: &PadState) -> (Option<usize>, bool) {
        self.anim_timer = self.anim_timer.wrapping_add(1);

        let b = pad.buttons;
        let left = b.is_held(button::LEFT);
        let right = b.is_held(button::RIGHT);

        let total_cities = ALL_CITIES.len();

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
            (Some(self.selected_idx), false)
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
        draw_text(68, 26, "EXPLORE CITIES - SATELLITE GPS", (255, 225, 30), 1);
        gpu::draw_rect_flat(28, 38, 264, 2, 0, 200, 255);

        if self.selected_idx >= ALL_CITIES.len() {
            return;
        }

        let city: &CityDef = ALL_CITIES[self.selected_idx];

        // 1. City index indicator: "< CITY 1 / 7 >"
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
        idx_buf[11] = b'0' + (ALL_CITIES.len() as u8);
        idx_buf[12] = b' ';
        idx_buf[13] = b'>';

        let mut ix = 110u16;
        for &ch in &idx_buf {
            crate::ui::font::draw_char(ix, 46, ch, (180, 200, 220), 1);
            ix += 6;
        }

        // 2. City Name Header (2x scale bold neon)
        let name_x = match self.selected_idx {
            0 => 96,  // "CAPE TOWN"
            1 => 96,  // "MELBOURNE"
            2 => 110, // "LONDON"
            3 => 110, // "SYDNEY"
            4 => 96,  // "NEW YORK"
            5 => 114, // "TOKYO"
            _ => 96,  // "SINGAPORE"
        };
        draw_text(name_x, 60, city.name, (255, 235, 40), 2);

        // 3. Info Card Panel
        gpu::draw_rect_flat(30, 84, 260, 104, 20, 24, 32);
        gpu::draw_rect_flat(32, 86, 256, 100, 28, 34, 46);

        // Country & District info
        let country_label = match self.selected_idx {
            0 => "COUNTRY: SOUTH AFRICA (ATLANTIC COAST)",
            1 => "COUNTRY: AUSTRALIA (PORT PHILLIP & YARRA)",
            2 => "COUNTRY: UNITED KINGDOM (THAMES RIVER)",
            3 => "COUNTRY: AUSTRALIA (PORT JACKSON HARBOUR)",
            4 => "COUNTRY: UNITED STATES (MANHATTAN ISLAND)",
            5 => "COUNTRY: JAPAN (TOKYO BAY & EXPRESSWAY)",
            _ => "COUNTRY: SINGAPORE (MARINA BAY RES.)",
        };
        draw_text(40, 94, country_label, (255, 255, 255), 1);

        // Active District / Circuit
        if let Some(first_race) = city.races.first() {
            draw_text(40, 108, "DISTRICT:", (160, 180, 200), 1);
            draw_text(100, 108, first_race.district, (0, 220, 255), 1);

            draw_text(40, 122, "CIRCUIT:", (160, 180, 200), 1);
            draw_text(100, 122, first_race.name, (255, 200, 50), 1);
        }

        // Real-world OpenStreetMap GPS Coordinates
        let (start_x, start_y) = if let Some(race) = city.races.first() {
            (race.start_pos.x, race.start_pos.y)
        } else {
            (arduracer_core::Fixed::ZERO, arduracer_core::Fixed::ZERO)
        };
        let (lat_e7, lon_e7) = city.world_to_gps(start_x, start_y);

        let mut gps_buf = [b' '; 32];
        let gps_len = CityDef::format_gps_into(lat_e7, lon_e7, &mut gps_buf);

        draw_text(40, 138, "OSM GPS:", (160, 180, 200), 1);
        let mut gx = 100u16;
        for &b in &gps_buf[..gps_len] {
            crate::ui::font::draw_char(gx, 138, b, (120, 255, 140), 1);
            gx += 6;
        }

        // Map Size & Satellite Ortho view specs
        draw_text(
            40,
            154,
            "AREA: 128x128 CELLS (4096m x 4096m)",
            (180, 190, 210),
            1,
        );
        draw_text(
            40,
            168,
            "VIEW: PHOTOREAL SATELLITE ORTHO",
            (255, 140, 50),
            1,
        );

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
