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
}
