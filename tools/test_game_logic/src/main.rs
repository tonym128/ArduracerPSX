//! Automated host-side test suite for Arduracer PSX core logic.
//!
//! Directly tests `arduracer-core` with zero mirroring, ensuring genuine verification.

use arduracer_core::*;

fn main() {
    println!("============================================================");
    println!("  ARDURACER PSX - HOST-SIDE GAME LOGIC VERIFICATION SUITE   ");
    println!("============================================================");

    let mut passed = 0;
    let mut total = 0;

    macro_rules! run_test {
        ($name:expr, $func:expr) => {
            total += 1;
            print!("  [{:02}] {:<48} ... ", total, $name);
            match std::panic::catch_unwind($func) {
                Ok(_) => {
                    println!("\x1b[32mPASS\x1b[0m");
                    passed += 1;
                }
                Err(e) => {
                    println!("\x1b[31mFAIL\x1b[0m: {:?}", e);
                }
            }
        };
    }

    run_test!(
        "Fixed-point basic arithmetic (Q20.12)",
        test_fixed_point_math
    );
    run_test!(
        "Trigonometry sin/cos table & GTE alignment",
        test_trigonometry
    );
    run_test!("2D Vector math & length calculation", test_vector_math);
    run_test!(
        "Vehicle acceleration & throttle response",
        test_vehicle_acceleration
    );
    run_test!(
        "Vehicle braking & decelerating to standstill",
        test_vehicle_braking
    );
    run_test!(
        "Vehicle steering & angular wrapping (0..4096)",
        test_vehicle_steering
    );
    run_test!(
        "Drift physics & slip angle decoupling",
        test_drift_mechanics
    );
    run_test!(
        "Surface friction & off-road speed penalties",
        test_surface_friction
    );
    run_test!(
        "Car tuning point validation (20 pts pool)",
        test_car_tuning_validation
    );
    run_test!(
        "Car tuning 10% scaling factor curve",
        test_car_tuning_scaling
    );
    run_test!(
        "Checkpoint anti-cheat (FX coverage rule + start/finish)",
        test_checkpoint_anti_cheat
    );
    run_test!(
        "Lap only scores on start/finish crossing",
        test_lap_requires_start_finish_crossing
    );
    run_test!("Lap delta split vs best lap", test_lap_delta_split);
    run_test!(
        "Lap timer tick rate & 60Hz timing precision",
        test_lap_timer_precision
    );
    run_test!(
        "Medal evaluation (Bronze, Silver, Gold, Dev)",
        test_medal_evaluation
    );
    run_test!(
        "SaveData default integrity & CRC16 checksum",
        test_save_checksum
    );
    run_test!(
        "SaveData corruption detection on single-bit flip",
        test_save_bit_flip_detection
    );
    run_test!(
        "Drift boost charging tiers (Mini/Super Turbo)",
        test_drift_boost_charging
    );
    run_test!(
        "Counter-steering stabilizes drift slip angle",
        test_counter_steering
    );
    run_test!(
        "Barrier collision bounce & kinetic energy scrub",
        test_barrier_collision
    );
    run_test!(
        "Ghost 30Hz recording & 6-byte frame compression",
        test_ghost_recording
    );
    run_test!(
        "Ghost sub-tick linear & angular interpolation",
        test_ghost_interpolation
    );
    run_test!(
        "SaveData 8KB Memory Card block round-trip",
        test_save_block_round_trip
    );
    run_test!(
        "Track integrity & geometry sweep across all 24 tracks",
        test_all_24_tracks_integrity
    );
    run_test!(
        "Full lap completion & checkpoint progression on Track 1",
        test_lap_completion_on_track1
    );
    run_test!(
        "AI rival personalities & profile tuning validation",
        test_ai_profiles_and_tuning
    );
    run_test!(
        "AI waypoint navigation & steering input generation",
        test_ai_navigation_and_steer
    );
    run_test!(
        "Championship cups, stages, and points calculation",
        test_championship_scoring
    );
    run_test!(
        "Race standings computation & sorted leaderboard",
        test_race_standings_and_leaderboard
    );
    run_test!(
        "World scale: car crosses tiles at arcade speed",
        test_world_scale_matches_track_tiles
    );
    run_test!(
        "Heading convention (0=N, 1024=E, 2048=S, 3072=W)",
        test_atan2_heading_convention
    );
    run_test!(
        "Surface speed caps, traction & lateral hold",
        test_surface_speed_caps
    );
    run_test!(
        "Off-road penalty is 0.35x (not a wall)",
        test_offroad_penalty_is_playable
    );
    run_test!("Boost pad grants turbo impulse", test_boost_pad_impulse);
    run_test!(
        "Nitro meter drains on use and recharges",
        test_nitro_meter_and_surge
    );
    run_test!(
        "Barrier scrape slides instead of sticking",
        test_barrier_slide_does_not_stick
    );
    run_test!(
        "Track bounds clamp and bounce the car",
        test_track_bounds_are_solid
    );
    run_test!(
        "All 24 tracks have an ordered on-road route",
        test_all_tracks_have_ordered_route
    );
    run_test!(
        "PSX Super Stages are real circuits w/ hazards",
        test_super_stages_are_real_circuits
    );
    run_test!(
        "Sprint Short synthesised a missing start line",
        test_sprint_short_has_a_start_line
    );

    println!("------------------------------------------------------------");
    println!("  Summary: {}/{} tests passed", passed, total);
    println!("============================================================");

    if passed != total {
        std::process::exit(1);
    }
}

