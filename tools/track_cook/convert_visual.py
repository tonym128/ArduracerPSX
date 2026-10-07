#!/usr/bin/env python3
"""
Visual Image Bit-Depth Converter to In-Game 8-Bit Colour (8bpp).

Converts any source visual image (PNG of 1-bit, 2-bit, 4-bit, 8-bit grayscale,
8-bit paletted, 16-bit, 24-bit RGB, 32-bit RGBA, etc.) into the game's
8-bit colour depth (8bpp indexed with 256-color CLUT).

Usage:
    python3 tools/track_cook/convert_visual.py input.png output.bin [--clut output_clut.bin]
    python3 tools/track_cook/convert_visual.py --slice-city city.png --out-dir dist/city_chunks/
"""

import argparse
import os
import sys
from typing import Tuple, List, Optional
import numpy as np
from PIL import Image

PALETTE_SIZE_8BIT = 256

# Canonical 16 core track surface colours from LEVEL-FORMAT.md
CORE_PALETTE_16 = [
    (0x00, 0x00, 0x00),  # 0: VOID
    (0x3C, 0x3E, 0x44),  # 1: TARMAC
    (0x4A, 0x4C, 0x54),  # 2: TARMAC_WORN
    (0xE8, 0xE8, 0xF0),  # 3: KERB_WHITE
    (0xD8, 0x28, 0x3C),  # 4: KERB_RED
    (0x2E, 0x5A, 0x34),  # 5: GRASS
    (0x6B, 0x54, 0x32),  # 6: GRAVEL
    (0x7A, 0x60, 0x90),  # 7: SAND
    (0x8A, 0x1F, 0xB0),  # 8: OIL
    (0xFF, 0x8A, 0x10),  # 9: BOOST
    (0xF2, 0xF2, 0xF2),  # 10: START_LINE
    (0x00, 0xD2, 0xFF),  # 11: GATE
    (0x2A, 0x2E, 0x38),  # 12: WALL
    (0x5A, 0x46, 0x32),  # 13: TUNNEL
    (0x3A, 0x5A, 0x7A),  # 14: BRIDGE
    (0xFF, 0x2E, 0x88),  # 15: SCENERY
]


def pack_bgr555(r: int, g: int, b: int) -> int:
    """Converts 8-bit RGB to 16-bit PSX BGR555 halfword."""
    return (((b >> 3) & 0x1F) << 10) | (((g >> 3) & 0x1F) << 5) | ((r >> 3) & 0x1F)


def convert_image_to_8bit_colour(img: Image.Image) -> Tuple[bytes, List[Tuple[int, int, int]]]:
    """
    Converts any PIL image (regardless of source bit depth) to 8-bit colour:
    Returns (pixel_bytes, clut_rgb_256).
    """
    # 1. Handle source format / bit depth
    src_mode = img.mode

    if src_mode == 'RGBA':
        # Composite over black background
        bg = Image.new('RGB', img.size, (0, 0, 0))
        bg.paste(img, mask=img.split()[3])
        rgb_img = bg
    elif src_mode in ('1', 'L', 'I;16', 'I'):
        # Grayscale / bilevel -> RGB
        rgb_img = img.convert('RGB')
    elif src_mode == 'P':
        # Paletted image
        rgb_img = img.convert('RGB')
    else:
        rgb_img = img.convert('RGB')

    # 2. Extract / quantize to 256 colours
    # We ensure the 16 core surface colours are present in slots 0..15.
    clut = list(CORE_PALETTE_16)

    # Quantize to at most 240 additional colours
    quantized = rgb_img.quantize(colors=PALETTE_SIZE_8BIT - 16, method=Image.Quantize.MEDIANCUT)
    pal_data = quantized.getpalette() or []
    
    # Extract RGB triples from quantized palette
    for i in range(0, len(pal_data), 3):
        if len(clut) >= PALETTE_SIZE_8BIT:
            break
        color = (pal_data[i], pal_data[i + 1], pal_data[i + 2])
        if color not in clut:
            clut.append(color)

    # Pad palette to 256 entries if needed
    while len(clut) < PALETTE_SIZE_8BIT:
        clut.append((0, 0, 0))

    # 3. Map pixels to 8-bit indices
    # Convert rgb_img to numpy array
    rgb_arr = np.asarray(rgb_img, dtype=np.int32)
    h, w = rgb_arr.shape[:2]

    pal_arr = np.asarray(clut, dtype=np.int32)
    # Distance to each of the 256 palette entries
    # Chunked by rows if image is large to conserve memory
    indices = np.zeros((h, w), dtype=np.uint8)

    chunk_rows = 64
    for y_start in range(0, h, chunk_rows):
        y_end = min(y_start + chunk_rows, h)
        row_slice = rgb_arr[y_start:y_end]  # (rows, w, 3)
        dist = (
            (row_slice[:, :, None, 0] - pal_arr[None, None, :, 0]) ** 2
            + (row_slice[:, :, None, 1] - pal_arr[None, None, :, 1]) ** 2
            + (row_slice[:, :, None, 2] - pal_arr[None, None, :, 2]) ** 2
        )
        indices[y_start:y_end] = dist.argmin(axis=2).astype(np.uint8)

    pixel_bytes = indices.tobytes()
    return pixel_bytes, clut


