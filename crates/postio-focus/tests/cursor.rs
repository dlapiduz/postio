//! The cursor, the selection and the has-action filter, from the
//! controller's side (specs/009-focus-macos T039; contract invariants 1-3).
//!
//! What GTK's window did with `SingleSelection` and `SelectionState`,
//! asserted here so both frontends get it: the cursor and the selection are
//! different things, `x` never moves the cursor, `J`/`K` walk over a digest
//! without taking it, `!` keeps the cursor on the same message, and every
//! list opens on its first row.

use postio_config::paths::Platform;
use postio_core::CommandId;
use postio_core::state::Selection;
use postio_focus::{Effect, FocusController, Input, Intent, Policy, RowFacts, Rows};
use postio_model::{AccountId, MessageId, ThreadId};

/// A list of conversations, with digests at the positions named.
struct List {
    rows: Vec<RowFacts>,
}

impl List {
    fn of(len: i64, digests: &[i64]) -> Self {
        List {
            rows: (0..len)
                .map(|at| RowFacts {
                    id: MessageId::new(100 + at),
                    digest: digests.contains(&at),
                    threads: vec![ThreadId::new(500 + at)],
                })
                .collect(),
        }
    }
}

impl Rows for List {
    fn len(&self) -> u32 {
        self.rows.len() as u32
    }
    fn facts(&self, position: u32) -> Option<RowFacts> {
        self.rows.get(position as usize).cloned()
    }
    fn position_of(&self, message: MessageId) -> Option<u32> {
        self.rows
            .iter()
            .position(|row| row.id == message)
            .map(|at| at as u32)
    }
}

fn focus() -> FocusController {
    FocusController::new(Policy::for_platform(Platform::Apple))
}

fn shown(effects: &[Effect]) -> Vec<Intent> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Show(intent) => Some(intent.clone()),
            _ => None,
        })
        .collect()
}

fn cursor_of(effects: &[Effect]) -> Option<u32> {
    shown(effects)
        .into_iter()
        .rev()
        .find_map(|intent| match intent {
            Intent::Cursor { position, .. } => Some(position),
            _ => None,
        })
}

fn selection_of(effects: &[Effect]) -> Option<Selection> {
    shown(effects)
        .into_iter()
        .rev()
        .find_map(|intent| match intent {
            Intent::Selection { selection, .. } => Some(selection),
            _ => None,
        })
}

fn run(focus: &mut FocusController, command: CommandId, rows: &List) -> Vec<Effect> {
    focus.handle_on(Input::Command(command), rows)
}

#[test]
fn a_list_that_lands_opens_on_its_first_row() {
    // C30: every list opens with the cursor on its first row.
    let rows = List::of(5, &[]);
    let mut focus = focus();
    let _ = focus.open(postio_model::ListScope::Focus(
        postio_model::FocusScope::Inbox,
    ));
    let effects = focus.landed(&rows, true);
    assert_eq!(cursor_of(&effects), Some(0));
    assert_eq!(focus.cursor(), Some(0));
}

#[test]
fn j_and_k_move_the_cursor_and_nothing_else() {
    let rows = List::of(3, &[]);
    let mut focus = focus();
    let _ = focus.landed(&rows, true);
    let effects = run(&mut focus, CommandId::NextMessage, &rows);
    assert_eq!(cursor_of(&effects), Some(1));
    assert_eq!(selection_of(&effects), None, "the selection is not touched");
    let _ = run(&mut focus, CommandId::NextMessage, &rows);
    let effects = run(&mut focus, CommandId::NextMessage, &rows);
    assert_eq!(cursor_of(&effects), Some(2), "clamped at the last row");
    let effects = run(&mut focus, CommandId::FirstMessage, &rows);
    assert_eq!(cursor_of(&effects), Some(0));
    let effects = run(&mut focus, CommandId::LastMessage, &rows);
    assert_eq!(cursor_of(&effects), Some(2));
}