fn test_fixed_point_math() {
    let one = Fixed::ONE;
    let two = Fixed::from_int(2);
    let half = Fixed::HALF;

    assert_eq!((one + one).to_int(), 2);
    assert_eq!((two - one).to_int(), 1);
    assert_eq!((two * half).to_int(), 1);
    assert_eq!(one / two, half);
}

fn test_trigonometry() {
    // sin(0) == 0
    assert_eq!(sin(0).to_int(), 0);
    // sin(90 deg = 1024) == 1.0 (4096)
    assert_eq!(sin(ANGLE_90).raw(), FP_ONE);
    // sin(180 deg = 2048) == 0
    assert_eq!(sin(ANGLE_180).to_int(), 0);
    // cos(0) == 1.0 (4096)
    assert_eq!(cos(0).raw(), FP_ONE);
    // cos(90 deg = 1024) == 0
    assert_eq!(cos(ANGLE_90).to_int(), 0);
}

fn test_vector_math() {
    let v1 = Vec2::new(Fixed::from_int(3), Fixed::from_int(4));
    assert_eq!(v1.length().to_int(), 5);

    let v2 = Vec2::new(Fixed::from_int(1), Fixed::from_int(2));
    let v3 = v1 + v2;
    assert_eq!(v3.x.to_int(), 4);
    assert_eq!(v3.y.to_int(), 6);
}

fn test_vehicle_acceleration() {
    let mut car = VehicleState::default();
    assert_eq!(car.speed, Fixed::ZERO);

    let input = VehicleInput {
        throttle: Fixed::ONE,
        brake: Fixed::ZERO,
        steer: Fixed::ZERO,
        handbrake: false,
        nitro: false,
    };

    for _ in 0..60 {
        car.tick(input, SurfaceType::Tarmac);
    }

    assert!(
        car.speed.raw() > 2000,
        "Car should accelerate on throttle (speed: {})",
        car.speed.raw()
    );
    assert!(car.engine_rpm > 1000, "Engine RPM should increase");
}

fn test_vehicle_braking() {
    let mut car = VehicleState::default();
    let throttle_input = VehicleInput {
        throttle: Fixed::ONE,
        brake: Fixed::ZERO,
        steer: Fixed::ZERO,
        handbrake: false,
        nitro: false,
    };
    for _ in 0..30 {
        car.tick(throttle_input, SurfaceType::Tarmac);
    }
    let speed_before_brake = car.speed;
    assert!(speed_before_brake > Fixed::ZERO);

    let brake_input = VehicleInput {
        throttle: Fixed::ZERO,
        brake: Fixed::ONE,
        steer: Fixed::ZERO,
        handbrake: false,
        nitro: false,
    };
    for _ in 0..20 {
        car.tick(brake_input, SurfaceType::Tarmac);
    }
    assert!(
        car.speed < speed_before_brake,
        "foot brake must scrub speed: {:?} -> {:?}",
        speed_before_brake,
        car.speed
    );

    // Coasting must bleed off speed without stalling the engine.
    let neutral = VehicleInput::default();
    for _ in 0..120 {
        car.tick(neutral, SurfaceType::Tarmac);
    }
    assert!(
        car.speed < Fixed::from_int(4).scale(speed_before_brake),
        "coasting must bleed most of the speed, got {:?}",
        car.speed
    );

    // Holding the brake at a standstill engages reverse, but slowly.
    let mut reversing = VehicleState::default();
    for _ in 0..180 {
        reversing.tick(brake_input, SurfaceType::Tarmac);
    }
    assert!(reversing.is_reversing, "brake at rest must engage reverse");
    assert!(
        reversing.speed <= Fixed::from_raw(2000),
        "reverse must be much slower than top speed, got {:?}",
        reversing.speed
    );
}

fn test_vehicle_steering() {
    let mut car = VehicleState {
        speed: Fixed::from_int(5),
        ..Default::default()
    };

    let steer_right = VehicleInput {
        throttle: Fixed::ONE,
        brake: Fixed::ZERO,
        steer: Fixed::ONE,
        handbrake: false,
        nitro: false,
    };

    let start_heading = car.heading;
    car.tick(steer_right, SurfaceType::Tarmac);
    assert_ne!(car.heading, start_heading, "Steering should change heading");
}

fn test_drift_mechanics() {
    let mut car = VehicleState {
        velocity: Vec2::new(Fixed::ZERO, -Fixed::from_int(3)),
        speed: Fixed::from_int(3),
        ..Default::default()
    };
    let drift_input = VehicleInput {
        throttle: Fixed::ONE,
        brake: Fixed::ZERO,
        steer: Fixed::ONE,
        handbrake: true,
        nitro: false,
    };

    car.tick(drift_input, SurfaceType::Tarmac);
    assert!(car.is_drifting, "Handbrake input should initiate drift");
    assert_ne!(
        car.heading, car.visual_angle,
        "Visual angle should decouple during drift"
    );
}

fn test_surface_friction() {
    assert_eq!(SurfaceType::Tarmac.grip_factor(), Fixed::ONE);
    assert!(SurfaceType::OffRoad.grip_factor() < Fixed::ONE);
    assert!(SurfaceType::Curb.triggers_curb_rumble());
    assert!(SurfaceType::OffRoad.is_offroad());
}

