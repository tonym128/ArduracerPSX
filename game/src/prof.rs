//! Guest-side frame-timing profiler.
//!
//! With the `profiling` cargo feature off, every function here compiles to a
//! no-op and no accumulator RAM is reserved, so a shipping build pays nothing
//! for the span guards scattered through the frame loop. The module still
//! exists and still exports its stage and counter constants so the call sites
//! need no `cfg` of their own.
//!
//! # What it measures
//!
//! The R3000A has no performance counter. What this reads instead is the
//! emulator's guest cycle port at `0xBF802F08`, which the PSoXide emulator
//! decodes as its own `Bus::cycles` counter: the low 32 bits, counting one
//! R3000A clock period per retired instruction plus the memory and DMA stalls
//! charged against it. That port is an Expansion 2 register, so on real
//! hardware it reads back whatever the expansion peripheral happens to return.
//! That is why the counters are feature-gated rather than always-on.
//!
//! Everything here is in *guest cycles*, not wall-clock nanoseconds. At the
//! 33.8688 MHz NTSC clock a 60 Hz VBlank is 564,480 cycles; see [`BUDGET_CYCLES`]
//! for why that is the divisor rather than the emulator's own 571,236.
//!
//! # Nesting
//!
//! Spans are independent: each records its own elapsed time into its own slot,
//! so a parent row includes its children. The report prints them as separate
//! rows in tree order rather than trying to subtract children out of parents.
//!
//! # Not a shipping feature
//!
//! Reports go through [`dbg`], which prints to the Expansion 2 TTY that the
//! emulator mirrors to its own stdout. They are printed once per window rather
//! than once per frame, because a per-frame report through `putchar` would
//! itself dominate the frame it was measuring.

// Only the reporting half of this module touches the TTY; the span half reads
// an MMIO port directly.
#[cfg(feature = "profiling")]
use crate::dbg;

// ---------------------------------------------------------------------------
// Budget
// ---------------------------------------------------------------------------

/// Cycles available per frame at 30 Hz NTSC (33.8688 MHz / 30 = 2 VBlanks).
///
/// This is the naive figure, and it is what the emulator's own profile report
/// divides by too. The emulator's scheduler instead uses a slightly longer
/// beam-cadence period, so a frame reported here as 99% of budget may in
/// fact still have fit. Use it to compare stages and runs against each other,
/// not as the exact drop threshold.
pub const BUDGET_CYCLES: u32 = 1_128_960;

/// Frames per report window. 30 frames is one second at the target rate: short
/// enough to catch a transient, long enough that the report's own TTY traffic
/// does not distort the window it describes.
pub const WINDOW_FRAMES: u32 = 30;

// ---------------------------------------------------------------------------
// Stages
// ---------------------------------------------------------------------------

/// Number of stage slots. Sized to the highest stage constant plus one; the
/// compile-time guard below keeps that honest.
pub const STAGE_COUNT: usize = 22;

/// A frame spent blocked in VBlank, waiting for the display period.
///
/// The complement of every other stage: if the whole frame fits the budget,
/// this is where the slack goes. A `VBLANK` near zero across a whole window
/// means frames are running to the edge; exactly zero means display periods
/// are being missed.
pub const VBLANK: usize = 0;

/// Frame counter increment, fault check and the 300-frame heartbeat print.
///
/// Split out so the heartbeat's own cost is visible instead of landing in
/// whichever stage happens to contain it.
pub const LOOP_OVERHEAD: usize = 1;

/// One `poll_port1` SIO0 transaction.
pub const PAD_POLL: usize = 2;

/// Vehicle physics, track collision, surface resample, lap timer.
pub const SIM_PLAYER: usize = 3;

/// AI rival ticks and the standings recompute.
pub const SIM_RIVALS: usize = 4;

/// Ghost recording sample.
pub const SIM_GHOST: usize = 5;

/// Camera follow.
pub const CAMERA: usize = 6;

/// Audio engine synth, tire screech, SFX and rumble.
pub const AUDIO: usize = 7;

/// Framebuffer clear to the background colour.
pub const CLEAR: usize = 8;

/// Whole `render_track`: the per-pixel track blit plus its texture streaming.
pub const RENDER_TRACK: usize = 9;

/// Visible-tile enumeration: the camera window walk filling `visible_tiles`.
pub const TRACK_VISIBLE_SCAN: usize = 10;

