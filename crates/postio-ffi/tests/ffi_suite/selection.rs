//! The cursor, the selection, and the fact that they are not the same thing.
//!
//! `docs/PRODUCT.md` §9: **the cursor is not the selection.** Moving down the
//! list shows a message without adding it to anything; space toggles; shift
//! extends. `NSTableView` has its own idea of selection and will conflate the
//! two if left to itself, which is what makes shift-click destroy what the
//! user had built up — so the model lives here and the table is a *view* of
//! it.
//!
//! And `Everything { except }` is not an optimisation. *"Select all" is a
//! predicate, not a hundred thousand ids*: an action on it has to reach the
//! engine as a predicate too, or archiving a large mailbox means marshalling
//! every id across this boundary to do it.

use chrono::Utc;
use postio_ffi::{ScopeFfi, Session, SessionOptions};
use postio_model::Message;
use postio_storage::repository::MessageRepository;
use postio_storage::test_support;

/// A store with `count` messages in an inbox, opened and paged in.
///
/// Paged deliberately: a selection is about rows, and a window with nothing
/// resident cannot tell "row 3 is not selected" from "row 3 is not here".
fn listed(count: u32) -> std::sync::Arc<Session> {
    let database = test_support::memory();
    let mailbox = {
        let connection = database.connection().expect("a connection");
        let (account, inbox) = test_support::account_with_inbox(&connection);
        let repository = MessageRepository::new(&connection);
        for _ in 0..count {
            let mut message = Message::new(account.id, inbox, Utc::now());
            repository.create(&mut message).expect("a message");
        }
        inbox
    };
    let session =
        Session::open(SessionOptions::in_memory_with(database)).expect("a session over the store");
    session.open_scope(ScopeFfi::Mailbox {
        mailbox: mailbox.into(),
    });
    // The first ask misses and fetches behind the caller; settle so the rows
    // this file talks about are actually there.
    let _ = session.row_at(0);
    session.settle_for_test();
    session
}

/// The id at `row`, which by this point has arrived.
fn id_at(session: &Session, row: u32) -> i64 {
    session.row_at(row).expect("the page has landed").id
}

#[test]
fn moving_the_cursor_selects_nothing() {
    let session = listed(20);
    // The rule the whole file exists for. A list that built a selection as
    // the cursor moved would make `a` archive everything walked past, which
    // is the failure mode `PRODUCT.md` §9 is written against.
    for _ in 0..5 {
        session.invoke("next_message");
    }
    assert!(
        session.cursor_row().is_some(),
        "the cursor never moved, so this proves nothing"
    );
    assert_eq!(
        session.selected_messages(),
        Some(Vec::new()),
        "walking the list built a selection"
    );
    session.shutdown();
}

#[test]
fn space_toggles_the_row_the_cursor_is_on() {
    let session = listed(20);
    session.invoke("next_message");
    let first = session.cursor_message().expect("a cursor");

    session.invoke("toggle_selection");
    assert_eq!(session.selected_messages(), Some(vec![first]));
    assert!(session.is_selected(first));

    // ...and toggles it back off, rather than only ever adding.
    session.invoke("toggle_selection");
    assert_eq!(session.selected_messages(), Some(Vec::new()));
    assert!(!session.is_selected(first));
    session.shutdown();
}

#[test]
fn the_cursor_keeps_moving_over_a_selection_without_disturbing_it() {
    let session = listed(20);
    session.invoke("next_message");
    let marked = session.cursor_message().expect("a cursor");
    session.invoke("toggle_selection");

    for _ in 0..4 {
        session.invoke("next_message");
    }
    assert_ne!(
        session.cursor_message(),
        Some(marked),
        "the cursor did not move, so the assertion below is vacuous"
    );
    assert_eq!(
        session.selected_messages(),
        Some(vec![marked]),
        "moving the cursor changed what was marked"
    );
    session.shutdown();
}

#[test]
fn shift_extends_from_where_the_selection_started() {
    let session = listed(20);
    session.invoke("next_message");
    let anchor = session.cursor_message().expect("a cursor");

    for _ in 0..3 {
        session.invoke("extend_selection_down");
    }
    let marked = session
        .selected_messages()
        .expect("a list, not a predicate");
    assert_eq!(marked.len(), 4, "shift+j three times marks four rows");
    assert!(marked.contains(&anchor), "the anchor is part of the range");
    assert!(
        marked.contains(&session.cursor_message().expect("a cursor")),
        "the row the cursor ended on is part of the range"
    );
    session.shutdown();
}

#[test]
fn select_all_is_a_predicate_rather_than_every_id() {
    let session = listed(20);
    session.invoke("select_all");
    // `None` is the whole point: there is no list to hand back. A boundary
    // that answered a vector here would have materialised the mailbox, which
    // at a hundred thousand rows is the thing this list exists not to do.
    assert_eq!(session.selected_messages(), None);
    assert_eq!(
        session.selection_summary(),
        Some("20 selected".to_string()),
        "the count comes from the model, which knows it without enumerating"
    );

    // Taking three rows out keeps it a predicate.
    for row in 0..3 {
        session.toggle_selection(id_at(&session, row));
    }
    assert_eq!(
        session.selected_messages(),
        None,
        "deselecting three rows turned the predicate into a list of ids"
    );
    assert_eq!(session.selection_summary(), Some("17 selected".to_string()));
    session.shutdown();
}

#[test]
fn changing_folder_drops_the_selection() {
    let session = listed(20);
    session.invoke("next_message");
    session.invoke("toggle_selection");
    assert!(!session.selected_messages().unwrap_or_default().is_empty());

    // "These twelve" means something else the moment the list does, and an
    // action carrying a selection across would land on mail the user cannot
    // see.
    session.open_scope(ScopeFfi::Flagged { account: 1 });
    assert_eq!(session.selected_messages(), Some(Vec::new()));
    assert_eq!(session.cursor_message(), None, "the cursor came across too");
    session.shutdown();
}

#[test]
fn escape_clears_the_selection_before_anything_else() {
    let session = listed(20);
    session.invoke("next_message");
    session.invoke("toggle_selection");
    session.invoke("back");
    assert_eq!(
        session.selected_messages(),
        Some(Vec::new()),
        "Escape with mail marked has to mean `unmark it`"
    );
    session.shutdown();
}
