#!/usr/bin/env python3
"""Builds a PS1 `.mcd` whose save already has a chosen texture format.

Navigating the in-game settings menu headlessly is slow and fragile; the
memory card is a supported input to the emulator (`--memcard`), so seeding it
directly makes every texture format reachable in one deterministic boot.

The payload layout and CRC are the ones in `arduracer_core::save`, reimplemented
here so the two can be diffed by eye. If they ever disagree this file fails
loudly rather than silently booting the wrong format.
"""

import struct
import sys

BLOCK_SIZE = 8192
PAYLOAD_SIZE = 182
CHECKSUM_OFFSET = 180
SAVE_VERSION = 4
MAGIC = b"BAS-ARDURACER-01"

# Byte offsets within the payload, from the byte map in save.rs.
OFF_MAGIC = 0
OFF_VERSION = 16
OFF_MAX_UNLOCKED = 18
OFF_BEST_LAPS = 24
OFF_TEXTURE_FORMAT = 177
OFF_FRAMERATE = 178
OFF_CD_AUDIO = 179


def crc16(data: bytes) -> int:
    crc = 0xFFFF
    for b in data:
        crc ^= b << 8
        for _ in range(8):
            crc = ((crc << 1) ^ 0x1021) & 0xFFFF if crc & 0x8000 else (crc << 1) & 0xFFFF
    return crc


def build(texture_format: int, framerate: int = 0, cd_audio: int = 1,
          unlocked: int = 24, block_index: int = 0) -> bytes:
    payload = bytearray(PAYLOAD_SIZE)
    payload[OFF_MAGIC:OFF_MAGIC + 16] = MAGIC
    struct.pack_into("<H", payload, OFF_VERSION, SAVE_VERSION)
    payload[OFF_MAX_UNLOCKED] = unlocked
    # No best laps: any plausible value would need a CRC-valid plausible time,
    # and an all-zero `NO_BEST_LAP` is legal and sanitises to itself.
    payload[OFF_TEXTURE_FORMAT] = texture_format
    payload[OFF_FRAMERATE] = framerate
    payload[OFF_CD_AUDIO] = cd_audio
    struct.pack_into("<H", payload, CHECKSUM_OFFSET, crc16(bytes(payload[:CHECKSUM_OFFSET])))

    block = bytearray(BLOCK_SIZE)
    block[:PAYLOAD_SIZE] = payload

    # A memory card is 16 blocks of 8 KiB. Write the first block's header frames
    # so the emulator presents it as used rather than blank; the payload itself
    # is what the game reads.
    mcd = bytearray(128 * 1024)
    off = block_index * BLOCK_SIZE
    mcd[off:off + BLOCK_SIZE] = block
    # Directory frames live in blocks 15..127; mark block 0 in use so the card
    # does not look freshly formatted.
    dir_off = 15 * BLOCK_SIZE
    mcd[dir_off] = 0xFF
    mcd[dir_off + 1] = 0xFF
    mcd[dir_off + 2] = 0xFF
    mcd[dir_off + 3] = 0x0D  # number of directory frames in the first group
    mcd[dir_off + 4] = 0xFF
    mcd[dir_off + 5] = 0xFF
    mcd[dir_off + 6] = 0xFF
    mcd[dir_off + 7] = 0x08
    return bytes(mcd)


def main() -> int:
    if len(sys.argv) < 3:
        print("usage: seed_mcd.py <out.mcd> <texture_format 0..5> [cd_audio 0|1]")
        return 2
    out, fmt = sys.argv[1], int(sys.argv[2])
    cd = int(sys.argv[3]) if len(sys.argv) > 3 else 1
    data = build(fmt, cd_audio=cd)
    with open(out, "wb") as f:
        f.write(data)
    print(f"wrote {out}: texture_format={fmt} cd_audio={cd} crc=0x{crc16(data[:CHECKSUM_OFFSET]):04x}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())