def slice_city_visual(
    img: Image.Image,
    chunk_texel_size: int = 256,
) -> Tuple[List[Tuple[int, int, bytes]], List[Tuple[int, int, int]]]:
    """
    Slices a large city image of any bit-depth into 8-bit colour chunks
    (default: 256x256 texels per chunk, matching a PSX texture page).
    Returns (chunks_list, city_clut_256).
    """
    pixel_bytes, clut = convert_image_to_8bit_colour(img)
    w, h = img.size

    indices = np.frombuffer(pixel_bytes, dtype=np.uint8).reshape((h, w))
    chunks_x = (w + chunk_texel_size - 1) // chunk_texel_size
    chunks_y = (h + chunk_texel_size - 1) // chunk_texel_size

    chunks = []
    for cy in range(chunks_y):
        for cx in range(chunks_x):
            x0 = cx * chunk_texel_size
            y0 = cy * chunk_texel_size
            x1 = min(x0 + chunk_texel_size, w)
            y1 = min(y0 + chunk_texel_size, h)

            chunk_arr = np.zeros((chunk_texel_size, chunk_texel_size), dtype=np.uint8)
            sub = indices[y0:y1, x0:x1]
            chunk_arr[0 : y1 - y0, 0 : x1 - x0] = sub
            chunks.append((cx, cy, chunk_arr.tobytes()))

    return chunks, clut


def main():
    parser = argparse.ArgumentParser(description="Convert any image bit depth to 8-bit colour for Arduracer PSX")
    parser.add_argument("input", help="Source image path")
    parser.add_argument("output", nargs="?", help="Output 8bpp raw byte file")
    parser.add_argument("--clut", help="Output CLUT file (512 bytes BGR555)")
    parser.add_argument("--slice-city", action="store_true", help="Slice city into 256x256 chunks")
    parser.add_argument("--out-dir", default="dist/chunks", help="Directory for city chunks")
    args = parser.parse_args()

    if not os.path.exists(args.input):
        print(f"Error: {args.input} does not exist", file=sys.stderr)
        return 1

    img = Image.open(args.input)
    print(f"Loaded {args.input}: {img.size[0]}x{img.size[1]} mode={img.mode}")

    if args.slice_city:
        os.makedirs(args.out_dir, exist_ok=True)
        chunks, clut = slice_city_visual(img)
        print(f"Sliced into {len(chunks)} chunks of 256x256 texels (8-bit colour)")

        clut_path = os.path.join(args.out_dir, "city_clut.bin")
        with open(clut_path, "wb") as f:
            for r, g, b in clut:
                word = pack_bgr555(r, g, b)
                f.write(word.to_bytes(2, "little"))

        for cx, cy, cbytes in chunks:
            chunk_file = os.path.join(args.out_dir, f"chunk_{cx}_{cy}.bin")
            with open(chunk_file, "wb") as f:
                f.write(cbytes)
        print(f"Wrote {len(chunks)} chunks and CLUT to {args.out_dir}")
    else:
        if not args.output:
            print("Error: output path required when not slicing", file=sys.stderr)
            return 1
        pixel_bytes, clut = convert_image_to_8bit_colour(img)
        with open(args.output, "wb") as f:
            f.write(pixel_bytes)
        print(f"Wrote {len(pixel_bytes)} 8bpp texels to {args.output}")

        if args.clut:
            with open(args.clut, "wb") as f:
                for r, g, b in clut:
                    word = pack_bgr555(r, g, b)
                    f.write(word.to_bytes(2, "little"))
            print(f"Wrote 256-colour CLUT to {args.clut}")

    return 0


if __name__ == "__main__":
    sys.exit(main())
