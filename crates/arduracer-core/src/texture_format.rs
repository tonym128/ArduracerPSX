//! Player-selectable circuit texture format and frame pacing.
//!
//! Pure policy: no hardware, no statics, no state. It lives here rather than in
//! `game/src/gpu/tracktex.rs` because these are exactly the decisions that need
//! host tests -- the pinned-format-to-decoder mapping in particular shipped
//! once with `_ => Codec::Qoi` as its fallback arm, so selecting JPEG streamed a
//! QOI block through the JPEG decoder. That decodes without panicking and only
//! produces wrong pixels, so nothing at the call site would have caught it.

/// Which compressed format a circuit's 1024x1024 block is stored as.
///
/// Ordering is the preference order used by `Auto`: best first. SVG wins because
/// it is the only one that carries the authored vector geometry and synthesises
/// detail at decode; QOI is the lossless copy of the same raster; JPEG is the
/// always-present fallback for art too detailed to fit the 500 KB block.
#[derive(Copy, Clone, PartialEq, Eq, Debug, Default)]
pub enum TextureFormat {
    #[default]
    Auto,
    Svg,
    Qoi,
    Jpeg,
}

/// Which decoder serves each pinned format.
///
/// Kept separate from [`TextureFormat`] so the mapping is one total function
/// with a test, rather than an inline match buried in the selection path where
/// a wrong arm compiles fine and decodes garbage.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Codec {
    Jpeg,
    Qoi,
    /// SVG-derived base raster plus a per-surface detail pass.
    Arsv,
}

impl TextureFormat {
    /// Cycles to the next option, wrapping. Used by both menus.
    pub fn next(self) -> Self {
        match self {
            Self::Auto => Self::Svg,
            Self::Svg => Self::Qoi,
            Self::Qoi => Self::Jpeg,
            Self::Jpeg => Self::Auto,
        }
    }

    /// The label the menus render.
    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => "AUTO",
            Self::Svg => "SVG",
            Self::Qoi => "QOI",
            Self::Jpeg => "JPEG",
        }
    }

    /// Maps a persisted byte onto a format.
    ///
    /// Total by construction: a corrupt card cannot name a variant that does not
    /// exist, because everything outside the known range becomes `Auto`.
    pub fn from_saved(v: u8) -> Self {
        match v {
            1 => Self::Svg,
            2 => Self::Qoi,
            3 => Self::Jpeg,
            _ => Self::Auto,
        }
    }

    /// The byte written to the save file.
    pub fn to_saved(self) -> u8 {
        match self {
            Self::Auto => 0,
            Self::Svg => 1,
            Self::Qoi => 2,
            Self::Jpeg => 3,
        }
    }

    /// The decoder that serves this format when it is pinned.
    ///
    /// Every pin maps to its own decoder. `Auto` never reaches this in
    /// practice -- selection filters it out and walks the preference order --
    /// but it maps to JPEG rather than panicking, because JPEG is the one format
    /// the disc always carries.
    pub const fn codec(self) -> Codec {
        match self {
            Self::Svg => Codec::Arsv,
            Self::Qoi => Codec::Qoi,
            Self::Jpeg | Self::Auto => Codec::Jpeg,
        }
    }
}

/// Player-selectable framerate.
///
/// 30 FPS is the historical lock, in which a frame occupies two VBlank periods.
/// 60 FPS halves that, which only holds because the texture decode budget is
/// time-sliced and gives rows back when a frame is already behind.
#[derive(Copy, Clone, PartialEq, Eq, Debug, Default)]
pub enum Framerate {
    Fps30,
    #[default]
    Fps60,
}

/// One VBlank period in CPU cycles.
///
/// NTSC runs the CPU at 33.8688 MHz and a VBlank is 1/60 s, so one period is
/// 564,480 cycles. This is the 60 FPS frame budget.
pub const VBLANK_CYCLES_NTSC: u32 = 564_480;

