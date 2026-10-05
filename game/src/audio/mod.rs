//! PlayStation 1 Audio Engine for Arduracer PSX.
//!
//! Controls hardware SPU voices (engine synthesizer, tire screech, crash
//! impacts, turbo whoosh, checkpoint chimes) and Redbook CD-DA streaming music.

pub mod audio_policy;
pub mod cdda;
pub mod engine_audio;
pub mod sfx;
pub mod soundbank;
pub mod spu;

pub use audio_policy::{AudioPolicy, UiSound};
pub use cdda::{CddaController, CddaState};
pub use engine_audio::EngineAudio;
pub use sfx::SfxPlayer;
pub use spu::{init_spu_soundbank, play_intro_audio, stop_intro_audio};

use arduracer_core::{Fixed, SurfaceType, VehicleState};

/// Central audio subsystem managing hardware voices and disc audio streaming.
pub struct AudioSystem {
    pub engine: EngineAudio,
    pub sfx: SfxPlayer,
    pub cdda: CddaController,
    pub policy: AudioPolicy,
}

impl Default for AudioSystem {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioSystem {
    pub fn new() -> Self {
        init_spu_soundbank();

        let mut cdda = CddaController::new();
        cdda.init();

        AudioSystem {
            engine: EngineAudio::new(),
            sfx: SfxPlayer::new(),
            cdda,
            // Nothing is audible until a stage loads: the title screen and the
            // intro play CD-DA only.
            policy: AudioPolicy::new(),
        }
    }

    /// Ticks the audio subsystem once per 60Hz frame.
    ///
    /// Gated on the policy: when the race voices are disarmed (a menu, the
    /// results screen, the pause veil) this silences them and returns without
    /// touching the vehicle state, so no racing sound can leak out of a frame
    /// that never should have been audible.
    pub fn tick(
        &mut self,
        player: &VehicleState,
        throttle: Fixed,
        surface: SurfaceType,
        hit_wall: bool,
        checkpoints_cleared: u8,
    ) {
        if !self.policy.racing_voices() {
            self.silence_race_voices();
            return;
        }

        // 1. Dynamic engine synthesis
        self.engine.update(player.engine_rpm, throttle);

        // 2. Sound effect state machine (tire squeal, curb, impacts, boost, chime)
        self.sfx
            .update(player, surface, hit_wall, checkpoints_cleared);
    }

    /// Arms the engine synth and race SFX for a running stage.
    pub fn enter_race(&mut self) {
        self.policy.enter_race();
    }

    /// Applies the pause veil to the audio policy: pausing cuts the engine
    /// immediately, unpausing re-arms it.
    pub fn set_paused(&mut self, paused: bool) {
        self.policy.set_paused(paused);
        if paused {
            self.silence_race_voices();
        }
    }

    /// Cuts the engine synth and every race SFX voice.
    ///
    /// Idempotent, and safe to call on a state that was never audible.
    pub fn silence_race_voices(&mut self) {
        self.engine.silence();
        self.sfx.silence();
    }

    /// Disarms the race voices and cuts them: entering a menu, the pause veil,
    /// or the results screen.
    pub fn leave_race(&mut self) {
        self.policy.silence_race_voices();
        self.silence_race_voices();
    }

    /// Feeds one frame of button state and plays the resulting UI cue, if any.
    ///
    /// Only ever plays the navigation blip -- never the engine or a race SFX.
    /// Returns the cue so a test can assert on it.
    pub fn tick_ui(&mut self, buttons: u16) -> Option<UiSound> {
        let cue = self.policy.ui_sound(buttons);
        if cue.is_some() {
            self.sfx.play_ui_move();
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
