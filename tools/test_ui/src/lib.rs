//! Host-side tests for the game's hardware-free presentation logic.
//!
//! The modules under test are included textually (the same trick as
//! `tools/test_memcard`) precisely because they were written to have no
//! hardware dependencies: `pause_input.rs` takes button state as plain booleans
//! and never touches `psx-pad` or the GPU.
//!
//! This exists because of a bug that lived precisely in the untested gap. The
//! race loop used to write `PauseMenu::prev_buttons` itself before handing the
//! pad to `update()`, so the menu compared every frame against itself, no button
//! ever looked newly pressed, and entering the pause menu required a power cycle
//! to escape. The state machine below now owns all edge state, and
//! `pause_menu_can_always_be_dismissed` locks that in.

#![allow(dead_code)]

#[path = "../../../game/src/ui/pause_input.rs"]
mod pause_input;

#[path = "../../../game/src/gpu/camera.rs"]
mod camera;

#[path = "../../../game/src/gpu/car_geometry.rs"]
#[allow(unused_imports)]
mod car_geometry;

#[path = "../../../game/src/ui/tuning_input.rs"]
mod tuning_input;

#[path = "../../../game/src/gpu/effects_sim.rs"]
mod effects_sim;

#[cfg(test)]
mod tests {
    use super::pause_input::{PauseChoice, PauseFrame, PauseInput, PauseMenu};

    fn held(f: impl Fn(&mut PauseInput)) -> PauseInput {
        let mut input = PauseInput::default();
        f(&mut input);
        input
    }

    const IDLE: PauseInput = PauseInput {
        start: false,
        select: false,
        cross: false,
        circle: false,
        up: false,
        down: false,
    };

    /// Replays a scripted sequence of frames and returns every frame's verdict.
    fn run(frames: &[PauseInput]) -> Vec<PauseFrame> {
        let mut menu = PauseMenu::new();
        frames.iter().map(|f| menu.update(*f)).collect()
    }

    /// One DOWN navigation step: press, then release so the next press is an
    /// edge again. Holding the button across frames is a single edge, which is
    /// the behaviour `pause_menu_can_always_be_dismissed` already covers.
    fn step_down(menu: &mut PauseMenu) {
        menu.update(held(|i| i.down = true));
        menu.update(IDLE);
    }

    fn step_up(menu: &mut PauseMenu) {
        menu.update(held(|i| i.up = true));
        menu.update(IDLE);
    }

    /// The regression this whole crate exists for. Every one of these was
    /// unreachable when the race loop shared `prev_buttons` with the menu.
    #[test]
    fn pause_menu_can_always_be_dismissed() {
        // Press START to open. Nothing else held.
        let opened = run(&[held(|i| i.start = true)]);
        assert!(opened[0].start_pressed, "START must read as a fresh press");
        assert_eq!(opened[0].choice, PauseChoice::Resume);

        // The menu is open; pressing START again closes it, and again after
        // that. Entering the pause menu must never be a one-way door.
        let toggled = run(&[
            held(|i| i.start = true),
            IDLE,
            held(|i| i.start = true),
            IDLE,
            held(|i| i.start = true),
        ]);
        assert_eq!(
            toggled.iter().filter(|f| f.start_pressed).count(),
            3,
            "each START press must be seen exactly once"
        );

        // Holding START for a long stretch is one press, not one per frame --
        // otherwise the menu strobes while the button is down.
        let held_down = run(&[
            held(|i| i.start = true),
            held(|i| i.start = true),
            held(|i| i.start = true),
            held(|i| i.start = true),
        ]);
        assert_eq!(
            held_down.iter().filter(|f| f.start_pressed).count(),
            1,
            "a held button is one edge, not four"
        );

        // CROSS on the highlighted entry confirms it, on every entry.
        for idx in 0..PauseMenu::item_count() {
            let mut frames = vec![held(|i| i.start = true), IDLE];
            for _ in 0..idx {
                frames.push(held(|i| i.down = true));
                frames.push(IDLE);
            }
            frames.push(held(|i| i.cross = true));
            let out = run(&frames);
            let expected = match idx {
                0 => PauseChoice::Resume,
                1 => PauseChoice::RestartRace,
                _ => PauseChoice::QuitToMenu,
            };
            assert_eq!(
                out.last().expect("frames ran").choice,
                expected,
                "entry {idx} confirmed the wrong action"
            );
        }
    }

    /// The frame that opens the menu also carries the START press that opened
    /// it. If the caller acted on that frame's choice the veil would open and
    /// close again on the same frame, so the contract is that the menu reports
    /// the edge and the caller ignores the choice until the next frame.
    #[test]
    fn opening_frame_reports_but_does_not_consume() {
        let frames = run(&[held(|i| i.start = true), IDLE]);
        assert!(frames[0].start_pressed);
        assert_eq!(frames[0].choice, PauseChoice::Resume);
        // Next idle frame the menu is calm and waiting.
        assert!(!frames[1].start_pressed);
        assert_eq!(frames[1].choice, PauseChoice::None);
    }

    #[test]
    fn selection_wraps_at_both_ends() {
        let mut menu = PauseMenu::new();
        assert_eq!(menu.selected_idx, 0);

        // Up from the first entry wraps to the last.
        menu.update(held(|i| i.up = true));
        assert_eq!(menu.selected_idx, PauseMenu::item_count() - 1);

        // Down from the last wraps back to the first.
        menu.update(held(|i| i.down = true));
        assert_eq!(menu.selected_idx, 0);

        // Navigation stops at every entry; it never runs off the end.
        for _ in 0..(PauseMenu::item_count() + 3) {
            menu.update(held(|i| i.down = true));
            assert!(menu.selected_idx < PauseMenu::item_count());
        }
    }

    #[test]
    fn select_is_reported_separately_from_the_menu() {
        // Select toggles the HUD and must not also confirm a menu entry.
        let out = run(&[held(|i| i.select = true), IDLE]);
        assert!(out[0].select_pressed);
        assert_eq!(out[0].choice, PauseChoice::None);

        // And holding it down is still a single toggle.
        let held_select = run(&[held(|i| i.select = true), held(|i| i.select = true), IDLE]);
        assert_eq!(held_select.iter().filter(|f| f.select_pressed).count(), 1);
    }

    #[test]
    fn circle_resumes_without_moving_the_highlight() {
        let mut menu = PauseMenu::new();
        menu.update(held(|i| i.down = true));
        let after_nav = menu.selected_idx;
        let out = menu.update(held(|i| i.circle = true));
        assert_eq!(out.choice, PauseChoice::Resume);
        assert_eq!(menu.selected_idx, after_nav);
    }

    #[test]
    fn sync_edges_swallows_a_button_held_across_a_state_change() {
        let mut menu = PauseMenu::new();
        // Player holds CROSS while the track reloads.
        menu.update(held(|i| i.cross = true));

        // The state change adopts what is held right now as the baseline.
        let still_held = held(|i| i.cross = true);
        menu.sync_edges(still_held);

        // Releasing and pressing again is a real press, and confirms normally.
        let out = menu.update(PauseInput::default());
        assert_eq!(out.choice, PauseChoice::None);
        assert_eq!(
            menu.update(held(|i| i.cross = true)).choice,
            PauseChoice::Resume
        );
    }

