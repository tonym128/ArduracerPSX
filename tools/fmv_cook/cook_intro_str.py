#!/usr/bin/env python3
"""
Arduracer PSX - MDEC Full-Motion Video (STR) Cooker.

Generates 320x240 @ 15 fps Version-2 PlayStation STR video streams for the
game's attract intro cinematics, fully adhering to PSX MDEC hardware specifications.
Encodes from source MP4 video assets when available, with graceful procedural fallback.
"""

import math
import os
import shutil
import struct
import subprocess
import sys
import tempfile

# 1x CD speed (75 sectors/s) / 15 fps = exactly 5 sectors per frame.
SECTORS_PER_FRAME = 5


def find_repo_root():
    """Locates the repository root directory."""
    d = os.path.dirname(os.path.abspath(__file__))
    while d and d != os.path.dirname(d):
        if os.path.exists(os.path.join(d, "Makefile")) and os.path.exists(
            os.path.join(d, "game")
        ):
            return d
        d = os.path.dirname(d)
    return os.getcwd()


def find_psxavenc():
    """Locates psxavenc executable on host system."""
    path = shutil.which("psxavenc")
    if path:
        return path
    candidates = [
        os.path.expanduser("~/.local/bin/psxavenc"),
        os.path.expanduser("~/.cargo/bin/psxavenc"),
        "/usr/local/bin/psxavenc",
        "/usr/bin/psxavenc",
    ]
    for c in candidates:
        if os.path.isfile(c) and os.access(c, os.X_OK):
            return c
    return None


class BitWriter:
    def __init__(self):
        self.words = []
        self.current_word = 0
        self.bits_left = 16

    def write(self, val, num_bits):
        val &= (1 << num_bits) - 1
        while num_bits > 0:
            take = min(num_bits, self.bits_left)
            shift = num_bits - take
            chunk = (val >> shift) & ((1 << take) - 1)
            self.current_word |= chunk << (self.bits_left - take)
            self.bits_left -= take
            num_bits -= take
            if self.bits_left == 0:
                self.words.append(self.current_word)
                self.current_word = 0
                self.bits_left = 16

    def finish(self):
        if self.bits_left < 16:
            self.words.append(self.current_word)
            self.current_word = 0
            self.bits_left = 16
        res = bytearray()
        for w in self.words:
            res.extend(struct.pack("<H", w))
        return bytes(res)


def rgb_to_mdec_dc(r, g, b):
    """Converts RGB (0..255) to 10-bit signed MDEC DC values for Cr, Cb, Y."""
    y = 0.299 * r + 0.587 * g + 0.114 * b
    cb = -0.1687 * r - 0.3313 * g + 0.5 * b
    cr = 0.5 * r - 0.4187 * g - 0.0813 * b

    dc_y = max(-512, min(511, int((y - 128.0) * 3.8))) & 0x3FF
    dc_cb = max(-512, min(511, int(cb * 3.8))) & 0x3FF
    dc_cr = max(-512, min(511, int(cr * 3.8))) & 0x3FF
    return dc_cr, dc_cb, dc_y


def render_frame_pixels(frame_idx, total_frames=60):
    """Renders 20x15 macroblock RGB grid for procedural fallback intro."""
    grid = [[(0, 0, 0) for _ in range(20)] for _ in range(15)]

    if frame_idx < 15:
        intensity = frame_idx / 15.0
    elif frame_idx > 50:
        intensity = (total_frames - frame_idx) / 10.0
    else:
        intensity = 1.0

    for row in range(15):
        for col in range(20):
            bg_b = int(40 + (row * 6))
            bg_r = int(10 + (col * 2))
            bg_g = int(15 + (row * 2))

            is_stripe = row == 5 or row == 9
            is_center_banner = row in (6, 7, 8) and 3 <= col <= 16
            is_pulse = math.sin(frame_idx * 0.4 + col * 0.3) > 0.0

            if is_stripe:
                r, g, b = (220, 30, 45)
            elif is_center_banner:
                if is_pulse:
                    r, g, b = (255, 230, 20)
                else:
                    r, g, b = (255, 255, 255)
            else:
                r, g, b = (bg_r, bg_g, bg_b)

            grid[row][col] = (
                int(r * intensity),
                int(g * intensity),
                int(b * intensity),
            )

    return grid


