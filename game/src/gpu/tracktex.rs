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

pub use arduracer_core::texture_format::{Codec, Framerate, TextureFormat};
use arduracer_core::visual_tex;
use arduracer_core::{Fixed, TrackDef, Vec2, FP_SHIFT};

use crate::cd_fs::{DiscReader, SectorPoll};
use crate::gpu::camera::{
    Camera, M7_SCAN_RADIUS, M7_SCAN_RADIUS_COMPRESSED, M7_Z0, M7_Z_FAR, M7_Z_FAR_COMPRESSED,
    M7_Z_NEAR,
};
use crate::gpu::m7_grid;
use crate::gpu::m7_mesh;
use crate::gpu::texlayout::{
    self, MAX_VRAM_SLOTS, TILES_PER_BLOCK_AXIS, TILES_PER_TPAGE, TILES_PER_TPAGE_AXIS,
    TIM4_TPAGE_COUNT, TIM4_VRAM_PAGES, TIM8_TPAGE_COUNT, TIM8_VRAM_PAGES, TRACK_SLOT_COUNT,
    TRACK_TPAGE_COUNT, TRACK_VRAM_SLOTS, VRAM_SLOT_COUNT, VRAM_TILE_DIM,
};
use psx_gpu as gpu;
use psx_gpu::material::TextureMaterial;
use psx_hw::gpu::{gp0, pack_color, pack_vertex};
use psx_vram::{dma_copy_to_vram, upload_16bpp, upload_words, Clut, TexDepth, Tpage, VramRect};

/// Capacity of the in-RAM compressed block cache (one 1024x1024 block, <= 500 KB).
pub const RAM_CACHE_BLOCKS: usize = 1;

/// Maximum number of 64x64 tiles decompressed per frame (staggers decoding to eliminate frame drops).
pub const MAX_DECODES_PER_FRAME: usize = 1;

/// Maximum number of visible tiles tracked per frame (bounded to VRAM cache capacity of 64 slots).
pub const VISIBLE_TILES_MAX: usize = VRAM_SLOT_COUNT;

/// Texels per 64x64 VRAM tile.
const TILE_TEXELS: usize = 64;
/// Halfwords per 64x64 VRAM tile.
const TILE_PIXELS: usize = TILE_TEXELS * TILE_TEXELS;

// The codec enum and the format/pacing policy both live in `arduracer_core`:
// they are pure decisions, and `core` is where the host tests can reach them.

/// The player's texture-format preference, set from the menus and consulted by
/// [`codec_for_circuit`].
static mut TEXTURE_FORMAT: TextureFormat = TextureFormat::Auto;

/// VBlank periods in the current frame, mirrored from the player's framerate
/// choice.
///
/// This is a static rather than a parameter because the decode loop reads it
/// deep inside `render_track_extents`, behind two call sites that would
/// otherwise each have to thread a pacing argument they do not otherwise care
/// about. [`set_frame_period`] is the only writer.
static mut FRAME_PERIOD_VBLANKS: u32 = 2;

/// Records the current frame's VBlank period so the decode loop can stop before
/// it overruns the frame.
///
/// Called once per frame from the racing loop, before `render_track`. Getting
/// this wrong in the 60 FPS direction is not fatal (the loop simply admits
/// fewer rows), but in the other direction it would let decode run into the next
/// frame and cause the dropped VBlanks the setting exists to avoid.
pub fn set_frame_period(vblanks: u32) {
    unsafe { FRAME_PERIOD_VBLANKS = vblanks };
}

/// The current frame's VBlank period.
fn frame_period() -> u32 {
    let p = unsafe { FRAME_PERIOD_VBLANKS };
    if p == 0 {
        1
    } else {
        p
    }
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

#[repr(C, align(4))]
struct AlignedBlockData([u8; visual_tex::BLOCK_MAX_BYTES]);

const _: () = assert!(core::mem::align_of::<AlignedBlockData>() >= 4);

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
    block_data: AlignedBlockData,
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
            block_data: AlignedBlockData([0u8; visual_tex::BLOCK_MAX_BYTES]),
            byte_len: 0,
            valid: false,
        }
    }
}

/// Packs circuit and tile coordinates into a single 32-bit key for fast single-cycle cache probing.
#[inline(always)]
fn pack_tile_key(circuit: usize, bx: usize, by: usize, tx: usize, ty: usize) -> u32 {
    ((circuit as u32 & 0xFF) << 24)
        | ((bx as u32 & 0x0F) << 20)
        | ((by as u32 & 0x0F) << 16)
        | ((tx as u32 & 0xFF) << 8)
        | (ty as u32 & 0xFF)
}

/// A resident 64x64 tile in the VRAM cache.
#[derive(Copy, Clone)]
struct VramSlot {
    key: u32,
    /// The tile this slot has been filled for -- with its pixels once `key`
    /// matches, or its placeholder colour while it decodes. The grid floor
    /// samples a slot whatever it holds, so a slot must never show a tile
    /// other than the one the world places there; see [`claim_slot`].
    claim: u32,
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
            key: u32::MAX,
            claim: u32::MAX,
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

/// Active background block streaming job for compressed sheets (SVG/ARSV, QOI, JPEG).
#[derive(Copy, Clone)]
#[allow(dead_code)]
struct AsyncBlockJob {
    active: bool,
    circuit: usize,
    block_x: usize,
    block_y: usize,
    slot_idx: usize,
    block_lba: u32,
    sectors_total: usize,
    sectors_read: usize,
    byte_len: usize,
    codec: Codec,
    use_arsv: bool,
    use_qoi: bool,
    start_frame: u32,
}

impl AsyncBlockJob {
    const fn empty() -> Self {
        Self {
            active: false,
            circuit: usize::MAX,
            block_x: usize::MAX,
            block_y: usize::MAX,
            slot_idx: 0,
            block_lba: 0,
            sectors_total: 0,
            sectors_read: 0,
            byte_len: 0,
            codec: Codec::Jpeg,
            use_arsv: false,
            use_qoi: false,
            start_frame: 0,
        }
    }
}

/// 1-slot unified RAM block holding the 1024x1024 compressed map (<= 500 KB).
static mut RAM_BLOCKS: [RamBlock; RAM_CACHE_BLOCKS] = [RamBlock::new()];

/// Background staging sectors for compressed blocks (0 = direct streaming into resident RAM block).
pub const STAGING_SECTORS: usize = 0;

#[repr(align(4))]
struct StagingData([u8; STAGING_SECTORS * 2048]);

/// Background staging buffer for compressed blocks.
static mut BLOCK_STAGING: StagingData = StagingData([0u8; STAGING_SECTORS * 2048]);

/// VRAM LRU cache of decompressed 64x64 tiles.
///
/// Sized for the largest cache (256 slots at 4bpp, `NO1-034`); 8bpp uses the
/// first 128 and every direct-colour codec the first 64. See
/// [`TrackTexture::slot_count`].
static mut VRAM_SLOTS: [VramSlot; MAX_VRAM_SLOTS] = [VramSlot::empty(); MAX_VRAM_SLOTS];

/// Whether the floor is drawn as the screen-space grid (`m7_grid`), with every
/// visible tile at the slot its world position fixes ([`grid_slot`]).
static mut GRID_FLOOR: bool = false;

/// `slot_for_tile` value for a visible tile whose [`grid_slot`] a nearer
/// visible tile holds. Eight tiles apart is 1,280 world units, so this only
/// happens at the far corners of the view, ~1 tile a frame beyond 420 units;
/// such a tile is drawn flat over the grid instead.
const SLOT_CONFLICT: usize = usize::MAX - 1;

/// The cache slot a tile occupies on the grid floor: tile `(x mod 8, y mod 8)`,
/// so a 4x4 block of tiles is one 256x256 texture page and neighbouring tiles
/// are neighbouring texels. Blocks are 16 tiles a side, so the in-block tile
/// index has the same residues as the world one.
#[inline(always)]
const fn grid_slot(sub_tx: usize, sub_ty: usize) -> usize {
    let page = ((sub_ty >> 2) & 1) * 2 + ((sub_tx >> 2) & 1);
    page * TILES_PER_TPAGE + (sub_ty & 3) * TILES_PER_TPAGE_AXIS + (sub_tx & 3)
}

/// Whether this frame's floor can be the grid: a full-screen Mode 7 view of a
/// direct-colour circuit with 160-unit tiles, whose 64 slots form the 8x8
/// tile window [`grid_slot`] needs.
fn grid_floor_wanted(camera: &Camera, circuit: usize, tile_wu: (i32, i32)) -> bool {
    let tex = active_texture(circuit);
    camera.mode7
        && camera.viewport_w == 320
        && camera.viewport_h == 240
        && tex.bpp == 15
        && tex.slot_count() == TRACK_SLOT_COUNT
        && !codec_is_uncompressed(circuit)
        && tile_wu == (160, 160)
}

/// Fills `slot` with the placeholder colour of the tile `key` and makes it that
/// tile's, until its rows land. Cancels a decode still writing another tile
/// into it.
#[inline(never)]
fn claim_slot(vram_slots: &mut [VramSlot; MAX_VRAM_SLOTS], slot: usize, key: u32, checker: bool) {
    let job = unsafe { &mut *core::ptr::addr_of_mut!(ACTIVE_JOB) };
    if job.active && job.target_slot == slot {
        job.active = false;
    }
    // The fill and the uploads after it share the GPU's command stream with
    // the floor's DMA, so let that finish first, as an upload does.
    finish_floor_packets();
    let (x, y, _, _, _) = texlayout::vram_slot_coords(slot);
    let (r, g, b) = texlayout::placeholder_tile_colour(true, checker);
    gpu::fill_rect(x, y, VRAM_TILE_DIM, VRAM_TILE_DIM, r, g, b);
    vram_slots[slot] = VramSlot::empty();
    vram_slots[slot].claim = key;
}

/// Active incremental decode job stepping 1 MCU row per frame (~20% frame budget).
static mut ACTIVE_JOB: DecodeJob = DecodeJob::empty();

/// Active background block streaming job for compressed sheets.
static mut ASYNC_BLOCK_JOB: AsyncBlockJob = AsyncBlockJob::empty();

/// 64x64 tile staging buffer: 4096 halfwords (8 KB), shared by every codec.
///
/// 4-byte aligned rather than the 2 a `[u16; N]` would get. The TIM path DMAs
/// this buffer into VRAM directly, and `dma_copy_to_vram` refuses a source that
/// is not word-addressable; asking for 4 here satisfies that and still satisfies
/// the decoded-codec paths, which read it as `[u16]`.
#[repr(C, align(4))]
struct TileScratch([u16; TILE_PIXELS]);

const _: () = assert!(core::mem::align_of::<TileScratch>() >= 4);

/// The TIM path reads exactly one tile's sectors straight into this buffer, so
/// the buffer and the sector arithmetic have to agree on that tile's size. If
/// either moved alone the read would be short (leaving stale texels in the last
/// rows) or overlong (a panic at the copy), so pin both here.
const _: () = assert!(
    TILE_PIXELS * 2 == visual_tex::TIM_TILE_SECTORS as usize * 2048,
    "TILE_SCRATCH is not exactly one TIM tile"
);

/// The halfwords of the staging buffer.
///
/// Raw pointers rather than a reference: taking a mutable reference to a
/// mutable static is UB by the reference rules, and this is single threaded.
#[inline(always)]
fn scratch() -> *mut [u16; TILE_PIXELS] {
    unsafe { &mut (*core::ptr::addr_of_mut!(TILE_SCRATCH)).0 }
}

/// The staging buffer viewed as DMA words: 2048 words for a 64x64 tile.
#[inline(always)]
pub fn scratch_words() -> *mut [u32; TILE_PIXELS / 2] {
    // SAFETY: `TileScratch` is 4-byte aligned and holds exactly
    // `TILE_PIXELS` halfwords, so the same 8 KB is a valid `[u32;
    // TILE_PIXELS / 2]`. Nothing else reads the buffer while either view is
    // live; `upload_tile_dma` takes the word view and the decoders take the
    // halfword view, never both.
    unsafe {
        &mut *((core::ptr::addr_of_mut!(TILE_SCRATCH).cast::<u8>())
            .cast::<[u32; TILE_PIXELS / 2]>())
    }
}

static mut TILE_SCRATCH: TileScratch = TileScratch([0u16; TILE_PIXELS]);

static mut ACTIVE_CIRCUIT: usize = 0;
static mut FRAME_COUNTER: u32 = 0;

static mut DISC_READER: DiscReader = DiscReader::new();

/// The shared disc reader, for the non-blocking tile fetcher.
///
/// A function rather than a re-export so `async_tile` can reach it without
/// making the static itself public.
#[inline]
pub fn disc_reader() -> &'static mut DiscReader {
    unsafe { &mut *core::ptr::addr_of_mut!(DISC_READER) }
}
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

/// LBA of the uncompressed TIM circuit asset, when the disc carries one.
///
/// Optional like the two above it, and for a stronger reason: this asset is four
/// times the size of the compressed ones, so a disc mastered before it existed
/// has no such file. Probing is non-fatal and the pin degrades to Auto.
static mut TIMTRACKS_BIN_LBA: Option<u32> = None;

/// LBA of `LEVELS.BIN`, the packed tile grid for every shipped level.
static mut LEVELS_BIN_LBA: Option<u32> = None;

/// LBA of the uncompressed 4bpp (TIM4) circuit asset.
///
/// Optional for the same reason as [`TIMTRACKS_BIN_LBA`], and more so: TIM4 is
/// the newest format, so a disc mastered before it existed has no such file. A
/// TIM4 pin degrades to Auto rather than to a blank track.
static mut TIM4TRACKS_BIN_LBA: Option<u32> = None;

/// Circuit whose 16-colour palette is currently resident in the TIM4 CLUT.
///
/// Lets the palette be uploaded once per circuit rather than once per tile, and
/// makes `reset_texture_caches` able to invalidate it: the palette is VRAM
/// state, so dropping the tile cache without invalidating this would leave a
/// palette that no longer matches the tiles being drawn against it.
static mut TIM4_CLUT_CIRCUIT: usize = usize::MAX;

/// LBA of the uncompressed 8bpp (TIM8) circuit asset.
static mut TIM8TRACKS_BIN_LBA: Option<u32> = None;

/// Circuit whose 256-colour palette is currently resident in the TIM8 CLUT.
static mut TIM8_CLUT_CIRCUIT: usize = usize::MAX;

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

/// Whether the uncompressed TIM texture path is compiled in for this build.
#[cfg(feature = "tim-track-stream")]
const TIM_ENABLED: bool = true;
#[cfg(not(feature = "tim-track-stream"))]
const TIM_ENABLED: bool = false;

/// CD sectors per TIM tile, from the cooker-generated table.
///
/// Referenced rather than written out: the cooker derives it from
/// `TILE_DIM * TILE_DIM * 2`, so a literal `4` here would be a second answer to
/// the same question with nothing checking it against the first. Getting it
/// wrong does not fail loudly -- it reads four sectors' worth of the wrong
/// texels and the circuit renders as scrambled noise.
const TIM_TILE_SECTORS: u32 = visual_tex::TIM_TILE_SECTORS;

/// Whether `circuit` has an uncompressed block (`TIM`, `TIM4`, or `TIM8`) on
/// this disc, and where it starts `(block_lba, sector_count)`.
///
/// `None` covers every way the path can be unavailable: the feature is off, the
/// disc has no matching `.BIN`, or the cooker recorded no block for it.
#[inline]
fn uncompressed_block_for(
    codec: Codec,
    circuit: usize,
    block_x: usize,
    block_y: usize,
) -> Option<(u32, u32)> {
    let block_idx = visual_tex::circuit_block_index(circuit, block_x, block_y);
    if block_idx >= visual_tex::COUNT {
        return None;
    }
    let (enabled, bin_lba, entry) = match codec {
        Codec::Tim => (
            TIM_ENABLED,
            unsafe { TIMTRACKS_BIN_LBA },
            visual_tex::CIRCUIT_TIM_SECTORS[block_idx],
        ),
        Codec::Tim4 => (
            TIM4_ENABLED,
            unsafe { TIM4TRACKS_BIN_LBA },
            visual_tex::CIRCUIT_TIM4_SECTORS[block_idx],
        ),
        Codec::Tim8 => (
            TIM8_ENABLED,
            unsafe { TIM8TRACKS_BIN_LBA },
            visual_tex::CIRCUIT_TIM8_SECTORS[block_idx],
        ),
        _ => return None,
    };
    if !enabled || entry.sector_count == 0 {
        return None;
    }
    let lba = bin_lba?;
    Some((lba + entry.sector_offset, entry.sector_count))
}

#[inline]
fn tim_block_for(circuit: usize, block_x: usize, block_y: usize) -> Option<(u32, u32)> {
    uncompressed_block_for(Codec::Tim, circuit, block_x, block_y)
}

#[inline]
fn tim4_block_for(circuit: usize, block_x: usize, block_y: usize) -> Option<(u32, u32)> {
    uncompressed_block_for(Codec::Tim4, circuit, block_x, block_y)
}

#[inline]
fn tim8_block_for(circuit: usize, block_x: usize, block_y: usize) -> Option<(u32, u32)> {
    uncompressed_block_for(Codec::Tim8, circuit, block_x, block_y)
}

/// Whether `circuit` is served by the uncompressed TIM path.
///
/// Built from [`selects_tim`] rather than from [`tim_block_for`] alone:
/// availability is necessary, not sufficient -- `selects_tim` asks about the
/// pin too, and keeps that question out of the render loop.
#[inline]
fn codec_is_tim(circuit: usize) -> bool {
    arduracer_core::texture_format::selects_tim(
        unsafe { TEXTURE_FORMAT },
        tim_block_for(circuit, 0, 0).is_some(),
    )
}

/// Whether `circuit` is served by the uncompressed 4bpp path.
#[inline]
fn codec_is_tim4(circuit: usize) -> bool {
    arduracer_core::texture_format::selects_tim4(
        unsafe { TEXTURE_FORMAT },
        tim4_block_for(circuit, 0, 0).is_some(),
    )
}

/// Whether `circuit` is served by the uncompressed 8bpp path.
#[inline]
fn codec_is_tim8(circuit: usize) -> bool {
    arduracer_core::texture_format::selects_tim8(
        unsafe { TEXTURE_FORMAT },
        tim8_block_for(circuit, 0, 0).is_some(),
    )
}

/// Returns the active uncompressed streaming codec (`Tim`, `Tim4`, or `Tim8`)
/// for `circuit`, or `None` if `circuit` uses a RAM-block compressed codec
/// (`Arsv`, `Qoi`, or `Jpeg`).
#[inline]
fn uncompressed_codec_for(circuit: usize) -> Option<Codec> {
    if codec_is_tim(circuit) {
        Some(Codec::Tim)
    } else if codec_is_tim4(circuit) {
        Some(Codec::Tim4)
    } else if codec_is_tim8(circuit) {
        Some(Codec::Tim8)
    } else {
        None
    }
}

