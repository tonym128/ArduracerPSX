//! Host micro-benchmark: per-MCU-row decode cost, JPEG vs QOI.
//!
//! This is the number the streaming budget is actually built from. Both decoders
//! are compiled for the host here, so the absolute figure is not the R3000A's,
//! but the *ratio* is meaningful: both paths run the same integer algorithms
//! with the same inlining decisions, and the gap is dominated by algorithmic
//! work (Huffman + IDCT versus a byte-at-a-time colour decode) rather than by
//! anything host-specific.
//!
//! Run: cargo test --manifest-path crates/arduracer-core/Cargo.toml \
//!           --release --test qoi_bench -- --nocapture

use std::time::Instant;

use arduracer_core::jpeg::{
    decode_tile_row_64x16 as jpeg_row, JpegHeader, RESTART_INTERVAL_COUNT, TILE_PIXELS,
};
use arduracer_core::qoi::{
    decode_tile_row_64x16 as qoi_row, index_segments, QoiHeader, SEGMENT_COUNT,
};

const WIDTH: usize = 1024;
const HEIGHT: usize = 1024;
const TILES_PER_AXIS: usize = 16;

/// Times decoding every MCU row of every tile in a block, `reps` times.
///
/// Returns nanoseconds per MCU row, which is the unit the frame budget in
/// `game/src/gpu/tracktex.rs` cares about: one row is one decode step.
fn bench_jpeg(block: &[u8], reps: usize) -> f64 {
    let header = JpegHeader::parse(block).expect("jpeg header");
    let mut offsets = [0u32; RESTART_INTERVAL_COUNT];
    header.index_restarts(block, &mut offsets);

    let mut scratch = [0u16; TILE_PIXELS];
    let start = Instant::now();
    for _ in 0..reps {
        for ty in 0..TILES_PER_AXIS {
            for tx in 0..TILES_PER_AXIS {
                for row in 0..4 {
                    jpeg_row(block, &header, &offsets, tx, ty, row, &mut scratch);
                }
            }
        }
    }
    start.elapsed().as_nanos() as f64 / (reps * TILES_PER_AXIS * TILES_PER_AXIS * 4) as f64
}

fn bench_qoi(block: &[u8], reps: usize) -> f64 {
    let header = QoiHeader::parse(block).expect("qoi header");
    let mut offsets = [0u32; SEGMENT_COUNT];
    index_segments(block, &header, &mut offsets);

    let mut scratch = [0u16; TILE_PIXELS];
    let start = Instant::now();
    for _ in 0..reps {
        for ty in 0..TILES_PER_AXIS {
            for tx in 0..TILES_PER_AXIS {
                for row in 0..4 {
                    qoi_row(block, &header, &offsets, tx, ty, row, &mut scratch);
                }
            }
        }
    }
    start.elapsed().as_nanos() as f64 / (reps * TILES_PER_AXIS * TILES_PER_AXIS * 4) as f64
}

/// Also measures the once-per-CD-load segment/restart indexing, since that is
/// the other thing the feature flag changes.
fn bench_index_jpeg(block: &[u8], reps: usize) -> f64 {
    let header = JpegHeader::parse(block).expect("jpeg header");
    let mut offsets = [0u32; RESTART_INTERVAL_COUNT];
    let start = Instant::now();
    for _ in 0..reps {
        header.index_restarts(block, &mut offsets);
    }
    start.elapsed().as_nanos() as f64 / reps as f64
}

fn bench_index_qoi(block: &[u8], reps: usize) -> f64 {
    let header = QoiHeader::parse(block).expect("qoi header");
    let mut offsets = [0u32; SEGMENT_COUNT];
    let start = Instant::now();
    for _ in 0..reps {
        index_segments(block, &header, &mut offsets);
    }
    start.elapsed().as_nanos() as f64 / reps as f64
}

#[test]
fn bench_mcu_row_decode_jpeg_vs_qoi() {
    const REPS: usize = 20;

    let jpeg_path = "../../tracks/Amber_Mesa.jpg";
    let qoi_path = "tests/data/qoi_track.qoi";

    let Ok(jpeg_block) = std::fs::read(jpeg_path) else {
        eprintln!("skipping: {jpeg_path} not present");
        return;
    };
    let qoi_block = std::fs::read(qoi_path).expect("qoi fixture");

    println!(
        "\nblock: {} x {}, {} MCU rows per image",
        WIDTH,
        HEIGHT,
        TILES_PER_AXIS * TILES_PER_AXIS * 4
    );
    println!("  JPEG payload {:>8} KB", jpeg_block.len() as f64 / 1024.0);
    println!("  QOI  payload {:>8.1} KB", qoi_block.len() as f64 / 1024.0);

    // Warm up so the first measured rep is not paying page-fault cost.
    let _ = bench_jpeg(&jpeg_block, 1);
    let _ = bench_qoi(&qoi_block, 1);

    let j = bench_jpeg(&jpeg_block, REPS);
    let q = bench_qoi(&qoi_block, REPS);
    let ij = bench_index_jpeg(&jpeg_block, REPS);
    let iq = bench_index_qoi(&qoi_block, REPS);

    println!("\nper 64x16 MCU row decode:");
    println!("  JPEG  {j:>10.0} ns/row");
    println!("  QOI   {q:>10.0} ns/row");
    println!("  speedup {:.2}x\n", j / q);

    println!("whole 1024x1024 image (1024 rows):");
    println!("  JPEG  {:>10.2} ms", j * 1024.0 / 1e6);
    println!("  QOI   {:>10.2} ms", q * 1024.0 / 1e6);
    println!("  speedup {:.2}x\n", j / q);

    println!("per-CD-load block indexing:");
    println!("  JPEG restart index {ij:>8.0} ns");
    println!("  QOI  segment index {iq:>8.0} ns");
    println!("  speedup {:.2}x", ij / iq);

    // The claim this feature rests on. If QOI ever stops being meaningfully
    // faster, the flag's default should be revisited.
    assert!(
        q < j,
        "QOI MCU-row decode ({q:.0} ns) is not faster than JPEG ({j:.0} ns)"
    );
}
