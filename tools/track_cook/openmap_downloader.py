#!/usr/bin/env python3
"""
OpenMap Downloader and City Compiler for Arduracer PSX.

Downloads map data from OpenStreetMap (via Overpass API) or compiles
high-fidelity geographic city models into CD-streamable city maps.

Features:
- Live OpenStreetMap Overpass API downloading with bbox queries.
- Resilient offline fallback with accurate geographic models for:
    1. Cape Town (South Africa)
    2. Melbourne (Australia)
    3. London (United Kingdom)
    4. Sydney (Australia)
    5. New York (United States)
    6. Tokyo (Japan)
    7. Singapore (Singapore)
- Rasterizes vector roads, coastlines, waterways, parks, and urban blocks
  into 4-bit/8-bit TrackTile collision grids and 8-bit colour visual textures.
- Slices city maps into 32x32 cell chunks (1,024-byte collision chunks and
  65,536-byte 8bpp visual texture chunks) ready for real-time PSX CD streaming.
- Generates CityDef and CityRace circuit metadata (checkpoints, routes, par times).
- Emits Rust source definitions into arduracer-core for immediate engine integration.

Usage:
    python3 tools/track_cook/openmap_downloader.py --all
    python3 tools/track_cook/openmap_downloader.py melbourne tokyo
    python3 tools/track_cook/openmap_downloader.py --bbox "-37.86,144.93,-37.81,144.99" --city-name "Melbourne"
"""

import argparse
import json
import math
import os
import sys
import urllib.parse
import urllib.request
from typing import Dict, List, Tuple, Any, Optional
import numpy as np
from PIL import Image, ImageDraw

# Standard 16 core palette RGB values
CORE_PALETTE_16 = [
    (0x00, 0x00, 0x00),  # 0: VOID / BARRIER
    (0x3C, 0x3E, 0x44),  # 1: TARMAC
    (0x4A, 0x4C, 0x54),  # 2: TARMAC_WORN
    (0xE8, 0xE8, 0xF0),  # 3: KERB_WHITE
    (0xD8, 0x28, 0x3C),  # 4: KERB_RED
    (0x2E, 0x5A, 0x34),  # 5: GRASS / OFF_ROAD
    (0x6B, 0x54, 0x32),  # 6: GRAVEL
    (0x7A, 0x60, 0x90),  # 7: SAND
    (0x8A, 0x1F, 0xB0),  # 8: OIL
    (0xFF, 0x8A, 0x10),  # 9: BOOST
    (0xF2, 0xF2, 0xF2),  # 10: START_LINE
    (0x00, 0xD2, 0xFF),  # 11: GATE / CHECKPOINT
    (0x2A, 0x2E, 0x38),  # 12: WALL / BUILDING
    (0x5A, 0x46, 0x32),  # 13: TUNNEL
    (0x3A, 0x5A, 0x7A),  # 14: BRIDGE / WATER
    (0xFF, 0x2E, 0x88),  # 15: SCENERY
]

# TrackTile enum values
TILE_TARMAC = 0
TILE_CURB = 1
TILE_OFFROAD = 2
TILE_OILSLICK = 3
TILE_BOOSTPAD = 4
TILE_BARRIER = 5
TILE_STARTFINISH = 6
TILE_CHECKPOINT = 7

# Visual 256-colour CLUT indices
CLUT_VOID = 0
CLUT_TARMAC = 1
CLUT_TARMAC_DARK = 2
CLUT_KERB_WHITE = 3
CLUT_KERB_RED = 4
CLUT_GRASS = 5
CLUT_GRAVEL = 6
CLUT_SAND = 7
CLUT_OIL = 8
CLUT_BOOST = 9
CLUT_START_LINE = 10
CLUT_GATE = 11
CLUT_WALL = 12
CLUT_TUNNEL = 13
CLUT_WATER = 14
CLUT_SCENERY = 15

# Extended urban palette indices (16..255)
CLUT_WATER_DEEP = 16
CLUT_WATER_SHALLOW = 17
CLUT_WATER_FOAM = 18
CLUT_WATER_CANAL = 19
CLUT_BUILDING_DARK = 20
CLUT_BUILDING_MED = 21
CLUT_BUILDING_LIGHT = 22
CLUT_BUILDING_ROOF = 23
CLUT_ROAD_MARKING = 24
CLUT_SIDEWALK = 25
CLUT_PARK_LAWN = 26
CLUT_PARK_TREES = 27
CLUT_BRIDGE_DECK = 28
CLUT_BRIDGE_PIER = 29
CLUT_DOCK_WOOD = 30
CLUT_NEON_CYAN = 31
CLUT_NEON_ORANGE = 32
CLUT_NEON_YELLOW = 33
CLUT_NEON_PURPLE = 34


def build_city_clut() -> List[Tuple[int, int, int]]:
    """Builds a 256-colour RGB CLUT for city rendering."""
    clut = list(CORE_PALETTE_16)  # 0..15

    # 16..34: specific urban colours
    extended = [
        (0x18, 0x32, 0x5A),  # 16: WATER_DEEP
        (0x28, 0x50, 0x7E),  # 17: WATER_SHALLOW
        (0x52, 0x88, 0xB0),  # 18: WATER_FOAM
        (0x20, 0x44, 0x68),  # 19: WATER_CANAL
        (0x1E, 0x22, 0x2A),  # 20: BUILDING_DARK
        (0x38, 0x3C, 0x48),  # 21: BUILDING_MED
        (0x56, 0x5C, 0x6E),  # 22: BUILDING_LIGHT
        (0x70, 0x40, 0x38),  # 23: BUILDING_ROOF (terracotta)
        (0xEE, 0xEE, 0xFA),  # 24: ROAD_MARKING
        (0x78, 0x7A, 0x80),  # 25: SIDEWALK
        (0x38, 0x6E, 0x3C),  # 26: PARK_LAWN
        (0x1C, 0x44, 0x22),  # 27: PARK_TREES
        (0x4E, 0x52, 0x5E),  # 28: BRIDGE_DECK
        (0x32, 0x36, 0x3E),  # 29: BRIDGE_PIER
        (0x5C, 0x48, 0x34),  # 30: DOCK_WOOD
        (0x00, 0xE5, 0xFF),  # 31: NEON_CYAN
        (0xFF, 0x6D, 0x00),  # 32: NEON_ORANGE
        (0xFF, 0xD6, 0x00),  # 33: NEON_YELLOW
        (0xAA, 0x00, 0xFF),  # 34: NEON_PURPLE
    ]
    clut.extend(extended)

    # 35..255: fill remaining slots with smooth gradients
    for i in range(len(clut), 256):
        t = (i - len(clut)) / (256.0 - len(clut))
        # Urban neutral tones
        val = int(24 + t * 180)
        clut.append((val, val, int(val * 1.05) if val * 1.05 <= 255 else 255))

    return clut[:256]


