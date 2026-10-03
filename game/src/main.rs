//! Arduracer PSX - Bare-metal PlayStation 1 Entry Point.
//!
//! Locked 60 FPS overhead arcade racer built with PSoXide.

#![no_std]
#![no_main]

extern crate psx_rt;

pub mod audio;
pub mod ghost_player;
pub mod ghost_recorder;
pub mod gpu;
pub mod input;
pub mod memcard;
pub mod state;
pub mod ui;

use arduracer_core::{
    compute_standings, cos, sin, AiRacer, ChampionshipSession, Fixed, LapTimer, TrackDef, Vec2,
    VehicleState, AI_PROFILES, ALL_TRACKS,
};
use audio::AudioSystem;
use ghost_player::render_active_ghost;
use ghost_recorder::LapGhostRecorder;
use gpu::{render_car, render_hud, render_track, Camera, ParticleSystem, SkidmarkBuffer};
use input::{InputManager, InputProfile};
use memcard::MemoryCardManager;
use psx_gpu::{self as psx_gpu_mod, framebuf::FrameBuffer, Resolution, VideoMode};
use state::{GameState, StateManager};
use ui::{MenuItem, ResultsScreen};

pub const SCREEN_WIDTH: u16 = 320;
pub const SCREEN_HEIGHT: u16 = 240;

/// Spawns 5 AI rivals on staggered grid positions behind the player.
fn spawn_rivals(start_pos: Vec2, start_heading: u16) -> [AiRacer; 5] {
    let fwd_x = cos(start_heading);
    let fwd_y = sin(start_heading);
    let lat_x = -sin(start_heading);
    let lat_y = cos(start_heading);

    let offsets: [(i32, i32); 5] = [(-36, 14), (-72, -14), (-108, 14), (-144, -14), (-180, 0)];

    let mut rivals = [
        AiRacer::new(start_pos, start_heading, AI_PROFILES[0]),
        AiRacer::new(start_pos, start_heading, AI_PROFILES[1]),
        AiRacer::new(start_pos, start_heading, AI_PROFILES[2]),
        AiRacer::new(start_pos, start_heading, AI_PROFILES[3]),
        AiRacer::new(start_pos, start_heading, AI_PROFILES[4]),
    ];

    for i in 0..5 {
        let (fwd_dist, lat_dist) = offsets[i];
        let offset_x = fwd_x * Fixed::from_int(fwd_dist) + lat_x * Fixed::from_int(lat_dist);
        let offset_y = fwd_y * Fixed::from_int(fwd_dist) + lat_y * Fixed::from_int(lat_dist);
        rivals[i].state.position = start_pos + Vec2::new(offset_x, offset_y);
    }

    rivals
}

/// Root game structure stored in `.bss` to protect the 32 KiB stack.
pub struct ArduracerGame {
    pub frame_counter: u32,
    pub player: VehicleState,
    pub rivals: [AiRacer; 5],
    pub camera: Camera,
    pub particles: ParticleSystem,
    pub skidmarks: SkidmarkBuffer,
    pub timer: LapTimer<16>,
    pub current_track: &'static TrackDef,
    pub current_track_idx: usize,
    pub fb: FrameBuffer,
    pub audio: AudioSystem,
    pub input_mgr: InputManager,
    pub state_mgr: StateManager,
    pub ghost: LapGhostRecorder,
    pub memcard: MemoryCardManager,
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
        let rivals = spawn_rivals(track.start_pos, track.start_heading);
        let camera = Camera::new(track.start_pos);

        let audio = AudioSystem::new();
        let mut input_mgr = InputManager::new(InputProfile::ClassicArcade);
        input_mgr.init();

