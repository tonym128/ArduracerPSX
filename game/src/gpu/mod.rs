//! GPU Rendering Subsystem for Arduracer PSX.
//!
//! Organizes double-buffering, camera tracking, tile blitting, vehicle sprites,
//! particle systems, and the in-game HUD.

pub mod camera;
pub mod car_geometry;
pub mod car_renderer;
pub mod debug_overlay;
pub mod effects_sim;
pub mod gte_proj;
pub mod hud_renderer;
pub mod ot_pass;
pub mod palette;
pub mod particles;
pub mod props_renderer;
pub mod skidmarks;
pub mod texlayout;
pub mod texpipe;
pub mod tracktex;
pub mod transitions;

pub use camera::Camera;
pub use car_renderer::render_car;
pub use debug_overlay::DebugOverlay;
pub use effects_sim::{Particle, ParticleSystem, ParticleType, Skidmark, SkidmarkBuffer};
pub use hud_renderer::{bake_minimap, draw_ui_minimap, render_hud, render_replay_hud};
pub use props_renderer::render_track_props;
pub use texpipe::TextureSlot;
pub use tracktex::{
    init_track_texture, reset_texture_caches, set_frame_period, set_texture_format, texture_format,
    Framerate, TextureFormat,
};
pub use tracktex::{load_level_grid, prewarm_starting_tiles, render_track, render_track_extents};
pub use transitions::{ScreenTransition, TransitionKind};