/// VRAM LRU cache lookup across the visible tiles (phase 1 of `render_track`).
///
/// Pure cache bookkeeping, no decode and no VRAM write. Charged separately
/// from the decode because it is O(visible tiles x cache slots), so it grows
/// with the cache rather than with the decode budget.
pub const TRACK_CACHE_PROBE: usize = 11;

/// Synchronous CD-ROM sector read for one 1024x1024 JPEG block.
///
/// The prime stutter suspect. `DiscReader::read_sectors` is blocking and pulls
/// 100 sectors per block while spinning on the data-ready IRQ, so a single miss
/// on this row is worth several frames on its own.
pub const TRACK_CD_READ: usize = 12;

/// JPEG header parse plus `index_restarts` for a freshly streamed block.
///
/// A full linear walk of the ~160-200 KB entropy-coded scan hunting for `FF Dn`
/// markers, so it is O(block size) on every block load even though the result
/// stays valid for the block's whole residency.
pub const TRACK_JPEG_INDEX: usize = 13;

/// `decode_tile_64x64`: Huffman, IDCT, chroma upsample, colour convert.
///
/// 96 8x8 IDCTs per 64x64 tile. This times the work the per-frame decode
/// budget admits, not the work the visible window wants.
pub const TRACK_JPEG_DECODE: usize = 14;

/// `upload_16bpp`: 8 KB per tile streamed out through GP0(A0h).
pub const TRACK_VRAM_UPLOAD: usize = 15;

/// Textured quad emission for the visible tiles (phase 3 of `render_track`).
pub const TRACK_QUADS: usize = 16;

/// Skidmark buffer render.
pub const SKIDMARKS: usize = 17;

/// Ghost car playback render.
pub const GHOST_CAR: usize = 18;

/// Player car and AI rival car render.
pub const CARS: usize = 19;

/// Particle system tick and render.
pub const PARTICLES: usize = 20;

/// In-game HUD overlay.
pub const HUD: usize = 21;

const _: () = assert!(HUD == STAGE_COUNT - 1);

// ---------------------------------------------------------------------------
// Counters
// ---------------------------------------------------------------------------

/// Number of counter slots. Sized to the highest counter constant plus one.
pub const COUNTER_COUNT: usize = 9;

/// 64x64 tiles decompressed from JPEG into VRAM this frame.
pub const C_TILE_DECODES: usize = 0;

/// Visible tiles served from the VRAM cache without decoding.
pub const C_TILE_HITS: usize = 1;

/// Visible tiles the per-frame decode budget could not admit, drawn as flat
/// placeholder shading instead.
///
/// Non-zero means the streaming budget is behind the camera and the track is
/// visibly filling in a tile at a time.
pub const C_PLACEHOLDERS: usize = 2;

/// Visible tiles considered this frame.
pub const C_VISIBLE_TILES: usize = 3;

/// 1024x1024 JPEG blocks streamed from TRACKS.BIN.
pub const C_CD_BLOCKS: usize = 4;

/// 2048-byte CD sectors read from TRACKS.BIN.
pub const C_CD_SECTORS: usize = 5;

/// Frames whose stage total exceeded [`BUDGET_CYCLES`].
pub const C_OVER_BUDGET: usize = 6;

/// Whole VBlank periods dropped between consecutive frames.
///
/// The direct stutter measurement: the loop is vblank-locked, so this is
/// exactly the number of display periods the game failed to present. Zero
/// across a window is the "locked 60 FPS" claim made concrete.
pub const C_MISSED_VBLANKS: usize = 7;

/// 64x16 MCU rows decoded this frame. Four complete one 64x64 tile, so this is
/// the finer-grained view of decode work: a frame can decode a row without
/// completing a tile, which is the whole point of the time-sliced job.
pub const C_DECODE_ROWS: usize = 8;

const _: () = assert!(C_DECODE_ROWS == COUNTER_COUNT - 1);

// ---------------------------------------------------------------------------
// Instrumentation on
// ---------------------------------------------------------------------------

/// Whether `stage`'s cycles are already included in [`RENDER_TRACK`].
///
/// True for the sub-stages `render_track` opens on its own children. They are
/// reported as separate rows because they are the interesting numbers, but they
/// must be excluded from any *total*, or the same cycles get counted twice.
#[cfg(feature = "profiling")]
const fn is_nested_in_render_track(stage: usize) -> bool {
    matches!(
        stage,
        TRACK_VISIBLE_SCAN
            | TRACK_CACHE_PROBE
            | TRACK_CD_READ
            | TRACK_JPEG_INDEX
            | TRACK_JPEG_DECODE
            | TRACK_VRAM_UPLOAD
            | TRACK_QUADS
    )
}

