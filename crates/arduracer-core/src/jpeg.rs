//! Freestanding `#![no_std]` Baseline JPEG Block Decoder for PSX Texture Streaming.
//!
//! Decodes 64x64 pixel tiles on-the-fly directly from 1024x1024 baseline JPEG images
//! resident in PSX main RAM, emitting 16-bit BGR555 texels for direct VRAM upload.
//!
//! Key characteristics:
//! - Strictly zero heap allocation (`no_std` compatible).
//! - Fast 32-bit fixed-point integer 8x8 IDCT (IJG standard formulation).
//! - Direct O(1) random access to 64x64 blocks via Restart Markers (DRI = 4 MCUs).
//! - Zero CD-ROM access during decompression: decompresses RAM -> VRAM.

pub const BLOCK_TEXELS: usize = 1024;
pub const TILE_TEXELS: usize = 64;
pub const TILES_PER_AXIS: usize = BLOCK_TEXELS / TILE_TEXELS; // 16
pub const TOTAL_TILES: usize = TILES_PER_AXIS * TILES_PER_AXIS; // 256
pub const TILE_PIXELS: usize = TILE_TEXELS * TILE_TEXELS; // 4096

/// Number of 4-MCU restart intervals in a 1024x1024 image (64 rows * 16 intervals = 1024).
pub const RESTART_INTERVAL_COUNT: usize = 1024;

/// Zigzag to row-major natural index table.
const ZIGZAG: [usize; 64] = [
    0, 1, 5, 6, 14, 15, 27, 28, 2, 4, 7, 13, 16, 26, 29, 42, 3, 8, 12, 17, 25, 30, 41, 43, 9, 11,
    18, 24, 31, 40, 44, 53, 10, 19, 23, 32, 39, 45, 52, 54, 20, 22, 33, 38, 46, 51, 55, 60, 21, 34,
    37, 47, 50, 56, 59, 61, 35, 36, 48, 49, 57, 58, 62, 63,
];

// IDCT fixed-point constants (Q13)
const CONST_BITS: i32 = 13;
const PASS1_BITS: i32 = 2;
const FIX_0_298631336: i32 = 2446;
const FIX_0_390180644: i32 = 3196;
const FIX_0_541196100: i32 = 4433;
const FIX_0_765366865: i32 = 6270;
const FIX_0_899976223: i32 = 7373;
const FIX_1_175875602: i32 = 9633;
const FIX_1_501321110: i32 = 12299;
const FIX_1_847759065: i32 = 15137;
const FIX_1_961570560: i32 = 16069;
const FIX_2_053119869: i32 = 16819;
const FIX_2_562915447: i32 = 20995;
const FIX_3_072711026: i32 = 25172;

pub const HUFF_LUT_BITS: usize = 9;
pub const HUFF_LUT_SIZE: usize = 1 << HUFF_LUT_BITS; // 512

/// A compact canonical Huffman lookup table with 9-bit fast O(1) prefix table.
#[derive(Copy, Clone)]
pub struct HuffTable {
    counts: [u8; 16],
    symbols: [u8; 162],
    num_symbols: usize,
    /// Fast 9-bit lookup: high 8 bits = symbol, low 8 bits = length (0 if > 9 bits).
    lut: [u16; HUFF_LUT_SIZE],
    /// Min code per length (1..=16)
    min_code: [u32; 17],
    /// Max code per length (1..=16)
    max_code: [i32; 17],
    /// Symbol table offset per length (1..=16)
    val_offset: [usize; 17],
}

impl Default for HuffTable {
    fn default() -> Self {
        Self::empty()
    }
}

impl HuffTable {
    pub const fn empty() -> Self {
        Self {
            counts: [0; 16],
            symbols: [0; 162],
            num_symbols: 0,
            lut: [0; HUFF_LUT_SIZE],
            min_code: [0; 17],
            max_code: [0; 17],
            val_offset: [0; 17],
        }
    }

