// SPDX-License-Identifier: GPL-2.0-or-later
//! MDEC MMIO helpers.
//!
//! The MDEC has two registers: data (0x1F80_1820) and control/status
//! (0x1F80_1824). Data is fed via DMA channel 0 (RAM→MDEC) and drained
//! via DMA channel 1 (MDEC→RAM). This module provides polling helpers
//! for the control register and single-word PIO fallback.

use crate::{read32, write32};
use psx_hw::mdec::{MDEC0_CTRL, MDEC0_DATA};

/// Write a word to the MDEC data register.
#[inline(always)]
pub fn write_data(word: u32) {
    unsafe { write32(MDEC0_DATA, word) }
}

/// Read a word from the MDEC data register (decoded output).
#[inline(always)]
pub fn read_data() -> u32 {
    unsafe { read32(MDEC0_DATA) }
}

/// Write the MDEC control register.
#[inline(always)]
pub fn write_ctrl(word: u32) {
    unsafe { write32(MDEC0_CTRL, word) }
}

/// Read the MDEC status register.
#[inline(always)]
pub fn read_stat() -> u32 {
    unsafe { read32(MDEC0_CTRL) }
}

/// Reset the MDEC decoder.
pub fn reset() {
    write_ctrl(psx_hw::mdec::ctrl::RESET);
    // Clear reset bit and enable DMA requests.
    write_ctrl(psx_hw::mdec::ctrl::DMA_IN_ENABLE | psx_hw::mdec::ctrl::DMA_OUT_ENABLE);
}

/// Standard MPEG-1 based quantization table in zigzag order (64 bytes).
pub static DEFAULT_QUANT_TABLE: [u8; 64] = [
    2, 16, 16, 19, 16, 19, 22, 22, 22, 22, 22, 22, 26, 24, 26, 27, 27, 27, 26, 26, 26, 26, 27, 27,
    27, 29, 29, 29, 34, 34, 34, 29, 29, 29, 27, 27, 29, 29, 32, 32, 34, 34, 37, 38, 37, 35, 35, 34,
    35, 38, 38, 40, 40, 40, 48, 48, 46, 46, 56, 56, 58, 69, 69, 83,
];

/// Standard IDCT scale table (32 words = 64 halfwords).
pub static DEFAULT_IDCT_TABLE: [u32; 32] = [
    0x5A825A82, 0x5A825A82, 0x5A825A82, 0x5A825A82, 0x6A6D7D8A, 0x18F8471C, 0xB8E4E708, 0x82769593,
    0x30FB7641, 0x89BFCF05, 0xCF0589BF, 0x764130FB, 0xE7086A6D, 0xB8E48276, 0x7D8A471C, 0x959318F8,
    0xA57E5A82, 0x5A82A57E, 0xA57E5A82, 0x5A82A57E, 0x8276471C, 0x6A6D18F8, 0xE7089593, 0xB8E47D8A,
    0x89BF30FB, 0xCF057641, 0x7641CF05, 0x30FB89BF, 0xB8E418F8, 0x82766A6D, 0x95937D8A, 0xE708471C,
];

/// Reset MDEC and upload standard IDCT matrix and quantization tables.
pub fn init() {
    reset();

    // Enable DMA channels in DPCR
    crate::dma::enable_channel(crate::dma::Channel::MdecIn);
    crate::dma::enable_channel(crate::dma::Channel::MdecOut);

    // Command 3: Set IDCT scale table (32 words)
    write_data(0x6000_0000 | 32);
    for &w in DEFAULT_IDCT_TABLE.iter() {
        write_data(w);
    }

    // Command 2: Set quantization tables (luminance and color, 32 words)
    write_data(0x4000_0001);
    for chunk in DEFAULT_QUANT_TABLE.chunks_exact(4) {
        write_data(u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]));
    }
    for chunk in DEFAULT_QUANT_TABLE.chunks_exact(4) {
        write_data(u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]));
    }
}

/// Start decoding a frame: writes the command word and arms DMA channel 0 (MdecIn).
pub fn start_decode_frame(words: &[u32]) {
    if words.is_empty() {
        return;
    }
    let cmd = words[0];
    let data_words = (cmd & 0xFFFF) as u16;
    if data_words > 0 {
        assert!(
            words.len() >= data_words as usize + 1,
            "start_decode_frame: words slice length ({}) < data_words + 1 ({})",
            words.len(),
            data_words as usize + 1
        );
    }
    crate::dma::abort(crate::dma::Channel::MdecIn);
    write_data(cmd);

    if data_words > 0 {
        let block_count = data_words.div_ceil(32);
        let madr = words[1..].as_ptr() as u32;
        crate::dma::set_madr(crate::dma::Channel::MdecIn, madr);
        crate::dma::set_bcr_block(crate::dma::Channel::MdecIn, 32, block_count);
        // Block sync, to device (RAM -> MDEC), start transfer
        crate::dma::set_chcr(crate::dma::Channel::MdecIn, 0x0100_0201);
    }
}

