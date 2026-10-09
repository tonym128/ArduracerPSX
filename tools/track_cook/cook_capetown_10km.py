#!/usr/bin/env python3
"""
Downloads satellite imagery and OpenStreetMap data for 10 km^2 of Cape Town
(Atlantic Seaboard, Green Point, Mouille Point, V&A Waterfront) and compiles
into a 3x3 grid of 1024x1024 baseline JPEGs (<= 200 KB each) with aligned
collision data for Arduracer PSX.
"""

import math
import os
import sys
import json
import io
import time
import urllib.request
import urllib.parse
from PIL import Image, ImageDraw, ImageFilter, ImageEnhance
import numpy as np

# Bounding box for 10 km^2 Cape Town (3.162 km x 3.162 km square)
# Anchors: Sea Point Promenade, Mouille Point Lighthouse, Cape Town Stadium, V&A Waterfront
MIN_LAT = -33.92424
MAX_LAT = -33.89576
MIN_LON = 18.38884
MAX_LON = 18.42316

BLOCK_DIM = 1024
GRID_BLOCKS_X = 3
GRID_BLOCKS_Y = 3
TOTAL_W = BLOCK_DIM * GRID_BLOCKS_X  # 3072 texels
TOTAL_H = BLOCK_DIM * GRID_BLOCKS_Y  # 3072 texels

CELLS_PER_BLOCK = 80
TOTAL_CELLS_X = CELLS_PER_BLOCK * GRID_BLOCKS_X  # 240 cells
TOTAL_CELLS_Y = CELLS_PER_BLOCK * GRID_BLOCKS_Y  # 240 cells

TARGET_JPEG_BYTES = 200 * 1024  # 204,800 bytes (100 CD sectors)

def deg2num(lat_deg, lon_deg, zoom):
    lat_rad = math.radians(lat_deg)
    n = 2.0 ** zoom
    xtile = int((lon_deg + 180.0) / 360.0 * n)
    ytile = int((1.0 - math.asinh(math.tan(lat_rad)) / math.pi) / 2.0 * n)
    return (xtile, ytile)

def num2deg(xtile, ytile, zoom):
    n = 2.0 ** zoom
    lon_deg = xtile / n * 360.0 - 180.0
    lat_rad = math.atan(math.sinh(math.pi * (1 - 2 * ytile / n)))
    lat_deg = math.degrees(lat_rad)
    return (lat_deg, lon_deg)

def fetch_satellite_image(min_lat, max_lat, min_lon, max_lon, out_w, out_h) -> Image.Image:
    print(f"[Satellite] Fetching satellite imagery at Zoom 16 for [{min_lat:.4f}, {min_lon:.4f}] to [{max_lat:.4f}, {max_lon:.4f}]...")
    zoom = 16
    x0, y0 = deg2num(max_lat, min_lon, zoom)
    x1, y1 = deg2num(min_lat, max_lon, zoom)
    
    tile_w = 256
    tile_h = 256
    cols = x1 - x0 + 1
    rows = y1 - y0 + 1
    print(f"[Satellite] Stitching {cols}x{rows} = {cols*rows} satellite tiles...")

    stitched = Image.new("RGB", (cols * tile_w, rows * tile_h))
    
    # Northwest of tile grid
    grid_nw_lat, grid_nw_lon = num2deg(x0, y0, zoom)
    # Southeast of tile grid
    grid_se_lat, grid_se_lon = num2deg(x1 + 1, y1 + 1, zoom)

    for r, y in enumerate(range(y0, y1 + 1)):
        for c, x in enumerate(range(x0, x1 + 1)):
            url = f"https://server.arcgisonline.com/ArcGIS/rest/services/World_Imagery/MapServer/tile/{zoom}/{y}/{x}"
            req = urllib.request.Request(url, headers={"User-Agent": "Mozilla/5.0 ArduracerPSX/1.0"})
            success = False
            for attempt in range(3):
                try:
                    with urllib.request.urlopen(req, timeout=10) as resp:
                        tile_data = resp.read()
                        tile_im = Image.open(io.BytesIO(tile_data)).convert("RGB")
                        stitched.paste(tile_im, (c * tile_w, r * tile_h))
                        success = True
                        break
                except Exception as e:
                    time.sleep(0.5)
            if not success:
                print(f"[Satellite] Warning: Failed to fetch tile {zoom}/{y}/{x}, filling placeholder")
                placeholder = Image.new("RGB", (tile_w, tile_h), (40, 50, 45))
                stitched.paste(placeholder, (c * tile_w, r * tile_h))

    # Crop to exact bounding box
    crop_x0 = int((min_lon - grid_nw_lon) / (grid_se_lon - grid_nw_lon) * stitched.width)
    crop_x1 = int((max_lon - grid_nw_lon) / (grid_se_lon - grid_nw_lon) * stitched.width)
    crop_y0 = int((grid_nw_lat - max_lat) / (grid_nw_lat - grid_se_lat) * stitched.height)
    crop_y1 = int((grid_nw_lat - min_lat) / (grid_nw_lat - grid_se_lat) * stitched.height)

    cropped = stitched.crop((crop_x0, crop_y0, crop_x1, crop_y1))
    resized = cropped.resize((out_w, out_h), Image.Resampling.LANCZOS)
    return resized

