//! Incremental resync: what changed since last time, and the `UIDVALIDITY`
//! trap.
//!
//! No network and no server: `MockBackend` is the in-memory mail store the
//! whole sync engine is developed against (see
//! `crates/postio-imap/src/backend/mock.rs`).

use postio_imap::backend::{Fault, MailBackend, MockBackend, MockMailbox, MockMessage, UidSet};
use postio_imap::cancel::CancelToken;
use postio_model::{AccountId, Flag, FlagSet, Mailbox, Uid, UidValidity};
use postio_storage::repository::{ContactRepository, MessageRepository, SyncStateRepository};
use postio_storage::test_support;
use postio_sync::{Outcome, resync_mailbox, sync_mailbox};
use rusqlite::Connection;

fn times_ada_was_seen(connection: &Connection, account_id: AccountId) -> u32 {
    ContactRepository::new(connection)
        .list(Some(account_id))
        .expect("list contacts")
        .into_iter()
        .find(|contact| contact.address.normalized() == "ada@example.com")
        .map(|contact| contact.times_seen)
        .unwrap_or(0)
}

const INBOX: &str = "INBOX";
const VALIDITY: u32 = 1_707_000_000;

fn note(n: u32) -> Vec<u8> {
    format!(
        "From: Ada Lovelace <ada@example.com>\r\n\
         Subject: Note {n}\r\n\r\nBody {n}.\r\n"
    )
    .into_bytes()
}

async fn server_with_messages(count: u32) -> MockBackend {
    let mut mailbox = MockMailbox::new(INBOX).uid_validity(UidValidity::new(VALIDITY));
    for n in 1..=count {
        mailbox = mailbox.message(MockMessage::new(note(n)));
    }
    let backend = MockBackend::builder().mailbox(mailbox).build();
    backend.connect().await.expect("connect");
    backend
}

fn local(connection: &Connection) -> (AccountId, Mailbox) {
    let account = test_support::account(connection);
    let inbox = test_support::mailbox(connection, &account, INBOX);
    (account.id, inbox)
}

/// Runs the initial sync so the local store matches `backend`, as a fixture
/// step for tests that are about what happens *after* that.
async fn bootstrap(connection: &Connection, backend: &MockBackend, mailbox: &Mailbox) {
    sync_mailbox(connection, backend, mailbox, &CancelToken::new(), |_| {})
        .await
        .expect("bootstrap sync");
}

fn known_uids(connection: &Connection, mailbox: &Mailbox) -> Vec<u32> {
    MessageRepository::new(connection)
        .uids_in(mailbox.id, UidValidity::new(VALIDITY))
        .expect("uids_in")
        .into_iter()
        .map(Uid::get)
        .collect()
}

#[tokio::test]
async fn a_reconnect_with_no_server_changes_fetches_essentially_nothing() {
    let backend = server_with_messages(3).await;
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (_account, inbox) = local(&connection);
    bootstrap(&connection, &backend, &inbox).await;

    let calls_before = backend.calls();
    let outcome = resync_mailbox(&connection, &backend, &inbox, &CancelToken::new(), |_| {})
        .await
        .expect("resync");

    assert_eq!(outcome, Outcome::UpToDate);
    // Only the one SELECT this pass had to make to find out nothing changed —
    // no FETCH at all.
    assert_eq!(backend.calls() - calls_before, 1);
    assert_eq!(known_uids(&connection, &inbox), vec![1, 2, 3]);
}

