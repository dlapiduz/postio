//! #487: the conversation pane never showed who a message went to.
//!
//! `reading.rs` wires each expanded entry's reader through `fill_reader`,
//! which discarded the envelope's `to`/`cc` entirely because the reader's
//! whole header was hidden to avoid repeating the entry's own
//! sender/subject/date. This drives the real composition root
//! (`feed_the_window`) rather than a stub factory, because the bug was in
//! that wiring, not in `ConversationView` or `MessageHeader` in isolation —
//! both already had everything they needed.
//!
//! One test function, for the reason `thread_cursor_preview.rs` gives.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe: it races any other thread reading
// the environment. This test sets it before the app under test starts, which
// is the one moment it is sound. The crate's library code forbids `unsafe`.

use crate::settle;
use gtk::prelude::*;
use gtk::gdk;
use postio_app::feed_the_window;
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_model::ids::ThreadId;
use postio_model::{EmailAddress, Message, Thread};
use postio_session::Wiring;
use postio_storage::repository::{MessageRepository, ThreadRepository};
use postio_storage::{Database, test_support};



fn settle_until(done: impl Fn() -> bool) -> bool {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while std::time::Instant::now() < deadline {
        settle();
        if done() {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    done()
}

fn walk(widget: gtk::Widget, visit: &mut impl FnMut(&gtk::Widget)) {
    visit(&widget);
    let mut child = widget.first_child();
    while let Some(current) = child {
        walk(current.clone(), visit);
        child = current.next_sibling();
    }
}

/// Every visible label under `widget` carrying `css_class`, in tree order.
fn visible_labels_with_class(widget: &gtk::Widget, css_class: &str) -> Vec<String> {
    let mut found = Vec::new();
    walk(widget.clone(), &mut |widget| {
        if let Ok(label) = widget.clone().downcast::<gtk::Label>()
            && widget.is_visible()
            && widget.has_css_class(css_class)
        {
            found.push(label.text().to_string());
        }
    });
    found
}

/// One message to seed into a thread: when it arrived, and what it says.
struct ThreadedMessage<'a> {
    minute: i64,
    subject: &'a str,
    to: &'a [&'a str],
    cc: &'a [&'a str],
}

fn threaded_message(
    database: &Database,
    account: postio_model::ids::AccountId,
    mailbox: postio_model::ids::MailboxId,
    thread: ThreadId,
    spec: ThreadedMessage<'_>,
) {
    let connection = database.connection().expect("a connection");
    let mut message = Message::new(
        account,
        mailbox,
        chrono::Utc::now() + chrono::Duration::minutes(spec.minute),
    );
    message.from = vec![EmailAddress::new(Some("Ada Lovelace"), "ada@example.com")];
    message.to = spec
        .to
        .iter()
        .map(|address| EmailAddress::new(None::<String>, *address))
        .collect();
    message.cc = spec
        .cc
        .iter()
        .map(|address| EmailAddress::new(None::<String>, *address))
        .collect();
    message.subject = Some(spec.subject.to_owned());
    let id = MessageRepository::new(&connection)
        .create(&mut message)
        .expect("create the threaded message");
    ThreadRepository::new(&connection)
        .add_message(thread, id)
        .expect("join the message to the thread");
}

