//! The index schema's upgrade path (#490).
//!
//! `ensure_schema` was idempotent purely by `CREATE ... IF NOT EXISTS`,
//! which adds new tables and triggers and cannot change an existing table's
//! columns. When `list_id` joined `search_documents`, every store created
//! before it kept the old table — the CREATE was a silent no-op — while the
//! *triggers*, which did not exist yet, were created fresh and referenced
//! `new.list_id` immediately. First write, `no column named list_id`, and
//! search went dark on a previously-working store.
//!
//! The mechanism under test: the index records a schema version per half.
//! On mismatch the metadata half — `search_documents`, `messages_fts`, the
//! triggers — is dropped and rebuilt from the mail tables, which it is
//! entirely derived from. `message_bodies_fts` has its own version and is
//! never dropped for a metadata change, because refilling *it* means
//! re-reading every body blob.

use postio_index::index::ensure_schema;
use postio_model::{AccountScope, Message};
use postio_search::facets::Scope;
use postio_storage::repository::MessageRepository;
use postio_storage::test_support;
use rusqlite::Connection;

/// The index schema as it stood before `list_id` (48a2f96): no `list_id`
/// column anywhere, and the trigger set that maintained it. Abbreviated to
/// the pieces the collision needs — the table, the FTS mirror, and the
/// message-insert trigger.
const OLD_SCHEMA: &str = "\
CREATE TABLE IF NOT EXISTS search_documents (
    message_id  INTEGER PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE,
    sender      TEXT NOT NULL DEFAULT '',
    recipients  TEXT NOT NULL DEFAULT '',
    subject     TEXT NOT NULL DEFAULT '',
    filenames   TEXT NOT NULL DEFAULT ''
);
CREATE VIRTUAL TABLE IF NOT EXISTS messages_fts USING fts5(
    sender, recipients, subject, filenames,
    content = 'search_documents',
    content_rowid = 'message_id',
    tokenize = 'unicode61 remove_diacritics 2'
);
CREATE TRIGGER IF NOT EXISTS trg_search_documents_messages_ai
AFTER INSERT ON messages
BEGIN
    INSERT INTO search_documents (message_id, subject)
    VALUES (new.id, coalesce(new.subject, ''))
    ON CONFLICT (message_id) DO UPDATE SET subject = excluded.subject;
END;
CREATE TRIGGER IF NOT EXISTS trg_messages_fts_ai
AFTER INSERT ON search_documents
BEGIN
    INSERT INTO messages_fts (rowid, sender, recipients, subject, filenames)
    VALUES (new.message_id, new.sender, new.recipients, new.subject, new.filenames);
END;
";

fn a_listed_message(connection: &Connection, subject: &str) -> Message {
    let (account, mailbox) = test_support::account_with_inbox(connection);
    let mut message = Message::new(account.id, mailbox, chrono::Utc::now());
    message.subject = Some(subject.to_string());
    message.list_id = Some("harbour-dev.lists.example.org".to_string());
    MessageRepository::new(connection)
        .create(&mut message)
        .expect("create message");
    message
}

fn hits(connection: &Connection, account: postio_model::AccountId, query: &str) -> Vec<i64> {
    let parsed = postio_search::parse(query, chrono::Utc::now().date_naive());
    postio_index::search(
        connection,
        &postio_index::SearchRequest {
            account: AccountScope::Account(account),
            query: &parsed,
            scope: Scope::AllMail,
            limit: 10,
            order: postio_search::ResultOrder::Relevance,
        },
        chrono::Utc::now(),
    )
    .expect("search runs")
    .hits
    .iter()
    .map(|hit| hit.message_id.get())
    .collect()
}

#[test]
fn a_store_from_before_list_id_gains_the_column_and_searches() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    connection
        .execute_batch(OLD_SCHEMA)
        .expect("the old index schema applies");

    ensure_schema(&connection).expect("today's schema applies over the old one");

    // The write that used to die with `no column named list_id`.
    let message = a_listed_message(&connection, "Tuesday walkthrough");

    assert_eq!(
        hits(&connection, message.account_id, "list:harbour-dev"),
        vec![message.id.get()],
        "the upgraded index answers the query the new column exists for"
    );
}

#[test]
fn a_store_already_broken_by_the_mismatch_recovers() {
    // The state real stores are in: the old table, *plus* the new triggers a
    // newer binary's `ensure_schema` layered over it — the ones that
    // reference the column the table never gained. This is what the error
    // in the report was.
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    connection
        .execute_batch(OLD_SCHEMA)
        .expect("the old index schema applies");
    // What shipping `ensure_schema` did to it: no version mechanism meant
    // the new triggers landed over the old table. Reproduced by dropping
    // only the triggers and re-adding today's, exactly as IF NOT EXISTS
    // did — the *tables* stayed.
    connection
        .execute_batch(
            "DROP TRIGGER trg_search_documents_messages_ai;
             CREATE TRIGGER trg_search_documents_messages_ai
             AFTER INSERT ON messages
             BEGIN
                 INSERT INTO search_documents (message_id, subject, list_id)
                 VALUES (new.id, coalesce(new.subject, ''), coalesce(new.list_id, ''))
                 ON CONFLICT (message_id) DO UPDATE SET subject = excluded.subject, list_id = excluded.list_id;
             END;",
        )
        .expect("the mismatched trigger applies");

    ensure_schema(&connection).expect("the repaired schema applies");

    let message = a_listed_message(&connection, "Wednesday walkthrough");
    assert_eq!(
        hits(&connection, message.account_id, "walkthrough"),
        vec![message.id.get()],
        "a store the mismatch had already broken searches again"
    );
}

#[test]
fn a_metadata_upgrade_never_drops_the_body_index() {
    // The body index is refilled from blob reads — minutes on a real
    // archive — so a metadata version bump must leave it exactly as it is.
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    ensure_schema(&connection).expect("schema");
    let message = a_listed_message(&connection, "With a body");
    postio_index::index::index_body(&connection, message.id.get(), Some("the difference engine"))
        .expect("index a body");

    // Force a metadata rebuild the way the next added column will: by
    // regressing the recorded version, not by touching any table.
    connection
        .execute("UPDATE search_schema SET version = version - 1 WHERE half = 'metadata'", [])
        .expect("the version regresses");

    ensure_schema(&connection).expect("the rebuild applies");

    assert_eq!(
        hits(&connection, message.account_id, "difference"),
        vec![message.id.get()],
        "the body index survived the metadata rebuild untouched"
    );
    assert_eq!(
        hits(&connection, message.account_id, "list:harbour-dev"),
        vec![message.id.get()],
        "and the rebuilt metadata half still indexes what the store holds"
    );
}
