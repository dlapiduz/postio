//! Local search costs the same number of queries whatever it matches.
//!
//! `docs/PRODUCT.md` §18 budgets local search at 100ms, and `CLAUDE.md` calls
//! that "enforced by benches in CI". It is not: `bench.yml` compiles the bench
//! targets and deliberately times nothing, because a shared runner cannot
//! defend a millisecond budget. That decision is right, and it leaves the
//! budget as documentation (#100).
//!
//! So count the budget's cause. Search stays under 100ms because the work it
//! does is bounded by the *page*: `executor.rs` caps ranking at
//! `RANK_BY_RELEVANCE_LIMIT` matches and hydrates one candidate pool sized
//! from `limit`. That reasoning is written out at length in its comments and
//! nothing tests it. A change that fetched each hit's details in a query of
//! its own would return byte-identical results and pass every other test in
//! this workspace, while turning a common word into a slow search.
//!
//! **Statements, not rows.** The obvious measure for "does not read the whole
//! match set" is rows produced, and it is the right one on plain SQL — see
//! `postio-storage`'s `list_statement_count.rs`. It does not survive contact
//! with FTS5, whose cursor runs lookups of its own between two rows of the
//! statement being stepped. That leaves a row count dependent on how those
//! nested lookups are attributed: this same page measured 1,467 rows when
//! they were credited to the statement that was running and 51 when they were
//! not, and the count in between moves with how the index happens to be
//! segmented. `test_support::counting` takes the second reading and says so;
//! either way it is not a number to hang a budget on.
//!
//! The count of application statements does not move. It is four here — for a
//! query matching one message, for one matching 2,500, and for a page four
//! times wider.
//!
//! # It costs ~55s and stays on the merge path anyway
//!
//! Worth saying, because it is the slowest thing in this crate's suite and
//! the obvious reaction is to move it to the nightly tier beside
//! `total_hits_cap` (2026-09-19, when the landing gate gained a four-minute
//! budget and this was 53% of a 103s run).
//!
//! Do not. `total_hits_cap` is expensive for its *fixture* and asserts
//! something a nightly can catch a day later. This is the gate `CLAUDE.md`
//! asks for by name on a read path — the cause of a budget, counted — and the
//! change it exists to catch is one that returns byte-identical results and
//! passes everything else. A day is a long time to be shipping a search that
//! got slower with the size of the mailbox, and the cost is the 2,500
//! messages it needs to prove the count is flat, which is the test.
//!
//! `header:` is measured separately and against itself, because it is the one
//! operator that is not an FTS `MATCH`: ADR 0025 Q2 compiles it to a
//! correlated `EXISTS` over `message_headers`, which is a different *plan*
//! from every other operator and therefore not comparable to one. What has to
//! hold is the same property — that a `header:` matching the whole mailbox
//! costs no more statements than one matching a single message, and that a
//! wider page costs no more either.

use chrono::{TimeZone, Utc};
use postio_index::{SearchRequest, search};
use postio_model::AccountScope;
use postio_model::{EmailAddress, Message};
use postio_search::facets::Scope;
use postio_search::parse;
use postio_storage::repository::MessageRepository;
use postio_storage::test_support;
use postio_storage::test_support::counting::{Counts, counted_async, install};

/// Comfortably past `executor.rs`'s `RANK_BY_RELEVANCE_LIMIT` of 2,000, so the
/// broad query takes the too-many-to-rank path the budget is most at risk on.
const MATCHES: usize = 2_500;

async fn today() -> chrono::NaiveDate {
    chrono::NaiveDate::from_ymd_opt(2026, 8, 21).expect("a real date")
}

