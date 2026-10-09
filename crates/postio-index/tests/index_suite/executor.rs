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
use postio_storage::Connection;
use postio_storage::repository::MessageRepository;
use postio_storage::sql::bind;
use postio_storage::test_support;

pub(super) fn at(hour: u32) -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 8, 20, hour, 0, 0).unwrap()
}

pub(super) async fn message(
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
        .await
        .expect("create message");
    message
}

#[tokio::test]
async fn a_composed_operator_and_free_text_query_narrows_correctly() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    postio_index::index::ensure_schema(&connection)
        .await
        .expect("schema");
    let (account, mailbox) = test_support::account_with_inbox(&connection).await;

    let matching = message(
        &connection,
        &account,
        mailbox,
        "ada",
        "Quarterly report",
        at(9),
    )
    .await;
    let _wrong_sender = message(
        &connection,
        &account,
        mailbox,
        "bob",
        "Quarterly report",
        at(8),
    )
    .await;
    let _wrong_text = message(&connection, &account, mailbox, "ada", "Lunch plans", at(10)).await;

    let query = parse("report from:ada", at(12).date_naive());
    let request = SearchRequest {
        account: AccountScope::Account(account.id),
        query: &query,
        scope: Scope::AllMail,
        limit: 10,
        order: postio_search::ResultOrder::Relevance,
    };
    let results = search(&connection, &request, at(12)).await.expect("search");

    assert_eq!(results.hits.len(), 1);
    assert_eq!(results.hits[0].message_id, matching.id);
    assert_eq!(results.total_hits, 1);
}

#[tokio::test]
async fn list_names_a_mailing_list_by_its_list_id_not_by_a_recipient_address() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    postio_index::index::ensure_schema(&connection)
        .await
        .expect("schema");
    let (account, mailbox) = test_support::account_with_inbox(&connection).await;

    let mut on_list = message(
        &connection,
        &account,
        mailbox,
        "ada",
        "Tuesday walkthrough",
        at(9),
    )
    .await;
    on_list.list_id = Some("harbour-dev.lists.example.org".to_string());
    MessageRepository::new(&connection)
        .update(&mut on_list)
        .await
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
    )
    .await;
    off_list.to = vec![EmailAddress::new(
        None::<String>,
        "harbour-dev@lists.example.org",
    )];
    MessageRepository::new(&connection)
        .update(&mut off_list)
        .await
        .expect("update message");

    let query = parse("list:harbour-dev", at(12).date_naive());
    let request = SearchRequest {
        account: AccountScope::Account(account.id),
        query: &query,
        scope: Scope::AllMail,
        limit: 10,
        order: postio_search::ResultOrder::Relevance,
    };
    let results = search(&connection, &request, at(12)).await.expect("search");

    assert_eq!(results.hits.len(), 1);
    assert_eq!(results.hits[0].message_id, on_list.id);
}

#[tokio::test]
async fn negated_only_free_text_excludes_without_a_positive_match_expression() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    postio_index::index::ensure_schema(&connection)
        .await
        .expect("schema");
    let (account, mailbox) = test_support::account_with_inbox(&connection).await;

    let clean = message(&connection, &account, mailbox, "ada", "Weekly sync", at(9)).await;
    let spammy = message(&connection, &account, mailbox, "ada", "Weekly sync", at(10)).await;
    postio_index::index::index_body(&connection, spammy.id.get(), Some("buy cheap watches now"))
        .await
        .expect("index body");
    postio_index::index::index_body(&connection, clean.id.get(), Some("see you at standup"))
        .await
        .expect("index body");

    let query = parse("-watches", at(12).date_naive());
    let request = SearchRequest {
        account: AccountScope::Account(account.id),
        query: &query,
        scope: Scope::AllMail,
        limit: 10,
        order: postio_search::ResultOrder::Relevance,
    };
    let results = search(&connection, &request, at(12)).await.expect("search");

    assert_eq!(results.hits.len(), 1);
    assert_eq!(results.hits[0].message_id, clean.id);
}

#[tokio::test]
async fn total_hits_counts_every_match_regardless_of_the_page_limit() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    postio_index::index::ensure_schema(&connection)
        .await
        .expect("schema");
    let (account, mailbox) = test_support::account_with_inbox(&connection).await;

    for hour in 0..5 {
        message(
            &connection,
            &account,
            mailbox,
            "ada",
            "Standup notes",
            at(hour),
        )
        .await;
    }

    let query = parse("standup", at(12).date_naive());
    let request = SearchRequest {
        account: AccountScope::Account(account.id),
        query: &query,
        scope: Scope::AllMail,
        limit: 2,
        order: postio_search::ResultOrder::Relevance,
    };
    let results = search(&connection, &request, at(12)).await.expect("search");

    assert_eq!(results.hits.len(), 2, "limited to the page size");
    assert_eq!(results.total_hits, 5, "but the total reflects every match");
    assert!(!results.total_hits_capped);
}

#[tokio::test]
async fn a_structured_only_query_orders_newest_first_and_carries_no_snippet() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    postio_index::index::ensure_schema(&connection)
        .await
        .expect("schema");
    let (account, mailbox) = test_support::account_with_inbox(&connection).await;

    let older = message(&connection, &account, mailbox, "ada", "One", at(8)).await;
    let newer = message(&connection, &account, mailbox, "ada", "Two", at(10)).await;

    let query = parse("from:ada", at(12).date_naive());
    let request = SearchRequest {
        account: AccountScope::Account(account.id),
        query: &query,
        scope: Scope::AllMail,
        limit: 10,
        order: postio_search::ResultOrder::Relevance,
    };
    let results = search(&connection, &request, at(12)).await.expect("search");

    assert_eq!(results.hits.len(), 2);
    assert_eq!(results.hits[0].message_id, newer.id);
    assert_eq!(results.hits[1].message_id, older.id);
    assert!(results.hits.iter().all(|hit| hit.snippet.is_empty()));
}

/// A result row can show the message's first line without reading a body:
/// the hit carries the preview the list shows.
#[tokio::test]
async fn a_hit_carries_the_messages_first_line() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    postio_index::index::ensure_schema(&connection)
        .await
        .expect("schema");
    let (account, mailbox) = test_support::account_with_inbox(&connection).await;

    let mut with = Message::new(account.id, mailbox, at(8));
    with.from = vec![EmailAddress::new(Some("ada"), "ada@example.com")];
    with.subject = Some("Harbour".to_owned());
    with.preview = Some("Three berths free this weekend.".to_owned());
    MessageRepository::new(&connection)
        .create(&mut with)
        .await
        .expect("create message");
    message(
        &connection,
        &account,
        mailbox,
        "ada",
        "Harbour again",
        at(9),
    )
    .await;

    let found = search_for(&connection, &account, "harbour").await;
    let preview_of = |id| {
        found
            .hits
            .iter()
            .find(|hit| hit.message_id == id)
            .expect("hit")
            .preview
            .clone()
    };
    assert_eq!(
        preview_of(with.id).as_deref(),
        Some("Three berths free this weekend.")
    );
    assert_eq!(found.hits.len(), 2);
}

#[tokio::test]
async fn search_never_crosses_accounts() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    postio_index::index::ensure_schema(&connection)
        .await
        .expect("schema");
    let (account_a, mailbox_a) = test_support::account_with_inbox(&connection).await;
    let account_b = test_support::account(&connection).await;
    let mailbox_b = test_support::mailbox(&connection, &account_b, "INBOX")
        .await
        .id;

    let mine = message(
        &connection,
        &account_a,
        mailbox_a,
        "ada",
        "Shared subject",
        at(9),
    )
    .await;
    let _theirs = message(
        &connection,
        &account_b,
        mailbox_b,
        "ada",
        "Shared subject",
        at(9),
    )
    .await;

    let query = parse("shared", at(12).date_naive());
    let request = SearchRequest {
        account: AccountScope::Account(account_a.id),
        query: &query,
        scope: Scope::AllMail,
        limit: 10,
        order: postio_search::ResultOrder::Relevance,
    };
    let results = search(&connection, &request, at(12)).await.expect("search");

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
/// read it. The session layer's search is that caller.
#[tokio::test]
async fn a_matching_query_leaves_the_snippet_for_a_layer_that_can_read_bodies() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    postio_index::index::ensure_schema(&connection)
        .await
        .expect("schema");
    let (account, mailbox) = test_support::account_with_inbox(&connection).await;

    let found = message(
        &connection,
        &account,
        mailbox,
        "ada",
        "Rebuild status",
        at(9),
    )
    .await;
    postio_index::index::index_body(
        &connection,
        found.id.get(),
        Some("the maildir rebuild finished overnight"),
    )
    .await
    .expect("index body");

    let query = parse("rebuild", at(12).date_naive());
    let request = SearchRequest {
        account: AccountScope::Account(account.id),
        query: &query,
        scope: Scope::AllMail,
        limit: 10,
        order: postio_search::ResultOrder::Relevance,
    };
    let results = search(&connection, &request, at(12)).await.expect("search");

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

