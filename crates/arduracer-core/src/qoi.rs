//! Freestanding `#![no_std]` Segmented QOI Block Decoder for PSX Texture Streaming.
//!
//! Decodes 64x16 MCU rows on-the-fly from 1024x1024 segmented QOI blocks resident
//! in PSX main RAM, emitting 16-bit BGR555 texels for direct VRAM upload.
//!
//! # Why segmented QOI rather than plain QOI
//!
//! The QOI format is a single linear stream with no random-access points, so a
//! whole-image QOI cannot be sliced into individual MCU rows without decoding
//! everything before it. The VRAM streamer needs O(1) access to any 64x16 row,
//! so `tools/track_cook/qoi_codec.py` cuts the image into 1024 independently
//! decodable 64x16 segments and indexes them with a `u32` offset table. Each
//! segment restarts from the canonical QOI initial state (index array zeroed,
//! previous pixel `{0, 0, 0, 255}`), which makes the decoder stateless per
//! row and roughly an order of magnitude cheaper than the JPEG path's
//! Huffman + IDCT work.
//!
//! Segment order matches the JPEG restart-interval order exactly, so the two
//! codecs are drop-in interchangeable at the tile level:
//!
//! ```text
//! interval_idx = (tile_y * 4 + row) * 16 + tile_x
//! ```
//!
//! Key characteristics:
//! - Strictly zero heap allocation (`no_std` compatible).
//! - 64-entry colour index, integer wraparound arithmetic only.
//! - 16x16 Bayer ordered dither on the 8-bit to 5-bit reduction, matching the
//!   JPEG path's [`crate::jpeg`] output for equivalent colour.
//! - Zero CD-ROM access during decompression: RAM -> VRAM only.

/// Width and height of a decoded 64x64 tile in texels.
pub const TILE_TEXELS: usize = 64;
/// Raw texel count per 64x64 tile.
pub const TILE_PIXELS: usize = TILE_TEXELS * TILE_TEXELS;
/// Number of 64x16 MCU rows per 64x64 tile.
pub const ROWS_PER_TILE: usize = 4;
/// 64x16 segments along each axis of a 1024x1024 block.
pub const SEGMENTS_PER_AXIS: usize = 16;
/// Number of independently decodable segments in one 1024x1024 block.
pub const SEGMENT_COUNT: usize = SEGMENTS_PER_AXIS * SEGMENTS_PER_AXIS * ROWS_PER_TILE; // 1024
/// Segment width in texels (4 MCUs).
pub const SEGMENT_WIDTH: usize = 64;
/// Segment height in texels (1 MCU).
pub const SEGMENT_HEIGHT: usize = 16;
/// Pixels decoded per segment.
pub const SEGMENT_PIXELS: usize = SEGMENT_WIDTH * SEGMENT_HEIGHT;
/// Fixed container header size, chosen so the `u32` segment table is aligned.
pub const HEADER_BYTES: usize = 24;
/// Byte size of the `u32` segment offset table following the header.
pub const TABLE_BYTES: usize = SEGMENT_COUNT * 4;

/// Worst-case segment payload: 5 bytes per pixel plus the 8-byte end marker.
pub const SEGMENT_MAX_BYTES: usize = SEGMENT_PIXELS * 5 + 8;

/// Container magic, `b"ARQO"`.
pub const MAGIC: [u8; 4] = *b"ARQO";
/// Container format version understood by this decoder.
pub const VERSION: u8 = 1;

// QOI opcodes.
const OP_INDEX: u8 = 0x00;
const OP_DIFF: u8 = 0x40;
const OP_LUMA: u8 = 0x80;
const OP_RUN: u8 = 0xC0;
const OP_RGB: u8 = 0xFE;
const OP_RGBA: u8 = 0xFF;
const MASK_2: u8 = 0xC0;

/// 4x4 Bayer ordered dither matrix, matching [`crate::jpeg`].
const BAYER4X4: [[i32; 4]; 4] = [[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]];

/// Precomputed 16x16 Bayer table collapsed from [`BAYER4X4`].
///
/// Invariant: both `base_py = row * 16` and `base_px = mcu_x * 16` are multiples
/// of 4, so `out_y % 4 == py % 4` and `out_x % 4 == px % 4`. Indexing directly
/// avoids runtime modulo arithmetic in the inner loop.
const BAYER16: [[i32; 16]; 16] = build_bayer16();

const fn build_bayer16() -> [[i32; 16]; 16] {
    let mut table = [[0i32; 16]; 16];
    let mut y = 0;
    while y < 16 {
        let mut x = 0;
        while x < 16 {
            table[y][x] = BAYER4X4[y % 4][x % 4];
            x += 1;
        }
        y += 1;
    }
    table
}