/// Human-readable stage name, indexed by the stage constants above.
#[cfg(feature = "profiling")]
static STAGE_NAMES: [&str; STAGE_COUNT] = [
    "vblank",
    "loop",
    "pad",
    "sim:player",
    "sim:rivals",
    "sim:ghost",
    "camera",
    "audio",
    "clear",
    "render:track",
    "track:visible",
    "track:probe",
    "track:cd-read",
    "track:jpeg-index",
    "track:jpeg-decode",
    "track:vram-upload",
    "track:quads",
    "skidmarks",
    "ghost-car",
    "cars",
    "particles",
    "hud",
];

/// Human-readable counter name, indexed by the counter constants above.
#[cfg(feature = "profiling")]
static COUNTER_NAMES: [&str; COUNTER_COUNT] = [
    "tile-decodes",
    "tile-hits",
    "placeholders",
    "visible-tiles",
    "cd-blocks",
    "cd-sectors",
    "over-budget",
    "missed-vblanks",
    "decode-rows",
];

/// Cycle totals summed over the frames in the current window.
#[cfg(feature = "profiling")]
static mut WINDOW_STAGES: [u32; STAGE_COUNT] = [0; STAGE_COUNT];
/// Worst single-frame value per stage within the current window.
#[cfg(feature = "profiling")]
static mut WINDOW_MAX: [u32; STAGE_COUNT] = [0; STAGE_COUNT];
/// This frame's per-stage cycle totals.
#[cfg(feature = "profiling")]
static mut FRAME_STAGES: [u32; STAGE_COUNT] = [0; STAGE_COUNT];
/// This frame's counters.
#[cfg(feature = "profiling")]
static mut FRAME_COUNTERS: [u32; COUNTER_COUNT] = [0; COUNTER_COUNT];
/// Counter totals over the frames in the current window.
#[cfg(feature = "profiling")]
static mut WINDOW_COUNTERS: [u32; COUNTER_COUNT] = [0; COUNTER_COUNT];
/// Worst total frame cost seen in the current window.
#[cfg(feature = "profiling")]
static mut WORST_FRAME_CYCLES: u32 = 0;
/// Stage breakdown of that worst frame. This is what a spike gets attributed
/// to, and it is the most useful single row group in the report.
#[cfg(feature = "profiling")]
static mut WORST_FRAME_STAGES: [u32; STAGE_COUNT] = [0; STAGE_COUNT];
/// Frames folded into the current window so far.
#[cfg(feature = "profiling")]
static mut WINDOW_FRAME_COUNT: u32 = 0;
/// Sum of per-frame work across the window, for a mean-work row that does not
/// depend on the caller remembering to exclude the nested sub-stages.
#[cfg(feature = "profiling")]
static mut WINDOW_WORK: u32 = 0;
/// Cycle totals since boot.
#[cfg(feature = "profiling")]
static mut TOTAL_STAGES: [u32; STAGE_COUNT] = [0; STAGE_COUNT];
/// Counter totals since boot.
#[cfg(feature = "profiling")]
static mut TOTAL_COUNTERS: [u32; COUNTER_COUNT] = [0; COUNTER_COUNT];
/// Frames counted since boot.
#[cfg(feature = "profiling")]
static mut TOTAL_FRAMES: u32 = 0;
/// Whole VBlank periods dropped since boot.
#[cfg(feature = "profiling")]
static mut TOTAL_MISSED_VBLANKS: u32 = 0;
/// Previous frame's `vblank_count`, for the dropped-period delta.
#[cfg(feature = "profiling")]
static mut PREV_VBLANK: u32 = 0;
/// Whether `begin_frame` has already latched a previous vblank count.
#[cfg(feature = "profiling")]
static mut FRAME_LATCHED: bool = false;

/// Reads the emulator's guest cycle counter.
///
/// Expansion 2 port `0xBF802F08`; the emulator answers with the low 32 bits of
/// its bus cycle count, wrapping roughly every 126 seconds at 33.87 MHz. Every
/// use is a difference against a read taken microseconds earlier, so the wrap
/// only matters for a single span longer than that.
#[cfg(feature = "profiling")]
#[inline(always)]
fn cycles() -> u32 {
    unsafe { core::ptr::read_volatile(0xBF80_2F08 as *const u32) }
}