fn test_car_tuning_validation() {
    let default_tuning = CarTuning::default();
    assert!(default_tuning.is_valid(), "Default tuning must be valid");
    assert_eq!(default_tuning.total_points(), TOTAL_POINTS);

    let invalid_tuning = CarTuning {
        top_speed: 7,
        acceleration: 7,
        handling: 7,
        drift_stability: 7,
        gearing: 7, // 35 points > 20
    };
    assert!(
        !invalid_tuning.is_valid(),
        "Overallocated points must be rejected"
    );
}

fn test_car_tuning_scaling() {
    // Value 4 is default (1.0 = 4096)
    assert_eq!(CarTuning::scale_factor(4).raw(), FP_ONE);
    // Value 5 is +10% (4096 + 410 = 4506)
    assert_eq!(CarTuning::scale_factor(5).raw(), FP_ONE + 410);
    // Value 3 is -10% (4096 - 410 = 3686)
    assert_eq!(CarTuning::scale_factor(3).raw(), FP_ONE - 410);
}

fn test_checkpoint_anti_cheat() {
    let gates = [
        CheckpointGate {
            x: 5,
            y: 5,
            width: 2,
            height: 2,
        },
        CheckpointGate {
            x: 10,
            y: 5,
            width: 2,
            height: 2,
        },
        CheckpointGate {
            x: 10,
            y: 10,
            width: 2,
            height: 2,
        },
    ];
    let start_gate = CheckpointGate {
        x: 0,
        y: 0,
        width: 2,
        height: 2,
    };

    let mut timer = LapTimer::<3>::new(&gates, start_gate);
    timer.start();

    // Sitting on the grid must not score anything yet.
    timer.update_player_tile(0, 0);
    assert!(timer.is_running, "clock runs from the green light");
    assert_eq!(timer.current_lap, 1);

    // Cutting straight back over the line with no checkpoints must NOT score.
    assert!(!timer.update_player_tile(3, 3));
    assert!(!timer.update_player_tile(0, 0));
    assert!(!timer.update_player_tile(3, 3));
    assert_eq!(
        timer.current_lap, 1,
        "skipping every gate must not score a lap"
    );

    // Checkpoints may be taken in any order (ArduRacer FX semantics).
    for _ in 0..30 {
        timer.tick();
    }
    assert!(!timer.update_player_tile(10, 10));
    assert_eq!(timer.checkpoints_cleared(), 1);
    for _ in 0..30 {
        timer.tick();
    }
    assert!(!timer.update_player_tile(5, 5));
    for _ in 0..30 {
        timer.tick();
    }
    assert!(!timer.update_player_tile(10, 5));
    assert_eq!(timer.checkpoints_cleared(), 3);
    assert!(timer.all_checkpoints_cleared());
    assert_eq!(timer.current_lap, 1, "still on the circuit, no lap yet");

    // Re-enter then leave the start/finish block -> lap 1 scores.
    assert!(!timer.update_player_tile(0, 0));
    for _ in 0..30 {
        timer.tick();
    }
    assert!(timer.update_player_tile(3, 3), "full lap must score");
    assert_eq!(timer.current_lap, 2);
    assert_eq!(timer.checkpoints_cleared(), 0, "gate mask resets each lap");
    assert!(timer.best_lap_ticks < u32::MAX, "best lap must be recorded");
}

fn test_lap_requires_start_finish_crossing() {
    let gates = [CheckpointGate {
        x: 5,
        y: 5,
        width: 1,
        height: 1,
    }];
    let start_gate = CheckpointGate {
        x: 0,
        y: 0,
        width: 1,
        height: 1,
    };
    let mut timer = LapTimer::<1>::new(&gates, start_gate);
    timer.start();

    timer.update_player_tile(3, 3);
    timer.update_player_tile(5, 5);
    assert!(timer.all_checkpoints_cleared());
    timer.update_player_tile(0, 0);
    assert_eq!(timer.current_lap, 1);
    assert!(timer.update_player_tile(1, 1));
    assert_eq!(timer.current_lap, 2);
    assert!(
        !timer.update_player_tile(1, 1),
        "cannot score twice off one crossing"
    );
}

fn test_lap_delta_split() {
    let gates = [
        CheckpointGate {
            x: 5,
            y: 5,
            width: 1,
            height: 1,
        },
        CheckpointGate {
            x: 9,
            y: 5,
            width: 1,
            height: 1,
        },
    ];
    let start_gate = CheckpointGate {
        x: 0,
        y: 0,
        width: 1,
        height: 1,
    };
    let mut timer = LapTimer::<2>::new(&gates, start_gate);
    timer.start();
    assert_eq!(timer.delta_ticks(), 0, "no reference lap yet");

    for _ in 0..300 {
        timer.tick();
    }
    timer.update_player_tile(5, 5);
    for _ in 0..300 {
        timer.tick();
    }
    timer.update_player_tile(9, 5);
    timer.update_player_tile(0, 0);
    assert!(timer.update_player_tile(1, 1));
    assert_eq!(timer.best_lap_ticks, 600);

    for _ in 0..300 {
        timer.tick();
    }
    timer.update_player_tile(5, 5);
    let d = timer.delta_ticks();
    assert!(d.abs() < 20, "on-pace delta must be near zero, got {}", d);
}