#[tokio::test]
async fn filename_and_has_attachment_operators_filter_correctly() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    postio_index::index::ensure_schema(&connection)
        .await
        .expect("schema");
    let (account, mailbox) = test_support::account_with_inbox(&connection).await;

    let mut with_attachment = Message::new(account.id, mailbox, at(9));
    with_attachment.subject = Some("Numbers".to_string());
    let mut attachment = Attachment::new(with_attachment.id, "text/plain", 10);
    attachment.filename = Some("timings.txt".to_string());
    with_attachment.attachments = vec![attachment];
    MessageRepository::new(&connection)
        .create(&mut with_attachment)
        .await
        .expect("create");

    let mut without_attachment = Message::new(account.id, mailbox, at(10));
    without_attachment.subject = Some("Numbers".to_string());
    MessageRepository::new(&connection)
        .create(&mut without_attachment)
        .await
        .expect("create");

    let query = parse("has:attach filename:timings", at(12).date_naive());
    let request = SearchRequest {
        account: AccountScope::Account(account.id),
        query: &query,
        scope: Scope::AllMail,
        limit: 10,
        order: postio_search::ResultOrder::Relevance,
    };
    let results = search(&connection, &request, at(12)).await.expect("search");

    assert_eq!(results.hits.len(), 1);
    assert_eq!(results.hits[0].message_id, with_attachment.id);
}

// ---------------------------------------------------------------------------
// Scope and facets — canvas 2b's left column
// ---------------------------------------------------------------------------

/// An account with an inbox and a folder list mail is filed into, plus the
/// same subject in both so a query spans the scopes.
async fn split_across_scopes(connection: &Connection) -> (postio_model::Account, u64) {
    let (account, inbox) = test_support::account_with_inbox(connection).await;
    let folder = test_support::mailbox(connection, &account, "lkml").await;

    message(
        connection,
        &account,
        inbox,
        "ada",
        "Quarterly report",
        at(9),
    )
    .await;
    message(
        connection,
        &account,
        inbox,
        "bob",
        "Quarterly figures",
        at(10),
    )
    .await;
    message(
        connection,
        &account,
        folder.id,
        "lkml",
        "Quarterly patch queue",
        at(11),
    )
    .await;
    (account, 3)
}

#[tokio::test]
async fn a_scope_narrows_the_search_without_touching_the_query() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    postio_index::index::ensure_schema(&connection)
        .await
        .expect("schema");
    let (account, total) = split_across_scopes(&connection).await;

    let query = parse("quarterly", at(12).date_naive());
    let counts = async |scope| {
        let request = SearchRequest {
            account: AccountScope::Account(account.id),
            query: &query,
            scope,
            limit: 10,
            order: postio_search::ResultOrder::Relevance,
        };
        search(&connection, &request, at(12))
            .await
            .expect("search")
            .total_hits
    };

    assert_eq!(counts(Scope::AllMail).await, total);
    assert_eq!(counts(Scope::Inbox).await, 2);
    assert_eq!(counts(Scope::Lists).await, 1);
    assert_eq!(
        query.input(),
        "quarterly",
        "the scope is not a token; the box still says what was typed"
    );
}

#[tokio::test]
async fn the_scope_column_counts_what_switching_would_find() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    postio_index::index::ensure_schema(&connection)
        .await
        .expect("schema");
    let (account, total) = split_across_scopes(&connection).await;

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
    let facets = postio_index::executor::facets(&connection, &request)
        .await
        .expect("facets");

    assert_eq!(facets.hits(Scope::AllMail), total);
    assert_eq!(facets.hits(Scope::Inbox), 2);
    assert_eq!(facets.hits(Scope::Lists), 1);
}

#[tokio::test]
async fn refinements_are_measured_against_the_result_set_and_not_the_mailbox() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    postio_index::index::ensure_schema(&connection)
        .await
        .expect("schema");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;

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
            .await
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
    let facets = postio_index::executor::facets(&connection, &request)
        .await
        .expect("facets");

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

#[tokio::test]
async fn a_folder_the_matches_are_in_is_offered_as_a_token_that_parses_back() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    postio_index::index::ensure_schema(&connection)
        .await
        .expect("schema");
    let (account, _) = split_across_scopes(&connection).await;

    let query = parse("quarterly", at(12).date_naive());
    let request = SearchRequest {
        account: AccountScope::Account(account.id),
        query: &query,
        scope: Scope::AllMail,
        limit: 10,
        order: postio_search::ResultOrder::Relevance,
    };
    let facets = postio_index::executor::facets(&connection, &request)
        .await
        .expect("facets");

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
    let narrowed = search(&connection, &request, at(12)).await.expect("search");
    assert_eq!(narrowed.total_hits, folder.hits);
}

#[tokio::test]
async fn the_size_refinement_is_spelled_the_way_the_parser_reads_it() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    postio_index::index::ensure_schema(&connection)
        .await
        .expect("schema");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;

    let mut big = Message::new(account.id, inbox, at(9));
    big.subject = Some("Quarterly report".to_string());
    big.size = 4 * 1024 * 1024;
    let mut small = Message::new(account.id, inbox, at(10));
    small.subject = Some("Quarterly note".to_string());
    small.size = 900;
    for message in [&mut big, &mut small] {
        MessageRepository::new(&connection)
            .create(message)
            .await
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
    let facets = postio_index::executor::facets(&connection, &request)
        .await
        .expect("facets");

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
    let narrowed = search(&connection, &request, at(12)).await.expect("search");
    assert_eq!(
        narrowed.hits.len(),
        1,
        "the chip's own token has to reproduce the count it advertised"
    );
    assert_eq!(narrowed.hits[0].message_id, big.id);
}

#[tokio::test]
async fn a_query_that_matches_nothing_offers_nothing_rather_than_dead_ends() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    postio_index::index::ensure_schema(&connection)
        .await
        .expect("schema");
    let (account, _) = split_across_scopes(&connection).await;

    let query = parse("nothingmatchesthis", at(12).date_naive());
    let request = SearchRequest {
        account: AccountScope::Account(account.id),
        query: &query,
        scope: Scope::AllMail,
        limit: 10,
        order: postio_search::ResultOrder::Relevance,
    };
    let facets = postio_index::executor::facets(&connection, &request)
        .await
        .expect("facets");

    assert_eq!(facets.hits(Scope::AllMail), 0);
    assert!(facets.suggested(0).is_empty());
}

// ---------------------------------------------------------------------------
// Two indexes, one query — #408
// ---------------------------------------------------------------------------

/// A message with a subject and a body, both indexed.
async fn with_body(
    connection: &Connection,
    account: &postio_model::Account,
    mailbox: postio_model::MailboxId,
    subject: &str,
    body: &str,
    received_at: chrono::DateTime<Utc>,
) -> Message {
    let created = message(connection, account, mailbox, "ada", subject, received_at).await;
    postio_index::index::index_body(connection, created.id.get(), Some(body))
        .await
        .expect("index body");
    created
}

async fn found(
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
        .await
        .expect("search")
        .hits
        .into_iter()
        .map(|hit| hit.message_id)
        .collect()
}

#[tokio::test]
async fn free_text_reaches_the_body_index_and_the_metadata_index() {
    // The union. Before #407 both lived in one `messages_fts`, so one `MATCH`
    // covered them; a message matching in either is still one hit and neither
    // may be lost.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    postio_index::index::ensure_schema(&connection)
        .await
        .expect("schema");
    let (account, mailbox) = test_support::account_with_inbox(&connection).await;

    let in_subject = with_body(
        &connection,
        &account,
        mailbox,
        "The maildir plan",
        "nothing of note here",
        at(9),
    )
    .await;
    let in_body = with_body(
        &connection,
        &account,
        mailbox,
        "Nothing of note",
        "the maildir rebuild finished overnight",
        at(8),
    )
    .await;

    let mut ids = found(&connection, account.id, "maildir").await;
    ids.sort_by_key(|id| id.get());
    let mut expected = vec![in_subject.id, in_body.id];
    expected.sort_by_key(|id| id.get());

    assert_eq!(ids, expected, "a hit in either index is a hit");
}

