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
use arduracer_core::{Fixed, TrackDef, Vec2};

use crate::cd_fs::DiscReader;
use crate::gpu::camera::{Camera, M7_HORIZON, M7_Z_FAR, M7_Z_NEAR};
use crate::gpu::texlayout::{
    self, BLOCK_DIM, TILES_PER_BLOCK_AXIS, TRACK_TPAGE_COUNT, TRACK_VRAM_SLOTS, VRAM_SLOT_COUNT,
    VRAM_TILE_DIM,
};
use psx_gpu as gpu;
use psx_gpu::material::TextureMaterial;
use psx_vram::{dma_copy_to_vram, upload_16bpp, upload_words, Clut, TexDepth, Tpage, VramRect};

/// Capacity of the in-RAM compressed block cache (one 1024x1024 block, <= 500 KB).
pub const RAM_CACHE_BLOCKS: usize = 1;

/// Maximum number of 64x64 tiles decompressed per frame (staggers decoding to eliminate frame drops).
pub const MAX_DECODES_PER_FRAME: usize = 1;

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

/// Whether `circuit` has a TIM block on this disc, and where it starts.
///
/// `None` covers every way the path can be unavailable: the feature is off, the
/// disc has no `TIMTRACKS.BIN`, the circuit is one of the three 10 km^2 cities
/// (which ship JPEG only), or the cooker recorded no block for it.
///
/// This is the only place the per-tile sector arithmetic lives. A block is 256
/// tiles laid out row-major, four sectors each, so a tile's offset is pure
/// arithmetic on its coordinates -- there is no per-tile table to stream or
/// index, which is what makes a 2 MB uncompressed block affordable at all.
#[inline]
fn tim_block_for(circuit: usize) -> Option<(u32, u32)> {
    if !TIM_ENABLED {
        return None;
    }
    // The three cities are 10 km^2 procedurally cooked JPEG maps. They have no
    // SVG or QOI counterpart either, and TIM is no different: there is no art
    // to convert, only a generator to re-run.
    if active_city() != 0 {
        return None;
    }
    let lba = unsafe { TIMTRACKS_BIN_LBA }?;
    if circuit >= visual_tex::COUNT {
        return None;
    }
    let entry = visual_tex::CIRCUIT_TIM_SECTORS[circuit];
    if entry.sector_count == 0 {
        return None;
    }
    Some((lba + entry.sector_offset, entry.sector_count))
}

/// Whether `circuit` is served by the uncompressed TIM path.
///
/// Built from [`selects_tim`] rather than from [`tim_block_for`] alone, and the
/// difference is the whole bug this replaced.
///
/// Asking `tim_block_for(circuit).is_some()` means "does this disc carry a TIM
/// block for this circuit", which is true for *every* circuit on any disc
/// mastered with `TIMTRACKS.BIN`. Using it as the decode-path selector meant a
/// circuit pinned to QOI, JPEG or SVG still took the TIM branch: the RAM block
/// was loaded and never decoded from, every visible tile miss became a fresh
/// four-sector CD read, and CD-DA spent the race losing the drive to the
/// texture streamer. Availability is necessary, not sufficient -- `selects_tim`
/// asks about the pin too, and keeps that question out of the render loop.
#[inline]
fn codec_is_tim(circuit: usize) -> bool {
    arduracer_core::texture_format::selects_tim(
        unsafe { TEXTURE_FORMAT },
        tim_block_for(circuit).is_some(),
    )
}