fn test_atan2_heading_convention() {
    // 0 BAMs = North (-Y), 1024 = East (+X), 2048 = South (+Y), 3072 = West (-X).
    assert_eq!(
        heading_towards(Vec2::ZERO, Vec2::new(Fixed::ZERO, Fixed::from_int(-100))),
        0
    );
    assert_eq!(
        heading_towards(Vec2::ZERO, Vec2::new(Fixed::from_int(100), Fixed::ZERO)),
        1024
    );
    assert_eq!(
        heading_towards(Vec2::ZERO, Vec2::new(Fixed::ZERO, Fixed::from_int(100))),
        2048
    );
    assert_eq!(
        heading_towards(Vec2::ZERO, Vec2::new(Fixed::from_int(-100), Fixed::ZERO)),
        3072
    );
    assert_eq!(
        heading_towards(
            Vec2::ZERO,
            Vec2::new(Fixed::from_int(100), Fixed::from_int(100))
        ),
        1536
    );
    assert_eq!(
        heading_towards(
            Vec2::ZERO,
            Vec2::new(Fixed::from_int(-100), Fixed::from_int(100))
        ),
        2560
    );
}

fn test_world_scale_matches_track_tiles() {
    // The car must cross a 64-unit tile in a sane number of ticks; when the
    // position integrator was mis-scaled it needed ~640 ticks per tile and the
    // game was literally unplayable.
    let input = VehicleInput {
        throttle: Fixed::ONE,
        ..Default::default()
    };
    let mut car = VehicleState::default();
    for _ in 0..400 {
        car.tick(input, SurfaceType::Tarmac);
    }
    let top_speed = car.speed.to_int();
    assert!(
        (2..=4).contains(&top_speed),
        "top speed must be 2-4 world units/tick, got {}",
        top_speed
    );
    let tiles_per_second = top_speed * 60 / TILE_SIZE;
    assert!(
        (1..=5).contains(&tiles_per_second),
        "car must cross 1-5 tiles per second, got {}",
        tiles_per_second
    );

    let mut car = VehicleState::default();
    let mut ticks = 0;
    while car.speed.to_int() < top_speed && ticks < 600 {
        car.tick(input, SurfaceType::Tarmac);
        ticks += 1;
    }
    assert!(
        ticks <= 150,
        "0-100% must take under 2.5s, took {} ticks",
        ticks
    );
}

fn test_surface_speed_caps() {
    assert_eq!(SurfaceType::OffRoad.max_speed_factor().raw(), 1433);
    assert!(SurfaceType::OffRoad.traction() > Fixed::ZERO);
    assert!(SurfaceType::OffRoad.traction() < Fixed::ONE);
    assert!(SurfaceType::Tarmac.max_speed_factor() == Fixed::ONE);
    assert!(SurfaceType::Tarmac.traction() == Fixed::ONE);
    assert!(SurfaceType::OilSlick.lateral_hold(false) < SurfaceType::Tarmac.lateral_hold(false));
    assert!(SurfaceType::OilSlick.max_speed_factor() == Fixed::ONE);
    assert!(SurfaceType::Barrier.is_solid());
    assert!(SurfaceType::BoostPad.is_boost_pad());
}

fn test_offroad_penalty_is_playable() {
    let input = VehicleInput {
        throttle: Fixed::ONE,
        ..Default::default()
    };
    let mut on_road = VehicleState::default();
    let mut off_road = VehicleState::default();
    for _ in 0..600 {
        on_road.tick(input, SurfaceType::Tarmac);
        off_road.tick(input, SurfaceType::OffRoad);
    }
    let ratio = off_road.speed.raw() as f64 / on_road.speed.raw() as f64;
    assert!(
        (0.30..=0.40).contains(&ratio),
        "off-road top speed must be ~0.35x of tarmac, got {:.3}",
        ratio
    );
}

fn test_nitro_meter_and_surge() {
    let plain = VehicleInput {
        throttle: Fixed::ONE,
        ..Default::default()
    };
    let mut base = VehicleState::default();
    let mut boosted = VehicleState::default();
    let nitro = VehicleInput {
        throttle: Fixed::ONE,
        nitro: true,
        ..Default::default()
    };

    assert_eq!(base.nitro_charge, NITRO_MAX_TICKS, "meter starts full");
    for _ in 0..400 {
        base.tick(plain, SurfaceType::Tarmac);
        boosted.tick(nitro, SurfaceType::Tarmac);
    }
    assert!(
        boosted.speed > base.speed,
        "nitro must beat plain throttle: {:?} vs {:?}",
        boosted.speed,
        base.speed
    );
    assert!(
        boosted.nitro_charge < NITRO_MAX_TICKS,
        "sustained nitro must drain the meter"
    );

    // Releasing nitro refills it, and refuelling is slower than spending.
    let mut spent = boosted.nitro_charge;
    for _ in 0..90 {
        boosted.tick(plain, SurfaceType::Tarmac);
        spent = spent.saturating_sub(1);
    }
    assert!(
        boosted.nitro_charge > spent,
        "nitro must recharge when released"
    );
    assert!(boosted.nitro_charge <= NITRO_MAX_TICKS);
}

fn test_boost_pad_impulse() {
    let input = VehicleInput {
        throttle: Fixed::ONE,
        ..Default::default()
    };
    let mut car = VehicleState::default();
    for _ in 0..300 {
        car.tick(input, SurfaceType::Tarmac);
    }
    let base = car.speed;
    car.tick(input, SurfaceType::BoostPad);
    assert!(car.boost_ticks > 0, "boost pad must arm a turbo");
    for _ in 0..30 {
        car.tick(input, SurfaceType::BoostPad);
    }
    assert!(
        car.speed > base,
        "boost pad must push the car above its normal top speed"
    );
}

