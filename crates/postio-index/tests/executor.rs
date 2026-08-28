//! End-to-end tests for [`postio_index::search`]: a real (in-memory) schema,
//! real rows, a parsed query and the executor wired together.
//!
//! Ranking's *orderings* are covered by the pure `rank_score` unit tests in
//! `executor.rs` — nothing here needs to reproduce those, only confirm the
//! executor calls through to it and returns results in that order.

use chrono::{TimeZone, Utc};
use postio_index::{SearchRequest, search};
use postio_model::AccountScope;
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
        account: AccountScope::Account(account.id),
        query: &query,
        scope: Scope::AllMail,
        limit: 10,
        order: postio_search::ResultOrder::Relevance,
    };
    let results = search(&connection, &request, at(12)).expect("search");

    assert_eq!(results.hits.len(), 1);
    assert_eq!(results.hits[0].message_id, matching.id);
    assert_eq!(results.total_hits, 1);
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
        account: AccountScope::Account(account.id),
        query: &query,
        scope: Scope::AllMail,
        limit: 10,
        order: postio_search::ResultOrder::Relevance,
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
        account: AccountScope::Account(account.id),
        query: &query,
        scope: Scope::AllMail,
        limit: 10,
        order: postio_search::ResultOrder::Relevance,
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
        account: AccountScope::Account(account.id),
        query: &query,
        scope: Scope::AllMail,
        limit: 2,
        order: postio_search::ResultOrder::Relevance,
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
        account: AccountScope::Account(account.id),
        query: &query,
        scope: Scope::AllMail,
        limit: 5,
        order: postio_search::ResultOrder::Relevance,
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
        account: AccountScope::Account(account.id),
        query: &query,
        scope: Scope::AllMail,
        limit: 10,
        order: postio_search::ResultOrder::Relevance,
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
        account: AccountScope::Account(account_a.id),
        query: &query,
        scope: Scope::AllMail,
        limit: 10,
        order: postio_search::ResultOrder::Relevance,
    };
    let results = search(&connection, &request, at(12)).expect("search");

    assert_eq!(results.hits.len(), 1);
    assert_eq!(results.hits[0].message_id, mine.id);
}

/// The boundary #408 settled: the executor does not snippet any more.
///
/// `snippet()` is an FTS5 function over indexed content and the body index
/// has none (#407). Rather than give `postio-index` a blob-store dependency
/// so it could reconstruct one -- it is a rusqlite-only leaf and
/// `check-crate-boundaries.py` keeps it that way -- the excerpt is cut by
/// `postio_search::highlight::snippet` from the body text, by whoever can
/// read it. `postio_app::search` is that caller.
#[test]
fn a_matching_query_leaves_the_snippet_for_a_layer_that_can_read_bodies() {
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
        account: AccountScope::Account(account.id),
        query: &query,
        scope: Scope::AllMail,
        limit: 10,
        order: postio_search::ResultOrder::Relevance,
    };
    let results = search(&connection, &request, at(12)).expect("search");

    assert_eq!(
        results.hits.len(),
        1,
        "a body-only match is still a hit, through the body index"
    );
    assert!(
        results.hits[0].snippet.is_empty(),
        "the executor cannot read a body and must not invent an excerpt: {:?}",
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
        account: AccountScope::Account(account.id),
        query: &query,
        scope: Scope::AllMail,
        limit: 10,
        order: postio_search::ResultOrder::Relevance,
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
            account: AccountScope::Account(account.id),
            query: &query,
            scope,
            limit: 10,
            order: postio_search::ResultOrder::Relevance,
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
        account: AccountScope::Account(account.id),
        query: &query,
        scope: Scope::Inbox,
        limit: 10,
        order: postio_search::ResultOrder::Relevance,
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
        account: AccountScope::Account(account.id),
        query: &query,
        scope: Scope::AllMail,
        limit: 10,
        order: postio_search::ResultOrder::Relevance,
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
        account: AccountScope::Account(account.id),
        query: &query,
        scope: Scope::AllMail,
        limit: 10,
        order: postio_search::ResultOrder::Relevance,
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
        account: AccountScope::Account(account.id),
        query: &refined,
        scope: Scope::AllMail,
        limit: 10,
        order: postio_search::ResultOrder::Relevance,
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
        account: AccountScope::Account(account.id),
        query: &query,
        scope: Scope::AllMail,
        limit: 10,
        order: postio_search::ResultOrder::Relevance,
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
        account: AccountScope::Account(account.id),
        query: &refined,
        scope: Scope::AllMail,
        limit: 10,
        order: postio_search::ResultOrder::Relevance,
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
        account: AccountScope::Account(account.id),
        query: &query,
        scope: Scope::AllMail,
        limit: 10,
        order: postio_search::ResultOrder::Relevance,
    };
    let facets = postio_index::executor::facets(&connection, &request).expect("facets");

    assert_eq!(facets.hits(Scope::AllMail), 0);
    assert!(facets.suggested(0).is_empty());
}

