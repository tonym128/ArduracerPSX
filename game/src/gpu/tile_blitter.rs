//! Frustum-culled arcade track tilemap blitter.
//!
//! Renders the visible 64x64 track tiles within the camera viewport, mapping
//! [`TrackTile`] surfaces into distinct, vibrant arcade colors with hardware
//! clipping and seamless edge-to-edge alignment.

use crate::gpu::camera::Camera;
use arduracer_core::{TrackDef, TrackTile, ALL_TRACKS, FP_SHIFT, TILE_SIZE};
use psx_gpu as gpu;

const TILE: i32 = TILE_SIZE;

#[inline(always)]
fn is_corridor(tile: TrackTile) -> bool {
    matches!(
        tile,
        TrackTile::Tarmac
            | TrackTile::StartFinish
            | TrackTile::Checkpoint
            | TrackTile::BoostPad
            | TrackTile::OilSlick
    )
}

type Rgb = (u8, u8, u8);

/// Per-cup environment palette (biome).
#[derive(Copy, Clone)]
pub struct Palette {
    pub road: Rgb,
    pub road2: Rgb,
    pub grass: Rgb,
    pub patch_a: Rgb,
    pub patch_b: Rgb,
    pub curb_a: Rgb,
    pub curb_b: Rgb,
}

/// Bronze: classic GP speedway.
const PAL_SPEEDWAY: Palette = Palette {
    road: (44, 46, 52),
    road2: (40, 42, 48),
    grass: (28, 62, 34),
    patch_a: (22, 52, 28),
    patch_b: (24, 55, 30),
    curb_a: (225, 30, 45),
    curb_b: (245, 245, 250),
};
/// Silver: neon-lit night city, wet midnight asphalt.
const PAL_NEON_CITY: Palette = Palette {
    road: (24, 26, 40),
    road2: (22, 24, 36),
    grass: (14, 18, 34),
    patch_a: (10, 14, 28),
    patch_b: (18, 12, 36),
    curb_a: (255, 40, 200),
    curb_b: (40, 230, 255),
};
/// Gold: red canyon, tan tarmac and sandstone.
const PAL_CANYON: Palette = Palette {
    road: (78, 66, 56),
    road2: (72, 60, 50),
    grass: (150, 82, 46),
    patch_a: (130, 68, 38),
    patch_b: (166, 98, 56),
    curb_a: (230, 120, 30),
    curb_b: (250, 235, 200),
};
/// Platinum: alpine / marina, cool blue-grey road with snowy verges.
const PAL_ALPINE: Palette = Palette {
    road: (52, 60, 74),
    road2: (48, 56, 70),
    grass: (206, 218, 232),
    patch_a: (180, 198, 220),
    patch_b: (226, 234, 244),
    curb_a: (30, 90, 220),
    curb_b: (250, 250, 255),
};

/// Chooses the biome palette for a track from its cup (6 tracks per cup).
fn palette_for(track: &TrackDef) -> Palette {
    let mut cup = 0usize;
    for (i, t) in ALL_TRACKS.iter().enumerate() {
        if t.name == track.name {
            cup = i / 6;
            break;
        }
    }
    match cup {
        0 => PAL_SPEEDWAY,
        1 => PAL_NEON_CITY,
        2 => PAL_CANYON,
        _ => PAL_ALPINE,
    }
}

