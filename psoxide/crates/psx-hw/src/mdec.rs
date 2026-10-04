// SPDX-License-Identifier: GPL-2.0-or-later
//! MDEC hardware model.
//!
//! The PS1 Motion Decoder decompresses 15-bit YUV macroblocks
//! (16x16 pixels) from a bitstream fed via DMA0 (MDEC-in) and emits
//! decoded pixels via DMA1 (MDEC-out). This crate defines the register
//! addresses and control bitfields; behavior lives in the emulator and
//! SDK driver.
//!
//! Reference: nocash PSX-SPX "MDEC - Motion DECoder" chapter.

/// MDEC data/command register (MDEC0): writing feeds the bitstream,
/// reading drains decoded halfwords when DMA is not used.
pub const MDEC0_DATA: u32 = 0x1F80_1820;

/// MDEC status / control register (MDEC0).
/// Write to configure, read to poll status.
pub const MDEC0_CTRL: u32 = 0x1F80_1824;

/// MDEC write control bits (written to MDEC0_CTRL at 0x1F80_1824).
pub mod ctrl {
    /// Reset the MDEC (bit 31).
    pub const RESET: u32 = 1 << 31;
    /// Enable Data-Out request (bit 30): enables DMA1 (MDEC -> RAM) and port 1F801820h read.
    pub const DMA_OUT_ENABLE: u32 = 1 << 30;
    /// Enable Data-In request (bit 29): enables DMA0 (RAM -> MDEC) and port 1F801820h write.
    pub const DMA_IN_ENABLE: u32 = 1 << 29;
}

/// MDEC command 1 (Decode Macroblock) configuration bits (written to MDEC0_DATA).
pub mod cmd {
    /// Command 1: Decode Macroblock opcode (bits 31-29 = 001b).
    pub const OP_DECODE: u32 = 1 << 29;
    /// Output pixel depth: 15-bit RGB (bits 28-27 = 11b).
    pub const DEPTH_RGB15: u32 = 3 << 27;
    /// Output pixel depth: 24-bit RGB (bits 28-27 = 10b).
    pub const DEPTH_RGB24: u32 = 2 << 27;
    /// Set bit 15 of output 15-bit pixels (bit 25).
    pub const BIT15_SET: u32 = 1 << 25;
}

/// MDEC status bits (read from MDEC0_CTRL).
pub mod status {
    /// Data output FIFO is empty (nothing to read) (bit 31).
    pub const DATA_OUT_EMPTY: u32 = 1 << 31;
    /// Data input FIFO is full (cannot accept more words) (bit 30).
    pub const DATA_IN_FULL: u32 = 1 << 30;
    /// MDEC is busy decoding (bit 29).
    pub const BUSY: u32 = 1 << 29;
    /// Data input FIFO request / ready (DMA0) (bit 28).
    pub const DATA_IN_REQ: u32 = 1 << 28;
    /// Data output FIFO request / ready (DMA1) (bit 27).
    pub const DATA_OUT_REQ: u32 = 1 << 27;
}

/// Decoded macroblock dimensions.
pub const MACROBLOCK_SIZE: usize = 16;
/// Pixels per macroblock.
pub const MACROBLOCK_PIXELS: usize = MACROBLOCK_SIZE * MACROBLOCK_SIZE;
/// Halfwords per macroblock in 15-bit mode (one per pixel).
pub const MACROBLOCK_HALFWORDS: usize = MACROBLOCK_PIXELS;
/// Words per macroblock (two halfwords per word).
pub const MACROBLOCK_WORDS: usize = MACROBLOCK_HALFWORDS / 2;
/// Typical STR frame size in sectors (varies, but 320x240 = 300 macroblocks = 600 halfword pairs).
/// XA Mode2 Form2 payload bytes for STR/MDEC sectors.
pub const STR_SECTOR_USER_BYTES: usize = 2016;
/// Raw CD sector size for STR/MDEC.
pub const STR_SECTOR_RAW_BYTES: usize = 2352;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mdec_registers_in_io_window() {
        use crate::memory::io;
        let end = io::BASE + io::SIZE as u32;
        assert!((io::BASE..end).contains(&MDEC0_DATA));
        assert!((io::BASE..end).contains(&MDEC0_CTRL));
    }
    #[test]
    fn macroblock_math() {
        assert_eq!(MACROBLOCK_PIXELS, 256);
        assert_eq!(MACROBLOCK_WORDS, 128);
    }
    #[test]
    fn ctrl_bits_match_psx_spx() {
        assert_eq!(ctrl::RESET, 0x8000_0000);
        assert_eq!(ctrl::DMA_OUT_ENABLE, 0x4000_0000);
        assert_eq!(ctrl::DMA_IN_ENABLE, 0x2000_0000);
        assert_eq!(ctrl::DMA_OUT_ENABLE | ctrl::DMA_IN_ENABLE, 0x6000_0000);
    }
    #[test]
    fn cmd_bits_encode_rgb15_decode() {
        assert_eq!(cmd::OP_DECODE | cmd::DEPTH_RGB15, 0x3800_0000);
    }
}
