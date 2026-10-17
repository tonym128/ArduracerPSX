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
    let tpage_idx = (slot / TILES_PER_TPAGE) % TRACK_TPAGE_COUNT;
    let sub = slot % TILES_PER_TPAGE;
    let u = (sub % TILES_PER_TPAGE_AXIS) as u16 * VRAM_TILE_DIM;
    let v = (sub / TILES_PER_TPAGE_AXIS) as u16 * VRAM_TILE_DIM;
    let (tx, ty) = TRACK_VRAM_SLOTS[tpage_idx];
    (tx + u, ty + v, tpage_idx, u as u8, v as u8)
}

/// Halfwidth of a 64x64 4bpp tile in VRAM halfwords: four indices per word.
pub const TIM4_TILE_HALFWORDS_WIDE: u16 = VRAM_TILE_DIM / 4;

// --- Expanded indexed-colour caches (NO1-034) --------------------------------
//
// The 15bpp cache fills X = 384..896 with four 256-halfword-wide pages. An
// indexed page is narrower in VRAM (8bpp: 128 halfwords, 4bpp: 64 halfwords)
// while still holding 16 tiles, so the same band holds 2x the 8bpp pages and
// 4x the 4bpp pages. Codecs are never resident together (a format switch drops
// every tile), so the depths can share the band.
//
// The first four entries of each table are the original 15bpp origins, so
// slots 0..64 land exactly where they always did.

/// Number of 8bpp texture pages in the expanded cache.
pub const TIM8_TPAGE_COUNT: usize = 8;
/// 64x64 cache slots at 8bpp (8 pages x 16 tiles).
pub const TIM8_SLOT_COUNT: usize = TIM8_TPAGE_COUNT * TILES_PER_TPAGE;
/// Number of 4bpp texture pages in the expanded cache.
pub const TIM4_TPAGE_COUNT: usize = 16;
/// 64x64 cache slots at 4bpp (16 pages x 16 tiles).
pub const TIM4_SLOT_COUNT: usize = TIM4_TPAGE_COUNT * TILES_PER_TPAGE;
/// The largest slot count any codec uses; sizes the slot table.
pub const MAX_VRAM_SLOTS: usize = TIM4_SLOT_COUNT;
/// Width of one 8bpp texture page in VRAM halfwords.
pub const TIM8_PAGE_HALFWORDS: u16 = 128;

/// 8bpp page origins: the four 15bpp origins, then the free 128-halfword
/// columns beside them.
pub const TIM8_VRAM_PAGES: [(u16, u16); TIM8_TPAGE_COUNT] = [
    (384, 0),
    (640, 0),
    (384, 256),
    (640, 256),
    (512, 0),
    (768, 0),
    (512, 256),
    (768, 256),
];

/// 4bpp page origins: the eight 8bpp origins, then the 64-halfword columns
/// that remain between them.
pub const TIM4_VRAM_PAGES: [(u16, u16); TIM4_TPAGE_COUNT] = [
    (384, 0),
    (640, 0),
    (384, 256),
    (640, 256),
    (512, 0),
    (768, 0),
    (512, 256),
    (768, 256),
    (448, 0),
    (576, 0),
    (704, 0),
    (832, 0),
    (448, 256),
    (576, 256),
    (704, 256),
    (832, 256),
];

/// Cache slots available to a texture depth: 4bpp 256, 8bpp 128, 15bpp 64.
pub const fn slot_count_for_bpp(bpp: u8) -> usize {
    match bpp {
        4 => TIM4_SLOT_COUNT,
        8 => TIM8_SLOT_COUNT,
        _ => TRACK_SLOT_COUNT,
    }
}

