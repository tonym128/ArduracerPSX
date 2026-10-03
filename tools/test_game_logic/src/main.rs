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
    let mut car = VehicleState::default();
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
