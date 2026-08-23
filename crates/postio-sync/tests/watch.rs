//! Watching for new mail: IDLE on the pushed mailbox, interval polling
//! everywhere else, and the ways a watcher goes quietly deaf.
//!
//! The schedule itself is unit-tested in `src/watch.rs`. What needs a backend
//! is the part that can only be shown end to end: that mail appearing on the
//! server reaches the local store without anybody asking for it, that a
//! suspend never leaves a second connection behind, and that a server which
//! accepts `IDLE` and then says nothing is still unable to hide new mail.

use std::time::Duration;

use chrono::{DateTime, TimeDelta, TimeZone, Utc};
use postio_imap::backend::{
    AppendMessage, Capabilities, Fault, MailBackend, MailboxEvent, MockBackend, MockMailbox,
    MockMessage,
};
use postio_imap::cancel::CancelToken;
use postio_model::{Mailbox, MailboxId, UidValidity};
use postio_storage::repository::MessageRepository;
use postio_storage::test_support;
use postio_sync::watch::{Attention, Watch, WatchPolicy, Watcher};
use postio_sync::{resync_mailbox, sync_mailbox};

const INBOX: &str = "INBOX";
const ARCHIVE: &str = "Archive";
const VALIDITY: u32 = 1_707_000_000;

fn at(second: i64) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 3, 1, 9, 0, 0).unwrap() + TimeDelta::seconds(second)
}

fn note(n: u32) -> Vec<u8> {
    format!(
        "From: Ada Lovelace <ada@example.com>\r\n\
         Subject: Note {n}\r\n\r\nBody {n}.\r\n"
    )
    .into_bytes()
}

/// A connected server with `count` messages in INBOX and an empty Archive.
async fn server(count: u32) -> MockBackend {
    let mut inbox = MockMailbox::new(INBOX).uid_validity(UidValidity::new(VALIDITY));
    for n in 1..=count {
        inbox = inbox.message(MockMessage::new(note(n)));
    }
    let backend = MockBackend::builder()
        .mailbox(inbox)
        .mailbox(MockMailbox::new(ARCHIVE).uid_validity(UidValidity::new(VALIDITY)))
        .build();
    backend.connect().await.expect("connect");
    backend
}

/// The local half: a database with an account, an INBOX and an Archive.
struct Local {
    _database: postio_storage::Database,
    connection: postio_storage::PooledConnection,
    inbox: Mailbox,
    archive: Mailbox,
}

fn local() -> Local {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let account = test_support::account(&connection);
    let inbox = test_support::mailbox(&connection, &account, INBOX);
    let archive = test_support::mailbox(&connection, &account, ARCHIVE);
    Local {
        _database: database,
        connection,
        inbox,
        archive,
    }
}

/// The default policy, with the poll interval `[sync]` ships.
fn policy() -> WatchPolicy {
    WatchPolicy {
        idle: true,
        idle_refresh: Duration::from_secs(60),
        poll_interval: Duration::from_secs(300),
    }
}

async fn capabilities(backend: &MockBackend) -> Capabilities {
    backend.capabilities().await.expect("capabilities")
}

/// A watcher pushing on INBOX and polling Archive.
async fn watcher(backend: &MockBackend, local: &Local) -> Watcher {
    let mut watcher = Watcher::new(policy(), &capabilities(backend).await);
    watcher.watch(local.inbox.id, INBOX, Attention::Push);
    watcher.watch(local.archive.id, ARCHIVE, Attention::Poll);
    watcher
}

/// Runs the first push step, which is always a verifying `STATUS`, so a test
/// about what happens *afterwards* can start from a watcher that is idling.
async fn settle(
    watcher: &mut Watcher,
    backend: &MockBackend,
    inbox: MailboxId,
    now: DateTime<Utc>,
) {
    assert!(
        matches!(watcher.next_push(now), Watch::Poll { .. }),
        "the first push step verifies rather than idling blind"
    );
    let status = backend.status(INBOX).await.expect("status");
    watcher.observed(inbox, &status, now);
}

// ---------------------------------------------------------------------------
// New mail arrives without anybody asking — the acceptance criterion
// ---------------------------------------------------------------------------

