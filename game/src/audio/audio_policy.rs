//! Audio policy: which voices are allowed to sound, and when.
//!
//! Deliberately free of hardware dependencies: it decides *what* should be
//! audible and returns that decision, while `engine_audio.rs`, `sfx.rs` and
//! `spu.rs` own the SPU register writes. `tools/test_audio` exercises this
//! file directly.
//!
//! Why the split: the engine voice is a continuous loop that is keyed on at
//! boot and whose volume register is only ever rewritten from
//! `AudioSystem::tick`. `tick` is reached from the race arm of the state match
//! and nowhere else, so every other screen -- and the pause veil, which
//! `continue`s before the tick -- inherited whatever volume the last racing
//! frame happened to leave behind. That is why the engine droned through the
//! results screen and under the menu music.
//!
//! Owning the decision here rather than sprinkling `silence()` calls across the
//! state machine makes the failure mode a no-op instead of a drone: a screen
//! that forgets to opt back in stays quiet rather than buzzing.

/// Highest pitch the engine loop may be driven to, in Q12 (0x1000 = the
/// sample's own rate). Chosen so the 22.05 kHz loop never exceeds its Nyquist
/// limit (TASK-1207).
pub const MAX_ENGINE_PITCH: u16 = 0x0800;

/// Resting pitch, before the first `update` call.
pub const IDLE_PITCH: u16 = 0x0500;

/// RPM range the engine voice is mapped from.
pub const MIN_RPM: u16 = 900;
pub const MAX_RPM: u16 = 8500;

/// Pitch at `MIN_RPM` and at `MAX_RPM`.
pub const MIN_ENGINE_PITCH: u16 = 0x0400;

/// The RPM-to-pitch map, with the ceiling applied.
///
/// Split out as a pure function so `tools/test_audio` can assert the Nyquist
/// clamp without touching SPU registers.
pub const fn engine_pitch_for(rpm: u16) -> u16 {
    let rpm = if rpm < MIN_RPM {
        MIN_RPM
    } else if rpm > MAX_RPM {
        MAX_RPM
    } else {
        rpm
    };
    let span = (MAX_RPM - MIN_RPM) as u32;
    let raw = MIN_ENGINE_PITCH as u32
        + ((rpm - MIN_RPM) as u32 * (MAX_ENGINE_PITCH - MIN_ENGINE_PITCH) as u32) / span;
    if raw > MAX_ENGINE_PITCH as u32 {
        MAX_ENGINE_PITCH
    } else {
        raw as u16
    }
}

/// How long after an impact a further impact is ignored, in frames.
///
/// The crash sample is 0.25 s, so a quarter second is exactly its length: a car
/// grinding along a barrier produces one thud rather than a machine gun.
pub const CRASH_REFRACTORY_FRAMES: u8 = 15;

/// Edge-detects barrier impacts and rate-limits them.
///
/// Split out of `SfxPlayer` so `tools/test_audio` can pin the retrigger
/// behaviour: the surrounding `update` writes SPU registers and cannot be
/// linked on the host, but this is the whole of the decision.
///
/// Without it, `hit_wall` -- true on *every* frame the car touches a barrier --
/// restarted the impact sample 60 times a second, which is heard as a buzz
/// rather than a crash (TASK-1207b).
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct CrashGate {
    prev_hit_wall: bool,
    cooldown: u8,
}

impl CrashGate {
    pub const fn new() -> Self {
        CrashGate {
            prev_hit_wall: false,
            cooldown: 0,
        }
    }

    /// Whether this frame is a new impact worth sounding.
    ///
    /// Fires on the leading edge of contact only, and at most once per
    /// refractory period, so one knock is one thud even when the collision
    /// solver reports contact across several frames or the car bounces and
    /// re-touches.
    pub fn should_fire(&mut self, hit_wall: bool) -> bool {
        if self.cooldown > 0 {
            self.cooldown -= 1;
        }
        let fire = hit_wall && !self.prev_hit_wall && self.cooldown == 0;
        if fire {
            self.cooldown = CRASH_REFRACTORY_FRAMES;
        }
        self.prev_hit_wall = hit_wall;
        fire
    }

    /// Clears the edge state so a fresh contact after a silence counts again.
    pub fn reset(&mut self) {
        self.prev_hit_wall = false;
        self.cooldown = 0;
    }
}

/// A UI sound cue derived from a button press.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum UiSound {
    /// The highlight moved (D-pad in any direction).
    Move,
    /// An entry was confirmed.
    Confirm,
    /// The screen was dismissed without confirming.
    Back,
}

