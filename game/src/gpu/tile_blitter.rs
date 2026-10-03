//! Frustum-culled track tilemap blitter.
//!
//! Renders the visible 64x64 track tiles within the camera viewport, mapping
//! [`TrackTile`] surfaces into distinct, vibrant arcade colors. The camera zoom
//! is applied here so high speed genuinely reveals more of the road ahead
//! (GAME.md §5.1 "Dynamic Zoom").

use crate::gpu::camera::Camera;
use arduracer_core::{TrackDef, TrackTile, TILE_SIZE};
use psx_gpu as gpu;

const TILE: i32 = TILE_SIZE;

/// Renders all visible track tiles for the active camera frame.
pub fn render_track(track: &TrackDef, camera: &Camera, draw_y: i16) {
    let cam_x = camera.pos.x.to_int();
    let cam_y = camera.pos.y.to_int();

    // Zoom is 1.0 at rest and falls to ~0.90 at top speed: shrink the sampled
    // world radius so more of the circuit fits on screen.
    let zoom_q12 = camera.zoom.raw().clamp(3600, 4096);
    let half_w = (150 * 4096 / zoom_q12).max(90);
    let half_h = (110 * 4096 / zoom_q12).max(64);

    let min_tx = ((cam_x - half_w) / TILE).max(0) as u8;
    let max_tx = ((((cam_x + half_w) / TILE) + 1).max(0) as i32).min(track.width as i32) as u8;
    let min_ty = ((cam_y - half_h) / TILE).max(0) as u8;
    let max_ty = ((((cam_y + half_h) / TILE) + 1).max(0) as i32).min(track.height as i32) as u8;

    for ty in min_ty..max_ty {
        for tx in min_tx..max_tx {
            let tile = track.tile_at(tx, ty);
            let tile_world_x = (tx as i32) * TILE;
            let tile_world_y = (ty as i32) * TILE;

            // Integer zoom keeps the tilemap pixel-locked (no shimmer).
            let scale_step = 40; // per unit of (4096 - zoom)
            let shrink = ((4096 - zoom_q12) * scale_step / 4096).min(8) as i32;
            let draw_size = (TILE - shrink) as i16;

            let screen_x = (160 + (tile_world_x - cam_x)) as i16;
            let screen_y = (120 + (tile_world_y - cam_y)) as i16 + draw_y;

            let px = screen_x as u16;
            let py = screen_y as u16;

            match tile {
                TrackTile::Tarmac => {
                    // Dark asphalt with a subtle centre-line dash for readability.
                    gpu::fill_rect(px, py, draw_size as u16, draw_size as u16, 42, 44, 50);
                    if (tx + ty) % 4 < 2 {
                        gpu::fill_rect(px + 30, py, 4, draw_size as u16, 88, 90, 98);
                    }
                }
                TrackTile::StartFinish => {
                    // Checkered start / finish line.
                    gpu::fill_rect(px, py, draw_size as u16, draw_size as u16, 235, 235, 235);
                    for row in 0..4 {
                        let c = if (row + (tx as i32)) % 2 == 0 {
                            (24, 24, 28)
                        } else {
                            (235, 235, 235)
                        };
                        gpu::fill_rect(
                            px,
                            py + (row as u16) * 16,
                            draw_size as u16,
                            16,
                            c.0,
                            c.1,
                            c.2,
                        );
                    }
                }
                TrackTile::Checkpoint => {
                    // Neon cyan gate posts across the racing surface.
                    gpu::fill_rect(px, py, draw_size as u16, draw_size as u16, 30, 30, 38);
                    gpu::fill_rect(px, py + 6, draw_size as u16, 5, 0, 200, 255);
                    gpu::fill_rect(
                        px,
                        py + (draw_size as u16).saturating_sub(11),
                        draw_size as u16,
                        5,
                        0,
                        200,
                        255,
                    );
                }
                TrackTile::Curb => {
                    // Red / white rumble curb.
                    gpu::fill_rect(px, py, draw_size as u16, draw_size as u16, 210, 30, 30);
                    gpu::fill_rect(
                        px + 14,
                        py + 14,
                        (draw_size as u16).saturating_sub(28),
                        (draw_size as u16).saturating_sub(28),
                        245,
                        245,
                        245,
                    );
                }
                TrackTile::OffRoad => {
                    // Grass / gravel terrain.
                    gpu::fill_rect(px, py, draw_size as u16, draw_size as u16, 26, 58, 32);
                }
                TrackTile::OilSlick => {
                    // Slippery deep-purple patch.
                    gpu::fill_rect(px, py, draw_size as u16, draw_size as u16, 30, 24, 44);
                    gpu::fill_rect(px + 12, py + 12, 40, 40, 58, 12, 74);
                }
                TrackTile::BoostPad => {
                    // Neon orange speed booster.
                    gpu::fill_rect(px, py, draw_size as u16, draw_size as u16, 30, 30, 38);
                    for i in 0..3 {
                        gpu::fill_rect(
                            px + 8,
                            py + 16 + (i as u16) * 12,
                            (draw_size as u16).saturating_sub(16),
                            8,
                            255,
                            150 - (i as u8) * 40,
                            0,
                        );
                    }
                }
                TrackTile::Barrier => {
                    // Solid barrier.
                    gpu::fill_rect(px, py, draw_size as u16, draw_size as u16, 14, 14, 18);
                }
            }
        }
    }
}
