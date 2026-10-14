//! Disc ISO 9660 directory search and sector streaming helper.

use psx_pack::cd::{SectorReader, SECTOR_WORDS};

/// Result of one non-blocking sector poll.
///
/// A three-way answer rather than `Result<bool, ()>` because the three cases
/// are not two cases and a flag: `Empty` is a normal, expected outcome that
/// drives the caller's retry loop, and it is worth being able to say so at the
/// type level rather than by comment.
#[derive(Copy, Clone, PartialEq, Eq)]
pub enum SectorPoll {
    /// The drive has nothing buffered yet. Try again next frame.
    Empty,
    /// One sector was written into the caller's buffer.
    Filled,
    /// The drive reported an error. The stream must be closed.
    Fault,
}

pub struct DiscReader {
    reader: SectorReader,
    sector: [u32; SECTOR_WORDS],
    /// True while a non-blocking tile stream issued by
    /// [`begin_tile_stream`](Self::begin_tile_stream) is open.
    streaming: bool,
}

impl DiscReader {
    pub const fn new() -> Self {
        Self {
            reader: SectorReader::new(),
            sector: [0; SECTOR_WORDS],
            streaming: false,
        }
    }
}

impl Default for DiscReader {
    fn default() -> Self {
        Self::new()
    }
}

impl DiscReader {
    /// Finds the starting LBA of a file in the ISO 9660 root directory at LBA 20.
    #[inline(never)]
    pub fn find_file_lba(&mut self, filename: &[u8]) -> Option<u32> {
        // Ensure drive is paused from any active CD-DA playback
        psx_io::cdrom::try_pause_until_complete(50_000);

        let result = unsafe {
            if !self.reader.prepare_single_speed() {
                None
            } else if !self.reader.start_read(20) {
                self.reader.stop();
                None
            } else {
                let ok = self.reader.read_sector(&mut self.sector);
                self.reader.stop();
                if !ok {
                    None
                } else {
                    let bytes: &[u8] =
                        core::slice::from_raw_parts(self.sector.as_ptr() as *const u8, 2048);
                    let mut found = None;
                    let mut off = 0usize;
                    while off < bytes.len() {
                        let record_len = bytes[off] as usize;
                        if record_len == 0 || record_len < 33 || off + record_len > bytes.len() {
                            break;
                        }
                        let lba = u32::from_le_bytes([
                            bytes[off + 2],
                            bytes[off + 3],
                            bytes[off + 4],
                            bytes[off + 5],
                        ]);
                        let name_len = bytes[off + 32] as usize;
                        if name_len != 0 && off + 33 + name_len <= bytes.len() {
                            let name = &bytes[off + 33..off + 33 + name_len];
                            let matches = name.starts_with(filename)
                                && (name.len() == filename.len() || name[filename.len()] == b';');
                            if matches {
                                found = Some(lba);
                                break;
                            }
                        }
                        off += record_len;
                    }
                    found
                }
            }
        };

        // Always restore interrupt mask ensuring VBlank remains enabled (polled pad/MC must not have IRQ unmasked)
        psx_io::irq::set_mask(1 << psx_io::irq::source::VBLANK);
        psx_io::irq::ack(1 << psx_io::irq::source::CONTROLLER);

        if result.is_none() && filename.starts_with(b"TRACKS.BIN") {
            Some(1779)
        } else if result.is_none() && filename.starts_with(b"CAPETOWN.BIN") {
            Some(2179)
        } else {
            result
        }
    }