/// Charges elapsed cycles to a stage slot when it drops.
///
/// Drop rather than an explicit `end()`, so a span still closes on an early
/// `continue` out of the frame loop. The racing state's pause arm takes exactly
/// that path, and an unbalanced span there would silently poison the next 59
/// frames of the window.
#[cfg(feature = "profiling")]
pub struct Span {
    stage: usize,
    start: u32,
}

#[cfg(feature = "profiling")]
impl Drop for Span {
    #[inline(always)]
    fn drop(&mut self) {
        let elapsed = cycles().wrapping_sub(self.start);
        let slot = unsafe { &mut *core::ptr::addr_of_mut!(FRAME_STAGES) };
        slot[self.stage] = slot[self.stage].wrapping_add(elapsed);
    }
}

/// Stands in for [`Span`] when profiling is off, so `let _s = span(X);`
/// compiles away entirely.
#[cfg(not(feature = "profiling"))]
#[derive(Clone, Copy)]
pub struct Span;

/// Opens a timing span on `stage`. Bind the result so it drops at scope end.
#[cfg(feature = "profiling")]
#[inline(always)]
pub fn span(stage: usize) -> Span {
    Span {
        stage,
        start: cycles(),
    }
}

/// No-op stand-in; `stage` is unused so the call site stays warning-free.
#[cfg(not(feature = "profiling"))]
#[inline(always)]
pub fn span(_stage: usize) -> Span {
    Span
}

/// Adds `n` to a counter for the current frame.
#[cfg(feature = "profiling")]
#[inline(always)]
pub fn count(counter: usize, n: u32) {
    let slot = unsafe { &mut *core::ptr::addr_of_mut!(FRAME_COUNTERS) };
    slot[counter] = slot[counter].wrapping_add(n);
}

/// No-op stand-in for [`count`].
#[cfg(not(feature = "profiling"))]
#[inline(always)]
pub fn count(_counter: usize, _n: u32) {}

/// Sets a counter for the current frame, for the common "exactly once" case.
#[cfg(feature = "profiling")]
#[inline(always)]
pub fn set(counter: usize, n: u32) {
    let slot = unsafe { &mut *core::ptr::addr_of_mut!(FRAME_COUNTERS) };
    slot[counter] = n;
}

/// No-op stand-in for [`set`].
#[cfg(not(feature = "profiling"))]
#[inline(always)]
pub fn set(_counter: usize, _n: u32) {}

/// Latches the frame boundary: clears this frame's slots, and records any whole
/// VBlank periods dropped since the previous call.
///
/// Call once per frame, before the first `span`. `C_MISSED_VBLANKS` then
/// describes the frame that is starting rather than the one just ended.
#[cfg(feature = "profiling")]
pub fn begin_frame() {
    unsafe {
        let stages = &mut *core::ptr::addr_of_mut!(FRAME_STAGES);
        for s in stages.iter_mut() {
            *s = 0;
        }
        let counters = &mut *core::ptr::addr_of_mut!(FRAME_COUNTERS);
        for c in counters.iter_mut() {
            *c = 0;
        }

        let now = psx_rt::interrupts::vblank_count();
        if FRAME_LATCHED {
            let delta = now.wrapping_sub(PREV_VBLANK);
            if delta > 1 {
                let missed = delta - 1;
                let slot = &mut *core::ptr::addr_of_mut!(FRAME_COUNTERS);
                slot[C_MISSED_VBLANKS] = missed;
                let total = &mut *core::ptr::addr_of_mut!(TOTAL_MISSED_VBLANKS);
                *total = total.wrapping_add(missed);
            }
        } else {
            FRAME_LATCHED = true;
        }
        PREV_VBLANK = now;
    }
}

/// No-op stand-in for [`begin_frame`].
#[cfg(not(feature = "profiling"))]
#[inline(always)]
pub fn begin_frame() {}

