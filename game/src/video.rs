//! MDEC Full-Motion Video (FMV) Playback Engine for Arduracer PSX.
//!
//! Streams and decodes 320x240 @ 15 fps version-2 `.STR` files from the
//! CD-ROM through the MDEC (DMA0 in / DMA1 out), following the pump
//! discipline of the PSoXide `hello-fmv` reference player:
//!
//! - The drive streams at double speed (150 sectors/s). Cooked streams carry
//!   exactly 10 sectors per frame, so the drive and the 15 fps display agree.
//! - [`Stream::pump`] drains every ready sector into a two-slot frame ring
//!   and runs between every unit of work (bitstream decode, every MDEC
//!   column, every pacing wait), so the drive's small buffer never overruns.
//! - Playback stops at the end of the file, on a drive error, after a 2 s
//!   stall, or when the player presses START / CROSS / CIRCLE (edge
//!   triggered, so a button held through boot does not skip the intro).
//!
//! The caller must have initialised the GPU (display enabled) beforehand.

use core::ptr::addr_of_mut;
use psx_fmv::{bs, iso, mdec, str::FrameAssembler};
use psx_gpu as gpu;
use psx_pack::cd::{SectorReader, SECTOR_WORDS};
use psx_pad::{button, poll_port1};
use psx_rt::interrupts;
use psx_vram::VramRect;

const WIDTH: u16 = 320;
const HEIGHT: u16 = 240;
const COLUMNS: u16 = WIDTH / 16;
const ROWS: u32 = HEIGHT as u32 / 16;
/// 15bpp column of 16 x HEIGHT pixels, two per word.
const COLUMN_WORDS: usize = 8 * HEIGHT as usize;
/// VBlanks per movie frame: 60 Hz / 15 fps.
const VBLANKS_PER_FRAME: u32 = 4;
/// Frame slot: 16 chunks of 2016 bytes.
const SLOT_BYTES: usize = 16 * 2016;
/// Three slots: one filling, one queued, one being decoded.
const SLOTS: usize = 3;
/// MDEC run-length buffer: 64K halfwords.
const RLE_WORDS: usize = 32 * 1024;
/// End the stream if no sector arrives for this long (2 s).
const STALL_VBLANKS: u32 = 120;
/// Second display buffer row (first is row 0).
const BACK_Y: u16 = 256;

static mut READER: SectorReader = SectorReader::new();
static mut SECTOR: [u32; SECTOR_WORDS] = [0; SECTOR_WORDS];
static mut SLOT: [[u8; SLOT_BYTES]; SLOTS] = [[0; SLOT_BYTES]; SLOTS];
static mut RLE: [u32; RLE_WORDS] = [0; RLE_WORDS];
static mut COLUMN: [u32; COLUMN_WORDS] = [0; COLUMN_WORDS];

/// Why playback ended.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum VideoResult {
    /// Every sector of the file was streamed.
    Completed,
    /// The player skipped the movie.
    Skipped,
    /// The drive, file or MDEC could not be set up (no disc, no file...).
    Unavailable,
    /// Streaming stopped early on a drive error or stall.
    Interrupted,
}

fn slot_bytes(i: usize) -> &'static mut [u8] {
    // SAFETY: slots are disjoint statics; the pump only writes `filling`
    // and the decoder only reads the slot it took from `ready`.
    unsafe {
        let p = addr_of_mut!(SLOT[i % SLOTS]) as *mut u8;
        core::slice::from_raw_parts_mut(p, SLOT_BYTES)
    }
}

/// Streaming state owned by the pump.
struct Stream {
    sectors_left: u32,
    done: bool,
    error: bool,
    filling: usize,
    ready: Option<(usize, u32)>,
    decoding: Option<usize>,
    asm: FrameAssembler,
    last_sector_vblank: u32,
    skip: bool,
    prev_skip_held: bool,
}

impl Stream {
    /// Drain every sector the drive has ready, and sample the skip buttons.
    fn pump(&mut self) {
        let pad = poll_port1();
        let held = pad.buttons.is_held(button::START)
            || pad.buttons.is_held(button::CROSS)
            || pad.buttons.is_held(button::CIRCLE);
        if held && !self.prev_skip_held {
            self.skip = true;
        }
        self.prev_skip_held = held;

        while !self.done {
            // SAFETY: single-threaded; READER/SECTOR are only used here.
            let got =
                unsafe { (*addr_of_mut!(READER)).try_read_sector(&mut *addr_of_mut!(SECTOR)) };
            match got {
                Ok(true) => {}
                Ok(false) => return,
                Err(()) => {
                    self.error = true;
                    self.done = true;
                    return;
                }
            }
            self.last_sector_vblank = interrupts::vblank_count();
            self.sectors_left = self.sectors_left.saturating_sub(1);
            if self.sectors_left == 0 {
                self.done = true;
            }
            // SAFETY: the same word buffer viewed as bytes.
            let sector = unsafe {
                core::slice::from_raw_parts(addr_of_mut!(SECTOR) as *const u8, SECTOR_WORDS * 4)
            };
            if let Some(frame) = self.asm.add(sector, slot_bytes(self.filling)) {
                let done_slot = self.filling;
                // A queued frame that was never shown is dropped; its slot is reused.
                let next = match self.ready.replace((done_slot, frame.size)) {
                    Some((stale, _)) => stale,
                    None => (0..SLOTS)
                        .find(|&s| s != done_slot && Some(s) != self.decoding)
                        .unwrap_or(done_slot),
                };
                self.filling = next;
            }
        }
    }

    fn stalled(&self) -> bool {
        interrupts::vblank_count().wrapping_sub(self.last_sector_vblank) > STALL_VBLANKS
    }
}

