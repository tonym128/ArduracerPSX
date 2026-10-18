//! Cooked PS1 title-screen logo texture.

use psx_gpu as gpu;
use psx_gpu::material::TextureMaterial;
use psx_vram::{upload_bytes, Clut, TexDepth, Tpage, VramRect};

const IMAGE_W: u16 = 256;
const IMAGE_H: u16 = 90;
const IMAGE_VRAM_X: u16 = 896;
const IMAGE_VRAM_Y: u16 = 256;
const PALETTE_VRAM_X: u16 = 320;
const PALETTE_VRAM_Y: u16 = 480;

const INDEXED_BYTES: &[u8; IMAGE_W as usize * IMAGE_H as usize] =
    include_bytes!("../../assets/title_bg_idx.bin");
const PALETTE_BYTES: &[u8; 512] = include_bytes!("../../assets/title_bg_palette.bin");

pub fn upload() {
    // The x=896..1024 band below the minimap/font assets is clear of track
    // pages and both framebuffers. The 8bpp image occupies one full-width page.
    upload_bytes(
        VramRect::new(IMAGE_VRAM_X, IMAGE_VRAM_Y, IMAGE_W / 2, IMAGE_H),
        INDEXED_BYTES,
    );
    upload_bytes(
        VramRect::new(PALETTE_VRAM_X, PALETTE_VRAM_Y, 256, 1),
        PALETTE_BYTES,
    );
}

pub fn draw() {
    let tpage = Tpage::new(IMAGE_VRAM_X, IMAGE_VRAM_Y, TexDepth::Bit8);
    let clut = Clut::new(PALETTE_VRAM_X, PALETTE_VRAM_Y);
    let material =
        TextureMaterial::opaque(clut.uv_clut_word(), tpage.uv_tpage_word(0), (128, 128, 128));
    gpu::draw_sprite_material(32, 36, IMAGE_W, IMAGE_H, (0, 0), material);
}
