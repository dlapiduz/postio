use postio_index::index::{ensure_schema, index_bodies, index_body, messages_missing_body_text};
use postio_model::{BodyState, Message};
use postio_storage::Connection;
use postio_storage::repository::MessageRepository;
use postio_storage::test_support;

async fn a_message(connection: &Connection, subject: &str) -> i64 {
    let (account, mailbox) = test_support::account_with_inbox(connection).await;
    let mut message = Message::new(account.id, mailbox, chrono::Utc::now());
    message.subject = Some(subject.to_owned());
    message.sync.body_state = BodyState::Full;
    MessageRepository::new(connection)
        .create(&mut message)
        .await
        .expect("create");
    message.id.get()
}

async fn body_hits(connection: &Connection, query: &str) -> Vec<i64> {
    postio_storage::sql::all(
        connection,
        "SELECT m.id FROM message_search_bodies b JOIN messages m ON m.content_id=b.content_id WHERE fts_match(body_search, ?1) ORDER BY m.id",
        [postio_model::fold::fold(query)],
        |row| postio_storage::sql::RowExt::col(row, 0),
    )
    .await
    .expect("query")
}

async fn indexed_bodies(connection: &Connection) -> i64 {
    postio_storage::sql::scalar(connection, "SELECT count(*) FROM message_search_bodies", ())
        .await
        .expect("count")
}

#[tokio::test]
async fn a_body_is_searchable_in_a_table_of_its_own() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    ensure_schema(&connection).await.expect("schema");
    let id = a_message(&connection, "Quarterly report").await;

    index_body(&connection, id, Some("the difference engine is finished"))
        .await
        .expect("index");

    assert_eq!(body_hits(&connection, "difference").await, vec![id]);
    assert!(body_hits(&connection, "unrelated").await.is_empty());
}

#[tokio::test]
async fn many_bodies_indexed_at_once_are_found_as_one_at_a_time_would_be() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    ensure_schema(&connection).await.expect("schema");
    let first = a_message(&connection, "Quarterly report").await;
    let second = a_message(&connection, "Minutes").await;
    let already = a_message(&connection, "Earlier").await;
    index_body(&connection, already, Some("the earlier draft"))
        .await
        .expect("index");

    index_bodies(
        &connection,
        &[
            (first, "the difference engine is finished"),
            (second, "the Café's difference"),
        ],
    )
    .await
    .expect("index");

    assert_eq!(
        body_hits(&connection, "difference").await,
        vec![first, second]
    );
    assert_eq!(body_hits(&connection, "cafe").await, vec![second], "folded");
    assert_eq!(
        body_hits(&connection, "earlier").await,
        vec![already],
        "what was indexed before is still found"
    );
    assert_eq!(indexed_bodies(&connection).await, 3);
    index_body(&connection, first, Some("the analytical engine"))
        .await
        .expect("re-index");
    assert_eq!(body_hits(&connection, "analytical").await, vec![first]);
    assert_eq!(body_hits(&connection, "difference").await, vec![second]);
}

#[tokio::test]
async fn re_indexing_replaces_the_body_rather_than_adding_a_second_row() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    ensure_schema(&connection).await.expect("schema");
    let id = a_message(&connection, "Quarterly report").await;

    index_body(&connection, id, Some("the first draft"))
        .await
        .expect("index");
    index_body(&connection, id, Some("the second draft"))
        .await
        .expect("re-index");

    assert_eq!(body_hits(&connection, "second").await, vec![id]);
    assert!(
        body_hits(&connection, "first").await.is_empty(),
        "the previous text is still matchable"
    );
    assert_eq!(indexed_bodies(&connection).await, 1);
}

#[tokio::test]
async fn clearing_a_body_removes_it_from_the_index() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    ensure_schema(&connection).await.expect("schema");
    let id = a_message(&connection, "Quarterly report").await;
    index_body(&connection, id, Some("something"))
        .await
        .expect("index");

    index_body(&connection, id, None).await.expect("clear");

    assert!(body_hits(&connection, "something").await.is_empty());
    assert_eq!(indexed_bodies(&connection).await, 1);
}

#[tokio::test]
async fn deleting_a_message_takes_its_body_with_it() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    ensure_schema(&connection).await.expect("schema");
    let id = a_message(&connection, "Quarterly report").await;
    index_body(&connection, id, Some("the difference engine"))
        .await
        .expect("index");

    MessageRepository::new(&connection)
        .delete(&[postio_model::MessageId::new(id)])
        .await
        .expect("delete");

    assert!(body_hits(&connection, "difference").await.is_empty());
    assert_eq!(indexed_bodies(&connection).await, 0);
}

#[tokio::test]
async fn a_body_indexed_before_this_table_existed_is_found_by_the_maintenance_pass() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    ensure_schema(&connection).await.expect("schema");
    let id = a_message(&connection, "Quarterly report").await;

    assert_eq!(
        messages_missing_body_text(&connection, 10, None)
            .await
            .expect("candidates")
            .iter()
            .map(|row| row.id)
            .collect::<Vec<_>>(),
        vec![id],
        "a body that is local and not indexed here is exactly the work"
    );

    index_body(&connection, id, Some("already indexed"))
        .await
        .expect("catch up");

    assert!(
        messages_missing_body_text(&connection, 10, None)
            .await
            .expect("candidates")
            .iter()
            .map(|row| row.id)
            .collect::<Vec<_>>()
            .is_empty(),
        "and once it is here, the pass leaves it alone"
    );
}

#[tokio::test]
async fn a_message_whose_text_is_local_but_whose_payloads_are_not_is_still_indexed() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    ensure_schema(&connection).await.expect("schema");
    let id = a_message(&connection, "Statement attached").await;
    connection
        .execute(
            "UPDATE messages SET body_state = 'partial' WHERE id = ?1",
            [id],
        )
        .await
        .expect("the fixture writes");

    assert_eq!(
        messages_missing_body_text(&connection, 10, None)
            .await
            .expect("candidates")
            .iter()
            .map(|row| row.id)
            .collect::<Vec<_>>(),
        vec![id]
    );
}

#[tokio::test]
async fn a_message_whose_body_is_still_on_the_server_is_not_a_candidate() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    ensure_schema(&connection).await.expect("schema");
    let id = a_message(&connection, "Not fetched yet").await;
    connection
        .execute(
            "UPDATE messages SET body_state = 'headers_only' WHERE id = ?1",
            [id],
        )
        .await
        .expect("the fixture writes");

    assert!(
        messages_missing_body_text(&connection, 10, None)
            .await
            .expect("candidates")
            .iter()
            .map(|row| row.id)
            .collect::<Vec<_>>()
            .is_empty()
    );
}

#[tokio::test]
async fn a_message_with_nothing_to_index_leaves_the_candidate_set() {
    // An attachment-only message has no indexable text, but indexing it must still record that it
    // was tried, or the catch-up loop spins on it (#500).
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    ensure_schema(&connection).await.expect("schema");
    let id = a_message(&connection, "Report attached").await;

    assert_eq!(
        messages_missing_body_text(&connection, 10, None)
            .await
            .expect("candidates")
            .iter()
            .map(|row| row.id)
            .collect::<Vec<_>>(),
        vec![id],
        "local body, never indexed: exactly the work"
    );

    index_body(&connection, id, None)
        .await
        .expect("index nothing");

    assert!(
        messages_missing_body_text(&connection, 10, None)
            .await
            .expect("candidates")
            .iter()
            .map(|row| row.id)
            .collect::<Vec<_>>()
            .is_empty(),
        "tried and found empty is not the same state as never tried, \
         or the maintenance pass asks about this message for ever"
    );
}
