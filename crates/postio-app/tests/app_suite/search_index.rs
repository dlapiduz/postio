//! Opening the store makes its mail findable.
//!
//! `postio-x4e`, the ninth instance of `postio-bl2`: `postio_index`'s schema,
//! its triggers and its executor were all implemented and tested, and nothing
//! in the application created the index. `search_documents` and `messages_fts`
//! did not exist on any real store, so search had nothing to search — even
//! after `postio-svx` made the executor reachable at all.
//!
//! The assertion is the far end again: seed a store the way a synced account
//! looks, open it the way `run` opens it, and search it. Not "the executor
//! works" — `postio-index` proves that itself — but "a store this application
//! opened can be searched", which is the sentence that was false.

use chrono::Utc;
use gtk::{gdk, glib};
use postio_app::feed_the_window;
use postio_core::bridge::{Bridge, event_channel, handler_fn};
use postio_gtk::window::Window;
use postio_index::{SearchRequest, search};
use postio_model::AccountScope;
use postio_model::ids::MessageId;
use postio_model::{AccountId, BodyState};
use postio_search::facets::Scope;
use postio_search::parse;
use postio_session::{Wiring, ensure_search_index, index_local_bodies};
use postio_storage::repository::{MessageRepository, StoredBody};
use postio_storage::seed::seed_small;
use postio_storage::{BlobStore, Database, test_support};

pub fn a_store_the_application_opened_can_be_searched() {
    // Seeded first and indexed after, which is the order every existing
    // account is in: the mail was there long before the index was.
    let database = test_support::memory();
    let report = seed_small(&database, 11);
    assert!(report.message_count > 0, "seeded nothing to find");

    ensure_search_index(&database).expect("the index is part of opening the store");

    let connection = database.connection().expect("a connection");
    // A word every fixture in the corpus has a sender for. Searching for the
    // *sender* rather than a subject also proves the recipients half of the
    // backfill ran, not only the subject column.
    let query = parse("example.com", Utc::now().date_naive());
    let hits = search(
        &connection,
        &SearchRequest {
            account: AccountScope::Account(report.account.id),
            query: &query,
            scope: Scope::AllMail,
            limit: 50,
            order: postio_search::ResultOrder::Relevance,
        },
        Utc::now(),
    )
    .expect("the search runs")
    .hits;

    assert!(
        !hits.is_empty(),
        "the store holds {} messages and searching it finds none. Either the \
         index was never created — which is what postio-x4e was — or it was \
         created empty, which is the same bug with the backfill missing.",
        report.message_count
    );
}

/// Every message the seeded store put in the list, newest first.
fn all_messages(database: &Database) -> Vec<MessageId> {
    let connection = database.connection().expect("a connection");
    let mut statement = connection
        .prepare("SELECT id FROM messages ORDER BY received_at DESC")
        .expect("a statement");
    let rows = statement
        .query_map([], |row| row.get::<_, i64>(0))
        .expect("query");
    rows.map(|id| MessageId::new(id.expect("an id"))).collect()
}

/// Land a body for `id`, the way a settled backfill leaves one.
fn give_body(database: &Database, id: MessageId, text: Option<&str>, html: Option<&str>) {
    let connection = database.connection().expect("a connection");
    let stored = StoredBody {
        text: text.map(str::to_owned),
        html: html.map(str::to_owned),
        headers: None,
    };
    MessageRepository::new(&connection)
        .set_body(id, &stored, BodyState::Full)
        .expect("store the body");
}

fn hits(database: &Database, account: AccountId, query: &str) -> Vec<MessageId> {
    let connection = database.connection().expect("a connection");
    let parsed = parse(query, Utc::now().date_naive());
    search(
        &connection,
        &SearchRequest {
            account: AccountScope::Account(account),
            query: &parsed,
            scope: Scope::AllMail,
            limit: 50,
            order: postio_search::ResultOrder::Relevance,
        },
        Utc::now(),
    )
    .expect("the search runs")
    .hits
    .into_iter()
    .map(|hit| hit.message_id)
    .collect()
}

