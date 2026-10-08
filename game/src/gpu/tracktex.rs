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

use crate::gpu::camera::Camera;
use crate::gpu::texlayout::{TRACK_BAND1_H, TRACK_BAND1_W, TRACK_BAND1_X, TRACK_BAND1_Y};
use psx_gpu as gpu;
use psx_gpu::material::TextureMaterial;
use psx_vram::{upload_bytes, upload_clut, Clut, Color555, TexDepth, Tpage, VramRect};

/// Source texels are 8x8 world units (2560 world units / 320 texels).
///
/// Shared with `texlayout::TRACK_WU_PER_TEXEL` rather than restated: this is the
/// one number that has to agree between the cooker that drew the texture and the
/// renderer that samples it, and a private copy is how the two drifted last time
/// -- the cooker dropped a resolution step, the renderer's UV maths did not, and
/// the result was a track that looked correct and was half the size it claimed.
const WU_PER_TEXEL: i32 = crate::gpu::texlayout::TRACK_WU_PER_TEXEL as i32;

/// One source tile of the page grid: 256 texels.
const TILE_TEX: i32 = 256;

/// Source tiles per axis: 320 texels is 256 + 64, so two tiles, the second only
/// a quarter full.
const TILES: usize = 2;

/// The palette CLUT slot. 16 halfwords, 32 bytes; sits directly under the
/// minimap slot at the far right of VRAM.
const CLUT: Clut = Clut::new(704, 64);

/// Maps a source tile coordinate to its VRAM page.
///
/// Plain arithmetic: tile (px, py) is the VRAM page at
/// `(TRACK_BAND1_X + px * 64, py * 256)` in halfword columns. There is no fold --
/// see the module docs on why the 768-row image needed one and this does not.
///
/// Every returned coordinate is a valid `Tpage` origin: x is a multiple of
/// 64 (320 and 384 both are), and y is 0 or 256 by construction.
fn page_for(px: usize, py: usize) -> Tpage {
    Tpage::new(
        TRACK_BAND1_X + (px as u16) * 64,
        (py as u16) * 256,
        TexDepth::Bit4,
    )
}

/// Uploads one circuit's world visual (and the CLUT) to VRAM.
///
/// Call once per track load -- the image is static for a whole race, like the
/// minimap bake. `circuit` is the index into `visual_tex::PACKED`, which the
/// caller gets from `levels::ALL_TRACK_VISUALS` for the slot it loaded.
///
/// One `upload_bytes` rect: byte-for-byte the cooker-packed source, because the
/// CC50 upload copies rows of `rect.w` halfwords straight off the source and the
/// source rows are exactly `TRACK_TEX_W / 2` bytes.
///
/// This is a FIFO transfer of 12,800 words: a few milliseconds at release rates,
/// once per load. Deliberately *not* DMA: the dma helper refuses rows wider than
/// 16 words (silicon guard), and our rows are 40 words, so the FIFO path is the
/// only correct one on this target.
///
/// The index is asserted rather than trusted because the failure it guards is
/// silent: out of range would panic in a `no_std` game with no unwinding, on the
/// first track load, at whatever point in boot that happens to be.
pub fn init_track_texture(circuit: usize) {
    assert!(
        circuit < visual_tex::COUNT,
        "init_track_texture: circuit {circuit} out of range ({} authored)",
        visual_tex::COUNT
    );
    let mut clut = [Color555::BLACK; 16];
    for (i, &(r, g, b)) in visual_tex::CLUT_RGB.iter().enumerate() {
        clut[i] = Color555::rgb8(r, g, b);
    }
    upload_clut(CLUT, &clut);
    upload_bytes(
        VramRect::new(TRACK_BAND1_X, TRACK_BAND1_Y, TRACK_BAND1_W, TRACK_BAND1_H),
        &visual_tex::PACKED[circuit],
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
    render_track_extents(track.world_width(), track.world_height(), camera);
}

/// Renders the world texture clamped to arbitrary world extents (e.g. tracks or cities).
pub fn render_track_extents(world_w: i32, world_h: i32, camera: &Camera) {
    let (half_w, half_h) = camera.visible_half_extents();
    let cam_x = camera.pos.x.to_int();
    let cam_y = camera.pos.y.to_int();

    // The screen shows 1:1 at rest zoom, so the visible window in world
    // units maps directly to texels divided by 4. Clamped to the world so
    // a zoomed-out camera at the world edge never samples past the visual.
    let x0 = (cam_x - half_w).max(0);
    let x1 = (cam_x + half_w).min(world_w);
    let y0 = (cam_y - half_h).max(0);
    let y1 = (cam_y + half_h).min(world_h);

    for py in 0..TILES {
        for px in 0..TILES {
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
