//! Lightweight 5x7 ASCII Bitmap Font Blitter.
//!
//! Renders text directly using GPU filled rectangle primitives with zero
//! texture page requirements or VRAM consumption.

use psx_gpu as gpu;

/// Standard 5x7 ASCII font definitions (characters 0x20..0x5F).
/// Each column is encoded in one byte (bits 0..6).
#[rustfmt::skip]
static FONT_5X7: [[u8; 5]; 64] = [
    [0x00, 0x00, 0x00, 0x00, 0x00], // 0x20 Space
    [0x00, 0x00, 0x5F, 0x00, 0x00], // 0x21 !
    [0x00, 0x07, 0x00, 0x07, 0x00], // 0x22 "
    [0x14, 0x7F, 0x14, 0x7F, 0x14], // 0x23 #
    [0x24, 0x2A, 0x7F, 0x2A, 0x12], // 0x24 $
    [0x23, 0x13, 0x08, 0x64, 0x62], // 0x25 %
    [0x36, 0x49, 0x55, 0x22, 0x50], // 0x26 &
    [0x00, 0x05, 0x03, 0x00, 0x00], // 0x27 '
    [0x00, 0x1C, 0x22, 0x41, 0x00], // 0x28 (
    [0x00, 0x41, 0x22, 0x1C, 0x00], // 0x29 )
    [0x14, 0x08, 0x3E, 0x08, 0x14], // 0x2A *
    [0x08, 0x08, 0x3E, 0x08, 0x08], // 0x2B +
    [0x00, 0x50, 0x30, 0x00, 0x00], // 0x2C ,
    [0x08, 0x08, 0x08, 0x08, 0x08], // 0x2D -
    [0x00, 0x60, 0x60, 0x00, 0x00], // 0x2E .
    [0x20, 0x10, 0x08, 0x04, 0x02], // 0x2F /
    [0x3E, 0x41, 0x49, 0x41, 0x3E], // 0x30 0
    [0x00, 0x42, 0x7F, 0x40, 0x00], // 0x31 1
    [0x42, 0x61, 0x51, 0x49, 0x46], // 0x32 2
    [0x22, 0x41, 0x49, 0x49, 0x36], // 0x33 3
    [0x18, 0x14, 0x12, 0x7F, 0x10], // 0x34 4
    [0x27, 0x45, 0x45, 0x45, 0x39], // 0x35 5
    [0x3C, 0x4A, 0x49, 0x49, 0x30], // 0x36 6
    [0x01, 0x71, 0x09, 0x05, 0x03], // 0x37 7
    [0x36, 0x49, 0x49, 0x49, 0x36], // 0x38 8
    [0x06, 0x49, 0x49, 0x29, 0x1E], // 0x39 9
    [0x00, 0x36, 0x36, 0x00, 0x00], // 0x3A :
    [0x00, 0x56, 0x36, 0x00, 0x00], // 0x3B ;
    [0x08, 0x14, 0x22, 0x41, 0x00], // 0x3C <
    [0x14, 0x14, 0x14, 0x14, 0x14], // 0x3D =
    [0x00, 0x41, 0x22, 0x14, 0x08], // 0x3E >
    [0x02, 0x01, 0x51, 0x09, 0x06], // 0x3F ?
    [0x32, 0x49, 0x79, 0x41, 0x3E], // 0x40 @
    [0x7E, 0x11, 0x11, 0x11, 0x7E], // 0x41 A
    [0x7F, 0x49, 0x49, 0x49, 0x36], // 0x42 B
    [0x3E, 0x41, 0x41, 0x41, 0x22], // 0x43 C
    [0x7F, 0x41, 0x41, 0x41, 0x3E], // 0x44 D
    [0x7F, 0x49, 0x49, 0x49, 0x41], // 0x45 E
    [0x7F, 0x09, 0x09, 0x09, 0x01], // 0x46 F
    [0x3E, 0x41, 0x41, 0x49, 0x7A], // 0x47 G
    [0x7F, 0x08, 0x08, 0x08, 0x7F], // 0x48 H
    [0x00, 0x41, 0x7F, 0x41, 0x00], // 0x49 I
    [0x20, 0x40, 0x41, 0x3F, 0x01], // 0x4A J
    [0x7F, 0x08, 0x14, 0x22, 0x41], // 0x4B K
    [0x7F, 0x40, 0x40, 0x40, 0x40], // 0x4C L
    [0x7F, 0x02, 0x04, 0x02, 0x7F], // 0x4D M
    [0x7F, 0x04, 0x08, 0x10, 0x7F], // 0x4E N
    [0x3E, 0x41, 0x41, 0x41, 0x3E], // 0x4F O
    [0x7F, 0x09, 0x09, 0x09, 0x06], // 0x50 P
    [0x3E, 0x41, 0x51, 0x21, 0x5E], // 0x51 Q
    [0x7F, 0x09, 0x19, 0x29, 0x46], // 0x52 R
    [0x26, 0x49, 0x49, 0x49, 0x32], // 0x53 S
    [0x01, 0x01, 0x7F, 0x01, 0x01], // 0x54 T
    [0x3F, 0x40, 0x40, 0x40, 0x3F], // 0x55 U
    [0x1F, 0x20, 0x40, 0x20, 0x1F], // 0x56 V
    [0x3F, 0x40, 0x30, 0x40, 0x3F], // 0x57 W
    [0x63, 0x14, 0x08, 0x14, 0x63], // 0x58 X
    [0x07, 0x08, 0x70, 0x08, 0x07], // 0x59 Y
    [0x61, 0x51, 0x49, 0x45, 0x43], // 0x5A Z
    [0x00, 0x7F, 0x41, 0x41, 0x00], // 0x5B [
    [0x02, 0x04, 0x08, 0x10, 0x20], // 0x5C \
    [0x00, 0x41, 0x41, 0x7F, 0x00], // 0x5D ]
    [0x04, 0x02, 0x01, 0x02, 0x04], // 0x5E ^
    [0x40, 0x40, 0x40, 0x40, 0x40], // 0x5F _
];

