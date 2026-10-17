// SPDX-License-Identifier: GPL-2.0-or-later
//! Runtime silicon-class detection for the DMA controller.
//!
//! Real CL1/CL2 PS1 silicon can wedge a DMA channel: the start bit latches and
//! the channel stays busy forever while moving nothing. The PS1 has no
//! CPU-readable "revision" register, so the class is detected behaviorally --
//! the first transfer on a channel is probed with a *bounded* completion wait,
//! and if the channel is still busy when the budget runs out the channel is
//! marked [`DmaClass::Wedges`] and every later transfer takes the non-DMA
//! fallback (the GP0 FIFO for the GPU channel). The state is sticky -- probed
//! once, read thereafter.
//!
//! The CD-ROM (ch3) channel is probed the same way. Its fallback is the
//! byte-FIFO PIO reader rather than the GP0 FIFO: the CD data FIFO is a
//! push-through stream, so a transfer that never consumed it leaves the next
//! sector misaligned. The documented wedge behaviour -- "the start bit latches
//! and nothing moves" -- is what makes that fallback safe: re-arming BFRD and
//! popping the same sector by hand recovers cleanly from a wedge, because the
//! failed DMA left the FIFO exactly where PIO expects it.
//!
//! The state is plain `u8` statics, not atomics: the `mipsel-sony-psx` target
//! spec has `max-atomic-width = 0`, so `core::sync::atomic` offers nothing on
//! the device. A byte load/store is all the synchronisation the guest needs --
//! the R3000 is single-core, the only other thread of control is the VBlank
//! IRQ, and it never touches this state. Same pattern as [`disc_base`].
//!
//! The host cannot drive the DMA controller, so the decision layer
//! ([`DmaClass`], [`silicon_report`]) is the pure, testable core; the actual
//! probe (the bounded wait inside the transfer) is driven by the callers on
//! the target.

const S_UNKNOWN: u8 = 0;
const S_WORKS: u8 = 1;
const S_WEDGES: u8 = 2;
const S_DISABLED: u8 = 3;

/// Detected DMA capability for one channel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DmaClass {
    /// Not probed yet. The first transfer on the channel is the probe.
    Unknown,
    /// The channel completes transfers (safe silicon).
    Works,
    /// The channel wedges (CL1/CL2): a bounded completion wait timed out.
    Wedges,
    /// The channel is off by design; the non-DMA path is used.
    Disabled,
}

impl DmaClass {
    /// Whether this class drives the DMA path.
    ///
    /// [`Unknown`] counts as enabled: the first transfer is the probe, and the
    /// bounded completion wait inside it is the safety net (it cannot hang).
    #[inline]
    pub fn dma_enabled(self) -> bool {
        matches!(self, DmaClass::Unknown | DmaClass::Works)
    }

    /// True once the channel is known to wedge. Used to short-circuit: skip the
    /// DMA and go straight to the fallback so a stuck channel is never re-armed.
    #[inline]
    pub fn known_wedge(self) -> bool {
        self == DmaClass::Wedges
    }

    fn from_state(s: u8) -> DmaClass {
        match s {
            S_WORKS => DmaClass::Works,
            S_WEDGES => DmaClass::Wedges,
            S_DISABLED => DmaClass::Disabled,
            _ => DmaClass::Unknown,
        }
    }
}

/// Sticky state for the GPU (ch2) RAM↔VRAM DMA channel.
///
/// Forced into `.data` rather than `.bss` on the device: a chain-loaded blob
/// is linked as a bare `.text` image with no `.bss` to zero, so the probe must
/// start from the compiled `Unknown`, not from whatever the previous program
/// left in that memory.
#[cfg_attr(target_arch = "mips", link_section = ".data")]
static mut GPU_DMA: u8 = S_UNKNOWN;
/// Sticky state for the CD-ROM (ch3) DMA channel. Starts unprobed like
/// [`GPU_DMA`], for the same reason: a chain-loaded image has no `.bss` to
/// zero, so the probe must start from the compiled value.
#[cfg_attr(target_arch = "mips", link_section = ".data")]
static mut CD_DMA: u8 = S_UNKNOWN;
/// Sectors the CD reader has moved, split by the path that moved them.
///
/// Diagnostic totals only, and the direct evidence that ch3 is doing what the
/// boot report claims: a probe that passed but a count that never moves means
/// the reader is silently taking the fallback on every sector.
#[cfg_attr(target_arch = "mips", link_section = ".data")]
static mut CD_DMA_SECTORS: u32 = 0;
#[cfg_attr(target_arch = "mips", link_section = ".data")]
static mut CD_PIO_SECTORS: u32 = 0;

/// The class of the GPU (ch2) RAM↔VRAM DMA channel.
#[inline]
pub fn gpu_dma_class() -> DmaClass {
    // SAFETY: written only by `gpu_dma_mark_*` from the transfer path, never
    // from an IRQ; the guest is single-core, so the byte read is atomic.
    DmaClass::from_state(unsafe { GPU_DMA })
}

/// The class of the CD-ROM (ch3) DMA channel. [`DmaClass::Unknown`] until the
/// first sector read probes it.
#[inline]
pub fn cd_dma_class() -> DmaClass {
    // SAFETY: written only by `cd_dma_mark_*` from the sector-read path, never
    // from an IRQ; see `gpu_dma_class`.
    DmaClass::from_state(unsafe { CD_DMA })
}

/// Sectors the CD reader has moved so far, as `(via_ch3_dma, via_byte_fifo)`.
///
/// The two sum to every sector the reader delivered, so `via_ch3_dma` is the
/// live fraction of disc traffic the DMA controller is actually moving -- which
/// is a strictly stronger claim than the sticky class, which only says the probe
/// passed once.
#[inline]
pub fn cd_sector_counts() -> (u32, u32) {
    // SAFETY: written only by `cd_count_sector` from the sector-read path; see
    // `cd_dma_class`.
    unsafe { (CD_DMA_SECTORS, CD_PIO_SECTORS) }
}

