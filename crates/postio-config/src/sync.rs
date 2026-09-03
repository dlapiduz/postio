//! `[sync]` — how aggressively Postio talks to the server.
//!
//! ```toml
//! [sync]
//! check_for_mail = "idle"  # idle | poll | manual
//! poll_interval_secs = 300 # other folders, and idle's own fallback
//! max_connections = 5      # per account
//! sync_on_startup = true
//! body_fetch = "lazy"      # lazy | eager
//! attachment_fetch = "on_open" # on_open | eager | never
//! max_inline_bytes = 262144   # inline parts under this ride with the text
//! initial_sync_messages = 5000
//! notify = true            # desktop notifications for new mail
//! notify_roles = ["inbox"] # which mailboxes' arrivals notify
//! ```

use serde::{Deserialize, Deserializer, Serialize};

use crate::Extras;

/// How Postio learns about new mail (#867).
///
/// Replaces the old `idle: bool` -- `Manual` is a third state that bool had
/// no room for, and there is no such thing as `poll_interval_secs` meaning
/// something different under `Idle` than under `Poll`, so a tri-state enum
/// says what a bool plus a comment used to have to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckForMail {
    /// Hold an `IDLE` connection on INBOX for push delivery; every other
    /// mailbox, and INBOX itself as a backstop, still reconciles on
    /// `poll_interval_secs`.
    #[default]
    Idle,
    /// No `IDLE` connection at all -- every mailbox, INBOX included, is
    /// reconciled on `poll_interval_secs` alone. One connection cheaper,
    /// and a fit for a server or a network that does not hold `IDLE` well.
    Poll,
    /// Never checks on its own. `sync_on_startup` and a manual sync command
    /// are the only things that reach the server.
    Manual,
}

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
///
/// [`Deserialize`] is written by hand rather than derived: an existing
/// `config.toml` may still say `idle = true`/`idle = false` (#867), and that
/// has to keep meaning what it always meant -- `Idle`/`Poll` -- rather than
/// silently becoming the new field's default the moment nobody upgrades
/// their file. [`Serialize`] stays derived: whatever a person's file said,
/// this only ever writes `check_for_mail` back, which is what retires the
/// old key from a file this panel has touched.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SyncConfig {
    /// How Postio learns about new mail.
    pub check_for_mail: CheckForMail,
    /// Polling interval for folders without `IDLE`, in seconds.
    pub poll_interval_secs: u64,
    /// Maximum simultaneous IMAP connections per account.
    pub max_connections: u8,
    /// Start a sync as soon as the app opens.
    pub sync_on_startup: bool,
    /// When to download bodies.
    pub body_fetch: BodyFetch,
    /// When to download attachment payloads.
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
    pub max_inline_bytes: u64,
    /// How many messages the first sync reaches back for, newest first.
    pub initial_sync_messages: u32,
    /// Master switch for desktop notifications on new mail.
    pub notify: bool,
    /// Which mailbox roles produce a notification when mail arrives in them —
    /// the stable identifiers `postio_model::MailboxRole::as_str` emits, e.g.
    /// `"inbox"`, `"flagged"`. A role this build does not recognise is
    /// ignored rather than rejected, the same tolerance `extra` gives unknown
    /// keys elsewhere in this file.
    pub notify_roles: Vec<String>,
    /// Keys this version of Postio does not know, preserved verbatim.
    #[serde(flatten)]
    pub extra: Extras,
}

/// [`SyncConfig`]'s deserialize shadow: field for field the same, except
/// `check_for_mail` is optional and the retired `idle` key is still
/// accepted -- see [`SyncConfig`]'s own doc for why this has to be by hand.
#[derive(Deserialize)]
struct RawSyncConfig {
    #[serde(default)]
    check_for_mail: Option<CheckForMail>,
    /// The pre-#867 key. `Some` only when the file still has it; reconciled
    /// against `check_for_mail` in [`SyncConfig::deserialize`] and never
    /// itself written back out.
    #[serde(default)]
    idle: Option<bool>,
    #[serde(default = "poll_interval_secs")]
    poll_interval_secs: u64,
    #[serde(default = "max_connections")]
    max_connections: u8,
    #[serde(default = "crate::yes")]
    sync_on_startup: bool,
    #[serde(default)]
    body_fetch: BodyFetch,
    #[serde(default)]
    attachment_fetch: AttachmentFetch,
    #[serde(default = "max_inline_bytes")]
    max_inline_bytes: u64,
    #[serde(default = "initial_sync_messages")]
    initial_sync_messages: u32,
    #[serde(default = "crate::yes")]
    notify: bool,
    #[serde(default = "notify_roles")]
    notify_roles: Vec<String>,
    #[serde(flatten)]
    extra: Extras,
}

