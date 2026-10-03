//! Frustum-culled arcade track tilemap blitter.
//!
//! Renders the visible 64x64 track tiles within the camera viewport, mapping
//! [`TrackTile`] surfaces into distinct, vibrant arcade colors with hardware
//! clipping and seamless edge-to-edge alignment.

use crate::gpu::camera::Camera;
use arduracer_core::{TrackDef, TrackTile, TILE_SIZE};
use psx_gpu as gpu;

const TILE: i32 = TILE_SIZE;

/// Renders all visible track tiles for the active camera frame.
pub fn render_track(track: &TrackDef, camera: &Camera, _draw_y: i16) {
    let cam_x = camera.pos.x.to_int();
    let cam_y = camera.pos.y.to_int();

    // 320x240 screen viewport: ±180 px horizontal, ±140 px vertical for margin
    let half_w = 180;
    let half_h = 140;

    let min_tx = ((cam_x - half_w) / TILE).max(0) as u8;
    let max_tx = ((((cam_x + half_w) / TILE) + 1).max(0) as i32).min(track.width as i32) as u8;
    let min_ty = ((cam_y - half_h) / TILE).max(0) as u8;
    let max_ty = ((((cam_y + half_h) / TILE) + 1).max(0) as i32).min(track.height as i32) as u8;

    let tile_sz = TILE as u16;

    for ty in min_ty..max_ty {
        for tx in min_tx..max_tx {
            let tile = track.tile_at(tx, ty);
            let tile_world_x = (tx as i32) * TILE;
            let tile_world_y = (ty as i32) * TILE;

            let screen_x = (160 + (tile_world_x - cam_x)) as i16;
            let screen_y = (120 + (tile_world_y - cam_y)) as i16;

            match tile {
                TrackTile::Tarmac => {
                    // Dark asphalt road base
                    gpu::draw_rect_flat(screen_x, screen_y, tile_sz, tile_sz, 38, 40, 46);

                    // Road texture grain / racing groove
                    let hash = ((tx as i32 * 7) ^ (ty as i32 * 13)) & 3;
                    if hash == 0 {
                        gpu::draw_rect_flat(screen_x + 8, screen_y + 8, 48, 48, 34, 36, 42);
                    }

                    // Dashed white/yellow road centerline
                    if (tx + ty) % 2 == 0 {
                        gpu::draw_rect_flat(screen_x + 30, screen_y + 16, 4, 32, 220, 220, 230);
                    }
                }
                TrackTile::StartFinish => {
                    // Base track surface
                    gpu::draw_rect_flat(screen_x, screen_y, tile_sz, tile_sz, 38, 40, 46);

                    // Checkered start/finish gantry band across the tile (4 rows of 8x8 checks)
                    for r in 0..4i16 {
                        for c in 0..8i16 {
                            let (cr, cg, cb) = if (r + c) % 2 == 0 {
                                (240, 240, 245) // Pure White
                            } else {
                                (20, 20, 26) // Carbon Black
                            };
                            gpu::draw_rect_flat(
                                screen_x + c * 8,
                                screen_y + 16 + r * 8,
                                8,
                                8,
                                cr,
                                cg,
                                cb,
                            );
                        }
                    }
                    // Yellow start grid pole lines
                    gpu::draw_rect_flat(screen_x, screen_y + 14, tile_sz, 2, 255, 215, 0);
                    gpu::draw_rect_flat(screen_x, screen_y + 48, tile_sz, 2, 255, 215, 0);
                }
                TrackTile::Checkpoint => {
                    // Dark asphalt underlay
                    gpu::draw_rect_flat(screen_x, screen_y, tile_sz, tile_sz, 36, 38, 44);

                    // Luminous neon cyan timing beam across the track
                    gpu::draw_rect_flat(screen_x, screen_y + 24, tile_sz, 16, 0, 180, 240);
                    gpu::draw_rect_flat(screen_x, screen_y + 28, tile_sz, 8, 120, 240, 255);

                    // Yellow timing sensor pylons on lateral edges
                    gpu::draw_rect_flat(screen_x, screen_y + 16, 6, 32, 255, 220, 20);
                    gpu::draw_rect_flat(screen_x + 58, screen_y + 16, 6, 32, 255, 220, 20);
                }
                TrackTile::Curb => {
                    // Gravel verge base
                    gpu::draw_rect_flat(screen_x, screen_y, tile_sz, tile_sz, 50, 48, 42);

                    // Alternating Red & White rumble curb blocks (4 diagonal stripes)
                    for i in 0..4i16 {
                        let (cr, cg, cb) = if (i + (tx as i16)) % 2 == 0 {
                            (225, 30, 45) // Crimson Red
                        } else {
                            (245, 245, 250) // Crisp White
                        };
                        gpu::draw_rect_flat(screen_x + i * 16, screen_y, 16, tile_sz, cr, cg, cb);
                    }
                    // Inner tarmac border transition
                    gpu::draw_rect_flat(screen_x + 8, screen_y + 8, 48, 48, 42, 44, 50);
                }
                TrackTile::OffRoad => {
                    // Rich emerald grass terrain
                    gpu::draw_rect_flat(screen_x, screen_y, tile_sz, tile_sz, 28, 62, 34);

                    // Subtle darker grass patches for organic texture
                    let patch = ((tx as i32 * 11) + (ty as i32 * 5)) & 3;
                    if patch == 0 {
                        gpu::draw_rect_flat(screen_x + 12, screen_y + 12, 24, 24, 22, 52, 28);
                    } else if patch == 1 {
                        gpu::draw_rect_flat(screen_x + 36, screen_y + 28, 20, 20, 24, 55, 30);
                    }
                }
                TrackTile::OilSlick => {
                    // Asphalt base
                    gpu::draw_rect_flat(screen_x, screen_y, tile_sz, tile_sz, 36, 38, 44);

                    // Iridescent dark purple slick puddle
                    gpu::draw_rect_flat(screen_x + 8, screen_y + 8, 48, 48, 32, 18, 48);
                    gpu::draw_rect_flat(screen_x + 14, screen_y + 14, 36, 36, 56, 16, 78);
                    // Shimmer reflection highlight
                    gpu::draw_rect_flat(screen_x + 20, screen_y + 20, 16, 8, 120, 40, 160);
                }
                TrackTile::BoostPad => {
                    // Dark tarmac track bed
                    gpu::draw_rect_flat(screen_x, screen_y, tile_sz, tile_sz, 32, 34, 40);

                    // Glowing neon orange/yellow chevron booster arrows
                    let chevrons = [10i16, 24, 38];
                    for (idx, &cy) in chevrons.iter().enumerate() {
                        let (cr, cg) = if idx == 0 {
                            (255, 230) // Golden yellow leading edge
                        } else if idx == 1 {
                            (255, 170) // Hot orange mid
                        } else {
                            (255, 110) // Deep amber rear
                        };
                        gpu::draw_rect_flat(screen_x + 12, screen_y + cy, 40, 8, cr, cg, 0);
                        // Center arrow tip
                        gpu::draw_rect_flat(screen_x + 26, screen_y + cy - 4, 12, 4, cr, cg, 0);
                    }
                }
                TrackTile::Barrier => {
                    // Armco steel barrier with yellow/black hazard markings
                    gpu::draw_rect_flat(screen_x, screen_y, tile_sz, tile_sz, 18, 20, 26);
                    for i in 0..4i16 {
                        let (br, bg, bb) = if (i + (tx as i16)) % 2 == 0 {
                            (240, 200, 20) // Safety Yellow
                        } else {
                            (30, 30, 35) // Hazard Black
                        };
                        gpu::draw_rect_flat(screen_x + i * 16, screen_y + 16, 16, 32, br, bg, bb);
                    }
                    // Steel guardrail cap
                    gpu::draw_rect_flat(screen_x, screen_y + 12, tile_sz, 4, 180, 190, 205);
                    gpu::draw_rect_flat(screen_x, screen_y + 48, tile_sz, 4, 140, 150, 165);
                }
            }
        }
    }
}
