//! `[sync]` — how aggressively Postio talks to the server.
//!
//! ```toml
//! [sync]
//! idle = true              # keep an IDLE connection on INBOX
//! poll_interval_secs = 300 # other folders, and the IDLE fallback
//! max_connections = 5      # per account
//! sync_on_startup = true
//! body_fetch = "lazy"      # lazy | eager
//! attachment_fetch = "on_open" # on_open | eager | never
//! max_inline_bytes = 262144   # inline parts under this ride with the text
//! initial_sync_messages = 5000
//! notify = true            # desktop notifications for new mail
//! notify_roles = ["inbox"] # which mailboxes' arrivals notify
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

/// When an attachment's bytes are downloaded — ADR 0017's payload axis.
///
/// A different question from [`BodyFetch`], and it has to be: on a real
/// mailbox, attachment payloads are ~90% of the bytes and none of the words.
/// Every message's *text* is fetched to completion because that is what makes
/// search complete and offline reading real; a PDF contributes its filename to
/// search and nothing else. So this is the choice between a 1.4 GB store and a
/// 12.4 GB one, and it is worth being asked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttachmentFetch {
    /// Fetch a payload when it is opened or saved, and never before. The
    /// local-first default: metadata is synced for every part, so
    /// `has:attachment` and `filename:` answer with nothing downloaded.
    #[default]
    OnOpen,
    /// Download payloads behind the text too, for a complete offline archive.
    Eager,
    /// Never download a payload, not even on open. Filename search and
    /// nothing more.
    Never,
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

/// 256 KiB — ADR 0017's number for [`SyncConfig::max_inline_bytes`].
fn max_inline_bytes() -> u64 {
    256 * 1024
}

/// Notify for the one mailbox a person actually watches, by default. A
/// desktop notification for every folder an archive rule quietly files mail
/// into is noise, not news.
fn notify_roles() -> Vec<String> {
    vec!["inbox".to_owned()]
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
    /// When to download attachment payloads.
    #[serde(default)]
    pub attachment_fetch: AttachmentFetch,
    /// The largest inline part that is fetched with the message's text
    /// rather than left on the payload axis, in bytes.
    ///
    /// ADR 0017's "inline parts ride with the text". A `cid:` image is not
    /// something the reader offers to download, it is part of the sentence
    /// the message is making — and since remote images are blocked by
    /// default, these are the images that are supposed to appear. Under this
    /// figure a part is text; over it, a payload, so HTML mail reads
    /// correctly offline without pulling the forty-megabyte video somebody
    /// embedded. `0` turns the rule off.
    #[serde(default = "max_inline_bytes")]
    pub max_inline_bytes: u64,
    /// How many messages the first sync reaches back for, newest first.
    #[serde(default = "initial_sync_messages")]
    pub initial_sync_messages: u32,
    /// Master switch for desktop notifications on new mail.
    #[serde(default = "crate::yes")]
    pub notify: bool,
    /// Which mailbox roles produce a notification when mail arrives in them —
    /// the stable identifiers `postio_model::MailboxRole::as_str` emits, e.g.
    /// `"inbox"`, `"flagged"`. A role this build does not recognise is
    /// ignored rather than rejected, the same tolerance `extra` gives unknown
    /// keys elsewhere in this file.
    #[serde(default = "notify_roles")]
    pub notify_roles: Vec<String>,
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
            attachment_fetch: AttachmentFetch::default(),
            max_inline_bytes: max_inline_bytes(),
            initial_sync_messages: initial_sync_messages(),
            notify: true,
            notify_roles: notify_roles(),
            extra: Extras::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notifications_default_on_for_inbox_only() {
        let sync = SyncConfig::default();
        assert!(sync.notify);
        assert_eq!(sync.notify_roles, vec!["inbox".to_owned()]);
    }

    #[test]
    fn notify_settings_round_trip_through_toml() {
        let text = "notify = false\nnotify_roles = [\"inbox\", \"flagged\"]\n";
        let sync: SyncConfig = toml::from_str(text).expect("parse");
        assert!(!sync.notify);
        assert_eq!(
            sync.notify_roles,
            vec!["inbox".to_owned(), "flagged".to_owned()]
        );
    }

    #[test]
    fn the_inline_cap_defaults_to_adr_0017s_figure() {
        // A file that has never heard of the key still reads inline images
        // with the text, which is what #751 was about.
        let sync: SyncConfig = toml::from_str("").expect("parse");
        assert_eq!(sync.max_inline_bytes, 256 * 1024);
    }

    #[test]
    fn the_inline_cap_can_be_retuned_or_turned_off() {
        let sync: SyncConfig = toml::from_str("max_inline_bytes = 0\n").expect("parse");
        assert_eq!(sync.max_inline_bytes, 0);
    }

    #[test]
    fn an_empty_sync_table_still_notifies_the_inbox() {
        // The common case: nobody has ever typed [sync] at all.
        let sync: SyncConfig = toml::from_str("").expect("parse");
        assert!(sync.notify);
        assert_eq!(sync.notify_roles, vec!["inbox".to_owned()]);
    }
}
