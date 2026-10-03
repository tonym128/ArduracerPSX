//! PlayStation 1 Memory Card Persistence Manager.
//!
//! Manages 8 KB save blocks on Memory Card Port 1 with CRC16 integrity checks,
//! par time records, trophy medals, and car tuning configurations.

use arduracer_core::save::SaveData;

pub const MEMCARD_BLOCK_SIZE: usize = 8192;

pub struct MemoryCardManager {
    pub save_data: SaveData,
    pub card_detected: bool,
    pub is_dirty: bool,
}

impl MemoryCardManager {
    pub fn new() -> Self {
        MemoryCardManager {
            save_data: SaveData::default(),
            card_detected: false,
            is_dirty: false,
        }
    }

    /// Attempts to read save data from a raw 8KB block buffer.
    pub fn load_from_block(&mut self, block: &[u8]) -> bool {
        if let Some(loaded) = SaveData::from_block_bytes(block) {
            self.save_data = loaded;
            self.card_detected = true;
            self.is_dirty = false;
            true
        } else {
            false
        }
    }

    /// Serializes the current save data into an 8KB block buffer.
    pub fn save_to_block(&mut self, block: &mut [u8; MEMCARD_BLOCK_SIZE]) {
        self.save_data.checksum = self.save_data.compute_checksum();
        self.save_data.to_block_bytes(block);
        self.is_dirty = false;
    }

    /// Updates lap record and marks memory card state dirty for saving.
    pub fn record_lap(&mut self, track_idx: usize, lap_ticks: u32, medal: u8) -> bool {
        let improved = self.save_data.update_best_lap(track_idx, lap_ticks, medal);
        if improved {
            self.is_dirty = true;
        }
        improved
    }
}
