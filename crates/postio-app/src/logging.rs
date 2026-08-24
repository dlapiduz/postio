//! Where the application's own account of itself goes.
//!
//! Libraries emit; the binary decides where it lands. Every crate below this
//! one calls `tracing`'s macros and knows nothing about subscribers, formats
//! or destinations — which is what lets a test capture the same records this
//! module sends to a terminal.
//!
//! # This runs before almost everything
//!
//! [`init`] is the first thing `main` does, ahead of `adw::init` and well
//! ahead of the window. Startup is exactly when a trace is worth having: an
//! account that will not open, a store that will not migrate and a keyring
//! that will not answer all happen before there is any UI to report them in.
//!
//! # Two ways to set the level, and which wins
//!
//! `POSTIO_LOG` is for a run you are starting. `[logging]` in `config.toml` is
//! for a process that is *already running* and already misbehaving, which is
//! when a log matters most — the file is watched, so raising the level reaches
//! a live Postio without restarting it and without losing the state you were
//! trying to observe.
//!
//! `POSTIO_LOG` wins, and keeps winning: an operator who set the environment
//! for this run meant it, and a config reload must not quietly take it back.
//!
//! # The `log` bridge is not optional
//!
//! `io-imap` emits through the `log` crate, and one of its records is load
//! bearing — `postio-imap`'s skip counter watches for a dropped untagged
//! response and turns it into `ResyncIntegrityLost`. `log::set_logger`
//! succeeds once per process, so the bridge and that counter cannot both
//! install themselves; [`postio_imap::imap::install_skip_counter_forwarding_to`]
//! composes them. If that composition ever fails, this module says so at
//! `warn` rather than letting an integrity check go quiet.
//!
//! # What must never appear here
//!
//! No level unlocks message content. Not bodies, not subjects, not recipient
//! addresses, not passwords, not file contents — at `trace`, in a debug build,
//! ever. Log ids, counts, mailbox names, durations and outcomes. Where an
//! address would genuinely make a line useful, log the domain or an opaque
//! account id and never the local part. `postio-runtime`'s `logging_privacy`
//! test enforces this against the real `.eml` corpus rather than trusting the
//! rule to be remembered.

use std::path::Path;

use postio_config::validate::Checked;
use postio_config::watch::ConfigWatcher;
use postio_config::{Config, LoggingConfig};
use tracing_log::LogTracer;
use tracing_subscriber::Layer;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::{EnvFilter, Registry, fmt, reload};

/// The environment variable that pins the level for one run.
pub const LEVEL_ENV: &str = "POSTIO_LOG";

/// The live level, and whether anything is allowed to change it.
pub struct Logging {
    reload: reload::Handle<EnvFilter, Registry>,
    /// Set when `POSTIO_LOG` chose the level, which makes it final.
    pinned: bool,
}

/// Start logging, and return the handle that can turn it up later.
///
/// Best effort: a subscriber that will not install costs the log and nothing
/// else. An application that refused to start because it could not open a log
/// would be a worse answer than one running quietly.
pub fn init(config: &LoggingConfig) -> Logging {
    let pinned = std::env::var(LEVEL_ENV)
        .ok()
        .filter(|value| !value.trim().is_empty());
    let directive = pinned.clone().unwrap_or_else(|| config.directive());

    let (filter, reload) = reload::Layer::new(parse(&directive, config));

    // stderr, not stdout: this is diagnostics, and stdout belongs to whatever
    // the process is actually for. `with_ansi` follows the terminal, so a
    // redirected log is plain text rather than escape codes.
    let stderr = fmt::layer()
        .with_writer(std::io::stderr)
        .with_target(true)
        .with_ansi(std::io::IsTerminal::is_terminal(&std::io::stderr()));
    let stderr = if config.timestamps {
        stderr.boxed()
    } else {
        stderr.without_time().boxed()
    };

    // journald when the socket is there. Under Flatpak it usually is not, and
    // a sandbox with no journal is not a failure — stderr is still going to
    // the portal's log.
    let journald = tracing_journald::layer().ok();

    // Before the subscriber, and deliberately not through
    // `SubscriberInitExt::init`. That helper calls `LogTracer::init` itself,
    // which is a `log::set_logger` — so it would win the one call the process
    // gets and leave `io-imap`'s skip counter inert. `set_global_default`
    // installs the subscriber and nothing else.
    let bridged = bridge_log_records();
    let installed = Registry::default().with(filter).with(stderr).with(journald);
    let _ = tracing::subscriber::set_global_default(installed);

    // Said after the subscriber exists, or nobody would hear it.
    if !bridged {
        tracing::warn!(
            "another logger was installed first: io-imap's skipped-response counter is \
             inert, so a resync that silently dropped deltas will not be reported"
        );
    }

    Logging {
        reload,
        pinned: pinned.is_some(),
    }
}

