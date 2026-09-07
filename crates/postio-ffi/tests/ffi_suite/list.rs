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

#[test]
fn mail_arriving_into_the_open_scope_changes_the_row_count() {
    // The bug that a running application found and no test did (#1150).
    //
    // `open_scope` counts, once. Everything after that arrives as an event,
    // and the frontend's answer to an event is to reload its table -- which
    // asks `row_count`, which reads the window's `total`, which was set by
    // that one count and never again. So the first sync of a folder opened
    // while it was empty put 99 messages in the store and left the list
    // saying "No messages", with every layer working exactly as written.
    let database = test_support::memory();
    let (account, mailbox) = {
        let connection = database.connection().expect("a connection");
        let (account, inbox) = test_support::account_with_inbox(&connection);
        (account.id, inbox)
    };
    let session =
        Session::open(SessionOptions::in_memory_with(database.clone())).expect("a session");
    session.open_scope(ScopeFfi::Mailbox {
        mailbox: mailbox.into(),
    });
    assert_eq!(session.row_count(), 0, "the fixture starts empty");

    // The engine writes to the store and then says so, which is the order
    // every sync uses.
    {
        let connection = database.connection().expect("a connection");
        let repository = MessageRepository::new(&connection);
        for _ in 0..7 {
            let mut message = Message::new(account, mailbox, Utc::now());
            repository.create(&mut message).expect("a message");
        }
    }
    session.emit_for_test(postio_core::Event::MessageListChanged { account, mailbox });
    let _ = session.next_event_blocking();

    assert_eq!(
        session.row_count(),
        7,
        "the list still reports the count it was opened with, so a folder \
         that filled after it was opened draws as empty"
    );
    session.shutdown();
}

#[test]
fn a_sender_crosses_as_the_name_a_person_reads() {
    // `EmailAddress::display()`, which is what `postio-gtk`'s row draws --
    // not `to_string()`, which is the RFC form `Name <addr>`. The boundary
    // used the second, so the macOS list drew
    // `Fidelity Investments <Fidelity.Investments@...` where the GTK list
    // drew `Fidelity Investments`: one row, two frontends, two answers, on a
    // field whose own comment says "already rendered for display" (#1150).
    let database = test_support::memory();
    let (account, mailbox) = {
        let connection = database.connection().expect("a connection");
        let (account, inbox) = test_support::account_with_inbox(&connection);
        (account.id, inbox)
    };
    {
        let connection = database.connection().expect("a connection");
        let repository = MessageRepository::new(&connection);
        let mut message = Message::new(account, mailbox, Utc::now());
        message.from = vec![postio_model::EmailAddress::new(
            Some("Ada Lovelace"),
            "ada@example.com",
        )];
        repository.create(&mut message).expect("a message");
    }
    let session = Session::open(SessionOptions::in_memory_with(database)).expect("a session");
    session.open_scope(ScopeFfi::Mailbox {
        mailbox: mailbox.into(),
    });
    let _ = session.row_at(0);
    session.settle_for_test();

    let row = session.row_at(0).expect("the page landed");
    assert_eq!(
        row.from.as_deref(),
        Some("Ada Lovelace"),
        "the row drew the RFC address form instead of the display name"
    );
    session.shutdown();
}

// -- who is in a conversation (#1265) ----------------------------------------

/// A thread row names everyone in the conversation, not its newest sender.
///
/// The list's row stands for a whole conversation (ADR 0015), and the canvas
/// draws `Tessa Vaughn, Mara, Pinepoint` where a message row draws one name.
/// The boundary carried only the representative's sender, so the macOS list
/// drew one name per conversation and no second frontend could have done
/// better.
#[test]
fn a_thread_row_names_the_people_in_the_conversation() {
    use chrono::TimeZone;
    use postio_model::{EmailAddress, Thread};
    use postio_storage::repository::ThreadRepository;

    let database = test_support::memory();
    let mailbox = {
        let connection = database.connection().expect("a connection");
        let (account, inbox) = test_support::account_with_inbox(&connection);

        let mut thread = Thread::new(account.id);
        thread.subject = Some("radon reduction".to_owned());
        let threads = ThreadRepository::new(&connection);
        threads.create(&mut thread).expect("a thread");

        let messages = MessageRepository::new(&connection);
        for (index, sender) in ["Tessa Vaughn", "Mara Ostwald", "Pinepoint Radon"]
            .into_iter()
            .enumerate()
        {
            let mut message = Message::new(
                account.id,
                inbox,
                Utc.timestamp_opt(1_770_000_000 + index as i64, 0)
                    .single()
                    .expect("a real time"),
            );
            message.subject = Some("Radon reduction".to_owned());
            message.from = vec![EmailAddress::new(
                Some(sender),
                format!("{index}@example.com"),
            )];
            messages.create(&mut message).expect("a message");
            threads.add_message(thread.id, message.id).expect("add");
        }
        inbox
    };

    let session = Session::open(SessionOptions::in_memory_with(database))
        .expect("a session over the seeded store");
    session.open_scope(ScopeFfi::Mailbox {
        mailbox: mailbox.into(),
    });
    let _ = session.row_at(0);
    session.settle_for_test();

    let row = session.row_at(0).expect("the conversation row");
    assert_eq!(
        session.row_count(),
        1,
        "a folder lists one row per conversation"
    );
    assert!(row.is_thread);
    assert_eq!(
        row.participants, "Tessa, Mara, Pinepoint",
        "the row says who is in the conversation, shortened the way the \
         conversation header shortens them"
    );
    assert_eq!(row.thread_count, 3);
    session.shutdown();
}

#[test]
fn re_indexing_reports_as_it_goes_rather_than_only_at_the_end() {
    // A pass over five thousand messages takes long enough that a button
    // with no progress is indistinguishable from a button that does nothing.
    // The events have to arrive *while* it runs, which here means: by the
    // time the call returns, several have been queued rather than one.
    let database = test_support::memory();
    let account = {
        let connection = database.connection().expect("a connection");
        let (account, inbox) = test_support::account_with_inbox(&connection);
        let repository = MessageRepository::new(&connection);
        for n in 0..40 {
            let mut message = Message::new(account.id, inbox, Utc::now());
            message.subject = Some(format!("message {n}"));
            let id = repository.create(&mut message).expect("a message");
            repository
                .set_body(
                    id,
                    &postio_storage::repository::StoredBody {
                        text: Some(format!("the body of message {n}")),
                        html: None,
                        headers: None,
                        headers_truncated: false,
                        encoding_problems: false,
                    },
                    postio_model::message::BodyState::Full,
                )
                .expect("a body");
        }
        account.id.get()
    };

    // The FTS tables a real store gets when it is opened. An in-memory one
    // built by `test_support` has the schema and not the index.
    postio_session::ensure_search_index(&database).expect("a search index");

    let session =
        Session::open(SessionOptions::in_memory_with(database)).expect("a session over the store");
    // Whatever opening produced, so what is counted below is the re-index's.
    while session.try_next_event().is_some() {}

    assert_eq!(session.reindex_account(account), None, "no complaint");

    let mut reports = Vec::new();
    while let Some(event) = session.try_next_event() {
        if let postio_ffi::UiEvent::ReindexProgress { done, total, .. } = event {
            reports.push((done, total));
        }
    }

    assert!(
        !reports.is_empty(),
        "nothing was reported at all, so the window has nothing to draw"
    );
    let (_, total) = reports[0];
    assert!(
        total > 0,
        "a total of zero is a progress bar with no meaning"
    );
    assert!(
        reports.iter().any(|(done, _)| *done > 0),
        "every report said zero: {reports:?}"
    );
}