/// The 4bpp equivalent of a VRAM cache slot's origin.
///
/// The 15bpp layout steps 64 halfwords per tile column, because a 64-texel
/// 15bpp tile *is* 64 halfwords wide. At 4bpp the same 64 texels are only 16
/// halfwords, so stepping 64 would leave three quarters of every page column
/// unused and push the last tile column off the 4bpp page (which is 64 halfwords
/// wide, not 256).
///
/// The 4x4 grid therefore packs tightly: 4 * 16 = 64 halfwords fills one page
/// column exactly, which is why 16 tiles at 4bpp and 16 at 15bpp both fill a
/// 256x256 page and the two layouts can share the same page origins.
///
/// Valid for `slot < TIM4_SLOT_COUNT` (256); slots 0..64 match the 15bpp pages.
pub const fn vram_slot_coords_tim4(slot: usize) -> (u16, u16, usize, u8, u8) {
    let tpage_idx = (slot / TILES_PER_TPAGE) % TIM4_TPAGE_COUNT;
    let sub = slot % TILES_PER_TPAGE;
    // Same texel-space offsets as the 15bpp layout: a tile is 64 texels across
    // either way. Only the VRAM halfword origin differs, by the 4bpp packing.
    let u = (sub % TILES_PER_TPAGE_AXIS) as u16 * VRAM_TILE_DIM;
    let v = (sub / TILES_PER_TPAGE_AXIS) as u16 * VRAM_TILE_DIM;
    let (tx, ty) = TIM4_VRAM_PAGES[tpage_idx];
    (tx + u / 4, ty + v, tpage_idx, u as u8, v as u8)
}

/// Halfwidth of a 64x64 8bpp tile in VRAM halfwords: two indices per word.
pub const TIM8_TILE_HALFWORDS_WIDE: u16 = VRAM_TILE_DIM / 2;

/// The 8bpp equivalent of a VRAM cache slot's origin.
///
/// An 8bpp texture page is 128 halfwords wide by 256 rows (256x256 texels, two
/// per word), so 4 tiles across at 32 halfwords each fills one page column
/// exactly (4 * 32 = 128 halfwords).
///
/// Valid for `slot < TIM8_SLOT_COUNT` (128); slots 0..64 match the 15bpp pages.
pub const fn vram_slot_coords_tim8(slot: usize) -> (u16, u16, usize, u8, u8) {
    let tpage_idx = (slot / TILES_PER_TPAGE) % TIM8_TPAGE_COUNT;
    let sub = slot % TILES_PER_TPAGE;
    let u = (sub % TILES_PER_TPAGE_AXIS) as u16 * VRAM_TILE_DIM;
    let v = (sub / TILES_PER_TPAGE_AXIS) as u16 * VRAM_TILE_DIM;
    let (tx, ty) = TIM8_VRAM_PAGES[tpage_idx];
    (tx + u / 2, ty + v, tpage_idx, u as u8, v as u8)
}

/// Bytes one 64x64 tile costs in VRAM: 64 * 64 * 2 = 8,192 bytes (8 KB).
pub const TRACK_TILE_BYTES: usize = (TRACK_TILE_DIM as usize) * (TRACK_TILE_DIM as usize) * 2;

/// Where the 16-entry level-palette CLUT lives. 16 halfwords at 15 bits.
pub const TRACK_CLUT_X: u16 = 896;
pub const TRACK_CLUT_Y: u16 = 64;

/// Where the 256-entry TIM8 palette CLUT lives: row 480 (below the dual 320x240 framebuffers,
/// clear of both framebuffers and clear of the track TPages at X=384..896).
pub const TIM8_CLUT_X: u16 = 0;
pub const TIM8_CLUT_Y: u16 = 480;
pub const TIM8_PALETTE_ENTRIES: usize = 256;
pub const TIM8_PALETTE_BYTES: usize = TIM8_PALETTE_ENTRIES * 2;

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

/// Maps a tile index on an axis of `extent` tiles to a pixel coordinate in the
/// minimap's `inner` area.
#[inline]
pub const fn minimap_tile_px(extent: u8, tile: i32, inner: i32) -> i32 {
    let e = if extent == 0 { 1 } else { extent as i32 };
    tile * inner / e
}

/// Inverse of `minimap_tile_px`: the tile whose block contains `pixel`, for the
/// bake's per-pixel downsample.
#[inline]
pub const fn minimap_px_tile(extent: u8, pixel: i32, inner: i32) -> i32 {
    let e = if extent == 0 { 1 } else { extent as i32 };
    let i = if inner == 0 { 1 } else { inner };
    pixel * e / i
}

