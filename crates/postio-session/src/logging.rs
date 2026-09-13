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
//! bearing — `postio-account`'s skip counter watches for a dropped untagged
//! response and turns it into `ResyncIntegrityLost`. `log::set_logger`
//! succeeds once per process, so the bridge and that counter cannot both
//! install themselves; [`postio_account::imap::install_skip_counter_forwarding_to`]
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
    "postio_bench",
    "postio_body",
    "postio_config",
    "postio_core",
    "postio_ffi",
    "postio_gtk",
    "postio_account",
    "postio_jmap",
    "postio_gmail",
    "postio_index",
    "postio_model",
    "postio_runtime",
    "postio_search",
    "postio_session",
    "postio_smtp",
    "postio_storage",
    "postio_sync",
    "postio_test_support",
    "postio_ui",
    "io_imap",
];

/// What everything else is held to when a bare level was asked for.
///
/// A third party still gets to say something went wrong; it does not get to
/// narrate.
const OTHERS: &str = "warn";

/// Third parties whose `warn` is not a warning, and what to hold them to.
///
/// [`OTHERS`] lets any dependency say something went wrong, which is right
/// until one of them says it constantly about something nobody can act on.
///
/// `imap_codec` repairs responses that omit a `text` field and logs each
/// repair at `warn`. That is correct behaviour and iCloud triggers it on
/// essentially every `SELECT` and `FETCH`: measured against a live account,
/// an INBOX resync of 92 messages produced 486 lines at `info`, and 429 of
/// them — 88% — were that one warning. A level that fires constantly, that
/// the user cannot act on, and that means nothing is wrong is a level
/// everyone learns to skip, which is the level real problems arrive at.
///
/// This is a *directive*, not a change in `postio-account`, so
/// `POSTIO_LOG=imap_codec=trace` still reaches it: [`scope`] puts these
/// defaults in front of whatever was asked for and the later directive wins,
/// and asking for a target by name is the one unambiguous way to say you want
/// it.
///
/// `io_imap` is deliberately **not** here even though it is the noisier
/// crate. It is in [`OURS`], and `postio-account`'s skip counter reads its
/// records — see the module docs. Quieting it here would not actually break
/// the counter (that runs in the `log` layer, before tracing sees anything,
/// and counts `io_imap` at `debug`), but it would take away the output
/// somebody debugging a resync needs.
/// `html5ever` is the second. It logs *"foster parenting not implemented"* at
/// `warn` every time it repairs a mis-nested table, which is most real HTML
/// mail: a live run produced hundreds of lines inside two seconds and buried
/// the three that mattered. Nothing is wrong, nobody can act on it, and the
/// reader is not even parsing for the user's benefit at that point -- it is
/// building a search excerpt. `off` rather than `error`, because unlike
/// `imap_codec` there is no level of it worth keeping by default.
const QUIET: &[(&str, &str)] = &[("imap_codec", "error"), ("html5ever", "off")];

