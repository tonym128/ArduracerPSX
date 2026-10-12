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
///
/// TIM is deliberately *not* in the `Auto` order. It is lossless and needs no
/// decode at all, which sounds strictly better, but it is four times the disc
/// footprint and it reads four sectors per VRAM tile instead of one 500 KB
/// block per 256 tiles, so it loses badly on drive contention -- which is
/// audible as CD-DA stutter. It is offered as a deliberate player choice, on
/// the same footing as the other pins, and never chosen for them.
#[derive(Copy, Clone, PartialEq, Eq, Debug, Default)]
pub enum TextureFormat {
    #[default]
    Auto,
    Svg,
    Qoi,
    Jpeg,
    Tim,
    /// Uncompressed 4bpp, CLUT-indexed: a 16-colour palette per circuit.
    Tim4,
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
    /// Uncompressed 15bpp tile, DMA'd from RAM straight into VRAM.
    Tim,
    /// Uncompressed 4bpp CLUT-indexed tile, DMA'd the same way.
    Tim4,
}

impl TextureFormat {
    /// Cycles to the next option, wrapping.
    ///
    /// Walks the persisted bytes in numeric order, and must stay that way.
    /// Both menus cycle `texture_format` as a raw byte rather than going through
    /// this function, so a "nicer" order here would be a second, invisible
    /// source of truth describing a cycle the player never sees -- and the only
    /// way to notice the two had drifted would be to read both files.
    ///
    /// Note this is *display* order, not the `Auto` preference order above;
    /// they are unrelated and both deliberate.
    pub fn next(self) -> Self {
        match self {
            Self::Auto => Self::Svg,
            Self::Svg => Self::Qoi,
            Self::Qoi => Self::Jpeg,
            Self::Jpeg => Self::Tim,
            Self::Tim => Self::Tim4,
            Self::Tim4 => Self::Auto,
        }
    }