    /// Builds decoding lookup tables from JPEG DHT counts and symbols.
    pub fn build(&mut self, counts: &[u8; 16], symbols: &[u8]) {
        self.counts = *counts;
        self.num_symbols = symbols.len().min(162);
        self.symbols[..self.num_symbols].copy_from_slice(&symbols[..self.num_symbols]);
        self.lut = [0; HUFF_LUT_SIZE];

        let mut code: u32 = 0;
        let mut sym_idx = 0usize;

        for len in 1..=16 {
            let count = self.counts[len - 1] as usize;
            if count == 0 {
                self.min_code[len] = 0;
                self.max_code[len] = -1;
                self.val_offset[len] = sym_idx;
                code <<= 1;
                continue;
            }

            self.min_code[len] = code;
            self.val_offset[len] = sym_idx;

            for _ in 0..count {
                if sym_idx < self.num_symbols {
                    let sym = self.symbols[sym_idx];
                    if len <= HUFF_LUT_BITS {
                        let fill_bits = HUFF_LUT_BITS - len;
                        let base = (code << fill_bits) as usize;
                        let entries = 1usize << fill_bits;
                        for e in 0..entries {
                            self.lut[base + e] = ((sym as u16) << 8) | (len as u16);
                        }
                    }
                    sym_idx += 1;
                }
                code += 1;
            }
            self.max_code[len] = (code - 1) as i32;
            code <<= 1;
        }
    }

    /// Decodes one Huffman symbol from the bit reader.
    #[inline(always)]
    pub fn decode(&self, br: &mut BitReader) -> Option<u8> {
        let peek = br.peek_bits(HUFF_LUT_BITS as u8);
        let entry = self.lut[peek as usize];
        let len = (entry & 0xFF) as u8;
        if len > 0 {
            br.consume_bits(len);
            return Some((entry >> 8) as u8);
        }

        // Slower path for codes longer than 9 bits (typically <2% of entropy stream)
        let code = br.peek_bits(16);
        for l in 10..=16 {
            let cur_code = (code >> (16 - l)) as i32;
            if cur_code <= self.max_code[l] {
                br.consume_bits(l as u8);
                let idx = self.val_offset[l] + (cur_code - self.min_code[l] as i32) as usize;
                if idx < self.num_symbols {
                    return Some(self.symbols[idx]);
                }
                break;
            }
        }
        None
    }
}

/// Helper for reading variable-length bits with byte-stuffing handling (`0xFF 0x00`).
pub struct BitReader<'a> {
    data: &'a [u8],
    pos: usize,
    buf: u32,
    bits_left: u8,
}

impl<'a> BitReader<'a> {
    #[inline(always)]
    pub fn new(data: &'a [u8], start_offset: usize) -> Self {
        let mut r = Self {
            data,
            pos: start_offset,
            buf: 0,
            bits_left: 0,
        };
        r.refill();
        r
    }

    #[inline(always)]
    fn refill(&mut self) {
        while self.bits_left <= 24 && self.pos < self.data.len() {
            let b = self.data[self.pos];
            self.pos += 1;
            if b == 0xFF && self.pos < self.data.len() {
                let next = self.data[self.pos];
                if next == 0x00 {
                    self.pos += 1; // Skip byte stuffing
                } else if (0xD0..=0xD7).contains(&next) || next == 0xD9 {
                    // Reached restart or end marker
                    self.pos -= 1;
                    self.buf = (self.buf << 8) | (b as u32);
                    self.bits_left += 8;
                    break;
                }
            }
            self.buf = (self.buf << 8) | (b as u32);
            self.bits_left += 8;
        }
    }

    #[inline(always)]
    pub fn peek_bits(&mut self, n: u8) -> u32 {
        if self.bits_left < n {
            self.refill();
        }
        if self.bits_left < n {
            (self.buf << (n - self.bits_left)) & ((1 << n) - 1)
        } else {
            (self.buf >> (self.bits_left - n)) & ((1 << n) - 1)
        }
    }

    #[inline(always)]
    pub fn consume_bits(&mut self, n: u8) {
        self.bits_left = self.bits_left.saturating_sub(n);
        if self.bits_left <= 16 {
            self.refill();
        }
    }

    #[inline(always)]
    pub fn read_bits(&mut self, n: u8) -> u32 {
        if n == 0 {
            return 0;
        }
        let v = self.peek_bits(n);
        self.consume_bits(n);
        v
    }

    #[inline(always)]
    pub fn read_signed(&mut self, n: u8) -> i16 {
        if n == 0 {
            return 0;
        }
        let v = self.read_bits(n);
        if v < (1 << (n - 1)) {
            (v as i32 + ((-1i32) << n) + 1) as i16
        } else {
            v as i16
        }
    }
}