/// Whether `circuit` uses any direct-from-disc uncompressed/paletted format.
#[inline]
pub fn codec_is_uncompressed(circuit: usize) -> bool {
    uncompressed_codec_for(circuit).is_some()
}

/// Whether the uncompressed TIM4 texture path is compiled in for this build.
#[cfg(feature = "tim4-track-stream")]
const TIM4_ENABLED: bool = true;
#[cfg(not(feature = "tim4-track-stream"))]
const TIM4_ENABLED: bool = false;

/// Sectors per 4bpp tile, from the cooker-generated table. One, by construction:
/// a 64x64 4bpp tile is 2048 bytes.
const TIM4_TILE_SECTORS: u32 = visual_tex::TIM4_TILE_SECTORS;

/// The VRAM row holding a circuit's 16-colour TIM4 palette.
///
/// One row below the level palette at [`texlayout::TRACK_CLUT_Y`], which is
/// already occupied, and clear of the minimap above it. A CLUT is addressed as
/// a row of 16 halfwords at a 16-aligned X, so Y is a VRAM row index and any
/// free row is a free palette.
const TIM4_CLUT: Clut = Clut::new(texlayout::TRACK_CLUT_X, texlayout::TRACK_CLUT_Y + 1);

/// 4bpp VRAM rectangle for one 64x64 tile.
///
/// **Halfwords, not texels.** `GP0(A0h)` counts width in 16-bit words and
/// height in VRAM rows, and four 4-bit indices share a word, so a 64x64 *texel*
/// tile is a 32x32 *rect* of halfwords. Passing 64x64 here would move 8 KB into
/// a 2 KB slot and write over the neighbouring tiles.
///
/// The GPU applies the 4bpp block swizzle as it writes, which is why the disc
/// payload is plain row-major texels and no pre-swizzling happens here.
const TIM4_TILE_RECT: (u16, u16) = (
    visual_tex::TIM4_TILE_HALFWORDS_WIDE as u16,
    visual_tex::TIM4_TILE_ROWS as u16,
);

/// Three different divisions of the same 64x64 tile meet here, and they are all
/// easy to confuse with each other or with the 8bpp case.
const _: () = assert!(
    visual_tex::TIM4_TILE_HALFWORDS == TIM4_TILE_RECT.0 as usize * TIM4_TILE_RECT.1 as usize,
    "the 4bpp upload rect must hold exactly one tile's halfwords"
);
const _: () = assert!(
    visual_tex::TIM4_TILE_HALFWORDS * 2 == visual_tex::TIM4_TILE_BYTES,
    "halfword count and byte count must describe the same tile"
);
const _: () = assert!(
    TIM4_TILE_RECT.0 as usize == visual_tex::TIM4_TILE_HALFWORDS_WIDE,
    "the 4bpp upload rect must be as wide as 4 texels-per-halfword makes it"
);
const _: () = assert!(
    TIM4_TILE_RECT.1 as usize == visual_tex::TIM4_TILE_DIM,
    "4bpp packing halves the width, never the height"
);
const _: () = assert!(
    visual_tex::TIM4_TILE_BYTES as u32 == TIM4_TILE_SECTORS * 2048,
    "a 4bpp tile must be a whole number of CD sectors"
);

/// Whether the uncompressed TIM8 texture path is compiled in for this build.
#[cfg(feature = "tim8-track-stream")]
const TIM8_ENABLED: bool = true;
#[cfg(not(feature = "tim8-track-stream"))]
const TIM8_ENABLED: bool = false;

/// Sectors per 8bpp tile, from the cooker-generated table. Two, by construction:
/// a 64x64 8bpp tile is 4096 bytes.
const TIM8_TILE_SECTORS: u32 = visual_tex::TIM8_TILE_SECTORS;

/// The VRAM row holding a circuit's 256-colour TIM8 palette.
///
/// Row 480 (below dual 320x240 framebuffers, clear of track TPages at X=384..896).
const TIM8_CLUT: Clut = Clut::new(texlayout::TIM8_CLUT_X, texlayout::TIM8_CLUT_Y);

/// 8bpp VRAM rectangle for one 64x64 tile.
///
/// Halfwords, not texels. GP0(A0h) counts width in 16-bit words and height in VRAM rows.
/// Two 8-bit indices share a word, so a 64x64 texel tile is a 32x64 rect of halfwords.
const TIM8_TILE_RECT: (u16, u16) = (
    visual_tex::TIM8_TILE_HALFWORDS_WIDE as u16,
    visual_tex::TIM8_TILE_ROWS as u16,
);

const _: () = assert!(
    visual_tex::TIM8_TILE_HALFWORDS == TIM8_TILE_RECT.0 as usize * TIM8_TILE_RECT.1 as usize,
    "the 8bpp upload rect must hold exactly one tile's halfwords"
);
const _: () = assert!(
    visual_tex::TIM8_TILE_HALFWORDS * 2 == visual_tex::TIM8_TILE_BYTES,
    "halfword count and byte count must describe the same tile"
);
const _: () = assert!(
    TIM8_TILE_RECT.0 as usize == visual_tex::TIM8_TILE_HALFWORDS_WIDE,
    "the 8bpp upload rect must be as wide as 2 texels-per-halfword makes it"
);
const _: () = assert!(
    TIM8_TILE_RECT.1 as usize == visual_tex::TIM8_TILE_DIM,
    "8bpp packing halves the width, never the height"
);
const _: () = assert!(
    visual_tex::TIM8_TILE_BYTES as u32 == TIM8_TILE_SECTORS * 2048,
    "an 8bpp tile must be a whole number of CD sectors"
);

/// Sampling parameters for whichever codec is active.
///
/// 15bpp and 4bpp differ in three places and agree everywhere else: the tpage
/// depth, whether a CLUT is sampled alongside, and how many texels one U unit
/// spans. Bundling them means the quad emitters -- the flat path and the Mode 7
/// grid -- stay written once instead of gaining a `match codec` each.
#[derive(Copy, Clone)]
struct TrackTexture {
    /// Page table for this depth: 4 pages at 15bpp, 8 at 8bpp, 16 at 4bpp.
    tpages: &'static [Tpage],
    /// CLUT handle for indexed formats, 0 for 15bpp direct colour.
    clut_word: u16,
    /// Texels per U unit. Always 0: the GPU performs the depth-specific
    /// division itself (`X >> 2` at 4bpp, `X` at 15bpp), so the renderer hands
    /// it texel coordinates at every depth. This is a field rather than a
    /// constant so a future depth can be added without re-plumbing both
    /// emitters -- it is currently not that.
    u_shift: u8,
    /// Bits per texel of the resident tiles (4, 8 or 15).
    bpp: u8,
}

impl TrackTexture {
    /// Cache slots usable at this depth (`NO1-034`).
    #[inline(always)]
    fn slot_count(&self) -> usize {
        texlayout::slot_count_for_bpp(self.bpp)
    }

    /// `(tpage_idx, u, v)` of a slot at this depth.
    #[inline(always)]
    fn slot_uv(&self, slot: usize) -> (usize, u8, u8) {
        let (_, _, tp, u, v) = match self.bpp {
            4 => texlayout::vram_slot_coords_tim4(slot),
            8 => texlayout::vram_slot_coords_tim8(slot),
            _ => texlayout::vram_slot_coords(slot),
        };
        (tp, u, v)
    }
}

/// Builds a page table from a list of origins at one depth.
const fn build_tpages<const N: usize>(origins: [(u16, u16); N], depth: TexDepth) -> [Tpage; N] {
    let mut out = [Tpage::new(origins[0].0, origins[0].1, depth); N];
    let mut i = 0;
    while i < N {
        out[i] = Tpage::new(origins[i].0, origins[i].1, depth);
        i += 1;
    }
    out
}

/// Tpage set for uncompressed 4bpp tiles.
///
/// The first four page *origins* match [`TPAGES`], but the two depths do not
/// share the slot geometry. A 256x256 page holds 16 64x64 tiles at either depth
/// -- halving the bytes per texel doubles the tiles per page -- but it fills
/// the page differently: at 15bpp a tile is 64 halfwords wide, at 4bpp it is
/// 16. So the 4bpp grid is packed four times as tightly and reads through
/// [`texlayout::vram_slot_coords_tim4`]. Because a 4bpp page is only 64
/// halfwords wide, the cache band holds 16 of them (`NO1-034`).
///
/// The page regions are never live at the same time -- switching codec drops
/// every cached tile -- so sharing origins costs nothing.
static TIM4_TPAGES: [Tpage; TIM4_TPAGE_COUNT] = build_tpages(TIM4_VRAM_PAGES, TexDepth::Bit4);

/// Tpage set for uncompressed 8bpp tiles (8 pages of 128 halfwords).
static TIM8_TPAGES: [Tpage; TIM8_TPAGE_COUNT] = build_tpages(TIM8_VRAM_PAGES, TexDepth::Bit8);

/// Tpage set for direct-colour tiles.
static TPAGES_15: [Tpage; TRACK_TPAGE_COUNT] = TPAGES;

/// The sampling parameters for the codec currently in force on `circuit`.
#[inline]
fn active_texture(circuit: usize) -> TrackTexture {
    if codec_is_tim8(circuit) {
        TrackTexture {
            tpages: &TIM8_TPAGES,
            clut_word: TIM8_CLUT.uv_clut_word(),
            u_shift: 0,
            bpp: 8,
        }
    } else if codec_is_tim4(circuit) {
        TrackTexture {
            tpages: &TIM4_TPAGES,
            clut_word: TIM4_CLUT.uv_clut_word(),
            u_shift: 0,
            bpp: 4,
        }
    } else {
        TrackTexture {
            tpages: &TPAGES_15,
            clut_word: 0,
            u_shift: 0,
            bpp: 15,
        }
    }
}

/// Returns whether the currently active circuit is a night/dark stage (`NO1-033`).
pub fn is_night_circuit() -> bool {
    let circuit = unsafe { ACTIVE_CIRCUIT };
    crate::gpu::palette::sky_palette_for_circuit(circuit).is_night
}

/// Chooses the codec for a circuit, best available first.
///
/// Four formats. Three are chosen by [`codec_for_circuit`]'s preference walk and
/// one is not:
///
/// 1. **SVG** (`ARSV`): the authored vector geometry rasterised at full 1024,
///    with a per-surface detail pass synthesised at decode. Smoothest edges and
///    the only format that adds texture rather than reproducing a stored one.
/// 2. **QOI**: lossless copy of the same raster, no detail pass.
/// 3. **JPEG**: lossy, and the only option for circuits whose art is too
///    detailed to fit the 500 KB block losslessly.
/// 4. **TIM**: uncompressed 15bpp, DMA'd from RAM straight to VRAM with no
///    decode step at all. Deliberately *outside* the walk below.
///
/// Every step down is a pure fallback: the feature flag off, the disc missing
/// the asset, or the circuit's table entry empty all fall through to the next.
/// A disc cooked before this format existed therefore still renders.
///
/// Why TIM is not in the walk: it is the only format here that is not
/// stream-one-block-then-decode. It has no block to stream -- a 1024x1024 15bpp
/// block is 2 MB, twice the machine's RAM -- so it reads four sectors per
/// 64x64 VRAM tile instead. On a track laid out as one block that is 1024 short
/// reads where the compressed formats do one 500 KB read. Each one hands the
/// drive away from CD-DA playback, which is audible. That trade is worth
/// offering and is not worth making on the player's behalf, so `Auto` never
/// chooses it.
#[inline]
fn codec_for_circuit(circuit: usize) -> Codec {
    // Copied out of the static rather than read through a shared reference:
    // the LBAs are written during the one-shot probe in `init_track_texture`,
    // and a shared borrow of a mutable static outlives what it needs to.
    let have_qoi_asset = unsafe { QOITRACKS_BIN_LBA }.is_some();
    let have_svg_asset = unsafe { SVGTRACKS_BIN_LBA }.is_some();
    let have_jpeg_asset = unsafe { TRACKS_BIN_LBA }.is_some();
    let known = circuit < visual_tex::CIRCUIT_COUNT;
    let base_block = if known {
        visual_tex::circuit_block_index(circuit, 0, 0)
    } else {
        0
    };

    // A pinned format is honoured first, but only if this build can actually
    // decode it and the disc carries it. Pinning something unavailable therefore
    // degrades to Auto rather than to a broken screen.
    let pinned = unsafe { TEXTURE_FORMAT };
    if pinned != TextureFormat::Auto {
        let ok = match pinned {
            TextureFormat::Svg => {
                SVG_ENABLED
                    && have_svg_asset
                    && known
                    && visual_tex::CIRCUIT_ARSV_SECTORS[base_block].sector_count > 0
            }
            TextureFormat::Qoi => {
                QOI_ENABLED
                    && have_qoi_asset
                    && known
                    && visual_tex::CIRCUIT_QOI_SECTORS[base_block].sector_count > 0
            }
            // Availability check, which is the right question here: this arm
            // *is* the place that decides whether a TIM pin can be honoured.
            // The decode paths ask `codec_is_tim` instead, because they need to
            // know what was selected, not what exists.
            TextureFormat::Tim => tim_block_for(circuit, 0, 0).is_some(),
            TextureFormat::Tim4 => tim4_block_for(circuit, 0, 0).is_some(),
            TextureFormat::Tim8 => tim8_block_for(circuit, 0, 0).is_some(),
            TextureFormat::Jpeg => {
                have_jpeg_asset
                    && known
                    && visual_tex::CIRCUIT_BLOCK_SECTORS[base_block].sector_count > 0
            }
            TextureFormat::Auto => false,
        };
        if ok {
            return pinned.codec();
        }
    }

    if SVG_ENABLED
        && have_svg_asset
        && known
        && visual_tex::CIRCUIT_ARSV_SECTORS[base_block].sector_count > 0
    {
        Codec::Arsv
    } else if QOI_ENABLED
        && have_qoi_asset
        && known
        && visual_tex::CIRCUIT_QOI_SECTORS[base_block].sector_count > 0
    {
        Codec::Qoi
    } else {
        Codec::Jpeg
    }
}

/// Sets the texture-format preference.
///
/// Takes effect on the next block load; call [`reset_caches`] to make it apply
/// to the circuit currently on screen.
pub fn set_texture_format(format: TextureFormat) {
    unsafe { TEXTURE_FORMAT = format };
}

/// The current texture-format preference, for rendering the menus.
pub fn texture_format() -> TextureFormat {
    unsafe { TEXTURE_FORMAT }
}

/// Returns `(z_far, scan_radius)` for `circuit` based on whether its active codec
/// is uncompressed (TIM, TIM4, TIM8) or compressed (SVG, QOI, JPEG).
///
/// Compressed formats stream their block from RAM and enjoy an extended view distance
/// (+2 visual tiles = +320 world units).
#[inline]
pub fn m7_view_params(circuit: usize) -> (i32, i32) {
    if !codec_is_uncompressed(circuit) {
        (M7_Z_FAR_COMPRESSED, M7_SCAN_RADIUS_COMPRESSED)
    } else {
        (M7_Z_FAR, M7_SCAN_RADIUS)
    }
}

/// Returns the far plane depth `z_far` for `circuit`.
#[inline]
pub fn m7_z_far(circuit: usize) -> i32 {
    if !codec_is_uncompressed(circuit) {
        M7_Z_FAR_COMPRESSED
    } else {
        M7_Z_FAR
    }
}

/// Drops every cached tile so a codec change takes effect immediately.
///
/// The RAM block holds bytes in whichever codec was read, and each VRAM slot
/// holds texels decoded from it. Switching format without dropping both would
/// leave a slot's `block_id` matching while its contents came from the other
/// decoder, which reads as corrupted geometry rather than as a stale tile.
pub fn reset_texture_caches() {
    unsafe {
        // Raw pointers rather than `iter_mut`: taking a mutable reference to a
        // mutable static is UB by the reference rules, and this is a single
        // threaded console.
        core::ptr::write_bytes(
            core::ptr::addr_of_mut!(RAM_BLOCKS) as *mut u8,
            0,
            core::mem::size_of_val(&*core::ptr::addr_of!(RAM_BLOCKS)),
        );
        core::ptr::write_bytes(
            core::ptr::addr_of_mut!(VRAM_SLOTS) as *mut u8,
            0,
            core::mem::size_of_val(&*core::ptr::addr_of!(VRAM_SLOTS)),
        );
        ACTIVE_CIRCUIT = 0;
        FRAME_COUNTER = 0;
        // The TIM4/TIM8 palette is VRAM state that outlives a tile, so dropping the
        // tiles without invalidating it would leave the next circuit's tiles
        // being drawn through the previous circuit's colours. Same for the
        // streaming FIFO: its sectors belong to a circuit that may have changed.
        TIM4_CLUT_CIRCUIT = usize::MAX;
        TIM8_CLUT_CIRCUIT = usize::MAX;
        // Nothing buffered across a format switch any more: a tile in
        // flight dies with the decoder that asked for it.
        crate::async_tile::cancel();
        cancel_async_block_load();
    }
}

/// Prepares the track texture streaming cache for a new circuit or city.
/// Grid dimensions of every shipped level, in `LEVELS.BIN` order.
pub use arduracer_core::levels::LEVEL_DIMS;

/// Loads `track`'s tile grid from disc and binds it.
///
/// Call after `init_track_texture`, which is what locates the asset.
pub fn load_level_grid(circuit: usize, track: &'static arduracer_core::TrackDef) -> bool {
    // 31 circuits occupy 0..30; the cities follow at 31..33.
    if circuit >= LEVEL_DIMS.len() {
        return false;
    }
    let reader = unsafe { &mut *core::ptr::addr_of_mut!(DISC_READER) };
    crate::level_grid::load(
        reader,
        unsafe { LEVELS_BIN_LBA },
        track,
        circuit,
        &LEVEL_DIMS,
    )
}

fn track_def_for(circuit: usize) -> Option<&'static arduracer_core::TrackDef> {
    match circuit {
        0..=39 => arduracer_core::levels::ALL_TRACKS.get(circuit).copied(),
        40 => Some(&arduracer_core::TRACK_CAPETOWN),
        41 => Some(&arduracer_core::TRACK_MELBOURNE),
        42 => Some(&arduracer_core::TRACK_LONDON),
        _ => None,
    }
}

