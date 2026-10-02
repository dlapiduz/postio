//! A store a host opened makes its mail findable.
//!
//! `postio-x4e`, the ninth instance of `postio-bl2`: `postio_index`'s schema,
//! its triggers and its executor were all implemented and tested, and nothing
//! created the index on a real store. So the assertions are not "the executor
//! works" -- `postio-index` proves that -- but that mail already on this
//! machine is searchable once the store is opened and the host's idle passes
//! have run:
//!
//! - a store seeded before the index existed finds its mail by sender;
//! - a store that predates body indexing catches up exactly once (#327);
//! - header blocks already on disk answer `header:` without being asked for
//!   (ADR 0025).
//!
//! Bodies already on disk becoming searchable through the host's own idle
//! passes is `the_host_makes_bodies_already_on_disk_searchable_once_it_catches_up`
//! in the host's unit tests.

use std::time::{Duration, Instant};

use chrono::Utc;
use postio_account::secret::MemorySecretStore;
use postio_client::protocol::{ClientKind, Search};
use postio_host::Host;
use postio_index::{SearchRequest, search};
use postio_model::ids::MessageId;
use postio_model::{AccountId, AccountScope, BodyState};
use postio_search::facets::Scope;
use postio_search::parse;
use postio_session::{ensure_search_index, index_local_bodies};
use postio_storage::repository::{MessageRepository, StoredBody};
use postio_storage::seed::seed_small;
use postio_storage::{Store, test_support};

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a test runtime")
}

/// Every message the seeded store put where All mail looks, newest first.
///
/// The seed files mail into Drafts, Trash and Junk too, and a search of All
/// mail leaves those three out (#1523): a sample drawn from them would be
/// absent from the results for a reason that has nothing to do with whether
/// its body was indexed.
async fn all_messages(database: &Store) -> Vec<MessageId> {
    let connection = database.connect().await.expect("a connection");
    let mut statement = connection
        .prepare(
            "SELECT m.id FROM messages m \
             JOIN mailboxes b ON b.id = m.mailbox_id \
             WHERE b.role NOT IN ('drafts', 'junk', 'trash') \
             ORDER BY m.received_at DESC",
        )
        .await
        .expect("a statement");
    let rows = postio_storage::sql::mapped(&mut statement, (), |row| {
        postio_storage::sql::RowExt::col::<i64>(row, 0)
    })
    .await
    .expect("query");
    rows.into_iter().map(MessageId::new).collect()
}

/// Land a body (and header block) for `id`, the way a settled backfill leaves
/// one, and leave the indexes alone.
async fn give_body(
    database: &Store,
    id: MessageId,
    text: Option<&str>,
    html: Option<&str>,
    headers: Option<&str>,
) {
    let connection = database.connect().await.expect("a connection");
    let stored = StoredBody {
        text: text.map(str::to_owned),
        html: html.map(str::to_owned),
        headers: headers.map(str::to_owned),
        headers_truncated: false,
        encoding_problems: false,
    };
    MessageRepository::new(&connection)
        .set_body(id, &stored, BodyState::Full)
        .await
        .expect("store the body");
}

async fn hits(database: &Store, account: AccountId, query: &str) -> Vec<MessageId> {
    let connection = database.connect().await.expect("a connection");
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
    .await
    .expect("the search runs")
    .hits
    .into_iter()
    .map(|hit| hit.message_id)
    .collect()
}

#[test]
fn a_store_seeded_before_the_index_existed_can_be_searched_once_opened() {
    let rt = runtime();
    // Seeded first and indexed after, which is the order every existing
    // account is in: the mail was there long before the index was.
    let database = rt.block_on(test_support::memory());
    let report = rt.block_on(seed_small(&database, 11));
    assert!(report.message_count > 0, "seeded nothing to find");

    rt.block_on(ensure_search_index(&database))
        .expect("the index is part of opening the store");

    // Searching for the sender rather than a subject also proves the
    // recipients half of the backfill ran, not only the subject column.
    let found = rt.block_on(hits(&database, report.account.id, "example.com"));
    assert!(
        !found.is_empty(),
        "the store holds {} messages and searching it finds none: the index \
         was never created, or was created empty",
        report.message_count
    );
}

