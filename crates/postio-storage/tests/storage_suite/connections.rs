use postio_storage::test_support::{self, counting};
use postio_storage::{Connection, sql};

async fn cache_size(connection: &Connection) -> i64 {
    sql::one(connection, "PRAGMA cache_size", (), |row| {
        sql::RowExt::col(row, 0)
    })
    .await
    .expect("the pragma answers")
}

#[tokio::test]
async fn a_background_connection_carries_a_small_cache() {
    // Lanes, body fetches and the indexer write across the whole file, so their caches saturate at
    // the cap with pages nothing rereads.
    let store = test_support::memory().await;
    let interactive = store.connect().await.expect("a connection");
    let background = store
        .connect_background()
        .await
        .expect("a background connection");
    assert_eq!(cache_size(&interactive).await, -65536);
    assert_eq!(cache_size(&background).await, -4096);
}

#[tokio::test]
async fn reads_take_turns_on_a_few_long_lived_connections() {
    // The checkouts are the process's: alone, or a neighbour's are counted too.
    if postio_test_support::alone!("reads_take_turns_on_a_few_long_lived_connections") {
        return;
    }
    let store = test_support::memory().await;
    let before = counting::checkouts();
    for _ in 0..5 {
        let reader = store.read().await.expect("a reader");
        let answered: i64 = sql::one(&reader, "SELECT 1", (), |row| sql::RowExt::col(row, 0))
            .await
            .expect("a read");
        assert_eq!(answered, 1);
    }
    let opened = counting::checkouts() - before;
    assert_eq!(
        opened, 1,
        "five reads taken in turn opened {opened} connections"
    );
}
