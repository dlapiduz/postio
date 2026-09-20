//! The queue drainer, against the `MailBackend` mock.
//!
//! No network and no server: `MockBackend` is an in-memory mail store with
//! injectable faults, which is how the whole sync engine is developed
//! (`crates/postio-account/src/backend/mod.rs`). Every conflict below is a thing a
//! real server does.

use chrono::{DateTime, TimeDelta, TimeZone, Utc};
use postio_account::backend::{Fault, MailBackend, MockBackend, MockMailbox, MockMessage, UidSet};
use postio_model::{
    AccountId, Flag, FlagSet, MailboxId, Message, MessageId, Operation, OperationState,
    OperationTarget, Uid, UidValidity,
};
use postio_storage::Connection;
use postio_storage::repository::{
    MailboxRepository, MessageRepository, OperationQueueRepository, QueuedOperation,
};
use postio_storage::test_support;
use postio_sync::{DrainReport, Drainer, FailedOperation, RetryPolicy};

const INBOX: &str = "INBOX";
const ARCHIVE: &str = "Archive";
const TRASH: &str = "Deleted Messages";

fn at(hour: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 3, 1, hour, 0, 0).unwrap()
}

fn flags(raw: &str) -> FlagSet {
    raw.split_whitespace().map(Flag::parse).collect()
}

/// A server with three folders and one message in the inbox.
async fn server() -> MockBackend {
    let backend = MockBackend::builder()
        .mailbox(
            MockMailbox::new(INBOX)
                .uid_validity(UidValidity::new(1_707_000_000))
                .message(MockMessage::new(
                    b"From: Ada Lovelace <ada@example.com>\r\n\
                      Subject: Analytical engine\r\n\r\nNotes.\r\n"
                        .to_vec(),
                )),
        )
        .mailbox(MockMailbox::new(ARCHIVE))
        .mailbox(MockMailbox::new(TRASH))
        .build();
    backend.connect().await.expect("connect");
    backend
}

/// The local store mirroring that server: three mailboxes and one message at
/// UID 1 in the inbox.
struct Local {
    account: AccountId,
    inbox: MailboxId,
    archive: MailboxId,
    trash: MailboxId,
    message: MessageId,
}

async fn local(connection: &Connection) -> Local {
    let account = test_support::account(connection).await;
    let inbox = test_support::mailbox(connection, &account, INBOX).await.id;
    let archive = test_support::mailbox(connection, &account, ARCHIVE)
        .await
        .id;
    let trash = test_support::mailbox(connection, &account, TRASH).await.id;

    let mut message = Message::new(account.id, inbox, at(8));
    message.server.uid = Some(Uid::new(1));
    message.server.uid_validity = Some(UidValidity::new(1_707_000_000));
    message.server.remote_id = Some(postio_model::RemoteId::new("1707000000:1"));
    let message = MessageRepository::new(connection)
        .create(&mut message)
        .await
        .expect("create the message");

    Local {
        account: account.id,
        inbox,
        archive,
        trash,
        message,
    }
}

async fn enqueue(
    connection: &Connection,
    local: &Local,
    operation: Operation,
    when: DateTime<Utc>,
) {
    OperationQueueRepository::new(connection)
        .enqueue(
            local.account,
            OperationTarget::Message(local.message),
            &operation,
            when,
        )
        .await
        .expect("enqueue");
}

async fn rows(connection: &Connection, account: AccountId) -> Vec<QueuedOperation> {
    let queue = OperationQueueRepository::new(connection);
    let mut all = Vec::new();
    let mut id = 1;
    while let Some(row) = queue
        .get(postio_model::OperationId::new(id))
        .await
        .expect("get")
    {
        all.push(row);
        id += 1;
    }
    all.retain(|row| row.account_id == account);
    all
}

async fn server_flags(backend: &MockBackend, mailbox: &str, uid: u32) -> Option<FlagSet> {
    backend
        .fetch_headers(
            mailbox,
            &UidSet::single(Uid::new(uid)),
            None,
            &postio_account::cancel::CancelToken::new(),
        )
        .await
        .expect("fetch")
        .into_iter()
        .next()
        .map(|message| message.flags)
}

async fn count(backend: &MockBackend, mailbox: &str) -> usize {
    backend.status(mailbox).await.expect("status").exists as usize
}

// ---------------------------------------------------------------------------
// The happy path, and the offline case the queue exists for
// ---------------------------------------------------------------------------

