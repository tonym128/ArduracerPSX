//! Hardware-free VRAM layout and quantisation for the texture pipeline.
//!
//! Split from `texpipe.rs` so `tools/test_ui` can check it. `texpipe.rs` issues
//! GP0 commands and DMA transfers; everything *decidable* about where a texture
//! goes and what colour it stores lives here, where a mistake is a failing test
//! rather than a corrupted back buffer.
//!
//! This exists for TASK-1204 (no texture pipeline existed at all) and TASK-1202
//! (the minimap rebuilt a static image every frame at ~43 % of the frame budget).

use arduracer_core::TrackTile;

/// Screen width, and therefore where the framebuffers end horizontally.
pub const SCREEN_W: u16 = 320;
/// Framebuffer height; the two buffers stack to `2 * this`.
pub const FB_HEIGHT: u16 = 240;

/// Where the minimap texture slot lives: top-left of the 896-.. free
/// band at the far right of VRAM, clear of the track texture.
pub const TEXTURE_X: u16 = 896;
pub const TEXTURE_Y: u16 = 0;

/// Largest texture dimension the pipeline accepts.
///
/// 64 is one 8-texel unit either side of the 56x56 minimap panel, which keeps the
/// blit a single sprite with no sub-rectangle addressing.
pub const MAX_DIM: u16 = 64;

/// Width of the minimap panel, and its inner drawing area.
pub const MINIMAP_SIZE: u16 = 56;
pub const MINIMAP_INNER: i32 = (MINIMAP_SIZE - 8) as i32;

/// Bytes the slot costs in VRAM: 2 per pixel at 15 bits per channel.
pub const SLOT_BYTES: usize = MAX_DIM as usize * MAX_DIM as usize * 2;

/// A 15bpp page covers 256 texels on a side and sits on the 256-halfword
/// page grid VRAM is divided into at 15bpp.
pub const TRACK_PAGE_TEXELS: u16 = 256;
/// A page spans 64 rows of VRAM halfwords.
pub const TRACK_PAGE_COLS: u16 = 64;

/// The world visual is 1280x1280 texels, or 5 x 5 of 256x256 pages.
pub const TRACK_TEX_W: u16 = 1280;
pub const TRACK_TEX_H: u16 = 1280;

/// Decompressed VRAM tile dimension: 64x64 texels.
pub const VRAM_TILE_DIM: u16 = 64;
pub const TRACK_TILE_DIM: u16 = 64;

/// 1024x1024 block dimensions in texels.
pub const BLOCK_DIM: usize = 1024;
pub const TILES_PER_BLOCK_AXIS: usize = 16;
pub const TILES_PER_BLOCK: usize = 256;

/// Number of 64x64 tiles per 256x256 VRAM page (4x4 = 16).
pub const TILES_PER_TPAGE_AXIS: usize = 4;
pub const TILES_PER_TPAGE: usize = 16;

/// Number of 256x256 VRAM texture pages (4).
pub const TRACK_TPAGE_COUNT: usize = 4;

/// Total number of 64x64 VRAM cache slots across all 4 Tpages (4 * 16 = 64).
pub const TRACK_SLOT_COUNT: usize = TRACK_TPAGE_COUNT * TILES_PER_TPAGE; // 64
pub const VRAM_SLOT_COUNT: usize = TRACK_SLOT_COUNT;

/// 4 resident 256x256 Tpage origins in VRAM in 15bpp direct colour.
/// Page 0: (384, 0)
/// Page 1: (640, 0)
/// Page 2: (384, 256)
/// Page 3: (640, 256)
pub const TRACK_VRAM_SLOTS: [(u16, u16); TRACK_TPAGE_COUNT] =
    [(384, 0), (640, 0), (384, 256), (640, 256)];

/// Returns VRAM coordinates (x, y, tpage_idx, u, v) for a 64x64 slot (0..64).
pub const fn vram_slot_coords(slot: usize) -> (u16, u16, usize, u8, u8) {
    let tpage_idx = slot / TILES_PER_TPAGE;
    let sub = slot % TILES_PER_TPAGE;
    let u = (sub % TILES_PER_TPAGE_AXIS) as u16 * VRAM_TILE_DIM;
    let v = (sub / TILES_PER_TPAGE_AXIS) as u16 * VRAM_TILE_DIM;
    let (tx, ty) = TRACK_VRAM_SLOTS[tpage_idx];
    (tx + u, ty + v, tpage_idx, u as u8, v as u8)
}

/// Bytes one 64x64 tile costs in VRAM: 64 * 64 * 2 = 8,192 bytes (8 KB).
pub const TRACK_TILE_BYTES: usize = (TRACK_TILE_DIM as usize) * (TRACK_TILE_DIM as usize) * 2;

/// Where the 16-entry level-palette CLUT lives. 16 halfwords at 15 bits.
pub const TRACK_CLUT_X: u16 = 896;
pub const TRACK_CLUT_Y: u16 = 64;

/// Bytes one circuit's world visual costs uncompressed in total: 1280*1280 texels at 2
/// bytes each (15-bit direct colour BGR555).
pub const TRACK_TEX_BYTES: usize = 1280 * 1280 * 2;

