use serde::{Deserialize, Deserializer, Serialize};

use crate::Extras;

/// `Manual` is a third state the old `idle: bool` had no room for (#867).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckForMail {
    /// INBOX still reconciles on `poll_interval_secs` as a backstop.
    #[default]
    Idle,
    /// One connection cheaper; fits a server or network that does not hold `IDLE` well.
    Poll,
    Manual,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BodyFetch {
    #[default]
    Lazy,
    Eager,
}

/// A different question from `BodyFetch`: attachment payloads are ~90% of the bytes and none of the
/// words, so this is the choice between a 1.4 GB store and a 12.4 GB one (ADR 0017).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttachmentFetch {
    /// Metadata is synced for every part, so `has:attachment` and `filename:` answer with nothing
    /// downloaded.
    #[default]
    OnOpen,
    Eager,
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

/// Written to the file as `"all"`.
pub const ALL_MESSAGES: u32 = u32::MAX;

/// The file is something a person reads, and `4294967295` is how a `u32` says no cap, not how a
/// person does.
mod depth {
    use serde::{Deserialize, Deserializer, Serializer};

    use super::ALL_MESSAGES;

    const ALL: &str = "all";

    pub(super) fn serialize<S: Serializer>(count: &u32, serializer: S) -> Result<S::Ok, S::Error> {
        if *count == ALL_MESSAGES {
            serializer.serialize_str(ALL)
        } else {
            serializer.serialize_u32(*count)
        }
    }

    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Written {
        Count(u32),
        Word(String),
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u32, D::Error> {
        match Written::deserialize(deserializer)? {
            Written::Count(count) => Ok(count),
            Written::Word(word) if word == ALL => Ok(ALL_MESSAGES),
            Written::Word(word) => Err(serde::de::Error::custom(format!(
                "`initial_sync_messages` is a number of messages or \"{ALL}\", not \"{word}\""
            ))),
        }
    }
}

fn max_inline_bytes() -> u64 {
    256 * 1024
}

/// A notification for every folder an archive rule quietly files into is noise.
fn notify_roles() -> Vec<String> {
    vec!["inbox".to_owned()]
}

/// `Deserialize` is by hand: an existing file may still say `idle = true`/`false` (#867), which
/// must keep
/// meaning `Idle`/`Poll`. `Serialize` writes only `check_for_mail`, which retires the old key.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SyncConfig {
    pub check_for_mail: CheckForMail,
    pub poll_interval_secs: u64,
    pub max_connections: u8,
    pub sync_on_startup: bool,
    pub body_fetch: BodyFetch,
    pub attachment_fetch: AttachmentFetch,
    /// ADR 0017: inline parts ride with the text. A `cid:` image is part of the sentence the
    /// message is
    /// making and remote images are blocked, so under this figure a part is text; over it, a
    /// payload.
    /// `0` turns the rule off.
    pub max_inline_bytes: u64,
    #[serde(serialize_with = "depth::serialize")]
    pub initial_sync_messages: u32,
    pub notify: bool,
    /// A role this build does not recognise is ignored rather than rejected.
    pub notify_roles: Vec<String>,
    pub badge: bool,
    /// Permanently delete Junk older than this many days, on this machine and
    /// on the server (#1932); `0`, the default, never. Flagged mail is kept.
    pub empty_junk_after_days: u32,
    pub empty_trash_after_days: u32,
    #[serde(flatten)]
    pub extra: Extras,
}

/// Field for field the same, except `check_for_mail` is optional and the retired `idle` key is
/// still accepted.
#[derive(Deserialize)]
struct RawSyncConfig {
    #[serde(default)]
    check_for_mail: Option<CheckForMail>,
    /// The pre-#867 key; never written back out.
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
    #[serde(
        default = "initial_sync_messages",
        deserialize_with = "depth::deserialize"
    )]
    initial_sync_messages: u32,
    #[serde(default = "crate::yes")]
    notify: bool,
    #[serde(default = "notify_roles")]
    notify_roles: Vec<String>,
    #[serde(default = "crate::yes")]
    badge: bool,
    #[serde(default)]
    empty_junk_after_days: u32,
    #[serde(default)]
    empty_trash_after_days: u32,
    #[serde(flatten)]
    extra: Extras,
}

impl<'de> Deserialize<'de> for SyncConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = RawSyncConfig::deserialize(deserializer)?;
        // An explicit `check_for_mail` wins outright; the old key is translated only when there is
        // nothing newer.
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
            badge: raw.badge,
            empty_junk_after_days: raw.empty_junk_after_days,
            empty_trash_after_days: raw.empty_trash_after_days,
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
            badge: true,
            empty_junk_after_days: 0,
            empty_trash_after_days: 0,
            extra: Extras::new(),
        }
    }
}

