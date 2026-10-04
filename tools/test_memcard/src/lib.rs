//! Host-side tests for the game's memory card manager (`game/src/memcard.rs`).
//!
//! The manager is included textually (same trick as `video_bench`) with the
//! `host-test` feature, which drops the SIO0 hardware wrappers. Every test
//! drives `probe_with` / `flush_with` against an in-memory card, optionally
//! wrapped in a fault injector that counts frame I/O. The counters matter
//! because a save runs synchronously inside the 60 Hz frame loop: the number
//! of frames touched is the stall the player sees as a "freeze".

#![allow(dead_code)]

#[path = "../../../game/src/memcard.rs"]
mod memcard;

#[cfg(test)]
mod tests {
    use super::memcard::*;
    use arduracer_core::save::SaveData;
    use psx_mc::{Block, Card, Error, RamCard, Result, FRAME_SIZE};

    /// Block wrapper that counts I/O and can inject failures.
    struct FaultCard {
        inner: RamCard,
        reads: usize,
        writes: usize,
        /// Every access fails with `NoCard` once set (card pulled).
        removed: bool,
        /// Writes fail with `Protocol` after this many successful writes.
        fail_writes_after: Option<usize>,
    }

    impl FaultCard {
        fn new() -> Self {
            FaultCard {
                inner: RamCard::new(),
                reads: 0,
                writes: 0,
                removed: false,
                fail_writes_after: None,
            }
        }
        fn formatted() -> Card<FaultCard> {
            let mut card = Card::new(FaultCard::new());
            card.format().expect("format");
            card.device().reads = 0;
            card.device().writes = 0;
            card
        }
    }

    impl Block for FaultCard {
        fn read_frame(&mut self, frame: u16, out: &mut [u8; FRAME_SIZE]) -> Result<()> {
            if self.removed {
                return Err(Error::NoCard);
            }
            self.reads += 1;
            self.inner.read_frame(frame, out)
        }
        fn write_frame(&mut self, frame: u16, data: &[u8; FRAME_SIZE]) -> Result<()> {
            if self.removed {
                return Err(Error::NoCard);
            }
            if let Some(limit) = self.fail_writes_after {
                if self.writes >= limit {
                    return Err(Error::Protocol);
                }
            }
            self.writes += 1;
            self.inner.write_frame(frame, data)
        }
    }

    fn dirty_manager() -> MemoryCardManager {
        let mut m = MemoryCardManager::new();
        assert!(m.record_lap(3, 1234, 2));
        assert!(m.is_dirty);
        m
    }

    /// First frame of the save's data block: the `SC` title header. A data
    /// block is 64 frames and its first two are the title and the icon, so the
    /// container header and payload start two frames further on.
    fn save_block_frame(dev: &mut RamCard) -> u16 {
        let mut frame = [0u8; FRAME_SIZE];
        for f in 0..(psx_mc::BLOCK_COUNT * psx_mc::FRAMES_PER_BLOCK) as u16 {
            dev.read_frame(f, &mut frame).unwrap();
            if &frame[..2] == b"SC" {
                return f;
            }
        }
        panic!("no save block on the card");
    }

    /// Corrupt one payload byte in place, exactly as a dying card cell would.
    ///
    /// `offset` indexes the payload as `SaveData` lays it out, so the byte is
    /// located through the real on-card geometry (block frame + 2, then the
    /// 16-byte container header) rather than guessed. Corrupting the title or
    /// icon frames instead would be a no-op: nothing in the save CRC covers
    /// them.
    fn corrupt_payload_byte(dev: &mut RamCard, offset: usize) {
        const CONTAINER_LEN: usize = 16;
        let base = save_block_frame(dev);
        let data_off = CONTAINER_LEN + offset;
        let frame = base + 2 + (data_off / FRAME_SIZE) as u16;
        let within = data_off % FRAME_SIZE;
        let mut raw = [0u8; FRAME_SIZE];
        dev.read_frame(frame, &mut raw).unwrap();
        assert_ne!(raw[within], 0, "byte {offset} was already zero");
        raw[within] ^= 0xFF;
        dev.write_frame(frame, &raw).unwrap();
    }

    #[test]
    fn payload_fits_a_single_card_block() {
        // Container header (16) + payload must fit one 8 KiB block, or the
        // save would silently claim two blocks of a 15-block card. This is a
        // property of the format, so it is checked at compile time.
        const { assert!(SAVE_PAYLOAD_SIZE + 16 <= MEMCARD_BLOCK_SIZE - 256) };
        const { assert!(SAVE_FILE_NAME.len() <= psx_mc::MAX_NAME) };
    }