/// World units per visual texel: the 2560-unit world resampled to 1280 texels.
/// 2 world units per texel (2560 / 1280 = 2).
pub const TRACK_WU_PER_TEXEL: u16 = 2;

/// Quantises an 8-bit RGB triple to the VRAM word `BBBBBGGGGGRRRRR`.
///
/// Shares its rule with `gpu::palette::to_bgr555`, so a colour that is visible in
/// a flat polygon is the same colour in a sprite. A test asserts the two agree.
pub const fn pack_bgr555(r: u8, g: u8, b: u8) -> u16 {
    (((b as u16) >> 3) << 10) | (((g as u16) >> 3) << 5) | ((r as u16) >> 3)
}

/// Pixels per tile along each axis when scaling a `w` x `h` circuit into the
/// minimap's inner area.
///
/// Never zero, even for a degenerate grid, or every tile would stack on one pixel
/// and the outline would smear.
pub const fn minimap_step(extent: u8) -> i32 {
    let divisor = if extent == 0 { 1 } else { extent as i32 };
    let step = MINIMAP_INNER / divisor;
    if step < 1 {
        1
    } else {
        step
    }
}

/// Whether the texture slot would overlap either framebuffer.
///
/// The buffers occupy X `0..SCREEN_W`, Y `0..2*FB_HEIGHT`. A slot starting at or
/// right of `SCREEN_W` is clear of both. This is the invariant that an earlier
/// draft of the pipeline got wrong: it placed the slot at Y 480, which is inside
/// the second framebuffer's Y range and would have overwritten the back buffer
/// with texture data mid-frame.
pub const fn slot_overlaps_framebuffers() -> bool {
    TEXTURE_X < SCREEN_W && TEXTURE_Y < 2 * FB_HEIGHT
}

/// The colour a tile type is drawn in on the minimap.
///
/// One match, consulted once per tile *per bake* rather than once per tile per
/// frame, which is most of why baking is cheaper.
pub const fn minimap_colour(tile: TrackTile) -> (u8, u8, u8) {
    match tile {
        TrackTile::StartFinish => (255, 255, 255),
        TrackTile::Checkpoint => (0, 210, 255),
        TrackTile::Curb => (180, 185, 195),
        TrackTile::BoostPad => (255, 160, 20),
        TrackTile::Barrier => (35, 35, 45),
        // An oil slick is a hazard with near-zero grip and was previously
        // indistinguishable from plain off-road green on the minimap -- a player
        // could not see the thing they are about to hit.
        TrackTile::OilSlick => (120, 40, 160),
        _ if tile.is_road() => (100, 110, 125),
        _ => (25, 45, 28),
    }
}

// --- Compile-time layout invariants -----------------------------------------

const _: () = assert!(TEXTURE_Y + MAX_DIM <= 512, "slot overflows VRAM height");
const _: () = assert!(TEXTURE_X + MAX_DIM <= 1024, "slot overflows VRAM width");
const _: () = assert!(
    !slot_overlaps_framebuffers(),
    "the texture slot overlaps a framebuffer and would corrupt it"
);
const _: () = assert!(MAX_DIM as usize * MAX_DIM as usize * 2 == SLOT_BYTES);

// Verify all 4 VRAM streaming Tpages:
const _: () = {
    let mut i = 0;
    while i < TRACK_TPAGE_COUNT {
        let (x, y) = TRACK_VRAM_SLOTS[i];
        assert!(x % 64 == 0, "page X must be multiple of 64");
        assert!(y == 0 || y == 256, "page Y must be 0 or 256");
        assert!(x >= SCREEN_W, "page touches framebuffer");
        assert!(x + 256 <= TEXTURE_X, "page touches minimap");
        assert!(x + 256 <= 1024, "page overflows VRAM width");
        assert!(y + 256 <= 512, "page overflows VRAM height");
        i += 1;
    }
    // Verify all 64 64x64 cache slots are within bounds
    let mut s = 0;
    while s < TRACK_SLOT_COUNT {
        let (sx, sy, tpage, u, v) = vram_slot_coords(s);
        assert!(sx >= SCREEN_W, "slot touches framebuffer");
        assert!(sx + VRAM_TILE_DIM <= TEXTURE_X, "slot touches minimap");
        assert!(sy + VRAM_TILE_DIM <= 512, "slot overflows VRAM height");
        assert!(tpage < TRACK_TPAGE_COUNT, "invalid tpage index");
        assert!((u as u16) + VRAM_TILE_DIM <= 256, "slot U overflows page");
        assert!((v as u16) + VRAM_TILE_DIM <= 256, "slot V overflows page");
        s += 1;
    }
};

// CLUT below the minimap slot, inside the same 64-column strip.
const _: () = assert!(TRACK_CLUT_X == TEXTURE_X, "CLUT and minimap diverged");
const _: () = assert!(
    TRACK_CLUT_Y >= TEXTURE_Y + MAX_DIM,
    "the CLUT row sits inside the minimap slot"
);
