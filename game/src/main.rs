//! Arduracer PSX - Bare-metal PlayStation 1 Entry Point.
//!
//! Locked 60 FPS overhead arcade racer built with PSoXide.

#![no_std]
#![no_main]

extern crate psx_rt;

use arduracer_core::{Fixed, SurfaceType, VehicleInput, VehicleState};
use psx_gpu::{self as gpu, framebuf::FrameBuffer, Resolution, VideoMode};

pub const SCREEN_WIDTH: u16 = 320;
pub const SCREEN_HEIGHT: u16 = 240;

/// Root game structure stored in `.bss` to protect the 32 KiB stack.
pub struct ArduracerGame {
    pub frame_counter: u32,
    pub player: VehicleState,
    pub fb: FrameBuffer,
}

impl ArduracerGame {
    pub fn new() -> Self {
        gpu::init(VideoMode::Ntsc, Resolution::R320X240);
        let fb = FrameBuffer::new(SCREEN_WIDTH, SCREEN_HEIGHT);
        gpu::set_draw_area(0, 0, SCREEN_WIDTH - 1, SCREEN_HEIGHT - 1);
        gpu::set_draw_offset(0, 0);

        ArduracerGame {
            frame_counter: 0,
            player: VehicleState::default(),
            fb,
        }
    }

    pub fn run(&mut self) -> ! {
        loop {
            psx_rt::interrupts::wait_vblank();
            self.fb.swap();

            self.frame_counter = self.frame_counter.wrapping_add(1);

            // Simulation tick
            let input = VehicleInput {
                throttle: Fixed::ONE,
                brake: Fixed::ZERO,
                steer: Fixed::ZERO,
                handbrake: false,
            };
            self.player.tick(input, SurfaceType::Tarmac);

            // Clear active draw buffer with arcade navy blue
            let target_y = self.fb.buffer_y(self.fb.drawing);
            gpu::fill_rect(0, target_y, SCREEN_WIDTH, SCREEN_HEIGHT, 0, 24, 48);
        }
    }
}

static mut GAME: Option<ArduracerGame> = None;

#[no_mangle]
fn main() -> ! {
    unsafe {
        let slot = core::ptr::addr_of_mut!(GAME);
        *slot = Some(ArduracerGame::new());
        match (*slot).as_mut() {
            Some(game) => game.run(),
            None => loop {},
        }
    }
}
