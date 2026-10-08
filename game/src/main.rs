//! Arduracer PSX - Bare-metal PlayStation 1 Entry Point.
//!
//! Locked 60 FPS overhead arcade racer built with PSoXide.

#![no_std]
#![no_main]

extern crate psx_rt;

pub mod audio;
pub mod cd_fs;
pub mod ghost_player;
pub mod ghost_recorder;
pub mod gpu;
pub mod input;
pub mod memcard;
pub mod state;
pub mod ui;
pub mod video;

use arduracer_core::{
    compute_standings, AiRacer, ChampionshipSession, CityDef, Fixed, LapTimer, StartPhase,
    StartSequence, TrackDef, Vec2, VehicleState, AI_PROFILES, ALL_CITIES, ALL_TRACKS,
    ALL_TRACK_VISUALS,
};
use audio::AudioSystem;
use ghost_player::render_active_ghost;
use ghost_recorder::LapGhostRecorder;
use gpu::{
    bake_city_minimap, bake_minimap, init_track_texture, render_car, render_city_hud, render_hud,
    render_track, render_track_extents, Camera, ParticleSystem, SkidmarkBuffer, TextureSlot,
};
use input::{InputManager, InputProfile};
use memcard::{MemcardStatus, MemoryCardManager};
use psx_gpu::{self as psx_gpu_mod, framebuf::FrameBuffer, Resolution, VideoMode};
use state::{GameState, StateManager};
use ui::font::draw_text;
use ui::{MenuItem, PauseChoice, PauseMenu, ResultsScreen};

pub const SCREEN_WIDTH: u16 = 320;
pub const SCREEN_HEIGHT: u16 = 240;

/// Staggered grid slots behind the pole car, alternating sides of the road.
const GRID_OFFSETS: [(i32, i32); 5] = [(-48, 16), (-96, -16), (-144, 16), (-192, -16), (-240, 0)];

/// Whether AI rivals take part in a race.
///
/// **Off, deliberately.** One authored circuit exists (`Hells Bells`), so every
/// grid slot holds the same oval. Five rivals would be stacked on one racing
/// line, racing the player for a position on a circuit they all know as well as
/// the car does -- noise, not a race.
///
/// Gating all three of *drive*, *spawn* and *render* from one flag is the point.
/// Rivals that are spawned but not ticked are parked on the start line, still
/// drawn and still occupying road, so a tick-only gate would leave the player
/// driving away from five stationary cars. The rivals themselves are left in
/// place rather than deleted, so this is a constant flip and the standings code
/// keeps compiling against a real array.
///
/// Switch back on as soon as a second circuit exists to race on.
const RIVALS_ENABLED: bool = false;

