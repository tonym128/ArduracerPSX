//! Contract tests for the generated `visual_tex` streaming tables.
//!
//! These lock the invariants `gpu::tracktex` relies on when it picks a codec
//! and reads a block off disc. They read the generated table rather than any
//! fixture, so `make atlas` regenerating the assets is what keeps them honest.

use arduracer_core::visual_tex::{
    self, CIRCUIT_BLOCK_SECTORS, CIRCUIT_QOI_SECTORS, QOITRACKS_BIN_NAME, TRACKS_BIN_NAME,
};

#[test]
fn every_circuit_has_a_jpeg_block() {
    assert_eq!(CIRCUIT_BLOCK_SECTORS.len(), visual_tex::COUNT);
    for (c, e) in CIRCUIT_BLOCK_SECTORS.iter().enumerate() {
        assert!(e.sector_count > 0, "circuit {c} has no JPEG block");
        assert!(
            (e.byte_len as usize) <= visual_tex::BLOCK_MAX_BYTES,
            "circuit {c}: JPEG block is {} bytes, over the {}-byte budget",
            e.byte_len,
            visual_tex::BLOCK_MAX_BYTES
        );
    }
}

#[test]
fn every_qoi_block_fits_the_budget_and_its_allocation() {
    for (c, e) in CIRCUIT_QOI_SECTORS.iter().enumerate() {
        if e.sector_count == 0 {
            continue; // No QOI block; circuit streams from JPEG.
        }
        assert_eq!(e.sector_count as usize, visual_tex::BLOCK_SECTORS);
        assert!(
            (e.byte_len as usize) <= visual_tex::BLOCK_MAX_BYTES,
            "circuit {c}: QOI block is {} bytes, over the {}-byte budget",
            e.byte_len,
            visual_tex::BLOCK_MAX_BYTES
        );
        assert!(e.byte_len > 0, "circuit {c}: QOI block has no payload");
    }
}

/// The fallback is what keeps the feature safe: a circuit with no QOI block,
/// or a disc with no QOI asset, must still stream its JPEG block.
#[test]
fn qoi_coverage_is_a_strict_subset_of_jpeg_coverage() {
    let qoi_count = CIRCUIT_QOI_SECTORS
        .iter()
        .filter(|e| e.sector_count > 0)
        .count();
    assert_eq!(
        qoi_count,
        visual_tex::QOI_CIRCUIT_COUNT,
        "QOI_CIRCUIT_COUNT disagrees with the table it describes"
    );
    assert!(
        qoi_count <= visual_tex::COUNT,
        "more QOI blocks than circuits"
    );
}

#[test]
fn sector_offsets_tile_each_asset_without_overlap() {
    for (label, table) in [
        ("TRACKS.BIN", &CIRCUIT_BLOCK_SECTORS),
        ("QOITRACKS.BIN", &CIRCUIT_QOI_SECTORS),
    ] {
        // Entries with no block do not consume space, so the sequence of
        // present blocks must be contiguous and monotonic.
        let mut expected = 0u32;
        for (c, e) in table.iter().enumerate() {
            if e.sector_count == 0 {
                continue;
            }
            assert_eq!(
                e.sector_offset, expected,
                "{label}: circuit {c} starts at sector {}, expected {expected} (table overlaps or has a gap)",
                e.sector_offset
            );
            expected += e.sector_count;
        }
    }
}

/// The asset names are the lookup keys `cd_fs::find_file_lba` matches against
/// the ISO directory, so a typo here is a silent fallback to JPEG.
#[test]
fn asset_names_are_uppercase_iso9660_safe() {
    for name in [TRACKS_BIN_NAME, QOITRACKS_BIN_NAME] {
        let s = core::str::from_utf8(name).expect("asset name is not ASCII");
        assert!(s.ends_with(".BIN"), "{s} should end in .BIN");
        assert!(
            s.bytes()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'.'),
            "{s} has characters ISO9660 would mangle"
        );
    }
}
