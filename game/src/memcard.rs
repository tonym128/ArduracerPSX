//! PlayStation 1 Memory Card Persistence Manager.
//!
//! Talks to the real card on port 1 through `psx-mc` (TASK-702 / GAME.md §8):
//!
//! * BIOS file name `BAS-ARDURACER-01`, human title `ARDURACER PSX`.
//! * 1-block save holding the `SaveData` struct (lap records, medals, tuning
//!   slots, preferences) with its 16-bit CRC.
//! * Custom 16x16 16-colour BIOS icon showing a checkered racing flag.
//! * Every failure path degrades gracefully to in-memory defaults rather than
//!   hanging the 60 Hz frame loop on SIO retries.

use arduracer_core::save::SaveData;
use psx_mc::{Block, Card, SaveIcon, FRAME_SIZE};
#[cfg(not(feature = "host-test"))]
use psx_mc::{HardwareCard, Slot};

pub const MEMCARD_BLOCK_SIZE: usize = 8192;
/// BIOS file name (must stay <= 20 ASCII characters).
pub const SAVE_FILE_NAME: &str = "BAS-ARDURACER-01";
/// Label shown by the console's card manager.
pub const SAVE_TITLE: &str = "ARDURACER PSX";
/// Payload size actually written: the serialised `SaveData`, not a padded 8 KB
/// block, so the file fits in a single 8 KB memory card block. Taken from the
/// format itself so the two can never drift apart.
pub const SAVE_PAYLOAD_SIZE: usize = arduracer_core::save::PAYLOAD_SIZE;

/// How long a status message stays on screen after a card operation.
const STATUS_HOLD_FRAMES: u16 = 150;

/// Outcome of the most recent card operation, surfaced to the UI.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum MemcardStatus {
    /// No attempt made yet (fresh boot).
    #[default]
    Uninitialised,
    /// Loaded an existing save successfully.
    Loaded,
    /// No card, or no save on it: running with defaults.
    FreshProfile,
    /// Card present but the save failed CRC / magic validation.
    Corrupt,
    /// Write failed (card removed, full, or write-protected).
    WriteFailed,
    /// Wrote the save successfully.
    Saved,
}

pub struct MemoryCardManager {
    pub save_data: SaveData,
    /// Scratch buffer used to build the on-card payload.
    pub block: [u8; MEMCARD_BLOCK_SIZE],
    pub card_detected: bool,
    pub is_dirty: bool,
    pub status: MemcardStatus,
    /// Status queued for on-screen display with the frames it has left to
    /// live. A card operation can fail silently -- a pulled card or a full one
    /// costs the player their records -- so the outcome is parked here for the
    /// frame loop to show rather than dropped at the call site.
    pub notice: Option<(MemcardStatus, u16)>,
}

impl Default for MemoryCardManager {
    fn default() -> Self {
        MemoryCardManager::new()
    }
}

impl MemoryCardManager {
    /// Builds an in-memory profile. Called once during boot; the hardware probe
    /// then overwrites it if a card with a valid save is present.
    pub fn new() -> Self {
        MemoryCardManager {
            save_data: SaveData::default(),
            block: [0; MEMCARD_BLOCK_SIZE],
            card_detected: false,
            is_dirty: false,
            status: MemcardStatus::Uninitialised,
            notice: None,
        }
    }

    /// Records the outcome of a card operation and queues it for display.
    ///
    /// Only the outcomes the player needs to act on are queued; a clean load
    /// or a blank card is not worth a message.
    fn set_status(&mut self, status: MemcardStatus) -> MemcardStatus {
        self.status = status;
        if matches!(
            status,
            MemcardStatus::Saved | MemcardStatus::Corrupt | MemcardStatus::WriteFailed
        ) {
            self.notice = Some((status, STATUS_HOLD_FRAMES));
        }
        status
    }

    /// Ages any queued card message out. Call once per rendered frame.
    pub fn tick_notice(&mut self) {
        if let Some((_, frames)) = &mut self.notice {
            *frames = frames.saturating_sub(1);
            if *frames == 0 {
                self.notice = None;
            }
        }
    }

    /// The queued card message, if one is still on screen. Does not consume it.
    pub fn notice(&self) -> Option<MemcardStatus> {
        self.notice.map(|(status, _)| status)
    }

    #[cfg(not(feature = "host-test"))]
    fn card() -> Card<HardwareCard> {
        Card::new(HardwareCard::new(Slot::One))
    }

    /// Opens the card on port 1 and loads the save if one is present.
    ///
    /// Never blocks on failure: the race loop calls this during boot only.
    #[cfg(not(feature = "host-test"))]
    pub fn probe(&mut self) -> MemcardStatus {
        let mut card = Self::card();
        self.probe_with(&mut card)
    }

