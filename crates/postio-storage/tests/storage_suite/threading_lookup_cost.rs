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
//!
//! **And it asks for the bound columns, not the absence of `SCAN`** — the
//! upgrade #1587 forced. The first version of this gate checked
//! `scans().is_empty()` and passed for weeks while every message threaded
//! walked the account's whole link table, because Turso's planner would not
//! bind an equality through the index's `COLLATE NOCASE` column and answered
//! with a one-column `SEARCH ... (account_id=?)` — a scan wearing a seek's
//! clothes, invisible to a gate that only greps for `SCAN`. A lookup whose
//! key has two columns is only a lookup if the plan binds both.

use postio_storage::test_support;
use postio_storage::test_support::counting::scans;

/// The lookup `ThreadingRepository::thread_of` makes, once per message.
///
/// Written out rather than reached through the repository because the point
/// is the SQL the planner sees. Binary equality against the folded key —
/// `COLLATE NOCASE` is gone from the query because it forced a collated
/// index, and this planner will not bind through one (#1587).
const THREAD_OF: &str = "SELECT thread_id FROM thread_links \
                          WHERE account_id = ?1 AND rfc_message_id = ?2";

/// The statements `commit_batch` pays per message, each with the columns its
/// plan must bind. One entry per hot lookup, so the next planner surprise
/// fails here with the statement's name on it.
const PER_MESSAGE_LOOKUPS: &[(&str, &str, &[&str])] = &[
    (
        "thread_of",
        THREAD_OF,
        &["account_id=?", "rfc_message_id=?"],
    ),
    (
        // The cue lookup's compound shape. `IN` on the index's second column
        // gets a one-column prefix from this planner; a `UNION ALL` arm gets
        // the full key. One statement either way — the statement-count gate
        // next door holds that line.
        "thread links by cue",
        "SELECT rfc_message_id, thread_id FROM thread_links \
         WHERE account_id = ?1 AND rfc_message_id = ?2 \
         UNION ALL \
         SELECT rfc_message_id, thread_id FROM thread_links \
         WHERE account_id = ?1 AND rfc_message_id = ?3",
        &["account_id=?", "rfc_message_id=?"],
    ),
    (
        "thread by subject",
        "SELECT id FROM threads WHERE account_id = ?1 AND subject = ?2 ORDER BY id",
        &["account_id=?", "subject=?"],
    ),
    (
        "message by uid",
        "SELECT id FROM messages \
         WHERE mailbox_id = ?1 AND uid_validity = ?2 AND uid = ?3",
        &["mailbox_id=?", "uid_validity=?", "uid=?"],
    ),
    (
        "contact by address",
        "SELECT id FROM contacts WHERE account_id = ?1 AND address_normalized = ?2",
        &["account_id=?", "address_normalized=?"],
    ),
];

#[tokio::test]
async fn every_per_message_lookup_binds_its_whole_key() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("a connection");

    for (name, sql, bound) in PER_MESSAGE_LOOKUPS {
        let plan = test_support::plan(&connection, sql).await;
        for column in *bound {
            assert!(
                plan.contains(column),
                "{name}: the plan does not bind {column}, so this lookup \
                 walks rather than seeks — a per-message cost that grows \
                 with the store, which is #1587's whole disease.\n{plan}"
            );
        }
    }
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
