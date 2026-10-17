//! Freestanding `#![no_std]` SVG-derived track texture decoder with per-surface
//! detail synthesis, for direct 15bpp VRAM upload.
//!
//! # What this format is
//!
//! `tools/track_cook/build_atlas.py` rasterises each circuit's authored SVG at
//! full 1024x1024 and stores it losslessly, together with the 80x80 surface
//! code map the physics already uses. This module turns that pair back into a
//! 15bpp texture at decode time, and adds a cheap procedural detail pass on top
//! so flat raster regions read as a surface rather than as paint.
//!
//! Two properties make it worth a third format:
//!
//! 1. **Straight lines.** The raster is the SVG's own anti-aliased geometry, not
//!    an 80x80 cell majority vote, so road edges are smooth diagonals instead of
//!    a 12.8 px staircase.
//!
//! 2. **Headroom for detail.** Dropping the cook-time procedural grain is what
//!    makes this affordable: that grain cost ~325 KB of a 396 KB QOI block as
//!    per-pixel noise. Recomputing detail here instead costs no disc space at
//!    all, and it is integer-only, so it needs no FPU the R3000A lacks.
//!
//! # Container layout (`ARSV`)
//!
//! ```text
//! +----------------------------+ 0
//! | header (32 bytes)          |
//! +----------------------------+ 32
//! | surface code map           |  grid_w * grid_h nibbles, packed
//! +----------------------------+
//! | segmented-QOI base raster   |  reuse `crate::qoi` verbatim
//! +----------------------------+
//! ```
//!
//! The base raster reuses the segmented QOI container unchanged rather than
//! growing a second implementation of the same thing; this module adds the code
//! map and the detail pass on top.
//!
//! # Detail synthesis
//!
//! Detail is selected per texel from the surface code of the texel's cell, but
//! *evaluated per texel*: the code picks which function runs, it does not
//! quantise the result. That is deliberate. Quantising detail to the 12.8 px
//! cell would reintroduce exactly the blockiness the SVG raster exists to
//! remove, while evaluating at texel frequency keeps edges smooth.
//!
//! Every function is integer-only and branch-cheap: they read a small integer
//! hash of `(x, y)` and return a signed delta. No floats, no FPU, no state.

pub use crate::jpeg::TILE_PIXELS;
use crate::qoi;

/// Texels per 64x64 VRAM tile, i.e. the scratch buffer's row stride.
const TILE_TEXELS: usize = 64;

/// Container magic, `b"ARSV"`.
pub const MAGIC: [u8; 4] = *b"ARSV";
/// Container format version understood by this decoder.
pub const VERSION: u8 = 1;

/// Fixed header size, 32 bytes, so the code map and the QOI payload that follow
/// it stay naturally aligned: magic(4), version(1), flags(2), base_codec(1),
/// width(4), height(4), grid_w(2), grid_h(2), code_bytes(4), raster_bytes(4),
/// and 4 reserved.
pub const HEADER_BYTES: usize = 32;

/// Base raster is stored as a segmented-QOI payload. The only value defined.
pub const BASE_CODEC_SEGMENTED_QOI: u8 = 1;

// Surface codes. These mirror `generate_svg_circuit.py`'s constants, which are
// the level palette; `visual_tex::CLUT_RGB` indexes the same order. They are
// repeated rather than imported because that module is generated, and a codec
// must not change shape when the cooker is re-run.
pub const VOID: u8 = 0;
pub const TARMAC: u8 = 1;
pub const TARMAC_WORN: u8 = 2;
pub const KERB_WHITE: u8 = 3;
pub const KERB_RED: u8 = 4;
pub const GRASS: u8 = 5;
pub const GRAVEL: u8 = 6;
pub const SAND: u8 = 7;
pub const OIL: u8 = 8;
pub const BOOST: u8 = 9;
pub const START_LINE: u8 = 10;
pub const GATE: u8 = 11;
pub const WALL: u8 = 12;
pub const TUNNEL: u8 = 13;
pub const BRIDGE: u8 = 14;
pub const SCENERY: u8 = 15;
/// Highest defined surface code.
pub const MAX_CODE: u8 = 15;

