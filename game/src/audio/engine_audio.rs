//! Dynamic Multi-Layer Engine RPM Synthesizer & Spatial 3D Rival Audio (`NO1-037`, `NO1-039`).
//!
//! Synthesizes continuous engine pitch modulation across:
//! - Voice 0 (`VOICE_ENGINE`): Primary cylinder pulse loop (idle to mid RPM).
//! - Voice 1 (`VOICE_ENGINE_HIGH`): High-RPM screamer harmonic wail (crossfades in above 3500 RPM).
//! - Voice 2 (`VOICE_BLOWOFF`): Turbo blow-off valve sneeze and exhaust pop on upshift and throttle release.
//! - Voices 6..10 (`VOICE_RIVALS`): Up to 5 AI Rivals spatial engines with distance attenuation,
//!   Doppler pitch shift, and stereo panning!

use crate::audio::audio_policy::{engine_pitch_for, IDLE_PITCH, MAX_ENGINE_PITCH};
use crate::audio::spu::{VOICE_BLOWOFF, VOICE_ENGINE, VOICE_ENGINE_HIGH, VOICE_RIVALS};
use arduracer_core::{AiRacer, Fixed, VehicleState};
use psx_spu::{Pitch, Voice, Volume};

pub struct EngineAudio {
    pub current_pitch: u16,
    pub current_vol: i16,
    pub high_pitch: u16,
    pub high_vol: i16,
    prev_gear: u8,
    prev_throttle: Fixed,
    rival_vol: [i16; 5],
}

impl Default for EngineAudio {
    fn default() -> Self {
        Self::new()
    }
}

impl EngineAudio {
    pub const fn new() -> Self {
        EngineAudio {
            current_pitch: IDLE_PITCH,
            current_vol: 0,
            high_pitch: IDLE_PITCH,
            high_vol: 0,
            prev_gear: 1,
            prev_throttle: Fixed::ZERO,
            rival_vol: [0; 5],
        }
    }

    /// Cuts every engine voice immediately (player and rivals).
    pub fn silence(&mut self) {
        if self.current_vol != 0 || self.high_vol != 0 {
            self.current_vol = 0;
            self.high_vol = 0;
            VOICE_ENGINE.set_volume(Volume::SILENCE, Volume::SILENCE);
            VOICE_ENGINE_HIGH.set_volume(Volume::SILENCE, Volume::SILENCE);
        }
        for (i, voice) in VOICE_RIVALS.iter().enumerate() {
            if self.rival_vol[i] != 0 {
                self.rival_vol[i] = 0;
                voice.set_volume(Volume::SILENCE, Volume::SILENCE);
                Voice::key_off(voice.mask());
            }
        }
    }

    /// Whether the engine voice is currently audible.
    pub const fn is_silent(&self) -> bool {
        self.current_vol == 0 && self.high_vol == 0
    }