static mut VRAM_FONT_INITIALIZED: bool = false;

const FONT_VRAM_X: u16 = 896;
const FONT_VRAM_Y: u16 = 128;
const FONT_CLUT_X: u16 = 896;
const FONT_CLUT_Y: u16 = 66;

/// Uploads the 5x7 ASCII font sheet into VRAM at (896, 128) with a 16-entry 4bpp CLUT (`NO1-035`).
///
/// Arranges all 64 characters (0x20..=0x5F) into an 8x8 grid of 8x8 pixel cells.
/// Once initialized, single-scale HUD strings blit in 1 GPU sprite command per glyph.
pub fn init_vram_font() {
    let mut clut = [0u16; 16];
    clut[0] = 0x0000; // Transparent
    clut[1] = 0x7FFF; // White foreground
    clut[2] = 0x1084; // Dark drop shadow
    clut[3] = 0x7FFF; // Foreground fallback

    // 64x64 4bpp texture sheet = 16 halfwords per row * 64 rows = 1,024 halfwords
    let mut sheet = [0u16; 1024];

    // Pass 1: populate all foreground pixels (index 1)
    for (idx, &cols) in FONT_5X7.iter().enumerate() {
        let cell_col = idx % 8; // 0..8
        let cell_row = idx / 8; // 0..8

        for (col_idx, &col_bits) in cols.iter().enumerate() {
            let px = cell_col * 8 + col_idx; // 0..64
            let hw_col = px / 4;
            let nibble = (px % 4) as u16;

            for row in 0..7 {
                let py = cell_row * 8 + row; // 0..64
                if (col_bits & bit_mask(row as u16)) != 0 {
                    let hw_idx = py * 16 + hw_col;
                    // Pixel foreground index 1
                    sheet[hw_idx] |= 1u16 << (nibble * 4);
                }
            }
        }
    }

    // Pass 2: add subtle 1px drop shadow at index 2 strictly to unoccupied pixels
    for (idx, &cols) in FONT_5X7.iter().enumerate() {
        let cell_col = idx % 8; // 0..8
        let cell_row = idx / 8; // 0..8

        for (col_idx, &col_bits) in cols.iter().enumerate() {
            let px = cell_col * 8 + col_idx;

            for row in 0..7 {
                if (col_bits & bit_mask(row as u16)) != 0 {
                    let s_py = cell_row * 8 + row + 1;
                    let s_px = px + 1;
                    if s_py < (cell_row + 1) * 8 && s_px < (cell_col + 1) * 8 {
                        let s_hw = s_py * 16 + (s_px / 4);
                        let s_nib = (s_px % 4) as u16;
                        if (sheet[s_hw] & (0xF << (s_nib * 4))) == 0 {
                            sheet[s_hw] |= 2u16 << (s_nib * 4);
                        }
                    }
                }
            }
        }
    }

    psx_vram::upload_16bpp(
        psx_vram::VramRect::new(FONT_CLUT_X, FONT_CLUT_Y, 16, 1),
        &clut,
    );
    psx_vram::upload_16bpp(
        psx_vram::VramRect::new(FONT_VRAM_X, FONT_VRAM_Y, 16, 64),
        &sheet,
    );

    unsafe {
        VRAM_FONT_INITIALIZED = true;
    }
}

