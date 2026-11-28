//! Where the application's own account of itself goes. `init` runs first in `main`, before the
//! window, so startup failures are logged.
//! `POSTIO_LOG` pins the level for a run and always wins; `[logging]` in the watched `config.toml`
//! changes a live process.
//! No level unlocks message content: log ids, counts, mailbox names, durations and outcomes, never
//! an address's local part (`postio-runtime`'s `logging_privacy` test enforces it).

use std::path::Path;

use postio_config::validate::Checked;
use postio_config::watch::ConfigWatcher;
use postio_config::{Config, LoggingConfig};
use tracing_log::LogTracer;
use tracing_subscriber::Layer;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::{EnvFilter, Registry, fmt, reload};

pub const LEVEL_ENV: &str = "POSTIO_LOG";

pub struct Logging {
    reload: reload::Handle<EnvFilter, Registry>,
    pinned: bool,
}

/// Send panics through `tracing` as well as to stderr, chained after the previous hook. A panic in
/// a GTK signal handler aborts via `panic_cannot_unwind`, and without its message it looked like
/// heap corruption for six days (#1541). Idempotent.
pub fn report_panics() {
    static INSTALLED: std::sync::Once = std::sync::Once::new();
    INSTALLED.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let at = info.location().map_or_else(
                || "an unknown location".to_owned(),
                |location| location.to_string(),
            );
            let said = info
                .payload_as_str()
                .unwrap_or("a panic payload that is not a string");
            tracing::error!(at = %at, "panicked: {said}");
            previous(info);
        }));
    });
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Destination {
    StderrAndJournal,
    /// journald alone: the terminal app, whose stderr is its screen. With no journal the log goes
    /// nowhere.
    Journal,
}

/// Start logging to stderr and journald, and return the handle that can turn it up later. Best
/// effort: a subscriber that will not install costs the log, not the start.
pub fn init(config: &LoggingConfig) -> Logging {
    init_to(config, Destination::StderrAndJournal)
}

pub fn init_to(config: &LoggingConfig, destination: Destination) -> Logging {
    let pinned = std::env::var(LEVEL_ENV)
        .ok()
        .filter(|value| !value.trim().is_empty());
    let directive = pinned.clone().unwrap_or_else(|| config.directive());

    let (filter, reload) = reload::Layer::new(parse(&directive, config));

    let stderr = stderr_layer(config, destination);

    // journald when the socket is there; under Flatpak it usually is not, which is not a failure.
    let journald = tracing_journald::layer().ok();

    let _ = LogTracer::init();
    let installed = Registry::default().with(filter).with(stderr).with(journald);
    let _ = tracing::subscriber::set_global_default(installed);

    // After the subscriber, so a panic between the two is not reported into a void.
    report_panics();

    Logging {
        reload,
        pinned: pinned.is_some(),
    }
}

/// The stderr half of the log, when `destination` has one. Not stdout, which belongs to the
/// process's purpose; ANSI follows the terminal.
fn stderr_layer<S>(
    config: &LoggingConfig,
    destination: Destination,
) -> Option<Box<dyn Layer<S> + Send + Sync + 'static>>
where
    S: tracing::Subscriber + for<'span> tracing_subscriber::registry::LookupSpan<'span>,
{
    if destination == Destination::Journal {
        return None;
    }
    let stderr = fmt::layer()
        .with_writer(std::io::stderr)
        .with_target(true)
        .with_ansi(std::io::IsTerminal::is_terminal(&std::io::stderr()));
    Some(if config.timestamps {
        stderr.boxed()
    } else {
        stderr.without_time().boxed()
    })
}

impl Logging {
    /// Apply a new `[logging]` section to the running process; ignored when `POSTIO_LOG` pinned the
    /// level.
    pub fn apply(&self, config: &LoggingConfig) {
        if self.pinned {
            return;
        }
        let directive = config.directive();
        if self.reload.reload(parse(&directive, config)).is_err() {
            return;
        }
        tracing::info!(level = %directive, "logging level changed");
    }

