//! Host tests for the segmented QOI decoder in [`arduracer_core::qoi`].
//!
//! The fixtures under `tests/data/` are produced by
//! `tools/track_cook/qoi_codec.py`, so these tests check the Rust decoder
//! against an encoder that was written independently from the spec. A shared
//! bug in one implementation that the other happens to agree with is the
//! failure mode this is most useful for catching.

use arduracer_core::qoi::{
    decode_tile_row_64x16, index_segments, QoiHeader, HEADER_BYTES, PAYLOAD_BASE,
    SEGMENTS_PER_AXIS, SEGMENT_COUNT, SEGMENT_HEIGHT, SEGMENT_PIXELS, TILE_PIXELS, TILE_TEXELS,
};

const WIDTH: usize = 1024;
const HEIGHT: usize = 1024;

fn load(name: &str) -> Vec<u8> {
    let path = format!("tests/data/qoi_{name}.qoi");
    std::fs::read(&path).unwrap_or_else(|e| panic!("missing fixture {path}: {e}"))
}

/// Reference decode of one segment, written straight from the QOI spec.
fn reference_segment(block: &[u8], start: usize) -> Vec<[u8; 3]> {
    let mut index = [[0u8, 0, 0, 0]; 64];
    let (mut r, mut g, mut b) = (0u8, 0u8, 0u8);
    let mut out = Vec::with_capacity(SEGMENT_PIXELS);
    let mut pos = start;
    let mut run = 0usize;

    for _ in 0..SEGMENT_PIXELS {
        if run > 0 {
            run -= 1;
        } else {
            let b1 = block[pos];
            pos += 1;
            match b1 {
                0xFE => {
                    r = block[pos];
                    g = block[pos + 1];
                    b = block[pos + 2];
                    pos += 3;
                }
                0xFF => {
                    r = block[pos];
                    g = block[pos + 1];
                    b = block[pos + 2];
                    pos += 4;
                }
                v if v & 0xC0 == 0x00 => {
                    let e = index[(v & 0x3F) as usize];
                    r = e[0];
                    g = e[1];
                    b = e[2];
                }
                v if v & 0xC0 == 0x40 => {
                    r = r.wrapping_add(((v >> 4) & 0x03).wrapping_sub(2));
                    g = g.wrapping_add(((v >> 2) & 0x03).wrapping_sub(2));
                    b = b.wrapping_add((v & 0x03).wrapping_sub(2));
                }
                v if v & 0xC0 == 0x80 => {
                    let b2 = block[pos];
                    pos += 1;
                    let vg = (v & 0x3F).wrapping_sub(32);
                    r = r.wrapping_add(vg.wrapping_sub(8).wrapping_add((b2 >> 4) & 0x0F));
                    g = g.wrapping_add(vg);
                    b = b.wrapping_add(vg.wrapping_sub(8).wrapping_add(b2 & 0x0F));
                }
                v if v & 0xC0 == 0xC0 => run = (v & 0x3F) as usize,
                _ => panic!("bad opcode 0x{b1:02x}"),
            }
            let h = ((r as usize * 3) + (g as usize * 5) + (b as usize * 7) + (255 * 11)) & 63;
            index[h] = [r, g, b, 255];
        }
        out.push([r, g, b]);
    }
    out
}

const BAYER4X4: [[i32; 4]; 4] = [[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]];

fn bayer16(py: usize, px: usize) -> i32 {
    BAYER4X4[py % 4][px % 4]
}

/// Decodes every tile of a block through the production entry point.
fn decode_all(block: &[u8], header: &QoiHeader, offsets: &[u32; SEGMENT_COUNT]) -> Vec<u16> {
    let mut img = vec![0u16; WIDTH * HEIGHT];
    for tile_y in 0..SEGMENTS_PER_AXIS {
        for tile_x in 0..SEGMENTS_PER_AXIS {
            let mut scratch = [0u16; TILE_PIXELS];
            for row in 0..4 {
                decode_tile_row_64x16(block, header, offsets, tile_x, tile_y, row, &mut scratch);
            }
            for y in 0..TILE_TEXELS {
                for x in 0..TILE_TEXELS {
                    img[(tile_y * TILE_TEXELS + y) * WIDTH + tile_x * TILE_TEXELS + x] =
                        scratch[y * TILE_TEXELS + x];
                }
            }
        }
    }
    img
}

#[test]
fn parses_header_of_every_fixture() {
    for name in ["flat", "checker", "track"] {
        let block = load(name);
        let h = QoiHeader::parse(&block).unwrap_or_else(|| panic!("{name}: header did not parse"));
        assert_eq!(h.width as usize, WIDTH, "{name}");
        assert_eq!(h.height as usize, HEIGHT, "{name}");
        assert_eq!(h.channels, 3, "{name}");
        assert_eq!(h.segment_count as usize, SEGMENT_COUNT, "{name}");
        assert_eq!(h.segment_width as usize, 64, "{name}");
        assert_eq!(h.segment_height as usize, SEGMENT_HEIGHT, "{name}");
    }
}