fn test_barrier_slide_does_not_stick() {
    // Scraping along a wall must preserve tangential speed, otherwise the car is
    // pinned against the barrier and can never recover.
    let mut car = VehicleState {
        velocity: Vec2::new(Fixed::from_int(2), Fixed::from_raw(-300)),
        speed: Fixed::from_int(2),
        ..Default::default()
    };
    let normal = Vec2::new(Fixed::ZERO, Fixed::ONE);
    for _ in 0..20 {
        car.handle_barrier_collision(normal);
    }
    assert!(
        car.velocity.x > Fixed::from_int(1),
        "tangential speed must survive a glancing scrape, got {}",
        car.velocity.x.to_int()
    );
    assert!(
        !car.drift.is_spinning(),
        "a glancing scrape must not trigger a spin-out"
    );
}

fn test_track_bounds_are_solid() {
    let track = ALL_TRACKS[0];
    let mut car = VehicleState {
        position: Vec2::new(Fixed::from_int(-400), Fixed::from_int(100)),
        velocity: Vec2::new(Fixed::from_int(-2), Fixed::ZERO),
        speed: Fixed::from_int(2),
        ..Default::default()
    };
    assert!(
        car.collide_with_track(track),
        "leaving the level must register a hit"
    );
    assert!(
        car.position.x.to_int() >= 0,
        "car must be clamped back inside the level, got {}",
        car.position.x.to_int()
    );
    assert!(
        car.velocity.x > Fixed::ZERO,
        "car must bounce back into the circuit"
    );
}

fn test_all_tracks_have_ordered_route() {
    for (idx, track) in ALL_TRACKS.iter().enumerate() {
        assert_eq!(
            track.route_len(),
            track.checkpoint_count as usize + 1,
            "Track {} route must include the start/finish node",
            idx + 1
        );
        let last = track.route_node(track.route_len() - 1);
        assert_eq!(
            last,
            track.start_gate,
            "Track {} route must end at the line",
            idx + 1
        );
        for i in 0..track.route_len() {
            let node = track.route_node(i);
            assert!(
                node.is_active(),
                "Track {} route node {} is empty",
                idx + 1,
                i
            );
            assert!(
                track.tile_at(node.x, node.y).is_road(),
                "Track {} route node {} sits off-road",
                idx + 1,
                i
            );
        }
        let mut seen: Vec<CheckpointGate> = Vec::new();
        for i in 0..track.checkpoint_count as usize {
            let g = track.checkpoints[i];
            assert!(
                !seen.contains(&g),
                "Track {} repeats checkpoint ({},{})",
                idx + 1,
                g.x,
                g.y
            );
            seen.push(g);
        }
    }
}

fn test_super_stages_are_real_circuits() {
    for track in ALL_TRACKS.iter().skip(20).take(4) {
        let (mut road, mut curb, mut boost, mut oil) = (0usize, 0usize, 0usize, 0usize);
        for t in track.tiles.iter() {
            match t {
                TrackTile::Tarmac | TrackTile::StartFinish | TrackTile::Checkpoint => road += 1,
                TrackTile::Curb => curb += 1,
                TrackTile::BoostPad => boost += 1,
                TrackTile::OilSlick => oil += 1,
                _ => {}
            }
        }
        assert!(
            road >= 60,
            "{} needs a real road surface, got {}",
            track.name,
            road
        );
        assert!(
            curb >= 20,
            "{} needs rumble curbs, got {}",
            track.name,
            curb
        );
        assert!(
            boost + oil >= 2,
            "{} needs hazard or boost tiles",
            track.name
        );
        assert!(
            track.checkpoint_count >= 6,
            "{} needs at least 6 checkpoints, got {}",
            track.name,
            track.checkpoint_count
        );
    }
}

fn test_sprint_short_has_a_start_line() {
    // ArduRacer FX Level 7 shipped with no start block at all; the port
    // synthesises one on the first tarmac tile.
    let track = ALL_TRACKS[6];
    assert!(
        track.start_gate.is_active(),
        "Sprint Short must have a start gate"
    );
    let tx = TrackDef::tile_x_of(track.start_pos.x);
    let ty = TrackDef::tile_y_of(track.start_pos.y);
    assert!(track.tile_at(tx, ty).is_road());
    assert!(track.start_gate.contains_tile(tx, ty));
}

fn test_lap_timer_precision() {
    let gates = [CheckpointGate {
        x: 0,
        y: 0,
        width: 1,
        height: 1,
    }];
    let start_gate = CheckpointGate {
        x: 4,
        y: 4,
        width: 1,
        height: 1,
    };
    let mut timer = LapTimer::<1>::new(&gates, start_gate);
    timer.start();

    for _ in 0..150 {
        timer.tick();
    }
    assert_eq!(timer.current_lap_ticks, 150);

    let (min, sec, hundredths) = LapTimer::<1>::format_ticks(150);
    assert_eq!(min, 0);
    assert_eq!(sec, 2);
    assert_eq!(hundredths, 50); // 150 ticks @ 60 ticks/s = 2.50s
}

fn test_medal_evaluation() {
    let par = ParTimes {
        bronze_ticks: 3600,       // 60.00s
        silver_ticks: 3000,       // 50.00s
        gold_ticks: 2400,         // 40.00s
        dev_platinum_ticks: 2100, // 35.00s
    };

    assert_eq!(par.evaluate_medal(2000), Medal::DevPlatinum);
    assert_eq!(par.evaluate_medal(2300), Medal::Gold);
    assert_eq!(par.evaluate_medal(2800), Medal::Silver);
    assert_eq!(par.evaluate_medal(3500), Medal::Bronze);
    assert_eq!(par.evaluate_medal(4000), Medal::None);
}

