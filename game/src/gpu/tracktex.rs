//! Streamed Track Texture Renderer with Hierarchical RAM and VRAM LRU Caching.
//!
//! # Architecture
//!
//! 1. **RAM Block Cache (1024x1024 blocks @ <= 500 KB each):**
//!    Main PSX RAM maintains a single resident 1024x1024 compressed block.
//!    Blocks are streamed from CD-ROM only when crossing large 1024-texel
//!    geographic boundaries.
//!
//! 2. **VRAM Tile Cache (64x64 blocks @ 8 KB each):**
//!    PSX VRAM maintains a 64-slot LRU cache across 4 texture pages (16 slots of
//!    64x64 per 256x256 Tpage = 64 slots total = 512 KB VRAM pool).
//!
//! 3. **CD-ROM Decoupling & Audio Continuity:**
//!    64x64 tiles are decompressed on-demand directly from the in-memory block
//!    in RAM into VRAM. This NEVER touches the CD-ROM drive during active
//!    gameplay, guaranteeing that CD-DA Redbook audio plays with zero
//!    interruptions, zero head seeks, and zero audio stutter.
//!
//! 4. **Massive Playfield Scalability:**
//!    The coordinate addressing maps arbitrary world extents to an unbounded 2D
//!    grid of 1024x1024 blocks. Both RAM and VRAM consumption stay strictly
//!    constant and bounded regardless of world size.
//!
//! # Codecs
//!
//! Two codecs are supported, selected per circuit by the generated table in
//! [`arduracer_core::visual_tex`]:
//!
//! - **QOI** (`arduracer_core::qoi`): segmented Quite OK Image blocks. Roughly 4x
//!   cheaper per MCU row than JPEG, because decoding a colour run needs no
//!   Huffman symbol decode and no inverse DCT, and the per-CD-load segment
//!   index is O(segments) rather than a linear scan of the whole entropy stream.
//!
//! - **JPEG** (`arduracer_core::jpeg`): baseline blocks with DRI=4 restart
//!   markers. Still the fallback for any circuit whose art does not fit the
//!   500 KB block budget losslessly, which QOI cannot do for highly detailed
//!   imagery.
//!
//! Both share one 500 KB RAM buffer. A second buffer for the other codec would
//! cost another 512 KB of the 2 MB main RAM, which does not fit the static-RAM
//! ceiling, and only one codec is ever active for a given circuit anyway.

use arduracer_core::visual_tex;
use arduracer_core::{Fixed, TrackDef, Vec2};

use crate::cd_fs::DiscReader;
use crate::gpu::camera::Camera;
use crate::gpu::texlayout::{
    self, BLOCK_DIM, TILES_PER_BLOCK_AXIS, TRACK_TPAGE_COUNT, TRACK_VRAM_SLOTS, VRAM_SLOT_COUNT,
    VRAM_TILE_DIM,
};
use psx_gpu as gpu;
use psx_gpu::material::TextureMaterial;
use psx_vram::{upload_16bpp, TexDepth, Tpage, VramRect};

/// Capacity of the in-RAM compressed block cache (one 1024x1024 block, <= 500 KB).
pub const RAM_CACHE_BLOCKS: usize = 1;

/// Maximum number of 64x64 tiles decompressed per frame (staggers decoding to eliminate frame drops).
pub const MAX_DECODES_PER_FRAME: usize = 1;

/// Texels per 64x64 VRAM tile.
const TILE_TEXELS: usize = 64;
/// Halfwords per 64x64 VRAM tile.
const TILE_PIXELS: usize = TILE_TEXELS * TILE_TEXELS;

/// Which compressed codec a resident RAM block holds.
#[derive(Copy, Clone, PartialEq, Eq)]
enum Codec {
    Jpeg,
    Qoi,
    /// SVG-derived base raster plus a per-surface detail pass.
    Arsv,
}

/// Both codecs index exactly 1024 independently decodable spans per 1024x1024
/// block: JPEG restart intervals (DRI=4) and QOI segments are the same
/// partition of the image, visited in the same order. One array therefore
/// serves either codec, and switching codecs cannot leave a stale index behind
/// because the entry is rebuilt on every block load.
const BLOCK_SPANS: usize = 1024;

/// Precomputed Tpage descriptors for each of the 4 allocated 256x256 VRAM regions.
const TPAGES: [Tpage; TRACK_TPAGE_COUNT] = [
    Tpage::new(
        TRACK_VRAM_SLOTS[0].0,
        TRACK_VRAM_SLOTS[0].1,
        TexDepth::Bit15,
    ),
    Tpage::new(
        TRACK_VRAM_SLOTS[1].0,
        TRACK_VRAM_SLOTS[1].1,
        TexDepth::Bit15,
    ),
    Tpage::new(
        TRACK_VRAM_SLOTS[2].0,
        TRACK_VRAM_SLOTS[2].1,
        TexDepth::Bit15,
    ),
    Tpage::new(
        TRACK_VRAM_SLOTS[3].0,
        TRACK_VRAM_SLOTS[3].1,
        TexDepth::Bit15,
    ),
];

/// An in-RAM resident 1024x1024 compressed image block.
///
/// `block_data` is shared by both codecs: only one is loaded per circuit, and
/// holding a second 512 KB buffer would exceed the static-RAM ceiling.
struct RamBlock {
    resident_circuit: usize,
    resident_block_x: usize,
    resident_block_y: usize,
    last_used: u32,
    codec: Codec,
    /// Restart-interval (JPEG) or segment (QOI) offsets. See [`BLOCK_SPANS`].
    offsets: [u32; BLOCK_SPANS],
    jpeg_header: arduracer_core::jpeg::JpegHeader,
    qoi_header: arduracer_core::qoi::QoiHeader,
    /// Parsed `ARSV` header and base-raster segment index.
    arsv_block: arduracer_core::svgtex::SvgBlock,
    block_data: [u8; visual_tex::BLOCK_MAX_BYTES],
    byte_len: usize,
    valid: bool,
}

