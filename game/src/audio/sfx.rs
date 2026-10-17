//! Tire Squeal, Multi-Surface and Environmental Sound Effects Player.
//!
//! Manages procedural tire screech volume/pitch scaling, surface loops
//! (dirt/gravel/grass), barrier collision impacts, turbo boost whooshes,
//! checkpoint completion chimes, jump landings, countdown beeps, start horn,
//! and arcade announcer vocoder clips (`NO1-037`, `NO1-039`).

use crate::audio::audio_policy::CrashGate;
use crate::audio::soundbank::{
    SURFACE_GRASS_RATE, SURFACE_GRAVEL_RATE, VOICE_CHECKPOINT_RATE, VOICE_FINAL_LAP_RATE,
    VOICE_GO_RATE, VOICE_NEW_RECORD_RATE,
};
use crate::audio::spu::{
    SpuSoundbankAddrs, CURB_PITCH, CURB_VOLUME, VOICE_ANNOUNCER, VOICE_BOOST, VOICE_CHIME,
    VOICE_COUNTDOWN, VOICE_CRASH, VOICE_CURB, VOICE_GO_HORN, VOICE_JUMP, VOICE_MEDAL, VOICE_SKID,
    VOICE_SURFACE, VOICE_UI,
};
use arduracer_core::timing::{StartPhase, StartSequence};
use arduracer_core::{Fixed, SurfaceType, VehicleState};
use psx_spu::{Adsr, Pitch, Voice, Volume};

/// Panning helper: computes left/right channel volumes from a center baseline (160 px).
pub fn pan_stereo(base_vol: i16, screen_x: i16) -> (Volume, Volume) {
    let pan_right = screen_x.clamp(0, 320) as i32;
    let vl = if pan_right <= 160 {
        base_vol
    } else {
        ((base_vol as i32 * (320 - pan_right)) / 160) as i16
    };
    let vr = if pan_right >= 160 {
        base_vol
    } else {
        ((base_vol as i32 * pan_right) / 160) as i16
    };
    (Volume(vl), Volume(vr))
}

/// Arcade announcer voice clips (`NO1-037`).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum AnnouncerClip {
    Checkpoint,
    Go,
    FinalLap,
    NewRecord,
}

pub struct SfxPlayer {
    skid_playing: bool,
    skid_vol: i16,
    curb_playing: bool,
    surface_playing: bool,
    current_surface: Option<SurfaceType>,
    surface_vol: i16,
    prev_boost_ticks: u16,
    prev_checkpoints: u8,
    prev_airborne_ticks: u16,
    prev_lit: u8,
    prev_phase_go: bool,
    prev_final_lap: bool,
    /// Edge detection and rate limiting for barrier impacts. The decision
    /// itself lives in `audio_policy::CrashGate` so it can be host-tested.
    crash: CrashGate,
    addrs: Option<SpuSoundbankAddrs>,
}

impl Default for SfxPlayer {
    fn default() -> Self {
        Self::new()
    }
}

impl SfxPlayer {
    pub const fn new() -> Self {
        SfxPlayer {
            skid_playing: false,
            skid_vol: 0,
            curb_playing: false,
            surface_playing: false,
            current_surface: None,
            surface_vol: 0,
            prev_boost_ticks: 0,
            prev_checkpoints: 0,
            prev_airborne_ticks: 0,
            prev_lit: 0,
            prev_phase_go: false,
            prev_final_lap: false,
            crash: CrashGate::new(),
            addrs: None,
        }
    }

    /// Stores the uploaded soundbank SPU addresses for dynamic voice reconfiguration.
    pub fn set_addrs(&mut self, addrs: SpuSoundbankAddrs) {
        self.addrs = Some(addrs);
    }

    /// Triggers a barrier crash impact sound effect.
    pub fn play_crash(&self) {
        VOICE_CRASH.set_volume(Volume::MAX, Volume::MAX);
        Voice::key_on(VOICE_CRASH.mask());
    }

    /// Triggers a barrier crash impact with stereo panning (`NO1-039`).
    pub fn play_crash_panned(&self, screen_x: i16) {
        let (vl, vr) = pan_stereo(Volume::MAX.0, screen_x);
        VOICE_CRASH.set_volume(vl, vr);
        Voice::key_on(VOICE_CRASH.mask());
    }