    /// The label the menus render.
    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => "AUTO",
            Self::Svg => "SVG",
            Self::Qoi => "QOI",
            Self::Tim => "TIM",
            Self::Tim4 => "TIM4",
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
            4 => Self::Tim,
            5 => Self::Tim4,
            _ => Self::Auto,
        }
    }

    /// The byte written to the save file.
    ///
    /// Each uncompressed format took the next free byte, appended rather than
    /// inserted into the sequence, so every byte an existing card can already
    /// hold keeps its meaning. Renumbering would have silently reinterpreted a
    /// player's saved JPEG choice as a different format, which decodes to
    /// garbage rather than failing.
    pub fn to_saved(self) -> u8 {
        match self {
            Self::Auto => 0,
            Self::Svg => 1,
            Self::Qoi => 2,
            Self::Jpeg => 3,
            Self::Tim => 4,
            Self::Tim4 => 5,
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
            Self::Tim => Codec::Tim,
            Self::Tim4 => Codec::Tim4,
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
        assert_eq!(TextureFormat::Tim.codec(), Codec::Tim);
        assert_eq!(TextureFormat::Tim4.codec(), Codec::Tim4);
        assert_eq!(TextureFormat::Jpeg.codec(), Codec::Jpeg);
    }

    #[test]
    fn no_two_pins_share_a_decoder() {
        let all = [
            TextureFormat::Svg.codec(),
            TextureFormat::Qoi.codec(),
            TextureFormat::Tim.codec(),
            TextureFormat::Tim4.codec(),
            TextureFormat::Jpeg.codec(),
        ];
        for i in 0..all.len() {
            for j in (i + 1)..all.len() {
                assert_ne!(all[i], all[j], "two formats share decoder {:?}", all[i]);
            }
        }
    }

    /// Adding TIM4 must not renumber anything that came before it, for the same
    /// reason TIM's byte was appended rather than inserted.
    #[test]
    fn adding_tim4_did_not_renumber_the_existing_formats() {
        assert_eq!(TextureFormat::Auto.to_saved(), 0);
        assert_eq!(TextureFormat::Svg.to_saved(), 1);
        assert_eq!(TextureFormat::Qoi.to_saved(), 2);
        assert_eq!(TextureFormat::Jpeg.to_saved(), 3);
        assert_eq!(TextureFormat::Tim.to_saved(), 4);
        assert_eq!(TextureFormat::Tim4.to_saved(), 5);
        for f in [
            TextureFormat::Auto,
            TextureFormat::Svg,
            TextureFormat::Qoi,
            TextureFormat::Jpeg,
            TextureFormat::Tim,
        ] {
            assert_eq!(TextureFormat::from_saved(f.to_saved()), f);
        }
    }

    #[test]
    fn the_tim4_pin_takes_the_tim4_path_only_when_the_block_is_there() {
        assert!(selects_tim4(TextureFormat::Tim4, true));
        assert!(
            !selects_tim4(TextureFormat::Tim4, false),
            "a pin to TIM4 on a disc without TIM4TRACKS.BIN must fall back"
        );
    }

    /// Two uncompressed formats on one disc is the case the availability/selection
    /// split exists for: a disc can carry both, so "a TIM block exists" and "a
    /// TIM4 block exists" are both true everywhere and neither identifies which
    /// path a circuit took.
    #[test]
    fn the_two_uncompressed_pins_never_select_each_other() {
        // Both assets present, which is the normal disc: neither availability
        // flag distinguishes them, so only the pin can.
        assert!(
            !selects_tim4(TextureFormat::Tim, true),
            "the 15bpp pin must not take the 4bpp path even with both assets present"
        );
        assert!(
            !selects_tim(TextureFormat::Tim4, true),
            "the 4bpp pin must not take the 15bpp path even with both assets present"
        );
        // And each still selects itself.
        assert!(selects_tim(TextureFormat::Tim, true));
        assert!(selects_tim4(TextureFormat::Tim4, true));
    }

    #[test]
    fn texture_format_bytes_round_trip_and_default_safely() {
        for f in [
            TextureFormat::Auto,
            TextureFormat::Svg,
            TextureFormat::Qoi,
            TextureFormat::Jpeg,
            TextureFormat::Tim,
            TextureFormat::Tim4,
        ] {
            assert_eq!(TextureFormat::from_saved(f.to_saved()), f);
        }
        for b in 6u8..=255 {
            assert_eq!(TextureFormat::from_saved(b), TextureFormat::Auto);
        }
    }

    /// TIM was appended as byte 4 rather than inserted into the sequence, so a
    /// card written before it existed must still read back as what it meant.
    /// Renumbering would reinterpret a saved JPEG choice as TIM, which decodes
    /// to garbage rather than failing loudly.
    #[test]
    fn adding_tim_did_not_renumber_the_existing_formats() {
        assert_eq!(TextureFormat::Auto.to_saved(), 0);
        assert_eq!(TextureFormat::Svg.to_saved(), 1);
        assert_eq!(TextureFormat::Qoi.to_saved(), 2);
        assert_eq!(TextureFormat::Jpeg.to_saved(), 3);
        assert_eq!(TextureFormat::Tim.to_saved(), 4);
        for f in [
            TextureFormat::Auto,
            TextureFormat::Svg,
            TextureFormat::Qoi,
            TextureFormat::Jpeg,
        ] {
            assert_eq!(TextureFormat::from_saved(f.to_saved()), f);
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
        for _ in 0..6 {
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
                TextureFormat::Tim,
                TextureFormat::Tim4,
                TextureFormat::Auto,
            ]
        );
    }

    /// Both menus cycle the persisted byte with `rem_euclid`, not this
    /// function, so the two orderings have to agree or the enum's cycle is a
    /// description of something the player never sees.
    #[test]
    fn cycling_visits_the_persisted_bytes_in_order() {
        let mut f = TextureFormat::Auto;
        let mut by_cycle = Vec::new();
        for _ in 0..6 {
            f = f.next();
            by_cycle.push(f.to_saved());
        }

        // Exactly what a menu does: `(byte + 1) % (MAX + 1)`.
        let mut byte = 0u8;
        let mut by_byte = Vec::new();
        for _ in 0..6 {
            byte = (byte + 1) % 6;
            by_byte.push(byte);
        }

        assert_eq!(
            by_cycle, by_byte,
            "TextureFormat::next and the menus' byte cycle must be the same order"
        );
    }
}