/// Whether `circuit` is served by the uncompressed 4bpp path.
///
/// Same rule and same reason as [`codec_is_tim`]: availability alone is not a
/// selection. A disc carrying `TIM4TRACKS.BIN` reports availability for every
/// circuit, so a predicate reading only that would send SVG, QOI, JPEG and 15bpp
/// TIM down the 4bpp path and leave their RAM block undecoded.
#[inline]
fn codec_is_tim4(circuit: usize) -> bool {
    arduracer_core::texture_format::selects_tim4(
        unsafe { TEXTURE_FORMAT },
        tim4_block_for(circuit).is_some(),
    )
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
/// easy to confuse with each other or with the 8bpp case. The 8bpp figure is
/// the trap: 8bpp packs *two* texels per halfword, 4bpp packs *four*, so the
/// 8bpp conversion produces a 32x32 halfword rect whose *area* is right and
/// whose shape is not -- which renders as a sheared, striped track rather than
/// an obvious failure:
///
/// * 4096 texels
/// * 2048 *bytes* (four texels share a halfword)
/// * 1024 *halfwords* (four texels share a halfword)
/// * a 32x32 rect of halfwords, because the GPU counts width in words and
///   height in rows, and 4bpp packs two texel rows into each
///
/// These pin the relationship. The halfword count shipped once as 2048 -- the
/// 8bpp figure -- which would have made the FIFO fallback upload twice the tile
/// and overrun the slot by 2 KB.
const _: () = assert!(
    visual_tex::TIM4_TILE_HALFWORDS == TIM4_TILE_RECT.0 as usize * TIM4_TILE_RECT.1 as usize,
    "the 4bpp upload rect must hold exactly one tile's halfwords"
);
const _: () = assert!(
    visual_tex::TIM4_TILE_HALFWORDS * 2 == visual_tex::TIM4_TILE_BYTES,
    "halfword count and byte count must describe the same tile"
);
// The area assert above cannot catch the shape, because halving the width and
// halving the height cancel: 32x32 and 16x64 are both 1024 halfwords. Only one
// of them matches where the GPU later samples. These two pin the shape.
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

/// Whether `circuit` has a TIM4 block on this disc, and where it starts.
///
/// `None` covers every way the path can be unavailable: the feature is off, the
/// disc has no `TIM4TRACKS.BIN`, the circuit is one of the three 10 km^2 cities
/// (no art to convert, only a generator to re-run), or the cooker recorded no
/// block for it.
#[inline]
fn tim4_block_for(circuit: usize) -> Option<(u32, u32)> {
    if !TIM4_ENABLED {
        return None;
    }
    if active_city() != 0 {
        return None;
    }
    let lba = unsafe { TIM4TRACKS_BIN_LBA }?;
    if circuit >= visual_tex::COUNT {
        return None;
    }
    let entry = visual_tex::CIRCUIT_TIM4_SECTORS[circuit];
    if entry.sector_count == 0 {
        return None;
    }
    Some((lba + entry.sector_offset, entry.sector_count))
}

/// Reads `ACTIVE_CITY` without taking a reference to a mutable static.
///
/// `load_block_into_ram` copies the same field out for the same reason.
#[inline(always)]
fn active_city() -> usize {
    unsafe { core::ptr::read_volatile(core::ptr::addr_of!(ACTIVE_CITY)) }
}

/// Sampling parameters for whichever codec is active.
///
/// 15bpp and 4bpp differ in three places and agree everywhere else: the tpage
/// depth, whether a CLUT is sampled alongside, and how many texels one U unit
/// spans. Bundling them means the quad emitters -- the flat path and the Mode 7
/// grid -- stay written once instead of gaining a `match codec` each.
#[derive(Copy, Clone)]
struct TrackTexture {
    tpages: [Tpage; TRACK_TPAGE_COUNT],
    /// CLUT handle for indexed formats, 0 for 15bpp direct colour.
    clut_word: u16,
    /// Texels per U unit. Always 0: the GPU performs the depth-specific
    /// division itself (`X >> 2` at 4bpp, `X` at 15bpp), so the renderer hands
    /// it texel coordinates at every depth. This is a field rather than a
    /// constant so a future depth can be added without re-plumbing both
    /// emitters -- it is currently not that.
    u_shift: u8,
}

/// Tpage set for uncompressed 4bpp tiles.
///
/// Same page *origins* as [`TPAGES`], but the two depths do not share the slot
/// geometry. A 256x256 page holds 16 64x64 tiles at either depth -- halving the
/// bytes per texel doubles the tiles per page -- but it fills the page
/// differently: at 15bpp a tile is 64 halfwords wide, at 4bpp it is 16. So the
/// 4bpp grid is packed four times as tightly and reads through
/// [`texlayout::vram_slot_coords_tim4`].
///
/// The page regions are never live at the same time -- switching codec drops
/// every cached tile -- so sharing origins costs nothing.
const TIM4_TPAGES: [Tpage; TRACK_TPAGE_COUNT] = [
    Tpage::new(TRACK_VRAM_SLOTS[0].0, TRACK_VRAM_SLOTS[0].1, TexDepth::Bit4),
    Tpage::new(TRACK_VRAM_SLOTS[1].0, TRACK_VRAM_SLOTS[1].1, TexDepth::Bit4),
    Tpage::new(TRACK_VRAM_SLOTS[2].0, TRACK_VRAM_SLOTS[2].1, TexDepth::Bit4),
    Tpage::new(TRACK_VRAM_SLOTS[3].0, TRACK_VRAM_SLOTS[3].1, TexDepth::Bit4),
];

/// The sampling parameters for the codec currently in force on `circuit`.
#[inline]
fn active_texture(circuit: usize) -> TrackTexture {
    if codec_is_tim4(circuit) {
        TrackTexture {
            tpages: TIM4_TPAGES,
            clut_word: TIM4_CLUT.uv_clut_word(),
            u_shift: 0,
        }
    } else {
        TrackTexture {
            tpages: TPAGES,
            clut_word: 0,
            u_shift: 0,
        }
    }
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
    let known = circuit < visual_tex::COUNT;

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
                    && visual_tex::CIRCUIT_ARSV_SECTORS[circuit].sector_count > 0
            }
            TextureFormat::Qoi => {
                QOI_ENABLED
                    && have_qoi_asset
                    && known
                    && visual_tex::CIRCUIT_QOI_SECTORS[circuit].sector_count > 0
            }
            // Availability check, which is the right question here: this arm
            // *is* the place that decides whether a TIM pin can be honoured.
            // The decode paths ask `codec_is_tim` instead, because they need to
            // know what was selected, not what exists.
            TextureFormat::Tim => tim_block_for(circuit).is_some(),
            TextureFormat::Tim4 => tim4_block_for(circuit).is_some(),
            TextureFormat::Jpeg => {
                have_jpeg_asset
                    && known
                    && visual_tex::CIRCUIT_BLOCK_SECTORS[circuit].sector_count > 0
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
        // The TIM4 palette is VRAM state that outlives a tile, so dropping the
        // tiles without invalidating it would leave the next circuit's tiles
        // being drawn through the previous circuit's colours. Same for the
        // streaming FIFO: its sectors belong to a circuit that may have changed.
        TIM4_CLUT_CIRCUIT = usize::MAX;
        // Nothing buffered across a format switch any more: a tile in
        // flight dies with the decoder that asked for it.
        crate::async_tile::cancel();
    }
}

/// Prepares the track texture streaming cache for a new circuit or city.
#[allow(clippy::needless_range_loop)]
/// Grid dimensions of every shipped level, in `LEVELS.BIN` order.
///
/// The prefix-sum table [`crate::level_grid::offset_in_asset`] indexes. It
/// duplicates `TrackDef::width`/`height` deliberately: the loader computes an
/// offset for a level it has not bound yet, and asking a `TrackDef` for its own
/// dimensions would require the grid to already exist -- which is the thing being
/// loaded. A mismatch here is the one failure that produces a plausible-looking
/// level that is quietly the wrong terrain, so it is pinned by a host test.
pub const LEVEL_DIMS: [(u8, u8); 27] = [
    // 24 circuits
    (80, 80),
    (80, 80),
    (80, 80),
    (80, 80),
    (80, 80),
    (80, 80),
    (80, 80),
    (80, 80),
    (80, 80),
    (80, 80),
    (80, 80),
    (80, 80),
    (80, 80),
    (80, 80),
    (80, 80),
    (80, 80),
    (80, 80),
    (80, 80),
    (80, 80),
    (80, 80),
    (80, 80),
    (80, 80),
    (80, 80),
    (80, 80),
    // 3 free-roam cities
    (240, 240),
    (240, 240),
    (240, 240),
];

