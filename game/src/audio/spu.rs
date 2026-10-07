//! PSX SPU Driver and Sample Bank Loader.
//!
//! Initializes SPU hardware registers and transfers pre-cooked ADPCM audio
//! blocks into SPU Sound RAM (starting at 0x1010).

use crate::audio::soundbank::*;
use psx_spu::{self as spu, Adsr, CdVolume, SpuAddr, Voice, Volume};

pub const VOICE_ENGINE: Voice = Voice::V0;
pub const VOICE_SKID: Voice = Voice::V1;
pub const VOICE_CRASH: Voice = Voice::V2;
pub const VOICE_BOOST: Voice = Voice::V3;
pub const VOICE_CHIME: Voice = Voice::V4;
pub const VOICE_INTRO: Voice = Voice::V5;
pub const VOICE_UI: Voice = Voice::V6;
/// Curb rumble. Needs its own voice: the skid voice is keyed on and off by the
/// drift state machine, so sharing it made the rumble inaudible (TASK-1207a).
pub const VOICE_CURB: Voice = Voice::V7;

/// Curb rumble level and pitch. Quieter and lower than the skid voice so it
/// reads as a rumble strip rather than a squeal.
pub const CURB_VOLUME: i16 = 0x0800;
pub const CURB_PITCH: u16 = 0x0800;

// SPU RAM layout, in upload order. The intro sample is placed *after* the
// soundbank rather than at a fixed low address: the bank grows every time a
// sample is added, and a fixed intro address silently started overlapping the
// tail of the bank (TASK-1207). `BANK_END` must therefore be >= 0x5000.
pub const BANK_START: u32 = 0x1010;
pub const INTRO_ADDR: SpuAddr = SpuAddr::new(0x5000);
pub const INTRO_SAMPLE_RATE: u32 = 16000;
pub const INTRO_ADPCM_DATA: &[u8] = include_bytes!("../../../assets/INTRO.ADPCM");

/// Prepares and begins playback of intro attract audio via SPU Voice 5.
pub fn play_intro_audio() {
    if INTRO_ADPCM_DATA.len() <= 16 {
        return;
    }
    spu::init();
    spu::set_main_volume(Volume::MAX, Volume::MAX);
    spu::upload_adpcm(INTRO_ADDR, INTRO_ADPCM_DATA);
    VOICE_INTRO.configure_sample(INTRO_ADDR, INTRO_SAMPLE_RATE, Volume::MAX, Adsr::sample());
    Voice::key_on(VOICE_INTRO.mask());
}

/// Immediately silences the intro attract audio.
pub fn stop_intro_audio() {
    Voice::key_off(VOICE_INTRO.mask());
}

/// SPU Sound RAM layout descriptor for all game sound effects.
pub struct SpuSoundbankAddrs {
    pub engine_addr: SpuAddr,
    pub skid_addr: SpuAddr,
    pub crash_addr: SpuAddr,
    pub boost_addr: SpuAddr,
    pub chime_addr: SpuAddr,
    pub ui_addr: SpuAddr,
}

/// Where the soundbank ends once every sample is rounded up to the 8-byte
/// alignment `init_spu_soundbank` applies. Mirrors that function exactly; the
/// lengths are consts, so this folds at compile time.
pub const fn bank_end() -> u32 {
    let mut offset = BANK_START;
    offset = (offset + ENGINE_LOOP.len() as u32 + 7) & !7;
    offset = (offset + TIRE_SCREECH.len() as u32 + 7) & !7;
    offset = (offset + CRASH_IMPACT.len() as u32 + 7) & !7;
    offset = (offset + BOOST_WHOOSH.len() as u32 + 7) & !7;
    offset = (offset + CHECKPOINT_CHIME.len() as u32 + 7) & !7;
    offset = (offset + UI_MOVE.len() as u32 + 7) & !7;
    offset
}

/// Compile-time guard against the bank growing into the intro sample.
/// Without this, adding a sample is silently safe right up until it truncates
/// the intro audio (TASK-1207).
const _: () = assert!(
    bank_end() <= INTRO_ADDR.byte_offset(),
    "soundbank overlaps the intro sample: lower INTRO_ADDR or shrink the bank"
);

