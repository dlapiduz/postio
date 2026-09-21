//! Threading a message costs the same whatever the account holds.
//!
//! `commit_batch` batches its upsert and then threads **one message at a
//! time** — `for message in &written { threading.thread(message).await? }` —
//! so every statement `thread` issues is paid per message, per batch, for the
//! whole of a first sync. A lookup that scans instead of seeking is therefore
//! not a constant factor: it is work proportional to everything synced so
//! far, charged against every message synced next.
//!
//! That is the shape a real account showed. Header fetches cost 4–610 ms per
//! batch while the commits behind them cost 8.7–31.5 s — 49 to 157 ms per
//! message, against the 0.88–2.80 ms #78 measured and #726 confirmed flat
//! from an empty store to 131 MB. Those numbers predate the storage engine
//! changing, and a plan that seeks on one engine does not necessarily seek on
//! another: `docs/notes/` already records this engine declining to seek a
//! row-value cursor that SQLite seeks happily.
//!
//! So this asks the planner directly, which is the instrument
//! [`postio_storage::test_support::counting`] exists to provide: counts and
//! plans are the same on any machine, where a stopwatch on a desktop running
//! four builds is not.

use postio_storage::test_support;
use postio_storage::test_support::counting::scans;

/// The lookup `ThreadingRepository::thread_of` makes, once per message.
///
/// Written out rather than reached through the repository because the point
/// is the SQL the planner sees, and `scans` takes a statement.
const THREAD_OF: &str = "SELECT thread_id FROM thread_links \
                          WHERE account_id = ?1 AND rfc_message_id = ?2 COLLATE NOCASE";

#[tokio::test]
async fn threading_a_message_does_not_scan_the_links_table() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("a connection");

    let scanned = scans(&connection, THREAD_OF).await;

    assert!(
        scanned.is_empty(),
        "threading's per-message lookup plans as a scan of {scanned:?}. \
         `idx_thread_links_lookup` covers (account_id, rfc_message_id COLLATE \
         NOCASE) precisely so this seeks; if the planner will not use it, then \
         every message threaded costs a walk of every link the account has \
         already made — which is charged once per message inside \
         `commit_batch`, for the whole of a first sync."
    );
}

/// The control: a query that *must* scan, so a passing assertion above is
/// evidence rather than an instrument that quietly returned nothing.
///
/// `scans` ends in `unwrap_or_default()`, so an `EXPLAIN QUERY PLAN` this
/// engine refuses would read as "no scans" and every budget written with it
/// would pass for ever.
#[tokio::test]
async fn the_planner_instrument_can_still_see_a_scan() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("a connection");

    let scanned = scans(
        &connection,
        "SELECT count(*) FROM thread_links WHERE rfc_message_id LIKE 'x%'",
    )
    .await;

    assert!(
        !scanned.is_empty(),
        "the planner reported no scan for a query that has no index to use, \
         so `scans` is answering nothing and every assertion written with it \
         is vacuous"
    );
}
