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

/// Where the minimap texture slot lives: top-left of the 704-.. free
/// band at the far right of VRAM, clear of the track texture.
///
/// The full world-visual 4bpp texture (see below) claims the whole strip
/// x 320..704 above row 512, so the minimap moved out of its old home at
/// x 320 and now shares the far-right strip with only the 16-entry CLUT.
pub const TEXTURE_X: u16 = 704;
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

/// A 4bpp page covers 256 texels on a side and sits on the 64x256-halfword
/// page grid VRAM is divided into.
///
/// Each page is 64 halfword columns wide and 256 rows tall, so at 4bpp it
/// holds 256x256 texels. This is what caps a *window* of texture at 256
/// texels per page, and why a 15-bit window would have been seven page-tiles
/// wide: at 16bpp a page only holds 64 texels across.
pub const TRACK_PAGE_TEXELS: u16 = 256;
/// A page spans 64 rows of VRAM halfwords.
pub const TRACK_PAGE_COLS: u16 = 64;

/// The world visual is 768x768 texels, or 3 x 3 of 256x256 pages.
pub const TRACK_TEX_W: u16 = 768;
pub const TRACK_TEX_H: u16 = 768;

/// VRAM band 1 of the world visual: x 320..512, y 0..512.
///
/// Holds source rows 0..=511, all 192 halfword columns. `768 texels = 192
/// halfwords`, so one source row is exactly 384 bytes and a band row is the
/// same 384 bytes: the upload is one contiguous rect.
pub const TRACK_BAND1_X: u16 = 320;
pub const TRACK_BAND1_Y: u16 = 0;
pub const TRACK_BAND1_W: u16 = 192;
pub const TRACK_BAND1_H: u16 = 512;

/// VRAM band 2 of the world visual: x 512..704, y 0..256.
///
/// The 768-row source needs three 256-row page stripes, but VRAM is only
/// 512 rows tall, so the third stripe is folded down next to the first two
/// -- pages (0..2, 2) of the 3x3 page grid live here at page-y 0.
///
/// Why not a single 192-wide band running 768 rows? VRAM is 512 rows: a
/// texture cannot overflow the VRAM frame, and a 4bpp page never wraps its
/// V coordinate (it is 8-bit and the page is 256 rows tall), so rows past
/// 512 would be unreachable.
pub const TRACK_BAND2_X: u16 = 512;
pub const TRACK_BAND2_Y: u16 = 0;
pub const TRACK_BAND2_W: u16 = 192;
pub const TRACK_BAND2_H: u16 = 256;

/// Where the 16-entry level-palette CLUT lives. 16 halfwords at 15 bits.
pub const TRACK_CLUT_X: u16 = 704;
pub const TRACK_CLUT_Y: u16 = 64;

/// Bytes the whole world visual costs in VRAM: 768*768 texels at half a
/// byte each, split between the two bands.
pub const TRACK_TEX_BYTES: usize = 768 * 768 / 2;

/// World units per visual texel: the 3072-unit world resampled to 768 texels.
///
/// At rest zoom the screen shows the world 1:1 (a world unit is a screen
/// pixel), so a texel is a 4-by-4 patch of screen -- the track reads as
/// smooth but slightly soft in up close, and the kerb/racing-line painting
/// keeps it from looking flat.
pub const TRACK_WU_PER_TEXEL: u16 = 4;

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

// Track texture bands must not touch the framebuffers (x >= SCREEN_W, since
// the bands start at 320 the horizontal check is sufficient for band 1).
const _: () = assert!(TRACK_BAND1_X >= SCREEN_W, "band 1 touches a framebuffer");
const _: () = assert!(TRACK_BAND2_X >= SCREEN_W, "band 2 touches a framebuffer");
const _: () = assert!(
    TRACK_BAND1_X + TRACK_BAND1_W <= 1024 && TRACK_BAND1_Y + TRACK_BAND1_H <= 512,
    "band 1 overflows VRAM"
);
const _: () = assert!(
    TRACK_BAND2_X + TRACK_BAND2_W <= 1024 && TRACK_BAND2_Y + TRACK_BAND2_H <= 512,
    "band 2 overflows VRAM"
);
// The bands cannot be one rect: VRAM is 512 rows, the source is 768.
const _: () = assert!(TRACK_TEX_H > 512, "the two-band split is stale");
// Band 1 must not run into band 2 or the minimap strip.
const _: () = assert!(
    TRACK_BAND1_X + TRACK_BAND1_W <= TRACK_BAND2_X,
    "band 1 overlaps band 2"
);
const _: () = assert!(
    TRACK_BAND2_X + TRACK_BAND2_W <= TEXTURE_X,
    "band 2 overlaps the minimap slot"
);
// CLUT below the minimap slot, inside the same 64-column strip.
const _: () = assert!(TRACK_CLUT_X == TEXTURE_X, "CLUT and minimap diverged");
const _: () = assert!(
    TRACK_CLUT_Y >= TEXTURE_Y + MAX_DIM,
    "the CLUT row sits inside the minimap slot"
);
// Upload sizes: each band row is exactly one source row of 384 bytes, and the
// byte totals must add up to the whole image.
const _: () = assert!(TRACK_TEX_W as usize / 2 == TRACK_BAND1_W as usize * 2);
const _: () = assert!(
    TRACK_BAND1_W as usize * TRACK_BAND1_H as usize * 2
        + TRACK_BAND2_W as usize * TRACK_BAND2_H as usize * 2
        == TRACK_TEX_BYTES,
    "the two bands do not tile the whole visual"
);
