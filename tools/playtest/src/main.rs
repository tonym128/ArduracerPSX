//! Arduracer PSX playability verifier.
//!
//! Runs the *real* `arduracer-core` simulation headlessly on the host and proves
//! that every shipped circuit is actually drivable:
//!
//! * a reference "good driver" completes N timed laps on all 24 tracks,
//! * all 5 rival AI personalities complete laps on all 24 tracks,
//! * the measured lap times are sane against the track's par times,
//! * checkpoints are reachable and ordered along the racing line.
//!
//! It also emits `tools/track_cook/par_calibration.json`, the par-time table
//! consumed by `build_atlas.py`, so medal targets stay achievable instead of
//! being aspirational numbers inherited from the 8-bit original.
//!
//! # Par-time drift is a failure
//!
//! `levels.rs` carries a *generated* par table (`dev`/`gold`/`silver`/`bronze`
//! ticks per circuit). Any physics or tuning change that moves a measured lap
//! time invalidates it, and an invalidated par table silently moves every medal
//! boundary in the game. So a measured-vs-stored mismatch is reported as an
//! error and the process exits non-zero, exactly like an undrivable circuit.
//!
//! The escape hatch is the recalibration workflow itself: `--calibrate`
//! rewrites `tools/track_cook/par_calibration.json` from the fresh
//! measurements (and `make atlas-and-calibrate` then regenerates `levels.rs`),
//! so a `--calibrate` run never fails on the drift it is about to fix.
//! `--allow-par-drift` prints the drift without failing, for the window between
//! landing a physics change and re-committing the recalibrated table. Neither is
//! used by CI: `make playtest` runs with the strict default.
//!
//! Usage:
//!   cargo run --manifest-path tools/playtest/Cargo.toml --release
//!   cargo run --manifest-path tools/playtest/Cargo.toml --release -- --calibrate
//!   cargo run --manifest-path tools/playtest/Cargo.toml --release -- --allow-par-drift
//!   cargo run --manifest-path tools/playtest/Cargo.toml --release -- --render

use std::fmt::Write as _;

use arduracer_core::*;

#[cfg(test)]
mod tests;

/// Laps the reference driver attempts per track.
const LAPS_PER_TRACK: u8 = 5;
/// Safety bound on simulated ticks before a lap is declared "not completed".
const TICK_BUDGET: u32 = 60_000;
/// Medal spread. Dev Platinum is the fastest tuned reference lap (verified
/// drivable by this harness), Gold is the default-tune reference lap, and
/// Silver/Bronze add slack so a first clean run still earns a medal.
const SILVER_FACTOR: f64 = 1.12;
const BRONZE_FACTOR: f64 = 1.28;
/// How much faster than the default tune the best swept preset must lap, as a
/// percentage, for `TUNING_SWEEP` to have proven anything.
///
/// `best_tuning_for` returns the *default* lap unchanged when no preset beats
/// it, so without this the sweep would report "ok" if the Garage sliders were
/// silently dropped from the physics, or if every preset tied. Measured across
/// all 24 circuits the best preset is 7.9%-22.4% quicker than the default
/// tune, so a 5% floor is comfortably below the real signal while still being
/// an order of magnitude above the noise of a one-tick timing change.
const MIN_TUNING_GAIN_PCT: u32 = 5;

// ---------------------------------------------------------------------------
// Reference driver
// ---------------------------------------------------------------------------

/// Deterministic arcade driver used as the playability oracle. It follows the
/// track's ordered route, brakes for corner severity and eases off for hairpins.
struct ReferenceDriver {
    car: VehicleState,
    node: usize,
}

impl ReferenceDriver {
    fn new(track: &TrackDef, tuning: CarTuning) -> Self {
        ReferenceDriver {
            car: VehicleState::new(track.start_pos, track.start_heading, tuning),
            node: 0,
        }
    }