/// Wait for the next VBlank edge while keeping the drive drained.
fn wait_vblank_pumping(st: &mut Stream) {
    let start = interrupts::vblank_count();
    while interrupts::vblank_count() == start {
        st.pump();
    }
}

/// Read one sector synchronously (directory lookups before streaming).
fn read_single_sector(lba: u32) -> Option<&'static [u8]> {
    // SAFETY: single-threaded use of the reader and its sector buffer.
    unsafe {
        let r = &mut *addr_of_mut!(READER);
        if !r.start_read(lba) {
            return None;
        }
        let ok = r.read_sector(&mut *addr_of_mut!(SECTOR));
        r.stop();
        if !ok {
            return None;
        }
        Some(core::slice::from_raw_parts(
            addr_of_mut!(SECTOR) as *const u8,
            SECTOR_WORDS * 4,
        ))
    }
}

/// Plays an FMV `.STR` file from the CD-ROM root directory.
pub fn play_video(file_name: &str) -> VideoResult {
    // 1. Drive: double-speed 2048-byte data mode.
    // SAFETY: nothing else drives the CD during boot-time playback.
    if !unsafe { (*addr_of_mut!(READER)).prepare() } {
        return VideoResult::Unavailable;
    }

    // 2. Locate the movie through the ISO9660 root directory.
    let Some(pvd) = read_single_sector(iso::PVD_LBA) else {
        return VideoResult::Unavailable;
    };
    let Some((root_lba, _)) = iso::root_directory(pvd) else {
        return VideoResult::Unavailable;
    };
    let Some(root) = read_single_sector(root_lba) else {
        return VideoResult::Unavailable;
    };
    let Some((movie_lba, movie_bytes)) = iso::find_in_directory(root, file_name) else {
        return VideoResult::Unavailable;
    };
    let total_sectors = movie_bytes.div_ceil(2048);
    if total_sectors == 0 {
        return VideoResult::Unavailable;
    }

    // 3. MDEC tables.
    mdec::reset();
    if !mdec::load_tables() {
        return VideoResult::Unavailable;
    }

    // 4. Black out both display buffers before the first frame lands.
    gpu::fill_rect(0, 0, WIDTH, 512, 0, 0, 0);
    gpu::draw_sync();

    let mut st = Stream {
        sectors_left: total_sectors,
        done: false,
        error: false,
        filling: 0,
        ready: None,
        decoding: None,
        asm: FrameAssembler::new(),
        last_sector_vblank: interrupts::vblank_count(),
        skip: false,
        // Assume held so a button down at boot must be released first.
        prev_skip_held: true,
    };

    // SAFETY: the reader was prepared above and is idle.
    if !unsafe { (*addr_of_mut!(READER)).start_read(movie_lba) } {
        return VideoResult::Unavailable;
    }

    // SAFETY: RLE and COLUMN are only touched by this loop.
    let rle = unsafe { &mut *addr_of_mut!(RLE) };
    let rle16 =
        unsafe { core::slice::from_raw_parts_mut(rle.as_mut_ptr() as *mut u16, RLE_WORDS * 2) };
    let column = unsafe { &mut *addr_of_mut!(COLUMN) };

    let mut back_y: u16 = BACK_Y;
    let mut next_flip = interrupts::vblank_count();

    loop {
        st.pump();
        if st.skip {
            break;
        }
        let Some((slot, bytes)) = st.ready.take() else {
            if st.done || st.stalled() {
                break;
            }
            continue;
        };

        // Bitstream -> MDEC run-length, pumping once per macroblock column.
        st.decoding = Some(slot);
        let frame = &slot_bytes(slot)[..(bytes as usize).min(SLOT_BYTES)];
        let Ok(words) =
            bs::decode_frame(frame, rle16, COLUMNS as u32 * ROWS, ROWS, &mut || st.pump())
        else {
            st.decoding = None;
            continue;
        };
        st.decoding = None; // the bitstream is fully consumed

        // MDEC decode, column by column into the back buffer.
        // SAFETY: RLE stays untouched until decode_finish below.
        unsafe { mdec::decode_start(rle, words, mdec::DECODE_15BPP) };
        let mut ok = true;
        for c in 0..COLUMNS {
            if !mdec::read_column(column) {
                ok = false;
                break;
            }
            psx_vram::upload_words(VramRect::new(c * 16, back_y, 16, HEIGHT), column);
            st.pump();
        }
        if !mdec::decode_finish() || !ok {
            mdec::reset();
            let _ = mdec::load_tables();
            continue;
        }

        // Pace to 15 fps and flip at a VBlank; the drive keeps draining.
        while (interrupts::vblank_count().wrapping_sub(next_flip) as i32) < 0 {
            st.pump();
        }
        wait_vblank_pumping(&mut st);
        psx_io::gpu::write_gp1(0x0500_0000 | ((back_y as u32) << 10));
        next_flip = interrupts::vblank_count().wrapping_add(VBLANKS_PER_FRAME - 1);
        back_y = if back_y == 0 { BACK_Y } else { 0 };
    }

    // SAFETY: stop the stream we started.
    unsafe { (*addr_of_mut!(READER)).stop() };
    // Hand the display back at row 0, blanked.
    gpu::fill_rect(0, 0, WIDTH, 512, 0, 0, 0);
    gpu::draw_sync();
    psx_io::gpu::write_gp1(0x0500_0000);

    if st.skip {
        VideoResult::Skipped
    } else if st.error || (st.sectors_left > 0) {
        VideoResult::Interrupted
    } else {
        VideoResult::Completed
    }
}
