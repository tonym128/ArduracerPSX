//! AI Opponent Profiles and Rival Personalities.
//!
//! Defines 5 distinct AI personalities with custom liveries, tuning setups,
//! and driving styles (Speeder, Tactician, Drifter, Brawler, Rookie).

use crate::tuning::CarTuning;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct AiProfile {
    pub name: &'static str,
    pub tuning: CarTuning,
    pub color: (u8, u8, u8),
    pub aggression: u8,     // 1..10
    pub drift_tendency: u8, // 1..10
}

pub const AI_PROFILES: [AiProfile; 5] = [
    // 1. VIPER (The Speeder) - Max top speed, high aggression
    AiProfile {
        name: "VIPER",
        tuning: CarTuning {
            top_speed: 7,
            acceleration: 4,
            handling: 3,
            drift_stability: 2,
            gearing: 4,
        },
        color: (40, 120, 240), // Cobalt Blue
        aggression: 9,
        drift_tendency: 4,
    },
    // 2. APEX (The Tactician) - Max handling and grip, clean racing lines
    AiProfile {
        name: "APEX",
        tuning: CarTuning {
            top_speed: 4,
            acceleration: 4,
            handling: 6,
            drift_stability: 3,
            gearing: 3,
        },
        color: (240, 200, 30), // Gold Yellow
        aggression: 5,
        drift_tendency: 2,
    },
    // 3. BLAZE (The Drifter) - High drift stability and acceleration
    AiProfile {
        name: "BLAZE",
        tuning: CarTuning {
            top_speed: 3,
            acceleration: 5,
            handling: 2,
            drift_stability: 6,
            gearing: 4,
        },
        color: (245, 100, 25), // Neon Orange
        aggression: 7,
        drift_tendency: 9,
    },
    // 4. TITAN (The Brawler) - Heavy acceleration and inside corner diving
    AiProfile {
        name: "TITAN",
        tuning: CarTuning {
            top_speed: 4,
            acceleration: 6,
            handling: 3,
            drift_stability: 4,
            gearing: 3,
        },
        color: (160, 40, 200), // Purple Shadow
        aggression: 8,
        drift_tendency: 5,
    },
    // 5. ROOKIE (The Rookie) - Balanced, careful braking
    AiProfile {
        name: "ROOKIE",
        tuning: CarTuning {
            top_speed: 4,
            acceleration: 4,
            handling: 4,
            drift_stability: 4,
            gearing: 4,
        },
        color: (40, 210, 100), // Emerald Green
        aggression: 3,
        drift_tendency: 3,
    },
];
