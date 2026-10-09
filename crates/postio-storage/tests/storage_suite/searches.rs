//! What the search bar remembers (spec 010, D15 and D16): the last twenty
//! queries a person ran, and per saved search the newest message they have
//! seen its results up to.

use chrono::{DateTime, TimeDelta, TimeZone, Utc};
use postio_storage::searches::SearchRepository;
use postio_storage::test_support;

fn at(minutes: i64) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 1, 9, 0, 0).unwrap() + TimeDelta::minutes(minutes)
}

#[tokio::test]
async fn remembering_a_query_twice_keeps_one_row_with_the_latest_run() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let searches = SearchRepository::new(&connection);

    searches
        .remember("from:ada", 3, at(0))
        .await
        .expect("first");
    searches
        .remember("from:ada", 5, at(9))
        .await
        .expect("again");

    let recent = searches.recent().await.expect("recent");
    assert_eq!(recent.len(), 1);
    assert_eq!(recent[0].query, "from:ada");
    assert_eq!(recent[0].hits, 5);
    assert_eq!(recent[0].last_run_at, at(9));
}

#[tokio::test]
async fn recent_is_newest_first() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let searches = SearchRepository::new(&connection);

    searches.remember("one", 1, at(1)).await.unwrap();
    searches.remember("three", 1, at(3)).await.unwrap();
    searches.remember("two", 1, at(2)).await.unwrap();

    let queries: Vec<_> = searches
        .recent()
        .await
        .unwrap()
        .into_iter()
        .map(|search| search.query)
        .collect();
    assert_eq!(queries, ["three", "two", "one"]);
}

#[tokio::test]
async fn only_the_newest_twenty_are_kept() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let searches = SearchRepository::new(&connection);

    for n in 0..25 {
        searches
            .remember(&format!("query {n}"), 0, at(n))
            .await
            .unwrap();
    }

    let recent = searches.recent().await.unwrap();
    assert_eq!(recent.len(), 20);
    assert_eq!(recent[0].query, "query 24");
    assert_eq!(recent[19].query, "query 5");
}

#[tokio::test]
async fn forgetting_deletes_one_query_and_says_whether_it_was_there() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let searches = SearchRepository::new(&connection);
    searches.remember("keep", 1, at(1)).await.unwrap();
    searches.remember("drop", 1, at(2)).await.unwrap();

    assert!(searches.forget("drop").await.unwrap());
    assert!(!searches.forget("drop").await.unwrap());

    let queries: Vec<_> = searches
        .recent()
        .await
        .unwrap()
        .into_iter()
        .map(|search| search.query)
        .collect();
    assert_eq!(queries, ["keep"]);
}

#[tokio::test]
async fn a_saved_search_is_seen_up_to_what_was_marked() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let searches = SearchRepository::new(&connection);

    assert_eq!(searches.seen_up_to("vip").await.unwrap(), None);
    searches.mark_seen("vip", at(5)).await.unwrap();
    assert_eq!(searches.seen_up_to("vip").await.unwrap(), Some(at(5)));
    searches.mark_seen("vip", at(8)).await.unwrap();
    assert_eq!(searches.seen_up_to("vip").await.unwrap(), Some(at(8)));
    assert_eq!(searches.seen_up_to("other").await.unwrap(), None);
}

#[tokio::test]
async fn forget_seen_except_drops_the_rows_of_deleted_saved_searches() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let searches = SearchRepository::new(&connection);
    searches.mark_seen("kept", at(1)).await.unwrap();
    searches.mark_seen("gone", at(2)).await.unwrap();

    let removed = searches.forget_seen_except(&["kept"]).await.unwrap();

    assert_eq!(removed, 1);
    assert_eq!(searches.seen_up_to("kept").await.unwrap(), Some(at(1)));
    assert_eq!(searches.seen_up_to("gone").await.unwrap(), None);

    // No saved searches left: every row goes.
    assert_eq!(searches.forget_seen_except(&[]).await.unwrap(), 1);
    assert_eq!(searches.seen_up_to("kept").await.unwrap(), None);
}
