//! Melbourne 10 km^2 World Definition.
//!
//! Master definition for the seamless 10 km^2 free roam map in Melbourne,
//! spanning Albert Park Grand Prix Circuit, Lakeside Drive, and St Kilda.

use crate::math::{Fixed, Vec2};
use crate::timing::{CheckpointGate, ParTimes};
use crate::track::{TrackDef, TrackTile, MAX_TRACK_CHECKPOINTS};

#[allow(unsafe_code)]
/// 240x240 tile collision grid (57,600 tiles).
pub static MELBOURNE_TILES: [TrackTile; 57600] = unsafe {
    core::mem::transmute(*include_bytes!(
        "../../../tracks/melbourne_10km/melbourne_10km_collision.bin"
    ))
};

/// Track definition for Melbourne 10 km^2 Free Roam.
pub static TRACK_MELBOURNE: TrackDef = TrackDef {
    name: "MELBOURNE 10KM",
    width: 240,
    height: 240,
    start_pos: Vec2 {
        x: Fixed::from_raw(4011 * 4096),
        y: Fixed::from_raw(5679 * 4096),
    },
    start_heading: 3072, // North along Lakeside Drive
    start_gate: CheckpointGate {
        x: 125,
        y: 177,
        width: 4,
        height: 4,
    },
    par_times: ParTimes {
        bronze_ticks: 0,
        silver_ticks: 0,
        gold_ticks: 0,
        dev_platinum_ticks: 0,
    },
    checkpoint_count: 0,
    checkpoints: [CheckpointGate {
        x: 0,
        y: 0,
        width: 0,
        height: 0,
    }; MAX_TRACK_CHECKPOINTS],
    tiles: &MELBOURNE_TILES,
    route: &[],
    half_width: 32,
};