#[test]
fn x_selects_the_cursors_row_without_moving_the_cursor() {
    let rows = List::of(3, &[]);
    let mut focus = focus();
    let _ = focus.landed(&rows, true);
    let effects = run(&mut focus, CommandId::ToggleSelection, &rows);
    assert_eq!(cursor_of(&effects), None, "x never moves the cursor");
    assert_eq!(
        selection_of(&effects),
        Some(Selection::These(vec![MessageId::new(100)]))
    );
    let effects = run(&mut focus, CommandId::ToggleSelection, &rows);
    assert_eq!(selection_of(&effects), Some(Selection::These(Vec::new())));
}

#[test]
fn x_on_a_digest_takes_nothing() {
    let rows = List::of(3, &[0]);
    let mut focus = focus();
    let _ = focus.landed(&rows, true);
    let effects = run(&mut focus, CommandId::ToggleSelection, &rows);
    assert_eq!(selection_of(&effects), None);
}

#[test]
fn shift_j_extends_over_a_digest_without_taking_it() {
    let rows = List::of(4, &[1]);
    let mut focus = focus();
    let _ = focus.landed(&rows, true);
    let _ = run(&mut focus, CommandId::ExtendSelectionDown, &rows);
    let effects = run(&mut focus, CommandId::ExtendSelectionDown, &rows);
    assert_eq!(cursor_of(&effects), Some(2));
    assert_eq!(
        selection_of(&effects),
        Some(Selection::These(vec![
            MessageId::new(100),
            MessageId::new(102)
        ])),
        "rows 0 and 2; the digest at 1 is walked over"
    );
}

#[test]
fn capital_x_selects_everything_as_a_predicate() {
    // C19: a predicate over the view, not the rows on screen.
    let rows = List::of(3, &[]);
    let mut focus = focus();
    let _ = focus.handle(Input::Accounts(vec![AccountId::new(1)]));
    let _ = focus.landed(&rows, true);
    let effects = run(&mut focus, CommandId::SelectAll, &rows);
    assert!(matches!(
        selection_of(&effects),
        Some(Selection::Everything { .. })
    ));
}

#[test]
fn back_clears_the_selection_and_leaves_the_cursor() {
    let rows = List::of(3, &[]);
    let mut focus = focus();
    let _ = focus.landed(&rows, true);
    let _ = run(&mut focus, CommandId::NextMessage, &rows);
    let _ = run(&mut focus, CommandId::ToggleSelection, &rows);
    let effects = run(&mut focus, CommandId::Back, &rows);
    assert_eq!(selection_of(&effects), Some(Selection::These(Vec::new())));
    assert_eq!(cursor_of(&effects), None);
    assert_eq!(focus.cursor(), Some(1));
}

#[test]
fn has_action_clears_the_selection_and_keeps_the_cursor_on_its_message() {
    let inbox = List::of(5, &[]);
    let mut focus = focus();
    let _ = focus.open(postio_model::ListScope::Focus(
        postio_model::FocusScope::Inbox,
    ));
    let _ = focus.landed(&inbox, true);
    let _ = run(&mut focus, CommandId::NextMessage, &inbox);
    let _ = run(&mut focus, CommandId::NextMessage, &inbox);
    let _ = run(&mut focus, CommandId::ToggleSelection, &inbox);

    let effects = run(&mut focus, CommandId::ToggleHasAction, &inbox);
    assert_eq!(selection_of(&effects), Some(Selection::These(Vec::new())));
    assert!(focus.has_action());
    assert!(
        effects.iter().any(|effect| matches!(
            effect,
            Effect::Ask(
                _,
                postio_focus::Request::OpenScope {
                    scope: postio_model::ListScope::Focus(postio_model::FocusScope::HasAction),
                    ..
                }
            )
        )),
        "the narrowed list is opened"
    );

    // The narrowed list holds message 102 at position 0.
    let narrowed = List {
        rows: vec![RowFacts {
            id: MessageId::new(102),
            digest: false,
            threads: Vec::new(),
        }],
    };
    let effects = focus.landed(&narrowed, true);
    assert_eq!(cursor_of(&effects), Some(0), "on the same message");

    // Off again: the message is at position 2 of the inbox.
    let _ = run(&mut focus, CommandId::ToggleHasAction, &narrowed);
    let effects = focus.landed(&inbox, true);
    assert_eq!(cursor_of(&effects), Some(2));
}