impl RamBlock {
    const fn new() -> Self {
        Self {
            resident_circuit: 0,
            resident_block_x: 0,
            resident_block_y: 0,
            last_used: 0,
            codec: Codec::Jpeg,
            offsets: [0; BLOCK_SPANS],
            jpeg_header: arduracer_core::jpeg::JpegHeader {
                width: 0,
                height: 0,
                restart_interval: 0,
                q_tables: [[0; 64]; 2],
                dc_huffman: [arduracer_core::jpeg::HuffTable::empty(); 2],
                ac_huffman: [arduracer_core::jpeg::HuffTable::empty(); 2],
                scan_offset: 0,
            },
            qoi_header: arduracer_core::qoi::QoiHeader {
                width: 0,
                height: 0,
                channels: 0,
                segment_count: 0,
                segment_width: 0,
                segment_height: 0,
            },
            arsv_block: arduracer_core::svgtex::SvgBlock::empty(),
            block_data: [0u8; visual_tex::BLOCK_MAX_BYTES],
            byte_len: 0,
            valid: false,
        }
    }
}

/// A resident 64x64 tile in the VRAM cache.
#[derive(Copy, Clone)]
struct VramSlot {
    resident_circuit: usize,
    resident_block_x: usize,
    resident_block_y: usize,
    resident_tile_x: usize,
    resident_tile_y: usize,
    last_used: u32,
}

impl VramSlot {
    const fn empty() -> Self {
        Self {
            resident_circuit: usize::MAX,
            resident_block_x: usize::MAX,
            resident_block_y: usize::MAX,
            resident_tile_x: usize::MAX,
            resident_tile_y: usize::MAX,
            last_used: 0,
        }
    }
}

/// Active incremental tile decode job (time-sliced over 4 consecutive frames).
#[derive(Copy, Clone)]
struct DecodeJob {
    active: bool,
    circuit: usize,
    block_x: usize,
    block_y: usize,
    tile_x: usize,
    tile_y: usize,
    target_slot: usize,
    next_row: usize, // 0..4 (4 means all rows decoded and uploaded)
}

impl DecodeJob {
    const fn empty() -> Self {
        Self {
            active: false,
            circuit: usize::MAX,
            block_x: usize::MAX,
            block_y: usize::MAX,
            tile_x: usize::MAX,
            tile_y: usize::MAX,
            target_slot: usize::MAX,
            next_row: 0,
        }
    }
}

/// 1-slot unified RAM block holding the 1024x1024 compressed map (<= 500 KB).
static mut RAM_BLOCKS: [RamBlock; RAM_CACHE_BLOCKS] = [RamBlock::new()];

/// 64-slot VRAM LRU cache holding decompressed 64x64 tiles in 15bpp direct colour.
static mut VRAM_SLOTS: [VramSlot; VRAM_SLOT_COUNT] = [VramSlot::empty(); VRAM_SLOT_COUNT];

/// Active incremental decode job stepping 1 MCU row per frame (~20% frame budget).
static mut ACTIVE_JOB: DecodeJob = DecodeJob::empty();

/// Temporary scratch buffer for 64x64 tile decompression (4096 halfwords = 8 KB).
static mut TILE_SCRATCH: [u16; TILE_PIXELS] = [0u16; TILE_PIXELS];

static mut ACTIVE_CIRCUIT: usize = 0;
static mut FRAME_COUNTER: u32 = 0;

static mut DISC_READER: DiscReader = DiscReader::new();
static mut TRACKS_BIN_LBA: Option<u32> = None;
static mut TRACKS_PROBED: bool = false;

/// LBA of the segmented-QOI circuit table, when the disc carries one.
///
/// Optional on purpose: a disc mastered before this feature, or one where the
/// QOI asset failed to fit, simply has no such file. Probing is therefore
/// non-fatal and the JPEG table is used instead.
static mut QOITRACKS_BIN_LBA: Option<u32> = None;

/// LBA of the SVG-derived circuit asset. Optional for the same reason as
/// [`QOITRACKS_BIN_LBA`]: an older disc, or one cooked before this format
/// existed, simply has no such file and every circuit falls back.
static mut SVGTRACKS_BIN_LBA: Option<u32> = None;

static mut CAPETOWN_BIN_LBA: Option<u32> = None;
static mut MELBOURNE_BIN_LBA: Option<u32> = None;
static mut LONDON_BIN_LBA: Option<u32> = None;
static mut ACTIVE_CITY: usize = 0; // 0 = standard circuits, 99 = Cape Town, 98 = Melbourne, 97 = London

/// Whether QOI streaming is compiled in for this build.
#[cfg(feature = "qoi-track-stream")]
const QOI_ENABLED: bool = true;
#[cfg(not(feature = "qoi-track-stream"))]
const QOI_ENABLED: bool = false;

/// Whether the SVG-derived texture path is compiled in for this build.
#[cfg(feature = "svg-track-texture")]
const SVG_ENABLED: bool = true;
#[cfg(not(feature = "svg-track-texture"))]
const SVG_ENABLED: bool = false;