/// Parsed metadata for an `ARSV` block.
#[derive(Copy, Clone)]
pub struct SvgHeader {
    pub width: u16,
    pub height: u16,
    pub grid_w: u16,
    pub grid_h: u16,
    pub code_bytes: u32,
    pub raster_bytes: u32,
    /// `log2(width)` when the raster is a power of two, else `u32::MAX`.
    ///
    /// The cell lookup is `texel * grid / extent` for every texel, and a
    /// `DIV` is tens of cycles on the R3000A against a handful for a shift.
    /// The cooker only ever emits 1024px rasters, so this is the difference
    /// between the detail pass costing 2.25x a QOI row and costing far less.
    cell_shift: u32,
}

impl SvgHeader {
    /// A const-constructible, entirely zeroed header. See [`SvgBlock::empty`].
    ///
    /// Every field must be zero here, not merely valid-looking. The PSX block
    /// cache is a `static mut` of ~520 KB, and the linker only places such a
    /// static in `.bss` (NOLOAD, zero-filled for free) when its initialiser is
    /// all zeros; a single non-zero byte moves the whole array into `.data`,
    /// where the PSX loader has to copy ~520 KB of zeroes into RAM at boot for
    /// no benefit. Total RAM is unchanged either way, so the gate would not
    /// catch it, but the executable grows by half a megabyte.
    pub const fn empty() -> Self {
        Self {
            width: 0,
            height: 0,
            grid_w: 0,
            grid_h: 0,
            code_bytes: 0,
            raster_bytes: 0,
            cell_shift: 0,
        }
    }
}

impl Default for SvgHeader {
    fn default() -> Self {
        Self::empty()
    }
}

impl SvgHeader {
    /// Maps a global texel coordinate to its cell index.
    ///
    /// Uses a shift when the raster extent is a power of two and falls back to
    /// division otherwise, so correctness does not depend on the fast path.
    #[inline(always)]
    fn cell_of(texel: usize, grid: usize, extent: usize, shift: u32) -> usize {
        if shift != u32::MAX {
            (texel * grid) >> shift
        } else {
            (texel * grid) / extent
        }
    }
}

/// Reads a big-endian `u32` without assuming alignment.
#[inline(always)]
fn be_u32(data: &[u8], at: usize) -> u32 {
    ((data[at] as u32) << 24)
        | ((data[at + 1] as u32) << 16)
        | ((data[at + 2] as u32) << 8)
        | (data[at + 3] as u32)
}

impl SvgHeader {
    /// Parses and validates the fixed header.
    ///
    /// Strict by design: a malformed geometry would otherwise be sampled with
    /// the wrong cell stride and produce a texture that is subtly wrong rather
    /// than visibly broken.
    pub fn parse(data: &[u8]) -> Option<Self> {
        if data.len() < HEADER_BYTES || data[0..4] != MAGIC || data[4] != VERSION {
            return None;
        }
        if data[7] != BASE_CODEC_SEGMENTED_QOI {
            return None;
        }

        let width = be_u32(data, 8) as usize;
        let height = be_u32(data, 12) as usize;
        let grid_w = u16::from_be_bytes([data[16], data[17]]) as usize;
        let grid_h = u16::from_be_bytes([data[18], data[19]]) as usize;
        let code_bytes = be_u32(data, 20) as usize;
        let raster_bytes = be_u32(data, 24) as usize;

        if width == 0 || height == 0 || grid_w == 0 || grid_h == 0 {
            return None;
        }
        // Packed nibbles: two cells per byte.
        if code_bytes != grid_w * grid_h / 2 {
            return None;
        }
        if HEADER_BYTES + code_bytes + raster_bytes > data.len() {
            return None;
        }

        // Power-of-two extents take the shift path; anything else still decodes
        // correctly via the division fallback below.
        let cell_shift = if width.is_power_of_two() && width != 0 {
            width.trailing_zeros()
        } else {
            u32::MAX
        };

        Some(Self {
            width: width as u16,
            height: height as u16,
            grid_w: grid_w as u16,
            grid_h: grid_h as u16,
            code_bytes: code_bytes as u32,
            raster_bytes: raster_bytes as u32,
            cell_shift,
        })
    }