#[tokio::test]
async fn an_empty_queue_is_an_idle_pass() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let local = local(&connection).await;
    let backend = server().await;

    let report = Drainer::new(&backend)
        .drain(&connection, local.account, at(9))
        .await
        .expect("drain");

    assert!(report.is_idle());
    assert_eq!(report, DrainReport::default());
}

#[tokio::test]
async fn a_flag_change_queued_offline_reaches_the_server_on_the_next_pass() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let local = local(&connection).await;
    let backend = server().await;

    enqueue(
        &connection,
        &local,
        Operation::SetFlags {
            flags: flags("\\Seen"),
        },
        at(9),
    )
    .await;

    let report = Drainer::new(&backend)
        .drain(&connection, local.account, at(10))
        .await
        .expect("drain");

    assert_eq!(report.applied, 1);
    assert!(report.failed.is_empty());
    assert!(
        server_flags(&backend, INBOX, 1)
            .await
            .expect("the message")
            .is_seen()
    );
    assert_eq!(
        rows(&connection, local.account).await[0].state,
        OperationState::Done
    );
}

#[tokio::test]
async fn a_queue_of_offline_actions_applies_in_order() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let local = local(&connection).await;
    let backend = server().await;

    // Read it, flag it, then archive it — a minute of work with no connection.
    enqueue(
        &connection,
        &local,
        Operation::SetFlags {
            flags: flags("\\Seen"),
        },
        at(9),
    )
    .await;
    enqueue(
        &connection,
        &local,
        Operation::SetFlags {
            flags: flags("\\Flagged"),
        },
        at(9),
    )
    .await;
    enqueue(
        &connection,
        &local,
        Operation::Move {
            from: local.inbox,
            to: local.archive,
        },
        at(9),
    )
    .await;

    let report = Drainer::new(&backend)
        .drain(&connection, local.account, at(10))
        .await
        .expect("drain");

    assert_eq!(report.applied, 3, "all three rows settled");
    assert_eq!(report.settled(), 3);
    assert_eq!(count(&backend, INBOX).await, 0);
    assert_eq!(count(&backend, ARCHIVE).await, 1);

    let archived = server_flags(&backend, ARCHIVE, 1)
        .await
        .expect("the message");
    assert!(
        archived.is_seen() && archived.is_flagged(),
        "the flags were applied before the move, as the user did them"
    );
}

#[tokio::test]
async fn redundant_work_never_reaches_the_server() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let local = local(&connection).await;
    let backend = server().await;

    // Archived, then undone, while offline.
    enqueue(
        &connection,
        &local,
        Operation::Move {
            from: local.inbox,
            to: local.archive,
        },
        at(9),
    )
    .await;
    enqueue(
        &connection,
        &local,
        Operation::Move {
            from: local.archive,
            to: local.inbox,
        },
        at(9),
    )
    .await;

    let before = backend.calls();
    let report = Drainer::new(&backend)
        .drain(&connection, local.account, at(10))
        .await
        .expect("drain");

    assert_eq!(report.coalesced, 2);
    assert_eq!(report.applied, 0);
    assert_eq!(
        count(&backend, ARCHIVE).await,
        0,
        "the message never visited the archive on the server"
    );
    assert!(
        backend.calls() - before <= 2,
        "one CAPABILITY and one STATUS, no MOVE"
    );
    for row in rows(&connection, local.account).await {
        assert_eq!(row.state, OperationState::Done, "and both rows are settled");
    }
}

#[tokio::test]
async fn a_delete_moves_the_message_to_the_trash() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let local = local(&connection).await;
    let backend = server().await;

    enqueue(
        &connection,
        &local,
        Operation::Delete {
            from: local.inbox,
            trash: local.trash,
        },
        at(9),
    )
    .await;

    let report = Drainer::new(&backend)
        .drain(&connection, local.account, at(10))
        .await
        .expect("drain");

    assert_eq!(report.applied, 1);
    assert_eq!(count(&backend, TRASH).await, 1);
    assert_eq!(count(&backend, INBOX).await, 0);
}

