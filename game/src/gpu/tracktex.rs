//! Per-pixel track renderer.
//!
//! The old `tile_blitter.rs` drew the circuit as one flat GP0 rectangle per
//! 32x32 surface cell -- the world is 96x96 cells by construction, so the
//! road was a 32-world-unit staircase: blocky borders, no kerb painting, no
//! racing line. This module replaces that layer with a genuine textured
//! quad: the whole world visual, baked at per-pixel resolution into one
//! 4-bit CLUT texture in VRAM, sampled a window at a time.
//!
//! # Design, and why not the other candidates
//!
//! *4bpp, not 15bpp.* The level palette has exactly 16 colours, so 4bpp
//! carries the visual losslessly; a 15bpp texture would cost 2 bytes per
//! texel and still not help. More decisively, the hardware caps a single
//! texture page at 256 texels wide in 4bpp but only 64 in 15bpp -- so a
//! wide direct-colour window (like the one the first draft planned) needs
//! seven page-tiles across, and a wide 4bpp one needs only three. The
//! CLUT is 16 halfwords and free.
//!
//! *Whole world resident, not a streamed window.* The task sketch was a
//! 15-bit window sized to the visible rect, re-uploaded as the camera
//! moves. The upload path is GP0 FIFO word writes (the DMA path is
//! opt-in only because a Clayton-s silicon erratum can wedge channel 2;
//! see `psx_vram::upload_bytes`), and a 51 KB tile upload is a few ms
//! of CPU at release rates -- it would hitch every time the camera
//! re-centred. The 2560-world at 4bpp is 320x320 texels = 51 KB, which
//! *fits* VRAM with room to spare: ~300 KB of framebuffers + 51 KB of
//! track + 8 KB of minimap ≈ 360 KB of 1 MB. One upload at track load
//! costs the same few ms once, and then the per-frame cost is zero -- a
//! UV window re-selected per quad instead of a VRAM re-upload.
//! "Streaming" therefore happens in the *addressing* (a UV rect into the
//! resident world texture), not in DMA.
//!
//! *One circuit resident at a time.* `visual_tex::PACKED` holds a packed
//! texture for every authored circuit, but only the active one is ever
//! uploaded, so VRAM holds 51 KB rather than four times that. The RAM
//! side is the constraint that dictated the texture size: all four live
//! in `.rodata` for the whole life of the program, and at the previous
//! 768x768 they would have been 1.15 MB against a static-RAM ceiling of
//! under a megabyte.
//!
//! *One VRAM band, no fold.* A 4bpp page is 256 texels tall and VRAM is
//! 512 rows, so the old 768-row image needed three page stripes and the
//! third had to be laid out beside the first two -- band 1 at x 320..512,
//! y 0..512, band 2 at x 512..704, y 0..256 -- which `page_for` encoded.
//! The 320-row image fits inside 512 rows outright, so there is one band
//! and `page_for` is plain page arithmetic again.
//!
//! # Failure modes guarded here
//!
//! *UV wrap at the page edge.* A 4bpp primitive's U coordinate is 8 bits
//! and wraps inside its 256-texel page. No quad may span a page boundary
//! in one primitive: it would sample the page's left edge at the seam.
//! Hence one quad per 3x3 tile intersecting the visible rect, each with
//! its own `Tpage`. At a 256-texel-to-UV 1:1 fit, a quad covering the
//! whole tile would need u1 = 256, which overflows u8 -- capping at 255
//! buys one texel of stretch, which is the cheaper error than a wrap.
//!
//! *Zoom.* The visible window is `camera.visible_half_extents()` around
//! the camera centre; quads are axis-aligned in screen space, which is
//! what the camera's uniform-scale zoom produces, and the PSX quads are
//! affine-mapped over a linear screen rect, which is perspective-correct
//! for a uniform scale.
//!
//! *Culling.* The visible rect is clamped to the track bounds so a
//! zoom-out at the world edge cannot sample outside the world.

use arduracer_core::visual_tex;
use arduracer_core::{Fixed, TrackDef, Vec2};