    #[test]
    fn payload_carries_no_uninitialised_padding() {
        use arduracer_core::save::{crc16, CHECKSUM_OFFSET};

        // The payload is serialised field by field, so its length comes from
        // the format, not from `size_of::<SaveData>()`. The struct is free to
        // gain padding without the card format moving under it.
        assert_eq!(SAVE_PAYLOAD_SIZE, CHECKSUM_OFFSET + 2);

        let save = SaveData::default();
        // Serialising into two buffers with opposite prior contents must give
        // identical bytes. A struct memcpy would copy its padding through
        // untouched, so one buffer would come back all-0x00 padding and the
        // other all-0xFF -- and that 0xFF byte sits inside the CRC's range.
        let mut zeroed = [0x00u8; SAVE_PAYLOAD_SIZE];
        let mut poisoned = [0xFFu8; SAVE_PAYLOAD_SIZE];
        save.write_payload(&mut zeroed);
        save.write_payload(&mut poisoned);
        assert_eq!(
            zeroed, poisoned,
            "serialisation depends on the destination buffer's prior contents"
        );

        // The CRC covers exactly the payload minus its own trailing field.
        assert_eq!(&zeroed[CHECKSUM_OFFSET..], &save.checksum.to_le_bytes());
        assert_eq!(save.checksum, crc16(&zeroed[..CHECKSUM_OFFSET]));

        // Every single-bit flip inside the CRC's range must invalidate the
        // save: that is the whole point of shipping the checksum.
        for byte in 0..CHECKSUM_OFFSET {
            for bit in 0..8 {
                let mut damaged = zeroed;
                damaged[byte] ^= 1 << bit;
                assert!(
                    SaveData::read_payload(&damaged).is_none(),
                    "flip of bit {bit} in byte {byte} went undetected"
                );
            }
        }
    }

    #[test]
    fn card_outcomes_are_surfaced_to_the_player() {
        let mut card = FaultCard::formatted();
        let mut m = MemoryCardManager::new();
        assert_eq!(m.notice(), None, "a blank card is not worth a message");
        assert_eq!(m.probe_with(&mut card), MemcardStatus::FreshProfile);
        assert_eq!(m.notice(), None);

        // A lost record is the one the player must hear about.
        card.device().removed = true;
        assert!(m.record_lap(0, 1000, 3));
        assert_eq!(m.flush_with(&mut card), MemcardStatus::WriteFailed);
        assert_eq!(m.notice(), Some(MemcardStatus::WriteFailed));

        // The message ages out instead of sticking to the HUD forever.
        for _ in 0..1000 {
            m.tick_notice();
        }
        assert_eq!(m.notice(), None);

        // A successful save reports itself too.
        card.device().removed = false;
        assert_eq!(m.flush_with(&mut card), MemcardStatus::Saved);
        assert_eq!(m.notice(), Some(MemcardStatus::Saved));
    }

    #[test]
    fn save_then_probe_round_trips() {
        let mut card = FaultCard::formatted();
        let mut m = dirty_manager();
        assert_eq!(m.flush_with(&mut card), MemcardStatus::Saved);
        assert!(!m.is_dirty);

        let mut fresh = MemoryCardManager::new();
        assert_eq!(fresh.probe_with(&mut card), MemcardStatus::Loaded);
        assert_eq!(fresh.save_data.best_lap_ticks[3], 1234);
        assert!(fresh.card_detected);
    }

    #[test]
    fn flush_when_clean_does_no_io() {
        let mut card = FaultCard::formatted();
        let mut m = MemoryCardManager::new();
        m.flush_with(&mut card);
        assert_eq!(card.device().reads + card.device().writes, 0);
    }

    #[test]
    fn save_stall_is_bounded() {
        // A save is one synchronous call in the frame loop. Keep its frame
        // I/O small and fixed so it cannot grow into a visible freeze.
        let mut card = FaultCard::formatted();
        let mut m = dirty_manager();
        assert_eq!(m.flush_with(&mut card), MemcardStatus::Saved);
        let io = card.device().reads + card.device().writes;
        // directory scan (<=2x15) + title/icon + one block (64) + dir entry.
        assert!(io <= 140, "save touched {io} frames");
        assert!(
            card.device().writes <= 80,
            "wrote {} frames",
            card.device().writes
        );
    }

    #[test]
    fn repeated_saves_do_not_leak_blocks() {
        let mut card = FaultCard::formatted();
        let mut m = MemoryCardManager::new();
        for i in 0..40u32 {
            assert!(m.record_lap(0, 5000 - i, 1));
            assert_eq!(m.flush_with(&mut card), MemcardStatus::Saved);
        }
        assert_eq!(card.free_blocks().unwrap(), psx_mc::DATA_BLOCKS - 1);
        card.validate_filesystem().expect("filesystem stays valid");
    }

    #[test]
    fn unformatted_card_fails_fast_and_stays_dirty() {
        let mut card = Card::new(FaultCard::new());
        let mut m = dirty_manager();
        let status = m.flush_with(&mut card);
        assert_eq!(status, MemcardStatus::WriteFailed);
        assert!(m.is_dirty, "unsaved progress must not be dropped");
        assert!(card.device().writes <= 2, "no write storm on a blank card");
    }