/// Parsed metadata for a 1024x1024 baseline JPEG image.
#[derive(Copy, Clone)]
pub struct JpegHeader {
    pub width: u16,
    pub height: u16,
    pub restart_interval: u16,
    pub q_tables: [[u16; 64]; 2],
    pub dc_huffman: [HuffTable; 2],
    pub ac_huffman: [HuffTable; 2],
    pub scan_offset: usize,
}

impl Default for JpegHeader {
    fn default() -> Self {
        Self {
            width: 0,
            height: 0,
            restart_interval: 4,
            q_tables: [[0; 64]; 2],
            dc_huffman: [HuffTable::empty(), HuffTable::empty()],
            ac_huffman: [HuffTable::empty(), HuffTable::empty()],
            scan_offset: 0,
        }
    }
}

impl JpegHeader {
    /// Parses headers from a 1024x1024 baseline JPEG buffer.
    pub fn parse(data: &[u8]) -> Option<Self> {
        if data.len() < 4 || data[0] != 0xFF || data[1] != 0xD8 {
            return None;
        }
        let mut header = Self::default();
        let mut pos = 2usize;

        while pos + 4 <= data.len() {
            if data[pos] != 0xFF {
                pos += 1;
                continue;
            }
            let marker = data[pos + 1];
            pos += 2;

            if marker == 0xD8 {
                continue;
            }
            if marker == 0xD9 {
                break;
            }
            if (0xD0..=0xD7).contains(&marker) {
                continue;
            }

            let len = u16::from_be_bytes([data[pos], data[pos + 1]]) as usize;
            if pos + len > data.len() || len < 2 {
                return None;
            }
            let payload = &data[pos + 2..pos + len];

            match marker {
                0xDB => {
                    // DQT: Quantization table
                    let mut p = 0;
                    while p < payload.len() {
                        let id = (payload[p] & 0x0F) as usize;
                        p += 1;
                        if id < 2 && p + 64 <= payload.len() {
                            for k in 0..64 {
                                header.q_tables[id][k] = payload[p + k] as u16;
                            }
                            p += 64;
                        } else {
                            break;
                        }
                    }
                }
                0xC0 if payload.len() >= 6 => {
                    // SOF0: Baseline DCT frame header
                    header.height = u16::from_be_bytes([payload[1], payload[2]]);
                    header.width = u16::from_be_bytes([payload[3], payload[4]]);
                }
                0xC4 => {
                    // DHT: Huffman table
                    let mut p = 0;
                    while p + 17 <= payload.len() {
                        let info = payload[p];
                        let is_ac = (info & 0x10) != 0;
                        let id = (info & 0x0F) as usize;
                        p += 1;
                        let mut counts = [0u8; 16];
                        counts.copy_from_slice(&payload[p..p + 16]);
                        p += 16;
                        let total: usize = counts.iter().map(|&c| c as usize).sum();
                        if p + total <= payload.len() && id < 2 {
                            let syms = &payload[p..p + total];
                            if is_ac {
                                header.ac_huffman[id].build(&counts, syms);
                            } else {
                                header.dc_huffman[id].build(&counts, syms);
                            }
                            p += total;
                        } else {
                            break;
                        }
                    }
                }
                0xDD if payload.len() >= 2 => {
                    // DRI: Define Restart Interval
                    header.restart_interval = u16::from_be_bytes([payload[0], payload[1]]);
                }
                0xDA => {
                    // SOS: Start of Scan
                    header.scan_offset = pos + len;
                    return Some(header);
                }
                _ => {}
            }
            pos += len;
        }

        if header.scan_offset > 0 {
            Some(header)
        } else {
            None
        }
    }