/// Record that one sector reached RAM. `true` if ch3 moved it, `false` if the
/// byte-FIFO reader popped it.
#[inline]
pub fn cd_count_sector(via_dma: bool) {
    // SAFETY: written only by the sector-read path, never from an IRQ. Not
    // atomic (this target has `max-atomic-width = 0`) and does not need to be:
    // the guest is single-core and these are diagnostic totals, so a lost
    // increment would be a wrong *number*, never a wrong transfer.
    unsafe {
        if via_dma {
            CD_DMA_SECTORS = CD_DMA_SECTORS.wrapping_add(1);
        } else {
            CD_PIO_SECTORS = CD_PIO_SECTORS.wrapping_add(1);
        }
    }
}

/// Mark the GPU (ch2) channel working after a bounded completion wait
/// succeeded (the probe passed).
#[inline]
pub fn gpu_dma_mark_works() {
    // SAFETY: single-core guest; the probe writes the byte once, readers see
    // either the old or the new value, and the sticky semantics tolerate both.
    unsafe { GPU_DMA = S_WORKS };
}

/// Mark the GPU (ch2) channel wedged after a bounded completion wait timed out
/// (the probe convicted it). Every later transfer takes the FIFO path.
#[inline]
pub fn gpu_dma_mark_wedged() {
    // SAFETY: as `gpu_dma_mark_works`.
    unsafe { GPU_DMA = S_WEDGES };
}

/// Mark the CD-ROM (ch3) channel working after a sector transfer completed
/// within its bounded wait (the probe passed).
#[inline]
pub fn cd_dma_mark_works() {
    // SAFETY: as `gpu_dma_mark_works`.
    unsafe { CD_DMA = S_WORKS };
}

/// Mark the CD-ROM (ch3) channel wedged after a sector transfer stayed busy
/// for the whole bounded wait. Every later sector read takes the byte-FIFO PIO
/// path.
#[inline]
pub fn cd_dma_mark_wedged() {
    // SAFETY: as `gpu_dma_mark_works`.
    unsafe { CD_DMA = S_WEDGES };
}

/// A one-shot silicon report for the host/boot diagnostic to log.
///
/// Returns the per-channel classes: `(gpu_ch2, cd_ch3)`.
#[inline]
pub fn silicon_report() -> (DmaClass, DmaClass) {
    (gpu_dma_class(), cd_dma_class())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dma_enabled_semantics() {
        assert!(DmaClass::Unknown.dma_enabled());
        assert!(DmaClass::Works.dma_enabled());
        assert!(!DmaClass::Wedges.dma_enabled());
        assert!(!DmaClass::Disabled.dma_enabled());
    }

    #[test]
    fn known_wedge_semantics() {
        assert!(DmaClass::Wedges.known_wedge());
        assert!(!DmaClass::Unknown.known_wedge());
        assert!(!DmaClass::Works.known_wedge());
        assert!(!DmaClass::Disabled.known_wedge());
    }

    #[test]
    fn state_roundtrip() {
        assert_eq!(DmaClass::from_state(S_UNKNOWN), DmaClass::Unknown);
        assert_eq!(DmaClass::from_state(S_WORKS), DmaClass::Works);
        assert_eq!(DmaClass::from_state(S_WEDGES), DmaClass::Wedges);
        assert_eq!(DmaClass::from_state(S_DISABLED), DmaClass::Disabled);
        // Unrecognised byte values decode as "not probed yet".
        assert_eq!(DmaClass::from_state(0xFF), DmaClass::Unknown);
    }

    #[test]
    fn report_is_consistent() {
        let (g, c) = silicon_report();
        assert_eq!(g, gpu_dma_class());
        assert_eq!(c, cd_dma_class());
        // The host never drives the DMA controller, so no probe runs and both
        // channels stay at the compiled initial state: unprobed, which is the
        // only state that lets a transfer path arm a channel.
        assert_eq!(g, DmaClass::Unknown);
        assert_eq!(c, DmaClass::Unknown);
    }

    #[test]
    fn cd_sector_counts_split_by_path_and_sum_to_the_traffic() {
        // The host never reads a sector, so this is the only test that mutates the
        // counters -- and it is the reason they are asserted to start at zero.
        assert_eq!(cd_sector_counts(), (0, 0), "host test run must start clean");
        cd_count_sector(true);
        cd_count_sector(true);
        cd_count_sector(false);
        assert_eq!(cd_sector_counts(), (2, 1));
        // The sum is the property the boot log's ratio depends on: it must equal
        // every sector the reader delivered, whichever path moved it.
        let (dma, pio) = cd_sector_counts();
        assert_eq!(dma + pio, 3);
    }

    #[test]
    fn each_class_has_its_own_state_byte() {
        // The two channels share one decoder, so a collision between class
        // bytes would let convicting one channel report the other's class --
        // and the boot log names each channel separately.
        let bytes = [S_UNKNOWN, S_WORKS, S_WEDGES, S_DISABLED];
        for (i, a) in bytes.iter().enumerate() {
            for b in bytes.iter().skip(i + 1) {
                assert_ne!(a, b, "state bytes collide: {a} and {b}");
            }
        }
        // A wedge is a real decision, not the disabled-by-design sentinel: the
        // two produce different log lines.
        assert_ne!(DmaClass::from_state(S_WEDGES), DmaClass::Disabled);
        assert!(DmaClass::from_state(S_WEDGES).known_wedge());
        assert!(!DmaClass::Disabled.known_wedge());
    }
}