pub fn init_track_texture(circuit: usize) {
    unsafe {
        ACTIVE_CIRCUIT = circuit;
        let vram_slots = &mut *core::ptr::addr_of_mut!(VRAM_SLOTS);
        for slot in vram_slots.iter_mut() {
            *slot = VramSlot::empty();
        }
        let ram_blocks = &mut *core::ptr::addr_of_mut!(RAM_BLOCKS);
        for block in ram_blocks.iter_mut() {
            block.resident_circuit = usize::MAX;
            block.valid = false;
            block.last_used = 0;
        }
        ACTIVE_JOB = DecodeJob::empty();
        crate::async_tile::cancel();
        FRAME_COUNTER = 0;

        if !TRACKS_PROBED {
            let reader = &mut *core::ptr::addr_of_mut!(DISC_READER);
            crate::dbg::println(
                "[TRACKTEX] Probing disc for TRACKS.BIN, QOITRACKS.BIN, SVGTRACKS.BIN, TIMTRACKS.BIN, TIM4TRACKS.BIN, TIM8TRACKS.BIN...",
            );
            TRACKS_BIN_LBA = reader.find_file_lba(visual_tex::TRACKS_BIN_NAME);
            QOITRACKS_BIN_LBA = reader.find_file_lba(visual_tex::QOITRACKS_BIN_NAME);
            SVGTRACKS_BIN_LBA = reader.find_file_lba(visual_tex::SVGTRACKS_BIN_NAME);
            TIMTRACKS_BIN_LBA = reader.find_file_lba(visual_tex::TIMTRACKS_BIN_NAME);
            TIM4TRACKS_BIN_LBA = reader.find_file_lba(visual_tex::TIM4TRACKS_BIN_NAME);
            TIM8TRACKS_BIN_LBA = reader.find_file_lba(visual_tex::TIM8TRACKS_BIN_NAME);
            LEVELS_BIN_LBA = reader.find_file_lba(visual_tex::LEVELS_BIN_NAME);
            crate::dbg::print("[TRACKTEX] TRACKS: 0x");
            if let Some(lba) = TRACKS_BIN_LBA {
                crate::dbg::print_hex(lba);
            } else {
                crate::dbg::print("NONE");
            }
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
            crate::dbg::print("  TIMTRACKS: 0x");
            if let Some(lba) = TIMTRACKS_BIN_LBA {
                crate::dbg::print_hex(lba);
            } else {
                crate::dbg::print("NONE");
            }
            crate::dbg::print("  TIM4TRACKS: 0x");
            if let Some(lba) = TIM4TRACKS_BIN_LBA {
                crate::dbg::print_hex(lba);
            } else {
                crate::dbg::print("NONE");
            }
            crate::dbg::print("  TIM8TRACKS: 0x");
            if let Some(lba) = TIM8TRACKS_BIN_LBA {
                crate::dbg::print_hex(lba);
            } else {
                crate::dbg::print("NONE");
            }
            crate::dbg::println("");
            TRACKS_PROBED = true;
        }

        // Preload initial 1024x1024 block for the circuit into RAM slot 0 before race starts
        let (init_bx, init_by) = if circuit < visual_tex::CIRCUIT_COUNT {
            let (g_bx, g_by) = visual_tex::circuit_block_grid(circuit);
            if g_bx > 1 || g_by > 1 {
                if let Some(trk) = track_def_for(circuit) {
                    let b_wu_x = (trk.world_width() / g_bx as i32).max(1);
                    let b_wu_y = (trk.world_height() / g_by as i32).max(1);
                    let bx = (trk.start_pos.x.to_int() / b_wu_x).clamp(0, g_bx as i32 - 1) as usize;
                    let by = (trk.start_pos.y.to_int() / b_wu_y).clamp(0, g_by as i32 - 1) as usize;
                    (bx, by)
                } else {
                    (0, 0)
                }
            } else {
                (0, 0)
            }
        } else {
            (0, 0)
        };
        crate::dbg::print("[TRACKTEX] circuit ");
        crate::dbg::print_dec(circuit as u32);
        crate::dbg::print(" codec=");
        match codec_for_circuit(circuit) {
            Codec::Arsv => crate::dbg::print("svg"),
            Codec::Qoi => crate::dbg::print("qoi"),
            Codec::Tim => crate::dbg::print("tim"),
            Codec::Tim4 => crate::dbg::print("tim4"),
            Codec::Tim8 => crate::dbg::print("tim8"),
            Codec::Jpeg => crate::dbg::print("jpeg"),
        }
        crate::dbg::println("");

        // TIM/TIM4/TIM8 keep no RAM block, so there is nothing to preload: their tiles come
        // from the disc one at a time, on demand.
        if !codec_is_uncompressed(circuit) {
            load_block_sync(circuit, init_bx, init_by, 0);
        }

        if let Some(codec) = uncompressed_codec_for(circuit) {
            crate::async_tile::cancel();
            if !ensure_uncompressed_palette(codec, circuit) {
                crate::dbg::println("  Uncompressed palette unreadable, falling back");
                set_texture_format(TextureFormat::Auto);
            }
        }
    }
}

/// Decodes one 64x16 MCU row of a resident block into `scratch`, dispatching on
/// the block's codec.
///
/// Both codecs address the same segment of the same tile, so this is the only
/// place the two differ. Anything other than a healthy block leaves `scratch`
/// untouched and returns false, and the caller paints its fallback pattern.
///
/// TIM never reaches here: it has no block to decode rows out of, and its
/// callers branch to [`upload_tile_dma`] before they get this far. The arm
/// returns false rather than decoding something, so that if a future caller
/// forgets the branch the tile comes out as the placeholder pattern instead of
/// as whatever happened to be in the staging buffer.
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
    let data = &ram_slot.block_data.0[..ram_slot.byte_len];

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
        Codec::Tim => false,
        Codec::Tim4 => false,
        Codec::Tim8 => false,
    }
}

/// Ensures the circuit's CLUT palette is loaded into VRAM for indexed formats
/// (`TIM4` and `TIM8`). Direct-color formats (`TIM`, `ARSV`, `QOI`, `JPEG`) return true.
fn ensure_uncompressed_palette(codec: Codec, circuit: usize) -> bool {
    let (block_lba, clut, entries_count, clut_circuit_ptr) = match codec {
        Codec::Tim4 => {
            let Some((lba, _)) = tim4_block_for(circuit, 0, 0) else {
                return false;
            };
            (
                lba,
                TIM4_CLUT,
                visual_tex::TIM4_PALETTE_ENTRIES,
                core::ptr::addr_of_mut!(TIM4_CLUT_CIRCUIT),
            )
        }
        Codec::Tim8 => {
            let Some((lba, _)) = tim8_block_for(circuit, 0, 0) else {
                return false;
            };
            (
                lba,
                TIM8_CLUT,
                visual_tex::TIM8_PALETTE_ENTRIES,
                core::ptr::addr_of_mut!(TIM8_CLUT_CIRCUIT),
            )
        }
        _ => return true,
    };

    if unsafe { *clut_circuit_ptr } == circuit {
        return true;
    }

    let scratch = unsafe { &mut *scratch() };
    let reader = unsafe { &mut *core::ptr::addr_of_mut!(DISC_READER) };
    crate::prof::count(crate::prof::C_CD_SECTORS, 1);
    let claimed = crate::drive::claim(crate::drive::Owner::Texture);
    let read = {
        let _read = crate::prof::span(crate::prof::TRACK_CD_READ);
        reader.read_sectors(block_lba, 1, unsafe {
            core::slice::from_raw_parts_mut(scratch.as_mut_ptr().cast::<u8>(), 2048)
        })
    };
    if claimed {
        crate::drive::release(crate::drive::Owner::Texture);
    }
    if !read {
        return false;
    }

    let entries =
        unsafe { core::slice::from_raw_parts(scratch.as_ptr().cast::<u16>(), entries_count) };
    let rect = VramRect::new(clut.x(), clut.y(), entries_count as u16, 1);
    upload_16bpp(rect, entries);
    unsafe { *clut_circuit_ptr = circuit };
    true
}

/// Computes the VRAM destination rectangle and 32-bit word count for a 64x64 tile
/// across all 6 texture formats.
#[inline]
fn tile_vram_rect_and_words(codec: Codec, target_slot: usize) -> (VramRect, usize) {
    match codec {
        Codec::Tim4 => {
            let (vx, vy, _, _, _) = texlayout::vram_slot_coords_tim4(target_slot);
            (
                VramRect::new(vx, vy, TIM4_TILE_RECT.0, TIM4_TILE_RECT.1),
                visual_tex::TIM4_TILE_HALFWORDS / 2,
            )
        }
        Codec::Tim8 => {
            let (vx, vy, _, _, _) = texlayout::vram_slot_coords_tim8(target_slot);
            (
                VramRect::new(vx, vy, TIM8_TILE_RECT.0, TIM8_TILE_RECT.1),
                visual_tex::TIM8_TILE_HALFWORDS / 2,
            )
        }
        _ => {
            let (vx, vy, _, _, _) = texlayout::vram_slot_coords(target_slot);
            (
                VramRect::new(vx, vy, VRAM_TILE_DIM, VRAM_TILE_DIM),
                TILE_PIXELS / 2,
            )
        }
    }
}

/// Unified VRAM tile uploader for ALL codecs (TIM, TIM4, TIM8, SVG, QOI, JPEG).
///
/// Uses GPU Block DMA (Channel 2) first (~1,924 cycles) and automatically falls
/// back to GP0 FIFO if the DMA channel did not report complete.
#[inline(never)]
fn upload_tile_to_vram(codec: Codec, target_slot: usize, src: *const u32) {
    // Same DMA channel as the floor's packets; let any walk finish first.
    finish_floor_packets();
    let (rect, word_count) = tile_vram_rect_and_words(codec, target_slot);
    let _upload = crate::prof::span(crate::prof::TRACK_VRAM_UPLOAD);
    let via_dma = dma_copy_to_vram(rect, src);
    crate::note_upload_path(via_dma);
    crate::prof::count(
        if via_dma {
            crate::prof::C_UPLOAD_DMA
        } else {
            crate::prof::C_UPLOAD_FIFO
        },
        1,
    );
    if !via_dma {
        upload_words(rect, unsafe {
            core::slice::from_raw_parts(src, word_count)
        });
    }
}

#[inline(always)]
fn mark_vram_slot_resident(
    target_slot: usize,
    circuit: usize,
    bx: usize,
    by: usize,
    sub_tx: usize,
    sub_ty: usize,
    frame: u32,
) {
    let vram_slots = unsafe { &mut *core::ptr::addr_of_mut!(VRAM_SLOTS) };
    vram_slots[target_slot].key = pack_tile_key(circuit, bx, by, sub_tx, sub_ty);
    vram_slots[target_slot].claim = vram_slots[target_slot].key;
    vram_slots[target_slot].resident_circuit = circuit;
    vram_slots[target_slot].resident_block_x = bx;
    vram_slots[target_slot].resident_block_y = by;
    vram_slots[target_slot].resident_tile_x = sub_tx;
    vram_slots[target_slot].resident_tile_y = sub_ty;
    vram_slots[target_slot].last_used = frame;
}

/// Resolves LBA, available remaining sectors, and tile sector count for any uncompressed format.
fn uncompressed_tile_lba(
    codec: Codec,
    circuit: usize,
    block_x: usize,
    block_y: usize,
    tile_x: usize,
    tile_y: usize,
) -> Option<(u32, u32, u32, crate::async_tile::Codec)> {
    if tile_x >= TILES_PER_BLOCK_AXIS || tile_y >= TILES_PER_BLOCK_AXIS {
        return None;
    }
    let (block_lba, block_sectors) = uncompressed_block_for(codec, circuit, block_x, block_y)?;
    let (palette_sectors, tile_sectors, async_codec) = match codec {
        Codec::Tim => (0, TIM_TILE_SECTORS, crate::async_tile::Codec::Tim),
        Codec::Tim4 => (
            visual_tex::TIM4_PALETTE_SECTORS,
            TIM4_TILE_SECTORS,
            crate::async_tile::Codec::Tim4,
        ),
        Codec::Tim8 => (
            visual_tex::TIM8_PALETTE_SECTORS,
            TIM8_TILE_SECTORS,
            crate::async_tile::Codec::Tim8,
        ),
        _ => return None,
    };
    let tile_index = (tile_y * TILES_PER_BLOCK_AXIS + tile_x) as u32;
    let tile_lba = block_lba
        .wrapping_add(palette_sectors)
        .wrapping_add(tile_index * tile_sectors);
    let block_end = block_lba + block_sectors;
    if tile_lba + tile_sectors > block_end {
        return None;
    }
    Some((tile_lba, block_end - tile_lba, tile_sectors, async_codec))
}

/// Outcome of attempting to upload an uncompressed or paletted tile to VRAM.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
enum TileStepResult {
    /// Tile data received and uploaded to VRAM. Slot is now resident.
    Uploaded,
    /// Tile is currently being fetched asynchronously; waiting on sectors.
    Pending,
    /// Permanent error (invalid bounds, missing block, or CD fault).
    Failed,
}

