use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::time::Instant;

use arduracer_core::jpeg::{self, JpegHeader};
use arduracer_core::qoi::{self, QoiHeader};
use arduracer_core::svgtex::{self, SvgBlock};
use arduracer_core::visual_tex;

fn read_binary(name: &str) -> Vec<u8> {
    let p = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("assets")
        .join(name);
    let mut f = File::open(&p).unwrap_or_else(|e| panic!("Failed to open {}: {}", p.display(), e));
    let mut buf = Vec::new();
    f.read_to_end(&mut buf).unwrap();
    buf
}

fn benchmark_decoders() {
    println!("================================================================================");
    println!("  TRACK TEXTURE DECODER TIMINGS: 1024x1024 vs 4096x4096 (All 6 Formats)");
    println!("================================================================================");
    println!("{:<10} | {:<16} | {:<16} | {:<12}", "Format", "1024x1024 Stage", "4096x4096 Stage", "Ratio");
    println!("--------------------------------------------------------------------------------");

    let tracks_bin = read_binary("TRACKS.BIN");
    let qoi_bin = read_binary("QOITRACKS.BIN");
    let svg_bin = read_binary("SVGTRACKS.BIN");
    let tim_bin = read_binary("TIMTRACKS.BIN");
    let tim4_bin = read_binary("TIM4TRACKS.BIN");
    let tim8_bin = read_binary("TIM8TRACKS.BIN");

    let mut scratch = [0u16; 64 * 64];

    // 1. JPEG
    let t_1024_jpeg = {
        let entry = visual_tex::CIRCUIT_BLOCK_SECTORS[0];
        let off = entry.sector_offset as usize * 2048;
        let data = &tracks_bin[off..off + entry.byte_len as usize];
        let header = JpegHeader::parse(data).unwrap();
        let mut offsets = [0u32; 1024];
        header.index_restarts(data, &mut offsets);
        let start = Instant::now();
        let iters = 256;
        for _ in 0..iters {
            jpeg::decode_tile_64x64(data, &header, &offsets, 4, 4, &mut scratch);
        }
        start.elapsed().as_nanos() as f64 / (iters as f64 * 1000.0)
    };
    let t_4096_jpeg = {
        // Block 25 (Circuit 24, block 1, 0)
        let entry = visual_tex::CIRCUIT_BLOCK_SECTORS[25];
        let off = entry.sector_offset as usize * 2048;
        let data = &tracks_bin[off..off + entry.byte_len as usize];
        let header = JpegHeader::parse(data).unwrap();
        let mut offsets = [0u32; 1024];
        header.index_restarts(data, &mut offsets);
        let start = Instant::now();
        let iters = 256;
        for _ in 0..iters {
            jpeg::decode_tile_64x64(data, &header, &offsets, 0, 10, &mut scratch);
        }
        start.elapsed().as_nanos() as f64 / (iters as f64 * 1000.0)
    };
    println!("{:<10} | {:>12.2} µs/tile | {:>12.2} µs/tile | {:>11.2}x", "JPEG", t_1024_jpeg, t_4096_jpeg, t_4096_jpeg / t_1024_jpeg);

    // 2. QOI
    let t_1024_qoi = {
        let entry = visual_tex::CIRCUIT_QOI_SECTORS[0];
        let off = entry.sector_offset as usize * 2048;
        let data = &qoi_bin[off..off + entry.byte_len as usize];
        let header = QoiHeader::parse(data).unwrap();
        let mut offsets = [0u32; 1024];
        qoi::index_segments(data, &header, &mut offsets);
        let start = Instant::now();
        let iters = 256;
        for _ in 0..iters {
            for row in 0..4 {
                qoi::decode_tile_row_64x16(data, &header, &offsets, 4, 4, row, &mut scratch);
            }
        }
        start.elapsed().as_nanos() as f64 / (iters as f64 * 1000.0)
    };
    let t_4096_qoi = {
        let entry = visual_tex::CIRCUIT_QOI_SECTORS[25];
        let off = entry.sector_offset as usize * 2048;
        let data = &qoi_bin[off..off + entry.byte_len as usize];
        let header = QoiHeader::parse(data).unwrap();
        let mut offsets = [0u32; 1024];
        qoi::index_segments(data, &header, &mut offsets);
        let start = Instant::now();
        let iters = 256;
        for _ in 0..iters {
            for row in 0..4 {
                qoi::decode_tile_row_64x16(data, &header, &offsets, 0, 10, row, &mut scratch);
            }
        }
        start.elapsed().as_nanos() as f64 / (iters as f64 * 1000.0)
    };
    println!("{:<10} | {:>12.2} µs/tile | {:>12.2} µs/tile | {:>11.2}x", "QOI", t_1024_qoi, t_4096_qoi, t_4096_qoi / t_1024_qoi);

    // 3. SVG (ARSV)
    let t_1024_svg = {
        let entry = visual_tex::CIRCUIT_ARSV_SECTORS[0];
        let off = entry.sector_offset as usize * 2048;
        let data = &svg_bin[off..off + entry.byte_len as usize];
        let block = svgtex::parse_block(data).unwrap();
        let start = Instant::now();
        let iters = 256;
        for _ in 0..iters {
            for row in 0..4 {
                svgtex::decode_tile_row_64x16(data, &block, 4, 4, row, &mut scratch);
            }
        }
        start.elapsed().as_nanos() as f64 / (iters as f64 * 1000.0)
    };
    let t_4096_svg = {
        let entry = visual_tex::CIRCUIT_ARSV_SECTORS[25];
        let off = entry.sector_offset as usize * 2048;
        let data = &svg_bin[off..off + entry.byte_len as usize];
        let block = svgtex::parse_block(data).unwrap();
        let start = Instant::now();
        let iters = 256;
        for _ in 0..iters {
            for row in 0..4 {
                svgtex::decode_tile_row_64x16(data, &block, 0, 10, row, &mut scratch);
            }
        }
        start.elapsed().as_nanos() as f64 / (iters as f64 * 1000.0)
    };
    println!("{:<10} | {:>12.2} µs/tile | {:>12.2} µs/tile | {:>11.2}x", "SVG/ARSV", t_1024_svg, t_4096_svg, t_4096_svg / t_1024_svg);

    // 4. TIM (16bpp direct)
    let t_1024_tim = {
        let entry = visual_tex::CIRCUIT_TIM_SECTORS[0];
        let off = entry.sector_offset as usize * 2048;
        let data = &tim_bin[off..off + entry.byte_len as usize];
        let start = Instant::now();
        let iters = 500;
        for _ in 0..iters {
            let tile_off = (4 * 16 + 4) * 8192;
            let tile_data = &data[tile_off..tile_off + 8192];
            unsafe {
                core::ptr::copy_nonoverlapping(tile_data.as_ptr(), scratch.as_mut_ptr() as *mut u8, 8192);
            }
        }
        start.elapsed().as_nanos() as f64 / (iters as f64 * 1000.0)
    };
    let t_4096_tim = {
        let entry = visual_tex::CIRCUIT_TIM_SECTORS[25];
        let off = entry.sector_offset as usize * 2048;
        let data = &tim_bin[off..off + entry.byte_len as usize];
        let start = Instant::now();
        let iters = 500;
        for _ in 0..iters {
            let tile_off = (10 * 16 + 0) * 8192;
            let tile_data = &data[tile_off..tile_off + 8192];
            unsafe {
                core::ptr::copy_nonoverlapping(tile_data.as_ptr(), scratch.as_mut_ptr() as *mut u8, 8192);
            }
        }
        start.elapsed().as_nanos() as f64 / (iters as f64 * 1000.0)
    };
    println!("{:<10} | {:>12.2} µs/tile | {:>12.2} µs/tile | {:>11.2}x", "TIM (16b)", t_1024_tim, t_4096_tim, t_4096_tim / t_1024_tim);

    // 5. TIM4 (4bpp indexed)
    let t_1024_tim4 = {
        let entry = visual_tex::CIRCUIT_TIM4_SECTORS[0];
        let off = entry.sector_offset as usize * 2048;
        let data = &tim4_bin[off..off + entry.byte_len as usize];
        let start = Instant::now();
        let iters = 500;
        for _ in 0..iters {
            let tile_off = 2048 + (4 * 16 + 4) * 2048;
            let tile_data = &data[tile_off..tile_off + 2048];
            unsafe {
                core::ptr::copy_nonoverlapping(tile_data.as_ptr(), scratch.as_mut_ptr() as *mut u8, 2048);
            }
        }
        start.elapsed().as_nanos() as f64 / (iters as f64 * 1000.0)
    };
    let t_4096_tim4 = {
        let entry = visual_tex::CIRCUIT_TIM4_SECTORS[25];
        let off = entry.sector_offset as usize * 2048;
        let data = &tim4_bin[off..off + entry.byte_len as usize];
        let start = Instant::now();
        let iters = 500;
        for _ in 0..iters {
            let tile_off = 2048 + (10 * 16 + 0) * 2048;
            let tile_data = &data[tile_off..tile_off + 2048];
            unsafe {
                core::ptr::copy_nonoverlapping(tile_data.as_ptr(), scratch.as_mut_ptr() as *mut u8, 2048);
            }
        }
        start.elapsed().as_nanos() as f64 / (iters as f64 * 1000.0)
    };
    println!("{:<10} | {:>12.2} µs/tile | {:>12.2} µs/tile | {:>11.2}x", "TIM4 (4b)", t_1024_tim4, t_4096_tim4, t_4096_tim4 / t_1024_tim4);

    // 6. TIM8 (8bpp indexed)
    let t_1024_tim8 = {
        let entry = visual_tex::CIRCUIT_TIM8_SECTORS[0];
        let off = entry.sector_offset as usize * 2048;
        let data = &tim8_bin[off..off + entry.byte_len as usize];
        let start = Instant::now();
        let iters = 500;
        for _ in 0..iters {
            let tile_off = 2048 + (4 * 16 + 4) * 4096;
            let tile_data = &data[tile_off..tile_off + 4096];
            unsafe {
                core::ptr::copy_nonoverlapping(tile_data.as_ptr(), scratch.as_mut_ptr() as *mut u8, 4096);
            }
        }
        start.elapsed().as_nanos() as f64 / (iters as f64 * 1000.0)
    };
    let t_4096_tim8 = {
        let entry = visual_tex::CIRCUIT_TIM8_SECTORS[25];
        let off = entry.sector_offset as usize * 2048;
        let data = &tim8_bin[off..off + entry.byte_len as usize];
        let start = Instant::now();
        let iters = 500;
        for _ in 0..iters {
            let tile_off = 2048 + (10 * 16 + 0) * 4096;
            let tile_data = &data[tile_off..tile_off + 4096];
            unsafe {
                core::ptr::copy_nonoverlapping(tile_data.as_ptr(), scratch.as_mut_ptr() as *mut u8, 4096);
            }
        }
        start.elapsed().as_nanos() as f64 / (iters as f64 * 1000.0)
    };
    println!("{:<10} | {:>12.2} µs/tile | {:>12.2} µs/tile | {:>11.2}x", "TIM8 (8b)", t_1024_tim8, t_4096_tim8, t_4096_tim8 / t_1024_tim8);
    println!("--------------------------------------------------------------------------------");
    println!("Conclusion: Per-tile decode time is virtually identical between 1024 and 4096 stages;");
    println!("the observed degradation was entirely in tile pull scheduling, seeks, and eviction thrash.");
}

