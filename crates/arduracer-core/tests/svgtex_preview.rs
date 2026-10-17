//! Visual preview for the SVG texture path.
//!
//! Ignored by default because it writes files. Run it to dump the decoded
//! texture as a PPM so the detail pass can be eyeballed against the flat
//! base raster and the old blocky artwork:
//!
//! ```sh
//! cargo test --manifest-path crates/arduracer-core/Cargo.toml \
//!     --release --test svgtex_preview -- --ignored --nocapture
//! ```
//!
//! Detail textures are a judgement call that no assertion can settle, so this
//! exists to make that judgement reviewable instead of theoretical.

use arduracer_core::qoi::{self, QoiHeader};
use arduracer_core::svgtex::{code_at, parse_block, SvgHeader};

const W: usize = 1024;
const H: usize = 1024;

fn write_ppm(path: &str, img: &[[u8; 3]]) {
    let mut out = format!("P6\n{W} {H}\n255\n").into_bytes();
    for px in img {
        out.extend_from_slice(px);
    }
    std::fs::write(path, out).expect("write ppm");
}

/// Expands 15bpp BGR555 back to 8-bit RGB for viewing.
fn to_rgb(v: u16) -> [u8; 3] {
    let r = (v & 31) as u8;
    let g = ((v >> 5) & 31) as u8;
    let b = ((v >> 10) & 31) as u8;
    [r * 8, g * 8, b * 8]
}

#[test]
#[ignore = "writes preview PPMs to /tmp"]
fn dump_decoded_previews() {
    let Ok(data) = std::fs::read("tests/data/svg_track.arsv") else {
        eprintln!("fixture missing; skipping preview");
        return;
    };
    let block = parse_block(&data).expect("parse");
    let h: SvgHeader = block.header;

    let raster = &data[h.raster_offset()..h.raster_offset() + h.raster_bytes as usize];
    let qoi_header = QoiHeader::parse(raster).expect("inner qoi");
    let codes = &data[h.code_offset()..h.code_offset() + h.code_bytes as usize];

    let mut detailed = vec![[0u8; 3]; W * H];
    let mut flat = vec![[0u8; 3]; W * H];
    let mut scratch = [0u16; qoi::TILE_PIXELS];
    let mut rgb = [0u8; qoi::SEGMENT_PIXELS * 3];

    const BAYER: [[i32; 4]; 4] = [[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]];
    for ty in 0..16 {
        for tx in 0..16 {
            for row in 0..4 {
                assert!(arduracer_core::svgtex::decode_tile_row_64x16(
                    &data,
                    &block,
                    tx,
                    ty,
                    row,
                    &mut scratch
                ));
                let seg = (ty * 4 + row) * 16 + tx;
                qoi::decode_segment_rgb(raster, &qoi_header, &block.offsets, seg, &mut rgb);
                for py in 0..16 {
                    for px in 0..64 {
                        let gx = tx * 64 + px;
                        let gy = ty * 64 + row * 16 + py;
                        detailed[gy * W + gx] = to_rgb(scratch[(row * 16 + py) * 64 + px]);
                        let s = (py * 64 + px) * 3;
                        let d = BAYER[gy & 3][gx & 3] >> 1;
                        let r = (rgb[s] as i32 + d).clamp(0, 255) >> 3;
                        let g = (rgb[s + 1] as i32 + d).clamp(0, 255) >> 3;
                        let b = (rgb[s + 2] as i32 + d).clamp(0, 255) >> 3;
                        flat[gy * W + gx] = [(r * 8) as u8, (g * 8) as u8, (b * 8) as u8];
                    }
                }
            }
        }
    }

    write_ppm("/tmp/svgtex_detailed.ppm", &detailed);
    write_ppm("/tmp/svgtex_flat.ppm", &flat);
    let _ = code_at(codes, 0);
    eprintln!("wrote /tmp/svgtex_detailed.ppm and /tmp/svgtex_flat.ppm");
}
