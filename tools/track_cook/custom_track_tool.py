#!/usr/bin/env python3
"""Arduracer PSX — TrackMania Custom Track Studio Tool.

Encodes, decodes, validates, and renders custom tracks matching the PS1 binary
CustomTrackDef layout (256 bytes, CRC16 CCITT-FALSE).
"""

import argparse
import base64
import json
import os
import struct
import sys
from typing import Optional

# Exact constants from crates/arduracer-core/src/track_builder.rs
MAGIC = b"ARTR"
VERSION = 1
SERIALIZED_SIZE = 256
MAX_BLOCKS = 64
MACRO_GRID_DIM = 20
TILE_GRID_DIM = 80
BLOCK_TILE_DIM = 4

BLOCK_KINDS = {
    0: "StartGrid",
    1: "Straight",
    2: "Turn90",
    3: "Hairpin",
    4: "Chicane",
    5: "DirtSection",
    6: "IceBobsleigh",
    7: "JumpRamp",
    8: "BridgeOverpass",
    9: "FreeWheel",
    10: "ReactorBoost",
    11: "CheckpointGate",
}

BLOCK_NAME_TO_ID = {v.lower(): k for k, v in BLOCK_KINDS.items()}
BLOCK_NAME_TO_ID.update({
    "start": 0, "startgrid": 0, "start_grid": 0,
    "straight": 1,
    "turn90": 2, "turn_90": 2, "turn": 2, "corner": 2,
    "hairpin": 3,
    "chicane": 4,
    "dirt": 5, "dirt_section": 5, "rally": 5,
    "ice": 6, "ice_bobsleigh": 6, "bobsleigh": 6,
    "jump": 7, "jump_ramp": 7, "ramp": 7,
    "bridge": 8, "bridge_overpass": 8, "overpass": 8,
    "freewheel": 9, "free_wheel": 9, "red": 9,
    "reactor": 10, "reactor_boost": 10, "boost": 10,
    "checkpoint": 11, "checkpoint_gate": 11, "gate": 11,
})


def crc16(data: bytes) -> int:
    """CRC16-CCITT (False, poly 0x1021, init 0xFFFF)."""
    crc = 0xFFFF
    for b in data:
        crc ^= b << 8
        for _ in range(8):
            crc = ((crc << 1) ^ 0x1021) & 0xFFFF if crc & 0x8000 else (crc << 1) & 0xFFFF
    return crc


class PlacedBlock:
    def __init__(self, x: int, y: int, kind: int, orientation: int = 0):
        self.x = int(x) & 0xFF
        self.y = int(y) & 0xFF
        self.kind = int(kind) & 0x0F
        self.orientation = int(orientation) & 0x03

    def pack(self) -> bytes:
        b2 = (self.kind & 0x0F) | ((self.orientation & 0x03) << 4)
        return bytes([self.x, self.y, b2])

    @classmethod
    def unpack(cls, b: bytes) -> "PlacedBlock":
        x, y = b[0], b[1]
        kind = b[2] & 0x0F
        orientation = (b[2] >> 4) & 0x03
        return cls(x, y, kind, orientation)

    def to_dict(self) -> dict:
        return {
            "x": self.x,
            "y": self.y,
            "kind": BLOCK_KINDS.get(self.kind, f"Unknown_{self.kind}"),
            "kind_id": self.kind,
            "orientation": self.orientation,
            "heading_deg": self.orientation * 90,
        }