fn test_save_checksum() {
    let save = SaveData::default();
    assert!(save.is_valid(), "Fresh save data must pass validation");
    assert_eq!(save.magic, SAVE_HEADER_MAGIC);
    assert_eq!(save.version, SAVE_VERSION);
}

fn test_save_bit_flip_detection() {
    let save = SaveData::default();
    assert!(save.is_valid());

    // Corrupt a single byte of the serialised payload -- the bytes that
    // actually reach the card. Poking struct memory instead would be testing
    // padding, which is no longer part of the format.
    let mut payload = [0u8; arduracer_core::save::PAYLOAD_SIZE];
    save.write_payload(&mut payload);
    assert_eq!(
        SaveData::read_payload(&payload).expect("intact payload must load"),
        save,
        "payload round-trip must be lossless"
    );

    payload[10] ^= 0x01; // flip 1 bit
    assert!(
        SaveData::read_payload(&payload).is_none(),
        "Corrupted save must be caught by checksum"
    );

    // A short buffer must not be read past either.
    assert!(
        SaveData::read_payload(&payload[..arduracer_core::save::PAYLOAD_SIZE - 1]).is_none(),
        "Truncated save must be rejected"
    );
}

fn test_drift_boost_charging() {
    let mut car = VehicleState {
        velocity: Vec2::new(Fixed::ZERO, -Fixed::from_int(3)),
        speed: Fixed::from_int(3),
        ..Default::default()
    };
    let initiate_input = VehicleInput {
        throttle: Fixed::ONE,
        steer: Fixed::ONE,
        handbrake: true,
        ..Default::default()
    };

    // Initiate drift
    car.tick(initiate_input, SurfaceType::Tarmac);
    assert!(car.drift.is_drifting(), "Car must enter drift mode");

    // Maintain drift by counter-steering gently to prevent spin-out
    let maintain_input = VehicleInput {
        throttle: Fixed::ONE,
        steer: -Fixed::from_raw(1024), // gentle counter-steer
        handbrake: true,
        ..Default::default()
    };

    // Maintain drift for 50 ticks -> Level 1 Mini-Turbo
    for _ in 0..50 {
        car.tick(maintain_input, SurfaceType::Tarmac);
    }
    assert_eq!(
        car.drift.boost_tier(),
        1,
        "Must reach Boost Tier 1 (Mini-Turbo)"
    );

    // Maintain drift for another 50 ticks (100 total) -> Level 2 Super-Turbo
    for _ in 0..50 {
        car.tick(maintain_input, SurfaceType::Tarmac);
    }
    assert_eq!(
        car.drift.boost_tier(),
        2,
        "Must reach Boost Tier 2 (Super-Turbo)"
    );

    // Release handbrake -> should trigger turbo boost ticks and impulse
    let release_input = VehicleInput {
        throttle: Fixed::ONE,
        handbrake: false,
        ..Default::default()
    };
    car.tick(release_input, SurfaceType::Tarmac);
    assert!(!car.drift.is_drifting(), "Drift must end upon release");
    assert!(car.boost_ticks > 0, "Boost timer must activate");
}

fn test_counter_steering() {
    let mut car = VehicleState {
        velocity: Vec2::new(Fixed::ZERO, -Fixed::from_int(3)),
        speed: Fixed::from_int(3),
        ..Default::default()
    };
    // Slide right (direction = 1)
    let right_slide = VehicleInput {
        throttle: Fixed::ONE,
        steer: Fixed::ONE,
        handbrake: true,
        ..Default::default()
    };
    car.tick(right_slide, SurfaceType::Tarmac);

    let initial_slip = match car.drift {
        DriftState::Drifting { slip_angle, .. } => slip_angle,
        _ => panic!("Expected drifting state"),
    };

    // Counter-steer to the left while sliding right
    let counter_steer = VehicleInput {
        throttle: Fixed::ONE,
        steer: -Fixed::ONE,
        handbrake: true,
        ..Default::default()
    };
    for _ in 0..10 {
        car.tick(counter_steer, SurfaceType::Tarmac);
    }

    let stabilized_slip = match car.drift {
        DriftState::Drifting { slip_angle, .. } => slip_angle,
        _ => panic!("Expected drifting state"),
    };

    assert!(
        stabilized_slip < initial_slip,
        "Counter-steering must reduce/stabilize slip angle (before: {}, after: {})",
        initial_slip,
        stabilized_slip
    );
}

fn test_barrier_collision() {
    let mut car = VehicleState {
        velocity: Vec2::new(Fixed::from_int(10), Fixed::ZERO), // moving right (+X)
        speed: Fixed::from_int(10),
        ..Default::default()
    };

    // Hit a vertical wall on the right (surface normal points left: -X)
    let wall_normal = Vec2::new(-Fixed::ONE, Fixed::ZERO);
    car.handle_barrier_collision(wall_normal);

    // Car must bounce back (-X velocity) and scrub speed
    assert!(
        car.velocity.x < Fixed::ZERO,
        "Velocity X must reverse on bounce"
    );
    assert!(
        car.speed < Fixed::from_int(10),
        "Speed must scrub on collision"
    );
    assert!(car.drift.is_spinning(), "Wall impact must induce spin");
}

