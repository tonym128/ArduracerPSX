#!/usr/bin/env python3
"""
PSX VAG Audio Cooker for Arduracer PSX.

Converts standard mono 16-bit PCM WAV audio files into Sony PlayStation 1
VAG / ADPCM audio format (16-byte sound blocks with 4-bit compressed samples).
Also provides synthetic procedural sound generators for racing sound effects
(engine revs, tire screeches, wall impacts, and turbo whoosh).
"""

import subprocess
import sys
import os
import struct
import math
import wave

# Standard PSX ADPCM predictor filter coefficients (Q.6 format)
PREDICTORS = [
    (0.0, 0.0),
    (60.0 / 64.0, 0.0),
    (115.0 / 64.0, -52.0 / 64.0),
    (98.0 / 64.0, -55.0 / 64.0),
    (122.0 / 64.0, -60.0 / 64.0),
]

def encode_adpcm_block(samples_28, prev1, prev2, is_loop_start=False, is_loop_end=False):
    """
    Encodes 28 16-bit signed PCM samples into one 16-byte PSX ADPCM block.
    Finds the optimal predictor and shift factor minimizing mean-square error.
    """
    best_filter = 0
    best_shift = 0
    best_error = float('inf')
    best_nibbles = [0] * 28
    best_p1 = prev1
    best_p2 = prev2

    for f_idx, (k0, k1) in enumerate(PREDICTORS):
        for shift in range(13):
            scale = 1 << shift
            p1 = prev1
            p2 = prev2
            error = 0.0
            nibbles = []
            valid = True

            for s in samples_28:
                predicted = p1 * k0 + p2 * k1
                diff = s - predicted
                # Quantize to 4-bit signed integer (-8 to +7)
                q = int(round(diff / scale))
                if q < -8:
                    q = -8
                elif q > 7:
                    q = 7
                nibbles.append(q & 0x0F)

                # Reconstruct sample through SPU decoder model
                recon = predicted + (q * scale)
                # Clamp to 16-bit signed
                recon = max(-32768, min(32767, recon))
                p2 = p1
                p1 = recon

                diff_recon = s - recon
                error += diff_recon * diff_recon

            if error < best_error:
                best_error = error
                best_filter = f_idx
                best_shift = shift
                best_nibbles = nibbles
                best_p1 = p1
                best_p2 = p2

    # Assemble 16-byte block
    # Byte 0: Predictor (high nibble) and Shift (low nibble)
    b0 = ((best_filter & 0x0F) << 4) | (best_shift & 0x0F)
    
    # Byte 1: Flags
    # 0x00 = standard block
    # 0x04 = loop start
    # 0x03 = loop end + repeat (continuous loop)
    # 0x01 = loop end without repeat (one-shot)
    flags = 0
    if is_loop_start:
        flags |= 0x04
    if is_loop_end:
        flags |= 0x03  # Loop repeat
        
    data = bytearray(16)
    data[0] = b0
    data[1] = flags

    # Pack 28 nibbles into 14 bytes (low nibble first, then high nibble)
    for i in range(14):
        low_nibble = best_nibbles[i * 2] & 0x0F
        high_nibble = best_nibbles[i * 2 + 1] & 0x0F
        data[2 + i] = (high_nibble << 4) | low_nibble

    return bytes(data), best_p1, best_p2

def encode_pcm_to_vag(pcm_samples, sample_rate=22050, loop=False, name="SOUND"):
    """Encodes a list of 16-bit signed PCM samples into VAG ADPCM bytes."""
    # Ensure sample length is multiple of 28
    remainder = len(pcm_samples) % 28
    if remainder != 0:
        pcm_samples = list(pcm_samples) + [0] * (28 - remainder)

    blocks = []
    prev1 = 0.0
    prev2 = 0.0
    num_blocks = len(pcm_samples) // 28

    for i in range(num_blocks):
        block_samples = pcm_samples[i * 28 : (i + 1) * 28]
        is_start = (i == 0 and loop)
        is_end = (i == num_blocks - 1)
        blk_bytes, prev1, prev2 = encode_adpcm_block(block_samples, prev1, prev2, is_start, is_end)
        blocks.append(blk_bytes)

    # If one-shot and loop is False, append a terminal silence block with flag 0x01
    if not loop:
        silence_blk = bytearray(16)
        silence_blk[1] = 0x01 # 1-shot END flag
        blocks.append(bytes(silence_blk))

    adpcm_data = b"".join(blocks)

    # 48-byte VAG Header
    header = bytearray(48)
    header[0:4] = b"VAGp"
    struct.pack_into(">I", header, 4, 0x00000020) # Version 0x20
    struct.pack_into(">I", header, 8, 0)          # Reserved
    struct.pack_into(">I", header, 12, len(adpcm_data)) # Data size
    struct.pack_into(">I", header, 16, sample_rate)     # Sample rate
    name_bytes = name.encode('ascii')[:15]
    header[32 : 32 + len(name_bytes)] = name_bytes

    return bytes(header) + adpcm_data