#[inline(never)]
fn upload_uncompressed_tile_async(
    codec: Codec,
    job: &DecodeJob,
    current_frame: u32,
) -> TileStepResult {
    let Some((tile_lba, available, tile_sectors, async_codec)) = uncompressed_tile_lba(
        codec,
        job.circuit,
        job.block_x,
        job.block_y,
        job.tile_x,
        job.tile_y,
    ) else {
        return TileStepResult::Failed;
    };

    let src = {
        let _read = crate::prof::span(crate::prof::TRACK_CD_READ);
        match crate::async_tile::fetch(
            disc_reader(),
            async_codec,
            tile_lba,
            available,
            current_frame,
        ) {
            crate::async_tile::FetchStatus::Ready(p) => p,
            crate::async_tile::FetchStatus::Pending => return TileStepResult::Pending,
            crate::async_tile::FetchStatus::Failed => return TileStepResult::Failed,
        }
    };
    crate::prof::count(crate::prof::C_CD_SECTORS, tile_sectors);

    upload_tile_to_vram(codec, job.target_slot, src);
    mark_vram_slot_resident(
        job.target_slot,
        job.circuit,
        job.block_x,
        job.block_y,
        job.tile_x,
        job.tile_y,
        current_frame,
    );
    TileStepResult::Uploaded
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
fn upload_uncompressed_tile_sync(
    codec: Codec,
    circuit: usize,
    block_x: usize,
    block_y: usize,
    tile_x: usize,
    tile_y: usize,
    target_slot: usize,
) -> bool {
    if !ensure_uncompressed_palette(codec, circuit) {
        return false;
    }
    let Some((tile_lba, _, tile_sectors, _)) =
        uncompressed_tile_lba(codec, circuit, block_x, block_y, tile_x, tile_y)
    else {
        return false;
    };
    let byte_count = tile_sectors as usize * 2048;
    let dst = unsafe { core::slice::from_raw_parts_mut(scratch() as *mut u8, byte_count) };
    if !disc_reader().read_sectors(tile_lba, tile_sectors as usize, dst) {
        return false;
    }
    let src = scratch_words() as *const u32;
    upload_tile_to_vram(codec, target_slot, src);
    mark_vram_slot_resident(target_slot, circuit, block_x, block_y, tile_x, tile_y, 1);
    true
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
    if let Some(codec) = uncompressed_codec_for(circuit) {
        if !upload_uncompressed_tile_sync(codec, circuit, bx, by, sub_tx, sub_ty, target_slot) {
            let slot = unsafe { &mut *core::ptr::addr_of_mut!(VRAM_SLOTS) };
            slot[target_slot] = VramSlot::empty();
        }
        return;
    }

    if !is_ram_block_resident(circuit, bx, by) && !load_block_sync(circuit, bx, by, 0) {
        return;
    }
    let ram_slot = unsafe { &RAM_BLOCKS[0] };
    let scratch = unsafe { &mut *scratch() };

    for row in 0..4 {
        if !decode_row(ram_slot, sub_tx, sub_ty, row, scratch) {
            paint_fallback_row(row * 16, scratch);
        }
    }

    upload_tile_to_vram(ram_slot.codec, target_slot, scratch_words() as *const u32);
    mark_vram_slot_resident(target_slot, circuit, bx, by, sub_tx, sub_ty, 1);
}

/// Pre-warms the starting grid tiles directly into VRAM so the race starts with 100% texture residency.
#[inline(never)]
pub fn prewarm_starting_tiles(world_w: i32, world_h: i32, camera: &Camera) {
    let active_circuit = unsafe { ACTIVE_CIRCUIT };
    let (_, scan_radius) = m7_view_params(active_circuit);
    let (half_w, half_h) = if camera.mode7 {
        (scan_radius, scan_radius)
    } else {
        camera.visible_half_extents()
    };
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
    let (g_bx, g_by) = visual_tex::circuit_block_grid(active_circuit);
    let (block_wu_x, block_wu_y) = (
        (world_w / g_bx as i32).max(1),
        (world_h / g_by as i32).max(1),
    );

    let tile_wu_x = (block_wu_x / (TILES_PER_BLOCK_AXIS as i32)).max(1);
    let tile_wu_y = (block_wu_y / (TILES_PER_BLOCK_AXIS as i32)).max(1);

    let min_tx = (x0 / tile_wu_x).max(0);
    let max_tx = (x1 - 1).max(0) / tile_wu_x;
    let min_ty = (y0 / tile_wu_y).max(0);
    let max_ty = (y1 - 1).max(0) / tile_wu_y;

    let mut candidates = [(0usize, 0usize, 0usize, 0usize, 0u32, 0i32); 128];
    let mut cand_count = 0;
    for ty in min_ty..=max_ty {
        for tx in min_tx..=max_tx {
            let wx0 = tx * tile_wu_x;
            let wy0 = ty * tile_wu_y;
            let depth = if camera.mode7 {
                match m7_tile_visible(camera, wx0, wy0, tile_wu_x, tile_wu_y) {
                    Some(z) => z,
                    None => continue,
                }
            } else {
                0
            };

            let bx = (wx0 / block_wu_x) as usize;
            let by = (wy0 / block_wu_y) as usize;
            let sub_tx = ((wx0 % block_wu_x) / tile_wu_x)
                .clamp(0, (TILES_PER_BLOCK_AXIS - 1) as i32) as usize;
            let sub_ty = ((wy0 % block_wu_y) / tile_wu_y)
                .clamp(0, (TILES_PER_BLOCK_AXIS - 1) as i32) as usize;
            let b_idx = visual_tex::circuit_block_index(active_circuit, bx, by) as u32;
            let tile_idx = b_idx * (TILES_PER_BLOCK_AXIS * TILES_PER_BLOCK_AXIS) as u32
                + (sub_ty * TILES_PER_BLOCK_AXIS + sub_tx) as u32;

            if cand_count < candidates.len() {
                candidates[cand_count] = (bx, by, sub_tx, sub_ty, tile_idx, depth);
                cand_count += 1;
            }
        }
    }

    // In Mode 7, sort candidates by depth ascending so the nearest tiles to the car
    // are chosen first to fill the 64 VRAM cache slots.
    if camera.mode7 {
        for i in 1..cand_count {
            let mut j = i;
            while j > 0 && candidates[j - 1].5 > candidates[j].5 {
                candidates.swap(j - 1, j);
                j -= 1;
            }
        }
    }

    // The grid floor needs each tile at its world-position slot, and the
    // first frame would otherwise drop a cache laid out any other way and
    // stream the whole view again. Where two tiles share a slot the nearer
    // (earlier, after the sort above) keeps it.
    let grid = grid_floor_wanted(camera, active_circuit, (tile_wu_x, tile_wu_y));
    unsafe { GRID_FLOOR = grid };
    let n_slots = active_texture(active_circuit).slot_count();
    let mut taken = [false; MAX_VRAM_SLOTS];
    let mut tiles = [(0usize, 0usize, 0usize, 0usize, 0u32, 0usize); 128];
    let mut count = 0;
    let mut shadowed = [false; 128];
    for (k, c) in candidates[..cand_count].iter().enumerate() {
        if count >= n_slots.min(tiles.len()) {
            break;
        }
        let slot = if grid { grid_slot(c.2, c.3) } else { count };
        if taken[slot] {
            shadowed[k] = true;
            continue;
        }
        taken[slot] = true;
        tiles[count] = (c.0, c.1, c.2, c.3, c.4, slot);
        count += 1;
    }
    // A tile whose slot a nearer one took goes to a free one, where the
    // track pass finds it and draws it over the grid.
    let mut free = 0;
    for (k, c) in candidates[..cand_count].iter().enumerate() {
        if !shadowed[k] || count >= tiles.len() {
            continue;
        }
        while free < n_slots && taken[free] {
            free += 1;
        }
        if free >= n_slots {
            break;
        }
        taken[free] = true;
        tiles[count] = (c.0, c.1, c.2, c.3, c.4, free);
        count += 1;
    }

    // Sort by tile_idx (disc LBA / block index) across ALL formats:
    // - For uncompressed disc streaming: CD laser moves strictly forward without backwards seeks.
    // - For compressed formats: groups decodes by block so each compressed block is loaded into RAM
    //   once rather than repeatedly reloading blocks across boundary transitions!
    for i in 1..count {
        let mut j = i;
        while j > 0 && tiles[j - 1].4 > tiles[j].4 {
            tiles.swap(j - 1, j);
            j -= 1;
        }
    }

    for &(bx, by, sub_tx, sub_ty, _, slot) in tiles[..count].iter() {
        decode_and_upload_full_tile(active_circuit, bx, by, sub_tx, sub_ty, slot);
    }
}

/// Resolves the CD-ROM LBA, sector entry, and codec flags for a compressed block.
fn block_location(
    circuit: usize,
    block_x: usize,
    block_y: usize,
    codec: Codec,
) -> Option<(u32, visual_tex::TileSectorEntry, bool, bool)> {
    let c = visual_tex::circuit_block_index(circuit, block_x, block_y);
    let use_arsv = codec == Codec::Arsv && visual_tex::CIRCUIT_ARSV_SECTORS[c].sector_count > 0;
    let use_qoi = codec == Codec::Qoi && visual_tex::CIRCUIT_QOI_SECTORS[c].sector_count > 0;

    let (bin_lba, entry) = if use_arsv {
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
    };
    let lba = bin_lba?;
    Some((lba + entry.sector_offset, entry, use_arsv, use_qoi))
}

/// Synchronously reads a compressed block into RAM during loading screens.
#[inline(never)]
fn load_block_sync(circuit: usize, block_x: usize, block_y: usize, slot_idx: usize) -> bool {
    let codec = codec_for_circuit(circuit);
    if codec == Codec::Tim || codec == Codec::Tim4 || codec == Codec::Tim8 {
        let ram_slot = unsafe { &mut (*core::ptr::addr_of_mut!(RAM_BLOCKS))[slot_idx] };
        ram_slot.resident_circuit = circuit;
        ram_slot.resident_block_x = block_x;
        ram_slot.resident_block_y = block_y;
        ram_slot.byte_len = 0;
        ram_slot.valid = false;
        return false;
    }

    let Some((block_lba, entry, use_arsv, use_qoi)) =
        block_location(circuit, block_x, block_y, codec)
    else {
        let ram_slot = unsafe { &mut (*core::ptr::addr_of_mut!(RAM_BLOCKS))[slot_idx] };
        ram_slot.resident_circuit = circuit;
        ram_slot.resident_block_x = block_x;
        ram_slot.resident_block_y = block_y;
        ram_slot.byte_len = 0;
        ram_slot.valid = false;
        return false;
    };

    let ram_slot = unsafe { &mut (*core::ptr::addr_of_mut!(RAM_BLOCKS))[slot_idx] };
    let reader = unsafe { &mut *core::ptr::addr_of_mut!(DISC_READER) };

    // Read only the actual compressed payload sectors instead of full 250 sectors
    let sectors_to_read = (entry.byte_len as usize).div_ceil(2048);
    crate::prof::count(crate::prof::C_CD_BLOCKS, 1);
    crate::prof::count(crate::prof::C_CD_SECTORS, sectors_to_read as u32);
    let claimed = crate::drive::claim(crate::drive::Owner::Texture);
    let read_ok = {
        let _read = crate::prof::span(crate::prof::TRACK_CD_READ);
        sectors_to_read <= visual_tex::BLOCK_SECTORS
            && reader.read_sectors(block_lba, sectors_to_read, &mut ram_slot.block_data.0)
    };
    if claimed {
        crate::drive::release(crate::drive::Owner::Texture);
    }

    if read_ok {
        let byte_len = entry.byte_len as usize;
        ram_slot.byte_len = byte_len;
        ram_slot.codec = if use_arsv {
            Codec::Arsv
        } else if use_qoi {
            Codec::Qoi
        } else {
            Codec::Jpeg
        };

        let block = &ram_slot.block_data.0[..byte_len];
        let mut loaded = false;

        let _index = crate::prof::span(crate::prof::TRACK_BLOCK_INDEX);
        if use_arsv {
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
            return true;
        }
    }

    ram_slot.resident_circuit = circuit;
    ram_slot.resident_block_x = block_x;
    ram_slot.resident_block_y = block_y;
    ram_slot.byte_len = 0;
    ram_slot.valid = false;
    false
}

/// Cancels any in-flight asynchronous compressed block streaming job and releases the drive.
pub fn cancel_async_block_load() {
    let job = unsafe { &mut *core::ptr::addr_of_mut!(ASYNC_BLOCK_JOB) };
    if job.active {
        let reader = unsafe { &mut *core::ptr::addr_of_mut!(DISC_READER) };
        reader.stop_stream();
        crate::drive::release(crate::drive::Owner::Texture);
        job.active = false;
    }
}

/// Begins streaming a 1024x1024 compressed block (SVG/ARSV, QOI, JPEG) asynchronously in the background.
fn start_async_block_load(
    circuit: usize,
    block_x: usize,
    block_y: usize,
    slot_idx: usize,
    current_frame: u32,
) -> bool {
    let job = unsafe { &mut *core::ptr::addr_of_mut!(ASYNC_BLOCK_JOB) };
    if job.active {
        return false;
    }
    let codec = codec_for_circuit(circuit);
    if codec == Codec::Tim || codec == Codec::Tim4 || codec == Codec::Tim8 {
        return false;
    }
    let Some((block_lba, entry, use_arsv, use_qoi)) =
        block_location(circuit, block_x, block_y, codec)
    else {
        return false;
    };
    if entry.byte_len == 0 {
        return false;
    }

    if !crate::drive::claim(crate::drive::Owner::Texture) {
        return false;
    }

    let reader = unsafe { &mut *core::ptr::addr_of_mut!(DISC_READER) };
    if !reader.begin_tile_stream(block_lba) {
        crate::drive::release(crate::drive::Owner::Texture);
        return false;
    }

    let sectors_to_read = (entry.byte_len as usize).div_ceil(2048);
    let staging_available = false;

    if !staging_available {
        // Fallback for massive blocks: invalidate RAM_BLOCKS[0] and active job directly
        let active_job = unsafe { &mut *core::ptr::addr_of_mut!(ACTIVE_JOB) };
        if active_job.active
            && (active_job.circuit != circuit
                || active_job.block_x != block_x
                || active_job.block_y != block_y)
        {
            active_job.active = false;
        }

        let ram_slot = unsafe { &mut (*core::ptr::addr_of_mut!(RAM_BLOCKS))[slot_idx] };
        ram_slot.valid = false;
        ram_slot.resident_circuit = circuit;
        ram_slot.resident_block_x = block_x;
        ram_slot.resident_block_y = block_y;
        ram_slot.byte_len = 0;
    }

    crate::prof::count(crate::prof::C_CD_BLOCKS, 1);
    crate::prof::count(crate::prof::C_CD_SECTORS, sectors_to_read as u32);

    *job = AsyncBlockJob {
        active: true,
        circuit,
        block_x,
        block_y,
        slot_idx,
        block_lba,
        sectors_total: sectors_to_read,
        sectors_read: 0,
        byte_len: entry.byte_len as usize,
        codec,
        use_arsv,
        use_qoi,
        start_frame: current_frame,
    };
    true
}

/// Drains arrived sectors from the CD drive for the in-flight compressed block fetch.
fn step_async_block_load(current_frame: u32) {
    let job = unsafe { &mut *core::ptr::addr_of_mut!(ASYNC_BLOCK_JOB) };
    if !job.active {
        return;
    }

    // 5-second stall watchdog
    if current_frame.wrapping_sub(job.start_frame) > 300 {
        let reader = unsafe { &mut *core::ptr::addr_of_mut!(DISC_READER) };
        reader.stop_stream();
        crate::drive::release(crate::drive::Owner::Texture);
        job.active = false;
        return;
    }

    let ram_slot = unsafe { &mut (*core::ptr::addr_of_mut!(RAM_BLOCKS))[job.slot_idx] };
    let reader = unsafe { &mut *core::ptr::addr_of_mut!(DISC_READER) };
    let staging_available = false;

    while job.sectors_read < job.sectors_total {
        let offset = job.sectors_read * 2048;
        let dst = if staging_available {
            unsafe {
                &mut *(((*core::ptr::addr_of_mut!(BLOCK_STAGING)).0.as_mut_ptr()).add(offset)
                    as *mut [u32; 512])
            }
        } else {
            if offset + 2048 > ram_slot.block_data.0.len() {
                break;
            }
            unsafe { &mut *(ram_slot.block_data.0.as_mut_ptr().add(offset) as *mut [u32; 512]) }
        };
        match reader.try_tile_sector(dst) {
            SectorPoll::Filled => {
                job.sectors_read += 1;
            }
            SectorPoll::Empty => {
                return;
            }
            SectorPoll::Fault => {
                reader.stop_stream();
                crate::drive::release(crate::drive::Owner::Texture);
                job.active = false;
                if !staging_available {
                    ram_slot.valid = false;
                }
                return;
            }
        }
    }

    // All payload sectors have arrived: parse headers and index offsets
    if job.sectors_read >= job.sectors_total {
        reader.stop_stream();
        crate::drive::release(crate::drive::Owner::Texture);

        let byte_len = job.byte_len;
        if staging_available {
            let active_job = unsafe { &mut *core::ptr::addr_of_mut!(ACTIVE_JOB) };
            if active_job.active {
                active_job.active = false;
            }
            unsafe {
                core::ptr::copy_nonoverlapping(
                    (*core::ptr::addr_of!(BLOCK_STAGING)).0.as_ptr(),
                    ram_slot.block_data.0.as_mut_ptr(),
                    byte_len,
                );
            }
        }
        ram_slot.byte_len = byte_len;
        ram_slot.codec = if job.use_arsv {
            Codec::Arsv
        } else if job.use_qoi {
            Codec::Qoi
        } else {
            Codec::Jpeg
        };

        let block = &ram_slot.block_data.0[..byte_len];
        let mut loaded = false;

        let _index = crate::prof::span(crate::prof::TRACK_BLOCK_INDEX);
        if job.use_arsv {
            if let Some(svg_block) = arduracer_core::svgtex::parse_block(block) {
                debug_assert_eq!(svg_block.offsets.len(), BLOCK_SPANS);
                ram_slot.arsv_block = svg_block;
                loaded = true;
            }
        } else if job.use_qoi {
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
            ram_slot.resident_circuit = job.circuit;
            ram_slot.resident_block_x = job.block_x;
            ram_slot.resident_block_y = job.block_y;
            ram_slot.valid = true;
            ram_slot.last_used = current_frame;
        } else {
            ram_slot.valid = false;
        }

        job.active = false;
    }
}

/// Continuously drains arriving CD-ROM sectors for background compressed block loads
/// and uncompressed TIM tile fetches. Call during VBlank waits and intra-frame checkpoints
/// so sectors are serviced within <13.33ms to avoid CD controller FIFO overruns.
pub fn pump_cd_streams() {
    let frame = unsafe { FRAME_COUNTER };
    let reader = unsafe { &mut *core::ptr::addr_of_mut!(DISC_READER) };
    crate::async_tile::pump(reader, frame);
    step_async_block_load(frame);
}

/// Returns whether `(circuit, block_x, block_y)` is currently valid and resident in RAM.
#[inline]
fn is_ram_block_resident(circuit: usize, block_x: usize, block_y: usize) -> bool {
    let blocks = unsafe { &*core::ptr::addr_of!(RAM_BLOCKS) };
    for b in blocks.iter().take(RAM_CACHE_BLOCKS) {
        if b.valid
            && b.resident_circuit == circuit
            && b.resident_block_x == block_x
            && b.resident_block_y == block_y
        {
            return true;
        }
    }
    false
}

/// Ensures the required 1024x1024 block is resident in RAM.
///
/// Returns `Some(slot_idx)` if the block is already resident and valid, or
/// `None` if it is still streaming in the background across frames.
#[inline(never)]
fn ensure_ram_block(
    circuit: usize,
    block_x: usize,
    block_y: usize,
    current_frame: u32,
) -> Option<usize> {
    let blocks = unsafe { &mut *core::ptr::addr_of_mut!(RAM_BLOCKS) };
    for (i, b) in blocks.iter_mut().enumerate().take(RAM_CACHE_BLOCKS) {
        if b.valid
            && b.resident_circuit == circuit
            && b.resident_block_x == block_x
            && b.resident_block_y == block_y
        {
            b.last_used = current_frame;
            return Some(i);
        }
    }

    let job = unsafe { &*core::ptr::addr_of!(ASYNC_BLOCK_JOB) };
    if job.active {
        // If an async block load is already active:
        // Do NOT cancel it to chase a different block; let it finish streaming!
        return None;
    }

    start_async_block_load(circuit, block_x, block_y, 0, current_frame);
    None
}

/// Advances the active time-sliced decode job by 1 MCU row (64x16 pixels).
///
/// Time-slicing keeps each frame's decode cost bounded regardless of codec:
/// the JPEG path spends ~20% of the frame budget on a row, QOI noticeably less.
/// When row 3 finishes, the full 64x64 tile is uploaded to VRAM and marked resident.
///
/// TIM does not use the row machinery at all. Its whole tile arrives in one
/// transfer, so there is nothing to slice: the job completes in a single step,
/// and the cost of that step is the CD read the other formats amortise across
/// 256 tiles. The per-frame ceiling is unchanged, which keeps a stalled drive
/// from turning into a frame overrun.
/// Outcome of advancing the active tile decode job.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
enum StepResult {
    /// Full 64x64 tile uploaded to VRAM and marked resident.
    Done,
    /// One MCU row decoded (for incremental formats JPEG/QOI/ARSV).
    RowAdvanced,
    /// Asynchronous CD read in flight (TIM/TIM4/TIM8); keep job active and slot reserved.
    Pending,
    /// Permanent error or stall; slot emptied and job cleared.
    Failed,
}

#[inline(never)]
fn step_decode_job(current_frame: u32) -> StepResult {
    let job = unsafe { &mut *core::ptr::addr_of_mut!(ACTIVE_JOB) };
    if !job.active {
        return StepResult::Failed;
    }

    if let Some(codec) = uncompressed_codec_for(job.circuit) {
        crate::prof::count(crate::prof::C_DECODE_ROWS, 1);
        match upload_uncompressed_tile_async(codec, job, current_frame) {
            TileStepResult::Uploaded => {
                crate::prof::count(crate::prof::C_TILE_DECODES, 1);
                job.active = false;
                return StepResult::Done;
            }
            TileStepResult::Pending => {
                return StepResult::Pending;
            }
            TileStepResult::Failed => {
                let slots = unsafe { &mut *core::ptr::addr_of_mut!(VRAM_SLOTS) };
                slots[job.target_slot] = VramSlot::empty();
                job.active = false;
                return StepResult::Failed;
            }
        }
    }

    let Some(ram_slot_idx) = ensure_ram_block(job.circuit, job.block_x, job.block_y, current_frame)
    else {
        return StepResult::Pending;
    };
    let ram_slot = unsafe { &RAM_BLOCKS[ram_slot_idx] };
    let scratch = unsafe { &mut *scratch() };

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
        // All 4 MCU rows (64x64 pixels) complete! Upload to VRAM via DMA.
        crate::prof::count(crate::prof::C_TILE_DECODES, 1);
        upload_tile_to_vram(
            ram_slot.codec,
            job.target_slot,
            scratch_words() as *const u32,
        );
        mark_vram_slot_resident(
            job.target_slot,
            job.circuit,
            job.block_x,
            job.block_y,
            job.tile_x,
            job.tile_y,
            current_frame,
        );

        job.active = false;
        StepResult::Done
    } else {
        StepResult::RowAdvanced
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

/// Scanlines one decode row takes, the recent worst case: a row started when
/// the check passes still has to finish, and a QOI row is ~45 scanlines
/// (~70k instructions), not a rounding error.
static mut DECODE_ROW_LINES: u32 = 48;
/// What the end-of-frame decode works from: this frame's candidate tiles, as
/// the track pass left them. See [`decode_in_spare_time`].
struct SpareDecode {
    valid: bool,
    current_frame: u32,
    active_circuit: usize,
    last_flip_vblank: u32,
    rows_left: usize,
    visible: [TileRef; VISIBLE_TILES_MAX],
    visible_depths: [i32; VISIBLE_TILES_MAX],
    visible_count: usize,
    prefetch: [TileRef; PREFETCH_MAX],
    prefetch_count: usize,
    camera: Option<Camera>,
    tile_wu: (i32, i32),
    slot_for_tile: [usize; VISIBLE_TILES_MAX],
    used_slots: [bool; MAX_VRAM_SLOTS],
}

static mut SPARE_DECODE: SpareDecode = SpareDecode {
    valid: false,
    current_frame: 0,
    active_circuit: 0,
    last_flip_vblank: 0,
    rows_left: 0,
    visible: [(0, 0, 0, 0, 0, 0); VISIBLE_TILES_MAX],
    visible_depths: [0; VISIBLE_TILES_MAX],
    visible_count: 0,
    prefetch: [(0, 0, 0, 0, 0, 0); PREFETCH_MAX],
    prefetch_count: 0,
    camera: None,
    tile_wu: (0, 0),
    slot_for_tile: [usize::MAX; VISIBLE_TILES_MAX],
    used_slots: [false; MAX_VRAM_SLOTS],
};

/// Track passes drawn this frame; split-screen draws two.
static mut TRACK_PASSES: u32 = 0;

/// The VBlank count and the scanlines since that VBlank, read as one
/// consistent pair. Root counter 1 free-runs on HBlank; the VBlank handler
/// records it at every edge (`psx_rt::interrupts::vblank_timer1`).
fn display_time() -> (u32, u32) {
    loop {
        let vblank = psx_rt::interrupts::vblank_count();
        let at_vblank = psx_rt::interrupts::vblank_timer1();
        let raw = psx_io::timers::counter(psx_io::timers::Timer::Timer1) as u32;
        if psx_rt::interrupts::vblank_count() == vblank {
            return (
                vblank,
                arduracer_core::texture_format::lines_since_vblank(raw, at_vblank),
            );
        }
    }
}

/// Scanlines left before the deadline of the frame that began at
/// `last_flip_vblank`; zero once it has passed.
pub fn frame_slack_lines(last_flip_vblank: u32) -> u32 {
    let (vblank, line) = display_time();
    arduracer_core::texture_format::frame_lines_left(
        vblank.wrapping_sub(last_flip_vblank),
        frame_period(),
        line,
    )
}

/// Folds the scanlines between two `display_time` readings into a decaying
/// worst case (`next_decode_reserve`).
fn next_row_lines(previous: u32, from: (u32, u32), to: (u32, u32)) -> u32 {
    use arduracer_core::texture_format::{next_decode_reserve, LINES_PER_VBLANK};
    let lines = (to.0.wrapping_sub(from.0) * LINES_PER_VBLANK + to.1).saturating_sub(from.1);
    if lines > LINES_PER_VBLANK {
        return previous;
    }
    next_decode_reserve(previous, lines)
}

/// Decodes texture rows in the time left before the frame's deadline, once
/// everything else in the frame is drawn. Called once a frame, before the
/// pacing wait.
///
/// The track pass decodes only the row a missing visible tile forces. It used
/// to decode further rows mid-frame, against a measured reserve for the floor,
/// props, cars and HUD still to come, but in a race the floor after decoding
/// ranged from 130k to 300k instructions, so the guess came up short exactly
/// on the heavy frames. Here the time left is known, so a row only starts when
/// one fits ([`DECODE_ROW_LINES`], the recent worst case).
pub fn decode_in_spare_time() {
    // SAFETY: single-threaded; never touched from an interrupt.
    let spare = unsafe { &mut *core::ptr::addr_of_mut!(SPARE_DECODE) };
    unsafe { TRACK_PASSES = 0 };
    if !spare.valid {
        return;
    }
    spare.valid = false;
    let Some(camera) = spare.camera else {
        return;
    };
    if unsafe { ACTIVE_CIRCUIT } != spare.active_circuit {
        return;
    }
    let vram_slots = unsafe { &mut *core::ptr::addr_of_mut!(VRAM_SLOTS) };
    let period = frame_period();
    let mut row_start = display_time();
    let mut rows = 0;
    while rows < spare.rows_left {
        let (vblank, line) = display_time();
        if rows > 0 {
            unsafe {
                DECODE_ROW_LINES = next_row_lines(DECODE_ROW_LINES, row_start, (vblank, line))
            };
        }
        row_start = (vblank, line);
        let lines_left = arduracer_core::texture_format::frame_lines_left(
            vblank.wrapping_sub(spare.last_flip_vblank),
            period,
            line,
        );
        if !arduracer_core::texture_format::decode_row_allowed(
            lines_left,
            unsafe { DECODE_ROW_LINES } + SPARE_MARGIN_LINES,
            0,
            0,
        ) {
            break;
        }
        pump_cd_streams();
        let did_work = step_or_schedule_decode_row(
            spare.current_frame,
            spare.active_circuit,
            &TileCandidates {
                visible: &spare.visible,
                visible_depths: &spare.visible_depths,
                visible_count: spare.visible_count,
                prefetch: &spare.prefetch,
                prefetch_count: spare.prefetch_count,
                camera: &camera,
                tile_wu: spare.tile_wu,
            },
            &mut spare.slot_for_tile,
            &mut spare.used_slots,
            vram_slots,
        );
        if !did_work {
            break;
        }
        rows += 1;
    }
    if rows > 0 {
        let end = display_time();
        unsafe { DECODE_ROW_LINES = next_row_lines(DECODE_ROW_LINES, row_start, end) };
    }
}

/// Scanlines kept clear of the deadline after the last spare-time row, for
/// the frame close and the flip.
const SPARE_MARGIN_LINES: u32 = 8;

/// Tile reference tuple: `(tile_x, tile_y, block_x, block_y, sub_tx, sub_ty)`.
/// Shared by the visible list and the prefetch list so one scheduler can read
/// either.
type TileRef = (i32, i32, usize, usize, usize, usize);

/// Hard cap on prefetch candidates held on the stack.
///
/// The lookahead is one tile row above and one below the visible rectangle, so
/// the real count is `2 * tiles-per-row` (a 320-unit-wide window over 160-unit
/// tiles is ~3). The cap is a hard bound so a wide window (zoomed out, or a
/// small-tile circuit) degrades to "prefetch fewer" instead of growing the
/// stack without limit. `visible_tiles` uses the same shape at `VISIBLE_TILES_MAX`.
const PREFETCH_MAX: usize = 32;

/// Whether `tile` currently has its pixels resident in a VRAM slot.
#[inline]
fn tile_resident(vram_slots: &[VramSlot], circuit: usize, tile: &TileRef) -> bool {
    let (_, _, bx, by, sub_tx, sub_ty) = *tile;
    let target_key = pack_tile_key(circuit, bx, by, sub_tx, sub_ty);
    vram_slots.iter().any(|slot| slot.key == target_key)
}

/// Priority key for a prefetch tile, across all 6 texture codecs (visible
/// tiles use `visible_decode_priority`, by exact nearest depth):
///   - Needs RAM block (0 = already resident in RAM or uncompressed, 1 = requires async block switch)
///   - In Mode 7: depth tier (0 = near [z < 150], 1 = mid [150 <= z < 220], 2 = far [z >= 220])
///   - Sub-tile index in disc / span order (sub_ty * 16 + sub_tx)
#[inline(always)]
fn tile_priority_key(
    candidates: &TileCandidates<'_>,
    active_circuit: usize,
    uncompressed: bool,
    tile: &TileRef,
) -> u32 {
    let (tx, ty, _, _, _, _) = *tile;
    let needs_ram_block = ram_block_state(active_circuit, uncompressed, tile);

    let depth_tier = if candidates.camera.mode7 {
        let cam_qx = candidates.camera.pos.x.raw() >> 8;
        let cam_qy = candidates.camera.pos.y.raw() >> 8;
        let wx0 = tx * candidates.tile_wu.0;
        let wy0 = ty * candidates.tile_wu.1;
        let (_, z_q4) = candidates.camera.view_from_offset_q4(
            (2 * wx0 + candidates.tile_wu.0) * 8 - cam_qx,
            (2 * wy0 + candidates.tile_wu.1) * 8 - cam_qy,
        );
        let z = z_q4 >> 4;
        if z < 140 {
            0u32
        } else if z < 210 {
            1u32
        } else {
            2u32
        }
    } else {
        0u32
    };

    (needs_ram_block << 28)
        | (depth_tier << 20)
        | (tile_disc_order(active_circuit, tile) & 0x000F_FFFF)
}

/// 0 when `tile`'s compressed block is in RAM (or the codec has none), 1 while
/// it is streaming in, 2 when it is not loaded.
#[inline(always)]
fn ram_block_state(active_circuit: usize, uncompressed: bool, tile: &TileRef) -> u32 {
    let (_, _, bx, by, _, _) = *tile;
    if uncompressed || is_ram_block_resident(active_circuit, bx, by) {
        return 0;
    }
    let job = unsafe { &*core::ptr::addr_of!(ASYNC_BLOCK_JOB) };
    if job.active && job.circuit == active_circuit && job.block_x == bx && job.block_y == by {
        1
    } else {
        2
    }
}

/// `tile`'s position in disc / span order across the circuit's blocks.
#[inline(always)]
fn tile_disc_order(active_circuit: usize, tile: &TileRef) -> u32 {
    let (_, _, bx, by, sub_tx, sub_ty) = *tile;
    let b_idx = visual_tex::circuit_block_index(active_circuit, bx, by) as u32;
    b_idx * (TILES_PER_BLOCK_AXIS * TILES_PER_BLOCK_AXIS) as u32
        + (sub_ty * TILES_PER_BLOCK_AXIS + sub_tx) as u32
}

/// The tile lists one frame's decode scheduler draws candidates from.
///
/// Bundled rather than passed as four loose parameters so the ordering rule
/// (visible before prefetch) is stated once, in one place.
struct TileCandidates<'a> {
    /// Tiles inside the screen rectangle. Phase 3 draws these.
    visible: &'a [TileRef],
    /// Each visible tile's nearest view depth (Q4); 0 outside Mode 7.
    visible_depths: &'a [i32],
    visible_count: usize,
    /// One tile row above and below the visible rectangle. Decode-only.
    prefetch: &'a [TileRef],
    prefetch_count: usize,
    camera: &'a Camera,
    tile_wu: (i32, i32),
}

/// Helper that steps the active MCU row decode job or schedules the next tile
/// that needs decoding.
///
/// Candidates are considered visible-first, prefetch-second, so a tile that is on
/// screen always outranks one that is a row away: the lookahead can never trade
/// a placeholder for a prefetch. Returns true if 1 MCU row was decoded, or false
/// if every visible tile is resident and nothing is left to prefetch.
#[inline(always)]
fn step_or_schedule_decode_row(
    current_frame: u32,
    active_circuit: usize,
    candidates: &TileCandidates<'_>,
    slot_for_tile: &mut [usize; VISIBLE_TILES_MAX],
    used_slots: &mut [bool; MAX_VRAM_SLOTS],
    vram_slots: &mut [VramSlot; MAX_VRAM_SLOTS],
) -> bool {
    let vis_count = candidates.visible_count;
    let job = unsafe { &mut *core::ptr::addr_of_mut!(ACTIVE_JOB) };
    if !job.active {
        // No active decode job running: pick the next tile that needs one.
        // We evaluate candidate tiles using a unified priority key:
        //   - For uncompressed disc streaming: depth tier (near/mid/far) ensures the
        //     road in front of the car loads first, with ascending disc LBA within each tier.
        //   - For compressed codecs (SVG, QOI, JPEG): tiles from an ALREADY resident RAM block
        //     are prioritized over ones needing an async block switch, and within the block,
        //     Mode 7 depth tiers ensure foreground road decodes before distant horizon tiles.
        let uncompressed = codec_is_uncompressed(active_circuit);

        // 1. Check visible tiles first (highest priority)
        let mut best_vis: Option<(usize, usize, usize, usize, u32)> = None;
        for (i, &slot) in slot_for_tile.iter().enumerate().take(vis_count) {
            if slot == usize::MAX || slot == SLOT_CONFLICT {
                let tile = candidates.visible[i];
                let key = arduracer_core::texture_format::visible_decode_priority(
                    ram_block_state(active_circuit, uncompressed, &tile),
                    candidates.visible_depths[i] >> 4,
                    tile_disc_order(active_circuit, &tile),
                );
                if best_vis.is_none() || key < best_vis.unwrap().4 {
                    let (_, _, bx, by, sub_tx, sub_ty) = tile;
                    best_vis = Some((bx, by, sub_tx, sub_ty, key));
                }
            }
        }

        let candidate = if let Some((bx, by, sub_tx, sub_ty, _)) = best_vis {
            Some((bx, by, sub_tx, sub_ty))
        } else {
            // 2. All visible tiles are resident: consider prefetch lookahead
            let mut best_prefetch: Option<(usize, usize, usize, usize, u32)> = None;
            let grid = unsafe { GRID_FLOOR };
            for tile in &candidates.prefetch[..candidates.prefetch_count] {
                let (_, _, pbx, pby, psx, psy) = *tile;
                // On the grid a tile can only go to its own slot; skip it
                // while a visible tile holds that.
                if grid {
                    let s = grid_slot(psx, psy);
                    if used_slots[s]
                        && vram_slots[s].claim != pack_tile_key(active_circuit, pbx, pby, psx, psy)
                    {
                        continue;
                    }
                }
                if !tile_resident(vram_slots, active_circuit, tile) {
                    let key = tile_priority_key(candidates, active_circuit, uncompressed, tile);
                    if best_prefetch.is_none() || key < best_prefetch.unwrap().4 {
                        let (_, _, bx, by, sub_tx, sub_ty) = *tile;
                        best_prefetch = Some((bx, by, sub_tx, sub_ty, key));
                    }
                }
            }
            best_prefetch.map(|(bx, by, sub_tx, sub_ty, _)| (bx, by, sub_tx, sub_ty))
        };
        let Some((bx, by, sub_tx, sub_ty)) = candidate else {
            return false;
        };

        if !uncompressed && !is_ram_block_resident(active_circuit, bx, by) {
            return false;
        }

        // Find oldest VRAM slot not currently visible in this frame. `used_slots`
        // holds both visible residents and in-flight decode targets, so this can
        // only evict a tile nothing on screen needs.
        let mut best_slot = usize::MAX;
        let mut oldest_age = u32::MAX;
        let cam_tx = if candidates.tile_wu.0 > 0 {
            candidates.camera.pos.x.to_int() / candidates.tile_wu.0
        } else {
            0
        };
        let cam_ty = if candidates.tile_wu.1 > 0 {
            candidates.camera.pos.y.to_int() / candidates.tile_wu.1
        } else {
            0
        };

        let n_slots = active_texture(active_circuit).slot_count();
        for s_idx in 0..n_slots {
            if !used_slots[s_idx] {
                let slot = &vram_slots[s_idx];
                if slot.resident_circuit != active_circuit {
                    best_slot = s_idx;
                    break;
                }
                // In Mode 7, protect tiles in the immediate neighborhood of the vehicle
                // so steering back or camera oscillations don't force immediate re-streams.
                let effective_age = if candidates.camera.mode7 {
                    let slot_world_tx = slot.resident_block_x as i32
                        * (TILES_PER_BLOCK_AXIS as i32)
                        + slot.resident_tile_x as i32;
                    let slot_world_ty = slot.resident_block_y as i32
                        * (TILES_PER_BLOCK_AXIS as i32)
                        + slot.resident_tile_y as i32;
                    let d_tx = (slot_world_tx - cam_tx).abs();
                    let d_ty = (slot_world_ty - cam_ty).abs();
                    let tile_dist = d_tx.max(d_ty);
                    if tile_dist <= 2 {
                        slot.last_used.saturating_add(500)
                    } else if tile_dist >= 4 {
                        slot.last_used.saturating_sub(500)
                    } else {
                        slot.last_used
                    }
                } else {
                    slot.last_used
                };

                if effective_age < oldest_age {
                    oldest_age = effective_age;
                    best_slot = s_idx;
                }
            }
        }

        // On the grid a tile goes to its own slot, unless a nearer visible
        // tile holds that; then to the spare slot chosen above.
        let key = pack_tile_key(active_circuit, bx, by, sub_tx, sub_ty);
        let own = grid_slot(sub_tx, sub_ty);
        let grid_own = unsafe { GRID_FLOOR } && (!used_slots[own] || vram_slots[own].claim == key);
        if grid_own {
            best_slot = own;
        }
        if best_slot == usize::MAX {
            return false;
        }

        // Invalidate the slot immediately so Phase 1 in this or subsequent frames
        // will not hit the old tile key and draw stale/wrong textures! On the
        // grid the slot is also painted this tile's placeholder colour, since
        // the grid shows whatever a slot holds.
        if grid_own {
            if vram_slots[best_slot].claim != key {
                claim_slot(vram_slots, best_slot, key, ((sub_tx ^ sub_ty) & 1) != 0);
            }
        } else {
            vram_slots[best_slot] = VramSlot::empty();
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
    } else {
        used_slots[job.target_slot] = true;
    }

    let job_bx = job.block_x;
    let job_by = job.block_y;
    let job_tx = job.tile_x;
    let job_ty = job.tile_y;
    let job_slot = job.target_slot;

    let res = step_decode_job(current_frame);

    // If step_decode_job completed and the slot is resident, update slot_for_tile
    // so Phase 3 renders it immediately textured without placeholder!
    match res {
        StepResult::Done => {
            if vram_slots[job_slot].resident_circuit == active_circuit {
                for (i, tile) in candidates.visible.iter().take(vis_count).enumerate() {
                    let (_, _, bx, by, sub_tx, sub_ty) = *tile;
                    if bx == job_bx && by == job_by && sub_tx == job_tx && sub_ty == job_ty {
                        slot_for_tile[i] = job_slot;
                        used_slots[job_slot] = true;
                    }
                }
            }
            true
        }
        StepResult::RowAdvanced => true,
        StepResult::Pending => false,
        StepResult::Failed => false,
    }
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
    let active_circuit = unsafe { ACTIVE_CIRCUIT };
    let (z_far, scan_radius) = m7_view_params(active_circuit);
    let (half_w, half_h) = if camera.mode7 {
        (scan_radius, scan_radius)
    } else {
        camera.visible_half_extents()
    };
    let cam_x = camera.pos.x.to_int();
    let cam_y = camera.pos.y.to_int();

    let x0 = (cam_x - half_w).max(0);
    let x1 = (cam_x + half_w).min(world_w);
    let y0 = (cam_y - half_h).max(0);
    let y1 = (cam_y + half_h).min(world_h);

    // The sky first, as DMA packets: it is the biggest fill of the frame,
    // and drawn immediately the CPU waited for each primitive (~85k cycles a
    // frame of GP0 stalls). Queued here, the GPU paints it while the tile scan
    // and decode below run on the CPU. It also stands in for the frame clear,
    // which Mode 7 skips, so it is drawn even when there is no track.
    if camera.mode7 {
        draw_m7_backdrop(camera);
    }
    if x0 >= x1 || y0 >= y1 {
        finish_floor_packets();
        return;
    }

    unsafe {
        FRAME_COUNTER = FRAME_COUNTER.wrapping_add(1);
    }
    let current_frame = unsafe { FRAME_COUNTER };
    // Advance any tile or block fetch already in flight before deciding what this frame
    // wants. The drive pushes on its own clock, so this cannot be left to the
    // tile request below: a frame that asks for nothing would leave the sectors
    // sitting in the drive's two-slot FIFO and overrun it.
    pump_cd_streams();
    // Sampling parameters for whichever codec this circuit selected: tpage
    // depth, CLUT handle, and how many texels a U unit spans. Bound once here
    // so both the flat and Mode 7 emitters read the same thing.
    let tex = active_texture(active_circuit);
    let vram_slots = unsafe { &mut *core::ptr::addr_of_mut!(VRAM_SLOTS) };

    // Compute exact world units per 64x64 tile along X and Y axes.
    let (g_bx, g_by) = visual_tex::circuit_block_grid(active_circuit);
    let (block_wu_x, block_wu_y) = (
        (world_w / g_bx as i32).max(1),
        (world_h / g_by as i32).max(1),
    );

    let tile_wu_x = (block_wu_x / (TILES_PER_BLOCK_AXIS as i32)).max(1);
    let tile_wu_y = (block_wu_y / (TILES_PER_BLOCK_AXIS as i32)).max(1);

    let min_tx = (x0 / tile_wu_x).max(0);
    let max_tx = (x1 - 1).max(0) / tile_wu_x;
    let min_ty = (y0 / tile_wu_y).max(0);
    let max_ty = (y1 - 1).max(0) / tile_wu_y;

    // Maximum 64 visible tiles per frame (bounded to VRAM cache capacity of 64 slots)
    let mut visible_tiles: [TileRef; VISIBLE_TILES_MAX] = [(0, 0, 0, 0, 0, 0); VISIBLE_TILES_MAX];
    let mut vis_depths = [0i32; VISIBLE_TILES_MAX];
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

                if tx0 < tx1 && ty0 < ty1 {
                    let depth = if camera.mode7 {
                        match m7_tile_visible(camera, wx0, wy0, tile_wu_x, tile_wu_y) {
                            Some(z) => z,
                            None => continue,
                        }
                    } else {
                        0
                    };

                    let bx = (wx0 / block_wu_x) as usize;
                    let by = (wy0 / block_wu_y) as usize;
                    let sub_tx = ((wx0 % block_wu_x) / tile_wu_x)
                        .clamp(0, (TILES_PER_BLOCK_AXIS - 1) as i32)
                        as usize;
                    let sub_ty = ((wy0 % block_wu_y) / tile_wu_y)
                        .clamp(0, (TILES_PER_BLOCK_AXIS - 1) as i32)
                        as usize;

                    let tile_ref = (tx, ty, bx, by, sub_tx, sub_ty);
                    if vis_count < VISIBLE_TILES_MAX {
                        visible_tiles[vis_count] = tile_ref;
                        vis_depths[vis_count] = depth;
                        vis_count += 1;
                    } else if camera.mode7 {
                        // Keep the 64 nearest tiles to the camera: replace the furthest entry if this tile is closer
                        let mut max_idx = 0;
                        let mut max_z = vis_depths[0];
                        for k in 1..VISIBLE_TILES_MAX {
                            if vis_depths[k] > max_z {
                                max_z = vis_depths[k];
                                max_idx = k;
                            }
                        }
                        if depth < max_z {
                            visible_tiles[max_idx] = tile_ref;
                            vis_depths[max_idx] = depth;
                        }
                    }
                }
            }
        }
    }

    // Lookahead above and below the visible rectangle.
    //
    // A tile's decode is time-sliced across 4 MCU rows, so a tile first seen at
    // the screen edge spends several frames as a placeholder. Starting its
    // decode a row early means it is usually resident by the time the camera
    // reaches it.
    //
    // These are candidates only -- Phase 3 still draws `visible_tiles`, so a
    // prefetched tile costs a decode row and a slot, never a polygon.
    let mut prefetch_tiles: [TileRef; PREFETCH_MAX] = [(0, 0, 0, 0, 0, 0); PREFETCH_MAX];
    let mut prefetch_count = 0usize;
    {
        let world_max_tx = (world_w - 1).max(0) / tile_wu_x;
        let world_max_ty = (world_h - 1).max(0) / tile_wu_y;

        if camera.mode7 {
            // Mode 7 forward heading lookahead:
            // The camera faces along heading vector (sin(h), -cos(h)).
            // Scan tile distances beyond the far plane along the viewing cone,
            // covering forward travel and slight turns.
            let fwd_sin = camera.hsin() as i64;
            let fwd_cos = camera.hcos() as i64;
            let right_cos = fwd_cos;
            let right_sin = fwd_sin;

            // Distances beyond far plane: M7_Z0 = 120
            let fwd_far = z_far - M7_Z0;
            let fwd_dists = [fwd_far + 20, fwd_far + 70, fwd_far + 120];
            let lat_offsets = [-tile_wu_x, 0, tile_wu_x];

            'm7_prefetch: for &fwd_dist in &fwd_dists {
                for &lat_offset in &lat_offsets {
                    if prefetch_count >= PREFETCH_MAX {
                        break 'm7_prefetch;
                    }
                    let dx = ((fwd_dist as i64 * fwd_sin + lat_offset as i64 * right_cos)
                        >> FP_SHIFT) as i32;
                    let dy = ((-fwd_dist as i64 * fwd_cos + lat_offset as i64 * right_sin)
                        >> FP_SHIFT) as i32;
                    let px = cam_x + dx;
                    let py = cam_y + dy;
                    if px < 0 || py < 0 || px >= world_w || py >= world_h {
                        continue;
                    }
                    let tx = px / tile_wu_x;
                    let ty = py / tile_wu_y;
                    if tx < 0 || tx > world_max_tx || ty < 0 || ty > world_max_ty {
                        continue;
                    }
                    if visible_tiles[..vis_count]
                        .iter()
                        .any(|t| t.0 == tx && t.1 == ty)
                    {
                        continue;
                    }
                    if prefetch_tiles[..prefetch_count]
                        .iter()
                        .any(|t| t.0 == tx && t.1 == ty)
                    {
                        continue;
                    }
                    let wx0 = tx * tile_wu_x;
                    let wy0 = ty * tile_wu_y;
                    let bx = (wx0 / block_wu_x) as usize;
                    let by = (wy0 / block_wu_y) as usize;
                    let sub_tx = ((wx0 % block_wu_x) / tile_wu_x)
                        .clamp(0, (TILES_PER_BLOCK_AXIS - 1) as i32)
                        as usize;
                    let sub_ty = ((wy0 % block_wu_y) / tile_wu_y)
                        .clamp(0, (TILES_PER_BLOCK_AXIS - 1) as i32)
                        as usize;
                    let tile_ref = (tx, ty, bx, by, sub_tx, sub_ty);
                    if tile_resident(vram_slots, active_circuit, &tile_ref) {
                        continue;
                    }
                    prefetch_tiles[prefetch_count] = tile_ref;
                    prefetch_count += 1;
                }
            }
        } else {
            let lookahead_tiles = if !codec_is_uncompressed(active_circuit) {
                2
            } else {
                1
            };
            // Vertical lookahead: rows immediately above and below the frustum
            for step in 1..=lookahead_tiles {
                for ty in [min_ty.saturating_sub(step), max_ty + step] {
                    if !(0..=world_max_ty).contains(&ty) || (min_ty..=max_ty).contains(&ty) {
                        continue;
                    }
                    for tx in min_tx..=max_tx {
                        if prefetch_count >= PREFETCH_MAX {
                            break;
                        }
                        let wx0 = tx * tile_wu_x;
                        let wy0 = ty * tile_wu_y;
                        let bx = (wx0 / block_wu_x) as usize;
                        let by = (wy0 / block_wu_y) as usize;
                        let sub_tx = ((wx0 % block_wu_x) / tile_wu_x)
                            .clamp(0, (TILES_PER_BLOCK_AXIS - 1) as i32)
                            as usize;
                        let sub_ty = ((wy0 % block_wu_y) / tile_wu_y)
                            .clamp(0, (TILES_PER_BLOCK_AXIS - 1) as i32)
                            as usize;
                        prefetch_tiles[prefetch_count] = (tx, ty, bx, by, sub_tx, sub_ty);
                        prefetch_count += 1;
                    }
                }
            }

            // Horizontal lookahead: columns immediately left and right of the frustum
            for step in 1..=lookahead_tiles {
                for tx in [min_tx.saturating_sub(step), max_tx + step] {
                    if !(0..=world_max_tx).contains(&tx) || (min_tx..=max_tx).contains(&tx) {
                        continue;
                    }
                    for ty in min_ty..=max_ty {
                        if prefetch_count >= PREFETCH_MAX {
                            break;
                        }
                        let wx0 = tx * tile_wu_x;
                        let wy0 = ty * tile_wu_y;
                        let bx = (wx0 / block_wu_x) as usize;
                        let by = (wy0 / block_wu_y) as usize;
                        let sub_tx = ((wx0 % block_wu_x) / tile_wu_x)
                            .clamp(0, (TILES_PER_BLOCK_AXIS - 1) as i32)
                            as usize;
                        let sub_ty = ((wy0 % block_wu_y) / tile_wu_y)
                            .clamp(0, (TILES_PER_BLOCK_AXIS - 1) as i32)
                            as usize;
                        prefetch_tiles[prefetch_count] = (tx, ty, bx, by, sub_tx, sub_ty);
                        prefetch_count += 1;
                    }
                }
            }
        }

        // For uncompressed streaming, sort prefetch candidates in ascending disc LBA order
        if codec_is_uncompressed(active_circuit) {
            for i in 1..prefetch_count {
                let mut j = i;
                while j > 0 {
                    let prev_b = visual_tex::circuit_block_index(
                        active_circuit,
                        prefetch_tiles[j - 1].2,
                        prefetch_tiles[j - 1].3,
                    );
                    let curr_b = visual_tex::circuit_block_index(
                        active_circuit,
                        prefetch_tiles[j].2,
                        prefetch_tiles[j].3,
                    );
                    let prev_idx = prev_b * (TILES_PER_BLOCK_AXIS * TILES_PER_BLOCK_AXIS)
                        + prefetch_tiles[j - 1].5 * TILES_PER_BLOCK_AXIS
                        + prefetch_tiles[j - 1].4;
                    let curr_idx = curr_b * (TILES_PER_BLOCK_AXIS * TILES_PER_BLOCK_AXIS)
                        + prefetch_tiles[j].5 * TILES_PER_BLOCK_AXIS
                        + prefetch_tiles[j].4;
                    if prev_idx > curr_idx {
                        prefetch_tiles.swap(j - 1, j);
                        j -= 1;
                    } else {
                        break;
                    }
                }
            }
        }
    }

    let mut slot_for_tile = [usize::MAX; VISIBLE_TILES_MAX];
    let mut used_slots = [false; MAX_VRAM_SLOTS];

    // The grid floor needs every tile at its world-position slot; switching to
    // or from it drops the cache, as a codec switch does.
    let grid = grid_floor_wanted(camera, active_circuit, (tile_wu_x, tile_wu_y));
    if grid != unsafe { GRID_FLOOR } {
        for slot in vram_slots.iter_mut() {
            *slot = VramSlot::empty();
        }
        unsafe {
            (*core::ptr::addr_of_mut!(ACTIVE_JOB)).active = false;
            GRID_FLOOR = grid;
        }
    }

    // Visible tiles drawn over the grid floor from a slot other than their
    // own (bit per visible index); see `SLOT_CONFLICT`.
    let mut overlay = 0u64;
    const _: () = assert!(VISIBLE_TILES_MAX <= 64);

    // Phase 1: Match already-resident 64x64 tiles in VRAM LRU cache
    if grid {
        // Nearest first, so where two visible tiles share a slot the nearer
        // one keeps it.
        let _probe = crate::prof::span(crate::prof::TRACK_CACHE_PROBE);
        let mut order = [0u8; VISIBLE_TILES_MAX];
        for i in 0..vis_count {
            let mut j = i;
            while j > 0 && vis_depths[order[j - 1] as usize] > vis_depths[i] {
                order[j] = order[j - 1];
                j -= 1;
            }
            order[j] = i as u8;
        }
        for &o in order.iter().take(vis_count) {
            let i = o as usize;
            let (tx, ty, bx, by, sub_tx, sub_ty) = visible_tiles[i];
            let key = pack_tile_key(active_circuit, bx, by, sub_tx, sub_ty);
            let s = grid_slot(sub_tx, sub_ty);
            if used_slots[s] {
                // Its own slot is taken: it may be decoded in a spare one
                // (see the scheduler), and is then drawn over the grid.
                slot_for_tile[i] = SLOT_CONFLICT;
                for (k, slot) in vram_slots.iter().enumerate().take(TRACK_SLOT_COUNT) {
                    if slot.key == key && !used_slots[k] {
                        slot_for_tile[i] = k;
                        overlay |= 1u64 << i;
                        used_slots[k] = true;
                        break;
                    }
                }
                continue;
            }
            used_slots[s] = true;
            if vram_slots[s].key == key {
                slot_for_tile[i] = s;
                vram_slots[s].last_used = current_frame;
                crate::prof::count(crate::prof::C_TILE_HITS, 1);
            } else if vram_slots[s].claim != key {
                claim_slot(vram_slots, s, key, ((tx ^ ty) & 1) != 0);
            }
        }
    } else {
        let _probe = crate::prof::span(crate::prof::TRACK_CACHE_PROBE);
        let n_slots = tex.slot_count();
        for i in 0..vis_count {
            let (_, _, bx, by, sub_tx, sub_ty) = visible_tiles[i];
            let target_key = pack_tile_key(active_circuit, bx, by, sub_tx, sub_ty);
            let mut found = None;

            for (s_idx, slot) in vram_slots.iter().enumerate().take(n_slots) {
                if slot.key == target_key {
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

    // Reserve any in-flight decode target slot so Phase 2 won't evict it
    let job = unsafe { &*core::ptr::addr_of!(ACTIVE_JOB) };
    if job.active {
        used_slots[job.target_slot] = true;
    }

    // Phase 1.5: Pump asynchronous block streaming for compressed codecs (ARSV/SVG, QOI, JPEG).
    // Polls arriving disc sectors into RAM_BLOCKS[0] and finalizes block indexing when complete.
    pump_cd_streams();

    // Lookahead block streaming: if no async block load is active and all visible tiles from
    // the currently resident block are already in VRAM, check if upcoming visible or prefetch
    // tiles need a different block. If so, start streaming it in the background now!
    if !codec_is_uncompressed(active_circuit) {
        let async_job = unsafe { &*core::ptr::addr_of!(ASYNC_BLOCK_JOB) };
        if !async_job.active {
            let car_bx = ((cam_x / block_wu_x) as usize).min(g_bx.saturating_sub(1));
            let car_by = ((cam_y / block_wu_y) as usize).min(g_by.saturating_sub(1));
            let mut needed_block = if !is_ram_block_resident(active_circuit, car_bx, car_by) {
                Some((car_bx, car_by))
            } else {
                None
            };

            if needed_block.is_none() {
                let mut closest_z = i32::MAX;
                for i in 0..vis_count {
                    if slot_for_tile[i] == usize::MAX {
                        let (_, _, bx, by, _, _) = visible_tiles[i];
                        if !is_ram_block_resident(active_circuit, bx, by) {
                            let z = vis_depths[i];
                            if z < closest_z {
                                closest_z = z;
                                needed_block = Some((bx, by));
                            }
                        }
                    }
                }
            }
            if needed_block.is_none() {
                for i in 0..prefetch_count {
                    let tile = &prefetch_tiles[i];
                    if !tile_resident(vram_slots, active_circuit, tile) {
                        let (_, _, bx, by, _, _) = *tile;
                        if !is_ram_block_resident(active_circuit, bx, by) {
                            needed_block = Some((bx, by));
                            break;
                        }
                    }
                }
            }
            if let Some((bx, by)) = needed_block {
                start_async_block_load(active_circuit, bx, by, 0, current_frame);
            }
        }
    }

    // Phase 2: Progress the active time-sliced decode job. Only the row a
    // missing visible tile forces is decoded here, before the floor; the rest
    // of the budget waits for the end of the frame (`decode_in_spare_time`),
    // where the time left is known rather than guessed.
    let is_multiblock = visual_tex::circuit_block_grid(active_circuit).0 > 1;
    let active_codec = codec_for_circuit(active_circuit);
    let rows_per_budget =
        arduracer_core::texture_format::rows_per_budget_for_codec(active_codec, is_multiblock);
    let min_rows = {
        let job = unsafe { &*core::ptr::addr_of!(ACTIVE_JOB) };
        if job.active || (0..vis_count).any(|i| slot_for_tile[i] == usize::MAX) {
            1
        } else {
            0
        }
    };
    let max_rows = (max_decode_rows * rows_per_budget).max(min_rows);
    let mut rows_decoded = 0;
    let row_start = display_time();
    while rows_decoded < min_rows {
        pump_cd_streams();
        let did_work = step_or_schedule_decode_row(
            current_frame,
            active_circuit,
            &TileCandidates {
                visible: &visible_tiles,
                visible_depths: &vis_depths,
                visible_count: vis_count,
                prefetch: &prefetch_tiles,
                prefetch_count,
                camera,
                tile_wu: (tile_wu_x, tile_wu_y),
            },
            &mut slot_for_tile,
            &mut used_slots,
            vram_slots,
        );
        if !did_work {
            break;
        }
        rows_decoded += 1;
    }
    // SAFETY: single-threaded; never touched from an interrupt.
    unsafe {
        if rows_decoded > 0 {
            DECODE_ROW_LINES = next_row_lines(DECODE_ROW_LINES, row_start, display_time());
        }
        // Split-screen draws two passes a frame; the spare time goes to each
        // viewport's candidates on alternate frames.
        let pass = TRACK_PASSES;
        TRACK_PASSES += 1;
        if pass == 0 || current_frame & 1 == 1 {
            let spare = &mut *core::ptr::addr_of_mut!(SPARE_DECODE);
            spare.valid = true;
            spare.current_frame = current_frame;
            spare.active_circuit = active_circuit;
            spare.last_flip_vblank = last_flip_vblank;
            spare.rows_left = max_rows - rows_decoded;
            spare.visible = visible_tiles;
            spare.visible_depths = vis_depths;
            spare.visible_count = vis_count;
            spare.prefetch = prefetch_tiles;
            spare.prefetch_count = prefetch_count;
            spare.camera = Some(*camera);
            spare.tile_wu = (tile_wu_x, tile_wu_y);
            spare.slot_for_tile = slot_for_tile;
            spare.used_slots = used_slots;
        }
    }

    // In Mode 7, sort visible tiles by depth descending (far to near) so painter's algorithm
    // renders background and distant horizon tiles first, and near foreground tiles over them!
    if camera.mode7 && vis_count > 1 {
        for i in 1..vis_count {
            let mut j = i;
            while j > 0 && vis_depths[j - 1] < vis_depths[j] {
                vis_depths.swap(j - 1, j);
                visible_tiles.swap(j - 1, j);
                slot_for_tile.swap(j - 1, j);
                j -= 1;
            }
        }
    }

    // Phase 3: Render visible quads (textured for resident tiles, placeholder flat for pending)
    let _quads = crate::prof::span(crate::prof::TRACK_QUADS);
    if camera.mode7 {
        // The ground mesh projects through the GTE; load this camera once for
        // every tile below. Nothing between here and the tile loop touches the
        // GTE, and the car and prop passes reload it for themselves.
        crate::gpu::gte_proj::load_m7_camera_gte(camera);
    }
    // One mesh workspace and one set of per-frame floor constants for every
    // tile this frame; see `MeshScratch` and `FloorFrame`.
    let mut mesh_scratch = m7_mesh::MeshScratch::new();
    let floor_frame = FloorFrame::new(camera);
    if grid {
        let pending = (0..vis_count)
            .filter(|&i| slot_for_tile[i] >= SLOT_CONFLICT)
            .count();
        crate::prof::count(crate::prof::C_PLACEHOLDERS, pending as u32);
        draw_grid_floor(
            camera,
            &floor_frame,
            (tile_wu_x, tile_wu_y),
            (world_w, world_h),
        );
        // A tile whose slot a nearer one holds would show that tile's
        // texture through the grid, so it is drawn over it with the tile
        // mesh: textured from a spare slot once decoded, flat until then.
        for i in 0..vis_count {
            let s_idx = slot_for_tile[i];
            let decoded_elsewhere = overlay & (1u64 << i) != 0;
            if s_idx != SLOT_CONFLICT && !decoded_elsewhere {
                continue;
            }
            let s_idx = if decoded_elsewhere { s_idx } else { usize::MAX };
            let (tx, ty, _, _, _, _) = visible_tiles[i];
            // SAFETY: see `FloorStack`.
            unsafe {
                FloorStack::run(|| {
                    draw_tile_m7(
                        camera,
                        (tx, ty),
                        (tile_wu_x, tile_wu_y),
                        (world_w, world_h),
                        s_idx,
                        &floor_frame,
                        &mut mesh_scratch,
                    )
                })
            };
        }
        finish_floor_packets();
        return;
    }
    for i in 0..vis_count {
        if (i & 3) == 0 {
            pump_cd_streams();
        }
        let (tx, ty, _, _, _, _) = visible_tiles[i];
        let s_idx = slot_for_tile[i];

        if s_idx == usize::MAX {
            crate::prof::count(crate::prof::C_PLACEHOLDERS, 1);
        }

        if camera.mode7 {
            // SAFETY: see `FloorStack`. Nothing else keeps data in the
            // scratchpad, and `make stack-guard` proves the tree fits.
            unsafe {
                FloorStack::run(|| {
                    draw_tile_m7(
                        camera,
                        (tx, ty),
                        (tile_wu_x, tile_wu_y),
                        (world_w, world_h),
                        s_idx,
                        &floor_frame,
                        &mut mesh_scratch,
                    )
                })
            };
            continue;
        }

        let wx0 = tx * tile_wu_x;
        let wy0 = ty * tile_wu_y;
        let (tx0, tx1, ty0, ty1) = if camera.rotating {
            // When the viewport rotates, clamping to axis-aligned camera bounding boxes
            // distorts the rotated quad texture. Use full unclipped tile boundaries.
            (wx0, wx0 + tile_wu_x, wy0, wy0 + tile_wu_y)
        } else {
            (
                x0.max(wx0),
                x1.min(wx0 + tile_wu_x),
                y0.max(wy0),
                y1.min(wy0 + tile_wu_y),
            )
        };

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
            let (tpage_idx, u_base, v_base) = tex.slot_uv(s_idx);

            // Map fractional position inside the tile to 0..64 UV texels
            let local_u0 = (((tx0 - wx0) * (VRAM_TILE_DIM as i32)) / tile_wu_x) as u16;
            let local_u1 = (((tx1 - wx0) * (VRAM_TILE_DIM as i32)) / tile_wu_x) as u16;
            let local_v0 = (((ty0 - wy0) * (VRAM_TILE_DIM as i32)) / tile_wu_y) as u16;
            let local_v1 = (((ty1 - wy0) * (VRAM_TILE_DIM as i32)) / tile_wu_y) as u16;

            let u0 = (((u_base as u32 + local_u0 as u32) >> tex.u_shift).min(255)) as u8;
            let u1 = (((u_base as u32 + local_u1 as u32) >> tex.u_shift).min(255)) as u8;
            let v0 = ((v_base as u32 + local_v0 as u32).min(255)) as u8;
            let v1 = ((v_base as u32 + local_v1 as u32).min(255)) as u8;

            let tpage = tex.tpages[tpage_idx];
            let tint = if is_night_circuit() {
                (0x58, 0x58, 0x5E)
            } else {
                (0x80, 0x80, 0x80)
            };
            let material = TextureMaterial::opaque(tex.clut_word, tpage.uv_tpage_word(0), tint);
            gpu::draw_quad_textured_material(
                [tl, tr, bl, br],
                [(u0, v0), (u1, v0), (u0, v1), (u1, v1)],
                material,
            );
        }
    }
    // The Mode 7 floor went out as DMA packets; everything drawn after this
    // writes the command port directly, so the walk must be done first.
    finish_floor_packets();
}

// ---------------------------------------------------------------------------
// Mode 7 chase view
// ---------------------------------------------------------------------------

/// True if any part of the tile can fall inside the Mode 7 frustum, returning its projected depth.
///
/// A conservative circle-vs-trapezoid test on the tile centre: the tile's
/// bounding radius pads both the depth range and the lateral edge test
/// (screen half-width is `1.33 * z`, tested as `5 * lateral <= 7 * z`).
#[inline(always)]
fn m7_tile_visible(camera: &Camera, wx0: i32, wy0: i32, tw: i32, th: i32) -> Option<i32> {
    let cam_qx = camera.pos.x.raw() >> 8;
    let cam_qy = camera.pos.y.raw() >> 8;
    let (_, z0) = camera.view_from_offset_q4(wx0 * 16 - cam_qx, wy0 * 16 - cam_qy);
    let (_, z1) = camera.view_from_offset_q4((wx0 + tw) * 16 - cam_qx, wy0 * 16 - cam_qy);
    let (_, z2) = camera.view_from_offset_q4(wx0 * 16 - cam_qx, (wy0 + th) * 16 - cam_qy);
    let (_, z3) = camera.view_from_offset_q4((wx0 + tw) * 16 - cam_qx, (wy0 + th) * 16 - cam_qy);
    let zmin = z0.min(z1).min(z2).min(z3);
    let zmax = z0.max(z1).max(z2).max(z3);
    if zmax < (M7_Z_NEAR << 4) || zmin > (camera.z_far << 4) {
        return None;
    }
    let (right, z) =
        camera.view_from_offset_q4((2 * wx0 + tw) * 8 - cam_qx, (2 * wy0 + th) * 8 - cam_qy);
    let rad = tw.max(th) * 12;
    if (right.abs() - rad) * 5 <= (z + rad).max(0) * 7 {
        Some(zmin)
    } else {
        None
    }
}

/// Rows the parallax skyline is raised above the Mode 7 grid's first row.
///
/// The road is painted over the backdrop from `track_top` downward, so a
/// skyline anchored exactly at `track_top` loses its entire foot to the
/// carriageway and reads as peaks sitting on bare ground. Lifting the base puts
/// the ridge line in the sky, where a horizon belongs.
pub const M7_SILHOUETTE_LIFT: i16 = 5;

/// Renders multi-stop Gouraud sky gradients, scrolling parallax silhouettes, and ground backdrop (`NO1-032`).
fn draw_m7_backdrop(camera: &Camera) {
    let mut out = floor_packets().at;
    let sky = crate::gpu::palette::sky_palette_for_circuit(unsafe { ACTIVE_CIRCUIT });
    let track_top = camera.m7_track_top().clamp(28, 140);
    let roll = (camera.roll_bam / 8).clamp(-12, 12);
    let y_l = track_top - roll;
    let y_r = track_top + roll;

    // Segment 1: Zenith to Mid-sky (0 .. track_top * 3 / 5)
    let mid_y = (track_top * 3) / 5;
    tri_gouraud(
        &mut out,
        [(0, 0), (320, 0), (0, mid_y)],
        [sky.zenith, sky.zenith, sky.mid_sky],
    );
    tri_gouraud(
        &mut out,
        [(320, 0), (320, mid_y), (0, mid_y)],
        [sky.zenith, sky.mid_sky, sky.mid_sky],
    );

    // Segment 2: Mid-sky to Track Top (mid_y .. y_l/y_r)
    tri_gouraud(
        &mut out,
        [(0, mid_y), (320, mid_y), (0, y_l)],
        [sky.mid_sky, sky.mid_sky, sky.horizon],
    );
    tri_gouraud(
        &mut out,
        [(320, mid_y), (320, y_r), (0, y_l)],
        [sky.mid_sky, sky.horizon, sky.horizon],
    );

    let v_h = camera.viewport_h;
    // Ground backdrop below track top (painted over by Mode 7 track tiles)
    tri_gouraud(
        &mut out,
        [(0, y_l), (320, y_r), (0, v_h)],
        [sky.ground_haze, sky.ground_haze, sky.ground_deep],
    );
    tri_gouraud(
        &mut out,
        [(320, y_r), (320, v_h), (0, v_h)],
        [sky.ground_haze, sky.ground_deep, sky.ground_deep],
    );

    // Parallax Horizon Silhouette: shifted by camera heading, anchored directly at track top (zero gap)
    let heading_shift = ((camera.heading as i32 * 320) >> 12) as i16;
    let col_dark = sky.silhouette_dark;
    let col_lit = sky.silhouette_lit;

    // Biome-specific silhouette heights
    let sil_heights: [i16; 8] = match sky.biome_style {
        1 => [16, 28, 12, 34, 22, 18, 30, 14], // Neon City skyscrapers
        2 => [18, 14, 24, 20, 16, 22, 12, 26], // Canyon mesas
        3 => [22, 32, 16, 28, 36, 18, 24, 30], // Alpine jagged mountain peaks
        4 => [14, 20, 26, 16, 32, 18, 22, 28], // Obsidian cyber structures
        5 => [12, 18, 14, 22, 16, 20, 14, 24], // Stadium grandstand canopy
        _ => [10, 16, 12, 20, 14, 18, 12, 16], // Speedway coastal hills
    };

    for (i, &h) in sil_heights.iter().enumerate() {
        let x0 = (i as i16 * 44 - heading_shift).rem_euclid(352) - 16;
        let x1 = x0 + 44;
        let mid_x = x0 + 22;
        // The skyline base sits `M7_SILHOUETTE_LIFT` rows *above* the grid's
        // first row. Anchoring it exactly at `track_top` buried the foot of
        // every mountain under the Mode 7 road: the backdrop is drawn first and
        // the track tiles paint over it from `track_top` down, so the ridge
        // line was swallowed and the peaks floated on bare ground. The skirt
        // still runs to the viewport bottom, so nothing below the ridge is left
        // unpainted -- the ridge line itself is simply visible now.
        let roll_term = (roll as i32 * (mid_x as i32 - 160) / 160) as i16;
        let base_y = (track_top - M7_SILHOUETTE_LIFT + roll_term).max(0);
        let peak_y = base_y - h;
        // The skirt only has to reach the floor's first row, wherever roll
        // puts it under this mountain; the floor paints everything below. It
        // used to run to the bottom of the screen: eight 44-pixel columns of
        // fill the floor then covered, and the CPU waited for the GPU to draw
        // each one (~60k cycles a frame of GP0 stalls).
        let skirt_bottom = (track_top + roll.abs() + 2).min(v_h);
        let skirt_h = (skirt_bottom - base_y).max(0) as u16;
        if sky.biome_style == 1 || sky.biome_style == 4 {
            // Solid base skirt down into ground so track tiles seamlessly overlap the skyline
            rect_flat(
                &mut out, x0, base_y, 44, skirt_h, col_dark.0, col_dark.1, col_dark.2,
            );
            // City skyscraper / cyber tower block
            let rx0 = x0 + 6;
            let rw = 32;
            let tower_h = (skirt_bottom - peak_y).max(0) as u16;
            rect_flat(
                &mut out, rx0, peak_y, rw, tower_h, col_dark.0, col_dark.1, col_dark.2,
            );
            rect_flat(
                &mut out,
                rx0 + 2,
                peak_y,
                2,
                tower_h,
                col_lit.0,
                col_lit.1,
                col_lit.2,
            );
        } else {
            // Mountain / mesa / stadium triangular ridge
            tri_flat(
                &mut out,
                [(x0, base_y), (mid_x, peak_y), (x1, base_y)],
                col_dark.0,
                col_dark.1,
                col_dark.2,
            );
            // Solid base skirt down into ground so track tiles seamlessly overlap the mountains
            rect_flat(
                &mut out, x0, base_y, 44, skirt_h, col_dark.0, col_dark.1, col_dark.2,
            );
            line_mono(
                &mut out, mid_x, peak_y, x1, base_y, col_lit.0, col_lit.1, col_lit.2,
            );
        }
    }
    // Start the GPU on it now, while the CPU scans and decodes tiles.
    let packets = floor_packets();
    packets.at = out;
    packets.kick();
}

/// The backdrop's primitives, as packets in the floor's buffer; each matches
/// the `psx_gpu` immediate-mode call of the same name.
fn tri_gouraud(out: &mut PacketCursor, v: [(i16, i16); 3], c: [(u8, u8, u8); 3]) {
    let w = out.node(6);
    let op = gp0::polygon_opcode(true, false, false, false, false);
    for k in 0..3 {
        w[2 * k] = pack_color(c[k].0, c[k].1, c[k].2);
        w[2 * k + 1] = pack_vertex(v[k].0, v[k].1);
    }
    w[0] |= op;
}

fn tri_flat(out: &mut PacketCursor, v: [(i16, i16); 3], r: u8, g: u8, b: u8) {
    let w = out.node(4);
    w[0] = gp0::polygon_opcode(false, false, false, false, false) | pack_color(r, g, b);
    for k in 0..3 {
        w[1 + k] = pack_vertex(v[k].0, v[k].1);
    }
}

#[allow(clippy::too_many_arguments)]
fn rect_flat(out: &mut PacketCursor, x: i16, y: i16, w: u16, h: u16, r: u8, g: u8, b: u8) {
    let (x1, y1) = (x + w as i16, y + h as i16);
    let p = out.node(5);
    p[0] = gp0::polygon_opcode(false, true, false, false, false) | pack_color(r, g, b);
    p[1] = pack_vertex(x, y);
    p[2] = pack_vertex(x1, y);
    p[3] = pack_vertex(x, y1);
    p[4] = pack_vertex(x1, y1);
}

#[allow(clippy::too_many_arguments)]
fn line_mono(out: &mut PacketCursor, x0: i16, y0: i16, x1: i16, y1: i16, r: u8, g: u8, b: u8) {
    let w = out.node(3);
    w[0] = 0x4000_0000 | pack_color(r, g, b);
    w[1] = pack_vertex(x0, y0);
    w[2] = pack_vertex(x1, y1);
}

/// Largest ground offset (Q4) handed to the GTE, inside its `i16` input.
const M7_GTE_REACH_Q4: i32 = 32_000;

/// Screen rectangle the ground mesh culls against.
const M7_VIEW: (i16, i16, i16, i16) = (0, 0, 320, 240);

/// Projects ground offsets through the GTE, three at a time (`RTPT`).
///
/// Requires [`gte_proj::load_m7_camera_gte`] for the camera being drawn. The
/// GTE does the heading rotation and the perspective divide that the software
/// path spent two R3000 `DIV`s per vertex on, which is what pays for the finer
/// near-camera grid, and it leaves each point's depth in `SZ`, exactly the
/// value `Camera::view_from_offset_q4` computes. `SZ` saturates at zero, so a
/// point nearer than the near plane gets its true (possibly negative) depth
/// recomputed in software; its screen position is never used, because the mesh
/// clips every polygon to `M7_Z_NEAR` first.
struct GteGround {
    hsin: i32,
    hcos: i32,
}

impl GteGround {
    fn new(camera: &Camera) -> Self {
        Self {
            hsin: camera.hsin(),
            hcos: camera.hcos(),
        }
    }

    #[inline(always)]
    fn finish(&self, p: &mut m7_mesh::MeshVert, r: psx_gte::scene::Projected) {
        p.sx = r.sx;
        p.sy = r.sy;
        let sz = r.sz as i32;
        p.z = if sz >= M7_Z_NEAR << 4 {
            sz
        } else {
            ((p.dx * self.hsin - p.dy * self.hcos) >> 12) + (M7_Z0 << 4)
        };
    }
}

impl m7_mesh::Projector for GteGround {
    /// One point at a time (`RTPS`). `RTPT` does three for less than three
    /// times the GTE time, but measured here it saved almost no instructions
    /// -- the per-point work is the depth fix-up and outcode, not the GTE --
    /// while its unrolled loop cost ~7k cycles a frame in instruction-cache
    /// refills: on this path the cache is the scarcer resource.
    fn project(&mut self, pts: &mut [m7_mesh::MeshVert], code: &mut [u8], oc: &m7_mesh::Outcoder) {
        use psx_gte::{math::Vec3I16, scene};
        for (p, c) in pts.iter_mut().zip(code.iter_mut()) {
            // No clamp: `draw_tile_m7` never meshes a tile with a corner outside
            // the `i16` range (see `M7_GTE_REACH_Q4`).
            let r = scene::project_vertex_scheduled(Vec3I16::new(p.dx as i16, 0, p.dy as i16));
            self.finish(p, r);
            *c = oc.code(p);
        }
    }
}

/// Distance fog for the ground: full brightness out to 150 units, fading to
/// ~55% at the far plane, darker again at night.
///
/// The fade divides by the same range for every polygon, so it is a
/// reciprocal taken once per tile and one multiply per polygon -- a per-polygon
/// `DIV` was a measurable share of the floor on the R3000.
#[derive(Copy, Clone)]
struct GroundFog {
    /// `58 / (far - 150)` in Q16.
    fade_q16: i32,
    night: bool,
}

impl GroundFog {
    fn new(z_far: i32, night: bool) -> Self {
        Self {
            fade_q16: (58 << 16) / (z_far - 150).max(1),
            night,
        }
    }

    /// Modulation for a polygon whose depth runs `za..zb` (Q4).
    #[inline(always)]
    fn shade(self, za: i32, zb: i32) -> u8 {
        let z_units = (za + zb) >> 5;
        let base = if z_units <= 150 {
            128
        } else {
            (128 - (((z_units - 150) * self.fade_q16) >> 16)).max(70)
        };
        if self.night {
            ((base * 86) >> 7) as u8
        } else {
            base as u8
        }
    }
}

/// Words per half of the floor's packet buffer.
///
/// Sized to hold the whole sky backdrop (~190 words; see
/// [`draw_m7_backdrop`]) in one half, so it goes to the GPU in one kick that
/// never waits: split across two, the second kick would wait for the GPU to
/// fill the first. Double-buffering means it never needs to hold a whole
/// frame of floor, which is ~25 quads per half.
const FLOOR_CHUNK_WORDS: usize = 256;

/// The Mode 7 floor's GPU packets, double-buffered for linked-list DMA.
///
/// Why not write GP0 directly, as every other draw does: each primitive then
/// costs a ready-wait on the GPU plus a port write per word, and with the CPU
/// stalled for every one of them the finer floor mesh spent over a thousand
/// cycles per polygon just handing them over. Here the CPU writes packets to
/// RAM, and each half is given to DMA channel 2 as soon as it fills, so the
/// GPU draws one half while the CPU builds the next. The walk waits for the
/// previous one before it starts, so a half is never rewritten while the GPU
/// is still reading it.
///
/// Every primitive is its own node, which keeps each under the GPU FIFO's
/// 16-word node limit ([`gpu::MAX_NODE_WORDS`]).
struct FloorPackets {
    buf: [[u32; FLOOR_CHUNK_WORDS]; 2],
    /// Where the next node goes. While a tile draws, its [`GroundSink`] holds
    /// the live copy; see [`PacketCursor`].
    at: PacketCursor,
    /// Whether a DMA walk may still be running.
    in_flight: bool,
    /// Never written by the GPU or linked; see [`PacketCursor::node`].
    spare: [u32; NODE_MAX_WORDS],
}

/// The write position in [`FloorPackets`].
///
/// A tile takes a copy for itself and puts it back when it is done, so every
/// primitive it writes reads the position from its own frame -- the scratchpad
/// -- instead of from the static in uncached RAM. With the writer an
/// out-of-line call that reloaded this state, opening a node cost ~90 cycles.
#[derive(Copy, Clone)]
struct PacketCursor {
    /// Half being filled.
    cur: usize,
    /// Words used in it.
    len: usize,
    /// One past the index of the last node's tag in the current half, to link
    /// the next; 0 when the half is empty. Offset by one so the whole static
    /// starts zeroed and lives in `.bss`, not `.data`.
    prev: usize,
    /// That tag's word count, already shifted into place, so linking the next
    /// node is a store rather than a read-modify-write of uncached RAM.
    prev_size: u32,
}

/// Payload words of the largest floor node, a textured quad.
const NODE_MAX_WORDS: usize = 9;
const _: () = assert!(NODE_MAX_WORDS <= gpu::MAX_NODE_WORDS);

static mut FLOOR_PACKETS: FloorPackets = FloorPackets {
    buf: [[0; FLOOR_CHUNK_WORDS]; 2],
    at: PacketCursor {
        cur: 0,
        len: 0,
        prev: 0,
        prev_size: 0,
    },
    in_flight: false,
    spare: [0; NODE_MAX_WORDS],
};

/// Tag terminating a linked-list chain.
const LIST_END: u32 = 0x00FF_FFFF;

impl PacketCursor {
    /// Opens a node of `words` payload words (at most [`NODE_MAX_WORDS`]) and
    /// returns them to fill.
    ///
    /// Always a full-size array, so a writer's constant indices need no bounds
    /// checks: there were nine per quad. Every node leaves room for the largest,
    /// which keeps the array inside the half; the `spare` arm is never taken,
    /// and is there only because the compiler needs one.
    ///
    /// Inline, since it is a handful of instructions once the position is in
    /// the caller's frame; only the half-full case goes out of line.
    #[inline(always)]
    fn node(&mut self, words: usize) -> &'static mut [u32; NODE_MAX_WORDS] {
        const LAST: usize = FLOOR_CHUNK_WORDS - 1 - NODE_MAX_WORDS;
        if self.len > LAST {
            self.kick();
        }
        // `kick` empties the half; the clamp only lets the compiler see it.
        let base = self.len.min(LAST);
        let packets = floor_packets();
        let buf = &mut packets.buf[self.cur & 1];
        if self.prev != 0 {
            let addr = (buf.as_ptr() as u32 + (base as u32) * 4) & 0x00FF_FFFF;
            buf[(self.prev - 1).min(LAST)] = self.prev_size | addr;
        }
        let size = (words as u32) << 24;
        buf[base] = size | LIST_END;
        self.prev = base + 1;
        self.prev_size = size;
        self.len = base + 1 + words;
        match buf[base + 1..].first_chunk_mut::<NODE_MAX_WORDS>() {
            Some(w) => w,
            None => &mut packets.spare,
        }
    }

    /// Hands the filled half to the GPU through the shared state.
    #[cold]
    #[inline(never)]
    fn kick(&mut self) {
        let packets = floor_packets();
        packets.at = *self;
        packets.kick();
        *self = packets.at;
    }
}

impl FloorPackets {
    /// Hands the current half to the GPU and starts filling the other.
    ///
    /// Cold and out of line: inlined, the DMA kick doubled the size of every
    /// primitive writer on the floor's per-tile path.
    #[cold]
    #[inline(never)]
    fn kick(&mut self) {
        if self.at.len == 0 {
            return;
        }
        {
            // The kick waits for the previous walk before starting this one.
            let _w = crate::prof::span(crate::prof::TRACK_M7_DMA_WAIT);
            gpu::submit_linked_list_async(self.buf[self.at.cur & 1].as_ptr());
        }
        self.in_flight = true;
        self.at.cur ^= 1;
        self.at.len = 0;
        self.at.prev = 0;
    }

    /// Submits whatever is queued and waits for the walk to finish, so the
    /// command port is free for immediate-mode drawing again.
    #[inline(never)]
    fn finish(&mut self) {
        self.kick();
        if self.in_flight {
            let _w = crate::prof::span(crate::prof::TRACK_M7_DMA_WAIT);
            gpu::submit_linked_list_wait();
            self.in_flight = false;
        }
    }
}

/// The floor's packet buffer.
#[inline(always)]
fn floor_packets() -> &'static mut FloorPackets {
    unsafe { &mut *core::ptr::addr_of_mut!(FLOOR_PACKETS) }
}