class CustomTrack:
    def __init__(self, name: str = "CUSTOM ARENA", author_time_ticks: int = 0):
        self.name = name[:16]
        self.author_time_ticks = int(author_time_ticks)
        self.blocks: list[PlacedBlock] = []

    def add_block(self, x: int, y: int, kind: int, orientation: int = 0):
        if len(self.blocks) >= MAX_BLOCKS:
            raise ValueError(f"Maximum of {MAX_BLOCKS} blocks exceeded")
        # Replace if same (x, y)
        for i, b in enumerate(self.blocks):
            if b.x == x and b.y == y:
                self.blocks[i] = PlacedBlock(x, y, kind, orientation)
                return
        self.blocks.append(PlacedBlock(x, y, kind, orientation))

    def serialize(self) -> bytes:
        buf = bytearray(SERIALIZED_SIZE)
        buf[0:4] = MAGIC
        buf[4] = VERSION
        
        name_bytes = self.name.encode("ascii", "replace")[:16]
        buf[5:5 + len(name_bytes)] = name_bytes
        
        struct.pack_into("<I", buf, 21, self.author_time_ticks)
        buf[25] = min(len(self.blocks), MAX_BLOCKS)

        off = 28
        for b in self.blocks[:MAX_BLOCKS]:
            buf[off:off + 3] = b.pack()
            off += 3

        # CRC covers 0..26 and 28..256
        crc_data = bytes(buf[0:26]) + bytes(buf[28:256])
        ck = crc16(crc_data)
        struct.pack_into("<H", buf, 26, ck)
        return bytes(buf)

    @classmethod
    def deserialize(cls, data: bytes) -> "CustomTrack":
        if len(data) != SERIALIZED_SIZE:
            raise ValueError(f"Expected {SERIALIZED_SIZE} bytes, got {len(data)}")
        if data[0:4] != MAGIC:
            raise ValueError(f"Invalid magic: {data[0:4]!r}")
        if data[4] != VERSION:
            raise ValueError(f"Unsupported version: {data[4]}")

        stored_crc = struct.unpack_from("<H", data, 26)[0]
        crc_data = bytes(data[0:26]) + bytes(data[28:256])
        if crc16(crc_data) != stored_crc:
            raise ValueError(f"CRC mismatch: expected 0x{stored_crc:04x}, got 0x{crc16(crc_data):04x}")

        raw_name = data[5:21].split(b"\x00")[0]
        name = raw_name.decode("ascii", "replace")
        author_time = struct.unpack_from("<I", data, 21)[0]
        count = min(data[25], MAX_BLOCKS)

        track = cls(name, author_time)
        off = 28
        for _ in range(count):
            track.blocks.append(PlacedBlock.unpack(data[off:off + 3]))
            off += 3
        return track

    def to_base64(self) -> str:
        return base64.urlsafe_b64encode(self.serialize()).decode("ascii")

    @classmethod
    def from_base64(cls, s: str) -> "CustomTrack":
        s = s.strip()
        if s.startswith("#track="):
            s = s[7:]
        # Add padding if missing
        pad = len(s) % 4
        if pad:
            s += "=" * (4 - pad)
        data = base64.urlsafe_b64decode(s)
        return cls.deserialize(data)

    def validate(self) -> list[str]:
        errors = []
        if len(self.blocks) < 3:
            errors.append("Track must contain at least 3 placed blocks")
        starts = sum(1 for b in self.blocks if b.kind == 0)
        if starts == 0:
            errors.append("Track must have a Start/Finish Grid block")
        elif starts > 1:
            errors.append(f"Track has {starts} Start/Finish Grid blocks; only 1 allowed")

        for b in self.blocks:
            if b.x >= MACRO_GRID_DIM or b.y >= MACRO_GRID_DIM:
                errors.append(f"Block at ({b.x}, {b.y}) is out of bounds (max 19)")
        return errors

    def to_svg(self, width: int = 600, height: int = 600) -> str:
        """Renders the custom track into a clean SVG document."""
        cell = width / MACRO_GRID_DIM
        lines = [
            f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {width} {height}" width="{width}" height="{height}">',
            f'  <rect width="{width}" height="{height}" fill="#10141e"/>',
            '  <!-- Grid Lines -->',
        ]
        for i in range(MACRO_GRID_DIM + 1):
            pos = i * cell
            lines.append(f'  <line x1="{pos}" y1="0" x2="{pos}" y2="{height}" stroke="#1f2738" stroke-width="1"/>')
            lines.append(f'  <line x1="0" y1="{pos}" x2="{width}" y2="{pos}" stroke="#1f2738" stroke-width="1"/>')

        palette = {
            0: ("#22c55e", "START"),
            1: ("#475569", "STR"),
            2: ("#64748b", "T90"),
            3: ("#94a3b8", "HAIR"),
            4: ("#cbd5e1", "CHIC"),
            5: ("#b45309", "DIRT"),
            6: ("#38bdf8", "ICE"),
            7: ("#f97316", "JUMP"),
            8: ("#8b5cf6", "BRID"),
            9: ("#ef4444", "FREE"),
            10: ("#06b6d4", "BOOST"),
            11: ("#eab308", "GATE"),
        }

        lines.append('  <!-- Placed Blocks -->')
        for b in self.blocks:
            bx = b.x * cell
            by = b.y * cell
            color, label = palette.get(b.kind, ("#6b7280", "BLK"))
            rot = b.orientation * 90
            cx = bx + cell / 2
            cy = by + cell / 2

            lines.append(f'  <g transform="rotate({rot} {cx} {cy})">')
            lines.append(f'    <rect x="{bx + 2}" y="{by + 2}" width="{cell - 4}" height="{cell - 4}" rx="3" fill="{color}" opacity="0.9"/>')
            lines.append(f'    <text x="{cx}" y="{cy + 4}" fill="#ffffff" font-family="sans-serif" font-size="{cell * 0.35:.1f}" font-weight="bold" text-anchor="middle">{label}</text>')
            # Arrow indicating orientation
            lines.append(f'    <polygon points="{cx},{by + 4} {cx - 4},{by + 10} {cx + 4},{by + 10}" fill="#ffffff"/>')
            lines.append('  </g>')

        lines.append(f'  <text x="12" y="{height - 12}" fill="#94a3b8" font-family="monospace" font-size="12">{self.name} • {len(self.blocks)} BLOCKS</text>')
        lines.append('</svg>')
        return "\n".join(lines)