    /// Scans the bitstream after SOS and records starting byte offsets of all restart intervals.
    pub fn index_restarts(&self, data: &[u8], restart_offsets: &mut [u32; RESTART_INTERVAL_COUNT]) {
        restart_offsets[0] = self.scan_offset as u32;
        let mut interval = 1usize;
        let mut i = self.scan_offset;

        while i + 1 < data.len() && interval < RESTART_INTERVAL_COUNT {
            if data[i] == 0xFF {
                let m = data[i + 1];
                if (0xD0..=0xD7).contains(&m) {
                    restart_offsets[interval] = (i + 2) as u32;
                    interval += 1;
                    i += 2;
                    continue;
                } else if m == 0xD9 {
                    break;
                } else if m == 0x00 {
                    i += 2;
                    continue;
                }
            }
            i += 1;
        }

        // Fill remaining intervals with last valid offset as fallback
        let last = restart_offsets[interval.saturating_sub(1)];
        for rem in &mut restart_offsets[interval..RESTART_INTERVAL_COUNT] {
            *rem = last;
        }
    }
}

/// In-place 1D IDCT row pass.
#[inline(always)]
fn idct_row(block: &mut [i32; 64], offset: usize) {
    let ws0 = block[offset];
    let ws1 = block[offset + 1];
    let ws2 = block[offset + 2];
    let ws3 = block[offset + 3];
    let ws4 = block[offset + 4];
    let ws5 = block[offset + 5];
    let ws6 = block[offset + 6];
    let ws7 = block[offset + 7];

    if ws0 == 0 && ws1 == 0 && ws2 == 0 && ws3 == 0 && ws4 == 0 && ws5 == 0 && ws6 == 0 && ws7 == 0
    {
        return;
    }

    if ws1 == 0 && ws2 == 0 && ws3 == 0 && ws4 == 0 && ws5 == 0 && ws6 == 0 && ws7 == 0 {
        let val = ws0 << PASS1_BITS;
        block[offset] = val;
        block[offset + 1] = val;
        block[offset + 2] = val;
        block[offset + 3] = val;
        block[offset + 4] = val;
        block[offset + 5] = val;
        block[offset + 6] = val;
        block[offset + 7] = val;
        return;
    }

    let z1 = (ws2 + ws6) * FIX_0_541196100;
    let tmp2 = z1 + ws6 * (-FIX_1_847759065);
    let tmp3 = z1 + ws2 * FIX_0_765366865;

    let tmp0 = (ws0 + ws4) << CONST_BITS;
    let tmp1 = (ws0 - ws4) << CONST_BITS;

    let tmp10 = tmp0 + tmp3;
    let tmp13 = tmp0 - tmp3;
    let tmp11 = tmp1 + tmp2;
    let tmp12 = tmp1 - tmp2;

    let z1 = ws7 + ws1;
    let z2 = ws5 + ws3;
    let z3 = ws7 + ws3;
    let z4 = ws5 + ws1;
    let z5 = (z3 + z4) * FIX_1_175875602;

    let u0 = ws7 * FIX_0_298631336 + z1 * (-FIX_0_899976223) + (z3 * (-FIX_1_961570560) + z5);
    let u1 = ws5 * FIX_2_053119869 + z2 * (-FIX_2_562915447) + (z4 * (-FIX_0_390180644) + z5);
    let u2 = ws3 * FIX_3_072711026 + z2 * (-FIX_2_562915447) + (z3 * (-FIX_1_961570560) + z5);
    let u3 = ws1 * FIX_1_501321110 + z1 * (-FIX_0_899976223) + (z4 * (-FIX_0_390180644) + z5);

    block[offset] =
        (tmp10 + u3 + (1 << (CONST_BITS - PASS1_BITS - 1))) >> (CONST_BITS - PASS1_BITS);
    block[offset + 7] =
        (tmp10 - u3 + (1 << (CONST_BITS - PASS1_BITS - 1))) >> (CONST_BITS - PASS1_BITS);
    block[offset + 1] =
        (tmp11 + u2 + (1 << (CONST_BITS - PASS1_BITS - 1))) >> (CONST_BITS - PASS1_BITS);
    block[offset + 6] =
        (tmp11 - u2 + (1 << (CONST_BITS - PASS1_BITS - 1))) >> (CONST_BITS - PASS1_BITS);
    block[offset + 2] =
        (tmp12 + u1 + (1 << (CONST_BITS - PASS1_BITS - 1))) >> (CONST_BITS - PASS1_BITS);
    block[offset + 5] =
        (tmp12 - u1 + (1 << (CONST_BITS - PASS1_BITS - 1))) >> (CONST_BITS - PASS1_BITS);
    block[offset + 3] =
        (tmp13 + u0 + (1 << (CONST_BITS - PASS1_BITS - 1))) >> (CONST_BITS - PASS1_BITS);
    block[offset + 4] =
        (tmp13 - u0 + (1 << (CONST_BITS - PASS1_BITS - 1))) >> (CONST_BITS - PASS1_BITS);
}

