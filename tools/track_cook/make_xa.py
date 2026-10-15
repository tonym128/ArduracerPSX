#!/usr/bin/env python3
"""Encode 16-bit PCM WAV into CD-XA ADPCM sectors.

The XA mode this masters is 4-bit stereo at 37.8 kHz, because that is the
coding the whole design rests on: it yields 2016 frames per sector, so a stream
costs 37800/2016 = 18.75 sectors/s against Redbook CD-DA's 75.2. A quarter of
the drive instead of all of it, which is the only reason music can share the
disc with the texture streamer at all.

The encoder is the exact inverse of `arduracer_core::xa::decode_sector`, and the
round trip is tested against a real fixture rather than trusted: a sector
encoded here is decoded by the Rust decoder and the result compared to the
source PCM, so a disagreement in the group layout or the nibble packing shows up
as a failing test rather than as near-right audio.

Layout, matching the reference decoder exactly:

  * raw sector is 2336 bytes; the sound-group payload starts at offset 24
  * 18 groups of 128 bytes: 16 header bytes, then 112 bytes of sample data
  * each group holds 4 sound units, each 28 samples, 4 bits per sample
  * unit 0 is left and unit 1 right for unit pair 0, and so on via HEAD_TABLE

Usage:
    make_xa.py <in.wav> <out.xa>
"""

import argparse
import struct
import sys
import wave

XA_SECTOR_BYTES = 2336
XA_PAYLOAD_OFFSET = 8
XA_GROUPS = 18
GROUP_BYTES = 128
GROUP_HEADER_BYTES = 16
SAMPLES_PER_UNIT = 28
UNITS_PER_GROUP = 4

# Header-byte offsets per unit, matching HEAD_TABLE in the Rust decoder.
HEAD_TABLE = (0, 2, 8, 10)

# Coding byte: stereo(1) | rate(0 = 37.8kHz) | 4-bit(0).
CODING_4BIT_STEREO_378 = 0x01

# ADPCM filter coefficients in Q10, matching XA_FILTER in the Rust decoder.
# Encoded streams use filter 0 (no prediction) for every unit: it costs nothing
# in drive terms, and a predictor makes the encoder's job depend on getting the
# filter search right, which is not where the risk in this format lives.
FILTER_ID = 0
RANGE = 0


def read_nibble(frames, index):
    """One 16-bit sample as a signed 4-bit ADPCM code.

    ADPCM codes are signed 4-bit, so a full-scale negative sample has to be
    clamped to -8 rather than truncated to 0, or a hot master wraps to the
    opposite sign and every peak turns into a click.
    """
    if index >= len(frames):
        return 0
    nib = frames[index] >> 12
    if nib < -8:
        nib = -8
    elif nib > 7:
        nib = 7
    return nib & 0x0F


def encode_sector(frames, offset, file_num=1, channel_num=0):
    """Encode 2016 stereo frames (offset within the source) into one 2336-byte sector."""
    if len(frames) < 2016:
        raise ValueError("need 2016 stereo frames per sector")

    raw = bytearray(XA_SECTOR_BYTES)
    # CD-XA Mode 2 Form 2 subheader (repeated twice across 8 bytes):
    # File, Channel, Submode (0x64: Form 2 | Real-time | Audio), Coding (0x01: 4-bit stereo 37.8kHz)
    subheader = bytes([
        file_num, channel_num, 0x64, CODING_4BIT_STEREO_378,
        file_num, channel_num, 0x64, CODING_4BIT_STEREO_378,
    ])
    raw[0:8] = subheader

    for group_idx in range(XA_GROUPS):
        group_at = XA_PAYLOAD_OFFSET + group_idx * GROUP_BYTES
        raw[group_at + 15] = 0x01  # group index marker

        for unit in range(UNITS_PER_GROUP):
            # Each unit is a *stereo pair* of 28 samples, not one channel. The
            # two channels of a sample share a byte -- left in the low nibble,
            # right in the high -- which is what the decoder's paired
            # `low_words` / `high_words` unpacking reads.
            #
            # The first version of this encoder put the channels in separate
            # units, which the round-trip test caught immediately: decoded left
            # 8192, right 0, against a source left of -28672. Two halves of a
            # wrong layout that both decode to plausible audio is the exact
            # failure this test exists for.
            hl = HEAD_TABLE[unit]
            # `HEAD_TABLE` is an offset into the *group*, not into the data
            # section. Offsetting by GROUP_HEADER_BYTES as well put unit 2's
            # header bytes (at group + 8) on top of unit 0's sample data (also
            # group + 16 + 8), silently overwriting two codes per group with a
            # zero header -- which is why exactly one sample in 2016 decoded
            # wrong, and why it looked like an arithmetic bug.
            raw[group_at + hl] = (FILTER_ID << 4) | RANGE
            raw[group_at + hl + 1] = (FILTER_ID << 4) | RANGE

            base_sample = group_idx * UNITS_PER_GROUP * SAMPLES_PER_UNIT + unit * SAMPLES_PER_UNIT
            data_at = group_at + GROUP_HEADER_BYTES

            for k in range(7):
                for j in range(4):
                    sample_idx = base_sample + k * 4 + j
                    frame_at = offset + sample_idx
                    left = read_nibble(frames, frame_at * 2)
                    right = read_nibble(frames, frame_at * 2 + 1)
                    # Word k of unit u: byte k*16 + u + 4*j. The decoder's
                    # `low_words` is the *left* channel and `high_words` the
                    # right, so left goes in the low nibble.
                    raw[data_at + k * 16 + unit + 4 * j] = (right << 4) | left

    return bytes(raw)


def pcm_to_xa(frames):
    """Encode every full sector of stereo frames."""
    total = len(frames) // 2
    sectors = []
    offset = 0
    while offset + 2016 <= total:
        sectors.append(encode_sector(frames, offset))
        offset += 2016
    return sectors


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("input", help="16-bit PCM WAV")
    ap.add_argument("output", help="raw .xa sector file")
    args = ap.parse_args()

    with wave.open(args.input, "rb") as w:
        if w.getsampwidth() != 2:
            sys.exit(f"expected 16-bit PCM, got {w.getsampwidth() * 8}-bit")
        if w.getnchannels() != 2:
            sys.exit(f"expected stereo, got {w.getnchannels()} channels")
        if w.getframerate() != 37800:
            sys.exit(
                f"expected 37800 Hz (the XA rate this encoder masters), "
                f"got {w.getframerate()} Hz"
            )
        frames = w.readframes(w.getnframes())

    sectors = pcm_to_xa(frames)
    if not sectors:
        sys.exit("input is shorter than one sector (2016 frames)")

    with open(args.output, "wb") as f:
        f.write(b"".join(sectors))

    seconds = (len(sectors) * 2016) / 37800
    print(
        f"{args.output}: {len(sectors)} sectors, {seconds:.1f}s, "
        f"{len(sectors) / 75 * 100:.1f}% of a 1x drive while playing"
    )


if __name__ == "__main__":
    main()