    /// `sync_edges` has to work for every button, not just CROSS: it is called
    /// on state changes the caller cannot predict (track reload, leaving the
    /// race), and any button it fails to re-baseline reads as a fresh press on
    /// the first frame back.
    #[test]
    fn sync_edges_rebaselines_every_button() {
        let all_held = |i: &mut PauseInput| {
            i.start = true;
            i.select = true;
            i.cross = true;
            i.circle = true;
            i.up = true;
            i.down = true;
        };

        let mut menu = PauseMenu::new();
        // One frame with everything down: every button is an edge.
        let first = menu.update(held(all_held));
        assert!(first.start_pressed);
        assert!(first.select_pressed);

        // A state change happens while they are all still down.
        menu.sync_edges(held(all_held));

        // Release, then press each one again: each must read as a fresh press.
        menu.update(PauseInput::default());
        let again = menu.update(held(all_held));
        assert!(again.start_pressed, "START lost its edge baseline");
        assert!(again.select_pressed, "SELECT lost its edge baseline");
        assert_eq!(again.choice, PauseChoice::Resume);

        // Navigation re-baselined too, or the highlight would jump twice.
        let after = menu.selected_idx;
        menu.sync_edges(held(|i| i.down = true));
        menu.update(PauseInput::default());
        menu.update(held(|i| i.down = true));
        assert_eq!(
            menu.selected_idx,
            after + 1,
            "DOWN moved the highlight by more than one step"
        );
    }

    /// The two navigation buttons are checked in an `if / else if` chain, so a
    /// frame that reports both (a d-pad cross, or a stick knocking the gate) is
    /// resolved in favour of UP. That precedence is load-bearing and was
    /// unasserted, which is exactly how it silently inverts.
    #[test]
    fn a_simultaneous_up_and_down_press_moves_up_once() {
        let both = run(&[held(|i| {
            i.up = true;
            i.down = true;
        })]);
        assert_eq!(
            both[0].choice,
            PauseChoice::None,
            "navigation must not confirm anything"
        );
        assert_eq!(both.len(), 1, "one frame in, one frame out");

        let mut both_menu = PauseMenu::new();
        both_menu.update(held(|i| {
            i.up = true;
            i.down = true;
        }));
        let mut up_menu = PauseMenu::new();
        up_menu.update(held(|i| i.up = true));
        assert_eq!(
            both_menu.selected_idx, up_menu.selected_idx,
            "UP and DOWN together must behave exactly like UP alone"
        );
        assert_eq!(both_menu.selected_idx, PauseMenu::item_count() - 1);
    }

    /// Navigation must wrap by exactly one entry in either direction from
    /// *every* index -- including the `last + 1 >= ITEM_COUNT` boundary, which a
    /// `<` would get wrong.
    #[test]
    fn navigation_is_a_one_step_cycle_through_every_entry() {
        let count = PauseMenu::item_count() as i32;
        assert!(count > 1, "the menu needs at least two entries to wrap");

        for start in 0..count {
            let mut down = PauseMenu::new();
            for _ in 0..start {
                step_down(&mut down);
            }
            assert_eq!(down.selected_idx as i32, start);
            down.update(held(|i| i.down = true));
            assert_eq!(
                down.selected_idx as i32,
                (start + 1) % count,
                "DOWN from entry {start}"
            );

            // `start` steps UP from entry 0 lands on `count - start`, so one
            // more must land on `count - start - 1`.
            let mut up = PauseMenu::new();
            for _ in 0..start {
                step_up(&mut up);
            }
            assert_eq!(up.selected_idx as i32, (count - start) % count);
            up.update(held(|i| i.up = true));
            assert_eq!(
                up.selected_idx as i32,
                (count - start - 1) % count,
                "UP from entry {start}"
            );
        }

        // A full lap in either direction returns to where it started.
        let mut lap = PauseMenu::new();
        for _ in 0..count {
            step_down(&mut lap);
        }
        assert_eq!(lap.selected_idx, 0);
        for _ in 0..count {
            step_up(&mut lap);
        }
        assert_eq!(lap.selected_idx, 0);
    }

    /// The confirm chain is `start || circle` before `cross`, so a frame that
    /// reports a "close the menu" button alongside a "confirm entry" button must
    /// close, not confirm. Getting this backwards would let a player restart the
    /// race by pressing START while CROSS is also down.
    #[test]
    fn close_buttons_win_over_cross_on_a_shared_frame() {
        // Park the highlight on "Restart Race" so a CROSS leak is visible.
        for closer in ["start", "circle"] {
            let mut menu = PauseMenu::new();
            menu.update(held(|i| i.down = true));
            assert_eq!(menu.selected_idx, 1, "expected to park on Restart Race");

            let frame = held(|i| {
                i.cross = true;
                if closer == "start" {
                    i.start = true;
                } else {
                    i.circle = true;
                }
            });
            let out = menu.update(frame);
            assert_eq!(
                out.choice,
                PauseChoice::Resume,
                "{closer} + CROSS on one frame must resume, not restart"
            );
        }

        // And with neither close button down, CROSS on the same highlight does
        // restart -- so the two assertions above really do discriminate.
        let mut menu = PauseMenu::new();
        menu.update(held(|i| i.down = true));
        assert_eq!(
            menu.update(held(|i| i.cross = true)).choice,
            PauseChoice::RestartRace
        );
    }

    /// Confirming an entry must leave the machine in a coherent state: the
    /// highlight is kept (so reopening the menu resumes where the player was),
    /// the same entry can be confirmed again, and START still toggles. A reset
    /// here would silently move the highlight every time the menu is opened.
    #[test]
    fn the_menu_stays_coherent_after_a_confirmation() {
        let mut menu = PauseMenu::new();
        for _ in 0..PauseMenu::item_count() - 1 {
            step_down(&mut menu);
        }
        let parked = menu.selected_idx;
        assert_eq!(parked, PauseMenu::item_count() - 1);

        assert_eq!(
            menu.update(held(|i| i.cross = true)).choice,
            PauseChoice::QuitToMenu
        );
        assert_eq!(menu.selected_idx, parked, "confirming moved the highlight");
        menu.update(IDLE);
        assert_eq!(
            menu.update(held(|i| i.cross = true)).choice,
            PauseChoice::QuitToMenu,
            "releasing and pressing CROSS again must confirm again"
        );

        // START is still live after a confirmation, so the menu is escapable.
        let opened = menu.update(held(|i| i.start = true));
        assert!(opened.start_pressed, "START went dead after a confirm");
        assert_eq!(opened.choice, PauseChoice::Resume);
    }

    /// `update` documents that it must be called exactly once per rendered frame.
    /// Skipping one leaves the previous frame's `PauseInput` behind, so a
    /// release-and-repress across the gap reads as a hold and the press is lost.
    /// The caller cannot see the internal `prev`, so this is only observable --
    /// and only preventable -- from here.
    #[test]
    fn a_skipped_frame_swallows_the_next_press() {
        let mut menu = PauseMenu::new();
        // Frame 1: START pressed.
        assert!(menu.update(held(|i| i.start = true)).start_pressed);
        // The caller skips a frame (track reload) while START is released.
        // `sync_edges` is the documented remedy...
        menu.sync_edges(PauseInput::default());
        // ...and with it, the next press is seen.
        assert!(
            menu.update(held(|i| i.start = true)).start_pressed,
            "sync_edges should have restored the edge"
        );

        // Without the remedy the press is lost. Modelled by *not* calling
        // `update` on the release frame, so `prev` still says START is down.
        let mut stale = PauseMenu::new();
        assert!(stale.update(held(|i| i.start = true)).start_pressed);
        // Frame 2 is skipped entirely; START was released and pressed again.
        assert!(
            !stale.update(held(|i| i.start = true)).start_pressed,
            "a stale baseline must read the re-press as a hold"
        );
        // Only once a frame has actually been observed does the edge return.
        assert!(stale.update(PauseInput::default()).choice == PauseChoice::None);
        assert!(stale.update(held(|i| i.start = true)).start_pressed);
    }