/// In-place 1D IDCT column pass with range clamping (0..=255).
#[inline(always)]
fn idct_col(block: &mut [i32; 64], offset: usize, out: &mut [u8; 64]) {
    let ws0 = block[offset];
    let ws1 = block[offset + 8];
    let ws2 = block[offset + 16];
    let ws3 = block[offset + 24];
    let ws4 = block[offset + 32];
    let ws5 = block[offset + 40];
    let ws6 = block[offset + 48];
    let ws7 = block[offset + 56];

    let shift = CONST_BITS + PASS1_BITS + 3;

    if ws1 == 0 && ws2 == 0 && ws3 == 0 && ws4 == 0 && ws5 == 0 && ws6 == 0 && ws7 == 0 {
        let val = ((ws0 + (1 << (PASS1_BITS + 2))) >> (PASS1_BITS + 3)).clamp(-128, 127) + 128;
        let b = val as u8;
        out[offset] = b;
        out[offset + 8] = b;
        out[offset + 16] = b;
        out[offset + 24] = b;
        out[offset + 32] = b;
        out[offset + 40] = b;
        out[offset + 48] = b;
        out[offset + 56] = b;
        return;
    }

    let z1 = (ws2 + ws6) * FIX_0_541196100;
    let tmp2 = z1 + ws6 * (-FIX_1_847759065);
    let tmp3 = z1 + ws2 * FIX_0_765366865;

    let tmp0 = (ws0 + ws4) << CONST_BITS;
    let tmp1 = (ws0 - ws4) << CONST_BITS;

    let tmp10 = tmp0 + tmp3;
    let tmp13 = tmp0 - tmp3;
    let tmp11 = tmp1 + tmp2;
    let tmp12 = tmp1 - tmp2;

    let z1 = ws7 + ws1;
    let z2 = ws5 + ws3;
    let z3 = ws7 + ws3;
    let z4 = ws5 + ws1;
    let z5 = (z3 + z4) * FIX_1_175875602;

    let u0 = ws7 * FIX_0_298631336 + z1 * (-FIX_0_899976223) + (z3 * (-FIX_1_961570560) + z5);
    let u1 = ws5 * FIX_2_053119869 + z2 * (-FIX_2_562915447) + (z4 * (-FIX_0_390180644) + z5);
    let u2 = ws3 * FIX_3_072711026 + z2 * (-FIX_2_562915447) + (z3 * (-FIX_1_961570560) + z5);
    let u3 = ws1 * FIX_1_501321110 + z1 * (-FIX_0_899976223) + (z4 * (-FIX_0_390180644) + z5);

    let clamp_sample = |v: i32| -> u8 {
        let sample = ((v + (1 << (shift - 1))) >> shift) + 128;
        sample.clamp(0, 255) as u8
    };

    out[offset] = clamp_sample(tmp10 + u3);
    out[offset + 56] = clamp_sample(tmp10 - u3);
    out[offset + 8] = clamp_sample(tmp11 + u2);
    out[offset + 48] = clamp_sample(tmp11 - u2);
    out[offset + 16] = clamp_sample(tmp12 + u1);
    out[offset + 40] = clamp_sample(tmp12 - u1);
    out[offset + 24] = clamp_sample(tmp13 + u0);
    out[offset + 32] = clamp_sample(tmp13 - u0);
}

/// Performs 2D 8x8 IDCT.
#[inline(always)]
fn idct_8x8(coeffs: &[i32; 64], out_pixels: &mut [u8; 64]) {
    let mut ws = *coeffs;
    for r in 0..8 {
        idct_row(&mut ws, r * 8);
    }
    for c in 0..8 {
        idct_col(&mut ws, c, out_pixels);
    }
}