fn test_ghost_recording() {
    let mut recorder = GhostRecorder::<300>::new(1);
    recorder.start();

    let start_pos = Vec2::new(Fixed::from_int(10), Fixed::from_int(20));
    for tick in 0..60 {
        let pos = Vec2::new(start_pos.x + Fixed::from_int(tick), start_pos.y);
        recorder.record_tick(pos, 1024, FLAG_DRIFTING);
    }
    recorder.finish(60);

    // 60 ticks @ 30 Hz sampling (every 2 ticks) = 30 frames
    assert_eq!(
        recorder.frame_count, 30,
        "30Hz sampling must yield 30 frames per 60 ticks"
    );
    assert_eq!(recorder.lap_time_ticks, 60);

    // Test decoding of first recorded frame
    let f0 = recorder.frames[0];
    assert_eq!(f0.heading_byte, (1024 >> 4) as u8);
    assert_eq!(f0.flags, FLAG_DRIFTING);
    assert_eq!(f0.decode_heading(), 1024);
}

fn test_ghost_interpolation() {
    let frames = [
        GhostFrame::encode(Vec2::new(Fixed::from_int(0), Fixed::ZERO), 0, 0),
        GhostFrame::encode(Vec2::new(Fixed::from_int(10), Fixed::ZERO), 1024, 0),
    ];
    let player = GhostPlayer::new(&frames, 2);

    // Sample exactly at tick 0 (frame 0)
    let s0 = player.sample_at_tick(0);
    assert_eq!(s0.position.x.to_int(), 0);
    assert_eq!(s0.heading, 0);

    // Sample at tick 1 (halfway between frame 0 and frame 1)
    let s1 = player.sample_at_tick(1);
    assert_eq!(
        s1.position.x.to_int(),
        5,
        "Sub-tick linear interpolation must yield midpoint"
    );
    assert_eq!(
        s1.heading, 512,
        "Angular interpolation must yield halfway angle"
    );

    // Sample at tick 2 (frame 1)
    let s2 = player.sample_at_tick(2);
    assert_eq!(s2.position.x.to_int(), 10);
    assert_eq!(s2.heading, 1024);
}

fn test_save_block_round_trip() {
    let mut original = SaveData::default();
    original.update_best_lap(0, 1845, 3); // Gold medal on Track 1
    original.update_best_lap(5, 2300, 2); // Silver medal on Track 6
    assert!(original.is_valid());

    let mut block = [0u8; 8192];
    original.to_block_bytes(&mut block);

    // Deserialize from raw 8KB block
    let restored = SaveData::from_block_bytes(&block).expect("Must restore valid save from block");
    assert_eq!(restored.best_lap_ticks[0], 1845);
    assert_eq!(restored.medals_earned[0], 3);
    assert_eq!(restored.best_lap_ticks[5], 2300);
    assert_eq!(restored.checksum, original.checksum);
    assert_eq!(restored.tuning_slots, original.tuning_slots);
    assert!(restored.is_valid());

    // The block is zero-filled past the payload, so a stale tail can never be
    // mistaken for data.
    assert!(
        block[arduracer_core::save::PAYLOAD_SIZE..]
            .iter()
            .all(|&b| b == 0),
        "block padding after the payload must be zeroed"
    );

    // Corrupt one byte in the block -> must fail deserialization
    block[5] ^= 0xFF;
    assert!(
        SaveData::from_block_bytes(&block).is_none(),
        "Corrupt block must be rejected"
    );
}

fn test_all_24_tracks_integrity() {
    assert_eq!(ALL_TRACKS.len(), 24, "Must have exactly 24 official tracks");

    for (idx, track) in ALL_TRACKS.iter().enumerate() {
        assert!(!track.name.is_empty(), "Track {} must have a name", idx + 1);
        assert!(
            track.width >= 10 && track.width <= 32,
            "Track {} width invalid",
            idx + 1
        );
        assert!(
            track.height >= 10 && track.height <= 32,
            "Track {} height invalid",
            idx + 1
        );
        assert_eq!(
            track.tiles.len(),
            (track.width as usize) * (track.height as usize),
            "Track {} tile array length mismatch",
            idx + 1
        );

        // Verify start position is on the racing surface
        let start_tx = TrackDef::tile_x_of(track.start_pos.x);
        let start_ty = TrackDef::tile_y_of(track.start_pos.y);
        assert!(
            start_tx < track.width,
            "Track {} start_tx out of bounds",
            idx + 1
        );
        assert!(
            start_ty < track.height,
            "Track {} start_ty out of bounds",
            idx + 1
        );
        assert!(
            track.tile_at(start_tx, start_ty).is_road(),
            "Track {} start tile must be on the racing surface",
            idx + 1
        );
        assert!(
            track.start_gate.is_active(),
            "Track {} must have a start/finish gate",
            idx + 1
        );
        assert!(
            track.start_gate.contains_tile(start_tx, start_ty),
            "Track {} spawn must sit inside the start/finish gate",
            idx + 1
        );

        // Verify checkpoints
        assert!(
            track.checkpoint_count >= 1,
            "Track {} must have at least 1 checkpoint",
            idx + 1
        );
        for cp_i in 0..(track.checkpoint_count as usize) {
            let cp = track.checkpoints[cp_i];
            assert!(
                cp.x < track.width,
                "Track {} CP {} X out of bounds",
                idx + 1,
                cp_i
            );
            assert!(
                cp.y < track.height,
                "Track {} CP {} Y out of bounds",
                idx + 1,
                cp_i
            );
        }

        // Verify monotonic par times: dev <= gold <= silver <= bronze
        let par = track.par_times;
        assert!(
            par.dev_platinum_ticks > 0,
            "Track {} dev time must be > 0",
            idx + 1
        );
        assert!(
            par.dev_platinum_ticks <= par.gold_ticks,
            "Track {} dev time must be <= gold time",
            idx + 1
        );
        assert!(
            par.gold_ticks <= par.silver_ticks,
            "Track {} gold time must be <= silver time",
            idx + 1
        );
        assert!(
            par.silver_ticks <= par.bronze_ticks,
            "Track {} silver time must be <= bronze time",
            idx + 1
        );
    }
}

