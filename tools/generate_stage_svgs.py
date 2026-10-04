#!/usr/bin/env python3
"""
Generate color-coded SVG visualizations for all 24 Arduracer PSX stages.
Reads track tile data from crates/arduracer-core/src/levels.rs
"""

import re
import os
from pathlib import Path

# TrackTile enum values (must match crates/arduracer-core/src/track.rs)
TRACK_TILE_COLORS = {
    'Tarmac': '#2a2a2a',       # Dark charcoal asphalt
    'Curb': '#cc0000',         # Bright red for curbs
    'OffRoad': '#3d5a2a',      # Grass/gravel green-brown
    'OilSlick': '#1a0a2e',     # Dark purple iridescent
    'BoostPad': '#00ffff',     # Cyan boost
    'Barrier': '#4a4a4a',      # Dark grey barrier
    'StartFinish': '#ffffff',  # White start/finish
    'Checkpoint': '#ffff00',   # Yellow checkpoint
}

# Light versions for grid lines
TRACK_TILE_COLORS_LIGHT = {
    'Tarmac': '#3a3a3a',
    'Curb': '#ff3333',
    'OffRoad': '#4d6a3a',
    'OilSlick': '#2a1a3e',
    'BoostPad': '#33ffff',
    'Barrier': '#5a5a5a',
    'StartFinish': '#ffffff',
    'Checkpoint': '#ffff33',
}

TILE_SIZE = 8  # SVG pixels per track tile
GRID_STROKE = 0.5

def parse_levels_rs(filepath):
    """Parse the levels.rs file to extract track definitions."""
    with open(filepath, 'r') as f:
        content = f.read()

    tracks = []
    
    # Find all TRACK_XX_TILES arrays
    pattern = r'static TRACK_(\d+)_TILES: \[TrackTile; \d+\] = \[(.*?)\];'
    matches = re.findall(pattern, content, re.DOTALL)
    
    for track_num_str, tiles_content in matches:
        track_num = int(track_num_str)
        
        # Parse tile names
        tile_names = re.findall(r'TrackTile::(\w+)', tiles_content)
        
        # Find the TrackDef for dimensions
        def_pattern = rf'pub const TRACK_{track_num_str}: TrackDef = TrackDef \{{(.*?)\n\}};'
        def_match = re.search(def_pattern, content, re.DOTALL)
        
        width = 10
        height = 10
        name = f"Track {track_num}"
        
        if def_match:
            def_content = def_match.group(1)
            w_match = re.search(r'width: (\d+)', def_content)
            h_match = re.search(r'height: (\d+)', def_content)
            n_match = re.search(r'name: "([^"]+)"', def_content)
            if w_match:
                width = int(w_match.group(1))
            if h_match:
                height = int(h_match.group(1))
            if n_match:
                name = n_match.group(1)
        
        # Verify tile count matches dimensions
        expected = width * height
        if len(tile_names) != expected:
            print(f"Warning: Track {track_num} has {len(tile_names)} tiles, expected {expected} ({width}x{height})")
            # Pad or truncate
            if len(tile_names) < expected:
                tile_names += ['OffRoad'] * (expected - len(tile_names))
            else:
                tile_names = tile_names[:expected]
        
        tracks.append({
            'num': track_num,
            'name': name,
            'width': width,
            'height': height,
            'tiles': tile_names
        })
    
    return sorted(tracks, key=lambda t: t['num'])

