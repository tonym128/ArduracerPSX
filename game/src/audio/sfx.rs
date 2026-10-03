//! Tire Squeal and Environmental Sound Effects Player.
//!
//! Manages procedural tire screech volume/pitch scaling, barrier collision
//! impacts, turbo boost whooshes, and checkpoint completion chimes.

use crate::audio::spu::{VOICE_BOOST, VOICE_CHIME, VOICE_CRASH, VOICE_SKID};
use arduracer_core::{Fixed, VehicleState};
use psx_spu::{Pitch, Voice, Volume};

pub struct SfxPlayer {
    skid_playing: bool,
    skid_vol: i16,
    prev_boost_ticks: u16,
    prev_speed: Fixed,
    prev_checkpoints: u8,
}

impl SfxPlayer {
    pub const fn new() -> Self {
        SfxPlayer {
            skid_playing: false,
            skid_vol: 0,
            prev_boost_ticks: 0,
            prev_speed: Fixed::ZERO,
            prev_checkpoints: 0,
        }
    }

    /// Triggers a barrier crash impact sound effect.
    pub fn play_crash(&self) {
        Voice::key_on(VOICE_CRASH.mask());
    }

    /// Triggers a turbo boost activation whoosh sound effect.
    pub fn play_boost(&self) {
        Voice::key_on(VOICE_BOOST.mask());
    }

    /// Triggers a checkpoint crossing chime.
    pub fn play_checkpoint(&self) {
        Voice::key_on(VOICE_CHIME.mask());
    }

    /// Updates SFX state each 60Hz tick based on vehicle and checkpoint status.
    pub fn update(&mut self, player: &VehicleState, checkpoints_cleared: u8) {
        // 1. Tire Screech during drift
        if player.is_drifting && player.speed > Fixed::from_int(2) {
            if !self.skid_playing {
                Voice::key_on(VOICE_SKID.mask());
                self.skid_playing = true;
            }

            // Volume scales with speed and slip angle (up to 0x2400)
            let speed_ratio = (player.speed.raw().min(40960) * 0x2400) / 40960;
            let target_vol = speed_ratio as i16;
            self.skid_vol = self
                .skid_vol
                .saturating_add((target_vol - self.skid_vol) >> 1);

            // Pitch slightly shifts up with higher speed
            let target_pitch = 0x0E00 + ((player.speed.raw() as u32 * 0x0600) / 40960) as u16;
            VOICE_SKID.set_pitch(Pitch::raw(target_pitch));
            let vol = Volume(self.skid_vol);
            VOICE_SKID.set_volume(vol, vol);
        } else if self.skid_playing {
            // Rapid fadeout
            self.skid_vol = self.skid_vol.saturating_sub(self.skid_vol >> 1);
            let vol = Volume(self.skid_vol);
            VOICE_SKID.set_volume(vol, vol);
            if self.skid_vol < 100 {
                Voice::key_off(VOICE_SKID.mask());
                self.skid_playing = false;
                self.skid_vol = 0;
            }
        }

        // 2. Barrier collision detection: sudden large drop in speed (> 2.0 within one tick)
        let speed_loss = self.prev_speed - player.speed;
        if speed_loss > Fixed::from_int(2) {
            self.play_crash();
        }

        // 3. Boost activation trigger: boost_ticks transitioned from 0 to > 0
        if player.boost_ticks > 0 && self.prev_boost_ticks == 0 {
            self.play_boost();
        }

        // 4. Checkpoint chime trigger: checkpoint count increased
        if checkpoints_cleared > self.prev_checkpoints {
            self.play_checkpoint();
        }

        self.prev_boost_ticks = player.boost_ticks;
        self.prev_speed = player.speed;
        self.prev_checkpoints = checkpoints_cleared;
    }
}