    /// Triggers a turbo boost activation whoosh sound effect.
    pub fn play_boost(&self) {
        VOICE_BOOST.set_volume(Volume::MAX, Volume::MAX);
        Voice::key_on(VOICE_BOOST.mask());
    }

    /// Triggers a turbo boost activation whoosh with stereo panning (`NO1-039`).
    pub fn play_boost_panned(&self, screen_x: i16) {
        let (vl, vr) = pan_stereo(Volume::MAX.0, screen_x);
        VOICE_BOOST.set_volume(vl, vr);
        Voice::key_on(VOICE_BOOST.mask());
    }

    /// Triggers a checkpoint crossing chime.
    pub fn play_checkpoint(&self) {
        VOICE_CHIME.set_volume(Volume::MAX, Volume::MAX);
        Voice::key_on(VOICE_CHIME.mask());
    }

    /// Triggers a checkpoint crossing chime with stereo panning (`NO1-039`).
    pub fn play_checkpoint_panned(&self, screen_x: i16) {
        let (vl, vr) = pan_stereo(Volume::MAX.0, screen_x);
        VOICE_CHIME.set_volume(vl, vr);
        Voice::key_on(VOICE_CHIME.mask());
    }

    /// Triggers a jump landing suspension thump with stereo panning (`NO1-037`, `NO1-039`).
    pub fn play_jump_landing(&self, screen_x: i16) {
        let (vl, vr) = pan_stereo(Volume::MAX.0, screen_x);
        VOICE_JUMP.set_volume(vl, vr);
        Voice::key_on(VOICE_JUMP.mask());
    }

    /// Triggers an arcade announcer vocoder clip (`NO1-037`).
    pub fn play_announcer(&self, clip: AnnouncerClip) {
        if let Some(addrs) = self.addrs {
            let (addr, rate) = match clip {
                AnnouncerClip::Checkpoint => (addrs.voice_checkpoint_addr, VOICE_CHECKPOINT_RATE),
                AnnouncerClip::Go => (addrs.voice_go_addr, VOICE_GO_RATE),
                AnnouncerClip::FinalLap => (addrs.voice_final_lap_addr, VOICE_FINAL_LAP_RATE),
                AnnouncerClip::NewRecord => (addrs.voice_record_addr, VOICE_NEW_RECORD_RATE),
            };
            VOICE_ANNOUNCER.configure_sample(addr, rate, Volume::MAX, Adsr::sample_one_shot());
            Voice::key_on(VOICE_ANNOUNCER.mask());
        }
    }

    /// Triggers the medal & new record fanfare arpeggio (`NO1-037`).
    pub fn play_medal(&self) {
        VOICE_MEDAL.set_volume(Volume::MAX, Volume::MAX);
        Voice::key_on(VOICE_MEDAL.mask());
    }

    /// Triggers a menu navigation blip.
    pub fn play_ui_move(&self) {
        VOICE_UI.set_pitch(Pitch::raw(0x1000));
        Voice::key_on(VOICE_UI.mask());
    }

    /// Triggers a menu UI sound effect with pitch tailored to Move, Confirm, or Back.
    pub fn play_ui(&self, cue: crate::audio::audio_policy::UiSound) {
        use crate::audio::audio_policy::UiSound;
        let pitch = match cue {
            UiSound::Move => 0x1000,
            UiSound::Confirm => 0x1500,
            UiSound::Back => 0x0B80,
        };
        VOICE_UI.set_pitch(Pitch::raw(pitch));
        Voice::key_on(VOICE_UI.mask());
    }

