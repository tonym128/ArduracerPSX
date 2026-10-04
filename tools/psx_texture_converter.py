#!/usr/bin/env python3
"""
PS1 Texture Converter for Arduracer PSX.

Converts PNG/SVG/JPEG images to PS1 TIM format suitable for VRAM upload.
Supports:
- 4-bit (16-color) and 8-bit (256-color) CLUT modes
- 16-bit direct color (1555/565)
- PS1 VRAM page alignment (64-pixel width pages)
- CLUT generation with optional quantization
- Megatexture atlas packing
- TIM file output with proper headers

PS1 TIM Format Reference:
- Header: 4 bytes magic (0x10), 4 bytes flags
- CLUT: 16/256 entries × 2 bytes (1555 format)
- Image: width × height pixels, 4/8/16 bpp
- VRAM coordinates for upload (x, y, w, h)
"""

import struct
import argparse
import sys
from pathlib import Path
from typing import Optional, Tuple, List
from dataclasses import dataclass
from enum import IntEnum

try:
    from PIL import Image
    PIL_AVAILABLE = True
    ImageType = Image.Image
except ImportError:
    PIL_AVAILABLE = False
    ImageType = None
    print("Warning: PIL/Pillow not available. Install with: pip install pillow cairosvg")

try:
    import cairosvg
    CAIROSVG_AVAILABLE = True
except ImportError:
    CAIROSVG_AVAILABLE = False


class PS1ColorDepth(IntEnum):
    """PS1 color depth modes (TIM format codes)."""
    BPP_4 = 0   # 16 colors (CLUT)
    BPP_8 = 1   # 256 colors (CLUT)
    BPP_16 = 2  # 16-bit direct (1555)
    BPP_24 = 3  # 24-bit direct (not used in VRAM)


@dataclass
class TIMHeader:
    """PS1 TIM file header."""
    magic: int = 0x10           # Always 0x10
    flags: int = 0              # Bit 0-2: bpp, Bit 3: has_clut
    clut_x: int = 0             # CLUT VRAM X position
    clut_y: int = 0             # CLUT VRAM Y position
    clut_w: int = 0             # CLUT width (num colors)
    clut_h: int = 1             # CLUT height (always 1)
    img_x: int = 0              # Image VRAM X position
    img_y: int = 0              # Image VRAM Y position
    img_w: int = 0              # Image width in pixels
    img_h: int = 0              # Image height in pixels


@dataclass
class PS1Texture:
    """PS1 texture with all data ready for VRAM upload."""
    header: TIMHeader
    clut_data: bytes            # CLUT entries (16 or 256 × 2 bytes)
    image_data: bytes           # Pixel data
    width: int
    height: int
    bpp: int                    # 4, 8, or 16
    has_clut: bool


def rgb888_to_1555(r: int, g: int, b: int, a: int = 255) -> int:
    """Convert 8-bit RGB to PS1 1555 format (1-bit alpha, 5-bit RGB)."""
    # PS1 uses 1555: A(1) R(5) G(5) B(5)
    # Alpha: 0 = transparent, 1 = opaque (semi-transparency handled via GPU blend mode)
    a_bit = 1 if a >= 128 else 0
    r5 = (r >> 3) & 0x1F
    g5 = (g >> 3) & 0x1F
    b5 = (b >> 3) & 0x1F
    return (a_bit << 15) | (r5 << 10) | (g5 << 5) | b5


def rgb888_to_565(r: int, g: int, b: int) -> int:
    """Convert 8-bit RGB to 565 format (no alpha)."""
    r5 = (r >> 3) & 0x1F
    g6 = (g >> 2) & 0x3F
    b5 = (b >> 3) & 0x1F
    return (r5 << 11) | (g6 << 5) | b5


def quantize_to_palette(image: "ImageType", max_colors: int) -> Tuple["ImageType", List[Tuple[int, int, int, int]]]:
    """Quantize image to max_colors using PIL's adaptive palette. Returns paletted 'P' mode image."""
    # Convert to RGBA if not already
    if image.mode != 'RGBA':
        image = image.convert('RGBA')
    
    # For RGBA, we need to quantize RGB first, then reapply alpha
    # Split alpha channel
    r, g, b, a = image.split()
    rgb_image = Image.merge('RGB', (r, g, b))  # type: ignore
    
    # Quantize RGB
    quantized_rgb = rgb_image.quantize(colors=max_colors, method=Image.Quantize.MEDIANCUT)  # type: ignore
    
    # Get palette
    palette = quantized_rgb.getpalette()
    palette_rgba = []
    for i in range(max_colors):
        pr = palette[i * 3]
        pg = palette[i * 3 + 1]
        pb = palette[i * 3 + 2]
        palette_rgba.append((pr, pg, pb, 255))
    
    # Return the paletted image (mode 'P') and palette
    return quantized_rgb, palette_rgba