    fn tick(&mut self, track: &TrackDef) {
        let route = track.route_len();
        let target = TrackDef::gate_centre(&track.route_node(self.node % route));
        let desired = heading_towards(self.car.position, target);
        let err = angle_error(self.car.heading, desired);
        let abs_err = err.abs();

        let steer = Fixed::from_raw((err * 6).clamp(-Fixed::ONE.raw(), Fixed::ONE.raw()));

        // Corner-aware throttle schedule.
        let (throttle, brake, handbrake) = if self.car.speed < Fixed::from_int(3) {
            (Fixed::ONE, Fixed::ZERO, false)
        } else if abs_err > 1600 {
            (Fixed::from_raw(1024), Fixed::from_raw(3072), false)
        } else if abs_err > 700 {
            (Fixed::from_raw(2458), Fixed::ZERO, false)
        } else {
            (Fixed::ONE, Fixed::ZERO, false)
        };

        self.car.tick_on_track(
            VehicleInput {
                throttle,
                brake,
                steer,
                handbrake,
                nitro: false,
            },
            track,
        );

        let tx = TrackDef::tile_x_of(self.car.position.x);
        let ty = TrackDef::tile_y_of(self.car.position.y);
        if track.route_node(self.node % route).contains_tile(tx, ty) {
            self.node += 1;
        }
    }
}

struct LapRun {
    laps_completed: u8,
    best_ticks: u32,
    ticks_used: u32,
    max_speed: f64,
    stuck: bool,
}

fn drive_track(track: &TrackDef, tuning: CarTuning) -> LapRun {
    let mut driver = ReferenceDriver::new(track, tuning);
    let mut timer = LapTimer::<16>::new(track.checkpoint_slice(), track.start_gate);
    timer.start();

    let mut best = u32::MAX;
    let mut ticks = 0u32;
    let mut max_speed = 0.0f64;
    let mut last_pos = driver.car.position;
    let mut stuck_ticks = 0u32;

    let mut laps_done = 0u8;
    while ticks < TICK_BUDGET && !timer.is_finished {
        driver.tick(track);
        timer.tick();
        if timer.update_player_tile(
            TrackDef::tile_x_of(driver.car.position.x),
            TrackDef::tile_y_of(driver.car.position.y),
        ) {
            laps_done += 1;
            best = best.min(timer.last_completed_lap_ticks);
        }
        let spd = driver.car.speed.raw() as f64 / 4096.0;
        if spd > max_speed {
            max_speed = spd;
        }

        if (driver.car.position - last_pos).length_squared().raw() < 4 {
            stuck_ticks += 1;
            if stuck_ticks > 300 {
                break;
            }
        } else {
            stuck_ticks = 0;
        }
        last_pos = driver.car.position;

        ticks += 1;
    }

    LapRun {
        laps_completed: laps_done,
        best_ticks: best,
        ticks_used: ticks,
        max_speed,
        stuck: stuck_ticks > 0,
    }
}

/// Tuning presets swept to prove the Garage measurably changes pace.
/// Each must satisfy the 20-point budget of GAME.md §3.2.
const TUNING_SWEEP: [CarTuning; 6] = [
    CarTuning {
        top_speed: 4,
        acceleration: 4,
        handling: 4,
        drift_stability: 4,
        gearing: 4,
    },
    CarTuning {
        top_speed: 7,
        acceleration: 5,
        handling: 4,
        drift_stability: 2,
        gearing: 2,
    },
    CarTuning {
        top_speed: 5,
        acceleration: 7,
        handling: 4,
        drift_stability: 2,
        gearing: 2,
    },
    CarTuning {
        top_speed: 3,
        acceleration: 4,
        handling: 7,
        drift_stability: 4,
        gearing: 2,
    },
    CarTuning {
        top_speed: 6,
        acceleration: 6,
        handling: 3,
        drift_stability: 3,
        gearing: 2,
    },
    CarTuning {
        top_speed: 4,
        acceleration: 4,
        handling: 2,
        drift_stability: 6,
        gearing: 4,
    },
];

/// Tuning whose lap sets the Dev Platinum target (the sweep winner).
///
/// Returns `(preset, best_lap)`. If no preset completes a full set of laps
/// faster than `default_best`, the default preset is returned unchanged --
/// callers must therefore assert the returned lap actually beats the default
/// (see [`MIN_TUNING_GAIN_PCT`]) instead of assuming the sweep won.
fn best_tuning_for(track: &TrackDef, default_best: u32) -> (CarTuning, u32) {
    let mut best = (TUNING_SWEEP[0], default_best);
    for t in TUNING_SWEEP.iter().skip(1) {
        let run = drive_track(track, *t);
        if run.laps_completed == LAPS_PER_TRACK
            && run.best_ticks != u32::MAX
            && run.best_ticks < best.1
        {
            best = (*t, run.best_ticks);
        }
    }
    best
}

