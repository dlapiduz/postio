//! `ensure_schema` has to index the mail that is already there.
//!
//! The triggers below it maintain `search_documents` for every message that
//! arrives *after* the schema exists. That is the easy half, and on its own it
//! is worthless: Postio is being retro-fitted with search over stores that
//! already hold tens of thousands of messages, and on a real account the first
//! run of a new index is the run where nothing has arrived yet.
//!
//! `postio-qhz.7` learned this the expensive way one layer down — migration
//! 0003's triggers were correct and the cached mailbox counts stayed zero on
//! every existing store until a one-time backfill was added to the same
//! migration. This is the same shape, so it gets the same answer and a test
//! that says so.

use chrono::Utc;
use postio_index::{SearchRequest, index, search};
use postio_model::AccountScope;
use postio_model::{EmailAddress, Message};
use postio_search::facets::Scope;
use postio_search::parse;
use postio_storage::repository::MessageRepository;
use postio_storage::test_support;

/// A message written *before* the index exists, which is every message on a
/// store that predates the feature.
fn existing_message(
    connection: &rusqlite::Connection,
    account: postio_model::AccountId,
    mailbox: postio_model::MailboxId,
    subject: &str,
) -> Message {
    let mut message = Message::new(account, mailbox, Utc::now());
    message.from = vec![EmailAddress::new(Some("Ada Lovelace"), "ada@example.com")];
    message.subject = Some(subject.to_string());
    MessageRepository::new(connection)
        .create(&mut message)
        .expect("create message");
    message
}

fn find(connection: &rusqlite::Connection, account: postio_model::AccountId, text: &str) -> usize {
    let query = parse(text, Utc::now().date_naive());
    let request = SearchRequest {
        account: AccountScope::Account(account),
        query: &query,
        scope: Scope::AllMail,
        limit: 10,
    };
    search(connection, &request, Utc::now())
        .expect("search")
        .hits
        .len()
}

#[test]
fn mail_that_predates_the_index_is_still_findable() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, mailbox) = test_support::account_with_inbox(&connection);

    // Written first. No schema, no triggers, nothing watching.
    let before = existing_message(&connection, account.id, mailbox, "Quarterly report");

    index::ensure_schema(&connection).expect("schema");

    // And one after, which the triggers handle.
    let after = existing_message(&connection, account.id, mailbox, "Quarterly forecast");

    assert_eq!(
        find(&connection, account.id, "forecast"),
        1,
        "a message inserted after the schema is not indexed, so the triggers \
         themselves are broken"
    );
    assert_eq!(
        find(&connection, account.id, "report"),
        1,
        "the message that was already in the store is not findable. Triggers \
         only index what arrives after them, and on a real account every \
         message arrived before this feature did — so search would come up \
         empty on a store holding tens of thousands of them. `ensure_schema` \
         has to backfill, exactly as migration 0003 does for the mailbox \
         counts."
    );
    let _ = (before, after);
}

#[test]
fn running_it_twice_does_not_duplicate_what_it_indexed() {
    // It runs on every application start, so the backfill has to be a no-op
    // the second time rather than a second copy of every document.
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, mailbox) = test_support::account_with_inbox(&connection);
    existing_message(&connection, account.id, mailbox, "Quarterly report");

    index::ensure_schema(&connection).expect("first");
    index::ensure_schema(&connection).expect("second");

    assert_eq!(
        find(&connection, account.id, "report"),
        1,
        "the message is indexed more than once"
    );
    let documents: i64 = connection
        .query_row("SELECT count(*) FROM search_documents", [], |row| {
            row.get(0)
        })
        .expect("counting documents");
    assert_eq!(
        documents, 1,
        "search_documents holds {documents} rows for one message"
    );
}