// ---------------------------------------------------------------------------
// Conflicts
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_message_deleted_remotely_settles_the_operation_and_asks_for_a_resync() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let local = local(&connection).await;
    let backend = server().await;

    // Another client archived it while we were offline. Our queue still holds
    // a flag change against the inbox.
    backend
        .move_messages(
            INBOX,
            &[postio_model::RemoteId::new("1707000000:1")],
            ARCHIVE,
        )
        .await
        .expect("the other client's move");

    enqueue(
        &connection,
        &local,
        Operation::SetFlags {
            flags: flags("\\Seen"),
        },
        at(9),
    )
    .await;

    let report = Drainer::new(&backend)
        .drain(&connection, local.account, at(10))
        .await
        .expect("drain");

    assert_eq!(report.obsolete, 1, "there was nothing there to flag");
    assert_eq!(report.applied, 0);
    assert!(
        report.failed.is_empty(),
        "the user did nothing wrong; this is not an error to show them"
    );
    assert_eq!(
        report.needs_resync,
        vec![local.inbox],
        "the local row disagrees with the server, so the mailbox is stale"
    );

    let row = &rows(&connection, local.account).await[0];
    assert_eq!(row.state, OperationState::Done);
    assert!(
        row.last_error
            .as_deref()
            .is_some_and(|note| note.contains("no longer in that mailbox")),
        "and why, for the bug report: {:?}",
        row.last_error
    );
}

#[tokio::test]
async fn a_move_confirmed_without_copyuid_is_applied_and_the_destination_resynced() {
    // RFC 4315 §3: a UIDPLUS server SHOULD return COPYUID and MAY omit it --
    // a UIDNOTSTICKY destination, or one the account may write to but not
    // select. Absence means the new UIDs are unknown, which the client
    // "can discover by selecting the destination mailbox". It does not mean
    // the message was gone, and reading it that way settled a successful
    // move as obsolete, never retried it, and condemned the *source* to a
    // resync (#903).
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let local = local(&connection).await;
    let backend = server().await;
    backend.omit_uid_mappings();

    enqueue(
        &connection,
        &local,
        Operation::Move {
            from: local.inbox,
            to: local.archive,
        },
        at(9),
    )
    .await;

    let report = Drainer::new(&backend)
        .drain(&connection, local.account, at(10))
        .await
        .expect("drain");

    assert_eq!(
        report.applied, 1,
        "the server moved the message; a missing COPYUID is not a missing message"
    );
    assert_eq!(report.obsolete, 0, "nothing was obsolete: {report:?}");
    assert_eq!(count(&backend, INBOX).await, 0);
    assert_eq!(count(&backend, ARCHIVE).await, 1);
    assert_eq!(
        report.needs_resync,
        vec![local.archive],
        "the destination is where the UIDs can be discovered, so it is what gets resynced"
    );
}

#[tokio::test]
async fn a_message_moved_on_both_sides_does_not_move_twice() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let local = local(&connection).await;
    let backend = server().await;

    // The server moved it to the archive already; we queued the same move.
    backend
        .move_messages(
            INBOX,
            &[postio_model::RemoteId::new("1707000000:1")],
            ARCHIVE,
        )
        .await
        .expect("the other client's move");

    enqueue(
        &connection,
        &local,
        Operation::Move {
            from: local.inbox,
            to: local.archive,
        },
        at(9),
    )
    .await;

    let report = Drainer::new(&backend)
        .drain(&connection, local.account, at(10))
        .await
        .expect("drain");

    // On the wire this is indistinguishable from a server that moved the
    // message and omitted COPYUID (RFC 4315 §3), so it settles the same way
    // (#903): applied, nothing duplicated, and the destination resynced --
    // which is where both readings are reconciled, because it is where the
    // message is.
    assert_eq!(report.applied, 1);
    assert_eq!(report.obsolete, 0);
    assert_eq!(
        count(&backend, ARCHIVE).await,
        1,
        "one copy, not two: the message is not duplicated by replaying our intent"
    );
    assert_eq!(report.needs_resync, vec![local.archive]);
}

#[tokio::test]
async fn a_renumbered_mailbox_fails_the_operation_rather_than_acting_on_the_wrong_message() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let local = local(&connection).await;
    let backend = server().await;

    backend.change_uid_validity(INBOX, UidValidity::new(1_900_000_000));

    enqueue(
        &connection,
        &local,
        Operation::SetFlags {
            flags: flags("\\Seen"),
        },
        at(9),
    )
    .await;

    let report = Drainer::new(&backend)
        .drain(&connection, local.account, at(10))
        .await
        .expect("drain");

    assert_eq!(report.failed.len(), 1);
    assert!(
        report.failed[0].reason.contains("UIDVALIDITY"),
        "{}",
        report.failed[0].reason
    );
    assert_eq!(
        report.needs_resync,
        vec![local.inbox],
        "the whole mailbox has to be refetched before the UID means anything"
    );
    assert_eq!(report.deferred, 0, "retrying would flag the wrong message");
}