#[tokio::test]
async fn mail_that_arrives_while_idling_reaches_the_local_store() {
    let backend = server(2).await;
    let local = local();
    sync_mailbox(
        &local.connection,
        &backend,
        &local.inbox,
        &CancelToken::new(),
        |_| {},
    )
    .await
    .expect("bootstrap");

    let mut watcher = watcher(&backend, &local).await;
    settle(&mut watcher, &backend, local.inbox.id, at(0)).await;

    let Watch::Idle {
        path,
        timeout,
        cancel,
        ..
    } = watcher.next_push(at(1))
    else {
        panic!("a verified mailbox is idled on");
    };
    assert_eq!(path, INBOX);

    // Mail lands while the IDLE is outstanding.
    let delivery = backend.clone();
    tokio::spawn(async move {
        delivery
            .append(INBOX, &AppendMessage::new(note(3)))
            .await
            .expect("deliver");
    });

    let events = backend
        .idle(INBOX, timeout, &cancel)
        .await
        .expect("idle returns");
    assert!(
        !events.is_empty(),
        "the server announced the delivery, so IDLE must not report silence"
    );
    assert!(
        watcher.woke(local.inbox.id, &events, at(2)).needs_resync(),
        "an announced change needs a pull"
    );

    resync_mailbox(
        &local.connection,
        &backend,
        &local.inbox,
        &CancelToken::new(),
        |_| {},
    )
    .await
    .expect("resync");

    assert_eq!(
        MessageRepository::new(&local.connection)
            .uids_in(local.inbox.id, UidValidity::new(VALIDITY))
            .expect("uids")
            .len(),
        3,
        "the message that arrived during IDLE must be local now, with nobody \
         having pressed refresh"
    );
}

// ---------------------------------------------------------------------------
// Re-arming — the acceptance criterion
// ---------------------------------------------------------------------------

#[tokio::test]
async fn idle_re_arms_well_inside_the_servers_timeout() {
    let backend = server(1).await;
    let local = local();
    let mut watcher = watcher(&backend, &local).await;

    // A server that drops an IDLE it has not heard from in this long. RFC 2177
    // §3 caps a client at 29 minutes; middle-boxes are stricter.
    let tolerance = TimeDelta::minutes(5);

    let mut now = at(0);
    let mut last_spoke = now;
    let mut idles = 0;

    // An hour of a mailbox where nothing at all happens.
    while now < at(3_600) {
        match watcher.next_push(now) {
            Watch::Idle { timeout, .. } => {
                idles += 1;
                let held = TimeDelta::from_std(timeout).expect("a sane timeout");
                assert!(
                    held < tolerance,
                    "an IDLE held for {held} outlives a server that drops at \
                     {tolerance}: the connection goes deaf with no error"
                );
                now += held;
                watcher.woke(local.inbox.id, &[], now);
                last_spoke = now;
            }
            Watch::Poll { .. } => {
                let status = backend.status(INBOX).await.expect("status");
                watcher.observed(local.inbox.id, &status, now);
                last_spoke = now;
            }
            Watch::Wait { until } => now = until.expect("a watching push lane always comes back"),
        }
        assert!(
            now - last_spoke <= tolerance,
            "the connection sat silent for {}; past that the server drops it \
             and new mail simply stops appearing",
            now - last_spoke
        );
    }

    assert!(idles > 1, "the watcher must re-arm, not idle once and stop");
}

#[tokio::test]
async fn a_server_that_accepts_idle_and_then_says_nothing_cannot_hide_mail() {
    let backend = server(1).await;
    let local = local();
    sync_mailbox(
        &local.connection,
        &backend,
        &local.inbox,
        &CancelToken::new(),
        |_| {},
    )
    .await
    .expect("bootstrap");

    let mut watcher = watcher(&backend, &local).await;
    settle(&mut watcher, &backend, local.inbox.id, at(0)).await;

    // Mail arrives, and this server never says so: the untagged EXISTS is
    // simply not sent, which is indistinguishable from a quiet mailbox.
    backend
        .append(INBOX, &AppendMessage::new(note(2)))
        .await
        .expect("deliver");
    backend
        .idle(INBOX, Duration::from_millis(1), &CancelToken::new())
        .await
        .expect("swallow the announcement this server would not have sent");

    let mut now = at(1);
    let mut noticed = false;
    while now <= at(600) && !noticed {
        match watcher.next_push(now) {
            Watch::Idle { timeout, .. } => {
                now += TimeDelta::from_std(timeout).expect("a sane timeout");
                // The deaf server: silence, every time.
                watcher.woke(local.inbox.id, &[], now);
            }
            Watch::Poll { .. } => {
                let status = backend.status(INBOX).await.expect("status");
                noticed = watcher
                    .observed(local.inbox.id, &status, now)
                    .needs_resync();
            }
            Watch::Wait { until } => now = until.expect("still watching"),
        }
    }

    assert!(
        noticed,
        "a mailbox that is only ever idled on is only as reliable as the \
         server's IDLE; the poll interval is the floor under that"
    );

    resync_mailbox(
        &local.connection,
        &backend,
        &local.inbox,
        &CancelToken::new(),
        |_| {},
    )
    .await
    .expect("resync");
    assert_eq!(
        MessageRepository::new(&local.connection)
            .uids_in(local.inbox.id, UidValidity::new(VALIDITY))
            .expect("uids")
            .len(),
        2
    );
}