/// Issue #327: a store that predates body indexing catches up, once.
///
/// `postio_index::index::index_body` was written, tested and benched, and
/// nothing in the workspace ever called it — so `search_documents.body` was
/// empty on every message in every real store. The metadata columns are kept
/// by SQL trigger and were always right, which is why this presented as
/// "search is inconsistent" rather than "search is broken": the same word
/// found a message by its subject and not by its body.
///
/// `postio-sync` now indexes a body where it lands, which covers everything
/// fetched from here on. This is the other half — mail that was already on
/// this machine when that call did not exist, and any body whose index write
/// was lost to a crash between the commit point and it.
pub fn a_store_that_predates_body_indexing_catches_up() {
    let database = test_support::memory();
    let report = seed_small(&database, 29);

    let messages = all_messages(&database);
    assert!(messages.len() > 2, "not enough seeded mail to tell apart");

    // A word that appears in no subject, no address and no filename in the
    // corpus, so a hit for it can only have come from a body.
    let (plain, markup, never_fetched) = (messages[0], messages[1], messages[2]);
    give_body(
        &database,
        plain,
        Some("The turbines held at ninety-one percent all quarter."),
        None,
    );
    give_body(
        &database,
        markup,
        None,
        Some(
            "<div><p>The <a href=\"https://tracker.example/c?i=7\">turbines</a> \
             held steady.</p></div>",
        ),
    );
    // `never_fetched` keeps its seeded `BodyState::NotFetched` and gets no
    // blob: its body is on the server, and the index must not claim to have
    // read it.

    ensure_search_index(&database).expect("the index is part of opening the store");
    assert!(
        hits(&database, report.account.id, "turbines").is_empty(),
        "nothing has indexed a body yet, so this cannot be measuring the pass"
    );

    let indexed = index_local_bodies(&database).expect("the pass runs");
    assert_eq!(
        indexed, 2,
        "the pass should index exactly the two messages whose body is on          this machine"
    );

    let found = hits(&database, report.account.id, "turbines");
    assert!(
        found.contains(&plain),
        "a word that appears only in a message's body finds nothing (#327)"
    );
    assert!(
        found.contains(&markup),
        "an HTML-only message is not findable by anything it actually says"
    );
    assert!(
        !found.contains(&never_fetched),
        "a message whose body is still on the server was indexed anyway, so          search is answering for a corpus this machine does not have"
    );

    // Markup and link targets are not the message. Indexing them would make
    // every HTML message a hit for `div`, and every message carrying one
    // tracking redirect a hit for whatever campaign shared that shortener.
    for markup_word in ["div", "href", "tracker.example"] {
        assert!(
            hits(&database, report.account.id, markup_word).is_empty(),
            "{markup_word:?} matched, so a message is a hit for a word it \
             never contained"
        );
    }

    // Idempotent: the second pass finds nothing left to do, and the first
    // pass's rows are not duplicated. `search_documents` is keyed by
    // `message_id`, so a duplicate would be a second FTS row for one message
    // — one message appearing twice in its own result list.
    assert_eq!(
        index_local_bodies(&database).expect("a second pass"),
        0,
        "the pass indexed the same bodies again, so it is not safe to run on          every start"
    );
    let again = hits(&database, report.account.id, "turbines");
    assert_eq!(again.len(), 2, "one message, one hit: {again:?}");
}

/// And nobody has to ask for it.
///
/// The pass above is a function; this is the question that function has to
/// answer *yes* to before #327 is closed — can a person reach it in the
/// running application? Four capabilities in this repository have been built,
/// tested and wired to nothing, and the worst of them shipped a mail client
/// that could not read mail with every test green. A body index that only a
/// test ever runs would be the fifth.
///
/// So: no call to `index_local_bodies` here. A store with a local body, the
/// same `feed_the_window` the binary calls, and then wait for the word to
/// become findable.
///
/// Nothing here touches the network — `start_syncing` is the half that opens
/// a socket and this never calls it.
pub fn opening_the_window_indexes_local_bodies_without_being_asked() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }

    let database = test_support::memory();
    let report = seed_small(&database, 31);
    let directory = tempfile::tempdir().expect("a blob directory");
    let blobs = BlobStore::open(
        directory.path().to_path_buf(),
        &postio_storage::test_support::blob_keys(),
    )
    .expect("a blob store");

    let target = all_messages(&database)[0];
    give_body(
        &database,
        target,
        Some("A word no header in the corpus carries: photogrammetry."),
        None,
    );
    ensure_search_index(&database).expect("the index is part of opening the store");
    assert!(
        hits(&database, report.account.id, "photogrammetry").is_empty(),
        "the body is not indexed yet, so this cannot be measuring the wiring"
    );

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

    // ── the same call `run` makes, and nothing else ──────────────────────
    let _wired = feed_the_window(&window, &wiring).expect("the seeded store has an account");

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let mut found = false;
    while std::time::Instant::now() < deadline && !found {
        while glib::MainContext::default().iteration(false) {}
        found = hits(&database, report.account.id, "photogrammetry").contains(&target);
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(
        found,
        "the store was opened the way the application opens it and a body \
         already on this machine is still not searchable. `index_local_bodies` \
         exists and nothing runs it."
    );

    bridge.shutdown();
}