/// Renders all visible track tiles for the active camera frame.
pub fn render_track(track: &TrackDef, camera: &Camera, _draw_y: i16) {
    let pal = palette_for(track);
    let cam_x = camera.pos.x.to_int();
    let cam_y = camera.pos.y.to_int();

    // Visible world extent, widened by the camera's zoom so a zoomed-out camera
    // draws the extra tiles that come into view. Without this the road would end
    // in mid-air at speed.
    let (half_w, half_h) = camera.visible_half_extents();

    let min_tx = ((cam_x - half_w) / TILE).max(0) as u8;
    let max_tx = (((cam_x + half_w) / TILE) + 1)
        .max(0)
        .min(track.width as i32) as u8;
    let min_ty = ((cam_y - half_h) / TILE).max(0) as u8;
    let max_ty = (((cam_y + half_h) / TILE) + 1)
        .max(0)
        .min(track.height as i32) as u8;

    // Zoom is applied here and to every rectangle below, so the road scales with
    // the sprites drawn by `Camera::world_to_screen`.
    let z = camera.zoom.raw();
    let kx = |v: i32| -> i16 {
        (((v as i64 * z as i64) >> FP_SHIFT) as i32).clamp(-32000, 32000) as i16
    };
    let kw =
        |v: i32| -> u16 { (((v as i64 * z as i64) >> FP_SHIFT) as i32).clamp(1, 32000) as u16 };
    let tile_sz = kw(TILE);

    for ty in min_ty..max_ty {
        for tx in min_tx..max_tx {
            let tile = track.tile_at(tx, ty);
            let tile_world_x = (tx as i32) * TILE;
            let tile_world_y = (ty as i32) * TILE;

            let screen_x = 160 + kx(tile_world_x - cam_x);
            let screen_y = 120 + kx(tile_world_y - cam_y);

            // Inspect orthogonal neighbor surfaces to determine road corridor flow
            let north_is_corr = ty > 0 && is_corridor(track.tile_at(tx, ty - 1));
            let south_is_corr = ty + 1 < track.height && is_corridor(track.tile_at(tx, ty + 1));
            let west_is_corr = tx > 0 && is_corridor(track.tile_at(tx - 1, ty));
            let east_is_corr = tx + 1 < track.width && is_corridor(track.tile_at(tx + 1, ty));

            let vert_flow = north_is_corr || south_is_corr;
            let horiz_flow = west_is_corr || east_is_corr;

            match tile {
                TrackTile::Tarmac => {
                    // Dark asphalt road base
                    gpu::draw_rect_flat(
                        screen_x, screen_y, tile_sz, tile_sz, pal.road.0, pal.road.1, pal.road.2,
                    );

                    // Road texture grain / racing groove
                    let hash = ((tx as i32 * 7) ^ (ty as i32 * 13)) & 3;
                    if hash == 0 {
                        gpu::draw_rect_flat(
                            screen_x + kx(8),
                            screen_y + kx(8),
                            kw(48),
                            kw(48),
                            34,
                            36,
                            42,
                        );
                    }

                    // Dashed road centerline along the center lane of the road
                    let is_vert_lane = vert_flow && west_is_corr && east_is_corr;
                    let is_horiz_lane = horiz_flow && north_is_corr && south_is_corr;
                    let is_corridor_vert = vert_flow && !horiz_flow;
                    let is_corridor_horiz = horiz_flow && !vert_flow;

                    if is_corridor_vert || (is_vert_lane && !is_horiz_lane) {
                        // Continuous dashed stripe down the road center
                        gpu::draw_rect_flat(
                            screen_x + kx(30),
                            screen_y + kx(8),
                            kw(4),
                            kw(48),
                            220,
                            220,
                            230,
                        );
                    } else if is_corridor_horiz || (is_horiz_lane && !is_vert_lane) {
                        // Continuous dashed stripe across the road center
                        gpu::draw_rect_flat(
                            screen_x + kx(8),
                            screen_y + kx(30),
                            kw(48),
                            kw(4),
                            220,
                            220,
                            230,
                        );
                    }
                }
                TrackTile::StartFinish => {
                    // Base track surface
                    gpu::draw_rect_flat(
                        screen_x, screen_y, tile_sz, tile_sz, pal.road.0, pal.road.1, pal.road.2,
                    );

                    // Start/finish line must be perpendicular to initial car heading
                    let start_is_vert_travel =
                        ((track.start_heading.wrapping_add(512)) % 2048) < 1024;
                    if start_is_vert_travel {
                        // Across horizontal width for vertical track travel
                        for r in 0..4i32 {
                            for c in 0..8i32 {
                                let (cr, cg, cb) = if (r + c) % 2 == 0 {
                                    (240, 240, 245)
                                } else {
                                    (20, 20, 26)
                                };
                                gpu::draw_rect_flat(
                                    screen_x + kx(c * 8),
                                    screen_y + kx(16 + r * 8),
                                    kw(8),
                                    kw(8),
                                    cr,
                                    cg,
                                    cb,
                                );
                            }
                        }
                        gpu::draw_rect_flat(
                            screen_x,
                            screen_y + kx(14),
                            tile_sz,
                            kw(2),
                            255,
                            215,
                            0,
                        );
                        gpu::draw_rect_flat(
                            screen_x,
                            screen_y + kx(48),
                            tile_sz,
                            kw(2),
                            255,
                            215,
                            0,
                        );
                    } else {
                        // Across vertical height for horizontal track travel
                        for c in 0..4i32 {
                            for r in 0..8i32 {
                                let (cr, cg, cb) = if (r + c) % 2 == 0 {
                                    (240, 240, 245)
                                } else {
                                    (20, 20, 26)
                                };
                                gpu::draw_rect_flat(
                                    screen_x + kx(16 + c * 8),
                                    screen_y + kx(r * 8),
                                    kw(8),
                                    kw(8),
                                    cr,
                                    cg,
                                    cb,
                                );
                            }
                        }
                        gpu::draw_rect_flat(
                            screen_x + kx(14),
                            screen_y,
                            kw(2),
                            tile_sz,
                            255,
                            215,
                            0,
                        );
                        gpu::draw_rect_flat(
                            screen_x + kx(48),
                            screen_y,
                            kw(2),
                            tile_sz,
                            255,
                            215,
                            0,
                        );
                    }
                }
                TrackTile::Checkpoint => {
                    // Dark asphalt underlay
                    gpu::draw_rect_flat(
                        screen_x,
                        screen_y,
                        tile_sz,
                        tile_sz,
                        pal.road2.0,
                        pal.road2.1,
                        pal.road2.2,
                    );

                    if vert_flow && !horiz_flow {
                        // Luminous neon cyan timing beam across the vertical track
                        gpu::draw_rect_flat(
                            screen_x,
                            screen_y + kx(24),
                            tile_sz,
                            kw(16),
                            0,
                            180,
                            240,
                        );
                        gpu::draw_rect_flat(
                            screen_x,
                            screen_y + kx(28),
                            tile_sz,
                            kw(8),
                            120,
                            240,
                            255,
                        );
                        // Yellow timing sensor pylons on lateral edges
                        gpu::draw_rect_flat(
                            screen_x,
                            screen_y + kx(16),
                            kw(6),
                            kw(32),
                            255,
                            220,
                            20,
                        );
                        gpu::draw_rect_flat(
                            screen_x + kx(58),
                            screen_y + kx(16),
                            kw(6),
                            kw(32),
                            255,
                            220,
                            20,
                        );
                    } else {
                        // Luminous neon cyan timing beam across horizontal track
                        gpu::draw_rect_flat(
                            screen_x + kx(24),
                            screen_y,
                            kw(16),
                            tile_sz,
                            0,
                            180,
                            240,
                        );
                        gpu::draw_rect_flat(
                            screen_x + kx(28),
                            screen_y,
                            kw(8),
                            tile_sz,
                            120,
                            240,
                            255,
                        );
                        // Yellow timing sensor pylons on top and bottom
                        gpu::draw_rect_flat(
                            screen_x + kx(16),
                            screen_y,
                            kw(32),
                            kw(6),
                            255,
                            220,
                            20,
                        );
                        gpu::draw_rect_flat(
                            screen_x + kx(16),
                            screen_y + kx(58),
                            kw(32),
                            kw(6),
                            255,
                            220,
                            20,
                        );
                    }
                }
                TrackTile::Curb => {
                    // Dark asphalt underlay so curb blends with tarmac
                    gpu::draw_rect_flat(
                        screen_x, screen_y, tile_sz, tile_sz, pal.road.0, pal.road.1, pal.road.2,
                    );

                    // Alternating Red & White rumble curb blocks
                    let is_vert_curb =
                        (west_is_corr || east_is_corr) && !(north_is_corr || south_is_corr);
                    if is_vert_curb {
                        // Vertical curb bordering corridor: stripes alternate vertically
                        for i in 0..4i32 {
                            let (cr, cg, cb) = if (i + tx as i32 + ty as i32) % 2 == 0 {
                                pal.curb_a
                            } else {
                                pal.curb_b
                            };
                            gpu::draw_rect_flat(
                                screen_x,
                                screen_y + kx(i * 16),
                                tile_sz,
                                kw(16),
                                cr,
                                cg,
                                cb,
                            );
                        }
                    } else {
                        // Horizontal or corner curb: stripes alternate horizontally
                        for i in 0..4i32 {
                            let (cr, cg, cb) = if (i + tx as i32 + ty as i32) % 2 == 0 {
                                pal.curb_a
                            } else {
                                pal.curb_b
                            };
                            gpu::draw_rect_flat(
                                screen_x + kx(i * 16),
                                screen_y,
                                kw(16),
                                tile_sz,
                                cr,
                                cg,
                                cb,
                            );
                        }
                    }
                }
                TrackTile::OffRoad => {
                    // Rich emerald grass terrain
                    gpu::draw_rect_flat(
                        screen_x,
                        screen_y,
                        tile_sz,
                        tile_sz,
                        pal.grass.0,
                        pal.grass.1,
                        pal.grass.2,
                    );

                    // Subtle darker grass patches for organic texture
                    let patch = ((tx as i32 * 11) + (ty as i32 * 5)) & 3;
                    if patch == 0 {
                        gpu::draw_rect_flat(
                            screen_x + kx(12),
                            screen_y + kx(12),
                            kw(24),
                            kw(24),
                            pal.patch_a.0,
                            pal.patch_a.1,
                            pal.patch_a.2,
                        );
                    } else if patch == 1 {
                        gpu::draw_rect_flat(
                            screen_x + kx(36),
                            screen_y + kx(28),
                            kw(20),
                            kw(20),
                            pal.patch_b.0,
                            pal.patch_b.1,
                            pal.patch_b.2,
                        );
                    }
                }
                TrackTile::OilSlick => {
                    // Asphalt base
                    gpu::draw_rect_flat(
                        screen_x, screen_y, tile_sz, tile_sz, pal.road.0, pal.road.1, pal.road.2,
                    );

                    // Iridescent dark purple slick puddle
                    gpu::draw_rect_flat(
                        screen_x + kx(8),
                        screen_y + kx(8),
                        kw(48),
                        kw(48),
                        32,
                        18,
                        48,
                    );
                    gpu::draw_rect_flat(
                        screen_x + kx(14),
                        screen_y + kx(14),
                        kw(36),
                        kw(36),
                        56,
                        16,
                        78,
                    );
                    // Shimmer reflection highlight
                    gpu::draw_rect_flat(
                        screen_x + kx(20),
                        screen_y + kx(20),
                        kw(16),
                        kw(8),
                        120,
                        40,
                        160,
                    );
                }
                TrackTile::BoostPad => {
                    // Dark tarmac track bed
                    gpu::draw_rect_flat(screen_x, screen_y, tile_sz, tile_sz, 32, 34, 40);

                    // Glowing neon orange/yellow chevron booster arrows
                    let chevrons = [10i32, 24, 38];
                    if vert_flow && !horiz_flow {
                        for (idx, &cy) in chevrons.iter().enumerate() {
                            let (cr, cg) = if idx == 0 {
                                (255, 230)
                            } else if idx == 1 {
                                (255, 170)
                            } else {
                                (255, 110)
                            };
                            gpu::draw_rect_flat(
                                screen_x + kx(12),
                                screen_y + kx(cy),
                                kw(40),
                                kw(8),
                                cr,
                                cg,
                                0,
                            );
                            gpu::draw_rect_flat(
                                screen_x + kx(26),
                                screen_y + kx(cy - 4),
                                kw(12),
                                kw(4),
                                cr,
                                cg,
                                0,
                            );
                        }
                    } else {
                        // Pointing right along horizontal road
                        for (idx, &cx) in chevrons.iter().enumerate() {
                            let (cr, cg) = if idx == 0 {
                                (255, 230)
                            } else if idx == 1 {
                                (255, 170)
                            } else {
                                (255, 110)
                            };
                            gpu::draw_rect_flat(
                                screen_x + kx(cx),
                                screen_y + kx(12),
                                kw(8),
                                kw(40),
                                cr,
                                cg,
                                0,
                            );
                            gpu::draw_rect_flat(
                                screen_x + kx(cx + 4),
                                screen_y + kx(26),
                                kw(4),
                                kw(12),
                                cr,
                                cg,
                                0,
                            );
                        }
                    }
                }
                TrackTile::Barrier => {
                    // Armco steel barrier with yellow/black hazard markings
                    gpu::draw_rect_flat(screen_x, screen_y, tile_sz, tile_sz, 18, 20, 26);
                    for i in 0..4i32 {
                        let (br, bg, bb) = if (i + tx as i32 + ty as i32) % 2 == 0 {
                            (240, 200, 20) // Safety Yellow
                        } else {
                            (30, 30, 35) // Hazard Black
                        };
                        gpu::draw_rect_flat(
                            screen_x + kx(i * 16),
                            screen_y + kx(16),
                            kw(16),
                            kw(32),
                            br,
                            bg,
                            bb,
                        );
                    }
                    // Steel guardrail cap
                    gpu::draw_rect_flat(screen_x, screen_y + kx(12), tile_sz, kw(4), 180, 190, 205);
                    gpu::draw_rect_flat(screen_x, screen_y + kx(48), tile_sz, kw(4), 140, 150, 165);
                }
            }
        }
    }
}
