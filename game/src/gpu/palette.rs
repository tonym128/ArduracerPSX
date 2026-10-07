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