        ArduracerGame {
            frame_counter: 0,
            player,
            rivals,
            camera,
            particles: ParticleSystem::new(),
            skidmarks: SkidmarkBuffer::new(),
            timer,
            current_track: track,
            current_track_idx: 0,
            fb,
            audio,
            input_mgr,
            state_mgr: StateManager::new(),
            ghost: LapGhostRecorder::new(0),
            memcard: MemoryCardManager::new(),
        }
    }

    /// Loads and resets the active circuit.
    pub fn load_track(&mut self, track_idx: usize) {
        let idx = track_idx % ALL_TRACKS.len();
        self.current_track_idx = idx;
        self.current_track = ALL_TRACKS[idx];
        self.ghost.reset(idx as u8);
        self.reset_race();
    }

    /// Resets the current race state to the starting grid.
    pub fn reset_race(&mut self) {
        let track = self.current_track;
        self.timer = LapTimer::new(&track.checkpoints[..track.checkpoint_count as usize]);
        self.player = VehicleState::new(
            track.start_pos,
            track.start_heading,
            self.state_mgr.garage.tuning,
        );
        self.rivals = spawn_rivals(track.start_pos, track.start_heading);
        self.camera = Camera::new(track.start_pos);
        self.particles = ParticleSystem::new();
        self.skidmarks = SkidmarkBuffer::new();
        self.ghost.start_lap();
        self.timer.start();
    }

    pub fn run(&mut self) -> ! {
        loop {
            // 1. Synchronize to 60Hz NTSC VBlank
            psx_rt::interrupts::wait_vblank();
            self.fb.swap();

            let draw_y = self.fb.buffer_y(self.fb.drawing) as i16;
            self.frame_counter = self.frame_counter.wrapping_add(1);

            let pad = psx_pad::poll_port1();

            match self.state_mgr.current {
                GameState::Title => {
                    if self.state_mgr.title.update(&pad) {
                        self.state_mgr.current = GameState::MainMenu;
                    }
                    psx_gpu_mod::fill_rect(
                        0,
                        draw_y as u16,
                        SCREEN_WIDTH,
                        SCREEN_HEIGHT,
                        12,
                        14,
                        20,
                    );
                    self.state_mgr.title.render(draw_y);
                }
                GameState::MainMenu => {
                    if let Some(item) = self.state_mgr.menu.update(&pad) {
                        match item {
                            MenuItem::TimeTrial => {
                                self.state_mgr.championship = None;
                                self.state_mgr.current = GameState::TrackSelect;
                            }
                            MenuItem::GrandPrix => {
                                self.state_mgr.championship = Some(ChampionshipSession::new(0));
                                self.state_mgr.current = GameState::TrackSelect;
                            }
                            MenuItem::TuningGarage => {
                                self.state_mgr.current = GameState::Garage;
                            }
                            MenuItem::Records => {
                                self.state_mgr.current = GameState::TrackSelect;
                            }
                        }
                    }
                    psx_gpu_mod::fill_rect(
                        0,
                        draw_y as u16,
                        SCREEN_WIDTH,
                        SCREEN_HEIGHT,
                        15,
                        18,
                        25,
                    );
                    self.state_mgr.menu.render(draw_y);
                }
                GameState::Garage => {
                    if self.state_mgr.garage.update(&pad) {
                        self.player.tuning = self.state_mgr.garage.tuning;
                        self.state_mgr.current = GameState::MainMenu;
                    }
                    psx_gpu_mod::fill_rect(
                        0,
                        draw_y as u16,
                        SCREEN_WIDTH,
                        SCREEN_HEIGHT,
                        15,
                        18,
                        25,
                    );
                    self.state_mgr.garage.render(draw_y);
                }
                GameState::TrackSelect => {
                    let (confirmed, cancelled) = self.state_mgr.track_select.update(&pad);
                    if let Some(track_idx) = confirmed {
                        let target_idx = if let Some(ref mut champ) = self.state_mgr.championship {
                            let cup_idx = ((track_idx / 6) as u8).min(3);
                            champ.cup_index = cup_idx;
                            champ.current_stage = 0;
                            champ.current_track_idx()
                        } else {
                            track_idx
                        };
                        self.load_track(target_idx);
                        self.state_mgr.current = GameState::Racing;
                    } else if cancelled {
                        self.state_mgr.current = GameState::MainMenu;
                    }
                    psx_gpu_mod::fill_rect(
                        0,
                        draw_y as u16,
                        SCREEN_WIDTH,
                        SCREEN_HEIGHT,
                        15,
                        18,
                        25,
                    );
                    self.state_mgr.track_select.render(draw_y);
                }
                GameState::Racing => {
                    // Query surface beneath vehicle
                    let tx = (self.player.position.x.to_int() / 64) as u8;
                    let ty = (self.player.position.y.to_int() / 64) as u8;
                    let surface = self.current_track.surface_at(tx, ty);

                    // Controller input poll and haptics update
                    let input = self.input_mgr.update(&self.player, surface);
                    self.player.tick(input, surface);
                    self.timer.tick();
                    self.timer.update_player_tile(tx, ty);

                    // AI rivals tick with dynamic obstacle avoidance
                    let mut other_positions = [Vec2::ZERO; 6];
                    other_positions[0] = self.player.position;
                    for i in 0..5 {
                        other_positions[i + 1] = self.rivals[i].state.position;
                    }
                    for i in 0..5 {
                        self.rivals[i].tick(self.current_track, &other_positions);
                    }

                    // Compute dynamic race standings
                    let standings = compute_standings(
                        self.player.position,
                        self.timer.current_lap,
                        self.timer.next_checkpoint_idx,
                        self.timer.is_finished,
                        &self.rivals,
                        self.current_track,
                    );
                    let mut player_rank = 1u8;
                    for (place, &competitor_idx) in standings.iter().enumerate() {
                        if competitor_idx == 0 {
                            player_rank = (place + 1) as u8;
                            break;
                        }
                    }

                    // Ghost telemetry sample
                    self.ghost.record_tick(&self.player);

                    // Check race completion (5 laps)
                    if self.timer.is_finished {
                        let medal = self
                            .current_track
                            .par_times
                            .evaluate_medal(self.timer.best_lap_ticks);
                        let is_new = self.memcard.record_lap(
                            self.current_track_idx,
                            self.timer.best_lap_ticks,
                            medal as u8,
                        );
                        self.ghost.finish_lap(self.timer.best_lap_ticks, is_new);

                        if let Some(ref mut champ) = self.state_mgr.championship {
                            champ.award_stage_points(standings);
                        }

                        self.state_mgr.results = Some(ResultsScreen::new(
                            self.timer.best_lap_ticks,
                            self.timer.current_lap_ticks,
                            self.current_track,
                            player_rank,
                        ));
                        self.state_mgr.current = GameState::Results;
                    }

                    // Audio engine tick (engine RPM synth, tire screech, SFX)
                    self.audio
                        .tick(&self.player, input.throttle, self.timer.next_checkpoint_idx);

                    // Emit smoke particles and skidmarks during hard turns or drifts
                    if self.player.is_drifting {
                        self.particles.emit_smoke(self.player.position);
                        self.skidmarks
                            .emit(self.player.position, self.player.visual_angle);
                    }
                    self.particles.tick();
                    self.skidmarks.tick();

                    // Camera update
                    self.camera.update(
                        self.player.position,
                        self.player.velocity,
                        self.player.speed,
                    );

                    // Render Pass:
                    // a. Clear background
                    psx_gpu_mod::fill_rect(
                        0,
                        draw_y as u16,
                        SCREEN_WIDTH,
                        SCREEN_HEIGHT,
                        18,
                        20,
                        26,
                    );
                    // b. Track tilemap
                    render_track(self.current_track, &self.camera, draw_y);
                    // c. Skidmarks on track
                    self.skidmarks.render(&self.camera, draw_y);
                    // d. Active ghost car playback
                    render_active_ghost(
                        &self.ghost,
                        &self.camera,
                        self.timer.current_lap_ticks,
                        draw_y,
                    );
                    // e. Particle effects
                    self.particles.render(&self.camera, draw_y);
                    // f. AI Rivals rendering
                    for rival in &self.rivals {
                        render_car(
                            &rival.state,
                            &self.camera,
                            draw_y,
                            false,
                            rival.profile.color,
                        );
                    }
                    // g. Player race car (Crimson Red: 220, 25, 45)
                    render_car(&self.player, &self.camera, draw_y, false, (220, 25, 45));
                    // h. In-Game HUD overlay
                    render_hud(
                        &self.player,
                        &self.timer,
                        self.current_track,
                        player_rank,
                        &self.rivals,
                        draw_y,
                    );
                }
                GameState::Results => {
                    psx_gpu_mod::fill_rect(
                        0,
                        draw_y as u16,
                        SCREEN_WIDTH,
                        SCREEN_HEIGHT,
                        14,
                        16,
                        22,
                    );
                    let mut action_cont = false;
                    let mut action_exit = false;
                    if let Some(ref mut results) = self.state_mgr.results {
                        let (cont, exit) = results.update(&pad);
                        action_cont = cont;
                        action_exit = exit;
                        results.render(draw_y);
                    }
                    if action_cont {
                        let next_track = if let Some(ref mut champ) = self.state_mgr.championship {
                            if champ.advance_stage() {
                                None
                            } else {
                                Some(champ.current_track_idx())
                            }
                        } else {
                            None
                        };

                        if let Some(track_idx) = next_track {
                            self.load_track(track_idx);
                            self.state_mgr.current = GameState::Racing;
                        } else if self.state_mgr.championship.is_some() {
                            self.state_mgr.championship = None;
                            self.state_mgr.current = GameState::MainMenu;
                        } else {
                            self.reset_race();
                            self.state_mgr.current = GameState::Racing;
                        }
                    } else if action_exit {
                        self.state_mgr.championship = None;
                        self.state_mgr.current = GameState::MainMenu;
                    }
                }
            }
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