def generate_svg(track, output_dir):
    """Generate SVG for a single track."""
    width = track['width']
    height = track['height']
    tiles = track['tiles']
    name = track['name']
    num = track['num']
    
    svg_width = width * TILE_SIZE
    svg_height = height * TILE_SIZE
    
    # Add padding for legend
    padding = 40
    legend_height = 120
    total_height = svg_height + padding + legend_height
    
    svg_lines = []
    svg_lines.append(f'<?xml version="1.0" encoding="UTF-8"?>')
    svg_lines.append(f'<svg xmlns="http://www.w3.org/2000/svg" width="{svg_width + 2*padding}" height="{total_height}" viewBox="0 0 {svg_width + 2*padding} {total_height}">')
    svg_lines.append(f'  <!-- {name} (Track {num:02d}) - {width}x{height} tiles -->')
    svg_lines.append(f'  <rect width="100%" height="100%" fill="#1a1a2e"/>')
    
    # Title
    svg_lines.append(f'  <text x="{padding + svg_width//2}" y="25" text-anchor="middle" font-family="monospace" font-size="14" fill="#e0e0e0">{name} (Track {num:02d})</text>')
    svg_lines.append(f'  <text x="{padding + svg_width//2}" y="40" text-anchor="middle" font-family="monospace" font-size="10" fill="#888">{width}×{height} tiles • {width*height} total</text>')
    
    # Draw grid
    y_offset = padding + 50
    
    for y in range(height):
        for x in range(width):
            idx = y * width + x
            tile_type = tiles[idx]
            color = TRACK_TILE_COLORS.get(tile_type, '#ff00ff')
            
            px = padding + x * TILE_SIZE
            py = y_offset + y * TILE_SIZE
            
            svg_lines.append(f'  <rect x="{px}" y="{py}" width="{TILE_SIZE}" height="{TILE_SIZE}" fill="{color}" stroke="#0a0a1a" stroke-width="{GRID_STROKE}"/>')
    
    # Legend
    legend_y = y_offset + svg_height + 20
    legend_items = [
        ('Tarmac', 'Racing surface (optimal grip)'),
        ('Curb', 'Rumble curb (vibration, slight grip loss)'),
        ('OffRoad', 'Off-road (grass/gravel/sand, severe penalty)'),
        ('OilSlick', 'Oil slick (near-zero lateral grip)'),
        ('BoostPad', 'Boost pad (instant speed impulse)'),
        ('Barrier', 'Solid barrier (impassable)'),
        ('StartFinish', 'Start/Finish line'),
        ('Checkpoint', 'Checkpoint gate'),
    ]
    
    svg_lines.append(f'  <text x="{padding}" y="{legend_y - 5}" font-family="monospace" font-size="11" fill="#aaa">Legend:</text>')
    
    col_width = (svg_width) // 4
    for i, (tile_type, desc) in enumerate(legend_items):
        col = i % 4
        row = i // 4
        lx = padding + col * col_width
        ly = legend_y + row * 28
        
        color = TRACK_TILE_COLORS.get(tile_type, '#ff00ff')
        svg_lines.append(f'  <rect x="{lx}" y="{ly}" width="16" height="16" fill="{color}" stroke="#333" stroke-width="1"/>')
        svg_lines.append(f'  <text x="{lx + 22}" y="{ly + 12}" font-family="monospace" font-size="10" fill="#ccc">{tile_type}: {desc}</text>')
    
    # Coordinate grid labels (every 5 tiles)
    svg_lines.append(f'  <g font-family="monospace" font-size="7" fill="#555">')
    for x in range(0, width, 5):
        px = padding + x * TILE_SIZE
        svg_lines.append(f'    <text x="{px + TILE_SIZE//2}" y="{y_offset - 8}" text-anchor="middle">{x}</text>')
    for y in range(0, height, 5):
        py = y_offset + y * TILE_SIZE
        svg_lines.append(f'    <text x="{padding - 5}" y="{py + TILE_SIZE//2 + 2}" text-anchor="end">{y}</text>')
    svg_lines.append(f'  </g>')
    
    svg_lines.append('</svg>')
    
    filename = f"stage_{num:02d}_{name.lower().replace(' ', '_').replace('\'', '').replace('’', '')}.svg"
    filepath = Path(output_dir) / filename
    
    with open(filepath, 'w') as f:
        f.write('\n'.join(svg_lines))
    
    print(f"Generated: {filename} ({width}x{height})")
    return filepath

