//! Counting `io-imap`'s silently skipped untagged responses.
//!
//! `io-imap`'s send primitive drops any untagged response line it cannot
//! decode rather than failing the command (`send.rs`, added for
//! pimalaya/himalaya#641), logging only
//! `debug!("skipping undecodable untagged response")` at the `io_imap`
//! target plus a `trace!` of the raw bytes. iCloud has historically sent
//! malformed FETCH sequence numbers under QRESYNC (Apple Developer Forums
//! thread 694251); `imap_types` models a sequence number as `NonZeroU32`, so
//! such a line cannot decode. The result is silent: a `CHANGEDSINCE` FETCH —
//! the incremental resync primitive [`super::fetch_headers`] implements —
//! completes `Ok`, having quietly dropped one or more of the deltas it was
//! sent to report. See ADR 0001 and this crate's parent bead.
//!
//! There is no other hook into this behaviour: it is a `debug!()` log
//! record and nothing else, so this module installs a [`log::Log`] that
//! watches for exactly that record, counts it, and forwards every record —
//! including that one — to whatever logger the application already
//! installed. [`super::fetch_headers`] snapshots the counter around its own
//! `CHANGEDSINCE` round trip and turns a nonzero delta into
//! [`BackendError::ResyncIntegrityLost`](crate::backend::BackendError::ResyncIntegrityLost),
//! whose [`requires_full_resync`](crate::backend::BackendError::requires_full_resync)
//! already tells a caller to fall back to a full resync — the same
//! predicate a `UIDVALIDITY` change reports through.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};

use log::{Level, LevelFilter, Log, Metadata, Record};

/// The crate prefix `io-imap`'s log records carry.
///
/// `send.rs` calls `debug!("skipping undecodable untagged response")` with
/// no explicit `target:`, so the record's target is whatever module emitted
/// it (`io_imap::send`), not the bare crate name — a prefix match, the same
/// as an `RUST_LOG=io_imap=debug` directive would apply in a real logging
/// framework, not an exact one.
const TARGET_PREFIX: &str = "io_imap";

/// A substring of the message `io-imap`'s `send.rs` logs when it drops an
/// untagged response it could not decode.
const SKIPPED_UNTAGGED: &str = "skipping undecodable untagged response";

fn is_io_imap_target(target: &str) -> bool {
    target.starts_with(TARGET_PREFIX)
}

static COUNT: AtomicU64 = AtomicU64::new(0);
static INSTALLED: OnceLock<()> = OnceLock::new();

struct SkipCountingLogger {
    inner: &'static dyn Log,
}

impl Log for SkipCountingLogger {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        self.inner.enabled(metadata)
            || (is_io_imap_target(metadata.target()) && metadata.level() <= Level::Debug)
    }

    fn log(&self, record: &Record<'_>) {
        if is_io_imap_target(record.target())
            && record.level() == Level::Debug
            && record.args().to_string().contains(SKIPPED_UNTAGGED)
        {
            COUNT.fetch_add(1, Ordering::Relaxed);
        }
        self.inner.log(record);
    }

    fn flush(&self) {
        self.inner.flush();
    }
}

/// Installs the counting logger. Idempotent — safe to call more than once,
/// which every test in this crate that touches it does.
///
/// # Call this after the application sets up its own logger
///
/// `log::set_logger` succeeds only once per process. This function captures
/// whatever [`log::logger()`] returns *at the time it runs* as the logger it
/// forwards every record to, so calling it before the application installs
/// its own (`env_logger`, `tracing-log`, or similar) would make this the
/// *only* logger for the rest of the process, silently swallowing the
/// application's own log output.
///
/// # Why this raises the process's log level
///
/// The `log` crate's macros drop a record before any installed [`Log`] is
/// even consulted unless the process's max level admits it — there is no
/// per-target filter at this layer. Since the skip this module exists to
/// catch is logged at `debug!` and nothing else observes it, catching it
/// needs that level enabled everywhere this crate ships, release builds
/// included: this is a resync-correctness guard, not a diagnostic left on
/// for developers. A `release_max_level_*` feature enabled anywhere in the
/// dependency graph would compile the record out before this runtime check
/// ever applies; none is enabled in this workspace today.
pub fn install() {
    INSTALLED.get_or_init(|| {
        let inner = log::logger();
        // `set_boxed_logger` leaks its box to obtain a `&'static dyn Log`
        // internally, so `log::logger()` keeps returning this instance for
        // the rest of the process from here on — the recursion in `inner`
        // terminates at whatever was registered before this call.
        let _ = log::set_boxed_logger(Box::new(SkipCountingLogger { inner }));
        log::set_max_level(log::max_level().max(LevelFilter::Debug));
    });
}

/// How many undecodable untagged responses `io-imap` has skipped since the
/// process started (or since whenever [`install`] first ran — the counter
/// is inert until then).
///
/// Process-wide by construction, since there is exactly one [`Log`] per
/// process: a caller cannot ask "how many during just this call" any other
/// way, so [`super::fetch_headers`] snapshots this before its own round trip
/// and compares after. A nonzero delta means that specific call may have
/// silently missed a delta and its result cannot be trusted as a complete
/// incremental pull.
pub fn skipped_untagged_responses() -> u64 {
    COUNT.load(Ordering::Relaxed)
}

/// Serializes callers that need an exclusive before/after snapshot of
/// [`skipped_untagged_responses`].
///
/// The counter has no per-operation scope — there is exactly one `Log` per
/// process, so a skip during *any* concurrent `io-imap` command lands in the
/// same counter. Two resync-shaped fetches measuring a delta at the same
/// time would each risk attributing the other's skip to itself. Holding
/// this for "snapshot, run the fetch, snapshot again" serializes those
/// measurements against each other without affecting anything that doesn't
/// take this lock — an ordinary fetch with no `CHANGEDSINCE` never touches
/// it. A `CHANGEDSINCE` fetch is not a hot path, so trading its parallelism
/// with itself for a delta that is actually correct is the right side of
/// this trade.
///
/// An async mutex, not `std::sync::Mutex`: the guard is held across the
/// fetch's own `.await` points.
///
/// Exported for tests above this module, not only [`super::fetch_headers`]:
/// a test proving an undecodable line is *tolerated* outside a
/// `CHANGEDSINCE` fetch still makes `io-imap` skip a real line and bump the
/// real counter — production code has no reason to bracket that path, but a
/// concurrently-running test measuring a delta of its own does, or it would
/// intermittently see this test's skip as its own.
pub async fn exclusive_measurement() -> tokio::sync::MutexGuard<'static, ()> {
    static LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    LOCK.lock().await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_logger_counts_only_the_exact_record_it_exists_to_catch() {
        // One test function, not several: `install` and the counter are
        // process-wide, and `cargo test` runs functions within one binary
        // concurrently by default, so splitting these into separate `#[test]`
        // functions would race on the same global counter.
        install();

        let before = skipped_untagged_responses();
        log::debug!(target: "io_imap", "connected");
        assert_eq!(
            skipped_untagged_responses(),
            before,
            "an unrelated io_imap debug record must not be counted"
        );

        let before = skipped_untagged_responses();
        log::debug!(target: "something_else", "skipping undecodable untagged response");
        assert_eq!(
            skipped_untagged_responses(),
            before,
            "the same message from a different target must not be counted"
        );

        let before = skipped_untagged_responses();
        log::debug!(target: "io_imap", "skipping undecodable untagged response: {:?}", b"* -1 FETCH");
        assert_eq!(
            skipped_untagged_responses(),
            before + 1,
            "the exact record io-imap logs for a dropped line must be counted"
        );
    }
}
