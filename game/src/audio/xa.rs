//! CD-XA music playback.
//!
//! # The hardware does the decoding
//!
//! This is the whole reason the format exists, and it is worth stating plainly
//! because the obvious implementation -- decode ADPCM on the CPU, buffer PCM,
//! write it to the SPU's CD input -- is the wrong one.
//!
//! The SPU decodes XA-ADPCM itself. With `Setmode` bit 6 set, the drive routes
//! matching sectors straight to the SPU, which decompresses them and mixes the
//! result through the same CD audio path as Redbook. No CPU involvement, no
//! buffering, no per-frame feeding: the sector rate is the only cost, and at
//! 37.8 kHz stereo 4-bit that is 18.75 sectors per second against CD-DA's 75.2.
//!
//! psx-spx is explicit that the Setmode sector-size bit is "don't care" here,
//! because "the hardware does always decompress all 900h bytes" -- 2304 of the
//! 2328 bytes a whole-sector read delivers.
//!
//! # So what is this module
//!
//! Three things, all of them about the drive rather than the audio:
//!
//! 1. Opening the stream with the right Setmode.
//! 2. Giving the drive up when a texture run needs it, and taking it back.
//! 3. Resuming where it stopped, which needs the drive's position, because
//!    restarting a track from its top whenever a texture arrives is the
//!    "music restarting over and over" bug this whole format was meant to avoid.
//!
//! # Interleaving
//!
//! Real XA discs interleave audio sectors (every Nth sector is audio).
//! At single speed (75 sectors/s), 1:4 interleave matches the 37.8 kHz stereo
//! 4-bit XA consumption rate of 18.75 sectors/s precisely (75 / 4 = 18.75).
//! Dummy sectors (channel 255) fill the gaps and are discarded by the hardware filter.

use crate::cd_fs::DiscReader;

/// File and channel numbers our authored XA files carry in every sector
/// subheader. `Setfilter` is set to match, so the drive routes only these
/// sectors to the SPU and leaves everything else to the CPU.
pub const XA_FILE: u8 = 1;
pub const XA_CHANNEL: u8 = 0;

/// The disc file name for a given music track.
///
/// `MUSIC1.XA` .. `MUSIC6.XA`. ISO9660 stores these with a `;1` version
/// suffix, and `find_file_lba` matches on the name as a prefix before the
/// semicolon, so the returned slice is the bare name with no version.
pub fn track_file_name(track: u8) -> [u8; 9] {
    let mut out = [0u8; 9];
    out.copy_from_slice(b"MUSIC0.XA");
    out[5] = b'0' + track;
    out
}

/// Where the drive was when it was last paused.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
struct Position {
    lba: u32,
    valid: bool,
}

/// The XA music stream.
///
/// A global rather than a field, because the drive arbiter's preempt hook is a
/// plain `fn()` and cannot capture the game state. The same reason `DiscReader`
/// is a global.
pub struct XaPlayer {
    /// LBA of the track's first sector.
    track_lba: u32,
    /// Where the drive was when it was last paused.
    stopped_at: Position,
    /// The drive stream is open.
    stream_open: bool,
    /// Preempted by a texture run; owed a resume.
    suspended: bool,
}

impl Default for XaPlayer {
    fn default() -> Self {
        Self::new()
    }
}

/// The one XA music stream.
static mut PLAYER: XaPlayer = XaPlayer {
    track_lba: 0,
    stopped_at: Position {
        lba: 0,
        valid: false,
    },
    stream_open: false,
    suspended: false,
};

/// The preempt hook the drive arbiter calls when textures need the drive.
///
/// A plain `fn()` because the arbiter stores one and cannot capture state; it
/// reaches the player through the global.
pub fn player_suspend() {
    player().suspend();
}

/// The global player, for the arbiter's preempt hook.
pub fn player() -> &'static mut XaPlayer {
    unsafe { &mut *core::ptr::addr_of_mut!(PLAYER) }
}

impl XaPlayer {
    pub const fn new() -> Self {
        XaPlayer {
            track_lba: 0,
            stopped_at: Position {
                lba: 0,
                valid: false,
            },
            stream_open: false,
            suspended: false,
        }
    }

    /// Whether a track is loaded.
    pub fn has_track(&self) -> bool {
        self.track_lba != 0
    }

    /// Whether the player holds a drive stream.
    pub fn fetching(&self) -> bool {
        self.stream_open
    }

    /// Whether the player was preempted and owes a resume.
    pub fn is_suspended(&self) -> bool {
        self.suspended
    }

    /// Opens the stream at `lba`.
    ///
    /// Takes the drive from whoever holds it -- textures preempt music, never
    /// the reverse, because a paused music stream resumes from a remembered
    /// position while an interrupted texture run leaves a tile half-written.
    pub fn start_track(&mut self, reader: &mut DiscReader, lba: u32) {
        self.track_lba = lba;
        self.stopped_at = Position { lba, valid: false };
        self.resume(reader);
    }

    /// Opens or reopens the stream, at the saved position if there is one.
    ///
    /// Returns false when the drive is not available, which is the ordinary
    /// answer on a frame where textures are mid-run; the player simply tries
    /// again next frame.
    pub fn resume(&mut self, reader: &mut DiscReader) -> bool {
        if self.stream_open || self.track_lba == 0 {
            return self.stream_open;
        }
        if !crate::drive::claim(crate::drive::Owner::Xa) {
            return false;
        }
        let from = if self.stopped_at.valid {
            let offset = self.stopped_at.lba.saturating_sub(self.track_lba);
            self.track_lba + (offset & !3)
        } else {
            self.track_lba
        };
        if !reader.begin_xa_stream(from) {
            crate::drive::release(crate::drive::Owner::Xa);
            return false;
        }
        self.stream_open = true;
        self.suspended = false;
        true
    }

    /// Gives the drive to a texture run, remembering where the stream got to.
    ///
    /// The position is read *after* the pause, because the drive stops wherever
    /// it happened to be and there is no other way to know. Resuming from the
    /// top of the track instead would restart the song on every texture fetch.
    pub fn suspend(&mut self) {
        if !self.stream_open {
            return;
        }
        // The reader is a global, so the arbiter's plain `fn()` hook can reach
        // it without capturing anything.
        let reader = crate::gpu::tracktex::disc_reader();
        if let Some(lba) = reader.xa_position() {
            self.stopped_at = Position { lba, valid: true };
        }
        reader.stop_xa_stream();
        self.stream_open = false;
        self.suspended = true;
        crate::drive::release(crate::drive::Owner::Xa);
    }

    /// Resumes the stream if a texture run preempted it.
    ///
    /// The stream is continuous, so this is the only per-frame work: if it was
    /// paused, reopen it at the remembered position. Called once per frame from
    /// the render path, because the drive arbiter only acts when something asks.
    pub fn service(&mut self, reader: &mut DiscReader) {
        if !self.suspended || self.track_lba == 0 {
            return;
        }
        self.resume(reader);
    }

    /// Closes the stream and drops the track.
    pub fn stop(&mut self, reader: &mut DiscReader) {
        if self.stream_open {
            reader.stop_xa_stream();
            crate::drive::release(crate::drive::Owner::Xa);
        }
        self.stream_open = false;
        self.suspended = false;
        self.track_lba = 0;
        self.stopped_at = Position {
            lba: 0,
            valid: false,
        };
    }
}
