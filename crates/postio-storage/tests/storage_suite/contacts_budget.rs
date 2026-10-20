//! What contacts cost the store — counted, not timed (Principle V).
//!
//! Three paths, each with the budget that governs it:
//!
//! - **Recording**, paid per correspondent on every message a sync writes
//!   (#728). The counter sees *reads* — the seam counts every `sql::first`,
//!   `all` and `one`, and an `execute` goes uncounted — so this holds the
//!   read half exactly: one lookup per known correspondent, which finds the
//!   address and its owner together. The writes behind it (the sighting, the
//!   person's aggregates) are two literal statements the code states in
//!   `record_in`'s doc comment, and `threading_lookup_cost.rs` holds that each
//!   seeks rather than walks.
//! - **Completion**, which runs on the UI thread as the user types a
//!   recipient: the 16 ms budget, so a fixed number of statements and no full
//!   scan whatever the address book holds.
//! - **The finder's load**, which holds people in memory for fuzzy matching:
//!   bounded by its cap, never by the size of the address book.

use chrono::{TimeZone, Utc};
use postio_model::{EmailAddress, Message};
use postio_storage::Connection;
use postio_storage::repository::ContactRepository;
use postio_storage::test_support;
use postio_storage::test_support::counting::{counted_async, scans};

/// Enough people that a scan or a sort over all of them is a real cost.
const PEOPLE: usize = 20_000;

/// An address book the size of a real one: a person per address, each with
/// the terms recording would have given them.
async fn fill(connection: &Connection) {
    connection
        .execute_batch(&format!(
            "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < {PEOPLE})
             INSERT INTO addresses (address, address_normalized)
             SELECT 'p' || i || '@example.com', 'p' || i || '@example.com' FROM n;
             WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < {PEOPLE})
             INSERT INTO contacts (name, sort_key, name_key, source, state, preferred_address,
                                   times_seen, last_seen_at, written, created_at, updated_at)
             SELECT 'Person ' || i, 'person ' || i, 'person ' || i,
                    CASE WHEN i % 3 = 0 THEN 'user' ELSE 'mail' END, 'live', i,
                    i % 50, 1700000000 + i, i % 2, 0, 0
               FROM n;
             UPDATE addresses SET contact_id = id;
             WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < {PEOPLE})
             INSERT INTO contact_terms (term, contact_id)
             SELECT 'person', i FROM n UNION ALL SELECT 'p' || i, i FROM n;"
        ))
        .await
        .expect("fill the address book");
}

fn message(account: &postio_model::Account, inbox: postio_model::MailboxId) -> Message {
    let mut message = Message::new(
        account.id,
        inbox,
        Utc.with_ymd_and_hms(2026, 3, 1, 0, 0, 0).unwrap(),
    );
    message.from = vec![EmailAddress::new(Some("Ada"), "ada@example.com")];
    message.to = vec![
        EmailAddress::new(Some("Grace"), "grace@example.org"),
        EmailAddress::new(Some("Quinn"), "quinn@example.net"),
    ];
    message.cc = vec![
        EmailAddress::new(Some("Katherine"), "katherine@example.com"),
        EmailAddress::new(Some("Alan"), "alan@example.org"),
    ];
    message
}

#[tokio::test]
async fn recording_a_known_correspondent_costs_one_read() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("a connection");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let contacts = ContactRepository::new(&connection);
    let message = message(&account, inbox);
    let addresses = 5;

    let first = counted_async(|| async {
        contacts
            .record_message(&message, &[])
            .await
            .expect("first sighting");
    })
    .await;
    let again = counted_async(|| async {
        contacts
            .record_message(&message, &[])
            .await
            .expect("second sighting");
    })
    .await;

    // This message was never stored, so each address row is made here and
    // read back: two reads an address. Sync never pays that -- it writes a
    // message's recipients before recording it -- and the second pass below
    // is the shape it does pay.
    assert_eq!(
        first.statements,
        2 * addresses,
        "a first sighting of {addresses} unstored addresses"
    );
    assert_eq!(
        again.statements, addresses,
        "recording {addresses} known correspondents read {} times; the budget \
         is one read each -- the address and its owner in one lookup -- and \
         this runs for every address on every message a first sync writes \
         (#728)",
        again.statements
    );
    assert_eq!(again.rows, addresses, "one row per lookup, nothing wider");
}

/// The completion read, as `ContactRepository::complete` spells it with a
/// prefix. Copied, like `contact_rank_index.rs` copies its twin, because the
/// SQL the planner sees is the thing being asked about.
const COMPLETE: &str = "SELECT id, name, organization, note, source, state, \
     preferred_address, seen_name, times_seen, last_seen_at, written FROM contacts \
     WHERE state = 'live' \
       AND id IN (SELECT contact_id FROM contact_terms WHERE term >= ?1 AND term < ?2) \
     ORDER BY CASE WHEN source = 'mail' THEN 1 ELSE 0 END, \
              last_seen_at DESC, times_seen DESC, id LIMIT ?3";

#[tokio::test]
async fn completion_is_two_statements_and_no_scan_however_large_the_address_book() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("a connection");
    fill(&connection).await;
    let contacts = ContactRepository::new(&connection);
    let _ = contacts.complete("pers", 8).await.expect("warm");

    let mut offered = 0;
    let broad = counted_async(|| async {
        offered = contacts.complete("pers", 8).await.expect("complete").len();
    })
    .await;
    let narrow = counted_async(|| async {
        contacts.complete("p1234", 8).await.expect("complete");
    })
    .await;
    let empty = counted_async(|| async {
        contacts.complete("", 8).await.expect("complete");
    })
    .await;

    assert_eq!(offered, 8, "the limit bounds the people");
    for (name, counts) in [("broad", broad), ("narrow", narrow), ("empty", empty)] {
        assert_eq!(
            counts.statements, 2,
            "{name} completion: the people, then their addresses"
        );
        assert_eq!(
            counts.rows,
            8 + 8,
            "{name} completion: eight people with one address each, and not a \
             row more whatever the address book holds"
        );
    }
    let scanned = scans(&connection, COMPLETE).await;
    assert!(
        scanned.is_empty(),
        "completion scans {scanned:?} on every keystroke of a recipient"
    );
}

#[tokio::test]
async fn the_finders_load_is_one_statement_bounded_by_its_cap() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("a connection");
    fill(&connection).await;
    let contacts = ContactRepository::new(&connection);

    let mut held = 0;
    let counts = counted_async(|| async {
        held = contacts.people(500).await.expect("people").len();
    })
    .await;

    assert_eq!(held, 500, "the cap bounds what the finder holds");
    assert_eq!(counts.statements, 1);
    assert_eq!(
        counts.rows, 500,
        "one row per address, and the cap is the bound"
    );
}