/// Parsed metadata for a segmented 1024x1024 QOI block.
#[derive(Copy, Clone)]
pub struct QoiHeader {
    pub width: u16,
    pub height: u16,
    pub channels: u8,
    pub segment_count: u16,
    pub segment_width: u16,
    pub segment_height: u16,
}

impl Default for QoiHeader {
    fn default() -> Self {
        Self {
            width: 0,
            height: 0,
            channels: 3,
            segment_count: SEGMENT_COUNT as u16,
            segment_width: SEGMENT_WIDTH as u16,
            segment_height: SEGMENT_HEIGHT as u16,
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

impl QoiHeader {
    /// Parses the fixed container header from a segmented QOI block.
    ///
    /// Returns `None` for a block that is not this container format, or whose
    /// geometry is not the one the VRAM streamer addresses. Being strict here
    /// is deliberate: a mis-sized block would otherwise be decoded into the
    /// wrong tiles rather than rejected.
    pub fn parse(data: &[u8]) -> Option<Self> {
        if data.len() < HEADER_BYTES + TABLE_BYTES {
            return None;
        }
        if data[0..4] != MAGIC || data[4] != VERSION {
            return None;
        }

        let channels = data[7];
        if channels != 3 && channels != 4 {
            return None;
        }

        let width = be_u32(data, 8);
        let height = be_u32(data, 12);
        let segment_count = u16::from_be_bytes([data[16], data[17]]) as usize;
        let segment_width = u16::from_be_bytes([data[18], data[19]]) as usize;
        let segment_height = u16::from_be_bytes([data[20], data[21]]) as usize;

        if width != (SEGMENTS_PER_AXIS * SEGMENT_WIDTH) as u32
            || height != (SEGMENTS_PER_AXIS * ROWS_PER_TILE * SEGMENT_HEIGHT) as u32
            || segment_count != SEGMENT_COUNT
            || segment_width != SEGMENT_WIDTH
            || segment_height != SEGMENT_HEIGHT
        {
            return None;
        }

        Some(Self {
            width: width as u16,
            height: height as u16,
            channels,
            segment_count: segment_count as u16,
            segment_width: segment_width as u16,
            segment_height: segment_height as u16,
        })
    }
}

/// Byte offset of the first segment payload, i.e. the end of the header + table.
pub const PAYLOAD_BASE: usize = HEADER_BYTES + TABLE_BYTES;

/// Reads the segment offset table, validating that every segment starts inside
/// the block and after the table itself.
///
/// This mirrors [`crate::jpeg::JpegHeader::index_restarts`]: it is a linear walk
/// over the block performed once per CD load, profiled separately from the read
/// because it is CPU work rather than drive latency. A malformed offset is
/// clamped rather than rejected, because the decoder already bounds its reads
/// by the block length and a clamped segment still produces defined output.
#[inline(never)]
pub fn index_segments(data: &[u8], header: &QoiHeader, offsets: &mut [u32; SEGMENT_COUNT]) {
    let limit = data.len();

    for (i, slot) in offsets
        .iter_mut()
        .enumerate()
        .take(header.segment_count as usize)
    {
        let at = HEADER_BYTES + i * 4;
        let start = if at + 4 <= limit {
            PAYLOAD_BASE.saturating_add(be_u32(data, at) as usize)
        } else {
            PAYLOAD_BASE
        };

        *slot = start.min(limit) as u32;
    }
}

/// QOI's colour index hash. Must match the encoder's exactly.
#[inline(always)]
fn color_hash(r: u8, g: u8, b: u8, a: u8) -> usize {
    ((r as usize * 3) + (g as usize * 5) + (b as usize * 7) + (a as usize * 11)) & 63
}

/// Decodes one 64x16 MCU row (`row` 0..4) within a 64x64 tile from a segmented
/// QOI block.
///
/// Mirrors [`crate::jpeg::decode_tile_row_64x16`] exactly in shape, so the two
/// codecs are interchangeable call sites. Emits 1024 BGR555 texels directly
/// into `out_bgr555` at scanlines `row * 16 .. row * 16 + 16`.
#[inline(never)]
#[allow(clippy::needless_range_loop)]
pub fn decode_tile_row_64x16(
    block_data: &[u8],
    header: &QoiHeader,
    segment_offsets: &[u32; SEGMENT_COUNT],
    tile_x: usize,
    tile_y: usize,
    row: usize,
    out_bgr555: &mut [u16; TILE_PIXELS],
) {
    if tile_x >= SEGMENTS_PER_AXIS || tile_y >= SEGMENTS_PER_AXIS || row >= ROWS_PER_TILE {
        return;
    }

    let segment_idx = (tile_y * ROWS_PER_TILE + row) * SEGMENTS_PER_AXIS + tile_x;
    if segment_idx >= SEGMENT_COUNT {
        return;
    }

    // QOI's 64-entry colour index, zero-initialised. This is the only state a
    // segment needs, which is what makes random access possible.
    let mut index = [[0u8; 4]; 64];

    let mut px_r: u8 = 0;
    let mut px_g: u8 = 0;
    let mut px_b: u8 = 0;
    let px_a: u8 = 255;

    let channels = header.channels;
    let has_alpha = channels == 4;

    let mut pos = segment_offsets[segment_idx] as usize;
    let limit = block_data.len();
    let base_py = row * SEGMENT_HEIGHT;

    // Decode into a local RGB row buffer first, then convert to BGR555. QOI's
    // output is already RGB, so there is no colour transform, only the 8->5 bit
    // ordered dither the GPU needs.
    let mut rgb = [0u8; SEGMENT_PIXELS * 3];
    let mut produced = 0usize;

    while produced < SEGMENT_PIXELS {
        if pos >= limit {
            break;
        }

        let b1 = block_data[pos];
        pos += 1;

        if b1 == OP_RGB {
            if pos + 3 > limit {
                break;
            }
            px_r = block_data[pos];
            px_g = block_data[pos + 1];
            px_b = block_data[pos + 2];
            pos += 3;
        } else if b1 == OP_RGBA {
            if pos + 4 > limit {
                break;
            }
            px_r = block_data[pos];
            px_g = block_data[pos + 1];
            px_b = block_data[pos + 2];
            // Alpha is not written out: the streamer only ever emits opaque
            // 15bpp direct colour.
            pos += 4;
        } else if (b1 & MASK_2) == OP_INDEX {
            let e = index[(b1 & 0x3F) as usize];
            px_r = e[0];
            px_g = e[1];
            px_b = e[2];
        } else if (b1 & MASK_2) == OP_DIFF {
            px_r = px_r.wrapping_add((((b1 >> 4) & 0x03) as i32 - 2) as u8);
            px_g = px_g.wrapping_add((((b1 >> 2) & 0x03) as i32 - 2) as u8);
            px_b = px_b.wrapping_add((b1 & 0x03).wrapping_sub(2));
        } else if (b1 & MASK_2) == OP_LUMA {
            if pos >= limit {
                break;
            }
            let b2 = block_data[pos];
            pos += 1;
            let vg = ((b1 & 0x3F) as i32 - 32) as u8;
            px_r = px_r.wrapping_add(vg.wrapping_sub(8).wrapping_add((b2 >> 4) & 0x0F));
            px_g = px_g.wrapping_add(vg);
            px_b = px_b.wrapping_add(vg.wrapping_sub(8).wrapping_add(b2 & 0x0F));
        } else if (b1 & MASK_2) == OP_RUN {
            // A run byte with payload N repeats the previous pixel N+1 times
            // in total: the chunk's own pixel plus N repeats. Runs do not touch
            // the index, because this colour was recorded when first decoded.
            let run = (b1 & 0x3F) as usize + 1;
            for _ in 0..run {
                if produced >= SEGMENT_PIXELS {
                    break;
                }
                let o = produced * 3;
                rgb[o] = px_r;
                rgb[o + 1] = px_g;
                rgb[o + 2] = px_b;
                produced += 1;
            }
            continue;
        } else {
            // No other 2-bit tag exists (0xFE/0xFF handled above).
            break;
        }

        let slot = &mut index[color_hash(px_r, px_g, px_b, if has_alpha { px_a } else { 255 })];
        slot[0] = px_r;
        slot[1] = px_g;
        slot[2] = px_b;

        let o = produced * 3;
        rgb[o] = px_r;
        rgb[o + 1] = px_g;
        rgb[o + 2] = px_b;
        produced += 1;
    }

    // 8-bit RGB -> 15bpp BGR555 with the same 16x16 Bayer dither the JPEG path
    // uses, so equivalent colour lands on the same texel values.
    //
    // The table is 16 wide, and a segment is 64, so the x index wraps at 16.
    // That matches the JPEG path, whose `px` is the position within a 16-wide
    // MCU: `out_x % 16 == px % 16` because both bases are multiples of 16.
    #[allow(clippy::needless_range_loop)]
    for py in 0..SEGMENT_HEIGHT {
        let out_y = base_py + py;
        let bayer_row = &BAYER16[py];
        for px in 0..SEGMENT_WIDTH {
            let src = (py * SEGMENT_WIDTH + px) * 3;
            let dither = bayer_row[px & 15] >> 1;
            let r = (rgb[src] as i32 + dither).clamp(0, 255) >> 3;
            let g = (rgb[src + 1] as i32 + dither).clamp(0, 255) >> 3;
            let b = (rgb[src + 2] as i32 + dither).clamp(0, 255) >> 3;
            let bgr555 = (b << 10) | (g << 5) | r;
            out_bgr555[out_y * TILE_TEXELS + px] = bgr555 as u16;
        }
    }
}