/// Decodes one 8x8 coefficient block from the bitstream.
#[inline(always)]
fn decode_block(
    br: &mut BitReader,
    dc_table: &HuffTable,
    ac_table: &HuffTable,
    q_table: &[u16; 64],
    last_dc: &mut i32,
    out_samples: &mut [u8; 64],
) {
    // Decode DC coefficient
    let dc_size = dc_table.decode(br).unwrap_or(0);
    let dc_diff = br.read_signed(dc_size) as i32;
    *last_dc += dc_diff;
    let dc_val = *last_dc * (q_table[0] as i32);

    // Fast AC early-out: decode first AC symbol.
    // If first symbol is 0 (EOB), all 63 AC coefficients are zero!
    let first_sym = ac_table.decode(br).unwrap_or(0);
    if first_sym == 0 {
        // Pure DC flat block: 2D IDCT collapses to a constant spatial level.
        // Bit-for-bit identical to full idct_col(idct_row(dc)):
        // sample = ((dc_val + 4) >> 3).clamp(-128, 127) + 128
        let sample = (((dc_val + 4) >> 3).clamp(-128, 127) + 128) as u8;
        out_samples.fill(sample);
        return;
    }

    let mut coeffs = [0i32; 64];
    coeffs[0] = dc_val;

    // Process the first AC symbol already fetched
    let run = (first_sym >> 4) as usize;
    let size = first_sym & 0x0F;
    let mut k = 1 + run;
    if k < 64 {
        if size > 0 {
            let val = br.read_signed(size) as i32;
            let zz = ZIGZAG[k];
            coeffs[zz] = val * (q_table[k] as i32);
        }
        k += 1;

        // Decode remaining AC coefficients
        while k < 64 {
            let sym = ac_table.decode(br).unwrap_or(0);
            if sym == 0 {
                // EOB: End of Block
                break;
            }
            let run = (sym >> 4) as usize;
            let size = sym & 0x0F;
            k += run;
            if k >= 64 {
                break;
            }
            if size > 0 {
                let val = br.read_signed(size) as i32;
                let zz = ZIGZAG[k];
                coeffs[zz] = val * (q_table[k] as i32);
            }
            k += 1;
        }
    }

    idct_8x8(&coeffs, out_samples);
}

