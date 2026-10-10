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
//!
//! # A valid checksum is not a valid save
//!
//! The CRC proves the bytes arrived intact; it says nothing about whether the
//! *values* in them are legal. A card that was half-written before, a save
//! editor, or a byte pattern that simply happens to satisfy the polynomial can
//! all carry a perfect checksum and an `active_tuning_slot` of 255 (later used
//! as `tuning_slots[..3]`), a `max_unlocked_level` of 0 (progression bricked
//! behind a race that can no longer be started), a 200 top-speed slider (a
//! ~70 u/tick car), or an implausible lap time. [`SaveData::read_payload`]
//! therefore sanitises every field after the checksum passes -- see
//! [`SaveData::sanitised`].

use crate::championship::{ChampionshipSession, Difficulty, CUP_COUNT, STAGES_PER_CUP};
use crate::timing::{Medal, MIN_PLAUSIBLE_LAP_TICKS, NO_BEST_LAP};
use crate::tuning::CarTuning;

pub const TOTAL_TRACKS: usize = 24;
pub const SAVE_HEADER_MAGIC: [u8; 16] = *b"BAS-ARDURACER-01";
pub const SAVE_VERSION: u16 = 1;

/// Tuning presets stored per save.
pub const TOTAL_TUNING_SLOTS: usize = 3;

/// Highest medal a save can record, as stored in `medals_earned`.
pub const MAX_MEDAL: u8 = Medal::DevPlatinum as u8;