    /// The `Default` impls and the derived `Default`s on the public types are
    /// part of the crate's surface (callers build these from
    /// `PauseMenu::default()`), and were the only lines with no coverage at all.
    #[test]
    fn defaults_match_a_freshly_constructed_menu() {
        let fresh = PauseMenu::new();
        let defaulted = PauseMenu::default();
        assert_eq!(fresh, defaulted);
        assert_eq!(defaulted.selected_idx, 0);

        assert_eq!(PauseMenu::item_count(), 3);
        assert_eq!(PauseChoice::default(), PauseChoice::None);
        assert_eq!(
            PauseFrame::default(),
            PauseFrame {
                start_pressed: false,
                select_pressed: false,
                choice: PauseChoice::None,
            }
        );
        assert_eq!(PauseInput::default(), IDLE);

        // A defaulted menu really is a cold one: its first CROSS is an edge.
        let mut menu = PauseMenu::default();
        assert!(menu.update(held(|i| i.cross = true)).choice == PauseChoice::Resume);
    }
}

#[cfg(test)]
mod camera_tests {
    use super::camera::{Camera, SCREEN_H, SCREEN_W};
    use arduracer_core::{Fixed, Vec2, FP_ONE};

    /// A world big enough that the camera is never bounds-limited, so these
    /// tests measure the projection itself.
    const BIG: (i32, i32) = (100_000, 100_000);

    fn cam_at_rest() -> Camera {
        let mut c = Camera::new(Vec2::new(Fixed::from_int(50_000), Fixed::from_int(50_000)));
        // One update at zero speed: rest zoom, no lead.
        c.update(c.pos, Vec2::ZERO, Fixed::ZERO, BIG.0, BIG.1);
        c
    }

    /// Zoom has to reach the projection. Before this was wired the field was
    /// computed every frame and then ignored, so speed-reactive zoom did
    /// nothing at all.
    #[test]
    fn zoom_actually_scales_the_projection() {
        let cam = cam_at_rest();
        assert_eq!(cam.zoom, Fixed::from_raw(FP_ONE), "rest zoom must be 1:1");

        let at_centre = cam.world_offset(Vec2::new(cam.pos.x, cam.pos.y));
        assert_eq!(at_centre, (0, 0), "camera centre maps to screen centre");

        // A point 100 units to the right is 100 px right of centre at 1:1.
        let probe = Vec2::new(cam.pos.x + Fixed::from_int(100), cam.pos.y);
        assert_eq!(cam.world_offset(probe), (100, 0));

        // Zoom out and the same world distance must cover fewer pixels.
        // The target is a fixed world point, so the camera converges instead of
        // chasing its own position and drifting forever.
        let target = cam.pos;
        let mut fast = cam;
        for _ in 0..600 {
            fast.update(
                target,
                Vec2::new(Fixed::from_int(30), Fixed::ZERO),
                Fixed::from_raw(14_000),
                BIG.0,
                BIG.1,
            );
        }
        assert!(
            fast.zoom < Fixed::from_raw(FP_ONE),
            "top speed must widen the field of view, got {:?}",
            fast.zoom
        );
        let probe_fast = Vec2::new(target.x + Fixed::from_int(100), target.y);
        let (dx, _) = fast.world_offset(probe_fast);
        assert!(dx > 0 && dx < 100, "zoom not applied: dx={dx}");
    }

    /// Zoom has to ease monotonically toward its target, never overshoot.
    #[test]
    fn zoom_is_monotonic_and_bounded() {
        let mut cam = cam_at_rest();
        let mut previous = cam.zoom.raw();
        for tick in 0..300 {
            let speed = Fixed::from_raw(tick * 60);
            cam.update(cam.pos, Vec2::new(speed, Fixed::ZERO), speed, BIG.0, BIG.1);
            let now = cam.zoom.raw();
            assert!(
                now <= previous,
                "zoom went the wrong way at tick {tick}: {previous} -> {now}"
            );
            assert!(now > 3_000 && now <= FP_ONE, "zoom out of range: {now}");
            previous = now;
        }
    }

    /// The car should sit behind screen centre in the direction of travel, so
    /// the driver sees the corner rather than the boot lid.
    #[test]
    fn lookahead_leads_the_car_in_the_direction_of_travel() {
        let mut cam = Camera::new(Vec2::ZERO);
        for _ in 0..240 {
            cam.update(
                Vec2::ZERO,
                Vec2::new(Fixed::from_int(20), Fixed::ZERO),
                Fixed::from_int(8_000),
                BIG.0,
                BIG.1,
            );
        }
        assert!(cam.pos.x > Fixed::ZERO, "camera must lead to the right");
        // The car is then *behind* the camera centre on screen.
        let (sx, _) = cam.world_offset(Vec2::ZERO);
        assert!(sx < 0, "car should sit left of centre, got {sx}");
    }

    /// At a standstill there is no velocity to derive a lead from, but the car
    /// must still not sit dead centre.
    #[test]
    fn stationary_car_is_not_dead_centre() {
        let mut cam = Camera::new(Vec2::ZERO);
        for _ in 0..240 {
            cam.update(Vec2::ZERO, Vec2::ZERO, Fixed::ZERO, BIG.0, BIG.1);
        }
        let (sx, sy) = cam.world_offset(Vec2::ZERO);
        assert!(
            sx != 0 || sy != 0,
            "stationary camera collapsed onto the car"
        );
    }

    /// The view rectangle must never leave the circuit: this is the "never see
    /// off the track" requirement, and it is what the old world-bounds car clamp
    /// failed to do visually.
    #[test]
    fn camera_never_shows_outside_the_circuit() {
        // Larger than the view in both axes, so the clamp has real work to do.
        let world = (4_000i32, 3_000i32);
        let (hw, hh) = cam_at_rest().visible_half_extents();
        assert!(
            hw * 2 < world.0 && hh * 2 < world.1,
            "test needs a world larger than the view"
        );

        // Drive the camera hard into each corner and check every corner pixel.
        let targets = [
            Vec2::new(Fixed::from_int(0), Fixed::from_int(0)),
            Vec2::new(Fixed::from_int(world.0), Fixed::from_int(0)),
            Vec2::new(Fixed::from_int(0), Fixed::from_int(world.1)),
            Vec2::new(Fixed::from_int(world.0), Fixed::from_int(world.1)),
        ];
        for t in targets {
            let mut cam = Camera::new(t);
            for _ in 0..200 {
                cam.update(
                    t,
                    Vec2::new(Fixed::from_int(40), Fixed::ZERO),
                    Fixed::from_raw(14_000),
                    world.0,
                    world.1,
                );
            }
            // Extents must be read at the camera's *final* zoom: it is wider at
            // speed, and using the rest-time value would test the wrong rectangle.
            let (hw, hh) = cam.visible_half_extents();
            let (cx, cy) = (cam.pos.x.to_int(), cam.pos.y.to_int());
            // The whole view rectangle must sit inside the circuit.
            assert!(cx - hw >= 0, "camera x {cx} shows {} px past 0", hw - cx);
            assert!(
                cx + hw <= world.0,
                "camera x {cx} shows {} px past {}",
                cx + hw - world.0,
                world.0
            );
            assert!(cy - hh >= 0, "camera y {cy} shows {} px past 0", hh - cy);
            assert!(
                cy + hh <= world.1,
                "camera y {cy} shows {} px past {}",
                cy + hh - world.1,
                world.1
            );
        }
    }

