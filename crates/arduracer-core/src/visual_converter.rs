//! Visual image bit-depth converter to in-game 8-bit colour (8bpp).
//!
//! Handles source images of arbitrary bit depth:
//! - 1-bit monochrome (bilevel: 1 bit per pixel)
//! - 2-bit and 4-bit indexed colour (paletted)
//! - 8-bit grayscale
//! - 8-bit indexed colour (paletted)
//! - 16-bit grayscale
//! - 24-bit direct colour (RGB888)
//! - 32-bit direct colour with alpha (RGBA8888)
//!
//! All source formats are converted into the in-game 8-bit colour representation:
//! - 8-bit indexed pixels (1 byte per texel, 0..=255 index into CLUT)
//! - 256-entry Color Lookup Table (CLUT) as 24-bit RGB tuples `(r, g, b)`
//!   which map directly to PSX 16-bit BGR555 entries.

#[cfg(test)]
use std::vec::Vec;

/// Standard in-game palette size for 8-bit colour.
pub const PALETTE_SIZE_8BIT: usize = 256;

/// Standard level palette core colours (16 fixed entries from LEVEL-FORMAT.md).
pub const CORE_PALETTE_16: [(u8, u8, u8); 16] = [
    (0x00, 0x00, 0x00), // 0: VOID
    (0x3C, 0x3E, 0x44), // 1: TARMAC
    (0x4A, 0x4C, 0x54), // 2: TARMAC_WORN
    (0xE8, 0xE8, 0xF0), // 3: KERB_WHITE
    (0xD8, 0x28, 0x3C), // 4: KERB_RED
    (0x2E, 0x5A, 0x34), // 5: GRASS
    (0x6B, 0x54, 0x32), // 6: GRAVEL
    (0x7A, 0x60, 0x90), // 7: SAND
    (0x8A, 0x1F, 0xB0), // 8: OIL
    (0xFF, 0x8A, 0x10), // 9: BOOST
    (0xF2, 0xF2, 0xF2), // 10: START_LINE
    (0x00, 0xD2, 0xFF), // 11: GATE
    (0x2A, 0x2E, 0x38), // 12: WALL
    (0x5A, 0x46, 0x32), // 13: TUNNEL
    (0x3A, 0x5A, 0x7A), // 14: BRIDGE
    (0xFF, 0x2E, 0x88), // 15: SCENERY
];

/// Source image pixel formats supported by the converter.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum SourceBitDepth {
    /// 1-bit monochrome (1 bpp, 0 = black, 1 = white, or 2-entry palette).
    Bit1,
    /// 2-bit indexed (2 bpp, 4-entry palette).
    Bit2,
    /// 4-bit indexed (4 bpp, 16-entry palette).
    Bit4,
    /// 8-bit grayscale (8 bpp, 0..=255 luminance).
    Bit8Gray,
    /// 8-bit indexed (8 bpp with up to 256-entry palette).
    Bit8Paletted,
    /// 16-bit grayscale (16 bpp, 0..=65535 luminance).
    Bit16Gray,
    /// 24-bit direct colour (3 bytes per pixel: R, G, B).
    Bit24Rgb,
    /// 32-bit direct colour with alpha (4 bytes per pixel: R, G, B, A).
    Bit32Rgba,
}

/// Converted 8-bit colour image representation for tests and tools.
#[cfg(test)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConvertedVisual8 {
    /// Width in texels.
    pub width: usize,
    /// Height in texels.
    pub height: usize,
    /// 8-bit pixel stream: 1 byte per texel, row-major.
    pub pixels: Vec<u8>,
    /// 256-entry colour palette (CLUT) as RGB triples.
    pub clut: [(u8, u8, u8); PALETTE_SIZE_8BIT],
}

#[cfg(test)]
impl ConvertedVisual8 {
    /// Byte length of the 8bpp texture data (`width * height`).
    #[inline]
    pub fn byte_len(&self) -> usize {
        self.pixels.len()
    }

    /// Fetches the 8-bit palette index at `(x, y)`.
    #[inline]
    pub fn pixel_at(&self, x: usize, y: usize) -> u8 {
        if x < self.width && y < self.height {
            self.pixels[y * self.width + x]
        } else {
            0
        }
    }

