use arduracer_core::host_grids;
use arduracer_core::math::{Fixed, Vec2};
use arduracer_core::timing::CheckpointGate;
use arduracer_core::tuning::CarTuning;
use arduracer_core::vehicle::{VehicleInput, VehicleState};
use arduracer_core::{TrackDef, TrackTile, TRACK_CAPETOWN, TRACK_LONDON, TRACK_MELBOURNE};

#[test]
fn test_unloaded_track_traps_car_in_barrier() {
    // When a level grid fails to load (tiles: None), tile_at reports Barrier for
    // every coordinate. This highlights the problem the user experienced when
    // level_grid::load failed on the city maps.
    let unloaded = TrackDef {
        name: "MOCK UNLOADED CITY",
        width: 240,
        height: 240,
        start_pos: TRACK_CAPETOWN.start_pos,
        start_heading: 1024,
        start_gate: CheckpointGate {
            x: 152,
            y: 97,
            width: 4,
            height: 4,
        },
        par_times: TRACK_CAPETOWN.par_times,
        checkpoint_count: 0,
        checkpoints: TRACK_CAPETOWN.checkpoints,
        tiles: None,
        route: &[],
        half_width: 32,
    };

    let tx = TrackDef::tile_x_of(unloaded.start_pos.x);
    let ty = TrackDef::tile_y_of(unloaded.start_pos.y);

    // On an unloaded track, every tile is a solid Barrier
    assert_eq!(unloaded.tile_at(tx, ty), TrackTile::Barrier);
    assert!(unloaded.tile_at(tx, ty).is_solid());

    let mut car = VehicleState::new(
        unloaded.start_pos,
        unloaded.start_heading,
        CarTuning::default(),
    );
    let input = VehicleInput {
        throttle: Fixed::ONE,
        steer: Fixed::ZERO,
        ..VehicleInput::default()
    };

    // Attempt to accelerate forward inside the barrier
    for _ in 0..60 {
        car.tick_on_track(input, &unloaded);
    }

    // If all tiles are solid (unloaded track), max_drivable_x/y collapse to 0,
    // clamping the car into the corner wall at (0, 0) with zero speed.
    assert_eq!(
        car.speed,
        Fixed::ZERO,
        "Car in an unloaded map must have 0 speed"
    );
    assert_eq!(
        car.position,
        Vec2::ZERO,
        "Car in an unloaded map is clamped to corner (0, 0)"
    );
}

#[test]
fn test_loaded_city_maps_start_on_road_and_can_move() {
    host_grids::ensure_installed();
    let cities: [&TrackDef; 3] = [&TRACK_CAPETOWN, &TRACK_MELBOURNE, &TRACK_LONDON];

    for city in cities {
        assert!(city.tiles.is_some(), "{}: tiles must be loaded", city.name);
        assert_eq!(
            city.tile_grid().len(),
            240 * 240,
            "{}: grid must be 240x240",
            city.name
        );

        let tx = TrackDef::tile_x_of(city.start_pos.x);
        let ty = TrackDef::tile_y_of(city.start_pos.y);
        let start_tile = city.tile_at(tx, ty);

        // Verify the car does NOT start in a wall
        assert!(
            !start_tile.is_solid(),
            "{}: start tile at ({tx}, {ty}) is solid ({:?})",
            city.name,
            start_tile
        );
        assert!(
            start_tile.is_road() || start_tile == TrackTile::Curb,
            "{}: start tile at ({tx}, {ty}) must be a road surface ({:?})",
            city.name,
            start_tile
        );

        let surface = city.surface_at(tx, ty);
        assert!(
            surface.traction() > Fixed::ZERO,
            "{}: start surface must have traction",
            city.name
        );
        assert!(
            surface.max_speed_factor() > Fixed::ZERO,
            "{}: start surface must allow speed",
            city.name
        );

        let mut car = VehicleState::new(city.start_pos, city.start_heading, CarTuning::default());
        let input = VehicleInput {
            throttle: Fixed::ONE,
            steer: Fixed::ZERO,
            ..VehicleInput::default()
        };

        // Run vehicle physics simulation for 60 ticks (1 second)
        for _ in 0..60 {
            car.tick_on_track(input, city);
        }

        assert!(
            car.speed > Fixed::from_int(2),
            "{}: car must accelerate to cruising speed on city map (speed = {:?})",
            city.name,
            car.speed
        );
        assert_ne!(
            car.position, city.start_pos,
            "{}: car position must advance after accelerating",
            city.name
        );
    }
}

