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

/// The list read for one view's first page, as `ContactRepository::page`
/// spells it -- the view's predicate, the own-address exclusion, the seek.
async fn list_plans(connection: &Connection) -> Vec<(String, Vec<String>)> {
    let mut plans = Vec::new();
    for (name, sql) in postio_storage::test_support::contact_list_statements() {
        plans.push((name.to_owned(), scans(connection, &sql).await));
    }
    plans
}

#[tokio::test]
async fn a_contacts_page_is_one_statement_and_no_more_rows_than_it_shows() {
    use postio_model::ContactView;
    use postio_storage::repository::ContactCursor;

    let database = test_support::memory().await;
    let connection = database.connect().await.expect("a connection");
    let _account = test_support::account(&connection).await;
    fill(&connection).await;
    let contacts = ContactRepository::new(&connection);
    let _ = contacts
        .page(ContactView::Written, None, 50)
        .await
        .expect("warm");

    for view in [
        ContactView::Written,
        ContactView::Everyone,
        ContactView::Deleted,
    ] {
        let mut shown = Vec::new();
        let first = counted_async(|| async {
            shown = contacts.page(view, None, 50).await.expect("page");
        })
        .await;
        assert_eq!(first.statements, 1, "{view:?}: one statement a page");
        assert!(
            first.rows <= 50,
            "{view:?}: {} rows for a page of 50",
            first.rows
        );

        if let Some(last) = shown.last() {
            let cursor = ContactCursor::after(last);
            let deep = counted_async(|| async {
                contacts
                    .page(view, Some(&cursor), 50)
                    .await
                    .expect("next page");
            })
            .await;
            assert_eq!(
                deep.statements, 1,
                "{view:?}: the next page is one statement too"
            );
            assert!(deep.rows <= 50);
        }
    }

    let filtered = counted_async(|| async {
        contacts
            .filtered(ContactView::Everyone, "pers", 500)
            .await
            .expect("filter");
    })
    .await;
    assert_eq!(filtered.statements, 1, "a filtered page is one statement");
    assert!(
        filtered.rows <= 500,
        "the cap bounds a filter: {} rows",
        filtered.rows
    );

    // The planner's half. `accounts` and `identities` are the user's own
    // addresses -- a handful of rows, read once per statement to keep them
    // out of the list -- and scanning those is expected; scanning people is
    // not.
    for (name, scanned) in list_plans(&connection).await {
        let people: Vec<&String> = scanned
            .iter()
            .filter(|step| !step.contains("accounts") && !step.contains("identities"))
            .collect();
        assert!(
            people.is_empty(),
            "{name} scans {people:?}: a keystroke or a scroll would walk everyone"
        );
    }
}

#[tokio::test]
async fn a_cursor_page_seeks_past_the_cursor_instead_of_filtering_down_to_it() {
    // docs/notes/2026-09-12-a-row-value-cursor-is-a-filter-not-a-seek.md:
    // this engine seeks on a bare inequality on the sort column and filters
    // on a row value, so the cursor carries the redundant `sort_key >= ?`.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("a connection");
    let cursors: Vec<(&str, String)> = postio_storage::test_support::contact_list_statements()
        .into_iter()
        .filter(|(name, _)| name.ends_with("after"))
        .collect();
    assert_eq!(
        cursors.len(),
        3,
        "one cursor read per view; an empty list here would make this test vacuous"
    );
    for (name, sql) in cursors {
        let plan = test_support::plan(&connection, &sql).await;
        assert!(
            plan.contains("sort_key>"),
            "{name} does not seek on sort_key; every page walks from the top:\n{plan}"
        );
    }
}

#[tokio::test]
async fn a_persons_detail_is_four_statements_whatever_the_address_book_holds() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("a connection");
    fill(&connection).await;
    let contacts = ContactRepository::new(&connection);
    let id = postio_model::ContactId::new(1234);

    let mut messages = None;
    let counts = counted_async(|| async {
        messages = contacts
            .detail(id)
            .await
            .expect("detail")
            .map(|detail| detail.messages);
    })
    .await;

    assert_eq!(messages, Some(0), "person 1234 exists and has no mail");
    assert_eq!(
        counts.statements, 4,
        "the person, their addresses, their groups, the distinct-message count"
    );
    assert_eq!(
        counts.rows,
        1 + 1 + 1,
        "one person, one address, one count row"
    );
}
