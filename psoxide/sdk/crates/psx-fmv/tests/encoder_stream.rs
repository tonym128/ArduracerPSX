// SPDX-License-Identifier: GPL-2.0-or-later
//! Round trip against real encoder output: demux every frame of a
//! 2048-byte-sector `.str` from psxavenc (`-t strv -v v2`) and decode it.
//!
//! Set `PSX_FMV_TEST_STR=/path/movie.str` to run it; without the variable
//! the test passes trivially, so `cargo test` needs no media on disk.

use psx_fmv::bs;
use psx_fmv::str::FrameAssembler;

#[test]
fn decodes_every_frame_of_a_psxavenc_stream() {
    let Ok(path) = std::env::var("PSX_FMV_TEST_STR") else {
        return;
    };
    let data = std::fs::read(&path).expect("read STR");
    let mut asm = FrameAssembler::new();
    let mut buf = vec![0u8; 32 * 2016];
    let mut out = vec![0u16; 128 * 1024];
    let mut frames = 0;
    for sector in data.chunks_exact(2048) {
        let Some(frame) = asm.add(sector, &mut buf) else {
            continue;
        };
        let header = bs::Header::parse(&buf).unwrap();
        let mbs = (frame.width as u32).div_ceil(16) * (frame.height as u32).div_ceil(16);
        let mut pumps = 0;
        let words = bs::decode_frame(
            &buf[..frame.size as usize],
            &mut out,
            mbs,
            (frame.height as u32).div_ceil(16),
            &mut || pumps += 1,
        )
        .unwrap_or_else(|e| panic!("frame {}: {e:?}", frame.number));
        // The encoder's announced size is the exact padded run-length size.
        assert_eq!(words, header.mdec_words as usize, "frame {}", frame.number);
        assert_eq!(
            pumps,
            (frame.width as u32).div_ceil(16),
            "one pump per column"
        );
        // Exactly width/16 * height/16 macroblocks: count DC-bearing blocks.
        let mut blocks = 0;
        let mut at_block_start = true;
        for &hw in &out[..words * 2] {
            if at_block_start && hw != bs::END_OF_BLOCK {
                blocks += 1;
                at_block_start = false;
            } else if hw == bs::END_OF_BLOCK {
                at_block_start = true;
            }
        }
        assert_eq!(blocks, mbs * 6, "frame {}", frame.number);
        frames += 1;
    }
    assert_eq!(asm.dropped, 0);
    assert!(frames > 0);
    eprintln!("decoded {frames} frames from {path}");
}

/// Same round trip against a CD-XA STR (2336-byte sectors, psxavenc
/// `-t str`): skip the audio sectors the drive would have routed to the
/// SPU and decode what is left.
#[test]
fn decodes_every_frame_of_a_cdxa_str() {
    let Ok(path) = std::env::var("PSX_FMV_TEST_XA_STR") else {
        return;
    };
    let data = std::fs::read(&path).expect("read STR");
    let mut asm = psx_fmv::str::FrameAssembler::new();
    let mut buf = vec![0u8; 32 * 2016];
    let mut out = vec![0u16; 128 * 1024];
    let mut frames = 0;
    let mut video_sectors = 0;
    let mut markers = 0;
    for raw in data.chunks_exact(2336) {
        // Sub-header pairs occupy the first 8 bytes; user data follows.
        let sector = &raw[8..8 + 2048];
        let Some(chunk) = psx_fmv::str::Chunk::parse(sector) else {
            continue;
        };
        // A zero-size frame is the trailing end-of-file marker an STR carries
        // so a streaming player can find the boundary; a real player ends the
        // movie on its frame number without decoding it.
        if chunk.size == 0 {
            markers += 1;
            continue;
        }
        video_sectors += 1;
        let Some(frame) = asm.add(sector, &mut buf) else {
            continue;
        };
        let mbs = (frame.width as u32).div_ceil(16) * (frame.height as u32).div_ceil(16);
        let words = bs::decode_frame(
            &buf[..frame.size as usize],
            &mut out,
            mbs,
            (frame.height as u32).div_ceil(16),
            &mut || {},
        )
        .unwrap_or_else(|e| panic!("frame {}: {e:?}", frame.number));
        assert_eq!(words, bs::Header::parse(&buf).unwrap().mdec_words as usize);
        frames += 1;
    }
    eprintln!(
        "decoded {frames} frames from {video_sectors} video sectors \
         ({markers} end-of-file markers) in {path}"
    );
    assert_eq!(asm.dropped, 0);
    assert_eq!(markers, 1, "expected exactly one end-of-file marker");
    assert!(frames > 0);
}
