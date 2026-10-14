//! London 10 km^2 World Definition.
//!
//! Master definition for the seamless 10 km^2 free roam map in London,
//! spanning Westminster, The Mall, Trafalgar Square, Waterloo, and the Thames.

use crate::math::{Fixed, Vec2};
use crate::timing::{CheckpointGate, ParTimes};
use crate::track::{TrackDef, MAX_TRACK_CHECKPOINTS};

/// Track definition for London 10 km^2 Free Roam.
// The runtime rebinds `tiles` to the grid it reads from the disc, so this
// definition has to be writable. A `TrackDef` whose `tiles` is `None` is a
// pure constant, so without the explicit section rustc promotes the whole
// literal into `.rodata`: the rebind then writes to read-only memory,
// silently does nothing, and every tile reads as `Barrier` -- the AI has no
// road to follow and the car never leaves the grid.
//
// `link_section` is the only construct that can say "put this in RAM", and the
// crate denies it by default, so this is a deliberate, one-per-level exception.
#[allow(unsafe_code)]
#[link_section = ".data"]
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
    tiles: None,
    route: &[],
    half_width: 32,
};