    /// A circuit narrower than the view cannot be framed without showing
    /// out-of-bounds, so the camera has to centre on it instead of clamping to
    /// an edge -- which would push more off-world on the far side.
    #[test]
    fn world_narrower_than_the_view_centres() {
        let world = (200i32, 150i32);
        let mut cam = Camera::new(Vec2::new(Fixed::from_int(0), Fixed::from_int(0)));
        for _ in 0..200 {
            cam.update(
                Vec2::new(Fixed::from_int(0), Fixed::from_int(0)),
                Vec2::new(Fixed::from_int(30), Fixed::ZERO),
                Fixed::from_raw(14_000),
                world.0,
                world.1,
            );
        }
        assert_eq!(cam.pos.x.to_int(), world.0 / 2);
        assert_eq!(cam.pos.y.to_int(), world.1 / 2);
    }

    /// Visible extents and the projection must agree, or the tile renderer and
    /// the sprites drift apart at speed.
    #[test]
    fn visible_extents_match_the_projection() {
        let mut cam = cam_at_rest();
        for _ in 0..300 {
            cam.update(
                cam.pos,
                Vec2::new(Fixed::from_int(25), Fixed::ZERO),
                Fixed::from_raw(12_000),
                BIG.0,
                BIG.1,
            );
        }
        let (hw, hh) = cam.visible_half_extents();
        let right = cam.world_offset(Vec2::new(cam.pos.x + Fixed::from_int(hw), cam.pos.y));
        let bottom = cam.world_offset(Vec2::new(cam.pos.x, cam.pos.y + Fixed::from_int(hh)));
        assert_eq!(right.0, SCREEN_W / 2, "half-width must reach the edge");
        assert_eq!(bottom.1, SCREEN_H / 2, "half-height must reach the edge");
    }
}

#[cfg(test)]
mod start_tests {
    use arduracer_core::{
        StartPhase, StartSequence, COUNTDOWN_GO_TICKS, COUNTDOWN_LIGHTS, COUNTDOWN_LIGHT_TICKS,
    };

    fn run(ticks: u16) -> StartSequence {
        let mut s = StartSequence::new();
        for _ in 0..ticks {
            s.tick();
        }
        s
    }

    /// The point of the whole feature: the grid holds, then the lights go out.
    #[test]
    fn the_lights_come_on_one_at_a_time_then_go_out() {
        assert_eq!(run(0).phase(), StartPhase::Grid);
        assert_eq!(
            run(COUNTDOWN_LIGHT_TICKS - 1).phase(),
            StartPhase::Grid,
            "the first light must not come on instantly"
        );
        assert_eq!(run(COUNTDOWN_LIGHT_TICKS).phase(), StartPhase::Lit(1));
        assert_eq!(run(COUNTDOWN_LIGHT_TICKS * 2).phase(), StartPhase::Lit(2));
        assert_eq!(run(COUNTDOWN_LIGHT_TICKS * 3).phase(), StartPhase::Lit(3));
        let lights_end = StartSequence::lights_out_tick();
        assert_eq!(run(lights_end).phase(), StartPhase::Go);
        assert_eq!(
            run(lights_end + COUNTDOWN_GO_TICKS).phase(),
            StartPhase::Racing
        );
    }

    /// Every light must be seen, and none skipped: this is the sequence a player
    /// reads to time their launch.
    #[test]
    fn every_light_is_visible_for_its_full_duration() {
        for n in 1..=COUNTDOWN_LIGHTS {
            let start = COUNTDOWN_LIGHT_TICKS * n as u16;
            for offset in 0..COUNTDOWN_LIGHT_TICKS {
                assert_eq!(
                    run(start + offset).phase(),
                    StartPhase::Lit(n),
                    "light {n} flickered early at +{offset}"
                );
            }
        }
    }

    /// Input is refused on the grid and through all three lights, then released.
    /// The clock only arms on GO, so a driver who rests on the throttle from the
    /// start is not also entitled to a flying lap.
    #[test]
    fn input_is_locked_until_the_lights_go_out() {
        let lights_end = StartSequence::lights_out_tick();
        for t in 0..lights_end {
            assert!(
                !run(t).accepts_input(),
                "input accepted at tick {t}, before GO"
            );
        }
        assert!(run(lights_end).accepts_input(), "input must free on GO");
        assert!(run(lights_end + COUNTDOWN_GO_TICKS).accepts_input());
    }

    /// `just_started` is the one-shot the race loop uses to arm the lap clock, so
    /// it must be true for exactly one tick.
    #[test]
    fn the_start_edge_fires_exactly_once() {
        let lights_end = StartSequence::lights_out_tick();
        assert!(!run(lights_end - 1).just_started());
        assert!(run(lights_end).just_started(), "must fire on the GO tick");
        assert!(
            !run(lights_end + 1).just_started(),
            "must not re-fire after GO"
        );
        assert!(!run(lights_end + COUNTDOWN_GO_TICKS).just_started());

        let mut count = 0;
        let mut s = StartSequence::new();
        for _ in 0..(lights_end + COUNTDOWN_GO_TICKS * 2) {
            s.tick();
            if s.just_started() {
                count += 1;
            }
        }
        assert_eq!(count, 1, "start edge fired {count} times");
    }

    /// Input frees on the same tick the clock arms, so the player gets the whole
    /// GO hold to react.
    #[test]
    fn the_go_hold_is_usable() {
        let lights_end = StartSequence::lights_out_tick();
        let usable = COUNTDOWN_GO_TICKS;
        assert!(usable > 0, "there must be time to react to GO");
        // The first GO tick is the start edge itself; every tick after it is
        // reaction time the player actually gets.
        assert!(run(lights_end).just_started());
        for t in lights_end + 1..lights_end + usable {
            assert!(run(t).accepts_input(), "input locked during the GO hold");
            assert!(!run(t).just_started(), "start edge repeated");
        }
    }
}

/// Vehicle polygon geometry. `car_renderer.rs` draws through `psx_gpu` and so
/// cannot be linked on the host, but the vertex maths behind it is pure integer
/// arithmetic and is exactly where TASK-1203's bug lived.
#[cfg(test)]
mod car_geometry_tests {
    use super::car_geometry::{make_quad, make_rect_at, twice_area, Rot};

    /// The eight cardinal headings plus four diagonals, in BAMs (1024 per
    /// quarter turn).
    const HEADINGS: [u16; 12] = [0, 256, 512, 768, 128, 384, 640, 896, 170, 682, 854, 1098];

    /// The cockpit canopy, with the fixed length arguments from
    /// `car_renderer::render_car`.
    const CANOPY: (i32, i32, i32, i32) = (4, 5, 4, 7);

    #[test]
    fn the_cockpit_canopy_has_area_at_every_heading() {
        let (w_front, w_rear, l_front, l_rear) = CANOPY;
        for h in HEADINGS {
            let pts = make_quad(
                160,
                120,
                w_front,
                w_rear,
                l_front,
                l_rear,
                Rot::from_bams(h),
            );
            assert!(twice_area(&pts) > 0, "canopy collapsed at {h} BAM: {pts:?}");
        }
    }

    /// The winding order is load-bearing: emitted bowtie-style, every quad was
    /// a self-intersecting polygon, so the SPU filled a figure-of-eight instead
    /// of the shape and the shoelace area summed to zero even for valid
    /// dimensions.
    #[test]
    fn vertices_are_emitted_in_perimeter_order_not_bowtie_order() {
        let pts = make_quad(160, 120, 6, 7, 12, 12, Rot::from_bams(0));
        // Every consecutive pair, including the closing edge, must share an
        // edge of the quadrilateral rather than cutting across it: a bowtie
        // pairs front-right with rear-left.
        // At 0 BAM the transform is x = cx + u, y = cy - v, so the nose sits at
        // the smaller y. A 12-wide nose over a 14-wide tail, 24 long.
        let quad = [
            (154, 108), // front-left
            (166, 108), // front-right
            (167, 132), // rear-right
            (153, 132), // rear-left
        ];
        assert_eq!(pts, quad, "vertex order or positions changed");
        // Trapezoid rule: parallel sides 12 and 14, height 24.
        assert_eq!(
            twice_area(&pts),
            2 * ((12 + 14) * 24 / 2),
            "the trapezoid must enclose its own area"
        );
    }