def apply_alpha_to_paletted(paletted_image: "ImageType", alpha_channel: "ImageType") -> "ImageType":
    """Apply alpha channel to a paletted image by converting to RGBA."""
    # Convert paletted to RGB first
    rgb = paletted_image.convert('RGB')
    # Merge with alpha
    rgba = Image.merge('RGBA', (*rgb.split(), alpha_channel))  # type: ignore
    return rgba


def build_clut(palette: List[Tuple[int, int, int, int]], bpp: int) -> bytes:
    """Build CLUT data from palette (16 or 256 entries, 1555 format)."""
    clut_bytes = bytearray()
    for r, g, b, a in palette:
        color1555 = rgb888_to_1555(r, g, b, a)
        clut_bytes.extend(struct.pack('<H', color1555))
    return bytes(clut_bytes)


def encode_4bpp(image: Image.Image, width: int, height: int) -> bytes:
    """Encode 4bpp (16-color) image data. Two pixels per byte."""
    data = bytearray()
    pixels = image.load()
    for y in range(height):
        for x in range(0, width, 2):
            p1 = pixels[x, y] if x < width else 0
            p2 = pixels[x + 1, y] if x + 1 < width else 0
            # Each pixel is 0-15 (palette index)
            byte_val = (p1 & 0x0F) | ((p2 & 0x0F) << 4)
            data.append(byte_val)
    return bytes(data)


def encode_8bpp(image: Image.Image, width: int, height: int) -> bytes:
    """Encode 8bpp (256-color) image data. One pixel per byte."""
    data = bytearray()
    pixels = image.load()
    for y in range(height):
        for x in range(width):
            data.append(pixels[x, y] & 0xFF)
    return bytes(data)


def encode_16bpp(image: Image.Image, width: int, height: int) -> bytes:
    """Encode 16bpp direct color image data (1555 format)."""
    data = bytearray()
    pixels = image.load()
    for y in range(height):
        for x in range(width):
            r, g, b, a = pixels[x, y]
            color1555 = rgb888_to_1555(r, g, b, a)
            data.extend(struct.pack('<H', color1555))
    return bytes(data)


def align_vram_x(x: int, bpp: int) -> int:
    """Align VRAM X coordinate to PS1 page boundary (64 pixels for 4bpp, 32 for 8bpp, 16 for 16bpp)."""
    if bpp == 4:
        return (x + 63) // 64 * 64
    elif bpp == 8:
        return (x + 31) // 32 * 32
    else:  # 16bpp
        return (x + 15) // 16 * 16


