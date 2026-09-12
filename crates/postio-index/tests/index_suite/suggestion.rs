//! A search that found nothing offers the word that was meant. #1524, ADR 0037.
//!
//! The ranking is unit-tested in `postio-search` with no database at all.
//! What those tests cannot see is whether the vocabulary they rank is
//! actually there: `fts5vocab` is declared against a live index, over an
//! external-content table, in a temp schema — and a suggestion drawn from an
//! empty term list is a feature that silently never fires.

use chrono::{TimeZone, Utc};
use postio_index::{SearchRequest, search};
use postio_model::AccountScope;
use postio_model::{EmailAddress, Message};
use postio_search::facets::Scope;
use postio_search::parse;
use postio_storage::repository::MessageRepository;
use postio_storage::test_support;
use rusqlite::Connection;

fn at(hour: u32) -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 8, 20, hour, 0, 0).unwrap()
}

fn from(
    connection: &Connection,
    account: &postio_model::Account,
    mailbox: postio_model::MailboxId,
    name: &str,
) {
    let mut message = Message::new(account.id, mailbox, at(9));
    message.from = vec![EmailAddress::new(Some(name), format!("{name}@example.com"))];
    message.subject = Some("Voice lessons".to_owned());
    MessageRepository::new(connection)
        .create(&mut message)
        .expect("create message");
}

fn results_for(
    connection: &Connection,
    account: &postio_model::Account,
    typed: &str,
) -> postio_search::SearchResults {
    let query = parse(typed, at(12).date_naive());
    let request = SearchRequest {
        account: AccountScope::Account(account.id),
        query: &query,
        scope: Scope::AllMail,
        limit: 10,
        order: postio_search::ResultOrder::Relevance,
    };
    search(connection, &request, at(12)).expect("search")
}

#[test]
fn a_misspelled_name_is_answered_with_the_one_in_the_mailbox() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    postio_index::index::ensure_schema(&connection).expect("schema");
    let (account, mailbox) = test_support::account_with_inbox(&connection);
    for _ in 0..3 {
        from(&connection, &account, mailbox, "hannah");
    }

    let results = results_for(&connection, &account, "hanah");

    assert_eq!(results.total_hits, 0, "the match itself stays exact");
    let offered = results
        .suggestion
        .expect("a mailbox holding `hannah` should offer it for `hanah`");
    assert_eq!(offered.term, "hannah");
    assert_eq!(
        offered.documents, 3,
        "the offer says what it would find, which is what makes it worth taking"
    );
}

#[test]
fn a_query_that_found_something_is_not_second_guessed() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    postio_index::index::ensure_schema(&connection).expect("schema");
    let (account, mailbox) = test_support::account_with_inbox(&connection);
    from(&connection, &account, mailbox, "hannah");

    let results = results_for(&connection, &account, "hannah");

    assert!(results.total_hits > 0);
    assert_eq!(
        results.suggestion, None,
        "a query that worked is not one to offer alternatives to"
    );
}

#[test]
fn a_word_the_mailbox_does_not_resemble_gets_no_offer() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    postio_index::index::ensure_schema(&connection).expect("schema");
    let (account, mailbox) = test_support::account_with_inbox(&connection);
    from(&connection, &account, mailbox, "hannah");

    let results = results_for(&connection, &account, "xylophone");

    assert_eq!(results.total_hits, 0);
    assert_eq!(results.suggestion, None, "nothing here resembles it");
}

#[test]
fn a_query_with_a_filter_is_left_alone() {
    // `from:ada hanah` found nothing, and the filter is at least as likely to
    // be why. Correcting the word would answer a question nobody asked.
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    postio_index::index::ensure_schema(&connection).expect("schema");
    let (account, mailbox) = test_support::account_with_inbox(&connection);
    from(&connection, &account, mailbox, "hannah");

    let results = results_for(&connection, &account, "from:ada hanah");

    assert_eq!(results.total_hits, 0);
    assert_eq!(results.suggestion, None);
}
