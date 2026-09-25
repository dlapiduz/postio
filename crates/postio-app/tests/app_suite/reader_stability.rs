//! The reading pane holds still while a person moves through mail.
//!
//! The maintainer's words: *"the interface feels glitchy"*, *"things jump
//! around"*. Every layer under the pane passed its own tests while the pane
//! drew a conversation two or three times on the way to showing it, put the
//! new thread's header over the old thread's document, and threw the reader
//! back to the top whenever a late body arrived. Those are all things a
//! person sees and no single layer can, so they are asserted here, through
//! the real composition root, on what reached the screen.
//!
//! Nothing here touches the network.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe: it races any other thread reading
// the environment. Each case sets it before the app under test starts, which
// is the one moment it is sound.

use crate::{settle, settle_until};
use gtk::gdk;
use gtk::prelude::*;
use postio_app::feed_the_window;
use postio_core::bridge::{Bridge, event_channel, handler_fn};
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_model::ids::{AccountId, MailboxId, MessageId, ThreadId};
use postio_session::Wiring;
use postio_storage::repository::{MessageRepository, StoredBody, ThreadRepository};
use postio_storage::{BlobStore, Store, test_support};

/// Where a seeded message goes.
struct Seat {
    account: AccountId,
    mailbox: MailboxId,
}

/// A thread of `count` messages in `seat`'s mailbox, every one with a body
/// that names the thread, `hours` hours ago.
async fn seed_thread(
    database: &Store,
    seat: &Seat,
    name: &str,
    count: usize,
    hours: i64,
) -> (ThreadId, Vec<MessageId>) {
    let connection = database.connect().await.expect("a connection");
    let mut thread = postio_model::Thread::new(seat.account);
    let thread = ThreadRepository::new(&connection)
        .create(&mut thread)
        .await
        .expect("create the thread");
    let mut ids = Vec::new();
    for index in 0..count {
        let mut message = postio_model::Message::new(
            seat.account,
            seat.mailbox,
            chrono::Utc::now() - chrono::Duration::hours(hours)
                + chrono::Duration::minutes(index as i64),
        );
        message.subject = Some(format!("{name} subject"));
        message.from = vec![postio_model::EmailAddress::new(
            Some("Ada Lovelace"),
            "ada@example.com",
        )];
        message.to = vec![postio_model::EmailAddress::new(
            Some("Grace Hopper"),
            "grace@example.com",
        )];
        message.sync.body_state = postio_model::BodyState::Full;
        message.flags.insert(postio_model::Flag::Seen);
        let id = MessageRepository::new(&connection)
            .create(&mut message)
            .await
            .expect("create the message");
        ThreadRepository::new(&connection)
            .add_message(thread, id)
            .await
            .expect("join the message to the thread");
        MessageRepository::new(&connection)
            .set_body(
                id,
                &StoredBody {
                    text: Some(body_of(name, index)),
                    html: None,
                    headers: None,
                    headers_truncated: false,
                    encoding_problems: false,
                },
                postio_model::BodyState::Full,
            )
            .await
            .expect("store the body");
        ids.push(id);
    }
    (thread, ids)
}

fn body_of(name: &str, index: usize) -> String {
    format!("the {name} body number {index}")
}

/// Whether `document` draws every message of the thread `name`, with its body.
fn holds_whole(document: &str, name: &str, count: usize) -> bool {
    document.matches("<details").count() == count
        && (0..count).all(|index| document.contains(&body_of(name, index)))
}

/// Which seeded thread a document is drawing, by the bodies it holds.
fn drawing(document: &str, names: &[&str]) -> Option<String> {
    names
        .iter()
        .find(|name| document.contains(&format!("the {name} body")))
        .map(|name| (*name).to_owned())
}

/// Turn the loop for `duration`, calling `each` on every turn.
async fn watch_for(duration: std::time::Duration, mut each: impl FnMut()) {
    let until = std::time::Instant::now() + postio_test_support::scaled(duration);
    while std::time::Instant::now() < until {
        settle();
        each();
        tokio::time::sleep(std::time::Duration::from_millis(2)).await;
    }
}

fn skip_without_display() -> bool {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return true;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);
    app::install_icons(&display);
    false
}

