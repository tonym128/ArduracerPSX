//! CD-DA Redbook Audio Streaming Controller.
//!
//! Controls Redbook CD-DA track playback streamed directly from disc via the
//! CD-ROM drive controller into the SPU CD audio mixer channel.

use psx_io::cdrom;
use psx_spu::{self as spu, CdVolume};

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
}

impl CddaController {
    pub const fn new() -> Self {
        CddaController {
            current_track: 2, // Track 1 is game data; Audio tracks start at 2
            state: CddaState::Stopped,
            volume: CdVolume::MAX,
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
    pub fn play_track(&mut self, track: u8) {
        if self.current_track == track && self.state == CddaState::Playing {
            return;
        }
        self.current_track = track;
        // The mode byte is drive-global and shared with the FMV sector reader,
        // so re-assert ours before every Play: a stale 2x/streaming mode would
        // otherwise leave the track silent (CD-DA enable clear) or running fast.
        self.apply_mode();
        cdrom::try_demute(50_000);
        cdrom::try_play_track(track, 50_000);
        self.state = CddaState::Playing;
    }

    /// Pauses CD-DA playback.
    pub fn pause(&mut self) {
        if self.state == CddaState::Playing {
            cdrom::try_pause(50_000);
            self.state = CddaState::Paused;
        }
    }

    /// Resumes playback of the paused CD-DA track.
    pub fn resume(&mut self) {
        if self.state == CddaState::Paused {
            cdrom::try_demute(50_000);
            cdrom::try_play_track(self.current_track, 50_000);
            self.state = CddaState::Playing;
        }
    }

    /// Stops CD-DA playback completely.
    pub fn stop(&mut self) {
        cdrom::try_stop(50_000);
        self.state = CddaState::Stopped;
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