def fetch_osm_highways(min_lat, max_lat, min_lon, max_lon):
    print("[OSM] Querying OpenStreetMap highways via Overpass API...")
    query = f"""
    [out:json][timeout:30];
    (
      way["highway"]({min_lat},{min_lon},{max_lat},{max_lon});
    );
    out geom;
    """
    url = "https://overpass-api.de/api/interpreter"
    req = urllib.request.Request(
        url,
        data=urllib.parse.urlencode({"data": query}).encode("utf-8"),
        headers={"User-Agent": "ArduracerPSX/1.0"}
    )
    try:
        with urllib.request.urlopen(req, timeout=25) as resp:
            data = json.loads(resp.read().decode("utf-8"))
            ways = data.get("elements", [])
            print(f"[OSM] Successfully fetched {len(ways)} highway segments!")
            return ways
    except Exception as e:
        print(f"[OSM] Error fetching OSM data: {e}")
        return []

def compress_1024_jpeg(im: Image.Image, target_bytes: int = TARGET_JPEG_BYTES) -> bytes:
    low = 5
    high = 95
    best_data = None
    while low <= high:
        mid = (low + high) // 2
        buf = io.BytesIO()
        im.save(buf, format="JPEG", quality=mid, restart_marker_blocks=4)
        data = buf.getvalue()
        if len(data) <= target_bytes:
            best_data = data
            low = mid + 1
        else:
            high = mid - 1
    if best_data is None:
        buf = io.BytesIO()
        im.save(buf, format="JPEG", quality=5, restart_marker_blocks=4)
        best_data = buf.getvalue()
    return best_data

