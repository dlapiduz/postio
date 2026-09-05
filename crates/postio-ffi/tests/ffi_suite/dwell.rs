//! Resting on a message marks it read; passing over it does not (#71, #1159).
//!
//! The macOS frontend marked nothing at all until #1159 — no dwell path
//! existed, so a Mac's inbox count never moved however much mail was read.
//! These hold the boundary half: that the verb reaches the store, and that it
//! is the *dwell* verb rather than the toggle, which is a different thing with
//! different undo behaviour.

use chrono::Utc;
use postio_core::bridge::Bridge;
use postio_core::dispatch::Dispatcher;
use postio_core::state::SharedState;
use postio_ffi::{ScopeFfi, Session, SessionOptions};
use postio_model::{Flag, Message};
use postio_storage::repository::MessageRepository;
use postio_storage::test_support;

/// Whether the store says `message` carries `\Seen`.
fn is_read(database: &postio_storage::Database, message: i64) -> bool {
    let connection = database.connection().expect("a connection");
    MessageRepository::new(&connection)
        .get(postio_model::ids::MessageId::new(message))
        .expect("a read")
        .is_some_and(|message| message.flags.contains(&Flag::Seen))
}

/// A store with one unread message, and the session over it.
fn one_unread() -> (std::sync::Arc<Session>, postio_storage::Database, i64) {
    let database = test_support::memory();
    let (mailbox, message) = {
        let connection = database.connection().expect("a connection");
        let (account, inbox) = test_support::account_with_inbox(&connection);
        let repository = MessageRepository::new(&connection);
        let mut message = Message::new(account.id, inbox, Utc::now());
        message.flags.remove(&Flag::Seen);
        repository.create(&mut message).expect("a message");
        (inbox, message.id.get())
    };
    // The real action handlers on the bus. An in-memory session's default
    // bridge takes commands and drops them, so a verb dispatched against it
    // would prove only that nothing panicked.
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
    .expect("a session over the store");
    session.open_scope(ScopeFfi::Mailbox {
        mailbox: mailbox.into(),
    });
    let _ = session.row_at(0);
    session.settle_for_test();
    (session, database, message)
}

#[test]
fn a_dwell_marks_the_message_read() {
    let (session, database, message) = one_unread();
    assert!(!is_read(&database, message), "the fixture starts unread");

    session.mark_read_on_dwell(message);
    assert!(
        settle_until(|| is_read(&database, message)),
        "the cursor rested on a message and it was never marked read"
    );
    session.shutdown();
}

#[test]
fn the_delay_is_the_shared_one() {
    // Not a number chosen in a frontend. #71's rule is that the delay
    // separates a sweep from a read, and two frontends that picked their own
    // would separate them differently — on the one rule where being wrong
    // destroys unread state rather than merely looking wrong.
    let session = Session::open(SessionOptions::in_memory()).expect("a session");
    assert_eq!(
        session.dwell_milliseconds_ffi(),
        postio_ui::dwell::DWELL_TO_READ.as_millis() as u64
    );
    session.shutdown();
}

/// Drive until `done`, or give up. The verb is local-first: it writes and
/// returns, and the write lands on the runtime a moment later.
fn settle_until(done: impl Fn() -> bool) -> bool {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while std::time::Instant::now() < deadline {
        if done() {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    done()
}