#[tokio::test]
async fn two_words_need_both_in_the_body_too() {
    // The body half of the match is folded to meet the folded body text,
    // and folding the whole expression lowercased its `AND` too: to the
    // index a lowercase `and` is a word, not an operator, so two words
    // asked the bodies for either word *or "and"*. On a real mailbox that
    // was most of it -- 10,000 hits for a phrase that matched 87, and two
    // hundred seconds to count them, which stopped the search box. One
    // body has both words; the other has only "and".
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    postio_index::index::ensure_schema(&connection)
        .await
        .expect("schema");
    let (account, mailbox) = test_support::account_with_inbox(&connection).await;

    let both = with_body(
        &connection,
        &account,
        mailbox,
        "Thursday",
        "the meeting agenda is attached",
        at(9),
    )
    .await;
    with_body(
        &connection,
        &account,
        mailbox,
        "Groceries",
        "bread and milk",
        at(8),
    )
    .await;

    assert_eq!(
        found(&connection, account.id, "meeting agenda").await,
        vec![both.id],
        "two words matched a body holding neither, through its \"and\""
    );
    assert_eq!(
        found(&connection, account.id, "Meeting Agenda").await,
        vec![both.id],
        "and the fold still meets the body however the words are cased"
    );
}

#[tokio::test]
async fn a_subject_match_outranks_a_body_match() {
    // The ranking decision, stated as the behaviour it exists to produce
    // rather than as a number. Somebody searching "invoice" wants the message
    // *about* invoices above the one that mentions them in passing, and the
    // single six-column bm25 used to deliver that through its own length
    // normalisation. Two scores means saying it on purpose.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    postio_index::index::ensure_schema(&connection)
        .await
        .expect("schema");
    let (account, mailbox) = test_support::account_with_inbox(&connection).await;

    // Same age, so recency cannot be what decides it.
    let mentions_it = with_body(
        &connection,
        &account,
        mailbox,
        "Weekly notes",
        "the invoice was mentioned again",
        at(9),
    )
    .await;
    let about_it = with_body(
        &connection,
        &account,
        mailbox,
        "Invoice for August",
        "attached, as agreed",
        at(9),
    )
    .await;

    assert_eq!(
        found(&connection, account.id, "invoice").await,
        vec![about_it.id, mentions_it.id]
    );
}

#[tokio::test]
async fn matching_in_both_indexes_beats_matching_in_either() {
    // Why the two scores are summed rather than the better one taken: a
    // message whose subject *and* body are about the thing is the most
    // relevant thing there is.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    postio_index::index::ensure_schema(&connection)
        .await
        .expect("schema");
    let (account, mailbox) = test_support::account_with_inbox(&connection).await;

    let subject_only = with_body(
        &connection,
        &account,
        mailbox,
        "Invoice for August",
        "attached, as agreed",
        at(9),
    )
    .await;
    let both = with_body(
        &connection,
        &account,
        mailbox,
        "Invoice for August",
        "the invoice is attached",
        at(9),
    )
    .await;

    let ranked = found(&connection, account.id, "invoice").await;
    assert_eq!(ranked.first(), Some(&both.id), "{ranked:?}");
    assert!(ranked.contains(&subject_only.id));
}

#[tokio::test]
async fn a_negated_term_excludes_a_body_match_too() {
    // `-spam` has to mean it wherever the word is. While the exclusion asked
    // only `messages_fts`, a message whose only "spam" was in its body came
    // back from a query that had explicitly refused it.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    postio_index::index::ensure_schema(&connection)
        .await
        .expect("schema");
    let (account, mailbox) = test_support::account_with_inbox(&connection).await;

    let clean = with_body(
        &connection,
        &account,
        mailbox,
        "Quarterly report",
        "the numbers are attached",
        at(9),
    )
    .await;
    let _spam_in_body = with_body(
        &connection,
        &account,
        mailbox,
        "Quarterly report",
        "this is spam, frankly",
        at(8),
    )
    .await;

    assert_eq!(
        found(&connection, account.id, "report -spam").await,
        vec![clean.id]
    );
}

#[tokio::test]
async fn only_negated_free_text_still_excludes_a_body_match() {
    // The other exclusion path: with no positive term there is no `MATCH` to
    // fold the negation into, so it is built condition by condition — and
    // that half had to learn about the body index as well.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    postio_index::index::ensure_schema(&connection)
        .await
        .expect("schema");
    let (account, mailbox) = test_support::account_with_inbox(&connection).await;

    let clean = with_body(
        &connection,
        &account,
        mailbox,
        "Quarterly report",
        "the numbers are attached",
        at(9),
    )
    .await;
    let _spam_in_body = with_body(
        &connection,
        &account,
        mailbox,
        "Lunch plans",
        "this is spam, frankly",
        at(8),
    )
    .await;

    assert_eq!(
        found(&connection, account.id, "-spam").await,
        vec![clean.id]
    );
}

#[tokio::test]
async fn a_body_that_was_re_indexed_no_longer_matches_its_old_words() {
    // Through the executor, not only through the table: a contentless index
    // is delete-then-insert, and a stale row would show up here as search
    // returning a message for words it no longer contains.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    postio_index::index::ensure_schema(&connection)
        .await
        .expect("schema");
    let (account, mailbox) = test_support::account_with_inbox(&connection).await;

    let changed = with_body(
        &connection,
        &account,
        mailbox,
        "Draft",
        "the first draft",
        at(9),
    )
    .await;
    postio_index::index::index_body(&connection, changed.id.get(), Some("the second draft"))
        .await
        .expect("re-index");

    assert_eq!(
        found(&connection, account.id, "second").await,
        vec![changed.id]
    );
    assert!(found(&connection, account.id, "first").await.is_empty());
}

#[tokio::test]
async fn newest_order_answers_in_date_order_however_the_ranking_disagrees() {
    // #499: the list column says `Newest ▾` and has to be able to mean it.
    // Relevance is the default and stays ranked; asking for `Newest` must
    // come back in plain date order even when the ranking would put an
    // older message first -- here, the one whose subject says the word.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    postio_index::index::ensure_schema(&connection)
        .await
        .expect("schema");
    let (account, mailbox) = test_support::account_with_inbox(&connection).await;

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
        )
        .await;
    }

    // Older, and about the thing: its subject says it.
    let dense = with_body(
        &connection,
        &account,
        mailbox,
        "Quarterly report",
        "The figures are attached.",
        at(6),
    )
    .await;
    // Newer, and a glancing match in passing.
    let recent = with_body(
        &connection,
        &account,
        mailbox,
        "Notes",
        "One report among other things entirely",
        at(11),
    )
    .await;

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
    .await
    .expect("search");
    assert_eq!(
        ranked.hits[0].message_id,
        dense.id,
        "relevance still ranks: the message about it outweighs five hours of \
         recency (scores: {:?})",
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
    .await
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
async fn body_here(connection: &Connection, message: &Message) {
    connection
        .execute(
            "UPDATE messages SET body_state = 'full' WHERE id = ?1",
            [message.id.get()],
        )
        .await
        .expect("mark the body present");
}

/// Puts `message` in the state a body that has not arrived yet is in.
async fn body_not_here(connection: &Connection, message: &Message) {
    connection
        .execute(
            "UPDATE messages SET body_state = 'headers_only' WHERE id = ?1",
            [message.id.get()],
        )
        .await
        .expect("mark the body missing");
}

async fn search_for(
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
    search(connection, &request, at(12)).await.expect("search")
}

#[tokio::test]
async fn a_search_over_a_corpus_still_filling_reports_it() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    postio_index::index::ensure_schema(&connection)
        .await
        .expect("schema");
    let (account, mailbox) = test_support::account_with_inbox(&connection).await;

    let here = message(&connection, &account, mailbox, "ada", "Quarterly", at(9)).await;
    postio_index::index::index_body(&connection, here.id.get(), Some("the figures"))
        .await
        .expect("index");
    body_here(&connection, &here).await;

    assert!(
        search_for(&connection, &account, "quarterly")
            .await
            .corpus_complete,
        "every message here has a body, so there is nothing to caveat"
    );

    // A second message whose body has not been fetched. It cannot match on
    // anything it says, and the count is a floor because of it.
    let absent = message(&connection, &account, mailbox, "bob", "Quarterly", at(8)).await;
    body_not_here(&connection, &absent).await;

    let results = search_for(&connection, &account, "quarterly").await;
    assert!(
        !results.corpus_complete,
        "a message whose body has not arrived is unsearchable and unmentioned, \
         so the hit count reads as the whole mailbox when it is not (#352)"
    );

    // And the caveat is about the corpus, not about this query: a search that
    // matches nothing is just as incomplete as one that matches everything.
    assert!(
        !search_for(&connection, &account, "nothingmatchesthis")
            .await
            .corpus_complete
    );
}