/// Rectangular pixel extent in minimap coordinates for a given tile `(tx, ty)`
/// within a circuit of dimensions `(width, height)`.
///
/// Returns `(px, py, pw, ph)` all strictly inside `0..MINIMAP_INNER`.
/// Guaranteed non-zero width and height without leaving seams between adjacent tiles
/// or overflowing the minimap boundary regardless of circuit dimensions (up to `MAX_TRACK_DIM`).
pub const fn minimap_tile_rect(tx: u8, ty: u8, width: u8, height: u8) -> (i32, i32, i32, i32) {
    let w_div = if width == 0 { 1 } else { width as i32 };
    let h_div = if height == 0 { 1 } else { height as i32 };

    let x0 = (tx as i32 * MINIMAP_INNER) / w_div;
    let x1 = (((tx as i32 + 1) * MINIMAP_INNER) + w_div - 1) / w_div;
    let pw = if x1 > x0 { x1 - x0 } else { 1 };

    let y0 = (ty as i32 * MINIMAP_INNER) / h_div;
    let y1 = (((ty as i32 + 1) * MINIMAP_INNER) + h_div - 1) / h_div;
    let ph = if y1 > y0 { y1 - y0 } else { 1 };

    (x0, y0, pw, ph)
}

/// Converts a world coordinate in fixed point (Q20.12) to a minimap offset `0..MINIMAP_INNER`.
///
/// Scales continuously across the entire track extent (`extent_tiles * TILE_SIZE`).
pub fn minimap_world_pos(world: arduracer_core::Fixed, extent_tiles: u8) -> i16 {
    let extent = if extent_tiles == 0 {
        1
    } else {
        extent_tiles as i32
    };
    let total_world = extent * arduracer_core::TILE_SIZE;
    let units = world.to_int().clamp(0, total_world);
    ((units * MINIMAP_INNER) / total_world) as i16
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
        TrackTile::SuperBoostPad => (255, 50, 40),
        TrackTile::EngineOff => (230, 60, 120),
        TrackTile::EngineReset => (50, 230, 110),
        TrackTile::JumpRamp => (255, 220, 40),
        TrackTile::Bridge => (135, 148, 170),
        TrackTile::Tunnel => (70, 78, 92),
        TrackTile::Dirt => (165, 118, 70),
        TrackTile::Ice => (150, 220, 250),
        TrackTile::Sand => (210, 180, 95),
        TrackTile::Barrier => (35, 35, 45),
        // An oil slick is a hazard with near-zero grip and was previously
        // indistinguishable from plain off-road green on the minimap -- a player
        // could not see the thing they are about to hit.
        TrackTile::OilSlick => (120, 40, 160),
        _ if tile.is_road() => (100, 110, 125),
        _ => (25, 45, 28),
    }
}