def main():
    root = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
    out_dir = os.path.join(root, "dist", "capetown_10km")
    tracks_dir = os.path.join(root, "tracks", "capetown_10km")
    os.makedirs(out_dir, exist_ok=True)
    os.makedirs(tracks_dir, exist_ok=True)

    print("=================================================================")
    print("  ARDURACER PSX - CAPE TOWN 10 KM^2 MAP & VISUAL ATLAS COOKER    ")
    print("=================================================================")
    print(f"BBox: [{MIN_LAT}, {MIN_LON}] to [{MAX_LAT}, {MAX_LON}]")
    print(f"Area: 3.162 km x 3.162 km = 10.00 km^2")
    print(f"Grid: {GRID_BLOCKS_X}x{GRID_BLOCKS_Y} blocks of 1024x1024 = {TOTAL_W}x{TOTAL_H} texels")
    print(f"Cells: {TOTAL_CELLS_X}x{TOTAL_CELLS_Y} collision cells (32 WU/cell)")

    # 1. Fetch satellite base imagery
    cache_sat = os.path.join(out_dir, "capetown_satellite_3072.png")
    if os.path.exists(cache_sat):
        print(f"[Satellite] Loading cached {cache_sat}...")
        sat_img = Image.open(cache_sat).convert("RGB")
    else:
        sat_img = fetch_satellite_image(MIN_LAT, MAX_LAT, MIN_LON, MAX_LON, TOTAL_W, TOTAL_H)
        sat_img.save(cache_sat)
        print(f"[Satellite] Saved stitched satellite imagery to {cache_sat}")

    # 2. Fetch OSM highways
    cache_osm = os.path.join(out_dir, "capetown_highways.json")
    if os.path.exists(cache_osm):
        print(f"[OSM] Loading cached {cache_osm}...")
        with open(cache_osm) as f:
            highways = json.load(f)
    else:
        highways = fetch_osm_highways(MIN_LAT, MAX_LAT, MIN_LON, MAX_LON)
        if highways:
            with open(cache_osm, "w") as f:
                json.dump(highways, f)

    # 3. Build collision cell grid (240x240) and road overlay
    # TrackTile values: 0=OffRoad, 1=Tarmac, 2=Curb, 3=Barrier
    collision_grid = np.zeros((TOTAL_CELLS_Y, TOTAL_CELLS_X), dtype=np.uint8)

    # Highway classification & rendering widths
    # Major roads: Helen Suzman Blvd, Beach Rd, Western Blvd, Buitengracht St, Somerset Rd
    major_types = {"primary", "trunk", "secondary", "motorway"}
    mid_types = {"tertiary", "residential", "unclassified"}

    road_mask_canvas = Image.new("L", (TOTAL_CELLS_X, TOTAL_CELLS_Y), 0)
    draw_collision = ImageDraw.Draw(road_mask_canvas)

    # Overlay for high-res visual satellite tinting
    vis_overlay = sat_img.copy()
    draw_vis = ImageDraw.Draw(vis_overlay)

    def to_cell_xy(lat, lon):
        cx = (lon - MIN_LON) / (MAX_LON - MIN_LON) * TOTAL_CELLS_X
        cy = (MAX_LAT - lat) / (MAX_LAT - MIN_LAT) * TOTAL_CELLS_Y
        return (cx, cy)

    def to_texel_xy(lat, lon):
        tx = (lon - MIN_LON) / (MAX_LON - MIN_LON) * TOTAL_W
        ty = (MAX_LAT - lat) / (MAX_LAT - MIN_LAT) * TOTAL_H
        return (tx, ty)

    road_count = 0
    major_roads = []
    for hw in highways:
        geom = hw.get("geometry", [])
        if len(geom) < 2:
            continue
        hw_type = hw.get("tags", {}).get("highway", "")
        name = hw.get("tags", {}).get("name", "")
        
        # Determine road width in cells and texels
        if hw_type in major_types:
            cell_w = 4
            tex_w = 16
            color = (55, 60, 72)
            curb_color = (200, 205, 215)
        elif hw_type in mid_types:
            cell_w = 2
            tex_w = 10
            color = (65, 70, 80)
            curb_color = (180, 185, 195)
        elif hw_type in {"service", "living_street", "track"}:
            cell_w = 1
            tex_w = 6
            color = (75, 78, 85)
            curb_color = None
        else:
            continue

        road_count += 1
        cell_pts = [to_cell_xy(p["lat"], p["lon"]) for p in geom]
        tex_pts = [to_texel_xy(p["lat"], p["lon"]) for p in geom]

        # Draw collision mask
        draw_collision.line(cell_pts, fill=255, width=cell_w)

        # Draw enhanced visual road markings on satellite overlay
        if curb_color:
            draw_vis.line(tex_pts, fill=curb_color, width=tex_w + 4)
        draw_vis.line(tex_pts, fill=color, width=tex_w)

    print(f"[Compiler] Rasterized {road_count} road segments across Cape Town map.")

    # Convert collision mask to TrackTile array (1 = Tarmac, 0 = OffRoad)
    mask_np = np.array(road_mask_canvas)
    collision_grid[mask_np > 128] = 1  # TrackTile::Tarmac

    # Add curbs around roads
    curb_mask = (Image.fromarray(mask_np).filter(ImageFilter.MaxFilter(3)))
    curb_np = np.array(curb_mask)
    curb_only = (curb_np > 64) & (collision_grid == 0)
    collision_grid[curb_only] = 2  # TrackTile::Curb

    # Save full collision map
    col_out_path = os.path.join(out_dir, "capetown_10km_collision.bin")
    with open(col_out_path, "wb") as f_col:
        f_col.write(collision_grid.tobytes())
    print(f"[Compiler] Saved collision grid ({TOTAL_CELLS_X}x{TOTAL_CELLS_Y} = {len(collision_grid.tobytes())} bytes) -> {col_out_path}")

    # Save full blended visual map
    vis_out_path = os.path.join(out_dir, "capetown_10km_visual_full.png")
    vis_overlay.save(vis_out_path)
    print(f"[Compiler] Saved composite visual map -> {vis_out_path}")

    # 4. Slice into 3x3 grid of 1024x1024 blocks and compress to <= 200 KB JPEGs
    manifest_entries = []
    print("[Compiler] Slicing into 9 1024x1024 JPEG blocks with DRI=4 restart markers...")
    
    bin_path = os.path.join(out_dir, "CAPETOWN.BIN")
    with open(bin_path, "wb") as f_bin:
        current_sector = 0
        SECTOR_SIZE = 2048
        SECTORS_PER_BLOCK = 100

        for by in range(GRID_BLOCKS_Y):
            for bx in range(GRID_BLOCKS_X):
                block_idx = by * GRID_BLOCKS_X + bx
                block_name = f"capetown_b{bx}_b{by}"
                
                # Crop 1024x1024 region
                crop_rect = (bx * BLOCK_DIM, by * BLOCK_DIM, (bx + 1) * BLOCK_DIM, (by + 1) * BLOCK_DIM)
                block_im = vis_overlay.crop(crop_rect)

                # Compress to <= 200 KB baseline JPEG with DRI=4
                jpeg_data = compress_1024_jpeg(block_im)
                b_len = len(jpeg_data)

                # Export individual JPEG for inspection in tracks/capetown_10km/ and dist/
                jpg_inspect1 = os.path.join(tracks_dir, f"{block_name}.jpg")
                jpg_inspect2 = os.path.join(out_dir, f"{block_name}.jpg")
                with open(jpg_inspect1, "wb") as fj:
                    fj.write(jpeg_data)
                with open(jpg_inspect2, "wb") as fj:
                    fj.write(jpeg_data)

                # Write to CAPETOWN.BIN aligned to 100 CD sectors
                pad_len = (SECTORS_PER_BLOCK * SECTOR_SIZE) - b_len
                f_bin.write(jpeg_data)
                if pad_len > 0:
                    f_bin.write(b"\x00" * pad_len)

                manifest_entries.append({
                    "block_x": bx,
                    "block_y": by,
                    "sector_offset": current_sector,
                    "sector_count": SECTORS_PER_BLOCK,
                    "byte_len": b_len,
                    "file": f"{block_name}.jpg",
                    "kb": round(b_len / 1024.0, 2),
                })
                current_sector += SECTORS_PER_BLOCK
                print(f"  Block ({bx}, {by}) -> {block_name}.jpg: {b_len} bytes ({b_len/1024:.2f} KB, sectors {current_sector-SECTORS_PER_BLOCK}..{current_sector})")

    # 5. Emit JSON manifest and metadata
    manifest = {
        "name": "Cape Town Atlantic Seaboard & Green Point",
        "area_km2": 10.0,
        "width_km": 3.162,
        "height_km": 3.162,
        "bbox": {
            "min_lat": MIN_LAT,
            "min_lon": MIN_LON,
            "max_lat": MAX_LAT,
            "max_lon": MAX_LON,
        },
        "blocks_x": GRID_BLOCKS_X,
        "blocks_y": GRID_BLOCKS_Y,
        "block_dim": BLOCK_DIM,
        "total_texels_w": TOTAL_W,
        "total_texels_h": TOTAL_H,
        "cells_x": TOTAL_CELLS_X,
        "cells_y": TOTAL_CELLS_Y,
        "sectors_per_block": 100,
        "blocks": manifest_entries,
    }
    manifest_path = os.path.join(out_dir, "capetown_manifest.json")
    with open(manifest_path, "w") as fm:
        json.dump(manifest, fm, indent=2)

    print(f"\nSUCCESS! Mastered Cape Town 10 km^2 CD container:")
    print(f"  Container: {bin_path} ({current_sector} sectors = {current_sector * 2048 / 1024:.1f} KB)")
    print(f"  Manifest:  {manifest_path}")
    print(f"  JPEG blocks exported to {tracks_dir}/ and {out_dir}/")

if __name__ == "__main__":
    main()
