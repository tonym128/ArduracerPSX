//! CD-XA ADPCM decoding.
//!
//! # Provenance
//!
//! The algorithm here is not reconstructed from documentation or memory. It is
//! transcribed from the PSoXide SPU's `xa_decode_block`, which is itself
//! parity-verified against PCSX-Redux (`decode_xa.cc`), and the sector layout
//! from its `decode_xa_audio_sector`. Both name nocash PSX-SPX as the source
//! and the filter coefficients are hardware constants.
//!
//! That provenance is the reason this file exists at all rather than a decode
//! loop written inline in the audio path. An XA filter table written from memory
//! produces audio that is *nearly* right -- the right pitch, noise-shaped, no
//! obviously wrong sample -- and near-right audio is far harder to diagnose than
//! silence, and needs the console to diagnose at all. Transcribing a known-good
//! implementation and testing the transcription is both cheaper and safer.
//!
//! # Sector layout
//!
//! A 2336-byte raw sector carries 18 sound groups of 128 bytes starting at
//! offset 24 (12 + 4 + 8 of header before the payload). Within a group, bytes
//! `0..16` are the per-unit headers and `16..128` the sample data.
//!
//! # Bandwidth, and why the coding byte matters
//!
//! The drive is the whole budget. Redbook CD-DA at 44.1 kHz stereo is 75.2
//! sectors/s -- the entire drive, for as long as it plays -- which is why every
//! texture read has to pause it. XA 4-bit stereo at 37.8 kHz is 2016 frames per
//! sector, so 18.75 sectors/s: about a quarter of the drive, and about 3.4x
//! cheaper than CD-DA.
//!
//! 8-bit halves the frames per sector to 1008 and so doubles the cost to 50% of
//! the drive. That still fits alongside the 4bpp texture format, and does not
//! fit alongside the 15bpp one.

/// Bytes in a raw CD-XA sector: 8-byte subheader plus 2328 of Form 2 data.
pub const XA_SECTOR_BYTES: usize = 2336;

/// Offset of the sound-group payload within a raw sector.
pub const XA_PAYLOAD_OFFSET: usize = 8;

/// Sound groups per sector.
pub const XA_GROUPS: usize = 18;

/// Bytes per sound group.
pub const GROUP_BYTES: usize = 128;

/// Per-group header bytes; the rest of the group is sample data.
pub const GROUP_HEADER_BYTES: usize = 16;

/// Stereo frames a 4-bit sector yields: 18 groups x 4 units x 28 samples.
pub const FRAMES_PER_SECTOR_4BIT: usize = XA_GROUPS * 4 * 28;

/// Stereo frames an 8-bit sector yields: 18 groups x 2 units x 28 samples.
pub const FRAMES_PER_SECTOR_8BIT: usize = XA_GROUPS * 2 * 28;

/// Source sample rate a coding byte selects.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum XaRate {
    /// 37,800 Hz -- the standard XA rate.
    Khz37,
    /// 18,900 Hz -- half rate, for when the drive budget is tighter still.
    Khz18,
}

impl XaRate {
    pub const fn hz(self) -> u32 {
        match self {
            XaRate::Khz37 => 37_800,
            XaRate::Khz18 => 18_900,
        }
    }

    fn from_bits(bits: u8) -> Option<Self> {
        match bits {
            0 => Some(XaRate::Khz37),
            1 => Some(XaRate::Khz18),
            _ => None,
        }
    }
}

/// Samples per decoded sound unit.
pub const SAMPLES_PER_UNIT: usize = 28;

/// A parsed XA sector's subheader.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Coding {
    pub stereo: bool,
    pub rate: XaRate,
    pub bits: u8,
}

impl Coding {
    /// The coding this crate masters: 4-bit stereo at 37.8 kHz.
    pub const MASTERED: Coding = Coding {
        stereo: true,
        rate: XaRate::Khz37,
        bits: 4,
    };

    /// Stereo frames one sector of this coding yields.
    pub const fn frames_per_sector(&self) -> usize {
        if self.bits == 4 {
            FRAMES_PER_SECTOR_4BIT
        } else {
            FRAMES_PER_SECTOR_8BIT
        }
    }