/// Drains the floor's DMA before anything else talks to the GPU.
///
/// The floor pass calls this itself when it ends; it is also called before a
/// VRAM upload, since that uses the same DMA channel.
#[inline(never)]
pub fn finish_floor_packets() {
    floor_packets().finish();
}

/// Writes ground polygons as linked-list packets.
///
/// Every word that is the same for the whole tile -- the command opcode, CLUT
/// and tpage -- is packed once here, and the mesh already carries each
/// vertex's page coordinate, so a cell costs its four vertex words, four UV
/// words and one header.
///
/// A tile whose texture is still decoding is drawn flat in its placeholder
/// colour by the same sink (`flat`), rather than by a second sink type: each
/// sink type instantiates the whole mesh pipeline again, and the copy cost
/// ~5 KB of a static-RAM budget with none to spare.
struct GroundSink {
    quad_cmd: u32,
    tri_cmd: u32,
    clut: u32,
    tpage: u32,
    fog: GroundFog,
    /// Placeholder colour, when the tile is drawn flat.
    flat: Option<u32>,
    polys: u32,
    /// This tile's copy of the packet write position; see [`PacketCursor`].
    out: PacketCursor,
}

impl GroundSink {
    /// A vertex's packed UV.
    #[inline(always)]
    fn uv(&self, v: &m7_mesh::MeshVert) -> u32 {
        v.u as u32 | ((v.v as u32) << 8)
    }