    #[test]
    fn rectangles_are_emitted_in_perimeter_order() {
        let pts = make_rect_at(160, 120, 0, -11, 8, 2, Rot::from_bams(0));
        assert_eq!(pts, [(152, 129), (168, 129), (168, 133), (152, 133)]);
        assert_eq!(twice_area(&pts), 2 * 16 * 4);
    }

    #[test]
    fn every_rendered_car_part_has_area_at_every_heading() {
        // The drop shadow, body, left stripe and canopy as `render_car` calls
        // them. A zero-area part is invisible; the audit found one and nothing
        // complained.
        let parts: [(&str, i32, i32, i32, i32); 4] = [
            ("shadow", 7, 8, 12, 12),
            ("body", 6, 7, 12, 12),
            ("stripe", 1, 1, 12, 12),
            ("canopy", 4, 5, 4, 7),
        ];
        for (name, w_front, w_rear, l_front, l_rear) in parts {
            for h in HEADINGS {
                let pts = make_quad(
                    160,
                    120,
                    w_front,
                    w_rear,
                    l_front,
                    l_rear,
                    Rot::from_bams(h),
                );
                assert!(twice_area(&pts) > 0, "{name} collapsed at {h} BAM: {pts:?}");
            }
        }
    }

    #[test]
    fn rectangles_have_area_at_every_heading() {
        // `make_rect_at` is used for the spoiler wing and the wheel pods.
        for h in HEADINGS {
            let wing = make_rect_at(160, 120, 0, -11, 8, 2, Rot::from_bams(h));
            assert!(twice_area(&wing) > 0, "wing collapsed at {h} BAM");
        }
    }

    #[test]
    fn a_rotated_part_keeps_its_area_across_headings() {
        // Rotation must not change the area: a heading-dependent silhouette
        // would mean the fixed-point transform is scaling rather than rotating.
        let reference = twice_area(&make_quad(160, 120, 6, 7, 12, 12, Rot::from_bams(0)));
        for h in HEADINGS {
            let area = twice_area(&make_quad(160, 120, 6, 7, 12, 12, Rot::from_bams(h)));
            // Q20.12 truncation costs a little, and 45-degree headings trade one
            // axis for the other, so allow a few percent.
            let slack: i64 = reference / 20 + 8;
            assert!(
                (area - reference).abs() <= slack,
                "body area at {h} BAM was {area}, expected about {reference}"
            );
        }
    }
}

/// TASK-1209: the pause menu opened itself on frame 1 of a race.
#[cfg(test)]
mod pause_start_arming_tests {
    use super::pause_input::{PauseChoice, PauseFrame, PauseInput, PauseMenu};

    const NOTHING: PauseInput = PauseInput {
        start: false,
        select: false,
        cross: false,
        circle: false,
        up: false,
        down: false,
    };

    /// A player holding `Start` while navigating to the track select.
    const START_HELD: PauseInput = PauseInput {
        start: true,
        ..NOTHING
    };

    fn fresh_start_press() -> PauseFrame {
        let mut menu = PauseMenu::new();
        menu.arm_for_race_start();
        menu.update(START_HELD)
    }

    #[test]
    fn a_held_start_button_does_not_pause_the_first_frame_of_a_race() {
        let frame = fresh_start_press();
        assert!(
            !frame.start_pressed,
            "the veil opened on frame 1 with Start still held from the menu"
        );
        assert_eq!(frame.choice, PauseChoice::None);
    }

    #[test]
    fn a_held_confirm_button_does_not_confirm_a_menu_item_on_frame_one() {
        let frame = fresh_start_press();
        assert_eq!(
            frame.choice,
            PauseChoice::None,
            "a held Cross must not restart or quit straight after a restart"
        );
    }

    #[test]
    fn a_held_direction_does_not_move_the_highlight_on_frame_one() {
        let mut menu = PauseMenu::new();
        menu.arm_for_race_start();
        menu.update(PauseInput {
            down: true,
            ..NOTHING
        });
        assert_eq!(
            menu.selected_idx, 0,
            "the highlight moved from a button held before the race loaded"
        );
    }

    #[test]
    fn start_still_pauses_once_released_and_pressed_again() {
        let mut menu = PauseMenu::new();
        menu.arm_for_race_start();
        // Held across the transition.
        assert!(!menu.update(START_HELD).start_pressed);
        // Released.
        assert!(!menu.update(NOTHING).start_pressed);
        // Pressed again: the veil opens.
        assert!(
            menu.update(START_HELD).start_pressed,
            "arming must not make the pause button unusable"
        );
    }

    #[test]
    fn without_arming_a_held_start_does_open_the_veil() {
        // Pins *why* `arm_for_race_start` exists: this is the pre-fix
        // behaviour, and it is the bug.
        let mut menu = PauseMenu::new();
        assert!(
            menu.update(START_HELD).start_pressed,
            "expected the un-armed menu to treat a held Start as a fresh press"
        );
    }

    #[test]
    fn arming_does_not_reset_the_highlight() {
        // The menu deliberately remembers its selection across opens; arming
        // must not quietly rewind the player's choice.
        let mut menu = PauseMenu::new();
        menu.update(PauseInput {
            down: true,
            ..NOTHING
        });
        menu.update(NOTHING);
        let chosen = menu.selected_idx;
        assert_eq!(chosen, 1);
        menu.arm_for_race_start();
        assert_eq!(
            menu.selected_idx, chosen,
            "arming changed the remembered selection"
        );
    }
}

/// TASK-1210: the garage built setups `memcard::store_tuning` silently refused.
///
/// The garage used to allow slider values 0..=10 while `CarTuning::is_valid` --
/// the exact gate the card write applies -- requires 1..=7. A player who pushed
/// a slider to 8 built a setup the garage accepted, the exit path "saved", and
/// `store_tuning` dropped it: `is_dirty` stayed false so the following
/// `flush()` returned early, and no error appeared anywhere.
///
/// The invariant that matters is therefore not "the sliders feel right" but
/// "no setup the garage can produce is outside the range the card write
/// accepts". These tests assert that as a property over a long random walk.
#[cfg(test)]
mod tuning_budget_tests {
    use super::tuning_input::{Reject, Slider, TuningInput, TuningMenu};
    use arduracer_core::tuning::{CarTuning, DEFAULT_SLIDER, MAX_SLIDER, MIN_SLIDER, TOTAL_POINTS};

    const IDLE: TuningInput = TuningInput {
        up: false,
        down: false,
        left: false,
        right: false,
        cross: false,
        circle: false,
        start: false,
        triangle: false,
    };

    fn press(buttons: TuningInput) -> TuningInput {
        buttons
    }

    fn release() -> TuningInput {
        IDLE
    }

    /// Every slider in turn, so a walk exercises all five axes.
    const SLIDERS: [Slider; 5] = [
        Slider::TopSpeed,
        Slider::Acceleration,
        Slider::Handling,
        Slider::DriftStability,
        Slider::Gearing,
    ];

