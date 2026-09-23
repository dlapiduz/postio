//! The conversation rail, as the Mac draws it (#1576, #1595).
//!
//! The rules are `postio_ui::reader::rail`'s and are tested there. What is
//! tested here is that they cross whole: the ladder, the rows, and the one
//! state machine every way of moving the mark goes through -- so a Mac and a
//! Linux desktop cannot disagree about where `J` lands or when the observer
//! is listened to.

use postio_ffi::{RailFfi, RailPresentationFfi, rail_presentation};

#[test]
fn the_ladder_is_the_shared_one() {
    assert_eq!(
        rail_presentation(1400, 6, false),
        Some(RailPresentationFfi::Full)
    );
    assert_eq!(
        rail_presentation(1200, 6, false),
        Some(RailPresentationFfi::Narrow)
    );
    assert_eq!(
        rail_presentation(900, 6, false),
        Some(RailPresentationFfi::Popover)
    );
    assert_eq!(
        rail_presentation(1400, 1, false),
        None,
        "one message has nothing to index"
    );
    assert_eq!(rail_presentation(1400, 6, true), None, "`⇧I` hides it");
}

#[test]
fn choosing_a_message_marks_it_and_the_pane_follows() {
    let rail = RailFfi::new(4);
    let effect = rail.activate(2);
    assert!(effect.mark_moved);
    assert!(effect.scroll.is_some(), "the pane has to be taken there");
    assert_eq!(rail.marked(), Some(2));
}

#[test]
fn the_observer_is_not_listened_to_while_the_pane_is_being_taken_somewhere() {
    // What it sees mid-scroll is the pane passing over messages on its way
    // to the one the reader chose -- listened to, the mark would flicker
    // back, or land short when the chosen message is too short to fill the
    // pane.
    let rail = RailFfi::new(4);
    let scroll = rail.activate(3).scroll.expect("a scroll");
    let effect = rail.observed(Some(2));
    assert!(!effect.mark_moved);
    assert_eq!(rail.marked(), Some(3));

    rail.settled(scroll);
    assert!(
        rail.observed(Some(2)).mark_moved,
        "once it settles, the observer speaks"
    );
    assert_eq!(rail.marked(), Some(2));
}

#[test]
fn a_late_settle_for_an_older_scroll_does_not_end_a_newer_ones_suppression() {
    let rail = RailFfi::new(5);
    let first = rail.activate(1).scroll.expect("a scroll");
    let _second = rail.activate(4).scroll.expect("a scroll");
    rail.settled(first);
    assert!(!rail.observed(Some(2)).mark_moved);
    assert_eq!(rail.marked(), Some(4));
}

#[test]
fn j_and_k_walk_and_stop_at_the_ends() {
    let rail = RailFfi::new(3);
    assert!(rail.next_message().mark_moved);
    assert_eq!(
        rail.marked(),
        Some(0),
        "`J` from nowhere starts at the first"
    );
    rail.activate(2);
    assert!(!rail.next_message().mark_moved, "and stops at the last");
    let fresh = RailFfi::new(3);
    fresh.previous_message();
    assert_eq!(
        fresh.marked(),
        Some(2),
        "`K` from nowhere starts at the last"
    );
}

#[test]
fn a_new_conversation_starts_unmarked() {
    let rail = RailFfi::new(3);
    rail.activate(1);
    rail.set_conversation(5);
    assert_eq!(rail.marked(), None);
}
