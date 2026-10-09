//! MDEC Full-Motion Video (STR) Playback Engine for Arduracer PSX.
//!
//! Streams and decodes 320x240 @ 15 fps Version-2 `.STR` files from the
//! CD-ROM through the MDEC hardware decoder (DMA0 in / DMA1 out), using
//! the proven 15 FPS playback architecture from Plattypus.
//!
//! # Architecture & Pipeline
//!
//! 1. **1x Speed BS v2 Stream**: Encoded with `psxavenc -t strv -v v2 -s 320x240 -r 15 -x 1`.
//!    At 1x speed (75 sectors/s), each 15 fps frame occupies exactly 5 sectors (10,240 bytes).
//!    The data rate matches drive throughput with zero buffer overruns and zero seek penalties.
//! 2. **6-Slot Frame Ring Buffer**: Assembles variable-length chunks into 6 slots with a
//!    strict FIFO ready queue (`ready_queue`), absorbing initial drive spin-up jitter.
//! 3. **Continuous Non-blocking Pump**: Drains sectors into the assembler via `try_read_sector()`
//!    whenever the drive has data ready: inside VBlank wait loops, between macroblock columns,
//!    and during CPU bitstream expansion.
//! 4. **Hardware DMA2 VRAM Upload**: Decoded 16-pixel macroblock columns are uploaded via
//!    DMA Channel 2 (`dma_copy_to_vram`) in block mode, with automatic fallback to GP0.
//! 5. **Locked 15.00 FPS Pacing**: Paced to every 4th VBlank on 60 Hz NTSC (`VBLANKS_PER_VIDEO_FRAME = 4`).

use core::ptr::addr_of_mut;

use psx_fmv::{bs, mdec, str as strfmt};
use psx_gpu as gpu;
use psx_gpu::framebuf::FrameBuffer;
use psx_io::dma;
use psx_pack::cd::{SectorReader, SECTOR_WORDS};
use psx_pad::{button, ButtonState, PadState};
use psx_vram::{dma_copy_to_vram, upload_words, VramRect};

pub const VIDEO_W: u16 = 320;
pub const VIDEO_H: u16 = 240;
/// Macroblock columns (320 / 16).
pub const COLUMNS: u16 = VIDEO_W / 16;
/// Macroblock rows (240 / 16).
pub const ROWS: u32 = VIDEO_H as u32 / 16;
/// Display periods each video frame is held for. Four gives 15 fps on a
/// 60 Hz NTSC display.
pub const VBLANKS_PER_VIDEO_FRAME: u32 = 4;

/// Sectors one frame may span (5 for a 1x BS v2 frame; 16 leaves ample headroom).
pub const MAX_CHUNKS: u16 = 16;
/// Reassembly buffer per slot, in u32 words.
pub const SLOT_WORDS: usize = MAX_CHUNKS as usize * strfmt::CHUNK_PAYLOAD_BYTES / 4;
/// Frame slots: one decoding, one ready, the rest filling.
pub const SLOTS: usize = 6;
/// MDEC run-length buffer in u32 words.
pub const RLE_WORDS: usize = 16 * 1024;
/// Decoded pixels in one 16x240 column: 8 words per row.
pub const COLUMN_WORDS: usize = 8 * VIDEO_H as usize;

/// Display periods with no sector and nothing to show before the stream is declared stalled.
const STALL_VBLANKS: u32 = 120;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum VideoResult {
    /// Every frame was presented to completion.
    Completed,
    /// The player skipped the movie via controller input.
    Skipped,
    /// No disc or movie file could be located.
    Unavailable,
    /// Playback encountered an irrecoverable drive error or timeout.
    Interrupted,
}

#[derive(Copy, Clone, PartialEq, Eq)]
enum SlotState {
    Filling,
    Ready,
    Decoding,
    Free,
}

