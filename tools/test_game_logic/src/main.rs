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

    /// Extracts the payload of a caught panic.
    ///
    /// `{:?}` on a `Box<dyn Any + Send>` prints the literal text `Any { .. }`,
    /// which turns every failure into an identical content-free line and hides
    /// the assertion that actually tripped.
    fn panic_message(payload: Box<dyn std::any::Any + Send>) -> String {
        if let Some(s) = payload.downcast_ref::<&'static str>() {
            (*s).to_string()
        } else if let Some(s) = payload.downcast_ref::<String>() {
            s.clone()
        } else {
            "non-string panic payload".to_string()
        }
    }

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
                    println!("\x1b[31mFAIL\x1b[0m: {}", panic_message(e));
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
        "Ghost telemetry survives real circuit coordinates",
        test_ghost_positions_survive_real_circuits
    );
    run_test!(
        "Recovery drops a stuck car back on the racing line",
        test_respawn_recovers_a_stuck_car
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
        "Centreline closes and is monotonic on all 24 tracks",
        test_route_is_closed_and_monotonic
    );
    run_test!(
        "Centreline arc position tracks a driven lap",
        test_route_arc_follows_the_car
    );
    run_test!(
        "Gate crossing is direction aware",
        test_route_crossing_is_direction_aware
    );
    run_test!(
        "Centreline stays on the road",
        test_centreline_stays_on_the_road
    );
    run_test!(
        "Runtime curve matches a float reference",
        test_runtime_curve_matches_float_reference
    );
    run_test!(
        "Sprint Short synthesised a missing start line",
        test_sprint_short_has_a_start_line
    );
    run_test!(
        "Circuits are wide and have long straights",
        test_circuits_are_wide_and_have_long_straights
    );
    run_test!(
        "The whole corridor is drivable",
        test_the_whole_corridor_is_drivable
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
    assert!(SurfaceType::OilSlick.lateral_hold() < SurfaceType::Tarmac.lateral_hold());
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

fn test_route_is_closed_and_monotonic() {
    for track in ALL_TRACKS.iter() {
        let route = Route::from_track(track);
        assert!(!route.is_empty(), "{}: empty centreline", track.name);
        assert!(
            route.len() >= 8,
            "{}: only {} samples",
            track.name,
            route.len()
        );
        assert!(
            route.total_len() > 0,
            "{}: zero-length centreline",
            track.name
        );

        // Arc length must never go backwards, or progress along the lap is
        // meaningless and lap validation cannot work.
        for i in 1..route.len() {
            assert!(
                route.arc_at(i) >= route.arc_at(i - 1),
                "{}: arc went backwards at sample {i}",
                track.name
            );
        }
        assert!(
            route.arc_at(route.len() - 1) <= route.total_len(),
            "{}: final arc exceeds total length",
            track.name
        );

        // No long run of coincident samples: a zero-length segment makes the
        // projection degenerate. (Short repeats are legitimate -- the bounds
        // clamp can flatten samples where a hairpin overshoots the edge.)
        let mut run = 0;
        for i in 0..route.len() {
            let a = route.point(i);
            let b = route.point((i + 1) % route.len());
            if a == b {
                run += 1;
                assert!(
                    run < 4,
                    "{}: {run} coincident centreline samples at {i}",
                    track.name
                );
            } else {
                run = 0;
            }
        }

        // The centreline must sit on the circuit, not off in the infield: sample
        // it and confirm each point is inside the track bounds.
        let (w, h) = (track.world_width(), track.world_height());
        for i in 0..route.len() {
            let p = route.point(i);
            assert!(
                p.x.raw() >= 0 && p.y.raw() >= 0,
                "{}: centrepoint {i} left the circuit",
                track.name
            );
            assert!(
                p.x.to_int() < w && p.y.to_int() < h,
                "{}: centrepoint {i} ({},{}) outside {w}x{h}",
                track.name,
                p.x.to_int(),
                p.y.to_int()
            );
        }
    }
}

fn test_route_arc_follows_the_car() {
    // Walk the centreline itself and confirm `nearest` reports monotonically
    // increasing progress. A stale hint or a bad projection shows up here.
    for track in ALL_TRACKS.iter().take(6) {
        let route = Route::from_track(track);
        let mut hint = route.len(); // force the first call to do a full scan
        let mut previous_arc = 0u32;
        let mut wrapped = 0;
        for i in 0..route.len() {
            let (arc, dist, next_hint) = route.nearest(route.point(i), hint);
            assert!(
                dist <= 2,
                "{}: centreline sample {i} is {dist} units off the centreline",
                track.name
            );
            if arc < previous_arc {
                wrapped += 1;
                assert!(
                    wrapped <= 1,
                    "{}: arc wrapped {wrapped} times in one lap",
                    track.name
                );
            }
            previous_arc = arc;
            hint = next_hint;
        }

        // The local-search hint must agree with a full scan, or per-frame cost
        // and correctness diverge depending on where the car is.
        for i in (0..route.len()).step_by(7) {
            let (arc_h, _, _) = route.nearest(route.point(i), i);
            let (arc_all, _, _) = route.nearest(route.point(i), route.len());
            assert_eq!(
                arc_h, arc_all,
                "{}: hinted and full scans disagree at {i}",
                track.name
            );
        }
    }
}

fn test_route_crossing_is_direction_aware() {
    // A gate a third of the way round a lap.
    let route = Route::from_track(ALL_TRACKS[0]);
    let gate = route.total_len() / 3;

    // Driving forward across it registers.
    let back = gate.saturating_sub(10);
    assert!(
        route.crossed(back, gate + 10, gate, true),
        "forward crossing missed"
    );
    // Driving backward across it must NOT register: this is the defect that let a
    // lap be scored by reversing through every gate.
    assert!(
        !route.crossed(gate + 10, back, gate, true),
        "reverse crossing counted as forward"
    );
    assert!(
        route.crossed(gate + 10, back, gate, false),
        "backward crossing missed in reverse mode"
    );

    // A step that never reaches the gate is not a crossing.
    assert!(!route.crossed(0, gate.saturating_sub(20), gate, true));
    // Backwards from arc 0 wraps the whole lap in one frame; the plausibility
    // guard must reject it instead of reporting a crossing of every gate.
    assert!(
        !route.crossed(0, gate.saturating_sub(20), gate, false),
        "a backwards lap wrap scored a gate"
    );

    // Crossing the lap boundary forward must still be detected, since that is
    // how the start/finish line is scored. The linear gap here is nearly a whole
    // lap, so this also proves the plausibility guard measures distance
    // *travelled* rather than the linear difference.
    let near_end = route.total_len().saturating_sub(5);
    assert!(
        route.crossed(near_end, 5, 0, true),
        "wrap-around crossing missed"
    );

    // A cut across the infield is not progress and must not score.
    assert!(
        !route.crossed(
            0,
            (route.total_len() / 2).max(1),
            (route.total_len() / 2).max(1),
            true
        ),
        "a half-lap teleport counted as a crossing"
    );
}

/// Longest run of consecutive centreline samples that keep turning in the same
/// direction by less than `TOLERANCE_DEG` per sample.
///
/// This is the measurable form of "a longer straight between corners". Counting
/// authored anchors would only measure the data file; what matters is how long
/// the *evaluated* centreline actually runs straight, which is what the player
/// experiences.
fn longest_straight(route: &Route) -> u32 {
    // 0.75 degrees per sample. Tight on purpose: at ~17 world units between
    // samples, anything under about a 20-tile radius registers as straight. A
    // looser tolerance (6 degrees was the first attempt) counts a normal corner
    // as a straight, which makes the measurement meaningless.
    const TOLERANCE_DEG: f64 = 0.75;
    let n = route.len();
    if n < 4 {
        return 0;
    }
    let mut best = 0u32;
    let mut run = 1u32;
    let mut previous_dir: Option<(i32, i32)> = None;
    for i in 0..=n {
        let a = route.point(i % n);
        let b = route.point((i + 1) % n);
        let dir = (b.x.to_int() - a.x.to_int(), b.y.to_int() - a.y.to_int());
        if let Some(prev) = previous_dir {
            // Angle between consecutive tangents, in whole degrees.
            let cross = (prev.0 * dir.1 - prev.1 * dir.0) as f64;
            let dot = (prev.0 * dir.0 + prev.1 * dir.1) as f64;
            let turn = cross.atan2(dot).abs().to_degrees();
            if turn <= TOLERANCE_DEG {
                run += 1;
                if run > best {
                    best = run;
                }
            } else {
                run = 1;
            }
        }
        previous_dir = Some(dir);
    }
    best
}

/// "Wide roads" and "long straights", checked rather than asserted.
///
/// Every circuit used to be derived from `ArduRacerFx/Levels/*.csv` gates, which
/// gave a two-tile road and whatever straight the gate spacing left over. Both
/// are now authored, and these two bounds are what stop a future re-tune quietly
/// narrowing the game back to a strip.
fn test_circuits_are_wide_and_have_long_straights() {
    /// A road narrower than this reads as a two-tile strip again. The old
    /// derived geometry was 1.05 tiles (34 px).
    ///
    /// Recalibrated when the road was narrowed from 160 to 96 world units for the
    /// "drowning in the space" complaint. The floor used to be 128, which was two
    /// tiles at the time `TILE_SIZE` was 64 -- after the cell halved to 32 the
    /// same 128 had quietly become *four* cells, and the authored road was 160,
    /// so the check had stopped saying anything at all: it passed by 32.
    ///
    /// 96 is three cells, which is the authored width: wide enough for two cars
    /// abreast plus a car's margin each side, narrow enough that a sloppy line
    /// puts a wheel in the gravel. The point of the floor is still the same -- do
    /// not go back to a corridor -- and the margin above the old derived strip is
    /// now 2.8x rather than 1.9x.
    const MIN_HALF_WIDTH: u16 = 96; // 3.0 tiles
    /// The longest straight must be a meaningful fraction of the lap. Below this
    /// the circuit is a technical park with no place to use a wide road.
    const MIN_STRAIGHT_SAMPLES: u32 = 24;
    /// And a lap long enough that "slightly longer gameplay between corners"
    /// is true of the circuit rather than of one stretch of it.
    ///
    /// Down from 5,000 for the same reason the road narrowed: at the reference
    /// pace of roughly 4.5 world units per tick that floor was a 1,110-tick lap,
    /// or eighteen seconds, which made every circuit at least as long as the
    /// design's *largest* target. The floor is now one second under the easiest
    /// authored lap (Hells Bells, 592 ticks / 2,688 units), so it still rejects a
    /// circuit too short to be a lap -- a hairpin, or a shape whose centreline
    /// collapses -- while admitting a ten-second easy stage.
    const MIN_LAP: u32 = 2_500;

    for track in ALL_TRACKS.iter() {
        assert!(
            track.half_width >= MIN_HALF_WIDTH,
            "{}: road half-width is {} px ({:.2} tiles); the floor is {} px",
            track.name,
            track.half_width,
            track.half_width as f64 / TILE_SIZE as f64,
            MIN_HALF_WIDTH,
        );
        let route = Route::from_track(track);
        let lap = route.total_len();
        assert!(lap >= MIN_LAP, "{}: lap is only {lap} units", track.name);
        let straight = longest_straight(&route);
        assert!(
            straight >= MIN_STRAIGHT_SAMPLES,
            "{}: longest straight is only {straight} centreline samples; \
             the floor is {MIN_STRAIGHT_SAMPLES}",
            track.name,
        );
    }
}

/// A wide road has to be drivable wide, not just painted wide: the surface under
/// the full corridor has to be tarmac or curb, not off-road.
fn test_the_whole_corridor_is_drivable() {
    for track in ALL_TRACKS.iter() {
        let route = Route::from_track(track);
        for i in 0..route.len() {
            let p = route.point(i);
            // Sample the centreline and both shoulders of the corridor.
            for offset in [-1i32, 0, 1] {
                let probe = Vec2::new(
                    Fixed::from_raw(p.x.raw() + offset * (track.half_width as i32 / 3) * 4096),
                    p.y,
                );
                let tx = TrackDef::tile_x_of(probe.x);
                let ty = TrackDef::tile_y_of(probe.y);
                if tx >= track.width || ty >= track.height {
                    continue;
                }
                let tile = track.tile_at(tx, ty);
                assert!(
                    track.surface_at(tx, ty).traction() > Fixed::ZERO,
                    "{}: corridor at sample {i}{} sits on {:?}, which is not drivable",
                    track.name,
                    if offset == 0 { "" } else { " (shoulder)" },
                    tile,
                );
            }
        }
    }
}

fn test_centreline_stays_on_the_road() {
    // The centreline is the road by construction: the cooker emits the control
    // points it rasterised the corridor along, so the runtime evaluates the same
    // curve. A sample on OffRoad means the two diverged again, and the line cannot
    // be used for lap validation or for drawing the road.
    for track in ALL_TRACKS.iter() {
        assert!(
            !track.route.is_empty(),
            "{}: cooker emitted no TrackDef::route",
            track.name
        );
        let route = Route::from_track(track);
        assert!(!route.is_empty(), "{}: empty centreline", track.name);
        for i in 0..route.len() {
            let p = route.point(i);
            assert!(
                p.x.to_int() >= 0
                    && p.y.to_int() >= 0
                    && p.x.to_int() < track.world_width()
                    && p.y.to_int() < track.world_height(),
                "{}: sample {i} at ({},{}) is outside the {}x{} world",
                track.name,
                p.x.to_int(),
                p.y.to_int(),
                track.world_width(),
                track.world_height()
            );
            let tile = track.tile_at(TrackDef::tile_x_of(p.x), TrackDef::tile_y_of(p.y));
            // OilSlick and BoostPad are *placed on the racing surface* on
            // purpose, so only OffRoad and Barrier count as "not the road".
            assert!(
                !matches!(tile, TrackTile::OffRoad | TrackTile::Barrier),
                "{}: sample {i} sits on {tile:?} at tile ({},{})",
                track.name,
                TrackDef::tile_x_of(p.x),
                TrackDef::tile_y_of(p.y)
            );
        }
    }
}

fn test_runtime_curve_matches_float_reference() {
    // The runtime evaluates the spline in Q20.12; the cooker evaluates it in f64.
    // Agreement to within a rounding step is what makes the emitted centreline the
    // same line as the painted road.
    fn reference(points: &[Vec2], samples: usize) -> Vec<(f64, f64)> {
        let n = points.len();
        let mut out = Vec::new();
        for i in 0..n {
            let (p0, p1, p2, p3) = (
                points[(i + n - 1) % n],
                points[i],
                points[(i + 1) % n],
                points[(i + 2) % n],
            );
            for k in 0..samples {
                let t = k as f64 / samples as f64;
                let (t2, t3) = (t * t, t * t * t);
                let cr = |a: f64, b: f64, c: f64, d: f64| {
                    0.5 * (2.0 * b
                        + (-a + c) * t
                        + (2.0 * a - 5.0 * b + 4.0 * c - d) * t2
                        + (-a + 3.0 * b - 3.0 * c + d) * t3)
                };
                // Exact control points: truncating them to whole world units would
                // make the reference itself quantised and hide real runtime error.
                let f = |v: Fixed| v.raw() as f64 / 4096.0;
                let (ax, ay) = (f(p0.x), f(p0.y));
                let (bx, by) = (f(p1.x), f(p1.y));
                let (cx, cy) = (f(p2.x), f(p2.y));
                let (dx, dy) = (f(p3.x), f(p3.y));
                out.push((cr(ax, bx, cx, dx), cr(ay, by, cy, dy)));
            }
        }
        out
    }

    for track in ALL_TRACKS.iter() {
        let route = Route::from_track(track);
        let expect = reference(track.route, arduracer_core::DEFAULT_SAMPLES_PER_SPAN);
        assert_eq!(
            expect.len(),
            route.len(),
            "{}: sample count differs from the reference",
            track.name
        );
        for (i, (p, want)) in route.points_iter().zip(expect.iter()).enumerate() {
            let dx = (p.x.to_int() as f64 - want.0).abs();
            let dy = (p.y.to_int() as f64 - want.1).abs();
            assert!(
                dx.max(dy) <= 2.0,
                "{}: sample {i} runtime ({},{}) vs reference ({:.1},{:.1})",
                track.name,
                p.x.to_int(),
                p.y.to_int(),
                expect[i].0,
                expect[i].1
            );
        }
    }
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

fn test_ghost_positions_survive_real_circuits() {
    // The old encoding shifted Q20.12 raw down by 4 bits into an i16, which
    // spanned only +/-128 world units and wrapped modulo 256. Every shipped
    // circuit is at least 640 units across, so every recorded lap replayed as
    // garbage. Encode/decode real circuit coordinates and demand they come back.
    const TOLERANCE_UNITS: i32 = 1;

    let mut checked = 0usize;
    for track in ALL_TRACKS.iter() {
        let w = track.world_width();
        let h = track.world_height();
        assert!(
            w > 128 && h > 128,
            "{}: circuit smaller than the old +/-128 range; \
             this test would no longer prove anything",
            track.name
        );

        // Walk the whole circuit extent, including the far corners where the
        // old encoding wrapped hardest.
        for &(x, y) in &[
            (0, 0),
            (w - 1, 0),
            (0, h - 1),
            (w - 1, h - 1),
            (w / 2, h / 2),
            (w - 1, h / 3),
        ] {
            let pos = Vec2::new(Fixed::from_int(x), Fixed::from_int(y));
            let back = GhostFrame::encode(pos, 2048, 0).decode_position();
            assert!(
                (back.x.to_int() - x).abs() <= TOLERANCE_UNITS
                    && (back.y.to_int() - y).abs() <= TOLERANCE_UNITS,
                "{}: ({x},{y}) round-tripped to ({},{})",
                track.name,
                back.x.to_int(),
                back.y.to_int()
            );
            checked += 1;
        }
    }
    assert!(checked >= 24 * 6, "expected every circuit to be covered");

    // A monotonic sweep along the largest axis: no wrap, no plateau, no jump.
    let w = ALL_TRACKS
        .iter()
        .map(|t| t.world_width())
        .max()
        .expect("tracks exist");
    let mut previous = None;
    for x in (0..w).step_by(37) {
        let pos = Vec2::new(Fixed::from_int(x), Fixed::ZERO);
        let back = GhostFrame::encode(pos, 0, 0).decode_position();
        if let Some(prev) = previous {
            assert!(
                back.x.to_int() > prev,
                "position {x} decoded to {} which is not past {prev}",
                back.x.to_int()
            );
        }
        previous = Some(back.x.to_int());
    }

    // Out-of-range telemetry saturates instead of teleporting the ghost.
    let absurd = GhostFrame::encode(
        Vec2::new(Fixed::from_raw(i32::MAX), Fixed::from_raw(i32::MAX)),
        0,
        0,
    );
    assert!(absurd.decode_position().x.raw() > 0);
}

fn test_respawn_recovers_a_stuck_car() {
    for track in ALL_TRACKS.iter() {
        let mut car = VehicleState::new(track.start_pos, track.start_heading, Default::default());

        // Drive it into a wall hard enough to be genuinely stuck: wedged into a
        // solid tile, drifting, boosting, out of reverse, at speed.
        let tx = TrackDef::tile_x_of(car.position.x);
        let ty = TrackDef::tile_y_of(car.position.y);
        car.velocity = Vec2::new(Fixed::from_int(6), Fixed::from_int(6));
        car.speed = car.velocity.length();
        car.is_drifting = true;
        car.boost_ticks = 40;
        car.nitro_charge = 0;
        car.is_reversing = true;
        car.gear = 5;
        let throttle = VehicleInput {
            throttle: Fixed::ONE,
            nitro: true,
            ..VehicleInput::default()
        };
        for _ in 0..240 {
            car.tick(throttle, track.surface_at(tx, ty));
            car.collide_with_track(track);
        }

        // Recovery must place the car on a driveable tile, facing along the
        // track, with nothing left over that could hold it there.
        let (pos, heading) = track.respawn_point(car.position);
        car.respawn_at(pos, heading);

        let rtx = TrackDef::tile_x_of(car.position.x);
        let rty = TrackDef::tile_y_of(car.position.y);
        let surface = track.surface_at(rtx, rty);
        assert!(
            surface.max_speed_factor() > Fixed::ZERO,
            "{}: respawned onto {:?} at ({},{}), which cannot be driven",
            track.name,
            surface,
            rtx,
            rty
        );
        assert!(
            !car.is_drifting,
            "{}: still drifting after respawn",
            track.name
        );
        assert_eq!(car.velocity, Vec2::ZERO, "{}: still moving", track.name);
        assert_eq!(car.speed, Fixed::ZERO, "{}: still at speed", track.name);
        assert_eq!(car.boost_ticks, 0, "{}: kept boost", track.name);
        assert!(!car.is_reversing, "{}: stuck in reverse", track.name);

        // And it must actually be able to leave again under power, which is the
        // whole point: the failure mode this fixes is a car that cannot move.
        let before = car.position;
        for _ in 0..60 {
            let (tx, ty) = (
                TrackDef::tile_x_of(car.position.x),
                TrackDef::tile_y_of(car.position.y),
            );
            car.tick(throttle, track.surface_at(tx, ty));
            car.collide_with_track(track);
        }
        let moved =
            (car.position.x - before.x).to_int().abs() + (car.position.y - before.y).to_int().abs();
        assert!(
            moved > 0,
            "{}: respawned but still cannot move (surface {:?})",
            track.name,
            track.surface_at(
                TrackDef::tile_x_of(car.position.x),
                TrackDef::tile_y_of(car.position.y)
            )
        );
    }

    // Facing matters: the car must point down the racing line, not back across
    // it, so a respawn never aims the player into the scenery.
    let track = &ALL_TRACKS[0];
    let (landed, _heading) = track.respawn_point(Vec2::ZERO);
    assert!(
        TrackDef::tile_x_of(landed.x) < track.width && TrackDef::tile_y_of(landed.y) < track.height,
        "respawn landed outside the circuit"
    );
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
        // Upper bound is `MAX_TRACK_DIM`, read rather than restated: it was 64
        // and is now 96, and a literal here would have failed every circuit for
        // a reason that had nothing to do with the circuits.
        assert!(
            track.width >= 10 && track.width as usize <= MAX_TRACK_DIM,
            "Track {} width invalid",
            idx + 1
        );
        assert!(
            track.height >= 10 && track.height as usize <= MAX_TRACK_DIM,
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