def generate_atlas_svg(tracks, output_dir):
    """Generate a single atlas SVG showing all tracks in a grid."""
    # Calculate grid layout
    cols = 6
    rows = (len(tracks) + cols - 1) // cols
    
    # Each track thumbnail: max 80x60 tiles, scaled down
    thumb_tile_size = 3
    max_w = max(t['width'] for t in tracks)
    max_h = max(t['height'] for t in tracks)
    thumb_w = max_w * thumb_tile_size
    thumb_h = max_h * thumb_tile_size
    
    padding = 10
    label_h = 16
    total_w = cols * (thumb_w + padding) + padding
    total_h = rows * (thumb_h + label_h + padding) + padding
    
    svg_lines = []
    svg_lines.append(f'<?xml version="1.0" encoding="UTF-8"?>')
    svg_lines.append(f'<svg xmlns="http://www.w3.org/2000/svg" width="{total_w}" height="{total_h}" viewBox="0 0 {total_w} {total_h}">')
    svg_lines.append(f'  <rect width="100%" height="100%" fill="#0d0d1a"/>')
    svg_lines.append(f'  <text x="{total_w//2}" y="20" text-anchor="middle" font-family="monospace" font-size="16" fill="#e0e0e0">Arduracer PSX - All 24 Stages Atlas</text>')
    
    for i, track in enumerate(tracks):
        col = i % cols
        row = i // cols
        
        x = padding + col * (thumb_w + padding)
        y = padding + 25 + row * (thumb_h + label_h + padding)
        
        width = track['width']
        height = track['height']
        tiles = track['tiles']
        name = track['name']
        num = track['num']
        
        # Track label
        svg_lines.append(f'  <text x="{x + thumb_w//2}" y="{y - 3}" text-anchor="middle" font-family="monospace" font-size="9" fill="#aaa">#{num:02d} {name}</text>')
        
        # Draw tiles
        for ty in range(height):
            for tx in range(width):
                idx = ty * width + tx
                tile_type = tiles[idx]
                color = TRACK_TILE_COLORS.get(tile_type, '#ff00ff')
                
                px = x + tx * thumb_tile_size
                py = y + label_h + ty * thumb_tile_size
                
                svg_lines.append(f'  <rect x="{px}" y="{py}" width="{thumb_tile_size}" height="{thumb_tile_size}" fill="{color}"/>')
        
        # Border
        svg_lines.append(f'  <rect x="{x}" y="{y + label_h}" width="{width * thumb_tile_size}" height="{height * thumb_tile_size}" fill="none" stroke="#333" stroke-width="1"/>')
    
    # Legend at bottom
    legend_y = total_h - 60
    svg_lines.append(f'  <text x="{padding}" y="{legend_y - 5}" font-family="monospace" font-size="10" fill="#888">Surface Types:</text>')
    
    legend_items = ['Tarmac', 'Curb', 'OffRoad', 'OilSlick', 'BoostPad', 'Barrier', 'StartFinish', 'Checkpoint']
    for i, tile_type in enumerate(legend_items):
        lx = padding + i * 100
        ly = legend_y
        color = TRACK_TILE_COLORS.get(tile_type, '#ff00ff')
        svg_lines.append(f'  <rect x="{lx}" y="{ly}" width="14" height="14" fill="{color}" stroke="#333"/>')
        svg_lines.append(f'  <text x="{lx + 18}" y="{ly + 10}" font-family="monospace" font-size="9" fill="#aaa">{tile_type}</text>')
    
    svg_lines.append('</svg>')
    
    filepath = Path(output_dir) / "atlas_all_stages.svg"
    with open(filepath, 'w') as f:
        f.write('\n'.join(svg_lines))
    
    print(f"Generated atlas: {filepath}")
    return filepath

def main():
    levels_path = Path(__file__).parent.parent / "crates" / "arduracer-core" / "src" / "levels.rs"
    output_dir = Path(__file__).parent.parent / "assets" / "stage_svgs"
    output_dir.mkdir(parents=True, exist_ok=True)
    
    print(f"Parsing {levels_path}...")
    tracks = parse_levels_rs(levels_path)
    print(f"Found {len(tracks)} tracks")
    
    for track in tracks:
        generate_svg(track, output_dir)
    
    generate_atlas_svg(tracks, output_dir)
    
    print(f"\nAll SVGs generated in {output_dir}")

if __name__ == "__main__":
    main()