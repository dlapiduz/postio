use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use postio_storage::test_support;

const TASKS: usize = 16;

const PER_TASK: usize = 12;

/// POSTIO-MEASUREMENT: opens 192 databases at once, 29 s on the reference machine; its answer is a failure count.
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
                // `temp()` panics on failure; catching it keeps every task going so one run reports
                // how
                // often it happens.
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