/// Decodes one 64x64 block from a 1024x1024 JPEG image into 16-bit direct-colour BGR555 texels.
#[inline(never)]
pub fn decode_tile_64x64(
    jpeg_data: &[u8],
    header: &JpegHeader,
    restart_offsets: &[u32; RESTART_INTERVAL_COUNT],
    tile_x: usize,
    tile_y: usize,
    out_bgr555: &mut [u16; TILE_PIXELS],
) {
    if tile_x >= TILES_PER_AXIS || tile_y >= TILES_PER_AXIS {
        return;
    }

    let mut y0 = [0u8; 64];
    let mut y1 = [0u8; 64];
    let mut y2 = [0u8; 64];
    let mut y3 = [0u8; 64];
    let mut cb = [0u8; 64];
    let mut cr = [0u8; 64];

    // A 64x64 block spans 4 MCU rows (each MCU is 16x16, 4 rows = 64 pixels).
    for row in 0..4 {
        let interval_idx = (tile_y * 4 + row) * TILES_PER_AXIS + tile_x;
        if interval_idx >= RESTART_INTERVAL_COUNT {
            break;
        }
        let start_offset = restart_offsets[interval_idx] as usize;
        let mut br = BitReader::new(jpeg_data, start_offset);

        // Reset DC predictors at the restart marker
        let mut dc_y = 0i32;
        let mut dc_cb = 0i32;
        let mut dc_cr = 0i32;

        // Exactly 4 MCUs in this 64-pixel interval
        for mcu_x in 0..4 {
            decode_block(
                &mut br,
                &header.dc_huffman[0],
                &header.ac_huffman[0],
                &header.q_tables[0],
                &mut dc_y,
                &mut y0,
            );
            decode_block(
                &mut br,
                &header.dc_huffman[0],
                &header.ac_huffman[0],
                &header.q_tables[0],
                &mut dc_y,
                &mut y1,
            );
            decode_block(
                &mut br,
                &header.dc_huffman[0],
                &header.ac_huffman[0],
                &header.q_tables[0],
                &mut dc_y,
                &mut y2,
            );
            decode_block(
                &mut br,
                &header.dc_huffman[0],
                &header.ac_huffman[0],
                &header.q_tables[0],
                &mut dc_y,
                &mut y3,
            );
            decode_block(
                &mut br,
                &header.dc_huffman[1],
                &header.ac_huffman[1],
                &header.q_tables[1],
                &mut dc_cb,
                &mut cb,
            );
            decode_block(
                &mut br,
                &header.dc_huffman[1],
                &header.ac_huffman[1],
                &header.q_tables[1],
                &mut dc_cr,
                &mut cr,
            );

            // Convert 16x16 MCU pixels to BGR555 using smooth bilinear chroma upsampling
            // and 4x4 Bayer ordered dithering for maximum visual fidelity in 15bpp direct colour.
            let base_px = mcu_x * 16;
            let base_py = row * 16;

            const BAYER4X4: [[i32; 4]; 4] =
                [[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]];

            // Bilinear sampling weights and indices for 8-to-16 upsampling (weights sum to 4)
            // (sample_0, sample_1, weight_0, weight_1)
            const INTERP_16: [(usize, usize, i32, i32); 16] = [
                (0, 0, 4, 0),
                (0, 1, 3, 1),
                (0, 1, 1, 3),
                (1, 2, 3, 1),
                (1, 2, 1, 3),
                (2, 3, 3, 1),
                (2, 3, 1, 3),
                (3, 4, 3, 1),
                (3, 4, 1, 3),
                (4, 5, 3, 1),
                (4, 5, 1, 3),
                (5, 6, 3, 1),
                (5, 6, 1, 3),
                (6, 7, 3, 1),
                (6, 7, 1, 3),
                (7, 7, 4, 0),
            ];

            #[allow(clippy::needless_range_loop)]
            for py in 0..16 {
                let out_y = base_py + py;
                let (y_block, y_sub_y) = if py < 8 {
                    ((&y0, &y1), py)
                } else {
                    ((&y2, &y3), py - 8)
                };
                let (cy0, cy1, wy0, wy1) = INTERP_16[py];

                for px in 0..16 {
                    let out_x = base_px + px;
                    let (y_arr, y_sub_x) = if px < 8 {
                        (y_block.0, px)
                    } else {
                        (y_block.1, px - 8)
                    };
                    let (cx0, cx1, wx0, wx1) = INTERP_16[px];

                    let y_val = y_arr[y_sub_y * 8 + y_sub_x] as i32;

                    // Bilinear interpolation for Cb and Cr
                    let cb_00 = cb[cy0 * 8 + cx0] as i32;
                    let cb_01 = cb[cy0 * 8 + cx1] as i32;
                    let cb_10 = cb[cy1 * 8 + cx0] as i32;
                    let cb_11 = cb[cy1 * 8 + cx1] as i32;
                    let cb_val = ((cb_00 * wx0 + cb_01 * wx1) * wy0
                        + (cb_10 * wx0 + cb_11 * wx1) * wy1)
                        / 16
                        - 128;

                    let cr_00 = cr[cy0 * 8 + cx0] as i32;
                    let cr_01 = cr[cy0 * 8 + cx1] as i32;
                    let cr_10 = cr[cy1 * 8 + cx0] as i32;
                    let cr_11 = cr[cy1 * 8 + cx1] as i32;
                    let cr_val = ((cr_00 * wx0 + cr_01 * wx1) * wy0
                        + (cr_10 * wx0 + cr_11 * wx1) * wy1)
                        / 16
                        - 128;

                    // ITU-R BT.601 integer fixed-point YCbCr to RGB conversion
                    let r_raw = y_val + ((359 * cr_val) >> 8);
                    let g_raw = y_val - ((88 * cb_val + 183 * cr_val) >> 8);
                    let b_raw = y_val + ((454 * cb_val) >> 8);

                    // 4x4 Bayer ordered dither for smooth 15-bit color gradients
                    let dither = BAYER4X4[out_y % 4][out_x % 4] >> 1; // 0..7

                    let r = ((r_raw + dither).clamp(0, 255) >> 3) as u16;
                    let g = ((g_raw + dither).clamp(0, 255) >> 3) as u16;
                    let b = ((b_raw + dither).clamp(0, 255) >> 3) as u16;

                    let bgr555 = (b << 10) | (g << 5) | r;
                    out_bgr555[out_y * TILE_TEXELS + out_x] = bgr555;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_huffman_does_not_panic() {
        let table = HuffTable::empty();
        let dummy = [0u8; 4];
        let mut br = BitReader::new(&dummy, 0);
        assert_eq!(table.decode(&mut br), None);
    }

    #[test]
    fn test_idct_dc_flat_block() {
        let mut coeffs = [0i32; 64];
        // In 2D DCT unscaled, DC = C * 8, where C is the spatial value - 128.
        // For spatial value 200, C = 72, DC = 72 * 8 = 576.
        coeffs[0] = 576;
        let mut out = [0u8; 64];
        idct_8x8(&coeffs, &mut out);
        for &px in out.iter() {
            // Should be within +/- 1 of 200
            assert!((px as i32 - 200).abs() <= 2, "Expected ~200, got {px}");
        }
    }

    #[test]
    fn test_dc_flat_block_exact_equivalence() {
        // Verify that (((dc_val + 4) >> 3).clamp(-128, 127) + 128) as u8
        // is 100% bit-for-bit identical to full idct_8x8 for any DC value.
        for dc in -2048..=2048 {
            let mut coeffs = [0i32; 64];
            coeffs[0] = dc;
            let mut idct_out = [0u8; 64];
            idct_8x8(&coeffs, &mut idct_out);

            let fast_sample = (((dc + 4) >> 3).clamp(-128, 127) + 128) as u8;
            for (idx, &px) in idct_out.iter().enumerate() {
                assert_eq!(
                    px, fast_sample,
                    "Mismatch at dc={dc}, idx={idx}: idct={px} vs fast={fast_sample}"
                );
            }
        }
    }

    #[test]
    fn test_zigzag_table_bounds() {
        assert_eq!(ZIGZAG.len(), 64);
        for &z in &ZIGZAG {
            assert!(z < 64);
        }
    }

    #[test]
    fn test_bit_reader_basic() {
        let data = [0b10110011, 0b11001010];
        let mut br = BitReader::new(&data, 0);
        assert_eq!(br.read_bits(4), 0b1011);
        assert_eq!(br.read_bits(4), 0b0011);
        assert_eq!(br.read_bits(8), 0b11001010);
    }

    #[test]
    fn test_bit_reader_byte_stuffing() {
        let data = [0xFF, 0x00, 0xAA];
        let mut br = BitReader::new(&data, 0);
        assert_eq!(br.read_bits(8), 0xFF);
        assert_eq!(br.read_bits(8), 0xAA);
    }

    #[test]
    fn test_decode_tile_from_real_jpeg() {
        let path = "../../tracks/test_100kb.jpg";
        if let Ok(data) = std::fs::read(path) {
            assert!(data.len() <= 102400, "File must be under 100 KB");
            let header = JpegHeader::parse(&data).expect("Must parse header");
            assert_eq!(header.width, 1024);
            assert_eq!(header.height, 1024);
            assert_eq!(header.restart_interval, 4);

            let mut restart_offsets = [0u32; RESTART_INTERVAL_COUNT];
            header.index_restarts(&data, &mut restart_offsets);

            let mut out = [0u16; TILE_PIXELS];
            decode_tile_64x64(&data, &header, &restart_offsets, 0, 0, &mut out);

            // Verify non-zero output and reasonable direct-colour BGR555 values
            let non_zero = out.iter().filter(|&&px| px != 0).count();
            assert!(
                non_zero > 1000,
                "Tile should contain non-zero pixels, got {non_zero}"
            );
        }
    }

    #[test]
    fn test_decode_capetown_tiles() {
        let path = "../../tracks/capetown_10km/capetown_b0_b0.jpg";
        if let Ok(data) = std::fs::read(path) {
            let header = JpegHeader::parse(&data).expect("Must parse header");
            assert_eq!(header.width, 1024);
            assert_eq!(header.height, 1024);
            assert_eq!(header.restart_interval, 4);

            let mut restart_offsets = [0u32; RESTART_INTERVAL_COUNT];
            header.index_restarts(&data, &mut restart_offsets);

            let mut out = [0u16; TILE_PIXELS];
            for ty in 0..4 {
                for tx in 0..4 {
                    decode_tile_64x64(&data, &header, &restart_offsets, tx, ty, &mut out);
                    let non_zero = out.iter().filter(|&&px| px != 0).count();
                    assert!(non_zero > 1000, "Tile ({tx},{ty}) non-zero: {non_zero}");
                }
            }
        }
    }
}