impl<'de> Deserialize<'de> for SyncConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = RawSyncConfig::deserialize(deserializer)?;
        // An explicit `check_for_mail` wins outright -- it is what a file
        // written by this version of Postio, or by hand following this
        // version's own docs, actually said. Only fall back to translating
        // the old key when there is nothing newer to trust.
        let check_for_mail = raw.check_for_mail.unwrap_or(match raw.idle {
            Some(true) | None => CheckForMail::Idle,
            Some(false) => CheckForMail::Poll,
        });
        Ok(SyncConfig {
            check_for_mail,
            poll_interval_secs: raw.poll_interval_secs,
            max_connections: raw.max_connections,
            sync_on_startup: raw.sync_on_startup,
            body_fetch: raw.body_fetch,
            attachment_fetch: raw.attachment_fetch,
            max_inline_bytes: raw.max_inline_bytes,
            initial_sync_messages: raw.initial_sync_messages,
            notify: raw.notify,
            notify_roles: raw.notify_roles,
            extra: raw.extra,
        })
    }
}

impl Default for SyncConfig {
    fn default() -> Self {
        Self {
            check_for_mail: CheckForMail::default(),
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

    // -- Acceptance: check_for_mail and the idle migration (#867) -----------

    #[test]
    fn an_empty_sync_table_checks_for_mail_by_idle() {
        let sync: SyncConfig = toml::from_str("").expect("parse");
        assert_eq!(sync.check_for_mail, CheckForMail::Idle);
    }

    #[test]
    fn check_for_mail_reads_all_three_values() {
        for (text, expected) in [
            ("check_for_mail = \"idle\"\n", CheckForMail::Idle),
            ("check_for_mail = \"poll\"\n", CheckForMail::Poll),
            ("check_for_mail = \"manual\"\n", CheckForMail::Manual),
        ] {
            let sync: SyncConfig = toml::from_str(text).expect("parse");
            assert_eq!(sync.check_for_mail, expected, "{text}");
        }
    }

    #[test]
    fn a_legacy_idle_true_migrates_to_idle() {
        let sync: SyncConfig = toml::from_str("idle = true\n").expect("parse");
        assert_eq!(sync.check_for_mail, CheckForMail::Idle);
    }

    #[test]
    fn a_legacy_idle_false_migrates_to_poll() {
        // Someone who explicitly turned IDLE off to save a connection asked
        // for polling, not for a manual-only mailbox nobody had a word for
        // yet -- the migration must land the same place their intent did.
        let sync: SyncConfig = toml::from_str("idle = false\n").expect("parse");
        assert_eq!(sync.check_for_mail, CheckForMail::Poll);
    }

    #[test]
    fn an_explicit_check_for_mail_wins_over_a_stale_legacy_idle_key() {
        // A file holding both is not a case this migration invented -- it is
        // what a hand-merged config or an old backup restored over a newer
        // one could produce. The key this version of Postio actually reads
        // and writes has to be the one that wins.
        let sync: SyncConfig =
            toml::from_str("check_for_mail = \"manual\"\nidle = true\n").expect("parse");
        assert_eq!(sync.check_for_mail, CheckForMail::Manual);
    }

    #[test]
    fn writing_back_never_reintroduces_the_legacy_idle_key() {
        let sync: SyncConfig = toml::from_str("idle = false\n").expect("parse");
        let written = toml::to_string(&sync).expect("serializes");
        assert!(
            !written.contains("idle ="),
            "the retired key must not survive a write: {written}"
        );
        assert!(written.contains("check_for_mail = \"poll\""), "{written}");
    }
}