    fn slider_name(slider: Slider) -> &'static str {
        match slider {
            Slider::TopSpeed => "top_speed",
            Slider::Acceleration => "acceleration",
            Slider::Handling => "handling",
            Slider::DriftStability => "drift_stability",
            Slider::Gearing => "gearing",
        }
    }

    /// Asserts every slider is inside the range `is_valid` checks.
    fn assert_all_in_range(menu: &TuningMenu, context: &str) {
        for slider in SLIDERS {
            let v = slider.value_of(&menu.tuning);
            assert!(
                (MIN_SLIDER..=MAX_SLIDER).contains(&v),
                "{context}: {} = {v} left {MIN_SLIDER}..={MAX_SLIDER} (setup {:?})",
                slider_name(slider),
                menu.tuning,
            );
        }
    }

    /// Moves one point from `from` to `to`, asserting both halves succeeded.
    fn move_point(menu: &mut TuningMenu, from: Slider, to: Slider) {
        menu.decrement(from).expect("a point must be free to move");
        menu.increment(to).expect("a freed point must be spendable");
    }

    /// A setup with every slider at the floor, leaving the whole budget spare.
    fn under_budget() -> TuningMenu {
        TuningMenu::new(CarTuning {
            top_speed: MIN_SLIDER,
            acceleration: MIN_SLIDER,
            handling: MIN_SLIDER,
            drift_stability: MIN_SLIDER,
            gearing: MIN_SLIDER,
        })
    }

    #[test]
    fn the_default_setup_is_saveable() {
        let menu = TuningMenu::new(CarTuning::default());
        assert!(menu.can_save());
        assert!(menu.tuning.is_valid());
    }

    /// The budget is fully allocated by default, so the garage's first move is
    /// always "free a point, then spend it elsewhere".
    #[test]
    fn the_default_setup_starts_with_the_whole_budget_spent() {
        let menu = TuningMenu::new(CarTuning::default());
        assert_eq!(menu.tuning.total_points(), TOTAL_POINTS);
        assert_eq!(menu.points_remaining(), 0);
    }

    /// The core regression, as a property over a long random walk: no slider
    /// ever leaves `1..=7`, and the budget is never overspent. Those are the
    /// only two ways a reachable setup can fail `is_valid` for a reason other
    /// than a temporarily unbalanced total, which the exit gate handles.
    #[test]
    fn no_sequence_of_adjustments_can_produce_an_illegal_setup() {
        let mut menu = TuningMenu::new(CarTuning::default());
        // Deterministic LCG so a failure is reproducible from the seed.
        let mut state: u32 = 0x1234_5678;
        let mut rand = move || {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            state >> 16
        };

        for step in 0..4_000 {
            let choice = rand() % 4;
            let buttons = match choice {
                0 => press(TuningInput {
                    right: true,
                    ..IDLE
                }),
                1 => press(TuningInput { left: true, ..IDLE }),
                2 => press(TuningInput { down: true, ..IDLE }),
                _ => press(TuningInput { up: true, ..IDLE }),
            };
            menu.update(buttons);
            menu.update(release());

            assert_all_in_range(&menu, &format!("step {step}"));
            assert!(
                menu.tuning.total_points() <= TOTAL_POINTS,
                "step {step}: budget overspent, total {} > {TOTAL_POINTS}",
                menu.tuning.total_points(),
            );
        }
    }

    /// A balanced setup is always reachable and always saveable, so the exit
    /// gate cannot lock the player out of the garage.
    #[test]
    fn a_balanced_setup_is_always_reachable_and_saveable() {
        let mut menu = TuningMenu::new(CarTuning::default());
        for i in 0..3_000 {
            // Deliberately biased toward increments and decrements so the walk
            // keeps crossing the balanced state.
            match i % 3 {
                0 => menu.update(press(TuningInput {
                    right: true,
                    ..IDLE
                })),
                _ => menu.update(press(TuningInput { left: true, ..IDLE })),
            };
            menu.update(release());
            if menu.tuning.total_points() == TOTAL_POINTS {
                assert_all_in_range(&menu, &format!("step {i}"));
                assert!(menu.can_save(), "a balanced setup must be saveable");
            }
        }
    }

    #[test]
    fn a_slider_cannot_be_pushed_past_the_maximum() {
        let mut menu = under_budget();
        for _ in 0..20 {
            menu.decrement(Slider::Acceleration).ok();
            menu.increment(Slider::TopSpeed).ok();
        }
        assert_eq!(
            menu.tuning.top_speed, MAX_SLIDER,
            "the slider exceeded the bound `is_valid` enforces"
        );
        assert_all_in_range(&menu, "after pushing to max");
    }

    #[test]
    fn a_slider_cannot_be_pulled_below_the_minimum() {
        let mut menu = under_budget();
        for _ in 0..20 {
            menu.decrement(Slider::TopSpeed).ok();
            menu.increment(Slider::Acceleration).ok();
        }
        assert_eq!(
            menu.tuning.top_speed, MIN_SLIDER,
            "the slider dropped below the bound `is_valid` enforces"
        );
        assert_all_in_range(&menu, "after pulling to min");
    }

    #[test]
    fn the_budget_cannot_be_overspent() {
        let mut menu = TuningMenu::new(CarTuning::default());
        for _ in 0..50 {
            for slider in SLIDERS {
                let _ = menu.increment(slider);
            }
        }
        assert_eq!(menu.tuning.total_points(), TOTAL_POINTS);
        assert_eq!(menu.points_remaining(), 0);
    }

    #[test]
    fn a_refusal_explains_itself() {
        // Cap top speed with room spare, so the refusal can only be the cap.
        let mut menu = under_budget();
        for _ in 0..(MAX_SLIDER - MIN_SLIDER) {
            menu.increment(Slider::TopSpeed).unwrap();
        }
        assert_eq!(menu.tuning.top_speed, MAX_SLIDER);
        assert!(
            menu.points_remaining() > 0,
            "the budget must have room spare, or this proves nothing"
        );
        assert_eq!(
            menu.increment(Slider::TopSpeed),
            Err(Reject::AtMax),
            "a capped slider must report the cap, not a budget problem"
        );
    }

    #[test]
    fn the_floor_also_explains_itself() {
        let mut menu = under_budget();
        assert_eq!(menu.tuning.gearing, MIN_SLIDER);
        assert_eq!(
            menu.decrement(Slider::Gearing),
            Err(Reject::AtMin),
            "a floored slider must report the floor"
        );
    }

    #[test]
    fn spending_the_last_point_reports_no_points_left() {
        // Spread the budget so no single slider hits its cap: 15 spare points
        // across four sliders is 3-4 each, inside the 1..=7 range.
        let mut menu = under_budget();
        let spare = TOTAL_POINTS - 5;
        let others = [
            Slider::TopSpeed,
            Slider::Acceleration,
            Slider::Handling,
            Slider::DriftStability,
        ];
        for (i, slider) in others.iter().enumerate() {
            let share = spare / others.len() as u8
                + if i < (spare % others.len() as u8) as usize {
                    1
                } else {
                    0
                };
            for _ in 0..share {
                menu.increment(*slider).unwrap();
            }
        }
        assert_eq!(menu.points_remaining(), 0);
        for slider in others {
            assert!(
                slider.value_of(&menu.tuning) < MAX_SLIDER,
                "a slider hit its cap, so the next refusal would be ambiguous"
            );
        }
        assert_eq!(menu.increment(others[0]), Err(Reject::NoPointsLeft));
        assert!(
            menu.can_save(),
            "a fully allocated setup is balanced and must still be saveable"
        );
    }

    /// Points may be freed before they are re-spent -- that is the mechanic.
    /// What must not happen is *leaving* with the budget unbalanced, because the
    /// card write would then discard the setup with no message.
    #[test]
    fn points_can_be_freed_before_they_are_re_spent() {
        let mut menu = TuningMenu::new(CarTuning::default());
        assert_eq!(
            menu.decrement(Slider::TopSpeed),
            Ok(()),
            "freeing a point must be allowed even with the budget spent"
        );
        assert_eq!(menu.points_remaining(), 1);
        assert!(!menu.can_save(), "an unbalanced setup must not be saveable");
    }

    #[test]
    fn leaving_with_points_unallocated_is_refused_and_explained() {
        let mut menu = TuningMenu::new(CarTuning::default());
        menu.update(press(TuningInput { left: true, ..IDLE }));
        menu.update(release());
        assert_eq!(menu.points_remaining(), 1);

        let frame = menu.update(press(TuningInput {
            circle: true,
            ..IDLE
        }));
        assert!(
            !frame.exited,
            "left the garage with an unspent point; `store_tuning` would drop it"
        );
        assert_eq!(frame.rejected, Some(Reject::Unbalanced));
    }

    #[test]
    fn leaving_with_sliders_out_of_range_is_impossible() {
        // Complements the walk above: the range invariant holds by
        // construction, so `can_save` can only be false because of the total.
        let mut menu = TuningMenu::new(CarTuning::default());
        for _ in 0..200 {
            menu.update(press(TuningInput { left: true, ..IDLE }));
            menu.update(release());
            assert_all_in_range(&menu, "after decrementing");
            assert_eq!(
                menu.can_save(),
                menu.tuning.total_points() == TOTAL_POINTS,
                "`can_save` disagreed with the budget for {:?}",
                menu.tuning
            );
        }
    }

    #[test]
    fn triangle_arms_a_confirmation_instead_of_wiping_the_setup() {
        let mut menu = TuningMenu::new(CarTuning::default());
        move_point(&mut menu, Slider::Gearing, Slider::TopSpeed);
        let tuned = menu.tuning;

        let frame = menu.update(press(TuningInput {
            triangle: true,
            ..IDLE
        }));
        assert!(frame.reset_requested, "triangle must ask first");
        assert!(
            !frame.reset_confirmed,
            "triangle must not reset on the same frame"
        );
        assert_eq!(menu.tuning, tuned, "the setup was wiped immediately");

        menu.update(release());
        assert!(menu.reset_pending, "the prompt must survive the release");
    }

    #[test]
    fn confirming_the_reset_restores_the_defaults() {
        let mut menu = TuningMenu::new(CarTuning::default());
        move_point(&mut menu, Slider::Gearing, Slider::TopSpeed);
        menu.update(press(TuningInput {
            triangle: true,
            ..IDLE
        }));
        menu.update(release());
        let frame = menu.update(press(TuningInput {
            cross: true,
            ..IDLE
        }));
        assert!(frame.reset_confirmed);
        assert_eq!(menu.tuning, CarTuning::default());
        assert!(
            !menu.reset_pending,
            "the prompt must clear after confirming"
        );
    }

    #[test]
    fn cancelling_the_reset_keeps_the_setup() {
        let mut menu = TuningMenu::new(CarTuning::default());
        move_point(&mut menu, Slider::Gearing, Slider::TopSpeed);
        let tuned = menu.tuning;
        menu.update(press(TuningInput {
            triangle: true,
            ..IDLE
        }));
        menu.update(release());
        let frame = menu.update(press(TuningInput {
            circle: true,
            ..IDLE
        }));
        assert!(frame.reset_cancelled);
        assert_eq!(menu.tuning, tuned, "cancel must not discard the setup");
        assert!(!menu.reset_pending);
    }

    #[test]
    fn the_dpad_does_not_move_sliders_while_the_reset_prompt_is_up() {
        let mut menu = TuningMenu::new(CarTuning::default());
        menu.selected = Slider::TopSpeed;
        menu.update(press(TuningInput {
            triangle: true,
            ..IDLE
        }));
        menu.update(release());
        menu.update(press(TuningInput { down: true, ..IDLE }));
        menu.update(release());
        assert_eq!(
            menu.selected,
            Slider::TopSpeed,
            "the slider selection moved underneath the prompt"
        );
    }

    #[test]
    fn a_held_exit_button_does_not_leave_on_the_first_frame() {
        // `Cross`/`Start` confirm or exit, and both navigate the main menu
        // immediately before this screen.
        let mut menu = TuningMenu::new(CarTuning::default());
        let held = TuningInput {
            circle: true,
            start: true,
            cross: true,
            ..IDLE
        };
        let frame = menu.update(held);
        assert!(!frame.exited, "held buttons saved-and-exited on frame 1");
        menu.update(release());
        assert!(menu.update(held).exited, "exiting must still work");
    }

    #[test]
    fn slider_selection_wraps_in_both_directions() {
        let mut menu = TuningMenu::new(CarTuning::default());
        assert_eq!(menu.selected, Slider::TopSpeed);
        menu.update(press(TuningInput { up: true, ..IDLE }));
        assert_eq!(menu.selected, Slider::Gearing, "up must wrap to the end");
        menu.update(release());
        menu.update(press(TuningInput { down: true, ..IDLE }));
        assert_eq!(
            menu.selected,
            Slider::TopSpeed,
            "down must wrap to the start"
        );
    }

    #[test]
    fn points_remaining_is_never_negative_or_above_the_budget() {
        let mut menu = TuningMenu::new(CarTuning::default());
        for _ in 0..40 {
            menu.decrement(Slider::Handling).ok();
            menu.increment(Slider::Handling).ok();
            let remaining = menu.points_remaining();
            assert!(
                remaining <= TOTAL_POINTS,
                "remaining {remaining} over budget"
            );
        }
    }

    /// The five sliders are all the same shape, so a wrong `match` arm in
    /// `value_of`/`set_value` would be invisible in the walk above.
    #[test]
    fn each_slider_reads_and_writes_its_own_field() {
        for slider in SLIDERS {
            let mut menu = under_budget();
            menu.increment(slider).unwrap();
            let after = slider.value_of(&menu.tuning);
            assert_eq!(
                after,
                MIN_SLIDER + 1,
                "{} read back the wrong field",
                slider_name(slider)
            );
            // And the bump must not have touched any other axis.
            let untouched = SLIDERS
                .iter()
                .filter(|s| **s != slider)
                .all(|s| s.value_of(&menu.tuning) == MIN_SLIDER);
            assert!(
                untouched,
                "adjusting {} moved another axis ({:?})",
                slider_name(slider),
                menu.tuning
            );
        }
    }

    #[test]
    fn the_default_value_sits_inside_the_legal_range() {
        // The gauge and the gate both index off these constants; if a slider
        // ever started outside the range the arithmetic would be nonsense.
        assert!((MIN_SLIDER..=MAX_SLIDER).contains(&DEFAULT_SLIDER));
    }
}