    /// Cuts every race SFX voice at once.
    ///
    /// The skid and curb voices are loops and need an explicit key-off; the crash, boost
    /// and chime voices are one-shots that self-terminate into the silence
    /// block, so they only need their envelope cut rather than being stopped
    /// mid-sample. Their *restart* triggers are suppressed instead, by having
    /// the caller stop ticking.
    ///
    /// Idempotent, so the state machine may call it on every non-racing frame.
    pub fn silence(&mut self) {
        self.prev_boost_ticks = 0;
        self.prev_checkpoints = 0;
        self.prev_airborne_ticks = 0;
        self.prev_lit = 0;
        self.prev_phase_go = false;
        self.prev_final_lap = false;
        self.crash.reset();

        if self.curb_playing {
            self.curb_playing = false;
            VOICE_CURB.set_volume(Volume::SILENCE, Volume::SILENCE);
            Voice::key_off(VOICE_CURB.mask());
        }

        if self.skid_playing {
            self.skid_playing = false;
            self.skid_vol = 0;
            VOICE_SKID.set_volume(Volume::SILENCE, Volume::SILENCE);
            Voice::key_off(VOICE_SKID.mask());
        } else if self.skid_vol != 0 {
            // Already stopped by the fade path, but the register may still hold
            // a non-zero value from a frame that raced the fade.
            self.skid_vol = 0;
            VOICE_SKID.set_volume(Volume::SILENCE, Volume::SILENCE);
        }

        if self.surface_playing {
            self.surface_playing = false;
            self.current_surface = None;
            self.surface_vol = 0;
            VOICE_SURFACE.set_volume(Volume::SILENCE, Volume::SILENCE);
            Voice::key_off(VOICE_SURFACE.mask());
        } else if self.surface_vol != 0 {
            self.surface_vol = 0;
            VOICE_SURFACE.set_volume(Volume::SILENCE, Volume::SILENCE);
        }
    }

    /// Whether any race SFX voice is currently live.
    pub const fn is_silent(&self) -> bool {
        !self.skid_playing
            && self.skid_vol == 0
            && !self.curb_playing
            && !self.surface_playing
            && self.surface_vol == 0
    }

