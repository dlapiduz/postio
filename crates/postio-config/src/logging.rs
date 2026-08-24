//! `[logging]` — how much Postio says about what it is doing, and where.
//!
//! ```toml
//! [logging]
//! level = "info"   # off | error | warn | info | debug | trace
//! filter = ""      # per-target override: "postio_sync=debug,io_imap=trace"
//! timestamps = true
//! ```
//!
//! # Why this is a setting and not only an environment variable
//!
//! `POSTIO_LOG` is the right tool when you are starting the process. It is the
//! wrong tool when the process is already running and misbehaving — which is
//! exactly when the log matters most, because the interesting state is the
//! state it is in *now*. `config.toml` is watched and applied live, so raising
//! the level here reaches a running Postio without restarting it and without
//! losing the situation you were trying to observe.
//!
//! Precedence is `POSTIO_LOG` over `[logging]`: an operator who set the
//! environment for this run meant it, and a live config reload must not
//! quietly take it back.
//!
//! # What must never appear in the output
//!
//! No level here unlocks message content. `trace` is protocol detail —
//! commands, state transitions, counts — not bodies, subjects or addresses.
//! That is a property of the call sites rather than of this section, and
//! `postio-runtime`'s `logging_privacy` test is what holds it down.

use serde::{Deserialize, Serialize};

use crate::Extras;

/// How much to say.
///
/// The meanings are fixed workspace-wide so a crate cannot invent its own:
///
/// | Level | Means | Example |
/// |---|---|---|
/// | `error` | Something was lost, or the user must act | a send failed permanently; resync integrity lost |
/// | `warn` | Degraded but continuing | fell back to a full resync; a capability was missing |
/// | `info` | Lifecycle — the default | account connected; sync started and finished |
/// | `debug` | Decisions and counts | chose the CONDSTORE path; drained 12 operations |
/// | `trace` | Protocol detail, off by default | command dispatch; state transitions |
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    /// Say nothing at all.
    Off,
    /// Something was lost, or the user has to act.
    Error,
    /// Degraded but continuing.
    Warn,
    /// Lifecycle. The default.
    #[default]
    Info,
    /// Decisions and counts.
    Debug,
    /// Protocol detail.
    Trace,
}

impl LogLevel {
    /// The spelling `tracing-subscriber`'s `EnvFilter` understands.
    ///
    /// A string rather than a `tracing` type because this crate is the
    /// *schema*: it is read by the settings panel and the validator as well as
    /// by the subscriber, and it must not drag a subscriber into everything
    /// that reads a config file.
    pub const fn as_str(self) -> &'static str {
        match self {
            LogLevel::Off => "off",
            LogLevel::Error => "error",
            LogLevel::Warn => "warn",
            LogLevel::Info => "info",
            LogLevel::Debug => "debug",
            LogLevel::Trace => "trace",
        }
    }
}

impl std::fmt::Display for LogLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The `[logging]` section.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LoggingConfig {
    /// How much to say, when `filter` does not say something more specific.
    #[serde(default)]
    pub level: LogLevel,
    /// A per-target override in `EnvFilter` syntax, e.g.
    /// `"postio_sync=debug,io_imap=trace"`.
    ///
    /// Empty means "just use `level`". Kept as an unparsed string on purpose:
    /// the schema layer has no business owning filter syntax, and a directive
    /// this build cannot parse has to survive a round trip like any other
    /// hand-edit rather than being dropped.
    #[serde(default)]
    pub filter: String,
    /// Prefix each line with the time it was emitted.
    #[serde(default = "crate::yes")]
    pub timestamps: bool,
    /// Keys this version of Postio does not know, preserved verbatim.
    #[serde(flatten)]
    pub extra: Extras,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: LogLevel::default(),
            filter: String::new(),
            timestamps: true,
            extra: Extras::new(),
        }
    }
}

impl LoggingConfig {
    /// The `EnvFilter` directive this section asks for.
    ///
    /// `filter` wins when it is set, because it is the more specific request;
    /// otherwise every target gets `level`.
    pub fn directive(&self) -> String {
        match self.filter.trim() {
            "" => self.level.to_string(),
            filter => filter.to_owned(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Config;

    #[test]
    fn the_default_is_lifecycle_only() {
        let logging = LoggingConfig::default();

        assert_eq!(logging.level, LogLevel::Info);
        assert_eq!(logging.directive(), "info");
        assert!(
            logging.timestamps,
            "a log line with no time on it cannot be correlated with anything"
        );
    }

    #[test]
    fn a_level_reads_off_the_file() {
        let config = Config::from_toml_str("[logging]\nlevel = \"debug\"\n").expect("valid");

        assert_eq!(config.logging.level, LogLevel::Debug);
        assert_eq!(config.logging.directive(), "debug");
    }

    #[test]
    fn a_filter_is_more_specific_than_a_level_and_wins() {
        let config = Config::from_toml_str(
            "[logging]\nlevel = \"warn\"\nfilter = \"postio_sync=debug,io_imap=trace\"\n",
        )
        .expect("valid");

        assert_eq!(
            config.logging.directive(),
            "postio_sync=debug,io_imap=trace",
            "asking for a specific target and then being given the blunt level \
             would make the setting useless"
        );
    }

    #[test]
    fn levels_are_ordered_from_quiet_to_loud() {
        // The settings panel offers these as a scale, so the order is part of
        // the schema rather than an accident of declaration.
        assert!(LogLevel::Off < LogLevel::Error);
        assert!(LogLevel::Error < LogLevel::Warn);
        assert!(LogLevel::Warn < LogLevel::Info);
        assert!(LogLevel::Info < LogLevel::Debug);
        assert!(LogLevel::Debug < LogLevel::Trace);
    }

    #[test]
    fn an_unknown_key_in_the_section_survives_a_round_trip() {
        // People hand-edit this file, and a key written by a newer Postio must
        // not be deleted by an older one saving over it.
        let config = Config::from_toml_str("[logging]\nlevel = \"debug\"\njournald = false\n")
            .expect("valid");

        assert!(config.logging.extra.contains_key("journald"));
        let written = toml::to_string(&config).expect("serializes");
        assert!(written.contains("journald"), "{written}");
    }
}