def encode_str_frame(frame_num, grid):
    """Encodes a 20x15 macroblock grid into a multi-sector STR frame."""
    bw = BitWriter()
    for col in range(20):
        for row in range(15):
            r, g, b = grid[row][col]
            dc_cr, dc_cb, dc_y = rgb_to_mdec_dc(r, g, b)
            # 6 blocks: Cr, Cb, Y0, Y1, Y2, Y3
            bw.write(dc_cr, 10)
            bw.write(0b10, 2)  # EOB
            bw.write(dc_cb, 10)
            bw.write(0b10, 2)
            for _ in range(4):
                bw.write(dc_y, 10)
                bw.write(0b10, 2)

    # V2 End-of-frame marker
    bw.write(0x1FF, 10)
    body = bw.finish()

    announced_mdec_words = 1824
    bs_header = struct.pack("<HHHH", announced_mdec_words, 0x3800, 1, 2)
    bs_data = bs_header + body

    chunk_size = 2016
    chunks = [
        bs_data[i : i + chunk_size] for i in range(0, len(bs_data), chunk_size)
    ]
    if len(chunks) > SECTORS_PER_FRAME:
        raise ValueError(
            f"frame {frame_num} needs {len(chunks)} sectors (> {SECTORS_PER_FRAME})"
        )
    while len(chunks) < SECTORS_PER_FRAME:
        chunks.append(b"")
    num_chunks = len(chunks)

    sectors = bytearray()
    for idx, c in enumerate(chunks):
        hdr = struct.pack(
            "<HHHHIIHH",
            0x0160,
            0x8001,
            idx,
            num_chunks,
            frame_num,
            len(bs_data),
            320,
            240,
        )
        hdr += bs_header
        hdr += b"\x00\x00\x00\x00"
        payload = c.ljust(2016, b"\x00")
        sectors += hdr + payload

    return bytes(sectors)


def cook_procedural_fallback(str_out, num_frames=60):
    """Fallback generator producing a procedural 60-frame attract loop."""
    os.makedirs(os.path.dirname(os.path.abspath(str_out)), exist_ok=True)
    with open(str_out, "wb") as f:
        for fr in range(1, num_frames + 1):
            grid = render_frame_pixels(fr, num_frames)
            sectors = encode_str_frame(fr, grid)
            f.write(sectors)
    size = os.path.getsize(str_out)
    print(
        f"Mastered Procedural FMV -> {str_out} ({size} bytes, {num_frames} frames @ 15 fps)"
    )


