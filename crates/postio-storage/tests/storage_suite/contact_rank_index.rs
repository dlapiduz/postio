//! Contact autocomplete is served by an index, not by sorting the address
//! book (#990).
//!
//! `idx_contacts_rank` was written for the ordering ADR 0007 Q6 originally
//! described, `(times_seen DESC, last_seen_at DESC)`. #424 changed the
//! ordering to recency-first and #430 corrected the ADR; the index was not
//! part of either, so it named an order nothing asked for.
//!
//! # Why swapping its columns is not the fix
//!
//! `ContactRepository::search` orders by
//!
//! ```text
//! CASE WHEN source = 'mail' THEN 1 ELSE 0 END, last_seen_at DESC, times_seen DESC, id
//! ```
//!
//! — a **band** first (contacts the user created outrank ones harvested from
//! headers), then recency, then frequency. SQLite can satisfy an `ORDER BY`
//! from an index only when the index's leading columns match it exactly, and
//! the leading term here is an expression. Measured on 20,000 contacts, an
//! index of `(last_seen_at DESC, times_seen DESC)` — the swap the issue
//! proposed — still plans as `SCAN contacts` + `USE TEMP B-TREE FOR ORDER BY`.
//! Only an index that leads with the same expression removes the sort.
//!
//! # Why the sort is worth removing
//!
//! Autocomplete runs on every keystroke of a recipient, and the popup opens
//! on the *empty* prefix — the case where nothing narrows the candidate set
//! and every contact is a candidate. A temporary b-tree over the whole
//! address book, per keystroke, is what the 16 ms interaction budget cannot
//! hold. `LIMIT 20` costs nothing when the index is walked in order and
//! everything when it is not: the sort has to see every row before it can
//! know which twenty come first.
//!
//! Two things are asserted together, for the reason `draft_indexes.rs` gives:
//! an index the planner declines to use still returns the right rows, by
//! scanning, so neither the shape nor the results alone can fail usefully.

use rusqlite::Connection;

use postio_storage::migrate;

/// Enough contacts that a sort over all of them is a real cost, and enough
/// that SQLite would not simply scan a tiny table whatever the index says.
const CONTACTS: usize = 20_000;

fn migrated() -> Connection {
    let mut connection = Connection::open_in_memory().expect("in-memory sqlite");
    connection
        .pragma_update(None, "foreign_keys", true)
        .expect("foreign keys");
    migrate(&mut connection).expect("migrate");
    connection
}

fn definition(connection: &Connection, index: &str) -> String {
    connection
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type = 'index' AND name = ?1",
            [index],
            |row| row.get::<_, String>(0),
        )
        .unwrap_or_else(|error| panic!("no index named {index}: {error}"))
}

fn plan(connection: &Connection, query: &str) -> String {
    let mut statement = connection
        .prepare(&format!("EXPLAIN QUERY PLAN {query}"))
        .expect("a query plan");
    let rows = statement
        .query_map([], |row| row.get::<_, String>(3))
        .expect("plan rows")
        .collect::<Result<Vec<_>, _>>()
        .expect("plan rows");
    rows.join("\n")
}

/// An address book the size of a real one.
fn fill(connection: &Connection) {
    connection
        .execute_batch(&format!(
            "INSERT INTO contacts
                 (address, address_normalized, name, times_seen, last_seen_at, source)
             WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < {CONTACTS})
             SELECT 'p' || i || '@example.com', 'p' || i || '@example.com',
                    'Person ' || i, i % 50, 1700000000 + i,
                    CASE WHEN i % 3 = 0 THEN 'user' ELSE 'mail' END
               FROM n;"
        ))
        .expect("fill the address book");
}

/// The `ORDER BY` `ContactRepository::search` issues, verbatim. Kept here
/// rather than imported because it is what this test is *about*: if the
/// repository's ordering changes again, this has to be updated in step, and
/// a copy that must be kept in step is exactly what makes that visible.
const SEARCH: &str = "SELECT id FROM contacts \
     WHERE suppressed = 0 AND ('' = '' OR address_normalized LIKE '' || '%') \
     ORDER BY CASE WHEN source = 'mail' THEN 1 ELSE 0 END, \
              last_seen_at DESC, times_seen DESC, id \
     LIMIT 20";

#[test]
fn autocomplete_is_answered_from_the_index_rather_than_by_sorting_everyone() {
    let connection = migrated();
    fill(&connection);

    let plan = plan(&connection, SEARCH);
    assert!(
        !plan.contains("TEMP B-TREE"),
        "contact autocomplete sorts the whole address book on every \
         keystroke. `LIMIT 20` cannot help: the sort has to see every row \
         before it knows which twenty come first.\n  plan: {plan}\n  index: {}",
        definition(&connection, "idx_contacts_rank")
    );
    assert!(
        plan.contains("idx_contacts_rank"),
        "the sort is gone but not because of this index, which is the one \
         that is supposed to serve it.\n  plan: {plan}"
    );
}

#[test]
fn the_index_leads_with_the_band_that_the_ordering_leads_with() {
    // The shape half. It is asserted separately from the plan because the
    // plan can be right for the wrong reason -- a future SQLite that
    // materialises differently, a table small enough to scan -- and because
    // this is the sentence that says *why* the index looks unusual.
    let connection = migrated();
    let sql = definition(&connection, "idx_contacts_rank");
    assert!(
        sql.contains("source") && sql.contains("last_seen_at") && sql.contains("times_seen"),
        "the index has to name every term of the ordering it serves: {sql}"
    );
    assert!(
        sql.find("source").unwrap() < sql.find("last_seen_at").unwrap(),
        "the band leads the ordering, so it has to lead the index -- an index \
         that starts at `last_seen_at` cannot serve an ORDER BY that starts \
         somewhere else: {sql}"
    );
    assert!(
        sql.find("last_seen_at").unwrap() < sql.find("times_seen").unwrap(),
        "recency leads frequency since #424, and #430 corrected the ADR to \
         say so: {sql}"
    );
}

#[test]
fn the_rows_still_come_back_in_the_order_the_product_promises() {
    // The results half. An index nothing uses still returns the right rows,
    // and an index the planner *does* use can return the wrong ones -- so
    // this asserts the answer rather than the plan: a user-created contact
    // outranks a harvested one, and within a band the more recent wins.
    let connection = migrated();
    fill(&connection);

    let mut statement = connection.prepare(SEARCH).expect("prepare");
    let ids: Vec<i64> = statement
        .query_map([], |row| row.get(0))
        .expect("query")
        .collect::<Result<_, _>>()
        .expect("rows");

    assert_eq!(ids.len(), 20);
    let source_of = |id: i64| -> String {
        connection
            .query_row("SELECT source FROM contacts WHERE id = ?1", [id], |row| {
                row.get(0)
            })
            .expect("a contact")
    };
    let last_seen_of = |id: i64| -> i64 {
        connection
            .query_row(
                "SELECT last_seen_at FROM contacts WHERE id = ?1",
                [id],
                |row| row.get(0),
            )
            .expect("a contact")
    };

    assert!(
        ids.iter().all(|id| source_of(*id) == "user"),
        "every one of the first twenty should be a contact the user created: \
         the harvested band sorts after it however recent it is"
    );
    let times: Vec<i64> = ids.iter().map(|id| last_seen_of(*id)).collect();
    let mut descending = times.clone();
    descending.sort_by(|a, b| b.cmp(a));
    assert_eq!(
        times, descending,
        "within a band, the most recent comes first"
    );
}
