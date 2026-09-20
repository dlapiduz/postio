//! What `total_hits` reports once a query matches more than the cap.
//!
//! `TOTAL_HITS_CAP` exists so a query matching an entire archive does not pay
//! to count it: past the cap the executor stops counting and says so, and the
//! reading list shows "1,000+" rather than a number nobody waited for. This
//! is the test that the cap is real — that the count stops *and* reports
//! itself stopped.
//!
//! POSTIO-MEASUREMENT: it costs 208 s, which is more than the rest of this
//! crate's suite put together, so it runs on the nightly timer rather than the
//! merge path (#1450). Not because it measures anything — it is an ordinary
//! assertion — but because of what it has to build first: the cap is only
//! reachable by inserting `TOTAL_HITS_CAP + 50` messages one at a time, and
//! that bulk load is the whole cost. `.config/nextest.toml`'s
//! `profile.default` filter is what holds it back; run it with
//!
//! ```text
//! cargo nextest run --profile nightly -p postio-index -E 'test(/^total_hits_cap::/)'
//! ```
//!
//! The cheap half of the same contract stays on the merge path:
//! `executor::total_hits_counts_past_the_page` proves an uncapped total is
//! exact, and needs five messages to do it.

use postio_index::executor::{SearchRequest, search};
use postio_model::AccountScope;
use postio_search::facets::Scope;
use postio_search::parse;
use postio_storage::test_support;

use super::executor::{at, message};

#[tokio::test]
async fn total_hits_stops_counting_at_the_cap() {
    use postio_search::TOTAL_HITS_CAP;

    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    postio_index::index::ensure_schema(&connection)
        .await
        .expect("schema");
    let (account, mailbox) = test_support::account_with_inbox(&connection).await;

    connection
        .execute_batch("BEGIN")
        .await
        .expect("start bulk load transaction");
    for _ in 0..(TOTAL_HITS_CAP + 50) {
        message(&connection, &account, mailbox, "ada", "Bulk notes", at(0)).await;
    }
    connection
        .execute_batch("COMMIT")
        .await
        .expect("commit bulk load transaction");

    let query = parse("bulk", at(12).date_naive());
    let request = SearchRequest {
        account: AccountScope::Account(account.id),
        query: &query,
        scope: Scope::AllMail,
        limit: 5,
        order: postio_search::ResultOrder::Relevance,
    };
    let results = search(&connection, &request, at(12)).await.expect("search");

    assert!(results.total_hits_capped);
    assert_eq!(results.total_hits, TOTAL_HITS_CAP);
}
