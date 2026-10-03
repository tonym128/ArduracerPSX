//! MDEC Full-Motion Video (FMV) Playback Engine for Arduracer PSX.
//!
//! Streams and decodes 320x240 @ 15 fps video files (.STR) from the CD-ROM drive
//! using the PlayStation MDEC coprocessor and DMA Channel 0/1.

#![allow(dead_code)]

use psx_fmv::{bs, iso, mdec, str::FrameAssembler};
use psx_pack::cd::{SectorReader, SECTOR_WORDS};
use psx_pad::{button, poll_port1};
use psx_rt::interrupts;
use psx_vram::VramRect;

const WIDTH: u16 = 320;
const HEIGHT: u16 = 240;
const COLUMNS: u16 = WIDTH / 16;
const ROWS: u32 = HEIGHT as u32 / 16;
const COLUMN_WORDS: usize = 8 * HEIGHT as usize;
const VBLANKS_PER_FRAME: u32 = 4; // 60Hz / 15fps
const SLOT_WORDS: usize = 16 * 2016 / 4;
const RLE_WORDS: usize = 32 * 1024;
const STALL_VBLANKS: u32 = 120; // 2 seconds timeout

static mut READER: SectorReader = SectorReader::new();
static mut SECTOR: [u32; SECTOR_WORDS] = [0; SECTOR_WORDS];
static mut FRAME_BUF: [u8; 32 * 2016] = [0; 32 * 2016];
static mut RLE: [u32; RLE_WORDS] = [0; RLE_WORDS];
static mut COLUMN: [u32; COLUMN_WORDS] = [0; COLUMN_WORDS];

/// Plays an FMV video file from CD-ROM.
/// Returns true if played to completion, or false if skipped by user or file not found.
pub fn play_video(file_name: &str) -> bool {
    // 1. Initialize MDEC hardware
    mdec::reset();
    if !mdec::load_tables() {
        return false;
    }

    // 2. Read Primary Volume Descriptor to find file on CD-ROM
    let pvd_bytes = match read_single_sector(iso::PVD_LBA) {
        Some(b) => b,
        None => return false,
    };
    let (root_lba, _) = match iso::root_directory(pvd_bytes) {
        Some(r) => r,
        None => return false,
    };
    let root_bytes = match read_single_sector(root_lba) {
        Some(b) => b,
        None => return false,
    };
    let (movie_lba, movie_sectors) = match iso::find_in_directory(root_bytes, file_name) {
        Some(f) => f,
        None => return false,
    };

    if movie_sectors == 0 {
        return false;
    }

    // 3. Start streaming read
    unsafe {
        if !(*core::ptr::addr_of_mut!(READER)).start_read(movie_lba) {
            return false;
        }
    }

    let mut asm = FrameAssembler::new();
    let mut back_y: u16 = 240;
    let mut last_sector_vblank = interrupts::vblank_count();
    let mut next_flip = interrupts::vblank_count();
    let mut skipped = false;

    let rle = unsafe { &mut *core::ptr::addr_of_mut!(RLE) };
    let rle16 =
        unsafe { core::slice::from_raw_parts_mut(rle.as_mut_ptr() as *mut u16, RLE_WORDS * 2) };
    let frame_buf = unsafe { &mut *core::ptr::addr_of_mut!(FRAME_BUF) };
    let column = unsafe { &mut *core::ptr::addr_of_mut!(COLUMN) };

    loop {
        // Check user skip input (START, CROSS, CIRCLE)
        let pad = poll_port1();
        if pad.buttons.is_held(button::START)
            || pad.buttons.is_held(button::CROSS)
            || pad.buttons.is_held(button::CIRCLE)
        {
            skipped = true;
            break;
        }

        // Try reading next sector
        let got = unsafe {
            (*core::ptr::addr_of_mut!(READER))
                .try_read_sector(&mut *core::ptr::addr_of_mut!(SECTOR))
        };

        match got {
            Ok(true) => {
                last_sector_vblank = interrupts::vblank_count();
                let sector_bytes = unsafe {
                    core::slice::from_raw_parts(
                        core::ptr::addr_of_mut!(SECTOR) as *const u8,
                        SECTOR_WORDS * 4,
                    )
                };

                if let Some(frame) = asm.add(sector_bytes, frame_buf) {
                    // Frame reassembled! Decode bitstream -> RLE
                    let decoded = bs::decode_frame(
                        &frame_buf[..frame.size as usize],
                        rle16,
                        COLUMNS as u32 * ROWS,
                        ROWS,
                        &mut || {},
                    );

                    if let Ok(words) = decoded {
                        unsafe { mdec::decode_start(rle, words, mdec::DECODE_15BPP) };

                        let mut ok = true;
                        for c in 0..COLUMNS {
                            if !mdec::read_column(column) {
                                ok = false;
                                break;
                            }
                            psx_vram::upload_words(
                                VramRect::new(c * 16, back_y, 16, HEIGHT),
                                column,
                            );
                        }

                        if ok && mdec::decode_finish() {
                            // Pace to 15 FPS
                            while (interrupts::vblank_count().wrapping_sub(next_flip) as i32) < 0 {
                                interrupts::wait_vblank();
                            }
                            interrupts::wait_vblank();
                            psx_io::gpu::write_gp1(0x0500_0000 | ((back_y as u32) << 10));
                            next_flip =
                                interrupts::vblank_count().wrapping_add(VBLANKS_PER_FRAME - 1);
                            back_y = if back_y == 0 { 240 } else { 0 };
                        }
                    }
                }
            }
            Ok(false) => {
                let idle = interrupts::vblank_count().wrapping_sub(last_sector_vblank);
                if idle > STALL_VBLANKS {
                    break;
                }
                interrupts::wait_vblank();
            }
            Err(_) => break,
        }
    }

    unsafe { (*core::ptr::addr_of_mut!(READER)).stop() };
    !skipped
}

fn read_single_sector(lba: u32) -> Option<&'static [u8]> {
    unsafe {
        let r = &mut *core::ptr::addr_of_mut!(READER);
        if !r.start_read(lba) {
            return None;
        }
        let ok = r.read_sector(&mut *core::ptr::addr_of_mut!(SECTOR));
        r.stop();
        if !ok {
            return None;
        }
        Some(core::slice::from_raw_parts(
            core::ptr::addr_of_mut!(SECTOR) as *const u8,
            SECTOR_WORDS * 4,
        ))
    }
}