    /// Header word for a polygon spanning depths `za..zb` (Q4).
    #[inline(always)]
    fn header(&self, cmd: u32, za: i32, zb: i32) -> u32 {
        let s = self.fog.shade(za, zb) as u32;
        cmd | s | (s << 8) | (s << 16)
    }
}

impl m7_mesh::Sink for GroundSink {
    fn quad(&mut self, q: [&m7_mesh::MeshVert; 4]) {
        self.polys += 1;
        if let Some(colour) = self.flat {
            let w = self.out.node(5);
            w[0] = 0x2800_0000 | colour;
            for k in 0..4 {
                w[1 + k] = pack_vertex(q[k].sx, q[k].sy);
            }
            return;
        }
        // Fog from the diagonal: the near and far corners of a ground cell.
        let header = self.header(self.quad_cmd, q[0].z, q[3].z);
        let w = self.out.node(9);
        w[0] = header;
        w[1] = pack_vertex(q[0].sx, q[0].sy);
        w[2] = self.uv(q[0]) | self.clut;
        w[3] = pack_vertex(q[1].sx, q[1].sy);
        w[4] = self.uv(q[1]) | self.tpage;
        w[5] = pack_vertex(q[2].sx, q[2].sy);
        w[6] = self.uv(q[2]);
        w[7] = pack_vertex(q[3].sx, q[3].sy);
        w[8] = self.uv(q[3]);
    }