/// TASK-1206: particles and skidmarks were emitted but effectively invisible.
#[cfg(test)]
mod effects_tests {
    use super::effects_sim::{
        ParticleSystem, ParticleType, SkidmarkBuffer, MAX_PARTICLES, MAX_SKIDMARKS,
        SKIDMARK_FADE_FROM, SKIDMARK_LIFE, SMOKE_LIFE, SPARK_LIFE,
    };
    use arduracer_core::{Fixed, Vec2};

    const SOMEWHERE: Vec2 = Vec2::new(Fixed::from_int(400), Fixed::from_int(400));

    /// The ring is stamped at most once per frame, so a mark cannot outlive the
    /// buffer or it is always overwritten mid-fade.
    #[test]
    fn a_skidmark_outlives_the_buffer_that_holds_it() {
        assert!(
            SKIDMARK_LIFE as usize <= MAX_SKIDMARKS,
            "life {SKIDMARK_LIFE} exceeds the {MAX_SKIDMARKS}-slot ring, so every \
             mark is overwritten before it fades"
        );
    }

    #[test]
    fn a_skidmark_actually_reaches_its_faded_stage() {
        // The regression: at life 180 against a 96-slot ring the fade threshold
        // was unreachable in practice.
        let mut marks = SkidmarkBuffer::new();
        marks.emit(SOMEWHERE, 0);
        let mut saw_fresh = false;
        let mut saw_faded = false;
        for _ in 0..SKIDMARK_LIFE {
            let (_, faded) = marks.visible().next().expect("mark must be live");
            if faded {
                saw_faded = true;
            } else {
                saw_fresh = true;
            }
            marks.tick();
        }
        assert!(saw_fresh, "the mark was never drawn as fresh rubber");
        assert!(
            saw_faded,
            "the mark never reached its faded stage, so the fade branch is dead code"
        );
    }