/// Chooses the codec for a circuit, best available first.
///
/// Three formats, in descending order of what they give the player:
///
/// 1. **SVG** (`ARSV`): the authored vector geometry rasterised at full 1024,
///    with a per-surface detail pass synthesised at decode. Smoothest edges and
///    the only format that adds texture rather than reproducing a stored one.
/// 2. **QOI**: lossless copy of the same raster, no detail pass.
/// 3. **JPEG**: lossy, and the only option for circuits whose art is too
///    detailed to fit the 500 KB block losslessly.
///
/// Every step down is a pure fallback: the feature flag off, the disc missing
/// the asset, or the circuit's table entry empty all fall through to the next.
/// A disc cooked before this format existed therefore still renders.
#[inline]
fn codec_for_circuit(circuit: usize) -> Codec {
    // Copied out of the static rather than read through a shared reference:
    // the LBAs are written during the one-shot probe in `init_track_texture`,
    // and a shared borrow of a mutable static outlives what it needs to.
    let have_qoi_asset = unsafe { QOITRACKS_BIN_LBA }.is_some();
    let have_svg_asset = unsafe { SVGTRACKS_BIN_LBA }.is_some();
    let known = circuit < visual_tex::COUNT;

    if SVG_ENABLED
        && have_svg_asset
        && known
        && visual_tex::CIRCUIT_ARSV_SECTORS[circuit].sector_count > 0
    {
        Codec::Arsv
    } else if QOI_ENABLED
        && have_qoi_asset
        && known
        && visual_tex::CIRCUIT_QOI_SECTORS[circuit].sector_count > 0
    {
        Codec::Qoi
    } else {
        Codec::Jpeg
    }
}

/// Prepares the track texture streaming cache for a new circuit or city.
#[allow(clippy::needless_range_loop)]
pub fn init_track_texture(circuit: usize) {
    let city_id = if circuit == 99 || circuit == 98 || circuit == 97 {
        circuit
    } else {
        0
    };
    unsafe {
        ACTIVE_CITY = city_id;
        ACTIVE_CIRCUIT = circuit;
        let vram_slots = &mut *core::ptr::addr_of_mut!(VRAM_SLOTS);
        for i in 0..VRAM_SLOT_COUNT {
            vram_slots[i] = VramSlot::empty();
        }
        let ram_blocks = &mut *core::ptr::addr_of_mut!(RAM_BLOCKS);
        for i in 0..RAM_CACHE_BLOCKS {
            ram_blocks[i].resident_circuit = usize::MAX;
            ram_blocks[i].valid = false;
            ram_blocks[i].last_used = 0;
        }
        ACTIVE_JOB = DecodeJob::empty();
        FRAME_COUNTER = 0;

        if !TRACKS_PROBED {
            let reader = &mut *core::ptr::addr_of_mut!(DISC_READER);
            crate::dbg::println(
                "[TRACKTEX] Probing disc for TRACKS.BIN, CAPETOWN.BIN, MELBOURNE.BIN, LONDON.BIN...",
            );
            TRACKS_BIN_LBA = reader.find_file_lba(visual_tex::TRACKS_BIN_NAME);
            QOITRACKS_BIN_LBA = reader.find_file_lba(visual_tex::QOITRACKS_BIN_NAME);
            SVGTRACKS_BIN_LBA = reader.find_file_lba(visual_tex::SVGTRACKS_BIN_NAME);
            CAPETOWN_BIN_LBA = reader.find_file_lba(visual_tex::CAPETOWN_BIN_NAME);
            MELBOURNE_BIN_LBA = reader.find_file_lba(visual_tex::MELBOURNE_BIN_NAME);
            LONDON_BIN_LBA = reader.find_file_lba(visual_tex::LONDON_BIN_NAME);
            crate::dbg::print("[TRACKTEX] TRACKS: 0x");
            if let Some(lba) = TRACKS_BIN_LBA {
                crate::dbg::print_hex(lba);
            } else {
                crate::dbg::print("NONE");
            }
            crate::dbg::print("  CAPETOWN: 0x");
            if let Some(lba) = CAPETOWN_BIN_LBA {
                crate::dbg::print_hex(lba);
            } else {
                crate::dbg::print("NONE");
            }
            crate::dbg::print("  MELBOURNE: 0x");
            if let Some(lba) = MELBOURNE_BIN_LBA {
                crate::dbg::print_hex(lba);
            } else {
                crate::dbg::print("NONE");
            }
            crate::dbg::print("  LONDON: 0x");
            if let Some(lba) = LONDON_BIN_LBA {
                crate::dbg::print_hex(lba);
            } else {
                crate::dbg::print("NONE");
            }
            // Reported last because it is the one that decides the codec: a
            // disc without it (or a build with the flag off) silently streams
            // every circuit from JPEG, and this line is how that shows up.
            crate::dbg::print("  QOITRACKS: 0x");
            if let Some(lba) = QOITRACKS_BIN_LBA {
                crate::dbg::print_hex(lba);
            } else {
                crate::dbg::print("NONE");
            }
            crate::dbg::print("  SVGTRACKS: 0x");
            if let Some(lba) = SVGTRACKS_BIN_LBA {
                crate::dbg::print_hex(lba);
            } else {
                crate::dbg::print("NONE");
            }
            crate::dbg::println("");
            TRACKS_PROBED = true;
        }

        // Preload initial 1024x1024 block for the circuit into RAM slot 0 before race starts
        // Cape Town: block (1, 1) covers Helen Suzman Blvd & Green Point stadium spawn
        // Melbourne: block (1, 2) covers Albert Park pit straight spawn (cell 125, 177)
        // London: block (1, 1) covers The Mall spawn (cell 89, 147)
        let (init_bx, init_by) = match city_id {
            99 => (1, 1),
            98 => (1, 2),
            97 => (1, 1),
            _ => (0, 0),
        };
        crate::dbg::print("[TRACKTEX] circuit ");
        crate::dbg::print_dec(circuit as u32);
        crate::dbg::print(" codec=");
        match codec_for_circuit(circuit) {
            Codec::Arsv => crate::dbg::print("svg"),
            Codec::Qoi => crate::dbg::print("qoi"),
            Codec::Jpeg => crate::dbg::print("jpeg"),
        }
        crate::dbg::println("");
        load_block_into_ram(circuit, init_bx, init_by, 0);
    }
}