def generate_engine_tone(sample_rate=22050, duration_sec=0.1, base_hz=110.0):
    """Generates a rich, resonant racing engine cylinder pulse waveform."""
    num_samples = int(sample_rate * duration_sec)
    samples = []
    for i in range(num_samples):
        t = i / sample_rate
        # Fundamental sawtooth-like harmonics
        val = 0.5 * math.sin(2 * math.pi * base_hz * t)
        val += 0.3 * math.sin(2 * math.pi * base_hz * 2 * t)
        val += 0.15 * math.sin(2 * math.pi * base_hz * 3 * t)
        val += 0.08 * math.sin(2 * math.pi * base_hz * 4 * t)
        val += 0.05 * math.sin(2 * math.pi * base_hz * 6 * t)
        # Soft clipping
        val = max(-1.0, min(1.0, val * 1.4))
        samples.append(int(val * 24000))
    return samples

def generate_skid_tone(sample_rate=22050, duration_sec=0.3):
    """Generates high-frequency friction screech for tire sliding on tarmac."""
    num_samples = int(sample_rate * duration_sec)
    samples = []
    # Band-limited frequency modulated noise
    lfsr = 0xACE1
    for i in range(num_samples):
        t = i / sample_rate
        lfsr = ((lfsr >> 1) ^ (-(lfsr & 1) & 0xB400)) & 0xFFFF
        noise = (lfsr / 32768.0) - 1.0
        # Chirp and resonance at ~2.5kHz and ~3.8kHz
        fm = math.sin(2 * math.pi * 80 * t)
        fric = math.sin(2 * math.pi * (2600 + fm * 400) * t)
        fric2 = math.sin(2 * math.pi * 3700 * t)
        val = (noise * 0.4) + (fric * 0.4) + (fric2 * 0.2)
        samples.append(int(val * 20000))
    return samples

def generate_crash_tone(sample_rate=22050, duration_sec=0.25):
    """Generates a heavy metallic barrier impact thump with crunchy noise decay."""
    num_samples = int(sample_rate * duration_sec)
    samples = []
    lfsr = 0x5423
    for i in range(num_samples):
        t = i / sample_rate
        lfsr = ((lfsr >> 1) ^ (-(lfsr & 1) & 0xB400)) & 0xFFFF
        noise = (lfsr / 32768.0) - 1.0
        decay = math.exp(-12.0 * t)
        low_thump = math.sin(2 * math.pi * (180.0 - 120.0 * t) * t)
        metal_ring = math.sin(2 * math.pi * 880.0 * t) * 0.4
        val = ((low_thump * 0.6) + (noise * 0.3) + metal_ring) * decay
        samples.append(int(max(-1.0, min(1.0, val)) * 30000))
    return samples

def generate_boost_whoosh(sample_rate=22050, duration_sec=0.4):
    """Generates an energetic high-speed turbo boost whoosh sound."""
    num_samples = int(sample_rate * duration_sec)
    samples = []
    lfsr = 0x1234
    for i in range(num_samples):
        t = i / sample_rate
        lfsr = ((lfsr >> 1) ^ (-(lfsr & 1) & 0xB400)) & 0xFFFF
        noise = (lfsr / 32768.0) - 1.0
        # Rising resonant sweep from 600 Hz to 2400 Hz
        freq = 600.0 + 1800.0 * (t / duration_sec)
        sweep = math.sin(2 * math.pi * freq * t)
        env = math.sin(math.pi * (t / duration_sec))
        val = (noise * 0.5 + sweep * 0.5) * env
        samples.append(int(val * 26000))
    return samples

