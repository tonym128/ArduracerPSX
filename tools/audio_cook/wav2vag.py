#!/usr/bin/env python3
"""
PSX VAG Audio Cooker for Arduracer PSX.

Converts standard mono 16-bit PCM WAV audio files into Sony PlayStation 1
VAG / ADPCM audio format (16-byte sound blocks with 4-bit compressed samples).
Also provides synthetic procedural sound generators for racing sound effects
(engine revs, tire screeches, wall impacts, and turbo whoosh).
"""

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

def export_rust_soundbank(out_rs_path):
    """Encodes all standard sound effects and writes a Rust module."""
    sfx_defs = [
        ("ENGINE_LOOP", generate_engine_tone(22050, 0.1, 110.0), 22050, True),
        ("TIRE_SCREECH", generate_skid_tone(22050, 0.25), 22050, True),
        ("CRASH_IMPACT", generate_crash_tone(22050, 0.25), 22050, False),
        ("BOOST_WHOOSH", generate_boost_whoosh(22050, 0.35), 22050, False),
        ("CHECKPOINT_CHIME", generate_checkpoint_chime(22050, 0.2), 22050, False),
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

def generate_cdda_synth(track_id, duration_sec=4.0, sample_rate=44100):
    """Generates a stereo 44.1kHz 16-bit arcade soundtrack pattern."""
    num_samples = int(duration_sec * sample_rate)
    # Ensure exact alignment to 588 stereo samples (2352 bytes sector)
    remainder = num_samples % 588
    if remainder != 0:
        num_samples += (588 - remainder)

    pcm_bytes = bytearray()
    
    # Base musical parameters by track
    params = {
        2: {"tempo": 138.0, "root": 220.0, "scale": [0, 3, 7, 10, 12]},       # Title: Minor Pentatonic
        3: {"tempo": 145.0, "root": 261.63, "scale": [0, 4, 7, 9, 12]},       # Circuit: Major Driving
        4: {"tempo": 120.0, "root": 196.0, "scale": [0, 2, 4, 7, 9, 12]},     # Coastal: Synthwave
        5: {"tempo": 150.0, "root": 146.83, "scale": [0, 3, 5, 6, 7, 10, 12]},# Cyber: Acid/Techno
        6: {"tempo": 140.0, "root": 164.81, "scale": [0, 3, 7, 8, 12]},       # Canyon: Phrygian
        7: {"tempo": 130.0, "root": 261.63, "scale": [0, 4, 7, 11, 12, 16]},  # Victory: Triumphant Major
    }
    cfg = params.get(track_id, params[2])
    tempo = cfg["tempo"]
    root = cfg["root"]
    scale = cfg["scale"]
    beat_sec = 60.0 / tempo
    sixteenth = beat_sec / 4.0

    for i in range(num_samples):
        t = i / sample_rate
        beat_idx = int(t / beat_sec)
        sub_idx = int(t / sixteenth) % len(scale)
        sub_t = (t % sixteenth) / sixteenth

        # Lead melody note
        semitone = scale[sub_idx]
        lead_freq = root * (2.0 ** (semitone / 12.0))
        lead_env = math.exp(-6.0 * sub_t)
        lead_val = (math.sin(2 * math.pi * lead_freq * t) + 0.3 * math.sin(4 * math.pi * lead_freq * t)) * lead_env

        # Bass octave
        bass_freq = (root / 2.0)
        bass_env = math.exp(-3.0 * ((t % beat_sec) / beat_sec))
        bass_val = math.sin(2 * math.pi * bass_freq * t) * bass_env

        # Rhythm kick on beat
        kick_t = (t % beat_sec)
        kick_env = math.exp(-24.0 * kick_t)
        kick_freq = 150.0 * math.exp(-30.0 * kick_t) + 45.0
        kick_val = math.sin(2 * math.pi * kick_freq * kick_t) * kick_env

        # Stereo mix
        left = int(max(-32767, min(32767, (lead_val * 0.45 + bass_val * 0.35 + kick_val * 0.4) * 22000)))
        right = int(max(-32767, min(32767, (lead_val * 0.40 + bass_val * 0.40 + kick_val * 0.4) * 22000)))

        pcm_bytes.extend(struct.pack("<hh", left, right))

    return bytes(pcm_bytes)

def cook_cdda_tracks(out_dir):
    """Synthesizes all 6 Redbook CD-DA audio tracks for Tracks 2 through 7."""
    os.makedirs(out_dir, exist_ok=True)
    track_names = [
        (2, "track02_title.raw"),
        (3, "track03_circuit.raw"),
        (4, "track04_coastal.raw"),
        (5, "track05_cyber.raw"),
        (6, "track06_canyon.raw"),
        (7, "track07_victory.raw"),
    ]
    for tid, fname in track_names:
        path = os.path.join(out_dir, fname)
        pcm = generate_cdda_synth(tid, duration_sec=4.0)
        with open(path, "wb") as f:
            f.write(pcm)
        sectors = len(pcm) // 2352
        print(f"Mastered CD-DA Track {tid:02d} -> {path} ({len(pcm)} bytes, {sectors} sectors)")

def main():
    if len(sys.argv) > 1 and sys.argv[1] == "--cook-soundbank":
        out_path = sys.argv[2] if len(sys.argv) > 2 else "game/src/audio/soundbank.rs"
        export_rust_soundbank(out_path)
    elif len(sys.argv) > 1 and sys.argv[1] == "--cook-cdda":
        out_dir = sys.argv[2] if len(sys.argv) > 2 else "assets/cdda"
        cook_cdda_tracks(out_dir)
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
        print("Usage: wav2vag.py <input.wav> <output.vag> [--loop] | --cook-soundbank [out.rs] | --cook-cdda [out_dir]")

if __name__ == "__main__":
    main()
