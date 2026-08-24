//! Search results reaching the message list, driven the way a person drives it.
//!
//! `postio-5w1.1`. `postio-1ag` wired every search surface `postio-gtk` had a
//! seam for — the box's live readout, the scope column, the refine chips, the
//! preview — and the message list underneath went on showing the folder,
//! because there was no seam to put a page of hits into. The seam landed
//! since; this is what says it is *connected*.
//!
//! # Why this is not `search_wiring.rs`
//!
//! That test builds a window and calls `search::install`, and never builds a
//! `Feed` at all. Everything it asserts is true of an application whose list
//! is still showing the inbox — which is precisely the state it was written
//! in. The gap it cannot see is this one.
//!
//! So this starts from `feed_the_window`, the same call `run` makes, and
//! drains the event stream through `commands::drain` the way `open_account`
//! does. Nothing is handed to the list directly: the assertion is that ids
//! the *store* matched came out of the *model*, having crossed the event bus
//! on the way. `Event::SearchResults` firing is not the claim — eight bugs on
//! this project were a tested widget in an application that did nothing, and
//! "the event was emitted" is the same assertion in a new place.
//!
//! One test function: GTK is single-threaded, and both the search and the
//! page reads answer on the thread-default main context, which the harness
//! would otherwise drive from two threads at once.
//!
//! Nothing here touches the network — `start_syncing` is the half that opens
//! a socket, and this never calls it.

use gtk::prelude::*;
use gtk::{gdk, glib};
use postio_app::{Wiring, commands, ensure_search_index, feed_the_window};
use postio_core::bridge::{Bridge, event_channel, handler_fn};
use postio_gtk::finder::{Mode, Query};
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_index::{SearchRequest, search};
use postio_model::MessageId;
use postio_search::facets::Scope;
use postio_storage::seed::seed_small;
use postio_storage::{BlobStore, test_support};

/// A word every fixture in the corpus carries in a header, so the query is
/// about the wiring rather than about the corpus.
const QUERY: &str = "example.com";