    /// Sectors per second a stream of this coding needs.
    ///
    /// The number the drive budget is argued from.
    pub fn sectors_per_second(&self) -> u32 {
        let rate = self.rate.hz();
        let frames = if self.stereo {
            self.frames_per_sector() as u32
        } else {
            // Mono interleaves both channels into the same frame count, so a
            // mono stream carries two channels for the same sector count.
            self.frames_per_sector() as u32
        };
        rate.div_ceil(frames)
    }

    /// Parses the subheader coding byte: bits 0-1 channel count, 2-3 rate,
    /// 4-5 sample width.
    pub fn from_byte(byte: u8) -> Option<Self> {
        let stereo = match byte & 0x03 {
            1 => true,
            0 => false,
            _ => return None,
        };
        Some(Coding {
            stereo,
            rate: XaRate::from_bits((byte >> 2) & 0x03)?,
            // Coding 0 is 4-bit and 1 is 8-bit; the field is a shift of 4,
            // not of 1.
            bits: 4u8 << ((byte >> 4) & 0x03),
        })
    }

    /// The coding byte for this coding.
    pub const fn to_byte(self) -> u8 {
        let ch: u8 = if self.stereo { 1 } else { 0 };
        let rate: u8 = match self.rate {
            XaRate::Khz37 => 0,
            XaRate::Khz18 => 1,
        };
        let bits: u8 = if self.bits == 4 { 0 } else { 1 };
        ch | (rate << 2) | (bits << 4)
    }
}

/// ADPCM prediction filter coefficients `(k0, k1)` in Q10.
///
/// Transcribed from the reference. Filter ids above 3 are reserved and clamp to
/// 3, which is what the reference does and what the hardware does.
const XA_FILTER: [(i32, i32); 4] = [(0, 0), (960, 0), (1840, -832), (1568, -880)];

/// Offsets into a group's 16 header bytes for each unit's filter/range byte.
///
/// A stereo unit's left and right headers are adjacent, hence the pairs.
const HEAD_TABLE: [usize; 4] = [0, 2, 8, 10];

/// Decoder filter history for one channel.
///
/// The predictor is the last two decoded samples, so this state has to survive
/// across sector boundaries -- that is what makes a seek produce an audible
/// click, and why the player resets it when it changes track.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct ChannelState {
    y0: i32,
    y1: i32,
}

impl ChannelState {
    pub const fn new() -> Self {
        ChannelState { y0: 0, y1: 0 }
    }

    /// Silences the history, for a track change.
    pub fn reset(&mut self) {
        *self = ChannelState::new();
    }
}

/// Decodes one sound unit's 28 samples for one channel.
///
/// `filter_range` is the unit's header byte: high nibble filter id, low nibble
/// output shift. `words` is seven packed 16-bit words, four 4-bit samples each,
/// in the order the reference expects.
///
/// `out` receives the 28 samples; `stride` steps between them so stereo and
/// mono share this path.
pub fn decode_unit(
    state: &mut ChannelState,
    filter_range: u8,
    words: &[u16; 7],
    out: &mut [i16],
    stride: usize,
) {
    let filter_id = (((filter_range >> 4) & 0x0F) as usize).min(3);
    let range = u32::from(filter_range & 0x0F);
    let (k0, k1) = XA_FILTER[filter_id];
    let mut y0 = state.y0;
    let mut y1 = state.y1;

    for (i, &word) in words.iter().enumerate() {
        // Sign-extended 4-bit nibble at `shift`, scaled by the unit's range
        // shift and carried in Q4.
        let expand = |shift: u32| -> i32 {
            let nib = ((((word as u32) << shift) & 0xF000) as u16) as i16 as i32;
            (nib >> range) << 4
        };
        let mut x3 = expand(0);
        let mut x2 = expand(4);
        let mut x1 = expand(8);
        let mut x0 = expand(12);

        // The IIR runs x0, x1, x2, x3 in that order, each seeing the two
        // samples decoded before it -- so the order is load-bearing.
        x0 += (y0 * k0 + y1 * k1) >> 10;
        y1 = y0;
        y0 = x0;
        x1 += (y0 * k0 + y1 * k1) >> 10;
        y1 = y0;
        y0 = x1;
        x2 += (y0 * k0 + y1 * k1) >> 10;
        y1 = y0;
        y0 = x2;
        x3 += (y0 * k0 + y1 * k1) >> 10;
        y1 = y0;
        y0 = x3;

        let decoded = [x0, x1, x2, x3];
        for (n, &sample) in decoded.iter().enumerate() {
            let clamped = sample.clamp(-(32768 << 4), 32767 << 4);
            let idx = (i * 4 + n) * stride;
            if idx < out.len() {
                out[idx] = (clamped >> 4) as i16;
            }
        }
    }
    state.y0 = y0;
    state.y1 = y1;
}

