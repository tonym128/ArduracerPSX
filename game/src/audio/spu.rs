//! PSX SPU Driver and Sample Bank Loader.
//!
//! Initializes SPU hardware registers, transfers pre-cooked ADPCM audio
//! blocks into SPU Sound RAM (starting at 0x1010), and manages hardware SPU
//! voice assignments and reverb (`NO1-037`, `NO1-038`).

use crate::audio::soundbank::*;
use psx_spu::{self as spu, Adsr, CdVolume, SpuAddr, Voice, Volume};

// ---------------------------------------------------------------------------
// SPU Hardware Voice Allocations (24 Voices Total, NO1-037)
// ---------------------------------------------------------------------------

/// Player low-to-mid RPM primary cylinder pulse engine loop.
pub const VOICE_ENGINE: Voice = Voice::V0;
/// Player high-RPM screaming harmonic engine loop.
pub const VOICE_ENGINE_HIGH: Voice = Voice::V1;
/// Turbo blow-off valve pneumatic hiss and exhaust backfire crackle.
pub const VOICE_BLOWOFF: Voice = Voice::V2;
/// Tire screech / lateral slip friction loop.
pub const VOICE_SKID: Voice = Voice::V3;
/// Curb sawtooth rumble loop.
pub const VOICE_CURB: Voice = Voice::V4;
/// Surface texture loop (gravel, dirt, grass, sand).
pub const VOICE_SURFACE: Voice = Voice::V5;

/// 5 AI Rival spatial engine loops (Voices 6..10).
pub const VOICE_RIVAL_0: Voice = Voice::V6;
pub const VOICE_RIVAL_1: Voice = Voice::V7;
pub const VOICE_RIVAL_2: Voice = Voice::new(8);
pub const VOICE_RIVAL_3: Voice = Voice::new(9);
pub const VOICE_RIVAL_4: Voice = Voice::new(10);

pub const VOICE_RIVALS: [Voice; 5] = [
    VOICE_RIVAL_0,
    VOICE_RIVAL_1,
    VOICE_RIVAL_2,
    VOICE_RIVAL_3,
    VOICE_RIVAL_4,
];

/// Barrier and vehicle crash impact one-shot.
pub const VOICE_CRASH: Voice = Voice::new(11);
/// Turbo and reactor super boost activation whoosh.
pub const VOICE_BOOST: Voice = Voice::new(12);
/// Checkpoint completion split chime.
pub const VOICE_CHIME: Voice = Voice::new(13);
/// Jump launch whoosh and suspension bottom-out landing thump.
pub const VOICE_JUMP: Voice = Voice::new(14);
/// 3-2-1 countdown beep.
pub const VOICE_COUNTDOWN: Voice = Voice::new(15);
/// GO! start horn.
pub const VOICE_GO_HORN: Voice = Voice::new(16);
/// Medal and new record fanfare arpeggio.
pub const VOICE_MEDAL: Voice = Voice::new(17);
/// Arcade announcer vocoder voice clips ("3-2-1-GO!", "CHECKPOINT!", "FINAL LAP!", "NEW RECORD!").
pub const VOICE_ANNOUNCER: Voice = Voice::new(18);
/// Menu UI navigation blip.
pub const VOICE_UI: Voice = Voice::new(19);
/// Attract mode intro audio.
pub const VOICE_INTRO: Voice = Voice::new(20);

/// Curb rumble level and pitch. Quieter and lower than the skid voice so it
/// reads as a rumble strip rather than a squeal.
pub const CURB_VOLUME: i16 = 0x0800;
pub const CURB_PITCH: u16 = 0x0800;

// SPU RAM layout:
// 0x0000..0x1010: SPU capture buffers and system workspace (4 KB)
// 0x1010..0x1FFFF: Soundbank sample data (~124 KB)
// 0x20000..0x37000: INTRO.ADPCM attract audio (91.5 KB)
// 0x78000..0x7FFFF: Hardware Reverb work area (32 KB, NO1-038)
pub const BANK_START: u32 = 0x1010;
pub const INTRO_ADDR: SpuAddr = SpuAddr::new(0x20000);
pub const INTRO_SAMPLE_RATE: u32 = 16000;
pub const INTRO_ADPCM_DATA: &[u8] = include_bytes!("../../../assets/INTRO.ADPCM");