/// `j` onto a thread draws it once, whole, and never under another's header.
///
/// On the bug the pane drew the one row the list held as a document of its
/// own, then the whole thread when the thread read answered -- two full
/// WebKit loads, the second guaranteed to differ because the newest message
/// only carries its `latest` badge in a thread of more than one. And the
/// header changed the moment the cursor did, while the previous thread's
/// document stayed under it until the new one was drawn.
pub fn moving_onto_a_thread_draws_it_once_whole_and_under_its_own_header() {
    crate::gtk_case(async {
        let state_dir = tempfile::tempdir().expect("a state directory");
        // SAFETY: first statement of a single-threaded test, before the app runs.
        unsafe { std::env::set_var("XDG_STATE_HOME", state_dir.path()) };
        if skip_without_display() {
            return;
        }

        let database = test_support::memory().await;
        let directory = tempfile::tempdir().expect("a blob directory");
        let blobs = BlobStore::open(
            directory.path().to_path_buf(),
            &postio_storage::test_support::blob_keys(),
        )
        .expect("a blob store");
        let (account, inbox) = {
            let connection = database.connect().await.expect("a connection");
            test_support::account_with_inbox(&connection).await
        };
        let seat = Seat {
            account: account.id,
            mailbox: inbox,
        };
        // Newest first in the list: alpha is where the window lands, bravo is
        // one `j` below it.
        seed_thread(&database, &seat, "alpha", 3, 1).await;
        seed_thread(&database, &seat, "bravo", 3, 5).await;
        let names = ["alpha", "bravo"];

        let (bridge, _replies) = Bridge::new(handler_fn(|_, _| async {})).expect("a runtime");
        let (sink, _events) = event_channel();
        let wiring = Wiring::new(database, blobs, bridge.handle(), sink, bridge.commands());

        let window = Window::default();
        window.present();
        settle();
        let _wired = feed_the_window(&window, &wiring)
            .await
            .expect("the store has an account");

        let list = window.list();
        assert!(
            settle_until(async || list.model().n_items() == 2).await,
            "the two seeded threads never reached the list"
        );
        let pane = window.conversation();
        assert!(
            settle_until(async || pane
                .thread_document()
                .is_some_and(|document| holds_whole(&document, "alpha", 3)))
            .await,
            "the window never showed the first thread whole, so nothing below \
             can be attributed to the keystroke"
        );
        // Let anything still queued for alpha land before counting.
        watch_for(std::time::Duration::from_millis(500), || {}).await;

        let before = postio_ui::test_support::renders_issued();
        window.handle_key(gdk::Key::j, gdk::ModifierType::empty());

        // Every turn: the header and the document must be about the same
        // thread. The header is native and changes the instant it is told;
        // the document is whatever was last handed to WebKit.
        let mut mismatches = Vec::new();
        watch_for(std::time::Duration::from_millis(900), || {
            let subject = pane.header().subject();
            let document = pane.thread_document().unwrap_or_default();
            if let Some(shown) = drawing(&document, &names)
                && !subject.starts_with(&shown)
            {
                mismatches.push(format!("header {subject:?} over {shown}'s document"));
            }
        })
        .await;

        let document = pane.thread_document().unwrap_or_default();
        assert!(
            holds_whole(&document, "bravo", 3),
            "`j` never drew the second thread whole"
        );
        let renders = postio_ui::test_support::renders_issued() - before;
        assert_eq!(
            renders, 1,
            "one keystroke onto a three-message thread cost {renders} documents. \
             Each is a full teardown and reload; the list's one row drawn \
             first and the whole thread after it is two"
        );
        mismatches.dedup();
        assert!(
            mismatches.is_empty(),
            "the header and the document disagreed about which thread is open: \
             {mismatches:?}"
        );

        window.destroy();
        bridge.shutdown();
    });
}

