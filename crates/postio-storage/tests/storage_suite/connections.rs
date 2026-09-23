//! One store, several connections: which page cache each carries, and how
//! many a run of reads opens (#1602).
//!
//! The engine keeps no pool: every `connect` is a fresh pager with an empty
//! page cache, capped by the `cache_size` pragma the store sets on it. So
//! the number of connections a surface opens is the number of caches it
//! fills from a cold file, and the cap on a connection that only writes
//! through is memory nobody reads.

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
    // Lanes, body fetches and the indexer write across the whole file, so
    // their caches saturate at the cap and hold clean pages nothing reads
    // again; five of them at 64 MiB was ~300 MB of a first sync's heap.
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
    // A read that opens its own connection pays a new pager, an empty cache
    // and the pragmas every time; a list page paid it twice per page. Reads
    // taken in turn share one warm connection.
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