/// Highest value the (still unwired) volume sliders may hold.
///
/// `SaveData::default` ships 7; the mixer takes a 0..=8 range so a corrupt 255
/// cannot arrive as a full-scale or wrapped setting.
pub const MAX_VOLUME: u8 = 8;

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
    pub tuning_slots: [CarTuning; TOTAL_TUNING_SLOTS],
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
            best_lap_ticks: [NO_BEST_LAP; TOTAL_TRACKS],
            medals_earned: [0; TOTAL_TRACKS],
            tuning_slots: [CarTuning::default(); TOTAL_TUNING_SLOTS],
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
    ///
    /// A checksum only proves the bytes survived the trip, so everything the
    /// CRC accepted is then put through [`SaveData::sanitised`]: no value that
    /// arrives from the card may index out of bounds, brick progression, or
    /// produce a car no tuning menu could build.
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

        let mut tuning_slots = [CarTuning::default(); TOTAL_TUNING_SLOTS];
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
        // Checksum first: sanitising corrupt-but-CRC-valid data is the point,
        // sanitising data that failed its CRC would launder a damaged save into
        // a trusted one.
        if !save.is_valid() {
            return None;
        }
        Some(save.sanitised())
    }

    /// Clamps every field into the domain the game can actually use.
    ///
    /// Clamping rather than rejecting, because most of what goes wrong on a card
    /// is one byte in one field and the other 160 bytes are a player's real
    /// progress; throwing the lot away would lose a championship for a bad
    /// volume. Each field is handled according to what a wrong value would cost:
    ///
    /// * `active_tuning_slot` -> last slot. Read as `tuning_slots[..3]`, so 255
    ///   is an out-of-bounds index, not a preference.
    /// * `max_unlocked_level` -> `1 ..= 24`. Zero is a brick: progression can
    ///   only ever be raised by finishing a race, and with nothing unlocked there
    ///   is no race to finish.
    /// * `sound_volume` / `music_volume` -> `0 ..= MAX_VOLUME`.
    /// * `rumble_enabled` -> 0 or 1 (it is a flag).
    /// * `_reserved0` -> 0.
    /// * `medals_earned` -> `0 ..= MAX_MEDAL`; a medal above Dev Platinum is not
    ///   a display state, so it would index past the medal table.
    /// * `best_lap_ticks` -> the `NO_BEST_LAP` sentinel or a plausible time. A
    ///   stored 0, 1 or 2 ticks is the cheat the timer now refuses to produce,
    ///   and there is no honest way it got onto the card, so it becomes "no lap"
    ///   (together with any medal claimed for it).
    /// * `tuning_slots` -> sliders inside `MIN_SLIDER ..= MAX_SLIDER`, and a slot
    ///   that still violates the 20-point budget falls back to the default. A
    ///   top_speed of 200 would otherwise scale the car to ~70 u/tick, ~20x the
    ///   arcade top speed.
    ///
    /// The checksum is recomputed afterwards so the returned save is internally
    /// consistent and the next write persists the repaired values. A save that
    /// needed no repair is returned untouched, checksum included.
    pub fn sanitised(mut self) -> Self {
        self.active_tuning_slot = self.active_tuning_slot.min((TOTAL_TUNING_SLOTS - 1) as u8);
        self.max_unlocked_level = self.max_unlocked_level.clamp(1, TOTAL_TRACKS as u8);
        self.sound_volume = self.sound_volume.min(MAX_VOLUME);
        self.music_volume = self.music_volume.min(MAX_VOLUME);
        self.rumble_enabled = u8::from(self.rumble_enabled != 0);
        // _reserved0 stores Grand Prix progression when active.
        // A valid active championship has:
        // - active flag bit 0 set (0x01)
        // - cup index (bits 1..=2) < CUP_COUNT (4)
        // - stage index (bits 3..=5) < STAGES_PER_CUP (6)
        // - difficulty (bits 6..=7) <= 2
        // If inactive or invalid, clear to 0.
        // To prevent an arbitrary corrupt byte (like 9 = active=1, cup=0, stage=1, diff=0)
        // from being treated as a valid save when raw reserved bytes are tested,
        // we can store difficulty encoded as (diff + 1) in bits 6..=7 (so diff+1 is 1, 2, or 3, never 0)
        // when active.
        if (self._reserved0 & 0x01) != 0 {
            let stage = (self._reserved0 >> 3) & 0x07;
            let diff_code = (self._reserved0 >> 6) & 0x03;
            if stage >= STAGES_PER_CUP || diff_code == 0 || diff_code > 3 {
                self._reserved0 = 0;
            }
        } else {
            self._reserved0 = 0;
        }

        for i in 0..TOTAL_TRACKS {
            self.medals_earned[i] = self.medals_earned[i].min(MAX_MEDAL);
            let ticks = self.best_lap_ticks[i];
            if ticks != NO_BEST_LAP && ticks < MIN_PLAUSIBLE_LAP_TICKS {
                self.best_lap_ticks[i] = NO_BEST_LAP;
                // A medal for a lap that cannot exist is the same lie.
                self.medals_earned[i] = 0;
            }
        }

        for slot in self.tuning_slots.iter_mut() {
            *slot = sanitised_tuning(*slot);
        }

        self.checksum = self.compute_checksum();
        self
    }

    /// Whether every field is inside the domain [`SaveData::sanitised`] enforces.
    ///
    /// Always true for a save that came through [`SaveData::read_payload`]; it
    /// exists so the memory-card layer can notice a save it assembled itself
    /// (or another crate mutated) is out of range.
    pub fn has_legal_fields(&self) -> bool {
        self.active_tuning_slot < TOTAL_TUNING_SLOTS as u8
            && self.max_unlocked_level >= 1
            && self.max_unlocked_level <= TOTAL_TRACKS as u8
            && self.sound_volume <= MAX_VOLUME
            && self.music_volume <= MAX_VOLUME
            && self.rumble_enabled <= 1
            && self.medals_earned.iter().all(|&m| m <= MAX_MEDAL)
            && self
                .best_lap_ticks
                .iter()
                .all(|&t| t == NO_BEST_LAP || t >= MIN_PLAUSIBLE_LAP_TICKS)
            && self.tuning_slots.iter().all(|s| s.is_valid())
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
    ///
    /// Refuses a time that could not physically be driven (and the
    /// `NO_BEST_LAP` sentinel, which means "no lap"): the memory card is the
    /// permanent record, so a cheat that reaches this function would otherwise
    /// outlast the session that made it. `medal` is clamped to
    /// [`MAX_MEDAL`], which is what a legitimate `evaluate_medal` can return.
    pub fn update_best_lap(&mut self, track_idx: usize, lap_ticks: u32, medal: u8) -> bool {
        if track_idx >= TOTAL_TRACKS {
            return false;
        }
        if lap_ticks < MIN_PLAUSIBLE_LAP_TICKS || lap_ticks == NO_BEST_LAP {
            return false;
        }
        let medal = medal.min(MAX_MEDAL);
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
    /// Fields outside their legal range are clamped; see [`SaveData::sanitised`].
    pub fn from_block_bytes(slice: &[u8]) -> Option<Self> {
        Self::read_payload(slice)
    }

    /// Returns true if an active championship session is saved on the card.
    pub fn has_saved_championship(&self) -> bool {
        (self._reserved0 & 0x01) != 0
    }

    /// Loads the in-progress championship session if present and valid.
    pub fn load_championship(&self) -> Option<ChampionshipSession> {
        if !self.has_saved_championship() {
            return None;
        }
        let cup = (self._reserved0 >> 1) & 0x03;
        let stage = (self._reserved0 >> 3) & 0x07;
        let diff_code = (self._reserved0 >> 6) & 0x03;
        if diff_code == 0 || diff_code > 3 {
            return None;
        }
        let difficulty = Difficulty::from_u8(diff_code - 1);

        if cup as usize >= CUP_COUNT || stage >= STAGES_PER_CUP {
            return None;
        }

        let mut session = ChampionshipSession::with_difficulty(cup, difficulty);
        session.current_stage = stage;
        Some(session)
    }

    /// Saves the current championship session state and recomputes the checksum.
    pub fn save_championship(&mut self, session: &ChampionshipSession) {
        let active = 1u8;
        let cup = (session.cup_index & 0x03) << 1;
        let stage = (session.current_stage & 0x07) << 3;
        let diff_code = ((session.difficulty as u8 + 1) & 0x03) << 6;
        self._reserved0 = active | cup | stage | diff_code;
        self.checksum = self.compute_checksum();
    }

    /// Clears any active championship state from the save data.
    pub fn clear_championship(&mut self) {
        if self._reserved0 != 0 {
            self._reserved0 = 0;
            self.checksum = self.compute_checksum();
        }
    }
}

/// Repairs one loaded tuning slot.
///
/// Sliders are clamped into the menu's range first so a single corrupt byte
/// costs one point of allocation rather than the whole preset. A slot that still
/// breaks the 20-point budget afterwards is structurally impossible -- no
/// legitimate writer emits one -- so it falls back to the default.
fn sanitised_tuning(slot: CarTuning) -> CarTuning {
    let clamped = CarTuning {
        top_speed: slot
            .top_speed
            .clamp(crate::tuning::MIN_SLIDER, crate::tuning::MAX_SLIDER),
        acceleration: slot
            .acceleration
            .clamp(crate::tuning::MIN_SLIDER, crate::tuning::MAX_SLIDER),
        handling: slot
            .handling
            .clamp(crate::tuning::MIN_SLIDER, crate::tuning::MAX_SLIDER),
        drift_stability: slot
            .drift_stability
            .clamp(crate::tuning::MIN_SLIDER, crate::tuning::MAX_SLIDER),
        gearing: slot
            .gearing
            .clamp(crate::tuning::MIN_SLIDER, crate::tuning::MAX_SLIDER),
    };
    if clamped.is_valid() {
        clamped
    } else {
        CarTuning::default()
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::FP_ONE;
    use crate::timing::MIN_PLAUSIBLE_LAP_TICKS;

    /// Serialises `save` the way the card would hold it.
    fn payload(save: &SaveData) -> [u8; PAYLOAD_SIZE] {
        let mut out = [0u8; PAYLOAD_SIZE];
        save.write_payload(&mut out);
        out
    }

    /// The attack this whole module defends against: a save whose CRC is
    /// *correct* for values no legitimate writer could produce. Built by
    /// mutating the struct and re-checksumming, so the loader has to catch the
    /// values rather than the checksum.
    fn corrupt(mutate: impl FnOnce(&mut SaveData)) -> SaveData {
        let mut save = SaveData::default();
        mutate(&mut save);
        save.checksum = save.compute_checksum();
        assert!(save.is_valid(), "the crafted save must pass the CRC");
        save
    }

    /// Round-trips a crafted save through the payload exactly as the card
    /// manager does.
    fn load(save: &SaveData) -> SaveData {
        match SaveData::read_payload(&payload(save)) {
            Some(loaded) => loaded,
            None => panic!("a CRC-valid payload must load"),
        }
    }

    #[test]
    fn a_clean_save_is_returned_byte_for_byte() {
        let mut original = SaveData::default();
        assert!(original.update_best_lap(0, 1845, 3));
        assert!(original.update_best_lap(5, 2300, 2));

        let restored = match SaveData::from_block_bytes(&{
            let mut block = [0u8; 8192];
            original.to_block_bytes(&mut block);
            block
        }) {
            Some(s) => s,
            None => panic!("a save this module wrote must load"),
        };
        assert_eq!(restored, original, "sanitising must be a no-op here");
        assert_eq!(restored.checksum, original.checksum);
        assert!(restored.is_valid());
        assert!(restored.has_legal_fields());
    }

    #[test]
    fn an_out_of_range_tuning_slot_cannot_index_out_of_bounds() {
        // game/src/main.rs:222 reads `tuning_slots[active_tuning_slot as usize]`.
        let loaded = load(&corrupt(|s| s.active_tuning_slot = 255));
        assert!(
            (loaded.active_tuning_slot as usize) < TOTAL_TUNING_SLOTS,
            "slot {} would index past the array",
            loaded.active_tuning_slot
        );
        let _ = loaded.tuning_slots[loaded.active_tuning_slot as usize];
        assert!(loaded.has_legal_fields());
    }

    #[test]
    fn zero_max_unlocked_level_does_not_brick_progression() {
        // `update_best_lap` can only raise it, and a race has to be startable
        // to raise it: 0 is unrecoverable.
        let loaded = load(&corrupt(|s| s.max_unlocked_level = 0));
        assert_eq!(
            loaded.max_unlocked_level, 1,
            "the first level is always open"
        );
        assert!(loaded.max_unlocked_level <= TOTAL_TRACKS as u8);

        let loaded = load(&corrupt(|s| s.max_unlocked_level = 255));
        assert_eq!(loaded.max_unlocked_level, TOTAL_TRACKS as u8);
        assert!(loaded.has_legal_fields());
    }

    #[test]
    fn mixer_and_flag_fields_are_clamped() {
        let loaded = load(&corrupt(|s| {
            s.sound_volume = 255;
            s.music_volume = 200;
            s.rumble_enabled = 77;
            s._reserved0 = 9;
        }));
        assert_eq!(loaded.sound_volume, MAX_VOLUME);
        assert_eq!(loaded.music_volume, MAX_VOLUME);
        assert_eq!(loaded.rumble_enabled, 1, "rumble is a flag");
        assert_eq!(loaded._reserved0, 0);

        let loaded = load(&corrupt(|s| {
            s.sound_volume = 0;
            s.music_volume = 0;
            s.rumble_enabled = 0;
        }));
        assert_eq!(loaded.sound_volume, 0, "muted is legal");
        assert_eq!(loaded.rumble_enabled, 0);
        assert!(loaded.has_legal_fields());
    }

    #[test]
    fn a_medal_above_dev_platinum_is_clamped() {
        let loaded = load(&corrupt(|s| s.medals_earned[3] = 200));
        assert_eq!(loaded.medals_earned[3], MAX_MEDAL);
        assert!(loaded.has_legal_fields());

        let mut all_bad = SaveData::default();
        for m in all_bad.medals_earned.iter_mut() {
            *m = 255;
        }
        all_bad.checksum = all_bad.compute_checksum();
        let loaded = load(&all_bad);
        assert!(loaded.medals_earned.iter().all(|&m| m <= MAX_MEDAL));
    }

    #[test]
    fn an_undriveable_stored_lap_becomes_no_lap() {
        // The 1-tick Dev Platinum exploit, arriving from the card rather than
        // from the timer: 1 tick persisted, and on load it is not a record.
        let loaded = load(&corrupt(|s| {
            s.best_lap_ticks[0] = 1;
            s.medals_earned[0] = MAX_MEDAL;
        }));
        assert_eq!(loaded.best_lap_ticks[0], NO_BEST_LAP);
        assert_eq!(loaded.medals_earned[0], 0, "no medal without a lap");

        let loaded = load(&corrupt(|s| s.best_lap_ticks[1] = 0));
        assert_eq!(loaded.best_lap_ticks[1], NO_BEST_LAP);

        // A real time on another track is untouched by the repair.
        let loaded = load(&corrupt(|s| s.best_lap_ticks[2] = 1234));
        assert_eq!(loaded.best_lap_ticks[2], 1234);
        assert!(loaded.has_legal_fields());
    }

    #[test]
    fn a_corrupt_tuning_slot_cannot_build_an_absurd_car() {
        // top_speed = 200 with everything else at 4: `scale_factor(200)` is
        // ~49x, i.e. a ~70 u/tick car. `CarTuning::is_valid` would have said no,
        // but nothing called it on loaded data.
        let loaded = load(&corrupt(|s| {
            s.tuning_slots[0] = CarTuning {
                top_speed: 200,
                acceleration: 4,
                handling: 4,
                drift_stability: 4,
                gearing: 4,
            };
        }));
        assert!(loaded.tuning_slots[0].is_valid());
        assert_eq!(loaded.tuning_slots[0], CarTuning::default());
        assert_eq!(
            CarTuning::scale_factor(loaded.tuning_slots[0].top_speed).raw(),
            FP_ONE,
            "a loaded save must not scale the car"
        );
        assert!(loaded.has_legal_fields());

        // A slot that is in range but breaks the 20-point budget is structurally
        // impossible too.
        let loaded = load(&corrupt(|s| {
            s.tuning_slots[1] = CarTuning {
                top_speed: 7,
                acceleration: 7,
                handling: 7,
                drift_stability: 7,
                gearing: 7,
            };
        }));
        assert_eq!(loaded.tuning_slots[1], CarTuning::default());

        // A legal preset from the tuning sweep survives untouched.
        let sweep = CarTuning {
            top_speed: 7,
            acceleration: 5,
            handling: 4,
            drift_stability: 2,
            gearing: 2,
        };
        assert!(sweep.is_valid());
        let loaded = load(&corrupt(|s| s.tuning_slots[2] = sweep));
        assert_eq!(loaded.tuning_slots[2], sweep);
    }

    #[test]
    fn a_repaired_save_round_trips_and_is_rewritable() {
        let damaged = corrupt(|s| {
            s.active_tuning_slot = 200;
            s.max_unlocked_level = 0;
            s.sound_volume = 255;
            s.best_lap_ticks[4] = 2;
            s.medals_earned[4] = 99;
            s.tuning_slots[0].top_speed = 255;
        });
        let repaired = load(&damaged);

        // Internally consistent, so it can be written back to the card as-is.
        assert!(repaired.is_valid());
        assert!(repaired.has_legal_fields());
        let mut block = [0u8; 8192];
        repaired.to_block_bytes(&mut block);
        let again = match SaveData::from_block_bytes(&block) {
            Some(s) => s,
            None => panic!("the repaired save must reload"),
        };
        assert_eq!(again, repaired, "sanitising must be idempotent");
        assert!(again.is_valid());
    }

    #[test]
    fn sanitising_never_launders_a_checksum_failure() {
        let mut good = payload(&SaveData::default());
        // Flip the reserved byte: legal-looking garbage, but the CRC must fail.
        good[23] ^= 0xFF;
        assert!(SaveData::read_payload(&good).is_none());

        // Same for every single-bit flip of the header and progress flags.
        for byte in 0..24usize {
            for bit in 0..8 {
                let mut damaged = payload(&SaveData::default());
                damaged[byte] ^= 1 << bit;
                assert!(
                    SaveData::read_payload(&damaged).is_none(),
                    "flip of bit {bit} in byte {byte} went undetected"
                );
            }
        }
    }

    #[test]
    fn update_best_lap_refuses_a_time_no_car_could_drive() {
        let mut save = SaveData::default();
        // The end-to-end bug: a cheat lap reaching the card layer directly.
        assert!(
            !save.update_best_lap(0, 1, MAX_MEDAL),
            "a 1-tick lap must not become a record"
        );
        assert_eq!(save.best_lap_ticks[0], NO_BEST_LAP);
        assert_eq!(save.medals_earned[0], 0);
        assert_eq!(
            save.max_unlocked_level, 1,
            "and it must not unlock anything"
        );

        // The sentinel leaking from a race with no recorded lap is refused too.
        assert!(!save.update_best_lap(0, NO_BEST_LAP, 0));
        assert_eq!(save.best_lap_ticks[0], NO_BEST_LAP);

        // A real lap is recorded, and an out-of-range index still is not.
        assert!(save.update_best_lap(0, MIN_PLAUSIBLE_LAP_TICKS, MAX_MEDAL));
        assert_eq!(
            save.best_lap_ticks[0], MIN_PLAUSIBLE_LAP_TICKS,
            "the floor itself must be recordable"
        );
        assert!(!save.update_best_lap(TOTAL_TRACKS, 600, 3));

        // A medal above the enum is clamped rather than persisted.
        assert!(save.update_best_lap(1, 900, 200));
        assert_eq!(save.medals_earned[1], MAX_MEDAL);
        assert!(save.is_valid());
        assert!(save.has_legal_fields());
    }

    #[test]
    fn championship_save_and_load_roundtrips() {
        let mut save = SaveData::default();
        assert!(!save.has_saved_championship());
        assert!(save.load_championship().is_none());

        let mut session = ChampionshipSession::with_difficulty(2, Difficulty::Hard);
        session.current_stage = 4;
        save.save_championship(&session);

        assert!(save.has_saved_championship());
        assert!(save.is_valid());

        // Roundtrip via block bytes
        let mut block = [0u8; 8192];
        save.to_block_bytes(&mut block);

        let loaded = SaveData::from_block_bytes(&block).expect("must parse");
        assert!(loaded.has_saved_championship());
        let loaded_session = loaded.load_championship().expect("must load session");
        assert_eq!(loaded_session.cup_index, 2);
        assert_eq!(loaded_session.current_stage, 4);
        assert_eq!(loaded_session.difficulty, Difficulty::Hard);

        // Clear championship
        let mut cleared = loaded;
        cleared.clear_championship();
        assert!(!cleared.has_saved_championship());
        assert!(cleared.load_championship().is_none());
        assert!(cleared.is_valid());
    }
}