// ---------------------------------------------------------------------------
// Two indexes, one query — #408
// ---------------------------------------------------------------------------

/// A message with a subject and a body, both indexed.
fn with_body(
    connection: &Connection,
    account: &postio_model::Account,
    mailbox: postio_model::MailboxId,
    subject: &str,
    body: &str,
    received_at: chrono::DateTime<Utc>,
) -> Message {
    let created = message(connection, account, mailbox, "ada", subject, received_at);
    postio_index::index::index_body(connection, created.id.get(), Some(body)).expect("index body");
    created
}

fn found(
    connection: &Connection,
    account: postio_model::AccountId,
    text: &str,
) -> Vec<postio_model::MessageId> {
    let query = parse(text, at(12).date_naive());
    let request = SearchRequest {
        account: AccountScope::Account(account),
        query: &query,
        scope: Scope::AllMail,
        limit: 10,
        order: postio_search::ResultOrder::Relevance,
    };
    search(connection, &request, at(12))
        .expect("search")
        .hits
        .into_iter()
        .map(|hit| hit.message_id)
        .collect()
}

#[test]
fn free_text_reaches_the_body_index_and_the_metadata_index() {
    // The union. Before #407 both lived in one `messages_fts`, so one `MATCH`
    // covered them; a message matching in either is still one hit and neither
    // may be lost.
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    postio_index::index::ensure_schema(&connection).expect("schema");
    let (account, mailbox) = test_support::account_with_inbox(&connection);

    let in_subject = with_body(
        &connection,
        &account,
        mailbox,
        "The maildir plan",
        "nothing of note here",
        at(9),
    );
    let in_body = with_body(
        &connection,
        &account,
        mailbox,
        "Nothing of note",
        "the maildir rebuild finished overnight",
        at(8),
    );

    let mut ids = found(&connection, account.id, "maildir");
    ids.sort_by_key(|id| id.get());
    let mut expected = vec![in_subject.id, in_body.id];
    expected.sort_by_key(|id| id.get());

    assert_eq!(ids, expected, "a hit in either index is a hit");
}

#[test]
fn a_subject_match_outranks_a_body_match() {
    // The ranking decision, stated as the behaviour it exists to produce
    // rather than as a number. Somebody searching "invoice" wants the message
    // *about* invoices above the one that mentions them in passing, and the
    // single six-column bm25 used to deliver that through its own length
    // normalisation. Two scores means saying it on purpose.
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    postio_index::index::ensure_schema(&connection).expect("schema");
    let (account, mailbox) = test_support::account_with_inbox(&connection);

    // Same age, so recency cannot be what decides it.
    let mentions_it = with_body(
        &connection,
        &account,
        mailbox,
        "Weekly notes",
        "the invoice was mentioned again",
        at(9),
    );
    let about_it = with_body(
        &connection,
        &account,
        mailbox,
        "Invoice for August",
        "attached, as agreed",
        at(9),
    );

    assert_eq!(
        found(&connection, account.id, "invoice"),
        vec![about_it.id, mentions_it.id]
    );
}

#[test]
fn matching_in_both_indexes_beats_matching_in_either() {
    // Why the two scores are summed rather than the better one taken: a
    // message whose subject *and* body are about the thing is the most
    // relevant thing there is.
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    postio_index::index::ensure_schema(&connection).expect("schema");
    let (account, mailbox) = test_support::account_with_inbox(&connection);

    let subject_only = with_body(
        &connection,
        &account,
        mailbox,
        "Invoice for August",
        "attached, as agreed",
        at(9),
    );
    let both = with_body(
        &connection,
        &account,
        mailbox,
        "Invoice for August",
        "the invoice is attached",
        at(9),
    );

    let ranked = found(&connection, account.id, "invoice");
    assert_eq!(ranked.first(), Some(&both.id), "{ranked:?}");
    assert!(ranked.contains(&subject_only.id));
}

#[test]
fn a_negated_term_excludes_a_body_match_too() {
    // `-spam` has to mean it wherever the word is. While the exclusion asked
    // only `messages_fts`, a message whose only "spam" was in its body came
    // back from a query that had explicitly refused it.
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    postio_index::index::ensure_schema(&connection).expect("schema");
    let (account, mailbox) = test_support::account_with_inbox(&connection);

    let clean = with_body(
        &connection,
        &account,
        mailbox,
        "Quarterly report",
        "the numbers are attached",
        at(9),
    );
    let _spam_in_body = with_body(
        &connection,
        &account,
        mailbox,
        "Quarterly report",
        "this is spam, frankly",
        at(8),
    );

    assert_eq!(
        found(&connection, account.id, "report -spam"),
        vec![clean.id]
    );
}