def convert_image_to_psx(
    input_path: Path,
    output_path: Path,
    bpp: int = 8,
    max_colors: int = 256,
    vram_x: int = 0,
    vram_y: int = 0,
    clut_vram_x: int = 0,
    clut_vram_y: int = 480,  # Typical CLUT area in VRAM
    transparent_color: Optional[Tuple[int, int, int]] = None,
    dither: bool = False,
    resize: Optional[Tuple[int, int]] = None
) -> PS1Texture:
    """
    Convert an image to PS1 TIM format.
    
    Args:
        input_path: Path to input image (PNG, JPEG, SVG)
        output_path: Path to output TIM file
        bpp: Bits per pixel (4, 8, or 16)
        max_colors: Max colors for palette (16 for 4bpp, 256 for 8bpp)
        vram_x, vram_y: Target VRAM position for image
        clut_vram_x, clut_vram_y: Target VRAM position for CLUT
        transparent_color: RGB color to make transparent (for 16bpp)
        dither: Enable Floyd-Steinberg dithering
        resize: Optional (width, height) to resize to
    """
    if not PIL_AVAILABLE:
        raise RuntimeError("PIL/Pillow required. Install: pip install pillow")
    
    # Load image
    if input_path.suffix.lower() == '.svg':
        if not CAIROSVG_AVAILABLE:
            raise RuntimeError("cairosvg required for SVG support. Install: pip install cairosvg")
        # Convert SVG to PNG in memory
        import io
        png_data = cairosvg.svg2png(url=str(input_path))
        image = Image.open(io.BytesIO(png_data))
    else:
        image = Image.open(input_path)
    
    # Ensure RGBA
    if image.mode != 'RGBA':
        image = image.convert('RGBA')
    
    # Resize if requested
    if resize:
        image = image.resize(resize, Image.Resampling.LANCZOS)
    
    width, height = image.size
    
    # Handle transparency
    if transparent_color:
        pixels = image.load()
        for y in range(height):
            for x in range(width):
                r, g, b, a = pixels[x, y]
                if (r, g, b) == transparent_color:
                    pixels[x, y] = (r, g, b, 0)
    
    # Map user-friendly bpp to TIM format codes
    bpp_to_tim = {4: PS1ColorDepth.BPP_4, 8: PS1ColorDepth.BPP_8, 16: PS1ColorDepth.BPP_16}
    tim_bpp = bpp_to_tim[bpp]
    
    # Quantize for CLUT modes
    has_clut = bpp in (4, 8)
    palette = None
    paletted_image = None
    
    if has_clut:
        if bpp == 4:
            max_colors = min(max_colors, 16)
        else:
            max_colors = min(max_colors, 256)
        
        if dither:
            # Dither on RGB, then convert to P mode
            rgb = image.convert('RGB')
            paletted_image = rgb.quantize(colors=max_colors, method=Image.Quantize.MEDIANCUT, dither=Image.Dither.FLOYDSTEINBERG)
            # Build palette from quantized image
            palette_data = paletted_image.getpalette()
            palette = []
            for i in range(max_colors):
                pr = palette_data[i * 3]
                pg = palette_data[i * 3 + 1]
                pb = palette_data[i * 3 + 2]
                palette.append((pr, pg, pb, 255))
        else:
            paletted_image, palette = quantize_to_palette(image, max_colors)
        
        # For encoding, we use the paletted image directly (mode 'P')
        encode_image = paletted_image
    else:
        # 16bpp direct color - no palette needed, use RGBA
        encode_image = image.convert('RGBA')
    
    # Align VRAM coordinates
    vram_x_aligned = align_vram_x(vram_x, bpp)
    
    # Build header
    flags = tim_bpp
    if has_clut:
        flags |= 0x08  # Has CLUT flag
    
    header = TIMHeader(
        magic=0x10,
        flags=flags,
        clut_x=clut_vram_x,
        clut_y=clut_vram_y,
        clut_w=max_colors if has_clut else 0,
        clut_h=1,
        img_x=vram_x_aligned,
        img_y=vram_y,
        img_w=width,
        img_h=height
    )
    
    # Build CLUT
    if has_clut:
        clut_data = build_clut(palette, bpp)
    else:
        clut_data = b''
    
    # Encode image data
    if bpp == 4:
        image_data = encode_4bpp(encode_image, width, height)
    elif bpp == 8:
        image_data = encode_8bpp(encode_image, width, height)
    elif bpp == 16:
        image_data = encode_16bpp(encode_image, width, height)
    else:
        raise ValueError(f"Unsupported BPP: {bpp}")
    
    # Write TIM file
    with open(output_path, 'wb') as f:
        # TIM Header (12 bytes)
        f.write(struct.pack('<I', header.magic))
        f.write(struct.pack('<I', header.flags))
        # Image header (12 bytes)
        f.write(struct.pack('<HHH', header.img_x, header.img_y, 0))  # x, y, dummy
        f.write(struct.pack('<HHH', header.img_w, header.img_h, 0))  # w, h, dummy
        
        if has_clut:
            # CLUT header (12 bytes)
            f.write(struct.pack('<HHH', header.clut_x, header.clut_y, 0))
            f.write(struct.pack('<HHH', header.clut_w, header.clut_h, 0))
            # CLUT data
            f.write(clut_data)
        
        # Image data
        f.write(image_data)
    
    print(f"Converted: {input_path.name} -> {output_path.name}")
    print(f"  Size: {width}x{height}, BPP: {bpp}, Colors: {max_colors if has_clut else 'N/A'}")
    print(f"  VRAM: ({vram_x_aligned}, {vram_y}) CLUT: ({clut_vram_x}, {clut_vram_y})")
    print(f"  Output: {output_path} ({output_path.stat().st_size} bytes)")
    
    return PS1Texture(
        header=header,
        clut_data=clut_data,
        image_data=image_data,
        width=width,
        height=height,
        bpp=bpp,
        has_clut=has_clut
    )


