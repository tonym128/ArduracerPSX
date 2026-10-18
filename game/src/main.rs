//! Arduracer PSX - Bare-metal PlayStation 1 Entry Point.
//!
//! Locked 30 FPS overhead arcade racer built with PSoXide.

#![no_std]
#![no_main]
#![cfg_attr(target_arch = "mips", feature(asm_experimental_arch))]

extern crate psx_rt;

pub mod async_tile;
pub mod audio;
pub mod autopilot;
pub mod cd_fs;
pub mod dbg;
pub mod drive;
pub mod ghost_player;
pub mod ghost_recorder;
pub mod gpu;
pub mod input;
pub mod level_grid;
pub mod memcard;
pub mod prof;
pub mod state;
pub mod ui;
pub mod video;
use arduracer_core::levels::GameMode;
use arduracer_core::save::CdAudioMode;
use arduracer_core::{
    compute_standings, grid_slot_offset_on, grid_slot_position_on, resolve_field, AiRacer,
    ChampionshipSession, Difficulty, Fixed, LapTimer, StartPhase, StartSequence, TrackDef, Vec2,
    VehicleState, AI_PROFILES, ALL_TRACKS, ALL_TRACK_VISUALS,
};
use audio::AudioSystem;
use ghost_player::render_active_ghost;
use ghost_recorder::LapGhostRecorder;
use gpu::{
    bake_minimap, codec_is_uncompressed, init_track_texture, prewarm_starting_tiles, render_car,
    render_hud, render_replay_hud, render_track, reset_texture_caches, set_frame_period,
    set_texture_format, texture_format, Camera, DebugOverlay, Framerate, ParticleSystem,
    SkidmarkBuffer, TextureFormat, TextureSlot,
};
use input::{InputManager, InputProfile};
use memcard::{MemcardStatus, MemoryCardManager};
use psx_gpu::{self as psx_gpu_mod, framebuf::FrameBuffer, Resolution, VideoMode};
use state::{GameState, StageSelectorSource, StateManager};
use ui::font::draw_text;
use ui::{
    CityInfo, GrandPrixSetupAction, MenuItem, PauseChoice, PauseMenu, ResultsAction, ResultsScreen,
    SettingsChoice, SettingsInput, SettingsValues, StageFilterTab, StageTableAction,
    TrackEditorAction, VictoryScreen,
};

pub const SCREEN_WIDTH: u16 = 320;
pub const SCREEN_HEIGHT: u16 = 240;

/// Whether AI rivals take part in a race.
const RIVALS_ENABLED: bool = true;

#[cfg(target_arch = "mips")]
#[inline]
fn is_pal_mode() -> bool {
    psx_io::gpu::gpustat().contains(psx_hw::gpu::GpuStat::VMODE_PAL)
}

#[cfg(not(target_arch = "mips"))]
#[inline]
fn is_pal_mode() -> bool {
    false
}

// VRAM upload-path counters live in `main` so the heartbeat can report them
// without reaching into the texture module's statics. Diagnostic only: a lost
// increment would misreport a number, never alter a transfer.
#[cfg_attr(target_arch = "mips", link_section = ".data")]
static mut UPLOAD_DMA_COUNT: u32 = 0;
#[cfg_attr(target_arch = "mips", link_section = ".data")]
static mut UPLOAD_FIFO_COUNT: u32 = 0;

/// VRAM uploads taken by each path, as `(via_ch2_dma, via_gp0_fifo)`.
pub fn upload_path_counts() -> (u32, u32) {
    // SAFETY: written only by the texture module's upload paths, read by the
    // main loop's heartbeat; single-core, and no IRQ touches these.
    unsafe { (UPLOAD_DMA_COUNT, UPLOAD_FIFO_COUNT) }
}

/// Record which path a VRAM upload took. Called once per tile, by whichever
/// upload path (`upload_tile_dma` for TIM, `upload_tile_dma4` for TIM4) moved it.
pub fn note_upload_path(via_dma: bool) {
    // SAFETY: as `upload_path_counts`. Plain `u32` rather than atomics because
    // this target has `max-atomic-width = 0` and these are diagnostic totals.
    unsafe {
        if via_dma {
            UPLOAD_DMA_COUNT = UPLOAD_DMA_COUNT.wrapping_add(1);
        } else {
            UPLOAD_FIFO_COUNT = UPLOAD_FIFO_COUNT.wrapping_add(1);
        }
    }
}

/// One-shot markers: each DMA channel's fallback decision is logged at most
/// once, independently of the other channel.
///
/// Plain `static mut`, not atomic: `mipsel-sony-psx` has `max-atomic-width = 0`,
/// so `core::sync::atomic` offers nothing on the device. A byte load/store is
/// all the synchronisation needed -- the R3000 is single-core and the only
/// other thread of control (the VBlank IRQ) never touches these flags.
#[cfg_attr(target_arch = "mips", link_section = ".data")]
static mut SILICON_REPORTED: bool = false;
#[cfg_attr(target_arch = "mips", link_section = ".data")]
static mut CD_SILICON_REPORTED: bool = false;

/// Compiling more than one format pin would silently pick one.
#[cfg(any(
    all(feature = "profile-tim", feature = "profile-tim4"),
    all(feature = "profile-tim", feature = "profile-tim8"),
    all(feature = "profile-tim", feature = "profile-qoi"),
    all(feature = "profile-tim4", feature = "profile-tim8"),
    all(feature = "profile-tim4", feature = "profile-qoi"),
    all(feature = "profile-tim8", feature = "profile-qoi")
))]
compile_error!("enable exactly one of profile-tim / profile-tim4 / profile-tim8 / profile-qoi");

/// The texture format a headless comparison run is pinned to, if any.
///
/// The format normally comes from the memory card, so a headless run always
/// boots at `AUTO` -> `SVG`, and the TIM/TIM4 DMA upload paths never execute.
/// That made a format comparison impossible without a save file authored for
/// each case, so the `profile-tim*` features pin it at build time instead.
///
/// Pinning is a *build* choice rather than a runtime one on purpose: it costs
/// the shipped binary nothing, it cannot be left switched on by a stray runtime
/// flag, and a comparison run is reproducible from its feature list alone.
/// With no pin feature -- every build that can reach a player's console -- this
/// returns `saved` unchanged.
#[inline]
#[allow(unused_variables)]
fn headless_texture_format(saved: u8) -> TextureFormat {
    #[cfg(feature = "profile-tim")]
    {
        return TextureFormat::Tim;
    }
    #[cfg(feature = "profile-tim4")]
    {
        return TextureFormat::Tim4;
    }
    #[cfg(feature = "profile-tim8")]
    {
        return TextureFormat::Tim8;
    }
    #[cfg(feature = "profile-qoi")]
    {
        return TextureFormat::Qoi;
    }
    #[cfg(all(
        not(feature = "profile-tim"),
        not(feature = "profile-tim4"),
        not(feature = "profile-tim8"),
        not(feature = "profile-qoi")
    ))]
    {
        TextureFormat::from_saved(saved)
    }
}

/// Logs the one-shot silicon-class fallback decision for each channel, once
/// that channel's probe has decided, plus the live ch3 sector counts.
///
/// The counts are the part worth watching while racing: the sticky class says
/// the probe passed once, the counter says whether sectors are still going over
/// that channel now. A DMA count that stops climbing while PIO keeps climbing
/// is a silent regression the one-shot line would never show.
///
/// The two probes fire from different transfers and can land in either order:
/// ch2 on the first RAM->VRAM DMA (the intro FMV's column upload, or the first
/// TIM/TIM4 prewarm tile if the intro was skipped), ch3 on the first CD sector
/// read. The decisions are sticky in `psx_io::silicon`, so a channel is
/// reported whenever a later call observes it decided.
fn log_silicon_fallback_once() {
    let (gpu, cd) = psx_io::silicon::silicon_report();
    let gpu_decided = !matches!(
        gpu,
        psx_io::silicon::DmaClass::Unknown | psx_io::silicon::DmaClass::Disabled
    );
    let cd_decided = !matches!(cd, psx_io::silicon::DmaClass::Unknown);
    if !gpu_decided && !cd_decided {
        // No probe has run yet; a later transfer decides and this reports then.
        return;
    }
    // SAFETY: the game loop is the only caller and no IRQ touches these flags;
    // the byte read/write is atomic on the single-core R3000.
    let (log_gpu, log_cd) = unsafe {
        let gpu_first = !SILICON_REPORTED;
        if gpu_decided {
            SILICON_REPORTED = true;
        }
        let cd_first = !CD_SILICON_REPORTED;
        if cd_decided {
            CD_SILICON_REPORTED = true;
        }
        (gpu_first && gpu_decided, cd_first && cd_decided)
    };
    // The CD counters are printed on every call, not gated behind the
    // one-shot flags: they are live totals and the question they answer ("is
    // ch3 actually moving disc data, or has it gone quiet back to PIO?") is
    // worth answering repeatedly during a race.
    let (dma_sectors, pio_sectors) = psx_io::silicon::cd_sector_counts();
    if dma_sectors != 0 || pio_sectors != 0 {
        dbg::print("[SILICON]   ch3 sectors: dma=");
        dbg::print_dec(dma_sectors);
        dbg::print(" pio=");
        dbg::print_dec(pio_sectors);
        dbg::println("");
    }
    if log_gpu {
        dbg::println("[SILICON] DMA silicon probe (per-channel decision):");
        match gpu {
            psx_io::silicon::DmaClass::Works => {
                dbg::println(
                    "[SILICON]   ch2 (GPU RAM->VRAM): OK -> DMA used for track/FMV uploads",
                );
            }
            psx_io::silicon::DmaClass::Wedges => {
                dbg::println(
                    "[SILICON]   ch2 (GPU RAM->VRAM): CL1/CL2 wedge detected -> uploads fall back to GP0 FIFO (sticky)",
                );
            }
            _ => {}
        }
    }
    if log_cd {
        match cd {
            psx_io::silicon::DmaClass::Works => {
                dbg::println(
                    "[SILICON]   ch3 (CD-ROM->RAM): OK -> CD sectors read via ch3 DMA (free CPU)",
                );
            }
            psx_io::silicon::DmaClass::Wedges => {
                dbg::println(
                    "[SILICON]   ch3 (CD-ROM->RAM): CL1/CL2 wedge detected -> CD falls back to byte-FIFO PIO (sticky)",
                );
            }
            psx_io::silicon::DmaClass::Disabled => {
                dbg::println(
                    "[SILICON]   ch3 (CD-ROM->RAM): disabled by design -> CD sectors read via byte-FIFO PIO",
                );
            }
            psx_io::silicon::DmaClass::Unknown => {}
        }
    }
}

