//! Back and forward between the inbox and search results (spec 010 US2
//! scenario 4, FR-020, D17, research R9).
//!
//! ⌘[ and ⌘] (and the swipe, which the Mac reports as the same commands)
//! move through what the main window has shown the way a browser moves
//! through pages: the inbox comes back as it was left, and so do the
//! results -- their query, tab, sort, focus ring and checked rows.

mod search_support;

use postio_core::CommandId;
use postio_focus::Intent;
use postio_search::results::{ConversationOrder, ResultsTab};
use postio_ui::keymap::KeyContext;
use search_support::*;

#[test]
fn entering_the_results_puts_the_inbox_behind_them() {
    let rows = List::of(3);
    let mut focus = mac();
    let _ = focus.landed(&rows, true);
    let _ = search(&mut focus, "atlas budget", &rows);
    assert!(focus.answers(CommandId::HistoryBack));
    let effects = run(&mut focus, CommandId::HistoryBack, &rows);
    assert_eq!(shown(&effects).first(), Some(&Intent::LeaveResults));
    assert!(!focus.in_results());
    assert_eq!(focus.key_context(), KeyContext::List);
}

#[test]
fn back_shows_the_inbox_on_its_cursor_and_selection() {
    let rows = List::of(3);
    let mut focus = mac();
    let _ = focus.landed(&rows, true);
    let _ = run(&mut focus, CommandId::NextMessage, &rows);
    let _ = run(&mut focus, CommandId::NextMessage, &rows);
    let _ = run(&mut focus, CommandId::ToggleSelection, &rows);
    let selection = focus.selection();

    let _ = search(&mut focus, "atlas budget", &rows);
    // The results' keys are the results': the inbox behind does not move.
    let _ = run(&mut focus, CommandId::NextMessage, &rows);
    let _ = run(&mut focus, CommandId::ToggleSelection, &rows);

    let effects = run(&mut focus, CommandId::HistoryBack, &rows);
    let intents = shown(&effects);
    assert!(intents.contains(&Intent::Cursor {
        position: 2,
        to_top: false
    }));
    assert!(intents.iter().any(|intent| matches!(
        intent,
        Intent::Selection { selection: shown, .. } if *shown == selection
    )));
    assert_eq!(focus.cursor(), Some(2));
    assert_eq!(focus.selection(), selection);
}

#[test]
fn forward_brings_the_results_back_as_they_were_left() {
    let rows = List::of(3);
    let mut focus = mac();
    let _ = search(&mut focus, "atlas budget", &rows);
    let effects = run(&mut focus, CommandId::ToggleResultOrder, &rows);
    let _ = settle(&mut focus, effects, &rows);
    let _ = run(&mut focus, CommandId::NextMessage, &rows);
    let _ = run(&mut focus, CommandId::NextMessage, &rows);
    let _ = run(&mut focus, CommandId::ToggleSelection, &rows);
    let checked = focus.result_row(2).expect("the third row").message;

    let _ = run(&mut focus, CommandId::HistoryBack, &rows);
    assert!(!focus.in_results());
    let effects = run(&mut focus, CommandId::HistoryForward, &rows);
    assert!(focus.in_results());
    assert_eq!(
        query_view(&effects).expect("the field").words,
        "atlas budget"
    );
    let effects = settle(&mut focus, effects, &rows);
    let view = results_view(&effects).expect("the results");
    assert_eq!(view.order, ConversationOrder::Newest, "the sort kept");
    assert_eq!(view.cursor, Some(2), "the focus ring where it was");
    assert_eq!(view.selected, 1);
    let row = focus.result_row(2).expect("the third row");
    assert_eq!(row.message, checked);
    assert!(row.checked, "and still checked");
}

#[test]
fn forward_keeps_the_tab() {
    let rows = List::of(3);
    let mut focus = mac();
    let _ = search(&mut focus, "atlas budget", &rows);
    let effects = run(&mut focus, CommandId::ResultsFiles, &rows);
    let view = results_view(&effects).expect("redrawn");
    assert!(view.tabs[1].selected && !view.tabs[0].selected);

    let _ = run(&mut focus, CommandId::HistoryBack, &rows);
    let effects = run(&mut focus, CommandId::HistoryForward, &rows);
    let effects = settle(&mut focus, effects, &rows);
    let view = results_view(&effects).expect("the results");
    assert_eq!(
        view.tabs.iter().position(|tab| tab.selected),
        Some(1),
        "Files, as left"
    );
    let _ = ResultsTab::Files;
}

#[test]
fn a_new_search_after_going_back_drops_what_was_ahead() {
    let rows = List::of(3);
    let mut focus = mac();
    let _ = search(&mut focus, "atlas", &rows);
    let _ = run(&mut focus, CommandId::HistoryBack, &rows);
    let _ = search(&mut focus, "harbor", &rows);
    let _ = run(&mut focus, CommandId::HistoryBack, &rows);
    let effects = run(&mut focus, CommandId::HistoryForward, &rows);
    assert_eq!(query_view(&effects).expect("the field").words, "harbor");
    assert!(
        run(&mut focus, CommandId::HistoryForward, &rows).is_empty(),
        "atlas is gone from ahead"
    );
}

#[test]
fn a_search_from_the_results_goes_behind_them() {
    let rows = List::of(3);
    let mut focus = mac();
    let _ = search(&mut focus, "atlas", &rows);
    let _ = search(&mut focus, "harbor", &rows);
    let effects = run(&mut focus, CommandId::HistoryBack, &rows);
    assert!(focus.in_results());
    assert_eq!(query_view(&effects).expect("the field").words, "atlas");
    let effects = run(&mut focus, CommandId::HistoryBack, &rows);
    assert_eq!(shown(&effects).first(), Some(&Intent::LeaveResults));
}

#[test]
fn history_keeps_fifty_entries() {
    let rows = List::of(3);
    let mut focus = mac();
    for n in 0..60 {
        let _ = search(&mut focus, &format!("atlas {n}"), &rows);
    }
    let mut steps = 0;
    while !run(&mut focus, CommandId::HistoryBack, &rows).is_empty() {
        steps += 1;
        assert!(steps <= 60, "back never ends");
    }
    assert_eq!(steps, 50);
    assert!(focus.in_results(), "the inbox was the oldest, and let go");
}

#[test]
fn back_with_nothing_behind_does_nothing() {
    let rows = List::of(3);
    let mut focus = mac();
    let _ = focus.landed(&rows, true);
    assert!(run(&mut focus, CommandId::HistoryBack, &rows).is_empty());
    assert!(run(&mut focus, CommandId::HistoryForward, &rows).is_empty());
    assert!(!focus.in_results());
}

#[test]
fn esc_from_a_search_run_from_the_results_goes_to_the_inbox() {
    // Esc's last rung is the inbox (D18), not the search before; ⌘] comes
    // back to the results Esc left.
    let rows = List::of(3);
    let mut focus = mac();
    let _ = focus.landed(&rows, true);
    let _ = search(&mut focus, "atlas", &rows);
    let _ = search(&mut focus, "harbor", &rows);
    let effects = run(&mut focus, CommandId::Back, &rows);
    assert_eq!(shown(&effects).first(), Some(&Intent::LeaveResults));
    assert!(!focus.in_results());
    let effects = run(&mut focus, CommandId::HistoryForward, &rows);
    assert_eq!(query_view(&effects).expect("the field").words, "harbor");
    let effects = run(&mut focus, CommandId::HistoryBack, &rows);
    assert_eq!(shown(&effects).first(), Some(&Intent::LeaveResults));
}