    /// Keep applying `[logging]` as `config.toml` changes, on its own watcher since logging runs
    /// before any window. The caller keeps the returned watcher.
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

/// Read `[logging]` off disk before anything has parsed the file properly; a file that will not
/// parse yields the default.
pub fn config_at(path: &Path) -> LoggingConfig {
    Config::load_from_path(path)
        .ok()
        .map(|config| config.logging)
        .unwrap_or_default()
}

/// Every crate whose account of itself is about Postio. `io_imap` is in it because its `debug!`
/// records are load bearing.
const OURS: &[&str] = &[
    "postio",
    "postio_ai",
    "postio_bench",
    "postio_body",
    "postio_calendar",
    "postio_dav",
    "postio_classify",
    "postio_client",
    "postio_config",
    "postio_contact_sync",
    "postio_core",
    "postio_demo",
    "postio_extract",
    "postio_ffi",
    "postio_focus",
    "postio_gtk",
    "postio_host",
    "postio_mcp",
    "postio_account",
    "postio_jmap",
    "postio_gmail",
    "postio_index",
    "postio_model",
    "postio_render",
    "postio_runtime",
    "postio_search",
    "postio_session",
    "postio_signature",
    "postio_smtp",
    "postio_transport",
    "postio_storage",
    "postio_sync",
    "postio_test_support",
    "postio_storyboard",
    "postio_tui",
    "postio_ui",
    "postio_vault",
    "postio_widgets",
    "postio_vcard",
    "io_imap",
];

/// What everything else is held to when a bare level was asked for.
const OTHERS: &str = "warn";

/// Third parties whose `warn` is not a warning, and what to hold them to. Each logs constantly
/// about something nobody can act on: `imap_codec` repairs iCloud responses (88% of a sync's `info`
/// output), `html5ever` logs on mis-nested tables, `tantivy` logs "save metas" per commit (99.5% of
/// a first sync's log).
///
/// A directive, so `POSTIO_LOG=imap_codec=trace` still reaches it: `scope` puts these in front and
/// the later directive wins, which also keeps `info,postio_sync=debug` from re-admitting them.
/// `io_imap` is not here: it is in `OURS`, and debugging a resync needs it.
const QUIET: &[(&str, &str)] = &[
    ("imap_codec", "error"),
    ("html5ever", "off"),
    ("tantivy", "warn"),
];

/// Turn a bare level into a directive that turns Postio up, not the world: `debug` literally would
/// include rustls listing 146 CA certificates.
///
/// A directive naming targets gets what it named, with the `QUIET` defaults in front, where the
/// last directive for a target wins.
fn scope(directive: &str) -> String {
    let bare = directive.trim();
    if bare.contains('=') || bare.contains(',') {
        // Named targets are appended after the `QUIET` defaults so the last directive wins and
        // `html5ever=trace` still reaches `html5ever`.
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
    // `off` means off; scoping it would raise everything else to `warn`.
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

/// A filter, or the default if the directive will not parse: a typo in `filter` must not silence
/// the application.
fn parse(directive: &str, config: &LoggingConfig) -> EnvFilter {
    match EnvFilter::try_new(scope(directive)) {
        Ok(filter) => filter,
        Err(_) => EnvFilter::new(scope(config.level.as_str())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use postio_config::LogLevel;

    #[test]
    fn a_terminal_app_logs_to_the_journal_and_never_onto_its_screen() {
        let config = LoggingConfig::default();
        assert!(stderr_layer::<Registry>(&config, Destination::Journal).is_none());
        assert!(
            stderr_layer::<Registry>(&config, Destination::StderrAndJournal).is_some(),
            "a windowed app's stderr is a log, and keeps getting one"
        );
    }

    #[test]
    fn a_bare_level_turns_postio_up_and_leaves_the_world_alone() {
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
    fn asking_for_more_postio_detail_does_not_re_admit_a_dependency_flood() {
        let scoped = scope("info,postio_sync=debug");

        assert!(
            scoped.contains("tantivy=warn"),
            "a global info floor must not turn tantivy's per-commit \
             bookkeeping back on: {scoped}"
        );
        assert!(
            scoped.find("tantivy=warn") < scoped.find("info,postio_sync=debug"),
            "the quiet defaults go in front so what was asked for still wins: \
             {scoped}"
        );
        assert!(
            scoped.contains("postio_sync=debug"),
            "and what was asked for is still there: {scoped}"
        );
    }

    #[test]
    fn a_dependency_that_warns_constantly_is_quiet_even_when_a_target_was_named() {
        assert!(
            scope("info").contains("html5ever=off"),
            "a bare level must still quiet the dependency"
        );
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
        // `OURS` is hand-maintained and forgetting an entry is silent, so it is checked against the
        // crates directory.
        let crates = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("the crate directory has a parent");
        const SILENT: &[&str] = &["postio_workspace_hack"];
        let mut missing = Vec::new();
        for entry in std::fs::read_dir(crates).expect("the crates directory") {
            let name = entry.expect("a directory entry").file_name();
            let name = name.to_string_lossy().replace('-', "_");
            if name.starts_with("postio_")
                && !OURS.contains(&name.as_str())
                && !SILENT.contains(&name.as_str())
            {
                missing.push(name);
            }
        }
        assert!(
            missing.is_empty(),
            "these crates would be held at `{OTHERS}` by a bare POSTIO_LOG level: {missing:?}"
        );
    }

    fn effective<'a>(scoped: &'a str, target: &str) -> Option<&'a str> {
        scoped.split(',').rfind(|directive| {
            directive
                .split_once('=')
                .is_some_and(|(named, _)| named == target)
        })
    }

    #[test]
    fn a_directive_naming_targets_gets_exactly_what_it_asked_for() {
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

        let rendered = filter.to_string();
        assert!(
            rendered.contains("postio_sync=debug"),
            "the configured filter must survive: {rendered}"
        );
    }

    #[test]
    fn the_codec_repair_warning_is_held_below_warn() {
        let scoped = scope("info");

        assert!(scoped.contains("imap_codec=error"), "{scoped}");
        assert!(
            scoped.starts_with("warn,"),
            "everything else still warns: {scoped}"
        );
    }

    #[test]
    fn asking_for_the_codec_by_name_still_reaches_it() {
        let asked = scope("imap_codec=trace");
        assert_eq!(effective(&asked, "imap_codec"), Some("imap_codec=trace"));

        let both = scope("imap_codec=debug,postio_sync=trace");
        assert_eq!(effective(&both, "imap_codec"), Some("imap_codec=debug"));
        assert_eq!(effective(&both, "postio_sync"), Some("postio_sync=trace"));
    }

    #[test]
    fn nothing_quiets_the_target_the_skip_counter_reads() {
        // `io_imap` stays at the level asked for: its output is what someone reads when a resync
        // goes wrong.
        assert!(
            QUIET.iter().all(|(target, _)| *target != "io_imap"),
            "io_imap must keep the level it was asked for"
        );
        assert!(scope("debug").contains("io_imap=debug"));
    }

    #[test]
    fn a_real_warning_still_gets_through_and_the_repair_does_not() {
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

    #[test]
    fn a_panic_is_reported_where_the_rest_of_the_diagnostics_go() {
        // #1541: a panic inside a GTK signal handler aborts, and its message went to a windowed
        // app's stderr and was lost.
        let captured = Captured::default();
        let subscriber = tracing_subscriber::fmt()
            .with_env_filter(EnvFilter::new("error"))
            .with_writer(captured.clone())
            .finish();

        tracing::subscriber::with_default(subscriber, || {
            report_panics();
            let outcome = std::panic::catch_unwind(|| panic!("the onboarding button gave way"));
            assert!(outcome.is_err(), "the panic still propagates to the caller");
        });

        let out = captured.text();
        assert!(
            out.contains("the onboarding button gave way"),
            "the panic's own message is the whole point of logging it: {out}"
        );
        assert!(
            out.contains("logging.rs"),
            "without the location a logged panic says no more than the abort did: {out}"
        );
    }

    use postio_test_support::logs::Captured;

    #[test]
    fn a_missing_config_file_still_yields_a_usable_level() {
        let logging = config_at(Path::new("/nonexistent/postio/config.toml"));

        assert_eq!(logging.level, LogLevel::Info);
    }
}