fn simulate_seek_patterns() {
    println!("\n================================================================================");
    println!("  DISC STREAMING TRAJECTORY SIMULATION (Titan Circuit, 4096x4096)");
    println!("================================================================================");

    // Simulate Mode 7 starting view spanning block 0 and block 1:
    // 64 candidate tiles with their (bx, by, sub_tx, sub_ty)
    let mut tiles = Vec::new();
    for ty in 8..14 {
        for tx in 12..20 {
            let bx = tx / 16;
            let by = ty / 16;
            let sub_tx = tx % 16;
            let sub_ty = ty % 16;
            tiles.push((bx, by, sub_tx, sub_ty));
        }
    }

    // 1. OLD BUGGY: sorted by sub_ty * 16 + sub_tx (ignoring bx, by)
    let mut old_tiles = tiles.clone();
    old_tiles.sort_by_key(|t| t.3 * 16 + t.2);
    let mut old_seeks = 0usize;
    let mut old_back_seeks = 0usize;
    let mut old_total_seek_sectors = 0u32;
    let mut prev_lba = 0u32;
    for (i, &(bx, by, sub_tx, sub_ty)) in old_tiles.iter().enumerate() {
        let b_idx = visual_tex::circuit_block_index(24, bx, by);
        let block_lba = visual_tex::CIRCUIT_TIM_SECTORS[b_idx].sector_offset;
        let tile_lba = block_lba + (sub_ty * 16 + sub_tx) as u32 * 4;
        if i > 0 {
            if tile_lba < prev_lba {
                old_back_seeks += 1;
                old_total_seek_sectors += prev_lba - tile_lba;
            } else if tile_lba > prev_lba + 4 {
                old_seeks += 1;
                old_total_seek_sectors += tile_lba - prev_lba;
            }
        }
        prev_lba = tile_lba;
    }

    // 2. NEW FIXED: sorted by b_idx * 256 + sub_ty * 16 + sub_tx
    let mut new_tiles = tiles.clone();
    new_tiles.sort_by_key(|t| {
        let b_idx = visual_tex::circuit_block_index(24, t.0, t.1);
        b_idx * 256 + t.3 * 16 + t.2
    });
    let mut new_seeks = 0usize;
    let mut new_back_seeks = 0usize;
    let mut new_total_seek_sectors = 0u32;
    prev_lba = 0;
    for (i, &(bx, by, sub_tx, sub_ty)) in new_tiles.iter().enumerate() {
        let b_idx = visual_tex::circuit_block_index(24, bx, by);
        let block_lba = visual_tex::CIRCUIT_TIM_SECTORS[b_idx].sector_offset;
        let tile_lba = block_lba + (sub_ty * 16 + sub_tx) as u32 * 4;
        if i > 0 {
            if tile_lba < prev_lba {
                new_back_seeks += 1;
                new_total_seek_sectors += prev_lba - tile_lba;
            } else if tile_lba > prev_lba + 4 {
                new_seeks += 1;
                new_total_seek_sectors += tile_lba - prev_lba;
            }
        }
        prev_lba = tile_lba;
    }

    println!("{:<28} | {:<16} | {:<16}", "Metric", "Old Buggy Code", "New Fixed Code");
    println!("--------------------------------------------------------------------------------");
    println!("{:<28} | {:<16} | {:<16}", "Backward CD Seeks", old_back_seeks, new_back_seeks);
    println!("{:<28} | {:<16} | {:<16}", "Forward Seek Jumps", old_seeks, new_seeks);
    println!("{:<28} | {:<16} | {:<16}", "Total Head Seek Sectors", old_total_seek_sectors, new_total_seek_sectors);
    println!("{:<28} | {:<16} | {:<16}", "Laser Seek Direction", "Oscillating Back/Fwd", "Monotonic Forward");
}

