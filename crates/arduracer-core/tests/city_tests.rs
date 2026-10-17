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