    /// Byte offset of the code map, i.e. just past the header.
    pub const fn code_offset(&self) -> usize {
        HEADER_BYTES
    }

    /// Byte offset of the QOI base raster that follows the code map.
    pub const fn raster_offset(&self) -> usize {
        HEADER_BYTES + self.code_bytes as usize
    }
}

/// Decodes one packed code-map byte into its two surface codes.
///
/// Low nibble is the lower cell index, matching the cooker's packing order.
#[inline(always)]
pub fn code_at(codes: &[u8], index: usize) -> u8 {
    let byte = codes[index >> 1];
    if index & 1 == 0 {
        byte & 0x0F
    } else {
        byte >> 4
    }
}

/// Builds the shared per-block decode state a row needs.
///
/// Holding this once per block rather than re-deriving it per row is what keeps
/// the SVG path competitive: the QOI header and the 1024 segment offsets are
/// indexed once per CD load, exactly like the JPEG restart index.
#[derive(Copy, Clone)]
pub struct SvgBlock {
    pub header: SvgHeader,
    pub offsets: [u32; qoi::SEGMENT_COUNT],
}

impl SvgBlock {
    /// A const-constructible empty block, for the static RAM slot initialiser.
    ///
    /// `Default` cannot be used there: it is not `const`, and the PSX block
    /// cache is a `static mut` initialised at compile time.
    pub const fn empty() -> Self {
        Self {
            header: SvgHeader::empty(),
            offsets: [0; qoi::SEGMENT_COUNT],
        }
    }
}

impl Default for SvgBlock {
    fn default() -> Self {
        Self::empty()
    }
}

/// Parses an `ARSV` block and indexes its QOI base raster.
///
/// This is the `ARSV` equivalent of `qoi::index_segments`, and it is the
/// decode-path analogue of the JPEG restart scan: one linear pass over the base
/// raster, done once per CD load.
#[inline(never)]
pub fn parse_block(data: &[u8]) -> Option<SvgBlock> {
    let header = SvgHeader::parse(data)?;
    let raster =
        &data[header.raster_offset()..header.raster_offset() + header.raster_bytes as usize];
    let qoi_header = qoi::QoiHeader::parse(raster)?;
    let mut offsets = [0u32; qoi::SEGMENT_COUNT];
    qoi::index_segments(raster, &qoi_header, &mut offsets);
    Some(SvgBlock { header, offsets })
}

/// Small integer hash of a texel, used to drive every detail function.
///
/// Deliberately cheap and deliberately *not* a quality hash: detail noise only
/// has to look irregular at texel scale, and a multiply-xor pair is a handful
/// of instructions on the R3000A's integer pipeline.
#[inline(always)]
fn texel_hash(x: usize, y: usize, salt: u32) -> u32 {
    let v = (x as u32).wrapping_mul(0x9E37_79B1)
        ^ (y as u32).wrapping_mul(0x85EB_CA6B)
        ^ salt.wrapping_mul(0xC2B2_AE35);
    v ^ (v >> 15)
}