/// Initializes SPU hardware and uploads all sound effects into SPU RAM.
pub fn init_spu_soundbank() -> SpuSoundbankAddrs {
    spu::init();

    // Set master volume to maximum
    spu::set_main_volume(Volume::MAX, Volume::MAX);
    spu::set_cd_volume(CdVolume::MAX, CdVolume::MAX);
    spu::enable_cd_audio(true);

    // Compute 8-byte aligned SPU addresses starting from 0x1010
    let mut current_offset: u32 = 0x1010;

    // 1. Engine Loop
    let engine_addr = SpuAddr::new(current_offset);
    spu::upload_adpcm(engine_addr, &ENGINE_LOOP);
    current_offset = (current_offset + ENGINE_LOOP.len() as u32 + 7) & !7;

    // 2. Tire Screech Loop
    let skid_addr = SpuAddr::new(current_offset);
    spu::upload_adpcm(skid_addr, &TIRE_SCREECH);
    current_offset = (current_offset + TIRE_SCREECH.len() as u32 + 7) & !7;

    // 3. Crash Impact One-shot
    let crash_addr = SpuAddr::new(current_offset);
    spu::upload_adpcm(crash_addr, &CRASH_IMPACT);
    current_offset = (current_offset + CRASH_IMPACT.len() as u32 + 7) & !7;

    // 4. Boost Whoosh One-shot
    let boost_addr = SpuAddr::new(current_offset);
    spu::upload_adpcm(boost_addr, &BOOST_WHOOSH);
    current_offset = (current_offset + BOOST_WHOOSH.len() as u32 + 7) & !7;

    // 5. Checkpoint Chime One-shot
    let chime_addr = SpuAddr::new(current_offset);
    spu::upload_adpcm(chime_addr, &CHECKPOINT_CHIME);
    current_offset = (current_offset + CHECKPOINT_CHIME.len() as u32 + 7) & !7;

    // 6. UI Navigation Blip One-shot
    let ui_addr = SpuAddr::new(current_offset);
    spu::upload_adpcm(ui_addr, &UI_MOVE);

    // `bank_end()` must agree with the layout built above, or the compile-time
    // overlap guard is checking the wrong number.
    debug_assert_eq!(bank_end(), (current_offset + UI_MOVE.len() as u32 + 7) & !7);

    // Configure loop voices with sustained sample envelope
    VOICE_ENGINE.configure_sample(
        engine_addr,
        ENGINE_LOOP_RATE,
        Volume::linear(1, 4),
        Adsr::sample(),
    );
    VOICE_ENGINE.set_loop_addr(engine_addr);

    VOICE_SKID.configure_sample(
        skid_addr,
        TIRE_SCREECH_RATE,
        Volume::SILENCE,
        Adsr::sample(),
    );
    VOICE_SKID.set_loop_addr(skid_addr);

    // Configure one-shot SFX voices with percussive / one-shot envelopes
    VOICE_CRASH.configure_sample(
        crash_addr,
        CRASH_IMPACT_RATE,
        Volume::MAX,
        Adsr::sample_one_shot(),
    );

    VOICE_BOOST.configure_sample(
        boost_addr,
        BOOST_WHOOSH_RATE,
        Volume::MAX,
        Adsr::sample_one_shot(),
    );

    VOICE_CHIME.configure_sample(
        chime_addr,
        CHECKPOINT_CHIME_RATE,
        Volume::MAX,
        Adsr::sample_one_shot(),
    );

    VOICE_UI.configure_sample(ui_addr, UI_MOVE_RATE, Volume::MAX, Adsr::sample_one_shot());

    // The curb voice plays the *skid* sample from its own address: a curb rumble
    // and a tire squeal are the same kind of noise, and a dedicated sample
    // would cost SPU RAM for no audible gain.
    VOICE_CURB.configure_sample(
        skid_addr,
        TIRE_SCREECH_RATE,
        Volume::SILENCE,
        Adsr::sample(),
    );
    VOICE_CURB.set_loop_addr(skid_addr);
    VOICE_CURB.set_pitch(psx_spu::Pitch::raw(CURB_PITCH));

    // Ensure intro audio voice is stopped
    Voice::key_off(VOICE_INTRO.mask());

    // The engine loop is a *continuous* voice: it has to be keyed on for the
    // hardware to decode it at all, and it stays keyed on for the whole run.
    //
    // It is keyed on here, silent, rather than on entering a race. Volume, not
    // key state, is what `EngineAudio` manages -- `silence` writes a zero
    // volume -- so re-keying per race would restart the ADPCM decoder
    // mid-sample and click on every restart. Keying on once at boot and
    // leaving it running means the policy alone decides whether the engine is
    // audible.
    //
    // This line was deleted when the policy gate was introduced, leaving the
    // comment above it describing a state the code never entered: nothing ever
    // keyed voice 0 on, so the engine was inaudible for the entire race. Writing
    // the volume alone does not start a voice (TASK-1215).
    VOICE_ENGINE.set_volume(Volume::SILENCE, Volume::SILENCE);
    Voice::key_on(VOICE_ENGINE.mask());

    SpuSoundbankAddrs {
        engine_addr,
        skid_addr,
        crash_addr,
        boost_addr,
        chime_addr,
        ui_addr,
    }
}
