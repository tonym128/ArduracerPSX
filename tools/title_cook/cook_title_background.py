#!/usr/bin/env python3
"""Cook the start-screen logo into a PS1 8bpp texture and RGB555 palette."""

import os
import sys

from PIL import Image, ImageOps

WIDTH = 256
HEIGHT = 90


def main():
    if len(sys.argv) != 4:
        raise SystemExit(
            "usage: cook_title_background.py source.png output-indexed.bin output-palette.bin"
        )

    source_path, indexed_path, palette_path = sys.argv[1:]
    with Image.open(source_path) as source:
        source = source.convert("RGBA")
        background = Image.new("RGBA", source.size, (0, 0, 0, 255))
        background.alpha_composite(source)
        rgb = ImageOps.contain(background.convert("RGB"), (WIDTH, HEIGHT), Image.Resampling.LANCZOS)

    canvas = Image.new("RGB", (WIDTH, HEIGHT), (0, 0, 0))
    canvas.paste(rgb, ((WIDTH - rgb.width) // 2, (HEIGHT - rgb.height) // 2))
    quantized = canvas.quantize(colors=255, method=Image.Quantize.MEDIANCUT, dither=Image.Dither.FLOYDSTEINBERG)

    indices = bytes(index + 1 for index in quantized.tobytes())
    source_palette = quantized.getpalette() or []
    source_palette += [0] * (768 - len(source_palette))
    palette = bytearray()
    # Index zero is reserved for transparent black; quantized source colours
    # occupy entries 1..255.
    palette.extend((0, 0))
    for index in range(255):
        red, green, blue = source_palette[index * 3 : index * 3 + 3]
        bgr555 = ((blue >> 3) << 10) | ((green >> 3) << 5) | (red >> 3)
        palette.extend((bgr555 & 0xFF, bgr555 >> 8))

    os.makedirs(os.path.dirname(indexed_path) or ".", exist_ok=True)
    os.makedirs(os.path.dirname(palette_path) or ".", exist_ok=True)
    with open(indexed_path, "wb") as output:
        output.write(indices)
    with open(palette_path, "wb") as output:
        output.write(palette)

    print(f"Cooked title image: {WIDTH}x{HEIGHT}, {len(indices)} indexed bytes")


if __name__ == "__main__":
    main()
