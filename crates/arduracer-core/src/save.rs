//! PlayStation 1 Memory Card Save Data Format and Integrity Checksum.
//!
//! Fits within a single 8 KB PlayStation Memory Card block (128 sectors of 64 bytes).
//!
//! # The struct is not the on-disk layout
//!
//! [`SaveData`] is serialised field by field into a fixed [`PAYLOAD_SIZE`]
//! byte buffer rather than memcpy'd. A `repr(C)` copy would drag the struct's
//! padding along with it -- one interior byte before the checksum and two
//! trailing bytes -- and `repr(Rust)` would let the compiler reorder fields.
//! Either way the CRC would end up covering bytes the format never defined,
//! and because that padding is uninitialised, the checksum would depend on
//! whatever happened to be on the stack and nondeterministic bytes would be
//! written to the card. Serialising explicitly makes the checksum cover
//! exactly the bytes that reach the card, by construction.
//!
//! Byte map of the payload (all integers little-endian):
//!
//! | Offset | Size | Field |
//! |--------|------|-------|
//! | 0      | 16   | `magic` |
//! | 16     | 2    | `version` |
//! | 18     | 6    | progress flags and preferences |
//! | 24     | 96   | `best_lap_ticks` (24 × `u32`) |
//! | 120    | 24   | `medals_earned` |
//! | 144    | 15   | `tuning_slots` (3 × 5 sliders) |
//! | 159    | 2    | `checksum` (CRC over bytes 0..159) |

use crate::tuning::CarTuning;

pub const TOTAL_TRACKS: usize = 24;
pub const SAVE_HEADER_MAGIC: [u8; 16] = *b"BAS-ARDURACER-01";
pub const SAVE_VERSION: u16 = 1;

/// On-card payload length: every field is written explicitly, so this is the
/// exact count of meaningful bytes with no padding in it.
pub const PAYLOAD_SIZE: usize = 161;

/// Offset of the trailing CRC within the payload. Every byte before it is
/// covered by the checksum.
pub const CHECKSUM_OFFSET: usize = 159;

/// Offset of `best_lap_ticks`, the first of the fixed-length arrays.
const OFF_BEST_LAPS: usize = 24;

/// 1-block PSX Save Data struct.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
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
    /// Serialises every field into `out` in the documented byte order.
    ///
    /// `out` must be at least [`PAYLOAD_SIZE`] bytes. Bounds-checked indexing
    /// only: no unchecked slices, so a short buffer panics loudly in a host
    /// test rather than corrupting memory on hardware.
    pub fn write_payload(&self, out: &mut [u8]) {
        let mut o = 0;
        out[o..o + 16].copy_from_slice(&self.magic);
        o += 16;
        out[o..o + 2].copy_from_slice(&self.version.to_le_bytes());
        o += 2;
        out[o] = self.max_unlocked_level;
        out[o + 1] = self.sound_volume;
        out[o + 2] = self.music_volume;
        out[o + 3] = self.rumble_enabled;
        out[o + 4] = self.active_tuning_slot;
        out[o + 5] = self._reserved0;
        o += 6;
        for ticks in self.best_lap_ticks.iter() {
            out[o..o + 4].copy_from_slice(&ticks.to_le_bytes());
            o += 4;
        }
        out[o..o + TOTAL_TRACKS].copy_from_slice(&self.medals_earned);
        o += TOTAL_TRACKS;
        for slot in self.tuning_slots.iter() {
            out[o] = slot.top_speed;
            out[o + 1] = slot.acceleration;
            out[o + 2] = slot.handling;
            out[o + 3] = slot.drift_stability;
            out[o + 4] = slot.gearing;
            o += 5;
        }
        // The field offsets are part of the on-card format; a field added to
        // the struct without a matching payload slot must not ship silently.
        debug_assert_eq!(o, CHECKSUM_OFFSET);
        out[o..o + 2].copy_from_slice(&self.checksum.to_le_bytes());
        o += 2;
        debug_assert_eq!(o, PAYLOAD_SIZE);
    }

    /// Reads a payload back, validating magic, version, and the CRC.
    ///
    /// Returns `None` for a short buffer or any failed check, so a truncated
    /// or corrupted save can never be mistaken for real progress.
    pub fn read_payload(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < PAYLOAD_SIZE {
            return None;
        }
        let mut o = 0;
        let mut magic = [0u8; 16];
        magic.copy_from_slice(&bytes[o..o + 16]);
        o += 16;
        let version = u16::from_le_bytes([bytes[o], bytes[o + 1]]);
        o += 2;
        let max_unlocked_level = bytes[o];
        let sound_volume = bytes[o + 1];
        let music_volume = bytes[o + 2];
        let rumble_enabled = bytes[o + 3];
        let active_tuning_slot = bytes[o + 4];
        let reserved = bytes[o + 5];
        o += 6;

        debug_assert_eq!(o, OFF_BEST_LAPS);
        let mut best_lap_ticks = [0u32; TOTAL_TRACKS];
        for ticks in best_lap_ticks.iter_mut() {
            *ticks = u32::from_le_bytes([bytes[o], bytes[o + 1], bytes[o + 2], bytes[o + 3]]);
            o += 4;
        }
        let mut medals_earned = [0u8; TOTAL_TRACKS];
        medals_earned.copy_from_slice(&bytes[o..o + TOTAL_TRACKS]);
        o += TOTAL_TRACKS;

        let mut tuning_slots = [CarTuning::default(); 3];
        for slot in tuning_slots.iter_mut() {
            *slot = CarTuning {
                top_speed: bytes[o],
                acceleration: bytes[o + 1],
                handling: bytes[o + 2],
                drift_stability: bytes[o + 3],
                gearing: bytes[o + 4],
            };
            o += 5;
        }

        debug_assert_eq!(o, CHECKSUM_OFFSET);
        let checksum = u16::from_le_bytes([bytes[o], bytes[o + 1]]);
        o += 2;
        debug_assert_eq!(o, PAYLOAD_SIZE);

        let save = SaveData {
            magic,
            version,
            max_unlocked_level,
            sound_volume,
            music_volume,
            rumble_enabled,
            active_tuning_slot,
            _reserved0: reserved,
            best_lap_ticks,
            medals_earned,
            tuning_slots,
            checksum,
        };
        save.is_valid().then_some(save)
    }

    /// CCITT-16 CRC over the payload bytes that precede the checksum field.
    ///
    /// Computed from the serialised bytes rather than from struct memory, so it
    /// always covers exactly what [`SaveData::write_payload`] puts on the card.
    pub fn compute_checksum(&self) -> u16 {
        let mut payload = [0u8; PAYLOAD_SIZE];
        self.write_payload(&mut payload);
        crc16(&payload[..CHECKSUM_OFFSET])
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
        self.write_payload(&mut block[..PAYLOAD_SIZE]);
    }

    /// Attempts to deserialize save data from a Memory Card block buffer.
    /// Returns Some(SaveData) if magic, version, and checksum are valid, None otherwise.
    pub fn from_block_bytes(slice: &[u8]) -> Option<Self> {
        Self::read_payload(slice)
    }
}

/// CRC-16/CCITT (polynomial 0x1021, initial value 0xFFFF), the checksum the
/// PS1 BIOS itself uses for save blocks.
pub fn crc16(bytes: &[u8]) -> u16 {
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
