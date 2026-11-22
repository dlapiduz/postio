//! Logs and counts WebKit web-process deaths under this crate's views, so test waits fail within
//! one loop turn instead of running to their deadline.

use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use webkit6::prelude::*;

static DEATHS: AtomicUsize = AtomicUsize::new(0);

static LAST_REASON: Mutex<Option<String>> = Mutex::new(None);

static PENDING: Mutex<Option<String>> = Mutex::new(None);

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

/// For test wait helpers: a wait that finds one fails at once naming it; taking it charges it to
/// the first wait after the death.
pub fn take_death() -> Option<String> {
    PENDING.lock().ok().and_then(|mut pending| pending.take())
}
