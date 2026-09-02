//! The aiming rules mean the same thing on this side of the FFI (#721).
//!
//! The other reader of `postio_core::aim::conformance_cases`;
//! `postio-ffi/tests/aiming.rs` is the first. Each case's rows are delivered
//! into the real GTK list model — the one `Window::act`'s adapter reads —
//! and the command that comes out is compared against the table. Same rows,
//! same gesture, same command, on both sides of the boundary.
//!
//! Why a table rather than two independent tests: #589 moved the rule into
//! `aim` so two frontends could not answer differently, and a test of `aim`
//! alone proves only the rule. The half it cannot reach is the *adapter* —
//! how each frontend reports a selection, a cursor, and what kind of row it
//! is holding. Two adapters can call the same correct function and still
//! disagree, because they disagree about what they hand it. GTK decides
//! "is this a conversation row" from a non-empty participant list; the FFI
//! decides it from an explicit `is_thread` flag. This is what keeps those
//! two answers the same answer.
//!
//! One test function, for the reason `wiring.rs` gives.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe: it races any other thread reading
// the environment. This test sets it before the app under test starts, which
// is the one moment it is sound. The crate's library code forbids `unsafe`.

use chrono::Utc;
use gtk::gdk;
use postio_core::aim::{self, Aim, RowKind};
use postio_gtk::list::{MessageList, Row};
use postio_gtk::{app, fonts, style};
use postio_model::EmailAddress;
use postio_model::ids::MessageId;

/// A GTK row of the kind a case describes.
///
/// The discriminator is the participant list, which is GTK's own answer to
/// "does this row stand for a conversation" — set here exactly as
/// `postio-app`'s feed sets it, so what is under test is the real rule and
/// not a flag invented for the fixture.
fn row_of(id: MessageId, kind: RowKind) -> Row {
    let (thread, participants) = match kind {
        RowKind::Thread(thread) => (
            Some(thread),
            vec![EmailAddress::new(Some("Ada Lovelace"), "ada@example.com")],
        ),
        RowKind::Message | RowKind::Missing => (None, Vec::new()),
    };
    Row {
        id,
        thread,
        from: Some(EmailAddress::new(Some("Ada Lovelace"), "ada@example.com")),
        subject: Some("the tide gate interlock".to_owned()),
        preview: None,
        received_at: Utc::now(),
        seen: true,
        flagged: false,
        answered: false,
        draft: false,
        has_attachments: false,
        thread_count: if participants.is_empty() { 1 } else { 2 },
        participants,
    }
}

pub fn the_gtk_adapter_aims_every_gesture_the_way_the_shared_table_says() {
    let state_dir = tempfile::tempdir().expect("a state directory");
    // SAFETY: first statement of a single-threaded test.
    unsafe { std::env::set_var("XDG_STATE_HOME", state_dir.path()) };

    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);
    app::install_icons(&display);

    let mut checked = 0;
    for case in aim::conformance_cases() {
        // A row the case calls `Missing` is one no list is holding, so it is
        // named by the cursor and never delivered — which is the state
        // `RowKind::Missing` exists for (#468).
        let held: Vec<Row> = case
            .rows
            .iter()
            .filter(|(_, kind)| !matches!(kind, RowKind::Missing))
            .map(|(id, kind)| row_of(*id, *kind))
            .collect();

        let model = MessageList::new();
        model.deliver_page(model.generation(), held.len() as u32, 0, held);

        let aim = Aim {
            scope: None,
            selection: &case.selection,
            cursor: case.cursor,
            rows: &model,
        };
        let command = aim::command_for(case.id, &aim);

        assert_eq!(
            command, case.expected,
            "{}: GTK aimed {} at {command:?}, and the shared table says {:?}",
            case.because, case.id, case.expected
        );
        checked += 1;
    }

    assert!(
        checked > 0,
        "the shared table is empty, so this proves nothing about either \
         frontend"
    );
}
