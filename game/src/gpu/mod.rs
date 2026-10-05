//! GPU Rendering Subsystem for Arduracer PSX.
//!
//! Organizes double-buffering, camera tracking, tile blitting, vehicle sprites,
//! particle systems, and the in-game HUD.

pub mod camera;
pub mod car_geometry;
pub mod car_renderer;
pub mod effects_sim;
pub mod hud_renderer;
pub mod palette;
pub mod particles;
pub mod skidmarks;
pub mod texlayout;
pub mod texpipe;
pub mod tile_blitter;

pub use camera::Camera;
pub use car_renderer::render_car;
pub use effects_sim::{Particle, ParticleSystem, ParticleType, Skidmark, SkidmarkBuffer};
pub use hud_renderer::{bake_minimap, render_hud};
pub use texpipe::TextureSlot;
pub use tile_blitter::render_track;