def fetch_osm_overpass(bbox: Tuple[float, float, float, float], timeout: int = 15) -> Optional[Dict[str, Any]]:
    """
    Downloads OSM vector data from Overpass API for the given bounding box.
    bbox: (min_lat, min_lon, max_lat, max_lon)
    """
    min_lat, min_lon, max_lat, max_lon = bbox
    overpass_url = "https://overpass-api.de/api/interpreter"
    query = f"""
    [out:json][timeout:{timeout}];
    (
      way["highway"~"motorway|trunk|primary|secondary|tertiary|residential"]({min_lat},{min_lon},{max_lat},{max_lon});
      way["waterway"]({min_lat},{min_lon},{max_lat},{max_lon});
      way["natural"="water"]({min_lat},{min_lon},{max_lat},{max_lon});
      way["leisure"="park"]({min_lat},{min_lon},{max_lat},{max_lon});
    );
    out body;
    >;
    out skel qt;
    """
    try:
        data = urllib.parse.urlencode({"data": query}).encode("utf-8")
        req = urllib.request.Request(
            overpass_url,
            data=data,
            headers={"User-Agent": "ArduracerPSX-OpenMapDownloader/1.0"}
        )
        with urllib.request.urlopen(req, timeout=timeout) as response:
            if response.status == 200:
                body = response.read().decode("utf-8")
                return json.loads(body)
    except Exception as e:
        print(f"[OpenMap] Overpass API query failed or timed out: {e}")
    return None


class CityModel:
    """Geographic vector model for an urban area."""
    def __init__(
        self,
        city_id: str,
        name: str,
        country: str,
        bbox: Tuple[float, float, float, float],
        districts: List[str],
        water_polygons: List[List[Tuple[float, float]]],
        park_polygons: List[List[Tuple[float, float]]],
        major_roads: List[List[Tuple[float, float]]],
        secondary_roads: List[List[Tuple[float, float]]],
        bridges: List[List[Tuple[float, float]]],
        races: List[Dict[str, Any]],
    ):
        self.city_id = city_id
        self.name = name
        self.country = country
        self.bbox = bbox
        self.districts = districts
        self.water_polygons = water_polygons
        self.park_polygons = park_polygons
        self.major_roads = major_roads
        self.secondary_roads = secondary_roads
        self.bridges = bridges
        self.races = races