/// Decodes one 64x16 MCU row of a resident block into `scratch`, dispatching on
/// the block's codec.
///
/// Both codecs address the same segment of the same tile, so this is the only
/// place the two differ. Anything other than a healthy block leaves `scratch`
/// untouched and returns false, and the caller paints its fallback pattern.
#[inline(always)]
fn decode_row(
    ram_slot: &RamBlock,
    tile_x: usize,
    tile_y: usize,
    row: usize,
    scratch: &mut [u16; TILE_PIXELS],
) -> bool {
    if !ram_slot.valid || ram_slot.byte_len == 0 {
        return false;
    }
    let data = &ram_slot.block_data[..ram_slot.byte_len];

    match ram_slot.codec {
        Codec::Jpeg => {
            arduracer_core::jpeg::decode_tile_row_64x16(
                data,
                &ram_slot.jpeg_header,
                &ram_slot.offsets,
                tile_x,
                tile_y,
                row,
                scratch,
            );
            true
        }
        Codec::Qoi => {
            arduracer_core::qoi::decode_tile_row_64x16(
                data,
                &ram_slot.qoi_header,
                &ram_slot.offsets,
                tile_x,
                tile_y,
                row,
                scratch,
            );
            true
        }
        Codec::Arsv => arduracer_core::svgtex::decode_tile_row_64x16(
            data,
            &ram_slot.arsv_block,
            tile_x,
            tile_y,
            row,
            scratch,
        ),
    }
}

/// Paints the procedural placeholder used when a block could not be decoded, so
/// the tile still renders as something rather than garbage.
#[inline(always)]
fn paint_fallback_row(start_y: usize, scratch: &mut [u16; TILE_PIXELS]) {
    for y in start_y..start_y + 16 {
        for x in 0..TILE_TEXELS {
            let c = if ((x >> 3) ^ (y >> 3)) & 1 == 0 {
                0x1CE7
            } else {
                0x2108
            };
            scratch[y * TILE_TEXELS + x] = c;
        }
    }
}

#[inline(never)]
fn decode_and_upload_full_tile(
    circuit: usize,
    bx: usize,
    by: usize,
    sub_tx: usize,
    sub_ty: usize,
    target_slot: usize,
) {
    let ram_slot_idx = ensure_ram_block(circuit, bx, by, 0);
    let ram_slot = unsafe { &RAM_BLOCKS[ram_slot_idx] };
    let scratch = unsafe { &mut *core::ptr::addr_of_mut!(TILE_SCRATCH) };

    for row in 0..4 {
        if !decode_row(ram_slot, sub_tx, sub_ty, row, scratch) {
            paint_fallback_row(row * 16, scratch);
        }
    }

    let (vx, vy, _, _, _) = texlayout::vram_slot_coords(target_slot);
    let rect = VramRect::new(vx, vy, VRAM_TILE_DIM, VRAM_TILE_DIM);
    upload_16bpp(rect, scratch);

    let vram_slots = unsafe { &mut *core::ptr::addr_of_mut!(VRAM_SLOTS) };
    vram_slots[target_slot].resident_circuit = circuit;
    vram_slots[target_slot].resident_block_x = bx;
    vram_slots[target_slot].resident_block_y = by;
    vram_slots[target_slot].resident_tile_x = sub_tx;
    vram_slots[target_slot].resident_tile_y = sub_ty;
    vram_slots[target_slot].last_used = 1;
}

/// Pre-warms the starting grid tiles directly into VRAM so the race starts with 100% texture residency.
#[inline(never)]
pub fn prewarm_starting_tiles(world_w: i32, world_h: i32, camera: &Camera) {
    let (half_w, half_h) = camera.visible_half_extents();
    let cam_x = camera.pos.x.to_int();
    let cam_y = camera.pos.y.to_int();

    let x0 = (cam_x - half_w).max(0);
    let x1 = (cam_x + half_w).min(world_w);
    let y0 = (cam_y - half_h).max(0);
    let y1 = (cam_y + half_h).min(world_h);

    if x0 >= x1 || y0 >= y1 {
        return;
    }

    let active_circuit = unsafe { ACTIVE_CIRCUIT };
    let block_wu_x = if world_w <= (BLOCK_DIM as i32 * 5 / 2) {
        world_w.max(1)
    } else {
        BLOCK_DIM as i32 * 5 / 2
    };
    let block_wu_y = if world_h <= (BLOCK_DIM as i32 * 5 / 2) {
        world_h.max(1)
    } else {
        BLOCK_DIM as i32 * 5 / 2
    };

    let tile_wu_x = (block_wu_x / (TILES_PER_BLOCK_AXIS as i32)).max(1);
    let tile_wu_y = (block_wu_y / (TILES_PER_BLOCK_AXIS as i32)).max(1);

    let min_tx = (x0 / tile_wu_x).max(0);
    let max_tx = (x1 - 1).max(0) / tile_wu_x;
    let min_ty = (y0 / tile_wu_y).max(0);
    let max_ty = (y1 - 1).max(0) / tile_wu_y;

    let mut next_slot = 0usize;
    for ty in min_ty..=max_ty {
        for tx in min_tx..=max_tx {
            let wx0 = tx * tile_wu_x;
            let wy0 = ty * tile_wu_y;
            let bx = (wx0 / block_wu_x) as usize;
            let by = (wy0 / block_wu_y) as usize;
            let sub_tx = ((wx0 % block_wu_x) / tile_wu_x)
                .clamp(0, (TILES_PER_BLOCK_AXIS - 1) as i32) as usize;
            let sub_ty = ((wy0 % block_wu_y) / tile_wu_y)
                .clamp(0, (TILES_PER_BLOCK_AXIS - 1) as i32) as usize;

            let slot_idx = next_slot % VRAM_SLOT_COUNT;
            decode_and_upload_full_tile(active_circuit, bx, by, sub_tx, sub_ty, slot_idx);

            next_slot += 1;
            if next_slot >= VRAM_SLOT_COUNT {
                break;
            }
        }
        if next_slot >= VRAM_SLOT_COUNT {
            break;
        }
    }
}