/// BSS-resident streaming and decode storage.
struct VideoStorage {
    slots: [[u32; SLOT_WORDS]; SLOTS],
    rle: [u32; RLE_WORDS],
    column: [u32; COLUMN_WORDS],
    sector: [u32; SECTOR_WORDS],
}

impl VideoStorage {
    const fn new() -> Self {
        Self {
            slots: [[0; SLOT_WORDS]; SLOTS],
            rle: [0; RLE_WORDS],
            column: [0; COLUMN_WORDS],
            sector: [0; SECTOR_WORDS],
        }
    }
}

static mut STORAGE: VideoStorage = VideoStorage::new();

pub struct VideoPlayer {
    pub finished: bool,
    pub using_cd: bool,
    pub cd_start_lba: u32,
    pub cd_total_sectors: u32,
    pub last_swap_vblank: u32,
    pub vram_dma_fallbacks: u32,
    pub vram_dma_columns: u32,
    pub frames_shown: u16,
    pub decode_errors: u16,
    pub dropped_frames: u16,
    pub cd_errors: u16,
    pub restarts: u32,
    pub stalled: bool,
    pub pumped_sectors: u32,
    pub overlapped_sectors: u32,

    next_flip_vblank: u32,
    saved_irq_mask: u32,
    irq_mask_saved: bool,

    asm: strfmt::FrameAssembler,
    fill_slot: usize,
    last_frame_seen: u32,
    slot_state: [SlotState; SLOTS],
    ready_queue: [usize; SLOTS],
    ready_len: usize,
    cd_reader: SectorReader,
    vram_dma_ok: bool,
    next_lba: u32,
    stream_live: bool,
    last_sector_vblank: u32,
    wait_started_vblank: u32,
    pub eof: bool,
}

impl Default for VideoPlayer {
    fn default() -> Self {
        Self::new()
    }
}

impl VideoPlayer {
    pub fn new() -> Self {
        Self {
            finished: false,
            using_cd: false,
            cd_start_lba: 0,
            cd_total_sectors: 0,
            last_swap_vblank: psx_rt::interrupts::vblank_count(),
            vram_dma_fallbacks: 0,
            vram_dma_columns: 0,
            frames_shown: 0,
            decode_errors: 0,
            dropped_frames: 0,
            cd_errors: 0,
            restarts: 0,
            stalled: false,
            pumped_sectors: 0,
            overlapped_sectors: 0,
            next_flip_vblank: 0,
            saved_irq_mask: 0,
            irq_mask_saved: false,
            asm: strfmt::FrameAssembler::new(),
            fill_slot: 0,
            last_frame_seen: 0,
            slot_state: [SlotState::Free; SLOTS],
            ready_queue: [0; SLOTS],
            ready_len: 0,
            cd_reader: SectorReader::new(),
            vram_dma_ok: true,
            next_lba: 0,
            stream_live: false,
            last_sector_vblank: 0,
            wait_started_vblank: 0,
            eof: false,
        }
    }

    pub fn start_video_named(&mut self, filename: &[u8]) {
        self.reset_for_playback();

        mdec::reset();
        let _ = mdec::load_tables();

        let storage = unsafe { &mut *addr_of_mut!(STORAGE) };

        crate::dbg::println("[VIDEO] Preparing CD reader at single speed (75 sec/s)...");
        let prepared = unsafe { self.cd_reader.prepare_single_speed() };
        crate::dbg::print("[VIDEO] prepare_single_speed returned ");
        crate::dbg::println(if prepared { "true" } else { "false" });

        psx_io::irq::set_mask(1 << psx_io::irq::source::VBLANK);

        if prepared {
            if let Some((lba, sector_count)) =
                unsafe { Self::find_movie_lba(filename, &mut self.cd_reader, storage) }
            {
                crate::dbg::print("[VIDEO] Located movie: LBA=0x");
                crate::dbg::print_hex(lba);
                crate::dbg::print(" total_sectors=");
                crate::dbg::print_dec(sector_count);
                crate::dbg::println("");

                self.cd_start_lba = lba;
                self.cd_total_sectors = sector_count;
                self.next_lba = lba;
                self.using_cd = true;
                self.fill_slot = 0;
                self.slot_state[0] = SlotState::Filling;
                let started = unsafe { self.cd_reader.start_read(lba) };
                crate::dbg::print("[VIDEO] start_read returned ");
                crate::dbg::println(if started { "true" } else { "false" });
                if started {
                    self.stream_live = true;
                }
            } else {
                crate::dbg::println("[VIDEO] Failed to locate movie file on disc!");
            }
        }
    }