// ---------------------------------------------------------------------------
// Suspend and resume — the acceptance criterion
// ---------------------------------------------------------------------------

#[tokio::test]
async fn suspending_stops_the_idle_in_flight_and_hands_out_no_second_one() {
    let backend = server(1).await;
    let local = local();
    let mut watcher = watcher(&backend, &local).await;
    settle(&mut watcher, &backend, local.inbox.id, at(0)).await;

    let Watch::Idle {
        cancel, timeout, ..
    } = watcher.next_push(at(1))
    else {
        panic!("expected an IDLE");
    };

    // The lid closes.
    watcher.suspend();
    assert!(
        cancel.is_cancelled(),
        "a suspend that leaves the IDLE running leaves a connection open on a \
         machine that is going to sleep"
    );
    assert!(
        matches!(watcher.next_push(at(2)), Watch::Wait { until: None }),
        "a suspended watcher issues nothing"
    );

    // The cancelled IDLE returns promptly and empty, and reports back.
    let events = backend
        .idle(INBOX, timeout, &cancel)
        .await
        .expect("a cancelled idle is not a failure");
    assert!(events.is_empty());
    watcher.woke(local.inbox.id, &events, at(3));

    assert!(
        matches!(watcher.next_push(at(4)), Watch::Wait { until: None }),
        "still suspended"
    );
}

#[tokio::test]
async fn resuming_re_arms_exactly_one_idle() {
    let backend = server(1).await;
    let local = local();
    let mut watcher = watcher(&backend, &local).await;
    settle(&mut watcher, &backend, local.inbox.id, at(0)).await;

    let Watch::Idle { .. } = watcher.next_push(at(1)) else {
        panic!("expected an IDLE");
    };

    watcher.suspend();
    watcher.resume(at(600));

    assert!(
        matches!(watcher.next_push(at(600)), Watch::Wait { until: None }),
        "the suspended IDLE has not reported back yet, so re-arming now would \
         open a second connection to the same mailbox"
    );

    watcher.woke(local.inbox.id, &[], at(601));
    let step = watcher.next_push(at(601));
    assert!(
        matches!(step, Watch::Poll { .. } | Watch::Idle { .. }),
        "a resumed watcher goes back to work, got {step:?}"
    );
}

#[tokio::test]
async fn an_unreported_step_is_never_handed_out_twice() {
    let backend = server(1).await;
    let local = local();
    let mut watcher = watcher(&backend, &local).await;

    assert!(matches!(watcher.next_push(at(0)), Watch::Poll { .. }));
    assert!(
        matches!(watcher.next_push(at(0)), Watch::Wait { until: None }),
        "handing the same mailbox out twice is how a second connection is \
         opened to it"
    );
}

// ---------------------------------------------------------------------------
// Capability gating
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_server_without_idle_is_polled_instead() {
    let backend = MockBackend::builder()
        .capabilities(["IMAP4rev1", "CONDSTORE", "QRESYNC", "UIDPLUS"])
        .mailbox(MockMailbox::new(INBOX))
        .build();
    backend.connect().await.expect("connect");
    let local = local();

    let mut watcher = Watcher::new(policy(), &capabilities(&backend).await);
    watcher.watch(local.inbox.id, INBOX, Attention::Push);
    settle(&mut watcher, &backend, local.inbox.id, at(0)).await;

    // An IDLE-capable watcher would idle here.
    match watcher.next_push(at(1)) {
        Watch::Wait { until: Some(until) } => assert_eq!(
            until,
            at(300),
            "without IDLE the mailbox is on the poll interval like any other"
        ),
        other => panic!("a server that cannot IDLE must never be idled on: {other:?}"),
    }
}

#[tokio::test]
async fn idle_turned_off_in_configuration_is_honoured() {
    let backend = server(1).await;
    let local = local();

    let mut watcher = Watcher::new(
        WatchPolicy {
            idle: false,
            ..policy()
        },
        &capabilities(&backend).await,
    );
    watcher.watch(local.inbox.id, INBOX, Attention::Push);
    settle(&mut watcher, &backend, local.inbox.id, at(0)).await;

    assert!(
        matches!(watcher.next_push(at(1)), Watch::Wait { .. }),
        "`[sync] idle = false` means poll, whatever the server can do"
    );
}