#[tokio::test]
async fn the_caveat_goes_away_when_the_bodies_arrive() {
    // The acceptance criterion that keeps this from becoming permanent
    // furniture. Under ADR 0016 every account ends up here.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    postio_index::index::ensure_schema(&connection)
        .await
        .expect("schema");
    let (account, mailbox) = test_support::account_with_inbox(&connection).await;

    let message = message(&connection, &account, mailbox, "ada", "Quarterly", at(9)).await;
    body_not_here(&connection, &message).await;
    assert!(
        !search_for(&connection, &account, "quarterly")
            .await
            .corpus_complete
    );

    connection
        .execute(
            "UPDATE messages SET body_state = 'full' WHERE id = ?1",
            [message.id.get()],
        )
        .await
        .expect("the body arrives");

    assert!(
        search_for(&connection, &account, "quarterly")
            .await
            .corpus_complete,
        "a fully backfilled account must not carry a permanent caveat"
    );
}

#[tokio::test]
async fn the_caveat_is_about_the_scope_that_was_searched() {
    // "Complete" has to mean complete *here*. An inbox whose bodies are all
    // local must not inherit a caveat earned by an archive nobody searched --
    // the claim on screen is about the search that was just run.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    postio_index::index::ensure_schema(&connection)
        .await
        .expect("schema");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let archive = test_support::mailbox(&connection, &account, "Archive")
        .await
        .id;

    let read = message(&connection, &account, inbox, "ada", "Quarterly", at(9)).await;
    postio_index::index::index_body(&connection, read.id.get(), Some("the figures"))
        .await
        .expect("index");
    body_here(&connection, &read).await;
    let unread = message(&connection, &account, archive, "bob", "Quarterly", at(8)).await;
    body_not_here(&connection, &unread).await;

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
            .await
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
            .await
            .expect("search")
            .corpus_complete,
        "and widening the scope to include it brings the caveat back"
    );
}

/// #746: `sender_times_seen` reaches the ranker from the `contacts` table.
///
/// The hydrate statement was rewritten to probe contacts by a hoisted
/// column (see `hydrate_probes_contacts_by_address_key` for the plan); this
/// pins the *semantics* of that rewrite: the value still correlates to the
/// right message's sender. Bob's message is newer, so recency alone would
/// rank it first — only ada's contact affinity flowing through can flip the
/// order this asserts.
#[tokio::test]
async fn sender_affinity_from_contacts_reaches_the_ranking() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    postio_index::index::ensure_schema(&connection)
        .await
        .expect("schema");
    let (account, mailbox) = test_support::account_with_inbox(&connection).await;

    let from_regular = message(
        &connection,
        &account,
        mailbox,
        "ada",
        "Quarterly report",
        at(9),
    )
    .await;
    let _from_stranger = message(
        &connection,
        &account,
        mailbox,
        "bob",
        "Quarterly report",
        at(10),
    )
    .await;
    connection
        .execute(
            "INSERT INTO contacts (account_id, address, address_normalized, times_seen)
             VALUES (?1, ?2, ?2, 80)",
            bind![account.id.get(), "ada@example.com"],
        )
        .await
        .expect("seed ada's contact");

    let query = parse("report", at(12).date_naive());
    let request = SearchRequest {
        account: AccountScope::Account(account.id),
        query: &query,
        scope: Scope::AllMail,
        limit: 10,
        order: postio_search::ResultOrder::Relevance,
    };
    let results = search(&connection, &request, at(12)).await.expect("search");

    assert_eq!(results.hits.len(), 2);
    assert_eq!(
        results.hits[0].message_id, from_regular.id,
        "ada is the frequent sender; without affinity, bob's newer message wins"
    );
}

// ---------------------------------------------------------------------------
// `header:` — ADR 0025's operator, and the only one that is not an FTS MATCH
// ---------------------------------------------------------------------------

/// A message carrying `headers`, indexed the way the sync path indexes one.
async fn with_headers(
    connection: &Connection,
    account: &postio_model::Account,
    mailbox: postio_model::MailboxId,
    subject: &str,
    headers: &[(&str, &str)],
) -> Message {
    let created = message(connection, account, mailbox, "ada", subject, at(9)).await;
    let headers: postio_model::Headers = headers.iter().copied().collect();
    postio_index::index::index_headers(connection, created.id.get(), &headers)
        .await
        .expect("index headers");
    created
}

#[tokio::test]
async fn header_finds_a_message_by_a_field_it_carries() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    postio_index::index::ensure_schema(&connection)
        .await
        .expect("schema");
    let (account, mailbox) = test_support::account_with_inbox(&connection).await;

    let mutt = with_headers(
        &connection,
        &account,
        mailbox,
        "Sent with mutt",
        &[("X-Mailer", "Mutt 1.5.24 (2015-08-30)")],
    )
    .await;
    let _plain = with_headers(&connection, &account, mailbox, "Sent plainly", &[]).await;

    assert_eq!(
        found(&connection, account.id, "header:x-mailer").await,
        vec![mutt.id],
        "presence, whatever the value says"
    );
    // The wire case is not the stored case and neither is what a person
    // types; RFC 5322 field names are case-insensitive and every spelling
    // has to reach the one row.
    assert_eq!(
        found(&connection, account.id, "header:X-MAILER").await,
        vec![mutt.id]
    );
}

#[tokio::test]
async fn a_header_value_is_matched_as_a_substring_case_insensitively() {
    // ADR 0025 Q6, and the motivating example: `X-Mailer` is
    // `Mutt 1.5.24 (2015-08-30)`, so an equality match would never fire.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    postio_index::index::ensure_schema(&connection)
        .await
        .expect("schema");
    let (account, mailbox) = test_support::account_with_inbox(&connection).await;

    let mutt = with_headers(
        &connection,
        &account,
        mailbox,
        "Sent with mutt",
        &[("X-Mailer", "Mutt 1.5.24 (2015-08-30)")],
    )
    .await;
    let _other = with_headers(
        &connection,
        &account,
        mailbox,
        "Sent with something else",
        &[("X-Mailer", "Thunderbird 128.0")],
    )
    .await;

    assert_eq!(
        found(&connection, account.id, "header:x-mailer=mutt").await,
        vec![mutt.id]
    );
    assert!(
        found(&connection, account.id, "header:x-mailer=pine")
            .await
            .is_empty(),
        "a value nothing carries matches nothing"
    );
}

#[tokio::test]
async fn a_name_from_one_header_never_pairs_with_a_value_from_another() {
    // #884's acceptance criterion, and the reason ADR 0025 rejected a single
    // FTS index over the whole block: there, `header:x-mailer=bulk` would
    // match this message, because it does have an `X-Mailer` and the word
    // `bulk` does appear in its headers. A normalized row makes the binding
    // structural rather than something a test has to hope for.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    postio_index::index::ensure_schema(&connection)
        .await
        .expect("schema");
    let (account, mailbox) = test_support::account_with_inbox(&connection).await;

    let _crossed = with_headers(
        &connection,
        &account,
        mailbox,
        "Two headers, no pairing",
        &[("X-Mailer", "Mutt 1.5.24"), ("Precedence", "bulk")],
    )
    .await;

    assert!(
        found(&connection, account.id, "header:x-mailer=bulk")
            .await
            .is_empty(),
        "`bulk` is the Precedence value; pairing it with X-Mailer is the bug"
    );
    assert!(
        found(&connection, account.id, "header:precedence=mutt")
            .await
            .is_empty(),
        "and the same crossing the other way"
    );
    assert_eq!(
        found(&connection, account.id, "header:precedence=bulk")
            .await
            .len(),
        1,
        "while each name still pairs with its own value"
    );
}

#[tokio::test]
async fn a_header_name_is_matched_exactly_and_never_as_a_substring() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    postio_index::index::ensure_schema(&connection)
        .await
        .expect("schema");
    let (account, mailbox) = test_support::account_with_inbox(&connection).await;

    let _mailer = with_headers(
        &connection,
        &account,
        mailbox,
        "Has X-Mailer",
        &[("X-Mailer", "Mutt")],
    )
    .await;

    assert!(
        found(&connection, account.id, "header:x-mail")
            .await
            .is_empty(),
        "`header:x-mail` must not find `X-Mailer` -- a prefix is a different field"
    );
    assert!(
        found(&connection, account.id, "header:mailer")
            .await
            .is_empty(),
        "nor a suffix"
    );
}