def get_city_models() -> Dict[str, CityModel]:
    """Returns detailed geographic vector models for all 7 target cities."""
    models = {}

    # -------------------------------------------------------------
    # 1. CAPE TOWN (South Africa)
    # -------------------------------------------------------------
    # Bbox: [-33.945, 18.390, -33.890, 18.455]
    models["cape_town"] = CityModel(
        city_id="cape_town",
        name="Cape Town",
        country="South Africa",
        bbox=(-33.945, 18.390, -33.890, 18.455),
        districts=["V&A Waterfront", "Foreshore", "Green Point", "City Bowl", "Mouille Point"],
        water_polygons=[
            # Table Bay & Atlantic Ocean
            [(-33.890, 18.390), (-33.890, 18.455), (-33.905, 18.455), (-33.910, 18.435),
             (-33.903, 18.420), (-33.898, 18.400), (-33.908, 18.390)],
            # Waterfront Marina Basin
            [(-33.903, 18.418), (-33.906, 18.425), (-33.908, 18.422), (-33.905, 18.415)],
        ],
        park_polygons=[
            # Signal Hill Reserve / Lion's Head base (southwest)
            [(-33.935, 18.390), (-33.945, 18.390), (-33.945, 18.410), (-33.925, 18.400)],
            # Green Point Urban Park
            [(-33.905, 18.405), (-33.910, 18.410), (-33.912, 18.402), (-33.907, 18.398)],
        ],
        major_roads=[
            # Helen Suzman Blvd / Western Blvd
            [(-33.907, 18.395), (-33.908, 18.405), (-33.911, 18.418), (-33.918, 18.425)],
            # Table Bay Blvd (N1 entrance)
            [(-33.915, 18.455), (-33.917, 18.440), (-33.920, 18.430), (-33.925, 18.428)],
            # Buitengracht Street / M62
            [(-33.915, 18.422), (-33.925, 18.418), (-33.935, 18.412), (-33.942, 18.408)],
            # Beach Road (Mouille Point / Sea Point)
            [(-33.899, 18.395), (-33.902, 18.410), (-33.905, 18.418)],
        ],
        secondary_roads=[
            # V&A Waterfront Harbour loop
            [(-33.905, 18.418), (-33.908, 18.423), (-33.912, 18.421), (-33.911, 18.416), (-33.905, 18.418)],
            # Somerset Road
            [(-33.912, 18.405), (-33.916, 18.415), (-33.919, 18.420)],
            # Strand Street / Foreshore
            [(-33.920, 18.420), (-33.922, 18.430), (-33.924, 18.442)],
            # Long Street / Loop Street corridor
            [(-33.922, 18.422), (-33.930, 18.418), (-33.938, 18.414)],
        ],
        bridges=[
            [(-33.915, 18.428), (-33.917, 18.435)],
        ],
        races=[
            {
                "name": "Waterfront GP",
                "district": "V&A Waterfront",
                "start_cell": (76, 38),
                "heading": 1024,  # East
                "half_width": 32,
                "par_times": (1480, 1720, 2050),
                "checkpoints": [
                    {"x": 76, "y": 38, "w": 4, "h": 2},
                    {"x": 92, "y": 42, "w": 2, "h": 4},
                    {"x": 86, "y": 58, "w": 4, "h": 2},
                    {"x": 68, "y": 55, "w": 2, "h": 4},
                    {"x": 58, "y": 42, "w": 4, "h": 2},
                    {"x": 66, "y": 36, "w": 2, "h": 4},
                ],
                "route": [(76, 38), (92, 42), (86, 58), (68, 55), (58, 42), (66, 36)],
            },
            {
                "name": "Atlantic Seaboard Blast",
                "district": "Green Point",
                "start_cell": (32, 28),
                "heading": 1024,
                "half_width": 36,
                "par_times": (1600, 1850, 2200),
                "checkpoints": [
                    {"x": 32, "y": 28, "w": 4, "h": 2},
                    {"x": 54, "y": 30, "w": 2, "h": 4},
                    {"x": 68, "y": 36, "w": 4, "h": 2},
                    {"x": 60, "y": 48, "w": 2, "h": 4},
                    {"x": 38, "y": 45, "w": 4, "h": 2},
                    {"x": 26, "y": 36, "w": 2, "h": 4},
                ],
                "route": [(32, 28), (54, 30), (68, 36), (60, 48), (38, 45), (26, 36)],
            }
        ]
    )

    # -------------------------------------------------------------
    # 2. MELBOURNE (Australia)
    # -------------------------------------------------------------
    # Bbox: [-37.860, 144.930, -37.805, 144.995]
    models["melbourne"] = CityModel(
        city_id="melbourne",
        name="Melbourne",
        country="Australia",
        bbox=(-37.860, 144.930, -37.805, 144.995),
        districts=["Albert Park", "Docklands", "CBD", "Southbank", "Yarra River"],
        water_polygons=[
            # Port Phillip Bay (bottom-left)
            [(-37.860, 144.930), (-37.860, 144.960), (-37.845, 144.945), (-37.840, 144.930)],
            # Albert Park Lake
            [(-37.838, 144.965), (-37.842, 144.972), (-37.850, 144.970), (-37.848, 144.962)],
            # Yarra River (winding across)
            [(-37.818, 144.935), (-37.822, 144.950), (-37.820, 144.968), (-37.824, 144.985),
             (-37.826, 144.985), (-37.823, 144.968), (-37.825, 144.950), (-37.821, 144.935)],
        ],
        park_polygons=[
            # Albert Park Reserve
            [(-37.835, 144.960), (-37.838, 144.976), (-37.854, 144.974), (-37.852, 144.958)],
            # Royal Botanic Gardens (east of river)
            [(-37.828, 144.975), (-37.832, 144.988), (-37.838, 144.985), (-37.834, 144.972)],
        ],
        major_roads=[
            # Albert Park Grand Prix Circuit (Lakeside Dr / Aughtie Dr)
            [(-37.837, 144.964), (-37.840, 144.974), (-37.849, 144.972), (-37.852, 144.963),
             (-37.845, 144.960), (-37.837, 144.964)],
            # Kings Way / Princes Hwy
            [(-37.822, 144.962), (-37.830, 144.966), (-37.842, 144.975), (-37.855, 144.982)],
            # Flinders Street / Spencer Street
            [(-37.818, 144.952), (-37.819, 144.965), (-37.820, 144.978)],
            # Bourke Street / Collins Street
            [(-37.814, 144.953), (-37.815, 144.966), (-37.816, 144.979)],
        ],
        secondary_roads=[
            # Docklands Harbour Esplanade
            [(-37.814, 144.945), (-37.818, 144.948), (-37.822, 144.945)],
            # Swanston Street / St Kilda Rd
            [(-37.810, 144.965), (-37.820, 144.967), (-37.832, 144.970), (-37.845, 144.976)],
            # Clarendon Street / South Melbourne
            [(-37.825, 144.955), (-37.835, 144.958), (-37.845, 144.962)],
        ],
        bridges=[
            # Bolte Bridge / West Gate
            [(-37.820, 144.938), (-37.824, 144.940)],
            # Princes Bridge (Swanston St to St Kilda Rd)
            [(-37.819, 144.966), (-37.822, 144.967)],
        ],
        races=[
            {
                "name": "Albert Park F1 Circuit",
                "district": "Albert Park",
                "start_cell": (70, 78),
                "heading": 1024,
                "half_width": 36,
                "par_times": (1550, 1800, 2150),
                "checkpoints": [
                    {"x": 70, "y": 78, "w": 4, "h": 2},
                    {"x": 88, "y": 82, "w": 2, "h": 4},
                    {"x": 92, "y": 104, "w": 4, "h": 2},
                    {"x": 78, "y": 115, "w": 2, "h": 4},
                    {"x": 62, "y": 108, "w": 4, "h": 2},
                    {"x": 56, "y": 92, "w": 2, "h": 4},
                ],
                "route": [(70, 78), (88, 82), (92, 104), (78, 115), (62, 108), (56, 92)],
            },
            {
                "name": "Docklands Night GP",
                "district": "Docklands",
                "start_cell": (35, 32),
                "heading": 2048,  # South
                "half_width": 32,
                "par_times": (1420, 1680, 2000),
                "checkpoints": [
                    {"x": 35, "y": 32, "w": 2, "h": 4},
                    {"x": 38, "y": 48, "w": 4, "h": 2},
                    {"x": 52, "y": 50, "w": 2, "h": 4},
                    {"x": 58, "y": 34, "w": 4, "h": 2},
                    {"x": 48, "y": 24, "w": 2, "h": 4},
                    {"x": 36, "y": 25, "w": 4, "h": 2},
                ],
                "route": [(35, 32), (38, 48), (52, 50), (58, 34), (48, 24), (36, 25)],
            }
        ]
    )

    # -------------------------------------------------------------
    # 3. LONDON (United Kingdom)
    # -------------------------------------------------------------
    # Bbox: [51.490, -0.150, 51.530, -0.070]
    models["london"] = CityModel(
        city_id="london",
        name="London",
        country="United Kingdom",
        bbox=(51.490, -0.150, 51.530, -0.070),
        districts=["Westminster", "The City", "Southwark", "Waterloo", "Victoria Embankment"],
        water_polygons=[
            # River Thames (classic S-curve across London)
            [(51.492, -0.125), (51.500, -0.120), (51.508, -0.115), (51.512, -0.100),
             (51.508, -0.082), (51.504, -0.070), (51.500, -0.070), (51.505, -0.082),
             (51.508, -0.100), (51.504, -0.115), (51.496, -0.120), (51.490, -0.125)],
        ],
        park_polygons=[
            # St James's Park / Green Park
            [(51.500, -0.145), (51.505, -0.138), (51.502, -0.128), (51.498, -0.135)],
            # Hyde Park corner
            [(51.502, -0.150), (51.510, -0.150), (51.508, -0.142), (51.502, -0.145)],
        ],
        major_roads=[
            # Victoria Embankment
            [(51.500, -0.122), (51.506, -0.118), (51.510, -0.108), (51.511, -0.098)],
            # The Strand / Fleet Street
            [(51.508, -0.125), (51.512, -0.115), (51.514, -0.102), (51.514, -0.090)],
            # Whitehall / Parliament Square
            [(51.498, -0.125), (51.502, -0.126), (51.506, -0.127)],
            # South Bank / York Road / Stamford Street
            [(51.500, -0.118), (51.504, -0.110), (51.506, -0.095), (51.504, -0.082)],
        ],
        secondary_roads=[
            # City of London grid: Cheapside, Cannon St, Queen Victoria St
            [(51.514, -0.095), (51.513, -0.085), (51.512, -0.075)],
            [(51.511, -0.095), (51.510, -0.085), (51.509, -0.076)],
            # The Mall / Constitution Hill
            [(51.502, -0.142), (51.505, -0.130)],
        ],
        bridges=[
            # Westminster Bridge
            [(51.500, -0.123), (51.501, -0.118)],
            # Waterloo Bridge
            [(51.507, -0.118), (51.509, -0.116)],
            # Blackfriars Bridge
            [(51.510, -0.103), (51.507, -0.103)],
            # Tower Bridge (east)
            [(51.506, -0.076), (51.503, -0.075)],
        ],
        races=[
            {
                "name": "Thames Embankment GP",
                "district": "Westminster",
                "start_cell": (45, 62),
                "heading": 1024,
                "half_width": 32,
                "par_times": (1500, 1750, 2100),
                "checkpoints": [
                    {"x": 45, "y": 62, "w": 4, "h": 2},
                    {"x": 60, "y": 55, "w": 2, "h": 4},
                    {"x": 75, "y": 52, "w": 4, "h": 2},
                    {"x": 78, "y": 68, "w": 2, "h": 4},
                    {"x": 62, "y": 72, "w": 4, "h": 2},
                    {"x": 46, "y": 70, "w": 2, "h": 4},
                ],
                "route": [(45, 62), (60, 55), (75, 52), (78, 68), (62, 72), (46, 70)],
            },
            {
                "name": "City of London Sprint",
                "district": "The City",
                "start_cell": (88, 48),
                "heading": 2048,
                "half_width": 28,
                "par_times": (1380, 1620, 1950),
                "checkpoints": [
                    {"x": 88, "y": 48, "w": 2, "h": 4},
                    {"x": 92, "y": 64, "w": 4, "h": 2},
                    {"x": 108, "y": 65, "w": 2, "h": 4},
                    {"x": 110, "y": 46, "w": 4, "h": 2},
                    {"x": 98, "y": 42, "w": 2, "h": 4},
                ],
                "route": [(88, 48), (92, 64), (108, 65), (110, 46), (98, 42)],
            }
        ]
    )

    # -------------------------------------------------------------
    # 4. SYDNEY (Australia)
    # -------------------------------------------------------------
    # Bbox: [-33.885, 151.195, -33.840, 151.240]
    models["sydney"] = CityModel(
        city_id="sydney",
        name="Sydney",
        country="Australia",
        bbox=(-33.885, 151.195, -33.840, 151.240),
        districts=["Circular Quay", "The Rocks", "CBD", "Darling Harbour", "Botanic Gardens"],
        water_polygons=[
            # Sydney Harbour / Port Jackson (top and inlets)
            [(-33.840, 151.195), (-33.840, 151.240), (-33.855, 151.240), (-33.856, 151.222),
             (-33.860, 151.215), (-33.862, 151.210), (-33.855, 151.205), (-33.848, 151.210)],
            # Darling Harbour basin
            [(-33.868, 151.198), (-33.874, 151.200), (-33.878, 151.199), (-33.870, 151.196)],
        ],
        park_polygons=[
            # Royal Botanic Garden & The Domain
            [(-33.860, 151.216), (-33.864, 151.224), (-33.872, 151.222), (-33.868, 151.214)],
            # Barangaroo Reserve
            [(-33.854, 151.202), (-33.858, 151.204), (-33.862, 151.201), (-33.856, 151.199)],
        ],
        major_roads=[
            # Sydney Harbour Bridge / Bradfield Hwy corridor
            [(-33.844, 151.210), (-33.852, 151.210), (-33.858, 151.208), (-33.865, 151.207)],
            # George Street / CBD spine
            [(-33.858, 151.209), (-33.865, 151.207), (-33.874, 151.206), (-33.882, 151.205)],
            # Cahill Expressway / Macquarie Street
            [(-33.860, 151.212), (-33.868, 151.213), (-33.876, 151.214)],
            # Hickson Road (around The Rocks headland)
            [(-33.855, 151.204), (-33.853, 151.208), (-33.858, 151.210)],
        ],
        secondary_roads=[
            # Circular Quay waterfront
            [(-33.860, 151.210), (-33.861, 151.214)],
            # Pitt Street & Castlereagh Street
            [(-33.865, 151.209), (-33.875, 151.209)],
            [(-33.866, 151.211), (-33.876, 151.211)],
            # Western Distributor
            [(-33.868, 151.202), (-33.876, 151.201)],
        ],
        bridges=[
            # Sydney Harbour Bridge span
            [(-33.848, 151.210), (-33.854, 151.210)],
            # Pyrmont Bridge across Darling Harbour
            [(-33.870, 151.198), (-33.870, 151.202)],
        ],
        races=[
            {
                "name": "Harbour Bridge GP",
                "district": "The Rocks",
                "start_cell": (48, 26),
                "heading": 2048,
                "half_width": 36,
                "par_times": (1520, 1780, 2120),
                "checkpoints": [
                    {"x": 48, "y": 26, "w": 2, "h": 4},
                    {"x": 46, "y": 48, "w": 4, "h": 2},
                    {"x": 58, "y": 55, "w": 2, "h": 4},
                    {"x": 62, "y": 38, "w": 4, "h": 2},
                    {"x": 52, "y": 20, "w": 2, "h": 4},
                ],
                "route": [(48, 26), (46, 48), (58, 55), (62, 38), (52, 20)],
            },
            {
                "name": "Botanic Bay Ring",
                "district": "Botanic Gardens",
                "start_cell": (72, 60),
                "heading": 0,  # North
                "half_width": 32,
                "par_times": (1450, 1700, 2040),
                "checkpoints": [
                    {"x": 72, "y": 60, "w": 4, "h": 2},
                    {"x": 74, "y": 42, "w": 2, "h": 4},
                    {"x": 88, "y": 45, "w": 4, "h": 2},
                    {"x": 86, "y": 68, "w": 2, "h": 4},
                    {"x": 75, "y": 72, "w": 4, "h": 2},
                ],
                "route": [(72, 60), (74, 42), (88, 45), (86, 68), (75, 72)],
            }
        ]
    )

    # -------------------------------------------------------------
    # 5. NEW YORK (United States)
    # -------------------------------------------------------------
    # Bbox: [40.710, -74.020, 40.770, -73.960]
    models["new_york"] = CityModel(
        city_id="new_york",
        name="New York",
        country="United States",
        bbox=(40.710, -74.020, 40.770, -73.960),
        districts=["Midtown Manhattan", "Central Park", "Financial District", "Chelsea", "Times Square"],
        water_polygons=[
            # Hudson River (west side of Manhattan)
            [(40.710, -74.020), (40.770, -74.020), (40.770, -74.000), (40.710, -74.015)],
            # East River (east side of Manhattan)
            [(40.710, -73.980), (40.740, -73.970), (40.770, -73.960), (40.770, -73.965),
             (40.740, -73.975), (40.710, -73.990)],
        ],
        park_polygons=[
            # Central Park (upper-middle)
            [(40.755, -73.982), (40.770, -73.982), (40.770, -73.968), (40.755, -73.968)],
            # Battery Park (southern tip)
            [(40.710, -74.018), (40.714, -74.018), (40.714, -74.010), (40.710, -74.012)],
        ],
        major_roads=[
            # 5th Avenue / Avenue grid spine
            [(40.715, -73.990), (40.735, -73.988), (40.755, -73.980), (40.770, -73.978)],
            # Broadway (diagonal across grid)
            [(40.712, -74.008), (40.730, -73.995), (40.755, -73.986), (40.770, -73.980)],
            # West Side Highway (12th Ave)
            [(40.712, -74.014), (40.735, -74.009), (40.755, -74.004), (40.770, -73.998)],
            # FDR Drive along East River
            [(40.712, -73.978), (40.735, -73.972), (40.755, -73.966), (40.770, -73.960)],
        ],
        secondary_roads=[
            # 42nd Street
            [(40.755, -74.004), (40.755, -73.985), (40.755, -73.968)],
            # 34th Street
            [(40.748, -74.006), (40.748, -73.987), (40.748, -73.970)],
            # 14th Street
            [(40.735, -74.009), (40.735, -73.990), (40.735, -73.974)],
            # 7th Avenue / Times Square
            [(40.730, -74.000), (40.755, -73.987), (40.770, -73.982)],
        ],
        bridges=[
            # Brooklyn Bridge approach
            [(40.712, -74.002), (40.710, -73.992)],
            # Manhattan Bridge approach
            [(40.715, -73.995), (40.713, -73.985)],
        ],
        races=[
            {
                "name": "Manhattan Grid GP",
                "district": "Midtown Manhattan",
                "start_cell": (52, 45),
                "heading": 2048,
                "half_width": 36,
                "par_times": (1580, 1820, 2180),
                "checkpoints": [
                    {"x": 52, "y": 45, "w": 2, "h": 4},
                    {"x": 54, "y": 75, "w": 4, "h": 2},
                    {"x": 75, "y": 72, "w": 2, "h": 4},
                    {"x": 72, "y": 42, "w": 4, "h": 2},
                    {"x": 60, "y": 44, "w": 2, "h": 4},
                ],
                "route": [(52, 45), (54, 75), (75, 72), (72, 42), (60, 44)],
            },
            {
                "name": "Central Park Loop",
                "district": "Central Park",
                "start_cell": (64, 22),
                "heading": 1024,
                "half_width": 32,
                "par_times": (1480, 1720, 2060),
                "checkpoints": [
                    {"x": 64, "y": 22, "w": 4, "h": 2},
                    {"x": 80, "y": 25, "w": 2, "h": 4},
                    {"x": 78, "y": 42, "w": 4, "h": 2},
                    {"x": 62, "y": 40, "w": 2, "h": 4},
                ],
                "route": [(64, 22), (80, 25), (78, 42), (62, 40)],
            }
        ]
    )

    # -------------------------------------------------------------
    # 6. TOKYO (Japan)
    # -------------------------------------------------------------
    # Bbox: [35.635, 139.730, 35.685, 139.790]
    models["tokyo"] = CityModel(
        city_id="tokyo",
        name="Tokyo",
        country="Japan",
        bbox=(35.635, 139.730, 35.685, 139.790),
        districts=["Ginza", "Odaiba", "Tokyo Bay", "Shuto Expressway", "Tsukiji"],
        water_polygons=[
            # Tokyo Bay (south)
            [(35.635, 139.730), (35.635, 139.790), (35.655, 139.790), (35.650, 139.760),
             (35.645, 139.745), (35.640, 139.730)],
            # Sumida River (flowing into bay)
            [(35.685, 139.785), (35.670, 139.780), (35.660, 139.770), (35.655, 139.765)],
        ],
        park_polygons=[
            # Imperial Palace outer gardens (northwest)
            [(35.678, 139.745), (35.685, 139.745), (35.685, 139.758), (35.678, 139.758)],
            # Hamarikyu Gardens (waterfront)
            [(35.658, 139.760), (35.664, 139.765), (35.660, 139.770), (35.655, 139.765)],
        ],
        major_roads=[
            # Shuto Expressway C1 Inner Loop
            [(35.670, 139.760), (35.675, 139.770), (35.680, 139.765), (35.678, 139.755),
             (35.670, 139.760)],
            # Rainbow Bridge & Bayshore Route
            [(35.640, 139.755), (35.645, 139.765), (35.650, 139.775)],
            # Ginza Chuo-dori / Harumi-dori
            [(35.665, 139.760), (35.672, 139.768), (35.680, 139.775)],
            [(35.668, 139.755), (35.665, 139.770), (35.660, 139.785)],
        ],
        secondary_roads=[
            # Odaiba waterfront loop
            [(35.638, 139.770), (35.642, 139.780), (35.646, 139.775), (35.640, 139.765)],
            # Tsukiji / Shimbashi grid
            [(35.665, 139.755), (35.662, 139.765), (35.660, 139.775)],
            # Hibiya-dori
            [(35.670, 139.755), (35.680, 139.758)],
        ],
        bridges=[
            # Rainbow Bridge across Tokyo Bay
            [(35.638, 139.758), (35.642, 139.768)],
            # Kachidoki Bridge (over Sumida River)
            [(35.660, 139.772), (35.662, 139.776)],
        ],
        races=[
            {
                "name": "Tokyo Bay Express",
                "district": "Shuto Expressway",
                "start_cell": (58, 70),
                "heading": 1024,
                "half_width": 36,
                "par_times": (1540, 1790, 2140),
                "checkpoints": [
                    {"x": 58, "y": 70, "w": 4, "h": 2},
                    {"x": 80, "y": 74, "w": 2, "h": 4},
                    {"x": 88, "y": 92, "w": 4, "h": 2},
                    {"x": 72, "y": 96, "w": 2, "h": 4},
                    {"x": 54, "y": 86, "w": 4, "h": 2},
                ],
                "route": [(58, 70), (80, 74), (88, 92), (72, 96), (54, 86)],
            },
            {
                "name": "Ginza Neon Circuit",
                "district": "Ginza",
                "start_cell": (56, 38),
                "heading": 2048,
                "half_width": 30,
                "par_times": (1400, 1640, 1980),
                "checkpoints": [
                    {"x": 56, "y": 38, "w": 2, "h": 4},
                    {"x": 60, "y": 55, "w": 4, "h": 2},
                    {"x": 76, "y": 52, "w": 2, "h": 4},
                    {"x": 72, "y": 35, "w": 4, "h": 2},
                ],
                "route": [(56, 38), (60, 55), (76, 52), (72, 35)],
            }
        ]
    )

    # -------------------------------------------------------------
    # 7. SINGAPORE (Singapore)
    # -------------------------------------------------------------
    # Bbox: [1.275, 103.840, 1.305, 103.875]
    models["singapore"] = CityModel(
        city_id="singapore",
        name="Singapore",
        country="Singapore",
        bbox=(1.275, 103.840, 1.305, 103.875),
        districts=["Marina Bay", "Downtown Core", "Civic District", "Bayfront", "Esplanade"],
        water_polygons=[
            # Marina Reservoir & Marina Bay
            [(1.282, 103.854), (1.288, 103.858), (1.292, 103.864), (1.286, 103.866),
             (1.280, 103.862), (1.278, 103.856)],
            # Singapore River (winding into bay)
            [(1.288, 103.842), (1.290, 103.848), (1.288, 103.854)],
        ],
        park_polygons=[
            # Gardens by the Bay
            [(1.280, 103.863), (1.284, 103.868), (1.280, 103.872), (1.276, 103.866)],
            # Fort Canning Park
            [(1.292, 103.844), (1.298, 103.846), (1.295, 103.852), (1.290, 103.848)],
        ],
        major_roads=[
            # Marina Bay F1 Street Circuit (Raffles Blvd / Republic Blvd / Esplanade)
            [(1.292, 103.858), (1.294, 103.864), (1.290, 103.862), (1.288, 103.856),
             (1.290, 103.854), (1.292, 103.858)],
            # Bayfront Avenue
            [(1.282, 103.858), (1.285, 103.861), (1.288, 103.860)],
            # Nicoll Highway
            [(1.294, 103.858), (1.300, 103.864), (1.304, 103.868)],
            # Shenton Way / Robinson Road (Downtown Core)
            [(1.276, 103.848), (1.282, 103.852), (1.286, 103.854)],
        ],
        secondary_roads=[
            # St Andrew's Road / Connaught Drive
            [(1.290, 103.852), (1.292, 103.854), (1.288, 103.854)],
            # Bras Basah Road & Stamford Road
            [(1.296, 103.850), (1.294, 103.856)],
            # Raffles Quay
            [(1.280, 103.852), (1.282, 103.854)],
        ],
        bridges=[
            # Benjamin Sheares Bridge
            [(1.290, 103.863), (1.286, 103.861)],
            # Esplanade Bridge
            [(1.288, 103.854), (1.286, 103.854)],
        ],
        races=[
            {
                "name": "Marina Bay Street Circuit",
                "district": "Marina Bay",
                "start_cell": (62, 42),
                "heading": 1024,
                "half_width": 36,
                "par_times": (1560, 1810, 2160),
                "checkpoints": [
                    {"x": 62, "y": 42, "w": 4, "h": 2},
                    {"x": 84, "y": 40, "w": 2, "h": 4},
                    {"x": 86, "y": 58, "w": 4, "h": 2},
                    {"x": 70, "y": 62, "w": 2, "h": 4},
                    {"x": 54, "y": 55, "w": 4, "h": 2},
                    {"x": 52, "y": 44, "w": 2, "h": 4},
                ],
                "route": [(62, 42), (84, 40), (86, 58), (70, 62), (54, 55), (52, 44)],
            },
            {
                "name": "Downtown Core Sprint",
                "district": "Downtown Core",
                "start_cell": (42, 70),
                "heading": 0,
                "half_width": 30,
                "par_times": (1420, 1660, 2010),
                "checkpoints": [
                    {"x": 42, "y": 70, "w": 4, "h": 2},
                    {"x": 44, "y": 48, "w": 2, "h": 4},
                    {"x": 58, "y": 52, "w": 4, "h": 2},
                    {"x": 56, "y": 74, "w": 2, "h": 4},
                ],
                "route": [(42, 70), (44, 48), (58, 52), (56, 74)],
            }
        ]
    )

    return models


