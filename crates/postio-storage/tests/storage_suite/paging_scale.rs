//! Keyset paging stays flat over a hundred thousand messages, by the clock.
//!
//! POSTIO-MEASUREMENT: it seeds 100,000 rows and walks 1,900 pages to time
//! one at the far end, 91 s on a CI runner, so it runs on the nightly timer.
//! `messages::a_cursor_page_seeks_past_the_cursor_instead_of_filtering_down_to_it` asks the planner the same question in a tenth of a
//! second on every pull request, and is the one that says why.

use std::time::{Duration, Instant};

use postio_storage::repository::{ListQuery, MessageRepository};
use postio_storage::test_support;

use crate::messages::seed;

#[tokio::test]
async fn paging_stays_flat_over_a_hundred_thousand_messages() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (_account, inbox) = test_support::account_with_inbox(&connection).await;
    seed(&connection, inbox, 100_000).await;
    let messages = MessageRepository::new(&connection);

    let query = ListQuery::mailbox(inbox).limit(50);
    let time = async |query: &ListQuery| -> (Duration, Vec<_>) {
        let start = Instant::now();
        let page = messages.page(query).await.expect("page");
        (start.elapsed(), page)
    };

    let (first_duration, first) = time(&query).await;
    assert_eq!(first.len(), 50, "a page is a window, never the mailbox");

    // Walk to the far end of the mailbox and time a page there.
    let mut cursor = first.last().expect("a row").cursor();
    let mut pages = 1;
    while pages < 1_900 {
        let page = messages
            .page(&query.clone().after(cursor))
            .await
            .expect("page");
        let Some(last) = page.last() else { break };
        cursor = last.cursor();
        pages += 1;
    }

    let (deep_duration, deep) = time(&query.clone().after(cursor)).await;
    assert_eq!(deep.len(), 50, "still a full window, 95000 rows in");
    assert!(
        deep_duration < first_duration * 5 + Duration::from_millis(3),
        "keyset paging is a seek, not a skip: first {first_duration:?}, deep {deep_duration:?}"
    );

    // And the whole mailbox is never materialized: the only way to see every
    // row is to ask for one window at a time.
    assert_eq!(
        messages
            .count(&ListQuery::mailbox(inbox))
            .await
            .expect("count"),
        100_000
    );
}