#[tokio::test]
async fn any_occurrence_of_a_repeated_header_can_match() {
    // `Received` chains are why `Headers` keeps duplicates and why the table
    // has an `ordinal`. ADR 0025 Q6: any of them matching is a match.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    postio_index::index::ensure_schema(&connection)
        .await
        .expect("schema");
    let (account, mailbox) = test_support::account_with_inbox(&connection).await;

    let relayed = with_headers(
        &connection,
        &account,
        mailbox,
        "Three hops",
        &[
            ("Received", "from first.example.com"),
            ("Received", "from second.example.com"),
            ("Received", "from third.example.com"),
        ],
    )
    .await;

    for hop in ["first", "second", "third"] {
        assert_eq!(
            found(&connection, account.id, &format!("header:received={hop}")).await,
            vec![relayed.id],
            "the {hop} hop is as matchable as any other"
        );
    }
}

#[tokio::test]
async fn a_negated_header_excludes_and_composes_with_other_operators() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    postio_index::index::ensure_schema(&connection)
        .await
        .expect("schema");
    let (account, mailbox) = test_support::account_with_inbox(&connection).await;

    let _bulk = with_headers(
        &connection,
        &account,
        mailbox,
        "Quarterly report",
        &[("Precedence", "bulk")],
    )
    .await;
    let personal = with_headers(
        &connection,
        &account,
        mailbox,
        "Quarterly report",
        &[("X-Mailer", "Mutt")],
    )
    .await;

    assert_eq!(
        found(
            &connection,
            account.id,
            "subject:quarterly -header:precedence"
        )
        .await,
        vec![personal.id],
        "`-header:` excludes exactly the messages that carry the field"
    );
}

#[tokio::test]
async fn a_wildcard_in_a_header_value_is_matched_literally() {
    // `%` and `_` mean something to `LIKE`, and header values are full of
    // them: a MIME boundary, a spam score. Without escaping,
    // `header:x-spam-status=100%` would match every message that has an
    // `X-Spam-Status` at all -- a wrong answer that reads like a right one.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    postio_index::index::ensure_schema(&connection)
        .await
        .expect("schema");
    let (account, mailbox) = test_support::account_with_inbox(&connection).await;

    let scored = with_headers(
        &connection,
        &account,
        mailbox,
        "Scored",
        &[("X-Spam-Status", "No, score=0.1 confidence=100%")],
    )
    .await;
    let _unscored = with_headers(
        &connection,
        &account,
        mailbox,
        "Also scored",
        &[("X-Spam-Status", "No, score=0.1")],
    )
    .await;

    assert_eq!(
        found(&connection, account.id, "header:x-spam-status=100%").await,
        vec![scored.id],
        "the `%` is part of what was asked for, not a wildcard"
    );
    assert!(
        found(&connection, account.id, "header:x-spam-status=score_0.1")
            .await
            .is_empty(),
        "and `_` matches an underscore, not any character"
    );
}

#[tokio::test]
async fn a_message_whose_headers_are_not_indexed_yet_is_not_a_false_negative_for_presence() {
    // The population ADR 0025 Q5 is about, stated as a search result rather
    // than as a pass: a message the catch-up has not reached carries no rows,
    // so `header:` cannot answer for it. What it must never do is answer
    // *wrongly* -- claiming the field is absent is a lie, so the honest
    // behaviour is that it is simply not a hit yet, and the pass is what
    // turns it into one.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    postio_index::index::ensure_schema(&connection)
        .await
        .expect("schema");
    let (account, mailbox) = test_support::account_with_inbox(&connection).await;

    let unindexed = message(&connection, &account, mailbox, "ada", "Not indexed", at(9)).await;

    assert!(
        found(&connection, account.id, "header:x-mailer")
            .await
            .is_empty()
    );

    let headers: postio_model::Headers = [("X-Mailer", "Mutt")].into_iter().collect();
    postio_index::index::index_headers(&connection, unindexed.id.get(), &headers)
        .await
        .expect("the pass reaches it");

    assert_eq!(
        found(&connection, account.id, "header:x-mailer").await,
        vec![unindexed.id],
        "and once it is indexed it is found -- nothing else had to change"
    );
}

/// One "quarterly" in the inbox, and one each in drafts, junk and trash.
async fn a_quarterly_in_every_role(connection: &Connection) -> postio_model::Account {
    let (account, inbox) = test_support::account_with_inbox(connection).await;
    message(
        connection,
        &account,
        inbox,
        "ada",
        "Quarterly report",
        at(9),
    )
    .await;
    for (path, role) in [
        ("Drafts", postio_model::MailboxRole::Drafts),
        ("Junk", postio_model::MailboxRole::Junk),
        ("Trash", postio_model::MailboxRole::Trash),
    ] {
        let mut mailbox = postio_model::Mailbox::new(account.id, path, Some('/'));
        mailbox.role = role;
        postio_storage::repository::MailboxRepository::new(connection)
            .create(&mut mailbox)
            .await
            .expect("create a folder with a role");
        message(
            connection,
            &account,
            mailbox.id,
            "bob",
            "Quarterly leftovers",
            at(10),
        )
        .await;
    }
    account
}

#[tokio::test]
async fn all_mail_leaves_out_drafts_junk_and_trash_unless_in_names_one() {
    // Maintainer's decision (#1523): a search is navigation, and what a
    // person is navigating to is almost never a draft of what they were
    // going to say, something they binned, or spam. Sent stays in. An
    // explicit `in:` reaches any of the three, because then they said so.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    postio_index::index::ensure_schema(&connection)
        .await
        .expect("schema");
    let account = a_quarterly_in_every_role(&connection).await;

    let hits = async |text: &str| {
        let query = parse(text, at(12).date_naive());
        let request = SearchRequest {
            account: AccountScope::Account(account.id),
            query: &query,
            scope: Scope::AllMail,
            limit: 10,
            order: postio_search::ResultOrder::Relevance,
        };
        search(&connection, &request, at(12))
            .await
            .expect("search")
            .total_hits
    };

    assert_eq!(
        hits("quarterly").await,
        1,
        "All mail means every folder except drafts, junk and trash"
    );
    assert_eq!(hits("quarterly in:trash").await, 1, "in: reaches the trash");
    assert_eq!(hits("quarterly in:junk").await, 1, "in: reaches junk");
    assert_eq!(hits("quarterly in:drafts").await, 1, "in: reaches drafts");

    // The scope column counts what switching would show, so it has to agree.
    let query = parse("quarterly", at(12).date_naive());
    let request = SearchRequest {
        account: AccountScope::Account(account.id),
        query: &query,
        scope: Scope::AllMail,
        limit: 10,
        order: postio_search::ResultOrder::Relevance,
    };
    let facets = postio_index::executor::facets(&connection, &request)
        .await
        .expect("facets");
    assert_eq!(
        facets.hits(Scope::AllMail),
        1,
        "\"All mail 4\" over a list of one row is a number nobody can act on"
    );
}

#[tokio::test]
async fn whether_the_corpus_is_complete_is_read_off_the_folders_not_the_messages() {
    // #1612: on a store whose backfill had finished -- the steady state --
    // the completeness check walked every message in the account, reading
    // each row for `body_state`, which is in no index. One statement, one
    // row, so a statement count cannot see it; the plan can.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    postio_index::index::ensure_schema(&connection)
        .await
        .expect("schema");
    let (account, _) = test_support::account_with_inbox(&connection).await;
    let parsed = parse("quarterly", at(12).date_naive());
    let request = SearchRequest {
        account: postio_model::AccountScope::Account(account.id),
        query: &parsed,
        scope: Scope::AllMail,
        order: postio_search::ResultOrder::Relevance,
        limit: 10,
    };
    let (sql, params) = postio_index::executor::corpus_complete_sql(&request);
    let plan: Vec<String> = postio_storage::sql::all(
        &connection,
        &format!("EXPLAIN QUERY PLAN {sql}"),
        params,
        |row| postio_storage::sql::RowExt::col(row, 3),
    )
    .await
    .expect("a plan");
    assert!(
        !plan.iter().any(|step| step.contains("messages")),
        "the completeness check reads the messages table:\n{}",
        plan.join("\n")
    );
}

