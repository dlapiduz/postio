//! End-to-end tests for [`postio_index::search`]: a real (in-memory) schema,
//! real rows, a parsed query and the executor wired together.
//!
//! Ranking's *orderings* are covered by the pure `rank_score` unit tests in
//! `executor.rs` — nothing here needs to reproduce those, only confirm the
//! executor calls through to it and returns results in that order.

use chrono::{TimeZone, Utc};
use postio_index::{SearchRequest, search};
use postio_model::{Attachment, EmailAddress, Message};
use postio_search::facets::Scope;
use postio_search::parse;
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
    postio_index::index::ensure_schema(&connection).expect("schema");
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
        scope: Scope::AllMail,
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
fn list_names_a_mailing_list_by_its_list_id_not_by_a_recipient_address() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    postio_index::index::ensure_schema(&connection).expect("schema");
    let (account, mailbox) = test_support::account_with_inbox(&connection);

    let mut on_list = message(
        &connection,
        &account,
        mailbox,
        "ada",
        "Tuesday walkthrough",
        at(9),
    );
    on_list.list_id = Some("harbour-dev.lists.example.org".to_string());
    MessageRepository::new(&connection)
        .update(&mut on_list)
        .expect("update message");

    // Mentions the list's address only among its recipients, with no
    // `List-Id` of its own — the old approximation would have matched this
    // one too.
    let mut off_list = message(
        &connection,
        &account,
        mailbox,
        "bob",
        "Fwd: for your files",
        at(10),
    );
    off_list.to = vec![EmailAddress::new(
        None::<String>,
        "harbour-dev@lists.example.org",
    )];
    MessageRepository::new(&connection)
        .update(&mut off_list)
        .expect("update message");

    let query = parse("list:harbour-dev", at(12).date_naive());
    let request = SearchRequest {
        account_id: account.id,
        query: &query,
        scope: Scope::AllMail,
        limit: 10,
    };
    let results = search(&connection, &request, at(12)).expect("search");

    assert_eq!(results.hits.len(), 1);
    assert_eq!(results.hits[0].message_id, on_list.id);
}

#[test]
fn negated_only_free_text_excludes_without_a_positive_match_expression() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    postio_index::index::ensure_schema(&connection).expect("schema");
    let (account, mailbox) = test_support::account_with_inbox(&connection);

    let clean = message(&connection, &account, mailbox, "ada", "Weekly sync", at(9));
    let spammy = message(&connection, &account, mailbox, "ada", "Weekly sync", at(10));
    postio_index::index::index_body(&connection, spammy.id.get(), Some("buy cheap watches now"))
        .expect("index body");
    postio_index::index::index_body(&connection, clean.id.get(), Some("see you at standup"))
        .expect("index body");

    let query = parse("-watches", at(12).date_naive());
    let request = SearchRequest {
        account_id: account.id,
        query: &query,
        scope: Scope::AllMail,
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
    postio_index::index::ensure_schema(&connection).expect("schema");
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
        scope: Scope::AllMail,
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
    postio_index::index::ensure_schema(&connection).expect("schema");
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
        scope: Scope::AllMail,
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
    postio_index::index::ensure_schema(&connection).expect("schema");
    let (account, mailbox) = test_support::account_with_inbox(&connection);

    let older = message(&connection, &account, mailbox, "ada", "One", at(8));
    let newer = message(&connection, &account, mailbox, "ada", "Two", at(10));

    let query = parse("from:ada", at(12).date_naive());
    let request = SearchRequest {
        account_id: account.id,
        query: &query,
        scope: Scope::AllMail,
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
    postio_index::index::ensure_schema(&connection).expect("schema");
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
        scope: Scope::AllMail,
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
    postio_index::index::ensure_schema(&connection).expect("schema");
    let (account, mailbox) = test_support::account_with_inbox(&connection);

    let found = message(
        &connection,
        &account,
        mailbox,
        "ada",
        "Rebuild status",
        at(9),
    );
    postio_index::index::index_body(
        &connection,
        found.id.get(),
        Some("the maildir rebuild finished overnight"),
    )
    .expect("index body");

    let query = parse("rebuild", at(12).date_naive());
    let request = SearchRequest {
        account_id: account.id,
        query: &query,
        scope: Scope::AllMail,
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
    postio_index::index::ensure_schema(&connection).expect("schema");
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
        scope: Scope::AllMail,
        limit: 10,
    };
    let results = search(&connection, &request, at(12)).expect("search");

    assert_eq!(results.hits.len(), 1);
    assert_eq!(results.hits[0].message_id, with_attachment.id);
}

// ---------------------------------------------------------------------------
// Scope and facets — canvas 2b's left column
// ---------------------------------------------------------------------------

/// An account with an inbox and a folder list mail is filed into, plus the
/// same subject in both so a query spans the scopes.
fn split_across_scopes(connection: &Connection) -> (postio_model::Account, u64) {
    let (account, inbox) = test_support::account_with_inbox(connection);
    let folder = test_support::mailbox(connection, &account, "lkml");

    message(
        connection,
        &account,
        inbox,
        "ada",
        "Quarterly report",
        at(9),
    );
    message(
        connection,
        &account,
        inbox,
        "bob",
        "Quarterly figures",
        at(10),
    );
    message(
        connection,
        &account,
        folder.id,
        "lkml",
        "Quarterly patch queue",
        at(11),
    );
    (account, 3)
}

#[test]
fn a_scope_narrows_the_search_without_touching_the_query() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    postio_index::index::ensure_schema(&connection).expect("schema");
    let (account, total) = split_across_scopes(&connection);

    let query = parse("quarterly", at(12).date_naive());
    let counts = |scope| {
        let request = SearchRequest {
            account_id: account.id,
            query: &query,
            scope,
            limit: 10,
        };
        search(&connection, &request, at(12))
            .expect("search")
            .total_hits
    };

    assert_eq!(counts(Scope::AllMail), total);
    assert_eq!(counts(Scope::Inbox), 2);
    assert_eq!(counts(Scope::Lists), 1);
    assert_eq!(
        query.input(),
        "quarterly",
        "the scope is not a token; the box still says what was typed"
    );
}