/// Streams a 1024x1024 compressed block (<= 500 KB) from CD-ROM into the
/// specified RAM cache slot.
#[inline(never)]
fn load_block_into_ram(circuit: usize, block_x: usize, block_y: usize, slot_idx: usize) {
    let city = unsafe { ACTIVE_CITY };
    let codec = codec_for_circuit(circuit);

    // City blocks (Cape Town / Melbourne / London) are shipped as JPEG only;
    // their 10 km^2 art is procedurally generated at cook time and has no
    // SVG or QOI counterpart, so they always take the JPEG table.
    let use_arsv = codec == Codec::Arsv && city == 0;
    let use_qoi = codec == Codec::Qoi && city == 0;

    let (bin_lba, entry) = match city {
        99 => {
            let b_idx = (block_y * 3 + block_x).min(8);
            (
                unsafe { CAPETOWN_BIN_LBA },
                visual_tex::CAPETOWN_BLOCK_SECTORS[b_idx],
            )
        }
        98 => {
            let b_idx = (block_y * 3 + block_x).min(8);
            (
                unsafe { MELBOURNE_BIN_LBA },
                visual_tex::MELBOURNE_BLOCK_SECTORS[b_idx],
            )
        }
        97 => {
            let b_idx = (block_y * 3 + block_x).min(8);
            (
                unsafe { LONDON_BIN_LBA },
                visual_tex::LONDON_BLOCK_SECTORS[b_idx],
            )
        }
        _ => {
            let c = circuit % visual_tex::COUNT;
            if use_arsv {
                (
                    unsafe { SVGTRACKS_BIN_LBA },
                    visual_tex::CIRCUIT_ARSV_SECTORS[c],
                )
            } else if use_qoi {
                (
                    unsafe { QOITRACKS_BIN_LBA },
                    visual_tex::CIRCUIT_QOI_SECTORS[c],
                )
            } else {
                (
                    unsafe { TRACKS_BIN_LBA },
                    visual_tex::CIRCUIT_BLOCK_SECTORS[c],
                )
            }
        }
    };
    let ram_slot = unsafe { &mut (*core::ptr::addr_of_mut!(RAM_BLOCKS))[slot_idx] };
    let mut loaded = false;

    if let Some(lba) = bin_lba {
        let block_lba = lba + entry.sector_offset;
        let sector_count = entry.sector_count as usize;
        let byte_len = entry.byte_len as usize;
        let reader = unsafe { &mut *core::ptr::addr_of_mut!(DISC_READER) };

        crate::prof::count(crate::prof::C_CD_BLOCKS, 1);
        crate::prof::count(crate::prof::C_CD_SECTORS, sector_count as u32);
        let _read = crate::prof::span(crate::prof::TRACK_CD_READ);
        if sector_count <= visual_tex::BLOCK_SECTORS
            && reader.read_sectors(block_lba, sector_count, &mut ram_slot.block_data)
        {
            ram_slot.byte_len = byte_len;
            ram_slot.codec = if use_arsv {
                Codec::Arsv
            } else if use_qoi {
                Codec::Qoi
            } else {
                Codec::Jpeg
            };

            // The header parse and the index walk are separated from the sector
            // read because they are different work at different scales: the
            // read is bounded by the drive, the index walk is CPU work over the
            // whole compressed block. QOI's index is O(segments); JPEG's is a
            // linear scan of the entire entropy-coded segment.
            let _index = crate::prof::span(crate::prof::TRACK_BLOCK_INDEX);
            let block = &ram_slot.block_data[..byte_len];

            if use_arsv {
                // `parse_block` does the header parse and the base-raster
                // segment index together; both are once-per-CD-load work, same
                // as the JPEG restart scan, so they share one profiled span.
                if let Some(svg_block) = arduracer_core::svgtex::parse_block(block) {
                    debug_assert_eq!(svg_block.offsets.len(), BLOCK_SPANS);
                    ram_slot.arsv_block = svg_block;
                    loaded = true;
                }
            } else if use_qoi {
                if let Some(header) = arduracer_core::qoi::QoiHeader::parse(block) {
                    debug_assert_eq!(header.segment_count as usize, BLOCK_SPANS);
                    arduracer_core::qoi::index_segments(block, &header, &mut ram_slot.offsets);
                    ram_slot.qoi_header = header;
                    loaded = true;
                }
            } else if let Some(header) = arduracer_core::jpeg::JpegHeader::parse(block) {
                header.index_restarts(block, &mut ram_slot.offsets);
                ram_slot.jpeg_header = header;
                loaded = true;
            }

            if loaded {
                ram_slot.resident_circuit = circuit;
                ram_slot.resident_block_x = block_x;
                ram_slot.resident_block_y = block_y;
                ram_slot.valid = true;
            }
        }
    }

    if !loaded {
        // Fallback pattern if disc read fails
        ram_slot.resident_circuit = circuit;
        ram_slot.resident_block_x = block_x;
        ram_slot.resident_block_y = block_y;
        ram_slot.byte_len = 0;
        ram_slot.valid = false;
    }
}

