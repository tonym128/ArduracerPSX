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
//! see `psx_vram::upload_bytes`), and a 128 KB tile upload is roughly
//! 13 ms of CPU at release rates -- that *is* the frame budget, and it
//! would hitch every time the camera re-centred. The 3072-world at 4bpp
//! is 768x768 texels = 288 KB, which *fits* VRAM: ~300 KB of
//! framebuffers + 288 KB of track + 8 KB of minimap ≈ 596 KB of 1 MB.
//! One upload at track load costs the same ~6-13 ms once, and then the
//! per-frame cost is zero -- a UV window re-selected per quad instead of
//! a VRAM re-upload. "Streaming" therefore happens in the *addressing*
//! (a UV rect into the resident world texture), not in DMA.
//!
//! *Split across two VRAM bands.* A 4bpp page is 256 texels tall, and VRAM
//! is only 512 rows, so the 768-row world is two page stripes. VRAM's
//! page grid only has rows y 0..256 and 256..512, so the third stripe of
//! the 3x3 page grid is laid out *beside* the first two rather than below
//! them: band 1 at x 320..512, y 0..512 holds source rows 0..=511, and
//! band 2 at x 512..704, y 0..256 holds rows 512..=767. The per-tile
//! page lookup (`page_for`) encodes that fold so the renderer can treat
//! each of the 3x3 source tiles as if VRAM had three page rows.
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

use crate::gpu::camera::Camera;
use psx_gpu as gpu;
use psx_gpu::material::TextureMaterial;
use psx_vram::{upload_bytes, upload_clut, Clut, Color555, TexDepth, Tpage, VramRect};

/// Source texels are 4x4 world units (3072 world units / 768 texels).
const WU_PER_TEXEL: i32 = 4;
/// One source tile of the page grid: 256 texels.
const TILE_TEX: i32 = 256;

/// The palette CLUT slot. 16 halfwords, 32 bytes; sits directly under the
/// minimap slot at the far right of VRAM.
const CLUT: Clut = Clut::new(704, 64);

/// Maps a source tile coordinate to its VRAM page.
///
/// The source is a 3x3 grid of 256-texel tiles; the world's VRAM footprint
/// is folded into band 1 (rows 0..=511 at x 320..512) and band 2 (rows
/// 512..=767 at x 512..704). Page (px, py) therefore sits at:
///
/// * `py < 2` -> VRAM page at `(320 + px*64, py*256)` in halfword columns,
/// * `py == 2` -> VRAM page at `(512 + px*64, 0)`.
///
/// Every returned coordinate is a valid `Tpage` origin: x is a multiple of
/// 64, and y is 0 or 256 by construction.
fn page_for(px: usize, py: usize) -> Tpage {
    if py < 2 {
        Tpage::new(320 + (px as u16) * 64, (py as u16) * 256, TexDepth::Bit4)
    } else {
        Tpage::new(512 + (px as u16) * 64, 0, TexDepth::Bit4)
    }
}

/// Uploads the world visual (and its CLUT) to VRAM.
///
/// Call once per track load -- the image is static for a whole race, like
/// the minimap bake. Two `upload_bytes` rects: one per band, byte-for-byte
/// the corresponding slice of the cooker-packed source, because the CC50
/// upload copies rows of `rect.w` halfwords straight off the source.
///
/// This is a FIFO transfer of 147,456 words: the release-profile cost is
/// roughly 6-13 ms, once per load. Deliberately *not* DMA: the dma
/// helper refuses rows wider than 16 words (silicon guard), and our rows
/// are 96 words, so the FIFO path is the only correct one on this target.
pub fn init_track_texture() {
    let mut clut = [Color555::BLACK; 16];
    for (i, &(r, g, b)) in visual_tex::CLUT_RGB.iter().enumerate() {
        clut[i] = Color555::rgb8(r, g, b);
    }
    upload_clut(CLUT, &clut);
    // Band 1: source rows 0..=511, one 384-byte row per VRAM row.
    upload_bytes(
        VramRect::new(320, 0, 192, 512),
        &visual_tex::PACKED[..512 * 384],
    );
    // Band 2: source rows 512..=767.
    upload_bytes(
        VramRect::new(512, 0, 192, 256),
        &visual_tex::PACKED[512 * 384..],
    );
}

/// Draws the visible world window as textured quads, one per source tile.
///
/// Signature kept from `tile_blitter::render_track` so the call sites in
/// the race loop and the pause veil do not change; `track` is only used
/// for the world extents.
///
/// The PSX textured quad maps a quad in affine fashion; over an
/// axis-aligned screen rect that is exactly what a uniform zoom needs, so
/// one quad per tile is enough -- no subdivision, no perspective divide.
pub fn render_track(track: &TrackDef, camera: &Camera) {
    let (half_w, half_h) = camera.visible_half_extents();
    let cam_x = camera.pos.x.to_int();
    let cam_y = camera.pos.y.to_int();

    // The screen shows 1:1 at rest zoom, so the visible window in world
    // units maps directly to texels divided by 4. Clamped to the world so
    // a zoomed-out camera at the world edge never samples past the visual.
    let x0 = (cam_x - half_w).max(0);
    let x1 = (cam_x + half_w).min(track.world_width());
    let y0 = (cam_y - half_h).max(0);
    let y1 = (cam_y + half_h).min(track.world_height());

    for py in 0..3 {
        for px in 0..3 {
            // Source tile (px, py) covers world rect
            // [px*1024, px*1024+1024) x [py*1024, py*1024+1024).
            let wx0 = px as i32 * TILE_TEX * WU_PER_TEXEL;
            let wy0 = py as i32 * TILE_TEX * WU_PER_TEXEL;
            let tx0 = x0.max(wx0);
            let tx1 = x1.min(wx0 + TILE_TEX * WU_PER_TEXEL);
            let ty0 = y0.max(wy0);
            let ty1 = y1.min(wy0 + TILE_TEX * WU_PER_TEXEL);
            if tx0 >= tx1 || ty0 >= ty1 {
                continue;
            }

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

            // Tile-local texel coordinates. u/v are 8 bits, so a whole-tile
            // quad needs u1 = 256 -> 0, which the `min(255)` caps; one texel
            // of stretch at the seam is invisible, a wrap is not.
            let u0 = ((tx0 - wx0) / WU_PER_TEXEL).clamp(0, 255) as u8;
            let u1 = ((tx1 - wx0) / WU_PER_TEXEL).min(255) as u8;
            let v0 = ((ty0 - wy0) / WU_PER_TEXEL).clamp(0, 255) as u8;
            let v1 = ((ty1 - wy0) / WU_PER_TEXEL).min(255) as u8;

            let tpage = page_for(px, py);
            let material = TextureMaterial::opaque(
                CLUT.uv_clut_word(),
                tpage.uv_tpage_word(0),
                (0x80, 0x80, 0x80),
            );
            gpu::draw_quad_textured_material(
                [tl, tr, bl, br],
                [(u0, v0), (u1, v0), (u0, v1), (u1, v1)],
                material,
            );
        }
    }
}