def main():
    parser = argparse.ArgumentParser(description="Arduracer PSX TrackMania Custom Track Tool")
    parser.add_argument("--decode", help="Base64 URL code or #track=... to decode")
    parser.add_argument("--encode", help="JSON track file to encode")
    parser.add_argument("--to-svg", help="Output path for SVG diagram")
    parser.add_argument("--to-bin", help="Output path for raw 256-byte track definition")
    parser.add_argument("--test", action="store_true", help="Run internal validation test suite")
    args = parser.parse_args()

    if args.test:
        print("Running CustomTrack self-tests...")
        t = CustomTrack("TEST TRACK", 1234)
        t.add_block(5, 5, 0, 0)  # Start
        t.add_block(6, 5, 1, 1)  # Straight
        t.add_block(7, 5, 2, 1)  # Turn90
        t.add_block(7, 6, 11, 2) # Checkpoint
        assert len(t.validate()) == 0, f"Validation errors: {t.validate()}"
        
        raw = t.serialize()
        assert len(raw) == SERIALIZED_SIZE
        t2 = CustomTrack.deserialize(raw)
        assert t2.name == "TEST TRACK"
        assert t2.author_time_ticks == 1234
        assert len(t2.blocks) == 4
        assert t2.blocks[0].kind == 0
        assert t2.blocks[3].kind == 11
        
        b64 = t.to_base64()
        t3 = CustomTrack.from_base64(b64)
        assert len(t3.blocks) == 4
        print(f"Self-tests passed! Generated base64: {b64}")
        return 0

    if args.decode:
        track = CustomTrack.from_base64(args.decode)
        errs = track.validate()
        status = "VALID" if not errs else f"INVALID: {', '.join(errs)}"
        print(f"Track: {track.name} ({len(track.blocks)} blocks, {track.author_time_ticks} ticks) [{status}]")
        for i, b in enumerate(track.blocks):
            print(f"  #{i+1:02d}: ({b.x:2d}, {b.y:2d}) -> {BLOCK_KINDS.get(b.kind)} rot={b.orientation*90}°")
        if args.to_svg:
            with open(args.to_svg, "w", encoding="utf-8") as f:
                f.write(track.to_svg())
            print(f"Exported SVG: {args.to_svg}")
        if args.to_bin:
            with open(args.to_bin, "wb") as f:
                f.write(track.serialize())
            print(f"Exported binary: {args.to_bin}")
        return 0

    if args.encode:
        with open(args.encode, "r", encoding="utf-8") as f:
            data = json.load(f)
        track = CustomTrack(data.get("name", "CUSTOM"), data.get("author_time_ticks", 0))
        for b in data.get("blocks", []):
            kind = BLOCK_NAME_TO_ID.get(str(b.get("kind", 1)).lower(), 1)
            track.add_block(b["x"], b["y"], kind, b.get("orientation", 0))
        b64 = track.to_base64()
        print(f"Shareable Code: #track={b64}")
        if args.to_bin:
            with open(args.to_bin, "wb") as f:
                f.write(track.serialize())
        return 0

    parser.print_help()
    return 1


if __name__ == "__main__":
    sys.exit(main())