pub fn an_expanded_entry_shows_who_it_went_to_without_repeating_its_header() {
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

    let database = test_support::memory();
    let (account, inbox) = {
        let connection = database.connection().expect("a connection");
        test_support::account_with_inbox(&connection)
    };
    let thread = {
        let connection = database.connection().expect("a connection");
        let mut thread = Thread::new(account.id);
        ThreadRepository::new(&connection)
            .create(&mut thread)
            .expect("create the thread")
    };
    // Two messages, two different recipient sets -- the point of a
    // conversation is exactly that this changes message to message.
    threaded_message(
        &database,
        account.id,
        inbox,
        thread,
        ThreadedMessage {
            minute: 0,
            subject: "the first message",
            to: &["grace@example.com"],
            cc: &[],
        },
    );
    threaded_message(
        &database,
        account.id,
        inbox,
        thread,
        ThreadedMessage {
            minute: 1,
            subject: "the second message",
            to: &["bob@example.com"],
            cc: &["carol@example.org"],
        },
    );

    let directory = tempfile::tempdir().expect("a blob directory");
    let blobs =
        postio_storage::BlobStore::open(directory.path().to_path_buf()).expect("a blob store");

    let (bridge, _replies) =
        postio_core::bridge::Bridge::new(postio_core::bridge::handler_fn(|_, _| async {}))
            .expect("a runtime");
    let (sink, _events) = postio_core::bridge::event_channel();
    let wiring = Wiring::new(
        database.clone(),
        blobs.clone(),
        bridge.handle(),
        sink,
        bridge.commands(),
    );

    let window = Window::default();
    window.present();
    settle();

    let _wired = feed_the_window(&window, &wiring).expect("the seeded store has an account");
    let list = window.list();
    assert!(
        settle_until(|| list.model().n_items() >= 1),
        "the fixture's conversation never reached the list"
    );

    list.first_row();
    let cursor = list.cursor_row().expect("a row to drill into");
    window.open_thread(&cursor);
    assert!(
        settle_until(|| window.conversation().len() == 2),
        "opening the thread never filled the reading pane with the \
         conversation"
    );

    let first_in_thread = window
        .thread()
        .rows()
        .first()
        .expect("the thread has a first row")
        .id;
    let second_in_thread = window.thread().rows()[1].id;

    // Both messages are unread, so both expand (well under the cap) and
    // both should be checkable.
    assert!(
        settle_until(|| window.conversation().is_expanded(first_in_thread)
            && window.conversation().is_expanded(second_in_thread)),
        "both fixture messages should have opened expanded"
    );

    // ── each expanded entry's reader gets its own recipients ─────────────
    for (message, expected_to) in [
        (first_in_thread, "grace@example.com"),
        (second_in_thread, "bob@example.com"),
    ] {
        assert!(
            settle_until(|| {
                window
                    .conversation()
                    .test_expanded_widget(message)
                    .is_some_and(|widget| {
                        !visible_labels_with_class(&widget, "postio-message-header-recipients")
                            .is_empty()
                    })
            }),
            "message {message:?}'s recipients never appeared in its reader"
        );
        let widget = window
            .conversation()
            .test_expanded_widget(message)
            .expect("the message is expanded");

        let recipients = visible_labels_with_class(&widget, "postio-message-header-recipients");
        assert!(
            recipients.iter().any(|line| line.contains(expected_to)),
            "message {message:?} should show {expected_to}, got {recipients:?}"
        );

        // ── and does not repeat the entry's own sender/subject/date ──────
        let subjects = visible_labels_with_class(&widget, "postio-message-header-subject");
        assert!(
            subjects.is_empty(),
            "the reader's own subject line must stay hidden inside a \
             conversation entry: {subjects:?}"
        );
        let senders = visible_labels_with_class(&widget, "postio-message-header-sender");
        assert!(
            senders.is_empty(),
            "the reader's own sender line must stay hidden inside a \
             conversation entry: {senders:?}"
        );
    }

    // ── the second message's Cc is reachable but not shown by default ────
    let second_widget = window
        .conversation()
        .test_expanded_widget(second_in_thread)
        .expect("the second message is expanded");
    let mut cc_toggle = None;
    walk(second_widget.clone(), &mut |widget| {
        if let Ok(button) = widget.clone().downcast::<gtk::ToggleButton>()
            && widget.is_visible()
        {
            cc_toggle = Some(button);
        }
    });
    let cc_toggle = cc_toggle.expect("a Cc disclosure for the message that has one");
    assert!(
        !cc_toggle.is_active(),
        "Cc must not cost space until it is asked for"
    );

    window.close();
}
