//! PlayStation 1 Audio Engine for Arduracer PSX.
//!
//! Controls hardware SPU voices (multi-layer engine synthesizer, tire screech,
//! multi-surface texturing, crash impacts, turbo whoosh, checkpoint chimes,
//! jump landings, countdown beeps, start horn, vocoder announcer clips) and
//! Redbook CD-DA / XA streaming music with dynamic hardware reverb (`NO1-037`..`NO1-041`).

pub mod audio_policy;
pub mod cdda;
pub mod engine_audio;
pub mod sfx;
pub mod soundbank;
pub mod spu;
pub mod xa;

pub use audio_policy::{AudioPolicy, UiSound};
pub use cdda::{CddaController, CddaState};
pub use engine_audio::EngineAudio;
pub use sfx::{pan_stereo, AnnouncerClip, SfxPlayer};
pub use spu::{init_spu_soundbank, SpuSoundbankAddrs};

use arduracer_core::timing::StartSequence;
use arduracer_core::{AiRacer, Fixed, SurfaceType, VehicleState};

/// Returns the on-screen track title for Redbook CD-DA / XA music tracks (`NO1-041`).
pub fn music_track_title(track: u8) -> &'static str {
    match track {
        2 => "RETRO ARCADE DRIFT",
        3 => "ASPHALT OVERDRIVE",
        4 => "NEON VELOCITY",
        5 => "CYBERNETIC HIGHWAY",
        6 => "APEX PREDATOR",
        7 => "VICTORY LAP",
        _ => "MANIA CIRCUIT BEAT",
    }
}

/// Central audio subsystem managing hardware voices and disc audio streaming.
pub struct AudioSystem {
    pub engine: EngineAudio,
    pub sfx: SfxPlayer,
    pub cdda: CddaController,
    pub xa: crate::audio::xa::XaPlayer,
    /// The player's chosen disc-audio mode, as a raw saved byte.
    pub cd_audio: u8,
    /// Master sound effects volume (0..=8, default 8).
    pub sound_volume: u8,
    /// Master music volume (0..=8, default 8).
    pub music_volume: u8,
    /// Whether music is smoothly ducked (-6 dB) during pause/results.
    pub is_ducked: bool,
    /// Display timer (60 Hz frames) for the on-screen track title toast banner.
    pub toast_ticks: u16,
    /// The track `play_music` was last asked to play, so a mode change can
    /// restart the same track on the new source.
    pub last_music_track: u8,
    /// Whether the car is currently inside a tunnel/canyon (for SPU reverb).
    pub is_tunnel: bool,
    pub policy: AudioPolicy,
}

impl Default for AudioSystem {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioSystem {
    /// Which disc-audio source is playing, for the telemetry line.
    pub fn music_source(&self) -> &'static str {
        match arduracer_core::save::CdAudioMode::from_saved(self.cd_audio) {
            arduracer_core::save::CdAudioMode::Xa => "xa",
            _ => "cdda",
        }
    }

    /// Stores the chosen mode, so the music path and the telemetry agree.
    pub fn set_cd_audio(&mut self, mode: u8) {
        self.cd_audio = mode;
    }

    /// Sets the master sound effects volume (0..=8) and updates SPU main registers (`NO1-040`).
    pub fn set_sound_volume(&mut self, vol: u8) {
        self.sound_volume = vol.min(8);
        let v = ((0x3FFF_u32 * self.sound_volume as u32) / 8) as i16;
        psx_spu::set_main_volume(psx_spu::Volume(v), psx_spu::Volume(v));
    }

    /// Sets the master music volume (0..=8) and updates CD/XA mixer gain (`NO1-040`).
    pub fn set_music_volume(&mut self, vol: u8) {
        self.music_volume = vol.min(8);
        self.apply_music_volume();
    }