    /// Fetches the RGB colour of the pixel at `(x, y)`.
    #[inline]
    pub fn rgb_at(&self, x: usize, y: usize) -> (u8, u8, u8) {
        let idx = self.pixel_at(x, y) as usize;
        self.clut[idx]
    }
}

/// Squared Euclidean distance between two RGB triples.
#[inline]
pub const fn color_distance_sq(c1: (u8, u8, u8), c2: (u8, u8, u8)) -> u32 {
    let dr = (c1.0 as i32) - (c2.0 as i32);
    let dg = (c1.1 as i32) - (c2.1 as i32);
    let db = (c1.2 as i32) - (c2.2 as i32);
    (dr * dr + dg * dg + db * db) as u32
}

/// Finds the nearest color in an RGB palette to a given RGB triple.
#[inline]
pub fn nearest_palette_index(color: (u8, u8, u8), palette: &[(u8, u8, u8)]) -> u8 {
    let mut best_idx = 0u8;
    let mut best_dist = u32::MAX;
    for (i, &pal_color) in palette.iter().enumerate() {
        let dist = color_distance_sq(color, pal_color);
        if dist < best_dist {
            best_dist = dist;
            best_idx = i as u8;
            if dist == 0 {
                break;
            }
        }
    }
    best_idx
}

/// Converts raw image bytes of any supported bit depth into an 8-bit colour visual
/// directly into the provided output slice and CLUT buffer (pure `core`, `#![no_std]`-safe).
///
/// Returns the number of pixels written into `out_pixels`.
pub fn convert_to_8bit_colour_slice(
    width: usize,
    height: usize,
    depth: SourceBitDepth,
    raw_data: &[u8],
    source_palette: Option<&[(u8, u8, u8)]>,
    out_pixels: &mut [u8],
    out_clut: &mut [(u8, u8, u8); PALETTE_SIZE_8BIT],
) -> usize {
    let total_pixels = width * height;
    assert!(out_pixels.len() >= total_pixels, "output buffer too small");

    // Seed the first 16 entries with the canonical core level palette
    out_clut[..16].copy_from_slice(&CORE_PALETTE_16);

    let mut written = 0usize;

    match depth {
        SourceBitDepth::Bit1 => {
            let pal = match source_palette {
                Some(p) if p.len() >= 2 => [(p[0].0, p[0].1, p[0].2), (p[1].0, p[1].1, p[1].2)],
                _ => [(0x00, 0x00, 0x00), (0xFF, 0xFF, 0xFF)],
            };
            out_clut[0] = pal[0];
            out_clut[1] = pal[1];

            let row_bytes = width.div_ceil(8);
            for y in 0..height {
                for x in 0..width {
                    let byte_idx = y * row_bytes + (x / 8);
                    let bit_offset = 7 - (x % 8);
                    let bit = if byte_idx < raw_data.len() {
                        (raw_data[byte_idx] >> bit_offset) & 1
                    } else {
                        0
                    };
                    out_pixels[written] = bit;
                    written += 1;
                }
            }
        }

        SourceBitDepth::Bit2 => {
            if let Some(p) = source_palette {
                let n = p.len().min(4);
                out_clut[..n].copy_from_slice(&p[..n]);
            } else {
                for (i, c) in out_clut.iter_mut().enumerate().take(4) {
                    let v = (i * 85) as u8;
                    *c = (v, v, v);
                }
            }

            let row_bytes = width.div_ceil(4);
            for y in 0..height {
                for x in 0..width {
                    let byte_idx = y * row_bytes + (x / 4);
                    let shift = (3 - (x % 4)) * 2;
                    let val = if byte_idx < raw_data.len() {
                        (raw_data[byte_idx] >> shift) & 0x03
                    } else {
                        0
                    };
                    out_pixels[written] = val;
                    written += 1;
                }
            }
        }

        SourceBitDepth::Bit4 => {
            if let Some(p) = source_palette {
                let n = p.len().min(16);
                out_clut[..n].copy_from_slice(&p[..n]);
            }

            let row_bytes = width.div_ceil(2);
            for y in 0..height {
                for x in 0..width {
                    let byte_idx = y * row_bytes + (x / 2);
                    let val = if byte_idx < raw_data.len() {
                        let b = raw_data[byte_idx];
                        if x % 2 == 0 {
                            b & 0x0F
                        } else {
                            (b >> 4) & 0x0F
                        }
                    } else {
                        0
                    };
                    out_pixels[written] = val;
                    written += 1;
                }
            }
        }

        SourceBitDepth::Bit8Gray => {
            for (i, c) in out_clut.iter_mut().enumerate().take(256) {
                let v = i as u8;
                *c = (v, v, v);
            }
            let count = total_pixels.min(raw_data.len());
            out_pixels[..count].copy_from_slice(&raw_data[..count]);
            out_pixels[count..total_pixels].fill(0);
            written = total_pixels;
        }

        SourceBitDepth::Bit8Paletted => {
            if let Some(p) = source_palette {
                let n = p.len().min(256);
                out_clut[..n].copy_from_slice(&p[..n]);
            }
            let count = total_pixels.min(raw_data.len());
            out_pixels[..count].copy_from_slice(&raw_data[..count]);
            out_pixels[count..total_pixels].fill(0);
            written = total_pixels;
        }

        SourceBitDepth::Bit16Gray => {
            for (i, c) in out_clut.iter_mut().enumerate().take(256) {
                let v = i as u8;
                *c = (v, v, v);
            }
            for i in 0..total_pixels {
                let offset = i * 2;
                let val8 = if offset + 1 < raw_data.len() {
                    raw_data[offset]
                } else {
                    0
                };
                out_pixels[written] = val8;
                written += 1;
            }
        }

        SourceBitDepth::Bit24Rgb => {
            quantize_rgb_slice(width, height, raw_data, 3, out_clut, out_pixels);
            written = total_pixels;
        }

        SourceBitDepth::Bit32Rgba => {
            quantize_rgb_slice(width, height, raw_data, 4, out_clut, out_pixels);
            written = total_pixels;
        }
    }

    written
}