#[test]
fn only_negated_free_text_still_excludes_a_body_match() {
    // The other exclusion path: with no positive term there is no `MATCH` to
    // fold the negation into, so it is built condition by condition — and
    // that half had to learn about the body index as well.
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    postio_index::index::ensure_schema(&connection).expect("schema");
    let (account, mailbox) = test_support::account_with_inbox(&connection);

    let clean = with_body(
        &connection,
        &account,
        mailbox,
        "Quarterly report",
        "the numbers are attached",
        at(9),
    );
    let _spam_in_body = with_body(
        &connection,
        &account,
        mailbox,
        "Lunch plans",
        "this is spam, frankly",
        at(8),
    );

    assert_eq!(found(&connection, account.id, "-spam"), vec![clean.id]);
}

#[test]
fn a_body_that_was_re_indexed_no_longer_matches_its_old_words() {
    // Through the executor, not only through the table: a contentless index
    // is delete-then-insert, and a stale row would show up here as search
    // returning a message for words it no longer contains.
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    postio_index::index::ensure_schema(&connection).expect("schema");
    let (account, mailbox) = test_support::account_with_inbox(&connection);

    let changed = with_body(
        &connection,
        &account,
        mailbox,
        "Draft",
        "the first draft",
        at(9),
    );
    postio_index::index::index_body(&connection, changed.id.get(), Some("the second draft"))
        .expect("re-index");

    assert_eq!(found(&connection, account.id, "second"), vec![changed.id]);
    assert!(found(&connection, account.id, "first").is_empty());
}

#[test]
fn newest_order_answers_in_date_order_however_the_ranking_disagrees() {
    // #499: the list column says `Newest ▾` and has to be able to mean it.
    // Relevance is the default and stays ranked; asking for `Newest` must
    // come back in plain date order even when bm25 would put an older,
    // denser match first.
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    postio_index::index::ensure_schema(&connection).expect("schema");
    let (account, mailbox) = test_support::account_with_inbox(&connection);

    // Enough non-matching mail that "report" is worth something: bm25's
    // IDF term goes to zero when every document in the corpus matches, and
    // a two-message corpus would leave the ranking to recency alone.
    for i in 0..20 {
        message(
            &connection,
            &account,
            mailbox,
            "carol",
            &format!("Entirely unrelated subject {i}"),
            at(7),
        );
    }

    // Older, but saturated with the term: the far better bm25 match.
    let dense = message(
        &connection,
        &account,
        mailbox,
        "ada",
        "report report report report report",
        at(6),
    );
    // Newer, and a glancing match.
    let recent = message(
        &connection,
        &account,
        mailbox,
        "bob",
        "One report among other things entirely",
        at(11),
    );

    let query = parse("report", at(12).date_naive());
    let ranked = search(
        &connection,
        &SearchRequest {
            account: AccountScope::Account(account.id),
            query: &query,
            scope: Scope::AllMail,
            limit: 10,
            order: postio_search::ResultOrder::Relevance,
        },
        at(12),
    )
    .expect("search");
    assert_eq!(
        ranked.hits[0].message_id,
        dense.id,
        "relevance still ranks: the dense match outweighs five hours of recency \
         (scores: {:?})",
        ranked
            .hits
            .iter()
            .map(|hit| (hit.message_id, hit.score))
            .collect::<Vec<_>>()
    );

    let newest = search(
        &connection,
        &SearchRequest {
            account: AccountScope::Account(account.id),
            query: &query,
            scope: Scope::AllMail,
            limit: 10,
            order: postio_search::ResultOrder::Newest,
        },
        at(12),
    )
    .expect("search");
    assert_eq!(
        newest
            .hits
            .iter()
            .map(|hit| hit.message_id)
            .collect::<Vec<_>>(),
        vec![recent.id, dense.id],
        "Newest means date order, exactly as a mailbox is ordered"
    );
}

// ---------------------------------------------------------------------------
// Saying so when the corpus is still filling (#352)
// ---------------------------------------------------------------------------
//
// Headers sync long before bodies, so between a first sync and the end of
// backfill a free-text query answers from a subset of the mailbox and says
// nothing about it. The result count reads as "this is what your mailbox
// contains" either way, which is the quiet kind of wrong.
//
// Transient, under ADR 0016: every folder backfills to completion by default,
// so this is a state that ends — which is why it is a boolean rather than the
// count #352 originally asked for, and why the surface says "still syncing".

