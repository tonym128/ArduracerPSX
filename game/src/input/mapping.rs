//! Button layout profiles and action mappings.
//!
//! Provides three controller layouts tailored for retro d-pad players,
//! modern trigger racers, and dual-analog enthusiasts.

#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum InputProfile {
    /// Layout A (Classic Arcade):
    /// - Cross: Throttle
    /// - Square: Brake / Reverse
    /// - Circle: Handbrake (Drift Initiate)
    /// - Triangle: Boost / Nitro
    /// - D-Pad / Left Stick: Steering
    #[default]
    ClassicArcade,

    /// Layout B (Modern GT / Triggers):
    /// - R2: Throttle
    /// - L2: Brake / Reverse
    /// - Square: Handbrake (Drift Initiate)
    /// - Cross: Boost / Nitro
    /// - D-Pad / Left Stick: Steering
    ModernTriggers,

    /// Layout C (Dual Analog Pro):
    /// - Right Stick Up: Throttle (analog modulation)
    /// - Right Stick Down: Brake (analog modulation)
    /// - Left Stick Horizontal: Steering (analog)
    /// - R1: Handbrake (Drift Initiate)
    /// - L1: Boost / Nitro
    DualAnalog,
}