fn quantize_rgb_slice(
    width: usize,
    height: usize,
    raw_data: &[u8],
    bytes_per_pixel: usize,
    clut: &mut [(u8, u8, u8); PALETTE_SIZE_8BIT],
    out_pixels: &mut [u8],
) {
    let total_pixels = width * height;
    let mut palette_count = 16usize;

    for i in 0..total_pixels {
        let offset = i * bytes_per_pixel;
        if offset + 2 >= raw_data.len() {
            break;
        }

        let r = raw_data[offset];
        let g = raw_data[offset + 1];
        let b = raw_data[offset + 2];
        let color = if bytes_per_pixel >= 4 {
            let a = raw_data[offset + 3] as u32;
            (
                ((r as u32 * a) / 255) as u8,
                ((g as u32 * a) / 255) as u8,
                ((b as u32 * a) / 255) as u8,
            )
        } else {
            (r, g, b)
        };

        let mut found = false;
        for existing in &clut[..palette_count] {
            if color_distance_sq(color, *existing) <= 4 {
                found = true;
                break;
            }
        }

        if !found && palette_count < PALETTE_SIZE_8BIT {
            clut[palette_count] = color;
            palette_count += 1;
        }
    }

    for (i, pixel) in out_pixels.iter_mut().enumerate().take(total_pixels) {
        let offset = i * bytes_per_pixel;
        if offset + 2 < raw_data.len() {
            let r = raw_data[offset];
            let g = raw_data[offset + 1];
            let b = raw_data[offset + 2];
            let color = if bytes_per_pixel >= 4 {
                let a = raw_data[offset + 3] as u32;
                (
                    ((r as u32 * a) / 255) as u8,
                    ((g as u32 * a) / 255) as u8,
                    ((b as u32 * a) / 255) as u8,
                )
            } else {
                (r, g, b)
            };
            *pixel = nearest_palette_index(color, &clut[..palette_count]);
        } else {
            *pixel = 0;
        }
    }
}

