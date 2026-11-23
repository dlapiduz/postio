use postio_model::{BodyState, Message};
use postio_storage::repository::MessageRepository;
use postio_storage::test_support;

const TEXTLESS: usize = 450;

#[tokio::test]
async fn a_store_full_of_textless_bodies_is_swept_once_and_left_alone() {
    let database = test_support::temp().await;
    let connection = database.connect().await.expect("checkout");
    postio_index::index::ensure_schema(&connection)
        .await
        .expect("schema");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;

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
    sink.emit(postio_core::Event::BodyLoaded {
        account,
        message,
        sections: postio_core::event::Sections::Text,
    });

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
    postio_index::index::clear_account_body_index(&connection, account.get())
        .await
        .expect("clear");
    drop(connection);

    sink.emit(postio_core::Event::BodyLoaded {
        account,
        message: named,
        sections: postio_core::event::Sections::Text,
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
