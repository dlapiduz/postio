//! What a search *costs*, counted rather than timed (#1149).
//!
//! `docs/PRODUCT.md` §1 puts local search under **100 ms** and `CLAUDE.md`
//! says what to do about that: gate the cause, not the effect.
//! `bench.yml` deliberately times nothing, because a shared runner cannot
//! defend a millisecond figure — so a wall-clock assertion here would be
//! green on a quiet machine, red on a loaded one, and evidence of nothing in
//! both cases. `postio_storage::test_support::counting` reads statements off
//! SQLite's trace hook, and those are the same numbers on any machine.
//!
//! **Statements, not rows**, and that is not a preference. `counting`'s own
//! docs say why: an FTS5 cursor runs its segment lookups between two rows of
//! the statement being stepped, so rows after one of those are attributed to
//! the lookup and the row count under-reports. One search of a common word
//! showed 1,111 internal invocations against a page of 25. A budget built on
//! that number would fail when SQLite merged segments differently and pass
//! when the application started reading whole mailboxes.
//!
//! What these hold is the *shape*: the work a search does must not grow with
//! the store it searches.

use chrono::Utc;
use postio_model::{BodyState, Message};
use postio_search::facets::Scope;
use postio_storage::repository::{MessageRepository, StoredBody};
use postio_storage::test_support;
use postio_storage::test_support::counting::{counted, install};

/// The word every seeded message contains, so a query matches all of them.
const COMMON: &str = "quarterly";

/// A store of `count` messages whose bodies are indexed and all match
/// [`COMMON`].
///
/// Indexed explicitly: `seed_large` writes messages but not the body index —
/// the indexer is a separate pass — and a search over an unindexed store
/// would measure an empty result set and call it cheap.
fn indexed(count: usize) -> (postio_storage::Database, postio_model::ids::AccountId) {
    let database = test_support::memory();
    let connection = database.connection().expect("a connection");
    let (account, inbox) = test_support::account_with_inbox(&connection);
    postio_index::index::ensure_schema(&connection).expect("the index schema");

    let repository = MessageRepository::new(&connection);
    for n in 0..count {
        let body = format!("message {n} about the {COMMON} numbers we discussed");
        let mut message = Message::new(account.id, inbox, Utc::now());
        message.subject = Some(format!("Report {n}"));
        message.sync.body_state = BodyState::Full;
        repository.create(&mut message).expect("a message");
        repository
            .set_body(
                message.id,
                &StoredBody {
                    text: Some(body.clone()),
                    html: None,
                    headers: None,
                    headers_truncated: false,
                    encoding_problems: false,
                },
                BodyState::Full,
            )
            .expect("a body");
        postio_index::index::index_body(&connection, message.id.get(), Some(&body))
            .expect("an indexed body");
    }
    let id = account.id;
    drop(connection);
    (database, id)
}

/// One search of [`COMMON`], and what SQLite did for it.
fn cost_of_searching(
    store: &(postio_storage::Database, postio_model::ids::AccountId),
) -> (usize, usize) {
    let (database, account) = store;
    let connection = database.connection().expect("a connection");
    let query = postio_search::parse(COMMON, Utc::now().date_naive());

    let run = |connection: &postio_storage::PooledConnection| {
        postio_session::search::execute(
            connection,
            postio_model::AccountScope::Account(*account),
            &query,
            Scope::AllMail,
            postio_search::ResultOrder::Relevance,
        )
        .expect("the search runs")
    };

    // Warm first: the first `prepare` of a statement pulls schema pages in,
    // and this is about the query's shape rather than a cold cache.
    let warm = run(&connection);
    let hits = warm.hits.len();

    install(&connection);
    let counts = counted(|| {
        let _ = run(&connection);
    });
    (counts.statements, hits)
}

#[test]
fn a_search_costs_the_same_statements_however_large_the_store() {
    // The property, and the only one that means anything: a search over ten
    // times the mail must not be ten times the work. A cost that grew with
    // the store is the shape that stops meeting 100ms as a mailbox fills, and
    // it is invisible to every test that only checks the hits are right.
    //
    // **Both stores are past the excerpt cap, and that is the whole
    // measurement.** A search is a fixed overhead plus one body read per
    // excerpt, and excerpts stop at `SNIPPET_HITS` — measured here, 40 hits
    // cost 44 statements and 200 cost 54, which is exactly the ten extra
    // excerpts between 40 and the cap of 50. So below the cap the cost does
    // rise with the hits, by design; above it nothing more is read and the
    // number goes flat. Comparing a small store against a large one would
    // measure that ramp and call it a regression, which is what the first
    // version of this test did.
    let smaller = indexed(120);
    let larger = indexed(400);

    let (smaller_statements, smaller_hits) = cost_of_searching(&smaller);
    let (larger_statements, larger_hits) = cost_of_searching(&larger);

    assert!(
        smaller_hits > 50 && larger_hits > 50,
        "both stores have to be past the excerpt cap for this to be flat; \
         got {smaller_hits} and {larger_hits} hits"
    );
    assert!(
        larger_hits > smaller_hits,
        "the larger store returned no more hits ({larger_hits} against \
         {smaller_hits}), so the two runs are not different enough to compare"
    );
    assert_eq!(
        smaller_statements, larger_statements,
        "a search over {larger_hits} hits issued {larger_statements} \
         statements where {smaller_hits} hits issued {smaller_statements}. \
         Past the excerpt cap a search reads nothing further, so this number \
         must not move — one that does is work growing with the store."
    );
}

#[test]
fn excerpts_are_cut_for_a_bounded_number_of_hits() {
    // `SNIPPET_HITS` is 50 out of `HIT_LIMIT`'s 200, and each excerpt costs a
    // body read. A regression that cut one per *hit* would be four times the
    // reads, invisible in a small fixture and invisible in the results —
    // every hit still comes back correct, just slower.
    //
    // Counted as the difference between a store with fewer matches than the
    // cap and one with more: past the cap the statement count must stop
    // rising, because the excerpts stop being cut.
    let under = indexed(20);
    let over = indexed(200);

    let (under_statements, under_hits) = cost_of_searching(&under);
    let (over_statements, over_hits) = cost_of_searching(&over);

    assert!(
        under_hits < 50 && over_hits > 50,
        "this needs one store under the excerpt cap and one over it; got \
         {under_hits} and {over_hits}"
    );
    // The two differ by exactly the excerpts between `under_hits` and the
    // cap, and nothing else: one body read each, and the rest of a search is
    // the same fixed work either way. Asserted as the exact figure rather
    // than as a ceiling, because a ceiling of 50 would still pass if the cap
    // moved to 49 or if the overhead grew by ten — and the number is derived
    // here rather than written down, so it stays true if `SNIPPET_HITS`
    // changes.
    let growth = over_statements.saturating_sub(under_statements);
    let expected = 50 - under_hits;
    assert_eq!(
        growth, expected,
        "{over_hits} hits cost {growth} more statements than {under_hits} did, \
         where the {expected} excerpts between {under_hits} and the cap of 50 \
         account for exactly {expected}. Past the cap no further excerpt is \
         cut, so anything else is a body read that should not be happening."
    );
}