/// Converts raw image bytes into a `ConvertedVisual8` structure (available in tests/std).
#[cfg(test)]
pub fn convert_to_8bit_colour(
    width: usize,
    height: usize,
    depth: SourceBitDepth,
    raw_data: &[u8],
    source_palette: Option<&[(u8, u8, u8)]>,
) -> ConvertedVisual8 {
    let mut pixels = vec![0u8; width * height];
    let mut clut = [(0u8, 0u8, 0u8); PALETTE_SIZE_8BIT];
    convert_to_8bit_colour_slice(
        width,
        height,
        depth,
        raw_data,
        source_palette,
        &mut pixels,
        &mut clut,
    );
    ConvertedVisual8 {
        width,
        height,
        pixels,
        clut,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn convert_1bit_monochrome_to_8bit_colour() {
        let raw = [0b1010_0000u8, 0b1111_0000u8];
        let converted = convert_to_8bit_colour(8, 2, SourceBitDepth::Bit1, &raw, None);

        assert_eq!(converted.width, 8);
        assert_eq!(converted.height, 2);
        assert_eq!(converted.byte_len(), 16);
        assert_eq!(converted.pixel_at(0, 0), 1);
        assert_eq!(converted.pixel_at(1, 0), 0);
        assert_eq!(converted.pixel_at(2, 0), 1);
        assert_eq!(converted.pixel_at(3, 0), 0);
        assert_eq!(converted.pixel_at(0, 1), 1);
        assert_eq!(converted.pixel_at(1, 1), 1);
        assert_eq!(converted.pixel_at(2, 1), 1);
        assert_eq!(converted.pixel_at(3, 1), 1);
        assert_eq!(converted.pixel_at(4, 1), 0);
    }

    #[test]
    fn convert_4bit_paletted_to_8bit_colour() {
        let raw = [0x53u8, 0xC1u8];
        let converted = convert_to_8bit_colour(4, 1, SourceBitDepth::Bit4, &raw, None);

        assert_eq!(converted.width, 4);
        assert_eq!(converted.height, 1);
        assert_eq!(converted.byte_len(), 4);
        assert_eq!(converted.pixel_at(0, 0), 3);
        assert_eq!(converted.pixel_at(1, 0), 5);
        assert_eq!(converted.pixel_at(2, 0), 1);
        assert_eq!(converted.pixel_at(3, 0), 12);
    }

    #[test]
    fn convert_8bit_grayscale_to_8bit_colour() {
        let raw = [0u8, 64, 128, 255];
        let converted = convert_to_8bit_colour(4, 1, SourceBitDepth::Bit8Gray, &raw, None);

        assert_eq!(converted.byte_len(), 4);
        assert_eq!(converted.pixel_at(0, 0), 0);
        assert_eq!(converted.pixel_at(1, 0), 64);
        assert_eq!(converted.pixel_at(2, 0), 128);
        assert_eq!(converted.pixel_at(3, 0), 255);
        assert_eq!(converted.rgb_at(2, 0), (128, 128, 128));
    }

    #[test]
    fn convert_16bit_grayscale_to_8bit_colour() {
        let raw = [0x80u8, 0x00, 0xFF, 0xFF];
        let converted = convert_to_8bit_colour(2, 1, SourceBitDepth::Bit16Gray, &raw, None);

        assert_eq!(converted.pixel_at(0, 0), 0x80);
        assert_eq!(converted.pixel_at(1, 0), 0xFF);
    }

    #[test]
    fn convert_24bit_rgb_to_8bit_colour() {
        let raw = [
            0x3C, 0x3E, 0x44, // Matches core palette tarmac
            0xD8, 0x28, 0x3C, // Matches core palette kerb red
            0x00, 0xFF, 0x00, // New dynamic colour
        ];
        let converted = convert_to_8bit_colour(3, 1, SourceBitDepth::Bit24Rgb, &raw, None);

        assert_eq!(converted.byte_len(), 3);
        assert_eq!(converted.pixel_at(0, 0), 1);
        assert_eq!(converted.pixel_at(1, 0), 4);
        let green_idx = converted.pixel_at(2, 0);
        assert!(green_idx >= 16);
        let rgb = converted.clut[green_idx as usize];
        assert_eq!(rgb, (0x00, 0xFF, 0x00));
    }

    #[test]
    fn convert_32bit_rgba_to_8bit_colour() {
        let raw = [
            0xFF, 0xFF, 0xFF, 0xFF, // Opaque white
            0x00, 0x00, 0x00, 0xFF, // Opaque black -> VOID (0)
        ];
        let converted = convert_to_8bit_colour(2, 1, SourceBitDepth::Bit32Rgba, &raw, None);

        assert_eq!(converted.byte_len(), 2);
        assert_eq!(converted.pixel_at(1, 0), 0);
    }
}
