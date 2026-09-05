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

/// What that error message actually means, pinned down (#710).
///
/// The branch this test arrived on said the message was "SQLCipher's generic
/// text for *any* failed key pragma, printed whatever the cause", and that
/// the issue title's reading -- that the key was empty -- was never the
/// likely one. Measured here, both halves of that are wrong, and it matters:
/// "generic" sends the next investigation away from the statement text,
/// which is the one place left to look.
///
/// Exactly one input shape produces it, and it is the literal one: a key
/// pragma whose **value string is zero-length**. In particular an empty
/// *hex payload* -- `"x''"`, which is what Postio's own `format!` would
/// produce from an empty key -- is **accepted**, so this error cannot be
/// reached by any value of the key at all.
///
/// Which is what makes it a useful signature rather than noise. Postio's
/// `db::configure` builds the statement as
/// `format!("PRAGMA key = \"x'{}'\";", *hex)` from a `Subkey` that is a
/// fixed-size `[u8; KEY_BYTES]` array, so `hex` has a fixed length and the
/// text has a compile-time constant shape. It cannot be `PRAGMA key = ""`.
/// Postio also never issues `PRAGMA rekey` anywhere, so the second half of
/// the message is not about this codebase either.
///
/// So a run that reports it is reporting that SQLCipher parsed a statement
/// Postio cannot have written -- which points back at the issue's own first
/// instinct, something going wrong below the Rust layer, and away from key
/// derivation entirely.
#[test]
fn an_empty_key_string_produces_the_reported_error() {
    let cases = [
        ("an empty double-quoted key", "PRAGMA key = \"\";", true),
        ("an empty single-quoted key", "PRAGMA key = '';", true),
        // What `format!` produces from an empty hex rendering. Accepted --
        // SQLCipher takes it as the four-character passphrase `x''` rather
        // than as zero bytes of hex.
        ("an empty hex payload", "PRAGMA key = \"x''\";", false),
        (
            "an ordinary hex key",
            "PRAGMA key = \"x'5a5a5a5a'\";",
            false,
        ),
    ];

    for (name, sql, expected_to_fail) in cases {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let connection =
            rusqlite::Connection::open(directory.path().join("probe.db")).expect("open");
        // The order `db::configure` uses: SQLCipher wants this before the key.
        connection
            .execute_batch("PRAGMA cipher_memory_security = OFF;")
            .expect("the pragma that goes before the key");

        let result = connection.execute_batch(sql);
        let text = result.as_ref().err().map(|error| error.to_string());
        let is_the_reported_error = text
            .as_deref()
            .is_some_and(|text| text.contains("PRAGMA key requires a key of one or more"));

        assert_eq!(
            is_the_reported_error, expected_to_fail,
            "{name}: a zero-length key string is one of the two ways into \
             #710's message, and this pins which key values take it. The \
             other way -- `sqlite3_key_v2` returning an error, with a \
             perfectly good key -- is `tests/key_pragma_failure.rs`, and it \
             is why this case no longer claims `and nothing else`. \
             Got {text:?}"
        );
    }
}
