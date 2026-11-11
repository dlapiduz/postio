//! A thread, read whole by one request (spec 013 T015, contracts/engine-threads.md).
//!
//! `Req::Thread` answers a window or an expansion with the members in date
//! order, each with its own line, the unsent reply, and the labels. It costs
//! the same number of statements whatever the thread's length (the budget is a
//! count, not a clock).

use std::sync::Arc;

use postio_client::protocol::ClientKind;
use postio_host::Host;
use postio_model::{Line, ThreadId};
use postio_storage::Store;
use postio_storage::sql::RowExt;
use postio_storage::test_support;
use postio_storage::test_support::counting::counted_async;

/// The thread whose subject is `subject`.
async fn thread_named(database: &Store, subject: &str) -> ThreadId {
    let connection = database.connect().await.expect("a connection");
    postio_storage::sql::first(
        &connection,
        "SELECT id FROM threads WHERE lower(subject) = lower(?1)",
        [subject.to_owned()],
        |row| Ok(ThreadId::new(row.col(0)?)),
    )
    .await
    .expect("a read")
    .unwrap_or_else(|| panic!("the seed has a thread called {subject:?}"))
}

/// The thread with `count` messages.
async fn thread_of(database: &Store, count: i64) -> ThreadId {
    let connection = database.connect().await.expect("a connection");
    postio_storage::sql::first(
        &connection,
        "SELECT id FROM threads WHERE message_count = ?1",
        [count],
        |row| Ok(ThreadId::new(row.col(0)?)),
    )
    .await
    .expect("a read")
    .unwrap_or_else(|| panic!("the seed has a thread of {count}"))
}

#[tokio::test(flavor = "multi_thread")]
async fn a_thread_reads_whole_with_its_answers_and_its_draft() {
    let (database, _) = postio_demo::seeded(postio_demo::Seed::Threads).await;
    let harbor = thread_named(&database, "Harbor API draft v3").await;
    let blobs = tempfile::tempdir().expect("a directory");
    let host = Host::start(
        database.clone(),
        postio_storage::BlobStore::open(blobs.path().to_owned(), &test_support::blob_keys())
            .expect("a blob store"),
        |wiring| wiring.with_secrets(Arc::new(postio_account::secret::MemorySecretStore::new())),
    )
    .expect("a host");
    let client = host.connect(ClientKind::Test);

    let view = client
        .thread(harbor)
        .await
        .expect("a read")
        .expect("Harbor is there");

    assert_eq!(view.thread, harbor);
    assert_eq!(
        view.subject.as_deref().map(str::to_lowercase),
        Some("harbor api draft v3".to_owned())
    );
    assert_eq!(view.members.len(), 6, "every member, whatever folder");
    assert_eq!(
        view.members[3].answers,
        Some(0),
        "message 4 answers message 1"
    );
    assert!(
        view.members.windows(2).all(|pair| pair[0].at <= pair[1].at),
        "oldest first"
    );
    assert!(
        view.members
            .iter()
            .all(|member| !matches!(&member.line, Line::Own(line) if line.is_empty())),
        "an empty own line is `Quoted`"
    );
    let draft = view.draft.expect("Harbor has an unsent reply");
    assert!(draft.first_words.starts_with("Thanks both."));
    assert_eq!(view.muted, None, "mute arrives with US5");
    assert_eq!(
        client.counts().of("Thread"),
        1,
        "one round trip, counted by its family"
    );

    assert_eq!(
        client
            .thread(ThreadId::new(1_000_000))
            .await
            .expect("a read"),
        None,
        "a thread that is gone is none, not a failure"
    );
    drop(client);
    drop(host);
}

/// The same statements at six messages as at forty-one. Counted on this
/// thread, so the host's answer is called directly, as the host calls it.
#[tokio::test]
async fn reading_a_thread_costs_the_same_at_any_length() {
    let (database, _) = postio_demo::seeded(postio_demo::Seed::Threads).await;
    let harbor = thread_named(&database, "Harbor API draft v3").await;
    let long = thread_of(&database, 41).await;
    // Warm the connection's own one-time work so both counts are the read.
    let _ = postio_host::threads::view(&database, harbor).await;

    let mut costs = Vec::new();
    for (thread, members) in [(harbor, 6), (long, 41)] {
        let mut read = None;
        let counts = counted_async(async || {
            read = Some(
                postio_host::threads::view(&database, thread)
                    .await
                    .expect("a read"),
            );
        })
        .await;
        let view = read.flatten().expect("the thread is there");
        assert_eq!(view.members.len(), members);
        costs.push(counts.statements);
    }

    assert_eq!(
        costs[0], costs[1],
        "six messages cost {} statements and forty-one cost {}",
        costs[0], costs[1]
    );
    assert!(costs[0] <= 4, "the read took {} statements", costs[0]);
}