/// Closes the frame: folds this frame's slots into the window, and once the
/// window is full prints the report and opens a new one.
#[cfg(feature = "profiling")]
pub fn end_frame() {
    unsafe {
        let frame_stages = *core::ptr::addr_of_mut!(FRAME_STAGES);
        let frame_counters = *core::ptr::addr_of_mut!(FRAME_COUNTERS);

        let window_stages = &mut *core::ptr::addr_of_mut!(WINDOW_STAGES);
        let window_max = &mut *core::ptr::addr_of_mut!(WINDOW_MAX);
        let window_counters = &mut *core::ptr::addr_of_mut!(WINDOW_COUNTERS);
        let total_stages = &mut *core::ptr::addr_of_mut!(TOTAL_STAGES);
        let total_counters = &mut *core::ptr::addr_of_mut!(TOTAL_COUNTERS);

        let mut total = 0u32;
        for i in 0..STAGE_COUNT {
            let v = frame_stages[i];
            // Only the top-level stages count toward the frame total. The
            // `track:*` sub-stages are nested inside `RENDER_TRACK`, so summing
            // them as well would count their cycles twice and report a frame as
            // over budget when the real figure is comfortably inside it.
            // `VBLANK` is excluded too: it is the slack after the frame's work
            // finished, not part of the work.
            if !is_nested_in_render_track(i) && i != VBLANK {
                total = total.wrapping_add(v);
            }
            window_stages[i] = window_stages[i].wrapping_add(v);
            if v > window_max[i] {
                window_max[i] = v;
            }
            total_stages[i] = total_stages[i].wrapping_add(v);
        }
        let work = &mut *core::ptr::addr_of_mut!(WINDOW_WORK);
        *work = work.wrapping_add(total);
        for i in 0..COUNTER_COUNT {
            let v = frame_counters[i];
            window_counters[i] = window_counters[i].wrapping_add(v);
            total_counters[i] = total_counters[i].wrapping_add(v);
        }

        if total > WORST_FRAME_CYCLES {
            WORST_FRAME_CYCLES = total;
            WORST_FRAME_STAGES = frame_stages;
        }

        if total > BUDGET_CYCLES {
            let w = &mut *core::ptr::addr_of_mut!(WINDOW_COUNTERS);
            w[C_OVER_BUDGET] = w[C_OVER_BUDGET].wrapping_add(1);
            let t = &mut *core::ptr::addr_of_mut!(TOTAL_COUNTERS);
            t[C_OVER_BUDGET] = t[C_OVER_BUDGET].wrapping_add(1);
        }

        let frames = &mut *core::ptr::addr_of_mut!(TOTAL_FRAMES);
        *frames = frames.wrapping_add(1);

        let window = &mut *core::ptr::addr_of_mut!(WINDOW_FRAME_COUNT);
        *window = window.wrapping_add(1);
        if *window < WINDOW_FRAMES {
            return;
        }
        *window = 0;
    }

    report_window();
}

/// No-op stand-in for [`end_frame`].
#[cfg(not(feature = "profiling"))]
#[inline(always)]
pub fn end_frame() {}

/// Prints one row of a stage table: mean, worst, and mean as a share of budget.
#[cfg(feature = "profiling")]
fn print_row(name: &str, mean: u32, worst: u32) {
    dbg::print("[PROF] ");
    dbg::print(name);
    dbg::print(" mean=");
    dbg::print_dec(mean);
    dbg::print(" worst=");
    dbg::print_dec(worst);
    dbg::print(" (");
    dbg::print_dec(mean * 100 / BUDGET_CYCLES);
    dbg::print("%)\n");
}