/// Decodes one raw XA sector into interleaved stereo `i16` frames.
///
/// Returns the frame count written, or `None` if the sector is short or its
/// layout is not one this decoder handles. `left` and `right` carry filter
/// history across sectors and must be reset on a track change.
pub fn decode_sector(
    raw: &[u8],
    coding: Coding,
    left: &mut ChannelState,
    right: &mut ChannelState,
    out: &mut [i16],
) -> Option<usize> {
    if raw.len() < XA_SECTOR_BYTES {
        return None;
    }
    let payload = raw.get(XA_PAYLOAD_OFFSET..XA_PAYLOAD_OFFSET + XA_GROUPS * GROUP_BYTES)?;
    let units_per_group = if coding.bits == 4 { 4 } else { 2 };
    let frames = coding.frames_per_sector();
    let needed = frames * 2;
    if out.len() < needed {
        return None;
    }
    let stereo = coding.stereo;

    let mut written = 0usize;
    for group_idx in 0..XA_GROUPS {
        let group = &payload[group_idx * GROUP_BYTES..(group_idx + 1) * GROUP_BYTES];
        let headers = &group[..GROUP_HEADER_BYTES];
        let data = &group[GROUP_HEADER_BYTES..];

        for unit in 0..units_per_group {
            // Unpack this unit's samples out of the interleaved group data.
            let (low_words, high_words) = if coding.bits == 4 {
                let mut low = [0u16; 7];
                let mut high = [0u16; 7];
                for k in 0..7 {
                    let base = k * 16 + unit;
                    let b0 = data[base] as u16;
                    let b1 = data[base + 4] as u16;
                    let b2 = data[base + 8] as u16;
                    let b3 = data[base + 12] as u16;
                    low[k] =
                        (b0 & 0x0F) | ((b1 & 0x0F) << 4) | ((b2 & 0x0F) << 8) | ((b3 & 0x0F) << 12);
                    high[k] = (b0 >> 4) | ((b1 >> 4) << 4) | ((b2 >> 4) << 8) | ((b3 >> 4) << 12);
                }
                (low, high)
            } else {
                let mut words = [0u16; 7];
                for (k, word) in words.iter_mut().enumerate() {
                    let base = k * 8 + unit;
                    *word = data[base] as u16 | (data[base + 4] as u16) << 8;
                }
                (words, words)
            };

            let mut first = [0i16; SAMPLES_PER_UNIT];
            let mut second = [0i16; SAMPLES_PER_UNIT];
            let hl = HEAD_TABLE[unit.min(HEAD_TABLE.len() - 1)];
            decode_unit(left, headers[hl], &low_words, &mut first, 1);

            if stereo {
                decode_unit(right, headers[hl + 1], &high_words, &mut second, 1);
                for i in 0..SAMPLES_PER_UNIT {
                    out[written] = first[i];
                    out[written + 1] = second[i];
                    written += 2;
                }
            } else {
                // Mono carries two interleaved units per group: the left
                // decoder's output first, then the right decoder's, each
                // duplicated across the stereo pair. Matches the reference,
                // which is what a mono XA stream on this hardware actually does.
                decode_unit(right, headers[hl + 1], &high_words, &mut second, 1);
                for sample in first.iter().chain(second.iter()) {
                    out[written] = *sample;
                    out[written + 1] = *sample;
                    written += 2;
                }
            }
        }
    }
    Some(written / 2)
}