/// Puts `message` in the state a downloaded body leaves it in.
///
/// `Message::new` starts at `not_fetched`, and indexing text does not move it
/// — the backfill lane sets it when it stores the blob. So a test that wants
/// "this body is here" has to say so, the same way sync does.
fn body_here(connection: &Connection, message: &Message) {
    connection
        .execute(
            "UPDATE messages SET body_state = 'full' WHERE id = ?1",
            [message.id.get()],
        )
        .expect("mark the body present");
}

/// Puts `message` in the state a body that has not arrived yet is in.
fn body_not_here(connection: &Connection, message: &Message) {
    connection
        .execute(
            "UPDATE messages SET body_state = 'headers_only' WHERE id = ?1",
            [message.id.get()],
        )
        .expect("mark the body missing");
}

fn search_for(
    connection: &Connection,
    account: &postio_model::Account,
    query: &str,
) -> postio_search::SearchResults {
    let parsed = parse(query, at(12).date_naive());
    let request = SearchRequest {
        account: postio_model::AccountScope::Account(account.id),
        query: &parsed,
        scope: Scope::AllMail,
        order: postio_search::ResultOrder::Relevance,
        limit: 10,
    };
    search(connection, &request, at(12)).expect("search")
}

#[test]
fn a_search_over_a_corpus_still_filling_reports_it() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    postio_index::index::ensure_schema(&connection).expect("schema");
    let (account, mailbox) = test_support::account_with_inbox(&connection);

    let here = message(&connection, &account, mailbox, "ada", "Quarterly", at(9));
    postio_index::index::index_body(&connection, here.id.get(), Some("the figures"))
        .expect("index");
    body_here(&connection, &here);

    assert!(
        search_for(&connection, &account, "quarterly").corpus_complete,
        "every message here has a body, so there is nothing to caveat"
    );

    // A second message whose body has not been fetched. It cannot match on
    // anything it says, and the count is a floor because of it.
    let absent = message(&connection, &account, mailbox, "bob", "Quarterly", at(8));
    body_not_here(&connection, &absent);

    let results = search_for(&connection, &account, "quarterly");
    assert!(
        !results.corpus_complete,
        "a message whose body has not arrived is unsearchable and unmentioned, \
         so the hit count reads as the whole mailbox when it is not (#352)"
    );

    // And the caveat is about the corpus, not about this query: a search that
    // matches nothing is just as incomplete as one that matches everything.
    assert!(!search_for(&connection, &account, "nothingmatchesthis").corpus_complete);
}

#[test]
fn the_caveat_goes_away_when_the_bodies_arrive() {
    // The acceptance criterion that keeps this from becoming permanent
    // furniture. Under ADR 0016 every account ends up here.
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    postio_index::index::ensure_schema(&connection).expect("schema");
    let (account, mailbox) = test_support::account_with_inbox(&connection);

    let message = message(&connection, &account, mailbox, "ada", "Quarterly", at(9));
    body_not_here(&connection, &message);
    assert!(!search_for(&connection, &account, "quarterly").corpus_complete);

    connection
        .execute(
            "UPDATE messages SET body_state = 'full' WHERE id = ?1",
            [message.id.get()],
        )
        .expect("the body arrives");

    assert!(
        search_for(&connection, &account, "quarterly").corpus_complete,
        "a fully backfilled account must not carry a permanent caveat"
    );
}

#[test]
fn the_caveat_is_about_the_scope_that_was_searched() {
    // "Complete" has to mean complete *here*. An inbox whose bodies are all
    // local must not inherit a caveat earned by an archive nobody searched --
    // the claim on screen is about the search that was just run.
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    postio_index::index::ensure_schema(&connection).expect("schema");
    let (account, inbox) = test_support::account_with_inbox(&connection);
    let archive = test_support::mailbox(&connection, &account, "Archive").id;

    let read = message(&connection, &account, inbox, "ada", "Quarterly", at(9));
    postio_index::index::index_body(&connection, read.id.get(), Some("the figures"))
        .expect("index");
    body_here(&connection, &read);
    let unread = message(&connection, &account, archive, "bob", "Quarterly", at(8));
    body_not_here(&connection, &unread);

    let parsed = parse("quarterly", at(12).date_naive());
    let inbox_only = SearchRequest {
        account: postio_model::AccountScope::Account(account.id),
        query: &parsed,
        scope: Scope::Inbox,
        order: postio_search::ResultOrder::Relevance,
        limit: 10,
    };
    assert!(
        search(&connection, &inbox_only, at(12))
            .expect("search")
            .corpus_complete,
        "the inbox is complete; the archive's outstanding body is not this \
         search's business"
    );

    let everything = SearchRequest {
        scope: Scope::AllMail,
        ..inbox_only
    };
    assert!(
        !search(&connection, &everything, at(12))
            .expect("search")
            .corpus_complete,
        "and widening the scope to include it brings the caveat back"
    );
}
