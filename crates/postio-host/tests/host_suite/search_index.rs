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

/// The seed files mail into Drafts, Trash and Junk too, which a search of All mail leaves out
/// (#1523); a sample drawn from them would be absent for a reason unrelated to indexing.
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
            mailboxes: None,
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
    let database = rt.block_on(test_support::memory());
    let report = rt.block_on(seed_small(&database, 11));
    assert!(report.message_count > 0, "seeded nothing to find");

    rt.block_on(ensure_search_index(&database))
        .expect("the index is part of opening the store");

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

    // A word in no subject, address or filename, so a hit can only come from a body.
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

    // Indexing markup and link targets would make every HTML message a hit for `div`.
    for markup_word in ["div", "href", "tracker.example"] {
        assert!(
            rt.block_on(hits(&database, report.account.id, markup_word))
                .is_empty(),
            "{markup_word:?} matched, so a message is a hit for a word it never contained"
        );
    }

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
    let client = host.connect(ClientKind::Focus);

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
    assert!(
        ask("header:x-mailer"),
        "presence is answerable too, not only a value match"
    );
}