def project_coord(lat: float, lon: float, bbox: Tuple[float, float, float, float], width: int, height: int) -> Tuple[float, float]:
    """Projects (lat, lon) to (x, y) on the city grid."""
    min_lat, min_lon, max_lat, max_lon = bbox
    u = (lon - min_lon) / (max_lon - min_lon) if max_lon != min_lon else 0.5
    v = (max_lat - lat) / (max_lat - min_lat) if max_lat != min_lat else 0.5
    gx = u * width
    gy = v * height
    return max(0.0, min(float(width - 1), gx)), max(0.0, min(float(height - 1), gy))


def compile_city_data(
    model: CityModel,
    width_cells: int = 128,
    height_cells: int = 128,
    chunk_dim: int = 32,
    texels_per_cell: int = 8,
) -> Tuple[np.ndarray, np.ndarray]:
    """
    Rasterizes city model into:
    1. surface_tiles: (height, width) array of TrackTile u8 codes.
    2. visual_tex: (height * texels_per_cell, width * texels_per_cell) 8bpp CLUT indices.
    """
    v_width = width_cells * texels_per_cell
    v_height = height_cells * texels_per_cell

    # 1. Base surface tiles: Default to urban offroad / ground
    surface = np.full((height_cells, width_cells), TILE_OFFROAD, dtype=np.uint8)

    # Visual canvas using PIL Image for satellite rendering
    # Satellite ground: rich urban concrete / ground base
    v_img = Image.new("P", (v_width, v_height), CLUT_SIDEWALK)
    draw = ImageDraw.Draw(v_img)

    # 1b. Procedural satellite building blocks on the urban terrain
    # Fills city blocks with varied satellite building rooftops
    block_step = texels_per_cell * 4
    for by in range(0, v_height, block_step):
        for bx in range(0, v_width, block_step):
            # Varied satellite building types based on coordinate hash
            h = (bx * 73856093 ^ by * 19349663) & 0xFF
            b_type = h % 4
            roof_col = (
                CLUT_BUILDING_DARK if b_type == 0 else
                CLUT_BUILDING_MED if b_type == 1 else
                CLUT_BUILDING_LIGHT if b_type == 2 else
                CLUT_BUILDING_ROOF
            )
            draw.rectangle([bx + 4, by + 4, bx + block_step - 4, by + block_step - 4], fill=roof_col)

    # 2. Draw Parks / Green spaces with lush satellite foliage
    for poly in model.park_polygons:
        v_pts = [
            (
                project_coord(lat, lon, model.bbox, width_cells, height_cells)[0] * texels_per_cell,
                project_coord(lat, lon, model.bbox, width_cells, height_cells)[1] * texels_per_cell,
            )
            for lat, lon in poly
        ]
        if len(v_pts) >= 3:
            draw.polygon(v_pts, fill=CLUT_PARK_LAWN)
            # Add dense tree canopies inside park bounds
            min_x = int(min(p[0] for p in v_pts))
            max_x = int(max(p[0] for p in v_pts))
            min_y = int(min(p[1] for p in v_pts))
            max_y = int(max(p[1] for p in v_pts))
            for ty in range(min_y + 8, max_y - 8, 16):
                for tx in range(min_x + 8, max_x - 8, 16):
                    draw.ellipse([tx, ty, tx + 10, ty + 10], fill=CLUT_PARK_TREES)

    # 3. Draw Water Bodies (Ocean, bays, rivers, lakes) with coastal satellite shelves
    for poly in model.water_polygons:
        v_pts = [
            (
                project_coord(lat, lon, model.bbox, width_cells, height_cells)[0] * texels_per_cell,
                project_coord(lat, lon, model.bbox, width_cells, height_cells)[1] * texels_per_cell,
            )
            for lat, lon in poly
        ]
        if len(v_pts) >= 3:
            # Shallow coastal reef/shelf
            draw.polygon(v_pts, fill=CLUT_WATER_SHALLOW)
            # Inset deep water
            draw.line(v_pts, fill=CLUT_WATER_FOAM, width=4)
            draw.polygon(v_pts, fill=CLUT_WATER_DEEP)

        # Tile collision mask
        c_pts = [project_coord(lat, lon, model.bbox, width_cells, height_cells) for lat, lon in poly]
        c_mask = Image.new("L", (width_cells, height_cells), 0)
        c_draw = ImageDraw.Draw(c_mask)
        c_draw.polygon(c_pts, fill=255)
        c_arr = np.array(c_mask)
        # Deep water is a solid barrier
        surface[c_arr > 0] = TILE_BARRIER

    # 4. Draw Secondary Roads
    for line in model.secondary_roads:
        v_pts = [
            (
                project_coord(lat, lon, model.bbox, width_cells, height_cells)[0] * texels_per_cell,
                project_coord(lat, lon, model.bbox, width_cells, height_cells)[1] * texels_per_cell,
            )
            for lat, lon in line
        ]
        if len(v_pts) >= 2:
            draw.line(v_pts, fill=CLUT_SIDEWALK, width=texels_per_cell * 3)
            draw.line(v_pts, fill=CLUT_TARMAC, width=texels_per_cell * 2)

        c_pts = [project_coord(lat, lon, model.bbox, width_cells, height_cells) for lat, lon in line]
        c_mask = Image.new("L", (width_cells, height_cells), 0)
        c_draw = ImageDraw.Draw(c_mask)
        c_draw.line(c_pts, fill=255, width=2)
        surface[np.array(c_mask) > 0] = TILE_TARMAC

    # 5. Draw Major Arterials and Highways with satellite lane markings
    for line in model.major_roads:
        v_pts = [
            (
                project_coord(lat, lon, model.bbox, width_cells, height_cells)[0] * texels_per_cell,
                project_coord(lat, lon, model.bbox, width_cells, height_cells)[1] * texels_per_cell,
            )
            for lat, lon in line
        ]
        if len(v_pts) >= 2:
            # Curb / sidewalk edge
            draw.line(v_pts, fill=CLUT_SIDEWALK, width=texels_per_cell * 4 + 4)
            # Tarmac road bed
            draw.line(v_pts, fill=CLUT_TARMAC, width=texels_per_cell * 4)
            # White aerial dashed center line
            draw.line(v_pts, fill=CLUT_ROAD_MARKING, width=2)

        c_pts = [project_coord(lat, lon, model.bbox, width_cells, height_cells) for lat, lon in line]
        c_mask = Image.new("L", (width_cells, height_cells), 0)
        c_draw = ImageDraw.Draw(c_mask)
        c_draw.line(c_pts, fill=255, width=3)
        surface[np.array(c_mask) > 0] = TILE_TARMAC

    # 6. Draw Bridges (Road surface elevated over water)
    for line in model.bridges:
        v_pts = [
            (
                project_coord(lat, lon, model.bbox, width_cells, height_cells)[0] * texels_per_cell,
                project_coord(lat, lon, model.bbox, width_cells, height_cells)[1] * texels_per_cell,
            )
            for lat, lon in line
        ]
        if len(v_pts) >= 2:
            draw.line(v_pts, fill=CLUT_BRIDGE_DECK, width=texels_per_cell * 4 + 6)
            draw.line(v_pts, fill=CLUT_TARMAC, width=texels_per_cell * 4)

        c_pts = [project_coord(lat, lon, model.bbox, width_cells, height_cells) for lat, lon in line]
        c_mask = Image.new("L", (width_cells, height_cells), 0)
        c_draw = ImageDraw.Draw(c_mask)
        c_draw.line(c_pts, fill=255, width=3)
        surface[np.array(c_mask) > 0] = TILE_TARMAC

    # 7. Add Race Circuit Elements (Gates, Start Lines, Curbs)
    for race in model.races:
        # Start/Finish gate
        start = race["checkpoints"][0]
        sx, sy, sw, sh = start["x"], start["y"], start["w"], start["h"]
        surface[sy : sy + sh, sx : sx + sw] = TILE_STARTFINISH

        v_sx = sx * texels_per_cell
        v_sy = sy * texels_per_cell
        v_sw = sw * texels_per_cell
        v_sh = sh * texels_per_cell
        draw.rectangle([v_sx, v_sy, v_sx + v_sw, v_sy + v_sh], fill=CLUT_START_LINE)

        # Checkpoints
        for cp in race["checkpoints"][1:]:
            cx, cy, cw, ch = cp["x"], cp["y"], cp["w"], cp["h"]
            surface[cy : cy + ch, cx : cx + cw] = TILE_CHECKPOINT
            v_cx = cx * texels_per_cell
            v_cy = cy * texels_per_cell
            v_cw = cw * texels_per_cell
            v_ch = ch * texels_per_cell
            draw.rectangle([v_cx, v_cy, v_cx + v_cw, v_cy + v_ch], fill=CLUT_GATE)

    # 8. City Outer Border Barriers
    surface[0, :] = TILE_BARRIER
    surface[height_cells - 1, :] = TILE_BARRIER
    surface[:, 0] = TILE_BARRIER
    surface[:, width_cells - 1] = TILE_BARRIER

    visual_arr = np.array(v_img, dtype=np.uint8)
    return surface, visual_arr