def pack_atlas(
    input_dir: Path,
    output_path: Path,
    bpp: int = 8,
    max_colors: int = 256,
    tile_size: int = 64,
    cols: int = 8,
    spacing: int = 1
) -> PS1Texture:
    """
    Pack multiple images into a megatexture atlas.
    
    Args:
        input_dir: Directory containing stage SVGs/PNGs
        output_path: Output TIM file
        bpp: Color depth
        max_colors: Max palette colors
        tile_size: Base tile size for layout
        cols: Number of columns in atlas
        spacing: Spacing between tiles
    """
    if not PIL_AVAILABLE:
        raise RuntimeError("PIL/Pillow required")
    
    # Find all stage images
    image_files = sorted(input_dir.glob("stage_*.svg")) + sorted(input_dir.glob("stage_*.png"))
    if not image_files:
        raise FileNotFoundError(f"No stage images found in {input_dir}")
    
    # Load all images first
    images = []
    for img_path in image_files:
        if img_path.suffix.lower() == '.svg':
            if not CAIROSVG_AVAILABLE:
                continue
            import io
            png_data = cairosvg.svg2png(url=str(img_path))
            img = Image.open(io.BytesIO(png_data))
        else:
            img = Image.open(img_path)
        img = img.convert('RGBA')
        images.append((img_path.stem, img))
    
    if not images:
        raise RuntimeError("No valid images loaded")
    
    # Calculate atlas dimensions
    rows = (len(images) + cols - 1) // cols
    max_w = max(img.width for _, img in images)
    max_h = max(img.height for _, img in images)
    
    cell_w = max_w + spacing
    cell_h = max_h + spacing + 20  # Extra for label
    
    atlas_w = cols * cell_w
    atlas_h = rows * cell_h
    
    # Align to VRAM page
    atlas_w = align_vram_x(atlas_w, bpp)
    
    # Create atlas canvas
    atlas = Image.new('RGBA', (atlas_w, atlas_h), (0, 0, 0, 0))
    
    # Paste images
    for i, (name, img) in enumerate(images):
        col = i % cols
        row = i // cols
        x = col * cell_w + spacing // 2
        y = row * cell_h + spacing // 2 + 20
        
        # Center in cell
        px = x + (max_w - img.width) // 2
        py = y + (max_h - img.height) // 2
        
        atlas.paste(img, (px, py), img)
        
        # Draw label (would need PIL draw - simplified)
    
    # Save temporary PNG for conversion
    temp_png = output_path.with_suffix('.png')
    atlas.save(temp_png)
    
    # Convert to TIM
    result = convert_image_to_psx(
        temp_png, output_path,
        bpp=bpp, max_colors=max_colors,
        vram_x=0, vram_y=0
    )
    
    # Clean up
    temp_png.unlink(missing_ok=True)
    
    return result


def main():
    parser = argparse.ArgumentParser(description="PS1 Texture Converter for Arduracer PSX")
    parser.add_argument('input', help='Input image file or directory')
    parser.add_argument('output', help='Output TIM file')
    parser.add_argument('--bpp', type=int, choices=[4, 8, 16], default=8, help='Bits per pixel (4, 8, 16)')
    parser.add_argument('--colors', type=int, default=256, help='Max colors for palette (16/256)')
    parser.add_argument('--vram-x', type=int, default=0, help='VRAM X position')
    parser.add_argument('--vram-y', type=int, default=0, help='VRAM Y position')
    parser.add_argument('--clut-x', type=int, default=0, help='CLUT VRAM X position')
    parser.add_argument('--clut-y', type=int, default=480, help='CLUT VRAM Y position')
    parser.add_argument('--transparent', type=str, help='Transparent color as RRGGBB hex')
    parser.add_argument('--dither', action='store_true', help='Enable dithering')
    parser.add_argument('--resize', type=str, help='Resize to WxH (e.g., 512x512)')
    parser.add_argument('--atlas', action='store_true', help='Pack directory as megatexture atlas')
    parser.add_argument('--atlas-cols', type=int, default=8, help='Atlas columns')
    
    args = parser.parse_args()
    
    input_path = Path(args.input)
    output_path = Path(args.output)
    
    if not input_path.exists():
        print(f"Error: Input not found: {input_path}")
        sys.exit(1)
    
    transparent = None
    if args.transparent:
        transparent = tuple(int(args.transparent[i:i+2], 16) for i in (0, 2, 4))
    
    resize = None
    if args.resize:
        w, h = map(int, args.resize.split('x'))
        resize = (w, h)
    
    try:
        if args.atlas or input_path.is_dir():
            pack_atlas(input_path, output_path, args.bpp, args.colors, cols=args.atlas_cols)
        else:
            convert_image_to_psx(
                input_path, output_path,
                bpp=args.bpp,
                max_colors=args.colors,
                vram_x=args.vram_x,
                vram_y=args.vram_y,
                clut_vram_x=args.clut_x,
                clut_vram_y=args.clut_y,
                transparent_color=transparent,
                dither=args.dither,
                resize=resize
            )
        print("Success!")
    except Exception as e:
        print(f"Error: {e}")
        sys.exit(1)


if __name__ == "__main__":
    main()