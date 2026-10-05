//! Minimal 15-bit texture pipeline: upload an image to VRAM once, blit it.
//!
//! PSX VRAM stores colour as 5 bits per channel (`BBBBBGGGGGRRRRR`), 2 bytes per
//! pixel. This module owns one rectangular slot in VRAM, an upload path, and a
//! single-sprite blit.
//!
//! This exists because the game had **no** texture pipeline at all (TASK-1204):
//! zero texture pages, zero CLUTs and zero textured primitives in `game/src`,
//! with VRAM 71 % idle. Everything on screen was rebuilt from flat-shaded
//! polygons and rectangles every frame. The worst offender was the HUD minimap,
//! which walked the entire tile grid issuing one GP0 rectangle per tile -- 1,444
//! rectangles, ~7,200 GP0 words and roughly 43 % of the frame budget on the
//! padded 38x38 circuits -- to redraw an image that cannot change while the
//! circuit does not (TASK-1202).
//!
//! # VRAM accounting
//!
//! | Region | Bytes |
//! | :--- | ---: |
//! | Framebuffers (2 x 320x240 at X 0..320) | 307,200 |
//! | Minimap texture (below) | 8,192 |
//! | **Free for future use** | **~210,000** |
//!
//! The slot sits at **X 320, Y 0**, not below the framebuffers. The buffers are
//! 320 wide on a 1024-wide VRAM, so X 320..1024 is entirely unused -- 704x480
//! pixels, ~338 KiB -- while the strip *below* both buffers is only 32 rows tall
//! and could not hold a 64-pixel image at all.
//!
//! The minimap slot is one 64x64 15-bit image: `64 * 64 * 2 = 8,192` bytes. It is
//! deliberately *not* page-aligned: `Tpage` and CLUT alignment only matter for
//! CLUT-indexed textures with wrapping, and a 15-bit direct-colour sprite is
//! addressed by UV word, so an aligned rect would waste 192 KiB of VRAM to save
//! nothing. Alignment is checked at compile time by [`TextureSlot::MAX_DIM`] and
//! by the rect assertions in `VramRect::new`.

use psx_gpu as gpu;
use psx_gpu::material::{BlendMode, TextureMaterial};
use psx_vram::{upload_16bpp, TexDepth, Tpage, VramRect};

use crate::gpu::palette::Rgb;
use crate::gpu::texlayout::{
    pack_bgr555, slot_overlaps_framebuffers, MAX_DIM, TEXTURE_X, TEXTURE_Y,
};

/// A fixed-size, 15-bit, direct-colour image resident in VRAM.
///
/// Deliberately not `Copy` or `Clone`: there is exactly one slot per instance and
/// two live handles to the same VRAM would be a bug waiting to happen.
pub struct TextureSlot {
    rect: VramRect,
    tpage: Tpage,
}

/// Composition scratch, in `.bss`.
///
/// One image is composed at a time, so a single static buffer serves every slot:
/// 8 KiB of BSS instead of 8 KiB of heap. `no_std` and single-threaded, so the
/// `static mut` is sound; every access goes through [`scratch`] below.
static mut SCRATCH: [u16; (MAX_DIM * MAX_DIM) as usize] = [0; (MAX_DIM * MAX_DIM) as usize];

/// Mutable access to the composition scratch.
///
/// # Safety
/// The caller must not re-enter this while holding the returned slice. Only the
/// `compose`/`upload` methods on [`TextureSlot`] use it, and they do not nest.
fn scratch() -> &'static mut [u16] {
    // SAFETY: single-threaded bare-metal target; the only other user is
    // `TextureSlot::compose`, which finishes before returning.
    unsafe { &mut *core::ptr::addr_of_mut!(SCRATCH) }
}

impl TextureSlot {
    /// Claims a `size` x `size` slot and prepares it for upload.
    ///
    /// # Panics
    /// If `size` exceeds [`MAX_DIM`] or the slot would fall outside VRAM. Both
    /// are compile-time constants in practice; this is a boot-time call.
    pub fn new(size: u16) -> Self {
        assert!(size > 0, "TextureSlot: size must be > 0");
        assert!(
            size <= MAX_DIM,
            "TextureSlot: {size}x{size} exceeds the {MAX_DIM} pipeline limit"
        );
        let rect = VramRect::new(TEXTURE_X, TEXTURE_Y, size, size);
        let tpage = Tpage::new(TEXTURE_X, TEXTURE_Y, TexDepth::Bit15);
        TextureSlot { rect, tpage }
    }

    /// Pixel dimensions of this slot.
    pub const fn size(&self) -> u16 {
        self.rect.w
    }

    /// Begins composing a fresh image: clears the scratch buffer.
    pub fn begin_compose(&self) {
        scratch().fill(0);
    }

    /// Writes one pixel in slot-local coordinates.
    ///
    /// Out-of-range coordinates are ignored rather than panicking: the minimap
    /// derives pixel positions from tile indices and the arithmetic can land one
    /// pixel outside at the maximum track dimension.
    pub fn set_pixel(&self, x: u16, y: u16, word: u16) {
        if x >= self.rect.w || y >= self.rect.h {
            return;
        }
        let idx = y as usize * self.rect.w as usize + x as usize;
        scratch()[idx] = word;
    }

    /// Writes one pixel from an 8-bit colour.
    pub fn set_pixel_rgb(&self, x: u16, y: u16, r: u8, g: u8, b: u8) {
        self.set_pixel(x, y, pack_bgr555(r, g, b));
    }

    /// Fills an axis-aligned region, clipped to the slot.
    ///
    /// Takes a colour triple rather than three channels: eight scalar arguments
    /// is past clippy's limit and reads worse than one colour anyway.
    pub fn fill_rect(&self, x: i32, y: i32, w: i32, h: i32, colour: Rgb) {
        if w <= 0 || h <= 0 {
            return;
        }
        let word = pack_bgr555(colour.0, colour.1, colour.2);
        for py in y..(y + h) {
            for px in x..(x + w) {
                if px < 0 || py < 0 {
                    continue;
                }
                self.set_pixel(px as u16, py as u16, word);
            }
        }
    }

    /// Uploads the composed image to VRAM.
    ///
    /// This is the expensive half -- a DMA transfer -- so it is called once per
    /// circuit load, never per frame.
    pub fn upload(&self) {
        let words = self.rect.w as usize * self.rect.h as usize;
        upload_16bpp(self.rect, &scratch()[..words]);
    }

    /// Blits the whole slot to the screen at `(x, y)`.
    ///
    /// One GP0(64h) sprite packet, regardless of how many pixels it covers --
    /// the property TASK-1202 needs.
    pub fn blit(&self, x: i16, y: i16, blend: BlendMode) {
        let material =
            TextureMaterial::blended(0, self.tpage.uv_tpage_word(0), (0xFF, 0xFF, 0xFF), blend);
        gpu::draw_sprite_material(x, y, self.rect.w, self.rect.h, (0, 0), material);
    }
}

// The slot/framebuffer and VRAM-bounds invariants live in `texlayout.rs` as
// compile-time assertions, so they hold wherever the layout is defined.
const _: () = assert!(!slot_overlaps_framebuffers());