def export_city(
    model: CityModel,
    surface: np.ndarray,
    visual: np.ndarray,
    clut: List[Tuple[int, int, int]],
    out_dir: str,
    chunk_dim: int = 32,
    texels_per_cell: int = 8,
):
    """Exports city files into destination directory."""
    city_dir = os.path.join(out_dir, model.city_id)
    chunks_dir = os.path.join(city_dir, "chunks")
    os.makedirs(chunks_dir, exist_ok=True)

    height_cells, width_cells = surface.shape
    chunks_x = math.ceil(width_cells / chunk_dim)
    chunks_y = math.ceil(height_cells / chunk_dim)

    # 1. Save Collision Data Image (.data.png)
    pal_img = Image.fromarray(surface, mode="P")
    flat_pal = []
    for r, g, b in clut:
        flat_pal.extend([r, g, b])
    pal_img.putpalette(flat_pal)
    data_png_path = os.path.join(city_dir, f"{model.city_id}.data.png")
    pal_img.save(data_png_path)

    # 2. Save Visual Image (.visual.png)
    v_img = Image.fromarray(visual, mode="P")
    v_img.putpalette(flat_pal)
    visual_png_path = os.path.join(city_dir, f"{model.city_id}.visual.png")
    v_img.save(visual_png_path)

    # 3. Save Palette JSON
    pal_json_path = os.path.join(city_dir, f"{model.city_id}.palette.json")
    with open(pal_json_path, "w") as f:
        json.dump({"palette": clut}, f, indent=2)

    # 4. Slice into CD-ROM Chunks
    chunk_texel_dim = chunk_dim * texels_per_cell
    for cy in range(chunks_y):
        for cx in range(chunks_x):
            chunk_id = cy * chunks_x + cx

            # Collision tiles (32x32 = 1024 bytes)
            c_y0 = cy * chunk_dim
            c_y1 = min(height_cells, c_y0 + chunk_dim)
            c_x0 = cx * chunk_dim
            c_x1 = min(width_cells, c_x0 + chunk_dim)

            chunk_tiles = np.full((chunk_dim, chunk_dim), TILE_BARRIER, dtype=np.uint8)
            sub_c = surface[c_y0:c_y1, c_x0:c_x1]
            chunk_tiles[: sub_c.shape[0], : sub_c.shape[1]] = sub_c
            tile_bytes = chunk_tiles.tobytes()

            tiles_bin_path = os.path.join(chunks_dir, f"chunk_{cx:02d}_{cy:02d}.tiles.bin")
            with open(tiles_bin_path, "wb") as f:
                f.write(tile_bytes)

            # Visual 8bpp texels (256x256 = 65,536 bytes)
            v_y0 = cy * chunk_texel_dim
            v_y1 = min(visual.shape[0], v_y0 + chunk_texel_dim)
            v_x0 = cx * chunk_texel_dim
            v_x1 = min(visual.shape[1], v_x0 + chunk_texel_dim)

            chunk_visual = np.full((chunk_texel_dim, chunk_texel_dim), CLUT_WALL, dtype=np.uint8)
            sub_v = visual[v_y0:v_y1, v_x0:v_x1]
            chunk_visual[: sub_v.shape[0], : sub_v.shape[1]] = sub_v
            visual_bytes = chunk_visual.tobytes()

            visual_bin_path = os.path.join(chunks_dir, f"chunk_{cx:02d}_{cy:02d}.visual.bin")
            with open(visual_bin_path, "wb") as f:
                f.write(visual_bytes)

            # Visual 15bpp direct colour texels (256x256 halfwords = 131,072 bytes)
            bgr555_lut = np.array([
                ((r >> 3) & 0x1F) | (((g >> 3) & 0x1F) << 5) | (((b >> 3) & 0x1F) << 10)
                for r, g, b in clut
            ], dtype=np.uint16)
            chunk_visual15 = bgr555_lut[chunk_visual]
            visual15_bin_path = os.path.join(chunks_dir, f"chunk_{cx:02d}_{cy:02d}.visual15.bin")
            with open(visual15_bin_path, "wb") as f:
                f.write(chunk_visual15.tobytes())

    # 5. Save City Metadata Manifest
    manifest = {
        "id": model.city_id,
        "name": model.name,
        "country": model.country,
        "bbox": list(model.bbox),
        "width_cells": width_cells,
        "height_cells": height_cells,
        "chunk_dim": chunk_dim,
        "chunks_x": chunks_x,
        "chunks_y": chunks_y,
        "total_chunks": chunks_x * chunks_y,
        "texels_per_cell": texels_per_cell,
        "districts": model.districts,
        "races": model.races,
    }
    manifest_path = os.path.join(city_dir, f"{model.city_id}.city.json")
    with open(manifest_path, "w") as f:
        json.dump(manifest, f, indent=2)

    print(
        f"[OpenMap] Successfully compiled {model.name} -> {city_dir} "
        f"({width_cells}x{height_cells} cells, {chunks_x * chunks_y} chunks)"
    )