/// Prepares and begins playback of intro attract audio via SPU Voice 20.
pub fn play_intro_audio() {
    if INTRO_ADPCM_DATA.len() <= 16 {
        crate::dbg::println("[SPU] INTRO.ADPCM data missing or empty. Skipping audio.");
        return;
    }
    crate::dbg::println("[SPU] Initializing SPU hardware and uploading INTRO.ADPCM...");
    spu::init();
    spu::set_main_volume(Volume::MAX, Volume::MAX);
    spu::upload_adpcm(INTRO_ADDR, INTRO_ADPCM_DATA);
    VOICE_INTRO.configure_sample(INTRO_ADDR, INTRO_SAMPLE_RATE, Volume::MAX, Adsr::sample());
    Voice::key_on(VOICE_INTRO.mask());
    crate::dbg::println("[SPU] Intro audio playback started (voice 20 keyed on).");
}

/// Immediately silences the intro attract audio.
pub fn stop_intro_audio() {
    Voice::key_off(VOICE_INTRO.mask());
    crate::dbg::println("[SPU] Intro audio stopped (voice 20 keyed off).");
}

/// SPU Sound RAM layout descriptor for all game sound effects (`NO1-037`).
#[derive(Copy, Clone, Debug)]
pub struct SpuSoundbankAddrs {
    pub engine_addr: SpuAddr,
    pub engine_high_addr: SpuAddr,
    pub blowoff_addr: SpuAddr,
    pub skid_addr: SpuAddr,
    pub gravel_addr: SpuAddr,
    pub grass_addr: SpuAddr,
    pub crash_addr: SpuAddr,
    pub boost_addr: SpuAddr,
    pub chime_addr: SpuAddr,
    pub jump_addr: SpuAddr,
    pub countdown_addr: SpuAddr,
    pub go_horn_addr: SpuAddr,
    pub medal_addr: SpuAddr,
    pub ui_addr: SpuAddr,
    pub voice_checkpoint_addr: SpuAddr,
    pub voice_go_addr: SpuAddr,
    pub voice_final_lap_addr: SpuAddr,
    pub voice_record_addr: SpuAddr,
}

/// Where the soundbank ends once every sample is rounded up to 8-byte alignment.
pub const fn bank_end() -> u32 {
    let mut offset = BANK_START;
    offset = (offset + ENGINE_LOOP.len() as u32 + 7) & !7;
    offset = (offset + ENGINE_HIGH.len() as u32 + 7) & !7;
    offset = (offset + BLOWOFF_POP.len() as u32 + 7) & !7;
    offset = (offset + TIRE_SCREECH.len() as u32 + 7) & !7;
    offset = (offset + SURFACE_GRAVEL.len() as u32 + 7) & !7;
    offset = (offset + SURFACE_GRASS.len() as u32 + 7) & !7;
    offset = (offset + CRASH_IMPACT.len() as u32 + 7) & !7;
    offset = (offset + BOOST_WHOOSH.len() as u32 + 7) & !7;
    offset = (offset + CHECKPOINT_CHIME.len() as u32 + 7) & !7;
    offset = (offset + JUMP_LANDING.len() as u32 + 7) & !7;
    offset = (offset + COUNTDOWN_BEEP.len() as u32 + 7) & !7;
    offset = (offset + GO_HORN.len() as u32 + 7) & !7;
    offset = (offset + MEDAL_FANFARE.len() as u32 + 7) & !7;
    offset = (offset + UI_MOVE.len() as u32 + 7) & !7;
    offset = (offset + VOICE_CHECKPOINT.len() as u32 + 7) & !7;
    offset = (offset + VOICE_GO.len() as u32 + 7) & !7;
    offset = (offset + VOICE_FINAL_LAP.len() as u32 + 7) & !7;
    offset = (offset + VOICE_NEW_RECORD.len() as u32 + 7) & !7;
    offset
}