/// The button bits the policy needs, mirroring `psx_pad::button`.
pub const BTN_UP: u16 = 1 << 0;
pub const BTN_DOWN: u16 = 1 << 1;
pub const BTN_LEFT: u16 = 1 << 2;
pub const BTN_RIGHT: u16 = 1 << 3;
pub const BTN_CROSS: u16 = 1 << 4;
pub const BTN_CIRCLE: u16 = 1 << 5;
pub const BTN_START: u16 = 1 << 8;
pub const BTN_SELECT: u16 = 1 << 9;

const DIRECTION_MASK: u16 = BTN_UP | BTN_DOWN | BTN_LEFT | BTN_RIGHT;
const CONFIRM_MASK: u16 = BTN_CROSS | BTN_START;
const CANCEL_MASK: u16 = BTN_CIRCLE;

/// Which voices the game currently wants to hear.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct AudioPolicy {
    /// Set while a stage is loaded, cleared on leaving it.
    in_race: bool,
    /// Set while the pause veil is up.
    paused: bool,
    /// Last frame's raw button mask; the sole edge-detection state.
    prev_buttons: u16,
}

impl AudioPolicy {
    pub const fn new() -> Self {
        AudioPolicy {
            // Silent until a race starts: nothing is audible during boot, the
            // intro, or the title screen.
            in_race: false,
            paused: false,
            prev_buttons: 0,
        }
    }

    /// Whether the engine synth and race SFX may sound this frame.
    ///
    /// The pause veil vetoes independently of the race state, so a stage that
    /// loads while the veil is up stays silent.
    pub const fn racing_voices(&self) -> bool {
        self.in_race && !self.paused
    }

    /// Arms the race voices, e.g. when a stage loads or the race restarts.
    pub fn enter_race(&mut self) {
        self.in_race = true;
    }

    /// Disarms the race voices. Called for the pause veil, the results screen,
    /// and every menu.
    pub fn silence_race_voices(&mut self) {
        self.in_race = false;
    }

    /// Applies the pause veil. Pausing silences the race voices immediately;
    /// unpausing re-arms them.
    pub fn set_paused(&mut self, paused: bool) {
        self.paused = paused;
    }

    /// Feeds one frame of button state and returns the UI cue to play.
    ///
    /// Fires at most one cue per frame, checked in descending priority: a
    /// confirm or cancel wins over a move, because on the frame the player
    /// presses CROSS the highlight did not also move. Held buttons do not
    /// retrigger -- a cue needs a genuine press edge.
    pub fn ui_sound(&mut self, buttons: u16) -> Option<UiSound> {
        let pressed = buttons & !self.prev_buttons;
        self.prev_buttons = buttons;

        if pressed & CONFIRM_MASK != 0 {
            Some(UiSound::Confirm)
        } else if pressed & CANCEL_MASK != 0 {
            Some(UiSound::Back)
        } else if pressed & DIRECTION_MASK != 0 {
            Some(UiSound::Move)
        } else {
            None
        }
    }

    /// Adopts the buttons held right now as already-seen.
    ///
    /// Used after a transition that skipped frames (a restart, say) so the same
    /// press is not re-read as a fresh cue against the new screen.
    pub fn sync_edges(&mut self, buttons: u16) {
        self.prev_buttons = buttons;
    }
}

/// Whether Redbook CD-DA playback is allowed to run, and which track holds the
/// drive.
///
/// This exists because CD-DA and the TIM texture streamer both want the same
/// physical drive. `cd_fs::read_sectors` pauses playback around every read and
/// resumes it afterwards, so with TIM selected the player is not hearing a bug,
/// they are hearing the drive being handed back and forth four sectors at a
/// time. That is a drive-latency problem, not something the audio path can
/// mask, which is why the fix is a player setting rather than a mix change.
///
/// It lives in `audio_policy.rs`, away from `cdda.rs`, for the same reason
/// [`AudioPolicy`] does: the decision -- may this request start, must the
/// running track be cut -- is pure and `tools/test_audio` can drive it, whereas
/// `CddaController` spends its body in CD-ROM register writes.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct CdAudioPolicy {
    enabled: bool,
    /// Track currently holding the drive, 0 when none.
    active_track: u8,
}

impl Default for CdAudioPolicy {
    fn default() -> Self {
        Self::new()
    }
}

impl CdAudioPolicy {
    pub const fn new() -> Self {
        Self {
            enabled: true,
            active_track: 0,
        }
    }

    /// Whether CD-DA playback is currently permitted.
    pub const fn enabled(&self) -> bool {
        self.enabled
    }

