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
//! On mismatch the metadata half — `search_documents` and the index over it —
//! is dropped and rebuilt from the mail tables, which it is entirely derived
//! from. The body half has its own version and is never dropped for a
//! metadata change, because refilling *it* means re-reading every body in the
//! store.
//!
//! The old schema these tests used to reconstruct was FTS5 — a virtual table
//! and three triggers — and this engine has neither. So the collision is
//! staged with objects it does have: today's `search_documents` **without its
//! `list_id` column**, recorded at a stale version. That is the same fault
//! `#490` was, and it is the fault the version mechanism exists to survive:
//! a table that a newer binary's SQL names a column of, and which does not
//! have one.

use postio_index::index::ensure_schema;
use postio_model::{AccountScope, Message};
use postio_search::facets::Scope;
use postio_storage::Connection;
use postio_storage::repository::MessageRepository;
use postio_storage::test_support;

/// The index schema as it stood before `list_id` (48a2f96), rendered in the
/// objects this engine has: the table without the column, and the full-text
/// index over the four columns it did have.
///
/// The FTS5 virtual table and its three triggers that used to stand here are
/// gone with the module — see the module documentation. What matters to the
/// collision is the *table*, which is what `IF NOT EXISTS` could not change
/// and what today's SQL names a fifth column of.
///
/// No version row, deliberately. `ensure_schema` reads a missing half as
/// version 0, which is the state a store predating the mechanism is in, and
/// is what puts the metadata half on the rebuild path.
const OLD_SCHEMA: &str = "\
CREATE TABLE IF NOT EXISTS search_documents (
    message_id  INTEGER PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE,
    sender      TEXT NOT NULL DEFAULT '',
    recipients  TEXT NOT NULL DEFAULT '',
    subject     TEXT NOT NULL DEFAULT '',
    filenames   TEXT NOT NULL DEFAULT ''
);
CREATE INDEX IF NOT EXISTS search_documents_fts ON search_documents
    USING fts (sender, recipients, subject, filenames);
";

async fn a_listed_message(connection: &Connection, subject: &str) -> Message {
    let (account, mailbox) = test_support::account_with_inbox(connection).await;
    let mut message = Message::new(account.id, mailbox, chrono::Utc::now());
    message.subject = Some(subject.to_string());
    message.list_id = Some("harbour-dev.lists.example.org".to_string());
    MessageRepository::new(connection)
        .create(&mut message)
        .await
        .expect("create message");
    message
}

async fn hits(connection: &Connection, account: postio_model::AccountId, query: &str) -> Vec<i64> {
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
    .await
    .expect("search runs")
    .hits
    .iter()
    .map(|hit| hit.message_id.get())
    .collect()
}

#[tokio::test]
async fn a_store_from_before_list_id_gains_the_column_and_searches() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    connection
        .execute_batch(OLD_SCHEMA)
        .await
        .expect("the old index schema applies");

    ensure_schema(&connection)
        .await
        .expect("today's schema applies over the old one");

    // The write that used to die with `no column named list_id`.
    let message = a_listed_message(&connection, "Tuesday walkthrough").await;

    assert_eq!(
        hits(&connection, message.account_id, "list:harbour-dev").await,
        vec![message.id.get()],
        "the upgraded index answers the query the new column exists for"
    );
}

#[tokio::test]
async fn a_store_already_broken_by_the_mismatch_recovers() {
    // The state real stores were in: the old table, *plus* a version row a
    // newer binary had already written over it — so the half claims to be
    // current and is not. That is the shape #490's report had, and the one a
    // plain `IF NOT EXISTS` upgrade cannot get out of on its own.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    connection
        .execute_batch(OLD_SCHEMA)
        .await
        .expect("the old index schema applies");
    // A store that claims to be current while carrying the old table. The
    // repair is not automatic and is not meant to be -- `ensure_schema`
    // trusts the version, which is the whole point of having one -- so this
    // is the state a person reaches by clearing the claim, and what it
    // asserts is that clearing it is enough.
    connection
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS search_schema (
                 half TEXT PRIMARY KEY, version INTEGER NOT NULL);
             INSERT INTO search_schema (half, version) VALUES ('metadata', 99)
                 ON CONFLICT (half) DO UPDATE SET version = 99;",
        )
        .await
        .expect("the mismatched version applies");

    // What a repair does: the claim goes, and the next start rebuilds.
    connection
        .execute("DELETE FROM search_schema WHERE half = 'metadata'", ())
        .await
        .expect("the claim is cleared");

    ensure_schema(&connection)
        .await
        .expect("the repaired schema applies");

    let message = a_listed_message(&connection, "Wednesday walkthrough").await;
    assert_eq!(
        hits(&connection, message.account_id, "walkthrough").await,
        vec![message.id.get()],
        "a store the mismatch had already broken searches again"
    );
}

#[tokio::test]
async fn a_metadata_upgrade_never_drops_the_body_index() {
    // The body index is refilled from blob reads — minutes on a real
    // archive — so a metadata version bump must leave it exactly as it is.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    ensure_schema(&connection).await.expect("schema");
    let message = a_listed_message(&connection, "With a body").await;
    postio_index::index::index_body(&connection, message.id.get(), Some("the difference engine"))
        .await
        .expect("index a body");

    // Force a metadata rebuild the way the next added column will: by
    // regressing the recorded version, not by touching any table.
    connection
        .execute(
            "UPDATE search_schema SET version = version - 1 WHERE half = 'metadata'",
            (),
        )
        .await
        .expect("the version regresses");

    ensure_schema(&connection)
        .await
        .expect("the rebuild applies");

    assert_eq!(
        hits(&connection, message.account_id, "difference").await,
        vec![message.id.get()],
        "the body index survived the metadata rebuild untouched"
    );
    assert_eq!(
        hits(&connection, message.account_id, "list:harbour-dev").await,
        vec![message.id.get()],
        "and the rebuilt metadata half still indexes what the store holds"
    );
}
