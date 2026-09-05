//! Opening many encrypted databases at once (#710).
//!
//! A full-workspace run under load produced one `PRAGMA key` failure and never
//! reproduced it. The message SQLCipher printed —
//!
//! ```text
//! An error occurred with PRAGMA key or rekey. PRAGMA key requires a key of
//! one or more characters.
//! ```
//!
//! reads like a claim that the key was empty, and is not: it is SQLCipher's
//! generic text for *any* failed key pragma, printed whatever the cause. The
//! key itself is a constant (`test_support::key()` derives from a fixed
//! master), so "the key was empty" was never the likely reading. What was
//! left is the issue's own hypothesis: something in the codec or in
//! libcrypto going wrong when many `Database::open` calls land at once.
//!
//! This exercises that directly instead of waiting for a flake. A workspace
//! run opens hundreds of these across many test binaries; this opens them in
//! one process, on many threads, as fast as it can.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use postio_storage::test_support;

/// How many threads open databases at once.
///
/// Above the core count on purpose: the interesting moment is threads being
/// descheduled part-way through an open, which is what a loaded machine does
/// to a full-workspace run and what a comfortable thread count would not.
const THREADS: usize = 16;

/// How many each opens.
const PER_THREAD: usize = 12;

#[test]
fn many_databases_open_at_once_without_a_key_failure() {
    let failures = Arc::new(AtomicUsize::new(0));
    let first = Arc::new(std::sync::Mutex::new(None::<String>));

    std::thread::scope(|scope| {
        for _ in 0..THREADS {
            let failures = Arc::clone(&failures);
            let first = Arc::clone(&first);
            scope.spawn(move || {
                for _ in 0..PER_THREAD {
                    // `temp()` panics on failure, which is the shape the
                    // issue reported. Catching it keeps every thread going so
                    // one run reports how *often* it happens rather than
                    // stopping at the first.
                    let opened = std::panic::catch_unwind(test_support::temp);
                    if let Err(panic) = opened {
                        failures.fetch_add(1, Ordering::SeqCst);
                        let text = panic
                            .downcast_ref::<String>()
                            .cloned()
                            .or_else(|| panic.downcast_ref::<&str>().map(|s| (*s).to_owned()))
                            .unwrap_or_else(|| "a panic with no message".to_owned());
                        *first.lock().expect("the first failure") = Some(text);
                    }
                }
            });
        }
    });

    let count = failures.load(Ordering::SeqCst);
    assert_eq!(
        count,
        0,
        "{count} of {} concurrent opens failed; the first said: {}",
        THREADS * PER_THREAD,
        first
            .lock()
            .expect("the first failure")
            .clone()
            .unwrap_or_default()
    );
}
