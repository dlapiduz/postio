//! A word plus a date filter is driven by the word's hits (#1809).
//!
//! The driven form (`HITS_JOIN` in `executor.rs`) walks the full-text hits
//! and looks each one's messages up. Before content identity (#1805) that
//! lookup was `m.id = hits.rid`, a rowid seek the planner could not decline.
//! Keyed by content it became `m.content_id = hits.rid`, an index seek on
//! `idx_messages_content` -- and a `WHERE` that also says `m.received_at >= ?`
//! offered the planner `idx_messages_account_list (account_id, received_at)`
//! instead: two bound columns against one, so it took that, and walked every
//! message in the date range *once per hit*. `quarterly after:2025-01-01` went
//! from under a millisecond to 1.2 s on 20k messages, with the same answer.
//!
//! The plan does not depend on the data (this store never runs `ANALYZE`;
//! see `docs/gotchas.md`), so a handful of messages asks the planner the
//! same question 20k would. What is asserted is the shape, named: each
//! statement that joins the hits seeks its messages by `content_id` and
//! nothing else, and nothing in it is a full scan of a table that grows with
//! the mailbox -- `docs/notes/2026-09-21-a-scan-wearing-a-seeks-clothes.md`
//! is why a gate that only greps for `SCAN` is not enough.

use chrono::{TimeZone, Utc};
use postio_index::executor::facets;
use postio_index::{SearchRequest, index, search};
use postio_model::{AccountScope, EmailAddress, Message};
use postio_search::{ResultOrder, facets::Scope, parse};
use postio_storage::repository::MessageRepository;
use postio_storage::test_support;
use postio_storage::test_support::counting;

/// The steps a driven statement may scan: the hits it is driven by, the
/// subqueries feeding them, constants, and the facet statement's own CTEs,
/// which hold at most `TOTAL_HITS_CAP` rows. Anything else is a table walk.
fn is_a_driven_scan(step: &str) -> bool {
    matches!(
        step,
        "SCAN hits" | "SCAN CONSTANT ROW" | "SCAN capped AS c" | "SCAN matched"
    ) || step.starts_with("SCAN (subquery-")
}

#[tokio::test]
async fn a_word_with_a_date_filter_seeks_each_hit_by_its_content() {
    let store = test_support::memory().await;
    let connection = store.connect().await.expect("checkout");
    index::ensure_schema(&connection).await.expect("schema");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let messages = MessageRepository::new(&connection);
    for (nth, month) in [1, 3, 5, 7].into_iter().enumerate() {
        let received = Utc.with_ymd_and_hms(2025, month, 10, 9, 0, 0).unwrap();
        let mut message = Message::new(account.id, inbox, received);
        message.from = vec![EmailAddress::new(Some("Ada"), "ada@example.com")];
        message.subject = Some(format!("quarterly figures {nth}"));
        messages.create(&mut message).await.expect("create message");
        index::index_body(&connection, message.id.get(), Some("the quarterly report"))
            .await
            .expect("index body");
    }
    let today = chrono::NaiveDate::from_ymd_opt(2025, 9, 1).unwrap();
    let now = Utc.with_ymd_and_hms(2025, 9, 1, 12, 0, 0).unwrap();

    for text in [
        "quarterly after:2025-04-01",
        "quarterly before:2025-04-01",
        "quarterly after:2025-02-01 before:2025-06-01",
    ] {
        let query = parse(text, today);
        for order in [ResultOrder::Relevance, ResultOrder::Newest] {
            let request = SearchRequest {
                account: AccountScope::Account(account.id),
                query: &query,
                scope: Scope::AllMail,
                limit: 25,
                order,
            };
            counting::record();
            let results = search(&connection, &request, now).await.expect("search");
            facets(&connection, &request).await.expect("facets");
            let statements = counting::recorded();
            assert!(
                !results.hits.is_empty(),
                "{text}: the fixture is meant to answer this"
            );

            let driven: Vec<&String> = statements
                .keys()
                .filter(|sql| sql.contains("hits.rid"))
                .collect();
            for sql in &driven {
                let steps = counting::plan_steps(&connection, sql)
                    .await
                    .unwrap_or_else(|error| panic!("cannot plan {sql}: {error}"));
                let walks: Vec<String> = counting::scans(&connection, sql)
                    .await
                    .into_iter()
                    .filter(|step| !is_a_driven_scan(step))
                    .collect();
                assert!(
                    walks.is_empty(),
                    "{text}, {order:?}: a statement driven by the hits scans \
                     {walks:?}.\n{sql}\n{steps:#?}"
                );
                let lookups: Vec<&String> = steps
                    .iter()
                    .filter(|step| step.starts_with("SEARCH m "))
                    .collect();
                assert!(
                    !lookups.is_empty()
                        && lookups.iter().all(|step| step.as_str()
                            == "SEARCH m USING INDEX idx_messages_content (content_id=?)"),
                    "{text}, {order:?}: each hit must be looked up by its content, \
                     `idx_messages_content (content_id=?)`; the plan seeks it \
                     with {lookups:?} -- a range walked once per hit (#1809).\n\
                     {sql}\n{steps:#?}"
                );
                let siblings: Vec<&String> = steps
                    .iter()
                    .filter(|step| step.starts_with("SEARCH earlier "))
                    .collect();
                assert!(
                    siblings.iter().all(|step| step.as_str()
                        == "SEARCH earlier USING INDEX idx_messages_content \
                            (content_id=? AND id<?)"),
                    "{text}, {order:?}: the sibling check must seek the content's \
                     other memberships, not a range: {siblings:?}\n{sql}"
                );
            }
            for start in ["SELECT count(*)", "SELECT m.id", "WITH capped"] {
                assert!(
                    driven.iter().any(|sql| sql.trim_start().starts_with(start)),
                    "{text}, {order:?}: no recorded statement joining the hits \
                     starts `{start}`, so this gate cannot see that statement's \
                     plan. Recorded: {:#?}",
                    statements.keys().collect::<Vec<_>>()
                );
            }
        }
    }
}