/// Ensures the required 1024x1024 block is resident in RAM, returning its RAM slot index.
#[inline(never)]
#[allow(clippy::needless_range_loop)]
fn ensure_ram_block(circuit: usize, block_x: usize, block_y: usize, current_frame: u32) -> usize {
    let blocks = unsafe { &mut *core::ptr::addr_of_mut!(RAM_BLOCKS) };
    for i in 0..RAM_CACHE_BLOCKS {
        let b = &mut blocks[i];
        if b.valid
            && b.resident_circuit == circuit
            && b.resident_block_x == block_x
            && b.resident_block_y == block_y
        {
            b.last_used = current_frame;
            return i;
        }
    }

    // Find least recently used RAM slot
    let mut best_slot = 0;
    let mut oldest_age = u32::MAX;
    for i in 0..RAM_CACHE_BLOCKS {
        if !blocks[i].valid {
            best_slot = i;
            break;
        }
        if blocks[i].last_used < oldest_age {
            oldest_age = blocks[i].last_used;
            best_slot = i;
        }
    }

    load_block_into_ram(circuit, block_x, block_y, best_slot);
    blocks[best_slot].last_used = current_frame;
    best_slot
}

/// Advances the active time-sliced decode job by 1 MCU row (64x16 pixels).
///
/// Time-slicing keeps each frame's decode cost bounded regardless of codec:
/// the JPEG path spends ~20% of the frame budget on a row, QOI noticeably less.
/// When row 3 finishes, the full 64x64 tile is uploaded to VRAM and marked resident.
#[inline(never)]
fn step_decode_job(current_frame: u32) {
    let job = unsafe { &mut *core::ptr::addr_of_mut!(ACTIVE_JOB) };
    if !job.active {
        return;
    }

    let ram_slot_idx = ensure_ram_block(job.circuit, job.block_x, job.block_y, current_frame);
    let ram_slot = unsafe { &RAM_BLOCKS[ram_slot_idx] };
    let scratch = unsafe { &mut *core::ptr::addr_of_mut!(TILE_SCRATCH) };

    crate::prof::count(crate::prof::C_DECODE_ROWS, 1);
    {
        let _s = crate::prof::span(crate::prof::TRACK_DECODE);
        if !decode_row(ram_slot, job.tile_x, job.tile_y, job.next_row, scratch) {
            // Block missing or undecodable: paint the fallback pattern for this row.
            paint_fallback_row(job.next_row * 16, scratch);
        }
    }

    job.next_row += 1;
    if job.next_row >= 4 {
        // All 4 MCU rows (64x64 pixels) complete! Upload to VRAM.
        crate::prof::count(crate::prof::C_TILE_DECODES, 1);
        let (vx, vy, _, _, _) = texlayout::vram_slot_coords(job.target_slot);
        let rect = VramRect::new(vx, vy, VRAM_TILE_DIM, VRAM_TILE_DIM);
        {
            let _s = crate::prof::span(crate::prof::TRACK_VRAM_UPLOAD);
            upload_16bpp(rect, scratch);
        }

        let vram_slots = unsafe { &mut *core::ptr::addr_of_mut!(VRAM_SLOTS) };
        vram_slots[job.target_slot].resident_circuit = job.circuit;
        vram_slots[job.target_slot].resident_block_x = job.block_x;
        vram_slots[job.target_slot].resident_block_y = job.block_y;
        vram_slots[job.target_slot].resident_tile_x = job.tile_x;
        vram_slots[job.target_slot].resident_tile_y = job.tile_y;
        vram_slots[job.target_slot].last_used = current_frame;

        job.active = false;
    }
}

/// Draws the visible world window as textured quads, streaming tiles dynamically.
pub fn render_track(
    track: &TrackDef,
    camera: &Camera,
    last_flip_vblank: u32,
    max_decode_rows: usize,
) {
    render_track_extents(
        track.world_width(),
        track.world_height(),
        camera,
        last_flip_vblank,
        max_decode_rows,
    );
}

