//! Frustum-culled track tilemap blitter.
//!
//! Renders the visible 64x64 track tiles within the camera viewport,
//! mapping surface types into distinct, vibrant arcade colors.

use crate::gpu::camera::Camera;
use arduracer_core::{SurfaceType, TrackDef};
use psx_gpu as gpu;

pub const TILE_SIZE: i16 = 64;

/// Renders all visible track tiles for the active camera frame.
pub fn render_track(track: &TrackDef, camera: &Camera, draw_y: i16) {
    let cam_world_x = camera.pos.x.to_int();
    let cam_world_y = camera.pos.y.to_int();

    // Determine visible tile range
    let min_tx = ((cam_world_x - 180) / (TILE_SIZE as i32)).max(0) as u8;
    let max_tx = (((cam_world_x + 180) / (TILE_SIZE as i32)) + 1).min(track.width as i32) as u8;
    let min_ty = ((cam_world_y - 140) / (TILE_SIZE as i32)).max(0) as u8;
    let max_ty = (((cam_world_y + 140) / (TILE_SIZE as i32)) + 1).min(track.height as i32) as u8;

    for ty in min_ty..max_ty {
        for tx in min_tx..max_tx {
            let tile_idx = (ty as usize) * (track.width as usize) + (tx as usize);
            let raw_tile = if tile_idx < track.tiles.len() {
                track.tiles[tile_idx]
            } else {
                20
            };

            let tile_world_x = (tx as i32) * (TILE_SIZE as i32);
            let tile_world_y = (ty as i32) * (TILE_SIZE as i32);

            let screen_x = 160 + (tile_world_x - cam_world_x) as i16;
            let screen_y = 120 + (tile_world_y - cam_world_y) as i16 + draw_y;

            // Draw tile background according to surface type
            let surface = track.surface_at(tx, ty);
            match surface {
                SurfaceType::Tarmac => {
                    // Check special tile types: Start line (24, 25) or Checkpoint (26, 27)
                    if raw_tile == 24 || raw_tile == 25 {
                        // Checkered Start/Finish line
                        gpu::fill_rect(screen_x as u16, screen_y as u16, 64, 64, 50, 50, 55);
                        // Checkered pattern inside
                        gpu::fill_rect(
                            (screen_x + 16) as u16,
                            screen_y as u16,
                            32,
                            64,
                            240,
                            240,
                            240,
                        );
                    } else if raw_tile == 26 || raw_tile == 27 {
                        // Neon Cyan Checkpoint Gate
                        gpu::fill_rect(screen_x as u16, screen_y as u16, 64, 64, 45, 45, 55);
                        gpu::fill_rect(
                            (screen_x + 2) as u16,
                            (screen_y + 2) as u16,
                            60,
                            6,
                            0,
                            200,
                            255,
                        );
                        gpu::fill_rect(
                            (screen_x + 2) as u16,
                            (screen_y + 56) as u16,
                            60,
                            6,
                            0,
                            200,
                            255,
                        );
                    } else {
                        // Standard Dark Asphalt Tarmac
                        gpu::fill_rect(screen_x as u16, screen_y as u16, 64, 64, 42, 44, 50);
                    }
                }
                SurfaceType::Curb => {
                    // Red & White striped rumble curb
                    gpu::fill_rect(screen_x as u16, screen_y as u16, 64, 64, 210, 30, 30);
                    gpu::fill_rect(
                        (screen_x + 16) as u16,
                        (screen_y + 16) as u16,
                        32,
                        32,
                        245,
                        245,
                        245,
                    );
                }
                SurfaceType::OffRoad => {
                    // Grass / Gravel terrain
                    gpu::fill_rect(screen_x as u16, screen_y as u16, 64, 64, 24, 55, 30);
                }
                SurfaceType::OilSlick => {
                    // Slippery deep purple oil patch
                    gpu::fill_rect(screen_x as u16, screen_y as u16, 64, 64, 42, 44, 50);
                    gpu::fill_rect(
                        (screen_x + 12) as u16,
                        (screen_y + 12) as u16,
                        40,
                        40,
                        50,
                        10,
                        65,
                    );
                }
                SurfaceType::BoostPad => {
                    // Neon Orange Speed Booster
                    gpu::fill_rect(screen_x as u16, screen_y as u16, 64, 64, 42, 44, 50);
                    gpu::fill_rect(
                        (screen_x + 8) as u16,
                        (screen_y + 20) as u16,
                        48,
                        24,
                        255,
                        140,
                        0,
                    );
                }
                SurfaceType::Barrier => {
                    // Solid barrier border
                    gpu::fill_rect(screen_x as u16, screen_y as u16, 64, 64, 15, 15, 20);
                }
            }
        }
    }
}