/// Loads `track`'s tile grid from disc and binds it.
///
/// Call after `init_track_texture`, which is what locates the asset.
pub fn load_level_grid(circuit: usize, track: &'static arduracer_core::TrackDef) -> bool {
    // 24 circuits occupy 0..23; the cities follow at 24..26.
    let index = match circuit {
        97 => 24,
        98 => 25,
        99 => 26,
        c => c,
    };
    if index >= LEVEL_DIMS.len() {
        return false;
    }
    let reader = unsafe { &mut *core::ptr::addr_of_mut!(DISC_READER) };
    crate::level_grid::load(reader, unsafe { LEVELS_BIN_LBA }, track, index, &LEVEL_DIMS)
}

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
        FRAME_COUNTER = 0;

        if !TRACKS_PROBED {
            let reader = &mut *core::ptr::addr_of_mut!(DISC_READER);
            crate::dbg::println(
                "[TRACKTEX] Probing disc for TRACKS.BIN, CAPETOWN.BIN, MELBOURNE.BIN, LONDON.BIN...",
            );
            TRACKS_BIN_LBA = reader.find_file_lba(visual_tex::TRACKS_BIN_NAME);
            QOITRACKS_BIN_LBA = reader.find_file_lba(visual_tex::QOITRACKS_BIN_NAME);
            SVGTRACKS_BIN_LBA = reader.find_file_lba(visual_tex::SVGTRACKS_BIN_NAME);
            TIMTRACKS_BIN_LBA = reader.find_file_lba(visual_tex::TIMTRACKS_BIN_NAME);
            TIM4TRACKS_BIN_LBA = reader.find_file_lba(visual_tex::TIM4TRACKS_BIN_NAME);
            LEVELS_BIN_LBA = reader.find_file_lba(visual_tex::LEVELS_BIN_NAME);
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
            Codec::Tim => crate::dbg::print("tim"),
            Codec::Tim4 => crate::dbg::print("tim4"),
            Codec::Jpeg => crate::dbg::print("jpeg"),
        }
        crate::dbg::println("");

        // TIM keeps no RAM block, so there is nothing to preload: its tiles come
        // from the disc one 8 KB at a time, on demand. Streaming a compressed
        // block here would read 500 KB of data nothing is going to decode.
        // `codec_is_tim`, not `tim_block_for`: see its note for why the
        // availability question is the wrong one.
        if !codec_is_tim(circuit) && !codec_is_tim4(circuit) {
            load_block_into_ram(circuit, init_bx, init_by, 0);
        }

        // The 4bpp path's palette is circuit state, not tile state: it must be
        // in VRAM before the first tile is drawn against it, and it is not
        // re-read per tile. If it cannot be read the format is unusable here,
        // so fall back rather than draw every tile through a stale palette.
        if codec_is_tim4(circuit) {
            // Raw pointer rather than a method call on the static: taking a
            // mutable reference to a mutable static is UB by the reference
            // rules, and this is single threaded.
            crate::async_tile::cancel();
            TIM4_CLUT_CIRCUIT = usize::MAX;
            if !upload_tim4_palette(circuit) {
                crate::dbg::println("  TIM4 palette unreadable, falling back");
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
        Codec::Tim => false,
        Codec::Tim4 => false,
    }
}

/// Reads `circuit`'s 16-colour palette from sector 0 of its TIM4 block and
/// uploads it to the TIM4 CLUT row.
///
/// Runs once per circuit, not once per tile. The disc holds the palette as
/// BGR555 halfwords already, so it is a straight copy: no decode, no quantise.
///
/// Returns false if the read or the upload did not happen, in which case the
/// caller should refuse the format rather than draw every tile through whatever
/// the CLUT happened to contain.
fn upload_tim4_palette(circuit: usize) -> bool {
    let Some((block_lba, _)) = tim4_block_for(circuit) else {
        return false;
    };
    if unsafe { TIM4_CLUT_CIRCUIT } == circuit {
        return true;
    }

    let scratch = unsafe { &mut *scratch() };
    let reader = unsafe { &mut *core::ptr::addr_of_mut!(DISC_READER) };
    crate::prof::count(crate::prof::C_CD_SECTORS, 1);
    let read = {
        let _read = crate::prof::span(crate::prof::TRACK_CD_READ);
        reader.read_sectors(block_lba, 1, unsafe {
            core::slice::from_raw_parts_mut(scratch.as_mut_ptr().cast::<u8>(), 2048)
        })
    };
    if !read {
        return false;
    }

    // SAFETY: the read above filled 16 halfwords' worth of bytes at the start
    // of the staging buffer, and `scratch` borrows it for the rest of this
    // scope. The disc holds the palette as BGR555 halfwords already, so this is
    // a reinterpretation, not a conversion.
    let entries = unsafe { core::slice::from_raw_parts(scratch.as_ptr().cast::<u16>(), 16) };
    let rect = VramRect::new(
        TIM4_CLUT.x(),
        TIM4_CLUT.y(),
        visual_tex::TIM4_PALETTE_ENTRIES as u16,
        1,
    );
    upload_16bpp(rect, entries);
    unsafe { TIM4_CLUT_CIRCUIT = circuit };
    true
}

/// Pulls one 4bpp tile for `circuit` into the FIFO and DMAs it into VRAM slot
/// `target_slot`.
///
/// A FIFO hit costs no CD access at all; a miss refills from the requested
/// tile's own sector, taking as many following sectors as the buffer holds. The
/// common case is a horizontal run of visible tiles, whose sectors are
/// consecutive, so a single read usually serves the next three.
#[inline(never)]
fn upload_tile_dma4(
    circuit: usize,
    tile_x: usize,
    tile_y: usize,
    target_slot: usize,
    current_frame: u32,
) -> bool {
    let Some((block_lba, block_sectors)) = tim4_block_for(circuit) else {
        return false;
    };
    if tile_x >= TILES_PER_BLOCK_AXIS || tile_y >= TILES_PER_BLOCK_AXIS {
        return false;
    }

    // The palette leads the block, so tile N is one sector further on than in
    // the 15bpp layout.
    let tile_index = (tile_y * TILES_PER_BLOCK_AXIS + tile_x) as u32;
    let tile_lba = block_lba
        .wrapping_add(visual_tex::TIM4_PALETTE_SECTORS)
        .wrapping_add(tile_index * TIM4_TILE_SECTORS);
    let block_end = block_lba + block_sectors;
    if tile_lba + TIM4_TILE_SECTORS > block_end {
        return false;
    }

    // Asynchronous: one seek covers a window of sectors, of which this tile is
    // one. A run of adjacent tiles costs one seek between them rather than one
    // each, which is the whole point -- a per-tile seek is ~150 ms and the tiles
    // the row-scan wants are contiguous on the disc.
    let src = {
        let _read = crate::prof::span(crate::prof::TRACK_CD_READ);
        let Some(p) = crate::async_tile::fetch(
            disc_reader(),
            crate::async_tile::Codec::Tim4,
            tile_lba,
            block_end - tile_lba,
            current_frame,
        ) else {
            return false;
        };
        p
    };
    crate::prof::count(crate::prof::C_CD_SECTORS, TIM4_TILE_SECTORS);

    // The 4bpp slot origin, not the 15bpp one: at this depth a 64-texel tile is
    // 16 halfwords across, so the slot grid steps a quarter as far.
    let (vx, vy, tpage_idx, _, _) = texlayout::vram_slot_coords_tim4(target_slot);
    let rect = VramRect::new(vx, vy, TIM4_TILE_RECT.0, TIM4_TILE_RECT.1);
    {
        let _upload = crate::prof::span(crate::prof::TRACK_VRAM_UPLOAD);
        // DMA first, GP0 FIFO if the channel did not report done. The FIFO path
        // re-emits GP0(A0h) from the rect origin, so it rewrites the whole tile
        // rather than leaving the aborted transfer's partial result behind.
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
                core::slice::from_raw_parts(src, visual_tex::TIM4_TILE_HALFWORDS / 2)
            });
        }
    }
    let _ = tpage_idx;

    let vram_slots = unsafe { &mut *core::ptr::addr_of_mut!(VRAM_SLOTS) };
    vram_slots[target_slot].resident_circuit = circuit;
    vram_slots[target_slot].resident_block_x = 0;
    vram_slots[target_slot].resident_block_y = 0;
    vram_slots[target_slot].resident_tile_x = tile_x;
    vram_slots[target_slot].resident_tile_y = tile_y;
    vram_slots[target_slot].last_used = current_frame;
    true
}

