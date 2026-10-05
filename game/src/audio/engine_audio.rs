//! Dynamic Engine RPM Pitch Synthesizer.
//!
//! Synthesizes continuous engine pitch modulation on SPU Voice 0, mapping
//! real-time RPM (`MIN_RPM`..=`MAX_RPM`) to an SPU pitch in
//! `MIN_ENGINE_PITCH`..=`MAX_ENGINE_PITCH`. The ceiling keeps the loop inside
//! its Nyquist limit; the map itself lives in `audio_policy.rs` so it can be
//! host-tested.

use crate::audio::audio_policy::{engine_pitch_for, IDLE_PITCH, MAX_ENGINE_PITCH};
use crate::audio::spu::VOICE_ENGINE;
use arduracer_core::Fixed;
use psx_spu::{Pitch, Volume};

pub struct EngineAudio {
    pub current_pitch: u16,
    pub current_vol: i16,
}

impl Default for EngineAudio {
    fn default() -> Self {
        Self::new()
    }
}

impl EngineAudio {
    pub const fn new() -> Self {
        EngineAudio {
            // Resting pitch, also clamped: the old seed of 0x0A00 was already
            // above the ceiling.
            current_pitch: IDLE_PITCH,
            // Silent, not `0x1000`. The idle target computed in `update` bottoms
            // out at `0x0C00` and can never reach zero, so a non-zero seed is
            // audible on its own -- it drones from construction until the first
            // `update` call, which the title screen never makes.
            current_vol: 0,
        }
    }

    /// Cuts the engine voice immediately, without waiting for the release slew.
    ///
    /// Idempotent: calling it on an already-silent engine is a no-op, so the
    /// state machine can call it on every frame it is not racing.
    pub fn silence(&mut self) {
        if self.current_vol == 0 {
            return;
        }
        self.current_vol = 0;
        VOICE_ENGINE.set_volume(Volume::SILENCE, Volume::SILENCE);
    }

    /// Whether the engine voice is currently audible.
    pub const fn is_silent(&self) -> bool {
        self.current_vol == 0
    }

    /// Updates engine synthesizer parameters based on vehicle state.
    pub fn update(&mut self, engine_rpm: u16, throttle: Fixed) {
        // Map RPM to an SPU pitch, clamped so the loop cannot be driven past
        // its own Nyquist limit.
        //
        // The pitch register is a Q12 multiplier on the sample rate, so 0x1000
        // replays a 22.05 kHz sample at exactly its recorded rate and anything
        // above that folds the harmonics back over themselves -- the old map
        // reached 0x3400, 3.25x, 2.4x past Nyquist, which is why the engine
        // sounds like a buzz at high RPM (TASK-1207).
        let target_pitch = engine_pitch_for(engine_rpm);

        // Smooth pitch transitions (1/4th step per 60Hz frame)
        if target_pitch > self.current_pitch {
            let delta = ((target_pitch - self.current_pitch) >> 2).max(1);
            self.current_pitch = self.current_pitch.saturating_add(delta);
        } else if target_pitch < self.current_pitch {
            let delta = ((self.current_pitch - target_pitch) >> 2).max(1);
            self.current_pitch = self.current_pitch.saturating_sub(delta);
        }

        // Modulate volume by throttle load:
        // Idle off-throttle: ~0x0C00 (subdued hum)
        // Full throttle: 0x2800 (roaring wide-open acceleration)
        let throttle_raw = throttle.raw().clamp(0, 4096);
        let target_vol = (0x0C00 + (throttle_raw * 0x1C00 / 4096)) as i16;

        if target_vol > self.current_vol {
            self.current_vol = self
                .current_vol
                .saturating_add((target_vol - self.current_vol) >> 2);
        } else if target_vol < self.current_vol {
            self.current_vol = self
                .current_vol
                .saturating_sub((self.current_vol - target_vol) >> 3);
        }

        VOICE_ENGINE.set_pitch(Pitch::raw(self.current_pitch.min(MAX_ENGINE_PITCH)));
        let vol = Volume(self.current_vol);
        VOICE_ENGINE.set_volume(vol, vol);
    }
}