/// Prints the current window: per-stage mean and worst, the worst single
/// frame's attribution, and the window's counters. Then resets the window.
#[cfg(feature = "profiling")]
pub fn report_window() {
    unsafe {
        let window_stages = *core::ptr::addr_of_mut!(WINDOW_STAGES);
        let window_max = *core::ptr::addr_of_mut!(WINDOW_MAX);
        let window_counters = *core::ptr::addr_of_mut!(WINDOW_COUNTERS);
        let total_counters = *core::ptr::addr_of_mut!(TOTAL_COUNTERS);
        let worst_total = *core::ptr::addr_of_mut!(WORST_FRAME_CYCLES);
        let worst_stages = *core::ptr::addr_of_mut!(WORST_FRAME_STAGES);
        let total_frames = *core::ptr::addr_of_mut!(TOTAL_FRAMES);

        dbg::println("[PROF] ===== window report =====");
        dbg::print("[PROF] frames-so-far=");
        dbg::print_dec(total_frames);
        dbg::print(" worst-frame=");
        dbg::print_dec(worst_total);
        dbg::print(" (");
        dbg::print_dec(worst_total * 100 / BUDGET_CYCLES);
        dbg::print("% of ");
        dbg::print_dec(BUDGET_CYCLES);
        dbg::print(")\n");
        // `VBLANK` is excluded from `worst-frame` above because it is the
        // *complement* of the frame's work: the slack left after the frame
        // finished, not part of the work itself. So `worst-frame` is the worst
        // amount of work a frame did, and these two rows are the matching
        // headroom and the implied mean.
        let mean_work = *core::ptr::addr_of!(WINDOW_WORK) / WINDOW_FRAMES;
        dbg::print("[PROF] mean-work=");
        dbg::print_dec(mean_work);
        dbg::print(" mean-vblank-slack=");
        dbg::print_dec(window_stages[VBLANK] / WINDOW_FRAMES);
        dbg::print(" slack-if-30fps=");
        dbg::print_dec(
            BUDGET_CYCLES.saturating_sub(*core::ptr::addr_of!(WINDOW_WORK) / WINDOW_FRAMES),
        );
        dbg::print("\n");

        for i in 0..STAGE_COUNT {
            // Skip untouched stages so the report is only as long as the work
            // that actually happened.
            if window_stages[i] == 0 {
                continue;
            }
            print_row(
                STAGE_NAMES[i],
                window_stages[i] / WINDOW_FRAMES,
                window_max[i],
            );
        }

        dbg::println("[PROF] --- worst frame attribution ---");
        if worst_total == 0 {
            dbg::println("[PROF] (none)");
        } else {
            for i in 0..STAGE_COUNT {
                if worst_stages[i] == 0 {
                    continue;
                }
                print_row(STAGE_NAMES[i], worst_stages[i], worst_stages[i]);
            }
        }

        dbg::println("[PROF] --- counters window/total ---");
        for i in 0..COUNTER_COUNT {
            if window_counters[i] == 0 && total_counters[i] == 0 {
                continue;
            }
            dbg::print("[PROF] ");
            dbg::print(COUNTER_NAMES[i]);
            dbg::print(" ");
            dbg::print_dec(window_counters[i]);
            dbg::print("/");
            dbg::print_dec(total_counters[i]);
            dbg::print("\n");
        }

        // Indexed writes through `addr_of_mut!` rather than `iter_mut()`: the
        // statics are still being read through raw pointers above, and a
        // `&mut` borrow of a `static mut` would be a 2024-edition hard error.
        let stages = &mut *core::ptr::addr_of_mut!(WINDOW_STAGES);
        for s in stages.iter_mut() {
            *s = 0;
        }
        let max = &mut *core::ptr::addr_of_mut!(WINDOW_MAX);
        for m in max.iter_mut() {
            *m = 0;
        }
        let counters = &mut *core::ptr::addr_of_mut!(WINDOW_COUNTERS);
        for c in counters.iter_mut() {
            *c = 0;
        }
        WINDOW_WORK = 0;
        WORST_FRAME_CYCLES = 0;
        WORST_FRAME_STAGES = [0; STAGE_COUNT];
    }
}

/// No-op stand-in for [`report_window`].
#[cfg(not(feature = "profiling"))]
#[inline(always)]
pub fn report_window() {}

/// Prints mean cycles per stage since boot, as a whole-run summary.
#[cfg(feature = "profiling")]
pub fn report_total() {
    unsafe {
        let total_stages = *core::ptr::addr_of_mut!(TOTAL_STAGES);
        let total_counters = *core::ptr::addr_of_mut!(TOTAL_COUNTERS);
        let total_frames = *core::ptr::addr_of_mut!(TOTAL_FRAMES);
        let total_missed = *core::ptr::addr_of_mut!(TOTAL_MISSED_VBLANKS);

        dbg::println("[PROF] ===== total report =====");
        dbg::print("[PROF] frames=");
        dbg::print_dec(total_frames);
        dbg::print(" missed-vblanks=");
        dbg::print_dec(total_missed);
        dbg::print("\n");

        // `checked_div` rather than an `if frames > 0` guard so a zero-frame
        // run still prints its counters instead of dividing by zero.
        for i in 0..STAGE_COUNT {
            if total_stages[i] == 0 {
                continue;
            }
            if let Some(mean) = total_stages[i].checked_div(total_frames) {
                print_row(STAGE_NAMES[i], mean, 0);
            }
        }

        dbg::println("[PROF] --- counters total ---");
        for i in 0..COUNTER_COUNT {
            if total_counters[i] == 0 {
                continue;
            }
            dbg::print("[PROF] ");
            dbg::print(COUNTER_NAMES[i]);
            dbg::print(" ");
            dbg::print_dec(total_counters[i]);
            dbg::print("\n");
        }
    }
}

/// No-op stand-in for [`report_total`].
#[cfg(not(feature = "profiling"))]
#[inline(always)]
pub fn report_total() {}