#[tokio::test]
async fn a_message_moved_today_is_as_old_as_its_own_date() {
    // A message archived or resynced today arrives in its folder today: the
    // server's arrival date is the move, not the mail. Ranked by that, last
    // month's invoice -- archived this morning -- beat this week's, which
    // was still where it landed. Age is the earlier of the two dates.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    postio_index::index::ensure_schema(&connection)
        .await
        .expect("schema");
    let (account, mailbox) = test_support::account_with_inbox(&connection).await;
    let now = Utc.with_ymd_and_hms(2026, 10, 8, 18, 0, 0).unwrap();
    let days = |n: i64| now - chrono::TimeDelta::days(n);

    for i in 0..20 {
        message(
            &connection,
            &account,
            mailbox,
            "carol",
            &format!("Entirely unrelated subject {i}"),
            days(40),
        )
        .await;
    }
    let store = |date: chrono::DateTime<Utc>, received: chrono::DateTime<Utc>, subject: &str| {
        let mut message = Message::new(account.id, mailbox, received);
        message.from = vec![EmailAddress::new(
            Some("Music Studio"),
            "studio@example.com",
        )];
        message.subject = Some(subject.to_string());
        message.date = Some(date);
        message
    };
    let mut older = store(
        days(21),
        now - chrono::TimeDelta::hours(5),
        "Lapiduz 9/17/2026 Invoice",
    );
    let mut newer = store(days(13), days(13), "Lapiduz 9/26/2026 Invoice");
    let repository = MessageRepository::new(&connection);
    repository.create(&mut older).await.expect("older");
    repository.create(&mut newer).await.expect("newer");

    let query = parse("invoice", now.date_naive());
    let ranked = search(
        &connection,
        &SearchRequest {
            account: AccountScope::Account(account.id),
            query: &query,
            scope: Scope::AllMail,
            limit: 10,
            order: postio_search::ResultOrder::Relevance,
        },
        now,
    )
    .await
    .expect("search");
    let order: Vec<_> = ranked.hits.iter().map(|hit| hit.message_id).collect();
    assert_eq!(
        order,
        vec![newer.id, older.id],
        "the newer invoice first, though the older one arrived in its folder today"
    );
}

/// One sender's issues, found by the sender's name: as good a match as each
/// other, so the newest leads, whatever the bodies say or how often each of
/// the sender's addresses was seen.
#[tokio::test]
async fn a_senders_mail_found_by_its_name_comes_newest_first() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    postio_index::index::ensure_schema(&connection)
        .await
        .expect("schema");
    let (account, mailbox) = test_support::account_with_inbox(&connection).await;
    let now = Utc.with_ymd_and_hms(2026, 10, 8, 18, 0, 0).unwrap();
    let days = |n: i64| now - chrono::TimeDelta::days(n);
    for i in 0..20 {
        message(
            &connection,
            &account,
            mailbox,
            "carol",
            &format!("Unrelated {i}"),
            days(40),
        )
        .await;
    }
    let repository = MessageRepository::new(&connection);
    let mut issues = Vec::new();
    for (age, subject, address) in [
        (8, "Distributed databases", "weekly@news.example.com"),
        (1, "Building resilient systems", "deep@news.example.com"),
        (0, "The Pulse: a new trend", "pulse@news.example.com"),
        (2, "The state of the industry", "deep@news.example.com"),
    ] {
        let mut issue = Message::new(account.id, mailbox, days(age));
        issue.from = vec![EmailAddress::new(Some("The Weekly Engineer"), address)];
        issue.subject = Some(subject.to_string());
        issue.date = Some(days(age));
        repository.create(&mut issue).await.expect("issue");
        issues.push((age, issue.id));
    }
    let query = parse("weekly engineer", now.date_naive());
    let ranked = search(
        &connection,
        &SearchRequest {
            account: AccountScope::Account(account.id),
            query: &query,
            scope: Scope::AllMail,
            limit: 10,
            order: postio_search::ResultOrder::Relevance,
        },
        now,
    )
    .await
    .expect("search");
    issues.sort_by_key(|(age, _)| *age);
    let newest_first: Vec<_> = issues.iter().map(|(_, id)| *id).collect();
    let order: Vec<_> = ranked.hits.iter().map(|hit| hit.message_id).collect();
    assert_eq!(order, newest_first);
}

/// A message whose subject says part of what was asked beats one that only
/// mentions it all in passing, when the two are otherwise as good.
#[tokio::test]
async fn a_subject_that_says_part_of_the_query_beats_a_passing_mention() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    postio_index::index::ensure_schema(&connection)
        .await
        .expect("schema");
    let (account, mailbox) = test_support::account_with_inbox(&connection).await;
    let now = Utc.with_ymd_and_hms(2026, 10, 8, 18, 0, 0).unwrap();
    let days = |n: i64| now - chrono::TimeDelta::days(n);
    for i in 0..20 {
        with_body(
            &connection,
            &account,
            mailbox,
            &format!("Unrelated {i}"),
            "nothing here",
            days(40),
        )
        .await;
    }
    let lessons = with_body(
        &connection,
        &account,
        mailbox,
        "Re: Voice Lessons",
        "Hello, I sent the zoom link again for the voice lessons on Tuesday.",
        days(13),
    )
    .await;
    let bingo = with_body(
        &connection,
        &account,
        mailbox,
        "Join us for musical bingo tomorrow",
        "Bingo night! Voice lessons raffle, a zoom link for those at home, and more voice lessons news. Zoom link below.",
        days(1),
    )
    .await;
    let query = parse("zoom link voice lessons", now.date_naive());
    let ranked = search(
        &connection,
        &SearchRequest {
            account: AccountScope::Account(account.id),
            query: &query,
            scope: Scope::AllMail,
            limit: 10,
            order: postio_search::ResultOrder::Relevance,
        },
        now,
    )
    .await
    .expect("search");
    let order: Vec<_> = ranked.hits.iter().map(|hit| hit.message_id).collect();
    assert_eq!(
        order.first(),
        Some(&lessons.id),
        "{order:?} (bingo is {:?})",
        bingo.id
    );
}

// ---------------------------------------------------------------------------
// Phrases, and the forgiving search the command bar asks for (ADR 0037 as
// amended): exact first, then near words -- never for a rule.
// ---------------------------------------------------------------------------

