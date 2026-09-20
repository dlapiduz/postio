//! The sync engine's thread does not borrow the ambient stack size (#1541).
//!
//! `Engine::spawn` puts the whole engine on a thread of its own and then
//! `block_on`s the sync loop there, so every frame of `sync_wave` →
//! `sync_pass` → `resync_mailbox` → the backend's `select` future is stacked
//! on that one thread. A `std::thread` spawned without `stack_size` gets
//! Rust's **2 MiB** default — not the 8 MiB the kernel gives the main thread
//! from `RLIMIT_STACK` — and nothing here ever asked for more.
//!
//! Measured on this workspace before the fix, by sweeping `RUST_MIN_STACK`
//! until a sync pass stopped fitting:
//!
//! | path | peak stack for one trivial sync |
//! |---|---|
//! | `MockBackend` | between 512 KiB and 768 KiB |
//! | real IMAP over loopback (`app_suite::attach_account`) | between 1024 KiB and 1088 KiB |
//!
//! So the smallest sync this project can express — one mailbox, one message,
//! no latency — already spends about **half** of the engine thread's ceiling,
//! and what remains has to cover a real account: a mailbox tree rather than
//! one folder, several passes overlapping inside `sync_wave`'s
//! `FuturesUnordered`, and a server response deep enough to parse. Three
//! aborts in two minutes on 2026-09-17 are what running out looks like —
//! `app_suite::attach_account` died with
//! `std::sys::pal::unix::stack_overflow::imp::signal_handler` on the frame
//! above `abort`, which is Rust's guard-page handler and not, as #1541
//! assumed from the `free(): corrupted unsorted chunks` seen in a separate
//! live run, memory corruption.
//!
//! # Why this runs in a child process
//!
//! `RUST_MIN_STACK` is read once and cached the first time a thread is
//! spawned, and the test harness has already spawned one by the time a case
//! runs. Setting it here would change nothing. So the case re-executes its
//! own binary with the variable set, and asserts the child got through a sync
//! pass: the fix is an explicit `stack_size`, which is precisely what makes
//! the child stop caring what the ambient default is. It is also why this is
//! a binary of its own rather than a `runtime_suite` module — that file's
//! own docs say a case needing a process-global has to stay out.

use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

use postio_account::backend::{MockBackend, MockMailbox, MockMessage};
use postio_runtime::engine::{Engine, EngineParts, NetworkSource, SystemClock};
use postio_storage::seed::seed_small;
use postio_storage::{BlobStore, test_support};

/// Set in the child, so the case knows to do the work rather than spawn again.
const CHILD: &str = "POSTIO_ENGINE_STACK_CHILD";

/// Comfortably under what a sync pass needs, and proven to abort the engine
/// thread before the fix: at this size six of `runtime_suite`'s seven
/// sync-driving cases died with `thread 'postio-sync' has overflowed its
/// stack`. Low enough to be unambiguous, high enough that the threads this
/// test is *not* about — the harness's own runtime, the store's — still run.
const AMBIENT_STACK: usize = 512 * 1024;

const CASE: &str = "a_sync_pass_does_not_run_on_whatever_stack_it_was_given";

/// One mailbox with a little mail in it. The assertion is that a pass ran at
/// all, so the cheapest server that provokes one is the right one.
fn server() -> MockBackend {
    let message = |n: u32| {
        format!(
            "From: Ada Lovelace <ada@example.com>\r\n\
             To: Postio <postio@example.net>\r\n\
             Subject: message {n}\r\n\
             Message-ID: <stack-{n}@example.com>\r\n\
             Date: Mon, 1 Jun 2026 09:00:00 +0000\r\n\
             \r\n\
             Body {n}.\r\n"
        )
        .into_bytes()
    };
    let mut inbox = MockMailbox::new("INBOX");
    for n in 1..=4 {
        inbox = inbox.message(MockMessage::new(message(n)));
    }
    MockBackend::builder().mailbox(inbox).build()
}

/// Drive one sync pass through a real `Engine`, on the engine's own thread.
async fn sync_once() {
    let database = test_support::memory().await;
    let report = seed_small(&database, 3).await;
    let directory = tempfile::tempdir().expect("a blob directory");
    let blobs = BlobStore::open(
        directory.path().to_path_buf(),
        &postio_storage::test_support::blob_keys(),
    )
    .expect("a blob store");
    let (sink, _events) = postio_core::bridge::event_channel();

    let backend = Arc::new(server());
    let engine = Engine::spawn(EngineParts {
        account: report.account.id,
        database: database.clone(),
        blobs,
        backend: backend.clone(),
        // Never dialled: nothing here queues a send.
        smtp: Arc::new(postio_smtp::transport::RustlsConnector::new().expect("a connector")),
        tokens: Arc::new(postio_account::auth::StoredPasswordSource::new(Arc::new(
            postio_account::secret::MemorySecretStore::default(),
        ))),
        events: sink,
        retry: Default::default(),
        backfill: Default::default(),
        reconnect: Default::default(),
        watch: Default::default(),
        network: NetworkSource::Ignored,
        mailbox_roles: Default::default(),
        clock: Arc::new(SystemClock),
    })
    .expect("the engine starts");

    // A header fetch is `resync_mailbox` reached and returned from, which is
    // the deepest frame in the crash and the whole point of the exercise.
    let waited = Instant::now();
    while backend.header_fetches().is_empty() {
        assert!(
            waited.elapsed() < Duration::from_secs(120),
            "no sync pass ever reached the server"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    engine.stop();
}

#[tokio::test]
async fn a_sync_pass_does_not_run_on_whatever_stack_it_was_given() {
    if std::env::var_os(CHILD).is_some() {
        sync_once().await;
        return;
    }

    let exe = std::env::current_exe().expect("a test binary has a path");
    let output = Command::new(exe)
        .args(["--exact", CASE, "--nocapture", "--test-threads=1"])
        .env(CHILD, "1")
        .env("RUST_MIN_STACK", AMBIENT_STACK.to_string())
        .output()
        .expect("the child test binary runs");

    let said = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !said.contains("has overflowed its stack"),
        "the engine thread overflowed a {} KiB ambient stack: `Engine::spawn` \
         is taking whatever `std::thread` defaults to instead of asking for a \
         stack that fits a sync pass.\n{said}",
        AMBIENT_STACK / 1024,
    );
    assert!(
        output.status.success(),
        "a sync pass under a {} KiB ambient stack did not finish: {:?}\n{said}",
        AMBIENT_STACK / 1024,
        output.status,
    );
}
