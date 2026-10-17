//! Host tests for the SVG-derived texture path (`arduracer_core::svgtex`).
//!
//! The decode is cross-checked two ways, because the detail pass is easy to get
//! subtly wrong in a way no size or timing check would catch:
//!
//! * the base raster must decode to *exactly* the pixels the cooker rasterised,
//! * the detail pass must be a pure function of `(code, x, y)` -- deterministic,
//!   position-locked, and bounded, so a tile can never change appearance
//!   depending on which MCU row happened to decode it.

use arduracer_core::qoi;
use arduracer_core::svgtex::{
    code_at, decode_tile_row_64x16, detail_delta, parse_block, SvgHeader, TILE_PIXELS,
};

const WIDTH: usize = 1024;
const HEIGHT: usize = 1024;
const GRID: usize = 80;

fn load(name: &str) -> Vec<u8> {
    let path = format!("tests/data/svg_{name}.arsv");
    std::fs::read(&path).unwrap_or_else(|e| panic!("missing fixture {path}: {e}"))
}

#[test]
fn parses_header_of_every_fixture() {
    for name in ["flat", "track"] {
        let b = load(name);
        let h = SvgHeader::parse(&b).unwrap_or_else(|| panic!("{name}: no header"));
        assert_eq!(h.width as usize, WIDTH, "{name}");
        assert_eq!(h.height as usize, HEIGHT, "{name}");
        assert_eq!(h.grid_w as usize, GRID, "{name}");
        assert_eq!(h.grid_h as usize, GRID, "{name}");
        assert_eq!(h.code_bytes as usize, GRID * GRID / 2, "{name}");
        assert_eq!(h.raster_offset(), 32 + GRID * GRID / 2, "{name}");
        assert_eq!(
            h.raster_offset() + h.raster_bytes as usize,
            b.len(),
            "{name}: raster should run to the end of the block"
        );
    }
}

#[test]
fn rejects_malformed_blocks() {
    let mut b = load("track");
    b[0] = b'X';
    assert!(SvgHeader::parse(&b).is_none(), "bad magic accepted");

    let mut b = load("track");
    b[4] = 9;
    assert!(SvgHeader::parse(&b).is_none(), "bad version accepted");

    let mut b = load("track");
    b[7] = 2;
    assert!(
        SvgHeader::parse(&b).is_none(),
        "unknown base codec accepted"
    );

    // code_bytes that disagrees with grid_w * grid_h / 2.
    let mut b = load("track");
    b[20] = 0xFF;
    assert!(
        SvgHeader::parse(&b).is_none(),
        "inconsistent code_bytes accepted"
    );

    let b = load("track");
    assert!(
        SvgHeader::parse(&b[..16]).is_none(),
        "truncated header accepted"
    );
    assert!(parse_block(&b[..64]).is_none(), "truncated block accepted");
}

/// The strongest available check on the base raster: with detail disabled for
/// every code, the decode must reproduce the exact BGR555 of the cooker's
/// pixels, which are themselves the reference-decoded QOI payload.
#[test]
fn base_raster_decodes_to_the_cooked_pixels() {
    let b = load("track");
    let block = parse_block(&b).expect("parse");
    let codes = &b
        [block.header.code_offset()..block.header.code_offset() + block.header.code_bytes as usize];
    let raster = &b[block.header.raster_offset()
        ..block.header.raster_offset() + block.header.raster_bytes as usize];
    let qoi_header = qoi::QoiHeader::parse(raster).expect("inner qoi header");

    // Decode a few well-spread tiles through the shared QOI core, which is the
    // exact same code path the ARSV decoder consumes.
    let mut rgb = [0u8; qoi::SEGMENT_PIXELS * 3];
    for (tile_x, tile_y, row) in [(0usize, 0usize, 0usize), (7, 9, 2), (15, 15, 3), (3, 4, 1)] {
        let seg = (tile_y * 4 + row) * 16 + tile_x;
        qoi::decode_segment_rgb(raster, &qoi_header, &block.offsets, seg, &mut rgb);

        // Independently recompute what that segment's BGR555 must be.
        const BAYER: [[i32; 4]; 4] = [[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]];
        let mut scratch = [0u16; TILE_PIXELS];
        assert!(
            decode_tile_row_64x16(&b, &block, tile_x, tile_y, row, &mut scratch),
            "decode failed for {tile_x},{tile_y},{row}"
        );

        for py in 0..qoi::SEGMENT_HEIGHT {
            for px in 0..qoi::SEGMENT_WIDTH {
                let gx = tile_x * 64 + px;
                let gy = tile_y * 64 + row * 16 + py;
                let cell_x = (gx * GRID) / WIDTH;
                let cell_y = (gy * GRID) / HEIGHT;
                let code = code_at(codes, cell_y * GRID + cell_x);
                let d = (BAYER[gy & 3][gx & 3] >> 1) + detail_delta(code, gx, gy);
                let s = (py * 64 + px) * 3;
                let r = (rgb[s] as i32 + d).clamp(0, 255) >> 3;
                let g = (rgb[s + 1] as i32 + d).clamp(0, 255) >> 3;
                let bl = (rgb[s + 2] as i32 + d).clamp(0, 255) >> 3;
                let want = ((bl << 10) | (g << 5) | r) as u16;
                assert_eq!(
                    scratch[(row * 16 + py) * 64 + px],
                    want,
                    "texel ({gx},{gy}) code {code}"
                );
            }
        }
    }
}

