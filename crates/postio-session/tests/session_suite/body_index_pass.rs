//! The catch-up pass over bodies that are already local (#500).
//!
//! The bug this guards: `index_local_bodies` is driven by
//! `messages_missing_body_text`, and a message with a local body and no
//! indexable text — an attachment-only DMARC report, an image — used to leave
//! no trace when indexed, so it stayed a candidate for ever. A store with one
//! full batch of them re-selected the same 200 messages in a tight loop for
//! as long as the app ran: a core at 100%, a stream of write transactions,
//! and the page cache the search path needed evicted under it.
//!
//! The index-level half (a textless body writes an empty row) is asserted in
//! `postio-index`'s own tests, where it was watched red. These are the pass's
//! own promises: it terminates on exactly the store shape that used to spin,
//! it leaves nothing behind for a second run, and it refuses to take the same
//! batch twice even if the index's contract regresses.

use postio_model::{BodyState, Message};
use postio_storage::repository::MessageRepository;
use postio_storage::test_support;

/// More textless messages than one `INDEX_BODY_BATCH`, so the pass has to
/// come back for a second batch — the shape that used to loop for ever.
const TEXTLESS: usize = 450;

#[tokio::test]
async fn a_store_full_of_textless_bodies_is_swept_once_and_left_alone() {
    let database = test_support::temp().await;
    let connection = database.connect().await.expect("checkout");
    postio_index::index::ensure_schema(&connection)
        .await
        .expect("schema");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;

    // Local body, no text: what an attachment-only message looks like to the
    // pass. `body` answers a row holding no parts, the indexable text is
    // empty, and before #500 that meant the message never left the candidate
    // set.
    let messages = MessageRepository::new(&connection);
    connection
        .execute_batch("BEGIN")
        .await
        .expect("begin fixture");
    for i in 0..TEXTLESS {
        let mut message = Message::new(
            account.id,
            inbox,
            chrono::Utc::now() - chrono::Duration::minutes(i as i64),
        );
        message.subject = Some(format!("Report {i} attached"));
        message.sync.body_state = BodyState::Full;
        messages.create(&mut message).await.expect("create");
    }
    connection
        .execute_batch("COMMIT")
        .await
        .expect("commit fixture");
    drop(connection);

    let indexed = postio_session::index_local_bodies(&database)
        .await
        .expect("the pass runs");
    assert_eq!(indexed, TEXTLESS, "every message was visited exactly once");

    let connection = database.connect().await.expect("checkout");
    assert!(
        postio_index::index::messages_missing_body_text(&connection, 10, None)
            .await
            .expect("candidates")
            .is_empty(),
        "a swept store leaves no candidates, or the next start sweeps it again"
    );

    drop(connection);
    let second = postio_session::index_local_bodies(&database)
        .await
        .expect("the second pass");
    assert_eq!(second, 0, "a caught-up store costs one query and no writes");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_body_that_lands_is_indexed_a_moment_later_by_the_indexer() {
    // Neither the store nor the fetch writes a body's search row any more
    // (`set_body_leaves_the_search_index_to_the_indexer`, and the backfill
    // suite): the indexer does, woken by the `BodyLoaded` every fetch emits,
    // in one batched write off the sync lane. This is the other half of that
    // contract -- that a body which lands while the application runs is
    // searchable a moment later, without anything asking on its behalf.
    use postio_core::bridge::EventHub;
    use postio_storage::repository::StoredBody;

    let database = test_support::temp().await;
    let (account, message) = {
        let connection = database.connect().await.expect("checkout");
        postio_index::index::ensure_schema(&connection)
            .await
            .expect("schema");
        let (account, inbox) = test_support::account_with_inbox(&connection).await;
        let mut message = Message::new(account.id, inbox, chrono::Utc::now());
        message.subject = Some("Landing".into());
        let id = MessageRepository::new(&connection)
            .create(&mut message)
            .await
            .expect("create");
        (account.id, id)
    };

    let hub = EventHub::new();
    let sink = hub.sink();
    let indexer = postio_session::spawn_body_indexer(
        database.clone(),
        Some(hub.subscribe("indexer")),
        &tokio::runtime::Handle::current(),
    );

    // The body lands the way the backfill lands it: stored, then announced.
    {
        let connection = database.connect().await.expect("checkout");
        MessageRepository::new(&connection)
            .set_body(
                message,
                &StoredBody {
                    text: Some("words worth finding".to_owned()),
                    html: None,
                    headers: None,
                    headers_truncated: false,
                    encoding_problems: false,
                },
                BodyState::Full,
            )
            .await
            .expect("store the body");
    }
    sink.emit(postio_core::Event::BodyLoaded { account, message });

    let deadline =
        std::time::Instant::now() + postio_test_support::scaled(std::time::Duration::from_secs(10));
    let indexed = loop {
        let connection = database.connect().await.expect("checkout");
        let pending = postio_index::index::messages_missing_body_text(&connection, 10, None)
            .await
            .expect("the queue");
        if pending.is_empty() {
            break true;
        }
        if std::time::Instant::now() > deadline {
            break false;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    };
    assert!(
        indexed,
        "a body that landed and was announced was never indexed: nothing \
         drained the queue"
    );
    drop(hub);
    let _ = tokio::time::timeout(std::time::Duration::from_secs(5), indexer).await;
}

/// #1549: a burst indexes the bodies it *names*, and no others.
///
/// The distinction is the whole fix. The loop used to answer a burst by
/// running a full pass — `messages_missing_body_text` over the mailbox — so a
/// burst naming two messages indexed every outstanding message and paid a
/// walk of the recency index to find them. On a real 61,000-message store that
/// was 640ms of scanning per message indexed, after every burst, for as long
/// as a sync ran.
///
/// So: two messages with local bodies, an event for one. Only that one is
/// indexed. Under the old loop both would be, which is what makes this a
/// regression test rather than a restatement.
///
/// The other one is not lost — the sweep at the next start is what covers it,
/// and `a_store_full_of_textless_bodies_is_swept_once_and_left_alone` is what
/// says the sweep still works.
#[tokio::test(flavor = "multi_thread")]
async fn a_burst_indexes_the_bodies_it_names_and_leaves_the_rest_alone() {
    use postio_core::bridge::EventHub;
    use postio_storage::repository::StoredBody;

    let database = test_support::temp().await;
    let (account, named, unnamed) = {
        let connection = database.connect().await.expect("checkout");
        postio_index::index::ensure_schema(&connection)
            .await
            .expect("schema");
        let (account, inbox) = test_support::account_with_inbox(&connection).await;
        let messages = MessageRepository::new(&connection);

        let store_one = |subject: &str, when| {
            let mut message = Message::new(account.id, inbox, when);
            message.subject = Some(subject.to_owned());
            message
        };
        let mut first = store_one("Named", chrono::Utc::now());
        let named = messages.create(&mut first).await.expect("create named");
        let mut second = store_one("Unnamed", chrono::Utc::now() - chrono::Duration::minutes(1));
        let unnamed = messages.create(&mut second).await.expect("create unnamed");

        for message in [named, unnamed] {
            messages
                .set_body(
                    message,
                    &StoredBody {
                        text: Some("words worth finding".to_owned()),
                        html: None,
                        headers: None,
                        headers_truncated: false,
                        encoding_problems: false,
                    },
                    BodyState::Full,
                )
                .await
                .expect("store the body");
        }
        (account.id, named, unnamed)
    };

    let hub = EventHub::new();
    let sink = hub.sink();
    let indexer = postio_session::spawn_body_indexer(
        database.clone(),
        Some(hub.subscribe("indexer")),
        &tokio::runtime::Handle::current(),
    );

    // The startup sweep runs first and indexes both — this test is about what
    // the *burst* does, so wait for the sweep to finish rather than racing it.
    let connection = database.connect().await.expect("checkout");
    let deadline =
        std::time::Instant::now() + postio_test_support::scaled(std::time::Duration::from_secs(10));
    while !postio_index::index::messages_missing_body_text(&connection, 10, None)
        .await
        .expect("queue")
        .is_empty()
        && std::time::Instant::now() < deadline
    {
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    // Put both back in the queue, so the burst has something to choose from.
    postio_index::index::clear_account_body_index(&connection, account.get())
        .await
        .expect("clear");
    drop(connection);

    sink.emit(postio_core::Event::BodyLoaded {
        account,
        message: named,
    });

    let deadline =
        std::time::Instant::now() + postio_test_support::scaled(std::time::Duration::from_secs(10));
    let settled = loop {
        let connection = database.connect().await.expect("checkout");
        let pending: Vec<i64> =
            postio_index::index::messages_missing_body_text(&connection, 10, None)
                .await
                .expect("queue")
                .iter()
                .map(|row| row.id)
                .collect();
        if pending == vec![unnamed.get()] {
            break true;
        }
        if std::time::Instant::now() > deadline {
            break false;
        }
        drop(connection);
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    };
    assert!(
        settled,
        "the burst should have indexed exactly the message it named, leaving \
         the other for the next sweep"
    );

    drop(hub);
    let _ = tokio::time::timeout(std::time::Duration::from_secs(5), indexer).await;
}
