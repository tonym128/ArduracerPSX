//! CD-DA Redbook Audio Streaming Controller.
//!
//! Controls Redbook CD-DA track playback streamed directly from disc via the
//! CD-ROM drive controller into the SPU CD audio mixer channel.

use psx_io::cdrom;
use psx_spu::{self as spu, CdVolume};

use crate::audio::audio_policy::CdAudioPolicy;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum CddaState {
    Stopped,
    Playing,
    Paused,
    Muted,
}

/// SetMode byte for Red Book playback: CD-DA enabled, single speed.
///
/// SetMode bit 7 is the drive's *rate* select and it governs CD-DA playback,
/// not just data reads (0 = 1x, 1 = 2x). Our masters are 44.1 kHz Red Book
/// audio (GAME.md 6.1), so bit 7 must stay clear or every track comes out
/// an octave up at double tempo. Streaming that genuinely wants 2x -- the FMV
/// reader -- programs the mode itself through `psx_pack::cd::SectorReader`.
const CDDA_PLAY_MODE: u8 = cdrom::MODE_CDDA;

/// Build-time guard on the invariant above: any future edit that sneaks the
/// double-speed bit back into the music mode breaks compilation instead of
/// silently doubling every track's tempo.
const _: () = assert!(CDDA_PLAY_MODE & cdrom::MODE_DOUBLE_SPEED == 0);

pub struct CddaController {
    pub current_track: u8,
    pub state: CddaState,
    pub volume: CdVolume,
    /// Whether the player permits Redbook playback, and which track holds the
    /// drive. Owns the decision; this type owns the register writes.
    policy: CdAudioPolicy,
}

impl Default for CddaController {
    fn default() -> Self {
        Self::new()
    }
}

static mut ACTIVE_CDDA_TRACK: u8 = 0;

/// Absolute BCD disc position to resume from, saved before playback is
/// suspended, plus whether the drive actually reported one.
///
/// Zero is a legal MSF, so it cannot double as "nothing saved"; the flag is
/// what says whether the position is real.
static mut SAVED_POSITION: [u8; 3] = [0; 3];
static mut SAVED_POSITION_VALID: bool = false;

/// Reads and remembers the drive's current absolute position.
///
/// Returns whether a usable position came back. Index 0 is the pregap, before
/// the music proper, so a position there is treated as unusable: seeking back to
/// it is indistinguishable from starting the track, and a half-reported reply
/// (short read, drive not ready) is no better.
fn save_position() -> bool {
    let parsed = cdrom::try_get_loc_p(50_000)
        .and_then(|resp| cdrom::PlayPosition::parse(&resp))
        .filter(|p| {
            p.index >= 1 && p.absolute_min <= 99 && p.absolute_sec < 60 && p.absolute_frame < 75
        });
    let saved = match parsed {
        Some(p) => {
            unsafe {
                let msf = &mut *core::ptr::addr_of_mut!(SAVED_POSITION);
                msf[0] = cdrom::bin_to_bcd(p.absolute_min);
                msf[1] = cdrom::bin_to_bcd(p.absolute_sec);
                msf[2] = cdrom::bin_to_bcd(p.absolute_frame);
            }
            true
        }
        None => false,
    };
    unsafe {
        core::ptr::write_volatile(core::ptr::addr_of_mut!(SAVED_POSITION_VALID), saved);
    }
    saved
}

/// Seeks back to the position [`save_position`] captured, consuming it.
///
/// Returns false if there was nothing to seek to or the seek did not answer, so
/// the caller can fall back to playing the track from its start. That fallback
/// is audible but never fatal; the alternative -- seeking unconditionally -- is
/// a seek to an arbitrary position whenever the first read went unanswered.
fn seek_to_saved_position() -> bool {
    if !unsafe { core::ptr::read_volatile(core::ptr::addr_of!(SAVED_POSITION_VALID)) } {
        return false;
    }
    unsafe {
        core::ptr::write_volatile(core::ptr::addr_of_mut!(SAVED_POSITION_VALID), false);
    }
    let msf = unsafe { *core::ptr::addr_of!(SAVED_POSITION) };
    cdrom::try_set_loc_msf(msf[0], msf[1], msf[2], 50_000).is_some()
}

