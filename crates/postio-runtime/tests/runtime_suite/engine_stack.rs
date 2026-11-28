use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

use postio_account::backend::{MockBackend, MockMailbox, MockMessage};
use postio_runtime::engine::{Engine, EngineParts, NetworkSource, SystemClock};
use postio_storage::seed::seed_small;
use postio_storage::{BlobStore, test_support};

const CHILD: &str = "POSTIO_ENGINE_STACK_CHILD";

/// Comfortably under what a sync pass needs, but high enough for the harness's own threads.
const AMBIENT_STACK: usize = 512 * 1024;

const CASE: &str = "a_sync_pass_does_not_run_on_whatever_stack_it_was_given";

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
        .args([
            "--exact",
            postio_test_support::this_test!(CASE).as_str(),
            "--nocapture",
            "--test-threads=1",
        ])
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
