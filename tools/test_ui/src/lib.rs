//! Host-side tests for the game's hardware-free UI logic.
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