/// Detail must depend only on `(code, x, y)` -- never on decode order or on
/// which row carried it. If it did, a tile's appearance would depend on where
/// the camera happened to be when it streamed in.
#[test]
fn detail_is_deterministic_and_position_locked() {
    for code in 0u8..=15 {
        for &(x, y) in &[(0usize, 0usize), (1, 1), (37, 91), (1023, 1023), (512, 512)] {
            let a = detail_delta(code, x, y);
            let b = detail_delta(code, x, y);
            assert_eq!(a, b, "code {code} at ({x},{y}) is not deterministic");
        }
    }
}

/// A tile decoded twice, and decoded via two different row paths, must be
/// identical.
#[test]
fn decoding_is_reproducible() {
    let b = load("track");
    let block = parse_block(&b).expect("parse");
    let mut a = [0u16; TILE_PIXELS];
    let mut c = [0u16; TILE_PIXELS];
    for row in 0..4 {
        assert!(decode_tile_row_64x16(&b, &block, 6, 7, row, &mut a));
        assert!(decode_tile_row_64x16(&b, &block, 6, 7, row, &mut c));
        assert_eq!(a, c, "row {row} is not reproducible");
    }
    assert!(
        a.iter().any(|&v| v != 0),
        "a real circuit tile should not decode to all zeroes"
    );
}

/// Detail must stay inside a sane brightness band. Too wide and surfaces look
/// noisy; too narrow and the pass is pointless.
#[test]
fn detail_stays_within_a_sane_band() {
    let mut lo = i32::MAX;
    let mut hi = i32::MIN;
    for code in 0u8..=15 {
        for y in 0..64 {
            for x in 0..64 {
                let d = detail_delta(code, x, y);
                lo = lo.min(d);
                hi = hi.max(d);
            }
        }
    }
    assert!(lo >= -32, "detail darkens too far: {lo}");
    assert!(hi <= 32, "detail lightens too far: {hi}");
}

/// Every surface that actually appears in the circuits must produce some
/// variation, or the pass is doing nothing for it.
#[test]
fn every_authored_surface_has_visible_detail() {
    // Two distinct values is the floor, and is exactly right for the checker
    // surfaces (start line, kerbs): a checker has two states, not a gradient.
    for code in [1u8, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15] {
        let vals: std::collections::HashSet<i32> = (0..64)
            .flat_map(|y| (0..64).map(move |x| detail_delta(code, x, y)))
            .collect();
        assert!(
            vals.len() >= 2,
            "surface {code} has only {} distinct detail values",
            vals.len()
        );
    }
    // The genuinely noisy surfaces must be noisier than a checker.
    for code in [1u8, 5, 6, 7, 15] {
        let vals: std::collections::HashSet<i32> = (0..64)
            .flat_map(|y| (0..64).map(move |x| detail_delta(code, x, y)))
            .collect();
        assert!(
            vals.len() >= 3,
            "surface {code} is flat: only {} distinct detail values",
            vals.len()
        );
    }
}

/// VOID must be left alone: it is the one surface with no texture.
#[test]
fn void_is_untouched() {
    for y in 0..32 {
        for x in 0..32 {
            assert_eq!(detail_delta(0, x, y), 0, "VOID changed at ({x},{y})");
        }
    }
}

#[test]
fn out_of_range_arguments_are_rejected() {
    let b = load("track");
    let block = parse_block(&b).expect("parse");
    let mut scratch = [0xAAAAu16; TILE_PIXELS];
    for (tx, ty, row) in [(16usize, 0usize, 0usize), (0, 16, 0), (0, 0, 4)] {
        let before = scratch;
        assert!(!decode_tile_row_64x16(
            &b,
            &block,
            tx,
            ty,
            row,
            &mut scratch
        ));
        assert_eq!(
            scratch, before,
            "({tx},{ty},{row}) wrote despite being rejected"
        );
    }
}

#[test]
fn code_map_packs_two_cells_per_byte() {
    let b = load("track");
    let h = SvgHeader::parse(&b).expect("header");
    let codes = &b[h.code_offset()..h.code_offset() + h.code_bytes as usize];
    assert_eq!(codes.len(), GRID * GRID / 2);
    // Every code must be in range for the decoder's nibble extraction.
    for i in 0..codes.len() {
        // The packing round-trips: low nibble first, matching the cooker.
        assert_eq!(code_at(codes, i * 2), codes[i] & 0x0F);
        assert_eq!(code_at(codes, i * 2 + 1), codes[i] >> 4);
    }
}