/// Returns a signed brightness delta for one texel, in 8-bit units.
///
/// The delta is applied to the raster's base colour *before* the 8->5 bit
/// reduction, so it perturbs the same quantisation boundary the Bayer dither
/// already dithers across. That keeps detail from banding: it reads as texture
/// rather than as contour lines.
#[inline(always)]
pub fn detail_delta(code: u8, x: usize, y: usize) -> i32 {
    match code {
        // Tarmac: tight, fine aggregate. Two octaves so it does not look like
        // a regular dot screen.
        TARMAC | TARMAC_WORN => {
            let h = texel_hash(
                x >> 1,
                y >> 1,
                if code == TARMAC_WORN { 0x51 } else { 0x11 },
            );
            let coarse = texel_hash(x >> 3, y >> 3, 0x77) & 3;
            -((h & 7) as i32) - 1 - coarse as i32
        }
        // Kerbs: the raster already carries the red/white check; add a faint
        // seam so the blocks read as separate units.
        KERB_WHITE | KERB_RED => {
            let h = texel_hash(x, y, 0x2B) & 3;
            (h as i32) - 2
        }
        // Grass: clumped high-frequency noise plus a slower mottle, so it
        // reads as vegetation rather than as static.
        GRASS => {
            let fine = (texel_hash(x, y, 0x9A) & 15) as i32 - 8;
            let clump = (texel_hash(x >> 2, y >> 2, 0x3D) & 7) as i32 - 4;
            fine + clump
        }
        // Gravel: sparse hard speckle, mostly bright chips on a dark base.
        GRAVEL => {
            let h = texel_hash(x, y, 0x5E);
            if h & 63 == 0 {
                22
            } else {
                -((h & 7) as i32) / 2 - 1
            }
        }
        // Sand: low-amplitude ripple aligned to the scanline.
        SAND => {
            let ripple = (((x >> 2) as i32 + (y >> 1) as i32) & 3) - 2;
            ripple * 2
        }
        // Oil slick: broad slow bands, the one surface where a smooth gradient
        // reads better than noise.
        OIL => {
            let band = (((x >> 4) as i32 * 3 + (y >> 4) as i32 * 5) % 11) - 5;
            band * 2
        }
        // Boost pad: chevrons across the pad's run direction.
        BOOST => {
            let chev = (((x as i32 + (y as i32).abs()) >> 2) & 3) - 2;
            chev * 3
        }
        // Start line: crisp checker, so the line stays legible at speed.
        START_LINE => {
            let checker = ((x >> 2) + (y >> 2)) & 1;
            if checker == 0 {
                26
            } else {
                -18
            }
        }
        // Gate posts: a vertical falloff either side of the post.
        GATE => {
            let d = (x & 15) as i32;
            if d < 2 {
                24
            } else if d > 13 {
                -14
            } else {
                0
            }
        }
        // Wall: 45-degree hatching, the classic barrier read.
        WALL => {
            let h = ((x + y) >> 1) & 3;
            (h as i32) - 2
        }
        // Tunnel: periodic ribs along the run direction.
        TUNNEL => {
            let rib = ((x >> 3) as i32) % 6;
            if rib == 0 {
                14
            } else {
                -6
            }
        }
        // Bridge: plank seams, transverse to the span.
        BRIDGE => {
            let plank = ((y >> 2) as i32) % 5;
            if plank == 0 {
                -20
            } else {
                0
            }
        }
        // Scenery: sparse vegetation clumps.
        SCENERY => {
            let h = texel_hash(x >> 1, y >> 1, 0xC4);
            match h & 31 {
                0 => 18,
                1 => -14,
                _ => 0,
            }
        }
        // VOID and any undefined code: leave the raster alone.
        _ => 0,
    }
}

/// The same 4x4 Bayer table the JPEG and QOI paths use, so all three formats
/// quantise identical base colour to identical texels and differ only in detail.
const BAYER4X4: [[i32; 4]; 4] = [[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]];