/// Compile-time guard ensuring the expanded soundbank never overlaps INTRO_ADDR.
const _: () = assert!(
    bank_end() <= INTRO_ADDR.byte_offset(),
    "soundbank overlaps the intro sample: lower INTRO_ADDR or shrink the bank"
);

/// Configures SPU Hardware Reverb work area and wet depth (`NO1-038`).
pub fn configure_reverb(enable: bool, wet_vol: i16) {
    #[cfg(target_arch = "mips")]
    unsafe {
        // Work area at top of RAM: 0x78000 / 8 = 0xF000
        core::ptr::write_volatile(0x1F80_1DA2 as *mut u16, 0xF000);
        let spucnt = core::ptr::read_volatile(0x1F80_1DAA as *const u16);
        if enable {
            // SPUCNT: bit 7 enables reverb processing
            core::ptr::write_volatile(0x1F80_1DAA as *mut u16, spucnt | 0x0080);
            let reverb_mask = VOICE_ENGINE.mask()
                | VOICE_ENGINE_HIGH.mask()
                | VOICE_BLOWOFF.mask()
                | VOICE_SKID.mask()
                | VOICE_CRASH.mask()
                | VOICE_BOOST.mask()
                | VOICE_CHIME.mask()
                | VOICE_JUMP.mask()
                | VOICE_ANNOUNCER.mask();
            core::ptr::write_volatile(0x1F80_1D98 as *mut u16, (reverb_mask & 0xFFFF) as u16);
            core::ptr::write_volatile(
                0x1F80_1D9A as *mut u16,
                ((reverb_mask >> 16) & 0x00FF) as u16,
            );
            core::ptr::write_volatile(0x1F80_1D84 as *mut u16, wet_vol as u16);
            core::ptr::write_volatile(0x1F80_1D86 as *mut u16, wet_vol as u16);
        } else {
            core::ptr::write_volatile(0x1F80_1DAA as *mut u16, spucnt & !0x0080);
            core::ptr::write_volatile(0x1F80_1D84 as *mut u16, 0);
            core::ptr::write_volatile(0x1F80_1D86 as *mut u16, 0);
        }
    }
    #[cfg(not(target_arch = "mips"))]
    {
        let _ = (enable, wet_vol);
    }
}