use crate::cd_fs::DiscReader;
use crate::gpu::camera::Camera;
use crate::gpu::texlayout::{
    TRACK_SLOT_COUNT, TRACK_TILE_DIM, TRACK_VRAM_SLOTS, TRACK_WU_PER_TEXEL,
};
use psx_gpu as gpu;
use psx_gpu::material::TextureMaterial;
use psx_vram::{upload_16bpp_stream, TexDepth, Tpage, VramRect};

/// Source texels are 2x2 world units (2560 world units / 1280 texels).
const WU_PER_TEXEL: i32 = TRACK_WU_PER_TEXEL as i32;

/// One source tile of the page grid: 256 texels.
const TILE_TEX: i32 = TRACK_TILE_DIM as i32;

/// Tile world size: 256 texels * 2 world units/texel = 512 world units.
const TILE_WU: i32 = TILE_TEX * WU_PER_TEXEL;

/// Maximum sectors per compressed tile bounce buffer (64 sectors = 128 KB).
const MAX_TILE_SECTORS: usize = 64;
const TILE_BUFFER_BYTES: usize = MAX_TILE_SECTORS * 2048;

/// Precomputed Tpage descriptors for each of the 4 working VRAM slots.
const SLOT_TPAGES: [Tpage; TRACK_SLOT_COUNT] = [
    Tpage::new(
        TRACK_VRAM_SLOTS[0].0,
        TRACK_VRAM_SLOTS[0].1,
        TexDepth::Bit15,
    ),
    Tpage::new(
        TRACK_VRAM_SLOTS[1].0,
        TRACK_VRAM_SLOTS[1].1,
        TexDepth::Bit15,
    ),
    Tpage::new(
        TRACK_VRAM_SLOTS[2].0,
        TRACK_VRAM_SLOTS[2].1,
        TexDepth::Bit15,
    ),
    Tpage::new(
        TRACK_VRAM_SLOTS[3].0,
        TRACK_VRAM_SLOTS[3].1,
        TexDepth::Bit15,
    ),
];

#[derive(Copy, Clone)]
struct VramSlot {
    resident_circuit: usize,
    resident_tile: Option<usize>,
    last_used: u32,
}

/// Dynamic 4-slot VRAM LRU tile cache.
static mut SLOTS: [VramSlot; TRACK_SLOT_COUNT] = [
    VramSlot {
        resident_circuit: usize::MAX,
        resident_tile: None,
        last_used: 0,
    },
    VramSlot {
        resident_circuit: usize::MAX,
        resident_tile: None,
        last_used: 0,
    },
    VramSlot {
        resident_circuit: usize::MAX,
        resident_tile: None,
        last_used: 0,
    },
    VramSlot {
        resident_circuit: usize::MAX,
        resident_tile: None,
        last_used: 0,
    },
];

static mut ACTIVE_CIRCUIT: usize = 0;
static mut FRAME_COUNTER: u32 = 0;

/// Static zero-allocation streaming ring buffer in .bss for decompressing
/// 15-bit direct colour circuit textures directly into VRAM.
static mut STREAM_RING: [u8; visual_tex::STREAM_WINDOW] = [0u8; visual_tex::STREAM_WINDOW];

/// Scratch bounce buffer for streaming sectors off the CD disc into VRAM.
static mut TILE_BOUNCE_BUFFER: [u8; TILE_BUFFER_BYTES] = [0u8; TILE_BUFFER_BYTES];
static mut DISC_READER: DiscReader = DiscReader::new();
static mut TRACKS_BIN_LBA: Option<u32> = None;
static mut TRACKS_PROBED: bool = false;

/// Prepares the track texture streaming cache for a new circuit.
#[allow(clippy::needless_range_loop)]
pub fn init_track_texture(circuit: usize) {
    let c = circuit % visual_tex::COUNT;
    unsafe {
        ACTIVE_CIRCUIT = c;
        for i in 0..TRACK_SLOT_COUNT {
            let slot = &mut *core::ptr::addr_of_mut!(SLOTS[i]);
            slot.resident_circuit = usize::MAX;
            slot.resident_tile = None;
            slot.last_used = 0;
        }
        FRAME_COUNTER = 0;
        if !TRACKS_PROBED {
            let reader = &mut *core::ptr::addr_of_mut!(DISC_READER);
            TRACKS_BIN_LBA = reader.find_file_lba(visual_tex::TRACKS_BIN_NAME);
            TRACKS_PROBED = true;
        }
    }
}

