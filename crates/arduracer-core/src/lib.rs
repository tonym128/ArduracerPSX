//! Arduracer PSX Core Simulation Library.
//!
//! Pure `#![no_std]` game engine logic, physics, tuning, timing, save data,
//! and track representation. Fully testable natively on the host workstation.
//!
//! `no_std` is applied only to non-test builds, so `cargo test` can exercise the
//! real game logic on the host while the `mipsel-sony-psx` target keeps its
//! freestanding guarantee.

// `no_std` only for the console. Host tools build this crate too, and
// `host_grids` reads LEVELS.BIN from disk on their behalf.
#![cfg_attr(target_arch = "mips", no_std)]
// Rejects unsafe code in target builds, per the AGENT.md hardware discipline.
#![cfg_attr(target_arch = "mips", deny(unsafe_code))]

pub mod ai;
pub mod ai_profiles;
pub mod capetown;
pub mod championship;
pub mod contact;
pub mod drift;
pub mod ghost;
pub mod jpeg;
pub mod levels;
pub mod london;
pub mod math;
pub mod melbourne;
pub mod qoi;
pub mod route;
pub mod save;
pub mod surface;
pub mod svgtex;
pub mod xa;
// Host tools only. The PS1 build loads its grid from the disc instead --
// see `game::level_grid` -- and has no filesystem to load from.
#[cfg(not(target_arch = "mips"))]
pub mod host_grids;
pub mod texture_format;
pub mod timing;
pub mod track;
pub mod track_builder;
pub mod tuning;
pub mod vehicle;
pub mod visual_converter;
pub mod visual_tex;

pub use ai::{angle_error, atan2_bams, heading_towards, AiRacer};
pub use ai_profiles::{AiProfile, AI_PROFILES};
pub use capetown::TRACK_CAPETOWN;
pub use championship::{
    compute_standings, grid_offset, grid_slot_offset_on, grid_slot_position, grid_slot_position_on,
    ChampionshipSession, Competitor, Difficulty, GridSlotOffset, CUP_COUNT, CUP_NAMES,
    GRID_SLOT_OFFSETS, POINTS_TABLE, STAGES_PER_CUP,
};
pub use contact::{
    collide_cars, resolve_field, CarContact, FieldContacts, CAR_BODY_HALF_SPAN, CAR_BODY_RADIUS,
    CAR_RESTITUTION, HARD_BUMP_THRESHOLD,
};
pub use drift::{DriftState, DRIFT_BOOST_LEVEL1_TICKS, DRIFT_BOOST_LEVEL2_TICKS, SPINOUT_TICKS};
pub use ghost::{
    author_ghost_time_gap_ticks, decode_compressed_ghost, encode_compressed_ghost,
    ghost_distance_units, sample_author_ghost, GhostFrame, GhostPlaybackState, GhostPlayer,
    GhostRecorder, FLAG_BOOSTING, FLAG_BRAKING, FLAG_DRIFTING, FLAG_SKIDMARK, GHOST_CRC_SIZE,
    GHOST_HEADER_SIZE, GHOST_MAGIC, GHOST_SAMPLE_INTERVAL_TICKS, GHOST_VERSION,
};
pub use levels::{ALL_TRACKS, ALL_TRACK_VISUALS, AUTHORED_TRACK_COUNT};
pub use london::TRACK_LONDON;
pub use math::{cos, sin, Fixed, Vec2, ANGLE_180, ANGLE_360, ANGLE_45, ANGLE_90, FP_ONE, FP_SHIFT};
pub use melbourne::TRACK_MELBOURNE;

pub use route::{Route, DEFAULT_SAMPLES_PER_SPAN, MAX_ROUTE_SAMPLES};
pub use save::{InputProfile, SaveData, SAVE_HEADER_MAGIC, SAVE_VERSION, TOTAL_TRACKS};
pub use surface::SurfaceType;
pub use timing::{
    CheckpointGate, LapTimer, LaunchOutcome, Medal, ParTimes, StartPhase, StartSequence,
    COUNTDOWN_GO_TICKS, COUNTDOWN_LIGHTS, COUNTDOWN_LIGHT_TICKS, LAUNCH_SWEET_SPOT_MAX_RPM,
    LAUNCH_SWEET_SPOT_MIN_RPM, TICKS_PER_SECOND, TOTAL_LAPS,
};
pub use track::{
    TrackDef, TrackTile, CAR_HALF_WIDTH, LANE_WIDTH, MAX_ROAD_HALF_WIDTH, MAX_TRACK_CHECKPOINTS,
    MAX_TRACK_DIM, MIN_ROAD_HALF_WIDTH, TILE_SIZE,
};
pub use track_builder::{
    CompiledCustomTrack, CustomTrackDef, PlacedBlock, TrackBlockKind, TrackBuildError,
    BLOCK_TILE_SIZE, CUSTOM_GRID_MACRO_DIM, CUSTOM_GRID_TILE_DIM, CUSTOM_TRACK_MAGIC,
    CUSTOM_TRACK_SERIALIZED_SIZE, CUSTOM_TRACK_VERSION, MAX_CUSTOM_BLOCKS,
    TOTAL_CUSTOM_TRACK_SLOTS,
};
pub use tuning::{CarTuning, DEFAULT_SLIDER, MAX_SLIDER, MIN_SLIDER, TOTAL_POINTS};
pub use vehicle::{VehicleInput, VehicleState, BASE_TOP_SPEED, BASE_TURN_RATE, NITRO_MAX_TICKS};

#[cfg(test)]
pub use visual_converter::{convert_to_8bit_colour, ConvertedVisual8};
pub use visual_converter::{
    convert_to_8bit_colour_slice, nearest_palette_index, SourceBitDepth, CORE_PALETTE_16,
    PALETTE_SIZE_8BIT,
};