    #[test]
    fn a_skidmark_expires_rather_than_lingering() {
        let mut marks = SkidmarkBuffer::new();
        marks.emit(SOMEWHERE, 0);
        assert_eq!(marks.live_count(), 1);
        for _ in 0..SKIDMARK_LIFE {
            marks.tick();
        }
        assert_eq!(marks.live_count(), 0, "the mark outlived its lifetime");
    }

    #[test]
    fn the_fade_threshold_is_inside_the_lifetime() {
        assert!(
            (SKIDMARK_FADE_FROM as usize) < SKIDMARK_LIFE as usize,
            "a fade threshold at or past the lifetime is unreachable"
        );
    }

    #[test]
    fn a_continuous_slide_fills_the_ring_and_stays_stable() {
        // Emitting every frame for longer than the ring is deep must not grow
        // the live count without bound.
        let mut marks = SkidmarkBuffer::new();
        for frame in 0..(MAX_SKIDMARKS * 3) {
            marks.emit(SOMEWHERE, frame as u16);
            marks.tick();
        }
        assert!(
            marks.live_count() <= MAX_SKIDMARKS,
            "live marks exceeded the ring capacity"
        );
        // Once the ring is saturated it stays saturated: a slot is reclaimed
        // only as it is reused, so the count sits at capacity less the one slot
        // whose life expires this frame.
        assert!(
            marks.live_count() >= MAX_SKIDMARKS - 1,
            "the ring never filled: only {} of {MAX_SKIDMARKS} slots live",
            marks.live_count()
        );
    }

    /// Smoke used to be emitted at the car's exact centre with zero velocity,
    /// so it never drifted clear of the opaque body quad drawn over it.
    #[test]
    fn smoke_drifts_away_from_where_it_was_emitted() {
        let mut particles = ParticleSystem::new();
        assert!(particles.emit_smoke(SOMEWHERE));
        let born = particles
            .pool
            .iter()
            .find(|p| p.ptype == ParticleType::TireSmoke)
            .expect("a puff must exist")
            .pos;
        assert_ne!(born, SOMEWHERE, "the puff was born inside the car");

        let mut moved = false;
        for _ in 0..8 {
            particles.tick();
            let now = particles
                .pool
                .iter()
                .find(|p| p.ptype == ParticleType::TireSmoke)
                .expect("the puff must still exist")
                .pos;
            if (now.y - born.y).abs() > Fixed::from_int(2) {
                moved = true;
            }
        }
        assert!(
            moved,
            "smoke never drifted; it sat on the car and was painted over"
        );
    }

    #[test]
    fn smoke_slows_as_it_disperses() {
        let mut particles = ParticleSystem::new();
        particles.emit_smoke(SOMEWHERE);
        let idx = particles
            .pool
            .iter()
            .position(|p| p.ptype == ParticleType::TireSmoke)
            .unwrap();
        let launch = particles.pool[idx].vel.y.raw().abs();
        for _ in 0..6 {
            particles.tick();
        }
        assert!(
            particles.pool[idx].vel.y.raw().abs() < launch,
            "smoke did not decelerate"
        );
    }

    /// `max_life` used to be written but never read; the renderer branched on
    /// absolute `life` bands, so appearance depended on the countdown value
    /// rather than on the particle's age.
    #[test]
    fn age_fraction_runs_from_birth_to_expiry() {
        let mut particles = ParticleSystem::new();
        particles.emit_smoke(SOMEWHERE);
        let idx = particles
            .pool
            .iter()
            .position(|p| p.ptype == ParticleType::TireSmoke)
            .unwrap();

        let birth = particles.pool[idx].age_fraction();
        assert_eq!(birth, 0, "a newborn particle reported as fully aged");

        // Tick to the last live frame. The puff is culled on the tick after
        // that, so the final live frame is `life == 1`, not `life == 0`.
        let mut previous = birth;
        for _ in 0..(SMOKE_LIFE - 1) {
            particles.tick();
            let age = particles.pool[idx].age_fraction();
            assert!(age >= previous, "age went backwards: {previous} then {age}");
            previous = age;
        }
        assert!(
            previous >= 240,
            "the puff never approached full age: {previous}"
        );
        // And once it is culled, the fraction saturates rather than wrapping.
        particles.tick();
        assert_eq!(
            particles.pool[idx].age_fraction(),
            255,
            "an expired particle did not report full age"
        );
    }

    #[test]
    fn age_fraction_is_monotonic_for_sparks_too() {
        let mut particles = ParticleSystem::new();
        particles.emit_sparks(SOMEWHERE, Vec2::new(Fixed::ONE, Fixed::ZERO));
        let idx = particles
            .pool
            .iter()
            .position(|p| p.ptype == ParticleType::Sparks)
            .unwrap();
        assert_eq!(particles.pool[idx].age_fraction(), 0);
        for _ in 0..SPARK_LIFE {
            particles.tick();
        }
        assert_eq!(particles.live_count(), 0, "sparks outlived their lifetime");
    }

    /// A dead particle must report a usable fraction rather than dividing by a
    /// zero `max_life`.
    #[test]
    fn a_dead_particle_reports_full_age_instead_of_dividing_by_zero() {
        let system = ParticleSystem::new();
        let p = system.pool[0];
        assert_eq!(p.max_life, 0);
        assert_eq!(p.age_fraction(), 255);
    }

    #[test]
    fn sparks_fly_outward_from_the_contact_normal() {
        let mut particles = ParticleSystem::new();
        let emitted = particles.emit_sparks(SOMEWHERE, Vec2::new(Fixed::ZERO, Fixed::ONE));
        assert_eq!(emitted, 3, "all three spark directions must be emitted");
        for p in particles.pool.iter().filter(|p| p.ptype.is_live()) {
            assert!(p.vel.length() > Fixed::ZERO, "a spark was emitted at rest");
        }
    }

    #[test]
    fn the_pool_does_not_overflow() {
        let mut particles = ParticleSystem::new();
        for _ in 0..(MAX_PARTICLES + 20) {
            particles.emit_smoke(SOMEWHERE);
        }
        assert_eq!(
            particles.live_count(),
            MAX_PARTICLES,
            "the pool exceeded its capacity instead of refusing new particles"
        );
    }

    #[test]
    fn exhausted_particles_are_reclaimed() {
        let mut particles = ParticleSystem::new();
        particles.emit_smoke(SOMEWHERE);
        assert_eq!(particles.live_count(), 1);
        for _ in 0..SMOKE_LIFE {
            particles.tick();
        }
        assert_eq!(particles.live_count(), 0);
        assert!(
            particles.emit_smoke(SOMEWHERE),
            "a full pool of dead particles stopped accepting new ones"
        );
    }
}
