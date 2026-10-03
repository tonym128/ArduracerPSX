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
        "Checkpoint anti-cheat & sequential gate clearing",
        test_checkpoint_anti_cheat
    );
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
    };
    for _ in 0..60 {
        car.tick(brake_input, SurfaceType::Tarmac);
    }
    assert_eq!(car.speed, Fixed::ZERO, "Car should come to complete stop");
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
        CheckpointGate {
            x: 0,
            y: 0,
            width: 2,
            height: 2,
        }, // Start/finish
    ];

    let mut timer = LapTimer::<4>::new(&gates);
    timer.start();

    // Trying to hit checkpoint 2 before checkpoint 0 must NOT register
    assert!(!timer.update_player_tile(10, 10));
    assert_eq!(timer.next_checkpoint_idx, 0);

    // Hit checkpoint 0
    assert!(!timer.update_player_tile(5, 5));
    assert_eq!(timer.next_checkpoint_idx, 1);

    // Hit checkpoint 1
    assert!(!timer.update_player_tile(10, 5));
    assert_eq!(timer.next_checkpoint_idx, 2);

    // Hit checkpoint 2
    assert!(!timer.update_player_tile(10, 10));
    assert_eq!(timer.next_checkpoint_idx, 3);

    // Hit checkpoint 3 (Start/Finish line) -> Lap 1 complete!
    assert!(timer.update_player_tile(0, 0));
    assert_eq!(timer.current_lap, 2);
    assert_eq!(timer.next_checkpoint_idx, 0);
}

fn test_lap_timer_precision() {
    let gates = [CheckpointGate {
        x: 0,
        y: 0,
        width: 1,
        height: 1,
    }];
    let mut timer = LapTimer::<1>::new(&gates);
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
    let mut save = SaveData::default();
    assert!(save.is_valid());

    // Corrupt a single byte inside save data
    let raw_bytes = unsafe {
        core::slice::from_raw_parts_mut(
            &mut save as *mut SaveData as *mut u8,
            core::mem::size_of::<SaveData>(),
        )
    };
    raw_bytes[10] ^= 0x01; // flip 1 bit

    assert!(
        !save.is_valid(),
        "Corrupted save must be caught by checksum"
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
    assert!(restored.is_valid());

    // Corrupt one byte in the block -> must fail deserialization
    block[5] ^= 0xFF;
    assert!(
        SaveData::from_block_bytes(&block).is_none(),
        "Corrupt block must be rejected"
    );
}