#[test]
fn the_scope_column_counts_what_switching_would_find() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    postio_index::index::ensure_schema(&connection).expect("schema");
    let (account, total) = split_across_scopes(&connection);

    let query = parse("quarterly", at(12).date_naive());
    // Measured from inside the Inbox scope: the other rows still have to say
    // what is behind them, or switching is a guess.
    let request = SearchRequest {
        account_id: account.id,
        query: &query,
        scope: Scope::Inbox,
        limit: 10,
    };
    let facets = postio_index::executor::facets(&connection, &request).expect("facets");

    assert_eq!(facets.hits(Scope::AllMail), total);
    assert_eq!(facets.hits(Scope::Inbox), 2);
    assert_eq!(facets.hits(Scope::Lists), 1);
}

#[test]
fn refinements_are_measured_against_the_result_set_and_not_the_mailbox() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    postio_index::index::ensure_schema(&connection).expect("schema");
    let (account, inbox) = test_support::account_with_inbox(&connection);

    // Two hits, one of them unread; and an unread message the query misses,
    // which must not be counted.
    let mut matching_unread = Message::new(account.id, inbox, at(9));
    matching_unread.subject = Some("Quarterly report".to_string());
    let mut matching_read = Message::new(account.id, inbox, at(10));
    matching_read.subject = Some("Quarterly figures".to_string());
    matching_read.flags.insert(postio_model::Flag::Seen);
    let mut missing = Message::new(account.id, inbox, at(11));
    missing.subject = Some("Lunch plans".to_string());
    for message in [&mut matching_unread, &mut matching_read, &mut missing] {
        MessageRepository::new(&connection)
            .create(message)
            .expect("create");
    }

    let query = parse("quarterly", at(12).date_naive());
    let request = SearchRequest {
        account_id: account.id,
        query: &query,
        scope: Scope::AllMail,
        limit: 10,
    };
    let facets = postio_index::executor::facets(&connection, &request).expect("facets");

    let unread = facets
        .refinements
        .iter()
        .find(|refinement| refinement.token == "is:unread")
        .expect("`is:unread` is always measured");
    assert_eq!(unread.hits, 1, "the third unread message is not a hit");

    // One of two, so it narrows without emptying: exactly what gets offered.
    let offered: Vec<&str> = facets
        .suggested(2)
        .iter()
        .map(|refinement| refinement.token.as_str())
        .collect();
    assert!(offered.contains(&"is:unread"), "offered: {offered:?}");
    assert!(
        !offered.contains(&"is:flagged"),
        "nothing is flagged, so it is a dead end: {offered:?}"
    );
}

