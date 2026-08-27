//! The list hears about a sync *while* it is running, not only when it ends.
//!
//! `postio-qhz.7`: on a live account with 81,716 messages in the store, the
//! message list showed nothing. The store, the query and the widget were all
//! correct — `crates/postio-gtk/src/list.rs` has the whole invalidation
//! machinery, and it reloads on `Event::MessageListChanged`. It was simply
//! never told.
//!
//! The engine collected every [`Progress`] report a pass produced into a
//! `Vec` and processed it after the pass returned. That is fine for the ten
//! messages a mock holds and wrong for a real mailbox: an initial sync of
//! 60,000 messages commits three hundred batches and emitted **nothing at
//! all** until the last one landed — no progress on the status line and no
//! rows in the list, for the whole of a sync that takes minutes.
//!
//! # What is asserted, and why it is a range
//!
//! Two properties, and they pull against each other:
//!
//! * The list is told *during* the pass, so rows appear as they arrive.
//! * It is not told *per batch*. Each notification costs the list a reload,
//!   and three hundred of them would blow the 16 ms interaction budget for
//!   the whole of a first sync — which is exactly when the application most
//!   needs to feel alive.
//!
//! So a pass of five batches must produce more than one event and far fewer
//! than five. See `postio_runtime::engine::REPAINT_INTERVAL`.
//!
//! # The clock is fake (#507)
//!
//! That "far fewer than five" used to be measured against
//! [`std::time::Instant::now`], so the number of events depended on how much
//! real wall-clock time separated batches committing — fine on an idle
//! machine, but a busy one can stretch that past `REPAINT_INTERVAL` for a
//! batch that would otherwise have coalesced, which is exactly what made
//! this test's own event count vary under load. [`FakeClock`] steps a fixed
//! amount every call instead, so the count this test asserts on is a
//! function of how many batches committed and nothing else.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use postio_core::Event;
use postio_core::bridge::{EventStream, event_channel};
use postio_imap::backend::{MockBackend, MockMailbox, MockMessage};
use postio_model::MailboxRole;
use postio_runtime::engine::{Clock, Engine, EngineParts, NetworkSource};
use postio_storage::{BlobStore, test_support};

/// Enough to need several batches: `postio_sync::initial::DEFAULT_BATCH_SIZE`
/// is 200, so this is five of them.
const MESSAGES: u32 = 1_000;

/// A clock that advances by a fixed `step` every call, regardless of how
/// much real time actually passed.
///
/// Five batches make five calls (one per commit, from `Committed::batch`),
/// so with a 200ms step and `REPAINT_INTERVAL` at 500ms the batches land at
/// 0, 200, 400, 600 and 800ms: the first is announced at once, the second
/// and third coalesce into it, and the fourth opens the next window — two
/// events, every run, on any machine.
struct FakeClock {
    base: Instant,
    step: Duration,
    calls: AtomicU32,
}

impl FakeClock {
    fn new(step: Duration) -> Self {
        FakeClock {
            base: Instant::now(),
            step,
            calls: AtomicU32::new(0),
        }
    }
}

impl Clock for FakeClock {
    fn now(&self) -> Instant {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        self.base + self.step * call
    }
}

fn server() -> MockBackend {
    let message = |n: u32| {
        format!(
            "From: Ada Lovelace <ada@example.com>\r\n\
             To: Postio <postio@example.net>\r\n\
             Subject: message {n}\r\n\
             Message-ID: <m-{n}@example.com>\r\n\
             Date: Mon, 1 Jun 2026 09:00:00 +0000\r\n\
             \r\n\
             Body {n}.\r\n"
        )
        .into_bytes()
    };
    let mut inbox = MockMailbox::new("INBOX");
    for n in 1..=MESSAGES {
        inbox = inbox.message(MockMessage::new(message(n)));
    }
    MockBackend::builder().mailbox(inbox).build()
}

fn drain(events: &EventStream) -> Vec<Event> {
    let mut seen = Vec::new();
    while let Some(event) = events.try_next() {
        seen.push(event);
    }
    seen
}

#[tokio::test(flavor = "multi_thread")]
async fn a_long_sync_tells_the_list_as_it_goes_and_not_once_per_batch() {
    let database = test_support::memory();
    let (account, inbox) = {
        let connection = database.connection().expect("a connection");
        let account = test_support::account(&connection);
        let inbox = test_support::mailbox(&connection, &account, "INBOX");
        (account, inbox)
    };
    assert_eq!(inbox.role, MailboxRole::Inbox);

    let directory = tempfile::tempdir().expect("a blob directory");
    let blobs = BlobStore::open(directory.keep()).expect("a blob store");
    let (sink, events) = event_channel();

    let engine = Engine::spawn(EngineParts {
        account: account.id,
        database: database.clone(),
        blobs,
        backend: Arc::new(server()),
        smtp: Arc::new(postio_smtp::transport::RustlsConnector::new().expect("a connector")),
        tokens: Arc::new(postio_imap::auth::StoredPasswordSource::new(Arc::new(
            postio_imap::secret::MemorySecretStore::default(),
        ))),
        events: sink,
        retry: Default::default(),
        backfill: Default::default(),
        reconnect: Default::default(),
        watch: Default::default(),
        network: NetworkSource::Ignored,
        mailbox_roles: Default::default(),
        clock: Arc::new(FakeClock::new(Duration::from_millis(200))),
    })
    .expect("the engine starts");

    let summary = engine.sync(inbox.id).await.expect("a sync pass");
    assert!(
        summary.inserted >= MESSAGES as usize,
        "the fixture did not actually sync a large mailbox: {summary:?}"
    );

    let told = drain(&events)
        .iter()
        .filter(
            |event| matches!(event, Event::MessageListChanged { mailbox, .. } if *mailbox == inbox.id),
        )
        .count();

    assert!(
        told > 1,
        "the list was told {told} time(s) — only at the end of the pass, so a \
         real first sync leaves it empty for as long as the sync takes"
    );
    // Five batches, a fixed 200ms apart on the fake clock: this is 2, on any
    // machine (see `FakeClock`'s own doc comment). The range stays loose
    // rather than tightening to `assert_eq!` so a change to
    // `DEFAULT_BATCH_SIZE` fails here with a comprehensible number rather
    // than a mysterious one -- but it no longer depends even a little on how
    // fast this machine happened to run the pass.
    assert!(
        told <= 3,
        "the list was told {told} times for five batches; that is close enough \
         to per-batch to cost a reload each time"
    );
}
