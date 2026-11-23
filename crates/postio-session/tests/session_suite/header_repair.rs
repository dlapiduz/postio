use postio_storage::repository::{MessageRepository, StoredBody};
use postio_storage::{BlobStore, test_support};

const RAW: &[u8] = b"From: Ada Lovelace <ada@example.com>\r\n\
                     Subject: the quarterly reconciliation\r\n\
                     X-Mailer: mutt 1.5.24\r\n\
                     \r\n\
                     the body, which is not a header\r\n";

async fn a_store_from_before() -> (test_support::TempStore, BlobStore, postio_model::MessageId) {
    let database = test_support::temp().await;
    let blobs = BlobStore::open(
        database.directory().join("blobs"),
        &postio_storage::test_support::blob_keys(),
    )
    .expect("a blob store");
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;

    let blob = blobs.put(RAW).expect("put the raw source");
    let mut message = postio_model::Message::new(account.id, inbox, chrono::Utc::now());
    message.raw_blob_id = Some(blob);
    let messages = MessageRepository::new(&connection);
    let id = messages.create(&mut message).await.expect("create");
    messages
        .set_body(
            id,
            &StoredBody {
                text: Some("the body, which is not a header".to_owned()),
                html: None,
                headers: None,
                headers_truncated: false,
                encoding_problems: false,
            },
            postio_model::BodyState::Full,
        )
        .await
        .expect("set");
    drop(connection);
    (database, blobs, id)
}

#[tokio::test]
async fn a_block_is_rebuilt_from_the_raw_source_already_on_disk() {
    let (database, blobs, id) = a_store_from_before().await;

    let repaired = postio_session::repair_header_blocks(&database, &blobs)
        .await
        .expect("the pass runs");

    assert_eq!(repaired, 1);
    let connection = database.connect().await.expect("checkout");
    let headers = MessageRepository::new(&connection)
        .headers(id)
        .await
        .expect("headers")
        .expect("the row");
    assert_eq!(headers.get("x-mailer"), Some("mutt 1.5.24"));
    assert_eq!(
        headers.get("subject"),
        Some("the quarterly reconciliation"),
        "the whole block, not the fields the row already had"
    );
    assert!(
        !headers.contains("the body"),
        "the body is not a header and must not reach the index"
    );
}

#[tokio::test]
async fn a_second_pass_finds_nothing_left_to_do() {
    let (database, blobs, _id) = a_store_from_before().await;

    assert_eq!(
        postio_session::repair_header_blocks(&database, &blobs)
            .await
            .expect("first"),
        1
    );
    assert_eq!(
        postio_session::repair_header_blocks(&database, &blobs)
            .await
            .expect("second"),
        0,
        "the repaired message came back, so the pass cannot terminate"
    );
}

#[tokio::test]
async fn a_message_whose_raw_source_has_gone_is_left_for_the_fetch_lane() {
    let (database, blobs, id) = a_store_from_before().await;
    let connection = database.connect().await.expect("checkout");
    let messages = MessageRepository::new(&connection);
    let blob = messages
        .get(id)
        .await
        .expect("get")
        .expect("row")
        .raw_blob_id
        .expect("a blob");
    std::fs::remove_file(blobs.path_of(&blob).expect("its path")).expect("evict it");
    drop(connection);

    let repaired = postio_session::repair_header_blocks(&database, &blobs)
        .await
        .expect("the pass runs");

    assert_eq!(repaired, 0, "there was nothing it could repair");
    let connection = database.connect().await.expect("checkout");
    let stored = MessageRepository::new(&connection)
        .body(id)
        .await
        .expect("body")
        .expect("the row");
    assert_eq!(
        stored.headers, None,
        "an empty block would be a lie the index could never be talked out of"
    );
}