def cook_mp4(mp4_path, str_out, adpcm_out=None):
    """Cooks intro STR and SPU ADPCM from source MP4 video using psxavenc."""
    psxavenc = find_psxavenc()
    if not psxavenc:
        print(
            "psxavenc not found; falling back to procedural FMV generator. The "
            "cooked FMV will be a silent placeholder, not the real cinematic.",
            file=sys.stderr,
        )
        # This used to degrade silently, and did: CI shipped a placeholder intro
        # because the runner had no psxavenc. Annotate so it is impossible to
        # miss, and so a build that wanted the real FMV can fail on it.
        if os.environ.get("GITHUB_ACTIONS"):
            print(
                "::warning title=FMV encoder missing::psxavenc was not found, so "
                "the intro FMV was replaced by a silent procedural placeholder. "
                "Install psxavenc (see the workflow's 'Install psxavenc' step) to "
                "ship the real cinematic.",
                file=sys.stderr,
            )
        if os.environ.get("ARTHURACER_REQUIRE_FMV_ENCODER"):
            raise SystemExit(
                "psxavenc is required (ARTHURACER_REQUIRE_FMV_ENCODER is set) "
                "but was not found."
            )
        return False

    os.makedirs(os.path.dirname(os.path.abspath(str_out)), exist_ok=True)

    # 1. Encode 320x240 @ 15 fps Version-2 STR video stream at 1x speed (5 sectors/frame)
    # The source is 16:9 widescreen (1280x720) with fine film grain. Unfiltered high-frequency
    # AC coefficients exhaust the 1x MDEC sector budget, causing macroblock desynchronization
    # and chromatic corruption (neon green horizontal bands). Preprocess with spatial/temporal
    # denoising (hqdn3d) and clean letterbox scaling (320x176 padded to 320x240) before psxavenc.
    print(f"Encoding {mp4_path} -> {str_out} with psxavenc...")
    with tempfile.NamedTemporaryFile(suffix=".mkv", delete=False) as tmp_mkv:
        tmp_mkv_path = tmp_mkv.name
    try:
        vf_filter = "hqdn3d=2.0:2.0:3.0:3.0,scale=w=320:h=176:flags=lanczos,pad=320:240:0:32:color=black"
        ffmpeg_vcmd = [
            "ffmpeg",
            "-y",
            "-hide_banner",
            "-loglevel",
            "error",
            "-i",
            mp4_path,
            "-vf",
            vf_filter,
            "-r",
            "15",
            "-c:v",
            "ffv1",
            "-an",
            tmp_mkv_path,
        ]
        fres = subprocess.run(ffmpeg_vcmd, capture_output=True, text=True)
        video_input = tmp_mkv_path if (fres.returncode == 0 and os.path.exists(tmp_mkv_path)) else mp4_path

        cmd = [
            psxavenc,
            "-q",
            "-t",
            "strv",
            "-v",
            "v2",
            "-s",
            "320x240",
            "-r",
            "15",
            "-x",
            "1",
            video_input,
            str_out,
        ]
        res = subprocess.run(cmd, capture_output=True, text=True)
        if res.returncode != 0:
            print(f"psxavenc video encoding failed:\n{res.stderr}", file=sys.stderr)
            return False
    finally:
        if os.path.exists(tmp_mkv_path):
            os.remove(tmp_mkv_path)

    size = os.path.getsize(str_out)
    num_frames = size // (SECTORS_PER_FRAME * 2048)
    print(
        f"Mastered FMV Cinematic -> {str_out} ({size} bytes, {num_frames} frames @ 15 fps)"
    )

    # 2. Extract and encode intro audio to SPU ADPCM
    if adpcm_out:
        os.makedirs(os.path.dirname(os.path.abspath(adpcm_out)), exist_ok=True)
        with tempfile.NamedTemporaryFile(suffix=".wav", delete=False) as tmp_wav:
            tmp_wav_path = tmp_wav.name
        try:
            ffmpeg_cmd = [
                "ffmpeg",
                "-y",
                "-hide_banner",
                "-loglevel",
                "error",
                "-i",
                mp4_path,
                "-vn",
                "-ar",
                "16000",
                "-ac",
                "1",
                "-c:a",
                "pcm_s16le",
                tmp_wav_path,
            ]
            fres = subprocess.run(ffmpeg_cmd, capture_output=True, text=True)
            if fres.returncode == 0 and os.path.exists(tmp_wav_path):
                # Raw SPU ADPCM format with one-shot end flag
                acmd = [
                    psxavenc,
                    "-t",
                    "spu",
                    "-f",
                    "16000",
                    tmp_wav_path,
                    adpcm_out,
                ]
                ares = subprocess.run(acmd, capture_output=True, text=True)
                if ares.returncode == 0:
                    asize = os.path.getsize(adpcm_out)
                    print(
                        f"Mastered Intro SPU ADPCM -> {adpcm_out} ({asize} bytes @ 16 kHz)"
                    )
                else:
                    print(
                        f"Warning: psxavenc SPU audio encoding failed: {ares.stderr}",
                        file=sys.stderr,
                    )
            else:
                print(
                    f"Warning: ffmpeg audio extraction failed: {fres.stderr}",
                    file=sys.stderr,
                )
        finally:
            if os.path.exists(tmp_wav_path):
                os.remove(tmp_wav_path)

    return True


def ensure_dummy_adpcm(adpcm_out):
    """Ensures a valid SPU ADPCM file exists so compilation never breaks."""
    if not os.path.exists(adpcm_out):
        os.makedirs(os.path.dirname(os.path.abspath(adpcm_out)), exist_ok=True)
        # 16-byte silent SPU ADPCM block with loop-end flag (0x01)
        dummy = bytearray(16)
        dummy[1] = 0x01
        with open(adpcm_out, "wb") as f:
            f.write(dummy)


def main():
    repo_root = find_repo_root()
    str_out = sys.argv[1] if len(sys.argv) > 1 else os.path.join(repo_root, "assets", "INTRO.STR")
    
    mp4_in = None
    if len(sys.argv) > 2:
        mp4_in = sys.argv[2]
    elif os.environ.get("INTRO_MP4"):
        mp4_in = os.environ["INTRO_MP4"]
    else:
        default_mp4 = os.path.join(repo_root, "AssetSource", "ArduracerPSX Intro.mp4")
        if os.path.exists(default_mp4):
            mp4_in = default_mp4

    adpcm_out = sys.argv[3] if len(sys.argv) > 3 else os.path.join(repo_root, "assets", "INTRO.ADPCM")

    success = False
    if mp4_in and os.path.exists(mp4_in):
        success = cook_mp4(mp4_in, str_out, adpcm_out)

    if not success:
        cook_procedural_fallback(str_out, 60)
        ensure_dummy_adpcm(adpcm_out)


if __name__ == "__main__":
    main()