/// Spawns 5 AI rivals on staggered grid positions behind the player.
fn spawn_rivals(start_pos: Vec2, start_heading: u16) -> [AiRacer; 5] {
    let mut rivals = [
        AiRacer::new(start_pos, start_heading, AI_PROFILES[0]),
        AiRacer::new(start_pos, start_heading, AI_PROFILES[1]),
        AiRacer::new(start_pos, start_heading, AI_PROFILES[2]),
        AiRacer::new(start_pos, start_heading, AI_PROFILES[3]),
        AiRacer::new(start_pos, start_heading, AI_PROFILES[4]),
    ];
    for (i, rival) in rivals.iter_mut().enumerate() {
        let (fwd, lat) = GRID_OFFSETS[i];
        rival.offset_from_pole(start_pos, start_heading, fwd, lat);
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
    pub pause: PauseMenu,
    pub paused: bool,
    pub show_hud: bool,
    /// VRAM-resident minimap image, re-baked whenever the circuit changes.
    pub minimap_texture: TextureSlot,
    pub current_track: &'static TrackDef,
    pub current_track_idx: usize,
    pub active_city: Option<&'static CityDef>,
    pub fb: FrameBuffer,
    pub audio: AudioSystem,
    pub input_mgr: InputManager,
    pub state_mgr: StateManager,
    pub ghost: LapGhostRecorder,
    pub memcard: MemoryCardManager,
    /// Three lights, then GO. The lap clock arms on GO, not at load.
    pub start: StartSequence,
}

impl Default for ArduracerGame {
    fn default() -> Self {
        Self::new()
    }
}

impl ArduracerGame {
    pub fn new() -> Self {
        psx_gpu_mod::init(VideoMode::Ntsc, Resolution::R320X240);

        // SPU audio & FMV attract intro from CD-ROM. Skippable; silently absent when the
        // disc has no INTRO.STR (e.g. EXE side-loaded in an emulator).
        audio::play_intro_audio();
        let intro_res = video::play_video("INTRO.STR");
        audio::stop_intro_audio();

        // Initialize the disc track texture streaming and locate TRACKS.BIN NOW,
        // while the drive is stopped and before CD-DA audio playback is started.
        init_track_texture(ALL_TRACK_VISUALS[0]);

        let fb = FrameBuffer::new(SCREEN_WIDTH, SCREEN_HEIGHT);
        psx_gpu_mod::set_draw_area(0, 0, SCREEN_WIDTH - 1, SCREEN_HEIGHT - 1);
        psx_gpu_mod::set_draw_offset(0, 0);

        let track = ALL_TRACKS[0];
        let timer = LapTimer::new(track.checkpoint_slice(), track.start_gate);
        let player = VehicleState::new(track.start_pos, track.start_heading, Default::default());
        let rivals = spawn_rivals(track.start_pos, track.start_heading);
        let camera = Camera::new(track.start_pos);

        // Probe the memory card once during boot so every later frame is
        // pure RAM work (GAME.md §8).
        let mut memcard = MemoryCardManager::new();
        memcard.probe();

        let mut audio = AudioSystem::new();
        audio.cdda.play_track(2);
        let mut input_mgr = InputManager::new(InputProfile::ClassicArcade);
        input_mgr.init();

        let mut state_mgr = StateManager::new();
        if intro_res == video::VideoResult::Skipped {
            state_mgr.current = GameState::MainMenu;
        }

        let game = ArduracerGame {
            frame_counter: 0,
            player,
            rivals,
            camera,
            particles: ParticleSystem::new(),
            skidmarks: SkidmarkBuffer::new(),
            timer,
            pause: PauseMenu::new(),
            paused: false,
            minimap_texture: TextureSlot::new(crate::gpu::texlayout::MAX_DIM),
            show_hud: true,
            current_track: track,
            current_track_idx: 0,
            active_city: None,
            fb,
            audio,
            input_mgr,
            state_mgr,
            ghost: LapGhostRecorder::new(0),
            start: StartSequence::new(),
            memcard,
        };
        // The boot track never goes through `load_track`, so bake its minimap
        // here or the HUD would blit an empty texture until the first load.
        bake_minimap(&game.minimap_texture, game.current_track);
        game
    }

    /// Loads and resets the active circuit.
    pub fn load_track(&mut self, track_idx: usize) {
        let idx = track_idx % ALL_TRACKS.len();
        self.active_city = None;
        self.current_track_idx = idx;
        self.current_track = ALL_TRACKS[idx];
        self.ghost.reset(idx as u8);
        self.reset_race();
        // Re-bake the minimap for the new circuit. Once per load: the image is
        // static for the whole race, so this must not be per-frame (TASK-1202).
        bake_minimap(&self.minimap_texture, self.current_track);
        self.audio.cdda.stop();
        init_track_texture(ALL_TRACK_VISUALS[idx]);

        // Play the CD-DA theme corresponding to the active cup:
        // Cup 1 (Tracks 1-6) -> CD-DA Track 3 ("Asphalt Adrenaline" Eurobeat)
        // Cup 2 (Tracks 7-12) -> CD-DA Track 4 ("Night Drift City" D&B)
        // Cup 3 (Tracks 13-18) -> CD-DA Track 5 ("Canyon Rush" Techno)
        // Cup 4 (Tracks 19-24) -> CD-DA Track 6 ("Apex Predator" Trance)
        let cdda_track = 3 + ((idx / 6) as u8).min(3);
        self.audio.cdda.play_track(cdda_track);
        // `Start` confirms through the title, main menu and track select, so a
        // player can load a stage still holding it. Without this the first
        // racing frame reads that as a fresh press and the pause veil opens
        // itself with the music stopped (TASK-1209).
        self.pause.arm_for_race_start();
        // A stage loaded: the engine synth may sound again. Still gated on
        // `!paused`, and `reset_race` below clears the veil, so this cannot arm
        // the engine while the pause menu is up.
        self.audio.enter_race();
    }

    /// Loads and resets the active city for free urban driving.
    pub fn load_city(&mut self, city_idx: usize) {
        let idx = city_idx % ALL_CITIES.len();
        let city = ALL_CITIES[idx];
        self.active_city = Some(city);
        let (start_pos, start_heading) = if let Some(race) = city.races.first() {
            (race.start_pos, race.start_heading)
        } else {
            (Vec2::new(Fixed::from_int(128), Fixed::from_int(128)), 0)
        };
        self.player = VehicleState::new(start_pos, start_heading, self.state_mgr.garage.tuning());
        self.camera = Camera::new(start_pos);
        self.particles = ParticleSystem::new();
        self.skidmarks = SkidmarkBuffer::new();
        bake_city_minimap(&self.minimap_texture, city);
        self.audio.cdda.stop();
        init_track_texture(ALL_TRACK_VISUALS[idx % ALL_TRACK_VISUALS.len()]);
        self.paused = false;
        self.show_hud = true;
        self.start = StartSequence::new();
        self.audio.cdda.play_track(4);
        self.pause.arm_for_race_start();
        self.audio.enter_race();
    }

    /// Resets the current race state to the starting grid.
    pub fn reset_race(&mut self) {
        let track = self.current_track;
        self.timer = LapTimer::new(track.checkpoint_slice(), track.start_gate);
        self.player = VehicleState::new(
            track.start_pos,
            track.start_heading,
            self.state_mgr.garage.tuning(),
        );
        self.rivals = spawn_rivals(track.start_pos, track.start_heading);
        self.camera = Camera::new(track.start_pos);
        self.particles = ParticleSystem::new();
        self.skidmarks = SkidmarkBuffer::new();
        self.ghost.start_lap();
        // The clock is deliberately NOT armed here: it starts on GO. See
        // `StartSequence`.
        self.start = StartSequence::new();
        self.paused = false;
        self.show_hud = true;
    }

    /// Draws the five rivals back-to-front by screen row.
    ///
    /// Cars were previously drawn in array order with the player unconditionally
    /// last, so two cars on the same stretch of road interpenetrated: the one
    /// nearer the camera was painted over by the one behind it (TASK-1206).
    /// Sorting by projected screen `y` gives a painter's order that matches
    /// which car is actually in front.
    fn render_rivals_sorted(&self) {
        if !RIVALS_ENABLED {
            return;
        }
        // Insertion sort by depth. Five elements: cheaper in code size than
        // allocating a buffer.
        let mut order = [0usize; 5];
        let mut depth = [0i16; 5];
        for (i, rival) in self.rivals.iter().enumerate() {
            order[i] = i;
            depth[i] = self.camera.world_to_screen(rival.state.position).1;
        }
        for i in 1..order.len() {
            let mut j = i;
            while j > 0 && depth[order[j - 1]] > depth[order[j]] {
                order.swap(j - 1, j);
                j -= 1;
            }
        }
        for idx in order {
            let rival = &self.rivals[idx];
            render_car(&rival.state, &self.camera, false, rival.profile.color);
        }
    }

    /// Repaints the last simulated race frame (used while paused).
    fn draw_frozen_race(&mut self) {
        self.fb.clear(18, 20, 26);
        if let Some(city) = self.active_city {
            render_track_extents(city.world_width(), city.world_height(), &self.camera);
        } else {
            render_track(self.current_track, &self.camera);
        }
        self.skidmarks.render(&self.camera);
        // Same ordering rule as the live frame: smoke belongs on top of the cars
        // it came from (TASK-1206).
        self.particles.render(&self.camera);
        if self.active_city.is_none() {
            for rival in &self.rivals {
                render_car(&rival.state, &self.camera, false, rival.profile.color);
            }
        }
        render_car(&self.player, &self.camera, false, (220, 25, 45));
    }

    pub fn run(&mut self) -> ! {
        // The queued display flip is applied by the VBlank handler, which
        // requires this counter (TASK-1201).
        psx_rt::interrupts::install_vblank_counter();

        loop {
            // 1. Synchronize to 60Hz NTSC VBlank. The handler has now applied
            //    last frame's queued flip, so program the draw target for the
            //    buffer we are about to fill.
            psx_rt::interrupts::wait_vblank();
            self.fb.apply_draw_target();

            self.frame_counter = self.frame_counter.wrapping_add(1);

            let pad = psx_pad::poll_port1();

            match self.state_mgr.current {
                GameState::Title => {
                    self.audio.ui_frame(pad.buttons.bits());
                    if self.state_mgr.title.update(&pad) {
                        self.state_mgr.current = GameState::MainMenu;
                    }
                    self.fb.clear(12, 14, 20);
                    self.state_mgr.title.render();
                }
                GameState::MainMenu => {
                    self.audio.ui_frame(pad.buttons.bits());
                    if let Some(item) = self.state_mgr.menu.update(&pad) {
                        match item {
                            MenuItem::TimeTrial => {
                                self.state_mgr.championship = None;
                                self.state_mgr.track_select.arm_for_entry();
                                self.state_mgr.current = GameState::TrackSelect;
                            }
                            MenuItem::GrandPrix => {
                                self.state_mgr.championship = Some(ChampionshipSession::new(0));
                                self.state_mgr.track_select.arm_for_entry();
                                self.state_mgr.current = GameState::TrackSelect;
                            }
                            MenuItem::CityDrive => {
                                self.state_mgr.city_select.arm_for_entry();
                                self.state_mgr.current = GameState::CitySelect;
                            }
                            MenuItem::TuningGarage => {
                                // Load the active preset from the memory card.
                                let slot = self.memcard.save_data.active_tuning_slot as usize;
                                if slot < 3 {
                                    self.state_mgr
                                        .garage
                                        .load_tuning(self.memcard.save_data.tuning_slots[slot]);
                                }
                                // A `Cross`/`Start` held on the main menu must
                                // not save-and-exit on the garage's first frame.
                                self.state_mgr.garage.sync_edges(&pad);
                                self.state_mgr.current = GameState::Garage;
                            }
                            MenuItem::Records => {
                                self.state_mgr.track_select.arm_for_entry();
                                self.state_mgr.current = GameState::TrackSelect;
                            }
                        }
                    }
                    self.fb.clear(15, 18, 25);
                    self.state_mgr.menu.render();
                }
                GameState::CitySelect => {
                    self.audio.ui_frame(pad.buttons.bits());
                    let (confirmed, cancelled) = self.state_mgr.city_select.update(&pad);
                    if let Some(city_idx) = confirmed {
                        self.load_city(city_idx);
                        self.state_mgr.current = GameState::Racing;
                    } else if cancelled {
                        self.state_mgr.current = GameState::MainMenu;
                    }
                    self.fb.clear(15, 18, 25);
                    self.state_mgr.city_select.render();
                }
                GameState::Garage => {
                    self.audio.ui_frame(pad.buttons.bits());
                    if self.state_mgr.garage.update(&pad) {
                        let tuning = self.state_mgr.garage.tuning();
                        self.player.tuning = tuning;
                        // Persist the preset to the active save slot (TASK-602).
                        // `store_tuning` reports a refusal: the garage's own
                        // bounds now match `is_valid`, so this should not fire,
                        // but a silent discard here would lose the player's work
                        // with no indication (TASK-1210).
                        // Persist to the preset the player selected in the
                        // garage, and record the selection so it is the one
                        // loaded next time (TASK-1217).
                        let slot = self.state_mgr.garage.slot();
                        if self.memcard.store_tuning(slot, tuning) {
                            self.memcard.save_data.active_tuning_slot = slot as u8;
                            self.memcard.mark_dirty();
                            self.memcard.flush();
                        }
                        self.state_mgr.current = GameState::MainMenu;
                    }
                    self.fb.clear(15, 18, 25);
                    self.state_mgr.garage.render();
                }
                GameState::TrackSelect => {
                    self.audio.ui_frame(pad.buttons.bits());
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
                    self.fb.clear(15, 18, 25);
                    self.state_mgr.track_select.render();
                }
                GameState::Racing => {
                    let track = self.current_track;

                    // GAME.md §7: Start pauses, Select toggles HUD / minimap.
                    // `PauseMenu` owns all button edge state -- the race loop
                    // must not touch it, or the menu sees every frame as
                    // "nothing newly pressed" and cannot be dismissed.
                    let frame = self.pause.update_from_pad(&pad);
                    let was_paused = self.paused;
                    if frame.start_pressed {
                        self.paused = !self.paused;
                        // Cutting the engine has to happen here rather than in
                        // `AudioSystem::tick`, because the paused arm of this
                        // arm `continue`s and never reaches the tick.
                        self.audio.set_paused(self.paused);
                        if self.paused {
                            self.audio.cdda.pause();
                        } else {
                            self.audio.cdda.resume();
                        }
                    }
                    if frame.select_pressed {
                        self.show_hud = !self.show_hud;
                    }

                    // Navigating the pause veil is a menu interaction, so it
                    // gets the UI blip and nothing else.
                    if self.paused {
                        self.audio.ui_frame(pad.buttons.bits());
                    }

                    if self.paused {
                        // The frame that opens the veil also reports the START
                        // press that opened it, so its confirm is ignored.
                        if was_paused {
                            match frame.choice {
                                PauseChoice::Resume => {
                                    self.paused = false;
                                    self.audio.set_paused(false);
                                    self.audio.cdda.resume();
                                }
                                PauseChoice::RestartRace => {
                                    if let Some(city) = self.active_city {
                                        let (start_pos, start_heading) =
                                            if let Some(race) = city.races.first() {
                                                (race.start_pos, race.start_heading)
                                            } else {
                                                (
                                                    Vec2::new(
                                                        Fixed::from_int(128),
                                                        Fixed::from_int(128),
                                                    ),
                                                    0,
                                                )
                                            };
                                        self.player = VehicleState::new(
                                            start_pos,
                                            start_heading,
                                            self.state_mgr.garage.tuning(),
                                        );
                                        self.camera = Camera::new(start_pos);
                                        self.particles = ParticleSystem::new();
                                        self.skidmarks = SkidmarkBuffer::new();
                                        self.start = StartSequence::new();
                                        self.audio.enter_race();
                                        self.pause.arm_for_race_start();
                                        self.pause.sync_edges(PauseMenu::input_from_pad(&pad));
                                        self.audio.cdda.play_track(4);
                                    } else {
                                        self.reset_race();
                                        self.audio.enter_race();
                                        // Same held-`Start` hazard as `load_track`:
                                        // the restart happens from inside the veil,
                                        // where the button is necessarily held.
                                        self.pause.arm_for_race_start();
                                        // Restarting skips frames; adopt the buttons
                                        // held right now so the same press is not
                                        // re-read against the fresh race.
                                        self.pause.sync_edges(PauseMenu::input_from_pad(&pad));
                                        let cdda_track =
                                            3 + ((self.current_track_idx / 6) as u8).min(3);
                                        self.audio.cdda.play_track(cdda_track);
                                    }
                                }
                                PauseChoice::QuitToMenu => {
                                    self.active_city = None;
                                    self.state_mgr.championship = None;
                                    self.state_mgr.current = GameState::MainMenu;
                                    // `self.paused` stays true across this
                                    // transition, so arming here would leak the
                                    // engine into the menu.
                                    self.audio.leave_race();
                                    self.audio.sync_ui_edges(pad.buttons.bits());
                                    self.audio.cdda.play_track(2);
                                }
                                PauseChoice::None => {}
                            }
                        }
                        // Repaint the frozen world under the pause veil.
                        self.draw_frozen_race();
                        self.pause.render();
                        // This arm `continue`s, so it has to close its own frame.
                        // Step 5 sits at the bottom of the loop, *after* this
                        // `continue`: without this call the veil was rasterised
                        // into the back buffer and never presented, so the screen
                        // kept showing the last racing frame and the pause menu
                        // was invisible (TASK-1215).
                        //
                        // The memory-card notice is redrawn here as well. It is
                        // normally step 4, which this arm also skips, and a save
                        // that failed mid-race has to stay on screen while the
                        // game is paused.
                        self.memcard.tick_notice();
                        if let Some(status) = self.memcard.notice() {
                            render_card_notice(status);
                        }
                        self.close_frame();
                        continue;
                    }

                    // Controller input poll (surface known from the previous tick
                    // so the rumble motors can react to curbs and impacts).
                    let pre_surface = if self.active_city.is_some() {
                        arduracer_core::SurfaceType::Tarmac
                    } else {
                        let pre_tx = TrackDef::tile_x_of(self.player.position.x);
                        let pre_ty = TrackDef::tile_y_of(self.player.position.y);
                        track.surface_at(pre_tx, pre_ty)
                    };
                    // Same pad sample the UI saw this frame, so a button press
                    // cannot open the pause menu and also be missing from the
                    // car's controls (TASK-1214).
                    let mut input = self.input_mgr.update(&pad, &self.player, pre_surface);

                    // Start sequence: hold the car on the grid, then arm the lap
                    // clock the instant the lights go out.
                    self.start.tick();
                    if self.active_city.is_none() {
                        if self.start.just_started() {
                            self.timer.start();
                        }
                        if !self.start.accepts_input() {
                            input.throttle = Fixed::ZERO;
                            input.brake = Fixed::ZERO;
                            input.steer = Fixed::ZERO;
                            input.handbrake = false;
                            input.nitro = false;
                        }
                    }

                    // Recovery: stuck, spun, or wedged with no way out. Drops the
                    // car on the nearest route node facing down the racing line
                    // and clears every state that could be pinning it.
                    if self.input_mgr.controller.respawn_pressed {
                        if let Some(city) = self.active_city {
                            let (pos, heading) = if let Some(race) = city.races.first() {
                                (race.start_pos, race.start_heading)
                            } else {
                                (Vec2::new(Fixed::from_int(128), Fixed::from_int(128)), 0)
                            };
                            self.player.respawn_at(pos, heading);
                        } else {
                            let (pos, heading) = track.respawn_point(self.player.position);
                            self.player.respawn_at(pos, heading);
                        }
                    }

                    // Simulate: physics tick, then resolve track + bounds.
                    self.player.tick(input, pre_surface);
                    let (hit_wall, surface) = if let Some(city) = self.active_city {
                        let ww = city.world_width();
                        let wh = city.world_height();
                        let mut hit = false;
                        let min_b = Fixed::from_int(32);
                        let max_bx = Fixed::from_int((ww - 32).max(64));
                        let max_by = Fixed::from_int((wh - 32).max(64));
                        if self.player.position.x < min_b {
                            self.player.position.x = min_b;
                            self.player.velocity.x = Fixed::ZERO;
                            hit = true;
                        } else if self.player.position.x > max_bx {
                            self.player.position.x = max_bx;
                            self.player.velocity.x = Fixed::ZERO;
                            hit = true;
                        }
                        if self.player.position.y < min_b {
                            self.player.position.y = min_b;
                            self.player.velocity.y = Fixed::ZERO;
                            hit = true;
                        } else if self.player.position.y > max_by {
                            self.player.position.y = max_by;
                            self.player.velocity.y = Fixed::ZERO;
                            hit = true;
                        }
                        (hit, arduracer_core::SurfaceType::Tarmac)
                    } else {
                        let hit = self.player.collide_with_track(track);
                        let tx = TrackDef::tile_x_of(self.player.position.x);
                        let ty = TrackDef::tile_y_of(self.player.position.y);
                        (hit, track.surface_at(tx, ty))
                    };

                    let mut player_rank = 1u8;
                    if self.active_city.is_none() {
                        let tx = TrackDef::tile_x_of(self.player.position.x);
                        let ty = TrackDef::tile_y_of(self.player.position.y);
                        self.timer.tick();
                        if self.timer.update_player_tile(tx, ty) {
                            // A lap was just scored. Promote this lap's telemetry to
                            // the ghost if it is the fastest so far, then start a
                            // fresh recording: without this the "ghost" was only ever
                            // the opening seconds of lap 1.
                            let lap_ticks = self.timer.last_completed_lap_ticks;
                            let is_record =
                                lap_ticks != 0 && lap_ticks == self.timer.best_lap_ticks;
                            self.ghost.finish_lap(lap_ticks, is_record);
                            self.ghost.start_lap();
                        }

                        // AI rivals tick with dynamic obstacle avoidance
                        let mut other_positions = [Vec2::ZERO; 6];
                        other_positions[0] = self.player.position;
                        for i in 0..5 {
                            other_positions[i + 1] = self.rivals[i].state.position;
                        }
                        if RIVALS_ENABLED
                            && (self.start.phase() == StartPhase::Racing
                                || self.start.just_started())
                        {
                            // Rivals launch with the player: held on the grid until
                            // the lights go out, like a standing start.
                            for i in 0..5 {
                                self.rivals[i].tick(track, &other_positions);
                            }
                        }

                        // Race standings: player checkpoint progress is compared on
                        // the same route index the rivals use.
                        let player_route_node =
                            self.timer.route_node_index(track.route_len()) as u8;
                        let standings = compute_standings(
                            self.player.position,
                            self.timer.current_lap,
                            player_route_node,
                            self.timer.is_finished,
                            &self.rivals,
                            track,
                        );
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
                            let medal = track.par_times.evaluate_medal(self.timer.best_lap_ticks);
                            self.memcard.record_lap(
                                self.current_track_idx,
                                self.timer.best_lap_ticks,
                                medal as u8,
                            );
                            // Every scored lap already promoted itself; this only
                            // closes out the in-progress final lap.
                            self.ghost.finish_lap(self.timer.best_lap_ticks, false);
                            // Persist the record before leaving the race screen.
                            if self.memcard.is_dirty {
                                self.memcard.flush();
                            }

                            if let Some(ref mut champ) = self.state_mgr.championship {
                                champ.award_stage_points(standings);
                            }

                            self.state_mgr.results = Some(ResultsScreen::new(
                                self.timer.best_lap_ticks,
                                self.timer.current_lap_ticks,
                                track,
                                player_rank,
                            ));
                            self.state_mgr.current = GameState::Results;
                            // Cut before the tick below, which would otherwise run
                            // once more this frame and re-latch a live engine volume
                            // over the victory fanfare.
                            self.audio.leave_race();
                            self.audio.sync_ui_edges(pad.buttons.bits());
                            self.audio.cdda.play_track(7);
                        }
                    }

                    let cleared = if self.active_city.is_some() {
                        0
                    } else {
                        self.timer.checkpoints_cleared() as u8
                    };
                    // Audio engine tick (engine RPM synth, tire screech, SFX)
                    self.audio
                        .tick(&self.player, input.throttle, surface, hit_wall, cleared);

                    // Emit smoke particles and skidmarks during hard turns or drifts
                    if self.player.is_drifting {
                        self.particles.emit_smoke(self.player.position);
                        self.skidmarks
                            .emit(self.player.position, self.player.visual_angle);
                    }
                    // Wall scrape sparks + heavy rumble on a barrier hit.
                    if hit_wall {
                        let away = self.player.velocity.scale(Fixed::from_raw(-256));
                        self.particles.emit_sparks(self.player.position, away);
                        self.input_mgr.rumble.trigger_impact(200);
                    }
                    if self.player.boost_ticks == 29 {
                        self.input_mgr.rumble.trigger_boost();
                    }
                    self.particles.tick();
                    self.skidmarks.tick();

                    // Camera update, clamped to the circuit or city so the view never
                    // leaves the world.
                    let (clamp_w, clamp_h) = if let Some(city) = self.active_city {
                        (city.world_width(), city.world_height())
                    } else {
                        (track.world_width(), track.world_height())
                    };
                    self.camera.update(
                        self.player.position,
                        self.player.velocity,
                        self.player.speed,
                        clamp_w,
                        clamp_h,
                    );

                    // Render Pass:
                    // a. Clear background
                    self.fb.clear(18, 20, 26);
                    // b. Track / city visual map
                    if let Some(city) = self.active_city {
                        render_track_extents(city.world_width(), city.world_height(), &self.camera);
                    } else {
                        render_track(track, &self.camera);
                    }
                    // c. Skidmarks on track
                    self.skidmarks.render(&self.camera);
                    if self.active_city.is_none() {
                        // d. Active ghost car playback
                        render_active_ghost(
                            &self.ghost,
                            &self.camera,
                            self.timer.current_lap_ticks,
                        );
                        // e. AI Rivals rendering
                        self.render_rivals_sorted();
                    }
                    // f. Player race car (Crimson Red: 220, 25, 45)
                    render_car(&self.player, &self.camera, false, (220, 25, 45));
                    // g. Particle effects
                    self.particles.render(&self.camera);
                    // h. Start lights (racing circuit only)
                    if self.active_city.is_none() {
                        render_start_lights(self.start.phase());
                    }

                    // i. In-Game HUD overlay (Select hides it for clean screenshots)
                    if self.show_hud {
                        if let Some(city) = self.active_city {
                            render_city_hud(&self.player, city, &self.minimap_texture);
                        } else {
                            render_hud(
                                &self.player,
                                &self.timer,
                                track,
                                player_rank,
                                &self.rivals,
                                &self.minimap_texture,
                            );
                        }
                    }
                }
                GameState::Results => {
                    self.fb.clear(14, 16, 22);
                    // Already silent via the Racing -> Results transition; this
                    // is the belt-and-braces path in case a future transition
                    // reaches Results some other way.
                    self.audio.ui_frame(pad.buttons.bits());
                    let mut action_cont = false;
                    let mut action_exit = false;
                    if let Some(ref mut results) = self.state_mgr.results {
                        let (cont, exit) = results.update(&pad);
                        action_cont = cont;
                        action_exit = exit;
                        results.render();
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
                            self.audio.leave_race();
                            self.audio.sync_ui_edges(pad.buttons.bits());
                            self.audio.cdda.play_track(2);
                        } else {
                            self.load_track(self.current_track_idx);
                            self.state_mgr.current = GameState::Racing;
                        }
                    } else if action_exit {
                        self.active_city = None;
                        self.state_mgr.championship = None;
                        self.state_mgr.current = GameState::MainMenu;
                        self.audio.leave_race();
                        self.audio.sync_ui_edges(pad.buttons.bits());
                        self.audio.cdda.play_track(2);
                    }
                }
            }

            // 4. Memory card notice. Drawn over every screen: the write that
            // failed happened during a race, and the player needs to know their
            // record is not on the card.
            self.memcard.tick_notice();
            if let Some(status) = self.memcard.notice() {
                render_card_notice(status);
            }

            // 5. Close the frame and queue the flip for the next VBlank.
            self.close_frame();
        }
    }

    /// Ends the frame: closes the GP0 command stream and queues the display flip
    /// for the next VBlank.
    ///
    /// Split out of the loop body because the pause arm `continue`s before
    /// reaching it, and a frame that is never flipped is never seen. That was
    /// the whole of the pause menu's disappearance: `draw_frozen_race` and
    /// `PauseMenu::render` both ran correctly, into a back buffer nothing ever
    /// presented.
    ///
    /// The old loop was a bare `wait_vblank(); fb.swap()`, and
    /// `FrameBuffer::swap` writes its three GP0 words with no
    /// `wait_cmd_ready()`. If the 256-word command FIFO was full those words
    /// were dropped and the draw area stayed pointed at the previous buffer;
    /// nothing also stopped frame N+1 being submitted while VBlank flipped to
    /// it, collapsing the double buffer to a one-deep queue (TASK-1201).
    ///
    /// GP0(1Fh) closes the command stream, and the VBlank handler applies the
    /// display-start word only once the GPU has reached that flag -- so the flip
    /// cannot land on a half-drawn frame.
    fn close_frame(&mut self) {
        psx_gpu_mod::signal_draw_done();
        let flip = self.fb.begin_deferred_swap();
        psx_rt::interrupts::queue_gp1_at_vblank(flip);
    }
}