def generate_checkpoint_chime(sample_rate=22050, duration_sec=0.2):
    """Generates a crisp high-pitched checkpoint completion chime."""
    num_samples = int(sample_rate * duration_sec)
    samples = []
    for i in range(num_samples):
        t = i / sample_rate
        env = math.exp(-16.0 * t)
        chime1 = math.sin(2 * math.pi * 1760.0 * t) # A6
        chime2 = math.sin(2 * math.pi * 2637.0 * t) # E7
        val = (chime1 * 0.6 + chime2 * 0.4) * env
        samples.append(int(val * 25000))
    return samples

def generate_ui_move_tone(sample_rate=22050, duration_sec=0.08):
    """Generates a short, dry, high-pitched blip for menu navigation.

    Menu audio has to sit under CD-DA music without competing with it, so this
    is deliberately quiet and percussive rather than tonal or ringing: a fast
    attack, an immediate exponential decay, and no low end to muddy the mix.
    """
    num_samples = int(sample_rate * duration_sec)
    samples = []
    for i in range(num_samples):
        t = i / sample_rate
        # Very fast decay so the blip cannot overlap the next one while a
        # button is being tapped repeatedly.
        env = math.exp(-55.0 * t)
        blip = math.sin(2 * math.pi * 1400.0 * t)
        overtone = math.sin(2 * math.pi * 2100.0 * t) * 0.3
        samples.append(int(max(-1.0, min(1.0, blip + overtone)) * env * 16000))
    return samples

def export_rust_soundbank(out_rs_path):
    """Encodes all standard sound effects and writes a Rust module."""
    sfx_defs = [
        ("ENGINE_LOOP", generate_engine_tone(22050, 0.1, 110.0), 22050, True),
        ("TIRE_SCREECH", generate_skid_tone(22050, 0.25), 22050, True),
        ("CRASH_IMPACT", generate_crash_tone(22050, 0.25), 22050, False),
        ("BOOST_WHOOSH", generate_boost_whoosh(22050, 0.35), 22050, False),
        ("CHECKPOINT_CHIME", generate_checkpoint_chime(22050, 0.2), 22050, False),
        ("UI_MOVE", generate_ui_move_tone(22050, 0.08), 22050, False),
    ]

    with open(out_rs_path, "w") as f:
        f.write("//! Pre-cooked VAG ADPCM soundbank for Arduracer PSX.\n")
        f.write("//! Generated by tools/audio_cook/wav2vag.py.\n\n")
        f.write("#![allow(dead_code)]\n\n")

        for name, pcm, rate, loop_flag in sfx_defs:
            vag_bytes = encode_pcm_to_vag(pcm, sample_rate=rate, loop=loop_flag, name=name)
            # We strip the 48-byte VAG header for raw SPU ADPCM upload
            adpcm_payload = vag_bytes[48:]
            f.write(f"/// {name} ADPCM audio block ({len(adpcm_payload)} bytes, {rate} Hz, loop={loop_flag}).\n")
            f.write(f"pub const {name}_RATE: u32 = {rate};\n")
            f.write(f"pub const {name}_LOOP: bool = {str(loop_flag).lower()};\n")
            f.write(f"pub const {name}: [u8; {len(adpcm_payload)}] = [\n    ")
            for idx, b in enumerate(adpcm_payload):
                f.write(f"0x{b:02x}, ")
                if (idx + 1) % 16 == 0:
                    f.write("\n    ")
            f.write("];\n\n")

    print(f"Generated Rust soundbank -> {out_rs_path}")

# CD-DA mastering. Red Book audio is 44.1 kHz, 16-bit, stereo, interleaved
# left-then-right, and the drive addresses it in 2352-byte sectors -- exactly
# 588 stereo frames per sector. Every track is padded with silence to a whole
# number of sectors so mkisopsx never has to truncate a partial sector.
CDDA_SAMPLE_RATE = 44100
CDDA_BYTES_PER_FRAME = 4  # 2 channels * 16 bits
CDDA_FRAMES_PER_SECTOR = 2352 // CDDA_BYTES_PER_FRAME
CDDA_CHANNELS = 2

# GAME.md section 6.1 track roles -> the AssetSource master that fills them.
# Kept as an explicit table so the disc layout stays auditable: every source is
# used exactly once and no role is left to chance.
CDDA_TRACKS = [
    (2, "track02_title.raw", "Asphalt_Overdrive.mp3", "Title / Menu - Neon Overdrive (synthwave)"),
    (3, "track03_circuit.raw", "Asphalt_Adrenaline.mp3", "Cup 1 - Asphalt Adrenaline (Eurobeat)"),
    (4, "track04_coastal.raw", "Asphalt_Pursuit.mp3", "Cup 2 - Night Drift City (D&B)"),
    (5, "track05_cyber.raw", "Burn_the_Asphalt.mp3", "Cup 3 - Canyon Rush (arcade techno)"),
    (6, "track06_canyon.raw", "Maximum_Throttle.mp3", "Cup 4 - Apex Predator (hard trance)"),
    (7, "track07_victory.raw", "The_Victory_Lap.mp3", "Victory / Podium Fanfare"),
]