/// Decodes one 64x16 MCU row (`row` 0..4) of a 64x64 tile from an `ARSV` block,
/// emitting BGR555 texels with per-surface detail applied.
///
/// Mirrors `crate::jpeg::decode_tile_row_64x16` and
/// `crate::qoi::decode_tile_row_64x16` in shape so the streamer has one
/// dispatch point across all three codecs. `scratch` is left untouched and
/// `false` is returned when the block is unusable.
#[inline(never)]
pub fn decode_tile_row_64x16(
    block_data: &[u8],
    block: &SvgBlock,
    tile_x: usize,
    tile_y: usize,
    row: usize,
    scratch: &mut [u16; TILE_PIXELS],
) -> bool {
    let header = &block.header;
    if tile_x >= qoi::SEGMENTS_PER_AXIS
        || tile_y >= qoi::SEGMENTS_PER_AXIS
        || row >= 4
        || block_data.len() < header.raster_offset() + header.raster_bytes as usize
    {
        return false;
    }

    // Two different y values are needed and conflating them misplaces the row
    // for every tile but the one at the origin: `tile_y` addresses the scratch
    // buffer, which is tile-local, while `base_gy` is the global texel row that
    // drives the code lookup, the detail hash and the Bayer dither.
    let local_y = row * qoi::SEGMENT_HEIGHT;
    let base_gy = tile_y * TILE_TEXELS + local_y;
    let base_px = tile_x * qoi::SEGMENT_WIDTH;
    let segment_idx = (tile_y * qoi::ROWS_PER_TILE + row) * qoi::SEGMENTS_PER_AXIS + tile_x;
    if segment_idx >= qoi::SEGMENT_COUNT {
        return false;
    }

    // Decode the base raster for this row. The QOI core is shared with the
    // QOI codec itself rather than reimplemented, so both formats decode
    // identical base colour to identical values and differ only in the detail
    // pass below.
    let raster =
        &block_data[header.raster_offset()..header.raster_offset() + header.raster_bytes as usize];
    let Some(qoi_header) = qoi::QoiHeader::parse(raster) else {
        return false;
    };
    if qoi_header.segment_count as usize != qoi::SEGMENT_COUNT {
        return false;
    }

    let mut rgb = [0u8; qoi::SEGMENT_PIXELS * 3];
    qoi::decode_segment_rgb(raster, &qoi_header, &block.offsets, segment_idx, &mut rgb);

    // Quantise to 15bpp and apply per-surface detail.
    let codes =
        &block_data[header.code_offset()..header.code_offset() + header.code_bytes as usize];
    let gw = header.grid_w as usize;
    let gh = header.grid_h as usize;
    let width = header.width as usize;
    let height = header.height as usize;
    if gw == 0 || gh == 0 || width == 0 || height == 0 {
        return false;
    }

    for py in 0..qoi::SEGMENT_HEIGHT {
        let gy = base_gy + py;
        let out_y = local_y + py;
        for px in 0..qoi::SEGMENT_WIDTH {
            let gx = base_px + px;
            let src = (py * qoi::SEGMENT_WIDTH + px) * 3;

            // Cell lookup: the code selects which detail function runs, but the
            // result is evaluated per texel, so edges stay smooth.
            let cell_x = SvgHeader::cell_of(gx, gw, width, header.cell_shift);
            let cell_y = SvgHeader::cell_of(gy, gh, height, header.cell_shift);
            let cell_x = if cell_x >= gw { gw - 1 } else { cell_x };
            let cell_y = if cell_y >= gh { gh - 1 } else { cell_y };
            let code = code_at(codes, cell_y * gw + cell_x);

            let dither = BAYER4X4[gy & 3][gx & 3] >> 1;
            let delta = detail_delta(code, gx, gy);

            let r = (rgb[src] as i32 + dither + delta).clamp(0, 255) >> 3;
            let g = (rgb[src + 1] as i32 + dither + delta).clamp(0, 255) >> 3;
            let b = (rgb[src + 2] as i32 + dither + delta).clamp(0, 255) >> 3;
            scratch[out_y * 64 + px] = ((b << 10) | (g << 5) | r) as u16;
        }
    }
    true
}