/// Returns whether the VRAM font sheet is resident and active.
#[inline]
pub fn is_vram_font_active() -> bool {
    unsafe { VRAM_FONT_INITIALIZED }
}

/// Draws a single ASCII character at screen position (x, y) with color (r, g, b) and scale.
pub fn draw_char(x: u16, y: u16, c: u8, color: (u8, u8, u8), scale: u16) {
    let ascii = if c.is_ascii_lowercase() {
        c - 32 // Convert lowercase to uppercase for 5x7 font
    } else {
        c
    };

    let idx = if (0x20..=0x5F).contains(&ascii) {
        (ascii - 0x20) as usize
    } else {
        0
    };

    // Fast-path: single-scale textured sprite blit from VRAM font sheet (`NO1-035`)
    if scale == 1 && is_vram_font_active() {
        let cell_col = (idx % 8) as u8;
        let cell_row = (idx / 8) as u8;
        let u = cell_col * 8;
        let v = (FONT_VRAM_Y as u8).wrapping_add(cell_row * 8);

        let tpage = psx_vram::Tpage::new(FONT_VRAM_X, 0, psx_vram::TexDepth::Bit4);
        let clut = psx_vram::Clut::new(FONT_CLUT_X, FONT_CLUT_Y);
        let mat = gpu::material::TextureMaterial::opaque(
            clut.uv_clut_word(),
            tpage.uv_tpage_word(0),
            color,
        );
        gpu::draw_sprite_material(x as i16, y as i16, 8, 8, (u, v), mat);
        return;
    }

    let cols = FONT_5X7[idx];

    // Fallback: procedural multi-rectangle blitter for scaled or uninitialized fonts
    let is_bright = color.0 > 40 || color.1 > 40 || color.2 > 40;
    if is_bright {
        let shadow_col = (10, 12, 18);
        let offset = if scale > 1 { 2 } else { 1 };
        for (col_idx, col_bits) in cols.iter().enumerate() {
            let mut row = 0u16;
            while row < 7 {
                if (col_bits & bit_mask(row)) != 0 {
                    let start_row = row;
                    while row < 7 && (col_bits & bit_mask(row)) != 0 {
                        row += 1;
                    }
                    let span = row - start_row;
                    let px = (x + offset + (col_idx as u16) * scale) as i16;
                    let py = (y + offset + start_row * scale) as i16;
                    gpu::draw_rect_flat(
                        px,
                        py,
                        scale,
                        span * scale,
                        shadow_col.0,
                        shadow_col.1,
                        shadow_col.2,
                    );
                } else {
                    row += 1;
                }
            }
        }
    }

    for (col_idx, col_bits) in cols.iter().enumerate() {
        let mut row = 0u16;
        while row < 7 {
            if (col_bits & bit_mask(row)) != 0 {
                let start_row = row;
                while row < 7 && (col_bits & bit_mask(row)) != 0 {
                    row += 1;
                }
                let span = row - start_row;
                let px = (x + (col_idx as u16) * scale) as i16;
                let py = (y + start_row * scale) as i16;
                gpu::draw_rect_flat(px, py, scale, span * scale, color.0, color.1, color.2);
            } else {
                row += 1;
            }
        }
    }
}

/// Column bit mask for glyph row `row` (0..7).
#[inline]
fn bit_mask(row: u16) -> u8 {
    1u8 << row
}

/// Draws an ASCII string at screen position (x, y) with given color and scale.
pub fn draw_text(x: u16, y: u16, text: &str, color: (u8, u8, u8), scale: u16) {
    let mut cur_x = x;
    let step = 6 * scale;
    for b in text.bytes() {
        draw_char(cur_x, y, b, color, scale);
        cur_x += step;
    }
}