    fn reset_for_playback(&mut self) {
        let now = psx_rt::interrupts::vblank_count();
        self.finished = false;
        self.using_cd = false;
        self.cd_start_lba = 0;
        self.cd_total_sectors = 0;
        self.vram_dma_fallbacks = 0;
        self.vram_dma_columns = 0;
        self.frames_shown = 0;
        self.decode_errors = 0;
        self.dropped_frames = 0;
        self.cd_errors = 0;
        self.restarts = 0;
        self.stalled = false;
        self.pumped_sectors = 0;
        self.overlapped_sectors = 0;
        self.next_flip_vblank = now;
        self.last_swap_vblank = now;
        self.asm = strfmt::FrameAssembler::new();
        self.fill_slot = 0;
        self.last_frame_seen = 0;
        self.slot_state = [SlotState::Free; SLOTS];
        self.ready_queue = [0; SLOTS];
        self.ready_len = 0;
        self.vram_dma_ok = true;
        self.next_lba = 0;
        self.stream_live = false;
        self.last_sector_vblank = now;
        self.wait_started_vblank = now;
        self.eof = false;
        self.saved_irq_mask = psx_io::irq::mask();
        self.irq_mask_saved = true;
    }

    fn take_ready_slot(&mut self) -> Option<usize> {
        if self.ready_len == 0 {
            return None;
        }
        let slot = self.ready_queue[0];
        self.ready_queue.copy_within(1..self.ready_len, 0);
        self.ready_len -= 1;
        Some(slot)
    }

    unsafe fn find_movie_lba(
        filename: &[u8],
        reader: &mut SectorReader,
        storage: &mut VideoStorage,
    ) -> Option<(u32, u32)> {
        let mut found = None;
        unsafe {
            if reader.start_read(20) {
                let ok = reader.read_sector(&mut storage.sector);
                reader.stop();
                if ok {
                    let bytes: &[u8] =
                        core::slice::from_raw_parts(storage.sector.as_ptr() as *const u8, 2048);
                    let mut off = 0usize;
                    while off < bytes.len() {
                        let record_len = bytes[off] as usize;
                        if record_len == 0 {
                            break;
                        }
                        if record_len < 33 || off + record_len > bytes.len() {
                            break;
                        }
                        let lba = u32::from_le_bytes([
                            bytes[off + 2],
                            bytes[off + 3],
                            bytes[off + 4],
                            bytes[off + 5],
                        ]);
                        let data_len = u32::from_le_bytes([
                            bytes[off + 10],
                            bytes[off + 11],
                            bytes[off + 12],
                            bytes[off + 13],
                        ]);
                        let sector_count = data_len.div_ceil(2048);
                        let name_len = bytes[off + 32] as usize;
                        if name_len != 0 && off + 33 + name_len <= bytes.len() {
                            let name = &bytes[off + 33..off + 33 + name_len];
                            let matches = name.starts_with(filename)
                                && (name.len() == filename.len() || name[filename.len()] == b';');
                            if matches {
                                found = Some((lba, sector_count));
                                break;
                            }
                        }
                        off += record_len;
                    }
                }
            }
        }
        if found.is_some() {
            found
        } else if filename.starts_with(b"INTRO.STR") {
            // Mastered disc layout fallback: INTRO.STR starts at LBA 1024, 755 sectors
            Some((1024, 755))
        } else {
            None
        }
    }