#[tokio::test]
async fn a_message_that_was_never_uploaded_has_nothing_to_send() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let local = local(&connection).await;
    let backend = server().await;

    let mut composed = Message::new(local.account, local.inbox, at(8));
    let composed = MessageRepository::new(&connection)
        .create(&mut composed)
        .await
        .expect("create");

    OperationQueueRepository::new(&connection)
        .enqueue(
            local.account,
            OperationTarget::Message(composed),
            &Operation::SetFlags {
                flags: flags("\\Seen"),
            },
            at(9),
        )
        .await
        .expect("enqueue");

    let report = Drainer::new(&backend)
        .drain(&connection, local.account, at(10))
        .await
        .expect("drain");

    assert_eq!(report.obsolete, 1);
    assert!(report.failed.is_empty());
    assert!(
        report.needs_resync.is_empty(),
        "nothing about the server is stale; the message was simply never there"
    );
}

#[tokio::test]
async fn a_missing_destination_mailbox_is_a_permanent_failure() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let local = local(&connection).await;
    let backend = server().await;

    enqueue(
        &connection,
        &local,
        Operation::Move {
            from: local.inbox,
            to: local.archive,
        },
        at(9),
    )
    .await;
    MailboxRepository::new(&connection)
        .delete(local.archive)
        .await
        .expect("delete the destination");

    let report = Drainer::new(&backend)
        .drain(&connection, local.account, at(10))
        .await
        .expect("drain");

    assert_eq!(report.failed.len(), 1);
    assert!(report.failed[0].reason.contains("destination"));
    assert_eq!(report.deferred, 0);
}

// ---------------------------------------------------------------------------
// Retry and backoff
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_transient_failure_comes_back_with_a_backoff() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let local = local(&connection).await;
    let backend = server().await;

    enqueue(
        &connection,
        &local,
        Operation::SetFlags {
            flags: flags("\\Seen"),
        },
        at(9),
    )
    .await;
    // The CAPABILITY the pass opens with succeeds; the STORE does not.
    backend.inject_after(1, Fault::Io("network is unreachable".to_owned()));

    let report = Drainer::new(&backend)
        .drain(&connection, local.account, at(10))
        .await
        .expect("drain");

    assert_eq!(report.deferred, 1);
    assert!(report.failed.is_empty(), "one failure is not a lost cause");

    let row = &rows(&connection, local.account).await[0];
    assert_eq!(row.state, OperationState::Pending, "still queued");
    assert_eq!(row.attempts, 1);
    assert_eq!(
        row.next_attempt_at,
        Some(at(10) + TimeDelta::seconds(2)),
        "the base backoff"
    );

    // Not due yet.
    let idle = Drainer::new(&backend)
        .drain(&connection, local.account, at(10) + TimeDelta::seconds(1))
        .await
        .expect("drain");
    assert!(idle.is_idle());

    // Due, and the server is back.
    let recovered = Drainer::new(&backend)
        .drain(&connection, local.account, at(10) + TimeDelta::seconds(3))
        .await
        .expect("drain");
    assert_eq!(recovered.applied, 1);
    assert!(
        server_flags(&backend, INBOX, 1)
            .await
            .expect("the message")
            .is_seen()
    );
}

#[tokio::test]
async fn a_server_that_asks_us_to_slow_down_is_obeyed() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let local = local(&connection).await;
    let backend = server().await;

    enqueue(
        &connection,
        &local,
        Operation::SetFlags {
            flags: flags("\\Seen"),
        },
        at(9),
    )
    .await;
    backend.inject_after(
        1,
        Fault::RateLimited(Some(std::time::Duration::from_secs(600))),
    );

    let report = Drainer::new(&backend)
        .drain(&connection, local.account, at(10))
        .await
        .expect("drain");

    assert_eq!(report.deferred, 1);
    assert_eq!(
        rows(&connection, local.account).await[0].next_attempt_at,
        Some(at(10) + TimeDelta::seconds(600)),
        "ten minutes, because that is what the server asked for"
    );
}