#[tokio::test]
async fn a_search_costs_the_same_queries_however_much_it_matches() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    postio_index::index::ensure_schema(&connection)
        .await
        .expect("schema");
    let (account, mailbox) = test_support::account_with_inbox(&connection).await;

    // Every message carries "quarterly"; each also carries its own number, so
    // one corpus answers both a query matching everything and one matching a
    // single message.
    let messages = MessageRepository::new(&connection);
    for nth in 0..MATCHES {
        let received = Utc.with_ymd_and_hms(2026, 8, 20, 9, 0, 0).unwrap()
            + chrono::Duration::seconds(nth as i64);
        let mut message = Message::new(account.id, mailbox, received);
        message.from = vec![EmailAddress::new(Some("ada"), "ada@example.com")];
        message.subject = Some(format!("quarterly report {nth}"));
        messages.create(&mut message).await.expect("create message");
        // One header block per message, the same shape: `X-Mailer` on all of
        // them so `header:x-mailer=mutt` matches everything, and the number
        // in the value so one query can pick out a single message.
        let headers: postio_model::Headers = [("X-Mailer", format!("Mutt 1.5.24 build {nth}"))]
            .into_iter()
            .collect();
        postio_index::index::index_headers(&connection, message.id.get(), &headers)
            .await
            .expect("index headers");
    }

    let broad_query = parse("quarterly", today().await);
    let narrow_query = parse(&format!("{}", MATCHES - 1), today().await);
    let now = Utc.with_ymd_and_hms(2026, 8, 21, 12, 0, 0).unwrap();

    install(&connection);
    let run = async |query, limit| {
        let request = SearchRequest {
            account: AccountScope::Account(account.id),
            query,
            scope: Scope::AllMail,
            limit,
            order: postio_search::ResultOrder::Relevance,
        };
        let mut hits = 0;
        let counts: Counts = counted_async(async || {
            hits = search(&connection, &request, now)
                .await
                .expect("a page of results")
                .hits
                .len();
        })
        .await;
        (counts, hits)
    };

    let (broad, broad_hits) = run(&broad_query, 25).await;
    let (narrow, narrow_hits) = run(&narrow_query, 25).await;

    assert_eq!(broad_hits, 25, "a broad query should fill a page of 25");
    assert!(
        (1..25).contains(&narrow_hits),
        "the narrow query matched {narrow_hits} messages; it is meant to match \
         a handful, so that it and the broad query are genuinely different \
         amounts of work"
    );
    assert_eq!(
        broad.statements, narrow.statements,
        "a query matching {MATCHES} messages issued {} statements where one \
         matching {narrow_hits} issued {}. Search that costs more queries the \
         more it matches is what §18's 100ms budget cannot survive, and it \
         returns the same results either way, so nothing else here notices.",
        broad.statements, narrow.statements
    );

    // The control, and the reason the equalities above are known to have
    // teeth: one extra query inside the counted block moves the number. #100
    // asks that each counted budget fail when the invariant it guards is
    // deliberately broken, and an N+1 is exactly this, once per hit.
    let with_one_more = counted_async(async || {
        let request = SearchRequest {
            account: AccountScope::Account(account.id),
            query: &broad_query,
            scope: Scope::AllMail,
            limit: 25,
            order: postio_search::ResultOrder::Relevance,
        };
        let _ = search(&connection, &request, now)
            .await
            .expect("a page of results");
        let _: i64 =
            postio_storage::sql::one(&connection, "SELECT count(*) FROM messages", (), |row| {
                postio_storage::sql::RowExt::col(row, 0)
            })
            .await
            .expect("one more query, standing in for a per-hit lookup");
    });
    assert_eq!(
        with_one_more.await.statements,
        broad.statements + 1,
        "adding a query to the counted block did not change the count, so the \
         equalities either side of this are not measuring what they claim"
    );

    // ADR 0025's new join. `header:` narrows on `message_headers.name` inside
    // a correlated `EXISTS`, which is a plan no other operator produces — so
    // it gets its own pair rather than being compared against the FTS ones.
    // A `header:` that matches the whole mailbox must cost what one matching
    // a single message costs; anything else is a per-match read on a path
    // that returns identical results either way.
    let header_broad_query = parse("header:x-mailer=mutt", today().await);
    let header_narrow_query = parse(
        &format!("header:x-mailer=\"build {}\"", MATCHES - 1),
        today().await,
    );
    let (header_broad, header_broad_hits) = run(&header_broad_query, 25).await;
    let (header_narrow, header_narrow_hits) = run(&header_narrow_query, 25).await;

    assert_eq!(
        header_broad_hits, 25,
        "`header:x-mailer=mutt` is on every message, so it should fill a page"
    );
    assert_eq!(
        header_narrow_hits, 1,
        "and the numbered value should pick out exactly one"
    );
    assert_eq!(
        header_broad.statements, header_narrow.statements,
        "a `header:` matching {MATCHES} messages issued {} statements where \
         one matching {header_narrow_hits} issued {}. The EXISTS is correlated \
         to the outer message, so a plan that lost `idx_message_headers_name` \
         would read the whole table per row and return the same results.",
        header_broad.statements, header_narrow.statements
    );

    let (header_wide, header_wide_hits) = run(&header_broad_query, 100).await;
    assert_eq!(header_wide_hits, 100, "a page of 100 should come back full");
    assert_eq!(
        header_wide.statements, header_broad.statements,
        "showing {header_wide_hits} `header:` hits issued {} statements where \
         {header_broad_hits} issued {}: a query per hit.",
        header_wide.statements, header_broad.statements
    );

    // The other shape an N+1 takes: per *hit* rather than per match. A wider
    // page must not cost more queries either — the pool is hydrated in one.
    let (wide, wide_hits) = run(&broad_query, 100).await;
    assert_eq!(wide_hits, 100, "a page of 100 should come back full");
    assert_eq!(
        wide.statements, broad.statements,
        "showing {wide_hits} hits issued {} statements where {broad_hits} \
         issued {}. That is a query per hit, which is invisible in the results \
         and fatal to the budget.",
        wide.statements, broad.statements
    );
}

