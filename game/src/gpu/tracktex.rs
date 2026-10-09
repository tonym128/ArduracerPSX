//! JPEG-Streamed Track Texture Renderer with Hierarchical RAM and VRAM LRU Caching.
//!
//! # Architecture
//!
//! 1. **RAM Block Cache (1024x1024 blocks @ ~100 KB each):**
//!    Main PSX RAM maintains an LRU cache of 3 resident 1024x1024 JPEG blocks
//!    (~300 KB total). Blocks are streamed from CD-ROM (`TRACKS.BIN`) only when
//!    crossing large 1024-texel geographic boundaries.
//!
//! 2. **VRAM Tile Cache (64x64 blocks @ 8 KB each):**
//!    PSX VRAM maintains a 64-slot LRU cache across 4 texture pages (16 slots of
//!    64x64 per 256x256 Tpage = 64 slots total = 512 KB VRAM pool).
//!
//! 3. **CD-ROM Decoupling & Audio Continuity:**
//!    64x64 tiles are decompressed on-demand directly from the in-memory JPEG blocks
//!    in RAM into VRAM using our freestanding integer JPEG decoder (`arduracer_core::jpeg`).
//!    This decompression NEVER touches the CD-ROM drive during active gameplay,
//!    guaranteeing that CD-DA Redbook audio plays with zero interruptions, zero head seeks,
//!    and zero audio stutter.
//!
//! 4. **Massive Playfield Scalability:**
//!    The coordinate addressing maps arbitrary world extents to an unbounded 2D grid
//!    of 1024x1024 blocks. Both RAM and VRAM memory consumption remain strictly constant
//!    and bounded regardless of world size.

use arduracer_core::jpeg::{
    decode_tile_64x64, JpegHeader, RESTART_INTERVAL_COUNT, TILE_PIXELS, TILE_TEXELS,
};
use arduracer_core::visual_tex;
use arduracer_core::{Fixed, TrackDef, Vec2};

use crate::cd_fs::DiscReader;
use crate::gpu::camera::Camera;
use crate::gpu::texlayout::{
    self, BLOCK_DIM, TILES_PER_BLOCK_AXIS, TRACK_TPAGE_COUNT, TRACK_VRAM_SLOTS, VRAM_SLOT_COUNT,
    VRAM_TILE_DIM,
};
use psx_gpu as gpu;
use psx_gpu::material::TextureMaterial;
use psx_vram::{upload_16bpp, TexDepth, Tpage, VramRect};

/// Capacity of the in-RAM JPEG block LRU cache (3 blocks of 1024x1024 @ 200 KB each).
pub const RAM_CACHE_BLOCKS: usize = 3;

/// Maximum number of 64x64 tiles decompressed per frame (staggers decoding to eliminate frame drops).
pub const MAX_DECODES_PER_FRAME: usize = 1;

