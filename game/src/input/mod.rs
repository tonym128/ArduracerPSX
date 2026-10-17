//! Input and DualShock Force Feedback Subsystem.
//!
//! Provides unified controller polling, layout profiles, and dual-motor haptics.

pub mod mapping;
pub mod pad;
pub mod rumble;

pub use mapping::InputProfile;
pub use pad::{ControllerDriver, MenuNavSynthesizer};
pub use rumble::RumbleDriver;

use arduracer_core::{SurfaceType, VehicleInput, VehicleState};

/// How strongly the low-frequency motor should shudder, derived from how far
/// off the racing surface the car is (GAME.md §7 "rough off-road shudder").
pub const OFFROAD_RUMBLE: u8 = 110;

/// Top-level input and haptic feedback manager.
pub struct InputManager {
    pub controller: ControllerDriver,
    pub rumble: RumbleDriver,
    pub nav: MenuNavSynthesizer,
    pub prev_connected: bool,
    pub probe_counter: u8,
    pub is_port2: bool,
}

impl InputManager {
    pub const fn new(profile: InputProfile) -> Self {
        InputManager {
            controller: ControllerDriver::new(profile),
            rumble: RumbleDriver::new(),
            nav: MenuNavSynthesizer::new(),
            prev_connected: false,
            probe_counter: 0,
            is_port2: false,
        }
    }

    pub const fn new_port2(profile: InputProfile) -> Self {
        InputManager {
            controller: ControllerDriver::new(profile),
            rumble: RumbleDriver::new(),
            nav: MenuNavSynthesizer::new(),
            prev_connected: false,
            probe_counter: 0,
            is_port2: true,
        }
    }

    /// Attempts to activate DualShock analog mode on boot.
    pub fn init(&mut self) {
        if !self.is_port2 {
            let _ = psx_pad::enable_analog_port1();
        } else {
            let _ = psx_pad::enable_analog_port2();
        }
        psx_io::irq::ack(1 << psx_io::irq::source::CONTROLLER);
    }

    /// Checks controller hot-plug state and re-arms DualShock analog mode if needed (`NO1-043`).
    pub fn check_hotplug(&mut self, pad: &psx_pad::PadState) {
        let connected = pad.is_connected();
        let just_connected = connected && !self.prev_connected;
        self.prev_connected = connected;

        self.probe_counter = (self.probe_counter + 1) % 60;
        if just_connected
            || (connected
                && !pad.is_analog()
                && !pad.is_negcon()
                && !pad.is_jogcon()
                && self.probe_counter == 0)
        {
            if !self.is_port2 {
                let _ = psx_pad::enable_analog_port1();
            } else {
                let _ = psx_pad::enable_analog_port2();
            }
            psx_io::irq::ack(1 << psx_io::irq::source::CONTROLLER);
        }
    }

    /// Synthesizes D-pad menu navigation buttons from the left analog stick with auto-repeat.
    pub fn synthesize_nav(&mut self, pad: &psx_pad::PadState) -> psx_pad::PadState {
        self.nav.update(pad)
    }

    /// Builds vehicle input from an already-polled pad sample, and updates the
    /// rumble motors.
    pub fn update(
        &mut self,
        pad: &psx_pad::PadState,
        player: &VehicleState,
        surface: SurfaceType,
    ) -> VehicleInput {
        let input = self.controller.poll_from(pad);

        self.rumble.tick(
            surface,
            player.speed,
            player.is_drifting,
            player.tire_scrub,
            player.engine_rpm,
        );

        let (small, large) = self.rumble.actuator_bytes();
        if !self.is_port2 {
            psx_pad::set_rumble_port1(small, large);
        } else {
            psx_pad::set_rumble_port2(small, large);
        }

        input
    }

    /// Cuts all DualShock vibration motors immediately.
    pub fn silence(&mut self) {
        self.rumble.small_motor = false;
        self.rumble.large_motor = 0;
        self.rumble.impact_ticks = 0;
        self.rumble.boost_ticks = 0;
        if !self.is_port2 {
            psx_pad::set_rumble_port1(false, 0);
        } else {
            psx_pad::set_rumble_port2(false, 0);
        }
    }
}