#[tokio::test]
async fn the_facets_walk_the_match_once_per_scope() {
    // #1612: facets walked the match set five times -- three scope counts,
    // then the flag refinements and the folder refinements, each its own
    // capped walk of the same match in the same scope. The two refinements
    // and the current scope's count are one walk grouped by folder; only
    // the scopes the user is *not* in need walks of their own.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    postio_index::index::ensure_schema(&connection)
        .await
        .expect("schema");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let archive = test_support::mailbox(&connection, &account, "Archive")
        .await
        .id;
    let messages = MessageRepository::new(&connection);
    for nth in 0..30 {
        let received = Utc.with_ymd_and_hms(2026, 8, 20, 9, 0, 0).unwrap()
            + chrono::Duration::seconds(nth as i64);
        let folder = if nth % 3 == 0 { archive } else { inbox };
        let mut message = Message::new(account.id, folder, received);
        message.from = vec![EmailAddress::new(Some("ada"), "ada@example.com")];
        message.subject = Some(format!("quarterly report {nth}"));
        if nth % 5 == 0 {
            message.flags = [postio_model::Flag::Flagged].into_iter().collect();
        }
        messages.create(&mut message).await.expect("create message");
    }
    let query = parse("quarterly", today().await);
    let request = SearchRequest {
        account: AccountScope::Account(account.id),
        query: &query,
        scope: Scope::AllMail,
        limit: 25,
        order: postio_search::ResultOrder::Relevance,
    };
    install(&connection);
    let mut facets = None;
    let counts: Counts = counted_async(async || {
        facets = Some(
            postio_index::executor::facets(&connection, &request)
                .await
                .expect("facets"),
        );
    })
    .await;
    let facets = facets.expect("facets ran");

    let hits = |scope: Scope| {
        facets
            .scopes
            .iter()
            .find(|count| count.scope == scope)
            .map(|count| count.hits)
    };
    assert_eq!(hits(Scope::AllMail), Some(30), "every message matched");
    assert_eq!(
        hits(Scope::Inbox),
        Some(20),
        "two in three are in the inbox"
    );
    let refinement = |token: &str| {
        facets
            .refinements
            .iter()
            .find(|refinement| refinement.token == token)
            .map(|refinement| refinement.hits)
    };
    assert_eq!(refinement("is:flagged"), Some(6));
    assert_eq!(refinement("in:INBOX"), Some(20));
    assert_eq!(refinement("in:Archive"), Some(10));
    assert!(
        counts.statements <= 3,
        "the facets took {} statements; the match is walked once per scope and \
         the refinements share the current scope's walk",
        counts.statements
    );
}