    /// One loop for flat and textured, kept small: it is cold, but it is
    /// static RAM all the same.
    #[cold]
    #[inline(never)]
    fn fan(&mut self, v: &[&m7_mesh::MeshVert]) {
        self.polys += 1;
        let (mut zmin, mut zmax) = (i32::MAX, i32::MIN);
        for p in v {
            zmin = zmin.min(p.z);
            zmax = zmax.max(p.z);
        }
        let flat = self.flat;
        let header = match flat {
            Some(colour) => 0x2000_0000 | colour,
            None => self.header(self.tri_cmd, zmin, zmax),
        };
        let Some((&a, rest)) = v.split_first() else {
            return;
        };
        for pair in rest.windows(2) {
            let (b, c) = (pair[0], pair[1]);
            if flat.is_some() {
                let w = self.out.node(4);
                w[0] = header;
                w[1] = pack_vertex(a.sx, a.sy);
                w[2] = pack_vertex(b.sx, b.sy);
                w[3] = pack_vertex(c.sx, c.sy);
            } else {
                let w = self.out.node(7);
                w[0] = header;
                w[1] = pack_vertex(a.sx, a.sy);
                w[2] = self.uv(a) | self.clut;
                w[3] = pack_vertex(b.sx, b.sy);
                w[4] = self.uv(b) | self.tpage;
                w[5] = pack_vertex(c.sx, c.sy);
                w[6] = self.uv(c);
            }
        }
    }
}