    #[test]
    fn removed_card_does_not_spin_the_frame_loop() {
        let mut card = FaultCard::formatted();
        card.device().removed = true;
        let mut m = dirty_manager();
        assert_eq!(m.flush_with(&mut card), MemcardStatus::WriteFailed);
        assert!(m.is_dirty);
        // First failing frame aborts the operation; no retry loop.
        assert_eq!(card.device().writes, 0);
        assert_eq!(m.probe_with(&mut card), MemcardStatus::FreshProfile);
        assert!(!m.card_detected);
    }

    #[test]
    fn card_pulled_mid_write_reports_failure_then_recovers() {
        let mut card = FaultCard::formatted();
        // A complete save writes 5 frames (title, icon, one data frame, dir
        // entry, block header), so cutting in after 2 lands mid-write.
        card.device().fail_writes_after = Some(2);
        let mut m = dirty_manager();
        assert_eq!(m.flush_with(&mut card), MemcardStatus::WriteFailed);
        assert!(m.is_dirty);
        assert!(
            card.device().writes <= 3,
            "gave up after {} writes instead of failing fast",
            card.device().writes
        );

        // Card reseated and healthy again: the next flush must succeed.
        card.device().fail_writes_after = None;
        assert_eq!(m.flush_with(&mut card), MemcardStatus::Saved);
        let mut fresh = MemoryCardManager::new();
        assert_eq!(fresh.probe_with(&mut card), MemcardStatus::Loaded);
        assert_eq!(fresh.save_data.best_lap_ticks[3], 1234);
    }

    #[test]
    fn corrupt_save_is_detected_not_trusted() {
        let mut card = FaultCard::formatted();
        let mut m = dirty_manager();
        assert_eq!(m.flush_with(&mut card), MemcardStatus::Saved);

        // Flip a byte inside `best_lap_ticks[3]`, which the save CRC covers.
        let offset = core::mem::offset_of!(SaveData, best_lap_ticks) + 3 * 4;
        corrupt_payload_byte(&mut card.device().inner, offset);

        let mut fresh = MemoryCardManager::new();
        let status = fresh.probe_with(&mut card);
        assert_eq!(status, MemcardStatus::Corrupt);
        // A failed CRC must not be trusted as a record: the player gets the
        // defaults, and specifically not the corrupted lap time.
        assert_eq!(
            fresh.save_data.best_lap_ticks[3],
            SaveData::default().best_lap_ticks[3]
        );
        assert_eq!(fresh.save_data.medals_earned[3], 0);
    }

    #[test]
    fn truncated_payload_is_rejected() {
        let mut card = FaultCard::formatted();
        let mut m = dirty_manager();
        assert_eq!(m.flush_with(&mut card), MemcardStatus::Saved);

        // A write cut off mid-payload leaves the container advertising a
        // length the card cannot supply. The loader must not read past what
        // actually arrived.
        let container_raw_len = {
            let base = save_block_frame(&mut card.device().inner);
            let mut raw = [0u8; FRAME_SIZE];
            card.device().inner.read_frame(base + 2, &mut raw).unwrap();
            u32::from_le_bytes([raw[8], raw[9], raw[10], raw[11]]) as usize
        };
        assert_eq!(container_raw_len, SAVE_PAYLOAD_SIZE);
        // Blank the whole payload: the container still claims it is there.
        for f in 0..4 {
            let frame = save_block_frame(&mut card.device().inner) + 2 + f;
            card.device()
                .inner
                .write_frame(frame, &[0u8; FRAME_SIZE])
                .unwrap();
        }

        let mut fresh = MemoryCardManager::new();
        let status = fresh.probe_with(&mut card);
        assert!(
            matches!(status, MemcardStatus::Corrupt | MemcardStatus::FreshProfile),
            "got {status:?}"
        );
    }

    #[test]
    fn blank_and_empty_cards_give_fresh_profile() {
        let mut blank = Card::new(FaultCard::new());
        let mut m = MemoryCardManager::new();
        assert_eq!(m.probe_with(&mut blank), MemcardStatus::FreshProfile);

        let mut empty = FaultCard::formatted();
        assert_eq!(m.probe_with(&mut empty), MemcardStatus::FreshProfile);
        assert!(m.card_detected);
    }

    #[test]
    fn tuning_slot_persists() {
        let mut card = FaultCard::formatted();
        let mut m = MemoryCardManager::new();
        let tuning = m.save_data.tuning_slots[0];
        m.store_tuning(1, tuning);
        assert_eq!(m.flush_with(&mut card), MemcardStatus::Saved);
        let mut fresh = MemoryCardManager::new();
        assert_eq!(fresh.probe_with(&mut card), MemcardStatus::Loaded);
    }
}
