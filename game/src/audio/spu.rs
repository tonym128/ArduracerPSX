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

/// SPU Sound RAM layout descriptor for all game sound effects.
pub struct SpuSoundbankAddrs {
    pub engine_addr: SpuAddr,
    pub skid_addr: SpuAddr,
    pub crash_addr: SpuAddr,
    pub boost_addr: SpuAddr,
    pub chime_addr: SpuAddr,
}

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

    // Start engine continuous loop immediately
    Voice::key_on(VOICE_ENGINE.mask());

    SpuSoundbankAddrs {
        engine_addr,
        skid_addr,
        crash_addr,
        boost_addr,
        chime_addr,
    }
}
