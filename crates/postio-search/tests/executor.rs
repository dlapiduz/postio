//! End-to-end tests for [`postio_search::search`]: a real (in-memory) schema,
//! real rows, a parsed query and the executor wired together.
//!
//! Ranking's *orderings* are covered by the pure `rank_score` unit tests in
//! `executor.rs` — nothing here needs to reproduce those, only confirm the
//! executor calls through to it and returns results in that order.
//!
//! Everything here needs the `index` cargo feature (`executor`, `index`,
//! `postio-model`); see `src/lib.rs` for why that feature defaults off.

#![cfg(feature = "index")]

use chrono::{TimeZone, Utc};
use postio_model::{Attachment, EmailAddress, Message};
use postio_search::{SearchRequest, parse, search};
use postio_storage::repository::MessageRepository;
use postio_storage::test_support;
use rusqlite::Connection;

fn at(hour: u32) -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 8, 20, hour, 0, 0).unwrap()
}

fn message(
    connection: &Connection,
    account: &postio_model::Account,
    mailbox: postio_model::MailboxId,
    from: &str,
    subject: &str,
    received_at: chrono::DateTime<Utc>,
) -> Message {
    let mut message = Message::new(account.id, mailbox, received_at);
    message.from = vec![EmailAddress::new(Some(from), format!("{from}@example.com"))];
    message.subject = Some(subject.to_string());
    MessageRepository::new(connection)
        .create(&mut message)
        .expect("create message");
    message
}

#[test]
fn a_composed_operator_and_free_text_query_narrows_correctly() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    postio_search::index::ensure_schema(&connection).expect("schema");
    let (account, mailbox) = test_support::account_with_inbox(&connection);

    let matching = message(
        &connection,
        &account,
        mailbox,
        "ada",
        "Quarterly report",
        at(9),
    );
    let _wrong_sender = message(
        &connection,
        &account,
        mailbox,
        "bob",
        "Quarterly report",
        at(8),
    );
    let _wrong_text = message(&connection, &account, mailbox, "ada", "Lunch plans", at(10));

    let query = parse("report from:ada", at(12).date_naive());
    let request = SearchRequest {
        account_id: account.id,
        query: &query,
        limit: 10,
    };
    let results = search(&connection, &request, at(12)).expect("search");

    assert_eq!(results.hits.len(), 1);
    assert_eq!(results.hits[0].message_id, matching.id);
    assert_eq!(results.total_hits, 1);
    assert!(
        results.hits[0].snippet.to_lowercase().contains("report"),
        "the snippet should highlight the free-text match, not the from: \
         operator's own messages_fts condition: {:?}",
        results.hits[0].snippet
    );
}

#[test]
fn negated_only_free_text_excludes_without_a_positive_match_expression() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    postio_search::index::ensure_schema(&connection).expect("schema");
    let (account, mailbox) = test_support::account_with_inbox(&connection);

    let clean = message(&connection, &account, mailbox, "ada", "Weekly sync", at(9));
    let spammy = message(&connection, &account, mailbox, "ada", "Weekly sync", at(10));
    postio_search::index::index_body(&connection, spammy.id.get(), Some("buy cheap watches now"))
        .expect("index body");
    postio_search::index::index_body(&connection, clean.id.get(), Some("see you at standup"))
        .expect("index body");

    let query = parse("-watches", at(12).date_naive());
    let request = SearchRequest {
        account_id: account.id,
        query: &query,
        limit: 10,
    };
    let results = search(&connection, &request, at(12)).expect("search");

    assert_eq!(results.hits.len(), 1);
    assert_eq!(results.hits[0].message_id, clean.id);
}

#[test]
fn total_hits_counts_every_match_regardless_of_the_page_limit() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    postio_search::index::ensure_schema(&connection).expect("schema");
    let (account, mailbox) = test_support::account_with_inbox(&connection);

    for hour in 0..5 {
        message(
            &connection,
            &account,
            mailbox,
            "ada",
            "Standup notes",
            at(hour),
        );
    }

    let query = parse("standup", at(12).date_naive());
    let request = SearchRequest {
        account_id: account.id,
        query: &query,
        limit: 2,
    };
    let results = search(&connection, &request, at(12)).expect("search");

    assert_eq!(results.hits.len(), 2, "limited to the page size");
    assert_eq!(results.total_hits, 5, "but the total reflects every match");
    assert!(!results.total_hits_capped);
}

