//! PlayStation 1 Audio Engine for Arduracer PSX.
//!
//! Controls hardware SPU voices (engine synthesizer, tire screech, crash
//! impacts, turbo whoosh, checkpoint chimes) and Redbook CD-DA streaming music.

pub mod cdda;
pub mod engine_audio;
pub mod sfx;
pub mod soundbank;
pub mod spu;

pub use cdda::{CddaController, CddaState};
pub use engine_audio::EngineAudio;
pub use sfx::SfxPlayer;
pub use spu::init_spu_soundbank;

use arduracer_core::{Fixed, VehicleState};

/// Central audio subsystem managing hardware voices and disc audio streaming.
pub struct AudioSystem {
    pub engine: EngineAudio,
    pub sfx: SfxPlayer,
    pub cdda: CddaController,
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
        }
    }

    /// Ticks the audio subsystem once per 60Hz frame.
    pub fn tick(&mut self, player: &VehicleState, throttle: Fixed, checkpoints_cleared: u8) {
        // 1. Dynamic engine synthesis
        self.engine.update(player.engine_rpm, throttle);

        // 2. Sound effect state machine (tire squeal, impacts, boost, chime)
        self.sfx.update(player, checkpoints_cleared);
    }
}