def decode_to_cdda_pcm(mp3_path):
    """Decodes an MP3 to sector-ready CD-DA PCM via ffmpeg.

    ffmpeg does the resampling and stereo downmix so the master keeps its
    original length; we only normalise the container to what the drive needs.
    """
    cmd = [
        "ffmpeg", "-v", "error", "-nostdin", "-i", mp3_path,
        "-f", "s16le", "-acodec", "pcm_s16le",
        "-ar", str(CDDA_SAMPLE_RATE), "-ac", str(CDDA_CHANNELS),
        "-",
    ]
    try:
        out = subprocess.run(cmd, check=True, stdout=subprocess.PIPE).stdout
    except FileNotFoundError:
        raise SystemExit(
            "error: ffmpeg is required to master CD-DA tracks from AssetSource/*.mp3"
        )
    except subprocess.CalledProcessError as exc:
        raise SystemExit(f"error: ffmpeg failed on {mp3_path} (exit {exc.returncode})")

    # Drop a partial trailing frame so the padding arithmetic stays whole.
    whole = len(out) - (len(out) % CDDA_BYTES_PER_FRAME)
    out = out[:whole]

    # Pad with silence up to the next sector boundary.
    rem_frames = (len(out) // CDDA_BYTES_PER_FRAME) % CDDA_FRAMES_PER_SECTOR
    if rem_frames:
        pad = (CDDA_FRAMES_PER_SECTOR - rem_frames) * CDDA_BYTES_PER_FRAME
        out += b"\x00" * pad
    return out


def cook_cdda_tracks(out_dir, source_dir):
    """Masters the 6 Red Book CD-DA audio tracks for disc tracks 2 through 7."""
    os.makedirs(out_dir, exist_ok=True)
    for tid, fname, source_name, role in CDDA_TRACKS:
        mp3 = os.path.join(source_dir, source_name)
        if not os.path.exists(mp3):
            raise SystemExit(f"error: missing CD-DA source {mp3}")
        pcm = decode_to_cdda_pcm(mp3)
        path = os.path.join(out_dir, fname)
        with open(path, "wb") as f:
            f.write(pcm)
        sectors = len(pcm) // 2352
        seconds = sectors * CDDA_FRAMES_PER_SECTOR / CDDA_SAMPLE_RATE
        print(
            f"Mastered CD-DA Track {tid:02d} -> {path} "
            f"({source_name}, {role}: {len(pcm)} bytes, {sectors} sectors, {seconds:.1f}s)"
        )


def main():
    if len(sys.argv) > 1 and sys.argv[1] == "--cook-soundbank":
        out_path = sys.argv[2] if len(sys.argv) > 2 else "game/src/audio/soundbank.rs"
        export_rust_soundbank(out_path)
    elif len(sys.argv) > 1 and sys.argv[1] == "--cook-cdda":
        out_dir = sys.argv[2] if len(sys.argv) > 2 else "assets/cdda"
        source_dir = sys.argv[3] if len(sys.argv) > 3 else "AssetSource"
        cook_cdda_tracks(out_dir, source_dir)
    elif len(sys.argv) > 2:
        in_wav = sys.argv[1]
        out_vag = sys.argv[2]
        loop = "--loop" in sys.argv
        with wave.open(in_wav, "rb") as w:
            rate = w.getframerate()
            n_frames = w.getnframes()
            raw_data = w.readframes(n_frames)
            samples = struct.unpack(f"<{n_frames}h", raw_data)
        vag_data = encode_pcm_to_vag(samples, sample_rate=rate, loop=loop, name=os.path.basename(out_vag))
        with open(out_vag, "wb") as f:
            f.write(vag_data)
        print(f"Wrote {out_vag} ({len(vag_data)} bytes)")
    else:
        print("Usage: wav2vag.py <input.wav> <output.vag> [--loop] | --cook-soundbank [out.rs] | --cook-cdda [out_dir] [source_dir]")

if __name__ == "__main__":
    main()
