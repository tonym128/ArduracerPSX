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

/// The world visual is 320x320 texels, or 2 x 2 of 256x256 pages of which the
/// far row and column are only 64 texels deep.
///
/// 320 rather than a multiple of 256 because it is `GRID * VISUAL_PX` for an
/// 80-cell grid at 4 px/cell, and that is the largest multiple of 4 the RAM
/// budget affords once there is one texture per circuit: four of them are
/// `4 * 320 * 320 / 2` = 205 KB in `.rodata`, where four at 768 square would be
/// 1.15 MB against a ceiling of under a megabyte.
pub const TRACK_TEX_W: u16 = 320;
pub const TRACK_TEX_H: u16 = 320;

/// VRAM band for the world visual: x 320..400, y 0..320.
///
/// **One** band, not two. The previous 768-row image needed three 256-row page
/// stripes against VRAM's 512 rows, so the third stripe had to be folded down
/// beside the first two and `tracktex` had to know the fold. 320 rows fit inside
/// 512 outright, so the fold is gone: one rect, one `upload_bytes`, one address
/// calculation.
///
/// Width is 80 halfwords (`320 texels / 4 per halfword`), which straddles two
/// 64-halfword pages: page column 0 at x 320 and page column 1 at x 384. The
/// band ends at 400, so page column 1 is only 16 halfwords wide -- which is
/// exactly the 64 texels of the image that live there, and the rest of that
/// page is left as whatever VRAM held.
pub const TRACK_BAND1_X: u16 = 320;
pub const TRACK_BAND1_Y: u16 = 0;
pub const TRACK_BAND1_W: u16 = 80;
pub const TRACK_BAND1_H: u16 = 320;

/// Where the 16-entry level-palette CLUT lives. 16 halfwords at 15 bits.
pub const TRACK_CLUT_X: u16 = 704;
pub const TRACK_CLUT_Y: u16 = 64;

/// Bytes one circuit's world visual costs in VRAM: 320*320 texels at half a
/// byte each. Uploaded on track load, so only the active circuit's is resident.
pub const TRACK_TEX_BYTES: usize = 320 * 320 / 2;

/// World units per visual texel: the 2560-unit world resampled to 320 texels.
///
/// 8, up from 4, and that is the direct cost of shrinking the world while
/// shrinking the texture to match. At rest zoom the screen shows the world 1:1
/// (a world unit is a screen pixel), so a texel is an 8-by-8 patch of screen:
/// the track still reads as a track, with softer edges up close than before.
/// The road is 6 cells across, i.e. 24 texels, and the kerb is nearly 3, so
/// both survive the drop -- what is lost is anti-aliasing, on a palette that
/// has sixteen flat colours for it to quantise to anyway.
pub const TRACK_WU_PER_TEXEL: u16 = 8;

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

// The track texture band must not touch the framebuffers (x >= SCREEN_W, and it
// starts at 320 so the horizontal check is sufficient).
const _: () = assert!(
    TRACK_BAND1_X >= SCREEN_W,
    "track band touches a framebuffer"
);
const _: () = assert!(
    TRACK_BAND1_X + TRACK_BAND1_W <= 1024 && TRACK_BAND1_Y + TRACK_BAND1_H <= 512,
    "track band overflows VRAM"
);
// One band is enough only while the image is no taller than VRAM. This is the
// assertion that failed when the image grew past 512 rows and needed a fold; it
// is here so the next growth says so at compile time instead of at 30 fps.
const _: () = assert!(
    TRACK_TEX_H <= 512,
    "the track visual is taller than VRAM and needs a band fold again"
);
// The band must stay clear of the minimap strip.
const _: () = assert!(
    TRACK_BAND1_X + TRACK_BAND1_W <= TEXTURE_X,
    "the track band overlaps the minimap slot"
);
// CLUT below the minimap slot, inside the same 64-column strip.
const _: () = assert!(TRACK_CLUT_X == TEXTURE_X, "CLUT and minimap diverged");
const _: () = assert!(
    TRACK_CLUT_Y >= TEXTURE_Y + MAX_DIM,
    "the CLUT row sits inside the minimap slot"
);
// Upload sizing: the band must be exactly one source row tall and as wide as the
// whole image, or `upload_bytes`'s "byte count must be exactly 2 x w x h" assert
// fires at runtime on the first track load rather than here. The band is in
// halfword columns and the visual is 4bpp, so four texels per halfword.
const _: () = assert!(TRACK_BAND1_W as usize * 4 == TRACK_TEX_W as usize);
const _: () = assert!(TRACK_BAND1_H == TRACK_TEX_H);
const _: () = assert!(
    TRACK_BAND1_W as usize * TRACK_BAND1_H as usize * 2 == TRACK_TEX_BYTES,
    "the band does not cover the whole visual"
);