    fn pump(&mut self, storage: &mut VideoStorage) {
        while self.using_cd && !self.eof {
            if self.cd_total_sectors > 0 && self.pumped_sectors >= self.cd_total_sectors {
                self.eof = true;
                if self.stream_live {
                    unsafe { self.cd_reader.stop() };
                    self.stream_live = false;
                }
                return;
            }

            if self.slot_state[self.fill_slot] != SlotState::Filling && !self.acquire_slot() {
                if self.stream_live {
                    unsafe { self.cd_reader.stop() };
                    self.stream_live = false;
                }
                return;
            }
            if !self.stream_live {
                if !unsafe { self.cd_reader.start_read(self.next_lba) } {
                    self.using_cd = false;
                    return;
                }
                self.stream_live = true;
                self.restarts += 1;
            }

            match unsafe { self.cd_reader.try_read_sector(&mut storage.sector) } {
                Ok(false) => return,
                Ok(true) => {}
                Err(()) => {
                    self.cd_errors += 1;
                    self.using_cd = false;
                    return;
                }
            }
            self.pumped_sectors += 1;
            self.next_lba = self.next_lba.wrapping_add(1);
            self.last_sector_vblank = psx_rt::interrupts::vblank_count();

            let sector: &[u8] =
                unsafe { core::slice::from_raw_parts(storage.sector.as_ptr() as *const u8, 2048) };

            let Some(chunk) = strfmt::Chunk::parse(sector) else {
                if self.last_frame_seen > 0 {
                    self.eof = true;
                    if self.stream_live {
                        unsafe { self.cd_reader.stop() };
                        self.stream_live = false;
                    }
                    return;
                }
                continue;
            };
            if chunk.frame == 1 && self.last_frame_seen > 1 {
                self.eof = true;
                if self.stream_live {
                    unsafe { self.cd_reader.stop() };
                    self.stream_live = false;
                }
                return;
            }
            self.last_frame_seen = chunk.frame;

            let dropped_before = self.asm.dropped;
            let buf: &mut [u8] = unsafe {
                let p = addr_of_mut!(storage.slots[self.fill_slot]) as *mut u8;
                core::slice::from_raw_parts_mut(p, SLOT_WORDS * 4)
            };
            if self.asm.add(sector, buf).is_some() {
                self.slot_state[self.fill_slot] = SlotState::Ready;
                if self.ready_len < SLOTS {
                    self.ready_queue[self.ready_len] = self.fill_slot;
                    self.ready_len += 1;
                }
                self.dropped_frames += (self.asm.dropped - dropped_before) as u16;
                self.acquire_slot();
            }
        }
    }

    fn acquire_slot(&mut self) -> bool {
        if let Some(s) = (0..SLOTS).find(|&s| self.slot_state[s] == SlotState::Free) {
            self.slot_state[s] = SlotState::Filling;
            self.fill_slot = s;
            true
        } else if let Some(oldest) = self.take_ready_slot() {
            self.dropped_frames += 1;
            self.slot_state[oldest] = SlotState::Filling;
            self.fill_slot = oldest;
            true
        } else {
            false
        }
    }

    pub fn update(&mut self, pad: &PadState, prev: &ButtonState) -> bool {
        if self.finished {
            return true;
        }
        let just_cross = pad.buttons.is_held(button::CROSS) && !prev.is_held(button::CROSS);
        let just_start = pad.buttons.is_held(button::START) && !prev.is_held(button::START);
        let just_circle = pad.buttons.is_held(button::CIRCLE) && !prev.is_held(button::CIRCLE);
        if just_cross || just_start || just_circle {
            self.stop();
            return true;
        }
        false
    }

