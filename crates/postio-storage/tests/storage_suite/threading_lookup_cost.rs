//! Threading costs the same whatever the account holds: `commit_batch` threads one message at a
//! time, so a scanning lookup is charged to every message synced after it. The plan must bind the
//! key's columns, not merely lack `SCAN`: a one-column `SEARCH` through a `COLLATE NOCASE` index is
//! a scan (#1587).

use postio_storage::test_support;
use postio_storage::test_support::counting::scans;

/// Written out because the point is the SQL the planner sees: binary equality on the folded key,
/// since `COLLATE NOCASE` forced a collated index this planner will not bind through (#1587).
const THREAD_OF: &str = "SELECT thread_id FROM thread_links \
                          WHERE account_id = ?1 AND rfc_message_id = ?2";

const PER_MESSAGE_LOOKUPS: &[(&str, &str, &[&str])] = &[
    (
        "thread_of",
        THREAD_OF,
        &["account_id=?", "rfc_message_id=?"],
    ),
    (
        // `IN` on the second column gets a one-column prefix from this planner; a `UNION ALL` arm
        // gets the full key.
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
        "address and its owner",
        "SELECT a.id, a.contact_id, c.name, c.seen_name, c.last_seen_at \
           FROM addresses a LEFT JOIN contacts c ON c.id = a.contact_id \
          WHERE a.address_normalized = ?1",
        &["address_normalized=?", "rowid=?"],
    ),
    (
        "sighting by address and account",
        "SELECT times_seen FROM contact_sightings \
          WHERE address_id = ?1 AND account_id = ?2",
        &["address_id=?", "account_id=?"],
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

/// The control: a query that must scan, since `scans` ends in `unwrap_or_default()` and a refused
/// `EXPLAIN` would read as no scans for ever.
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
