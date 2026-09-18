//! Whether a WebKit web process has died under one of this crate's views.
//!
//! WebKitGTK runs a process per view, and that process can die for reasons
//! the view never sees: an out-of-memory kill, a GPU stack that cannot come
//! up, a crash in the engine. From the application's side the only symptom
//! is silence — a document that never finishes loading, a JavaScript
//! callback that never fires. The signal that says why is
//! `web-process-terminated`, and every view this crate builds connects it
//! here, so the death is logged with its reason rather than diagnosed from
//! the absence of anything else.
//!
//! The count is what the test suites read. A test waiting on a document is
//! waiting on a process, and when that process has died the wait can only
//! end at its deadline — 240 s under nextest's cap, on the CI run that made
//! this module. `deaths` lets a wait helper notice within one turn of the
//! loop and fail saying what happened.

use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use webkit6::prelude::*;

/// How many web processes have died under this crate's views, ever.
static DEATHS: AtomicUsize = AtomicUsize::new(0);

/// Why the last one died, as WebKit reported it.
static LAST_REASON: Mutex<Option<String>> = Mutex::new(None);

/// A death nobody has acted on yet — see [`take_death`].
static PENDING: Mutex<Option<String>> = Mutex::new(None);

/// Connect `view`'s `web-process-terminated` signal, so a death is logged
/// and counted. Every `WebView` this crate builds calls this.
pub fn watch(view: &webkit6::WebView) {
    view.connect_web_process_terminated(|_, reason| {
        tracing::error!(
            ?reason,
            "a WebKit web process died ({reason:?}); the document it held is gone"
        );
        let reason = format!("{reason:?}");
        DEATHS.fetch_add(1, Ordering::SeqCst);
        if let Ok(mut last) = LAST_REASON.lock() {
            *last = Some(reason.clone());
        }
        if let Ok(mut pending) = PENDING.lock() {
            *pending = Some(reason);
        }
    });
}

/// How many web processes have died under this crate's views since the
/// process started.
///
/// Monotonic: a wait helper snapshots it on entry and fails when it moves,
/// which keeps a death in one test from failing the next one in the same
/// process.
pub fn deaths() -> usize {
    DEATHS.load(Ordering::SeqCst)
}

/// Why the last web process died, as WebKit reported it — `None` while none
/// has.
pub fn last_death() -> Option<String> {
    LAST_REASON.lock().ok().and_then(|reason| reason.clone())
}

/// A death nobody has acted on yet, taken: the reason, once.
///
/// For the test suites' wait helpers. A wait that finds one pending fails at
/// once, naming it, rather than running to its deadline for a document that
/// can no longer arrive — and taking it means the failure is charged to the
/// first wait after the death, not to every wait that follows.
pub fn take_death() -> Option<String> {
    PENDING.lock().ok().and_then(|mut pending| pending.take())
}