/// Whether a circuit is served by the uncompressed 15bpp (TIM) path.
///
/// Takes the pin and whether the disc actually carries the block, and answers
/// the question the *renderer* must ask before choosing a decode path.
///
/// This exists because asking the availability question alone is wrong, and
/// was: the decode paths branched on "does a TIM block exist for this circuit",
/// which is true for every circuit on any disc mastered with `TIMTRACKS.BIN`.
/// Circuits the player had left on QOI, JPEG or SVG all took the TIM branch
/// anyway, so the RAM block was loaded and then never decoded from, every tile
/// miss became a fresh four-sector CD read, and CD-DA lost the drive to it
/// continuously. Availability is a necessary condition here, not a sufficient
/// one.
///
/// `Auto` is false even when a TIM block exists, which is the whole point: TIM
/// is never selected on the player's behalf.
pub const fn selects_tim(pinned: TextureFormat, tim_available: bool) -> bool {
    tim_available && selects_codec(pinned, Codec::Tim)
}

/// Whether a circuit is served by the uncompressed 4bpp (TIM4) path.
///
/// Same rule as [`selects_tim`] with a different decoder, and it exists for the
/// same reason. TIM and TIM4 are separate assets and separate pins; a disc may
/// carry one without the other, so each availability flag is its own.
pub const fn selects_tim4(pinned: TextureFormat, tim4_available: bool) -> bool {
    tim4_available && selects_codec(pinned, Codec::Tim4)
}

/// Whether `pinned` names `want`, ignoring availability.
///
/// Shared by both uncompressed predicates so "does the pin name this decoder"
/// has one definition rather than one per format. `matches!` rather than `==`:
/// `PartialEq` is not const.
const fn selects_codec(pinned: TextureFormat, want: Codec) -> bool {
    matches!(
        (pinned.codec(), want),
        (Codec::Tim, Codec::Tim) | (Codec::Tim4, Codec::Tim4)
    )
}

/// The decode-budget tiers, keyed to VBlanks elapsed since the frame's step-1
/// stamp.
///
/// This is the exact arithmetic that shipped wrong. At 60 FPS the period is 1,
/// so a condition written as `elapsed + 1 >= period` is true on the first
/// iteration of every frame and the tier collapses to "admit nothing" -- the
/// tile cache never fills and every visible tile renders as a placeholder
/// (measured: 68,616 placeholders at 60 FPS versus 172 once fixed). The same
/// expression is harmless at 30 FPS, which is why the bug survived a merge.
///
/// It lives here, in `core`, for the same reason the codec mapping does: the
/// real loop is in `game/src/gpu/tracktex.rs`, where nothing can test it.
pub fn decode_row_budget(ticks_to_run: u32, elapsed_vblanks: u32, period: u32) -> usize {
    let period = if period == 0 { 1 } else { period };
    if ticks_to_run >= 3 || elapsed_vblanks >= period {
        0
    } else if ticks_to_run == 2 || elapsed_vblanks + 1 >= period {
        1
    } else {
        period as usize
    }
}

#[cfg(test)]
mod budget_tests {
    use super::*;

    /// The regression this module's `selects_tim` was added for: a disc that
    /// carries TIM blocks must not make *every* circuit take the TIM path.
    ///
    /// The broken version asked only "is a TIM block available", which is true
    /// for all of them, and the renderer then streamed raw texels off the disc
    /// instead of decoding the RAM block it had already loaded.
    #[test]
    fn only_the_tim_pin_selects_the_tim_path() {
        for f in [
            TextureFormat::Auto,
            TextureFormat::Svg,
            TextureFormat::Qoi,
            TextureFormat::Jpeg,
            TextureFormat::Tim4,
        ] {
            assert!(
                !selects_tim(f, true),
                "{f:?} must not take the TIM path just because TIM exists"
            );
            assert!(
                !selects_tim(f, false),
                "{f:?} must not take the TIM path without a TIM block either"
            );
        }
    }