#[test]
fn a_folder_the_matches_are_in_is_offered_as_a_token_that_parses_back() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    postio_index::index::ensure_schema(&connection).expect("schema");
    let (account, _) = split_across_scopes(&connection);

    let query = parse("quarterly", at(12).date_naive());
    let request = SearchRequest {
        account_id: account.id,
        query: &query,
        scope: Scope::AllMail,
        limit: 10,
    };
    let facets = postio_index::executor::facets(&connection, &request).expect("facets");

    let folder = facets
        .refinements
        .iter()
        .find(|refinement| refinement.token.starts_with("in:"))
        .expect("the matches are in folders, so a folder chip is offered");

    // A chip is a token the user could have typed, so it has to survive being
    // typed: appending it and re-running must narrow to what the chip claimed.
    let refined = postio_search::facets::append(query.input(), &folder.token);
    let refined = parse(&refined, at(12).date_naive());
    let request = SearchRequest {
        account_id: account.id,
        query: &refined,
        scope: Scope::AllMail,
        limit: 10,
    };
    let narrowed = search(&connection, &request, at(12)).expect("search");
    assert_eq!(narrowed.total_hits, folder.hits);
}

#[test]
fn the_size_refinement_is_spelled_the_way_the_parser_reads_it() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    postio_index::index::ensure_schema(&connection).expect("schema");
    let (account, inbox) = test_support::account_with_inbox(&connection);

    let mut big = Message::new(account.id, inbox, at(9));
    big.subject = Some("Quarterly report".to_string());
    big.size = 4 * 1024 * 1024;
    let mut small = Message::new(account.id, inbox, at(10));
    small.subject = Some("Quarterly note".to_string());
    small.size = 900;
    for message in [&mut big, &mut small] {
        MessageRepository::new(&connection)
            .create(message)
            .expect("create");
    }

    let query = parse("quarterly", at(12).date_naive());
    let request = SearchRequest {
        account_id: account.id,
        query: &query,
        scope: Scope::AllMail,
        limit: 10,
    };
    let facets = postio_index::executor::facets(&connection, &request).expect("facets");

    let large = facets
        .refinements
        .iter()
        .find(|refinement| refinement.token.starts_with("larger:"))
        .expect("`larger:` is always measured");
    assert_eq!(large.hits, 1);

    let refined = parse(
        &postio_search::facets::append(query.input(), &large.token),
        at(12).date_naive(),
    );
    let request = SearchRequest {
        account_id: account.id,
        query: &refined,
        scope: Scope::AllMail,
        limit: 10,
    };
    let narrowed = search(&connection, &request, at(12)).expect("search");
    assert_eq!(
        narrowed.hits.len(),
        1,
        "the chip's own token has to reproduce the count it advertised"
    );
    assert_eq!(narrowed.hits[0].message_id, big.id);
}

#[test]
fn a_query_that_matches_nothing_offers_nothing_rather_than_dead_ends() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    postio_index::index::ensure_schema(&connection).expect("schema");
    let (account, _) = split_across_scopes(&connection);

    let query = parse("nothingmatchesthis", at(12).date_naive());
    let request = SearchRequest {
        account_id: account.id,
        query: &query,
        scope: Scope::AllMail,
        limit: 10,
    };
    let facets = postio_index::executor::facets(&connection, &request).expect("facets");

    assert_eq!(facets.hits(Scope::AllMail), 0);
    assert!(facets.suggested(0).is_empty());
}

// ---------------------------------------------------------------------------
// Highlighting — canvas 2b's "preview · match highlighted"
// ---------------------------------------------------------------------------

#[test]
fn a_snippet_comes_back_with_the_match_marked_where_the_query_hit() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    postio_index::index::ensure_schema(&connection).expect("schema");
    let (account, inbox) = test_support::account_with_inbox(&connection);

    let mut message = Message::new(account.id, inbox, at(9));
    message.subject = Some("Index rebuild".to_string());
    MessageRepository::new(&connection)
        .create(&mut message)
        .expect("create");
    postio_index::index::index_body(
        &connection,
        message.id.get(),
        Some("the rebuild walks every maildir once per folder"),
    )
    .expect("index body");

    let query = parse("maildir", at(12).date_naive());
    let request = SearchRequest {
        account_id: account.id,
        query: &query,
        scope: Scope::AllMail,
        limit: 10,
    };
    let results = search(&connection, &request, at(12)).expect("search");

    let highlighted = postio_search::highlight::from_snippet(&results.hits[0].snippet);
    assert!(
        highlighted.is_highlighted(),
        "snippet: {:?}",
        results.hits[0].snippet
    );
    let matched: Vec<&str> = highlighted
        .runs()
        .into_iter()
        .filter(|(_, hit)| *hit)
        .map(|(run, _)| run)
        .collect();
    assert_eq!(matched, ["maildir"]);
    assert!(
        !highlighted
            .text
            .contains(postio_search::highlight::MATCH_START),
        "the markers are gone from the text a widget draws"
    );
}