#[test]
fn test_city_level_dims_and_disc_offsets_are_canonical() {
    use arduracer_core::levels::LEVEL_DIMS;

    assert_eq!(
        LEVEL_DIMS.len(),
        34,
        "LEVEL_DIMS must encompass all 31 circuits and 3 free-roam cities"
    );

    // Circuits 0..30 must be standard circuits, 31..33 are the 240x240 cities
    assert_eq!(LEVEL_DIMS[31], (240, 240), "Cape Town must be 240x240");
    assert_eq!(LEVEL_DIMS[32], (240, 240), "Melbourne must be 240x240");
    assert_eq!(LEVEL_DIMS[33], (240, 240), "London must be 240x240");

    let mut offset = 0usize;
    for (i, &(w, h)) in LEVEL_DIMS.iter().enumerate() {
        if i == 31 {
            assert_eq!(
                offset, 217_600,
                "Cape Town grid must start at byte 217,600 in LEVELS.BIN"
            );
        } else if i == 32 {
            assert_eq!(
                offset, 275_200,
                "Melbourne grid must start at byte 275,200 in LEVELS.BIN"
            );
        } else if i == 33 {
            assert_eq!(
                offset, 332_800,
                "London grid must start at byte 332,800 in LEVELS.BIN"
            );
        }
        offset += w as usize * h as usize;
    }
    assert_eq!(
        offset, 390_400,
        "Total uncompressed LEVELS.BIN size across all 34 levels must be 390,400 bytes"
    );
}

#[test]
fn test_city_block_sectors_match_circuit_block_indices() {
    use arduracer_core::visual_tex::{
        circuit_block_index, CAPETOWN_BLOCK_SECTORS, CIRCUIT_BLOCK_SECTORS, LONDON_BLOCK_SECTORS,
        MELBOURNE_BLOCK_SECTORS,
    };

    // Cape Town is circuit 31 (base block 34)
    assert_eq!(circuit_block_index(31, 0, 0), 34);
    for by in 0..3 {
        for bx in 0..3 {
            let expected_idx = circuit_block_index(31, bx, by);
            let block_idx = by * 3 + bx;
            assert_eq!(
                CAPETOWN_BLOCK_SECTORS[block_idx], CIRCUIT_BLOCK_SECTORS[expected_idx],
                "Cape Town block ({bx}, {by}) sector entry mismatch"
            );
            assert!(
                CAPETOWN_BLOCK_SECTORS[block_idx].sector_count > 0,
                "Cape Town block ({bx}, {by}) must have non-zero sectors"
            );
        }
    }

    // Melbourne is circuit 32 (base block 43)
    assert_eq!(circuit_block_index(32, 0, 0), 43);
    for by in 0..3 {
        for bx in 0..3 {
            let expected_idx = circuit_block_index(32, bx, by);
            let block_idx = by * 3 + bx;
            assert_eq!(
                MELBOURNE_BLOCK_SECTORS[block_idx], CIRCUIT_BLOCK_SECTORS[expected_idx],
                "Melbourne block ({bx}, {by}) sector entry mismatch"
            );
            assert!(
                MELBOURNE_BLOCK_SECTORS[block_idx].sector_count > 0,
                "Melbourne block ({bx}, {by}) must have non-zero sectors"
            );
        }
    }

    // London is circuit 33 (base block 52)
    assert_eq!(circuit_block_index(33, 0, 0), 52);
    for by in 0..3 {
        for bx in 0..3 {
            let expected_idx = circuit_block_index(33, bx, by);
            let block_idx = by * 3 + bx;
            assert_eq!(
                LONDON_BLOCK_SECTORS[block_idx], CIRCUIT_BLOCK_SECTORS[expected_idx],
                "London block ({bx}, {by}) sector entry mismatch"
            );
            assert!(
                LONDON_BLOCK_SECTORS[block_idx].sector_count > 0,
                "London block ({bx}, {by}) must have non-zero sectors"
            );
        }
    }
}
