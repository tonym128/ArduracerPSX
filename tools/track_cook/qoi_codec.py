"""Quite OK Images (QOI) encoder for Arduracer PSX track texture blocks.

Reference spec: https://qoiformat.org/qoi-specification.pdf

Stock QOI is a single linear stream with no random-access points, so a
whole-image QOI cannot be sliced into the 64x16 MCU rows the VRAM streamer
decodes per frame. This module therefore emits a *segmented* QOI block:

    +----------------------------+  0
    | ARQOI header (24 bytes)     |
    +----------------------------+  24
    | u32 segment_offsets[1024]  |  byte offset from the first payload byte
    +----------------------------+
    | segment payloads           |  each an independent QOI run of 64x16 px
    +----------------------------+

Each segment restarts QOI decoding from the canonical initial state (index array
zeroed, previous pixel {0,0,0,255}), giving O(1) random access to any 64x16 MCU
row -- the same guarantee the JPEG path gets from its DRI=4 restart markers.
Segment order matches the JPEG restart-interval order exactly:
``index = (tile_y * 4 + row) * 16 + tile_x``.

The opcode encoding below mirrors the reference C encoder in phoboslab/qoi
byte for byte, including its signed-difference ranges, so a segment payload is
a spec-valid QOI chunk stream.
"""

from __future__ import annotations

# ---------------------------------------------------------------------------
# Container constants (mirrored in crates/arduracer-core/src/qoi.rs)
# ---------------------------------------------------------------------------

#: Container magic, b"ARQO".
MAGIC = b"ARQO"
#: Container format version. Bump if the segment layout ever changes.
VERSION = 1
#: magic(4) + version(1) + flags(2) + channels(1) + width(4) + height(4)
#:   + segment_count(2) + segment_width(2) + segment_height(2) + reserved(2)
#: = 24, chosen so the u32 segment table starts naturally aligned.
HEADER_BYTES = 24

#: Width/height of a single segment in texels: 4 MCUs wide, one MCU tall.
SEGMENT_WIDTH = 64
SEGMENT_HEIGHT = 16
#: Rows of segments within one 64x64 VRAM tile.
ROWS_PER_TILE = 4
#: Segments along each image axis: 1024 / 64 = 16.
SEGMENTS_PER_AXIS = 16
#: Total segments per block: 16 * 16 * 4.
SEGMENT_COUNT = SEGMENTS_PER_AXIS * SEGMENTS_PER_AXIS * ROWS_PER_TILE
#: Byte size of the u32 segment offset table that follows the header.
TABLE_BYTES = SEGMENT_COUNT * 4
#: Pixels per segment.
SEGMENT_PIXELS = SEGMENT_WIDTH * SEGMENT_HEIGHT
#: Worst-case segment payload: 5 bytes per pixel plus the 8-byte end marker.
SEGMENT_MAX_BYTES = SEGMENT_PIXELS * 5 + 8

# QOI opcodes, from the reference implementation.
_OP_INDEX = 0x00
_OP_DIFF = 0x40
_OP_LUMA = 0x80
_OP_RUN = 0xC0
_OP_RGB = 0xFE
_MASK_2 = 0xC0

#: QOI end-of-stream marker: seven zero bytes then 0x01.
QOI_PADDING = b"\x00\x00\x00\x00\x00\x00\x00\x01"


def _color_hash(r: int, g: int, b: int, a: int) -> int:
    """QOI's index hash. Must match the decoder's exactly."""
    return (r * 3 + g * 5 + b * 7 + a * 11) & 63


