//! GPU Rendering Subsystem for Arduracer PSX.
//!
//! Organizes double-buffering, camera tracking, tile blitting, vehicle sprites,
//! particle systems, and the in-game HUD.

pub mod camera;
pub mod car_geometry;
pub mod car_renderer;
pub mod hud_renderer;
pub mod particles;
pub mod skidmarks;
pub mod tile_blitter;

pub use camera::Camera;
pub use car_renderer::render_car;
pub use hud_renderer::render_hud;
pub use particles::ParticleSystem;
pub use skidmarks::SkidmarkBuffer;
pub use tile_blitter::render_track;
