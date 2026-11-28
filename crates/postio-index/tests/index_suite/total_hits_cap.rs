//! POSTIO-MEASUREMENT: costs 208 s, since the cap is only reachable by inserting `TOTAL_HITS_CAP +
//! 50` messages one at a time, so it runs on the nightly timer (#1450).
//! `.config/nextest.toml`'s `profile.default` filter holds it back.

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
        mailboxes: None,
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