/// Initializes SPU hardware and uploads all sound effects into SPU Sound RAM (`NO1-037`).
pub fn init_spu_soundbank() -> SpuSoundbankAddrs {
    spu::init();

    // Set master volume to maximum by default
    spu::set_main_volume(Volume::MAX, Volume::MAX);
    spu::set_cd_volume(CdVolume::MAX, CdVolume::MAX);
    spu::enable_cd_audio(true);

    let mut current_offset: u32 = BANK_START;

    // 1. Primary Engine Loop
    let engine_addr = SpuAddr::new(current_offset);
    spu::upload_adpcm(engine_addr, &ENGINE_LOOP);
    current_offset = (current_offset + ENGINE_LOOP.len() as u32 + 7) & !7;

    // 2. High-RPM Engine Screamer Loop
    let engine_high_addr = SpuAddr::new(current_offset);
    spu::upload_adpcm(engine_high_addr, &ENGINE_HIGH);
    current_offset = (current_offset + ENGINE_HIGH.len() as u32 + 7) & !7;

    // 3. Blow-Off Valve & Backfire Pop
    let blowoff_addr = SpuAddr::new(current_offset);
    spu::upload_adpcm(blowoff_addr, &BLOWOFF_POP);
    current_offset = (current_offset + BLOWOFF_POP.len() as u32 + 7) & !7;

    // 4. Tire Screech Loop
    let skid_addr = SpuAddr::new(current_offset);
    spu::upload_adpcm(skid_addr, &TIRE_SCREECH);
    current_offset = (current_offset + TIRE_SCREECH.len() as u32 + 7) & !7;

    // 5. Gravel / Dirt Surface Loop
    let gravel_addr = SpuAddr::new(current_offset);
    spu::upload_adpcm(gravel_addr, &SURFACE_GRAVEL);
    current_offset = (current_offset + SURFACE_GRAVEL.len() as u32 + 7) & !7;

    // 6. Grass / Sand Surface Loop
    let grass_addr = SpuAddr::new(current_offset);
    spu::upload_adpcm(grass_addr, &SURFACE_GRASS);
    current_offset = (current_offset + SURFACE_GRASS.len() as u32 + 7) & !7;

    // 7. Crash Impact One-shot
    let crash_addr = SpuAddr::new(current_offset);
    spu::upload_adpcm(crash_addr, &CRASH_IMPACT);
    current_offset = (current_offset + CRASH_IMPACT.len() as u32 + 7) & !7;

    // 8. Boost Whoosh One-shot
    let boost_addr = SpuAddr::new(current_offset);
    spu::upload_adpcm(boost_addr, &BOOST_WHOOSH);
    current_offset = (current_offset + BOOST_WHOOSH.len() as u32 + 7) & !7;

    // 9. Checkpoint Chime One-shot
    let chime_addr = SpuAddr::new(current_offset);
    spu::upload_adpcm(chime_addr, &CHECKPOINT_CHIME);
    current_offset = (current_offset + CHECKPOINT_CHIME.len() as u32 + 7) & !7;

    // 10. Jump Landing Thump
    let jump_addr = SpuAddr::new(current_offset);
    spu::upload_adpcm(jump_addr, &JUMP_LANDING);
    current_offset = (current_offset + JUMP_LANDING.len() as u32 + 7) & !7;

    // 11. Countdown Beep One-shot
    let countdown_addr = SpuAddr::new(current_offset);
    spu::upload_adpcm(countdown_addr, &COUNTDOWN_BEEP);
    current_offset = (current_offset + COUNTDOWN_BEEP.len() as u32 + 7) & !7;

    // 12. GO Horn One-shot
    let go_horn_addr = SpuAddr::new(current_offset);
    spu::upload_adpcm(go_horn_addr, &GO_HORN);
    current_offset = (current_offset + GO_HORN.len() as u32 + 7) & !7;

    // 13. Medal Fanfare One-shot
    let medal_addr = SpuAddr::new(current_offset);
    spu::upload_adpcm(medal_addr, &MEDAL_FANFARE);
    current_offset = (current_offset + MEDAL_FANFARE.len() as u32 + 7) & !7;

    // 14. UI Navigation Blip
    let ui_addr = SpuAddr::new(current_offset);
    spu::upload_adpcm(ui_addr, &UI_MOVE);
    current_offset = (current_offset + UI_MOVE.len() as u32 + 7) & !7;

    // 15. Announcer: "CHECKPOINT!"
    let voice_checkpoint_addr = SpuAddr::new(current_offset);
    spu::upload_adpcm(voice_checkpoint_addr, &VOICE_CHECKPOINT);
    current_offset = (current_offset + VOICE_CHECKPOINT.len() as u32 + 7) & !7;

    // 16. Announcer: "GO!"
    let voice_go_addr = SpuAddr::new(current_offset);
    spu::upload_adpcm(voice_go_addr, &VOICE_GO);
    current_offset = (current_offset + VOICE_GO.len() as u32 + 7) & !7;

    // 17. Announcer: "FINAL LAP!"
    let voice_final_lap_addr = SpuAddr::new(current_offset);
    spu::upload_adpcm(voice_final_lap_addr, &VOICE_FINAL_LAP);
    current_offset = (current_offset + VOICE_FINAL_LAP.len() as u32 + 7) & !7;

    // 18. Announcer: "NEW RECORD!"
    let voice_record_addr = SpuAddr::new(current_offset);
    spu::upload_adpcm(voice_record_addr, &VOICE_NEW_RECORD);
    current_offset = (current_offset + VOICE_NEW_RECORD.len() as u32 + 7) & !7;

    debug_assert_eq!(bank_end(), current_offset);

    // Configure continuous player engine voices (Voices 0 & 1)
    VOICE_ENGINE.configure_sample(
        engine_addr,
        ENGINE_LOOP_RATE,
        Volume::linear(1, 4),
        Adsr::sample(),
    );
    VOICE_ENGINE.set_loop_addr(engine_addr);

    VOICE_ENGINE_HIGH.configure_sample(
        engine_high_addr,
        ENGINE_HIGH_RATE,
        Volume::SILENCE,
        Adsr::sample(),
    );
    VOICE_ENGINE_HIGH.set_loop_addr(engine_high_addr);

    // Configure blow-off / backfire pop voice
    VOICE_BLOWOFF.configure_sample(
        blowoff_addr,
        BLOWOFF_POP_RATE,
        Volume::MAX,
        Adsr::sample_one_shot(),
    );

    // Configure tire screech loop voice
    VOICE_SKID.configure_sample(
        skid_addr,
        TIRE_SCREECH_RATE,
        Volume::SILENCE,
        Adsr::sample(),
    );
    VOICE_SKID.set_loop_addr(skid_addr);

    // Configure curb rumble loop voice (reuses skid sample with lower pitch)
    VOICE_CURB.configure_sample(
        skid_addr,
        TIRE_SCREECH_RATE,
        Volume::SILENCE,
        Adsr::sample(),
    );
    VOICE_CURB.set_loop_addr(skid_addr);
    VOICE_CURB.set_pitch(psx_spu::Pitch::raw(CURB_PITCH));

    // Configure surface texture loop voice (gravel/dirt/grass)
    VOICE_SURFACE.configure_sample(
        gravel_addr,
        SURFACE_GRAVEL_RATE,
        Volume::SILENCE,
        Adsr::sample(),
    );
    VOICE_SURFACE.set_loop_addr(gravel_addr);

    // Configure 5 AI Rival spatial engine voices (Voices 6..10)
    for voice in VOICE_RIVALS {
        voice.configure_sample(
            engine_addr,
            ENGINE_LOOP_RATE,
            Volume::SILENCE,
            Adsr::sample(),
        );
        voice.set_loop_addr(engine_addr);
        voice.set_volume(Volume::SILENCE, Volume::SILENCE);
    }

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

    VOICE_JUMP.configure_sample(
        jump_addr,
        JUMP_LANDING_RATE,
        Volume::MAX,
        Adsr::sample_one_shot(),
    );

    VOICE_COUNTDOWN.configure_sample(
        countdown_addr,
        COUNTDOWN_BEEP_RATE,
        Volume::MAX,
        Adsr::sample_one_shot(),
    );

    VOICE_GO_HORN.configure_sample(
        go_horn_addr,
        GO_HORN_RATE,
        Volume::MAX,
        Adsr::sample_one_shot(),
    );

    VOICE_MEDAL.configure_sample(
        medal_addr,
        MEDAL_FANFARE_RATE,
        Volume::MAX,
        Adsr::sample_one_shot(),
    );

    VOICE_ANNOUNCER.configure_sample(
        voice_checkpoint_addr,
        VOICE_CHECKPOINT_RATE,
        Volume::MAX,
        Adsr::sample_one_shot(),
    );

    VOICE_UI.configure_sample(ui_addr, UI_MOVE_RATE, Volume::MAX, Adsr::sample_one_shot());

    // Ensure intro audio voice is stopped
    Voice::key_off(VOICE_INTRO.mask());

    // The engine loop is a continuous voice: it is keyed on at init, silent.
    VOICE_ENGINE.set_volume(Volume::SILENCE, Volume::SILENCE);
    Voice::key_on(VOICE_ENGINE.mask());

    VOICE_ENGINE_HIGH.set_volume(Volume::SILENCE, Volume::SILENCE);
    Voice::key_on(VOICE_ENGINE_HIGH.mask());

    SpuSoundbankAddrs {
        engine_addr,
        engine_high_addr,
        blowoff_addr,
        skid_addr,
        gravel_addr,
        grass_addr,
        crash_addr,
        boost_addr,
        chime_addr,
        jump_addr,
        countdown_addr,
        go_horn_addr,
        medal_addr,
        ui_addr,
        voice_checkpoint_addr,
        voice_go_addr,
        voice_final_lap_addr,
        voice_record_addr,
    }
}