/// A body that lands quickly is never preceded by the "waiting" plate.
///
/// The pane drew the plate the moment it knew there was no body -- a full
/// document load -- and the body a beat later, which is a flash of Postio
/// explaining a wait nobody had time to notice. Below a short threshold the
/// pane now leaves what it had on screen and draws the body straight away;
/// only a wait long enough to notice is explained.
pub fn a_body_that_lands_quickly_never_shows_the_waiting_plate() {
    crate::gtk_case(async {
        let state_dir = tempfile::tempdir().expect("a state directory");
        // SAFETY: first statement of a single-threaded test, before the app runs.
        unsafe { std::env::set_var("XDG_STATE_HOME", state_dir.path()) };
        if skip_without_display() {
            return;
        }

        let database = test_support::memory().await;
        let directory = tempfile::tempdir().expect("a blob directory");
        let blobs = BlobStore::open(
            directory.path().to_path_buf(),
            &postio_storage::test_support::blob_keys(),
        )
        .expect("a blob store");
        let (account, inbox) = {
            let connection = database.connect().await.expect("a connection");
            test_support::account_with_inbox(&connection).await
        };
        // Two plain messages, no thread: the newest has its body, the one
        // below it does not yet.
        let late = {
            let connection = database.connect().await.expect("a connection");
            let repository = MessageRepository::new(&connection);
            let mut late = postio_model::Message::new(
                account.id,
                inbox,
                chrono::Utc::now() - chrono::Duration::hours(2),
            );
            late.subject = Some("Late".into());
            late.sync.body_state = postio_model::BodyState::HeadersOnly;
            let late = repository.create(&mut late).await.expect("a message");
            let mut here = postio_model::Message::new(account.id, inbox, chrono::Utc::now());
            here.subject = Some("Here".into());
            here.sync.body_state = postio_model::BodyState::Full;
            let here = repository.create(&mut here).await.expect("a message");
            repository
                .set_body(
                    here,
                    &StoredBody {
                        text: Some("the body that was already here".into()),
                        html: None,
                        headers: None,
                        headers_truncated: false,
                        encoding_problems: false,
                    },
                    postio_model::BodyState::Full,
                )
                .await
                .expect("a body");
            late
        };

        let (bridge, _replies) = Bridge::new(handler_fn(|_, _| async {})).expect("a runtime");
        let (sink, _events) = event_channel();
        let wiring = Wiring::new(
            database.clone(),
            blobs,
            bridge.handle(),
            sink,
            bridge.commands(),
        );

        let window = Window::default();
        window.present();
        settle();
        let wired = feed_the_window(&window, &wiring)
            .await
            .expect("the store has an account");
        let list = window.list();
        assert!(
            settle_until(async || list.model().n_items() == 2).await,
            "the two seeded messages never reached the list"
        );
        assert!(
            settle_until(async || window
                .reader()
                .test_document()
                .contains("the body that was already here"))
            .await,
            "the first message never filled the pane"
        );

        // `j` onto the message with no body, and its body lands 40ms later --
        // well inside the time it takes to notice a plate at all.
        window.handle_key(gdk::Key::j, gdk::ModifierType::empty());
        let mut plates = 0;
        watch_for(std::time::Duration::from_millis(40), || {
            if window.reader().absent().is_some() {
                plates += 1;
            }
        })
        .await;
        {
            let connection = database.connect().await.expect("a connection");
            MessageRepository::new(&connection)
                .set_body(
                    late,
                    &StoredBody {
                        text: Some("the body that came late".into()),
                        html: None,
                        headers: None,
                        headers_truncated: false,
                        encoding_problems: false,
                    },
                    postio_model::BodyState::Full,
                )
                .await
                .expect("the late body");
        }
        wired.feeds.apply(&postio_core::Event::BodyLoaded {
            account: account.id,
            message: late,
        });
        watch_for(std::time::Duration::from_millis(600), || {
            if window.reader().absent().is_some() {
                plates += 1;
            }
        })
        .await;

        assert!(
            window
                .reader()
                .test_document()
                .contains("the body that came late"),
            "the late body never reached the pane"
        );
        assert_eq!(
            plates, 0,
            "the waiting plate flashed up for a body that landed in 40ms"
        );

        window.destroy();
        bridge.shutdown();
    });
}

/// A message with an HTML body, not in any thread, `hours` ago.
async fn seed_html_message(database: &Store, seat: &Seat, name: &str, hours: i64) -> MessageId {
    let connection = database.connect().await.expect("a connection");
    let repository = MessageRepository::new(&connection);
    let mut message = postio_model::Message::new(
        seat.account,
        seat.mailbox,
        chrono::Utc::now() - chrono::Duration::hours(hours),
    );
    message.subject = Some(format!("{name} subject"));
    message.from = vec![postio_model::EmailAddress::new(
        Some("Ada Lovelace"),
        "ada@example.com",
    )];
    message.sync.body_state = postio_model::BodyState::Full;
    message.flags.insert(postio_model::Flag::Seen);
    let id = repository.create(&mut message).await.expect("a message");
    repository
        .set_body(
            id,
            &StoredBody {
                text: None,
                html: Some(format!("<p>the {name} body, in <b>HTML</b></p>")),
                headers: None,
                headers_truncated: false,
                encoding_problems: false,
            },
            postio_model::BodyState::Full,
        )
        .await
        .expect("a body");
    id
}