    pub fn present(&mut self, fb: &mut FrameBuffer) {
        if self.finished {
            psx_rt::interrupts::wait_vblank();
            return;
        }
        let storage = unsafe { &mut *addr_of_mut!(STORAGE) };

        if !self.using_cd && !self.eof {
            self.stop();
            return;
        }

        // 1. Ensure at least one frame is ready before presenting (2 on startup for jitter buffer).
        let min_buffered = if self.frames_shown == 0 { 2 } else { 1 };
        if self.ready_len < min_buffered && !self.eof && self.using_cd {
            self.wait_started_vblank = psx_rt::interrupts::vblank_count();
            loop {
                self.pump(storage);
                if self.ready_len >= min_buffered || self.eof || !self.using_cd {
                    break;
                }
                let idle =
                    psx_rt::interrupts::vblank_count().wrapping_sub(self.wait_started_vblank);
                if idle > STALL_VBLANKS {
                    self.stalled = true;
                    break;
                }
                core::hint::spin_loop();
            }
        }

        // 2. Pop oldest frame in strict FIFO sequence, decode and upload.
        let popped_slot = self.take_ready_slot();
        if let Some(slot) = popped_slot {
            self.slot_state[slot] = SlotState::Decoding;
            self.decode_and_upload(slot, fb, storage);
            self.slot_state[slot] = SlotState::Free;
        } else if self.frames_shown > 0 && (self.eof || self.stalled) {
            self.stop();
            return;
        }

        // 3. Pace to 15 fps (every 4 VBlanks), pumping drive continuously.
        while (psx_rt::interrupts::vblank_count().wrapping_sub(self.next_flip_vblank) as i32) < 0 {
            self.pump(storage);
        }

        // 4. Flip on VBlank boundary.
        let v0 = psx_rt::interrupts::vblank_count();
        while psx_rt::interrupts::vblank_count() == v0 {
            self.pump(storage);
        }

        if popped_slot.is_some() {
            fb.swap();
            self.last_swap_vblank = psx_rt::interrupts::vblank_count();
            self.frames_shown += 1;
        }
        self.next_flip_vblank =
            psx_rt::interrupts::vblank_count().wrapping_add(VBLANKS_PER_VIDEO_FRAME - 1);

        // 5. Check movie completion.
        let drained = self.ready_len == 0 && popped_slot.is_none();
        if self.frames_shown > 0 && ((self.eof || self.stalled) && drained) {
            self.stop();
        }
    }

    fn decode_and_upload(&mut self, slot: usize, fb: &FrameBuffer, storage: &mut VideoStorage) {
        let frame: &[u8] = unsafe {
            let p = addr_of_mut!(storage.slots[slot]) as *const u8;
            core::slice::from_raw_parts(p, SLOT_WORDS * 4)
        };
        let rle16 = unsafe {
            core::slice::from_raw_parts_mut(addr_of_mut!(storage.rle) as *mut u16, RLE_WORDS * 2)
        };

        let selfp: *mut VideoPlayer = self;
        let storp: *mut VideoStorage = storage;
        let words = bs::decode_frame(frame, rle16, COLUMNS as u32 * ROWS, ROWS, &mut || unsafe {
            let before = (*selfp).pumped_sectors;
            (*selfp).pump(&mut *storp);
            (*selfp).overlapped_sectors += (*selfp).pumped_sectors - before;
        });

        let words = match words {
            Ok(w) if w > 0 => w,
            _ => {
                self.decode_errors += 1;
                return;
            }
        };

        unsafe {
            mdec::decode_start(
                core::slice::from_raw_parts(addr_of_mut!(storage.rle) as *const u32, words),
                words,
                mdec::DECODE_15BPP,
            );
        }

        let fb_y = fb.buffer_y(fb.drawing);
        let mut ok = true;
        for c in 0..COLUMNS {
            if !mdec::read_column(&mut storage.column) {
                ok = false;
                break;
            }
            let rect = VramRect::new(c * 16, fb_y, 16, VIDEO_H);
            if self.vram_dma_ok && dma_copy_to_vram(rect, storage.column.as_ptr()) {
                self.vram_dma_columns += 1;
            } else {
                if self.vram_dma_ok {
                    self.vram_dma_ok = false;
                    self.vram_dma_fallbacks += 1;
                }
                upload_words(rect, &storage.column);
            }
            let before = self.pumped_sectors;
            self.pump(storage);
            self.overlapped_sectors += self.pumped_sectors - before;
        }

        let finished = mdec::decode_finish();
        if !finished || !ok {
            self.decode_errors += 1;
            mdec::reset();
            let _ = mdec::load_tables();
            dma::abort(dma::Channel::MdecIn);
            dma::abort(dma::Channel::MdecOut);
        }
    }