// ---------------------------------------------------------------------------
// Everything that is not the pushed mailbox
// ---------------------------------------------------------------------------

#[tokio::test]
async fn other_mailboxes_are_polled_on_the_interval_and_not_before() {
    let backend = server(1).await;
    let local = local();
    let mut watcher = watcher(&backend, &local).await;

    let Watch::Poll { mailbox, path } = watcher.next_poll(at(0)) else {
        panic!("the first pass looks at every mailbox");
    };
    assert_eq!(mailbox, local.archive.id);
    assert_eq!(path, ARCHIVE);

    let status = backend.status(ARCHIVE).await.expect("status");
    watcher.observed(local.archive.id, &status, at(0));

    match watcher.next_poll(at(299)) {
        Watch::Wait { until: Some(until) } => assert_eq!(until, at(300)),
        other => panic!("polling a quiet folder every second is not polling: {other:?}"),
    }
    assert!(matches!(watcher.next_poll(at(300)), Watch::Poll { .. }));
}

#[tokio::test]
async fn a_poll_that_sees_nothing_new_asks_for_no_resync() {
    let backend = server(1).await;
    let local = local();
    let mut watcher = watcher(&backend, &local).await;

    watcher.next_poll(at(0));
    let status = backend.status(ARCHIVE).await.expect("status");
    assert!(
        watcher
            .observed(local.archive.id, &status, at(0))
            .needs_resync(),
        "the first look at a mailbox is always worth a pull: we have just \
         connected and do not know what we missed"
    );

    watcher.next_poll(at(300));
    let status = backend.status(ARCHIVE).await.expect("status");
    assert!(
        !watcher
            .observed(local.archive.id, &status, at(300))
            .needs_resync(),
        "an unchanged folder must cost one STATUS and nothing else"
    );
}

#[tokio::test]
async fn a_poll_that_sees_a_new_message_asks_for_a_resync() {
    let backend = server(1).await;
    let local = local();
    let mut watcher = watcher(&backend, &local).await;

    watcher.next_poll(at(0));
    let status = backend.status(ARCHIVE).await.expect("status");
    watcher.observed(local.archive.id, &status, at(0));

    backend
        .append(ARCHIVE, &AppendMessage::new(note(9)))
        .await
        .expect("deliver");

    watcher.next_poll(at(300));
    let status = backend.status(ARCHIVE).await.expect("status");
    assert!(
        watcher
            .observed(local.archive.id, &status, at(300))
            .needs_resync()
    );
}

#[tokio::test]
async fn the_pushed_mailbox_is_not_also_polled_on_the_shared_connection() {
    let backend = server(1).await;
    let local = local();
    let mut watcher = watcher(&backend, &local).await;

    let mut seen = Vec::new();
    for step in 0..4 {
        if let Watch::Poll { mailbox, .. } = watcher.next_poll(at(step * 300)) {
            seen.push(mailbox);
            let status = backend.status(ARCHIVE).await.expect("status");
            watcher.observed(mailbox, &status, at(step * 300));
        }
    }

    assert!(!seen.is_empty(), "the shared lane must do something");
    assert!(
        !seen.contains(&local.inbox.id),
        "the pushed mailbox belongs to the IDLE connection; polling it from \
         the shared one is the duplicate work IDLE exists to avoid"
    );
}

// ---------------------------------------------------------------------------
// Failure
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_failed_step_releases_the_mailbox_rather_than_wedging_it() {
    let backend = server(1).await;
    let local = local();
    let mut watcher = watcher(&backend, &local).await;

    watcher.next_push(at(0));
    backend.inject(Fault::Disconnect);
    backend.status(INBOX).await.expect_err("dropped");
    watcher.failed(local.inbox.id, at(0));

    // The mailbox is not wedged in flight forever; it comes back on the
    // interval, by which time the supervisor has had its say about the link.
    match watcher.next_push(at(1)) {
        Watch::Wait { until: Some(until) } => assert_eq!(until, at(300)),
        other => panic!("a failed step must not be retried instantly: {other:?}"),
    }
    assert!(matches!(watcher.next_push(at(300)), Watch::Poll { .. }));
}

#[tokio::test]
async fn a_vanished_message_reported_by_idle_is_a_change_like_any_other() {
    let backend = server(1).await;
    let local = local();
    let mut watcher = watcher(&backend, &local).await;

    let wake = watcher.woke(local.inbox.id, &[MailboxEvent::Expunged { seq: 1 }], at(0));

    assert!(
        wake.needs_resync(),
        "IDLE says only *that* something happened; the answer to any event is \
         a pull, never applying it as a diff"
    );
}
