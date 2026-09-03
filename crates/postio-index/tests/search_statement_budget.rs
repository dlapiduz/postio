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

use chrono::{TimeZone, Utc};
use postio_index::{SearchRequest, search};
use postio_model::AccountScope;
use postio_model::{EmailAddress, Message};
use postio_search::facets::Scope;
use postio_search::parse;
use postio_storage::repository::MessageRepository;
use postio_storage::test_support;
use postio_storage::test_support::counting::{Counts, counted, install};

/// Comfortably past `executor.rs`'s `RANK_BY_RELEVANCE_LIMIT` of 2,000, so the
/// broad query takes the too-many-to-rank path the budget is most at risk on.
const MATCHES: usize = 2_500;

fn today() -> chrono::NaiveDate {
    chrono::NaiveDate::from_ymd_opt(2026, 8, 21).expect("a real date")
}

#[test]
fn a_search_costs_the_same_queries_however_much_it_matches() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    postio_index::index::ensure_schema(&connection).expect("schema");
    let (account, mailbox) = test_support::account_with_inbox(&connection);

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
        messages.create(&mut message).expect("create message");
    }

    let broad_query = parse("quarterly", today());
    let narrow_query = parse(&format!("{}", MATCHES - 1), today());
    let now = Utc.with_ymd_and_hms(2026, 8, 21, 12, 0, 0).unwrap();

    install(&connection);
    let run = |query, limit| {
        let request = SearchRequest {
            account: AccountScope::Account(account.id),
            query,
            scope: Scope::AllMail,
            limit,
            order: postio_search::ResultOrder::Relevance,
        };
        let mut hits = 0;
        let counts: Counts = counted(|| {
            hits = search(&connection, &request, now)
                .expect("a page of results")
                .hits
                .len();
        });
        (counts, hits)
    };

    let (broad, broad_hits) = run(&broad_query, 25);
    let (narrow, narrow_hits) = run(&narrow_query, 25);

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
    let with_one_more = counted(|| {
        let request = SearchRequest {
            account: AccountScope::Account(account.id),
            query: &broad_query,
            scope: Scope::AllMail,
            limit: 25,
            order: postio_search::ResultOrder::Relevance,
        };
        let _ = search(&connection, &request, now).expect("a page of results");
        let _: i64 = connection
            .query_row("SELECT count(*) FROM messages", [], |row| row.get(0))
            .expect("one more query, standing in for a per-hit lookup");
    });
    assert_eq!(
        with_one_more.statements,
        broad.statements + 1,
        "adding a query to the counted block did not change the count, so the \
         equalities either side of this are not measuring what they claim"
    );

    // The other shape an N+1 takes: per *hit* rather than per match. A wider
    // page must not cost more queries either — the pool is hydrated in one.
    let (wide, wide_hits) = run(&broad_query, 100);
    assert_eq!(wide_hits, 100, "a page of 100 should come back full");
    assert_eq!(
        wide.statements, broad.statements,
        "showing {wide_hits} hits issued {} statements where {broad_hits} \
         issued {}. That is a query per hit, which is invisible in the results \
         and fatal to the budget.",
        wide.statements, broad.statements
    );
}