    /// Updates multi-layer player engine synthesizer and AI rival spatial audio (`NO1-037`, `NO1-039`).
    pub fn update(&mut self, player: &VehicleState, throttle: Fixed, rivals: &[AiRacer]) {
        // -------------------------------------------------------------
        // 1. Primary Engine Layer (VOICE_ENGINE: Low-to-Mid RPM Cylinder Pulse)
        // -------------------------------------------------------------
        let target_pitch = engine_pitch_for(player.engine_rpm);

        if target_pitch > self.current_pitch {
            let delta = ((target_pitch - self.current_pitch) >> 2).max(1);
            self.current_pitch = self.current_pitch.saturating_add(delta);
        } else if target_pitch < self.current_pitch {
            let delta = ((self.current_pitch - target_pitch) >> 2).max(1);
            self.current_pitch = self.current_pitch.saturating_sub(delta);
        }

        let throttle_raw = throttle.raw().clamp(0, 4096);
        // Base low/mid volume: louder at low/mid RPM, gently rolling off above 6000 RPM
        let base_target_vol = (0x0C00 + (throttle_raw * 0x1A00 / 4096)) as i16;
        let rpm_falloff = if player.engine_rpm > 6000 {
            ((player.engine_rpm - 6000) as i32 * 0x0800 / 2000) as i16
        } else {
            0
        };
        let target_vol = base_target_vol.saturating_sub(rpm_falloff);

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

        // -------------------------------------------------------------
        // 2. High-RPM Screamer Layer (VOICE_ENGINE_HIGH)
        // -------------------------------------------------------------
        // Crossfades in above 3500 RPM, reaching peak power at 7500..8000 RPM
        let target_high_vol = if player.engine_rpm > 3500 {
            let rpm_ratio = (player.engine_rpm - 3500) as i32;
            let throttle_mult = (throttle_raw + 1024).min(4096);
            ((rpm_ratio * 0x2400 / 4500) * throttle_mult / 4096) as i16
        } else {
            0
        };

        if target_high_vol > self.high_vol {
            self.high_vol = self
                .high_vol
                .saturating_add((target_high_vol - self.high_vol) >> 2);
        } else if target_high_vol < self.high_vol {
            self.high_vol = self
                .high_vol
                .saturating_sub((self.high_vol - target_high_vol) >> 3);
        }

        let high_target_pitch = engine_pitch_for(player.engine_rpm);
        VOICE_ENGINE_HIGH.set_pitch(Pitch::raw(high_target_pitch.min(MAX_ENGINE_PITCH)));
        let h_vol = Volume(self.high_vol);
        VOICE_ENGINE_HIGH.set_volume(h_vol, h_vol);

        // -------------------------------------------------------------
        // 3. Turbo Blow-Off Valve & Upshift Backfire (VOICE_BLOWOFF)
        // -------------------------------------------------------------
        let upshifted = player.gear > self.prev_gear && player.engine_rpm > 4500;
        let lift_off =
            (self.prev_throttle.raw() - throttle.raw()) > 2048 && player.engine_rpm > 5500;
        if upshifted || lift_off {
            let blowoff_pitch = if upshifted { 0x1000 } else { 0x0E00 };
            VOICE_BLOWOFF.set_pitch(Pitch::raw(blowoff_pitch));
            let blowoff_vol = if upshifted {
                Volume::MAX
            } else {
                Volume(0x2800)
            };
            VOICE_BLOWOFF.set_volume(blowoff_vol, blowoff_vol);
            Voice::key_on(VOICE_BLOWOFF.mask());
        }
        self.prev_gear = player.gear;
        self.prev_throttle = throttle;

        // -------------------------------------------------------------
        // 4. Spatial 3D Rival Engines with Doppler Pitch Shift (VOICE_RIVALS, NO1-039)
        // -------------------------------------------------------------
        for (i, &voice) in VOICE_RIVALS.iter().enumerate() {
            if i >= rivals.len() {
                if self.rival_vol[i] != 0 {
                    self.rival_vol[i] = 0;
                    voice.set_volume(Volume::SILENCE, Volume::SILENCE);
                    Voice::key_off(voice.mask());
                }
                continue;
            }

            let rival = &rivals[i];
            let rel_x = (rival.state.position.x - player.position.x).to_int();
            let rel_y = (rival.state.position.y - player.position.y).to_int();
            let dist = (rival.state.position - player.position).length().to_int();

            // Audible up to 320 world units (~5 track tiles)
            const MAX_AUDIBLE_DIST: i32 = 320;
            if dist < MAX_AUDIBLE_DIST {
                // Distance attenuation
                let atten = MAX_AUDIBLE_DIST - dist;
                let rival_base_vol = ((0x1800 * atten) / MAX_AUDIBLE_DIST) as i16;

                // Stereo panning based on relative position across the vehicle's heading
                let h = player.heading;
                let cos_h = arduracer_core::math::cos(h).raw();
                let sin_h = arduracer_core::math::sin(h).raw();
                let lateral_dist =
                    ((rel_x * cos_h + rel_y * sin_h) >> arduracer_core::FP_SHIFT).clamp(-120, 120);
                let pan_scale_left = (120 - lateral_dist).clamp(30, 210);
                let pan_scale_right = (120 + lateral_dist).clamp(30, 210);

                let vl = ((rival_base_vol as i32 * pan_scale_left) / 120).clamp(0, 0x3FFF) as i16;
                let vr = ((rival_base_vol as i32 * pan_scale_right) / 120).clamp(0, 0x3FFF) as i16;

                // Doppler shift: relative speed shifts pitch
                let rel_speed = rival.state.speed.raw() - player.speed.raw();
                let doppler_shift = (rel_speed / 48).clamp(-0x0280, 0x0300) as i16;
                let rival_base_pitch = engine_pitch_for(rival.state.engine_rpm) as i16;
                let final_pitch = (rival_base_pitch + doppler_shift).clamp(0x0600, 0x1E00) as u16;

                voice.set_pitch(Pitch::raw(final_pitch));
                voice.set_volume(Volume(vl), Volume(vr));

                if self.rival_vol[i] == 0 {
                    Voice::key_on(voice.mask());
                }
                self.rival_vol[i] = rival_base_vol;
            } else if self.rival_vol[i] != 0 {
                self.rival_vol[i] = 0;
                voice.set_volume(Volume::SILENCE, Volume::SILENCE);
                Voice::key_off(voice.mask());
            }
        }
    }
}