/// Helper that steps the active MCU row decode job or schedules the next visible un-cached tile.
/// Returns true if 1 MCU row was decoded, or false if all visible tiles are already resident in VRAM.
#[inline(always)]
fn step_or_schedule_decode_row(
    current_frame: u32,
    active_circuit: usize,
    visible_tiles: &[(i32, i32, usize, usize, usize, usize); 48],
    vis_count: usize,
    slot_for_tile: &mut [usize; 48],
    used_slots: &mut [bool; VRAM_SLOT_COUNT],
    vram_slots: &mut [VramSlot; VRAM_SLOT_COUNT],
) -> bool {
    let job = unsafe { &mut *core::ptr::addr_of_mut!(ACTIVE_JOB) };
    if !job.active {
        // No active decode job running: check if any visible tile needs decoding
        let mut scheduled = false;
        for i in 0..vis_count {
            let (_, _, bx, by, sub_tx, sub_ty) = visible_tiles[i];
            let s = slot_for_tile[i];
            let is_hit = s != usize::MAX && {
                let slot = &vram_slots[s];
                slot.resident_circuit == active_circuit
                    && slot.resident_block_x == bx
                    && slot.resident_block_y == by
                    && slot.resident_tile_x == sub_tx
                    && slot.resident_tile_y == sub_ty
            };
            if is_hit {
                continue;
            }

            // Find oldest VRAM slot not currently visible in this frame
            let mut best_slot = 0;
            let mut oldest_age = u32::MAX;
            for s_idx in 0..VRAM_SLOT_COUNT {
                if !used_slots[s_idx] {
                    let slot = &vram_slots[s_idx];
                    if slot.resident_circuit == usize::MAX {
                        best_slot = s_idx;
                        break;
                    }
                    if slot.last_used < oldest_age {
                        oldest_age = slot.last_used;
                        best_slot = s_idx;
                    }
                }
            }

            // Reserve this slot so subsequent decodes in this frame won't evict it
            used_slots[best_slot] = true;

            // Schedule incremental decode job starting at MCU row 0
            *job = DecodeJob {
                active: true,
                circuit: active_circuit,
                block_x: bx,
                block_y: by,
                tile_x: sub_tx,
                tile_y: sub_ty,
                target_slot: best_slot,
                next_row: 0,
            };
            scheduled = true;
            break;
        }

        if !scheduled {
            return false;
        }
    }

    let job_bx = job.block_x;
    let job_by = job.block_y;
    let job_tx = job.tile_x;
    let job_ty = job.tile_y;
    let job_slot = job.target_slot;

    step_decode_job(current_frame);

    // If step_decode_job completed all 4 rows, job.active is now false and the tile
    // was uploaded to VRAM. Update slot_for_tile so Phase 3 renders it immediately textured without placeholder!
    if !job.active {
        for i in 0..vis_count {
            let (_, _, bx, by, sub_tx, sub_ty) = visible_tiles[i];
            if bx == job_bx && by == job_by && sub_tx == job_tx && sub_ty == job_ty {
                slot_for_tile[i] = job_slot;
                used_slots[job_slot] = true;
            }
        }
    }

    true
}