/// Writes the grid floor's polygons as linked-list packets, like
/// [`GroundSink`] does the tile mesh's.
struct GridPackets {
    quad_cmd: u32,
    tri_cmd: u32,
    clut: u32,
    /// Each texture page's tpage word, already in place.
    tpages: [u32; TRACK_TPAGE_COUNT],
    fog: GroundFog,
    /// The current row's fogged headers.
    quad_header: u32,
    tri_header: u32,
    polys: u32,
    out: PacketCursor,
}

impl GridPackets {
    /// A vertex's texel coordinate within `page`, rounded. The far edge of a
    /// page is texel 256, which a byte cannot hold; it is drawn from 255.
    #[inline(always)]
    fn uv(v: &m7_grid::GridVert, page: (i32, i32)) -> u32 {
        let u = (((v.u + 128) >> 8) - (page.0 << 8)).clamp(0, 255) as u32;
        let t = (((v.v + 128) >> 8) - (page.1 << 8)).clamp(0, 255) as u32;
        u | (t << 8)
    }

    #[inline(always)]
    fn tpage(&self, page: (i32, i32)) -> u32 {
        self.tpages[(((page.1 & 1) << 1) | (page.0 & 1)) as usize]
    }
}

impl m7_grid::GridSink for GridPackets {
    fn row(&mut self, z_far: i32, z_near: i32) {
        let s = self.fog.shade(z_far, z_near) as u32;
        let shade = s | (s << 8) | (s << 16);
        self.quad_header = self.quad_cmd | shade;
        self.tri_header = self.tri_cmd | shade;
    }

    fn quad(&mut self, page: (i32, i32), q: [&m7_grid::GridVert; 4]) {
        self.polys += 1;
        let tpage = self.tpage(page);
        let w = self.out.node(9);
        w[0] = self.quad_header;
        w[1] = pack_vertex(q[0].sx, q[0].sy);
        w[2] = Self::uv(q[0], page) | self.clut;
        w[3] = pack_vertex(q[1].sx, q[1].sy);
        w[4] = Self::uv(q[1], page) | tpage;
        w[5] = pack_vertex(q[2].sx, q[2].sy);
        w[6] = Self::uv(q[2], page);
        w[7] = pack_vertex(q[3].sx, q[3].sy);
        w[8] = Self::uv(q[3], page);
    }

    #[cold]
    #[inline(never)]
    fn fan(&mut self, page: (i32, i32), v: &[&m7_grid::GridVert]) {
        self.polys += 1;
        let tpage = self.tpage(page);
        let Some((&a, rest)) = v.split_first() else {
            return;
        };
        for pair in rest.windows(2) {
            let (b, c) = (pair[0], pair[1]);
            let w = self.out.node(7);
            w[0] = self.tri_header;
            w[1] = pack_vertex(a.sx, a.sy);
            w[2] = Self::uv(a, page) | self.clut;
            w[3] = pack_vertex(b.sx, b.sy);
            w[4] = Self::uv(b, page) | tpage;
            w[5] = pack_vertex(c.sx, c.sy);
            w[6] = Self::uv(c, page);
        }
    }
}

/// Draws the whole Mode 7 floor as the screen-space grid (`m7_grid`).
#[inline(never)]
fn draw_grid_floor(camera: &Camera, frame: &FloorFrame, tile_wu: (i32, i32), world: (i32, i32)) {
    let tex = active_texture(unsafe { ACTIVE_CIRCUIT });
    let mut tpages = [0u32; TRACK_TPAGE_COUNT];
    for (t, p) in tpages.iter_mut().zip(tex.tpages.iter()) {
        let word = TextureMaterial::opaque(0, p.uv_tpage_word(0), (0, 0, 0)).tpage_word();
        *t = (word as u32) << 16;
    }
    let tile_tex = VRAM_TILE_DIM as i32;
    let grid = m7_grid::GridFrame::new(
        camera,
        m7_grid::SHAPE,
        tile_wu.0,
        tile_tex,
        (
            world.0 * tile_tex / tile_wu.0.max(1),
            world.1 * tile_tex / tile_wu.1.max(1),
        ),
    );
    let mut sink = GridPackets {
        quad_cmd: frame.quad_cmd,
        tri_cmd: frame.tri_cmd,
        clut: frame.clut,
        tpages,
        fog: frame.fog,
        quad_header: frame.quad_cmd,
        tri_header: frame.tri_cmd,
        polys: 0,
        out: floor_packets().at,
    };
    sink.out.node(1)[0] = frame.window;
    grid.emit(&mut sink);
    floor_packets().at = sink.out;
    crate::prof::count(crate::prof::C_M7_POLYS, sink.polys);
}

/// Everything about drawing the floor that is the same for every tile in a
/// frame, worked out once.
///
/// Per-tile setup used to rebuild these -- an outcoder with two divides, the
/// fog reciprocal, the material words -- and the per-tile path is what has to
/// fit the 4 KB instruction cache.
struct FloorFrame {
    oc: m7_mesh::Outcoder,
    fog: GroundFog,
    quad_cmd: u32,
    tri_cmd: u32,
    clut: u32,
    window: u32,
}

impl FloorFrame {
    #[inline(never)]
    fn new(camera: &Camera) -> Self {
        let tex = active_texture(unsafe { ACTIVE_CIRCUIT });
        let material = TextureMaterial::opaque(tex.clut_word, 0, (0, 0, 0));
        Self {
            oc: m7_mesh::Outcoder::new(camera, camera.z_far << 4, M7_VIEW),
            fog: GroundFog::new(camera.z_far, is_night_circuit()),
            quad_cmd: material.flat_textured_polygon_header(true),
            tri_cmd: material.flat_textured_polygon_header(false),
            clut: (material.clut_word() as u32) << 16,
            window: material.texture_window_word(),
        }
    }
}

/// The stack each floor tile is drawn on: the whole 1 KB CPU scratchpad.
///
/// The R3000 has no data cache, and the per-cell loop keeps more live values
/// than it has registers, so it reloads spilled values from its stack on every
/// cell -- about 60 stall cycles a cell with the stack in main RAM, as much as
/// it spends reading the vertices themselves. Scratchpad loads take one cycle.
///
/// Only `draw_tile_m7` runs here, not the tile loop around it: the loop also
/// services the CD, whose call tree is deep and reaches the BIOS.
/// `tools/stack_guard.py` proves the tree fits from the linked image (`make
/// stack-guard`, part of `ci-game`). Nothing else in the game uses the
/// scratchpad, and psx-rt's exception handler never touches `$sp`.
type FloorStack = psx_rt::scratchpad::ScratchpadStack<0, 1024>;

/// Draws one world tile through the Mode 7 projection.
///
/// The mesh -- depth-banded subdivision, stitching against finer neighbours and
/// near-plane clipping -- is built by [`m7_mesh::build_tile`]; see that module
/// for why each part exists. The sinks above turn it into GPU primitives: a
/// plain cell is one quad, a stitched or clipped cell a fan. Pending tiles draw
/// the same mesh flat-shaded.
#[inline(never)]
fn draw_tile_m7(
    camera: &Camera,
    tile: (i32, i32),
    tile_wu: (i32, i32),
    world: (i32, i32),
    s_idx: usize,
    frame: &FloorFrame,
    scratch: &mut m7_mesh::MeshScratch,
) {
    let Some(rect) = m7_mesh::TileRect::new(tile, tile_wu, world) else {
        return;
    };
    // The GTE takes 16-bit offsets. A tile reaching past them is thousands of
    // units away, far beyond the far plane, so it is simply not drawn.
    let (cam_qx, cam_qy) = (camera.pos.x.raw() >> 8, camera.pos.y.raw() >> 8);
    let reach = |w: i32, extent: i32, cam: i32| {
        (w * 16 - cam).abs().max(((w + extent) * 16 - cam).abs()) <= M7_GTE_REACH_Q4
    };
    if !reach(rect.wx0, rect.cw, cam_qx) || !reach(rect.wy0, rect.ch, cam_qy) {
        return;
    }
    let n = m7_mesh::rect_level(camera, &rect);
    let sides = m7_mesh::side_levels(camera, tile, tile_wu, world, n);
    let mut proj = GteGround::new(camera);

    let mut sink = GroundSink {
        quad_cmd: frame.quad_cmd,
        tri_cmd: frame.tri_cmd,
        clut: frame.clut,
        tpage: 0,
        fog: frame.fog,
        flat: None,
        polys: 0,
        out: floor_packets().at,
    };
    let map = if s_idx == usize::MAX {
        let (tx, ty) = tile;
        let (r, g, b) = texlayout::placeholder_tile_colour(true, ((tx ^ ty) & 1) != 0);
        sink.flat = Some(r as u32 | ((g as u32) << 8) | ((b as u32) << 16));
        m7_mesh::TexMap::LOCAL
    } else {
        let tex = active_texture(unsafe { ACTIVE_CIRCUIT });
        let (tpage_idx, u_base, v_base) = tex.slot_uv(s_idx);
        let tpage = TextureMaterial::opaque(0, tex.tpages[tpage_idx].uv_tpage_word(0), (0, 0, 0))
            .tpage_word();
        sink.tpage = (tpage as u32) << 16;
        sink.out.node(1)[0] = frame.window;
        m7_mesh::TexMap {
            u0: u_base as u16,
            v0: v_base as u16,
            u_shift: tex.u_shift,
        }
    };
    let points = m7_mesh::build_tile(
        camera, &rect, n, sides, map, &frame.oc, scratch, &mut proj, &mut sink,
    );
    floor_packets().at = sink.out;
    let polys = sink.polys;
    crate::prof::count(crate::prof::C_M7_POINTS, points as u32);
    crate::prof::count(crate::prof::C_M7_POLYS, polys);
}
