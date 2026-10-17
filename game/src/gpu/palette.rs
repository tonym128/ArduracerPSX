//! Per-cup environment palettes, and the 15-bit quantisation they must survive.
//!
//! Hardware-free so `tools/test_ui` can check it; the track renderer draws
//! through `psx_gpu` and cannot be linked on the host.
//!
//! This split exists because of TASK-1205: VRAM stores colour as 5 bits per
//! channel, and `PAL_SPEEDWAY`'s `road` (44,46,52) and `road2` (40,42,48) both
//! truncate to the *identical* word `0x18A5`. On tracks 1-6 the "dark asphalt"
//! underlay of every timing gate was therefore byte-identical to the tarmac
//! around it, and the gate read as invisible. The palettes were authored as
//! 8-bit RGB with no check that the values were actually distinguishable after
//! truncation.
//!
//! [`Palette::is_distinct`] is the check that would have caught it.

/// An 8-bit RGB triple.
pub type Rgb = (u8, u8, u8);

/// Quantise to the PSX VRAM word: 5 bits per channel, packed `BBBBBGGGGGRRRRR`.
///
/// This is the truncation the hardware performs, so two colours that share a
/// result are the same colour on screen no matter how different they are as
/// 8-bit values.
pub const fn to_bgr555(c: Rgb) -> u16 {
    let (r, g, b) = c;
    (((b as u16) >> 3) << 10) | (((g as u16) >> 3) << 5) | ((r as u16) >> 3)
}

/// Per-cup environment palette (biome).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Palette {
    pub road: Rgb,
    /// The darker asphalt underlay laid beneath timing gates.
    pub road2: Rgb,
    pub grass: Rgb,
    pub patch_a: Rgb,
    pub patch_b: Rgb,
    pub curb_a: Rgb,
    pub curb_b: Rgb,
}

impl Palette {
    /// Every colour in the palette, as `(field name, colour)`.
    pub const fn entries(&self) -> [(&'static str, Rgb); 7] {
        [
            ("road", self.road),
            ("road2", self.road2),
            ("grass", self.grass),
            ("patch_a", self.patch_a),
            ("patch_b", self.patch_b),
            ("curb_a", self.curb_a),
            ("curb_b", self.curb_b),
        ]
    }

    /// The first pair of colours that collapse to the same VRAM word.
    ///
    /// No allocation: the game crate is `no_std`, and there are only 21 pairs.
    pub fn first_collision(&self) -> Option<(&'static str, &'static str)> {
        let entries = self.entries();
        let mut i = 0;
        while i < entries.len() {
            let mut j = i + 1;
            while j < entries.len() {
                if to_bgr555(entries[i].1) == to_bgr555(entries[j].1) {
                    return Some((entries[i].0, entries[j].0));
                }
                j += 1;
            }
            i += 1;
        }
        None
    }

    /// Whether every colour remains distinguishable on hardware.
    pub fn is_distinct(&self) -> bool {
        self.first_collision().is_none()
    }
}

/// Bronze: classic GP speedway.
pub const PAL_SPEEDWAY: Palette = Palette {
    road: (44, 46, 52),
    // Was (40, 42, 48), which truncates to the *identical* VRAM word as `road`:
    // the gate underlay vanished into the tarmac (TASK-1205). Re-authored so it
    // lands at least two 5-bit buckets away in every channel -- one bucket is
    // technically distinct but under 3 % of luminance, which reads as nothing.
    road2: (24, 38, 44),
    grass: (28, 62, 34),
    patch_a: (22, 52, 28),
    patch_b: (24, 55, 30),
    curb_a: (225, 30, 45),
    curb_b: (245, 245, 250),
};

/// Silver: neon-lit night city, wet midnight asphalt.
pub const PAL_NEON_CITY: Palette = Palette {
    road: (24, 26, 40),
    road2: (22, 24, 36),
    grass: (14, 18, 34),
    patch_a: (10, 14, 28),
    patch_b: (18, 12, 36),
    curb_a: (255, 40, 200),
    curb_b: (40, 230, 255),
};

/// Gold: red canyon, tan tarmac and sandstone.
pub const PAL_CANYON: Palette = Palette {
    road: (78, 66, 56),
    road2: (72, 60, 50),
    grass: (150, 82, 46),
    patch_a: (130, 68, 38),
    patch_b: (166, 98, 56),
    curb_a: (230, 120, 30),
    curb_b: (250, 235, 200),
};

/// Platinum: alpine / marina, cool blue-grey road with snowy verges.
pub const PAL_ALPINE: Palette = Palette {
    road: (52, 60, 74),
    road2: (48, 56, 70),
    grass: (206, 218, 232),
    patch_a: (180, 198, 220),
    patch_b: (226, 234, 244),
    curb_a: (30, 90, 220),
    curb_b: (250, 250, 255),
};

/// Every palette, indexed by cup.
pub const PALETTES: [Palette; 4] = [PAL_SPEEDWAY, PAL_NEON_CITY, PAL_CANYON, PAL_ALPINE];

/// Returns the ground/environment [`Palette`] for `circuit_id` (`NO1-032`).
pub const fn biome_palette_for_circuit(circuit_id: usize) -> Palette {
    let cup = (circuit_id / 6) % 4;
    PALETTES[cup]
}

/// Gouraud sky gradient, parallax horizon silhouette, and night-lighting parameters (`NO1-032`, `NO1-033`).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct SkyPalette {
    pub zenith: Rgb,
    pub mid_sky: Rgb,
    pub horizon: Rgb,
    pub silhouette_dark: Rgb,
    pub silhouette_lit: Rgb,
    pub ground_haze: Rgb,
    pub ground_deep: Rgb,
    pub is_night: bool,
    /// `0` = Coastal/Speedway, `1` = Neon City Skyline, `2` = Red Canyon Mesas,
    /// `3` = Alpine Peaks, `4` = Obsidian Cyber, `5` = TrackMania Stadium Roof.
    pub biome_style: u8,
}