#[tokio::test]
async fn a_server_side_flag_change_and_deletion_both_reflect_locally() {
    let backend = server_with_messages(3).await;
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (_account, inbox) = local(&connection);
    bootstrap(&connection, &backend, &inbox).await;

    // Another client flags message 2 as seen...
    backend
        .store_flags(
            INBOX,
            &UidSet::single(Uid::new(2)),
            &postio_imap::backend::FlagChange::Add(FlagSet::from_iter([Flag::Seen])),
        )
        .await
        .expect("flag");
    // ...and deletes message 3 outright.
    backend
        .store_flags(
            INBOX,
            &UidSet::single(Uid::new(3)),
            &postio_imap::backend::FlagChange::Add(FlagSet::from_iter([Flag::Deleted])),
        )
        .await
        .expect("mark deleted");
    backend
        .expunge(INBOX, Some(&UidSet::single(Uid::new(3))))
        .await
        .expect("expunge");

    let outcome = resync_mailbox(&connection, &backend, &inbox, &CancelToken::new(), |_| {})
        .await
        .expect("resync");

    match outcome {
        Outcome::Incremental {
            changed,
            vanished,
            arrived,
        } => {
            assert_eq!(changed, 1, "only message 2's flag change was reported");
            assert_eq!(vanished, 1, "message 3 is gone");
            assert!(
                arrived.is_empty(),
                "a flag change is not new mail: {arrived:?}"
            );
        }
        other => panic!("expected an incremental resync, got {other:?}"),
    }

    let messages = MessageRepository::new(&connection);
    let seen = messages
        .by_uid(inbox.id, UidValidity::new(VALIDITY), Uid::new(2))
        .expect("look up message 2")
        .expect("message 2 still stored");
    assert!(
        seen.flags.is_seen(),
        "the flag change must reach the local row"
    );

    assert_eq!(
        known_uids(&connection, &inbox),
        vec![1, 2],
        "the expunged message must be gone locally too"
    );
}

/// postio-66j's "watch out for": a flag change resyncs the message that
/// already has a contact sighting from bootstrap, and must not add another
/// one — `times_seen` counting flag changes as new mail would overstate every
/// correspondent every time the user reads or archives something.
#[tokio::test]
async fn a_flag_only_change_does_not_double_count_the_correspondent() {
    let backend = server_with_messages(3).await;
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account_id, inbox) = local(&connection);
    bootstrap(&connection, &backend, &inbox).await;
    assert_eq!(
        times_ada_was_seen(&connection, account_id),
        3,
        "bootstrap already saw ada on all three messages"
    );

    backend
        .store_flags(
            INBOX,
            &UidSet::single(Uid::new(2)),
            &postio_imap::backend::FlagChange::Add(FlagSet::from_iter([Flag::Seen])),
        )
        .await
        .expect("flag");

    let outcome = resync_mailbox(&connection, &backend, &inbox, &CancelToken::new(), |_| {})
        .await
        .expect("resync");
    assert!(matches!(outcome, Outcome::Incremental { changed: 1, .. }));

    assert_eq!(
        times_ada_was_seen(&connection, account_id),
        3,
        "a flag change on a message already seen must not count as a new sighting"
    );
}

#[tokio::test]
async fn a_message_the_change_feed_never_mentions_still_arrives() {
    let backend = server_with_messages(2).await;
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (_account, inbox) = local(&connection);
    bootstrap(&connection, &backend, &inbox).await;

    // A delivery whose `MODSEQ` does not exceed the `HIGHESTMODSEQ` we hold,
    // so `CHANGEDSINCE` — which is strictly greater-than — will not report it.
    // RFC 7162 §3.1.2.1 says this cannot happen; servers say otherwise, and a
    // message that never appears is the worst failure a mail client has.
    backend
        .append(INBOX, &postio_imap::backend::AppendMessage::new(note(3)))
        .await
        .expect("deliver");

    let outcome = resync_mailbox(&connection, &backend, &inbox, &CancelToken::new(), |_| {})
        .await
        .expect("resync");

    match outcome {
        Outcome::Incremental {
            changed,
            vanished,
            arrived,
        } => {
            assert_eq!(changed, 1, "UIDNEXT moved, so the gap was fetched");
            assert_eq!(vanished, 0);
            assert_eq!(
                arrived.len(),
                1,
                "the gap UIDNEXT caught is exactly one new message"
            );
        }
        other => panic!("expected an incremental resync, got {other:?}"),
    }
    assert_eq!(
        known_uids(&connection, &inbox),
        vec![1, 2, 3],
        "UIDNEXT is the second witness for an arrival, and it cannot be wrong \
         without the server being incoherent"
    );
}

