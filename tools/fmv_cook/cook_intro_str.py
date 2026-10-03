#!/usr/bin/env python3
"""
Arduracer PSX - MDEC Full-Motion Video (STR) Cooker.

Generates 320x240 @ 15 fps Version-2 PlayStation STR video streams for the
game's attract intro cinematics, fully adhering to PSX MDEC hardware specifications.
"""

import os
import sys
import struct
import math

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
            self.current_word |= (chunk << (self.bits_left - take))
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
    """Renders 20x15 macroblock RGB grid for the intro animation."""
    grid = [[(0, 0, 0) for _ in range(20)] for _ in range(15)]
    progress = frame_idx / float(total_frames)

    # Fade envelope
    if frame_idx < 15:
        intensity = frame_idx / 15.0
    elif frame_idx > 50:
        intensity = (total_frames - frame_idx) / 10.0
    else:
        intensity = 1.0

    for row in range(15):
        for col in range(20):
            # Background deep PlayStation navy gradient
            bg_b = int(40 + (row * 6))
            bg_r = int(10 + (col * 2))
            bg_g = int(15 + (row * 2))

            # Center title emblem
            is_stripe = (row == 5 or row == 9)
            is_center_banner = (row in (6, 7, 8) and 3 <= col <= 16)
            is_pulse = math.sin(frame_idx * 0.4 + col * 0.3) > 0.0

            if is_stripe:
                # Crimson Red racing stripe
                r, g, b = (220, 30, 45)
            elif is_center_banner:
                if is_pulse:
                    # Pulsing Neon Yellow / Cyan
                    r, g, b = (255, 230, 20)
                else:
                    # Pure White
                    r, g, b = (255, 255, 255)
            else:
                r, g, b = (bg_r, bg_g, bg_b)

            grid[row][col] = (
                int(r * intensity),
                int(g * intensity),
                int(b * intensity)
            )

    return grid

def encode_str_frame(frame_num, grid):
    """Encodes a 20x15 macroblock grid into a multi-sector STR frame."""
    bw = BitWriter()
    # Macroblocks are ordered column-major: for col in 0..20, for row in 0..15
    for col in range(20):
        for row in range(15):
            r, g, b = grid[row][col]
            dc_cr, dc_cb, dc_y = rgb_to_mdec_dc(r, g, b)
            # 6 blocks: Cr, Cb, Y0, Y1, Y2, Y3
            bw.write(dc_cr, 10)
            bw.write(0b10, 2) # EOB
            bw.write(dc_cb, 10)
            bw.write(0b10, 2)
            for _ in range(4):
                bw.write(dc_y, 10)
                bw.write(0b10, 2)

    # V2 End-of-frame marker
    bw.write(0x1FF, 10)
    body = bw.finish()

    # 300 MBs * 6 blks * 2 hw = 3600 hw = 1800 words. Aligned to multiple of 32 words = 1824 words.
    announced_mdec_words = 1824
    bs_header = struct.pack("<HHHH", announced_mdec_words, 0x3800, 1, 2)
    bs_data = bs_header + body

    chunk_size = 2016
    chunks = [bs_data[i:i + chunk_size] for i in range(0, len(bs_data), chunk_size)]
    num_chunks = len(chunks)

    sectors = bytearray()
    for idx, c in enumerate(chunks):
        hdr = struct.pack("<HHHHIIHH", 0x0160, 0x8001, idx, num_chunks, frame_num, len(bs_data), 320, 240)
        hdr += bs_header
        hdr += b"\x00\x00\x00\x00"
        payload = c.ljust(2016, b"\x00")
        sectors += hdr + payload

    return bytes(sectors)

def cook_intro_str(output_path, num_frames=60):
    """Cooks the complete attract intro STR video file."""
    os.makedirs(os.path.dirname(os.path.abspath(output_path)), exist_ok=True)
    with open(output_path, "wb") as f:
        for fr in range(1, num_frames + 1):
            grid = render_frame_pixels(fr, num_frames)
            sectors = encode_str_frame(fr, grid)
            f.write(sectors)
    size = os.path.getsize(output_path)
    print(f"Mastered FMV Cinematic -> {output_path} ({size} bytes, {num_frames} frames @ 15 fps)")

if __name__ == "__main__":
    out = sys.argv[1] if len(sys.argv) > 1 else "assets/INTRO.STR"
    cook_intro_str(out, 60)
