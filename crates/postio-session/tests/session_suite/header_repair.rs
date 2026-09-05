//! Repairing the header blocks of mail that was downloaded before there was
//! anywhere to put them (#884).
//!
//! `messages.body_headers` has been NULL on every row in every store since
//! migration 0001, because both backfill paths passed `headers: None` on
//! purpose. New mail carries its block from now on; this is the pass for the
//! mail that is already here, and without it `header:` answers "no such mail"
//! for a mailbox somebody has been using for a year.
//!
//! **It reaches no network.** The raw source is already on disk for every
//! message this pass touches — that is what makes it a repair rather than a
//! re-download — and the test below proves it by handing the pass a store and
//! nothing else to talk to.

use postio_storage::repository::{MessageRepository, StoredBody};
use postio_storage::{BlobStore, test_support};

const RAW: &[u8] = b"From: Ada Lovelace <ada@example.com>\r\n\
                     Subject: the quarterly reconciliation\r\n\
                     X-Mailer: mutt 1.5.24\r\n\
                     \r\n\
                     the body, which is not a header\r\n";

/// A store holding one message whose body was fetched before blocks were
/// stored: raw source on disk, `body_headers` NULL.
fn a_store_from_before() -> (
    test_support::TempDatabase,
    BlobStore,
    postio_model::MessageId,
) {
    let database = test_support::temp();
    let blobs = BlobStore::open(
        database.directory().join("blobs"),
        &postio_storage::test_support::blob_keys(),
    )
    .expect("a blob store");
    let connection = database.connection().expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection);

    let blob = blobs.put(RAW).expect("put the raw source");
    let mut message = postio_model::Message::new(account.id, inbox, chrono::Utc::now());
    message.raw_blob_id = Some(blob);
    let messages = MessageRepository::new(&connection);
    let id = messages.create(&mut message).expect("create");
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
        .expect("set");
    drop(connection);
    (database, blobs, id)
}

#[test]
fn a_block_is_rebuilt_from_the_raw_source_already_on_disk() {
    let (database, blobs, id) = a_store_from_before();

    let repaired = postio_session::repair_header_blocks(&database, &blobs).expect("the pass runs");

    assert_eq!(repaired, 1);
    let connection = database.connection().expect("checkout");
    let headers = MessageRepository::new(&connection)
        .headers(id)
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

#[test]
fn a_second_pass_finds_nothing_left_to_do() {
    // The contract #500's guard is watching: repairing a message has to remove
    // it from the candidate query. A pass that kept being offered the same
    // batch would run at 100% of a core for as long as the app was open.
    let (database, blobs, _id) = a_store_from_before();

    assert_eq!(
        postio_session::repair_header_blocks(&database, &blobs).expect("first"),
        1
    );
    assert_eq!(
        postio_session::repair_header_blocks(&database, &blobs).expect("second"),
        0,
        "the repaired message came back, so the pass cannot terminate"
    );
}

#[test]
fn a_message_whose_raw_source_has_gone_is_left_for_the_fetch_lane() {
    // Eviction takes raw source first (PRODUCT.md §6), so this is the ordinary
    // state of the oldest mail in a store with a ceiling. The row still points
    // at a blob that is not there, and the pass must not treat "I could not
    // read it" as "there is nothing to read": writing an empty block would
    // make `header:` answer "no such header" for ever, with nothing left to
    // say otherwise.
    let (database, blobs, id) = a_store_from_before();
    let connection = database.connection().expect("checkout");
    let messages = MessageRepository::new(&connection);
    let blob = messages
        .get(id)
        .expect("get")
        .expect("row")
        .raw_blob_id
        .expect("a blob");
    std::fs::remove_file(blobs.path_of(&blob).expect("its path")).expect("evict it");
    drop(connection);

    let repaired = postio_session::repair_header_blocks(&database, &blobs).expect("the pass runs");

    assert_eq!(repaired, 0, "there was nothing it could repair");
    let connection = database.connection().expect("checkout");
    let stored = MessageRepository::new(&connection)
        .body(id)
        .expect("body")
        .expect("the row");
    assert_eq!(
        stored.headers, None,
        "an empty block would be a lie the index could never be talked out of"
    );
}