    /// Smoothly ducks or restores music volume by -6 dB during pause/results (`NO1-040`).
    pub fn duck_music(&mut self, duck: bool) {
        if self.is_ducked != duck {
            self.is_ducked = duck;
            self.apply_music_volume();
        }
    }

    /// Applies the effective CD/XA mixer volume to hardware (`NO1-040`).
    pub fn apply_music_volume(&mut self) {
        let eff = if self.is_ducked {
            (self.music_volume >> 1).max(1)
        } else {
            self.music_volume
        };
        let v = ((0x7FFF_u32 * eff as u32) / 8) as i16;
        self.cdda.set_volume(psx_spu::CdVolume(v));
    }

    /// Starts the music for `track`, on whichever source the player chose.
    pub fn play_music(&mut self, track: u8, reader: &mut crate::cd_fs::DiscReader) {
        self.last_music_track = track;
        self.toast_ticks = 150; // 2.5 seconds at 60 Hz
        if arduracer_core::save::CdAudioMode::from_saved(self.cd_audio)
            == arduracer_core::save::CdAudioMode::Xa
        {
            let name = crate::audio::xa::track_file_name(track);
            match reader.find_file_lba(&name) {
                Some(lba) => {
                    crate::dbg::print("[XA] track ");
                    crate::dbg::print_dec(track as u32);
                    crate::dbg::print(" at LBA ");
                    crate::dbg::print_dec(lba);
                    crate::dbg::println("");
                    crate::audio::xa::player().start_track(reader, lba);
                }
                None => {
                    crate::dbg::print("[XA] track ");
                    crate::dbg::print_dec(track as u32);
                    crate::dbg::println(" not on disc");
                }
            }
            self.apply_music_volume();
            return;
        }
        self.cdda.play_track(track);
        self.apply_music_volume();
    }

    /// Cycles to the next race music track (3..=6) and restarts playback (`NO1-041`).
    pub fn next_music_track(&mut self, reader: &mut crate::cd_fs::DiscReader) {
        let next = if self.last_music_track < 3 || self.last_music_track >= 6 {
            3
        } else {
            self.last_music_track + 1
        };
        self.play_music(next, reader);
    }