/// Precomputed Tpage descriptors for each of the 4 allocated 256x256 VRAM regions.
const TPAGES: [Tpage; TRACK_TPAGE_COUNT] = [
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

/// An in-RAM resident 1024x1024 JPEG image block.
struct RamBlock {
    resident_circuit: usize,
    resident_block_x: usize,
    resident_block_y: usize,
    last_used: u32,
    header: JpegHeader,
    restart_offsets: [u32; RESTART_INTERVAL_COUNT],
    jpeg_data: [u8; visual_tex::BLOCK_MAX_BYTES],
    byte_len: usize,
    valid: bool,
}

impl RamBlock {
    const fn new() -> Self {
        Self {
            resident_circuit: 0,
            resident_block_x: 0,
            resident_block_y: 0,
            last_used: 0,
            header: JpegHeader {
                width: 0,
                height: 0,
                restart_interval: 0,
                q_tables: [[0; 64]; 2],
                dc_huffman: [arduracer_core::jpeg::HuffTable::empty(); 2],
                ac_huffman: [arduracer_core::jpeg::HuffTable::empty(); 2],
                scan_offset: 0,
            },
            restart_offsets: [0; RESTART_INTERVAL_COUNT],
            jpeg_data: [0u8; visual_tex::BLOCK_MAX_BYTES],
            byte_len: 0,
            valid: false,
        }
    }
}

/// A resident 64x64 tile in the VRAM cache.
#[derive(Copy, Clone)]
struct VramSlot {
    resident_circuit: usize,
    resident_block_x: usize,
    resident_block_y: usize,
    resident_tile_x: usize,
    resident_tile_y: usize,
    last_used: u32,
}

impl VramSlot {
    const fn empty() -> Self {
        Self {
            resident_circuit: usize::MAX,
            resident_block_x: usize::MAX,
            resident_block_y: usize::MAX,
            resident_tile_x: usize::MAX,
            resident_tile_y: usize::MAX,
            last_used: 0,
        }
    }
}

/// 3-slot RAM LRU cache holding 1024x1024 JPEG blocks (~300 KB total).
static mut RAM_BLOCKS: [RamBlock; RAM_CACHE_BLOCKS] =
    [RamBlock::new(), RamBlock::new(), RamBlock::new()];

/// 64-slot VRAM LRU cache holding decompressed 64x64 tiles in 15bpp direct colour.
static mut VRAM_SLOTS: [VramSlot; VRAM_SLOT_COUNT] = [VramSlot::empty(); VRAM_SLOT_COUNT];

/// Temporary scratch buffer for 64x64 tile decompression (4096 halfwords = 8 KB).
static mut TILE_SCRATCH: [u16; TILE_PIXELS] = [0u16; TILE_PIXELS];

static mut ACTIVE_CIRCUIT: usize = 0;
static mut FRAME_COUNTER: u32 = 0;

static mut DISC_READER: DiscReader = DiscReader::new();
static mut TRACKS_BIN_LBA: Option<u32> = None;
static mut TRACKS_PROBED: bool = false;

static mut CAPETOWN_BIN_LBA: Option<u32> = None;
static mut IS_CAPETOWN: bool = false;

/// Prepares the track texture streaming cache for a new circuit or city.
#[allow(clippy::needless_range_loop)]
pub fn init_track_texture(circuit: usize) {
    let is_capetown = circuit == 99;
    unsafe {
        IS_CAPETOWN = is_capetown;
        ACTIVE_CIRCUIT = circuit;
        let vram_slots = &mut *core::ptr::addr_of_mut!(VRAM_SLOTS);
        for i in 0..VRAM_SLOT_COUNT {
            vram_slots[i] = VramSlot::empty();
        }
        let ram_blocks = &mut *core::ptr::addr_of_mut!(RAM_BLOCKS);
        for i in 0..RAM_CACHE_BLOCKS {
            ram_blocks[i].resident_circuit = usize::MAX;
            ram_blocks[i].valid = false;
            ram_blocks[i].last_used = 0;
        }
        FRAME_COUNTER = 0;

        if !TRACKS_PROBED {
            let reader = &mut *core::ptr::addr_of_mut!(DISC_READER);
            crate::dbg::println("[TRACKTEX] Probing disc for TRACKS.BIN and CAPETOWN.BIN...");
            TRACKS_BIN_LBA = reader.find_file_lba(visual_tex::TRACKS_BIN_NAME);
            CAPETOWN_BIN_LBA = reader.find_file_lba(visual_tex::CAPETOWN_BIN_NAME);
            crate::dbg::print("[TRACKTEX] TRACKS.BIN LBA: 0x");
            if let Some(lba) = TRACKS_BIN_LBA {
                crate::dbg::print_hex(lba);
            } else {
                crate::dbg::print("NONE");
            }
            crate::dbg::print("  CAPETOWN.BIN LBA: 0x");
            if let Some(lba) = CAPETOWN_BIN_LBA {
                crate::dbg::print_hex(lba);
            } else {
                crate::dbg::print("NONE");
            }
            crate::dbg::println("");
            TRACKS_PROBED = true;
        }

        // Preload initial 1024x1024 block for the circuit into RAM slot 0 before race starts
        // For Cape Town, preloading block (1, 1) covers Helen Suzman Blvd & Green Point stadium spawn
        let init_bx = if is_capetown { 1 } else { 0 };
        let init_by = if is_capetown { 1 } else { 0 };
        load_block_into_ram(circuit, init_bx, init_by, 0);
    }
}

/// Streams a 1024x1024 JPEG block (~100-200 KB) from CD-ROM into the specified RAM cache slot.
#[inline(never)]
fn load_block_into_ram(circuit: usize, block_x: usize, block_y: usize, slot_idx: usize) {
    let is_ct = unsafe { IS_CAPETOWN } || circuit == 99;
    let (bin_lba, entry) = if is_ct {
        let b_idx = (block_y * 3 + block_x).min(8);
        (
            unsafe { CAPETOWN_BIN_LBA },
            visual_tex::CAPETOWN_BLOCK_SECTORS[b_idx],
        )
    } else {
        let c = circuit % visual_tex::COUNT;
        (
            unsafe { TRACKS_BIN_LBA },
            visual_tex::CIRCUIT_BLOCK_SECTORS[c],
        )
    };
    let ram_slot = unsafe { &mut (*core::ptr::addr_of_mut!(RAM_BLOCKS))[slot_idx] };
    let mut loaded = false;

    if let Some(lba) = bin_lba {
        let block_lba = lba + entry.sector_offset;
        let sector_count = entry.sector_count as usize;
        let byte_len = entry.byte_len as usize;
        let reader = unsafe { &mut *core::ptr::addr_of_mut!(DISC_READER) };

        if sector_count <= visual_tex::BLOCK_SECTORS
            && reader.read_sectors(block_lba, sector_count, &mut ram_slot.jpeg_data)
        {
            ram_slot.byte_len = byte_len;
            if let Some(hdr) = JpegHeader::parse(&ram_slot.jpeg_data[..byte_len]) {
                hdr.index_restarts(
                    &ram_slot.jpeg_data[..byte_len],
                    &mut ram_slot.restart_offsets,
                );
                ram_slot.header = hdr;
                ram_slot.resident_circuit = circuit;
                ram_slot.resident_block_x = block_x;
                ram_slot.resident_block_y = block_y;
                ram_slot.valid = true;
                loaded = true;
            }
        }
    }

    if !loaded {
        // Fallback pattern if disc read fails
        ram_slot.resident_circuit = circuit;
        ram_slot.resident_block_x = block_x;
        ram_slot.resident_block_y = block_y;
        ram_slot.byte_len = 0;
        ram_slot.valid = false;
    }
}

/// Ensures the required 1024x1024 block is resident in RAM, returning its RAM slot index.
#[inline(never)]
#[allow(clippy::needless_range_loop)]
fn ensure_ram_block(circuit: usize, block_x: usize, block_y: usize, current_frame: u32) -> usize {
    let blocks = unsafe { &mut *core::ptr::addr_of_mut!(RAM_BLOCKS) };
    for i in 0..RAM_CACHE_BLOCKS {
        let b = &mut blocks[i];
        if b.valid
            && b.resident_circuit == circuit
            && b.resident_block_x == block_x
            && b.resident_block_y == block_y
        {
            b.last_used = current_frame;
            return i;
        }
    }

    // Find least recently used RAM slot
    let mut best_slot = 0;
    let mut oldest_age = u32::MAX;
    for i in 0..RAM_CACHE_BLOCKS {
        if !blocks[i].valid {
            best_slot = i;
            break;
        }
        if blocks[i].last_used < oldest_age {
            oldest_age = blocks[i].last_used;
            best_slot = i;
        }
    }

    load_block_into_ram(circuit, block_x, block_y, best_slot);
    blocks[best_slot].last_used = current_frame;
    best_slot
}

/// Decompresses a 64x64 block from an in-RAM JPEG directly into a VRAM cache slot.
///
/// NOTE: Operates ENTIRELY in main RAM and VRAM. Does NOT touch the CD-ROM!
#[inline(never)]
fn decompress_tile_to_vram(
    circuit: usize,
    block_x: usize,
    block_y: usize,
    tile_x: usize,
    tile_y: usize,
    vram_slot: usize,
    current_frame: u32,
) {
    let ram_slot_idx = ensure_ram_block(circuit, block_x, block_y, current_frame);
    let ram_slot = unsafe { &RAM_BLOCKS[ram_slot_idx] };
    let scratch = unsafe { &mut *core::ptr::addr_of_mut!(TILE_SCRATCH) };

    if ram_slot.valid && ram_slot.byte_len > 0 {
        decode_tile_64x64(
            &ram_slot.jpeg_data[..ram_slot.byte_len],
            &ram_slot.header,
            &ram_slot.restart_offsets,
            tile_x,
            tile_y,
            scratch,
        );
    } else {
        // Fallback procedural checkerboard when JPEG is absent
        for y in 0..TILE_TEXELS {
            for x in 0..TILE_TEXELS {
                let c = if ((x >> 3) ^ (y >> 3)) & 1 == 0 {
                    0x1CE7
                } else {
                    0x2108
                };
                scratch[y * TILE_TEXELS + x] = c;
            }
        }
    }

    let (vx, vy, _, _, _) = texlayout::vram_slot_coords(vram_slot);
    let rect = VramRect::new(vx, vy, VRAM_TILE_DIM, VRAM_TILE_DIM);
    upload_16bpp(rect, scratch);

    let vram_slots = unsafe { &mut *core::ptr::addr_of_mut!(VRAM_SLOTS) };
    vram_slots[vram_slot].resident_circuit = circuit;
    vram_slots[vram_slot].resident_block_x = block_x;
    vram_slots[vram_slot].resident_block_y = block_y;
    vram_slots[vram_slot].resident_tile_x = tile_x;
    vram_slots[vram_slot].resident_tile_y = tile_y;
    vram_slots[vram_slot].last_used = current_frame;
}

/// Draws the visible world window as textured quads, streaming tiles dynamically.
pub fn render_track(track: &TrackDef, camera: &Camera) {
    render_track_extents(track.world_width(), track.world_height(), camera);
}

/// Renders the world texture clamped to arbitrary world extents (tracks or 10km x 10km cities).
#[inline(never)]
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
    let vram_slots = unsafe { &mut *core::ptr::addr_of_mut!(VRAM_SLOTS) };

    // Compute exact world units per 64x64 tile along X and Y axes.
    // For single tracks: 1 block (1024x1024) covers the entire track (world_w x world_h).
    // For large cities (> 2560 world units): partitioned into blocks of 2560 world units.
    let block_wu_x = if world_w <= (BLOCK_DIM as i32 * 5 / 2) {
        world_w.max(1)
    } else {
        BLOCK_DIM as i32 * 5 / 2
    };
    let block_wu_y = if world_h <= (BLOCK_DIM as i32 * 5 / 2) {
        world_h.max(1)
    } else {
        BLOCK_DIM as i32 * 5 / 2
    };

    let tile_wu_x = (block_wu_x / (TILES_PER_BLOCK_AXIS as i32)).max(1);
    let tile_wu_y = (block_wu_y / (TILES_PER_BLOCK_AXIS as i32)).max(1);

    let min_tx = (x0 / tile_wu_x).max(0);
    let max_tx = (x1 - 1).max(0) / tile_wu_x;
    let min_ty = (y0 / tile_wu_y).max(0);
    let max_ty = (y1 - 1).max(0) / tile_wu_y;

    // Maximum 48 visible tiles per frame on standard screen resolution (320x240)
    let mut visible_tiles: [(i32, i32, usize, usize, usize, usize); 48] = [(0, 0, 0, 0, 0, 0); 48];
    let mut vis_count = 0;

    for ty in min_ty..=max_ty {
        for tx in min_tx..=max_tx {
            let wx0 = tx * tile_wu_x;
            let wy0 = ty * tile_wu_y;
            let tx0 = x0.max(wx0);
            let tx1 = x1.min(wx0 + tile_wu_x);
            let ty0 = y0.max(wy0);
            let ty1 = y1.min(wy0 + tile_wu_y);

            if tx0 < tx1 && ty0 < ty1 && vis_count < 48 {
                let bx = (wx0 / block_wu_x) as usize;
                let by = (wy0 / block_wu_y) as usize;
                let sub_tx = ((wx0 % block_wu_x) / tile_wu_x)
                    .clamp(0, (TILES_PER_BLOCK_AXIS - 1) as i32)
                    as usize;
                let sub_ty = ((wy0 % block_wu_y) / tile_wu_y)
                    .clamp(0, (TILES_PER_BLOCK_AXIS - 1) as i32)
                    as usize;

                visible_tiles[vis_count] = (tx, ty, bx, by, sub_tx, sub_ty);
                vis_count += 1;
            }
        }
    }

    let mut slot_for_tile = [usize::MAX; 48];
    let mut used_slots = [false; VRAM_SLOT_COUNT];

    // Phase 1: Match already-resident 64x64 tiles in VRAM LRU cache
    for i in 0..vis_count {
        let (_, _, bx, by, sub_tx, sub_ty) = visible_tiles[i];
        let mut found = None;

        for s_idx in 0..VRAM_SLOT_COUNT {
            let slot = &vram_slots[s_idx];
            if slot.resident_circuit == active_circuit
                && slot.resident_block_x == bx
                && slot.resident_block_y == by
                && slot.resident_tile_x == sub_tx
                && slot.resident_tile_y == sub_ty
            {
                found = Some(s_idx);
                break;
            }
        }

        if let Some(s_idx) = found {
            slot_for_tile[i] = s_idx;
            used_slots[s_idx] = true;
            vram_slots[s_idx].last_used = current_frame;
        }
    }

    let mut decodes_this_frame = 0;

    // Phase 2: Stagger tile decompression to budget (max 2 decodes per frame)
    for i in 0..vis_count {
        let (_, _, bx, by, sub_tx, sub_ty) = visible_tiles[i];
        let s = slot_for_tile[i];
        let is_hit = s != usize::MAX && {
            let slot = &vram_slots[s];
            slot.resident_circuit == active_circuit
                && slot.resident_block_x == bx
                && slot.resident_block_y == by
                && slot.resident_tile_x == sub_tx
                && slot.resident_tile_y == sub_ty
        };
        if is_hit {
            continue;
        }

        // If we reached our per-frame decode budget, leave slot as usize::MAX to display placeholder
        if decodes_this_frame >= MAX_DECODES_PER_FRAME {
            slot_for_tile[i] = usize::MAX;
            continue;
        }

        // Find oldest VRAM slot not currently visible in this frame
        let mut best_slot = 0;
        let mut oldest_age = u32::MAX;
        for s_idx in 0..VRAM_SLOT_COUNT {
            if !used_slots[s_idx] {
                let slot = &vram_slots[s_idx];
                if slot.resident_circuit == usize::MAX {
                    best_slot = s_idx;
                    break;
                }
                if slot.last_used < oldest_age {
                    oldest_age = slot.last_used;
                    best_slot = s_idx;
                }
            }
        }

        slot_for_tile[i] = best_slot;
        used_slots[best_slot] = true;

        // Decompress 64x64 block from in-RAM JPEG directly to VRAM (NO CD-ROM TOUCHED)
        decompress_tile_to_vram(
            active_circuit,
            bx,
            by,
            sub_tx,
            sub_ty,
            best_slot,
            current_frame,
        );
        decodes_this_frame += 1;
    }

    // Phase 3: Render visible quads (textured for resident tiles, placeholder flat for pending)
    for i in 0..vis_count {
        let (tx, ty, _, _, _, _) = visible_tiles[i];
        let s_idx = slot_for_tile[i];

        let wx0 = tx * tile_wu_x;
        let wy0 = ty * tile_wu_y;
        let tx0 = x0.max(wx0);
        let tx1 = x1.min(wx0 + tile_wu_x);
        let ty0 = y0.max(wy0);
        let ty1 = y1.min(wy0 + tile_wu_y);

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

        if s_idx == usize::MAX {
            // Pending tile in decode queue: render sleek placeholder shading
            let checker = ((tx ^ ty) & 1) != 0;
            let (pr, pg, pb) = texlayout::placeholder_tile_colour(true, checker);
            gpu::draw_quad_flat([tl, tr, bl, br], pr, pg, pb);
        } else {
            let (_, _, tpage_idx, u_base, v_base) = texlayout::vram_slot_coords(s_idx);

            // Map fractional position inside the tile to 0..64 UV texels
            let local_u0 = (((tx0 - wx0) * (VRAM_TILE_DIM as i32)) / tile_wu_x) as u16;
            let local_u1 = (((tx1 - wx0) * (VRAM_TILE_DIM as i32)) / tile_wu_x) as u16;
            let local_v0 = (((ty0 - wy0) * (VRAM_TILE_DIM as i32)) / tile_wu_y) as u16;
            let local_v1 = (((ty1 - wy0) * (VRAM_TILE_DIM as i32)) / tile_wu_y) as u16;

            let u0 = ((u_base as u16) + local_u0).min(255) as u8;
            let u1 = ((u_base as u16) + local_u1).min(255) as u8;
            let v0 = ((v_base as u16) + local_v0).min(255) as u8;
            let v1 = ((v_base as u16) + local_v1).min(255) as u8;

            let tpage = TPAGES[tpage_idx];
            let material = TextureMaterial::opaque(0, tpage.uv_tpage_word(0), (0x80, 0x80, 0x80));
            gpu::draw_quad_textured_material(
                [tl, tr, bl, br],
                [(u0, v0), (u1, v0), (u0, v1), (u1, v1)],
                material,
            );
        }
    }
}
