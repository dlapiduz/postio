//! A unified search searches every account (#961).
//!
//! The shape `/issue` §4 and `ARCHITECTURE.md` keep warning about, and this
//! one was measured: `postio-index`'s executor takes `AccountScope::Unified`
//! and documents its binding order for it, `postio-bench` defends a
//! performance budget for that path, the sidebar offers the scope and the
//! list honours it — and `postio-app` never constructed a request with it.
//! Every layer built, tested, benchmarked; the join missing.
//!
//! So this drives the join and nothing else. Two accounts, a word that exists
//! in exactly one of them, and the same query asked twice: once scoped to the
//! account that does not have it, once under Unified. Deliberately not a test
//! of the executor — `postio-index`'s `account_scope.rs` proves that half, and
//! it stayed green through the whole bug.
//!
//! One test function, for the reason `wiring.rs` gives.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe: it races any other thread reading
// the environment. This test sets it before the app under test starts, which
// is the one moment it is sound. The crate's library code forbids `unsafe`.

use crate::settle_until;
use gtk::{gdk, glib};
use postio_app::{Wired, feed_the_window};
use postio_core::bridge::{Bridge, event_channel, handler_fn};
use postio_gtk::finder::{Mode, Query};
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_model::{BodyState, Message};
use postio_session::{Wiring, ensure_search_index};
use postio_storage::repository::MessageRepository;
use postio_storage::seed::{seed_extra_account, seed_small};
use postio_storage::{BlobStore, Database, test_support};

/// A word no corpus fixture carries, planted in exactly one account.
///
/// The whole test turns on it being findable in one scope and not the other,
/// so it must not be a word the seeded mail could supply by accident.
const ONLY_IN_THE_SECOND: &str = "photogrammetry";

/// Put a message carrying [`ONLY_IN_THE_SECOND`] in `account`'s first mailbox.
fn plant(database: &Database, account: postio_model::AccountId) -> postio_model::MessageId {
    let connection = database.connection().expect("a connection");
    let mailbox: i64 = connection
        .query_row(
            "SELECT id FROM mailboxes WHERE account_id = ?1 ORDER BY id LIMIT 1",
            [account.get()],
            |row| row.get(0),
        )
        .expect("the seeded account has a mailbox");
    let mut message = Message::new(
        account,
        postio_model::MailboxId::new(mailbox),
        chrono::Utc::now(),
    );
    message.subject = Some(format!("Site survey by {ONLY_IN_THE_SECOND}"));
    message.sync.body_state = BodyState::NotFetched;
    MessageRepository::new(&connection)
        .create(&mut message)
        .expect("plant the message");
    message.id
}

/// How many messages the box says the current query matched.
fn hits(window: &Window) -> Option<u64> {
    window
        .finder()
        .live()
        .expect("the box has a live readout while searching")
        .outcome()
        .map(|outcome| outcome.hits)
}

fn search_for(window: &Window, text: &str) {
    let finder = window.finder();
    finder.open(Mode::Search);
    finder.set_query(Query {
        mode: Mode::Search,
        text: text.to_owned(),
    });
    finder
        .live()
        .expect("the box has a live readout while searching")
        .flush();
}

pub fn a_unified_search_reaches_every_account() {
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
    let first = seed_small(&database, 11);
    let second = seed_extra_account(&database, "Second", "grace@example.org", 12);
    let planted = plant(&database, second.account.id);
    ensure_search_index(&database).expect("the index is part of opening the store");

    let directory = tempfile::tempdir().expect("a blob directory");
    let blobs = BlobStore::open(
        directory.path().to_path_buf(),
        &postio_storage::test_support::blob_keys(),
    )
    .expect("a blob store");
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
    while glib::MainContext::default().iteration(false) {}

    let Wired { feeds: _feeds, .. } =
        feed_the_window(&window, &wiring).expect("the store has an account");

    // The window opens on one account, which is the first one seeded — see
    // `feed_the_window`, which sets the scope to `first_account` rather than
    // to Unified because the scope somebody left in is not remembered yet.
    assert_eq!(
        window.scope(),
        postio_core::Scope::Account(first.account.id),
        "the premise: the window starts scoped to one account, so the search \
         below has somewhere to *not* find the planted message"
    );

    // ── the same query, in the account that does not have it ─────────────
    search_for(&window, ONLY_IN_THE_SECOND);
    let answered = settle_until(|| hits(&window).is_some());
    assert!(answered, "the search never answered at all");
    assert_eq!(
        hits(&window),
        Some(0),
        "the word is planted in the *other* account, so a search scoped to \
         this one must not find it. If this is non-zero the search is already \
         ignoring the scope, and the assertion below would pass for the wrong \
         reason."
    );

    // ── switch to Unified, the way a person does ─────────────────────────
    // Row 0 of the account strip is the Unified row (`offer_unified: true`).
    // Clicking it rather than calling `window.set_scope` directly: the
    // gesture is what has to reach the search, and a test that set the scope
    // itself would pass with the sidebar wired to nothing.
    window.sidebar().test_click_account_row(0);
    assert!(
        settle_until(|| window.scope() == postio_core::Scope::Unified),
        "clicking the Unified row did not put the window in the unified scope"
    );

    // ── and the search follows it, without being retyped ─────────────────
    assert!(
        settle_until(|| hits(&window) == Some(1)),
        "switching to Unified left the search showing {:?} hits for a word \
         that exists in the other account. The executor has taken \
         `AccountScope::Unified` since #186 and `postio-bench` defends a \
         budget for it; the composition root never constructed one, so the \
         scope the sidebar offers changed what the *list* showed and never \
         what a *search* searched.",
        hits(&window)
    );

    // Which message, not merely how many: a count of one could be any row.
    let connection = database.connection().expect("a connection");
    let subject = MessageRepository::new(&connection)
        .get(planted)
        .expect("a read")
        .expect("the planted message is in the store")
        .subject
        .unwrap_or_default();
    assert!(
        subject.contains(ONLY_IN_THE_SECOND),
        "the planted message is the one that should have matched: {subject:?}"
    );
    drop(connection);

    // ── and back again, because a scope switch has to work both ways ─────
    window.sidebar().test_click_account_row(1);
    assert!(
        settle_until(|| window.scope() == postio_core::Scope::Account(first.account.id)),
        "clicking the first account's row did not narrow the window back"
    );
    assert!(
        settle_until(|| hits(&window) == Some(0)),
        "narrowing back to one account left the unified result set on screen. \
         A stale count is worse than a slow one: it is an answer to a question \
         nobody is asking any more."
    );
}