    pub fn stop(&mut self) {
        if self.finished {
            return;
        }
        self.finished = true;

        if self.irq_mask_saved {
            unsafe {
                self.cd_reader.stop();
                psx_io::irq::set_mask(1 << psx_io::irq::source::VBLANK);
                psx_io::irq::ack(1 << psx_io::irq::source::CONTROLLER);
            }
            self.irq_mask_saved = false;
        }
        self.stream_live = false;
        self.using_cd = false;
    }

    pub fn is_finished(&self) -> bool {
        self.finished
    }
}

/// Plays a named STR video file from CD-ROM at locked 15 FPS.
pub fn play_video(filename: &str) -> VideoResult {
    psx_rt::interrupts::install_vblank_counter();

    let mut fb = FrameBuffer::new(VIDEO_W, VIDEO_H);
    fb.clear(0, 0, 0);

    let mut player = VideoPlayer::new();
    player.start_video_named(filename.as_bytes());

    if !player.using_cd {
        crate::dbg::println("[VIDEO] Disc streaming not active. Returning Unavailable.");
        return VideoResult::Unavailable;
    }

    crate::dbg::println("[VIDEO] Entering video presentation loop...");
    let mut prev_buttons = psx_pad::ButtonState::NONE;
    let mut skipped = false;
    let mut log_timer = 0u32;

    while !player.is_finished() {
        let pad = psx_pad::poll_port1();
        if player.update(&pad, &prev_buttons) {
            crate::dbg::println("[VIDEO] Controller input detected -> skipping FMV");
            skipped = true;
            break;
        }
        prev_buttons = pad.buttons;
        player.present(&mut fb);

        log_timer += 1;
        if log_timer.is_multiple_of(30) {
            crate::dbg::print("[VIDEO] Progress: frames_shown=");
            crate::dbg::print_dec(player.frames_shown as u32);
            crate::dbg::print(" pumped_sectors=");
            crate::dbg::print_dec(player.pumped_sectors);
            crate::dbg::print(" cd_errors=");
            crate::dbg::print_dec(player.cd_errors as u32);
            crate::dbg::println("");
            crate::dbg::check_faults();
        }
    }

    crate::dbg::println("[VIDEO] Video playback loop finished. Stopping player...");
    player.stop();

    psx_gpu::set_draw_area(0, 0, VIDEO_W - 1, VIDEO_H - 1);
    psx_gpu::set_draw_offset(0, 0);
    fb.clear(0, 0, 0);
    gpu::draw_sync();

    let res = if skipped {
        VideoResult::Skipped
    } else if player.cd_errors > 0 || player.stalled {
        VideoResult::Interrupted
    } else {
        VideoResult::Completed
    };
    crate::dbg::print("[VIDEO] Result: ");
    match res {
        VideoResult::Completed => crate::dbg::println("Completed"),
        VideoResult::Skipped => crate::dbg::println("Skipped"),
        VideoResult::Interrupted => crate::dbg::println("Interrupted"),
        VideoResult::Unavailable => crate::dbg::println("Unavailable"),
    }
    res
}