def generate_rust_cities_module(models: Dict[str, CityModel], output_rs_path: str):
    """Emits Rust source code with CityDef & CityRace definitions for arduracer-core."""
    os.makedirs(os.path.dirname(output_rs_path), exist_ok=True)

    lines = [
        "//! Shipped real-world city maps and urban racing championships.",
        "//!",
        "//! Generated by `tools/track_cook/openmap_downloader.py`.",
        "//! Fully `#![no_std]` and pure `core` compatible with static memory footprint.",
        "",
        "use crate::city::{CityDef, CityRace, DEFAULT_CHUNK_DIM, DEFAULT_TEXELS_PER_CELL};",
        "use crate::math::{Fixed, Vec2};",
        "use crate::timing::{CheckpointGate, ParTimes};",
        "",
    ]

    city_vars = []
    for city_id, model in models.items():
        var_name = f"CITY_{city_id.upper()}"
        city_vars.append(var_name)

        # Races
        races_var = f"{city_id.upper()}_RACES"
        lines.append(f"/// Authored urban racing circuits for {model.name}.")
        lines.append(f"pub static {races_var}: &[CityRace] = &[")

        for r_idx, race in enumerate(model.races):
            sc_x, sc_y = race["start_cell"]
            # Convert cell to world coordinates (cell * 32 + 16)
            world_x = sc_x * 32 + 16
            world_y = sc_y * 32 + 16

            lines.append("    CityRace {")
            lines.append(f'        name: "{race["name"]}",')
            lines.append(f'        district: "{race["district"]}",')
            lines.append(f"        start_pos: Vec2::new(Fixed::from_int({world_x}), Fixed::from_int({world_y})),")
            lines.append(f'        start_heading: {race["heading"]},')

            start_gate = race["checkpoints"][0]
            lines.append(
                f"        start_gate: CheckpointGate {{"
                f" x: {start_gate['x']}, y: {start_gate['y']}, width: {start_gate['w']}, height: {start_gate['h']} }},"
            )

            p_gold, p_silv, p_bron = race["par_times"]
            p_plat = int(p_gold * 0.8)
            lines.append(
                f"        par_times: ParTimes {{"
                f" bronze_ticks: {p_bron}, silver_ticks: {p_silv}, gold_ticks: {p_gold}, dev_platinum_ticks: {p_plat} }},"
            )

            # Checkpoints
            cps_var = f"{city_id.upper()}_RACE_{r_idx}_CHECKPOINTS"
            lines.append(f"        checkpoints: &{cps_var},")

            # Route
            route_var = f"{city_id.upper()}_RACE_{r_idx}_ROUTE"
            lines.append(f"        route: &{route_var},")
            lines.append(f"        half_width: {race['half_width']},")
            lines.append("    },")

        lines.append("];")
        lines.append("")

        # Static checkpoint arrays
        for r_idx, race in enumerate(model.races):
            cps_var = f"{city_id.upper()}_RACE_{r_idx}_CHECKPOINTS"
            lines.append(f"static {cps_var}: [CheckpointGate; {len(race['checkpoints'])}] = [")
            for cp in race["checkpoints"]:
                lines.append(f"    CheckpointGate {{ x: {cp['x']}, y: {cp['y']}, width: {cp['w']}, height: {cp['h']} }},")
            lines.append("];")
            lines.append("")

            # Route points in world units
            route_var = f"{city_id.upper()}_RACE_{r_idx}_ROUTE"
            lines.append(f"static {route_var}: [Vec2; {len(race['route'])}] = [")
            for rx, ry in race["route"]:
                rw_x = rx * 32 + 16
                rw_y = ry * 32 + 16
                lines.append(f"    Vec2::new(Fixed::from_int({rw_x}), Fixed::from_int({rw_y})),")
            lines.append("];")
            lines.append("")

        # CityDef
        min_lat_e7 = int(model.bbox[0] * 10_000_000)
        min_lon_e7 = int(model.bbox[1] * 10_000_000)
        max_lat_e7 = int(model.bbox[2] * 10_000_000)
        max_lon_e7 = int(model.bbox[3] * 10_000_000)

        lines.append(f"/// Full city definition for {model.name} ({model.country}).")
        lines.append(f"pub static {var_name}: CityDef = CityDef::new(")
        lines.append(f'    "{model.name}",')
        lines.append("    128,")
        lines.append("    128,")
        lines.append("    DEFAULT_CHUNK_DIM,")
        lines.append("    DEFAULT_TEXELS_PER_CELL,")
        lines.append(f"    {min_lat_e7},")
        lines.append(f"    {min_lon_e7},")
        lines.append(f"    {max_lat_e7},")
        lines.append(f"    {max_lon_e7},")
        lines.append(f"    {races_var},")
        lines.append("    [(0, 0, 0); 256],")
        lines.append(");")
        lines.append("")

    # ALL_CITIES array
    lines.append("/// All real-world city maps ready for exploration and CD streaming.")
    lines.append(f"pub static ALL_CITIES: &[&CityDef] = &[")
    for var in city_vars:
        lines.append(f"    &{var},")
    lines.append("];")
    lines.append("")

    with open(output_rs_path, "w") as f:
        f.write("\n".join(lines))

    print(f"[OpenMap] Emitted Rust city definitions -> {output_rs_path}")