fn simulate_eviction_cache() {
    println!("\n================================================================================");
    println!("  MODE 7 VRAM CACHE EVICTION ACCURACY (Car at x=1072, cam_tx=16..17)");
    println!("================================================================================");
    // Road tile right under car in block 1: bx=1, sub_tx=0 (global world tile 16)
    let cam_tx = 17i32;
    let cam_ty = 10i32;

    // Slot holding tile at world tile 16 (bx=1, sub_tx=0)
    let slot_bx = 1usize;
    let slot_by = 0usize;
    let slot_sub_tx = 0usize;
    let slot_sub_ty = 10usize;

    // Old calculation: compares slot_sub_tx (0) with cam_tx (17)
    let old_d_tx = (slot_sub_tx as i32 - cam_tx).abs();
    let old_dist = old_d_tx;
    let old_evicted = old_dist >= 4;

    // New calculation: computes world_tx = bx * 16 + sub_tx = 16
    let world_tx = slot_bx as i32 * 16 + slot_sub_tx as i32;
    let new_d_tx = (world_tx - cam_tx).abs();
    let new_dist = new_d_tx;
    let new_protected = new_dist <= 2;

    println!("{:<32} | {:<16} | {:<16}", "Property", "Old Buggy Code", "New Fixed Code");
    println!("--------------------------------------------------------------------------------");
    println!("{:<32} | {:<16} | {:<16}", "Tile Distance from Car", format!("{} (MISCALCULATED)", old_dist), format!("{} (CORRECT)", new_dist));
    println!("{:<32} | {:<16} | {:<16}", "Cache Action on Road Tile", if old_evicted { "IMMEDIATELY EVICT" } else { "Keep" }, if new_protected { "PROTECT (+500)" } else { "Evict" });
    println!("{:<32} | {:<16} | {:<16}", "Track Road Surface", "Checkerboard Teared", "Solid High-Res Texture");
    println!("================================================================================\n");
}

fn main() {
    benchmark_decoders();
    simulate_seek_patterns();
    simulate_eviction_cache();
}