    /// Which track currently holds the drive, if any.
    pub const fn active_track(&self) -> Option<u8> {
        if self.active_track != 0 {
            Some(self.active_track)
        } else {
            None
        }
    }

    /// Applies the player's preference.
    ///
    /// Returns `true` when a track was playing and the caller must cut it: the
    /// drive has to be released, not merely stopped accepting new requests.
    /// Leaving the disc spinning is what produces the delayed, stuttering
    /// audio a player would blame on the texture streamer instead of on the
    /// setting they just turned off.
    ///
    /// Reports `false` when nothing holds the drive, so an idle toggle does not
    /// make the caller issue a stop command for a track that never started.
    pub fn set_enabled(&mut self, enabled: bool) -> bool {
        self.enabled = enabled;
        if enabled {
            return false;
        }
        let was_active = self.active_track != 0;
        self.active_track = 0;
        was_active
    }

    /// Decides whether a `play_track(track)` request may reach the drive.
    ///
    /// Returns the track to play, or `None` to skip the CD-ROM commands
    /// entirely. A repeat request for the track already playing is `None` too:
    /// re-issuing play for it would restart the song from its first sample, so
    /// a screen that asks every frame would click.
    pub fn request_play(&mut self, track: u8) -> Option<u8> {
        if !self.enabled || (self.active_track == track) {
            return None;
        }
        self.active_track = track;
        Some(track)
    }

    /// Releases the drive: playback stopped, or paused for a sector read.
    pub fn clear(&mut self) {
        self.active_track = 0;
    }
}

#[cfg(test)]
mod cd_audio_tests {
    use super::CdAudioPolicy;

    #[test]
    fn a_play_request_is_honoured_by_default() {
        let mut p = CdAudioPolicy::new();
        assert_eq!(p.request_play(3), Some(3));
        assert_eq!(p.active_track(), Some(3));
    }

    /// The whole point of the setting: with it off, nothing reaches the drive.
    #[test]
    fn disabling_suppresses_every_play_request() {
        let mut p = CdAudioPolicy::new();
        p.set_enabled(false);
        for track in 1..=8u8 {
            assert_eq!(
                p.request_play(track),
                None,
                "track {track} reached the drive"
            );
        }
        assert_eq!(p.active_track(), None);
    }

    /// Disabling must cut what is already playing, and report that it did, so
    /// the caller actually releases the drive.
    #[test]
    fn disabling_cuts_a_running_track_and_reports_it() {
        let mut p = CdAudioPolicy::new();
        assert_eq!(p.request_play(4), Some(4));
        assert!(
            p.set_enabled(false),
            "caller must be told to stop the drive"
        );
        assert_eq!(p.active_track(), None);
    }

    /// Turning it off when nothing is playing is not an error, and must not
    /// make the caller issue a spurious stop command.
    #[test]
    fn disabling_an_idle_drive_reports_nothing_to_cut() {
        let mut p = CdAudioPolicy::new();
        assert!(!p.set_enabled(false));
    }

    #[test]
    fn re_enabling_lets_playback_resume() {
        let mut p = CdAudioPolicy::new();
        p.set_enabled(false);
        assert!(!p.set_enabled(true), "nothing was playing to cut");
        assert_eq!(p.request_play(6), Some(6));
    }

    /// A screen that asks every frame must not restart the song each time.
    #[test]
    fn a_repeat_request_for_the_playing_track_is_ignored() {
        let mut p = CdAudioPolicy::new();
        assert_eq!(p.request_play(2), Some(2));
        assert_eq!(p.request_play(2), None);
        assert_eq!(
            p.request_play(3),
            Some(3),
            "a different track must go through"
        );
    }

    /// A sector read parks playback; the next request must be honoured again.
    #[test]
    fn clearing_releases_the_drive_without_disabling_it() {
        let mut p = CdAudioPolicy::new();
        p.request_play(2);
        p.clear();
        assert_eq!(p.active_track(), None);
        assert!(p.enabled());
        assert_eq!(p.request_play(2), Some(2));
    }

    /// `clear` is what a `stop()` calls. If it did not clear, the next request
    /// for the same track would be mistaken for a repeat and dropped, and the
    /// screen asking for the theme would get silence.
    #[test]
    fn a_track_can_be_played_again_after_being_stopped() {
        let mut p = CdAudioPolicy::new();
        assert_eq!(p.request_play(2), Some(2));
        p.clear(); // CddaController::stop
        assert_eq!(
            p.request_play(2),
            Some(2),
            "a stop must not mute the theme forever"
        );
    }
}