    #[test]
    fn only_the_tim4_pin_selects_the_tim4_path() {
        for f in [
            TextureFormat::Auto,
            TextureFormat::Svg,
            TextureFormat::Qoi,
            TextureFormat::Jpeg,
            TextureFormat::Tim,
        ] {
            assert!(
                !selects_tim4(f, true),
                "{f:?} must not take the TIM4 path just because TIM4 exists"
            );
            assert!(
                !selects_tim4(f, false),
                "{f:?} must not take the TIM4 path without a block either"
            );
        }
    }

    #[test]
    fn the_tim_pin_takes_the_tim_path_only_when_the_block_is_there() {
        assert!(selects_tim(TextureFormat::Tim, true));
        assert!(
            !selects_tim(TextureFormat::Tim, false),
            "a pin to TIM on a disc without TIMTRACKS.BIN must fall back, not half-work"
        );
    }

    /// Availability must not be load-bearing on its own. Every circuit on a
    /// TIM-bearing disc reports available, so a predicate reading only that
    /// answers yes for the whole disc. This is that predicate, stated as the
    /// contrast: it is true for every pin, `selects_tim` is true for one.
    #[test]
    fn availability_alone_answers_yes_for_the_whole_disc() {
        for f in [
            TextureFormat::Auto,
            TextureFormat::Svg,
            TextureFormat::Qoi,
            TextureFormat::Jpeg,
        ] {
            let availability_check = true; // "a TIM block exists for this circuit"
            assert!(availability_check);
            assert!(
                !selects_tim(f, availability_check),
                "{f:?} must not take the TIM path merely because one exists"
            );
        }
    }

    /// 60 FPS is the regression: one row must be admitted on the first
    /// iteration of a frame, which is `elapsed == 0`, `period == 1`.
    #[test]
    fn sixty_fps_admits_a_row_at_the_start_of_every_frame() {
        assert_eq!(decode_row_budget(1, 0, 1), 1, "60 FPS must admit a row");
    }

    /// Once the frame's single VBlank has elapsed there is no time left.
    #[test]
    fn sixty_fps_admits_nothing_once_the_vblank_has_passed() {
        assert_eq!(decode_row_budget(1, 1, 1), 0);
    }

    /// 30 FPS keeps exactly the behaviour it had before the generalisation:
    /// two rows early in the frame, one in its second half, none at the end.
    #[test]
    fn thirty_fps_matches_its_original_budget() {
        assert_eq!(decode_row_budget(1, 0, 2), 2);
        assert_eq!(decode_row_budget(1, 1, 2), 1);
        assert_eq!(decode_row_budget(1, 2, 2), 0);
    }

    /// Simulation pressure overrides pacing: a catch-up frame does no decoding.
    #[test]
    fn simulation_pressure_still_wins_over_pacing() {
        assert_eq!(decode_row_budget(3, 0, 1), 0);
        assert_eq!(decode_row_budget(4, 0, 2), 0);
        assert_eq!(decode_row_budget(2, 0, 1), 1);
        assert_eq!(decode_row_budget(2, 0, 2), 1);
    }

    /// Whatever the mode, the first iteration of a frame must admit work
    /// whenever the frame is not already over. This is the invariant whose
    /// violation produced the 68,616-placeholder build.
    #[test]
    fn a_frame_never_starts_admitting_nothing_while_it_has_time() {
        for period in [1u32, 2] {
            assert_ne!(
                decode_row_budget(1, 0, period),
                0,
                "period {period} admits nothing at the start of a frame"
            );
        }
    }

    /// A zero period must not divide or panic; it is treated as one VBlank.
    #[test]
    fn a_zero_period_degrades_to_one_vblank() {
        assert_eq!(decode_row_budget(1, 0, 0), 1);
        assert_eq!(decode_row_budget(1, 1, 0), 0);
    }
}