/// Streams one 64x64 15bpp tile from `TIMTRACKS.BIN` into VRAM slot
/// `target_slot`, and marks the slot resident on success.
///
/// This is the whole of the TIM path. There is no header, no entropy coding and
/// no inverse DCT: four sectors are read straight into the staging buffer and
/// the GPU's block-mode DMA moves them into VRAM. What the player is buying is
/// a tile upload that costs a CD read and nothing else.
///
/// The buffer is the same 8 KB the decoders use, so this format costs no extra
/// main RAM despite being the only one that needs its data resident at upload
/// time.
///
/// Returns false if the tile could not be read or transferred, leaving the slot
/// unclaimed so the caller paints its fallback and retries next frame.
#[inline(never)]
fn upload_tile_dma(
    circuit: usize,
    tile_x: usize,
    tile_y: usize,
    target_slot: usize,
    current_frame: u32,
) -> bool {
    let Some((block_lba, block_sectors)) = tim_block_for(circuit) else {
        return false;
    };
    if tile_x >= TILES_PER_BLOCK_AXIS || tile_y >= TILES_PER_BLOCK_AXIS {
        return false;
    }

    // Row-major over the 16x16 tile grid, four sectors each. The cooker lays
    // the block out in exactly this order, which is why there is no index.
    let tile_index = (tile_y * TILES_PER_BLOCK_AXIS + tile_x) as u32;
    let block_end = block_lba + block_sectors;
    let tile_lba = block_lba + tile_index * TIM_TILE_SECTORS;
    if tile_lba + TIM_TILE_SECTORS > block_end {
        return false;
    }

    // Four 2048-byte sectors exactly fill the 8 KB tile with no partial
    // sector and no repacking.
    let tile_bytes = TIM_TILE_SECTORS as usize * 2048;
    if tile_bytes != TILE_PIXELS * 2 {
        return false;
    }
    // Asynchronous, like the TIM4 path: issue the seek on one frame and take
    // the sectors on the next few. The blocking form waited here, inside the
    // render path, and that wait is what cost 8.6x a frame budget in the worst
    // measured case.
    let src = {
        let _read = crate::prof::span(crate::prof::TRACK_CD_READ);
        let Some(p) = crate::async_tile::fetch(
            disc_reader(),
            crate::async_tile::Codec::Tim,
            tile_lba,
            block_end - tile_lba,
            current_frame,
        ) else {
            // Still being fetched: the caller paints the placeholder and asks
            // again next frame. The four sectors of a TIM tile are contiguous,
            // so they are handed back as one run to DMA.
            let _ = tile_bytes;
            return false;
        };
        p
    };
    crate::prof::count(crate::prof::C_CD_SECTORS, TIM_TILE_SECTORS);

    let (vx, vy, _, _, _) = texlayout::vram_slot_coords(target_slot);
    let rect = VramRect::new(vx, vy, VRAM_TILE_DIM, VRAM_TILE_DIM);
    {
        let _upload = crate::prof::span(crate::prof::TRACK_VRAM_UPLOAD);
        // Block-mode DMA is the point of the format, but it is not safe enough
        // to be the only way to get the pixels in: a wedged DMA channel is a
        // documented outcome on this hardware, and the FIFO path costs the same
        // GP0 command stream with none of that risk. So DMA first, FIFO if the
        // channel did not report done.
        //
        // Falling back is correct rather than merely defensive: `dma_copy_to_vram`
        // aborts before returning false, and the FIFO path re-emits the
        // GP0(A0) header from the rect origin, so the whole tile is rewritten
        // rather than leaving the partial transfer the aborted one left behind.
        // Count the path taken, not just the outcome: "the DMA channel wedged"
        // and "the DMA path is fine but this tile's rect cannot be DMA'd" both
        // return false, and only the first is a silicon story.
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
                core::slice::from_raw_parts(src, TILE_PIXELS / 2)
            });
        }
    }

    let vram_slots = unsafe { &mut *core::ptr::addr_of_mut!(VRAM_SLOTS) };
    vram_slots[target_slot].resident_circuit = circuit;
    vram_slots[target_slot].resident_block_x = 0;
    vram_slots[target_slot].resident_block_y = 0;
    vram_slots[target_slot].resident_tile_x = tile_x;
    vram_slots[target_slot].resident_tile_y = tile_y;
    vram_slots[target_slot].last_used = current_frame;
    true
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
fn upload_tile_sync(
    circuit: usize,
    tile_x: usize,
    tile_y: usize,
    target_slot: usize,
) -> bool {
    let Some((block_lba, block_sectors)) = tim_block_for(circuit) else {
        return false;
    };
    if tile_x >= TILES_PER_BLOCK_AXIS || tile_y >= TILES_PER_BLOCK_AXIS {
        return false;
    }

    let tile_index = (tile_y * TILES_PER_BLOCK_AXIS + tile_x) as u32;
    let block_end = block_lba + block_sectors;
    let tile_lba = block_lba + tile_index * TIM_TILE_SECTORS;
    if tile_lba + TIM_TILE_SECTORS > block_end {
        return false;
    }

    let byte_count = TIM_TILE_SECTORS as usize * 2048;
    let dst = unsafe {
        core::slice::from_raw_parts_mut(scratch() as *mut u8, byte_count)
    };
    if !disc_reader().read_sectors(tile_lba, TIM_TILE_SECTORS as usize, dst) {
        return false;
    }

    let (vx, vy, _, _, _) = texlayout::vram_slot_coords(target_slot);
    let rect = VramRect::new(vx, vy, VRAM_TILE_DIM, VRAM_TILE_DIM);
    let src = scratch_words() as *const u32;
    let via_dma = dma_copy_to_vram(rect, src);
    if !via_dma {
        upload_words(rect, unsafe {
            core::slice::from_raw_parts(src, TILE_PIXELS / 2)
        });
    }

    let vram_slots = unsafe { &mut *core::ptr::addr_of_mut!(VRAM_SLOTS) };
    vram_slots[target_slot].resident_circuit = circuit;
    vram_slots[target_slot].resident_block_x = 0;
    vram_slots[target_slot].resident_block_y = 0;
    vram_slots[target_slot].resident_tile_x = tile_x;
    vram_slots[target_slot].resident_tile_y = tile_y;
    vram_slots[target_slot].last_used = 1;
    true
}

