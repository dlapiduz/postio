//! The aiming rules mean the same thing on this side of the FFI (#721).
//!
//! #589 moved "what does this gesture act on" into `postio_core::aim` so
//! that two frontends could not answer it differently, and left both as
//! adapters over it. Proving the *rule* once is easy — `aim`'s own tests do
//! it. The half a shared function cannot prove is the adapter: reading a
//! selection, a cursor and a row kind out of whatever model a frontend
//! keeps. Two adapters can call the same correct function and still
//! disagree, because they disagree about what they hand it.
//!
//! So `postio_core::aim::conformance_cases` is a table, and this is one of
//! its two readers: each case's rows are delivered into a real
//! `ListWindow<RowFfi>` — the model this boundary actually serves Swift
//! from — and the command that comes out is compared against the table.
//! `postio-app`'s `aiming.rs` fills the GTK list model from the same table
//! and asserts the same commands. Same rows, same gesture, same command, on
//! both sides.
//!
//! Nothing here touches the network or a display.

use postio_core::aim::{self, Aim, RowKind};
use postio_core::state::Selection;
use postio_ffi::RowFfi;
use postio_model::ids::MessageId;
use postio_ui::list::ListWindow;

/// A window holding exactly the rows a case describes.
///
/// Through `deliver`, not by reaching into the model: the point is that the
/// *shipped* row type answers `RowFacts` correctly, and a fixture that
/// bypassed `RowFfi` would prove nothing about the boundary.
fn window_of(rows: &[(MessageId, RowKind)]) -> ListWindow<RowFfi> {
    let mut window: ListWindow<RowFfi> = ListWindow::new();
    window.reset(rows.len() as u32);
    let delivered: Vec<RowFfi> = rows
        .iter()
        .map(|(id, kind)| {
            let (thread, is_thread) = match kind {
                RowKind::Thread(thread) => (Some(thread.get()), true),
                // A message row may still belong to a thread; what makes it a
                // *message* row is that it does not stand for one. Saying so
                // here is what keeps `is_thread` load-bearing rather than
                // inferable from `thread`.
                RowKind::Message => (None, false),
                RowKind::Missing => (None, false),
            };
            RowFfi {
                id: id.get(),
                thread,
                is_thread,
                ..sample_row()
            }
        })
        .collect();
    window.deliver(window.generation(), 0, delivered);
    window
}

/// The parts of a row this test does not care about.
fn sample_row() -> RowFfi {
    RowFfi {
        id: 0,
        thread: None,
        is_thread: false,
        from: None,
        subject: None,
        preview: None,
        received_at: 0,
        seen: true,
        flagged: false,
        answered: false,
        draft: false,
        has_attachments: false,
        thread_count: 1,
    }
}

#[test]
fn the_ffi_aims_every_gesture_the_way_the_shared_table_says() {
    for case in aim::conformance_cases() {
        // A row the case says is `Missing` is one no list holds, so it is
        // named by the cursor and left out of the window rather than
        // delivered as something.
        let held: Vec<(MessageId, RowKind)> = case
            .rows
            .iter()
            .filter(|(_, kind)| !matches!(kind, RowKind::Missing))
            .copied()
            .collect();
        let rows = window_of(&held);

        let aim = Aim {
            scope: None,
            selection: &case.selection,
            cursor: case.cursor,
            rows: &rows,
        };
        let command = aim::command_for(case.id, &aim);

        assert_eq!(
            command, case.expected,
            "{}: the FFI aimed {} at {command:?}, and the shared table says \
             {:?}",
            case.because, case.id, case.expected
        );
    }
}

/// The table has to be exercising the discriminator, not just agreeing with
/// itself: a `RowFfi` that reported every row as a message would still pass
/// the loop above if no case depended on a thread row.
#[test]
fn the_table_actually_distinguishes_a_conversation_row() {
    let thread_cases = aim::conformance_cases()
        .into_iter()
        .filter(|case| {
            case.rows
                .iter()
                .any(|(_, kind)| matches!(kind, RowKind::Thread(_)))
                && matches!(case.selection, Selection::These(ref marked) if marked.is_empty())
        })
        .count();
    assert!(
        thread_cases > 0,
        "no case aims at a conversation row with nothing marked, so this \
         suite would pass against a boundary that had lost the distinction"
    );
}

