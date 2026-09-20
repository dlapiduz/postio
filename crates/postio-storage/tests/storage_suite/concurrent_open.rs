//! Opening many encrypted databases at once (#710).
//!
//! A full-workspace run under load once produced one `PRAGMA key` failure and
//! never reproduced it. That message was SQLCipher's, and SQLCipher is gone —
//! but the shape of the worry is not the cipher's, it is concurrency's: a
//! workspace run opens hundreds of encrypted stores across many test binaries,
//! and the interesting moment is a task being descheduled part-way through an
//! open.
//!
//! So this outlives the engine that prompted it. It opens them in one process,
//! on many tasks, as fast as it can, and asserts that every one of them comes
//! back usable.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use postio_storage::test_support;

/// How many tasks open databases at once.
///
/// Above the core count on purpose: see the module documentation.
const TASKS: usize = 16;

/// How many each opens.
const PER_TASK: usize = 12;

#[tokio::test(flavor = "multi_thread")]
async fn many_databases_open_at_once_without_a_key_failure() {
    let failures = Arc::new(AtomicUsize::new(0));
    let first = Arc::new(std::sync::Mutex::new(None::<String>));

    let mut opens = Vec::new();
    for _ in 0..TASKS {
        let failures = Arc::clone(&failures);
        let first = Arc::clone(&first);
        opens.push(tokio::spawn(async move {
            for _ in 0..PER_TASK {
                // `temp()` panics on failure, which is the shape the issue
                // reported. Catching it keeps every task going, so one run
                // reports how *often* it happens rather than stopping at the
                // first.
                let opened = tokio::task::spawn(async { test_support::temp().await }).await;
                if let Err(panic) = opened {
                    failures.fetch_add(1, Ordering::SeqCst);
                    *first.lock().expect("the first failure") = Some(panic.to_string());
                }
            }
        }));
    }
    for open in opens {
        open.await.expect("a task that opens stores");
    }

    let count = failures.load(Ordering::SeqCst);
    assert_eq!(
        count,
        0,
        "{count} of {} concurrent opens failed; the first said: {}",
        TASKS * PER_TASK,
        first
            .lock()
            .expect("the first failure")
            .clone()
            .unwrap_or_default()
    );
}