/// Three-light start rig, centred above the car during the countdown.
///
/// Shows the lights coming on one at a time, then all out for GO. The lamp
/// positions match a real arcade rig so the "wait for all three, then go" read
/// is unambiguous.
fn render_start_lights(phase: StartPhase) {
    let lit = match phase {
        StartPhase::Grid => 0,
        StartPhase::Lit(n) => n as u16,
        StartPhase::Go | StartPhase::Racing => return,
    };
    let spacing = 22i16;
    let first_x = 160 - spacing;
    let y = 44;
    psx_gpu_mod::draw_rect_flat(112, y - 6, 96, 22, 16, 18, 24);
    for i in 0..3u16 {
        let x = first_x + (i as i16) * spacing;
        let (r, g, b) = if i < lit { (255, 60, 40) } else { (58, 26, 24) };
        psx_gpu_mod::draw_rect_flat(x - 7, y - 2, 14, 14, r, g, b);
    }
}

/// Bottom-of-screen banner for the outcome of the last memory card operation.
fn render_card_notice(status: MemcardStatus) {
    let (text, colour) = match status {
        MemcardStatus::Saved => ("SAVED TO MEMORY CARD", (70, 220, 110)),
        MemcardStatus::Corrupt => ("SAVE DAMAGED - NEW PROFILE", (240, 170, 40)),
        MemcardStatus::WriteFailed => ("SAVE FAILED - CHECK CARD", (235, 60, 60)),
        // The manager only queues the three above; anything else draws nothing.
        _ => return,
    };
    let w = SCREEN_WIDTH - 24;
    let y = SCREEN_HEIGHT as i16 - 26;
    psx_gpu_mod::draw_rect_flat(12, y, w, 16, 12, 14, 20);
    draw_text(16, y as u16 + 4, text, colour, 1);
}

static mut GAME: Option<ArduracerGame> = None;

#[no_mangle]
fn main() -> ! {
    unsafe {
        let slot = core::ptr::addr_of_mut!(GAME);
        *slot = Some(ArduracerGame::new());
        // `GAME` is assigned unconditionally on the line above, so the `None`
        // arm is unreachable. Panicking is strictly better than the `loop {}`
        // this replaced: a silent hang on a console with no OS to kill it.
        match (*slot).as_mut() {
            Some(game) => game.run(),
            None => panic!("GAME was assigned above and cannot be None"),
        }
    }
}