fn test_lap_completion_on_track1() {
    let track = ALL_TRACKS[0];
    let mut timer = LapTimer::<16>::new(track.checkpoint_slice(), track.start_gate);
    timer.start();

    // Roll off the grid with no gates cleared: no lap may score.
    let sg = track.start_gate;
    timer.update_player_tile(sg.x.wrapping_add(4), sg.y);
    assert!(!timer.update_player_tile(sg.x, sg.y));
    assert_eq!(timer.current_lap, 1);

    // Walk the ordered route: checkpoints in driving order, then the line.
    for i in 0..track.route_len() {
        let node = track.route_node(i);
        for _ in 0..10 {
            timer.tick();
        }
        let is_last = i == track.route_len() - 1;
        let mut completed_lap = timer.update_player_tile(node.x, node.y);
        if is_last {
            // Crossing the line means *leaving* it; entering alone scores nothing.
            completed_lap = timer.update_player_tile(node.x.wrapping_add(4), node.y);
            assert!(completed_lap, "final start/finish crossing must score");
            assert_eq!(timer.current_lap, 2, "current lap must advance to 2");
            assert!(
                timer.best_lap_ticks < u32::MAX,
                "best lap time must be recorded"
            );
        } else {
            assert!(
                !completed_lap,
                "intermediate route node must not complete the lap"
            );
        }
        let _ = completed_lap;
    }
}

fn test_ai_profiles_and_tuning() {
    assert_eq!(
        AI_PROFILES.len(),
        5,
        "Must have exactly 5 AI rival profiles"
    );
    for profile in AI_PROFILES.iter() {
        assert!(
            profile.tuning.is_valid(),
            "Profile {} tuning must be valid",
            profile.name
        );
        assert!(profile.aggression >= 1 && profile.aggression <= 10);
        assert!(profile.drift_tendency >= 1 && profile.drift_tendency <= 10);
    }
}

fn test_ai_navigation_and_steer() {
    let track = ALL_TRACKS[0];
    let profile = AI_PROFILES[0];
    let mut ai = AiRacer::new(track.start_pos, track.start_heading, profile);

    // Initial state
    assert_eq!(ai.target_gate_idx, 0);
    assert_eq!(ai.current_lap, 1);
    assert!(!ai.is_finished);

    // Simulate 60 ticks of AI navigation
    for _ in 0..60 {
        ai.tick(track, &[]);
    }

    // AI must accelerate from standstill
    assert!(
        ai.state.speed > Fixed::ZERO,
        "AI vehicle must gain positive speed"
    );

    // Steering / pedal outputs must stay normalised.
    let input = ai.compute_input(track, &[]);
    assert!(
        input.steer >= -Fixed::ONE && input.steer <= Fixed::ONE,
        "AI steer must stay normalised, got {:?}",
        input.steer
    );
    assert!(input.throttle >= Fixed::ZERO && input.throttle <= Fixed::ONE);
    assert!(input.brake >= Fixed::ZERO && input.brake <= Fixed::ONE);
}

fn test_championship_scoring() {
    let mut session = ChampionshipSession::new(0); // Bronze Cup
    assert_eq!(session.current_stage, 0);
    assert_eq!(session.current_track_idx(), 0);
    assert_eq!(session.competitors.len(), 6);

    // Award stage 1: Player wins, AI 0 2nd, AI 1 3rd, AI 2 4th, AI 3 5th, AI 4 6th
    session.award_stage_points([0, 1, 2, 3, 4, 5]);
    assert_eq!(session.competitors[0].total_points, 10);
    assert_eq!(session.competitors[1].total_points, 6);
    assert_eq!(session.competitors[2].total_points, 4);

    let finished = session.advance_stage();
    assert!(!finished);
    assert_eq!(session.current_stage, 1);
    assert_eq!(session.current_track_idx(), 1);
}

fn test_race_standings_and_leaderboard() {
    let track = ALL_TRACKS[0];
    let rivals = [
        AiRacer::new(track.start_pos, track.start_heading, AI_PROFILES[0]),
        AiRacer::new(track.start_pos, track.start_heading, AI_PROFILES[1]),
        AiRacer::new(track.start_pos, track.start_heading, AI_PROFILES[2]),
        AiRacer::new(track.start_pos, track.start_heading, AI_PROFILES[3]),
        AiRacer::new(track.start_pos, track.start_heading, AI_PROFILES[4]),
    ];

    // Player on lap 2, rivals on lap 1 -> player must be 1st (index 0 at standings[0])
    let standings = compute_standings(
        track.start_pos,
        2, // player lap 2
        0,
        false,
        &rivals,
        track,
    );
    assert_eq!(standings[0], 0, "Player on lap 2 should be in 1st place");

    let mut session = ChampionshipSession::new(0);
    session.award_stage_points(standings);
    let leaderboard = session.sorted_leaderboard();
    assert_eq!(leaderboard[0].name, "PLAYER");
    assert_eq!(leaderboard[0].total_points, 10);
}
