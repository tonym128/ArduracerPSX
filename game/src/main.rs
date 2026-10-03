//! Arduracer PSX - Bare-metal PlayStation 1 Entry Point.
//!
//! Locked 60 FPS overhead arcade racer built with PSoXide.

#![no_std]
#![no_main]

extern crate psx_rt;

pub mod audio;
pub mod gpu;
pub mod input;

use arduracer_core::{LapTimer, TrackDef, VehicleState, ALL_TRACKS};
use audio::AudioSystem;
use gpu::{render_car, render_hud, render_track, Camera, ParticleSystem, SkidmarkBuffer};
use input::{InputManager, InputProfile};
use psx_gpu::{self as psx_gpu_mod, framebuf::FrameBuffer, Resolution, VideoMode};

pub const SCREEN_WIDTH: u16 = 320;
pub const SCREEN_HEIGHT: u16 = 240;

/// Root game structure stored in `.bss` to protect the 32 KiB stack.
pub struct ArduracerGame {
    pub frame_counter: u32,
    pub player: VehicleState,
    pub camera: Camera,
    pub particles: ParticleSystem,
    pub skidmarks: SkidmarkBuffer,
    pub timer: LapTimer<16>,
    pub current_track: &'static TrackDef,
    pub fb: FrameBuffer,
    pub audio: AudioSystem,
    pub input_mgr: InputManager,
}

impl ArduracerGame {
    pub fn new() -> Self {
        psx_gpu_mod::init(VideoMode::Ntsc, Resolution::R320X240);
        let fb = FrameBuffer::new(SCREEN_WIDTH, SCREEN_HEIGHT);
        psx_gpu_mod::set_draw_area(0, 0, SCREEN_WIDTH - 1, SCREEN_HEIGHT - 1);
        psx_gpu_mod::set_draw_offset(0, 0);

        let track = ALL_TRACKS[0];
        let timer = LapTimer::new(&track.checkpoints[..track.checkpoint_count as usize]);
        let player = VehicleState::new(track.start_pos, track.start_heading, Default::default());
        let camera = Camera::new(track.start_pos);

        let audio = AudioSystem::new();
        let mut input_mgr = InputManager::new(InputProfile::ClassicArcade);
        input_mgr.init();

        ArduracerGame {
            frame_counter: 0,
            player,
            camera,
            particles: ParticleSystem::new(),
            skidmarks: SkidmarkBuffer::new(),
            timer,
            current_track: track,
            fb,
            audio,
            input_mgr,
        }
    }

    pub fn run(&mut self) -> ! {
        self.timer.start();

        loop {
            // 1. Synchronize to 60Hz NTSC VBlank
            psx_rt::interrupts::wait_vblank();
            self.fb.swap();

            let draw_y = self.fb.buffer_y(self.fb.drawing) as i16;
            self.frame_counter = self.frame_counter.wrapping_add(1);

            // 2. Query surface beneath vehicle
            let tx = (self.player.position.x.to_int() / 64) as u8;
            let ty = (self.player.position.y.to_int() / 64) as u8;
            let surface = self.current_track.surface_at(tx, ty);

            // 3. Controller input poll and haptics update
            let input = self.input_mgr.update(&self.player, surface);
            self.player.tick(input, surface);
            self.timer.tick();
            self.timer.update_player_tile(tx, ty);

            // 4. Audio engine tick (engine RPM synth, tire screech, SFX)
            self.audio
                .tick(&self.player, input.throttle, self.timer.next_checkpoint_idx);

            // 5. Emit smoke particles and skidmarks during hard turns or drifts
            if self.player.is_drifting {
                self.particles.emit_smoke(self.player.position);
                self.skidmarks
                    .emit(self.player.position, self.player.visual_angle);
            }
            self.particles.tick();
            self.skidmarks.tick();

            // 6. Camera update
            self.camera.update(
                self.player.position,
                self.player.velocity,
                self.player.speed,
            );

            // 6. Render Pass:
            // a. Clear background
            psx_gpu_mod::fill_rect(0, draw_y as u16, SCREEN_WIDTH, SCREEN_HEIGHT, 18, 20, 26);
            // b. Track tilemap
            render_track(self.current_track, &self.camera, draw_y);
            // c. Skidmarks on track
            self.skidmarks.render(&self.camera, draw_y);
            // d. Particle effects
            self.particles.render(&self.camera, draw_y);
            // e. Player race car (Crimson Red: 220, 25, 45)
            render_car(&self.player, &self.camera, draw_y, false, (220, 25, 45));
            // f. In-Game HUD overlay
            render_hud(&self.player, &self.timer, self.current_track, draw_y);
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