    /// Updates SFX state each 60Hz tick (`NO1-037`, `NO1-039`).
    #[allow(clippy::too_many_arguments)]
    pub fn update(
        &mut self,
        player: &VehicleState,
        surface: SurfaceType,
        hit_wall: bool,
        checkpoints_cleared: u8,
        start_seq: Option<&StartSequence>,
        is_final_lap: bool,
        screen_x: i16,
    ) {
        // 1. Tire Screech during drift or dynamic cornering scrub
        let is_scrubbing = (player.is_drifting || player.tire_scrub > Fixed::from_raw(2800))
            && player.speed > Fixed::from_int(2)
            && player.airborne_ticks == 0;
        if is_scrubbing {
            if !self.skid_playing {
                Voice::key_on(VOICE_SKID.mask());
                self.skid_playing = true;
            }

            // Volume scales with speed and scrub intensity (up to 0x2400)
            let speed_ratio = (player.speed.raw().min(40960) * 0x2400) / 40960;
            let scrub_mult = player.tire_scrub.raw().clamp(2048, 4096);
            let target_vol = ((speed_ratio as i64 * scrub_mult as i64) / 4096) as i16;
            self.skid_vol = self
                .skid_vol
                .saturating_add((target_vol - self.skid_vol) >> 1);

            // Pitch slightly shifts up with higher speed
            let target_pitch = 0x0E00 + ((player.speed.raw() as u32 * 0x0600) / 40960) as u16;
            VOICE_SKID.set_pitch(Pitch::raw(target_pitch));
            let (vl, vr) = pan_stereo(self.skid_vol, screen_x);
            VOICE_SKID.set_volume(vl, vr);
        } else if self.skid_playing {
            // Rapid fadeout
            self.skid_vol = self.skid_vol.saturating_sub(self.skid_vol >> 1);
            let (vl, vr) = pan_stereo(self.skid_vol, screen_x);
            VOICE_SKID.set_volume(vl, vr);
            if self.skid_vol < 100 {
                Voice::key_off(VOICE_SKID.mask());
                self.skid_playing = false;
                self.skid_vol = 0;
            }
        }

        // 2. Curb rumble: reuses the skid sample with lower pitch on VOICE_CURB
        let on_curb = surface.triggers_curb_rumble()
            && player.speed > Fixed::from_raw(600)
            && player.airborne_ticks == 0;
        if on_curb {
            if !self.curb_playing {
                VOICE_CURB.set_pitch(Pitch::raw(CURB_PITCH));
                let (vl, vr) = pan_stereo(CURB_VOLUME, screen_x);
                VOICE_CURB.set_volume(vl, vr);
                Voice::key_on(VOICE_CURB.mask());
                self.curb_playing = true;
            } else {
                let (vl, vr) = pan_stereo(CURB_VOLUME, screen_x);
                VOICE_CURB.set_volume(vl, vr);
            }
        } else if self.curb_playing {
            self.curb_playing = false;
            VOICE_CURB.set_volume(Volume::SILENCE, Volume::SILENCE);
            Voice::key_off(VOICE_CURB.mask());
        }

        // 3. Multi-Surface Texture Loop (Dirt, Gravel, Sand, Grass) (NO1-037)
        let is_dirt_or_sand = matches!(surface, SurfaceType::Dirt | SurfaceType::Sand);
        let is_grass = matches!(surface, SurfaceType::OffRoad);
        let on_textured_surface = (is_dirt_or_sand || is_grass)
            && player.speed > Fixed::from_raw(400)
            && player.airborne_ticks == 0;

        if on_textured_surface {
            let surface_changed = self.current_surface != Some(surface);
            if !self.surface_playing || surface_changed {
                if let Some(addrs) = self.addrs {
                    let (addr, rate) = if is_dirt_or_sand {
                        (addrs.gravel_addr, SURFACE_GRAVEL_RATE)
                    } else {
                        (addrs.grass_addr, SURFACE_GRASS_RATE)
                    };
                    VOICE_SURFACE.configure_sample(addr, rate, Volume::SILENCE, Adsr::sample());
                    VOICE_SURFACE.set_loop_addr(addr);
                    Voice::key_on(VOICE_SURFACE.mask());
                    self.surface_playing = true;
                    self.current_surface = Some(surface);
                }
            }

            let target_vol = ((player.speed.raw().min(40960) * 0x2200) / 40960) as i16;
            self.surface_vol = self
                .surface_vol
                .saturating_add((target_vol - self.surface_vol) >> 1);
            let (vl, vr) = pan_stereo(self.surface_vol, screen_x);
            VOICE_SURFACE.set_volume(vl, vr);
        } else if self.surface_playing {
            self.surface_vol = self.surface_vol.saturating_sub(self.surface_vol >> 1);
            let (vl, vr) = pan_stereo(self.surface_vol, screen_x);
            VOICE_SURFACE.set_volume(vl, vr);
            if self.surface_vol < 100 {
                Voice::key_off(VOICE_SURFACE.mask());
                self.surface_playing = false;
                self.current_surface = None;
                self.surface_vol = 0;
            }
        }

        // 4. Barrier collision (authoritative, from collision solver)
        if self.crash.should_fire(hit_wall) {
            self.play_crash_panned(screen_x);
        }

        // 5. Boost activation trigger
        if player.boost_ticks > 0 && self.prev_boost_ticks == 0 {
            self.play_boost_panned(screen_x);
        }

        // 6. Checkpoint crossing chime + voice announcer ("CHECKPOINT!")
        if checkpoints_cleared > self.prev_checkpoints {
            self.play_checkpoint_panned(screen_x);
            self.play_announcer(AnnouncerClip::Checkpoint);
        }

        // 7. Jump landing thump (transitioned from airborne to grounded)
        if player.airborne_ticks == 0 && self.prev_airborne_ticks >= 4 {
            self.play_jump_landing(screen_x);
        }

        // 8. Start countdown beeps and GO horn
        if let Some(seq) = start_seq {
            match seq.phase() {
                StartPhase::Lit(lit) if lit > self.prev_lit => {
                    let pitch = if lit >= 3 { 0x1200 } else { 0x1000 };
                    VOICE_COUNTDOWN.set_pitch(Pitch::raw(pitch));
                    VOICE_COUNTDOWN.set_volume(Volume::MAX, Volume::MAX);
                    Voice::key_on(VOICE_COUNTDOWN.mask());
                    self.prev_lit = lit;
                }
                StartPhase::Go if !self.prev_phase_go => {
                    self.prev_phase_go = true;
                    VOICE_GO_HORN.set_volume(Volume::MAX, Volume::MAX);
                    Voice::key_on(VOICE_GO_HORN.mask());
                    self.play_announcer(AnnouncerClip::Go);
                }
                _ => {}
            }
        }

        // 9. Final lap announcer voice clip ("FINAL LAP!")
        if is_final_lap && !self.prev_final_lap {
            self.play_announcer(AnnouncerClip::FinalLap);
        }

        self.prev_boost_ticks = player.boost_ticks;
        self.prev_checkpoints = checkpoints_cleared;
        self.prev_airborne_ticks = player.airborne_ticks;
        self.prev_final_lap = is_final_lap;
    }
}
