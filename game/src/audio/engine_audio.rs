//! Dynamic Engine RPM Pitch Synthesizer.
//!
//! Synthesizes continuous engine pitch modulation on SPU Voice 0, mapping
//! real-time RPM (1000–8500 RPM) to SPU hardware pitch registers (0x0800–0x3FFF).

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
            current_pitch: 0x0A00,
            current_vol: 0x1000,
        }
    }

    /// Updates engine synthesizer parameters based on vehicle state.
    pub fn update(&mut self, engine_rpm: u16, throttle: Fixed) {
        let rpm = engine_rpm.clamp(900, 8500);

        // Map 900..8500 RPM to SPU hardware pitch range 0x0700..0x3400
        // (44.1kHz playback rate multiplier in Q5.12)
        let rpm_span = (rpm - 900) as u32;
        let target_pitch = (0x0700u32 + (rpm_span * 0x2D00) / 7600) as u16;

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

        VOICE_ENGINE.set_pitch(Pitch::raw(self.current_pitch));
        let vol = Volume(self.current_vol);
        VOICE_ENGINE.set_volume(vol, vol);
    }
}