/// Returns the currently active playing CD-DA track, if any.
pub fn active_cdda_track() -> Option<u8> {
    let t = unsafe { core::ptr::read_volatile(core::ptr::addr_of!(ACTIVE_CDDA_TRACK)) };
    if t != 0 {
        Some(t)
    } else {
        None
    }
}

/// Safely pauses CD-DA playback for background data sector reading.
///
/// Records where the music had got to first. A block read can run to hundreds
/// of milliseconds, so without this the track is not merely paused for it but
/// restarted: see [`resume_after_cd_read`].
pub fn pause_for_cd_read() {
    let t = unsafe { core::ptr::read_volatile(core::ptr::addr_of!(ACTIVE_CDDA_TRACK)) };
    if t != 0 {
        save_position();
        cdrom::try_pause_until_complete(50_000);
    }
}

/// Resumes CD-DA playback following a background sector read.
///
/// Seeks back to the saved position rather than re-issuing Play. The two are
/// not interchangeable: CdlPlay's parameter is a *track number*, so it tells
/// the drive "start this track", which rewinds the song to its first sample.
/// With a texture block crossing every few seconds that is not a stutter, it is
/// the music restarting over and over.
///
/// Falls back to Play when no position was saved or the seek went
/// unanswered, which restarts the song but keeps the audio running.
pub fn resume_after_cd_read() {
    let t = unsafe { core::ptr::read_volatile(core::ptr::addr_of!(ACTIVE_CDDA_TRACK)) };
    if t != 0 {
        cdrom::try_set_mode(CDDA_PLAY_MODE, 50_000);
        cdrom::try_demute(50_000);
        if !seek_to_saved_position() {
            cdrom::try_play_track(t, 50_000);
        }
    }
}

impl CddaController {
    pub const fn new() -> Self {
        CddaController {
            current_track: 2, // Track 1 is game data; Audio tracks start at 2
            state: CddaState::Stopped,
            volume: CdVolume::MAX,
            policy: CdAudioPolicy::new(),
        }
    }

    /// Whether the player currently permits Redbook playback.
    pub const fn enabled(&self) -> bool {
        self.policy.enabled()
    }

    /// Applies the player's CD-audio preference.
    ///
    /// Disabling stops playback immediately rather than letting the current
    /// song finish. The reason is the drive, not the music: with CD-DA running
    /// the texture streamer has to pause and resume playback around every
    /// sector read, and the stutter that causes is present for as long as the
    /// disc is spinning. Leaving it running would leave the interference in
    /// place after the player asked for it to stop.
    pub fn set_enabled(&mut self, enabled: bool) {
        let must_cut = self.policy.set_enabled(enabled);
        if must_cut {
            self.halt();
        }
    }

    /// Releases the drive and drops out of any playing state.
    ///
    /// Used both by [`CddaController::stop`] and by the disable path, because
    /// "the player turned music off" and "the game stopped the music" want the
    /// same end state: nothing playing, nothing queued, no pending play
    /// command left for the next frame to discover.
    fn halt(&mut self) {
        cdrom::try_pause_until_complete(50_000);
        self.state = CddaState::Stopped;
        // The policy has to be told as well as the state machine. It is what
        // decides whether the next `play_track` is a repeat (and so is dropped)
        // or a fresh request; leaving it holding the old track would make a
        // stop followed by a request for the same track silently do nothing,
        // which is how the title music went missing after a race.
        self.policy.clear();
        unsafe {
            core::ptr::write_volatile(core::ptr::addr_of_mut!(ACTIVE_CDDA_TRACK), 0);
        }
    }

    /// Initializes CD-ROM drive mode for CD-DA playback.
    pub fn init(&mut self) {
        self.apply_mode();
        cdrom::try_demute(50_000);
        spu::set_cd_volume(self.volume, self.volume);
        spu::enable_cd_audio(true);
    }

