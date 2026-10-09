//! Input and DualShock Force Feedback Subsystem.
//!
//! Provides unified controller polling, layout profiles, and dual-motor haptics.

pub mod mapping;
pub mod pad;
pub mod rumble;

pub use mapping::InputProfile;
pub use pad::ControllerDriver;
pub use rumble::RumbleDriver;

use arduracer_core::{SurfaceType, VehicleInput, VehicleState};

/// How strongly the low-frequency motor should shudder, derived from how far
/// off the racing surface the car is (GAME.md §7 "rough off-road shudder").
pub const OFFROAD_RUMBLE: u8 = 110;

/// Top-level input and haptic feedback manager.
pub struct InputManager {
    pub controller: ControllerDriver,
    pub rumble: RumbleDriver,
}

impl InputManager {
    pub const fn new(profile: InputProfile) -> Self {
        InputManager {
            controller: ControllerDriver::new(profile),
            rumble: RumbleDriver::new(),
        }
    }

    /// Attempts to activate DualShock analog mode on boot.
    pub fn init(&mut self) {
        let _ = psx_pad::enable_analog_port1();
        psx_io::irq::ack(1 << psx_io::irq::source::CONTROLLER);
    }

    /// Builds vehicle input from an already-polled pad sample, and updates the
    /// rumble motors.
    ///
    /// Takes the sample rather than polling so the UI and the car react to the
    /// same instant. Polling here as well cost a second SIO0 transaction every
    /// racing frame (TASK-1214).
    pub fn update(
        &mut self,
        pad: &psx_pad::PadState,
        player: &VehicleState,
        surface: SurfaceType,
    ) -> VehicleInput {
        let input = self.controller.poll_from(pad);

        self.rumble
            .tick(surface, player.is_drifting, player.engine_rpm);

        input
    }
}
