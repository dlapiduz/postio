//! Turning the log on, from a frontend that has no `main` of its own.
//!
//! `postio-app` does this as the first thing in its `main`, and says why:
//! *startup is exactly when a trace is worth having — an account that will not
//! open, a store that will not migrate and a keyring that will not answer all
//! happen before there is any UI to report them in.* Every one of those is
//! more likely on macOS, not less: the Keychain is a second process that can
//! refuse, and an ad-hoc-signed build has a new code identity on every
//! rebuild.
//!
//! The macOS application had no equivalent, so `POSTIO_LOG` did nothing there
//! and a session that would not open was a blank window and no way to ask why.
//!
//! It is a free function rather than something `Session::open` does, because a
//! frontend wants the log running *before* it opens a session — which is the
//! call most worth tracing.

use std::sync::atomic::{AtomicBool, Ordering};

use postio_session::logging;

/// Whether the subscriber is already installed.
///
/// `tracing` refuses a second global subscriber, and an application lifecycle
/// makes calling this twice easy — a window reopening, a wake from sleep.
static STARTED: AtomicBool = AtomicBool::new(false);

/// Start logging, reading `[logging]` from this installation's config.
///
/// Idempotent: a second call returns rather than installing a second
/// subscriber.
///
/// **No message content ever reaches this.** `PRODUCT.md`'s rule is that logs
/// carry ids, counts and outcomes only, and it is enforced where the lines are
/// written rather than here; this only decides whether anyone is listening.
#[uniffi::export]
pub fn start_logging() {
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    let config_path = postio_config::paths::config_path().ok();
    let logging = logging::init(
        &config_path
            .as_deref()
            .map(logging::config_at)
            .unwrap_or_default(),
    );
    // Leaked deliberately, the way `postio-app` holds its own for the life of
    // the process: dropping the watcher stops the watch, and re-tuning a
    // running Postio through `[logging]` is the whole point of that section.
    // There is no other owner here -- this is a free function, not a `main`.
    std::mem::forget(config_path.as_deref().and_then(|path| logging.watch(path)));
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "postio starting");

    // First run: `config.toml` does not exist yet. Postio's defaults apply
    // with nothing on disk, so this changes *discoverability*, not behaviour
    // — the Config file pane, `⌘E` and a file manager all find something to
    // read and edit rather than a blank buffer that documents nothing.
    //
    // `postio-app` has done this since it had a settings surface; macOS
    // never did, and the empty editor was the first thing the pane showed
    // when it was built. Here rather than in `Session::open`, because
    // settings are a file and the settings window works with no session at
    // all — the one thing both frontends' launch paths have in common is
    // that they turn the log on.
    if let Some(path) = config_path.as_deref() {
        match postio_config::Config::seed_if_missing(path) {
            Ok(true) => tracing::info!(path = %path.display(), "seeded a starter config.toml"),
            Ok(false) => {}
            Err(error) => tracing::warn!(%error, "could not seed a starter config.toml"),
        }
    }
}
