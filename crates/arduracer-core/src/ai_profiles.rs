//! AI Opponent Profiles and Rival Personalities.
//!
//! Defines 5 distinct AI personalities with custom liveries, tuning setups,
//! and driving styles (Speeder, Tactician, Drifter, Brawler, Rookie).

use crate::math::{self, Fixed};
use crate::tuning::CarTuning;

/// Lowest documented aggression / drift-tendency value.
pub const MIN_PERSONALITY: u8 = 1;
/// Highest documented aggression / drift-tendency value.
pub const MAX_PERSONALITY: u8 = 10;
/// Aggression at which the braking bias is neutral (1.0x).
pub const NEUTRAL_AGGRESSION: u8 = 5;
/// Q20.12 step in the braking bias per point of aggression.
const AGGRESSION_STEP: i32 = 130;
/// Drift tendency at which a rival is an apex hunter and may hang the tail out.
pub const BLAZE_DRIFT_TENDENCY: u8 = 7;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct AiProfile {
    pub name: &'static str,
    pub tuning: CarTuning,
    pub color: (u8, u8, u8),
    /// Willingness to carry speed through corners: 1..=10 (see [`MIN_PERSONALITY`]
    /// and [`MAX_PERSONALITY`]). Always read through [`Aggression::brake_bias`],
    /// which clamps, because this is a bare `pub u8` on a `pub` struct.
    pub aggression: u8,
    /// Willingness to drift: 1..=10. Always read through
    /// [`DriftTendency::clamped`].
    pub drift_tendency: u8,
}

impl AiProfile {
    /// Whether every field of the profile is inside its documented range.
    ///
    /// Not enforced by the constructor -- `AI_PROFILES` is a `const`, and the
    /// five shipped personalities are hand-authored -- but it is what a save file
    /// or a future track editor must check before handing a profile to the AI.
    pub fn is_valid(&self) -> bool {
        self.aggression >= MIN_PERSONALITY
            && self.aggression <= MAX_PERSONALITY
            && self.drift_tendency >= MIN_PERSONALITY
            && self.drift_tendency <= MAX_PERSONALITY
            && self.tuning.is_valid()
    }
}

/// Personality aggression, clamped to its documented range.
pub struct Aggression;

impl Aggression {
    /// Lowest documented aggression.
    pub const MIN: u8 = MIN_PERSONALITY;
    /// Highest documented aggression.
    pub const MAX: u8 = MAX_PERSONALITY;
    /// Braking bias for an aggression value: `NEUTRAL_AGGRESSION` is neutral
    /// (1.0x) and each step either side is `AGGRESSION_STEP`/4096.
    ///
    /// The clamp is the point: the AI used to compute
    /// `4096 + (aggression - 5) * 130` straight off an unvalidated `pub u8`, so
    /// `aggression = 255` handed the rival an 8.93x top-speed multiplier and
    /// `aggression = 0` a 0.84x one, neither of which is a personality.
    pub fn brake_bias(aggression: u8) -> Fixed {
        let level = aggression.clamp(Self::MIN, Self::MAX) as i32;
        Fixed::from_raw(math::FP_ONE + (level - NEUTRAL_AGGRESSION as i32) * AGGRESSION_STEP)
    }
}

/// Personality drift tendency, clamped to its documented range.
pub struct DriftTendency;

impl DriftTendency {
    /// The value at which a rival is an apex hunter.
    pub const BLAZE: u8 = BLAZE_DRIFT_TENDENCY;
    /// Lowest documented drift tendency.
    pub const MIN: u8 = MIN_PERSONALITY;
    /// Highest documented drift tendency.
    pub const MAX: u8 = MAX_PERSONALITY;
    /// Drift tendency clamped to `MIN..=MAX`.
    pub fn clamped(drift_tendency: u8) -> u8 {
        drift_tendency.clamp(Self::MIN, Self::MAX)
    }
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