    /// Transport-agnostic probe (host tests drive this with a RAM card).
    pub fn probe_with<B: Block>(&mut self, card: &mut Card<B>) -> MemcardStatus {
        match card.is_formatted() {
            Ok(true) => {
                self.card_detected = true;
                // `len` is what the drive actually handed back. A file shorter
                // than a payload is a truncated write, so it is treated as
                // corrupt rather than read past.
                match card.read(SAVE_FILE_NAME, &mut self.block) {
                    Ok(len) if len >= SAVE_PAYLOAD_SIZE => {
                        match SaveData::from_block_bytes(&self.block) {
                            Some(save) => {
                                self.save_data = save;
                                self.is_dirty = false;
                                self.set_status(MemcardStatus::Loaded);
                            }
                            None => {
                                // Present but failed magic/version/CRC validation.
                                self.save_data = SaveData::default();
                                self.set_status(MemcardStatus::Corrupt);
                            }
                        }
                    }
                    Ok(_) => {
                        // Payload did not arrive in full.
                        self.save_data = SaveData::default();
                        self.set_status(MemcardStatus::Corrupt);
                    }
                    Err(_) => {
                        // Formatted card with no Arduracer save yet.
                        self.save_data = SaveData::default();
                        self.set_status(MemcardStatus::FreshProfile);
                    }
                }
            }
            Ok(false) => {
                self.card_detected = false;
                self.save_data = SaveData::default();
                self.set_status(MemcardStatus::FreshProfile);
            }
            Err(_) => {
                self.card_detected = false;
                self.save_data = SaveData::default();
                self.set_status(MemcardStatus::FreshProfile);
            }
        }
        self.status
    }

    /// Writes the save to the physical card. Only call when `is_dirty`.
    #[cfg(not(feature = "host-test"))]
    pub fn flush(&mut self) -> MemcardStatus {
        let mut card = Self::card();
        self.flush_with(&mut card)
    }

    /// Transport-agnostic flush (host tests drive this with a RAM card).
    pub fn flush_with<B: Block>(&mut self, card: &mut Card<B>) -> MemcardStatus {
        if !self.is_dirty {
            return self.status;
        }
        self.save_data.checksum = self.save_data.compute_checksum();
        self.save_data.to_block_bytes(&mut self.block);

        let status = match card.write_with_icon(
            SAVE_FILE_NAME,
            SAVE_TITLE,
            &self.block[..SAVE_PAYLOAD_SIZE],
            &racing_flag_icon(),
        ) {
            Ok(()) => {
                self.card_detected = true;
                self.is_dirty = false;
                MemcardStatus::Saved
            }
            // The record stays dirty, so the next flush retries it rather than
            // dropping the player's progress on the floor.
            Err(_) => MemcardStatus::WriteFailed,
        };
        self.set_status(status)
    }

    /// Updates lap record and marks memory card state dirty for saving.
    pub fn record_lap(&mut self, track_idx: usize, lap_ticks: u32, medal: u8) -> bool {
        let improved = self.save_data.update_best_lap(track_idx, lap_ticks, medal);
        if improved {
            self.is_dirty = true;
        }
        improved
    }

    /// Stores one of the three garage tuning presets.
    pub fn store_tuning(&mut self, slot: usize, tuning: arduracer_core::CarTuning) {
        if slot < 3 && tuning.is_valid() {
            self.save_data.tuning_slots[slot] = tuning;
            self.is_dirty = true;
        }
    }
}

/// BIOS icon geometry: a 16x16 4bpp image is one 128-byte card frame.
const ICON_WIDTH: usize = 16;
const ICON_HEIGHT: usize = 16;

/// 16x16 16-colour BIOS icon: a checkered racing flag (GAME.md §8).
///
/// The `psx-mc` filesystem writes a single icon frame, so this is the static
/// representative frame of the animated checkered-flag icon.
fn racing_flag_icon() -> SaveIcon {
    // BGR555 palette: 0 transparent, 1 checker black, 2 checker white,
    // 3 pole grey, 4 highlight red.
    let palette: [u16; 16] = [
        0x0000, // transparent
        0x0000, // black
        0x7FFF, // white
        0x4208, // mid grey
        0x001F, // red (BGR555)
        0x7BEF, // light grey
        0x0841, // dark grey
        0x7E00, // deep red
        0x7C00, 0x7A00, 0x7800, 0x7600, 0x7400, 0x7200, 0x7000, 0x0000,
    ];

    let mut pixels = [0u8; FRAME_SIZE];
    for y in 0..ICON_HEIGHT {
        for x in 0..ICON_WIDTH {
            let index: u8 = if x == 1 && (2..=13).contains(&y) {
                3 // flag pole
            } else if (2..=13).contains(&x) && (2..=8).contains(&y) {
                // 4x3 checkerboard of the flag cloth.
                let c = ((x - 2) / 3 + (y - 2) / 3) % 2;
                if c == 0 {
                    4
                } else {
                    2
                }
            } else if x == 1 && y == 14 {
                3 // pole base
            } else if y == ICON_HEIGHT - 1 {
                6 // ground shadow
            } else {
                0 // transparent
            };
            // 4bpp, left-to-right pixels per byte, low nibble first. A row is
            // 16 pixels = 8 bytes, so the row stride is half the width; using
            // the full width here walks off the end of the 128-byte frame on
            // every row below the eighth.
            let byte = y * ICON_WIDTH / 2 + x / 2;
            if x % 2 == 0 {
                pixels[byte] = (pixels[byte] & 0xF0) | (index & 0x0F);
            } else {
                pixels[byte] = (pixels[byte] & 0x0F) | (index << 4);
            }
        }
    }

    // 16x16 at 4bpp is exactly one 128-byte memory-card icon frame.
    debug_assert_eq!(ICON_WIDTH * ICON_HEIGHT / 2, FRAME_SIZE);
    SaveIcon::new(palette, pixels)
}
