//! London 10 km^2 World Definition.
//!
//! Master definition for the seamless 10 km^2 free roam map in London,
//! spanning Westminster, The Mall, Trafalgar Square, Waterloo, and the Thames.

use crate::math::{Fixed, Vec2};
use crate::timing::{CheckpointGate, ParTimes};
use crate::track::{TrackDef, TrackTile, MAX_TRACK_CHECKPOINTS};

#[allow(unsafe_code)]
/// 240x240 tile collision grid (57,600 tiles).
pub static LONDON_TILES: [TrackTile; 57600] = unsafe {
    core::mem::transmute(*include_bytes!(
        "../../../tracks/london_10km/london_10km_collision.bin"
    ))
};

/// Track definition for London 10 km^2 Free Roam.
pub static TRACK_LONDON: TrackDef = TrackDef {
    name: "LONDON 10KM",
    width: 240,
    height: 240,
    start_pos: Vec2 {
        x: Fixed::from_raw(11730944),
        y: Fixed::from_raw(19333120),
    },
    start_heading: 3758, // Facing North-East down The Mall towards Admiralty Arch & Trafalgar
    start_gate: CheckpointGate {
        x: 89,
        y: 147,
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
    tiles: &LONDON_TILES,
    route: &[],
    half_width: 32,
};
