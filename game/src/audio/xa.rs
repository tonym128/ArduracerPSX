//! CD-XA music playback.
//!
//! # Why this is a player and not a drive command
//!
//! Redbook CD-DA needs nothing from the host: the drive streams PCM straight
//! into the SPU's CD input port and the hardware mixes it. XA gives the drive an
//! ADPCM stream it cannot decode, so the CPU has to do it -- read sectors,
//! decode them, and write every stereo frame into that same port itself. At
//! 37.8 kHz that is 37,800 frames a second, which is the entire ongoing cost of
//! this format.
//!
//! # How it coexists with the texture streamer
//!
//! One drive. The texture fetcher and this player cannot both have a stream
//! open, so the player only seeks when the texture fetcher is idle, and it takes
//! a run of sectors rather than one, so a fetch is a few frames every few
//! seconds rather than a seek per sector.
//!
//! # Buffering
//!
//! [`RING_FRAMES`] is sized in frames of playback, not in sectors: at 37.8 kHz
//! and 60 fps a frame consumes 630 of them, so the ring holds about six frames
//! of audio. That is the jitter budget -- enough that a frame which runs long,
//! or a fetch that lands late, is absorbed without an audible gap, without
//! being large enough to delay a track change by more than the player tolerates.
//!
//! Underrun is not papered over. When the ring empties the player writes
//! silence, which is the honest output for "no audio arrived", and reports it,
//! so a starved player is visible rather than merely quiet.

use arduracer_core::save::CdAudioMode;
use arduracer_core::xa::{
    decode_sector, frames_per_video_frame, ChannelState, Coding, Ring, RUN_SECTORS, XA_SECTOR_BYTES,
};

/// Stereo frames the ring holds: a whole drive run plus the jitter budget.
///
/// Sized in `arduracer_core::xa` because the arithmetic behind it -- why it must
/// hold a whole run rather than just jitter -- is the drive budget argument, and
/// that belongs where it can be tested.
pub use arduracer_core::xa::MIN_RING_FRAMES as RING_FRAMES;

/// Frames a sector decodes to, for the mastered 4-bit coding.
const FRAMES_PER_SECTOR: usize = arduracer_core::xa::FRAMES_PER_SECTOR_4BIT;

/// CD-XA music player.
pub struct XaPlayer {
    ring: Ring,
    left: ChannelState,
    right: ChannelState,
    /// LBA of the current track's first sector, 0 when nothing is loaded.
    track_lba: u32,
    /// Sectors of the open run already decoded.
    run_have: usize,
    run_total: usize,
    run_open: bool,
    /// Frames fed per rendered frame, from the source rate and the frame rate.
    frames_per_video_frame: u32,
    /// Fractional remainder, so 630.5 frames/frame does not drift.
    accumulator: u32,
    /// Frames of silence written because nothing arrived.
    pub underruns: u32,
    /// Decoded frames handed to the SPU.
    pub frames_played: u32,
}

impl Default for XaPlayer {
    fn default() -> Self {
        XaPlayer::new()
    }
}

impl XaPlayer {
    pub const fn new() -> Self {
        XaPlayer {
            ring: Ring::new(),
            left: ChannelState::new(),
            right: ChannelState::new(),
            track_lba: 0,
            run_have: 0,
            run_total: 0,
            run_open: false,
            frames_per_video_frame: 630,
            accumulator: 0,
            underruns: 0,
            frames_played: 0,
        }
    }

    /// Recomputes the playback cadence for a given video frame rate.
    ///
    /// 37,800 / 60 = 630 exactly; at 30 fps it is 1260, which is why this is
    /// derived rather than hardcoded. Getting it wrong is not a subtle pitch
    /// shift -- it plays at the wrong speed, or runs the buffer dry.
    pub fn set_frame_rate(&mut self, video_hz: u32) {
        self.frames_per_video_frame = frames_per_video_frame(video_hz);
    }

    /// Points the player at a track and clears everything downstream of it.
    pub fn start_track(&mut self, lba: u32) {
        self.track_lba = lba;
        self.left.reset();
        self.right.reset();
        self.ring.clear();
        self.run_open = false;
        self.run_have = 0;
        self.run_total = 0;
    }

    /// Whether a track is loaded.
    pub fn has_track(&self) -> bool {
        self.track_lba != 0
    }

    /// Frames buffered and not yet played.
    pub fn buffered(&self) -> usize {
        self.ring.len()
    }

    /// Whether a drive run is open.
    pub fn fetching(&self) -> bool {
        self.run_open
    }

    /// Opens a run of sectors, if the drive is free.
    ///
    /// Returns false when the texture streamer holds the drive, which is the
    /// normal outcome on a frame that also wants tiles. One drive, one stream.
    pub fn begin_run(&mut self, textures_busy: bool) -> bool {
        if self.run_open || textures_busy || self.track_lba == 0 {
            return false;
        }
        self.run_have = 0;
        self.run_total = RUN_SECTORS;
        self.run_open = true;
        true
    }

    /// Decodes one just-read sector into the ring.
    ///
    /// The caller has already taken the sector off the drive; this only turns it
    /// into frames. Separate from the fetch so that a sector which never
    /// arrives costs a retry rather than a decode.
    pub fn decode_sector_into_ring(&mut self, sector: &[u8]) {
        let mut out = [0i16; FRAMES_PER_SECTOR * 2];
        if decode_sector(
            sector,
            Coding::MASTERED,
            &mut self.left,
            &mut self.right,
            &mut out,
        )
        .is_none()
        {
            return;
        }
        for frame in out.chunks_exact(2) {
            self.ring.push(frame[0], frame[1]);
        }
        self.run_have += 1;
        if self.run_have >= self.run_total {
            self.run_open = false;
        }
    }

    /// True once the open run has been fully consumed.
    pub fn run_complete(&self) -> bool {
        !self.run_open || self.run_have >= self.run_total
    }

    /// Writes this video frame's worth of audio into the SPU's CD input.
    ///
    /// Bursts rather than paces: the port is a register, not a DMA channel, so a
    /// software player can only write frames and hope the average lands. The
    /// ring is what makes the burst inaudible -- six frames of slack absorbs the
    /// jitter between when this runs and when the drive delivers.
    pub fn feed(&mut self) {
        self.accumulator += self.frames_per_video_frame;
        let mut due = self.accumulator;
        self.accumulator = 0;
        while due > 0 {
            match self.ring.pop() {
                Some((left, right)) => {
                    psx_spu::write_cd_input(left, right);
                    due -= 1;
                    self.frames_played += 1;
                }
                None => {
                    // Nothing arrived. Silence is the honest output; counting it
                    // is what makes a starved player visible instead of merely
                    // quiet.
                    psx_spu::write_cd_input(0, 0);
                    self.underruns += 1;
                    due -= 1;
                }
            }
        }
    }

    /// Whether this player should be running given the chosen mode.
    pub fn wants_playback(mode: CdAudioMode) -> bool {
        mode == CdAudioMode::Xa
    }
}

/// Bytes one raw sector occupies on the wire.
pub const SECTOR_BYTES: usize = XA_SECTOR_BYTES;

/// Frames the player must hold buffered before it is worth another seek.
pub const LOW_WATER: usize = arduracer_core::xa::JITTER_FRAMES;