/// The rule reaching the bus, not just the rule being right.
///
/// Everything above is pure: it proves `RowFfi` reports what `aim` needs.
/// What it cannot prove is that `Session::invoke` is wired to any of it —
/// #589's fourth acceptance criterion, and the reason this issue exists.
/// So this drives the boundary's own entry point against a real dispatcher
/// and reads the outcome back out of SQLite.
mod through_the_boundary {
    use chrono::Utc;
    use postio_core::bridge::Bridge;
    use postio_core::dispatch::Dispatcher;
    use postio_core::state::SharedState;
    use postio_ffi::{ScopeFfi, Session, SessionOptions};
    use postio_model::{Flag, Message};
    use postio_storage::repository::{MessageRepository, ThreadRepository};
    use postio_storage::test_support;

    /// A session over a store holding one conversation of two messages, with
    /// the real action handlers on the bus.
    fn conversation() -> (
        std::sync::Arc<Session>,
        ScopeFfi,
        Vec<i64>,
        postio_storage::Database,
    ) {
        let database = test_support::memory();
        let (mailbox, members) = {
            let connection = database.connection().expect("a connection");
            let (account, inbox) = test_support::account_with_inbox(&connection);
            let messages = MessageRepository::new(&connection);
            let threads = ThreadRepository::new(&connection);
            let mut thread = postio_model::Thread::new(account.id);
            threads.create(&mut thread).expect("a thread");
            let mut members = Vec::new();
            for _ in 0..2 {
                let mut message = Message::new(account.id, inbox, Utc::now());
                let id = messages.create(&mut message).expect("a message");
                threads.add_message(thread.id, id).expect("membership");
                members.push(id.get());
            }
            (inbox, members)
        };

        let state = SharedState::default();
        let bus = postio_session::actions::wire(
            Dispatcher::builder(),
            postio_session::actions::Actions::new(database.clone(), state),
        )
        .build();
        let (bridge, _replies) = Bridge::new(bus).expect("a runtime");
        // Leaked on purpose: the bridge has to outlive the session, and this
        // process ends with the test.
        let bridge = Box::leak(Box::new(bridge));
        let session = Session::open(
            SessionOptions::in_memory_with(database.clone())
                .on_bridge(bridge.handle(), bridge.commands()),
        )
        .expect("a session over the seeded store");
        (
            session,
            ScopeFfi::Mailbox {
                mailbox: mailbox.into(),
            },
            members,
            database,
        )
    }

    fn is_flagged(database: &postio_storage::Database, message: i64) -> bool {
        let connection = database.connection().expect("a connection");
        MessageRepository::new(&connection)
            .get(postio_model::ids::MessageId::new(message))
            .expect("a read")
            .expect("the message is still there")
            .flags
            .contains(&Flag::Flagged)
    }

    #[test]
    fn invoking_a_verb_on_a_conversation_row_acts_on_the_conversation() {
        let (session, scope, members, database) = conversation();
        session.open_scope(scope);
        // Draw the row, the way a table does. Pages load when something asks
        // for them, so a session that has only opened a scope is holding no
        // rows at all -- and a cursor pointing at a row nobody holds is
        // `RowKind::Missing`, which is deliberately *not* guessed into a
        // conversation (#468). The frontend draws, then the user acts.
        session.row_at(0);
        session.settle_for_test();
        let row = session.row_at(0).expect("the first row is resident now");
        assert!(
            row.is_thread,
            "the fixture's folder row should stand for the conversation, or \
             this test is not about aiming at one"
        );

        // The cursor on the row, nothing marked: the gesture is about the
        // row, and the row stands for a conversation (ADR 0015 Q3).
        session.set_cursor(Some(row.id));
        session.invoke("flag");
        session.settle_for_test();

        let flagged = settle_until(|| members.iter().all(|id| is_flagged(&database, *id)));
        assert!(
            flagged,
            "flagging a conversation row reached the bus but acted on {} of \
             its {} messages -- the boundary aimed at the row's own message \
             instead of the thread it stands for",
            members
                .iter()
                .filter(|id| is_flagged(&database, **id))
                .count(),
            members.len()
        );
        session.shutdown();
    }

    #[test]
    fn an_id_this_build_does_not_know_is_ignored_rather_than_fatal() {
        // It arrives from another process. A boundary that panicked on a
        // typo would be one Swift could crash.
        let (session, scope, _members, _database) = conversation();
        session.open_scope(scope);
        session.invoke("no_such_command");
        session.invoke("");
        assert!(session.is_open(), "an unknown id took the session down");
        session.shutdown();
    }

    fn settle_until(done: impl Fn() -> bool) -> bool {
        let deadline = std::time::Instant::now()
            + postio_test_support::scaled(std::time::Duration::from_secs(5));
        while std::time::Instant::now() < deadline {
            if done() {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        done()
    }
}