/// Run the main loop until `done`, or give up.
///
/// A search crosses to the runtime, answers over a channel, is emitted as an
/// event, is drained on another task, and only then asks for a page — which
/// crosses to the runtime again. A deadline rather than a spin count: what is
/// being waited for is several round trips.
fn settle_until(done: impl Fn() -> bool) -> bool {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while std::time::Instant::now() < deadline {
        while glib::MainContext::default().iteration(false) {}
        if done() {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    done()
}

/// Every id the list model is currently holding, in list order.
///
/// `peek` rather than anything that would fault a page in: this has to report
/// what the model *has*, so that "the list never filled" is distinguishable
/// from "reading it filled it".
fn listed(list: &postio_gtk::list::MessageList, total: u32) -> Vec<MessageId> {
    (0..total)
        .filter_map(|position| list.peek(position))
        .collect()
}

#[test]
fn a_query_puts_the_matching_messages_in_the_list() {
    let state_dir = std::env::temp_dir().join(format!("postio-results-{}", std::process::id()));
    std::fs::create_dir_all(&state_dir).unwrap();
    // SAFETY: first statement of a single-threaded test.
    unsafe { std::env::set_var("XDG_STATE_HOME", &state_dir) };

    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (run under `xvfb-run` to exercise this)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);
    app::install_icons(&display);

    // ── a store the application has opened ──────────────────────────────
    let database = test_support::memory();
    let report = seed_small(&database, 11);
    assert!(
        report.message_count > 0,
        "the fixture seeded no mail, so this test could not fail"
    );
    ensure_search_index(&database).expect("the index is part of opening the store");
    let directory = tempfile::tempdir().expect("a blob directory");
    let blobs = BlobStore::open(directory.keep()).expect("a blob store");

    let (bridge, replies) = Bridge::new(handler_fn(|_, _| async {})).expect("a runtime");
    let (sink, events) = event_channel();
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

    // ── the same call `open_account` makes ──────────────────────────────
    //
    // `search::install` is not called here: `feed_the_window` already makes
    // that call and leaks the `View`, because whether a window has a search
    // is the composition root's business. Calling it a second time puts two
    // views on the box's `connect_run` and the query answers into the one
    // this test cannot see — which is a way to write a test that fails
    // against a perfectly wired application.
    let wired = feed_the_window(&window, &wiring).expect("the store has an account");
    let feeds = wired.feeds.clone();
    let view = wired
        .search
        .expect("the store has an account, so search installed");

    // What carries `Event::SearchResults` from the sink `search.rs` emits
    // into to the `Feed` that turns it into rows. Without this the whole
    // chain under test is a function nobody calls.
    let notifier = postio_app::notifications::Notifier::new(
        database.clone(),
        wiring.store.clone(),
        wiring.runtime.clone(),
        Default::default(),
    );
    for stream in [events, replies] {
        commands::drain(&window, &feeds, stream, notifier.clone());
    }

    // ── the list starts on the folder ───────────────────────────────────
    let list = window.list().model();
    let filled = settle_until(|| !listed(&list, 1).is_empty());
    assert!(
        filled,
        "the list never showed the mailbox, so this test cannot tell a search \
         that failed to reach it from a list that was never fed at all"
    );
    let mailbox_rows = listed(&list, report.message_count as u32);
    let mailbox_name = window.list().mailbox_name();
    assert!(
        !feeds.messages.showing_results(),
        "the list is in result mode before anything was searched"
    );

    // ── type ────────────────────────────────────────────────────────────
    let finder = window.finder();
    finder.open(Mode::Search);
    finder.set_query(Query {
        mode: Mode::Search,
        text: QUERY.to_owned(),
    });
    let live = finder
        .live()
        .expect("the box has a live readout while searching");
    // What Enter does. A test that slept out the debounce would be measuring
    // the timer rather than the wiring.
    live.flush();

    let answered = settle_until(|| live.outcome().is_some_and(|outcome| outcome.hits > 0));
    let outcome = live.outcome().expect("the box answered");
    assert!(
        answered && outcome.hits > 0,
        "the store holds {} messages, every one of them from an {QUERY} \
         address, and the box reports {} hits — so the search itself is the \
         thing that is broken, not the list under it",
        report.message_count,
        outcome.hits
    );

    // ── 1. the hits are what the list is showing ────────────────────────
    let switched = settle_until(|| feeds.messages.showing_results());
    assert!(
        switched,
        "the box found {} hits and the list is still showing the folder. \
         `Event::SearchResults` is emitted by `search.rs::announce` and \
         handled by `Feed::apply`; if the list never changed mode, the event \
         is not reaching it — check that `commands::drain` is running over \
         the same sink `Wiring::events` holds.",
        outcome.hits
    );

    // The rows themselves, not the mode: a `Feed` that flipped into result
    // mode and then failed its page read leaves an empty list, which is the
    // same to a user as never having searched.
    let hits = outcome.hits as u32;
    let populated = settle_until(|| !listed(&list, hits).is_empty());
    let rows = listed(&list, hits);
    assert!(
        populated,
        "the list is in result mode over {} hits and holds no rows at all. \
         The ids reached `Feed::show_results` and the page read under \
         `ResultSource::rows` did not answer — which is `Sources`' impl, not \
         the seam.",
        outcome.hits
    );

    // ── 2. they are the messages the store matched, in rank order ───────
    //
    // The same query, run straight against the index, as an oracle the wiring
    // had no hand in. Comparing against the *mailbox* would not do: the scope
    // is All Mail, so a hit from Sent or Archive is a correct row that was
    // never in the folder the list was showing.
    //
    // Order is asserted, not just membership. Past `RANK_BY_RELEVANCE_LIMIT`
    // the index falls back to recency, so a result set that got silently
    // re-sorted on the way through would still hold the right ids — it would
    // look right in every test that only checked membership, and be wrong
    // exactly where ranking is the thing the user searched for.
    let account = postio_app::first_account(&database)
        .expect("the seeded store has an account")
        .id;
    // The scope column starts on All Mail — `search_wiring.rs` asserts that —
    // so this is the question the box actually asked.
    let view_scope = Scope::AllMail;
    let connection = database.connection().expect("a connection");
    let expected: Vec<MessageId> = search(
        &connection,
        &SearchRequest {
            account_id: account,
            query: &postio_search::parse(QUERY, chrono::Utc::now().date_naive()),
            scope: view_scope,
            limit: 200,
        },
        chrono::Utc::now(),
    )
    .expect("the index answers")
    .hits
    .iter()
    .map(|hit| hit.message_id)
    .collect();

    assert_eq!(
        rows,
        expected[..rows.len()],
        "the list is not showing the hits the index returned, or is showing \
         them in another order"
    );
    assert!(
        !mailbox_rows.is_empty() && rows != mailbox_rows[..rows.len().min(mailbox_rows.len())],
        "the list is showing exactly the folder it was showing before the \
         search, which is what a result set that never arrived looks like"
    );

    // ── 3. the count is the result set's, not the folder's ──────────────
    assert_eq!(
        list.n_items(),
        hits,
        "the list says it is {} rows long over a result set of {}. The \
         scrollbar and every page request are measured against this, so a \
         stale total pages against the folder's length.",
        list.n_items(),
        hits
    );

    // ── 4. the cursor moves the preview ─────────────────────────────────
    //
    // The preview starts on the best match, so this asserts it *moves* — a
    // pane wired to "the top hit" rather than to the cursor passes every
    // assertion above and fails this one.
    let first = view.preview().focused();
    assert!(
        first.is_some(),
        "nothing is previewed at all, so a cursor moving cannot be observed"
    );
    assert!(rows.len() > 1, "one hit cannot demonstrate a cursor moving");
    window.list().next_row();
    let moved = settle_until(|| view.preview().focused() != first);
    assert!(
        moved,
        "`j` through the results left the preview on {first:?}. \
         `View::set_focused` is driven by the list's cursor; if it is still \
         driven by the top hit, walking the results shows one message."
    );
    assert_eq!(
        view.preview().focused(),
        rows.get(1).copied(),
        "the preview followed the cursor onto a message that is not the row \
         the cursor is on"
    );

    // ── 5. `Esc` puts the mailbox back, where it was ────────────────────
    // The gesture, not the state: `press_escape` is what the key does, and
    // `close` would empty the box without telling anything that the search
    // is over.
    finder.press_escape();
    let left = settle_until(|| !feeds.messages.showing_results());
    assert!(
        left,
        "dismissing the box left the list showing the hits. `Esc` is the way \
         out of a search and the folder is what is behind it."
    );
    let back = settle_until(|| listed(&list, 1) == mailbox_rows[..1]);
    assert!(
        back,
        "the list came out of the search showing something other than the \
         folder it went in on"
    );
    assert_eq!(
        window.list().mailbox_name(),
        mailbox_name,
        "the column header is still counting results over a folder listing"
    );

    bridge.shutdown();
}