#[test]
fn a_kept_message_that_is_gone_puts_the_cursor_on_the_first_row() {
    let inbox = List::of(5, &[]);
    let mut focus = focus();
    let _ = focus.open(postio_model::ListScope::Focus(
        postio_model::FocusScope::Inbox,
    ));
    let _ = focus.landed(&inbox, true);
    let _ = run(&mut focus, CommandId::LastMessage, &inbox);
    let _ = run(&mut focus, CommandId::ToggleHasAction, &inbox);
    let empty_of_it = List::of(2, &[]);
    let effects = focus.landed(&empty_of_it, true);
    assert_eq!(cursor_of(&effects), Some(0));
}

#[test]
fn a_reread_that_is_not_an_opening_leaves_the_cursor_where_it_is() {
    let rows = List::of(5, &[]);
    let mut focus = focus();
    let _ = focus.landed(&rows, true);
    let _ = run(&mut focus, CommandId::NextMessage, &rows);
    let effects = focus.landed(&rows, false);
    assert_eq!(cursor_of(&effects), None);
    assert_eq!(focus.cursor(), Some(1));
}

#[test]
fn a_click_moves_the_cursor_and_leaves_the_selection() {
    // The pointer's cursor is the keyboard's: a click on a row is where `a`
    // lands next, and it selects nothing (spec 007 FR-016).
    let rows = List::of(4, &[]);
    let mut focus = focus();
    let _ = focus.landed(&rows, true);
    let _ = run(&mut focus, CommandId::ToggleSelection, &rows);
    let effects = focus.handle_on(Input::Point(2), &rows);
    assert_eq!(cursor_of(&effects), Some(2));
    assert_eq!(selection_of(&effects), None, "the selection is not touched");
    assert_eq!(focus.cursor(), Some(2));
    let effects = focus.handle_on(Input::Point(9), &rows);
    assert!(effects.is_empty(), "a row off the list moves nothing");
}

#[test]
fn a_command_click_toggles_the_row_it_lands_on() {
    let rows = List::of(4, &[2]);
    let mut focus = focus();
    let _ = focus.landed(&rows, true);
    let effects = focus.handle_on(
        Input::Pick {
            position: 3,
            range: false,
        },
        &rows,
    );
    assert_eq!(cursor_of(&effects), Some(3));
    assert_eq!(
        selection_of(&effects),
        Some(Selection::These(vec![MessageId::new(103)]))
    );
    let effects = focus.handle_on(
        Input::Pick {
            position: 2,
            range: false,
        },
        &rows,
    );
    assert_eq!(cursor_of(&effects), Some(2), "the cursor still follows");
    assert_eq!(selection_of(&effects), None, "a digest is never taken");
}

#[test]
fn a_shift_click_takes_the_range_from_the_anchor_skipping_digests() {
    let rows = List::of(6, &[3]);
    let mut focus = focus();
    let _ = focus.landed(&rows, true);
    let _ = focus.handle_on(Input::Point(1), &rows);
    let effects = focus.handle_on(
        Input::Pick {
            position: 4,
            range: true,
        },
        &rows,
    );
    assert_eq!(cursor_of(&effects), Some(4));
    assert_eq!(
        selection_of(&effects),
        Some(Selection::These(vec![
            MessageId::new(101),
            MessageId::new(102),
            MessageId::new(104)
        ])),
        "rows 1 to 4, the digest at 3 walked over"
    );
    assert_eq!(
        shown(&effects)
            .iter()
            .filter(|intent| matches!(intent, Intent::Selection { .. }))
            .count(),
        1,
        "one gesture, one selection drawn"
    );
    // Upwards, from the same anchor: the gesture's start stays put.
    let effects = focus.handle_on(
        Input::Pick {
            position: 0,
            range: true,
        },
        &rows,
    );
    assert_eq!(cursor_of(&effects), Some(0));
    let Some(Selection::These(mut picked)) = selection_of(&effects) else {
        panic!("a range names its rows");
    };
    picked.sort();
    assert_eq!(
        picked,
        vec![
            MessageId::new(100),
            MessageId::new(101),
            MessageId::new(102),
            MessageId::new(104)
        ]
    );
}