    /// Returns the active track title.
    pub fn current_music_title(&self) -> &'static str {
        music_track_title(self.last_music_track)
    }

    /// Decrements the track title toast display timer.
    pub fn tick_toast(&mut self) {
        if self.toast_ticks > 0 {
            self.toast_ticks -= 1;
        }
    }

    /// Switches the music source without changing track, if music is on.
    pub fn apply_music_mode(&mut self, reader: &mut crate::cd_fs::DiscReader) {
        use arduracer_core::save::CdAudioMode as Mode;
        let mode = Mode::from_saved(self.cd_audio);
        self.cdda.stop();
        crate::audio::xa::player().stop(reader);
        self.xa.stop(reader);
        if !mode.enabled() || self.last_music_track == 0 {
            return;
        }
        self.play_music(self.last_music_track, reader);
    }

    /// Pauses the active music stream (both CD-DA and XA) when entering the pause veil.
    pub fn pause_music(&mut self) {
        self.cdda.pause();
        crate::audio::xa::player().suspend();
    }

    /// Resumes the active music stream (CD-DA or XA) when leaving the pause veil.
    pub fn resume_music(&mut self, reader: &mut crate::cd_fs::DiscReader) {
        match arduracer_core::save::CdAudioMode::from_saved(self.cd_audio) {
            arduracer_core::save::CdAudioMode::Xa => {
                crate::audio::xa::player().resume(reader);
            }
            arduracer_core::save::CdAudioMode::Cdda => {
                self.cdda.resume();
            }
            arduracer_core::save::CdAudioMode::Off => {}
        }
        self.apply_music_volume();
    }

    /// Keeps the XA stream alive across a frame, resuming after a preemption.
    pub fn service_music(&mut self, reader: &mut crate::cd_fs::DiscReader) {
        crate::audio::xa::player().service(reader);
    }

    /// Configures hardware SPU reverb for tunnels, canyons, and stadiums (`NO1-038`).
    pub fn update_environment(&mut self, is_tunnel: bool) {
        if self.is_tunnel != is_tunnel {
            self.is_tunnel = is_tunnel;
            spu::configure_reverb(is_tunnel, if is_tunnel { 0x2C00 } else { 0 });
        }
    }

    pub fn new() -> Self {
        let addrs = init_spu_soundbank();

        let mut cdda = CddaController::new();
        cdda.init();

        let mut sfx = SfxPlayer::new();
        sfx.set_addrs(addrs);

        AudioSystem {
            engine: EngineAudio::new(),
            sfx,
            cdda,
            xa: crate::audio::xa::XaPlayer::new(),
            cd_audio: arduracer_core::save::CdAudioMode::default().as_u8(),
            sound_volume: 8,
            music_volume: 8,
            is_ducked: false,
            toast_ticks: 0,
            last_music_track: 0,
            is_tunnel: false,
            policy: AudioPolicy::new(),
        }
    }

    /// Ticks the audio subsystem once per 60Hz frame (`NO1-037`..`NO1-041`).
    #[allow(clippy::too_many_arguments)]
    pub fn tick(
        &mut self,
        player: &VehicleState,
        throttle: Fixed,
        surface: SurfaceType,
        hit_wall: bool,
        checkpoints_cleared: u8,
        rivals: &[AiRacer],
        start_seq: Option<&StartSequence>,
        is_final_lap: bool,
        screen_x: i16,
        is_tunnel: bool,
    ) {
        self.tick_toast();

        if !self.policy.racing_voices() {
            self.silence_race_voices();
            return;
        }

        // 1. Dynamic SPU Reverb (NO1-038)
        self.update_environment(is_tunnel);

        // 2. Multi-layer engine synthesis & rival spatial 3D audio (NO1-037, NO1-039)
        self.engine.update(player, throttle, rivals);

        // 3. Sound effect state machine (tire squeal, curb, multi-surface, impacts, boost, chime, jump, countdown, announcer)
        self.sfx.update(
            player,
            surface,
            hit_wall,
            checkpoints_cleared,
            start_seq,
            is_final_lap,
            screen_x,
        );
    }

    /// Arms the engine synth and race SFX for a running stage.
    pub fn enter_race(&mut self) {
        self.policy.enter_race();
        self.duck_music(false);
    }

    /// Applies the pause veil to the audio policy: pausing cuts the engine
    /// immediately and ducks music.
    pub fn set_paused(&mut self, paused: bool) {
        self.policy.set_paused(paused);
        self.duck_music(paused);
        if paused {
            self.silence_race_voices();
        }
    }

    /// Cuts the engine synth and every race SFX voice.
    pub fn silence_race_voices(&mut self) {
        self.engine.silence();
        self.sfx.silence();
    }

    /// Disarms the race voices and cuts them: entering a menu, the pause veil,
    /// or the results screen.
    pub fn leave_race(&mut self) {
        self.policy.silence_race_voices();
        self.silence_race_voices();
        self.update_environment(false);
        self.duck_music(true);
    }

    /// Feeds one frame of button state and plays the resulting UI cue, if any.
    pub fn tick_ui(&mut self, buttons: u16) -> Option<UiSound> {
        self.tick_toast();
        let cue = self.policy.ui_sound(buttons);
        if let Some(sound) = cue {
            self.sfx.play_ui(sound);
        }
        cue
    }

    /// [`Self::tick_ui`] for call sites that do not use the cue.
    pub fn ui_frame(&mut self, buttons: u16) {
        let _ = self.tick_ui(buttons);
    }

    /// Adopts the buttons held right now so a skipped frame is not re-read as a
    /// fresh cue.
    pub fn sync_ui_edges(&mut self, buttons: u16) {
        self.policy.sync_edges(buttons);
    }
}