/// Renders the world texture clamped to arbitrary world extents (tracks or 10km x 10km cities).
#[inline(never)]
#[allow(clippy::needless_range_loop)]
pub fn render_track_extents(
    world_w: i32,
    world_h: i32,
    camera: &Camera,
    last_flip_vblank: u32,
    max_decode_rows: usize,
) {
    let (half_w, half_h) = camera.visible_half_extents();
    let cam_x = camera.pos.x.to_int();
    let cam_y = camera.pos.y.to_int();

    let x0 = (cam_x - half_w).max(0);
    let x1 = (cam_x + half_w).min(world_w);
    let y0 = (cam_y - half_h).max(0);
    let y1 = (cam_y + half_h).min(world_h);

    if x0 >= x1 || y0 >= y1 {
        return;
    }

    unsafe {
        FRAME_COUNTER = FRAME_COUNTER.wrapping_add(1);
    }
    let current_frame = unsafe { FRAME_COUNTER };
    let active_circuit = unsafe { ACTIVE_CIRCUIT };
    let vram_slots = unsafe { &mut *core::ptr::addr_of_mut!(VRAM_SLOTS) };

    // Compute exact world units per 64x64 tile along X and Y axes.
    // For single tracks: 1 block (1024x1024) covers the entire track (world_w x world_h).
    // For large cities (> 2560 world units): partitioned into blocks of 2560 world units.
    let block_wu_x = if world_w <= (BLOCK_DIM as i32 * 5 / 2) {
        world_w.max(1)
    } else {
        BLOCK_DIM as i32 * 5 / 2
    };
    let block_wu_y = if world_h <= (BLOCK_DIM as i32 * 5 / 2) {
        world_h.max(1)
    } else {
        BLOCK_DIM as i32 * 5 / 2
    };

    let tile_wu_x = (block_wu_x / (TILES_PER_BLOCK_AXIS as i32)).max(1);
    let tile_wu_y = (block_wu_y / (TILES_PER_BLOCK_AXIS as i32)).max(1);

    let min_tx = (x0 / tile_wu_x).max(0);
    let max_tx = (x1 - 1).max(0) / tile_wu_x;
    let min_ty = (y0 / tile_wu_y).max(0);
    let max_ty = (y1 - 1).max(0) / tile_wu_y;

    // Maximum 48 visible tiles per frame on standard screen resolution (320x240)
    let mut visible_tiles: [(i32, i32, usize, usize, usize, usize); 48] = [(0, 0, 0, 0, 0, 0); 48];
    let mut vis_count = 0;

    {
        // Scoped in a block on purpose. A `let _span = ...` bound at function
        // scope would live until the end of the function, so this row would
        // silently include phase 1, the decodes and phase 3 as well.
        let _visible = crate::prof::span(crate::prof::TRACK_VISIBLE_SCAN);
        for ty in min_ty..=max_ty {
            for tx in min_tx..=max_tx {
                let wx0 = tx * tile_wu_x;
                let wy0 = ty * tile_wu_y;
                let tx0 = x0.max(wx0);
                let tx1 = x1.min(wx0 + tile_wu_x);
                let ty0 = y0.max(wy0);
                let ty1 = y1.min(wy0 + tile_wu_y);

                if tx0 < tx1 && ty0 < ty1 && vis_count < 48 {
                    let bx = (wx0 / block_wu_x) as usize;
                    let by = (wy0 / block_wu_y) as usize;
                    let sub_tx = ((wx0 % block_wu_x) / tile_wu_x)
                        .clamp(0, (TILES_PER_BLOCK_AXIS - 1) as i32)
                        as usize;
                    let sub_ty = ((wy0 % block_wu_y) / tile_wu_y)
                        .clamp(0, (TILES_PER_BLOCK_AXIS - 1) as i32)
                        as usize;

                    visible_tiles[vis_count] = (tx, ty, bx, by, sub_tx, sub_ty);
                    vis_count += 1;
                }
            }
        }
    }

    let mut slot_for_tile = [usize::MAX; 48];
    let mut used_slots = [false; VRAM_SLOT_COUNT];

    // Phase 1: Match already-resident 64x64 tiles in VRAM LRU cache
    {
        let _probe = crate::prof::span(crate::prof::TRACK_CACHE_PROBE);
        for i in 0..vis_count {
            let (_, _, bx, by, sub_tx, sub_ty) = visible_tiles[i];
            let mut found = None;

            for s_idx in 0..VRAM_SLOT_COUNT {
                let slot = &vram_slots[s_idx];
                if slot.resident_circuit == active_circuit
                    && slot.resident_block_x == bx
                    && slot.resident_block_y == by
                    && slot.resident_tile_x == sub_tx
                    && slot.resident_tile_y == sub_ty
                {
                    found = Some(s_idx);
                    break;
                }
            }

            if let Some(s_idx) = found {
                slot_for_tile[i] = s_idx;
                used_slots[s_idx] = true;
                vram_slots[s_idx].last_used = current_frame;
                crate::prof::count(crate::prof::C_TILE_HITS, 1);
            }
        }
    }
    crate::prof::set(crate::prof::C_VISIBLE_TILES, vis_count as u32);

    // Phase 2: Progress the active time-sliced decode job dynamically based on frame budget.
    // Up to `max_decode_rows` (e.g. 1 or 2) can be processed, provided the frame remains within
    // the first VBlank period. If VBlank 1 fires during decode or budget is exhausted, stop.
    let mut rows_decoded = 0;
    while rows_decoded < max_decode_rows {
        let elapsed_vblanks = psx_rt::interrupts::vblank_count().wrapping_sub(last_flip_vblank);
        if rows_decoded > 0 && elapsed_vblanks > 0 {
            break;
        }
        if elapsed_vblanks >= 2 {
            break;
        }

        let did_work = step_or_schedule_decode_row(
            current_frame,
            active_circuit,
            &visible_tiles,
            vis_count,
            &mut slot_for_tile,
            &mut used_slots,
            vram_slots,
        );
        if !did_work {
            break;
        }
        rows_decoded += 1;
    }

    // Phase 3: Render visible quads (textured for resident tiles, placeholder flat for pending)
    let _quads = crate::prof::span(crate::prof::TRACK_QUADS);
    for i in 0..vis_count {
        let (tx, ty, _, _, _, _) = visible_tiles[i];
        let s_idx = slot_for_tile[i];

        if s_idx == usize::MAX {
            crate::prof::count(crate::prof::C_PLACEHOLDERS, 1);
        }

        let wx0 = tx * tile_wu_x;
        let wy0 = ty * tile_wu_y;
        let tx0 = x0.max(wx0);
        let tx1 = x1.min(wx0 + tile_wu_x);
        let ty0 = y0.max(wy0);
        let ty1 = y1.min(wy0 + tile_wu_y);

        let tl = camera.world_to_screen(Vec2 {
            x: Fixed::from_int(tx0),
            y: Fixed::from_int(ty0),
        });
        let tr = camera.world_to_screen(Vec2 {
            x: Fixed::from_int(tx1),
            y: Fixed::from_int(ty0),
        });
        let bl = camera.world_to_screen(Vec2 {
            x: Fixed::from_int(tx0),
            y: Fixed::from_int(ty1),
        });
        let br = camera.world_to_screen(Vec2 {
            x: Fixed::from_int(tx1),
            y: Fixed::from_int(ty1),
        });

        if s_idx == usize::MAX {
            // Pending tile in decode queue: render sleek placeholder shading
            let checker = ((tx ^ ty) & 1) != 0;
            let (pr, pg, pb) = texlayout::placeholder_tile_colour(true, checker);
            gpu::draw_quad_flat([tl, tr, bl, br], pr, pg, pb);
        } else {
            let (_, _, tpage_idx, u_base, v_base) = texlayout::vram_slot_coords(s_idx);

            // Map fractional position inside the tile to 0..64 UV texels
            let local_u0 = (((tx0 - wx0) * (VRAM_TILE_DIM as i32)) / tile_wu_x) as u16;
            let local_u1 = (((tx1 - wx0) * (VRAM_TILE_DIM as i32)) / tile_wu_x) as u16;
            let local_v0 = (((ty0 - wy0) * (VRAM_TILE_DIM as i32)) / tile_wu_y) as u16;
            let local_v1 = (((ty1 - wy0) * (VRAM_TILE_DIM as i32)) / tile_wu_y) as u16;

            let u0 = ((u_base as u16) + local_u0).min(255) as u8;
            let u1 = ((u_base as u16) + local_u1).min(255) as u8;
            let v0 = ((v_base as u16) + local_v0).min(255) as u8;
            let v1 = ((v_base as u16) + local_v1).min(255) as u8;

            let tpage = TPAGES[tpage_idx];
            let material = TextureMaterial::opaque(0, tpage.uv_tpage_word(0), (0x80, 0x80, 0x80));
            gpu::draw_quad_textured_material(
                [tl, tr, bl, br],
                [(u0, v0), (u1, v0), (u0, v1), (u1, v1)],
                material,
            );
        }
    }
}