async fn forgiving_world() -> (
    postio_storage::Store,
    postio_model::Account,
    Vec<(&'static str, postio_model::MessageId)>,
) {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    postio_index::index::ensure_schema(&connection)
        .await
        .expect("schema");
    let (account, mailbox) = test_support::account_with_inbox(&connection).await;
    for i in 0..20 {
        with_body(
            &connection,
            &account,
            mailbox,
            &format!("Unrelated {i}"),
            "nothing here",
            at(1),
        )
        .await;
    }
    let mut made = Vec::new();
    for (name, subject, body, hour) in [
        (
            "phone bill",
            "Factura telefonica mama",
            "La factura del mes",
            3,
        ),
        (
            "bill phone",
            "Telefonica: nueva factura",
            "Adjuntamos la factura",
            4,
        ),
        (
            "tickets",
            "Buy two, get movie tickets half off",
            "Offer inside",
            5,
        ),
        ("ticket", "Your ticket for Saturday", "See you there", 6),
        (
            "southwest",
            "Your Southwest flight tomorrow",
            "Boarding at 9",
            7,
        ),
    ] {
        let message = with_body(&connection, &account, mailbox, subject, body, at(hour)).await;
        made.push((name, message.id));
    }
    drop(connection);
    (database, account, made)
}

async fn searched(
    database: &postio_storage::Store,
    account: &postio_model::Account,
    text: &str,
    forgiving: bool,
) -> Vec<postio_model::MessageId> {
    let connection = database.connect().await.expect("checkout");
    let mut query = parse(text, at(12).date_naive());
    if forgiving {
        query = query.forgiving();
    }
    search(
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
    .await
    .expect("search")
    .hits
    .iter()
    .map(|hit| hit.message_id)
    .collect()
}

fn id(made: &[(&str, postio_model::MessageId)], name: &str) -> postio_model::MessageId {
    made.iter().find(|(n, _)| *n == name).expect("made").1
}

#[tokio::test]
async fn a_quoted_phrase_matches_those_words_together() {
    let (database, account, made) = forgiving_world().await;
    assert_eq!(
        searched(&database, &account, "\"factura telefonica\"", false).await,
        vec![id(&made, "phone bill")],
        "the words side by side, in that order; not \"Telefonica: nueva factura\""
    );
    assert_eq!(
        searched(&database, &account, "factura telefonica", false)
            .await
            .len(),
        2,
        "unquoted, both words anywhere"
    );
}

#[tokio::test]
async fn a_forgiving_search_finds_a_plural() {
    let (database, account, made) = forgiving_world().await;
    let found = searched(&database, &account, "ticket", true).await;
    assert_eq!(
        found,
        vec![id(&made, "ticket"), id(&made, "tickets")],
        "the exact word first, then the plural"
    );
}

#[tokio::test]
async fn a_forgiving_search_finds_a_misspelling() {
    let (database, account, made) = forgiving_world().await;
    assert_eq!(
        searched(&database, &account, "southwset flight", true).await,
        vec![id(&made, "southwest")]
    );
}

#[tokio::test]
async fn a_forgiving_search_finds_a_misspelling_in_a_body() {
    let (database, account, made) = forgiving_world().await;
    assert_eq!(
        searched(&database, &account, "boardng", true).await,
        vec![id(&made, "southwest")],
        "the body index is folded, and its near words are read from it"
    );
}

/// A mailbox where a misspelling's neighbours are mostly the wrong word:
/// sixty trips beside one girl scout troop, every one saying "your" and
/// "or" -- the word a disjunction becomes if it is lowercased.
async fn crowded_world() -> (
    postio_storage::Store,
    postio_model::Account,
    Vec<(&'static str, postio_model::MessageId)>,
) {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    postio_index::index::ensure_schema(&connection)
        .await
        .expect("schema");
    let (account, mailbox) = test_support::account_with_inbox(&connection).await;
    for i in 0..60 {
        with_body(
            &connection,
            &account,
            mailbox,
            &format!("Your trip {i}"),
            "Scout the route, or stay home: your trip, your tour",
            at(1),
        )
        .await;
    }
    let mut made = Vec::new();
    for (name, subject, body, hour) in [
        ("troop", "Girl scout troop meeting", "Bring the cookies", 3),
        ("tour", "Redfin home tour confirmed", "See you Saturday", 4),
        ("your home", "Your Redfin home value", "Updated estimate", 5),
        // So "girl" has a word it begins, and is widened too.
        ("girls", "Girls night", "Bring snacks", 6),
    ] {
        let message = with_body(&connection, &account, mailbox, subject, body, at(hour)).await;
        made.push((name, message.id));
    }
    drop(connection);
    (database, account, made)
}

#[tokio::test]
async fn a_misspelling_is_read_beside_the_words_around_it() {
    // "trop" is one edit from "trip", which sixty messages hold, and from
    // "troop", which one does; the words beside it say which was meant.
    let (database, account, made) = crowded_world().await;
    assert_eq!(
        searched(&database, &account, "girl scout trop", true).await,
        vec![id(&made, "troop")]
    );
}

#[tokio::test]
async fn a_word_spelled_right_is_not_widened_to_its_neighbours() {
    // "tour" is a word the mailbox holds: "your", one edit away, is not
    // what was meant.
    let (database, account, made) = crowded_world().await;
    assert_eq!(
        searched(&database, &account, "redfn home tour", true).await,
        vec![id(&made, "tour")]
    );
}

#[tokio::test]
async fn a_rule_or_saved_search_stays_exact() {
    // ADR 0037: a rule acts on what its query says, never on a near miss.
    let (database, account, made) = forgiving_world().await;
    assert_eq!(
        searched(&database, &account, "ticket", false).await,
        vec![id(&made, "ticket")]
    );
    assert!(
        searched(&database, &account, "southwset", false)
            .await
            .is_empty()
    );
}

// ---------------------------------------------------------------------------
// The path GTK uses, pinned (spec 010 T003, FR-046)
// ---------------------------------------------------------------------------

/// One message of the pinned corpus: who, where, what, when, and its state.
struct Pinned {
    from: &'static str,
    folder: &'static str,
    subject: &'static str,
    body: &'static str,
    /// Days after 2026-06-01, at noon.
    day: i64,
    unread: bool,
    flagged: bool,
    file: Option<&'static str>,
}

/// Twenty-one messages across an inbox, an archive and a receipts folder.
/// A row's number in this table is its index in the recorded answers below.
const PINNED: [Pinned; 21] = [
    Pinned {
        from: "ada",
        folder: "INBOX",
        subject: "Re: Atlas Q3 budget, final numbers",
        body: "The final numbers for the atlas budget are in the sheet. Please approve by Friday.",
        day: 80,
        unread: true,
        flagged: false,
        file: Some("atlas-q3-budget.xlsx"),
    },
    Pinned {
        from: "tomas",
        folder: "INBOX",
        subject: "Atlas staffing plan for Q4",
        body: "Two more platform roles move the atlas budget up by nine percent.",
        day: 79,
        unread: true,
        flagged: true,
        file: None,
    },
    Pinned {
        from: "ada",
        folder: "Archive",
        subject: "Atlas budget template v2",
        body: "The summary sheet totals the budget for the year.",
        day: 40,
        unread: false,
        flagged: false,
        file: Some("atlas-budget-template.xlsx"),
    },
    Pinned {
        from: "priya",
        folder: "INBOX",
        subject: "Contractor invoices for September",
        body: "Two of them should be charged to the atlas budget line, not harbor.",
        day: 77,
        unread: false,
        flagged: false,
        file: Some("invoices-sep.pdf"),
    },
    Pinned {
        from: "finance",
        folder: "Archive",
        subject: "Q3 close: budget owners please review",
        body: "Owners are listed below. Please confirm by the thirtieth.",
        day: 74,
        unread: false,
        flagged: false,
        file: None,
    },
    Pinned {
        from: "ben",
        folder: "Archive",
        subject: "Re: Harbor SOW",
        body: "Legal agrees to move the vendor cost out of the atlas budget.",
        day: 71,
        unread: false,
        flagged: false,
        file: None,
    },
    Pinned {
        from: "ada",
        folder: "Receipts",
        subject: "Atlas September actuals",
        body: "Actuals are attached. Page two has the spend to date.",
        day: 66,
        unread: false,
        flagged: false,
        file: Some("atlas-sep-actuals.pdf"),
    },
    Pinned {
        from: "grace",
        folder: "Archive",
        subject: "Design headcount for Atlas",
        body: "The two design roles come out of the harbor budget, not atlas.",
        day: 62,
        unread: false,
        flagged: false,
        file: None,
    },
    Pinned {
        from: "tomas",
        folder: "Archive",
        subject: "Re: Atlas offsite",
        body: "The offsite fits inside the atlas budget if we skip the dinner.",
        day: 55,
        unread: false,
        flagged: false,
        file: None,
    },
    Pinned {
        from: "lena",
        folder: "Archive",
        subject: "Harbor vs Atlas split for shared tooling",
        body: "Proposing sixty forty between harbor and atlas for the runners.",
        day: 48,
        unread: false,
        flagged: true,
        file: None,
    },
    Pinned {
        from: "priya",
        folder: "INBOX",
        subject: "Invoice 2026-08, Atlas contractor hours",
        body: "Attached is the invoice for august, two hundred twelve hours.",
        day: 70,
        unread: true,
        flagged: false,
        file: Some("invoice-2026-08.pdf"),
    },
    Pinned {
        from: "ada",
        folder: "Archive",
        subject: "Budget approvals",
        body: "Budget approvals are due on the twenty-eighth. Review your lines.",
        day: 30,
        unread: false,
        flagged: false,
        file: None,
    },
    Pinned {
        from: "ben",
        folder: "INBOX",
        subject: "Harbor API questions",
        body: "Two questions on pagination and rate limits.",
        day: 60,
        unread: false,
        flagged: false,
        file: None,
    },
    Pinned {
        from: "lena",
        folder: "INBOX",
        subject: "Harbor weekly notes",
        body: "The pagination change is in review. The budget is unchanged.",
        day: 58,
        unread: false,
        flagged: false,
        file: None,
    },
    Pinned {
        from: "shop",
        folder: "Receipts",
        subject: "Your coffee order",
        body: "Thank you for your order. Here is your receipt.",
        day: 75,
        unread: false,
        flagged: false,
        file: None,
    },
    Pinned {
        from: "shop",
        folder: "Receipts",
        subject: "Bookshop receipt",
        body: "Thank you for your purchase.",
        day: 50,
        unread: false,
        flagged: false,
        file: None,
    },
    Pinned {
        from: "grace",
        folder: "INBOX",
        subject: "Design review",
        body: "The design review is Friday. Agenda: navigation and empty states.",
        day: 76,
        unread: true,
        flagged: false,
        file: None,
    },
    Pinned {
        from: "omar",
        folder: "Archive",
        subject: "Travel plans",
        body: "Flights are booked for the conference.",
        day: 12,
        unread: false,
        flagged: false,
        file: None,
    },
    Pinned {
        from: "sofia",
        folder: "Archive",
        subject: "Dinner on Saturday",
        body: "Are you free for dinner on Saturday?",
        day: 5,
        unread: false,
        flagged: false,
        file: None,
    },
    Pinned {
        from: "finance",
        folder: "INBOX",
        subject: "Quarterly planning",
        body: "Send your top three asks for the quarter. The atlas roadmap comes first.",
        day: 78,
        unread: false,
        flagged: false,
        file: None,
    },
    Pinned {
        from: "omar",
        folder: "INBOX",
        subject: "Reading list",
        body: "Two papers and a talk this month.",
        day: 20,
        unread: false,
        flagged: false,
        file: None,
    },
];

/// What the GTK path answered for one query over [`PINNED`] when it was
/// pinned (rows are indexes into the table, in the order the hits came): the hits' rows in order, the total, the matches per scope
/// (all mail, inbox, lists) and the refinements offered.
struct Answer {
    query: &'static str,
    scope: Scope,
    order: postio_search::ResultOrder,
    rows: &'static [usize],
    total: u64,
    scopes: [u64; 3],
    offered: &'static [(&'static str, u64)],
}

/// `executor::search` and `executor::facets` over a fixed corpus: the answers
/// the GTK app gets today. Every phase of spec 010 adds a second path
/// (`search_conversations`, `Fold`, the attachment-text arm, `label:` and
/// `has:action`); this test is what proves none of them moved the first
/// (FR-046, D11). It was green on arrival and must stay so. A change that
/// moves one of these answers on purpose edits the table in the same commit
/// and says why.
#[tokio::test]
async fn gtk_search_path_is_pinned() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    postio_index::index::ensure_schema(&connection)
        .await
        .expect("schema");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let archive = test_support::mailbox(&connection, &account, "Archive").await;
    let receipts = test_support::mailbox(&connection, &account, "Receipts").await;

    let start = Utc.with_ymd_and_hms(2026, 6, 1, 12, 0, 0).unwrap();
    let mut ids = Vec::new();
    for row in &PINNED {
        let mailbox = match row.folder {
            "INBOX" => inbox,
            "Archive" => archive.id,
            _ => receipts.id,
        };
        let mut created =
            Message::new(account.id, mailbox, start + chrono::Duration::days(row.day));
        created.from = vec![EmailAddress::new(
            Some(row.from),
            format!("{}@example.com", row.from),
        )];
        created.subject = Some(row.subject.to_string());
        if !row.unread {
            created.flags.insert(postio_model::Flag::Seen);
        }
        if row.flagged {
            created.flags.insert(postio_model::Flag::Flagged);
        }
        if let Some(name) = row.file {
            let mut file = Attachment::new(
                postio_model::MessageId::UNASSIGNED,
                "application/octet-stream",
                4096,
            );
            file.filename = Some(name.to_string());
            created.attachments.push(file);
        }
        MessageRepository::new(&connection)
            .create(&mut created)
            .await
            .expect("create");
        postio_index::index::index_body(&connection, created.id.get(), Some(row.body))
            .await
            .expect("index body");
        body_here(&connection, &created).await;
        ids.push(created.id);
    }

    let now = Utc.with_ymd_and_hms(2026, 8, 30, 12, 0, 0).unwrap();
    for expected in &PINNED_ANSWERS {
        let query = parse(expected.query, now.date_naive());
        let request = SearchRequest {
            account: AccountScope::Account(account.id),
            query: &query,
            scope: expected.scope,
            limit: 10,
            order: expected.order,
        };
        let results = search(&connection, &request, now).await.expect("search");
        let facets = postio_index::executor::facets(&connection, &request)
            .await
            .expect("facets");
        let rows: Vec<usize> = results
            .hits
            .iter()
            .map(|hit| {
                ids.iter()
                    .position(|id| *id == hit.message_id)
                    .expect("a pinned row")
            })
            .collect();
        let scopes = [
            facets.hits(Scope::AllMail),
            facets.hits(Scope::Inbox),
            facets.hits(Scope::Lists),
        ];
        let offered: Vec<(String, u64)> = facets
            .suggested(results.total_hits)
            .iter()
            .map(|refinement| (refinement.token.clone(), refinement.hits))
            .collect();
        let said = |what: &str| format!("{what} for {:?}", expected.query);
        assert_eq!(rows, expected.rows, "{}", said("hits and their order"));
        assert_eq!(results.total_hits, expected.total, "{}", said("the total"));
        assert_eq!(scopes, expected.scopes, "{}", said("the scope columns"));
        let offered: Vec<(&str, u64)> = offered
            .iter()
            .map(|(token, hits)| (token.as_str(), *hits))
            .collect();
        assert_eq!(offered, expected.offered, "{}", said("the refinements"));
    }
}