def encode_segment(pixels, channels: int = 3) -> bytes:
    """Encodes one 64x16 segment as a standalone QOI chunk stream.

    `pixels` is a flat sequence of ``SEGMENT_PIXELS * channels`` bytes. No QOI
    file header is emitted: the container supplies the geometry and the decoder
    stops at ``SEGMENT_PIXELS`` decoded pixels. The 8-byte end marker is
    appended so a truncated segment is detectable.
    """
    out = bytearray()
    index = [(0, 0, 0, 0)] * 64

    pr = pg = pb = 0
    pa = 255

    px_len = len(pixels)
    px_end = px_len - channels
    run = 0

    for pos in range(0, px_len, channels):
        r = pixels[pos]
        g = pixels[pos + 1]
        b = pixels[pos + 2]
        a = pixels[pos + 3] if channels == 4 else 255

        if (r, g, b, a) == (pr, pg, pb, pa):
            run += 1
            if run == 62 or pos == px_end:
                out.append(_OP_RUN | (run - 1))
                run = 0
        else:
            if run > 0:
                out.append(_OP_RUN | (run - 1))
                run = 0

            index_pos = _color_hash(r, g, b, a)
            if index[index_pos] == (r, g, b, a):
                out.append(_OP_INDEX | index_pos)
            else:
                index[index_pos] = (r, g, b, a)

                if a == pa:
                    # Signed 8-bit deltas, matching the reference's `signed char`
                    # arithmetic.
                    vr = (r - pr) & 0xFF
                    vg = (g - pg) & 0xFF
                    vb = (b - pb) & 0xFF
                    vr = vr - 256 if vr > 127 else vr
                    vg = vg - 256 if vg > 127 else vg
                    vb = vb - 256 if vb > 127 else vb

                    vg_r = (vr - vg) & 0xFF
                    vg_r = vg_r - 256 if vg_r > 127 else vg_r
                    vg_b = (vb - vg) & 0xFF
                    vg_b = vg_b - 256 if vg_b > 127 else vg_b

                    if -3 < vr < 2 and -3 < vg < 2 and -3 < vb < 2:
                        out.append(
                            _OP_DIFF
                            | ((vr + 2) << 4)
                            | ((vg + 2) << 2)
                            | (vb + 2)
                        )
                    elif (
                        -9 < vg_r < 8
                        and -33 < vg < 32
                        and -9 < vg_b < 8
                    ):
                        out.append(_OP_LUMA | (vg + 32))
                        out.append(((vg_r + 8) << 4) | (vg_b + 8))
                    else:
                        out.append(_OP_RGB)
                        out.append(r)
                        out.append(g)
                        out.append(b)
                else:
                    out.append(0xFF)  # QOI_OP_RGBA
                    out.append(r)
                    out.append(g)
                    out.append(b)
                    out.append(a)

        pr, pg, pb, pa = r, g, b, a

    out.extend(QOI_PADDING)
    return bytes(out)


def encode_block(rgb: bytes, width: int, height: int, channels: int = 3) -> bytes:
    """Encodes a `width` x `height` RGB(A) buffer into a segmented QOI block.

    The image is cut into the same 64x16 segments the VRAM streamer decodes, in
    the same order the JPEG restart intervals use, so the two paths are drop-in
    interchangeable at the tile level.
    """
    assert width == SEGMENTS_PER_AXIS * SEGMENT_WIDTH, width
    assert height == SEGMENTS_PER_AXIS * ROWS_PER_TILE * SEGMENT_HEIGHT, height

    stride = width * channels
    seg_bytes = SEGMENT_WIDTH * channels
    seg_y_count = height // SEGMENT_HEIGHT

    segments = []
    for seg_y in range(seg_y_count):
        row_base = seg_y * SEGMENT_HEIGHT * stride
        for seg_x in range(SEGMENTS_PER_AXIS):
            buf = bytearray()
            for row in range(SEGMENT_HEIGHT):
                off = row_base + row * stride + seg_x * seg_bytes
                buf += rgb[off : off + seg_bytes]
            segments.append(encode_segment(buf, channels))

    out = bytearray()
    out += MAGIC
    out.append(VERSION)
    out += (0).to_bytes(2, "big")  # flags: reserved, always 0
    out.append(channels)
    out += width.to_bytes(4, "big")
    out += height.to_bytes(4, "big")
    out += SEGMENT_COUNT.to_bytes(2, "big")
    out += SEGMENT_WIDTH.to_bytes(2, "big")
    out += SEGMENT_HEIGHT.to_bytes(2, "big")
    out += (0).to_bytes(2, "big")  # reserved, keeps the table u32-aligned
    assert len(out) == HEADER_BYTES, len(out)

    # Segment table: each offset is relative to the first payload byte, which
    # sits immediately after the table.
    table_off = len(out)
    out += b"\x00" * TABLE_BYTES
    payload_base = len(out)

    cursor = 0
    for idx, seg in enumerate(segments):
        assert len(seg) <= SEGMENT_MAX_BYTES, len(seg)
        out += seg
        at = table_off + idx * 4
        out[at : at + 4] = cursor.to_bytes(4, "big")
        cursor += len(seg)

    assert len(out) == payload_base + cursor, (len(out), payload_base, cursor)
    return bytes(out)


