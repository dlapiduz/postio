//! Turning the log on from a frontend with no `main`. A free function, so the log can run before a
//! session opens:
//! startup is when a trace is worth having.

use std::sync::atomic::{AtomicBool, Ordering};

use postio_session::logging;

/// Whether the subscriber is already installed: `tracing` refuses a second global one.
static STARTED: AtomicBool = AtomicBool::new(false);

/// Start logging, reading `[logging]` from this installation's config. Idempotent. No message
/// content reaches this:
/// logs carry ids, counts and outcomes only.
#[uniffi::export]
pub fn start_logging() {
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    // The installed file, or a demo's own: a demo seeds and watches nobody's.
    let config_path = crate::settings::config_path().ok();
    let logging = logging::init(
        &config_path
            .as_deref()
            .map(logging::config_at)
            .unwrap_or_default(),
    );
    // Leaked deliberately: dropping the watcher stops the watch, and `[logging]` re-tuning a
    // running Postio needs it.
    std::mem::forget(config_path.as_deref().and_then(|path| logging.watch(path)));
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "postio starting");

    // First run: write `config.toml` so the Config file pane, `⌘E` and a file manager find
    // something to edit; defaults apply either way. Here, not in `Session::open`, because the
    // settings window works with no session.
    if let Some(path) = config_path.as_deref() {
        match postio_config::Config::seed_if_missing(path) {
            Ok(true) => tracing::info!(path = %path.display(), "seeded a starter config.toml"),
            Ok(false) => {}
            Err(error) => tracing::warn!(%error, "could not seed a starter config.toml"),
        }
    }
}