/// Spawns 5 AI rivals on staggered grid positions according to championship grid slots and difficulty.
///
/// The lateral stagger is scaled to the road by [`grid_offset`]: a three-lane
/// circuit gets the full two-column grid, a one-lane circuit gets a single
/// file, because a fixed ±16 on a one-lane road is not a stagger but two cars
/// parked on opposite kerbs.
fn spawn_rivals(track: &TrackDef, champ: Option<&ChampionshipSession>) -> [AiRacer; 5] {
    let start_pos = track.start_pos;
    let start_heading = track.start_heading;
    let diff = champ.map(|c| c.difficulty).unwrap_or(Difficulty::Medium);
    let mut rivals = [
        AiRacer::with_difficulty(start_pos, start_heading, AI_PROFILES[0], diff),
        AiRacer::with_difficulty(start_pos, start_heading, AI_PROFILES[1], diff),
        AiRacer::with_difficulty(start_pos, start_heading, AI_PROFILES[2], diff),
        AiRacer::with_difficulty(start_pos, start_heading, AI_PROFILES[3], diff),
        AiRacer::with_difficulty(start_pos, start_heading, AI_PROFILES[4], diff),
    ];
    for (i, rival) in rivals.iter_mut().enumerate() {
        let slot = champ.map(|c| c.grid_slot_of(i + 1)).unwrap_or(i + 1);
        let scaled = grid_slot_offset_on(track, slot);
        rival.offset_from_pole(start_pos, start_heading, scaled.forward, scaled.lateral);
        rival.aim_timer = (i as u8) % 3;
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
    pub fb: FrameBuffer,
    pub audio: AudioSystem,
    pub input_mgr: InputManager,
    pub state_mgr: StateManager,
    pub ghost: LapGhostRecorder,
    pub memcard: MemoryCardManager,
    /// Three lights, then GO. The lap clock arms on GO, not at load.
    pub start: StartSequence,
    /// Fixed-timestep accumulator for 60 Hz simulation updates.
    pub sim_accumulator: u32,
    /// Last sampled VBlank counter value for measuring elapsed display periods.
    pub last_vblank_count: u32,
    /// Monotonic count of simulation ticks executed.
    pub sim_tick_counter: u32,
    /// VBlank counter value of the last display flip applied (for 30 FPS pacing).
    pub last_flip_vblank: u32,
    /// Player-selected frame pacing. 30 FPS occupies two VBlank periods per
    /// frame, 60 occupies one; every pacing decision below reads it.
    pub framerate: Framerate,
    /// Cached race standings across simulation sub-ticks.
    pub last_standings: [usize; 6],
    /// Cached player rank (1..=6) across simulation sub-ticks.
    pub last_player_rank: u8,
    /// 0 = Behind (Mode 7 Chase), 1 = Overhead (Top-down).
    pub camera_view: u8,
    /// 0 = Off, 1 = Simple FPS, 2 = Simple Graphing FPS, 3 = All Metrics.
    pub debug_metrics: u8,
    /// Live performance telemetry and rolling graph.
    pub debug_overlay: DebugOverlay,
    /// Edge detection state for racing gameplay buttons (e.g. Triangle view toggle).
    pub prev_pad_racing: psx_pad::ButtonState,
    /// Playback tick counter for Post-Race Action Replay Theater (`NO1-027`).
    pub replay_tick: u32,
    /// Active camera angle in Action Replay Theater (`0 = Broadcast TV, 1 = Chase, 2 = Helicopter`).
    pub replay_camera_mode: u8,
    /// 8-frame arcade screen transition overlay (`NO1-036`).
    pub transition: gpu::ScreenTransition,
    /// 2-Player Head-to-Head Split-Screen Mode (`NO1-045`).
    pub is_2p_mode: bool,
    /// Player 2 vehicle dynamics and telemetry state.
    pub player2: VehicleState,
    /// Player 2 lap timing and split tracking.
    pub player2_timer: LapTimer<16>,
    /// Player 2 chase camera.
    pub camera2: Camera,
    /// Player 2 controller and force feedback manager (Controller Port 2).
    pub input_mgr2: InputManager,
    /// True if Player 1 won the 2P duel; False if Player 2 won.
    pub p1_won_2p: bool,
    /// Whether the game is currently running a live AI autopilot attract demo race.
    pub is_attract_demo: bool,
    /// Frame counter for the 30-second attract demo race.
    pub attract_demo_timer: u32,
    /// Attract mode alternating cycle counter (even = FMV, odd = Live AI Demo).
    pub attract_cycle: u8,
    /// Synthetic AI autopilot driver used for attract mode demos and profiling.
    pub autopilot: autopilot::Autopilot,
    /// 2-Player Split-Screen layout mode: 0 = Horizontal, 1 = Vertical, 2 = Diagonal.
    pub split_mode: u8,
    /// Finish slowdown countdown ticks after crossing the finish line.
    pub finish_slowdown: u16,
    /// Whether the race completion has been recorded and slowdown initiated.
    pub finish_recorded: bool,
    /// Edge detection state for controller port 2 buttons.
    pub prev_pad2_racing: psx_pad::ButtonState,
}

impl Default for ArduracerGame {
    fn default() -> Self {
        Self::new()
    }
}

impl ArduracerGame {
    pub fn new() -> Self {
        let video_mode = if is_pal_mode() {
            dbg::log_step(1, 9, "Initializing GPU (320x240 PAL)...");
            VideoMode::Pal
        } else {
            dbg::log_step(1, 9, "Initializing GPU (320x240 NTSC)...");
            VideoMode::Ntsc
        };
        psx_gpu_mod::init(video_mode, Resolution::R320X240);
        crate::ui::font::init_vram_font();
        // The intro runs before AudioSystem loads the soundbank. Initialize
        // SPU here once; later victory movies must not reset live game voices.
        psx_spu::init();
        dbg::check_faults();

        dbg::log_step(2, 9, "Preparing SPU soundbank...");
        dbg::check_faults();

        dbg::log_step(3, 9, "Playing attract FMV (CD-streamed XA audio)...");
        // A profiling run wants frames on circuit, and the attract movie is
        // a streamed double-speed XA file. Skipping
        // it means the report windows are all racing frames.
        #[cfg(feature = "profiling")]
        let intro_res = video::VideoResult::Skipped;
        #[cfg(not(feature = "profiling"))]
        let intro_res = video::play_video("INTRO.STR");
        dbg::check_faults();

        dbg::log_step(4, 9, "Finishing intro FMV stream...");
        dbg::check_faults();
        // The FMV has just run a full CD sector stream (and, if it played, the
        // GPU column DMA), so both channel probes have had their chance by now.
        // Report here rather than waiting for the first track load: on a normal
        // boot that is several menu screens away.
        log_silicon_fallback_once();

        dbg::log_step(
            5,
            9,
            "Initializing track texture streaming & locating TRACKS.BIN...",
        );
        init_track_texture(ALL_TRACK_VISUALS[0]);
        gpu::load_level_grid(ALL_TRACK_VISUALS[0], ALL_TRACKS[0]);
        dbg::check_faults();

        let fb = FrameBuffer::new(SCREEN_WIDTH, SCREEN_HEIGHT);
        psx_gpu_mod::set_draw_area(0, 0, SCREEN_WIDTH - 1, SCREEN_HEIGHT - 1);
        psx_gpu_mod::set_draw_offset(0, 0);

        let track = ALL_TRACKS[0];
        let timer = LapTimer::new(track.checkpoint_slice(), track.start_gate);
        let player = VehicleState::new(track.start_pos, track.start_heading, Default::default());
        let rivals = spawn_rivals(track, None);
        let mut camera = Camera::new_chase(track.start_pos, track.start_heading);
        camera.set_extended_view_distance(!codec_is_uncompressed(ALL_TRACK_VISUALS[0]));

        dbg::log_step(6, 9, "Probing memory card...");
        let mut memcard = MemoryCardManager::new();
        memcard.probe();
        dbg::check_faults();

        dbg::log_step(
            7,
            9,
            "Initializing audio subsystem and cueing CD-DA Track 2...",
        );
        let mut audio = AudioSystem::new();
        // The mode has to be set before the first `play_music`, or the title
        // screen plays CD-DA regardless of what the player chose.
        let cd_audio = memcard.save_data.cd_audio;
        audio.set_cd_audio(cd_audio);
        audio.set_sound_volume(memcard.save_data.sound_volume);
        audio.set_music_volume(memcard.save_data.music_volume);
        audio.play_music(2, crate::gpu::tracktex::disc_reader());
        dbg::check_faults();

        dbg::log_step(8, 9, "Initializing DualShock analog controller manager...");
        let mut input_mgr = InputManager::new(InputProfile::ClassicArcade);
        input_mgr.init();
        let mut input_mgr2 = InputManager::new_port2(InputProfile::ClassicArcade);
        input_mgr2.init();
        let player2 = VehicleState::new(track.start_pos, track.start_heading, Default::default());
        let player2_timer = LapTimer::new(track.checkpoint_slice(), track.start_gate);
        let camera2 = Camera::new_chase(track.start_pos, track.start_heading);
        dbg::check_faults();

        let mut state_mgr = StateManager::new();
        if intro_res == video::VideoResult::Completed {
            state_mgr.title.start_white_fade();
        }
        if intro_res == video::VideoResult::Skipped {
            state_mgr.current = GameState::MainMenu;
            dbg::println("[STATE] Intro skipped -> starting at MainMenu");
        } else {
            dbg::println("[STATE] Intro completed/idle -> starting at Title");
        }

        let mut game = ArduracerGame {
            frame_counter: 0,
            player,
            rivals,
            camera,
            particles: ParticleSystem::new(),
            skidmarks: SkidmarkBuffer::new(),
            timer,
            pause: PauseMenu::new(),
            paused: false,
            minimap_texture: TextureSlot::new(crate::gpu::texlayout::MINIMAP_INNER as u16),
            show_hud: true,
            current_track: track,
            current_track_idx: 0,
            fb,
            audio,
            input_mgr,
            state_mgr,
            ghost: LapGhostRecorder::new(0),
            start: StartSequence::new(),
            sim_accumulator: 0,
            last_vblank_count: 0,
            sim_tick_counter: 0,
            last_flip_vblank: 0,
            framerate: Framerate::Fps60,
            last_standings: [0, 1, 2, 3, 4, 5],
            last_player_rank: 1,
            camera_view: 0,
            debug_metrics: 0,
            debug_overlay: DebugOverlay::new(),
            prev_pad_racing: psx_pad::ButtonState::from_bits(0),
            replay_tick: 0,
            replay_camera_mode: 0,
            transition: gpu::ScreenTransition::new(),
            is_2p_mode: false,
            player2,
            player2_timer,
            camera2,
            input_mgr2,
            p1_won_2p: false,
            memcard,
            is_attract_demo: false,
            attract_demo_timer: 0,
            attract_cycle: 0,
            autopilot: autopilot::Autopilot::new(track),
            split_mode: 0,
            finish_slowdown: 0,
            finish_recorded: false,
            prev_pad2_racing: psx_pad::ButtonState::from_bits(0),
        };
        // Restore the saved graphics settings before anything renders, so the
        // first frame already uses the player's chosen format and pacing rather
        // than the defaults for a frame or two.
        {
            let saved = SettingsValues {
                texture_format: game.memcard.save_data.texture_format,
                framerate: game.memcard.save_data.framerate,
                camera_view: game.camera_view,
                cd_audio: game.memcard.save_data.cd_audio,
                debug_metrics: game.debug_metrics,
                input_profile: game.memcard.save_data.input_profile,
                split_mode: game.split_mode,
            };
            game.input_mgr.controller.profile = match saved.input_profile {
                1 => InputProfile::ModernTriggers,
                2 => InputProfile::DualAnalog,
                _ => InputProfile::ClassicArcade,
            };
            game.input_mgr2.controller.profile = game.input_mgr.controller.profile;
            set_texture_format(headless_texture_format(saved.texture_format));
            game.framerate = Framerate::from_saved(saved.framerate);
            set_frame_period(game.framerate.vblanks_per_frame());
            prof::set_budget_cycles(game.framerate.budget_cycles());
            prof::set_expected_vblanks(game.framerate.vblanks_per_frame());
            // Applied before anything asks for a track: with TIM pinned and the
            // music left spinning, every tile read would hand the drive back to
            // CD-DA and hand it straight back, for the whole boot.
            let want_cdda = game.memcard.save_data.cd_audio_mode() != CdAudioMode::Xa
                && game.memcard.save_data.cd_audio_mode().enabled();
            game.audio.cdda.set_enabled(want_cdda);
            // Who yields to whom for the drive. The mode itself was already
            // applied to the audio system above, before the first play_music.
            crate::drive::install_preempt(crate::audio::xa::player_suspend);
            dbg::print("[BOOT] graphics: format=");
            dbg::print(texture_format().label());
            dbg::print(" framerate=");
            dbg::print(game.framerate.label());
            dbg::print(" cd-audio=");
            dbg::println(match game.memcard.save_data.cd_audio {
                2 => "xa",
                1 => "cdda",
                _ => "off",
            });
            game.sync_camera_view_distance();
        }

        dbg::log_step(9, 9, "Baking circuit minimap...");
        bake_minimap(&game.minimap_texture, game.current_track);
        dbg::check_faults();
        dbg::println("[BOOT] All 9 initialization steps completed!");
        game
    }

    /// Returns the active visual circuit / level ID.
    #[inline]
    pub fn active_circuit(&self) -> usize {
        if self.current_track_idx >= ALL_TRACKS.len() {
            self.current_track_idx
        } else {
            ALL_TRACK_VISUALS[self.current_track_idx.min(ALL_TRACK_VISUALS.len() - 1)]
        }
    }

    /// Synchronizes the camera view distance with the active track format:
    /// compressed formats (SVG / ARSV, QOI, JPEG) extend view distance by +2 tiles
    /// (560 depth), while uncompressed formats (TIM, TIM4, TIM8) stream from CD
    /// with the standard 240 depth to protect CD-DA audio bandwidth.
    #[inline]
    pub fn sync_camera_view_distance(&mut self) {
        let extended = !codec_is_uncompressed(self.active_circuit());
        self.camera.set_extended_view_distance(extended);
        // Player 2's split-screen viewport (`NO1-045`) shares the same codec.
        self.camera2.set_extended_view_distance(extended);
    }

    /// Loads and resets the active circuit.
    pub fn load_track(&mut self, track_idx: usize) {
        let idx = track_idx % ALL_TRACKS.len();
        let track_name = ALL_TRACKS[idx].name;
        crate::gpu::hud_renderer::render_loading_screen(track_name, "TRACKMANIA AUTHOR", 35);
        let flip = self.fb.begin_deferred_swap();
        psx_rt::interrupts::queue_gp1_at_vblank(flip);
        #[cfg(target_arch = "mips")]
        psx_rt::interrupts::wait_vblank();

        self.current_track_idx = idx;
        self.current_track = ALL_TRACKS[idx];
        self.ghost.reset(idx as u8);
        self.ghost
            .load_from_card_block(&self.memcard.block, idx as u8);
        self.reset_race();
        self.audio.cdda.stop();
        init_track_texture(ALL_TRACK_VISUALS[idx]);
        // After `init_track_texture`: that is what locates `LEVELS.BIN`. Loading
        // first finds no asset, leaves every tile `Barrier`, and the AI has no
        // road to follow -- the car sits on the grid doing nothing.
        gpu::load_level_grid(ALL_TRACK_VISUALS[idx], ALL_TRACKS[idx]);
        // Re-bake the minimap for the new circuit once the level grid is resident in RAM.
        // Once per load: the image is static for the whole race, so this must not be per-frame (TASK-1202).
        bake_minimap(&self.minimap_texture, self.current_track);
        prewarm_starting_tiles(
            self.current_track.world_width(),
            self.current_track.world_height(),
            &self.camera,
        );
        log_silicon_fallback_once();

        // Play the CD-DA theme corresponding to the active cup:
        // Cup 1 (Tracks 1-6) -> CD-DA Track 3 ("Asphalt Adrenaline" Eurobeat)
        // Cup 2 (Tracks 7-12) -> CD-DA Track 4 ("Night Drift City" D&B)
        // Cup 3 (Tracks 13-18) -> CD-DA Track 5 ("Canyon Rush" Techno)
        // Cup 4 (Tracks 19-24) -> CD-DA Track 6 ("Apex Predator" Trance)
        let cdda_track = 3 + ((idx / 6) as u8).min(3);
        self.audio
            .play_music(cdda_track, crate::gpu::tracktex::disc_reader());
        // `Start` confirms through the title, main menu and track select, so a
        // player can load a stage still holding it. Without this the first
        // racing frame reads that as a fresh press and the pause veil opens
        // itself with the music stopped (TASK-1209).
        self.pause.arm_for_race_start();
        // A stage loaded: the engine synth may sound again. Still gated on
        // `!paused`, and `reset_race` below clears the veil, so this cannot arm
        // the engine while the pause menu is up.
        self.audio.enter_race();
        self.transition
            .start_wipe_in(crate::gpu::TransitionKind::VenetianBlinds);
    }

    /// Loads a 10 km^2 city free roam map seamlessly.
    pub fn load_city_freeroam(&mut self, city: &'static CityInfo) {
        let track_name = city.track.name;
        crate::gpu::hud_renderer::render_loading_screen(track_name, "FREE ROAM METROPOLIS", 35);
        let flip = self.fb.begin_deferred_swap();
        psx_rt::interrupts::queue_gp1_at_vblank(flip);
        #[cfg(target_arch = "mips")]
        psx_rt::interrupts::wait_vblank();

        self.current_track_idx = city.track_id;
        self.current_track = city.track;
        self.ghost.reset(city.track_id as u8);
        self.reset_race();
        self.audio.cdda.stop();
        init_track_texture(city.track_id);
        gpu::load_level_grid(city.track_id, city.track);
        bake_minimap(&self.minimap_texture, self.current_track);
        prewarm_starting_tiles(
            self.current_track.world_width(),
            self.current_track.world_height(),
            &self.camera,
        );
        log_silicon_fallback_once();

        // CD-DA Track 4: "Coastal Drive" D&B - high tempo street cruise
        self.audio
            .play_music(4, crate::gpu::tracktex::disc_reader());
        self.pause.arm_for_race_start();
        self.audio.enter_race();
        self.transition
            .start_wipe_in(crate::gpu::TransitionKind::VenetianBlinds);
    }

    /// Loads a custom user-built TrackMania circuit for test drive and validation.
    pub fn load_custom_track(
        &mut self,
        custom_def: &arduracer_core::track_builder::CustomTrackDef,
    ) {
        crate::gpu::hud_renderer::render_loading_screen(
            "CUSTOM TRACK",
            "TRACK BUILDER TEST DRIVE",
            35,
        );
        let flip = self.fb.begin_deferred_swap();
        psx_rt::interrupts::queue_gp1_at_vblank(flip);
        #[cfg(target_arch = "mips")]
        psx_rt::interrupts::wait_vblank();

        if let Ok(bound_track) = crate::level_grid::compile_and_bind_custom_track(custom_def) {
            self.current_track = bound_track;
            self.current_track_idx = 999;
            self.ghost.reset(255);
            self.reset_race();
            self.audio.cdda.stop();
            init_track_texture(0);
            bake_minimap(&self.minimap_texture, self.current_track);
            prewarm_starting_tiles(
                self.current_track.world_width(),
                self.current_track.world_height(),
                &self.camera,
            );
            log_silicon_fallback_once();

            self.audio
                .play_music(6, crate::gpu::tracktex::disc_reader());
            self.pause.arm_for_race_start();
            self.audio.enter_race();
            self.transition
                .start_wipe_in(crate::gpu::TransitionKind::VenetianBlinds);
            self.state_mgr.current = GameState::Racing;
        } else {
            self.state_mgr
                .track_editor
                .set_status("COMPILATION FAILED!");
        }
    }

    /// Resets the current race state to the starting grid.
    pub fn reset_race(&mut self) {
        let track = self.current_track;
        let is_mania_sprint = (self.state_mgr.championship.is_none()
            && self.state_mgr.track_select.mode == GameMode::Mania
            && self.current_track_idx >= 24)
            || self.current_track_idx == 999;
        self.timer =
            LapTimer::with_par_times(track.checkpoint_slice(), track.start_gate, track.par_times)
                .with_target_laps(if is_mania_sprint {
                    1
                } else {
                    arduracer_core::TOTAL_LAPS
                });
        let player_slot = self
            .state_mgr
            .championship
            .as_ref()
            .map(|c| c.grid_slot_of(0))
            .unwrap_or(0);
        let player_pos = grid_slot_position_on(track, player_slot);
        self.player = VehicleState::new(
            player_pos,
            track.start_heading,
            self.state_mgr.garage.tuning(),
        );
        self.rivals = spawn_rivals(track, self.state_mgr.championship.as_ref());
        if self.is_2p_mode {
            let p1_pos = grid_slot_position_on(track, 0);
            let p2_pos = grid_slot_position_on(track, 1);
            self.player =
                VehicleState::new(p1_pos, track.start_heading, self.state_mgr.garage.tuning());
            self.player2 =
                VehicleState::new(p2_pos, track.start_heading, self.state_mgr.garage.tuning());
            self.player2_timer = LapTimer::with_par_times(
                track.checkpoint_slice(),
                track.start_gate,
                track.par_times,
            )
            .with_target_laps(if is_mania_sprint {
                1
            } else {
                arduracer_core::TOTAL_LAPS
            });
            self.p1_won_2p = false;
            self.camera = match self.camera_view {
                0 => Camera::new_chase(p1_pos, track.start_heading),
                2 => Camera::new_rotating(p1_pos, track.start_heading),
                _ => Camera::new(p1_pos),
            };
            self.camera.set_split_mode(self.split_mode, 1);
            self.camera2 = match self.camera_view {
                0 => Camera::new_chase(p2_pos, track.start_heading),
                2 => Camera::new_rotating(p2_pos, track.start_heading),
                _ => Camera::new(p2_pos),
            };
            self.camera2.set_split_mode(self.split_mode, 2);
        } else {
            self.camera = match self.camera_view {
                0 => Camera::new_chase(player_pos, track.start_heading),
                2 => Camera::new_rotating(player_pos, track.start_heading),
                _ => Camera::new(player_pos),
            };
            self.camera.set_split_mode(0, 0);
            self.camera2.set_split_mode(0, 0);
        }
        self.sync_camera_view_distance();
        self.particles = ParticleSystem::new();
        self.skidmarks = SkidmarkBuffer::new();
        self.ghost.start_lap();
        // The clock is deliberately NOT armed here: it starts on GO. See
        // `StartSequence`.
        self.start = StartSequence::new();
        self.paused = false;
        self.show_hud = true;
        self.finish_slowdown = 0;
        self.finish_recorded = false;
        self.sim_accumulator = 0;
        self.last_vblank_count = psx_rt::interrupts::vblank_count();
        self.sim_tick_counter = 0;
        self.last_flip_vblank = psx_rt::interrupts::vblank_count();
        self.last_standings = [0, 1, 2, 3, 4, 5];
        self.last_player_rank = 1;
    }

    /// Returns to the originating menu or trial stage selector after leaving a race.
    pub fn return_from_race(&mut self, pad_buttons: u16) {
        self.paused = false;
        self.audio.set_paused(false);
        self.audio.leave_race();
        self.audio.sync_ui_edges(pad_buttons);
        self.audio
            .play_music(2, crate::gpu::tracktex::disc_reader());

        if self.current_track_idx == 999 {
            self.state_mgr.stage_selector_source = StageSelectorSource::None;
            self.state_mgr.track_editor.arm_for_entry();
            self.state_mgr.current = GameState::TrackEditor;
        } else if self.current_track_idx >= ALL_TRACKS.len() {
            self.state_mgr.stage_selector_source = StageSelectorSource::None;
            self.state_mgr.city_select.arm_for_entry();
            self.state_mgr.current = GameState::CitySelect;
        } else if self.state_mgr.championship.is_some() {
            self.state_mgr.championship = None;
            self.state_mgr.champ_stage_backup = None;
            self.state_mgr.stage_selector_source = StageSelectorSource::None;
            self.state_mgr.current = GameState::MainMenu;
            self.state_mgr.menu.arm_for_entry();
        } else if self.is_2p_mode {
            self.state_mgr.track_select.arm_for_entry_with_track(
                GameMode::GrandPrix,
                false,
                self.current_track_idx,
            );
            self.state_mgr.current = GameState::TrackSelect;
        } else {
            // Single-player trial stage: return to the trial stage selector (Carousel or Table)
            // preserving the current stage index.
            match self.state_mgr.stage_selector_source {
                StageSelectorSource::StageTable => {
                    let tab = if self.state_mgr.stage_table.tab == StageFilterTab::All {
                        StageFilterTab::All
                    } else if self.current_track_idx >= 25 {
                        StageFilterTab::Mania
                    } else {
                        StageFilterTab::GrandPrix
                    };
                    self.state_mgr
                        .stage_table
                        .arm_for_entry_with_track(tab, self.current_track_idx);
                    self.state_mgr.current = GameState::StageTable;
                }
                _ => {
                    let mode = if self.current_track_idx >= 25 {
                        GameMode::Mania
                    } else {
                        GameMode::GrandPrix
                    };
                    self.state_mgr.track_select.arm_for_entry_with_track(
                        mode,
                        false,
                        self.current_track_idx,
                    );
                    self.state_mgr.current = GameState::TrackSelect;
                }
            }
        }
    }

    /// Draws all active vehicles (player + 5 rivals) back-to-front by screen row.
    ///
    /// Sorting all 6 cars by projected screen `y` gives a true painter's order
    /// so cars never interpenetrate or draw out of order.
    fn render_cars_sorted(&self, player_color: (u8, u8, u8)) {
        let is_freeroam = self.current_track.checkpoint_count == 0;
        if !RIVALS_ENABLED || is_freeroam {
            render_car(&self.player, &self.camera, false, player_color);
            return;
        }
        // Insertion sort by depth across all 6 cars. 0 = player, 1..=5 = rivals.
        let mut order = [0usize, 1, 2, 3, 4, 5];
        let mut depth = [0i16; 6];
        depth[0] = self.camera.vehicle_screen_y(self.player.position);
        for (i, rival) in self.rivals.iter().enumerate() {
            depth[i + 1] = self.camera.vehicle_screen_y(rival.state.position);
        }
        for i in 1..order.len() {
            let mut j = i;
            while j > 0 && depth[order[j - 1]] > depth[order[j]] {
                order.swap(j - 1, j);
                j -= 1;
            }
        }
        for idx in order {
            if idx == 0 {
                render_car(&self.player, &self.camera, false, player_color);
            } else {
                let rival = &self.rivals[idx - 1];
                render_car(&rival.state, &self.camera, false, rival.profile.color);
            }
        }
    }

    /// Draws the two 2P cars back-to-front through `camera`, far one first.
    ///
    /// Each split viewport has to sort for itself: the depth order that is
    /// correct looking out of player 1's camera is the *reverse* of the one
    /// correct looking out of player 2's whenever the two cars swap places.
    /// Drawing them in a fixed player-1-then-player-2 order -- which is what the
    /// 2P render path did -- makes the nearer car vanish behind the further one
    /// exactly when they are level with each other.
    fn render_2p_cars_sorted(&self, camera: &Camera) {
        const P1_COLOUR: (u8, u8, u8) = (220, 25, 45);
        const P2_COLOUR: (u8, u8, u8) = (30, 144, 255);

        let p1_y = camera.vehicle_screen_y(self.player.position);
        let p2_y = camera.vehicle_screen_y(self.player2.position);
        // Larger screen row is nearer the camera, so it draws last.
        if p1_y <= p2_y {
            render_car(&self.player2, camera, false, P2_COLOUR);
            render_car(&self.player, camera, false, P1_COLOUR);
        } else {
            render_car(&self.player, camera, false, P1_COLOUR);
            render_car(&self.player2, camera, false, P2_COLOUR);
        }
    }

    /// Repaints the last simulated race frame (used while paused).
    fn draw_frozen_race(&mut self) {
        self.sync_camera_view_distance();
        self.fb.clear(18, 20, 26);
        if !self.is_2p_mode {
            render_track(self.current_track, &self.camera, self.last_flip_vblank, 1);
            self.skidmarks.render(&self.camera);
            // Smoke belongs on top of the cars it came from (TASK-1206).
            self.particles.render(&self.camera);
            self.render_cars_sorted((220, 25, 45));
        } else {
            let target_y = self.fb.buffer_y(self.fb.drawing);
            match self.split_mode {
                1 => {
                    // Vertical Split (Left P1, Right P2)
                    psx_gpu_mod::set_draw_area(0, target_y, 157, target_y + 239);
                    psx_gpu_mod::set_draw_offset(0, target_y as i16);
                    render_track(self.current_track, &self.camera, self.last_flip_vblank, 1);
                    self.skidmarks.render(&self.camera);
                    self.particles.render(&self.camera);
                    self.render_2p_cars_sorted(&self.camera);

                    // Vertical Divider Bar (X=158..161)
                    psx_gpu_mod::set_draw_area(0, target_y, 319, target_y + 239);
                    psx_gpu_mod::set_draw_offset(0, target_y as i16);
                    psx_gpu_mod::draw_rect_flat(158, 0, 4, 240, 15, 18, 25);
                    psx_gpu_mod::draw_line_mono(158, 0, 158, 239, 255, 215, 0);
                    psx_gpu_mod::draw_line_mono(161, 0, 161, 239, 255, 215, 0);
                    for i in 0..12 {
                        let y = (i * 20) as i16;
                        let (r, g, b) = if (i % 2) == 0 {
                            (255, 255, 255)
                        } else {
                            (30, 30, 30)
                        };
                        psx_gpu_mod::draw_rect_flat(159, y, 2, 10, r, g, b);
                    }

                    // P2 Right Viewport
                    psx_gpu_mod::set_draw_area(162, target_y, 319, target_y + 239);
                    psx_gpu_mod::set_draw_offset(162, target_y as i16);
                    render_track(self.current_track, &self.camera2, self.last_flip_vblank, 1);
                    self.skidmarks.render(&self.camera2);
                    self.particles.render(&self.camera2);
                    self.render_2p_cars_sorted(&self.camera2);

                    psx_gpu_mod::set_draw_area(0, target_y, 319, target_y + 239);
                    psx_gpu_mod::set_draw_offset(0, target_y as i16);
                }
                2 => {
                    // Diagonal Split (Top-Left P1, Bottom-Right P2) via Hardware Mask Bit
                    psx_gpu_mod::set_draw_area(0, target_y, 319, target_y + 239);
                    psx_gpu_mod::set_draw_offset(0, target_y as i16);

                    // Step A: Set bit 15 on P2 triangle
                    psx_gpu_mod::set_mask_mode(true, false);
                    psx_gpu_mod::draw_tri_flat([(0, 240), (320, 0), (320, 240)], 0, 0, 0);

                    // Step B: Render P1 (skip if bit 15 == 1, tag drawn pixels with bit 15 = 1)
                    psx_gpu_mod::set_mask_mode(true, true);
                    render_track(self.current_track, &self.camera, self.last_flip_vblank, 1);
                    self.skidmarks.render(&self.camera);
                    self.particles.render(&self.camera);
                    self.render_2p_cars_sorted(&self.camera);

                    // Step C: Clear P2 triangle with background color (resets P2 pixels to bit 15 = 0)
                    psx_gpu_mod::set_mask_mode(false, false);
                    psx_gpu_mod::draw_tri_flat([(0, 240), (320, 0), (320, 240)], 18, 20, 26);

                    // Step D: Render P2 (skip if bit 15 == 1, draw into P2 region)
                    psx_gpu_mod::set_mask_mode(false, true);
                    render_track(self.current_track, &self.camera2, self.last_flip_vblank, 1);
                    self.skidmarks.render(&self.camera2);
                    self.particles.render(&self.camera2);
                    self.render_2p_cars_sorted(&self.camera2);

                    // Step E: Reset mask mode and draw diagonal checkered divider line
                    psx_gpu_mod::set_mask_mode(false, false);
                    psx_gpu_mod::draw_line_mono(318, 0, 0, 238, 255, 215, 0);
                    psx_gpu_mod::draw_line_mono(321, 0, 1, 241, 255, 215, 0);
                    psx_gpu_mod::draw_line_mono(319, 0, 0, 239, 15, 18, 25);
                    psx_gpu_mod::draw_line_mono(320, 0, 1, 240, 15, 18, 25);
                    for i in 0..16 {
                        let x0 = 320 - (i * 20) as i16;
                        let y0 = (i * 15) as i16;
                        let x1 = 320 - ((i + 1) * 20) as i16;
                        let y1 = ((i + 1) * 15) as i16;
                        let (r, g, b) = if (i % 2) == 0 {
                            (255, 255, 255)
                        } else {
                            (30, 30, 30)
                        };
                        psx_gpu_mod::draw_line_mono(x0, y0, x1, y1, r, g, b);
                    }
                }
                _ => {
                    // Horizontal Split (Top P1, Bottom P2)
                    // P1 Top Viewport
                    psx_gpu_mod::set_draw_area(0, target_y, 319, target_y + 117);
                    psx_gpu_mod::set_draw_offset(0, target_y as i16);
                    render_track(self.current_track, &self.camera, self.last_flip_vblank, 1);
                    self.skidmarks.render(&self.camera);
                    self.particles.render(&self.camera);
                    self.render_2p_cars_sorted(&self.camera);

                    // Divider Bar
                    psx_gpu_mod::set_draw_area(0, target_y, 319, target_y + 239);
                    psx_gpu_mod::set_draw_offset(0, target_y as i16);
                    psx_gpu_mod::draw_rect_flat(0, 118, 320, 4, 15, 18, 25);
                    psx_gpu_mod::draw_line_mono(0, 118, 319, 118, 255, 215, 0);
                    psx_gpu_mod::draw_line_mono(0, 121, 319, 121, 255, 215, 0);
                    for i in 0..16 {
                        let x = (i * 20) as i16;
                        let (r, g, b) = if (i % 2) == 0 {
                            (255, 255, 255)
                        } else {
                            (30, 30, 30)
                        };
                        psx_gpu_mod::draw_rect_flat(x, 119, 10, 2, r, g, b);
                    }

                    // P2 Bottom Viewport
                    psx_gpu_mod::set_draw_area(0, target_y + 122, 319, target_y + 239);
                    psx_gpu_mod::set_draw_offset(0, (target_y + 122) as i16);
                    render_track(self.current_track, &self.camera2, self.last_flip_vblank, 1);
                    self.skidmarks.render(&self.camera2);
                    self.particles.render(&self.camera2);
                    self.render_2p_cars_sorted(&self.camera2);

                    // Restore full viewport
                    psx_gpu_mod::set_draw_area(0, target_y, 319, target_y + 239);
                    psx_gpu_mod::set_draw_offset(0, target_y as i16);
                }
            }
        }
    }

    pub fn run(&mut self) -> ! {
        // The queued display flip is applied by the VBlank handler, which
        // requires this counter (TASK-1201).
        psx_rt::interrupts::install_vblank_counter();
        // Configure hardware root counter 2 to tick at system_clock / 8 (4.2336 MHz).
        psx_io::timers::set_mode(psx_io::timers::Timer::Timer2, 0x0200);
        dbg::println("[LOOP] Entering main game loop with VBlank interrupt installed.");
        dbg::check_faults();

        // A profiling run has no pad, so the title/menu/track-select chain is
        // unreachable: nothing ever presses Cross. Jump straight into the race
        // on circuit 0 so every report window covers racing frames. Loading
        // through `load_track` rather than poking the state field keeps the
        // minimap bake, CD-DA cue and pause-arming identical to a real entry.
        #[cfg(feature = "profiling")]
        {
            self.load_track(0);
            self.state_mgr.current = GameState::Racing;
            dbg::println("[PROF] Profiling build: entering Racing on circuit 0");
        }

        let mut first_frame = true;
        loop {
            // Latch the frame boundary before any timed work, so a dropped
            // VBlank period is attributed to the frame that lost it.
            prof::begin_frame();

            // 1. Synchronize to 30 FPS display flip (wait until the queued flip is applied at VBlank).
            //    The handler has now applied last frame's queued flip, so program the draw target
            //    for the buffer we are about to fill.
            {
                let _s = prof::span(prof::VBLANK);
                #[cfg(target_arch = "mips")]
                {
                    if first_frame {
                        psx_rt::interrupts::wait_vblank_with(|| {
                            crate::gpu::tracktex::pump_cd_streams();
                        });
                        self.last_flip_vblank = psx_rt::interrupts::vblank_count();
                    } else {
                        let mut wait_spins = 0u32;
                        while psx_rt::interrupts::gp1_queue_pending() && wait_spins < 8 {
                            psx_rt::interrupts::wait_vblank_with(|| {
                                crate::gpu::tracktex::pump_cd_streams();
                            });
                            wait_spins += 1;
                        }
                        if psx_rt::interrupts::gp1_queue_pending() {
                            let word = psx_rt::interrupts::take_pending_gp1();
                            if word != 0 {
                                psx_io::gpu::write_gp1(word);
                            }
                        }
                        self.last_flip_vblank = psx_rt::interrupts::vblank_count();
                    }
                }
                #[cfg(not(target_arch = "mips"))]
                {
                    psx_rt::interrupts::wait_vblank_with(|| {
                        crate::gpu::tracktex::pump_cd_streams();
                    });
                }
            }
            psx_gpu_mod::arm_draw_done();
            self.fb.apply_draw_target();

            let t_frame_start = psx_io::timers::counter(psx_io::timers::Timer::Timer2);
            let mut t_checkpoint = t_frame_start;
            let mut t_sim_ticks: u16 = 0;
            let mut t_track_ticks: u16 = 0;
            let mut t_cars_ticks: u16 = 0;
            let mut t_misc_ticks: u16 = 0;

            {
                let _s = prof::span(prof::LOOP_OVERHEAD);
                self.frame_counter = self.frame_counter.wrapping_add(1);
                if first_frame {
                    dbg::println("[LOOP] Frame 1 synchronized and rendering!");
                    first_frame = false;
                } else if self.frame_counter.is_multiple_of(300) {
                    dbg::print("[LOOP] Heartbeat: frame ");
                    dbg::print_dec(self.frame_counter);
                    dbg::println("");
                    // Live DMA telemetry on the same cadence as the heartbeat,
                    // so a fallback is visible while the stutter is happening
                    // rather than only at boot. Both channels in one line: the
                    // VRAM side (uploads) and the disc side (sectors).
                    let (dma_u, fifo_u) = upload_path_counts();
                    let (cd_dma, cd_pio) = psx_io::silicon::cd_sector_counts();
                    dbg::print("[LOOP] ch2 uploads dma=");
                    dbg::print_dec(dma_u);
                    dbg::print(" fifo=");
                    dbg::print_dec(fifo_u);
                    dbg::print(" | ch3 sectors dma=");
                    dbg::print_dec(cd_dma);
                    dbg::print(" pio=");
                    dbg::print_dec(cd_pio);
                    // The asynchronous tile fetch. `waited` is the number of
                    // frames that asked for a tile and found the sectors still
                    // in the drive, so it is what the non-blocking read costs;
                    // `stall` is non-zero only if the drive stopped delivering
                    // mid-tile, which should not happen with a one-tile stream.
                    dbg::print(" | tile landed=");
                    dbg::print_dec(crate::async_tile::landed());
                    dbg::print(" waited=");
                    dbg::print_dec(crate::async_tile::waited());
                    dbg::print(" seeks=");
                    dbg::print_dec(crate::async_tile::seeks());
                    dbg::print(" stall=");
                    dbg::print_dec(crate::async_tile::stalled());
                    dbg::print(" open=");
                    dbg::print_dec(crate::async_tile::outstanding());
                    // Player position, so a run can be *proved* to have driven
                    // rather than sat on the grid. Streaming counters only mean
                    // something next to a position that actually changes.
                    dbg::print(" | pos ");
                    dbg::print_dec_signed(self.player.position.x.to_int());
                    dbg::print(",");
                    dbg::print_dec_signed(self.player.position.y.to_int());
                    dbg::print(" mph=");
                    dbg::print_dec(self.player.speed.to_int().unsigned_abs());
                    dbg::println("");
                    dbg::check_faults();
                }
            }

            let mut pad = {
                let _s = prof::span(prof::PAD_POLL);
                psx_pad::poll_port1()
            };
            self.input_mgr.check_hotplug(&pad);
            if self.is_2p_mode {
                let pad2 = psx_pad::poll_port2();
                self.input_mgr2.check_hotplug(&pad2);
            }
            if self.state_mgr.current != GameState::Racing || self.paused {
                pad = self.input_mgr.synthesize_nav(&pad);
            }

            match self.state_mgr.current {
                GameState::Title => {
                    self.audio.ui_frame(pad.buttons.bits());
                    if self.state_mgr.title.update(&pad) {
                        dbg::println("[STATE] Title screen confirmed -> entering MainMenu");
                        self.state_mgr.current = GameState::MainMenu;
                    } else if self.state_mgr.title.timer() >= 1200 {
                        // 20 seconds idle -> enter Attract Mode
                        if self.attract_cycle.is_multiple_of(2) {
                            dbg::println("[ATTRACT] 20s idle -> Playing Intro FMV");
                            self.attract_cycle = self.attract_cycle.wrapping_add(1);
                            self.state_mgr.title = crate::ui::TitleScreen::new();
                            let intro_res = video::play_video("INTRO.STR");
                            self.state_mgr.title = crate::ui::TitleScreen::new();
                            if intro_res == video::VideoResult::Completed {
                                self.state_mgr.title.start_white_fade();
                            }
                        } else {
                            dbg::println("[ATTRACT] 20s idle -> Launching AI Autopilot Demo Race");
                            self.attract_cycle = self.attract_cycle.wrapping_add(1);
                            self.is_attract_demo = true;
                            self.attract_demo_timer = 0;
                            let demo_track = match (self.attract_cycle / 2) % 3 {
                                0 => 0,  // Cascade Falls
                                1 => 19, // Frostpeak Summit
                                _ => 25, // Metro 8
                            };
                            self.load_track(demo_track);
                            self.camera = Camera::new_rotating(
                                self.player.position,
                                self.current_track.start_heading,
                            );
                            self.autopilot = autopilot::Autopilot::new(self.current_track);
                            self.state_mgr.current = GameState::Racing;
                        }
                    }
                    self.fb.clear(0, 0, 0);
                    self.state_mgr.title.render();
                }
                GameState::MainMenu => {
                    self.audio.ui_frame(pad.buttons.bits());
                    if let Some(item) = self.state_mgr.menu.update(&pad) {
                        match item {
                            MenuItem::GrandPrix => {
                                self.is_2p_mode = false;
                                let has_saved = self.memcard.save_data.has_saved_championship();
                                self.state_mgr
                                    .gp_setup
                                    .arm_for_entry(has_saved, GameMode::GrandPrix);
                                self.state_mgr.current = GameState::GrandPrixSetup;
                            }
                            MenuItem::TimeTrial => {
                                self.is_2p_mode = false;
                                self.state_mgr.championship = None;
                                self.state_mgr.stage_selector_source =
                                    StageSelectorSource::TrackSelect;
                                self.state_mgr
                                    .track_select
                                    .arm_for_entry(GameMode::GrandPrix, false);
                                self.state_mgr.current = GameState::TrackSelect;
                            }
                            MenuItem::ManiaPrix => {
                                self.is_2p_mode = false;
                                self.state_mgr.stage_selector_source = StageSelectorSource::None;
                                self.state_mgr
                                    .gp_setup
                                    .arm_for_entry(false, GameMode::Mania);
                                self.state_mgr.current = GameState::GrandPrixSetup;
                            }
                            MenuItem::ManiaTrials => {
                                self.is_2p_mode = false;
                                self.state_mgr.championship = None;
                                self.state_mgr.stage_selector_source =
                                    StageSelectorSource::TrackSelect;
                                self.state_mgr
                                    .track_select
                                    .arm_for_entry(GameMode::Mania, false);
                                self.state_mgr.current = GameState::TrackSelect;
                            }
                            MenuItem::CityFreeDrive => {
                                self.is_2p_mode = false;
                                self.state_mgr.championship = None;
                                self.state_mgr.stage_selector_source = StageSelectorSource::None;
                                self.state_mgr.city_select.arm_for_entry();
                                self.state_mgr.current = GameState::CitySelect;
                            }
                            MenuItem::SplitScreen2P => {
                                self.state_mgr.championship = None;
                                self.is_2p_mode = true;
                                self.state_mgr.stage_selector_source =
                                    StageSelectorSource::TrackSelect;
                                self.state_mgr
                                    .track_select
                                    .arm_for_entry(GameMode::GrandPrix, false);
                                self.state_mgr.current = GameState::TrackSelect;
                            }
                            MenuItem::TrackBuilder => {
                                self.is_2p_mode = false;
                                self.state_mgr.championship = None;
                                self.state_mgr.stage_selector_source = StageSelectorSource::None;
                                if let Some(saved_track) = self
                                    .memcard
                                    .save_data
                                    .custom_track(self.state_mgr.track_editor.slot)
                                {
                                    self.state_mgr.track_editor.track = saved_track;
                                }
                                self.state_mgr.track_editor.arm_for_entry();
                                self.state_mgr.current = GameState::TrackEditor;
                            }
                            MenuItem::TuningGarage => {
                                self.state_mgr.stage_selector_source = StageSelectorSource::None;
                                // Load all three presets and the active slot from the memory card.
                                let slot = self.memcard.save_data.active_tuning_slot as usize;
                                self.state_mgr
                                    .garage
                                    .load_slots(&self.memcard.save_data.tuning_slots, slot);
                                // A `Cross`/`Start` held on the main menu must
                                // not save-and-exit on the garage's first frame.
                                self.state_mgr.garage.sync_edges(&pad);
                                self.state_mgr.current = GameState::Garage;
                            }
                            MenuItem::Records => {
                                self.state_mgr.championship = None;
                                self.state_mgr.stage_selector_source =
                                    StageSelectorSource::StageTable;
                                self.state_mgr
                                    .stage_table
                                    .arm_for_entry(StageFilterTab::All);
                                self.state_mgr.current = GameState::StageTable;
                            }
                            MenuItem::Settings => {
                                // Seed the screen from the settings in force so it
                                // never shows a stale copy of the truth.
                                self.state_mgr.settings.set_values(self.current_settings());
                                self.state_mgr.settings.arm_for_entry();
                                self.state_mgr.current = GameState::Settings;
                            }
                        }
                    }
                    self.fb.clear(15, 18, 25);
                    self.state_mgr.menu.render();
                }
                GameState::GrandPrixSetup => {
                    self.audio.ui_frame(pad.buttons.bits());
                    if let Some(action) = self.state_mgr.gp_setup.update(&pad) {
                        match action {
                            GrandPrixSetupAction::StartNew(cup_idx, diff) => {
                                let champ = ChampionshipSession::with_difficulty(cup_idx, diff);
                                self.state_mgr.championship = Some(champ);
                                self.state_mgr.champ_stage_backup = Some(champ);
                                self.memcard.save_data.save_championship(&champ);
                                self.memcard.mark_dirty();
                                self.memcard.flush();
                                self.load_track(champ.current_track_idx());
                                self.state_mgr.current = GameState::Racing;
                            }
                            GrandPrixSetupAction::Resume => {
                                if let Some(champ) = self.memcard.save_data.load_championship() {
                                    self.state_mgr.championship = Some(champ);
                                    self.state_mgr.champ_stage_backup = Some(champ);
                                    self.load_track(champ.current_track_idx());
                                    self.state_mgr.current = GameState::Racing;
                                }
                            }
                            GrandPrixSetupAction::Back => {
                                self.state_mgr.current = GameState::MainMenu;
                            }
                        }
                    }
                    self.fb.clear(15, 18, 25);
                    let saved_champ = self.memcard.save_data.load_championship();
                    self.state_mgr
                        .gp_setup
                        .render(saved_champ.as_ref(), Some(&self.memcard.save_data));
                }
                GameState::CitySelect => {
                    self.audio.ui_frame(pad.buttons.bits());
                    let (confirmed, cancelled) = self.state_mgr.city_select.update(&pad);
                    if let Some(city) = confirmed {
                        self.load_city_freeroam(city);
                        self.state_mgr.current = GameState::Racing;
                    } else if cancelled {
                        self.state_mgr.current = GameState::MainMenu;
                    }
                    self.fb.clear(15, 18, 25);
                    self.state_mgr.city_select.render();
                }
                GameState::TrackEditor => {
                    self.audio.ui_frame(pad.buttons.bits());
                    let action = self.state_mgr.track_editor.update(&pad);
                    match action {
                        TrackEditorAction::None => {}
                        TrackEditorAction::ExitToMenu => {
                            self.state_mgr.current = GameState::MainMenu;
                        }
                        TrackEditorAction::SaveToCard(slot, track_def) => {
                            self.memcard.save_data.set_custom_track(slot, &track_def);
                            self.memcard.mark_dirty();
                            self.memcard.flush();
                        }
                        TrackEditorAction::LoadFromCard(slot) => {
                            if let Some(track_def) = self.memcard.save_data.custom_track(slot) {
                                self.state_mgr.track_editor.load_track(track_def);
                            } else {
                                self.state_mgr.track_editor.track =
                                    arduracer_core::track_builder::CustomTrackDef::new(
                                        "CUSTOM ARENA",
                                    );
                                self.state_mgr.track_editor.set_status("SLOT IS EMPTY");
                            }
                        }
                        TrackEditorAction::TestDriveValidated(track_def) => {
                            self.load_custom_track(&track_def);
                        }
                    }
                    self.fb.clear(15, 18, 25);
                    self.state_mgr.track_editor.render();
                }
                GameState::Settings => {
                    self.audio.ui_frame(pad.buttons.bits());
                    let b = pad.buttons;
                    let input = SettingsInput {
                        cross: b.is_held(psx_pad::button::CROSS),
                        circle: b.is_held(psx_pad::button::CIRCLE),
                        triangle: b.is_held(psx_pad::button::TRIANGLE),
                        up: b.is_held(psx_pad::button::UP),
                        down: b.is_held(psx_pad::button::DOWN),
                        left: b.is_held(psx_pad::button::LEFT),
                        right: b.is_held(psx_pad::button::RIGHT),
                    };
                    match self.state_mgr.settings.update(input) {
                        SettingsChoice::Changed(values) => {
                            // Apply live: the player should see the format change
                            // and the new pacing without leaving the screen.
                            self.apply_settings(values);
                            self.state_mgr.settings.set_values(self.current_settings());
                        }
                        SettingsChoice::Exit => {
                            self.state_mgr.current = GameState::MainMenu;
                            // The button that closed the screen is necessarily
                            // held; arm so it does not re-trigger on the menu.
                            self.state_mgr.menu.arm_for_entry();
                        }
                        SettingsChoice::None => {}
                    }
                    self.fb.clear(15, 18, 25);
                    self.state_mgr.settings.render();
                }
                GameState::Garage => {
                    self.audio.ui_frame(pad.buttons.bits());
                    if self.state_mgr.garage.update(&pad) {
                        let tuning = self.state_mgr.garage.tuning();
                        self.player.tuning = tuning;
                        let slot = self.state_mgr.garage.slot();
                        let slots = *self.state_mgr.garage.slots();
                        let mut any_stored = false;
                        for (idx, preset) in slots.iter().copied().enumerate() {
                            if self.memcard.store_tuning(idx, preset) {
                                any_stored = true;
                            }
                        }
                        if self.memcard.store_tuning(slot, tuning) || any_stored {
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
                    if self.state_mgr.track_select.check_open_table() {
                        let initial_tab = match self.state_mgr.track_select.mode {
                            arduracer_core::levels::GameMode::Mania => StageFilterTab::Mania,
                            _ => StageFilterTab::GrandPrix,
                        };
                        self.state_mgr.stage_table.arm_for_entry_with_track(
                            initial_tab,
                            self.state_mgr.track_select.selected_track_idx,
                        );
                        self.state_mgr.stage_selector_source = StageSelectorSource::StageTable;
                        self.state_mgr.current = GameState::StageTable;
                    } else if let Some(track_idx) = confirmed {
                        if self.state_mgr.track_select.is_records_mode {
                            self.state_mgr.stage_selector_source = StageSelectorSource::None;
                            self.state_mgr.current = GameState::MainMenu;
                            self.state_mgr.menu.arm_for_entry();
                        } else {
                            let target_idx =
                                if let Some(ref mut champ) = self.state_mgr.championship {
                                    let cup_idx = arduracer_core::levels::ALL_CUPS
                                        .iter()
                                        .position(|c| c.track_indices.contains(&track_idx))
                                        .unwrap_or(0)
                                        as u8;
                                    champ.cup_index = cup_idx;
                                    champ.current_stage = 0;
                                    champ.current_track_idx()
                                } else {
                                    track_idx
                                };
                            self.state_mgr.stage_selector_source = StageSelectorSource::TrackSelect;
                            self.load_track(target_idx);
                            self.state_mgr.current = GameState::Racing;
                        }
                    } else if cancelled {
                        self.is_2p_mode = false;
                        self.state_mgr.stage_selector_source = StageSelectorSource::None;
                        self.state_mgr.current = GameState::MainMenu;
                        self.state_mgr.menu.arm_for_entry();
                    }
                    self.fb.clear(15, 18, 25);
                    self.state_mgr
                        .track_select
                        .render(Some(&self.memcard.save_data));
                }
                GameState::StageTable => {
                    self.audio.ui_frame(pad.buttons.bits());
                    match self.state_mgr.stage_table.update(&pad) {
                        StageTableAction::SelectStage(track_idx) => {
                            self.state_mgr.stage_selector_source = StageSelectorSource::StageTable;
                            self.load_track(track_idx);
                            self.state_mgr.current = GameState::Racing;
                        }
                        StageTableAction::Back => {
                            self.state_mgr.stage_selector_source = StageSelectorSource::None;
                            self.state_mgr.current = GameState::MainMenu;
                            self.state_mgr.menu.arm_for_entry();
                        }
                        StageTableAction::SwitchToCarousel => {
                            let sel_track = self
                                .state_mgr
                                .stage_table
                                .active_tracks()
                                .get(self.state_mgr.stage_table.selected_row)
                                .copied()
                                .unwrap_or(0);
                            let final_mode = if sel_track >= 25 {
                                GameMode::Mania
                            } else {
                                GameMode::GrandPrix
                            };
                            self.state_mgr.track_select.arm_for_entry_with_track(
                                final_mode,
                                self.state_mgr.track_select.is_records_mode,
                                sel_track,
                            );
                            self.state_mgr.stage_selector_source = StageSelectorSource::TrackSelect;
                            self.state_mgr.current = GameState::TrackSelect;
                        }
                        StageTableAction::None => {}
                    }
                    self.fb.clear(15, 18, 25);
                    self.state_mgr
                        .stage_table
                        .render(Some(&self.memcard.save_data));
                }
                GameState::Racing => {
                    let track = self.current_track;

                    if self.is_attract_demo {
                        let exit_pressed = pad.buttons.is_held(psx_pad::button::START)
                            || pad.buttons.is_held(psx_pad::button::CROSS)
                            || pad.buttons.is_held(psx_pad::button::CIRCLE)
                            || pad.buttons.is_held(psx_pad::button::SQUARE);
                        self.attract_demo_timer = self.attract_demo_timer.saturating_add(1);

                        if exit_pressed {
                            dbg::println(
                                "[ATTRACT] Button pressed during demo race -> entering MainMenu",
                            );
                            self.is_attract_demo = false;
                            self.audio.cdda.stop();
                            self.audio.leave_race();
                            self.camera = match self.camera_view {
                                0 => Camera::new_chase(
                                    self.player.position,
                                    self.current_track.start_heading,
                                ),
                                2 => Camera::new_rotating(
                                    self.player.position,
                                    self.current_track.start_heading,
                                ),
                                _ => Camera::new(self.player.position),
                            };
                            self.state_mgr.current = GameState::MainMenu;
                            self.fb.clear(12, 14, 20);
                            self.close_frame();
                            prof::end_frame();
                            continue;
                        } else if self.attract_demo_timer >= 1800 {
                            dbg::println(
                                "[ATTRACT] 30s demo timeout reached -> returning to TitleScreen",
                            );
                            self.is_attract_demo = false;
                            self.audio.cdda.stop();
                            self.audio.leave_race();
                            self.camera = match self.camera_view {
                                0 => Camera::new_chase(
                                    self.player.position,
                                    self.current_track.start_heading,
                                ),
                                2 => Camera::new_rotating(
                                    self.player.position,
                                    self.current_track.start_heading,
                                ),
                                _ => Camera::new(self.player.position),
                            };
                            self.state_mgr.title = crate::ui::TitleScreen::new();
                            self.state_mgr.current = GameState::Title;
                            self.fb.clear(12, 14, 20);
                            self.close_frame();
                            prof::end_frame();
                            continue;
                        }
                    }

                    // GAME.md §7: Start pauses, Select toggles HUD / minimap.
                    // `PauseMenu` owns all button edge state -- the race loop
                    // must not touch it, or the menu sees every frame as
                    // "nothing newly pressed" and cannot be dismissed.
                    let frame = self
                        .pause
                        .update_from_pad_with_values(&pad, self.current_settings());
                    let was_paused = self.paused;
                    if frame.start_pressed {
                        self.paused = !self.paused;
                        // Cutting the engine has to happen here rather than in
                        // `AudioSystem::tick`, because the paused arm of this
                        // arm `continue`s and never reaches the tick.
                        self.audio.set_paused(self.paused);
                        if self.paused {
                            self.input_mgr.silence();
                            self.audio.pause_music();
                        } else {
                            self.audio.resume_music(crate::gpu::tracktex::disc_reader());
                            self.sim_accumulator = 0;
                            self.last_vblank_count = psx_rt::interrupts::vblank_count();
                            self.last_flip_vblank = psx_rt::interrupts::vblank_count();
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
                                    self.audio.resume_music(crate::gpu::tracktex::disc_reader());
                                    self.sim_accumulator = 0;
                                    self.last_vblank_count = psx_rt::interrupts::vblank_count();
                                    self.last_flip_vblank = psx_rt::interrupts::vblank_count();
                                }
                                PauseChoice::RestartRace => {
                                    if self.timer.has_best_lap() {
                                        let medal = track
                                            .par_times
                                            .evaluate_medal(self.timer.best_lap_ticks);
                                        self.memcard.record_lap(
                                            self.current_track_idx,
                                            self.timer.best_lap_ticks,
                                            medal as u8,
                                        );
                                        if self.memcard.is_dirty {
                                            self.memcard.flush();
                                        }
                                    }
                                    if let Some(backup) = self.state_mgr.champ_stage_backup {
                                        self.state_mgr.championship = Some(backup);
                                    }
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
                                    let cdda_track = if self.current_track_idx >= ALL_TRACKS.len() {
                                        4
                                    } else {
                                        3 + ((self.current_track_idx / 6) as u8).min(3)
                                    };
                                    self.audio.play_music(
                                        cdda_track,
                                        crate::gpu::tracktex::disc_reader(),
                                    );
                                }
                                PauseChoice::Graphics(values) => {
                                    self.apply_settings(values);
                                }
                                PauseChoice::QuitToMenu => {
                                    if self.timer.has_best_lap() {
                                        if self.current_track_idx == 999 {
                                            self.state_mgr.track_editor.track.author_time_ticks =
                                                self.timer.best_lap_ticks;
                                            self.memcard.save_data.set_custom_track(
                                                self.state_mgr.track_editor.slot,
                                                &self.state_mgr.track_editor.track,
                                            );
                                            self.memcard.mark_dirty();
                                            self.state_mgr
                                                .track_editor
                                                .set_status("TRACK VALIDATED! AUTHOR TIME SET!");
                                        } else {
                                            let medal = track
                                                .par_times
                                                .evaluate_medal(self.timer.best_lap_ticks);
                                            self.memcard.record_lap(
                                                self.current_track_idx,
                                                self.timer.best_lap_ticks,
                                                medal as u8,
                                            );
                                        }
                                        if self.memcard.is_dirty {
                                            self.memcard.flush();
                                        }
                                    }
                                    self.input_mgr.silence();
                                    self.input_mgr2.silence();
                                    self.return_from_race(pad.buttons.bits());
                                    continue;
                                }
                                PauseChoice::None => {}
                            }
                        }
                        // Repaint the frozen world under the pause veil.
                        {
                            let _s = prof::span(prof::RENDER_TRACK);
                            self.draw_frozen_race();
                            self.pause.render();
                        }
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
                        // `prof::end_frame` has to be called on this path too: the
                        // `continue` below skips the one at the bottom of the
                        // loop, and skipping it would leave this frame's spans
                        // unaccounted and poison the rest of the window.
                        self.close_frame();
                        prof::end_frame();
                        continue;
                    }

                    // In-game camera viewpoint toggle on Triangle press (cycles Behind -> Overhead -> Rotating):
                    if pad
                        .buttons
                        .pressed_since(self.prev_pad_racing, psx_pad::button::TRIANGLE)
                    {
                        self.camera_view = (self.camera_view + 1) % 3;
                        self.camera.mode7 = self.camera_view == 0;
                        self.camera.rotating = self.camera_view == 2;
                        self.camera2.mode7 = self.camera.mode7;
                        self.camera2.rotating = self.camera.rotating;
                        if self.camera.mode7 || self.camera.rotating {
                            self.camera.set_heading(self.player.visual_angle);
                            self.camera2.set_heading(self.player2.visual_angle);
                        }
                    }
                    // In-game debug metrics overlay toggle on Select + R2 press (Off -> FPS -> Graph -> All):
                    if pad.buttons.is_held(psx_pad::button::SELECT)
                        && pad
                            .buttons
                            .pressed_since(self.prev_pad_racing, psx_pad::button::R2)
                    {
                        self.debug_metrics = (self.debug_metrics + 1) % 4;
                    }

                    if self.finish_recorded && self.finish_slowdown < 105 {
                        let pad_any = (pad.buttons.bits() & !self.prev_pad_racing.bits()) != 0;
                        let pad2_any = self.is_2p_mode && {
                            let p2 = psx_pad::poll_port2();
                            (p2.buttons.bits() & !self.prev_pad2_racing.bits()) != 0
                        };
                        if pad_any || pad2_any {
                            self.state_mgr.current = GameState::Results;
                            self.input_mgr.silence();
                            self.input_mgr2.silence();
                            self.audio.leave_race();
                            self.audio.sync_ui_edges(pad.buttons.bits());
                            self.audio
                                .play_music(7, crate::gpu::tracktex::disc_reader());
                            self.close_frame();
                            prof::end_frame();
                            continue;
                        }
                    }
                    self.prev_pad_racing = pad.buttons;
                    if self.is_2p_mode {
                        self.prev_pad2_racing = psx_pad::poll_port2().buttons;
                    }

                    // Frame-rate agnostic simulation updates:
                    let ticks_to_run = self.compute_ticks_to_run();
                    let mut player_rank = self.last_player_rank;
                    for tick_idx in 0..ticks_to_run {
                        let is_last = tick_idx + 1 == ticks_to_run;
                        let (rank, _) = self.tick_racing_simulation(&pad, track, is_last);
                        player_rank = rank;
                        if self.state_mgr.current != GameState::Racing {
                            break;
                        }
                    }

                    // Camera update, clamped to the circuit so the view never leaves the world.
                    {
                        let _s = prof::span(prof::CAMERA);
                        if self.camera.mode7 || self.camera.rotating {
                            self.camera.follow_heading(self.player.visual_angle);
                        }
                        self.camera.update(
                            self.player.position,
                            self.player.velocity,
                            self.player.speed,
                            track.world_width(),
                            track.world_height(),
                        );
                        self.camera.update_banking_and_fov(
                            self.player.slide_bam() as i16,
                            self.player.speed,
                            self.player.boost_ticks,
                            self.player.super_boost_ticks,
                            self.player.vz,
                        );

                        if self.is_2p_mode {
                            if self.camera2.mode7 || self.camera2.rotating {
                                self.camera2.follow_heading(self.player2.visual_angle);
                            }
                            self.camera2.update(
                                self.player2.position,
                                self.player2.velocity,
                                self.player2.speed,
                                track.world_width(),
                                track.world_height(),
                            );
                            self.camera2.update_banking_and_fov(
                                self.player2.slide_bam() as i16,
                                self.player2.speed,
                                self.player2.boost_ticks,
                                self.player2.super_boost_ticks,
                                self.player2.vz,
                            );
                        }
                    }
                    {
                        let t_now = psx_io::timers::counter(psx_io::timers::Timer::Timer2);
                        t_sim_ticks = t_now.wrapping_sub(t_checkpoint);
                        t_checkpoint = t_now;
                    }

                    crate::gpu::tracktex::pump_cd_streams();

                    // Render Pass:
                    // a. Clear background
                    {
                        let _s = prof::span(prof::CLEAR);
                        self.fb.clear(18, 20, 26);
                    }
                    self.sync_camera_view_distance();
                    if !self.is_2p_mode {
                        // b. Track visual map
                        {
                            set_frame_period(self.framerate.vblanks_per_frame());
                            prof::set_budget_cycles(self.framerate.budget_cycles());
                            prof::set_expected_vblanks(self.framerate.vblanks_per_frame());
                            let _s = prof::span(prof::RENDER_TRACK);
                            let elapsed = psx_rt::interrupts::vblank_count()
                                .wrapping_sub(self.last_flip_vblank);
                            let max_decode_rows = arduracer_core::texture_format::decode_row_budget(
                                ticks_to_run,
                                elapsed,
                                self.framerate.vblanks_per_frame(),
                            );
                            render_track(
                                track,
                                &self.camera,
                                self.last_flip_vblank,
                                max_decode_rows,
                            );
                        }
                        // b2. 3D Roadside props, Checkpoint arches & Start gantry truss (NO1-031)
                        crate::gpu::render_track_props(
                            &self.camera,
                            track,
                            if self.current_track_idx < ALL_TRACK_VISUALS.len() {
                                ALL_TRACK_VISUALS[self.current_track_idx]
                            } else {
                                self.current_track_idx
                            },
                            self.timer.checkpoint_mask,
                            Some(&self.start),
                            self.timer.current_lap_ticks,
                        );
                        {
                            let t_now = psx_io::timers::counter(psx_io::timers::Timer::Timer2);
                            t_track_ticks = t_now.wrapping_sub(t_checkpoint);
                            t_checkpoint = t_now;
                        }
                        crate::gpu::tracktex::pump_cd_streams();
                        // c. Skidmarks on track
                        {
                            let _s = prof::span(prof::SKIDMARKS);
                            self.skidmarks.render(&self.camera);
                        }
                        // d. Active ghost car playback (Dev Platinum Author Ghost + Personal Best Ghost)
                        {
                            let _s = prof::span(prof::GHOST_CAR);
                            render_active_ghost(
                                &self.ghost,
                                track,
                                &self.camera,
                                self.timer.current_lap_ticks,
                            );
                        }
                        // e. All race cars (player + rivals) sorted back-to-front
                        {
                            let _s = prof::span(prof::CARS);
                            self.render_cars_sorted((220, 25, 45));
                        }
                        {
                            let t_now = psx_io::timers::counter(psx_io::timers::Timer::Timer2);
                            t_cars_ticks = t_now.wrapping_sub(t_checkpoint);
                            t_checkpoint = t_now;
                        }
                        crate::gpu::tracktex::pump_cd_streams();
                        // g. Particle effects
                        // h. Start lights
                        {
                            let _s = prof::span(prof::PARTICLES);
                            self.particles.render(&self.camera);
                            render_start_lights(self.start.phase());
                        }
                        if self.finish_recorded {
                            let pulse = (self.frame_counter >> 2) & 1 == 0;
                            let banner_bg = if pulse { (220, 35, 55) } else { (180, 25, 45) };
                            psx_gpu_mod::draw_rect_flat(70, 96, 180, 36, 15, 18, 25);
                            psx_gpu_mod::draw_rect_flat(
                                72,
                                98,
                                176,
                                32,
                                banner_bg.0,
                                banner_bg.1,
                                banner_bg.2,
                            );
                            crate::ui::font::draw_text(112, 106, "FINISH!", (255, 235, 40), 2);
                        }

                        // i. In-Game HUD overlay (Select hides it for clean screenshots)
                        if self.show_hud {
                            let _s = prof::span(prof::HUD);
                            render_hud(
                                &self.player,
                                &self.timer,
                                track,
                                player_rank,
                                &self.rivals,
                                &self.minimap_texture,
                            );
                            if self.audio.toast_ticks > 0 {
                                crate::gpu::hud_renderer::render_music_toast(
                                    self.audio.current_music_title(),
                                    self.audio.toast_ticks,
                                );
                            }
                        }

                        // Attract Mode Arcade HUD Banner:
                        if self.is_attract_demo {
                            crate::ui::font::draw_text(
                                60,
                                16,
                                "★ LIVE AI DEMO RACE ★",
                                (0, 220, 255),
                                1,
                            );
                            if (self.attract_demo_timer / 30).is_multiple_of(2) {
                                crate::ui::font::draw_text(
                                    76,
                                    212,
                                    "PRESS START TO RACE",
                                    (255, 230, 0),
                                    1,
                                );
                            }
                        }

                        // j. In-game debug metrics and performance telemetry overlay
                        if self.show_hud && self.debug_metrics > 0 {
                            self.debug_overlay.mode = self.debug_metrics;
                            self.debug_overlay.render(
                                &self.player,
                                track,
                                self.framerate.vblanks_per_frame(),
                            );
                        }
                    } else {
                        // 2-Player Split-Screen Mode (NO1-045):
                        let target_y = self.fb.buffer_y(self.fb.drawing);
                        let elapsed =
                            psx_rt::interrupts::vblank_count().wrapping_sub(self.last_flip_vblank);
                        // Both viewports render the track inside one frame and
                        // share one tile cache, so the frame's decode budget is
                        // split evenly. Handing both passes the full budget let
                        // player 1 -- always drawn first -- spend it all, and
                        // player 2's tiles stayed placeholders.
                        let frame_decode_rows = arduracer_core::texture_format::decode_row_budget(
                            ticks_to_run,
                            elapsed,
                            self.framerate.vblanks_per_frame(),
                        );
                        let p1_decode_rows =
                            arduracer_core::texture_format::decode_row_budget_for_viewport(
                                frame_decode_rows,
                                2,
                                0,
                            );
                        let p2_decode_rows =
                            arduracer_core::texture_format::decode_row_budget_for_viewport(
                                frame_decode_rows,
                                2,
                                1,
                            );

                        match self.split_mode {
                            1 => {
                                // 1. Left Viewport (Player 1)
                                psx_gpu_mod::set_draw_area(0, target_y, 157, target_y + 239);
                                psx_gpu_mod::set_draw_offset(0, target_y as i16);
                                render_track(
                                    track,
                                    &self.camera,
                                    self.last_flip_vblank,
                                    p1_decode_rows,
                                );
                                crate::gpu::render_track_props(
                                    &self.camera,
                                    track,
                                    if self.current_track_idx < ALL_TRACK_VISUALS.len() {
                                        ALL_TRACK_VISUALS[self.current_track_idx]
                                    } else {
                                        self.current_track_idx
                                    },
                                    self.timer.checkpoint_mask,
                                    Some(&self.start),
                                    self.timer.current_lap_ticks,
                                );
                                self.skidmarks.render(&self.camera);
                                self.render_2p_cars_sorted(&self.camera);
                                self.particles.render(&self.camera);
                                if self.show_hud {
                                    crate::gpu::render_split_hud(
                                        &self.player,
                                        &self.timer,
                                        1,
                                        (220, 25, 45),
                                        self.split_mode,
                                    );
                                }

                                // 2. Vertical Divider Bar (X=158..161)
                                psx_gpu_mod::set_draw_area(0, target_y, 319, target_y + 239);
                                psx_gpu_mod::set_draw_offset(0, target_y as i16);
                                psx_gpu_mod::draw_rect_flat(158, 0, 4, 240, 15, 18, 25);
                                psx_gpu_mod::draw_line_mono(158, 0, 158, 239, 255, 215, 0);
                                psx_gpu_mod::draw_line_mono(161, 0, 161, 239, 255, 215, 0);
                                for i in 0..12 {
                                    let y = (i * 20) as i16;
                                    let (r, g, b) = if (i % 2) == 0 {
                                        (255, 255, 255)
                                    } else {
                                        (30, 30, 30)
                                    };
                                    psx_gpu_mod::draw_rect_flat(159, y, 2, 10, r, g, b);
                                }

                                // 3. Right Viewport (Player 2)
                                psx_gpu_mod::set_draw_area(162, target_y, 319, target_y + 239);
                                psx_gpu_mod::set_draw_offset(162, target_y as i16);
                                render_track(
                                    track,
                                    &self.camera2,
                                    self.last_flip_vblank,
                                    p2_decode_rows,
                                );
                                crate::gpu::render_track_props(
                                    &self.camera2,
                                    track,
                                    if self.current_track_idx < ALL_TRACK_VISUALS.len() {
                                        ALL_TRACK_VISUALS[self.current_track_idx]
                                    } else {
                                        self.current_track_idx
                                    },
                                    self.player2_timer.checkpoint_mask,
                                    Some(&self.start),
                                    self.player2_timer.current_lap_ticks,
                                );
                                self.skidmarks.render(&self.camera2);
                                self.render_2p_cars_sorted(&self.camera2);
                                self.particles.render(&self.camera2);
                                if self.show_hud {
                                    crate::gpu::render_split_hud(
                                        &self.player2,
                                        &self.player2_timer,
                                        2,
                                        (30, 144, 255),
                                        self.split_mode,
                                    );
                                }
                            }
                            2 => {
                                // Diagonal Split (Top-Left P1, Bottom-Right P2) via Hardware Mask Bit
                                psx_gpu_mod::set_draw_area(0, target_y, 319, target_y + 239);
                                psx_gpu_mod::set_draw_offset(0, target_y as i16);

                                // Step A: Set bit 15 on P2 triangle
                                psx_gpu_mod::set_mask_mode(true, false);
                                psx_gpu_mod::draw_tri_flat(
                                    [(0, 240), (320, 0), (320, 240)],
                                    0,
                                    0,
                                    0,
                                );

                                // Step B: Render P1
                                psx_gpu_mod::set_mask_mode(true, true);
                                render_track(
                                    track,
                                    &self.camera,
                                    self.last_flip_vblank,
                                    p1_decode_rows,
                                );
                                crate::gpu::render_track_props(
                                    &self.camera,
                                    track,
                                    if self.current_track_idx < ALL_TRACK_VISUALS.len() {
                                        ALL_TRACK_VISUALS[self.current_track_idx]
                                    } else {
                                        self.current_track_idx
                                    },
                                    self.timer.checkpoint_mask,
                                    Some(&self.start),
                                    self.timer.current_lap_ticks,
                                );
                                self.skidmarks.render(&self.camera);
                                self.render_2p_cars_sorted(&self.camera);
                                self.particles.render(&self.camera);
                                if self.show_hud {
                                    crate::gpu::render_split_hud(
                                        &self.player,
                                        &self.timer,
                                        1,
                                        (220, 25, 45),
                                        self.split_mode,
                                    );
                                }

                                // Step C: Clear P2 triangle with clear color (resets P2 pixels to bit 15 = 0)
                                psx_gpu_mod::set_mask_mode(false, false);
                                psx_gpu_mod::draw_tri_flat(
                                    [(0, 240), (320, 0), (320, 240)],
                                    18,
                                    20,
                                    26,
                                );

                                // Step D: Render P2
                                psx_gpu_mod::set_mask_mode(false, true);
                                render_track(
                                    track,
                                    &self.camera2,
                                    self.last_flip_vblank,
                                    p2_decode_rows,
                                );
                                crate::gpu::render_track_props(
                                    &self.camera2,
                                    track,
                                    if self.current_track_idx < ALL_TRACK_VISUALS.len() {
                                        ALL_TRACK_VISUALS[self.current_track_idx]
                                    } else {
                                        self.current_track_idx
                                    },
                                    self.player2_timer.checkpoint_mask,
                                    Some(&self.start),
                                    self.player2_timer.current_lap_ticks,
                                );
                                self.skidmarks.render(&self.camera2);
                                self.render_2p_cars_sorted(&self.camera2);
                                self.particles.render(&self.camera2);
                                if self.show_hud {
                                    crate::gpu::render_split_hud(
                                        &self.player2,
                                        &self.player2_timer,
                                        2,
                                        (30, 144, 255),
                                        self.split_mode,
                                    );
                                }

                                // Step E: Reset mask mode and draw diagonal checkered divider line
                                psx_gpu_mod::set_mask_mode(false, false);
                                psx_gpu_mod::draw_line_mono(318, 0, 0, 238, 255, 215, 0);
                                psx_gpu_mod::draw_line_mono(321, 0, 1, 241, 255, 215, 0);
                                psx_gpu_mod::draw_line_mono(319, 0, 0, 239, 15, 18, 25);
                                psx_gpu_mod::draw_line_mono(320, 0, 1, 240, 15, 18, 25);
                                for i in 0..16 {
                                    let x0 = 320 - (i * 20) as i16;
                                    let y0 = (i * 15) as i16;
                                    let x1 = 320 - ((i + 1) * 20) as i16;
                                    let y1 = ((i + 1) * 15) as i16;
                                    let (r, g, b) = if (i % 2) == 0 {
                                        (255, 255, 255)
                                    } else {
                                        (30, 30, 30)
                                    };
                                    psx_gpu_mod::draw_line_mono(x0, y0, x1, y1, r, g, b);
                                }
                            }
                            _ => {
                                // 1. Top Viewport (Player 1)
                                psx_gpu_mod::set_draw_area(0, target_y, 319, target_y + 117);
                                psx_gpu_mod::set_draw_offset(0, target_y as i16);
                                render_track(
                                    track,
                                    &self.camera,
                                    self.last_flip_vblank,
                                    p1_decode_rows,
                                );
                                crate::gpu::render_track_props(
                                    &self.camera,
                                    track,
                                    if self.current_track_idx < ALL_TRACK_VISUALS.len() {
                                        ALL_TRACK_VISUALS[self.current_track_idx]
                                    } else {
                                        self.current_track_idx
                                    },
                                    self.timer.checkpoint_mask,
                                    Some(&self.start),
                                    self.timer.current_lap_ticks,
                                );
                                self.skidmarks.render(&self.camera);
                                self.render_2p_cars_sorted(&self.camera);
                                self.particles.render(&self.camera);
                                if self.show_hud {
                                    crate::gpu::render_split_hud(
                                        &self.player,
                                        &self.timer,
                                        1,
                                        (220, 25, 45),
                                        self.split_mode,
                                    );
                                }

                                // 2. Center Checkered Divider Bar (Y=118..121)
                                psx_gpu_mod::set_draw_area(0, target_y, 319, target_y + 239);
                                psx_gpu_mod::set_draw_offset(0, target_y as i16);
                                psx_gpu_mod::draw_rect_flat(0, 118, 320, 4, 15, 18, 25);
                                psx_gpu_mod::draw_line_mono(0, 118, 319, 118, 255, 215, 0);
                                psx_gpu_mod::draw_line_mono(0, 121, 319, 121, 255, 215, 0);
                                for i in 0..16 {
                                    let x = (i * 20) as i16;
                                    let (r, g, b) = if (i % 2) == 0 {
                                        (255, 255, 255)
                                    } else {
                                        (30, 30, 30)
                                    };
                                    psx_gpu_mod::draw_rect_flat(x, 119, 10, 2, r, g, b);
                                }

                                // 3. Bottom Viewport (Player 2)
                                psx_gpu_mod::set_draw_area(0, target_y + 122, 319, target_y + 239);
                                psx_gpu_mod::set_draw_offset(0, (target_y + 122) as i16);
                                render_track(
                                    track,
                                    &self.camera2,
                                    self.last_flip_vblank,
                                    p2_decode_rows,
                                );
                                crate::gpu::render_track_props(
                                    &self.camera2,
                                    track,
                                    if self.current_track_idx < ALL_TRACK_VISUALS.len() {
                                        ALL_TRACK_VISUALS[self.current_track_idx]
                                    } else {
                                        self.current_track_idx
                                    },
                                    self.player2_timer.checkpoint_mask,
                                    Some(&self.start),
                                    self.player2_timer.current_lap_ticks,
                                );
                                self.skidmarks.render(&self.camera2);
                                self.render_2p_cars_sorted(&self.camera2);
                                self.particles.render(&self.camera2);
                                if self.show_hud {
                                    crate::gpu::render_split_hud(
                                        &self.player2,
                                        &self.player2_timer,
                                        2,
                                        (30, 144, 255),
                                        self.split_mode,
                                    );
                                }
                            }
                        }

                        // 4. Restore Full Viewport
                        psx_gpu_mod::set_draw_area(0, target_y, 319, target_y + 239);
                        psx_gpu_mod::set_draw_offset(0, target_y as i16);
                        if !self.start.accepts_input() {
                            render_start_lights(self.start.phase());
                        }
                        if self.finish_recorded {
                            let pulse = (self.frame_counter >> 2) & 1 == 0;
                            let banner_bg = if pulse { (220, 35, 55) } else { (180, 25, 45) };
                            psx_gpu_mod::draw_rect_flat(70, 96, 180, 36, 15, 18, 25);
                            psx_gpu_mod::draw_rect_flat(
                                72,
                                98,
                                176,
                                32,
                                banner_bg.0,
                                banner_bg.1,
                                banner_bg.2,
                            );
                            crate::ui::font::draw_text(112, 106, "FINISH!", (255, 235, 40), 2);
                        }
                    }
                    {
                        let t_now = psx_io::timers::counter(psx_io::timers::Timer::Timer2);
                        t_misc_ticks = t_now.wrapping_sub(t_checkpoint);
                    }
                    crate::gpu::tracktex::pump_cd_streams();
                }
                GameState::Replay => {
                    self.handle_replay_theater(&pad);
                }
                GameState::Results => {
                    self.handle_results_screen(&pad);
                }
                GameState::Victory => {
                    self.handle_victory_screen(&pad);
                }
            }

            // 4. Memory card notice. Drawn over every screen: the write that
            // failed happened during a race, and the player needs to know their
            // record is not on the card.
            self.memcard.tick_notice();
            if let Some(status) = self.memcard.notice() {
                render_card_notice(status);
            }

            let elapsed_vblanks =
                psx_rt::interrupts::vblank_count().wrapping_sub(self.last_flip_vblank);
            self.debug_overlay.mode = self.debug_metrics;
            self.debug_overlay.record_timings(
                elapsed_vblanks,
                t_sim_ticks,
                t_track_ticks,
                t_cars_ticks,
                t_misc_ticks,
            );

            // 5. Close the frame and queue the flip for the next VBlank.
            self.close_frame();
            prof::end_frame();
        }
    }

    fn compute_ticks_to_run(&mut self) -> u32 {
        let current_vblank = psx_rt::interrupts::vblank_count();
        if self.last_vblank_count == 0 {
            self.last_vblank_count = current_vblank;
            return 1;
        }
        let elapsed_vblanks = current_vblank.wrapping_sub(self.last_vblank_count);
        self.last_vblank_count = current_vblank;
        if elapsed_vblanks == 0 {
            return 1;
        }
        let clamped_vblanks = elapsed_vblanks.min(4);
        const TICK_COST: u32 = 5;
        let units_per_vblank = if is_pal_mode() { 6 } else { 5 };
        self.sim_accumulator = self
            .sim_accumulator
            .saturating_add(clamped_vblanks * units_per_vblank);
        let mut ticks = 0u32;
        while self.sim_accumulator >= TICK_COST && ticks < 4 {
            self.sim_accumulator -= TICK_COST;
            ticks += 1;
        }
        if ticks >= 4 {
            self.sim_accumulator = 0;
        }
        ticks.max(1)
    }

    #[inline(never)]
    fn tick_racing_simulation(
        &mut self,
        pad: &psx_pad::PadState,
        track: &'static TrackDef,
        compute_standings_now: bool,
    ) -> (u8, [usize; 6]) {
        self.sim_tick_counter = self.sim_tick_counter.wrapping_add(1);
        let sim_tick = self.sim_tick_counter;

        // Controller input poll (surface known from the previous tick
        // so the rumble motors can react to curbs and impacts).
        let pre_tx = TrackDef::tile_x_of(self.player.position.x);
        let pre_ty = TrackDef::tile_y_of(self.player.position.y);
        let pre_tile = track.tile_at(pre_tx, pre_ty);
        if matches!(
            pre_tile,
            arduracer_core::TrackTile::Checkpoint
                | arduracer_core::TrackTile::StartFinish
                | arduracer_core::TrackTile::EngineReset
        ) {
            self.player.free_wheeling = false;
        }
        let pre_surface = pre_tile.surface();
        let mut input = self.input_mgr.update(pad, &self.player, pre_surface);

        if self.is_attract_demo {
            self.autopilot
                .override_input(track, &mut self.player, &mut input);
        } else {
            #[cfg(feature = "profiling")]
            self.autopilot
                .override_input(track, &mut self.player, &mut input);
        }

        // Start sequence: hold the car on the grid, allow rev-matching throttle
        // feathering during the countdown, then arm the lap clock and evaluate
        // the launch RPM at the instant the lights go out.
        let raw_throttle = input.throttle;
        self.start.tick();
        if self.start.just_started() {
            self.timer.start();
            let outcome = StartSequence::evaluate_launch(self.player.engine_rpm);
            self.player.apply_launch_outcome(outcome);
            if outcome == arduracer_core::LaunchOutcome::TurboBoost {
                self.input_mgr.rumble.trigger_boost();
            }
        }
        if !self.start.accepts_input() {
            self.player.rev_engine_at_grid(raw_throttle);
            input.throttle = Fixed::ZERO;
            input.brake = Fixed::ZERO;
            input.steer = Fixed::ZERO;
            input.handbrake = false;
            input.nitro = false;
        }
        if self.finish_recorded {
            input.throttle = Fixed::ZERO;
            input.brake = Fixed::from_raw(2500);
            input.handbrake = false;
            input.nitro = false;
        }

        // TrackMania Dual Respawn & Instant Restart (NO1-012):
        // - Hold L1 (24 frames): Instant Full Track Restart back to starting grid.
        // - Double-tap L1: Standing Respawn (zero velocity) at last cleared checkpoint.
        // - Single-tap L1: Flying Respawn (80% entry speed) at last cleared checkpoint
        //   (or nearest route node if no checkpoint has been cleared yet).
        if self.input_mgr.controller.instant_restart_triggered {
            if self.timer.has_best_lap() {
                let medal = track.par_times.evaluate_medal(self.timer.best_lap_ticks);
                self.memcard.record_lap(
                    self.current_track_idx,
                    self.timer.best_lap_ticks,
                    medal as u8,
                );
            }
            self.reset_race();
            self.audio.enter_race();
            return (self.last_player_rank, self.last_standings);
        } else if self.input_mgr.controller.standing_respawn_pressed {
            let (last_cp, _) = self.timer.record_respawn();
            let (pos, heading) = if last_cp.is_some() {
                track.checkpoint_respawn_point(last_cp)
            } else {
                track.respawn_point(self.player.position)
            };
            self.player.respawn_at(pos, heading);
        } else if self.input_mgr.controller.respawn_pressed {
            let (last_cp, entry_speed) = self.timer.record_respawn();
            let (pos, heading) = if last_cp.is_some() {
                track.checkpoint_respawn_point(last_cp)
            } else {
                track.respawn_point(self.player.position)
            };
            if entry_speed > Fixed::ZERO {
                self.player.flying_respawn_at(pos, heading, entry_speed);
            } else {
                self.player.respawn_at(pos, heading);
            }
        }

        // Simulate: physics tick, then resolve track + bounds.
        let (hit_wall, surface) = {
            let _s = prof::span(prof::SIM_PLAYER);
            self.player.tick(input, pre_surface);
            let hit_wall = self.player.collide_with_track(track);
            let tx = TrackDef::tile_x_of(self.player.position.x);
            let ty = TrackDef::tile_y_of(self.player.position.y);
            let end_tile = track.tile_at(tx, ty);
            if matches!(
                end_tile,
                arduracer_core::TrackTile::Checkpoint
                    | arduracer_core::TrackTile::StartFinish
                    | arduracer_core::TrackTile::EngineReset
            ) {
                self.player.free_wheeling = false;
            }
            (hit_wall, end_tile.surface())
        };

        if self.is_2p_mode {
            let pad2 = psx_pad::poll_port2();
            let p2_tx = TrackDef::tile_x_of(self.player2.position.x);
            let p2_ty = TrackDef::tile_y_of(self.player2.position.y);
            let p2_tile = track.tile_at(p2_tx, p2_ty);
            if matches!(
                p2_tile,
                arduracer_core::TrackTile::Checkpoint
                    | arduracer_core::TrackTile::StartFinish
                    | arduracer_core::TrackTile::EngineReset
            ) {
                self.player2.free_wheeling = false;
            }
            let p2_surface = p2_tile.surface();
            let mut input2 = self.input_mgr2.update(&pad2, &self.player2, p2_surface);

            let raw_throttle2 = input2.throttle;
            if self.start.just_started() {
                self.player2_timer.start();
                let outcome2 = StartSequence::evaluate_launch(self.player2.engine_rpm);
                self.player2.apply_launch_outcome(outcome2);
                if outcome2 == arduracer_core::LaunchOutcome::TurboBoost {
                    self.input_mgr2.rumble.trigger_boost();
                }
            }
            if !self.start.accepts_input() {
                self.player2.rev_engine_at_grid(raw_throttle2);
                input2.throttle = Fixed::ZERO;
                input2.brake = Fixed::ZERO;
                input2.steer = Fixed::ZERO;
                input2.handbrake = false;
                input2.nitro = false;
            }
            if self.finish_recorded {
                input2.throttle = Fixed::ZERO;
                input2.brake = Fixed::from_raw(2500);
                input2.handbrake = false;
                input2.nitro = false;
            }

            // P2 Respawn
            if self.input_mgr2.controller.instant_restart_triggered {
                let p2_pos = grid_slot_position_on(track, 1);
                self.player2.respawn_at(p2_pos, track.start_heading);
            } else if self.input_mgr2.controller.standing_respawn_pressed {
                let (last_cp, _) = self.player2_timer.record_respawn();
                let (pos, heading) = if last_cp.is_some() {
                    track.checkpoint_respawn_point(last_cp)
                } else {
                    track.respawn_point(self.player2.position)
                };
                self.player2.respawn_at(pos, heading);
            } else if self.input_mgr2.controller.respawn_pressed {
                let (last_cp, entry_speed) = self.player2_timer.record_respawn();
                let (pos, heading) = if last_cp.is_some() {
                    track.checkpoint_respawn_point(last_cp)
                } else {
                    track.respawn_point(self.player2.position)
                };
                if entry_speed > Fixed::ZERO {
                    self.player2.flying_respawn_at(pos, heading, entry_speed);
                } else {
                    self.player2.respawn_at(pos, heading);
                }
            }

            // P2 Physics tick & track collision
            self.player2.tick(input2, p2_surface);
            self.player2.collide_with_track(track);

            // P2 Lap timer
            self.player2_timer.tick();
            self.player2_timer
                .update_player_on_track(&self.player2, track);

            // Car-to-car collision between Player 1 and Player 2!
            if let Some(contact) =
                arduracer_core::contact::collide_cars(&mut self.player, &mut self.player2)
            {
                if contact.closing_speed >= arduracer_core::contact::HARD_BUMP_THRESHOLD {
                    self.input_mgr.rumble.trigger_impact(180);
                    self.input_mgr2.rumble.trigger_impact(180);
                }
                self.player.collide_with_track(track);
                self.player2.collide_with_track(track);
            }

            // Check 2P duel completion
            if (self.timer.is_finished || self.player2_timer.is_finished) && !self.finish_recorded {
                self.finish_recorded = true;
                self.finish_slowdown = 120;
                let p1_won = self.timer.is_finished
                    && (!self.player2_timer.is_finished
                        || self.timer.current_lap_ticks <= self.player2_timer.current_lap_ticks);
                self.p1_won_2p = p1_won;
                let best_ticks = if p1_won {
                    if self.timer.has_best_lap() {
                        self.timer.best_lap_ticks
                    } else if self.timer.last_completed_lap_ticks > 0 {
                        self.timer.last_completed_lap_ticks
                    } else {
                        self.timer.current_lap_ticks
                    }
                } else {
                    if self.player2_timer.has_best_lap() {
                        self.player2_timer.best_lap_ticks
                    } else if self.player2_timer.last_completed_lap_ticks > 0 {
                        self.player2_timer.last_completed_lap_ticks
                    } else {
                        self.player2_timer.current_lap_ticks
                    }
                };
                let total_ticks = if p1_won {
                    self.timer.total_race_ticks
                } else {
                    self.player2_timer.total_race_ticks
                };
                self.state_mgr.results = Some(ResultsScreen::new_2p(
                    best_ticks,
                    total_ticks,
                    track,
                    p1_won,
                ));
                self.audio.sfx.play_medal();
            }
        }

        let player_rank;
        let prev_cp_mask = self.timer.checkpoint_mask;
        self.timer.tick();
        if self.timer.update_player_on_track(&self.player, track) {
            // A lap was just scored. Promote this lap's telemetry to
            // the ghost if it is the fastest so far, then start a
            // fresh recording: without this the "ghost" was only ever
            // the opening seconds of lap 1.
            let lap_ticks = self.timer.last_completed_lap_ticks;
            let is_record = lap_ticks != 0 && lap_ticks == self.timer.best_lap_ticks;
            self.ghost.finish_lap(lap_ticks, is_record);
            if is_record {
                self.ghost.save_to_card_block(&mut self.memcard.block);
                self.audio
                    .sfx
                    .play_announcer(crate::audio::AnnouncerClip::NewRecord);
                self.audio.sfx.play_medal();
            }
            self.ghost.start_lap();

            // Record completed lap times and trophies earned
            if self.current_track_idx == 999 {
                if self.timer.has_best_lap() {
                    self.state_mgr.track_editor.track.author_time_ticks = self.timer.best_lap_ticks;
                    self.memcard.save_data.set_custom_track(
                        self.state_mgr.track_editor.slot,
                        &self.state_mgr.track_editor.track,
                    );
                    self.memcard.mark_dirty();
                    self.state_mgr
                        .track_editor
                        .set_status("TRACK VALIDATED! AUTHOR TIME SET!");
                }
            } else if self.timer.has_best_lap() {
                let medal = track.par_times.evaluate_medal(self.timer.best_lap_ticks);
                self.memcard.record_lap(
                    self.current_track_idx,
                    self.timer.best_lap_ticks,
                    medal as u8,
                );
            }
        }
        if self.timer.checkpoint_mask != prev_cp_mask {
            // Clearing any checkpoint gate restores engine power in TrackMania Free-Wheeling sections.
            self.player.free_wheeling = false;
        }

        let (standings, contacts) = {
            let _s = prof::span(prof::SIM_RIVALS);
            let is_racing = self.start.phase() == StartPhase::Racing || self.start.just_started();

            // AI rivals tick with dynamic obstacle avoidance + aerodynamic slipstream drafting (NO1-015)
            if !self.is_2p_mode && RIVALS_ENABLED && is_racing {
                let mut other_positions = [Vec2::ZERO; 6];
                let mut rival_positions = [Vec2::ZERO; 5];
                other_positions[0] = self.player.position;
                for i in 0..5 {
                    other_positions[i + 1] = self.rivals[i].state.position;
                    rival_positions[i] = self.rivals[i].state.position;
                }
                self.player.update_slipstream(&rival_positions);
                // Rivals launch with the player: held on the grid until
                // the lights go out, like a standing start.
                for i in 0..5 {
                    self.rivals[i].state.update_slipstream(&other_positions);
                    let update_input =
                        self.start.just_started() || ((i % 2) == ((sim_tick as usize) % 2));
                    self.rivals[i].tick_interleaved(track, &other_positions, update_input);
                }
            }

            // Resolve vehicle-to-vehicle contacts (player <-> rival, rival <-> rival).
            let contacts = if !self.is_2p_mode && RIVALS_ENABLED && is_racing {
                resolve_field(&mut self.player, &mut self.rivals, track)
            } else {
                arduracer_core::contact::FieldContacts::default()
            };

            // Race standings: player checkpoint progress is compared on
            // the same route index the rivals use. Only recomputed on the
            // last simulation sub-tick of the frame or when the race finishes.
            let standings = if is_racing && (compute_standings_now || self.timer.is_finished) {
                let player_route_node = self.timer.route_node_index(track.route_len()) as u8;
                let s = compute_standings(
                    self.player.position,
                    self.timer.current_lap,
                    player_route_node,
                    self.timer.is_finished,
                    &self.rivals,
                    track,
                );
                self.last_standings = s;
                for (place, &competitor_idx) in s.iter().enumerate() {
                    if competitor_idx == 0 {
                        self.last_player_rank = (place + 1) as u8;
                        break;
                    }
                }
                s
            } else {
                self.last_standings
            };

            player_rank = self.last_player_rank;

            (standings, contacts)
        };

        // Ghost telemetry sample
        {
            let _s = prof::span(prof::SIM_GHOST);
            self.ghost.record_tick(&self.player);
        }

        // Check race completion (single player)
        if !self.is_2p_mode && self.timer.is_finished && !self.finish_recorded {
            self.finish_recorded = true;
            self.finish_slowdown = 120;
            if self.current_track_idx == 999 {
                self.state_mgr.track_editor.track.author_time_ticks = self.timer.best_lap_ticks;
                self.memcard.save_data.set_custom_track(
                    self.state_mgr.track_editor.slot,
                    &self.state_mgr.track_editor.track,
                );
                self.memcard.mark_dirty();
                self.state_mgr
                    .track_editor
                    .set_status("TRACK VALIDATED! AUTHOR TIME SET!");
            } else {
                let effective_best = if self.timer.has_best_lap() {
                    self.timer.best_lap_ticks
                } else if self.timer.last_completed_lap_ticks > 0 {
                    self.timer.last_completed_lap_ticks
                } else {
                    self.timer.current_lap_ticks
                };
                let time_medal = track.par_times.evaluate_medal(effective_best);
                let medal = if self.timer.target_laps == 1 {
                    time_medal.max(self.timer.evaluate_platform_medal())
                } else {
                    time_medal
                };
                self.memcard
                    .record_lap(self.current_track_idx, effective_best, medal as u8);
                // Every scored lap already promoted itself; this only
                // closes out the in-progress final lap.
                self.ghost.finish_lap(effective_best, false);
                self.ghost.save_to_card_block(&mut self.memcard.block);
            }
            // Persist the record before leaving the race screen.
            if self.memcard.is_dirty {
                self.memcard.flush();
            }

            let is_gp = self.state_mgr.championship.is_some();
            if let Some(ref mut champ) = self.state_mgr.championship {
                champ.award_stage_points(standings);
            }

            let display_best = if self.timer.has_best_lap() {
                self.timer.best_lap_ticks
            } else if self.timer.last_completed_lap_ticks > 0 {
                self.timer.last_completed_lap_ticks
            } else {
                self.timer.current_lap_ticks
            };
            self.state_mgr.results = Some(ResultsScreen::new(
                display_best,
                self.timer.total_race_ticks,
                track,
                player_rank,
                is_gp,
            ));
            self.audio.sfx.play_medal();
        }

        let cleared = self.timer.checkpoints_cleared() as u8;
        let any_impact = hit_wall || contacts.player_hard_hit();
        let is_tunnel = matches!(
            track.tile_at(
                TrackDef::tile_x_of(self.player.position.x),
                TrackDef::tile_y_of(self.player.position.y),
            ),
            arduracer_core::TrackTile::Tunnel
        );
        let screen_x = 160i16;
        // Audio engine tick (engine RPM synth, rival 3D audio, tire screech, multi-surface, announcer)
        {
            let _s = prof::span(prof::AUDIO);
            self.audio.tick(
                &self.player,
                input.throttle,
                surface,
                any_impact,
                cleared,
                &self.rivals,
                Some(&self.start),
                self.timer.is_final_lap(),
                screen_x,
                is_tunnel,
            );
            // Keep the XA music stream alive: resume it if a texture run
            // preempted it. Cheap when nothing is wrong.
            self.audio
                .service_music(crate::gpu::tracktex::disc_reader());
        }

        // Emit smoke particles and skidmarks during hard turns, limit scrubbing, or drifts
        if self.player.is_drifting || self.player.tire_scrub > Fixed::ONE {
            self.particles
                .emit_smoke_at(self.player.position, self.player.visual_angle);
            self.skidmarks
                .emit(self.player.position, self.player.visual_angle);
        } else if self.player.tire_scrub > Fixed::from_raw(3200) && (self.frame_counter & 1) == 0 {
            // Intermittent skidmarks when scrubbing tires near the limit of grip
            self.skidmarks
                .emit(self.player.position, self.player.visual_angle);
        }

        // AI rivals also lay down rubber and billow smoke when drifting or scrubbing hard
        if RIVALS_ENABLED {
            for (idx, rival) in self.rivals.iter().enumerate() {
                if !rival.is_finished {
                    let rc = &rival.state;
                    if rc.is_drifting || rc.tire_scrub > Fixed::ONE {
                        if ((self.frame_counter as usize + idx) & 1) == 0 {
                            self.particles.emit_smoke_at(rc.position, rc.visual_angle);
                        }
                        self.skidmarks.emit(rc.position, rc.visual_angle);
                    }
                }
            }
        }
        // Wall scrape sparks + heavy rumble on a barrier hit.
        if hit_wall {
            let away = self.player.velocity.scale(Fixed::from_raw(-256));
            self.particles.emit_sparks(self.player.position, away);
            self.input_mgr.rumble.trigger_impact(200);
        }
        // Vehicle collision sparks + proportional rumble on hard car bumps.
        if contacts.player_hard_hit() {
            let away = contacts.player_contact_normal.scale(Fixed::from_raw(256));
            self.particles
                .emit_sparks(contacts.player_contact_point, away);
            self.input_mgr.rumble.trigger_impact(160);
        }
        if self.player.boost_ticks == 29 {
            self.input_mgr.rumble.trigger_boost();
        }
        self.particles.tick();
        self.skidmarks.tick();

        if self.finish_recorded {
            if self.finish_slowdown > 0 {
                self.finish_slowdown -= 1;
            }
            if self.finish_slowdown == 0 {
                self.state_mgr.current = GameState::Results;
                self.input_mgr.silence();
                self.input_mgr2.silence();
                self.audio.leave_race();
                self.audio.sync_ui_edges(pad.buttons.bits());
                self.audio
                    .play_music(7, crate::gpu::tracktex::disc_reader());
            }
        }

        (player_rank, standings)
    }

    /// Post-Race Action Replay Theater with Broadcast TV Cameras (`NO1-027`).
    #[inline(never)]
    fn handle_replay_theater(&mut self, pad: &psx_pad::PadState) {
        let track = self.current_track;
        let b = pad.buttons;
        let prev = self.prev_pad_racing;
        self.prev_pad_racing = b;

        if b.pressed_since(prev, psx_pad::button::TRIANGLE)
            || b.pressed_since(prev, psx_pad::button::SELECT)
        {
            self.replay_camera_mode = (self.replay_camera_mode + 1) % 3;
            // Cutting to a new camera angle should be a cut, not a sweep in
            // from the previous angle's framing.
            self.camera.tv_unplaced = true;
        }
        if b.pressed_since(prev, psx_pad::button::CROSS)
            || b.pressed_since(prev, psx_pad::button::START)
            || b.pressed_since(prev, psx_pad::button::CIRCLE)
        {
            self.camera.pitch_px = 0;
            if let Some(ref mut results) = self.state_mgr.results {
                results.prev_buttons = b.bits();
            }
            self.state_mgr.current = GameState::Results;
            return;
        }

        let ticks_to_run = self.compute_ticks_to_run();
        let (frames, clip_ticks) = self.ghost.active_replay_clip();
        let total_ticks = if !frames.is_empty() {
            clip_ticks.max(1)
        } else {
            track.par_times.dev_platinum_ticks.max(1)
        };

        self.replay_tick = self.replay_tick.saturating_add(ticks_to_run);
        if self.replay_tick > total_ticks {
            self.replay_tick = 0;
        }

        let sample = if !frames.is_empty() {
            arduracer_core::GhostPlayer::new(frames, total_ticks).sample_at_tick(self.replay_tick)
        } else {
            arduracer_core::sample_author_ghost(track, self.replay_tick).unwrap_or_default()
        };

        self.player.position = sample.position;
        self.player.heading = sample.heading;
        self.player.visual_angle = sample.heading;
        self.player.is_drifting = (sample.flags & arduracer_core::FLAG_DRIFTING) != 0;
        self.player.boost_ticks = if (sample.flags & arduracer_core::FLAG_BOOSTING) != 0 {
            6
        } else {
            0
        };
        if self.player.is_drifting {
            self.particles
                .emit_smoke_at(self.player.position, self.player.visual_angle);
            self.skidmarks
                .emit(self.player.position, self.player.visual_angle);
        }
        self.particles.tick();
        self.skidmarks.tick();

        match self.replay_camera_mode {
            0 => {
                self.camera
                    .update_broadcast_tv(track, sample.position, sample.heading);
            }
            1 => {
                self.camera.mode7 = true;
                self.camera.rotating = false;
                self.camera.follow_heading(sample.heading);
                self.camera.update(
                    sample.position,
                    Vec2::ZERO,
                    Fixed::from_int(3),
                    track.world_width(),
                    track.world_height(),
                );
            }
            _ => {
                self.camera.mode7 = false;
                self.camera.rotating = true;
                self.camera.follow_heading(sample.heading);
                self.camera.update(
                    sample.position,
                    Vec2::ZERO,
                    Fixed::from_int(3),
                    track.world_width(),
                    track.world_height(),
                );
            }
        }

        self.sync_camera_view_distance();
        self.fb.clear(18, 20, 26);
        let elapsed = psx_rt::interrupts::vblank_count().wrapping_sub(self.last_flip_vblank);
        let max_decode_rows = arduracer_core::texture_format::decode_row_budget(
            ticks_to_run,
            elapsed,
            self.framerate.vblanks_per_frame(),
        );
        render_track(track, &self.camera, self.last_flip_vblank, max_decode_rows);
        self.skidmarks.render(&self.camera);
        render_active_ghost(&self.ghost, track, &self.camera, self.replay_tick);
        render_car(&self.player, &self.camera, false, (220, 25, 45));
        self.particles.render(&self.camera);
        render_replay_hud(
            track.name,
            self.replay_tick,
            total_ticks,
            self.replay_camera_mode,
        );
    }

    #[inline(never)]
    fn handle_results_screen(&mut self, pad: &psx_pad::PadState) {
        self.fb.clear(14, 16, 22);
        self.audio.ui_frame(pad.buttons.bits());
        let mut action: Option<ResultsAction> = None;
        if let Some(ref mut results) = self.state_mgr.results {
            action = results.update(pad);
            results.render(self.state_mgr.championship.as_ref());
        }
        match action {
            Some(ResultsAction::WatchReplay) => {
                self.replay_tick = 0;
                self.replay_camera_mode = 0;
                self.camera.tv_unplaced = true;
                self.prev_pad_racing = pad.buttons;
                self.state_mgr.current = GameState::Replay;
            }
            Some(ResultsAction::Continue) => {
                let next_step = if let Some(ref mut champ) = self.state_mgr.championship {
                    if champ.is_last_stage() {
                        None
                    } else {
                        champ.advance_stage();
                        Some(*champ)
                    }
                } else {
                    None
                };

                if let Some(champ) = next_step {
                    self.state_mgr.champ_stage_backup = Some(champ);
                    self.memcard.save_data.save_championship(&champ);
                    self.memcard.mark_dirty();
                    self.memcard.flush();
                    self.load_track(champ.current_track_idx());
                    self.state_mgr.current = GameState::Racing;
                } else if let Some(ref champ) = self.state_mgr.championship {
                    self.memcard.save_data.clear_championship();
                    self.memcard.mark_dirty();
                    self.memcard.flush();
                    #[cfg(not(feature = "profiling"))]
                    self.audio.pause_music();
                    #[cfg(not(feature = "profiling"))]
                    let _ = video::play_video(champ.victory_video_filename());
                    self.state_mgr.victory = Some(VictoryScreen::new(champ));
                    self.state_mgr.current = GameState::Victory;
                    self.audio
                        .play_music(7, crate::gpu::tracktex::disc_reader());
                } else {
                    self.return_from_race(pad.buttons.bits());
                }
            }
            Some(ResultsAction::RestartRace) => {
                if let Some(backup) = self.state_mgr.champ_stage_backup {
                    self.state_mgr.championship = Some(backup);
                    self.memcard.save_data.save_championship(&backup);
                    self.memcard.mark_dirty();
                    self.memcard.flush();
                }
                self.reset_race();
                self.audio.enter_race();
                self.pause.arm_for_race_start();
                self.pause.sync_edges(PauseMenu::input_from_pad(pad));
                let cdda_track = if self.current_track_idx >= ALL_TRACKS.len() {
                    4
                } else {
                    3 + ((self.current_track_idx / 6) as u8).min(3)
                };
                self.audio
                    .play_music(cdda_track, crate::gpu::tracktex::disc_reader());
                self.state_mgr.current = GameState::Racing;
            }
            Some(ResultsAction::ExitToMenu) => {
                if let Some(ref mut champ) = self.state_mgr.championship {
                    if champ.is_last_stage() {
                        self.memcard.save_data.clear_championship();
                    } else {
                        champ.advance_stage();
                        self.memcard.save_data.save_championship(champ);
                    }
                    self.memcard.mark_dirty();
                    self.memcard.flush();
                }
                self.return_from_race(pad.buttons.bits());
            }
            None => {}
        }
    }

    #[inline(never)]
    fn handle_victory_screen(&mut self, pad: &psx_pad::PadState) {
        self.fb.clear(14, 16, 22);
        self.audio.ui_frame(pad.buttons.bits());
        let finished = if let (Some(ref mut victory), Some(ref champ)) =
            (&mut self.state_mgr.victory, &self.state_mgr.championship)
        {
            let cont = victory.update(pad);
            victory.render(champ);
            cont
        } else {
            true
        };
        if finished {
            self.state_mgr.championship = None;
            self.state_mgr.champ_stage_backup = None;
            self.state_mgr.victory = None;
            self.state_mgr.current = GameState::MainMenu;
            self.audio.leave_race();
            self.audio.sync_ui_edges(pad.buttons.bits());
            self.audio
                .play_music(2, crate::gpu::tracktex::disc_reader());
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
    /// The graphics settings currently in force, in persisted form.
    fn current_settings(&self) -> SettingsValues {
        SettingsValues {
            texture_format: texture_format().to_saved(),
            framerate: self.framerate.to_saved(),
            camera_view: self.camera_view,
            cd_audio: self.audio.cd_audio,
            debug_metrics: self.debug_metrics,
            input_profile: self.input_mgr.controller.profile as u8,
            split_mode: self.split_mode,
        }
    }

    fn sync_camera_split_mode(&mut self) {
        if self.is_2p_mode {
            self.camera.set_split_mode(self.split_mode, 1);
            self.camera2.set_split_mode(self.split_mode, 2);
        } else {
            self.camera.set_split_mode(0, 0);
            self.camera2.set_split_mode(0, 0);
        }
    }

    /// Applies a settings change and persists it.
    ///
    /// Order matters. The texture format is pushed into the streamer and the
    /// caches dropped *before* the save is written, so a card write that fails
    /// still leaves the session in the state the player just chose; the next
    /// write picks it up. The reverse order could persist a setting and then
    /// fail to apply it.
    fn apply_settings(&mut self, values: SettingsValues) {
        let previous_format = texture_format();
        let previous_fps = self.framerate;
        let previous_cdda_enabled = self.audio.cdda.enabled();

        let format = TextureFormat::from_saved(values.texture_format);
        let framerate = Framerate::from_saved(values.framerate);

        self.camera_view = values.camera_view;
        self.debug_metrics = values.debug_metrics;
        self.debug_overlay.mode = values.debug_metrics;
        let mode7 = self.camera_view == 0;
        let rotating = self.camera_view == 2;
        if self.camera.mode7 != mode7 || self.camera.rotating != rotating {
            self.camera.mode7 = mode7;
            self.camera.rotating = rotating;
            if mode7 || rotating {
                self.camera.set_heading(self.player.visual_angle);
            }
        }

        if format != previous_format {
            set_texture_format(format);
            self.sync_camera_view_distance();
            // Drop every cached tile so the new codec is used for the circuit
            // already on screen, not just the next one loaded.
            reset_texture_caches();
            if self.state_mgr.current == GameState::Racing {
                let circuit = self.active_circuit();
                init_track_texture(circuit);
                // Prewarm, exactly as `load_track` does, and this is the whole
                // fix for the stutter on switching format mid-race.
                //
                // `reset_texture_caches` above just emptied all 64 VRAM slots, so
                // every visible tile is now a cache miss. Without this the
                // renderer draws placeholders and refills the cache at the
                // decode budget -- one MCU row per frame. A 320x240 view is
                // ~48 tiles at four rows each, so unpausing straight into that
                // is seconds of track visibly repainting itself, at speed, in
                // front of the car. Pausing does not hide it either: the frozen
                // frame admits one row per frame too, and the player changes the
                // setting and unpauses long before that finishes.
                //
                // Paying the same decode here moves the cost to a moment when
                // the simulation is already stopped, which is where `load_track`
                // has always paid it.
                prewarm_starting_tiles(
                    self.current_track.world_width(),
                    self.current_track.world_height(),
                    &self.camera,
                );
                log_silicon_fallback_once();
            }
        }

        if framerate != previous_fps {
            self.framerate = framerate;
            // Re-stamp the pacing counters: the new period is measured against
            // `last_flip_vblank`, and leaving it from the old mode would make
            // the first frame after the switch wait for the wrong duration.
            self.last_flip_vblank = psx_rt::interrupts::vblank_count();
            self.last_vblank_count = psx_rt::interrupts::vblank_count();
            self.sim_accumulator = 0;
            set_frame_period(self.framerate.vblanks_per_frame());
            prof::set_budget_cycles(self.framerate.budget_cycles());
            prof::set_expected_vblanks(self.framerate.vblanks_per_frame());
        }

        // Applied before the caches are dropped, so the drive is released before
        // the next frame starts pulling tiles off it. Turning the music back on
        // does not restart it here: the next `play_track` for whatever screen
        // the player is on does that, which keeps this function free of
        // knowledge about which screen that is.
        // Only CD-DA and Off reach the drive controller; XA is a separate
        // player with its own transfer, and selecting it must not leave the
        // Redbook stream holding the drive.
        let want_cdda =
            values.cd_audio_mode() != CdAudioMode::Xa && values.cd_audio_mode().enabled();
        if want_cdda != previous_cdda_enabled {
            self.audio.cdda.set_enabled(want_cdda);
        }
        // A change of source, not just of on/off: restart the current track on
        // whichever source the player now wants. Without this the choice only
        // takes effect on the next screen, which reads as the setting being
        // ignored.
        //
        // The audio system's copy is updated first and the switch keys off the
        // comparison, so it fires once per change rather than on every later
        // settings write -- a stale copy would restart the music on an unrelated
        // change such as a texture format tweak.
        let source_changed = values.cd_audio != self.audio.cd_audio;
        self.audio.set_cd_audio(values.cd_audio);
        if source_changed {
            self.audio
                .apply_music_mode(crate::gpu::tracktex::disc_reader());
        }

        dbg::print("[SETTINGS] applied format=");
        dbg::print(format.label());
        dbg::print(" framerate=");
        dbg::print(framerate.label());
        dbg::print(" view=");
        dbg::print(match self.camera_view {
            0 => "BEHIND",
            1 => "OVERHEAD",
            _ => "ROTATING",
        });
        dbg::print(" cd-audio=");
        dbg::println(match self.audio.cd_audio {
            2 => "xa",
            1 => "cdda",
            _ => "off",
        });

        self.input_mgr.controller.profile = match values.input_profile {
            1 => InputProfile::ModernTriggers,
            2 => InputProfile::DualAnalog,
            _ => InputProfile::ClassicArcade,
        };
        self.input_mgr2.controller.profile = self.input_mgr.controller.profile;
        self.split_mode = values.split_mode;
        self.sync_camera_split_mode();

        // Persist unconditionally: values are cheap and a card write is
        // already the established pattern for settings changes.
        self.memcard.save_data.texture_format = values.texture_format;
        self.memcard.save_data.framerate = values.framerate;
        self.memcard.save_data.cd_audio = values.cd_audio;
        self.memcard.save_data.input_profile = values.input_profile;
        self.memcard.mark_dirty();
        self.memcard.flush();
    }

    fn close_frame(&mut self) {
        self.transition.render();
        self.transition.tick();
        crate::gpu::tracktex::pump_cd_streams();
        psx_gpu_mod::signal_draw_done();
        #[cfg(target_arch = "mips")]
        {
            // Pace to the selected mode by holding the flip until a full frame
            // period has elapsed since the last one. One VBlank period is 60 FPS,
            // two is the historical 30 FPS lock.
            let period = self.framerate.vblanks_per_frame();
            while psx_rt::interrupts::vblank_count().wrapping_sub(self.last_flip_vblank) + 1
                < period
            {
                psx_rt::interrupts::wait_vblank_with(|| {
                    crate::gpu::tracktex::pump_cd_streams();
                });
            }
        }
        crate::gpu::tracktex::pump_cd_streams();
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
    dbg::println("");
    dbg::println("==================================================");
    dbg::println("      ARDURACER PSX - BOOTSTRAP INITIALIZING      ");
    dbg::println("==================================================");
    dbg::check_stack();
    dbg::check_faults();

    unsafe {
        let slot = core::ptr::addr_of_mut!(GAME);
        dbg::println("[MAIN] Initializing ArduracerGame in static BSS slot...");
        *slot = Some(ArduracerGame::new());
        dbg::println("[MAIN] ArduracerGame::new() completed successfully!");
        dbg::check_stack();
        dbg::check_faults();

        match (*slot).as_mut() {
            Some(game) => {
                dbg::println("[MAIN] Launching game.run()...");
                game.run();
            }
            None => panic!("GAME was assigned above and cannot be None"),
        }
    }
}