def decode_block(data: bytes, channels: int = 3):
    """Reference decoder for a segmented QOI block. Used by the host test suite
    and by `build_atlas.py` to assert the encoder is self-consistent."""
    assert data[:4] == MAGIC, "bad magic"
    assert data[4] == VERSION, f"bad version {data[4]}"
    width = int.from_bytes(data[8:12], "big")
    height = int.from_bytes(data[12:16], "big")
    count = int.from_bytes(data[16:18], "big")
    seg_w = int.from_bytes(data[18:20], "big")
    seg_h = int.from_bytes(data[20:22], "big")

    table_off = HEADER_BYTES
    assert count == SEGMENT_COUNT, count
    assert seg_w == SEGMENT_WIDTH and seg_h == SEGMENT_HEIGHT, (seg_w, seg_h)
    payload_base = table_off + TABLE_BYTES
    offsets = [
        int.from_bytes(data[table_off + i * 4 : table_off + i * 4 + 4], "big")
        for i in range(count)
    ]

    out = bytearray(width * height * 3)
    stride = width * 3
    for idx, off in enumerate(offsets):
        seg_x = idx % SEGMENTS_PER_AXIS
        seg_y = (idx // SEGMENTS_PER_AXIS) % (height // seg_h)
        seg = _decode_segment(data, payload_base + off, seg_w * seg_h, channels)
        for row in range(seg_h):
            dst = (seg_y * seg_h + row) * stride + seg_x * seg_w * 3
            out[dst : dst + seg_w * 3] = seg[row * seg_w * 3 : (row + 1) * seg_w * 3]
    return bytes(out)


def _decode_segment(data: bytes, start: int, n_pixels: int, channels: int = 3):
    """Decodes one segment's chunks, mirroring the reference `qoi_decode`."""
    index = [(0, 0, 0, 0)] * 64
    pr = pg = pb = 0
    pa = 255
    out = bytearray()
    pos = start
    run = 0

    for i in range(n_pixels):
        if run > 0:
            run -= 1
        else:
            b1 = data[pos]
            pos += 1
            if b1 == _OP_RGB:
                pr, pg, pb = data[pos], data[pos + 1], data[pos + 2]
                pos += 3
            elif b1 == 0xFF:  # QOI_OP_RGBA
                pr, pg, pb, pa = (
                    data[pos],
                    data[pos + 1],
                    data[pos + 2],
                    data[pos + 3],
                )
                pos += 4
            elif (b1 & _MASK_2) == _OP_INDEX:
                pr, pg, pb, pa = index[b1 & 0x3F]
            elif (b1 & _MASK_2) == _OP_DIFF:
                pr = (pr + ((b1 >> 4) & 0x03) - 2) & 0xFF
                pg = (pg + ((b1 >> 2) & 0x03) - 2) & 0xFF
                pb = (pb + (b1 & 0x03) - 2) & 0xFF
            elif (b1 & _MASK_2) == _OP_LUMA:
                b2 = data[pos]
                pos += 1
                vg = (b1 & 0x3F) - 32
                pr = (pr + vg - 8 + ((b2 >> 4) & 0x0F)) & 0xFF
                pg = (pg + vg) & 0xFF
                pb = (pb + vg - 8 + (b2 & 0x0F)) & 0xFF
            elif (b1 & _MASK_2) == _OP_RUN:
                run = b1 & 0x3F
            else:
                raise ValueError(f"unknown QOI opcode 0x{b1:02x} at {pos - 1}")

            index[_color_hash(pr, pg, pb, pa)] = (pr, pg, pb, pa)

        out.append(pr)
        out.append(pg)
        out.append(pb)

    return bytes(out)