#[tokio::test]
async fn an_arrival_during_resync_is_recorded_as_a_correspondent() {
    let backend = server_with_messages(2).await;
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account_id, inbox) = local(&connection);
    bootstrap(&connection, &backend, &inbox).await;
    assert_eq!(times_ada_was_seen(&connection, account_id), 2);

    backend
        .append(INBOX, &postio_imap::backend::AppendMessage::new(note(3)))
        .await
        .expect("deliver");

    resync_mailbox(&connection, &backend, &inbox, &CancelToken::new(), |_| {})
        .await
        .expect("resync");

    assert_eq!(
        times_ada_was_seen(&connection, account_id),
        3,
        "the incremental pull found a new message, so it must add a sighting"
    );
}

#[tokio::test]
async fn a_conforming_server_costs_no_extra_round_trip_for_arrivals() {
    let backend = server_with_messages(2).await;
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (_account, inbox) = local(&connection);
    bootstrap(&connection, &backend, &inbox).await;

    // A flag change *is* reported by the change feed, and moves no UIDs.
    backend
        .store_flags(
            INBOX,
            &UidSet::single(Uid::new(1)),
            &postio_imap::backend::FlagChange::Add(FlagSet::from_iter([Flag::Seen])),
        )
        .await
        .expect("flag");

    let before = backend.calls();
    resync_mailbox(&connection, &backend, &inbox, &CancelToken::new(), |_| {})
        .await
        .expect("resync");

    assert_eq!(
        backend.calls() - before,
        2,
        "one SELECT and one FETCH: when the change feed accounts for UIDNEXT \
         there is no gap left to ask about"
    );
}

#[tokio::test]
async fn a_uid_validity_change_wipes_and_rebuilds_the_mailbox() {
    let backend = server_with_messages(2).await;
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (_account, inbox) = local(&connection);
    bootstrap(&connection, &backend, &inbox).await;

    let new_validity = UidValidity::new(1_900_000_000);
    backend.change_uid_validity(INBOX, new_validity);
    // A real server only ever reports a UIDVALIDITY change once, at the
    // SELECT that reveals it, and behaves consistently under the new
    // generation from then on; the mock's stricter guard on every other call
    // exists to catch code that skips checking SELECT's answer, which is not
    // what is under test here.
    backend.acknowledge_uid_validity(INBOX);

    let outcome = resync_mailbox(&connection, &backend, &inbox, &CancelToken::new(), |_| {})
        .await
        .expect("resync");

    match outcome {
        Outcome::Full { reason, report } => {
            assert_eq!(reason, postio_model::FullResyncReason::UidValidityChanged);
            assert_eq!(report.inserted, 2);
        }
        other => panic!("expected a full resync, got {other:?}"),
    }

    let messages = MessageRepository::new(&connection);
    assert!(
        messages
            .by_uid(inbox.id, UidValidity::new(VALIDITY), Uid::new(1))
            .expect("look up under the old generation")
            .is_none(),
        "rows under the stale UIDVALIDITY must be gone"
    );
    let rebuilt = messages
        .by_uid(inbox.id, new_validity, Uid::new(1))
        .expect("look up under the new generation")
        .expect("rebuilt under the new generation");
    assert_eq!(rebuilt.server.uid_validity, Some(new_validity));

    let state = SyncStateRepository::new(&connection)
        .require(inbox.id)
        .expect("sync state");
    assert_eq!(state.uid_validity, Some(new_validity));
    assert!(state.has_synced());
}

#[tokio::test]
async fn a_transient_backend_failure_during_resync_is_not_treated_as_a_resync_result() {
    let backend = server_with_messages(1).await;
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (_account, inbox) = local(&connection);
    bootstrap(&connection, &backend, &inbox).await;

    backend.inject(Fault::Disconnect);
    let error = resync_mailbox(&connection, &backend, &inbox, &CancelToken::new(), |_| {})
        .await
        .expect_err("a dropped connection must surface as an error, not UpToDate");
    assert!(matches!(error, postio_sync::SyncError::Backend(_)));
}
