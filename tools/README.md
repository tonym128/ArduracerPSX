# Arduracer PSX - Asset Pipeline Tools

This directory contains tools for generating stage visualizations and converting them to PS1-native texture formats.

## Tools

### 1. Stage SVG Generator (`generate_stage_svgs.py`)

Generates color-coded SVG visualizations of all 24 tracks from the compiled track data in `crates/arduracer-core/src/levels.rs`.

**Usage:**
```bash
python3 generate_stage_svgs.py
```

**Output:**
- `assets/stage_svgs/stage_XX_name.svg` - Individual track visualizations (24 files)
- `assets/stage_svgs/atlas_all_stages.svg` - Combined atlas view of all tracks

**Color Coding:**
| Color | Surface Type | Gameplay Effect |
|-------|-------------|-----------------|
| Dark Grey | Tarmac | Optimal grip (1.0x) |
| Red | Curb | Rumble, slight grip loss (0.85x) |
| Green-Brown | OffRoad | Severe penalty (0.35x speed) |
| Dark Purple | OilSlick | Near-zero lateral grip |
| Cyan | BoostPad | Instant speed impulse |
| Dark Grey | Barrier | Impassable wall |
| White | StartFinish | Start/finish line |
| Yellow | Checkpoint | Checkpoint gate |

### 2. PS1 Texture Converter (`psx_texture_converter.py`)

Converts images (PNG, JPEG, SVG) to PS1 TIM format for direct VRAM upload.

**Dependencies:**
```bash
pip install --break-system-packages pillow cairosvg
```

**Usage:**
```bash
# Single image conversion
python3 psx_texture_converter.py input.svg output.tim --bpp 8 --colors 256 --resize 256x256

# Megatexture atlas packing
python3 psx_texture_converter.py assets/stage_svgs/ atlas.tim --atlas --bpp 8 --colors 256 --atlas-cols 6
```

**Options:**
| Option | Description |
|--------|-------------|
| `--bpp` | Bits per pixel: 4 (16-color), 8 (256-color), 16 (direct 1555) |
| `--colors` | Max palette colors (16 for 4bpp, 256 for 8bpp) |
| `--vram-x`, `--vram-y` | Target VRAM position for image |
| `--clut-x`, `--clut-y` | Target VRAM position for CLUT (default: 0, 480) |
| `--transparent RRGGBB` | Make specific RGB color transparent |
| `--dither` | Enable Floyd-Steinberg dithering |
| `--resize WxH` | Resize input before conversion |
| `--atlas` | Pack all images in directory as megatexture |
| `--atlas-cols N` | Number of columns in atlas (default: 8) |

## PS1 TIM Format

The converter outputs standard PS1 TIM files with:
- **Magic**: 0x10
- **Flags**: BPP code (0=4bpp, 1=8bpp, 2=16bpp) + 0x08 if CLUT present
- **Image Header**: VRAM (x, y), dimensions (w, h)
- **CLUT Header** (if present): VRAM (x, y), size (colors, 1)
- **CLUT Data**: 16/256 entries × 2 bytes (1555 format)
- **Image Data**: Packed pixel data

### VRAM Layout Guidelines

```
VRAM (1024×512, 16-bit):
┌─────────────────────────────────────────────┐
│ 0,0 - 1023,239    │ Frame Buffer 0 (320×240) │
│ 0,240 - 1023,479  │ Frame Buffer 1 (320×240) │
├─────────────────────────────────────────────┤
│ 0,480 - 255,480   │ CLUT Area (256 colors)  │
│ 256,480 - 511,511 │ Texture Pages           │
└─────────────────────────────────────────────┘
```

**Page Alignment Rules:**
- 4bpp: 64-pixel width alignment
- 8bpp: 32-pixel width alignment  
- 16bpp: 16-pixel width alignment

## Integration with Arduracer PSX

### Loading Textures at Runtime

```rust
// In your PSX game code (using psoxide SDK)
use psoxide::gpu::{Tim, Vram};

fn load_stage_texture(vram: &mut Vram, tim_data: &[u8]) -> Tim {
    let tim = Tim::from_bytes(tim_data).unwrap();
    
    // Upload CLUT if present
    if tim.has_clut() {
        vram.upload_clut(tim.clut_vram_x(), tim.clut_vram_y(), tim.clut_data());
    }
    
    // Upload image data
    vram.upload_texture(
        tim.img_vram_x(), 
        tim.img_vram_y(), 
        tim.width(), 
        tim.height(), 
        tim.image_data()
    );
    
    tim
}
```

### Megatexture Streaming

For the 24-stage megatexture (atlas.tim ~3.3MB at 8bpp):
1. Upload full atlas to VRAM during level load
2. Use texture page switching for different track regions
3. Or stream sub-rectangles on-demand using DMA

## Example Workflow

```bash
# 1. Generate SVGs from track data
cd /home/tonym/Projects/ArduracerPSX
python3 tools/generate_stage_svgs.py

# 2. Convert individual stages to 8bpp TIM (for unique per-track textures)
for svg in assets/stage_svgs/stage_*.svg; do
    base=$(basename "$svg" .svg)
    python3 tools/psx_texture_converter.py "$svg" "assets/tim/${base}.tim" --bpp 8 --colors 256 --resize 512x512
done

# 3. Create megatexture atlas for streaming
python3 tools/psx_texture_converter.py assets/stage_svgs/ assets/tim/atlas.tim --atlas --bpp 8 --colors 256 --atlas-cols 6
```

## File Structure

```
tools/
├── generate_stage_svgs.py    # SVG generator
├── psx_texture_converter.py  # TIM converter
├── README.md                 # This file
└── psx_texture_converter_test.py  # Validation script

assets/
├── stage_svgs/               # Generated SVGs (24 + atlas)
└── tim/                      # Converted TIM files (output)
```

## Notes

- SVGs are generated from actual track tile data, ensuring accuracy
- TIM files are compatible with standard PS1 GPU upload routines
- 8bpp recommended for megatexture (balance of quality/VRAM)
- 4bpp for memory-constrained unique textures
- 16bpp for HUD/UI elements needing alpha blending