#[test]
fn rejects_foreign_or_malformed_blocks() {
    // Wrong magic.
    let mut block = load("flat");
    block[0] = b'X';
    assert!(QoiHeader::parse(&block).is_none(), "bad magic accepted");

    // Wrong version.
    let mut block = load("flat");
    block[4] = 9;
    assert!(QoiHeader::parse(&block).is_none(), "bad version accepted");

    // Unsupported channel count.
    let mut block = load("flat");
    block[7] = 2;
    assert!(QoiHeader::parse(&block).is_none(), "bad channels accepted");

    // Truncated below header + table.
    let block = load("flat");
    assert!(QoiHeader::parse(&block[..HEADER_BYTES + SEGMENT_COUNT * 4 - 1]).is_none());
    assert!(QoiHeader::parse(&block[..4]).is_none());
}

#[test]
fn segment_table_points_inside_the_block() {
    for name in ["flat", "checker", "track"] {
        let block = load(name);
        let header = QoiHeader::parse(&block).expect("header");
        let mut offsets = [0u32; SEGMENT_COUNT];
        index_segments(&block, &header, &mut offsets);

        for (i, &off) in offsets.iter().enumerate() {
            let start = off as usize;
            assert!(
                start >= PAYLOAD_BASE,
                "{name}: segment {i} starts inside the table at {start}"
            );
            assert!(
                start < block.len(),
                "{name}: segment {i} starts past the block end ({start} >= {})",
                block.len()
            );
        }

        // Offsets are non-decreasing, so segments cannot overlap.
        for i in 1..SEGMENT_COUNT {
            assert!(
                offsets[i] >= offsets[i - 1],
                "{name}: segment table is not monotonic at {i}"
            );
        }
    }
}

#[test]
fn out_of_range_tile_arguments_are_ignored() {
    let block = load("track");
    let header = QoiHeader::parse(&block).expect("header");
    let mut offsets = [0u32; SEGMENT_COUNT];
    index_segments(&block, &header, &mut offsets);

    for (tx, ty, row) in [
        (SEGMENTS_PER_AXIS, 0, 0),
        (0, SEGMENTS_PER_AXIS, 0),
        (0, 0, 4),
        (usize::MAX, 0, 0),
    ] {
        let mut scratch = [0u16; TILE_PIXELS];
        decode_tile_row_64x16(&block, &header, &offsets, tx, ty, row, &mut scratch);
        assert!(
            scratch.iter().all(|&v| v == 0),
            "({tx},{ty},{row}) wrote output despite being out of range"
        );
    }
}

/// The core cross-implementation check: the Rust decoder must reproduce a
/// from-the-spec reference decode of every segment, exactly.
#[test]
fn matches_spec_reference_decode() {
    for name in ["flat", "checker", "track"] {
        let block = load(name);
        let header = QoiHeader::parse(&block).expect("header");
        let mut offsets = [0u32; SEGMENT_COUNT];
        index_segments(&block, &header, &mut offsets);

        for seg_idx in 0..SEGMENT_COUNT {
            let seg_x = seg_idx % SEGMENTS_PER_AXIS;
            let seg_y = (seg_idx / SEGMENTS_PER_AXIS) % (HEIGHT / SEGMENT_HEIGHT);
            let expected = reference_segment(&block, offsets[seg_idx] as usize);

            // Map the segment onto its 64x16 tile region and compare texels.
            let tile_x = seg_x;
            let tile_y = seg_y / 4;
            let row = seg_y % 4;
            let mut scratch = [0u16; TILE_PIXELS];
            decode_tile_row_64x16(&block, &header, &offsets, tile_x, tile_y, row, &mut scratch);

            for y in 0..SEGMENT_HEIGHT {
                for x in 0..64 {
                    let d = bayer16(y, x) >> 1;
                    let src = expected[y * 64 + x];
                    let r = (src[0] as i32 + d).clamp(0, 255) >> 3;
                    let g = (src[1] as i32 + d).clamp(0, 255) >> 3;
                    let b = (src[2] as i32 + d).clamp(0, 255) >> 3;
                    let want = ((b << 10) | (g << 5) | r) as u16;
                    let got = scratch[(row * 16 + y) * TILE_TEXELS + x];
                    assert_eq!(
                        got, want,
                        "{name}: segment {seg_idx} texel ({x},{y}) = {got:#06x}, want {want:#06x}"
                    );
                }
            }
        }
    }
}

/// Pins the decode of a real circuit's art. The digest was produced by the
/// Python encoder path, so this catches any drift in the production decoder
/// against shipped-looking content rather than only synthetic patterns.
#[test]
fn track_art_decodes_to_expected_image() {
    let block = load("track");
    let header = QoiHeader::parse(&block).expect("header");
    let mut offsets = [0u32; SEGMENT_COUNT];
    index_segments(&block, &header, &mut offsets);

    let img = decode_all(&block, &header, &offsets);

    // FNV-1a over the little-endian texel words.
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &v in &img {
        for byte in v.to_le_bytes() {
            hash ^= byte as u64;
            hash = hash.wrapping_mul(0x100_0000_01b3);
        }
    }
    assert_eq!(
        hash, 0x8a3b_5feb_e464_af15,
        "decoded circuit art no longer matches the pinned digest"
    );
}
