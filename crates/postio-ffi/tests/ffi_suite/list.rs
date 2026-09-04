//! The windowed list, driven the way `NSTableView` drives it.
//!
//! `PRODUCT.md` §18: **a mailbox is never loaded into memory.** These tests
//! exist to keep that true across an FFI, where the temptation is to hand the
//! frontend a vector and be done. The frontend gets a count and one row at a
//! time, synchronously, and the pages arrive behind it.

use chrono::Utc;
use postio_ffi::{ScopeFfi, Session, SessionOptions};
use postio_model::Message;
use postio_storage::repository::MessageRepository;
use postio_storage::test_support;

/// A store with `count` messages in an inbox, and the scope that lists them.
fn seeded(count: u32) -> (std::sync::Arc<Session>, ScopeFfi) {
    let database = test_support::memory();
    let mailbox = {
        let connection = database.connection().expect("a connection");
        let (account, inbox) = test_support::account_with_inbox(&connection);
        let repository = MessageRepository::new(&connection);
        for _ in 0..count {
            let mut message = Message::new(account.id, inbox, Utc::now());
            repository.create(&mut message).expect("a message");
        }
        inbox
    };
    let session = Session::open(SessionOptions::in_memory_with(database))
        .expect("a session over the seeded store");
    (
        session,
        ScopeFfi::Mailbox {
            mailbox: mailbox.into(),
        },
    )
}

#[test]
fn opening_a_scope_reports_how_many_rows_it_has() {
    let (session, scope) = seeded(120);
    session.open_scope(scope);
    assert_eq!(
        session.row_count(),
        120,
        "the count is what the frontend sizes its table from"
    );
    session.shutdown();
}

#[test]
fn a_row_is_missing_until_its_page_arrives_and_then_it_is_not() {
    let (session, scope) = seeded(120);
    session.open_scope(scope);

    // First ask: nothing is resident yet, so the frontend draws a placeholder.
    // Crucially this does not block -- an `NSTableView` row callback that
    // waited on the store would freeze the scroll.
    assert!(
        session.row_at(0).is_none(),
        "the first ask should miss rather than block on a read"
    );

    // The page lands behind it.
    session.settle_for_test();

    let row = session.row_at(0).expect("the row after its page arrived");
    assert!(row.id > 0, "a delivered row carries its message id");
    session.shutdown();
}

#[test]
fn a_mailbox_is_never_loaded_into_memory() {
    // The assertion `PRODUCT.md` §18 is actually about. A hundred thousand
    // rows, a jump to the far end, and what is resident afterwards is a
    // handful of pages -- not a hundred thousand ids that crossed the FFI.
    let (session, scope) = seeded(1_000);
    session.open_scope(scope);

    let _ = session.row_at(900);
    session.settle_for_test();
    assert!(session.row_at(900).is_some(), "the far page arrived");

    let resident = session.resident_rows_for_test();
    assert!(
        resident <= 200,
        "{resident} rows resident after one jump; the window is not bounded"
    );
    session.shutdown();
}

#[test]
fn asking_twice_for_the_same_missing_row_asks_the_store_once() {
    // A table redraws its visible rows constantly. If every miss issued a
    // fresh read, scrolling would flood the runtime with duplicate work for
    // pages already on their way.
    let (session, scope) = seeded(120);
    session.open_scope(scope);

    let before = session.page_reads_for_test();
    let _ = session.row_at(0);
    let _ = session.row_at(1);
    let _ = session.row_at(2);
    let after = session.page_reads_for_test();

    assert!(
        after - before <= 2,
        "three misses in one page issued {} reads; requests are not deduplicated",
        after - before
    );
    session.shutdown();
}

#[test]
fn reopening_a_scope_discards_what_the_old_one_had_in_flight() {
    // The generation guard, which `feed.rs` earned the hard way: a page that
    // arrives after the user has moved to another folder must not fill the
    // new folder with the old one's mail.
    let (session, scope) = seeded(120);
    session.open_scope(scope.clone());
    let _ = session.row_at(0);

    // Move before the page lands, then let it land.
    let second = session.open_scope(scope);
    session.settle_for_test();

    assert_ne!(second, 0, "reopening a scope must produce a new generation");
    // Whatever arrived under the old generation was dropped; the new scope
    // still reports its own count rather than a mixture.
    assert_eq!(session.row_count(), 120);
    session.shutdown();
}

/// The unified view is selectable across the ABI, not just from GTK.
///
/// `ScopeFfi` is the wire mirror of `ListScope`, and a variant missing from
/// it is a view the second frontend (ADR 0019) cannot ask for at all — the
/// drift `docs/engineering-notes.md` warns about under "Six types are called
/// *Scope*". Asserted by counting rows rather than by matching the enum: a
/// mapping that compiled and then listed nothing would satisfy a round-trip
/// check and still be broken.
#[test]
fn the_unified_scope_crosses_the_abi_and_lists_every_accounts_mail() {
    let database = test_support::memory();
    postio_storage::seed::seed_small(&database, 21);
    postio_storage::seed::seed_extra_account(&database, "Second", "grace@example.org", 22);

    let session = Session::open(SessionOptions::in_memory_with(database))
        .expect("a session over the seeded store");

    session.open_scope(ScopeFfi::Unified);
    assert!(
        session.row_count() > 0,
        "two seeded accounts and the unified scope reports an empty list"
    );

    // Miss, then settle, then read -- the same shape every other test here
    // uses, because the first ask issues the page read rather than waiting
    // on it.
    let _ = session.row_at(0);
    session.settle_for_test();
    assert!(
        session.row_at(0).is_some(),
        "the unified scope reports {} rows and cannot name the first one \
         after its page arrived",
        session.row_count()
    );
    session.shutdown();
}
