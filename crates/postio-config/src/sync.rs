//! `[sync]` — how aggressively Postio talks to the server.
//!
//! ```toml
//! [sync]
//! idle = true              # keep an IDLE connection on INBOX
//! poll_interval_secs = 300 # other folders, and the IDLE fallback
//! max_connections = 5      # per account
//! sync_on_startup = true
//! body_fetch = "lazy"      # lazy | eager
//! initial_sync_messages = 5000
//! ```

use serde::{Deserialize, Serialize};

use crate::Extras;

/// When message bodies are downloaded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BodyFetch {
    /// Headers first; bodies backfilled behind the UI. The local-first default.
    #[default]
    Lazy,
    /// Download bodies as soon as headers arrive.
    Eager,
}

fn poll_interval_secs() -> u64 {
    300
}

fn max_connections() -> u8 {
    5
}

fn initial_sync_messages() -> u32 {
    5_000
}

/// The `[sync]` section.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SyncConfig {
    /// Hold an `IDLE` connection on INBOX for push delivery.
    #[serde(default = "crate::yes")]
    pub idle: bool,
    /// Polling interval for folders without `IDLE`, in seconds.
    #[serde(default = "poll_interval_secs")]
    pub poll_interval_secs: u64,
    /// Maximum simultaneous IMAP connections per account.
    #[serde(default = "max_connections")]
    pub max_connections: u8,
    /// Start a sync as soon as the app opens.
    #[serde(default = "crate::yes")]
    pub sync_on_startup: bool,
    /// When to download bodies.
    #[serde(default)]
    pub body_fetch: BodyFetch,
    /// How many messages the first sync reaches back for, newest first.
    #[serde(default = "initial_sync_messages")]
    pub initial_sync_messages: u32,
    /// Keys this version of Postio does not know, preserved verbatim.
    #[serde(flatten)]
    pub extra: Extras,
}

impl Default for SyncConfig {
    fn default() -> Self {
        Self {
            idle: true,
            poll_interval_secs: poll_interval_secs(),
            max_connections: max_connections(),
            sync_on_startup: true,
            body_fetch: BodyFetch::default(),
            initial_sync_messages: initial_sync_messages(),
            extra: Extras::new(),
        }
    }
}