/// Draws the visible world window as textured quads, streaming tiles dynamically.
pub fn render_track(track: &TrackDef, camera: &Camera) {
    render_track_extents(track.world_width(), track.world_height(), camera);
}

/// Renders the world texture clamped to arbitrary world extents (e.g. tracks or cities).
#[allow(clippy::needless_range_loop)]
pub fn render_track_extents(world_w: i32, world_h: i32, camera: &Camera) {
    let (half_w, half_h) = camera.visible_half_extents();
    let cam_x = camera.pos.x.to_int();
    let cam_y = camera.pos.y.to_int();

    let x0 = (cam_x - half_w).max(0);
    let x1 = (cam_x + half_w).min(world_w);
    let y0 = (cam_y - half_h).max(0);
    let y1 = (cam_y + half_h).min(world_h);

    if x0 >= x1 || y0 >= y1 {
        return;
    }

    unsafe {
        FRAME_COUNTER = FRAME_COUNTER.wrapping_add(1);
    }
    let current_frame = unsafe { FRAME_COUNTER };
    let active_circuit = unsafe { ACTIVE_CIRCUIT };

    let tile_wu = TILE_WU;
    let max_grid_x = (visual_tex::TILES_X - 1) as i32;
    let max_grid_y = (visual_tex::TILES_Y - 1) as i32;

    let min_tx = (x0 / tile_wu).clamp(0, max_grid_x) as usize;
    let max_tx = ((x1 - 1).max(0) / tile_wu).clamp(0, max_grid_x) as usize;
    let min_ty = (y0 / tile_wu).clamp(0, max_grid_y) as usize;
    let max_ty = ((y1 - 1).max(0) / tile_wu).clamp(0, max_grid_y) as usize;

    let mut visible_tiles: [(usize, usize, usize); 4] = [(0, 0, 0); 4];
    let mut vis_count = 0;
    for ty in min_ty..=max_ty {
        for tx in min_tx..=max_tx {
            let wx0 = tx as i32 * tile_wu;
            let wy0 = ty as i32 * tile_wu;
            let tx0 = x0.max(wx0);
            let tx1 = x1.min(wx0 + tile_wu);
            let ty0 = y0.max(wy0);
            let ty1 = y1.min(wy0 + tile_wu);
            if tx0 < tx1 && ty0 < ty1 && vis_count < 4 {
                visible_tiles[vis_count] = (tx, ty, ty * visual_tex::TILES_X + tx);
                vis_count += 1;
            }
        }
    }

    let mut slot_for_tile = [0usize; 4];
    let mut used_slots = [false; 4];

    // Phase 1: Match already-resident tiles in VRAM cache
    for i in 0..vis_count {
        let (_, _, tile_idx) = visible_tiles[i];
        let mut found = None;
        unsafe {
            for s_idx in 0..TRACK_SLOT_COUNT {
                let slot = &*core::ptr::addr_of!(SLOTS[s_idx]);
                if slot.resident_circuit == active_circuit && slot.resident_tile == Some(tile_idx) {
                    found = Some(s_idx);
                    break;
                }
            }
        }
        if let Some(s_idx) = found {
            slot_for_tile[i] = s_idx;
            used_slots[s_idx] = true;
            unsafe {
                let slot = &mut *core::ptr::addr_of_mut!(SLOTS[s_idx]);
                slot.last_used = current_frame;
            }
        }
    }

    // Phase 2: Allocate slots for missing tiles and stream them directly into VRAM
    let ring = unsafe { &mut *core::ptr::addr_of_mut!(STREAM_RING) };
    for i in 0..vis_count {
        let (_, _, tile_idx) = visible_tiles[i];
        let is_hit = unsafe {
            let s = slot_for_tile[i];
            let slot = &*core::ptr::addr_of!(SLOTS[s]);
            slot.resident_circuit == active_circuit && slot.resident_tile == Some(tile_idx)
        };
        if is_hit {
            continue;
        }

        let mut best_slot = 0;
        let mut oldest_age = u32::MAX;
        unsafe {
            for s_idx in 0..TRACK_SLOT_COUNT {
                if !used_slots[s_idx] {
                    let slot = &*core::ptr::addr_of!(SLOTS[s_idx]);
                    if slot.resident_tile.is_none() {
                        best_slot = s_idx;
                        break;
                    }
                    if slot.last_used < oldest_age {
                        oldest_age = slot.last_used;
                        best_slot = s_idx;
                    }
                }
            }
        }

        slot_for_tile[i] = best_slot;
        used_slots[best_slot] = true;

        let (slot_x, slot_y) = TRACK_VRAM_SLOTS[best_slot];
        let rect = VramRect::new(slot_x, slot_y, TILE_TEX as u16, TILE_TEX as u16);
        let entry = visual_tex::CIRCUIT_TILE_SECTORS[active_circuit][tile_idx];
        let bin_lba = unsafe { TRACKS_BIN_LBA };
        let mut loaded = false;

        if let Some(lba) = bin_lba {
            let tile_lba = lba + entry.sector_offset;
            let sector_count = entry.sector_count as usize;
            let byte_len = entry.byte_len as usize;
            let reader = unsafe { &mut *core::ptr::addr_of_mut!(DISC_READER) };
            let buf = unsafe { &mut *core::ptr::addr_of_mut!(TILE_BOUNCE_BUFFER) };

            if sector_count <= MAX_TILE_SECTORS && reader.read_sectors(tile_lba, sector_count, buf)
            {
                let stream = &buf[..byte_len];
                upload_16bpp_stream(rect, |emit| {
                    visual_tex::decompress_stream(
                        stream,
                        visual_tex::RAW_TILE_HALFWORDS,
                        ring,
                        |hw| {
                            emit(hw);
                        },
                    );
                });
                loaded = true;
            }
        }

        if !loaded {
            // Graceful fallback pattern when CD-ROM / TRACKS.BIN is not present
            upload_16bpp_stream(rect, |emit| {
                for y in 0..TILE_TEX {
                    for x in 0..TILE_TEX {
                        let c = if ((x >> 4) ^ (y >> 4)) & 1 == 0 {
                            0x1CE7
                        } else {
                            0x2108
                        };
                        emit(c);
                    }
                }
            });
        }

        unsafe {
            SLOTS[best_slot].resident_circuit = active_circuit;
            SLOTS[best_slot].resident_tile = Some(tile_idx);
            SLOTS[best_slot].last_used = current_frame;
        }
    }

    // Phase 3: Render visible textured quads using assigned VRAM slot Tpages
    for i in 0..vis_count {
        let (tx, ty, _) = visible_tiles[i];
        let s_idx = slot_for_tile[i];

        let wx0 = tx as i32 * tile_wu;
        let wy0 = ty as i32 * tile_wu;
        let tx0 = x0.max(wx0);
        let tx1 = x1.min(wx0 + tile_wu);
        let ty0 = y0.max(wy0);
        let ty1 = y1.min(wy0 + tile_wu);

        let tl = camera.world_to_screen(Vec2 {
            x: Fixed::from_int(tx0),
            y: Fixed::from_int(ty0),
        });
        let tr = camera.world_to_screen(Vec2 {
            x: Fixed::from_int(tx1),
            y: Fixed::from_int(ty0),
        });
        let bl = camera.world_to_screen(Vec2 {
            x: Fixed::from_int(tx0),
            y: Fixed::from_int(ty1),
        });
        let br = camera.world_to_screen(Vec2 {
            x: Fixed::from_int(tx1),
            y: Fixed::from_int(ty1),
        });

        // Tile-local UV coordinates: 0..255
        let u0 = ((tx0 - wx0) / WU_PER_TEXEL).clamp(0, 255) as u8;
        let u1 = ((tx1 - wx0) / WU_PER_TEXEL).min(255) as u8;
        let v0 = ((ty0 - wy0) / WU_PER_TEXEL).clamp(0, 255) as u8;
        let v1 = ((ty1 - wy0) / WU_PER_TEXEL).min(255) as u8;

        let tpage = SLOT_TPAGES[s_idx];
        let material = TextureMaterial::opaque(0, tpage.uv_tpage_word(0), (0x80, 0x80, 0x80));
        gpu::draw_quad_textured_material(
            [tl, tr, bl, br],
            [(u0, v0), (u1, v0), (u0, v1), (u1, v1)],
            material,
        );
    }
}