impl Logging {
    /// Apply a new `[logging]` section to the running process.
    ///
    /// Ignored when `POSTIO_LOG` pinned the level: see the module docs.
    pub fn apply(&self, config: &LoggingConfig) {
        if self.pinned {
            return;
        }
        let directive = config.directive();
        if self.reload.reload(parse(&directive, config)).is_err() {
            // The subscriber is gone, which only happens during teardown.
            return;
        }
        tracing::info!(level = %directive, "logging level changed");
    }

    /// Keep applying `[logging]` as `config.toml` changes.
    ///
    /// Its own watcher rather than a share of the window's: logging is running
    /// long before there is a window, and a filter reload touches no widget,
    /// so it can be applied on the watcher's own thread with no hop to the
    /// main context. The returned watcher stops when it is dropped, so the
    /// caller has to keep it.
    pub fn watch(self, path: &Path) -> Option<ConfigWatcher> {
        ConfigWatcher::new(path, move |checked: Checked| {
            if let Some(config) = &checked.config {
                self.apply(&config.logging);
            }
        })
        .map_err(|error| tracing::warn!(%error, "config.toml will not be watched for log level"))
        .ok()
    }
}

/// Read `[logging]` off disk before anything has parsed the file properly.
///
/// The full parse happens later and reports its own problems; this only needs
/// a level, and it needs it before there is anywhere to report a problem to. A
/// file that will not parse yields the default, which is the same answer a
/// first run gets.
pub fn config_at(path: &Path) -> LoggingConfig {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| Config::from_toml_str(&text).ok())
        .map(|config| config.logging)
        .unwrap_or_default()
}

/// Every crate whose account of itself is *about Postio*.
///
/// `io_imap` is in the list because it is the protocol library Postio's own
/// IMAP crate drives, and because one of its `debug!` records is load bearing
/// — see the module docs.
const OURS: &[&str] = &[
    "postio",
    "postio_app",
    "postio_config",
    "postio_core",
    "postio_gtk",
    "postio_imap",
    "postio_index",
    "postio_model",
    "postio_runtime",
    "postio_search",
    "postio_smtp",
    "postio_storage",
    "postio_sync",
    "io_imap",
];

/// What everything else is held to when a bare level was asked for.
///
/// A third party still gets to say something went wrong; it does not get to
/// narrate.
const OTHERS: &str = "warn";

/// Turn a bare level into a directive that turns *Postio* up, not the world.
///
/// `POSTIO_LOG=debug` has to mean "tell me what Postio is doing". Applied
/// literally it means "tell me what every crate in the binary is doing", which
/// on this dependency graph is rustls enumerating 146 CA certificates before
/// the first line about mail — the same drowning that made
/// `G_MESSAGES_DEBUG=all` useless for diagnosing a sync.
///
/// A directive naming targets is passed through untouched: someone who wrote
/// `rustls=trace` wants rustls, and second-guessing that would take away the
/// only way to ask.
fn scope(directive: &str) -> String {
    let bare = directive.trim();
    if bare.contains('=') || bare.contains(',') {
        return bare.to_owned();
    }
    // `off` means off. Scoping it would raise everything else to `warn`,
    // which is louder than what was asked for.
    if bare.eq_ignore_ascii_case("off") {
        return bare.to_owned();
    }
    let mut scoped = String::from(OTHERS);
    for target in OURS {
        scoped.push(',');
        scoped.push_str(target);
        scoped.push('=');
        scoped.push_str(bare);
    }
    scoped
}

