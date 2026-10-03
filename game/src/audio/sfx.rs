//! Tire Squeal and Environmental Sound Effects Player.
//!
//! Manages procedural tire screech volume/pitch scaling, barrier collision
//! impacts, turbo boost whooshes, and checkpoint completion chimes.

use crate::audio::spu::{VOICE_BOOST, VOICE_CHIME, VOICE_CRASH, VOICE_SKID};
use arduracer_core::{Fixed, SurfaceType, VehicleState};
use psx_spu::{Pitch, Voice, Volume};

pub struct SfxPlayer {
    skid_playing: bool,
    skid_vol: i16,
    curb_playing: bool,
    prev_boost_ticks: u16,
    prev_checkpoints: u8,
}

impl SfxPlayer {
    pub const fn new() -> Self {
        SfxPlayer {
            skid_playing: false,
            skid_vol: 0,
            curb_playing: false,
            prev_boost_ticks: 0,
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

    /// Updates SFX state each 60Hz tick.
    ///
    /// `checkpoints_cleared` should be monotonic within a lap; `hit_wall` is the
    /// authoritative crash signal from the collision solver rather than a
    /// heuristic speed-delta guess (which is unreliable on a fixed-point model).
    pub fn update(
        &mut self,
        player: &VehicleState,
        surface: SurfaceType,
        hit_wall: bool,
        checkpoints_cleared: u8,
    ) {
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

        // 2. Curb rumble loop: reuses the skid voice at a low, gritty level so
        // clipping a rumble strip is audible without a dedicated sample.
        if surface.triggers_curb_rumble() && player.speed > Fixed::from_raw(600) {
            let vol = Volume(0x0800);
            VOICE_SKID.set_volume(vol, vol);
            self.curb_playing = true;
        } else if self.curb_playing && !self.skid_playing {
            let vol = Volume::SILENCE;
            VOICE_SKID.set_volume(vol, vol);
            Voice::key_off(VOICE_SKID.mask());
            self.curb_playing = false;
        }

        // 3. Barrier collision (authoritative, from the collision solver).
        if hit_wall {
            self.play_crash();
        }

        // 4. Boost activation trigger: boost_ticks transitioned from 0 to > 0
        if player.boost_ticks > 0 && self.prev_boost_ticks == 0 {
            self.play_boost();
        }

        // 5. Checkpoint chime trigger: checkpoint count increased
        if checkpoints_cleared > self.prev_checkpoints {
            self.play_checkpoint();
        }

        self.prev_boost_ticks = player.boost_ticks;
        self.prev_checkpoints = checkpoints_cleared;
    }
}
