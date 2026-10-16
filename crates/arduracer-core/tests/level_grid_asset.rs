//! `LEVELS.BIN` must be byte-for-byte the tile grids the engine used to compile
//! in, or the disc and the host tools disagree about what a level looks like.
//!
//! The grids are removed from the executable by this change, so there is no
//! longer a second copy to compare against at runtime. The golden file is that
//! second copy, captured while both existed: per-level FNV-1a over the raw disc
//! bytes, plus the cell count.
//!
//! Regenerate `tests/data/level_grid_golden.txt` only from the committed
//! `levels.rs` and the three city modules *before* they lose their arrays --
//! never from `LEVELS.BIN` itself, which is the thing under test.

use std::path::PathBuf;

/// Grid dimensions, mirroring `game::gpu::tracktex::LEVEL_DIMS`.
const LEVEL_DIMS: [(u8, u8); 28] = [
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
    (128, 128),
    (240, 240),
    (240, 240),
    (240, 240),
];

fn asset() -> Vec<u8> {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("../../assets/LEVELS.BIN");
    std::fs::read(&p).unwrap_or_else(|e| panic!("cannot read {}: {e}", p.display()))
}

fn golden() -> Vec<String> {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("tests/data/level_grid_golden.txt");
    std::fs::read_to_string(p)
        .expect("golden file missing")
        .lines()
        .map(str::to_string)
        .collect()
}

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

/// Every level's bytes, sliced by the same prefix sums the runtime loader uses.
fn slices(asset: &[u8]) -> Vec<&[u8]> {
    let mut out = Vec::new();
    let mut off = 0usize;
    for (w, h) in LEVEL_DIMS {
        let len = w as usize * h as usize;
        assert!(off + len <= asset.len(), "asset too short at {off}+{len}");
        out.push(&asset[off..off + len]);
        off += len;
    }
    out
}

#[test]
fn asset_length_is_exactly_the_sum_of_the_grids() {
    let expected: usize = LEVEL_DIMS
        .iter()
        .map(|(w, h)| *w as usize * *h as usize)
        .sum();
    assert_eq!(asset().len(), expected);
    assert_eq!(
        expected, 342_784,
        "24 circuits at 6,400 plus 1 circuit at 16,384 plus 3 cities at 57,600"
    );
}

#[test]
fn every_level_matches_its_golden_hash_and_size() {
    let asset = asset();
    let golden = golden();
    assert_eq!(golden.len(), LEVEL_DIMS.len(), "golden file out of date");

    for (i, (bytes, want)) in slices(&asset).into_iter().zip(golden.iter()).enumerate() {
        let want_hash = want.split(':').nth(1).expect("malformed golden line");
        let want_len: usize = want
            .split(':')
            .nth(2)
            .expect("malformed golden line")
            .parse()
            .unwrap();
        assert_eq!(bytes.len(), want_len, "level {i}: cell count changed");
        assert_eq!(
            format!("{:016x}", fnv1a(bytes)),
            want_hash,
            "level {i}: grid bytes differ from the committed source of truth"
        );
    }
}

/// The offset arithmetic the loader uses must land on the same bytes the golden
/// describes. This is the check that would have caught a wrong prefix sum: the
/// loader slices by its own table, and a mismatch shows up as a level that
/// silently becomes its neighbour's terrain.
#[test]
fn loader_offsets_agree_with_the_asset_layout() {
    let asset = asset();
    let golden = golden();
    let mut off = 0usize;
    for (i, (w, h)) in LEVEL_DIMS.into_iter().enumerate() {
        let len = w as usize * h as usize;
        // Mirrors `game::level_grid::offset_in_asset`.
        let mut prefix = 0usize;
        for (pw, ph) in LEVEL_DIMS.iter().take(i) {
            prefix += *pw as usize * *ph as usize;
        }
        assert_eq!(prefix, off, "level {i}: prefix sum disagrees");
        let bytes = &asset[prefix..prefix + len];
        let want_hash = golden[i].split(':').nth(1).unwrap();
        assert_eq!(format!("{:016x}", fnv1a(bytes)), want_hash, "level {i}");
        off += len;
    }
}