/// A filter, or the default if the directive will not parse.
///
/// A typo in `filter` must not silence the application: it falls back to the
/// section's level and says what it ignored, which is the opposite of what a
/// logging system quietly failing would do.
fn parse(directive: &str, config: &LoggingConfig) -> EnvFilter {
    match EnvFilter::try_new(scope(directive)) {
        Ok(filter) => filter,
        Err(_) => EnvFilter::new(scope(config.level.as_str())),
    }
}

/// Carry `log` records — `io-imap`'s — into `tracing`, without unhooking the
/// skip counter. See the module docs.
///
/// Returns whether both halves are live.
fn bridge_log_records() -> bool {
    let installed =
        postio_imap::imap::install_skip_counter_forwarding_to(Some(Box::new(LogTracer::new())));
    installed && postio_imap::imap::skip_counter_is_counting()
}

#[cfg(test)]
mod tests {
    use super::*;
    use postio_config::LogLevel;

    #[test]
    fn a_bare_level_turns_postio_up_and_leaves_the_world_alone() {
        // `POSTIO_LOG=debug` has to mean "tell me what Postio is doing".
        // Applied literally it means rustls enumerating 146 CA certificates
        // before the first line about mail.
        let scoped = scope("debug");

        assert!(scoped.starts_with("warn,"), "{scoped}");
        assert!(scoped.contains("postio_sync=debug"), "{scoped}");
        assert!(scoped.contains("io_imap=debug"), "{scoped}");
        assert!(
            !scoped.contains("rustls"),
            "third parties are held to `warn`, not named one by one"
        );
    }

    #[test]
    fn every_crate_in_the_workspace_is_in_the_scoped_list() {
        // `OURS` is hand-maintained, and the failure mode of forgetting an
        // entry is silence: the new crate is held at `warn` and nobody finds
        // out until they are trying to diagnose something in it. So the list
        // is checked against the directory that defines it.
        let crates = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("crates/postio-app has a parent");
        let mut missing = Vec::new();
        for entry in std::fs::read_dir(crates).expect("the crates directory") {
            let name = entry.expect("a directory entry").file_name();
            let name = name.to_string_lossy().replace('-', "_");
            if name.starts_with("postio_") && !OURS.contains(&name.as_str()) {
                missing.push(name);
            }
        }
        assert!(
            missing.is_empty(),
            "these crates would be held at `{OTHERS}` by a bare POSTIO_LOG level: {missing:?}"
        );
    }

    #[test]
    fn a_directive_naming_targets_is_passed_through_untouched() {
        // Someone who wrote `rustls=trace` wants rustls, and second-guessing
        // that would take away the only way to ask.
        assert_eq!(
            scope("rustls=trace,postio_sync=debug"),
            "rustls=trace,postio_sync=debug"
        );
        assert_eq!(scope("io_imap=trace"), "io_imap=trace");
    }

    #[test]
    fn off_means_off_rather_than_warn() {
        assert_eq!(scope("off"), "off");
    }

    #[test]
    fn a_malformed_filter_falls_back_to_the_level_rather_than_going_silent() {
        // A typo in `filter` is the one failure a logging system must not
        // answer with silence: the person editing it is editing it *because*
        // they need output.
        let config = LoggingConfig {
            level: LogLevel::Debug,
            filter: "not a filter=@!".to_string(),
            ..LoggingConfig::default()
        };

        let filter = parse(&config.directive(), &config);

        assert!(
            filter.to_string().contains("postio_sync=debug"),
            "fell back to something unusable: {filter}"
        );
    }

    #[test]
    fn a_good_filter_is_taken_as_written() {
        let config = LoggingConfig {
            level: LogLevel::Warn,
            filter: "postio_sync=debug".to_string(),
            ..LoggingConfig::default()
        };

        let filter = parse(&config.directive(), &config);

        assert_eq!(filter.to_string(), "postio_sync=debug");
    }

    #[test]
    fn a_missing_config_file_still_yields_a_usable_level() {
        let logging = config_at(Path::new("/nonexistent/postio/config.toml"));

        assert_eq!(logging.level, LogLevel::Info);
    }
}