#[test]
fn a_store_that_predates_body_indexing_catches_up_once() {
    let rt = runtime();
    let database = rt.block_on(test_support::memory());
    let report = rt.block_on(seed_small(&database, 29));
    let messages = rt.block_on(all_messages(&database));
    assert!(messages.len() > 2, "not enough seeded mail to tell apart");

    // A word that appears in no subject, address or filename in the corpus,
    // so a hit for it can only have come from a body.
    let (plain, markup, never_fetched) = (messages[0], messages[1], messages[2]);
    rt.block_on(give_body(
        &database,
        plain,
        Some("The turbines held at ninety-one percent all quarter."),
        None,
        None,
    ));
    rt.block_on(give_body(
        &database,
        markup,
        None,
        Some(
            "<div><p>The <a href=\"https://tracker.example/c?i=7\">turbines</a> \
             held steady.</p></div>",
        ),
        None,
    ));
    // `never_fetched` keeps its seeded `NotFetched` and gets no blob: its body
    // is on the server, and the index must not claim to have read it.

    rt.block_on(ensure_search_index(&database))
        .expect("the index is part of opening the store");
    assert!(
        rt.block_on(hits(&database, report.account.id, "turbines"))
            .is_empty(),
        "nothing has indexed a body yet, so this cannot be measuring the pass"
    );

    let indexed = rt
        .block_on(index_local_bodies(&database))
        .expect("the pass runs");
    assert_eq!(
        indexed, 2,
        "the pass should index exactly the two messages whose body is on this machine"
    );

    let found = rt.block_on(hits(&database, report.account.id, "turbines"));
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
        "a message whose body is still on the server was indexed anyway"
    );

    // Markup and link targets are not the message: indexing them would make
    // every HTML message a hit for `div`.
    for markup_word in ["div", "href", "tracker.example"] {
        assert!(
            rt.block_on(hits(&database, report.account.id, markup_word))
                .is_empty(),
            "{markup_word:?} matched, so a message is a hit for a word it never contained"
        );
    }

    // Idempotent: a second pass finds nothing left to do, and the first
    // pass's rows are not duplicated.
    assert_eq!(
        rt.block_on(index_local_bodies(&database))
            .expect("a second pass"),
        0,
        "the pass indexed the same bodies again, so it is not safe to run on every start"
    );
    let again = rt.block_on(hits(&database, report.account.id, "turbines"));
    assert_eq!(again.len(), 2, "one message, one hit: {again:?}");
}

#[test]
fn starting_the_idle_passes_indexes_the_header_blocks_already_on_disk() {
    let rt = runtime();
    let database = rt.block_on(test_support::memory());
    let report = rt.block_on(seed_small(&database, 37));
    let target = rt.block_on(all_messages(&database))[0];
    // A field no envelope column carries and no fixture in the corpus has, so
    // nothing but `message_headers` could answer for it.
    rt.block_on(give_body(
        &database,
        target,
        Some("a body, so the row looks fetched"),
        None,
        Some("X-Mailer: Photogrammetry 4.2\r\nContent-Type: text/plain; charset=utf-8"),
    ));
    rt.block_on(ensure_search_index(&database))
        .expect("the index is part of opening the store");
    assert!(
        rt.block_on(hits(
            &database,
            report.account.id,
            "header:x-mailer=photogrammetry"
        ))
        .is_empty(),
        "the block is stored and unindexed, so this cannot be measuring the passes"
    );

    let directory = tempfile::tempdir().expect("a blob directory");
    let blobs =
        postio_storage::BlobStore::open(directory.path().to_path_buf(), &test_support::blob_keys())
            .expect("a blob store");
    let host = Host::start(database.clone(), blobs, |wiring| {
        wiring.with_secrets(std::sync::Arc::new(MemorySecretStore::new()))
    })
    .expect("a host");
    let client = host.connect(ClientKind::Gtk);

    // Nobody calls `index_local_headers`: only the host's idle passes do.
    host.start_idle_passes();

    let ask = |query: &str| {
        rt.block_on(client.search(Search {
            account: AccountScope::Unified,
            query: query.into(),
            newest_first: true,
            scope: Scope::AllMail,
        }))
        .expect("an answer")
        .is_some_and(|found| found.ids.contains(&target))
    };
    let deadline = Instant::now() + postio_test_support::scaled(Duration::from_secs(30));
    while !ask("header:x-mailer=photogrammetry") {
        assert!(
            Instant::now() < deadline,
            "a stored header block never became findable with header:"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    // Presence and value are two different questions, and both have to reach
    // the running host.
    assert!(
        ask("header:x-mailer"),
        "presence is answerable too, not only a value match"
    );
}