const PINNED_ANSWERS: [Answer; 12] = {
    use postio_search::ResultOrder::{Newest, Relevance};
    [
        Answer {
            query: "atlas",
            scope: Scope::AllMail,
            order: Relevance,
            rows: &[0, 1, 10, 6, 7, 8, 9, 2, 19, 3],
            total: 11,
            scopes: [11, 5, 1],
            offered: &[
                ("has:attach", 5),
                ("in:Archive", 5),
                ("in:INBOX", 5),
                ("is:unread", 3),
            ],
        },
        Answer {
            query: "atlas budget",
            scope: Scope::AllMail,
            order: Relevance,
            rows: &[0, 2, 1, 7, 8, 3, 5],
            total: 7,
            scopes: [7, 3, 0],
            offered: &[
                ("in:Archive", 4),
                ("has:attach", 3),
                ("in:INBOX", 3),
                ("is:unread", 2),
            ],
        },
        Answer {
            query: "budget",
            scope: Scope::AllMail,
            order: Relevance,
            rows: &[0, 4, 2, 11, 1, 3, 5, 7, 13, 8],
            total: 10,
            scopes: [10, 4, 0],
            offered: &[
                ("in:Archive", 6),
                ("in:INBOX", 4),
                ("has:attach", 3),
                ("is:unread", 2),
            ],
        },
        Answer {
            query: "from:ada budget",
            scope: Scope::AllMail,
            order: Relevance,
            rows: &[0, 2, 11],
            total: 3,
            scopes: [3, 1, 0],
            offered: &[
                ("has:attach", 2),
                ("in:Archive", 2),
                ("is:unread", 1),
                ("in:INBOX", 1),
            ],
        },
        Answer {
            query: "in:archive atlas",
            scope: Scope::AllMail,
            order: Relevance,
            rows: &[7, 8, 9, 2, 5],
            total: 5,
            scopes: [5, 0, 0],
            offered: &[("is:flagged", 1), ("has:attach", 1)],
        },
        Answer {
            query: "has:attachment budget",
            scope: Scope::AllMail,
            order: Relevance,
            rows: &[0, 2, 3],
            total: 3,
            scopes: [3, 2, 0],
            offered: &[("in:INBOX", 2), ("is:unread", 1), ("in:Archive", 1)],
        },
        Answer {
            query: "is:unread",
            scope: Scope::AllMail,
            order: Newest,
            rows: &[0, 1, 16, 10],
            total: 4,
            scopes: [4, 4, 0],
            offered: &[("has:attach", 2), ("is:flagged", 1)],
        },
        Answer {
            query: "after:2026-07-15 atlas",
            scope: Scope::AllMail,
            order: Relevance,
            rows: &[0, 1, 10, 6, 7, 8, 9, 19, 3, 5],
            total: 10,
            scopes: [10, 5, 1],
            offered: &[
                ("in:INBOX", 5),
                ("has:attach", 4),
                ("in:Archive", 4),
                ("is:unread", 3),
            ],
        },
        Answer {
            query: "budget -atlas",
            scope: Scope::AllMail,
            order: Relevance,
            rows: &[4, 11, 13],
            total: 3,
            scopes: [3, 1, 0],
            offered: &[("in:Archive", 2), ("in:INBOX", 1)],
        },
        Answer {
            query: "subject:invoice",
            scope: Scope::Inbox,
            order: Relevance,
            rows: &[10],
            total: 1,
            scopes: [1, 1, 0],
            offered: &[],
        },
        Answer {
            query: "\"atlas budget\"",
            scope: Scope::AllMail,
            order: Newest,
            rows: &[0, 1, 3, 5, 8, 2],
            total: 6,
            scopes: [6, 3, 0],
            offered: &[
                ("has:attach", 3),
                ("in:Archive", 3),
                ("in:INBOX", 3),
                ("is:unread", 2),
            ],
        },
        Answer {
            query: "filename:xlsx",
            scope: Scope::AllMail,
            order: Relevance,
            rows: &[0, 2],
            total: 2,
            scopes: [2, 1, 0],
            offered: &[("is:unread", 1), ("in:Archive", 1), ("in:INBOX", 1)],
        },
    ]
};
