//! Cape Town 10 km^2 World Definition.
//!
//! Master definition for the seamless 10 km^2 free roam map in Cape Town,
//! spanning Green Point, the V&A Waterfront, Sea Point, and Signal Hill.

use crate::math::{Fixed, Vec2};
use crate::timing::{CheckpointGate, ParTimes};
use crate::track::{TrackDef, TrackTile, MAX_TRACK_CHECKPOINTS};

#[allow(unsafe_code)]
/// 240x240 tile collision grid (57,600 tiles).
pub static CAPETOWN_TILES: [TrackTile; 57600] = unsafe {
    core::mem::transmute(*include_bytes!(
        "../../../dist/capetown_10km/capetown_10km_collision.bin"
    ))
};

/// Track definition for Cape Town 10 km^2 Free Roam.
pub static TRACK_CAPETOWN: TrackDef = TrackDef {
    name: "CAPE TOWN 10KM",
    width: 240,
    height: 240,
    start_pos: Vec2 {
        x: Fixed::from_raw(4865 * 4096),
        y: Fixed::from_raw(3123 * 4096),
    },
    start_heading: 1024, // East along Helen Suzman Blvd
    start_gate: CheckpointGate {
        x: 152,
        y: 97,
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
    tiles: &CAPETOWN_TILES,
    route: &[],
    half_width: 32,
};