/// A measured-vs-stored par-time mismatch for one circuit, or `None` when the
/// generated table in `levels.rs` still matches what the simulation measures.
fn par_drift(track: &TrackDef, dev: u32, gold: u32, silver: u32, bronze: u32) -> Option<String> {
    let p = track.par_times;
    let mismatched: Vec<String> = [
        (p.dev_platinum_ticks, dev, "dev"),
        (p.gold_ticks, gold, "gold"),
        (p.silver_ticks, silver, "silver"),
        (p.bronze_ticks, bronze, "bronze"),
    ]
    .iter()
    .filter(|(stored, _, _)| *stored != u32::MAX)
    .filter(|(stored, measured, _)| stored != measured)
    .map(|(stored, measured, name)| format!("{name} {stored}->{measured}"))
    .collect();

    if mismatched.is_empty() {
        return None;
    }
    Some(format!(
        "{}: par table is stale ({}); run `make atlas-and-calibrate`",
        track.name,
        mismatched.join(", ")
    ))
}

// ---------------------------------------------------------------------------
// Track geometry checks
// ---------------------------------------------------------------------------

fn check_geometry(idx: usize, track: &TrackDef, errors: &mut Vec<String>) {
    let label = format!("track {} ({})", idx + 1, track.name);

    if track.tiles.len() != track.width as usize * track.height as usize {
        errors.push(format!(
            "{}: tile array is {} entries, expected {}",
            label,
            track.tiles.len(),
            track.width as usize * track.height as usize
        ));
        return;
    }

    if track.checkpoint_count == 0 {
        errors.push(format!("{}: no checkpoints", label));
        return;
    }
    if track.checkpoint_count as usize > MAX_TRACK_CHECKPOINTS {
        errors.push(format!("{}: too many checkpoints", label));
    }

    let start_tx = TrackDef::tile_x_of(track.start_pos.x);
    let start_ty = TrackDef::tile_y_of(track.start_pos.y);
    if start_tx >= track.width || start_ty >= track.height {
        errors.push(format!("{}: start position outside the grid", label));
    }
    if !track.tile_at(start_tx, start_ty).is_road() {
        errors.push(format!(
            "{}: start tile is not on the racing surface",
            label
        ));
    }
    if track.start_gate.width == 0 || track.start_gate.height == 0 {
        errors.push(format!("{}: start/finish gate is empty", label));
    }
    if !track.start_gate.contains_tile(
        TrackDef::tile_x_of(track.start_pos.x),
        TrackDef::tile_y_of(track.start_pos.y),
    ) {
        errors.push(format!(
            "{}: start position is not inside the start/finish gate",
            label
        ));
    }

    // Every checkpoint must be reachable from the start and sit on the road.
    let mut seen: Vec<CheckpointGate> = Vec::new();
    for i in 0..track.checkpoint_count as usize {
        let g = track.checkpoints[i];
        if g.x >= track.width || g.y >= track.height {
            errors.push(format!("{}: checkpoint {} out of bounds", label, i));
            continue;
        }
        if !g.is_active() {
            errors.push(format!("{}: checkpoint {} is empty", label, i));
        }
        if !track.tile_at(g.x, g.y).is_road() {
            errors.push(format!(
                "{}: checkpoint {} at ({},{}) is off the racing surface",
                label, i, g.x, g.y
            ));
        }
        if seen.contains(&g) {
            errors.push(format!(
                "{}: duplicate checkpoint {} at ({},{})",
                label, i, g.x, g.y
            ));
        }
        seen.push(g);
    }

    // Par times must be strictly ordered and reachable.
    let p = track.par_times;
    if !(p.dev_platinum_ticks > 0
        && p.dev_platinum_ticks <= p.gold_ticks
        && p.gold_ticks <= p.silver_ticks
        && p.silver_ticks <= p.bronze_ticks)
    {
        errors.push(format!(
            "{}: par times out of order (dev {} gold {} silver {} bronze {})",
            label, p.dev_platinum_ticks, p.gold_ticks, p.silver_ticks, p.bronze_ticks
        ));
    }
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

fn main() {
    let calibrate = std::env::args().any(|a| a == "--calibrate");
    let render = std::env::args().any(|a| a == "--render");
    let allow_par_drift = std::env::args().any(|a| a == "--allow-par-drift");
    let mut all_errors: Vec<String> = Vec::new();
    let mut par_drift_lines: Vec<String> = Vec::new();
    let mut par_table = String::new();

    println!("=======================================================================");
    println!("  ARDURACER PSX - PLAYABILITY VERIFICATION (all 24 circuits)");
    println!("=======================================================================");
    println!(
        "{:<3} {:<23} {:>3} {:>5} {:>8} {:>8} {:>8} {:>8} {:>8} {:>6} {:>4} {:<18}",
        "#",
        "circuit",
        "cp",
        "laps",
        "def_s",
        "tuned_s",
        "gold_s",
        "silver_s",
        "bronze_s",
        "vmax",
        "AI5",
        "verdict"
    );

    for (idx, track) in ALL_TRACKS.iter().enumerate() {
        if render {
            render_ascii(idx, track);
            continue;
        }
        let mut errors = Vec::new();
        check_geometry(idx, track, &mut errors);

        let run = drive_track(track, CarTuning::default());
        if run.laps_completed < LAPS_PER_TRACK {
            errors.push(format!(
                "reference driver completed only {}/{} laps in {} ticks{}",
                run.laps_completed,
                LAPS_PER_TRACK,
                run.ticks_used,
                if run.stuck { " (stalled)" } else { "" }
            ));
        }
        if run.best_ticks == u32::MAX {
            errors.push("no lap was ever timed".to_string());
        } else if run.best_ticks > track.par_times.bronze_ticks {
            errors.push(format!(
                "fastest reference lap {} ticks is slower than the Bronze target {}",
                run.best_ticks, track.par_times.bronze_ticks
            ));
        }

        // All five rival personalities must be able to race the circuit.
        let mut ai_laps_ok = 0u32;
        let mut ai_ticks = 0u32;
        for profile in AI_PROFILES.iter() {
            let mut ai = AiRacer::new(track.start_pos, track.start_heading, *profile);
            let mut t = 0u32;
            while t < TICK_BUDGET && !ai.is_finished {
                ai.tick(track, &[]);
                t += 1;
            }
            if ai.current_lap > LAPS_PER_TRACK {
                ai_laps_ok += 1;
            }
            ai_ticks = ai_ticks.max(t);
        }
        if ai_laps_ok as usize != AI_PROFILES.len() {
            errors.push(format!(
                "only {}/{} rival personalities completed the race",
                ai_laps_ok,
                AI_PROFILES.len()
            ));
        }

        // Medal targets: measured, not inherited from the 8-bit originals.
        let gold = if run.best_ticks == u32::MAX {
            1
        } else {
            run.best_ticks
        };
        let (dev_tuning, dev_best) = best_tuning_for(track, gold);
        let dev = dev_best.min(gold).max(1);
        let silver = ((gold as f64 * SILVER_FACTOR).round() as u32).max(gold);
        let bronze = ((gold as f64 * BRONZE_FACTOR).round() as u32).max(silver);

        if dev > gold {
            errors.push("dev platinum target is slower than the gold target".to_string());
        }

        // The sweep exists to prove the Garage sliders reach the physics. If the
        // best preset is not measurably quicker than the default tune then
        // tuning is either ignored by the vehicle model or every preset tied,
        // and the Dev Platinum target below is meaningless.
        if dev_best as u64 * 100 > gold as u64 * (100 - MIN_TUNING_GAIN_PCT) as u64 {
            errors.push(format!(
                "best swept tuning lap ({} ticks) is not at least {}% faster than the default tune ({} ticks): the Garage sliders are not reaching the physics",
                dev_best, MIN_TUNING_GAIN_PCT, gold
            ));
        }

        let _ = writeln!(
            par_table,
            "  {{ \"track\": \"{}\", \"dev\": {}, \"gold\": {}, \"silver\": {}, \"bronze\": {}, \"tuned_ticks\": {}, \"tuning\": [{}, {}, {}, {}, {}] }},",
            track.name, dev, gold, silver, bronze, dev_best,
            dev_tuning.top_speed, dev_tuning.acceleration, dev_tuning.handling,
            dev_tuning.drift_stability, dev_tuning.gearing
        );

        // Generated par table vs. what the simulation measures right now.
        let drift = par_drift(track, dev, gold, silver, bronze);
        if let Some(line) = &drift {
            par_drift_lines.push(line.clone());
            // `--calibrate` is about to rewrite the table, so it must not fail
            // on the drift it is fixing; `--allow-par-drift` is the explicit
            // opt-out for the recalibration window. Everything else is strict.
            if !allow_par_drift && !calibrate {
                errors.push(line.clone());
            }
        }

        let status = if !errors.is_empty() {
            "FAIL"
        } else if drift.is_some() {
            "PAR DRIFT"
        } else {
            "ok"
        };

        println!(
            "{:<3} {:<23} {:>3} {:>5} {:>8} {:>8} {:>8} {:>8} {:>8} {:>6} {:>4} {:<18}",
            idx + 1,
            track.name,
            track.checkpoint_count,
            run.laps_completed,
            fmt_ticks(run.best_ticks),
            fmt_ticks(dev_best),
            fmt_ticks(track.par_times.gold_ticks),
            fmt_ticks(track.par_times.silver_ticks),
            fmt_ticks(track.par_times.bronze_ticks),
            format!("{:.2}", run.max_speed),
            ai_laps_ok,
            status
        );

        for e in errors {
            println!("       -> {}", e);
            all_errors.push(e);
        }
    }

    if render {
        return;
    }

    println!("-----------------------------------------------------------------------");
    if all_errors.is_empty() {
        println!("  RESULT: all 24 circuits are playable (5 laps player + 5 AI rivals).");
    } else {
        println!("  RESULT: {} failure(s) found.", all_errors.len());
    }
    if !par_drift_lines.is_empty() {
        println!(
            "  PAR DRIFT: {} circuit(s) no longer match tools/track_cook/par_calibration.json:",
            par_drift_lines.len()
        );
        for line in &par_drift_lines {
            println!("       -> {}", line);
        }
        if allow_par_drift {
            println!("       (--allow-par-drift: reported only, not failed)");
        } else if calibrate {
            println!("       (--calibrate: par_calibration.json has been rewritten from these");
            println!(
                "        measurements. Run `make atlas-and-calibrate` to regenerate levels.rs."
            );
        } else {
            println!("       Fix with: make atlas-and-calibrate");
        }
    }
    println!("       (def_s = default tune reference lap, tuned_s = best swept tuning)");
    println!("        Par times are measured; re-run with --calibrate after tuning changes.)");
    println!("=======================================================================");

    if calibrate {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tools/track_cook/par_calibration.json");
        let rows = par_table
            .trim_end()
            .trim_end_matches(',')
            .lines()
            .map(|l| format!("    {}", l))
            .collect::<Vec<_>>()
            .join("\n");
        let json = format!(
            concat!(
                "{{\n",
                "  \"_generated_by\": \"cargo run --manifest-path tools/playtest/Cargo.toml ",
                "--release -- --calibrate\",\n",
                "  \"_note\": \"Lap measurements from the real arduracer-core simulation ",
                "(60 Hz ticks); consumed by build_atlas.py. ",
                "dev = fastest swept tuning, gold = default tuning.\",\n",
                "  \"factors\": {{ \"silver\": {}, \"bronze\": {} }},\n",
                "  \"tracks\": [\n{}\n  ]\n",
                "}}\n"
            ),
            SILVER_FACTOR, BRONZE_FACTOR, rows
        );
        std::fs::write(&path, json).expect("write par_calibration.json");
        println!("  Wrote {}", path.display());
    }

    if !all_errors.is_empty() {
        std::process::exit(1);
    }
}

/// ASCII art dump of a circuit, used to eyeball geometry during QA.
fn render_ascii(idx: usize, track: &TrackDef) {
    const GLYPH: [char; 8] = ['#', ',', '-', '~', '^', '@', 'S', 'C'];
    println!(
        "--- {} ({}) {}x{} ---",
        idx + 1,
        track.name,
        track.width,
        track.height
    );
    let route: Vec<(u8, u8)> = (0..track.checkpoint_count as usize)
        .map(|i| (track.checkpoints[i].x, track.checkpoints[i].y))
        .collect();
    for ty in 0..track.height {
        let mut line = String::new();
        for tx in 0..track.width {
            let t = track.tile_at(tx, ty);
            if track.start_gate.contains_tile(tx, ty) {
                line.push('S');
            } else if route.contains(&(tx, ty)) {
                line.push('C');
            } else {
                line.push(GLYPH[t as usize]);
            }
        }
        println!("  {}", line);
    }
}

fn fmt_ticks(ticks: u32) -> String {
    if ticks == u32::MAX || ticks == 0 {
        return "--".to_string();
    }
    let (m, s, cs) = LapTimer::<1>::format_ticks(ticks);
    format!("{}:{:02}.{:02}", m, s, cs)
}