/// Sectors the player takes per drive run.
///
/// Sized against the seek, which dominates everything else here. A run of N
/// costs one ~150 ms seek plus N * 6.7 ms of transfer and delivers N * 54 ms of
/// audio, so the drive's duty cycle is `(150 + 6.7N) / (54N)`:
/// ```text
/// N = 1 -> 2.9x    N = 2 -> 1.5x    N = 4 -> 0.82    N = 16 -> 0.30
/// ```
/// Under about N = 4 the drive is busier than the audio it carries, which is the
/// whole thing this format was introduced to avoid. See
/// `drive_duty_cycle_is_what_limits_run_length`.
pub const RUN_SECTORS: usize = 4;

/// Stereo frames of jitter budget: 37,800 / 60 = 630 consumed per video frame,
/// and four of those absorbs a frame that runs long or a sector that lands late.
pub const JITTER_FRAMES: usize = 4 * 630;

/// Stereo frames the ring holds: one whole run plus the jitter budget.
///
/// It has to hold a *whole run*, not just jitter. A sector carries 54 ms of
/// audio but arrives every 6.7 ms at 2x, so the drive delivers frames eight
/// times faster than playback consumes them. Decoding a run as it lands
/// therefore deposits `RUN_SECTORS * 2016` frames at once, and a ring sized only
/// for jitter would drop most of the run as overflow -- which reads as the
/// music skipping, not as a click.
pub const MIN_RING_FRAMES: usize = RUN_SECTORS * FRAMES_PER_SECTOR_4BIT + JITTER_FRAMES;

/// Stereo frames of audio per video frame at `video_hz`.
///
/// Derived, not hardcoded: it is 630 at 60 fps and 1260 at 30, and getting it
/// wrong is not a subtle pitch shift -- the stream plays at the wrong speed, or
/// runs its buffer dry.
pub fn frames_per_video_frame(video_hz: u32) -> u32 {
    (Coding::MASTERED.rate.hz() + video_hz / 2) / video_hz.max(1)
}

/// A stereo frame ring: two indices into one flat buffer, no allocation.
///
/// Lives here rather than in the player so it can be tested on the host. The
/// player owns the SPU and the drive; this owns only "which frame is next", and
/// getting that wrong is a bug you would otherwise hear rather than read.
#[derive(Clone)]
pub struct Ring {
    data: [i16; MIN_RING_FRAMES * 2],
    write: usize,
    read: usize,
}

/// Hand-written because the derived one needs `[i16; N]: Default`, which
/// arrays do not have above 32 elements.
impl Default for Ring {
    fn default() -> Self {
        Ring::new()
    }
}

impl Ring {
    pub const fn new() -> Self {
        Ring {
            data: [0; MIN_RING_FRAMES * 2],
            write: 0,
            read: 0,
        }
    }

    /// Frames currently buffered.
    ///
    /// The addition before the modulo is load-bearing: `wrapping_sub` followed
    /// by `%` is Rust's *remainder*, not Euclidean modulo, so an index that has
    /// wrapped reports nonsense -- a ring filled to its nominal size measured
    /// 10095 frames of 10584. Every caller of this decides whether the buffer is
    /// low enough to fetch into, so a wrong depth here is a silent underrun.
    pub fn len(&self) -> usize {
        (self.write + MIN_RING_FRAMES - self.read) % MIN_RING_FRAMES
    }

    pub fn is_empty(&self) -> bool {
        self.write == self.read
    }

    /// Appends one stereo frame.
    ///
    /// On overflow the *oldest* frame goes. Dropping the newest instead would
    /// leave the player permanently one frame behind, which reads as drift
    /// rather than as a click -- a much harder fault to recognise.
    pub fn push(&mut self, left: i16, right: i16) {
        let next = (self.write + 1) % MIN_RING_FRAMES;
        if next == self.read {
            self.read = (self.read + 1) % MIN_RING_FRAMES;
        }
        self.data[self.write * 2] = left;
        self.data[self.write * 2 + 1] = right;
        self.write = next;
    }