#[inline(never)]
fn upload_tile_sync4(
    circuit: usize,
    tile_x: usize,
    tile_y: usize,
    target_slot: usize,
) -> bool {
    let Some((block_lba, block_sectors)) = tim4_block_for(circuit) else {
        return false;
    };
    if tile_x >= TILES_PER_BLOCK_AXIS || tile_y >= TILES_PER_BLOCK_AXIS {
        return false;
    }
    if !upload_tim4_palette(circuit) {
        return false;
    }

    let tile_index = (tile_y * TILES_PER_BLOCK_AXIS + tile_x) as u32;
    let tile_lba = block_lba
        .wrapping_add(visual_tex::TIM4_PALETTE_SECTORS)
        .wrapping_add(tile_index * TIM4_TILE_SECTORS);
    let block_end = block_lba + block_sectors;
    if tile_lba + TIM4_TILE_SECTORS > block_end {
        return false;
    }

    let byte_count = TIM4_TILE_SECTORS as usize * 2048;
    let dst = unsafe {
        core::slice::from_raw_parts_mut(scratch() as *mut u8, byte_count)
    };
    if !disc_reader().read_sectors(tile_lba, TIM4_TILE_SECTORS as usize, dst) {
        return false;
    }

    let (vx, vy, _, _, _) = texlayout::vram_slot_coords_tim4(target_slot);
    let rect = VramRect::new(vx, vy, TIM4_TILE_RECT.0, TIM4_TILE_RECT.1);
    let src = scratch_words() as *const u32;
    let via_dma = dma_copy_to_vram(rect, src);
    if !via_dma {
        upload_words(rect, unsafe {
            core::slice::from_raw_parts(src, visual_tex::TIM4_TILE_HALFWORDS / 2)
        });
    }

    let vram_slots = unsafe { &mut *core::ptr::addr_of_mut!(VRAM_SLOTS) };
    vram_slots[target_slot].resident_circuit = circuit;
    vram_slots[target_slot].resident_block_x = 0;
    vram_slots[target_slot].resident_block_y = 0;
    vram_slots[target_slot].resident_tile_x = tile_x;
    vram_slots[target_slot].resident_tile_y = tile_y;
    vram_slots[target_slot].last_used = 1;
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
    // TIM has no block to decode out of: the tile comes straight off the disc.
    // During prewarm, read synchronously so the race starts with all visible tiles resident.
    if codec_is_tim(circuit) {
        if upload_tile_sync(circuit, sub_tx, sub_ty, target_slot) {
            return;
        }
        let slot = unsafe { &mut *core::ptr::addr_of_mut!(VRAM_SLOTS) };
        slot[target_slot] = VramSlot::empty();
        return;
    }

    // Same rule for the 4bpp path.
    if codec_is_tim4(circuit) {
        if upload_tile_sync4(circuit, sub_tx, sub_ty, target_slot) {
            return;
        }
        let slot = unsafe { &mut *core::ptr::addr_of_mut!(VRAM_SLOTS) };
        slot[target_slot] = VramSlot::empty();
        return;
    }

    let ram_slot_idx = ensure_ram_block(circuit, bx, by, 0);
    let ram_slot = unsafe { &RAM_BLOCKS[ram_slot_idx] };
    let scratch = unsafe { &mut *scratch() };

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
    let (half_w, half_h) = if camera.mode7 {
        (M7_SCAN_RADIUS, M7_SCAN_RADIUS)
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

    let mut tiles = [(0usize, 0usize, 0usize, 0usize, 0u32); VRAM_SLOT_COUNT];
    let mut count = 0;
    for ty in min_ty..=max_ty {
        for tx in min_tx..=max_tx {
            let wx0 = tx * tile_wu_x;
            let wy0 = ty * tile_wu_y;
            if camera.mode7 && !m7_tile_visible(camera, wx0, wy0, tile_wu_x, tile_wu_y) {
                continue;
            }
            let bx = (wx0 / block_wu_x) as usize;
            let by = (wy0 / block_wu_y) as usize;
            let sub_tx = ((wx0 % block_wu_x) / tile_wu_x)
                .clamp(0, (TILES_PER_BLOCK_AXIS - 1) as i32) as usize;
            let sub_ty = ((wy0 % block_wu_y) / tile_wu_y)
                .clamp(0, (TILES_PER_BLOCK_AXIS - 1) as i32) as usize;
            let tile_idx = (sub_ty * TILES_PER_BLOCK_AXIS + sub_tx) as u32;

            if count < VRAM_SLOT_COUNT {
                tiles[count] = (bx, by, sub_tx, sub_ty, tile_idx);
                count += 1;
            }
        }
    }

    // For uncompressed disc streaming, sort by tile_index (disc LBA)
    // so the disc head sweeps monotonically forward without backwards seeks!
    if codec_is_tim(active_circuit) || codec_is_tim4(active_circuit) {
        for i in 1..count {
            let mut j = i;
            while j > 0 && tiles[j - 1].4 > tiles[j].4 {
                tiles.swap(j - 1, j);
                j -= 1;
            }
        }
    }

    for (slot_idx, &(bx, by, sub_tx, sub_ty, _)) in tiles[..count].iter().enumerate() {
        decode_and_upload_full_tile(active_circuit, bx, by, sub_tx, sub_ty, slot_idx);
    }
}

/// Streams a 1024x1024 compressed block (<= 500 KB) from CD-ROM into the
/// specified RAM cache slot.
#[inline(never)]
fn load_block_into_ram(circuit: usize, block_x: usize, block_y: usize, slot_idx: usize) {
    let city = unsafe { ACTIVE_CITY };
    let codec = codec_for_circuit(circuit);

    // TIM has no block: its tiles are read individually by `upload_tile_dma`, and
    // the RAM buffer would only ever hold the last 8 KB of a 2 MB block. Mark
    // the slot invalid so a stray `ensure_ram_block` paints fallbacks instead of
    // handing a decoder 500 KB of texels to interpret.
    if codec == Codec::Tim {
        let ram_slot = unsafe { &mut (*core::ptr::addr_of_mut!(RAM_BLOCKS))[slot_idx] };
        ram_slot.resident_circuit = circuit;
        ram_slot.resident_block_x = block_x;
        ram_slot.resident_block_y = block_y;
        ram_slot.byte_len = 0;
        ram_slot.valid = false;
        return;
    }

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
///
/// TIM does not use the row machinery at all. Its whole tile arrives in one
/// transfer, so there is nothing to slice: the job completes in a single step,
/// and the cost of that step is the CD read the other formats amortise across
/// 256 tiles. The per-frame ceiling is unchanged, which keeps a stalled drive
/// from turning into a frame overrun.
#[inline(never)]
fn step_decode_job(current_frame: u32) {
    let job = unsafe { &mut *core::ptr::addr_of_mut!(ACTIVE_JOB) };
    if !job.active {
        return;
    }

    // `codec_is_tim`, not `tim_block_for`: asking only whether a TIM block exists
    // sends every codec on a TIM-bearing disc down this path and abandons its
    // RAM block. See `codec_is_tim`.
    if codec_is_tim(job.circuit) {
        crate::prof::count(crate::prof::C_DECODE_ROWS, 1);
        crate::prof::count(crate::prof::C_TILE_DECODES, 1);
        // A failed read leaves the slot empty, which renders as the placeholder
        // and retries next frame. `job.active` clears either way: a read that
        // is going to keep failing should not pin the queue on this tile.
        if !upload_tile_dma(
            job.circuit,
            job.tile_x,
            job.tile_y,
            job.target_slot,
            current_frame,
        ) {
            let slots = unsafe { &mut *core::ptr::addr_of_mut!(VRAM_SLOTS) };
            slots[job.target_slot] = VramSlot::empty();
        }
        job.active = false;
        return;
    }

    if codec_is_tim4(job.circuit) {
        crate::prof::count(crate::prof::C_DECODE_ROWS, 1);
        crate::prof::count(crate::prof::C_TILE_DECODES, 1);
        if !upload_tile_dma4(
            job.circuit,
            job.tile_x,
            job.tile_y,
            job.target_slot,
            current_frame,
        ) {
            let slots = unsafe { &mut *core::ptr::addr_of_mut!(VRAM_SLOTS) };
            slots[job.target_slot] = VramSlot::empty();
        }
        job.active = false;
        return;
    }

    let ram_slot_idx = ensure_ram_block(job.circuit, job.block_x, job.block_y, current_frame);
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
/// stack without limit. `visible_tiles` uses the same shape at 48.
const PREFETCH_MAX: usize = 16;

/// Whether `tile` currently has its pixels resident in a VRAM slot.
#[inline]
fn tile_resident(vram_slots: &[VramSlot; VRAM_SLOT_COUNT], circuit: usize, tile: &TileRef) -> bool {
    let (_, _, bx, by, sub_tx, sub_ty) = *tile;
    vram_slots.iter().any(|slot| {
        slot.resident_circuit == circuit
            && slot.resident_block_x == bx
            && slot.resident_block_y == by
            && slot.resident_tile_x == sub_tx
            && slot.resident_tile_y == sub_ty
    })
}

/// Index of the first entry of `list` that is not resident, as `(index, tile)`.
///
/// Used for the prefetch list, which has no `slot_for_tile` probe result of its
/// own. Visible tiles go through `slot_for_tile` instead, so their existing scan
/// order and slot bookkeeping are untouched.
fn first_missing_tile(
    list: &[TileRef],
    vram_slots: &[VramSlot; VRAM_SLOT_COUNT],
    circuit: usize,
) -> Option<(usize, TileRef)> {
    for (i, tile) in list.iter().enumerate() {
        if !tile_resident(vram_slots, circuit, tile) {
            return Some((i, *tile));
        }
    }
    None
}

/// The tile lists one frame's decode scheduler draws candidates from.
///
/// Bundled rather than passed as four loose parameters so the ordering rule
/// (visible before prefetch) is stated once, in one place.
struct TileCandidates<'a> {
    /// Tiles inside the screen rectangle. Phase 3 draws these.
    visible: &'a [TileRef],
    visible_count: usize,
    /// One tile row above and below the visible rectangle. Decode-only.
    prefetch: &'a [TileRef],
    prefetch_count: usize,
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
    slot_for_tile: &mut [usize; 48],
    used_slots: &mut [bool; VRAM_SLOT_COUNT],
    vram_slots: &mut [VramSlot; VRAM_SLOT_COUNT],
) -> bool {
    let vis_count = candidates.visible_count;
    let job = unsafe { &mut *core::ptr::addr_of_mut!(ACTIVE_JOB) };
    if !job.active {
        // No active decode job running: pick the next tile that needs one.
        // For uncompressed disc streaming, pick the missing visible tile with the
        // lowest tile_index (disc LBA) to keep CD seeks monotonic and sequential.
        let candidate = if codec_is_tim(active_circuit) || codec_is_tim4(active_circuit) {
            let mut best_vis: Option<(usize, usize, usize, usize, u32)> = None;
            for i in 0..vis_count {
                if slot_for_tile[i] == usize::MAX {
                    let (_, _, bx, by, sub_tx, sub_ty) = candidates.visible[i];
                    let idx = (sub_ty * TILES_PER_BLOCK_AXIS + sub_tx) as u32;
                    if best_vis.is_none() || idx < best_vis.unwrap().4 {
                        best_vis = Some((bx, by, sub_tx, sub_ty, idx));
                    }
                }
            }
            if let Some((bx, by, sub_tx, sub_ty, _)) = best_vis {
                Some((bx, by, sub_tx, sub_ty))
            } else {
                first_missing_tile(
                    &candidates.prefetch[..candidates.prefetch_count],
                    vram_slots,
                    active_circuit,
                )
                .map(|(_, (_, _, bx, by, sub_tx, sub_ty))| (bx, by, sub_tx, sub_ty))
            }
        } else {
            let visible_miss = (0..vis_count).find(|&i| slot_for_tile[i] == usize::MAX);
            match visible_miss {
                Some(i) => {
                    let (_, _, bx, by, sub_tx, sub_ty) = candidates.visible[i];
                    Some((bx, by, sub_tx, sub_ty))
                }
                None => first_missing_tile(
                    &candidates.prefetch[..candidates.prefetch_count],
                    vram_slots,
                    active_circuit,
                )
                .map(|(_, (_, _, bx, by, sub_tx, sub_ty))| (bx, by, sub_tx, sub_ty)),
            }
        };
        let Some((bx, by, sub_tx, sub_ty)) = candidate else {
            return false;
        };

        // Find oldest VRAM slot not currently visible in this frame. `used_slots`
        // holds both visible residents and in-flight decode targets, so this can
        // only evict a tile nothing on screen needs.
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
    }

    let job_bx = job.block_x;
    let job_by = job.block_y;
    let job_tx = job.tile_x;
    let job_ty = job.tile_y;
    let job_slot = job.target_slot;

    step_decode_job(current_frame);

    // If step_decode_job completed and the slot is resident, update slot_for_tile
    // so Phase 3 renders it immediately textured without placeholder!
    if !job.active && vram_slots[job_slot].resident_circuit == active_circuit {
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
    let (half_w, half_h) = if camera.mode7 {
        (M7_SCAN_RADIUS, M7_SCAN_RADIUS)
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

    unsafe {
        FRAME_COUNTER = FRAME_COUNTER.wrapping_add(1);
    }
    let current_frame = unsafe { FRAME_COUNTER };
    // Advance any tile fetch already in flight before deciding what this frame
    // wants. The drive pushes on its own clock, so this cannot be left to the
    // tile request below: a frame that asks for nothing would leave the sectors
    // sitting in the drive's two-slot FIFO and overrun it.
    crate::async_tile::pump(disc_reader(), current_frame);
    let active_circuit = unsafe { ACTIVE_CIRCUIT };
    // Sampling parameters for whichever codec this circuit selected: tpage
    // depth, CLUT handle, and how many texels a U unit spans. Bound once here
    // so both the flat and Mode 7 emitters read the same thing.
    let tex = active_texture(active_circuit);
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

                if tx0 < tx1
                    && ty0 < ty1
                    && vis_count < 48
                    && (!camera.mode7 || m7_tile_visible(camera, wx0, wy0, tile_wu_x, tile_wu_y))
                {
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

    // One tile row of lookahead above and below the visible rectangle.
    //
    // A tile's decode is time-sliced across 4 MCU rows, so a tile first seen at
    // the screen edge spends several frames as a placeholder. Starting its
    // decode a row early means it is usually resident by the time the camera
    // reaches it. Vertical only: adding columns as well would roughly double
    // the candidate set against a fixed 64-slot VRAM pool, and horizontal
    // scrolling is the cheap direction (tiles enter sideways, already decoded
    // by the row that was already in view).
    //
    // These are candidates only -- Phase 3 still draws `visible_tiles`, so a
    // prefetched tile costs a decode row and a slot, never a polygon.
    let mut prefetch_tiles: [TileRef; PREFETCH_MAX] = [(0, 0, 0, 0, 0, 0); PREFETCH_MAX];
    let mut prefetch_count = 0usize;
    {
        let world_max_tx = (world_w - 1).max(0) / tile_wu_x;
        let world_max_ty = (world_h - 1).max(0) / tile_wu_y;

        // Vertical lookahead: rows immediately above and below the frustum
        for ty in [min_ty.saturating_sub(1), max_ty + 1] {
            if !(0..=world_max_ty).contains(&ty) || (min_ty..=max_ty).contains(&ty) {
                continue;
            }
            for tx in min_tx..=max_tx {
                if prefetch_count >= PREFETCH_MAX {
                    break;
                }
                let wx0 = tx * tile_wu_x;
                let wy0 = ty * tile_wu_y;
                if camera.mode7 && !m7_tile_visible(camera, wx0, wy0, tile_wu_x, tile_wu_y) {
                    continue;
                }
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

        // Horizontal lookahead: columns immediately left and right of the frustum
        for tx in [min_tx.saturating_sub(1), max_tx + 1] {
            if !(0..=world_max_tx).contains(&tx) || (min_tx..=max_tx).contains(&tx) {
                continue;
            }
            for ty in min_ty..=max_ty {
                if prefetch_count >= PREFETCH_MAX {
                    break;
                }
                let wx0 = tx * tile_wu_x;
                let wy0 = ty * tile_wu_y;
                if camera.mode7 && !m7_tile_visible(camera, wx0, wy0, tile_wu_x, tile_wu_y) {
                    continue;
                }
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

        // For TIM/TIM4 streaming, sort prefetch candidates in ascending disc LBA order
        if codec_is_tim(active_circuit) || codec_is_tim4(active_circuit) {
            for i in 1..prefetch_count {
                let mut j = i;
                while j > 0 {
                    let prev_idx = prefetch_tiles[j - 1].5 * TILES_PER_BLOCK_AXIS + prefetch_tiles[j - 1].4;
                    let curr_idx = prefetch_tiles[j].5 * TILES_PER_BLOCK_AXIS + prefetch_tiles[j].4;
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
    // Up to `max_decode_rows` can be processed, provided the frame stays inside its own
    // VBlank period. If the period elapses during decode, or the budget is exhausted, stop.
    let mut rows_decoded = 0;
    let period = frame_period();
    while rows_decoded < max_decode_rows {
        let elapsed_vblanks = psx_rt::interrupts::vblank_count().wrapping_sub(last_flip_vblank);
        // Stop once the frame's own period has run out.
        //
        // `last_flip_vblank` is stamped in the frame's step 1, right after the
        // previous flip is applied, so `elapsed` counts VBlanks *since that
        // stamp*: the frame body runs at elapsed == 0 and ends when the next
        // VBlank lands at elapsed == period. The test must therefore be
        // `elapsed >= period`.
        //
        // It was `elapsed >= period || elapsed + 1 >= period`, and the second
        // clause is the bug: at 60 FPS the period is 1, so `0 + 1 >= 1` was
        // true on the very first iteration of every frame and the loop never
        // decoded anything at all. At 30 FPS the clause reduces to
        // `elapsed >= 1`, which happened to match the old hardcoded behaviour,
        // which is why only 60 FPS was affected.
        if elapsed_vblanks >= period {
            break;
        }

        let did_work = step_or_schedule_decode_row(
            current_frame,
            active_circuit,
            &TileCandidates {
                visible: &visible_tiles,
                visible_count: vis_count,
                prefetch: &prefetch_tiles,
                prefetch_count,
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

    // Phase 3: Render visible quads (textured for resident tiles, placeholder flat for pending)
    let _quads = crate::prof::span(crate::prof::TRACK_QUADS);
    if camera.mode7 {
        draw_m7_backdrop();
    }
    for i in 0..vis_count {
        let (tx, ty, _, _, _, _) = visible_tiles[i];
        let s_idx = slot_for_tile[i];

        if s_idx == usize::MAX {
            crate::prof::count(crate::prof::C_PLACEHOLDERS, 1);
        }

        if camera.mode7 {
            draw_tile_m7(
                camera,
                (tx, ty),
                (tile_wu_x, tile_wu_y),
                (world_w, world_h),
                s_idx,
            );
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
            let (_, _, tpage_idx, u_base, v_base) = texlayout::vram_slot_coords(s_idx);

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
            let material =
                TextureMaterial::opaque(tex.clut_word, tpage.uv_tpage_word(0), (0x80, 0x80, 0x80));
            gpu::draw_quad_textured_material(
                [tl, tr, bl, br],
                [(u0, v0), (u1, v0), (u0, v1), (u1, v1)],
                material,
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Mode 7 chase view
// ---------------------------------------------------------------------------

/// Half-width of the square of world units scanned for Mode 7 tiles. It must
/// reach the far corner of the frustum: `M7_Z_FAR` deep and ~1.33x that wide.
const M7_SCAN_RADIUS: i32 = 440;

/// True if any part of the tile can fall inside the Mode 7 frustum.
///
/// A conservative circle-vs-trapezoid test on the tile centre: the tile's
/// bounding radius pads both the depth range and the lateral edge test
/// (screen half-width is `1.33 * z`, tested as `5 * lateral <= 7 * z`).
fn m7_tile_visible(camera: &Camera, wx0: i32, wy0: i32, tw: i32, th: i32) -> bool {
    let cam_qx = camera.pos.x.raw() >> 8;
    let cam_qy = camera.pos.y.raw() >> 8;
    let (right, z) =
        camera.view_from_offset_q4((2 * wx0 + tw) * 8 - cam_qx, (2 * wy0 + th) * 8 - cam_qy);
    // 0.75 * the larger edge, in Q4 units: bounds the half-diagonal.
    let rad = tw.max(th) * 12;
    if z + rad < (M7_Z_NEAR << 4) || z - rad > (M7_Z_FAR << 4) {
        return false;
    }
    (right.abs() - rad) * 5 <= (z + rad).max(0) * 7
}

/// Sky bands above the horizon and distant haze down to where the tiles start.
fn draw_m7_backdrop() {
    let h = M7_HORIZON as i16;
    gpu::draw_rect_flat(0, 0, 320, 20, 28, 46, 96);
    gpu::draw_rect_flat(0, 20, 320, 20, 52, 84, 140);
    gpu::draw_rect_flat(0, 40, 320, (h - 40) as u16, 110, 140, 175);
    // Distant ground haze, fading into the fogged far tiles.
    gpu::draw_rect_flat(0, h, 320, 30, 70, 86, 110);
    gpu::draw_rect_flat(0, h + 30, 320, 240 - (h + 30) as u16, 52, 60, 74);
}

/// Draws one world tile through the Mode 7 projection.
///
/// The GPU maps textures affinely, so a big perspective quad swims. Each
/// textured tile is therefore a grid of small cells, finer near the camera
/// (where perspective changes fastest) and coarser far away; neighbouring
/// cells share projected corners, so the grid has no cracks. Pending tiles
/// are a single flat quad. Distance fog darkens the texture modulation.
#[inline(never)]
fn draw_tile_m7(
    camera: &Camera,
    tile: (i32, i32),
    tile_wu: (i32, i32),
    world: (i32, i32),
    s_idx: usize,
) {
    let (tx, ty) = tile;
    let (tw, th) = tile_wu;
    let wx0 = tx * tw;
    let wy0 = ty * th;
    let cw = (wx0 + tw).min(world.0) - wx0;
    let ch = (wy0 + th).min(world.1) - wy0;
    if cw <= 0 || ch <= 0 {
        return;
    }
    let cam_qx = camera.pos.x.raw() >> 8;
    let cam_qy = camera.pos.y.raw() >> 8;

    let textured = s_idx != usize::MAX;
    let (nx, ny) = if textured {
        let (_, zc) =
            camera.view_from_offset_q4((2 * wx0 + cw) * 8 - cam_qx, (2 * wy0 + ch) * 8 - cam_qy);
        let target = if zc < (150 << 4) {
            32
        } else if zc < (210 << 4) {
            64
        } else {
            128
        };
        (
            ((cw + target - 1) / target).clamp(1, 4),
            ((ch + target - 1) / target).clamp(1, 4),
        )
    } else {
        (1, 1)
    };

    // Projected grid corners, row stride 9 (max 8 cells + 1).
    let mut pts = [(0i16, 0i16); 81];
    let mut pz = [0i32; 81];
    for j in 0..=ny {
        for i in 0..=nx {
            let wx = wx0 + cw * i / nx;
            let wy = wy0 + ch * j / ny;
            let (r, z) = camera.view_from_offset_q4(wx * 16 - cam_qx, wy * 16 - cam_qy);
            let k = (j * 9 + i) as usize;
            pts[k] = Camera::project_rz(r, z);
            pz[k] = z;
        }
    }

    if !textured {
        let checker = ((tx ^ ty) & 1) != 0;
        let (pr, pg, pb) = texlayout::placeholder_tile_colour(true, checker);
        let (a, b, c, d) = (
            0usize,
            nx as usize,
            (ny * 9) as usize,
            (ny * 9 + nx) as usize,
        );
        gpu::draw_quad_flat([pts[a], pts[b], pts[c], pts[d]], pr, pg, pb);
        return;
    }

    let (_, _, tpage_idx, u_base, v_base) = texlayout::vram_slot_coords(s_idx);
    let tex = active_texture(unsafe { ACTIVE_CIRCUIT });
    let tpage = tex.tpages[tpage_idx];
    let tpage_word = tpage.uv_tpage_word(0);
    let u_shift = tex.u_shift;
    let clut_word = tex.clut_word;

    for j in 0..ny {
        for i in 0..nx {
            let a = (j * 9 + i) as usize;
            let b = a + 1;
            let c = a + 9;
            let d = c + 1;
            let zmax = pz[a].max(pz[b]).max(pz[c]).max(pz[d]);
            let zmin = pz[a].min(pz[b]).min(pz[c]).min(pz[d]);
            if zmax < (M7_Z_NEAR << 4) || zmin > (M7_Z_FAR << 4) {
                continue;
            }
            let (pa, pb, pc, pd) = (pts[a], pts[b], pts[c], pts[d]);
            let min_x = pa.0.min(pb.0).min(pc.0).min(pd.0);
            let max_x = pa.0.max(pb.0).max(pc.0).max(pd.0);
            let min_y = pa.1.min(pb.1).min(pc.1).min(pd.1);
            let max_y = pa.1.max(pb.1).max(pc.1).max(pd.1);
            // Off screen, or too large for the GPU's 1023x511 primitive limit.
            if max_x < 0 || min_x > 320 || max_y < 0 || min_y > 240 {
                continue;
            }
            if max_x as i32 - min_x as i32 > 1000 || max_y as i32 - min_y as i32 > 500 {
                continue;
            }

            let lu0 = ((cw * i / nx) * 64 / tw) as u16;
            let lu1 = ((cw * (i + 1) / nx) * 64 / tw) as u16;
            let lv0 = ((ch * j / ny) * 64 / th) as u16;
            let lv1 = ((ch * (j + 1) / ny) * 64 / th) as u16;
            let u0 = (((u_base as u32 + lu0 as u32) >> u_shift).min(255)) as u8;
            let u1 = (((u_base as u32 + lu1 as u32) >> u_shift).min(255)) as u8;
            let v0 = ((v_base as u32 + lv0 as u32).min(255)) as u8;
            let v1 = ((v_base as u32 + lv1 as u32).min(255)) as u8;

            // Fog: full brightness out to 150 units, fading to ~55% at the far plane.
            let z_units = (zmax + zmin) >> 5;
            let shade = if z_units <= 150 {
                128
            } else {
                (128 - (z_units - 150) * 58 / 120).max(70)
            } as u8;
            let material = TextureMaterial::opaque(clut_word, tpage_word, (shade, shade, shade));
            gpu::draw_quad_textured_material(
                [pa, pb, pc, pd],
                [(u0, v0), (u1, v0), (u0, v1), (u1, v1)],
                material,
            );
        }
    }
}
