//! `[logging]`: a setting as well as `POSTIO_LOG` because the config is watched and applied live,
//! so a running, misbehaving process can be made louder. `POSTIO_LOG` wins over `[logging]`. No
//! level unlocks message content; `postio-runtime`'s `logging_privacy` test holds that.

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
    Off,
    Error,
    Warn,
    #[default]
    Info,
    Debug,
    Trace,
}

impl LogLevel {
    /// A string rather than a `tracing` type: this crate is the schema and must not drag a
    /// subscriber into everything that reads a config file.
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LoggingConfig {
    #[serde(default)]
    pub level: LogLevel,
    /// Kept as an unparsed string: a directive this build cannot parse has to survive a round trip
    /// like any other hand-edit.
    #[serde(default)]
    pub filter: String,
    #[serde(default = "crate::yes")]
    pub timestamps: bool,
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
        assert!(LogLevel::Off < LogLevel::Error);
        assert!(LogLevel::Error < LogLevel::Warn);
        assert!(LogLevel::Warn < LogLevel::Info);
        assert!(LogLevel::Info < LogLevel::Debug);
        assert!(LogLevel::Debug < LogLevel::Trace);
    }

    #[test]
    fn an_unknown_key_in_the_section_survives_a_round_trip() {
        let config = Config::from_toml_str("[logging]\nlevel = \"debug\"\njournald = false\n")
            .expect("valid");

        assert!(config.logging.extra.contains_key("journald"));
        let written = toml::to_string(&config).expect("serializes");
        assert!(written.contains("journald"), "{written}");
    }
}