/// Returns a default placeholder RGB colour for a region in world space.
/// Uses a subtle checker on road vs terrain for instant visual recognition.
pub const fn placeholder_tile_colour(is_road: bool, checker: bool) -> (u8, u8, u8) {
    if is_road {
        if checker {
            (108, 118, 132)
        } else {
            (96, 105, 118)
        }
    } else if checker {
        (30, 52, 34)
    } else {
        (24, 42, 28)
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

// The 4bpp slot grid has to tile its page exactly. A 4bpp texture page is 64
// halfwords wide by 256 rows (256x256 texels, four per word), so 4 tiles across
// at 16 halfwords each is what fills it -- the 15bpp grid's 64-halfword step
// would run off the right-hand page column.
const _: () = assert!(
    TILES_PER_TPAGE_AXIS as u16 * TIM4_TILE_HALFWORDS_WIDE == TRACK_PAGE_COLS,
    "4 tiles across must exactly fill a 4bpp page column"
);
const _: () = assert!(
    TILES_PER_TPAGE_AXIS as u16 * VRAM_TILE_DIM == TRACK_PAGE_TEXELS,
    "4 tiles down must exactly fill a 4bpp page column"
);

// The 8bpp slot grid has to tile its page exactly. An 8bpp texture page is 128
// halfwords wide by 256 rows (256x256 texels, two per word), so 4 tiles across
// at 32 halfwords each is what fills it.
const _: () = assert!(
    TILES_PER_TPAGE_AXIS as u16 * TIM8_TILE_HALFWORDS_WIDE == 128,
    "4 tiles across must exactly fill an 8bpp page column"
);
const _: () = assert!(
    TILES_PER_TPAGE_AXIS as u16 * VRAM_TILE_DIM == TRACK_PAGE_TEXELS,
    "4 tiles down must exactly fill an 8bpp page column"
);

// Expanded indexed caches (NO1-034): every page is a legal tpage origin, sits
// inside the X = 384..896 band (clear of framebuffers and the minimap strip),
// and no two pages of the same depth overlap.
const _: () = {
    let mut i = 0;
    while i < TIM8_TPAGE_COUNT {
        let (x, y) = TIM8_VRAM_PAGES[i];
        assert!(x % 64 == 0 && (y == 0 || y == 256), "bad 8bpp tpage origin");
        assert!(x >= TRACK_VRAM_SLOTS[0].0, "8bpp page left of cache band");
        assert!(
            x + TIM8_PAGE_HALFWORDS <= TEXTURE_X,
            "8bpp page touches minimap"
        );
        let mut j = i + 1;
        while j < TIM8_TPAGE_COUNT {
            let (x2, y2) = TIM8_VRAM_PAGES[j];
            assert!(
                y != y2 || x + TIM8_PAGE_HALFWORDS <= x2 || x2 + TIM8_PAGE_HALFWORDS <= x,
                "8bpp pages overlap"
            );
            j += 1;
        }
        // The first four must be the original 15bpp origins.
        if i < TRACK_TPAGE_COUNT {
            assert!(
                x == TRACK_VRAM_SLOTS[i].0 && y == TRACK_VRAM_SLOTS[i].1,
                "8bpp slots 0..64 moved"
            );
        }
        i += 1;
    }
    let mut i = 0;
    while i < TIM4_TPAGE_COUNT {
        let (x, y) = TIM4_VRAM_PAGES[i];
        assert!(x % 64 == 0 && (y == 0 || y == 256), "bad 4bpp tpage origin");
        assert!(x >= TRACK_VRAM_SLOTS[0].0, "4bpp page left of cache band");
        assert!(
            x + TRACK_PAGE_COLS <= TEXTURE_X,
            "4bpp page touches minimap"
        );
        let mut j = i + 1;
        while j < TIM4_TPAGE_COUNT {
            let (x2, y2) = TIM4_VRAM_PAGES[j];
            assert!(
                y != y2 || x + TRACK_PAGE_COLS <= x2 || x2 + TRACK_PAGE_COLS <= x,
                "4bpp pages overlap"
            );
            j += 1;
        }
        if i < TRACK_TPAGE_COUNT {
            assert!(
                x == TRACK_VRAM_SLOTS[i].0 && y == TRACK_VRAM_SLOTS[i].1,
                "4bpp slots 0..64 moved"
            );
        }
        i += 1;
    }
    assert!(MAX_VRAM_SLOTS >= TIM8_SLOT_COUNT && MAX_VRAM_SLOTS >= TRACK_SLOT_COUNT);
};

// All layouts must agree on the texel-space UV offsets: the renderers sample
// through `u`/`v`, and only the VRAM halfword origin may differ between depths.
// If these ever diverge, tiles upload to one place and sample from another.
const _: () = {
    let mut s = 0;
    while s < TRACK_SLOT_COUNT {
        let (_, _, tp15, u15, v15) = vram_slot_coords(s);
        let (_, _, tp4, u4, v4) = vram_slot_coords_tim4(s);
        let (_, _, tp8, u8, v8) = vram_slot_coords_tim8(s);
        assert!(
            tp15 == tp4 && u15 == u4 && v15 == v4,
            "4bpp UVs diverge from 15bpp"
        );
        assert!(
            tp15 == tp8 && u15 == u8 && v15 == v8,
            "8bpp UVs diverge from 15bpp"
        );
        s += 1;
    }
};

// A 4bpp tile's texel-space U must survive the trip to an 8-bit UV word. The
// GPU divides U by four itself, so U is in texels and the deepest tile column
// (192) plus a full 64-texel tile must still fit in 255.
const _: () = assert!(
    (TILES_PER_TPAGE_AXIS as u16 - 1) * VRAM_TILE_DIM + VRAM_TILE_DIM <= 256,
    "4bpp UV U overflows the 8-bit vertex field"
);

// CLUT below the minimap slot, inside the same 64-column strip.
const _: () = assert!(TRACK_CLUT_X == TEXTURE_X, "CLUT and minimap diverged");
const _: () = assert!(
    TRACK_CLUT_Y >= TEXTURE_Y + MAX_DIM,
    "the CLUT row sits inside the minimap slot"
);

// TIM8 CLUT layout assertions.
const _: () = assert!(
    TIM8_CLUT_X.is_multiple_of(16),
    "TIM8 CLUT X must be 16-aligned"
);
const _: () = assert!(TIM8_CLUT_X + 256 <= 1024, "TIM8 CLUT overflows VRAM width");
const _: () = assert!(
    TIM8_CLUT_Y >= 2 * FB_HEIGHT,
    "TIM8 CLUT overlaps framebuffers"
);
const _: () = assert!(TIM8_CLUT_Y < 512, "TIM8 CLUT overflows VRAM height");
const _: () = assert!(
    TIM8_CLUT_X + 256 <= TRACK_VRAM_SLOTS[0].0,
    "TIM8 CLUT touches track TPages"
);

#[cfg(test)]
mod tests {
    use super::*;
    use arduracer_core::visual_tex;

    // The 4bpp regression. A 4bpp tile is 16 VRAM halfwords across and 64 rows
    // down. The 8bpp conversion (32x32) has the right *area* and the wrong
    // shape, and the shape is what the GPU samples by, so it renders as a
    // sheared, striped track rather than a clean failure.

    #[test]
    fn tim4_tile_is_16_halfwords_wide_and_64_rows_tall() {
        assert_eq!(TIM4_TILE_HALFWORDS_WIDE, 16);
        assert_eq!(VRAM_TILE_DIM, 64);
        // Both shapes have area 1024; only one is the 4bpp one.
        assert_eq!(TIM4_TILE_HALFWORDS_WIDE as usize * 64, 1024);
        assert_eq!(32 * 32, 1024);
    }

    /// The cooker-facing constants are the ones that actually fed the upload
    /// rect, so they are checked against this layout directly. They shipped at
    /// the 8bpp figure (32x32) while everything else here said 16x64, which is
    /// why the area-only assert in `tracktex` could not see it.
    #[test]
    fn cooker_constants_agree_with_this_layout() {
        assert_eq!(
            visual_tex::TIM4_TILE_HALFWORDS_WIDE,
            TIM4_TILE_HALFWORDS_WIDE as usize
        );
        assert_eq!(visual_tex::TIM4_TILE_ROWS, VRAM_TILE_DIM as usize);
        assert_eq!(visual_tex::TIM4_TEXELS_PER_HALFWORD, 4);
        // And the area the cooker reports is the product of the two.
        assert_eq!(
            visual_tex::TIM4_TILE_HALFWORDS,
            visual_tex::TIM4_TILE_HALFWORDS_WIDE * visual_tex::TIM4_TILE_ROWS
        );
    }

    #[test]
    fn tim4_slots_tile_their_page_exactly() {
        for s in 0..TRACK_SLOT_COUNT {
            let (sx, sy, tpage, _, _) = vram_slot_coords_tim4(s);
            let (px, py) = TRACK_VRAM_SLOTS[tpage];
            assert!(
                sx + TIM4_TILE_HALFWORDS_WIDE <= px + TRACK_PAGE_COLS,
                "slot {s} runs off the right edge of its 4bpp page"
            );
            assert!(
                sy + VRAM_TILE_DIM <= py + TRACK_PAGE_TEXELS,
                "slot {s} runs off the bottom of its 4bpp page"
            );
            assert!(sx >= SCREEN_W, "slot {s} touches framebuffer");
            assert!(
                sx + TIM4_TILE_HALFWORDS_WIDE <= TEXTURE_X,
                "slot {s} touches minimap"
            );
            assert!(sy + VRAM_TILE_DIM <= 512, "slot {s} overflows VRAM height");
        }
    }

    #[test]
    fn tim4_and_15bpp_agree_on_texel_uvs_but_not_on_halfword_origins() {
        for s in 0..TRACK_SLOT_COUNT {
            let (x15, y15, tp15, u15, v15) = vram_slot_coords(s);
            let (x4, y4, tp4, u4, v4) = vram_slot_coords_tim4(s);
            // The renderers sample through u/v, so these must match exactly or a
            // tile uploads to one place and samples from another.
            assert_eq!((tp15, u15, v15), (tp4, u4, v4), "slot {s} UVs diverge");
            // Rows always match: 4bpp halves the width axis only.
            assert_eq!(y15, y4, "slot {s} rows must match");
            // Columns past the first are packed tighter.
            let col = (s % TILES_PER_TPAGE) % TILES_PER_TPAGE_AXIS;
            if col != 0 {
                assert!(x4 < x15, "slot {s}: 4bpp origin should be tighter");
            } else {
                assert_eq!(x4, x15, "slot {s}: first column shares an origin");
            }
        }
    }

    #[test]
    fn tim4_texel_u_fits_the_eight_bit_vertex_field() {
        // U is carried in texels (the GPU divides by four itself), so the
        // deepest tile column must not overflow the 8-bit UV word.
        for s in 0..TILES_PER_TPAGE {
            let (_, _, _, u, _) = vram_slot_coords_tim4(s);
            assert!(
                (u as u16) + VRAM_TILE_DIM <= 256,
                "slot {s}: U overflows the vertex field"
            );
        }
        // The deepest column starts at 192 and ends at 256, i.e. the last texel
        // is U = 255 exactly.
        let (_, _, _, u, _) = vram_slot_coords_tim4(TILES_PER_TPAGE - 1);
        assert_eq!(u, 192);
    }

    /// Every slot of an expanded cache must own a disjoint VRAM rectangle, or
    /// two resident tiles would overwrite each other.
    fn assert_slots_disjoint(
        count: usize,
        hw_wide: u16,
        coords: fn(usize) -> (u16, u16, usize, u8, u8),
    ) {
        let mut rects = [(0u16, 0u16); MAX_VRAM_SLOTS];
        for (s, rect) in rects.iter_mut().enumerate().take(count) {
            let (x, y, _, _, _) = coords(s);
            assert!(
                x >= SCREEN_W && x + hw_wide <= TEXTURE_X,
                "slot {s} x out of band"
            );
            assert!(y + VRAM_TILE_DIM <= 512, "slot {s} y overflow");
            *rect = (x, y);
        }
        for i in 0..count {
            for j in (i + 1)..count {
                let (ax, ay) = rects[i];
                let (bx, by) = rects[j];
                let overlap_x = ax < bx + hw_wide && bx < ax + hw_wide;
                let overlap_y = ay < by + VRAM_TILE_DIM && by < ay + VRAM_TILE_DIM;
                assert!(!(overlap_x && overlap_y), "slots {i} and {j} overlap");
            }
        }
    }

    #[test]
    fn expanded_tim4_cache_has_256_disjoint_slots() {
        assert_eq!(TIM4_SLOT_COUNT, 256);
        assert_slots_disjoint(
            TIM4_SLOT_COUNT,
            TIM4_TILE_HALFWORDS_WIDE,
            vram_slot_coords_tim4,
        );
    }

    #[test]
    fn expanded_tim8_cache_has_128_disjoint_slots() {
        assert_eq!(TIM8_SLOT_COUNT, 128);
        assert_slots_disjoint(
            TIM8_SLOT_COUNT,
            TIM8_TILE_HALFWORDS_WIDE,
            vram_slot_coords_tim8,
        );
    }

    #[test]
    fn expanded_caches_keep_original_slots() {
        for s in 0..TRACK_SLOT_COUNT {
            let (x15, y15, tp, u, v) = vram_slot_coords(s);
            let (x4, y4, tp4, u4, v4) = vram_slot_coords_tim4(s);
            let (x8, y8, tp8, u8_, v8) = vram_slot_coords_tim8(s);
            assert_eq!((tp, u, v), (tp4, u4, v4));
            assert_eq!((tp, u, v), (tp8, u8_, v8));
            assert_eq!(y15, y4);
            assert_eq!(y15, y8);
            // Page origin (column 0) is shared by every depth.
            let page_x = TRACK_VRAM_SLOTS[tp].0;
            assert_eq!(x4 - u4 as u16 / 4, page_x);
            assert_eq!(x8 - u8_ as u16 / 2, page_x);
            assert!(x15 >= page_x);
        }
        assert_eq!(slot_count_for_bpp(4), 256);
        assert_eq!(slot_count_for_bpp(8), 128);
        assert_eq!(slot_count_for_bpp(15), 64);
    }
}
