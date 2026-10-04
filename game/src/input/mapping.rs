//! Button layout profiles and action mappings.
//!
//! Provides three controller layouts tailored for retro d-pad players,
//! modern trigger racers, and dual-analog enthusiasts.
//!
//! Two bindings are common to all three: **R1 is nitro** and **L1 resets the car
//! to the track**. Neither shoulder button carries a second job in any layout,
//! which is what keeps them unambiguous.

#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum InputProfile {
    /// Layout A (Classic Arcade):
    /// - Cross: Throttle
    /// - Square: Brake / Reverse
    /// - Circle: Handbrake (Drift Initiate)
    /// - R1: Boost / Nitro
    /// - L1: Reset Car to Track
    /// - D-Pad / Left Stick: Steering
    #[default]
    ClassicArcade,

    /// Layout B (Modern GT / Triggers):
    /// - R2: Throttle
    /// - L2: Brake / Reverse
    /// - Square: Handbrake (Drift Initiate)
    /// - R1: Boost / Nitro
    /// - L1: Reset Car to Track
    /// - D-Pad / Left Stick: Steering
    ModernTriggers,

    /// Layout C (Dual Analog Pro):
    /// - Right Stick Up: Throttle (analog modulation)
    /// - Right Stick Down: Brake (analog modulation)
    /// - Left Stick Horizontal: Steering (analog)
    /// - L2: Handbrake (Drift Initiate)
    /// - R1: Boost / Nitro
    /// - L1: Reset Car to Track
    DualAnalog,
}