/// The structured pane's write path (#874). `[sync]` is regenerated whole, so a comment on its
/// header or
/// keys does not survive.
pub fn patch_sync(text: &str, sync: &SyncConfig) -> crate::Result<String> {
    crate::splice::replace_section(text, "sync", &SyncOnly { sync })
}

#[derive(Serialize)]
struct SyncOnly<'a> {
    sync: &'a SyncConfig,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_cap_on_the_first_sync_is_written_as_all_and_read_back() {
        let sync = SyncConfig {
            initial_sync_messages: ALL_MESSAGES,
            ..SyncConfig::default()
        };
        let written = patch_sync("", &sync).expect("patch");
        assert!(
            written.contains("initial_sync_messages = \"all\""),
            "{written}"
        );
        assert!(!written.contains(&u32::MAX.to_string()), "{written}");
        let read: SyncOnlyOwned = toml::from_str(&written).expect("parse");
        assert_eq!(read.sync.initial_sync_messages, ALL_MESSAGES);

        let counted: SyncConfig =
            toml::from_str("initial_sync_messages = 500\n").expect("a count still parses");
        assert_eq!(counted.initial_sync_messages, 500);
        assert!(toml::from_str::<SyncConfig>("initial_sync_messages = \"some\"\n").is_err());
    }

    #[test]
    fn emptying_junk_and_trash_is_off_until_asked_for_and_round_trips() {
        let off = SyncConfig::default();
        assert_eq!(
            (off.empty_junk_after_days, off.empty_trash_after_days),
            (0, 0)
        );
        let on = SyncConfig {
            empty_junk_after_days: 30,
            empty_trash_after_days: 7,
            ..SyncConfig::default()
        };
        let written = patch_sync("", &on).expect("patch");
        assert!(written.contains("empty_junk_after_days = 30"), "{written}");
        let read: SyncOnlyOwned = toml::from_str(&written).expect("parse");
        assert_eq!(read.sync.empty_trash_after_days, 7);
    }

    #[derive(Deserialize)]
    struct SyncOnlyOwned {
        sync: SyncConfig,
    }

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
    fn the_unread_badge_is_on_unless_the_file_turns_it_off() {
        assert!(SyncConfig::default().badge);
        let sync: SyncConfig = toml::from_str("").expect("parse");
        assert!(sync.badge, "a file that never heard of the key shows it");
        let sync: SyncConfig = toml::from_str("badge = false\n").expect("parse");
        assert!(!sync.badge);
    }

    #[test]
    fn the_inline_cap_defaults_to_adr_0017s_figure() {
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
        let sync: SyncConfig = toml::from_str("").expect("parse");
        assert!(sync.notify);
        assert_eq!(sync.notify_roles, vec!["inbox".to_owned()]);
    }

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
        let sync: SyncConfig = toml::from_str("idle = false\n").expect("parse");
        assert_eq!(sync.check_for_mail, CheckForMail::Poll);
    }

    #[test]
    fn an_explicit_check_for_mail_wins_over_a_stale_legacy_idle_key() {
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

    #[test]
    fn patch_sync_rewrites_only_the_sync_table_leaving_everything_else_verbatim() {
        let original = "\
# a hand-written comment nobody wants to lose
[ui]
theme = \"dark\" # inline comment, also not to be lost

[sync]
check_for_mail = \"idle\"
poll_interval_secs = 300
";
        let mut sync: SyncConfig =
            toml::from_str("check_for_mail = \"idle\"\npoll_interval_secs = 300\n").expect("parse");
        sync.check_for_mail = CheckForMail::Manual;
        sync.notify = false;

        let patched = patch_sync(original, &sync).expect("patches");

        assert!(
            patched.contains("# a hand-written comment nobody wants to lose"),
            "a comment outside [sync] must survive verbatim: {patched}"
        );
        assert!(
            patched.contains("theme = \"dark\" # inline comment, also not to be lost"),
            "an unrelated section's own formatting must survive verbatim: {patched}"
        );

        let reparsed: SyncConfig =
            toml::from_str(patched.split("[sync]").nth(1).unwrap_or_default())
                .expect("the patched [sync] table still parses");
        assert_eq!(reparsed.check_for_mail, CheckForMail::Manual);
        assert!(!reparsed.notify);
    }

    #[test]
    fn patch_sync_adds_the_table_when_it_did_not_exist_before() {
        let original = "[ui]\ntheme = \"dark\"\n";
        let sync = SyncConfig::default();

        let patched = patch_sync(original, &sync).expect("patches");
        assert!(patched.contains("[sync]"));
        assert!(patched.contains("theme = \"dark\""));
    }
}