#[tokio::test]
async fn an_operation_that_keeps_failing_is_reported_rather_than_retried_forever() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let local = local(&connection).await;
    let backend = server().await;

    enqueue(
        &connection,
        &local,
        Operation::SetFlags {
            flags: flags("\\Seen"),
        },
        at(9),
    )
    .await;

    let policy = RetryPolicy {
        max_attempts: 3,
        ..RetryPolicy::default()
    };
    let mut when = at(10);
    let mut last = DrainReport::default();

    for _ in 0..3 {
        // `inject_after(1)` skips the CAPABILITY the pass opens with and lands
        // on the STORE. It is relative to the calls served so far, so it works
        // the same on every iteration.
        backend.inject_after(1, Fault::Io("network is unreachable".to_owned()));
        last = Drainer::with_policy(&backend, policy)
            .drain(&connection, local.account, when)
            .await
            .expect("drain");
        when += TimeDelta::hours(1);
    }

    assert_eq!(last.failed.len(), 1, "it is reported, not dropped");
    assert!(last.failed[0].reason.contains("gave up after 3 attempts"));
    assert_eq!(last.failed[0].op_type, "set_flags");
    assert_eq!(
        last.failed[0].target,
        OperationTarget::Message(local.message)
    );

    let row = &rows(&connection, local.account).await[0];
    assert_eq!(
        row.state,
        OperationState::Failed,
        "and it stays on the row so the user can see it"
    );
    assert!(row.last_error.is_some());

    let after = Drainer::with_policy(&backend, policy)
        .drain(&connection, local.account, when)
        .await
        .expect("drain");
    assert!(
        after.is_idle(),
        "a failed operation is not silently retried behind the user's back"
    );
}

#[tokio::test]
async fn a_permanent_refusal_is_not_retried() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let local = local(&connection).await;
    let backend = server().await;

    enqueue(
        &connection,
        &local,
        Operation::SetFlags {
            flags: flags("\\Seen"),
        },
        at(9),
    )
    .await;
    backend.inject_after(1, Fault::Rejected("permission denied".to_owned()));

    let report = Drainer::new(&backend)
        .drain(&connection, local.account, at(10))
        .await
        .expect("drain");

    assert_eq!(report.deferred, 0, "retrying a refusal just wastes battery");
    assert_eq!(report.failed.len(), 1);
    assert!(report.failed[0].reason.contains("permission denied"));
}

#[tokio::test]
async fn a_folded_step_defers_every_row_behind_it_together() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let local = local(&connection).await;
    let backend = server().await;

    enqueue(
        &connection,
        &local,
        Operation::SetFlags {
            flags: flags("\\Seen"),
        },
        at(9),
    )
    .await;
    enqueue(
        &connection,
        &local,
        Operation::SetFlags {
            flags: flags("\\Flagged"),
        },
        at(9),
    )
    .await;
    backend.inject_after(1, Fault::Io("network is unreachable".to_owned()));

    let report = Drainer::new(&backend)
        .drain(&connection, local.account, at(10))
        .await
        .expect("drain");

    assert_eq!(report.deferred, 2, "both rows, not just the one that led");
    for row in rows(&connection, local.account).await {
        assert_eq!(row.state, OperationState::Pending);
        assert_eq!(row.attempts, 1);
    }

    let recovered = Drainer::new(&backend)
        .drain(&connection, local.account, at(10) + TimeDelta::seconds(3))
        .await
        .expect("drain");
    assert_eq!(recovered.applied, 2);

    let stored = server_flags(&backend, INBOX, 1).await.expect("the message");
    assert!(
        stored.is_seen() && stored.is_flagged(),
        "and the fold still carried both flags"
    );
}

// ---------------------------------------------------------------------------
// Not implemented yet, but never silent
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_queued_send_is_reported_rather_than_left_pending_forever() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let local = local(&connection).await;
    let backend = server().await;

    OperationQueueRepository::new(&connection)
        .enqueue(
            local.account,
            OperationTarget::Draft(postio_model::DraftId::new(1)),
            &Operation::Send {
                draft: postio_model::DraftId::new(1),
            },
            at(9),
        )
        .await
        .expect("enqueue");

    let report = Drainer::new(&backend)
        .drain(&connection, local.account, at(10))
        .await
        .expect("drain");

    assert_eq!(report.failed.len(), 1);
    assert_eq!(report.failed[0].op_type, "send");
    assert!(report.failed[0].reason.contains("SMTP"));
}

/// A failed operation's toast says what did not happen before it says why
/// (#1487). The reason alone -- "550 mailbox unavailable" -- reads as a fact
/// about the world rather than as the fate of something the person asked
/// for, and the toast has no other context to lend it.
#[test]
fn a_failed_operation_says_what_did_not_happen_then_why() {
    let failed = FailedOperation {
        rows: Vec::new(),
        target: postio_model::operation::OperationTarget::Message(
            postio_model::ids::MessageId::new(7),
        ),
        op_type: "send",
        reason: "550 mailbox unavailable".to_owned(),
    };
    assert_eq!(failed.said(), "Not sent \u{2014} 550 mailbox unavailable");

    let moved = FailedOperation {
        op_type: "move",
        reason: "the message is no longer in the local store".to_owned(),
        ..failed
    };
    assert_eq!(
        moved.said(),
        "Not moved \u{2014} the message is no longer in the local store"
    );
}
