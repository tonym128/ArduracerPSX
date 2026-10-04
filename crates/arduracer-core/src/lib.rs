//! Arduracer PSX Core Simulation Library.
//!
//! Pure `#![no_std]` game engine logic, physics, tuning, timing, save data,
//! and track representation. Fully testable natively on the host workstation.
//!
//! `no_std` is applied only to non-test builds, so `cargo test` can exercise the
//! real game logic on the host while the `mipsel-sony-psx` target keeps its
//! freestanding guarantee.

#![cfg_attr(not(test), no_std)]
// Rejects unsafe code in target builds, per the AGENT.md hardware discipline.
#![cfg_attr(not(test), deny(unsafe_code))]

pub mod ai;
pub mod ai_profiles;
pub mod championship;
pub mod drift;
pub mod ghost;
pub mod levels;
pub mod math;
pub mod save;
pub mod surface;
pub mod timing;
pub mod track;
pub mod tuning;
pub mod vehicle;

pub use ai::{angle_error, atan2_bams, heading_towards, AiRacer};
pub use ai_profiles::{AiProfile, AI_PROFILES};
pub use championship::{compute_standings, ChampionshipSession, Competitor, POINTS_TABLE};
pub use drift::{DriftState, DRIFT_BOOST_LEVEL1_TICKS, DRIFT_BOOST_LEVEL2_TICKS, SPINOUT_TICKS};
pub use ghost::{
    GhostFrame, GhostPlaybackState, GhostPlayer, GhostRecorder, FLAG_BOOSTING, FLAG_BRAKING,
    FLAG_DRIFTING, FLAG_SKIDMARK, GHOST_MAGIC, GHOST_SAMPLE_INTERVAL_TICKS, GHOST_VERSION,
};
pub use levels::ALL_TRACKS;
pub use math::{cos, sin, Fixed, Vec2, ANGLE_180, ANGLE_360, ANGLE_45, ANGLE_90, FP_ONE, FP_SHIFT};
pub use save::{SaveData, SAVE_HEADER_MAGIC, SAVE_VERSION, TOTAL_TRACKS};
pub use surface::SurfaceType;
pub use timing::{
    CheckpointGate, LapTimer, Medal, ParTimes, StartPhase, StartSequence, COUNTDOWN_GO_TICKS,
    COUNTDOWN_LIGHTS, COUNTDOWN_LIGHT_TICKS, TICKS_PER_SECOND, TOTAL_LAPS,
};
pub use track::{TrackDef, TrackTile, MAX_TRACK_CHECKPOINTS, MAX_TRACK_DIM, TILE_SIZE};
pub use tuning::{CarTuning, DEFAULT_SLIDER, MAX_SLIDER, MIN_SLIDER, TOTAL_POINTS};
pub use vehicle::{VehicleInput, VehicleState, BASE_TOP_SPEED, BASE_TURN_RATE, NITRO_MAX_TICKS};