impl Framerate {
    /// VBlank periods a single frame may occupy.
    pub const fn vblanks_per_frame(self) -> u32 {
        match self {
            Self::Fps30 => 2,
            Self::Fps60 => 1,
        }
    }

    /// Cycles available to one frame.
    pub const fn budget_cycles(self) -> u32 {
        self.vblanks_per_frame() * VBLANK_CYCLES_NTSC
    }

    /// The label the menus render.
    pub fn label(self) -> &'static str {
        match self {
            Self::Fps30 => "30 FPS",
            Self::Fps60 => "60 FPS",
        }
    }

    /// Maps a persisted byte onto a mode.
    ///
    /// 60 is the default: measured frame work is ~86% of the 60 FPS budget once
    /// texture decode is excluded, and 30 stays one toggle away.
    pub fn from_saved(v: u8) -> Self {
        match v {
            1 => Self::Fps30,
            _ => Self::Fps60,
        }
    }

    /// The byte written to the save file.
    pub fn to_saved(self) -> u8 {
        match self {
            Self::Fps30 => 1,
            Self::Fps60 => 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The mapping this module exists to protect: each pin serves its own
    /// decoder, so no two options are silently the same thing.
    #[test]
    fn each_pinned_format_selects_its_own_decoder() {
        assert_eq!(TextureFormat::Svg.codec(), Codec::Arsv);
        assert_eq!(TextureFormat::Qoi.codec(), Codec::Qoi);
        assert_eq!(TextureFormat::Jpeg.codec(), Codec::Jpeg);
    }

    #[test]
    fn no_two_pins_share_a_decoder() {
        let all = [
            TextureFormat::Svg.codec(),
            TextureFormat::Qoi.codec(),
            TextureFormat::Jpeg.codec(),
        ];
        for i in 0..all.len() {
            for j in (i + 1)..all.len() {
                assert_ne!(all[i], all[j], "two formats share decoder {:?}", all[i]);
            }
        }
    }

    #[test]
    fn texture_format_bytes_round_trip_and_default_safely() {
        for f in [
            TextureFormat::Auto,
            TextureFormat::Svg,
            TextureFormat::Qoi,
            TextureFormat::Jpeg,
        ] {
            assert_eq!(TextureFormat::from_saved(f.to_saved()), f);
        }
        for b in 4u8..=255 {
            assert_eq!(TextureFormat::from_saved(b), TextureFormat::Auto);
        }
    }

    #[test]
    fn framerate_bytes_round_trip_and_default_safely() {
        for f in [Framerate::Fps30, Framerate::Fps60] {
            assert_eq!(Framerate::from_saved(f.to_saved()), f);
        }
        for b in 2u8..=255 {
            assert_eq!(Framerate::from_saved(b), Framerate::Fps60);
        }
    }

    /// Budget and period must stay in step; getting this backwards would halve
    /// or double the entire pacing budget.
    #[test]
    fn framerate_periods_and_budgets_agree() {
        assert_eq!(Framerate::Fps60.vblanks_per_frame(), 1);
        assert_eq!(Framerate::Fps30.vblanks_per_frame(), 2);
        assert_eq!(Framerate::Fps60.budget_cycles(), 564_480);
        assert_eq!(Framerate::Fps30.budget_cycles(), 1_128_960);
        assert_eq!(
            Framerate::Fps30.budget_cycles(),
            2 * Framerate::Fps60.budget_cycles()
        );
    }

    /// Cycling must reach every option and return, or a player could get stuck
    /// on one setting.
    #[test]
    fn cycling_visits_every_option_and_wraps() {
        let mut f = TextureFormat::Auto;
        let mut seen = vec![f];
        for _ in 0..4 {
            f = f.next();
            seen.push(f);
        }
        assert_eq!(
            seen,
            vec![
                TextureFormat::Auto,
                TextureFormat::Svg,
                TextureFormat::Qoi,
                TextureFormat::Jpeg,
                TextureFormat::Auto,
            ]
        );
    }
}
