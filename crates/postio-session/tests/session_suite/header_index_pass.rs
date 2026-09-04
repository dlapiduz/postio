//! The catch-up pass over header blocks that are already local (ADR 0025 Q5).
//!
//! `header:` matches `message_headers`, which is derived from
//! `messages.body_headers` — so on every store that exists, and after every
//! bump of the headers schema half, there is a mailbox's worth of blocks with
//! no rows. Without this pass `header:` answers "no such mail" across a
//! mailbox somebody has been using for a year, which is indistinguishable
//! from the feature being broken.
//!
//! The row-level promises (a block that parses to nothing still records that
//! it was tried; re-indexing replaces rather than accumulates) are asserted
//! in `postio-index`'s own tests. These are the pass's: it terminates, it
//! leaves nothing for a second run, and it makes mail that was already here
//! findable.

use postio_model::{AccountScope, BodyState, Message};
use postio_search::facets::Scope;
use postio_storage::repository::{MessageRepository, StoredBody};
use postio_storage::test_support;

/// More messages than one `INDEX_HEADERS_BATCH`, so the pass has to come back
/// for a second batch rather than finishing in one.
const MESSAGES: usize = 260;

fn a_block(nth: usize) -> String {
    format!(
        "Received: from hop-{nth}.example.com by mx.example.net\r\n\
         From: ada@example.com\r\n\
         Subject: Engine notes {nth}\r\n\
         X-Mailer: Mutt 1.5.24 (2015-08-30)"
    )
}

fn hits(connection: &rusqlite::Connection, account: postio_model::AccountId, query: &str) -> usize {
    let parsed = postio_search::parse(query, chrono::Utc::now().date_naive());
    postio_index::search(
        connection,
        &postio_index::SearchRequest {
            account: AccountScope::Account(account),
            query: &parsed,
            scope: Scope::AllMail,
            limit: 500,
            order: postio_search::ResultOrder::Relevance,
        },
        chrono::Utc::now(),
    )
    .expect("search runs")
    .total_hits as usize
}

#[test]
fn mail_that_was_already_here_becomes_findable_by_header_and_stays_swept() {
    let database = test_support::temp();
    let connection = database.connection().expect("checkout");
    postio_index::index::ensure_schema(&connection).expect("schema");
    let (account, inbox) = test_support::account_with_inbox(&connection);

    let messages = MessageRepository::new(&connection);
    for nth in 0..MESSAGES {
        let mut message = Message::new(
            account.id,
            inbox,
            chrono::Utc::now() - chrono::Duration::minutes(nth as i64),
        );
        message.subject = Some(format!("Engine notes {nth}"));
        messages.create(&mut message).expect("create");
        messages
            .set_body(
                message.id,
                &StoredBody {
                    text: Some("the body".to_string()),
                    html: None,
                    headers: Some(a_block(nth)),
                    headers_truncated: false,
                    encoding_problems: false,
                },
                BodyState::Full,
            )
            .expect("store a block");
    }

    // Before the pass: the blocks are on disk and `header:` cannot see them.
    assert_eq!(
        hits(&connection, account.id, "header:x-mailer=mutt"),
        0,
        "a stored block nothing has indexed is not yet a hit"
    );
    drop(connection);

    let indexed = postio_session::index_local_headers(&database).expect("the pass runs");
    assert_eq!(indexed, MESSAGES, "every block was visited exactly once");

    let connection = database.connection().expect("checkout");
    assert_eq!(
        hits(&connection, account.id, "header:x-mailer=mutt"),
        MESSAGES,
        "and now every one of them answers the operator the pass exists for"
    );
    assert_eq!(
        hits(&connection, account.id, "header:received=hop-7.example.com"),
        1,
        "including a value that is unique to one message"
    );
    assert!(
        postio_index::index::messages_missing_header_rows(&connection, 10)
            .expect("candidates")
            .is_empty(),
        "a swept store leaves no candidates, or the next start sweeps it again"
    );

    drop(connection);
    let second = postio_session::index_local_headers(&database).expect("the second pass");
    assert_eq!(second, 0, "a caught-up store costs one query and no writes");
}

#[test]
fn a_store_whose_blocks_were_never_written_gives_the_pass_nothing_to_do() {
    // ADR 0025 Q5's other two populations. `body_headers` NULL is
    // `repair_header_blocks`'s work or the backfill lane's, and offering it
    // here would be a batch this pass can make no progress on.
    let database = test_support::temp();
    let connection = database.connection().expect("checkout");
    postio_index::index::ensure_schema(&connection).expect("schema");
    let (account, inbox) = test_support::account_with_inbox(&connection);

    let messages = MessageRepository::new(&connection);
    for nth in 0..8 {
        let mut message = Message::new(
            account.id,
            inbox,
            chrono::Utc::now() - chrono::Duration::minutes(nth),
        );
        message.sync.body_state = BodyState::Full;
        messages.create(&mut message).expect("create");
    }
    drop(connection);

    assert_eq!(
        postio_session::index_local_headers(&database).expect("the pass runs"),
        0,
        "no stored block, nothing this pass can do without a network"
    );
}
