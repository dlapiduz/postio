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

#[test]
fn a_store_full_of_textless_bodies_is_swept_once_and_left_alone() {
    let database = test_support::temp();
    let connection = database.connection().expect("checkout");
    postio_index::index::ensure_schema(&connection).expect("schema");
    let (account, inbox) = test_support::account_with_inbox(&connection);

    // Local body, no text: what an attachment-only message looks like to the
    // pass. `body` answers a row holding no parts, the indexable text is
    // empty, and before #500 that meant the message never left the candidate
    // set.
    let messages = MessageRepository::new(&connection);
    connection.execute_batch("BEGIN").expect("begin fixture");
    for i in 0..TEXTLESS {
        let mut message = Message::new(
            account.id,
            inbox,
            chrono::Utc::now() - chrono::Duration::minutes(i as i64),
        );
        message.subject = Some(format!("Report {i} attached"));
        message.sync.body_state = BodyState::Full;
        messages.create(&mut message).expect("create");
    }
    connection.execute_batch("COMMIT").expect("commit fixture");
    drop(connection);

    let indexed = postio_session::index_local_bodies(&database).expect("the pass runs");
    assert_eq!(indexed, TEXTLESS, "every message was visited exactly once");

    let connection = database.connection().expect("checkout");
    assert!(
        postio_index::index::messages_missing_body_text(&connection, 10)
            .expect("candidates")
            .is_empty(),
        "a swept store leaves no candidates, or the next start sweeps it again"
    );

    drop(connection);
    let second = postio_session::index_local_bodies(&database).expect("the second pass");
    assert_eq!(second, 0, "a caught-up store costs one query and no writes");
}