/// Moving through mail sanitises nothing on the thread that draws the
/// window.
///
/// Every message the cursor settled on went through html5ever twice on the
/// main thread -- once to decide whether it reads as bulk, once to sanitise
/// it -- and so did every message of a thread the first time it was drawn.
/// The reads already happened on the runtime; the parses now happen there
/// too, and the main thread only hands WebKit a document. Counted the way
/// `postio_ui::test_support` counts everything else: per thread, so the
/// runtime's parses do not show up here and the main thread's cannot hide.
pub fn moving_through_mail_sanitises_nothing_on_the_main_thread() {
    crate::gtk_case(async {
        let state_dir = tempfile::tempdir().expect("a state directory");
        // SAFETY: first statement of a single-threaded test, before the app runs.
        unsafe { std::env::set_var("XDG_STATE_HOME", state_dir.path()) };
        if skip_without_display() {
            return;
        }

        let database = test_support::memory().await;
        let directory = tempfile::tempdir().expect("a blob directory");
        let blobs = BlobStore::open(
            directory.path().to_path_buf(),
            &postio_storage::test_support::blob_keys(),
        )
        .expect("a blob store");
        let (account, inbox) = {
            let connection = database.connect().await.expect("a connection");
            test_support::account_with_inbox(&connection).await
        };
        let seat = Seat {
            account: account.id,
            mailbox: inbox,
        };
        // Newest first: a single message, a thread, a single message.
        seed_html_message(&database, &seat, "first", 1).await;
        let (_, thread) = seed_thread(&database, &seat, "middle", 3, 3).await;
        {
            // The thread's bodies as HTML too, so every message costs both
            // parses when it is drawn.
            let connection = database.connect().await.expect("a connection");
            for (index, id) in thread.iter().enumerate() {
                MessageRepository::new(&connection)
                    .set_body(
                        *id,
                        &StoredBody {
                            text: None,
                            html: Some(format!("<p>{}</p>", body_of("middle", index))),
                            headers: None,
                            headers_truncated: false,
                            encoding_problems: false,
                        },
                        postio_model::BodyState::Full,
                    )
                    .await
                    .expect("an HTML body");
            }
        }
        seed_html_message(&database, &seat, "last", 5).await;

        let (bridge, _replies) = Bridge::new(handler_fn(|_, _| async {})).expect("a runtime");
        let (sink, _events) = event_channel();
        let wiring = Wiring::new(database, blobs, bridge.handle(), sink, bridge.commands());

        let window = Window::default();
        window.present();
        settle();
        let _wired = feed_the_window(&window, &wiring)
            .await
            .expect("the store has an account");
        let list = window.list();
        assert!(
            settle_until(async || list.model().n_items() == 3).await,
            "the three seeded rows never reached the list"
        );
        assert!(
            settle_until(async || window.reader().test_document().contains("the first body")).await,
            "the first message never filled the pane"
        );
        watch_for(std::time::Duration::from_millis(300), || {}).await;

        let sanitised = postio_ui::test_support::bodies_sanitised();
        let judged = postio_ui::test_support::bulk_judged();

        window.handle_key(gdk::Key::j, gdk::ModifierType::empty());
        let pane = window.conversation();
        assert!(
            settle_until(async || pane.thread_document().is_some_and(|document| {
                (0..3).all(|index| document.contains(&body_of("middle", index)))
            }))
            .await,
            "`j` never drew the thread whole"
        );
        window.handle_key(gdk::Key::j, gdk::ModifierType::empty());
        assert!(
            settle_until(async || window.reader().test_document().contains("the last body")).await,
            "`j` never drew the last message"
        );

        let sanitised = postio_ui::test_support::bodies_sanitised() - sanitised;
        let judged = postio_ui::test_support::bulk_judged() - judged;
        assert_eq!(
            (sanitised, judged),
            (0, 0),
            "moving over two messages and a three-message thread sanitised \
             {sanitised} bodies and parsed {judged} for reader view on the main \
             thread; both belong on the runtime, beside the read"
        );

        window.destroy();
        bridge.shutdown();
    });
}