pub const SKY_SPEEDWAY_SUNSET: SkyPalette = SkyPalette {
    zenith: (24, 42, 108),
    mid_sky: (95, 72, 140),
    horizon: (240, 130, 75),
    silhouette_dark: (32, 36, 62),
    silhouette_lit: (88, 64, 96),
    ground_haze: (72, 78, 92),
    ground_deep: PAL_SPEEDWAY.road,
    is_night: false,
    biome_style: 0,
};

pub const SKY_NEON_CYBER_NIGHT: SkyPalette = SkyPalette {
    zenith: (8, 10, 28),
    mid_sky: (26, 18, 58),
    horizon: (110, 32, 120),
    silhouette_dark: (12, 14, 30),
    silhouette_lit: (40, 180, 220),
    ground_haze: (28, 30, 52),
    ground_deep: PAL_NEON_CITY.road,
    is_night: true,
    biome_style: 1,
};

pub const SKY_CANYON_DUSK: SkyPalette = SkyPalette {
    zenith: (48, 28, 64),
    mid_sky: (165, 68, 48),
    horizon: (250, 175, 70),
    silhouette_dark: (76, 36, 26),
    silhouette_lit: (156, 78, 42),
    ground_haze: (125, 78, 52),
    ground_deep: PAL_CANYON.road,
    is_night: false,
    biome_style: 2,
};

pub const SKY_ALPINE_AURORA: SkyPalette = SkyPalette {
    zenith: (18, 38, 76),
    mid_sky: (46, 92, 148),
    horizon: (165, 215, 245),
    silhouette_dark: (58, 76, 106),
    silhouette_lit: (215, 230, 250),
    ground_haze: (130, 152, 178),
    ground_deep: PAL_ALPINE.road,
    is_night: false,
    biome_style: 3,
};

pub const SKY_OBSIDIAN_NIGHT: SkyPalette = SkyPalette {
    zenith: (6, 8, 18),
    mid_sky: (18, 26, 48),
    horizon: (45, 95, 135),
    silhouette_dark: (10, 14, 24),
    silhouette_lit: (0, 210, 255),
    ground_haze: (24, 34, 48),
    ground_deep: (20, 24, 34),
    is_night: true,
    biome_style: 4,
};

pub const SKY_MANIA_STADIUM: SkyPalette = SkyPalette {
    zenith: (14, 28, 68),
    mid_sky: (32, 68, 132),
    horizon: (90, 175, 240),
    silhouette_dark: (24, 38, 64),
    silhouette_lit: (60, 230, 140),
    ground_haze: (44, 62, 86),
    ground_deep: (36, 48, 64),
    is_night: false,
    biome_style: 5,
};

/// Returns the biome [`SkyPalette`] for `circuit_id` (`NO1-032`, `NO1-033`).
pub const fn sky_palette_for_circuit(circuit_id: usize) -> SkyPalette {
    if circuit_id >= 24 && circuit_id < 28 {
        SKY_MANIA_STADIUM
    } else if circuit_id >= 28 {
        // City free-roam maps
        SKY_SPEEDWAY_SUNSET
    } else {
        match circuit_id / 6 {
            0 => SKY_SPEEDWAY_SUNSET,
            1 => SKY_NEON_CYBER_NIGHT,
            2 => SKY_CANYON_DUSK,
            3 => SKY_ALPINE_AURORA,
            _ => SKY_OBSIDIAN_NIGHT,
        }
    }
}