#[test]
fn total_hits_stops_counting_at_the_cap() {
    use postio_search::TOTAL_HITS_CAP;

    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    postio_search::index::ensure_schema(&connection).expect("schema");
    let (account, mailbox) = test_support::account_with_inbox(&connection);

    connection
        .execute_batch("BEGIN")
        .expect("start bulk load transaction");
    for _ in 0..(TOTAL_HITS_CAP + 50) {
        message(&connection, &account, mailbox, "ada", "Bulk notes", at(0));
    }
    connection
        .execute_batch("COMMIT")
        .expect("commit bulk load transaction");

    let query = parse("bulk", at(12).date_naive());
    let request = SearchRequest {
        account_id: account.id,
        query: &query,
        limit: 5,
    };
    let results = search(&connection, &request, at(12)).expect("search");

    assert!(results.total_hits_capped);
    assert_eq!(results.total_hits, TOTAL_HITS_CAP);
}

#[test]
fn a_structured_only_query_orders_newest_first_and_carries_no_snippet() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    postio_search::index::ensure_schema(&connection).expect("schema");
    let (account, mailbox) = test_support::account_with_inbox(&connection);

    let older = message(&connection, &account, mailbox, "ada", "One", at(8));
    let newer = message(&connection, &account, mailbox, "ada", "Two", at(10));

    let query = parse("from:ada", at(12).date_naive());
    let request = SearchRequest {
        account_id: account.id,
        query: &query,
        limit: 10,
    };
    let results = search(&connection, &request, at(12)).expect("search");

    assert_eq!(results.hits.len(), 2);
    assert_eq!(results.hits[0].message_id, newer.id);
    assert_eq!(results.hits[1].message_id, older.id);
    assert!(results.hits.iter().all(|hit| hit.snippet.is_empty()));
}

#[test]
fn search_never_crosses_accounts() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    postio_search::index::ensure_schema(&connection).expect("schema");
    let (account_a, mailbox_a) = test_support::account_with_inbox(&connection);
    let account_b = test_support::account(&connection);
    let mailbox_b = test_support::mailbox(&connection, &account_b, "INBOX").id;

    let mine = message(
        &connection,
        &account_a,
        mailbox_a,
        "ada",
        "Shared subject",
        at(9),
    );
    let _theirs = message(
        &connection,
        &account_b,
        mailbox_b,
        "ada",
        "Shared subject",
        at(9),
    );

    let query = parse("shared", at(12).date_naive());
    let request = SearchRequest {
        account_id: account_a.id,
        query: &query,
        limit: 10,
    };
    let results = search(&connection, &request, at(12)).expect("search");

    assert_eq!(results.hits.len(), 1);
    assert_eq!(results.hits[0].message_id, mine.id);
}

#[test]
fn a_matching_query_produces_a_highlighted_snippet() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    postio_search::index::ensure_schema(&connection).expect("schema");
    let (account, mailbox) = test_support::account_with_inbox(&connection);

    let found = message(
        &connection,
        &account,
        mailbox,
        "ada",
        "Rebuild status",
        at(9),
    );
    postio_search::index::index_body(
        &connection,
        found.id.get(),
        Some("the maildir rebuild finished overnight"),
    )
    .expect("index body");

    let query = parse("rebuild", at(12).date_naive());
    let request = SearchRequest {
        account_id: account.id,
        query: &query,
        limit: 10,
    };
    let results = search(&connection, &request, at(12)).expect("search");

    assert_eq!(results.hits.len(), 1);
    assert!(
        results.hits[0].snippet.contains('\u{1}') && results.hits[0].snippet.contains('\u{2}'),
        "snippet should wrap the match: {:?}",
        results.hits[0].snippet
    );
}

#[test]
fn filename_and_has_attachment_operators_filter_correctly() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    postio_search::index::ensure_schema(&connection).expect("schema");
    let (account, mailbox) = test_support::account_with_inbox(&connection);

    let mut with_attachment = Message::new(account.id, mailbox, at(9));
    with_attachment.subject = Some("Numbers".to_string());
    let mut attachment = Attachment::new(with_attachment.id, "text/plain", 10);
    attachment.filename = Some("timings.txt".to_string());
    with_attachment.attachments = vec![attachment];
    MessageRepository::new(&connection)
        .create(&mut with_attachment)
        .expect("create");

    let mut without_attachment = Message::new(account.id, mailbox, at(10));
    without_attachment.subject = Some("Numbers".to_string());
    MessageRepository::new(&connection)
        .create(&mut without_attachment)
        .expect("create");

    let query = parse("has:attach filename:timings", at(12).date_naive());
    let request = SearchRequest {
        account_id: account.id,
        query: &query,
        limit: 10,
    };
    let results = search(&connection, &request, at(12)).expect("search");

    assert_eq!(results.hits.len(), 1);
    assert_eq!(results.hits[0].message_id, with_attachment.id);
}