    /// Programs the drive mode [`CDDA_PLAY_MODE`] describes.
    fn apply_mode(&mut self) {
        cdrom::try_set_mode(CDDA_PLAY_MODE, 50_000);
    }

    /// Begins playback of a specific 1-based CD-DA audio track (Track 2+).
    ///
    /// A no-op when the player has disabled CD-audio, or when the requested
    /// track is already playing. Both are decided by [`CdAudioPolicy`], so no
    /// CD-ROM command is issued for a request that will be refused.
    pub fn play_track(&mut self, track: u8) {
        let Some(track) = self.policy.request_play(track) else {
            return;
        };
        self.current_track = track;
        crate::dbg::print("[CDDA] Requesting play_track ");
        crate::dbg::print_dec(track as u32);
        crate::dbg::println("...");
        self.apply_mode();
        cdrom::try_demute(50_000);
        let ok = cdrom::try_play_track(track, 50_000);
        crate::dbg::print("[CDDA] try_play_track returned ");
        crate::dbg::println(if ok.is_some() { "OK" } else { "TIMEOUT/ERROR" });
        if ok.is_some() {
            self.state = CddaState::Playing;
            unsafe {
                core::ptr::write_volatile(core::ptr::addr_of_mut!(ACTIVE_CDDA_TRACK), track);
            }
        } else {
            self.state = CddaState::Stopped;
            self.policy.clear();
            unsafe {
                core::ptr::write_volatile(core::ptr::addr_of_mut!(ACTIVE_CDDA_TRACK), 0);
            }
        }
    }

    /// Pauses CD-DA playback.
    ///
    /// The position is recorded so [`CddaController::resume`] can carry on from
    /// here rather than from the start of the track.
    pub fn pause(&mut self) {
        if self.state == CddaState::Playing {
            save_position();
            cdrom::try_pause(50_000);
            self.state = CddaState::Paused;
            self.policy.clear();
            unsafe {
                core::ptr::write_volatile(core::ptr::addr_of_mut!(ACTIVE_CDDA_TRACK), 0);
            }
        }
    }

    /// Resumes playback of the paused CD-DA track.
    ///
    /// Seeks to where [`CddaController::pause`] left off. Restarting the track
    /// instead would mean the song began again every time the player opened the
    /// pause menu, which is audible within a couple of laps.
    pub fn resume(&mut self) {
        // Gated like any other play request: a pause can outlive the player
        // turning CD-audio off, and resuming would restart the disc behind
        // their back.
        if self.state == CddaState::Paused {
            if self.policy.request_play(self.current_track).is_none() {
                return;
            }
            cdrom::try_demute(50_000);
            let resumed = if !seek_to_saved_position() {
                cdrom::try_play_track(self.current_track, 50_000).is_some()
            } else {
                true
            };
            if resumed {
                self.state = CddaState::Playing;
                unsafe {
                    core::ptr::write_volatile(
                        core::ptr::addr_of_mut!(ACTIVE_CDDA_TRACK),
                        self.current_track,
                    );
                }
            } else {
                self.state = CddaState::Stopped;
                self.policy.clear();
                unsafe {
                    core::ptr::write_volatile(core::ptr::addr_of_mut!(ACTIVE_CDDA_TRACK), 0);
                }
            }
        }
    }

    /// Stops CD-DA playback completely.
    pub fn stop(&mut self) {
        self.halt();
    }

    /// Mutes CD-DA playback.
    pub fn mute(&mut self) {
        cdrom::try_mute(50_000);
        self.state = CddaState::Muted;
    }

    /// Unmutes CD-DA playback.
    pub fn demute(&mut self) {
        cdrom::try_demute(50_000);
        if self.state == CddaState::Muted {
            self.state = CddaState::Playing;
        }
    }

    /// Adjusts CD-DA playback volume.
    pub fn set_volume(&mut self, volume: CdVolume) {
        self.volume = volume;
        spu::set_cd_volume(volume, volume);
    }
}