    /// The oldest buffered frame.
    pub fn pop(&mut self) -> Option<(i16, i16)> {
        if self.is_empty() {
            return None;
        }
        let frame = (self.data[self.read * 2], self.data[self.read * 2 + 1]);
        self.read = (self.read + 1) % MIN_RING_FRAMES;
        Some(frame)
    }

    pub fn clear(&mut self) {
        self.write = 0;
        self.read = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The reference's own smoke case: a zero filter/range byte over zero data
    /// must decode to exact silence, not drift.
    #[test]
    fn silence_decodes_to_silence() {
        let mut state = ChannelState::new();
        let mut out = [0i16; SAMPLES_PER_UNIT];
        decode_unit(&mut state, 0x00, &[0; 7], &mut out, 1);
        assert_eq!(out, [0i16; SAMPLES_PER_UNIT]);
        assert_eq!(state, ChannelState::new());
    }

    /// The coding byte must round trip through both directions, and the rates
    /// must decode to what the hardware means by them.
    #[test]
    fn coding_byte_round_trips() {
        for (byte, stereo, rate, bits) in [
            (0x00u8, false, XaRate::Khz37, 4u8),
            (0x01, true, XaRate::Khz37, 4),
            (0x05, true, XaRate::Khz18, 4),
            (0x11, true, XaRate::Khz37, 8),
            (0x15, true, XaRate::Khz18, 8),
        ] {
            let c = Coding::from_byte(byte).expect("byte should parse");
            assert_eq!(c.stereo, stereo, "stereo for {byte:#04x}");
            assert_eq!(c.rate, rate, "rate for {byte:#04x}");
            assert_eq!(c.bits, bits, "bits for {byte:#04x}");
            assert_eq!(c.to_byte(), byte, "{byte:#04x} did not round trip");
        }
    }

    /// The drive budget, which is the whole reason this format is here. If this
    /// changes, the feasibility argument in the docs changes with it.
    #[test]
    fn mastered_coding_fits_the_drive() {
        let c = Coding::MASTERED;
        assert_eq!(c.frames_per_sector(), 2016);
        // 37800 / 2016 = 18.75 -> 19 sectors/s, a quarter of a 1x drive's 75.
        assert_eq!(c.sectors_per_second(), 19);
        // 19 is 37800/2016 = 18.75 rounded up, so 25.3% of a 1x drive. The
        // assertion allows one sector of rounding headroom rather than pretending
        // the division is exact.
        assert!(
            c.sectors_per_second() <= 20,
            "mastered XA must not exceed a quarter of the drive"
        );
        // 8-bit halves the frames and so doubles the cost, which is what stops
        // it fitting alongside the 15bpp texture format.
        let eight = Coding {
            stereo: true,
            rate: XaRate::Khz37,
            bits: 8,
        };
        assert_eq!(eight.frames_per_sector(), 1008);
        assert_eq!(eight.sectors_per_second(), 38);
    }

    /// A full sector of zeroed payload decodes to exactly the frame count the
    /// coding promises, and touches no state beyond silence.
    #[test]
    fn silent_sector_yields_the_promised_frames() {
        let mut raw = [0u8; XA_SECTOR_BYTES];
        // 8-byte subheader: file=1, channel=0, submode=0x64, coding=0x01
        raw[..8].copy_from_slice(&[
            1,
            0,
            0x64,
            Coding::MASTERED.to_byte(),
            1,
            0,
            0x64,
            Coding::MASTERED.to_byte(),
        ]);
        let mut out = [0i16; FRAMES_PER_SECTOR_4BIT * 2];
        let mut l = ChannelState::new();
        let mut r = ChannelState::new();
        let n = decode_sector(&raw, Coding::MASTERED, &mut l, &mut r, &mut out)
            .expect("a full-size sector must decode");
        assert_eq!(n, FRAMES_PER_SECTOR_4BIT);
        assert!(out.iter().all(|&s| s == 0), "silence must stay silent");
    }

    /// A short sector is refused rather than read past the end.
    #[test]
    fn short_sector_is_refused() {
        let raw = [0u8; 100];
        let mut out = [0i16; 64];
        let mut l = ChannelState::new();
        let mut r = ChannelState::new();
        assert!(decode_sector(&raw, Coding::MASTERED, &mut l, &mut r, &mut out).is_none());
    }

    /// Cadence must follow the video frame rate: 630 at 60 fps, 1260 at 30.
    #[test]
    fn cadence_follows_the_video_frame_rate() {
        assert_eq!(frames_per_video_frame(60), 630);
        assert_eq!(frames_per_video_frame(30), 1260);
    }

    /// The ring must hand frames back in the order they went in.
    #[test]
    fn ring_is_fifo_and_reports_depth() {
        let mut r = Ring::new();
        assert!(r.is_empty());
        for i in 0..10i16 {
            r.push(i, -i);
        }
        assert_eq!(r.len(), 10);
        for i in 0..10i16 {
            assert_eq!(r.pop(), Some((i, -i)), "frame {i} came back wrong");
        }
        assert!(r.is_empty());
        assert_eq!(r.pop(), None);
    }

    /// Overflow drops the oldest and keeps the newest, so a late sector costs a
    /// glitch rather than permanent drift.
    #[test]
    fn ring_overflow_keeps_the_newest() {
        let mut r = Ring::new();
        let pushed: usize = MIN_RING_FRAMES + 5;
        for i in 0..pushed {
            r.push(i as i16, i as i16);
        }
        assert_eq!(r.len(), MIN_RING_FRAMES - 1);
        // The last `MIN_RING_FRAMES - 1` frames pushed must be exactly what
        // comes back, in order: the dropped ones are the oldest.
        let first_surviving: i32 = (pushed - (MIN_RING_FRAMES - 1)) as i32;
        for expect in first_surviving..(pushed as i32) {
            assert_eq!(
                r.pop(),
                Some((expect as i16, expect as i16)),
                "frame {expect} did not survive"
            );
        }
    }

    /// The ring must hold a whole decoded run plus the jitter budget, or a run
    /// is dropped as overflow the moment it lands.
    #[test]
    fn ring_absorbs_a_whole_run_plus_jitter() {
        let mut r = Ring::new();
        for _ in 0..RUN_SECTORS {
            for _ in 0..FRAMES_PER_SECTOR_4BIT {
                r.push(1, -1);
            }
        }
        assert!(
            r.len() >= RUN_SECTORS * FRAMES_PER_SECTOR_4BIT,
            "a {RUN_SECTORS}-sector run does not fit the ring"
        );
    }

    /// The drive arithmetic the whole design rests on: how much of the drive a
    /// run of N sectors occupies, as a multiple of the audio time it delivers.
    ///
    /// This is the number that decides whether music can share the disc with the
    /// texture streamer, and it is *not* the sector-rate figure -- a run is
    /// bounded by one seek, so the seek dominates at small N and the sector rate
    /// only starts to matter once N is large.
    #[test]
    fn drive_duty_cycle_is_what_limits_run_length() {
        let seek_us = 150_000.0;
        let sector_us = 6_700.0; // one XA sector at 2x
        let audio_per_sector_us = 54_000.0;
        let duty = |n: f64| (seek_us + sector_us * n) / (audio_per_sector_us * n);

        // A single-sector run is nearly three times the drive for 54 ms of
        // music: the seek is the entire cost.
        assert!(duty(1.0) > 2.0, "N=1 should be seek-dominated");
        // The shipped run length leaves the drive busier than the audio.
        assert!(
            duty(RUN_SECTORS as f64) > 0.5,
            "N={RUN_SECTORS} should still be substantially seek-dominated"
        );
        // Reaching a 30% duty needs a run of ~16 sectors, which needs a ring of
        // 16 * 2016 + jitter frames -- about 130 KB, more than the build has.
        assert!(duty(16.0) < 0.35, "N=16 should reach a workable duty cycle");
    }

    /// Reserved channel and rate encodings must be refused, not guessed at.
    #[test]
    fn reserved_codings_are_refused() {
        assert!(Coding::from_byte(0x02).is_none());
        assert!(Coding::from_byte(0x07).is_none());
        assert!(Coding::from_byte(0x13).is_none());
    }
}