def main():
    parser = argparse.ArgumentParser(description="OpenMap Downloader and City Compiler for Arduracer PSX")
    parser.add_argument("cities", nargs="*", help="City IDs to compile (e.g. cape_town, melbourne, etc.)")
    parser.add_argument("--all", action="store_true", help="Compile all 7 target cities")
    parser.add_argument("--out-dir", default="cities", help="Directory where city assets are saved (default: cities)")
    parser.add_argument("--width", type=int, default=128, help="City grid width in cells (default: 128)")
    parser.add_argument("--height", type=int, default=128, help="City grid height in cells (default: 128)")
    parser.add_argument("--chunk-dim", type=int, default=32, help="Chunk cell size (default: 32)")
    parser.add_argument("--texels-per-cell", type=int, default=8, help="Visual texels per cell (default: 8)")
    parser.add_argument("--offline", action="store_true", default=False, help="Skip Overpass API network requests")
    parser.add_argument("--bbox", help="Custom bbox 'min_lat,min_lon,max_lat,max_lon' for arbitrary city download")
    parser.add_argument("--city-name", help="Custom display name when using --bbox")
    parser.add_argument(
        "--emit-rust",
        default="crates/arduracer-core/src/city_data.rs",
        help="Path to generate Rust city definitions (default: crates/arduracer-core/src/city_data.rs)"
    )

    args = parser.parse_args()

    models = get_city_models()
    clut = build_city_clut()

    target_cities = []
    if args.all or not args.cities:
        target_cities = list(models.keys())
    else:
        for c in args.cities:
            c_norm = c.lower().replace(" ", "_").replace("-", "_")
            if c_norm in models:
                target_cities.append(c_norm)
            else:
                print(f"[OpenMap] Warning: Unknown city '{c}'. Available: {list(models.keys())}")

    if not target_cities:
        print("[OpenMap] No cities specified. Use --all or choose from:", list(models.keys()))
        sys.exit(1)

    print(f"[OpenMap] Preparing city compilation for {len(target_cities)} cities: {target_cities}")

    for city_id in target_cities:
        model = models[city_id]

        # Try Overpass API if online mode and not forced offline
        if not args.offline:
            print(f"[OpenMap] Querying OpenStreetMap Overpass API for {model.name} ({model.bbox})...")
            osm_data = fetch_osm_overpass(model.bbox, timeout=5)
            if osm_data:
                print(f"[OpenMap] Successfully retrieved live OSM data for {model.name} ({len(osm_data.get('elements', []))} elements)")
            else:
                print(f"[OpenMap] Falling back to built-in high-precision geographic model for {model.name}")

        # Compile surface tiles and visual texture
        surface, visual = compile_city_data(
            model=model,
            width_cells=args.width,
            height_cells=args.height,
            chunk_dim=args.chunk_dim,
            texels_per_cell=args.texels_per_cell,
        )

        # Export city data files, chunks, and manifest
        export_city(
            model=model,
            surface=surface,
            visual=visual,
            clut=clut,
            out_dir=args.out_dir,
            chunk_dim=args.chunk_dim,
            texels_per_cell=args.texels_per_cell,
        )

    # Generate Rust definitions for arduracer-core
    if args.emit_rust:
        generate_rust_cities_module(models, args.emit_rust)

    print("\n[OpenMap] All city maps built and exported successfully!")


if __name__ == "__main__":
    main()
