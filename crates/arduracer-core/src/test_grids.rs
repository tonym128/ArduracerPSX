//! Host-test grid installation.
//!
//! The tile grids moved out of the executable and onto the disc, so the host
//! harness cannot read them from `levels.rs` any more -- and it must not be
//! given a private copy of them either. `GAME.md` §7 is explicit that testing a
//! *mirror* of the data is how this project acquires green tests that lie.
//!
//! So the tests load `LEVELS.BIN`, the same asset the console loads, and bind it
//! exactly the way the runtime does. A cooker bug or a prefix-sum mistake then
//! fails here rather than on hardware.
//!
//! Test builds are exempt from the crate's `unsafe_code` ban, which is what
//! makes the rebind expressible on the host at all.

#![cfg(test)]

use std::path::PathBuf;

use crate::TrackTile;

/// Shared staging, one grid at a time. Kept per-level so several levels can be
/// bound at once, which is what the existing tests assume.
const MAX: usize = 57_600;
static mut BUFFERS: [TrackTile; 27 * MAX] = [TrackTile::OffRoad; 27 * MAX];

/// Grid dimensions, mirroring `game::gpu::tracktex::LEVEL_DIMS`.
const LEVEL_DIMS: [(u8, u8); 27] = [
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
    (240, 240),
    (240, 240),
    (240, 240),
];

fn asset() -> Vec<u8> {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("../../assets/LEVELS.BIN");
    std::fs::read(&p).unwrap_or_else(|e| panic!("cannot read {}: {e}", p.display()))
}

/// Loads every level's grid from `LEVELS.BIN` and binds it.
///
/// Idempotent and cheap after the first call, because it runs from
/// `tile_grid`, which is on the hot path of every test that reads terrain.
pub fn ensure_installed() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let asset = asset();
        let mut off = 0usize;
        for (i, (w, h)) in LEVEL_DIMS.into_iter().enumerate() {
            let len = w as usize * h as usize;
            assert!(
                off + len <= asset.len(),
                "LEVELS.BIN is short at level {i}: {off}+{len} > {}",
                asset.len()
            );
            let base = i * MAX;
            let all = unsafe { &mut *core::ptr::addr_of_mut!(BUFFERS) };
            let buf = &mut all[base..base + len];
            for (cell, byte) in buf.iter_mut().zip(&asset[off..off + len]) {
                *cell = TrackTile::from_disc_byte(*byte);
            }
            if let Some(def) = def_for(i) {
                // SAFETY: test-only, single-threaded, and the write is the same
                // one the runtime performs through a volatile store.
                let def = def as *const crate::TrackDef as *mut crate::TrackDef;
                unsafe {
                    core::ptr::write_volatile(&raw mut (*def).tiles, Some(buf));
                }
            }
            off += len;
        }
    });
}

/// The `TrackDef` for a level index: circuits 0..23, then the three cities.
fn def_for(i: usize) -> Option<&'static crate::TrackDef> {
    match i {
        0..=23 => Some(crate::levels::ALL_TRACKS[i]),
        24 => Some(&crate::capetown::TRACK_CAPETOWN),
        25 => Some(&crate::melbourne::TRACK_MELBOURNE),
        26 => Some(&crate::london::TRACK_LONDON),
        _ => None,
    }
}

#[test]
fn installing_the_asset_binds_every_level() {
    ensure_installed();
    for (i, (w, h)) in LEVEL_DIMS.into_iter().enumerate() {
        let Some(def) = def_for(i) else { continue };
        assert_eq!(
            def.tile_grid().len(),
            w as usize * h as usize,
            "level {i} grid length"
        );
        assert!(def.tiles.is_some(), "level {i} did not bind");
    }
}