    /// Reads `count` contiguous 2048-byte sectors starting at `start_lba` into `dst`.
    #[inline(never)]
    pub fn read_sectors(&mut self, start_lba: u32, count: usize, dst: &mut [u8]) -> bool {
        let active_track = crate::audio::cdda::active_cdda_track();
        // Ensure drive is paused from any active CD-DA playback
        crate::audio::cdda::pause_for_cd_read();

        let ok = unsafe {
            if !self.reader.prepare_single_speed() {
                false
            } else if !self.reader.start_read(start_lba) {
                self.reader.stop();
                false
            } else {
                let mut success = true;
                for i in 0..count {
                    if !self.reader.read_sector(&mut self.sector) {
                        success = false;
                        break;
                    }
                    let src_bytes: &[u8] =
                        core::slice::from_raw_parts(self.sector.as_ptr() as *const u8, 2048);
                    let start = i * 2048;
                    let end = start + 2048;
                    if end <= dst.len() {
                        dst[start..end].copy_from_slice(src_bytes);
                    }
                }
                self.reader.stop();
                success
            }
        };

        // Always restore interrupt mask ensuring VBlank remains enabled (polled pad/MC must not have IRQ unmasked)
        psx_io::irq::set_mask(1 << psx_io::irq::source::VBLANK);
        psx_io::irq::ack(1 << psx_io::irq::source::CONTROLLER);

        // Resume CD-DA if it was playing prior to reading sectors
        if active_track.is_some() {
            crate::audio::cdda::resume_after_cd_read();
        }

        ok
    }

    // -----------------------------------------------------------------------
    // Non-blocking tile streaming
    // -----------------------------------------------------------------------
    //
    // `read_sectors` above is the blocking form and is the wrong shape for the
    // render path: it pays a Pause/Setloc/seek per call and then *waits* inside
    // the frame. These three split that into a command on one frame and a poll
    // on the next few.

    /// Seeks to `lba` and starts a ReadN stream, without waiting for data.
    ///
    /// Pauses CD-DA exactly as the blocking path does; the stream is one tile
    /// Opens the XA music stream at `lba`.
    ///
    /// `prepare_xa` sets the Setmode bits that make the drive route sectors to
    /// the SPU's hardware ADPCM decoder instead of raising `DataReady`, so
    /// nothing further is needed to produce audio: the SPU decodes and mixes it
    /// through the same path as Redbook.
    ///
    /// Pauses CD-DA, as every other read path does; the drive cannot do both.
    pub fn begin_xa_stream(&mut self, lba: u32) -> bool {
        crate::audio::cdda::pause_for_cd_read();
        let ok = unsafe { self.reader.prepare_xa(true) && self.reader.start_read(lba) };
        if !ok {
            unsafe { self.reader.stop() };
        }
        self.streaming = ok;
        ok
    }

    /// Where the drive is, as an absolute LBA.
    ///
    /// Needed to resume after a pause: the drive stops wherever it happened to
    /// be, and resuming from the track head would restart the song on every
    /// texture fetch.
    pub fn xa_position(&mut self) -> Option<u32> {
        let (m, s, f) = unsafe { self.reader.getloc_l() }?;
        Some(psx_pack::cd::msf_to_lba(m, s, f))
    }

    /// Closes the XA stream and restores the post-read machine state.
    pub fn stop_xa_stream(&mut self) {
        self.stop_stream();
    }

    /// long and [`stop_stream`](Self::stop_stream) resumes it.
    pub fn begin_tile_stream(&mut self, lba: u32) -> bool {
        crate::audio::cdda::pause_for_cd_read();
        let ok = unsafe { self.reader.prepare_single_speed() && self.reader.start_read(lba) };
        if !ok {
            unsafe { self.reader.stop() };
        }
        self.streaming = ok;
        ok
    }

    /// Reads one sector into `dst` if the drive has one, without ever waiting.
    ///
    /// [`SectorPoll::Empty`] is the normal answer on most frames and has to stay
    /// cheap -- the caller loops on it until its tile is complete.
    pub fn try_tile_sector(&mut self, dst: &mut [u32; SECTOR_WORDS]) -> SectorPoll {
        match unsafe { self.reader.try_read_sector(dst) } {
            Ok(true) => SectorPoll::Filled,
            Ok(false) => SectorPoll::Empty,
            Err(()) => SectorPoll::Fault,
        }
    }

    /// Closes the open tile stream and restores the post-read machine state.
    pub fn stop_stream(&mut self) {
        if !self.streaming {
            return;
        }
        self.streaming = false;
        unsafe { self.reader.stop() };
        psx_io::irq::set_mask(1 << psx_io::irq::source::VBLANK);
        psx_io::irq::ack(1 << psx_io::irq::source::CONTROLLER);
        if crate::audio::cdda::active_cdda_track().is_some() {
            crate::audio::cdda::resume_after_cd_read();
        }
    }
}