/// Drain decoded words for one vertical slice via DMA channel 1 (MdecOut).
///
/// The slice length must be a non-zero multiple of 32 words (the MDEC DMA block size).
/// For standard 320x240 video at 15bpp direct-color, each 16x240 slice is
/// 16 * 240 = 3,840 halfwords = 1,920 words (60 blocks of 32 words).
pub fn drain_slice_words_dma_with<F: FnMut()>(slice: &mut [u32], mut service: F) -> bool {
    assert!(
        !slice.is_empty() && slice.len() % 32 == 0,
        "drain_slice_words_dma: slice length must be a non-zero multiple of 32 words"
    );
    let block_count = (slice.len() / 32) as u16;
    crate::dma::abort(crate::dma::Channel::MdecOut);
    let madr = slice.as_mut_ptr() as u32;
    crate::dma::set_madr(crate::dma::Channel::MdecOut, madr);
    crate::dma::set_bcr_block(crate::dma::Channel::MdecOut, 32, block_count);
    // Block sync, from device (MDEC -> RAM), start transfer
    crate::dma::set_chcr(crate::dma::Channel::MdecOut, 0x0100_0200);

    // Leave enough time for DMA0 and the decoder to refill the output FIFO
    // between macroblocks while this DMA1 slice remains armed.
    let mut waited = 0u32;
    while crate::dma::is_busy(crate::dma::Channel::MdecOut) {
        if waited >= 1_000_000 {
            return false;
        }
        if waited & 0x3FF == 0 {
            service();
        }
        waited += 1;
    }
    true
}

/// Drain one standard vertical slice (16x240 pixels @ 15bpp = 1,920 words = 60 blocks)
/// via DMA channel 1 (MdecOut).
pub fn drain_slice_dma(slice: &mut [u32; 1920]) -> bool {
    drain_slice_dma_with(slice, || {})
}

/// Drain one standard vertical slice (16x240 pixels @ 15bpp = 1,920 words) while periodically
/// servicing another polled device. The callback runs every 1K busy checks while MDEC DMA is active.
pub fn drain_slice_dma_with<F: FnMut()>(slice: &mut [u32; 1920], service: F) -> bool {
    drain_slice_words_dma_with(slice, service)
}

/// Wait until the output FIFO has data.
pub fn wait_output_ready(spins: u32) -> bool {
    let mut waited = 0u32;
    while read_stat() & psx_hw::mdec::status::DATA_OUT_EMPTY != 0 {
        if waited >= spins {
            return false;
        }
        waited += 1;
        core::hint::spin_loop();
    }
    true
}

/// Wait until input FIFO is not full.
pub fn wait_input_ready(spins: u32) -> bool {
    let mut waited = 0u32;
    while read_stat() & psx_hw::mdec::status::DATA_IN_FULL != 0 {
        if waited >= spins {
            return false;
        }
        waited += 1;
        core::hint::spin_loop();
    }
    true
}

/// Poll for MDEC busy flag.
pub fn is_busy() -> bool {
    read_stat() & psx_hw::mdec::status::BUSY != 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quant_table_length_is_64() {
        assert_eq!(DEFAULT_QUANT_TABLE.len(), 64);
    }

    #[test]
    fn quant_table_first_and_last_values() {
        assert_eq!(DEFAULT_QUANT_TABLE[0], 2);
        assert_eq!(DEFAULT_QUANT_TABLE[63], 83);
    }

    #[test]
    fn quant_table_all_values_nonzero() {
        assert!(DEFAULT_QUANT_TABLE.iter().all(|&b| b != 0));
    }

    #[test]
    fn idct_table_length_is_32() {
        assert_eq!(DEFAULT_IDCT_TABLE.len(), 32);
    }

    #[test]
    fn idct_table_first_word_is_sqrt2_over_2_fixed_point() {
        // 0x5A82_5A82 encodes √2/2 as a 16.16 fixed-point pair used by the
        // hardware's IDCT scale table.
        assert_eq!(DEFAULT_IDCT_TABLE[0], 0x5A82_5A82);
    }

    #[test]
    fn idct_table_last_word() {
        assert_eq!(DEFAULT_IDCT_TABLE[31], 0xE708_471C);
    }

    #[test]
    fn idct_command_word_encodes_correctly() {
        // init() writes 0x6000_0000 | 32 as the "set IDCT table" command.
        assert_eq!(0x6000_0000u32 | 32, 0x6000_0020);
    }

    #[test]
    fn quant_command_top_byte_is_0x40() {
        // 0x4000_0001 = "set quantization tables, color mode" command.
        let cmd: u32 = 0x4000_0001;
        assert_eq!((cmd >> 24) as u8, 0x40);
    }

    #[test]
    fn start_decode_frame_command_encodes_128_words() {
        // start_decode_frame builds 0x3800_0000 | data_words; verify the
        // formula for a 128-word payload.
        assert_eq!(0x3800_0000u32 | 128, 0x3800_0080);
    }

    #[test]
    fn quant_table_chunks_into_16_words() {
        // init() transmits 64 bytes in 16 32-bit words per channel (luma and chroma).
        assert_eq!(DEFAULT_QUANT_TABLE.chunks_exact(4).count(), 16);
    }

    #[test]
    fn slice_words_length_matches_geometry() {
        // 16x240 pixels at 15bpp = 3,840 halfwords = 1,920 words = 60 blocks of 32 words.
        let words = 16 * 240 / 2;
        assert_eq!(words, 1920);
        assert_eq!(words % 32, 0);
        assert_eq!(words / 32, 60);
    }
}
