//! PlayStation 1 Memory Card Save Data Format and Integrity Checksum.
//!
//! Fits within a single 8 KB PlayStation Memory Card block (128 sectors of 64 bytes).

use crate::tuning::CarTuning;

pub const TOTAL_TRACKS: usize = 24;
pub const SAVE_HEADER_MAGIC: [u8; 16] = *b"BAS-ARDURACER-01";
pub const SAVE_VERSION: u16 = 1;

/// 1-block PSX Save Data struct.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct SaveData {
    pub magic: [u8; 16],
    pub version: u16,
    pub max_unlocked_level: u8,
    pub sound_volume: u8,
    pub music_volume: u8,
    pub rumble_enabled: u8,
    pub active_tuning_slot: u8,
    pub _reserved0: u8,
    pub best_lap_ticks: [u32; TOTAL_TRACKS],
    pub medals_earned: [u8; TOTAL_TRACKS],
    pub tuning_slots: [CarTuning; 3],
    pub checksum: u16,
}

impl Default for SaveData {
    fn default() -> Self {
        let mut save = SaveData {
            magic: SAVE_HEADER_MAGIC,
            version: SAVE_VERSION,
            max_unlocked_level: 1,
            sound_volume: 7,
            music_volume: 7,
            rumble_enabled: 1,
            active_tuning_slot: 0,
            _reserved0: 0,
            best_lap_ticks: [u32::MAX; TOTAL_TRACKS],
            medals_earned: [0; TOTAL_TRACKS],
            tuning_slots: [CarTuning::default(); 3],
            checksum: 0,
        };
        save.checksum = save.compute_checksum();
        save
    }
}

impl SaveData {
    /// Computes CCITT-16 / CRC16 checksum over all fields preceding the checksum field.
    pub fn compute_checksum(&self) -> u16 {
        let self_ptr = self as *const Self as *const u8;
        let checksum_ptr = &self.checksum as *const u16 as *const u8;
        let len = (checksum_ptr as usize) - (self_ptr as usize);
        let bytes = unsafe { core::slice::from_raw_parts(self_ptr, len) };

        let mut crc: u16 = 0xFFFF;
        for &b in bytes {
            crc ^= (b as u16) << 8;
            for _ in 0..8 {
                if (crc & 0x8000) != 0 {
                    crc = (crc << 1) ^ 0x1021;
                } else {
                    crc <<= 1;
                }
            }
        }
        crc
    }

    /// Validates magic header, version, and integrity checksum.
    pub fn is_valid(&self) -> bool {
        self.magic == SAVE_HEADER_MAGIC
            && self.version == SAVE_VERSION
            && self.checksum == self.compute_checksum()
    }

    /// Updates best lap time and recalculates checksum.
    pub fn update_best_lap(&mut self, track_idx: usize, lap_ticks: u32, medal: u8) -> bool {
        if track_idx >= TOTAL_TRACKS {
            return false;
        }
        let mut improved = false;
        if lap_ticks < self.best_lap_ticks[track_idx] {
            self.best_lap_ticks[track_idx] = lap_ticks;
            improved = true;
        }
        if medal > self.medals_earned[track_idx] {
            self.medals_earned[track_idx] = medal;
            improved = true;
        }
        if improved {
            if track_idx + 2 > self.max_unlocked_level as usize {
                self.max_unlocked_level = (track_idx + 2).min(TOTAL_TRACKS) as u8;
            }
            self.checksum = self.compute_checksum();
        }
        improved
    }

    /// Serializes save data into a standard 8 KB PlayStation Memory Card block buffer.
    pub fn to_block_bytes(&self, block: &mut [u8; 8192]) {
        block.fill(0);
        let src = unsafe {
            core::slice::from_raw_parts(
                self as *const Self as *const u8,
                core::mem::size_of::<Self>(),
            )
        };
        block[..src.len()].copy_from_slice(src);
    }

    /// Attempts to deserialize save data from a Memory Card block buffer.
    /// Returns Some(SaveData) if magic, version, and checksum are valid, None otherwise.
    pub fn from_block_bytes(slice: &[u8]) -> Option<Self> {
        let size = core::mem::size_of::<Self>();
        if slice.len() < size {
            return None;
        }
        let mut save = Self::default();
        let dst =
            unsafe { core::slice::from_raw_parts_mut(&mut save as *mut Self as *mut u8, size) };
        dst.copy_from_slice(&slice[..size]);

        if save.is_valid() {
            Some(save)
        } else {
            None
        }
    }
}