/// Turn a bare level into a directive that turns *Postio* up, not the world.
///
/// `POSTIO_LOG=debug` has to mean "tell me what Postio is doing". Applied
/// literally it means "tell me what every crate in the binary is doing", which
/// on this dependency graph is rustls enumerating 146 CA certificates before
/// the first line about mail — the same drowning that made
/// `G_MESSAGES_DEBUG=all` useless for diagnosing a sync.
///
/// A directive naming targets gets exactly what it named: someone who wrote
/// `rustls=trace` wants rustls, and second-guessing that would take away the
/// only way to ask. It is not passed through *untouched*, though -- the
/// [`QUIET`] defaults go in front of it, where the last directive for a target
/// wins, so naming one target no longer re-admits every warning the others
/// were quieted for.
fn scope(directive: &str) -> String {
    let bare = directive.trim();
    if bare.contains('=') || bare.contains(',') {
        // Named targets are still honoured -- they are simply appended after
        // the [`QUIET`] defaults rather than replacing them, so the last
        // directive for a target wins and `html5ever=trace` still reaches
        // `html5ever`.
        //
        // Passing these through untouched was the older behaviour and it had
        // the failure mode exactly backwards: `POSTIO_LOG=postio_account=debug`
        // is what someone types when they are reading a log *closely*, and it
        // was the one spelling that re-admitted every drowning warning a
        // dependency had been quieted for.
        let mut scoped = String::new();
        for (target, level) in QUIET {
            scoped.push_str(target);
            scoped.push('=');
            scoped.push_str(level);
            scoped.push(',');
        }
        scoped.push_str(bare);
        return scoped;
    }
    // `off` means off. Scoping it would raise everything else to `warn`,
    // which is louder than what was asked for.
    if bare.eq_ignore_ascii_case("off") {
        return bare.to_owned();
    }
    let mut scoped = String::from(OTHERS);
    for (target, level) in QUIET {
        scoped.push(',');
        scoped.push_str(target);
        scoped.push('=');
        scoped.push_str(level);
    }
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
        postio_account::imap::install_skip_counter_forwarding_to(Some(Box::new(LogTracer::new())));
    installed && postio_account::imap::skip_counter_is_counting()
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
    fn a_dependency_that_warns_constantly_is_quiet_even_when_a_target_was_named() {
        // `html5ever` logs "foster parenting not implemented" at `warn` for
        // every mis-nested table it repairs, which is most real HTML mail. A
        // live run produced hundreds of them in a two-second window and buried
        // the three lines that mattered -- the same drowning `imap_codec` was
        // quieted for.
        assert!(
            scope("info").contains("html5ever=off"),
            "a bare level must still quiet the dependency"
        );
        // And the case that actually bit: asking for one of *our* targets by
        // name used to pass the directive through untouched, which dropped
        // every QUIET default on the floor at exactly the moment someone was
        // reading the log closely.
        let named = scope("postio_account=debug");
        assert!(
            named.contains("html5ever=off"),
            "naming a target must not re-admit the noise: {named}"
        );
        assert!(
            named.contains("postio_account=debug"),
            "and it must still say what was asked for: {named}"
        );
    }

    /// Asking for a quieted target by name still reaches it.
    #[test]
    fn a_quieted_target_can_still_be_asked_for_by_name() {
        let asked = scope("html5ever=trace");
        assert_eq!(
            effective(&asked, "html5ever"),
            Some("html5ever=trace"),
            "the default must come first so an explicit ask overrides it: {asked}"
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

    /// The last directive naming a target is the one that applies.
    fn effective<'a>(scoped: &'a str, target: &str) -> Option<&'a str> {
        scoped.split(',').rfind(|directive| {
            directive
                .split_once('=')
                .is_some_and(|(named, _)| named == target)
        })
    }

    #[test]
    fn a_directive_naming_targets_gets_exactly_what_it_asked_for() {
        // Someone who wrote `rustls=trace` wants rustls, and second-guessing
        // that would take away the only way to ask.
        //
        // This used to be spelled as "passed through untouched", asserting the
        // string came back byte for byte. That was a stronger promise than the
        // sentence above needs, and it had a cost: it also handed back every
        // drowning `warn` the [`QUIET`] defaults exist to suppress, at exactly
        // the moment someone was reading a log closely enough to name a
        // target. The defaults go in front now and the ask still wins.
        let scoped = scope("rustls=trace,postio_sync=debug");
        assert_eq!(effective(&scoped, "rustls"), Some("rustls=trace"));
        assert_eq!(effective(&scoped, "postio_sync"), Some("postio_sync=debug"));

        let one = scope("io_imap=trace");
        assert_eq!(effective(&one, "io_imap"), Some("io_imap=trace"));
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

        // The QUIET defaults ride in front of it; what was written is what
        // applies to the target it names.
        let rendered = filter.to_string();
        assert!(
            rendered.contains("postio_sync=debug"),
            "the configured filter must survive: {rendered}"
        );
    }

    #[test]
    fn the_codec_repair_warning_is_held_below_warn() {
        // `postio-b9t.4`: 88% of a live sync's output at `info` was
        // imap_codec repairing responses iCloud sends without a `text`
        // field. Correct behaviour, constant, and nothing the user can do.
        let scoped = scope("info");

        assert!(scoped.contains("imap_codec=error"), "{scoped}");
        assert!(
            scoped.starts_with("warn,"),
            "everything else still warns: {scoped}"
        );
    }

    #[test]
    fn asking_for_the_codec_by_name_still_reaches_it() {
        // The fix is a directive rather than a change in postio-account
        // precisely so this keeps working for whoever is debugging the codec.
        let asked = scope("imap_codec=trace");
        assert_eq!(effective(&asked, "imap_codec"), Some("imap_codec=trace"));

        let both = scope("imap_codec=debug,postio_sync=trace");
        assert_eq!(effective(&both, "imap_codec"), Some("imap_codec=debug"));
        assert_eq!(effective(&both, "postio_sync"), Some("postio_sync=trace"));
    }

    #[test]
    fn nothing_quiets_the_target_the_skip_counter_reads() {
        // `postio-account`'s skip counter turns a dropped untagged response
        // into `ResyncIntegrityLost` — an integrity check, not a log line.
        //
        // A tracing directive could not actually silence it: the counter is a
        // `log::Log` wrapper that counts *before* delegating, its `enabled`
        // ignores the inner logger for `io_imap` at debug, and tracing only
        // ever sees what the bridge forwards afterwards. But `io_imap`'s
        // output is also what somebody reads when a resync goes wrong, so it
        // stays at whatever level was asked for.
        assert!(
            QUIET.iter().all(|(target, _)| *target != "io_imap"),
            "io_imap must keep the level it was asked for"
        );
        assert!(scope("debug").contains("io_imap=debug"));
    }

    #[test]
    fn a_real_warning_still_gets_through_and_the_repair_does_not() {
        // The acceptance criterion is about what actually reaches the log,
        // not about the shape of the directive, so this asserts on output.
        let captured = Captured::default();
        let subscriber = tracing_subscriber::fmt()
            .with_env_filter(EnvFilter::new(scope("info")))
            .with_writer(captured.clone())
            .finish();

        tracing::subscriber::with_default(subscriber, || {
            tracing::warn!(target: "imap_codec", "Rectified missing `text` to \"\"");
            tracing::warn!(target: "postio_sync", "the mailbox disagreed about UIDVALIDITY");
            tracing::error!(target: "imap_codec", "something actually broke");
        });

        let out = captured.text();
        assert!(
            !out.contains("Rectified"),
            "the repair warning should not reach the log: {out}"
        );
        assert!(
            out.contains("UIDVALIDITY"),
            "a real warning still has to get through: {out}"
        );
        assert!(
            out.contains("something actually broke"),
            "quieted is not silenced — the codec can still report a real error: {out}"
        );
    }

    /// Somewhere for a test subscriber to write, so an assertion can be about
    /// what came out rather than about the filter that was built.
    #[derive(Clone, Default)]
    struct Captured(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

    impl Captured {
        fn text(&self) -> String {
            String::from_utf8_lossy(&self.0.lock().expect("not poisoned")).into_owned()
        }
    }

    impl std::io::Write for Captured {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().expect("not poisoned").extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Captured {
        type Writer = Captured;

        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    #[test]
    fn a_missing_config_file_still_yields_a_usable_level() {
        let logging = config_at(Path::new("/nonexistent/postio/config.toml"));

        assert_eq!(logging.level, LogLevel::Info);
    }
}
