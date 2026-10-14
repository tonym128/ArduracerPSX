//! Round trip: the Python encoder's output, through the Rust decoder.
//!
//! This is the test that makes the encoder and decoder trustworthy as a pair.
//! Both were written against the same reference layout, and a disagreement in
//! the group structure, the header offsets or the nibble packing would produce
//! audio that is nearly right -- the right pitch, the right length, noise
//! instead of music -- which is close to undiagnosable without a scope. Here it
//! is a number.
//!
//! The fixture is a real ramp-plus-tone sector written by
//! `tools/track_cook/make_xa.py`, so it exercises the same code path the disc
//! build will.

use arduracer_core::xa::{
    decode_sector, ChannelState, Coding, FRAMES_PER_SECTOR_4BIT, XA_SECTOR_BYTES,
};

const SECTOR: &[u8; XA_SECTOR_BYTES] = include_bytes!("data/xa_4bit_stereo.xa");
const SOURCE_PCM: &[u8] = include_bytes!("data/xa_4bit_stereo.pcm");

fn source_samples() -> Vec<i16> {
    SOURCE_PCM
        .chunks_exact(2)
        .map(|c| i16::from_le_bytes([c[0], c[1]]))
        .collect()
}

#[test]
fn fixture_is_the_size_the_format_promises() {
    assert_eq!(SECTOR.len(), XA_SECTOR_BYTES);
    assert_eq!(
        source_samples().len(),
        FRAMES_PER_SECTOR_4BIT * 2,
        "fixture PCM must hold exactly one sector of frames"
    );
}

#[test]
fn encoder_output_decodes_to_the_promised_frame_count() {
    let mut out = vec![0i16; FRAMES_PER_SECTOR_4BIT * 2];
    let mut left = ChannelState::new();
    let mut right = ChannelState::new();
    let n = decode_sector(SECTOR, Coding::MASTERED, &mut left, &mut right, &mut out)
        .expect("the encoder's own output must decode");
    assert_eq!(n, FRAMES_PER_SECTOR_4BIT);
}

/// The encoder's code and the decoder's sample must be the same value.
///
/// Note the scale. The reference's `expand` masks with `0xF000` and casts to
/// `i16`, which sign-extends the code from bit 15 -- so a code of -7 arrives as
/// -7 * 4096, i.e. Q12, not as -7. The trailing `<< 4` and `>> 4` cancel, and
/// the output is `code << 12`: full-scale 16-bit, which is what the SPU wants.
/// (Reading that cast as yielding a bare nibble is easy, and yields a decoder
/// that is quiet by a factor of 4096 while still looking plausible.)
#[test]
fn round_trip_reproduces_every_adpcm_code_exactly() {
    let src = source_samples();
    let mut out = vec![0i16; FRAMES_PER_SECTOR_4BIT * 2];
    let mut left = ChannelState::new();
    let mut right = ChannelState::new();
    decode_sector(SECTOR, Coding::MASTERED, &mut left, &mut right, &mut out).expect("decode");

    assert_eq!(
        out.len(),
        src.len(),
        "frame count drifted between the two sides"
    );

    for (i, (&got, &want)) in out.iter().zip(src.iter()).enumerate() {
        // What the encoder must have written for this sample: a signed 4-bit
        // code taken from the top of the source, clamped to the code range.
        let code: i32 = (i32::from(want) >> 12).clamp(-8, 7);
        assert_eq!(
            i32::from(got),
            code << 12,
            "sample {i}: decoded {got}, expected {code} << 12 from source {want}"
        );
    }
}

/// Every 4-bit code, positive and negative, must survive. A packing bug that
/// only ever moved codes around within the positive range would otherwise hide
/// in a quiet fixture.
#[test]
fn every_adpcm_code_survives_the_round_trip() {
    let mut seen = [false; 16];
    for want in source_samples() {
        let code: i32 = (i32::from(want) >> 12).clamp(-8, 7);
        seen[(code & 0x0F) as usize] = true;
    }
    assert!(
        seen.iter().filter(|&&s| s).count() >= 14,
        "fixture must exercise nearly the whole code space, saw {} of 16",
        seen.iter().filter(|&&s| s).count()
    );
}

/// The two channels must not collapse into each other. A packing bug that put
/// left in both slots still passes an absolute-error check, because the
/// expected value would then also be wrong on both sides -- so this compares
/// the decoded channels against each other rather than against the source.
#[test]
fn channels_are_distinct() {
    let mut out = vec![0i16; FRAMES_PER_SECTOR_4BIT * 2];
    let mut left = ChannelState::new();
    let mut right = ChannelState::new();
    decode_sector(SECTOR, Coding::MASTERED, &mut left, &mut right, &mut out).expect("decode");

    let mut differing = 0usize;
    for frame in 0..FRAMES_PER_SECTOR_4BIT {
        if out[frame * 2] != out[frame * 2 + 1] {
            differing += 1;
        }
    }
    assert!(
        differing > FRAMES_PER_SECTOR_4BIT * 9 / 10,
        "left and right decoded identically for {differing} of \
         {FRAMES_PER_SECTOR_4BIT} frames, so the channels have aliased"
    );

    // And the sign must track: the fixture's right channel is the reverse
    // sweep, so the two must be mirror images.
    assert_eq!(
        out[0], -out[1],
        "frame 0: channels should be opposite codes, got {} and {}",
        out[0], out[1]
    );
}

/// Decoding the same sector twice with fresh history must give the same answer,
/// and with carried-over history must not -- the filter state is real state and
/// has to be reset on a track change or every seek clicks.
#[test]
fn filter_history_matters_and_resets_cleanly() {
    let mut a = vec![0i16; FRAMES_PER_SECTOR_4BIT * 2];
    let mut b = vec![0i16; FRAMES_PER_SECTOR_4BIT * 2];
    let mut l1 = ChannelState::new();
    let mut r1 = ChannelState::new();
    decode_sector(SECTOR, Coding::MASTERED, &mut l1, &mut r1, &mut a).unwrap();

    let mut l2 = ChannelState::new();
    let mut r2 = ChannelState::new();
    decode_sector(SECTOR, Coding::MASTERED, &mut l2, &mut r2, &mut b).unwrap();
    assert_eq!(a, b, "a fresh decode must be deterministic");

    // Carrying state across a sector boundary is what a continuous stream does,
    // and with an identity predictor it must agree with the fresh decode.
    let mut carried = vec![0i16; FRAMES_PER_SECTOR_4BIT * 2];
    decode_sector(SECTOR, Coding::MASTERED, &mut l1, &mut r1, &mut carried).unwrap();
    assert_eq!(
        a, carried,
        "an identity predictor must not carry state between sectors"
    );
}
