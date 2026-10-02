//! What Focus says about its own state: the one banner under the header
//! strip, and the sync label in the top bar (specs/007-postio-focus
//! contracts/focus-surface.md, "States"; screens 17-19).
//!
//! One banner at a time, the one that asks something of the person first:
//! a rejected password, then no network, then a first sync. The sync label
//! says the same in a word or two, and otherwise when mail last arrived.
//! Toolkit-free, so the rule is proven here in a second and the window only
//! draws what it is told.

use chrono::{DateTime, Local, TimeZone};
use postio_config::FocusConfig;
use postio_core::{CommandId, ConnectionState, FailureReason, Keymap};
use postio_model::AccountId;

use crate::status::SyncStatus;

/// What an account's banner needs to name: where it signs in, and as whom.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountFacts {
    /// The account.
    pub id: AccountId,
    /// The incoming server's host.
    pub server: String,
    /// The account's address.
    pub address: String,
}

/// The banner under the header strip, when one is shown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Banner {
    /// The server refused an account's password (screen 19).
    SignIn {
        /// The server that refused it.
        server: String,
        /// The address it refused.
        address: String,
    },
    /// The machine has no network (screen 18).
    Offline,
    /// An account's first sync is running (screen 17).
    FirstSync {
        /// Messages fetched so far.
        done: u32,
        /// Messages the pass expects.
        total: u32,
    },
}

/// What a banner's button does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BannerAction {
    /// Open the credential flow for the account that failed.
    UpdatePassword,
    /// Try the network again now rather than at the next backoff.
    Retry,
}

impl Banner {
    /// The banner's heading, drawn in bold.
    pub fn heading(&self) -> String {
        match self {
            Banner::SignIn { server, .. } => format!("Can't sign in to {server}"),
            Banner::Offline => "You're offline".to_owned(),
            Banner::FirstSync { .. } => "First sync".to_owned(),
        }
    }

    /// The sentence after the heading.
    pub fn sentence(&self) -> String {
        match self {
            Banner::SignIn { address, .. } => format!(
                "The server rejected the password for {address}. Mail on this computer is \
                 still available."
            ),
            Banner::Offline => {
                "Everything you do is saved here and syncs when you're back.".to_owned()
            }
            Banner::FirstSync { done, total } => format!(
                "{} of {} messages, newest first. You can read and search what's here.",
                count(*done),
                count(*total)
            ),
        }
    }

    /// What the banner says, as one line: its heading and its sentence.
    pub fn title(&self) -> String {
        format!("{}. {}", self.heading(), self.sentence())
    }

    /// The button's label and what it does, when the banner has one.
    pub fn action(&self) -> Option<(&'static str, BannerAction)> {
        match self {
            Banner::SignIn { .. } => {
                Some(("Update password\u{2026}", BannerAction::UpdatePassword))
            }
            Banner::Offline => Some(("Retry now", BannerAction::Retry)),
            Banner::FirstSync { .. } => None,
        }
    }

    /// How far the first sync has come, from 0 to 1, when it is one.
    pub fn progress(&self) -> Option<f64> {
        match self {
            Banner::FirstSync { done, total } if *total > 0 => {
                Some((f64::from(*done) / f64::from(*total)).clamp(0.0, 1.0))
            }
            _ => None,
        }
    }

    /// Whether the banner is drawn in the error colour.
    pub fn is_error(&self) -> bool {
        matches!(self, Banner::SignIn { .. })
    }
}

/// A pass in flight: fetched so far, and expected. `None` for a pass with
/// nothing to reach, and once it has finished (`SyncStatus`'s own reading).
fn running(status: &SyncStatus) -> Option<(u32, u32)> {
    match status.progress {
        Some((done, total)) if total > 0 && done < total => Some((done, total)),
        _ => None,
    }
}

fn count(value: u32) -> String {
    crate::selection::count(value)
}

/// The banner `statuses` call for, if any: a sign-in error first, then
/// offline, then a first sync.
///
/// Only a refused password is a sign-in banner: a setting that is wrong is
/// said by the sync label and the account's settings, and the banner's one
/// button could not fix it. Only a machine with no network is offline:
/// `Connecting` is backoff that retries on its own, and every launch passes
/// through it. Only a pass with no sync behind it is a first sync.
pub fn banner(statuses: &[(AccountId, SyncStatus)], accounts: &[AccountFacts]) -> Option<Banner> {
    let refused = statuses.iter().find_map(|(account, status)| {
        let refused = status.state
            == ConnectionState::Failing {
                reason: FailureReason::Auth,
            };
        refused
            .then(|| accounts.iter().find(|facts| facts.id == *account))
            .flatten()
    });
    if let Some(facts) = refused {
        return Some(Banner::SignIn {
            server: facts.server.clone(),
            address: facts.address.clone(),
        });
    }
    if statuses
        .iter()
        .any(|(_, status)| status.state == ConnectionState::Offline)
    {
        return Some(Banner::Offline);
    }
    statuses.iter().find_map(|(_, status)| {
        let (done, total) = running(status)?;
        status
            .last_sync
            .is_none()
            .then_some(Banner::FirstSync { done, total })
    })
}

/// What the sync label says, and the icon beside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncLabel {
    /// The words.
    pub text: String,
    /// The symbolic icon's name.
    pub icon: &'static str,
}

/// The sync label for `statuses`, as of `now`: when mail last arrived is
/// said as a clock time in `zone`, `last_synced` being the newest of the
/// accounts' last completed syncs.
pub fn sync_label<Tz: TimeZone>(
    statuses: &[(AccountId, SyncStatus)],
    last_synced: Option<DateTime<chrono::Utc>>,
    zone: &Tz,
) -> SyncLabel
where
    Tz::Offset: std::fmt::Display,
{
    let states = || statuses.iter().map(|(_, status)| status.state);
    if states().any(|state| matches!(state, ConnectionState::Failing { .. })) {
        return SyncLabel {
            text: "Sync failed".to_owned(),
            icon: "dialog-warning-symbolic",
        };
    }
    if states().any(|state| state == ConnectionState::Offline) {
        return SyncLabel {
            text: "Offline".to_owned(),
            icon: "network-offline-symbolic",
        };
    }
    let passes: Vec<(u32, u32)> = statuses
        .iter()
        .filter_map(|(_, status)| running(status))
        .collect();
    if !passes.is_empty() {
        let done = passes.iter().map(|(done, _)| done).sum();
        let total = passes.iter().map(|(_, total)| total).sum();
        return SyncLabel {
            text: format!("Syncing {} of {}", count(done), count(total)),
            icon: "emblem-synchronizing-symbolic",
        };
    }
    if let Some(at) = last_synced {
        return SyncLabel {
            text: format!("Synced {}", at.with_timezone(zone).format("%H:%M")),
            icon: "emblem-ok-symbolic",
        };
    }
    let text = if states().any(|state| state == ConnectionState::Connecting) {
        "Connecting"
    } else {
        "Not synced yet"
    };
    SyncLabel {
        text: text.to_owned(),
        icon: "emblem-synchronizing-symbolic",
    }
}

/// [`sync_label`] in the machine's own time zone.
pub fn sync_label_here(
    statuses: &[(AccountId, SyncStatus)],
    last_synced: Option<DateTime<chrono::Utc>>,
) -> SyncLabel {
    sync_label(statuses, last_synced, &Local)
}

/// What the empty inbox says under "Inbox is empty" (screen 16): when the
/// next digest comes, if there are digests, and shortcuts to only what
/// exists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmptyInbox {
    /// "Next digest: Weekly · Newsletters, Saturday 16:00", when a digest
    /// rule names a time.
    pub next_digest: Option<String>,
    /// The bold line: "Inbox is empty" once a pass has finished and found
    /// nothing, and otherwise what the inbox is waiting on.
    pub heading: String,
    /// The line under the heading when it is not the digest's: when mail
    /// last synced, or how far the first sync has come.
    pub detail: Option<String>,
    /// Each shortcut: its key under the keymap in force, what it says, and
    /// the command a click runs.
    pub shortcuts: Vec<(Option<String>, String, CommandId)>,
}

/// What an inbox with no rows may say (T220). "Empty" is a claim that a
/// pass has finished and found nothing; before that, no rows only means
/// mail has not arrived.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InboxSaying {
    /// A pass has completed: the inbox really is empty.
    Empty {
        /// When mail last synced, if known.
        last_synced: Option<DateTime<chrono::Utc>>,
    },
    /// No pass has completed yet and sync is on its way or running.
    Syncing {
        /// Messages fetched so far and expected, once the engine reports.
        progress: Option<(u32, u32)>,
    },
    /// No pass has completed and there is no network.
    Offline,
    /// No pass has completed and sync is failing.
    Failed,
}

/// What an inbox with no rows says, given every account's sync `statuses`
/// and when mail `last_synced` (a completed pass, this run or before).
pub fn inbox_saying(
    statuses: &[(AccountId, SyncStatus)],
    last_synced: Option<DateTime<chrono::Utc>>,
) -> InboxSaying {
    let passed = last_synced.is_some() || statuses.iter().any(|(_, s)| s.last_sync.is_some());
    if passed {
        return InboxSaying::Empty { last_synced };
    }
    let states = || statuses.iter().map(|(_, status)| status.state);
    if states().any(|state| matches!(state, ConnectionState::Failing { .. })) {
        return InboxSaying::Failed;
    }
    if states().any(|state| state == ConnectionState::Offline) {
        // A tracker that has heard nothing reads Offline too; only a
        // report of progress or a connection says otherwise.
        let heard = statuses
            .iter()
            .any(|(_, s)| s.state != ConnectionState::Offline || s.progress.is_some());
        if !heard {
            return InboxSaying::Offline;
        }
    }
    let passes: Vec<(u32, u32)> = statuses
        .iter()
        .filter_map(|(_, status)| running(status))
        .collect();
    let progress = (!passes.is_empty()).then(|| {
        (
            passes.iter().map(|(done, _)| done).sum(),
            passes.iter().map(|(_, total)| total).sum(),
        )
    });
    InboxSaying::Syncing { progress }
}

impl EmptyInbox {
    /// This page as `saying` has it: the words that say why there are no
    /// rows, and only the shortcuts that still make sense. `zone` is for
    /// the clock time; `keymap` for the key that syncs again.
    pub fn saying<Tz: TimeZone>(mut self, saying: &InboxSaying, keymap: &Keymap, zone: &Tz) -> Self
    where
        Tz::Offset: std::fmt::Display,
    {
        let retry = crate::hints::key(keymap, CommandId::Refresh)
            .map(|key| format!(" Press {key} to sync again."))
            .unwrap_or_default();
        match saying {
            InboxSaying::Empty { last_synced } => {
                self.detail = last_synced
                    .map(|at| format!("Synced {}", at.with_timezone(zone).format("%H:%M")));
                return self;
            }
            InboxSaying::Syncing { progress } => {
                "Syncing your inbox\u{2026}".clone_into(&mut self.heading);
                self.detail = Some(match progress {
                    Some((done, total)) => format!(
                        "{} of {} messages, newest first. Mail shows here as it arrives.",
                        count(*done),
                        count(*total)
                    ),
                    None => "Mail shows here as it arrives.".to_owned(),
                });
            }
            InboxSaying::Offline => {
                "Your inbox hasn't synced yet".clone_into(&mut self.heading);
                self.detail =
                    Some("You're offline. Mail shows here once the first sync runs.".to_owned());
            }
            InboxSaying::Failed => {
                "Your inbox hasn't synced yet".clone_into(&mut self.heading);
                self.detail = Some(format!("Sync failed.{retry}"));
            }
        }
        // Nothing has been filtered or archived that anyone could count yet.
        self.next_digest = None;
        self.shortcuts
            .retain(|(_, _, command)| *command == CommandId::Compose);
        self
    }
}

/// The empty inbox for `focus`'s rules, `filtered_today` messages filed
/// away since midnight, and `keymap`'s keys, as of `now`.
pub fn empty_inbox<Tz: TimeZone>(
    focus: &FocusConfig,
    filtered_today: u32,
    keymap: &Keymap,
    now: &DateTime<Tz>,
) -> EmptyInbox
where
    Tz::Offset: std::fmt::Display,
{
    let next_digest = focus
        .digests
        .iter()
        .filter_map(|rule| {
            let due = rule.due().ok()?;
            let when = crate::schedule::next_due(&due, now)?;
            Some((when, rule))
        })
        .min_by_key(|(when, _)| when.clone())
        .map(|(when, rule)| {
            // A week ahead or less, the weekday says it; further, the date.
            let day = if when.clone() - now.clone() < chrono::Duration::days(7) {
                when.format("%A %H:%M").to_string()
            } else {
                when.format("%A %-d %B %H:%M").to_string()
            };
            format!(
                "Next digest: {} \u{b7} {}, {day}",
                cadence(&rule.cadence),
                rule.name
            )
        });
    let mut shortcuts = Vec::new();
    if focus.filtering {
        shortcuts.push((
            crate::hints::key(keymap, CommandId::GoToFiltered),
            format!("{} filtered today", count(filtered_today)),
            CommandId::GoToFiltered,
        ));
    }
    shortcuts.push((
        crate::hints::key(keymap, CommandId::GoToArchive),
        "archive".to_owned(),
        CommandId::GoToArchive,
    ));
    shortcuts.push((
        crate::hints::key(keymap, CommandId::Compose),
        "compose".to_owned(),
        CommandId::Compose,
    ));
    EmptyInbox {
        heading: "Inbox is empty".to_owned(),
        detail: None,
        next_digest,
        shortcuts,
    }
}

/// A rule's cadence as the line says it: "Weekly".
fn cadence(written: &str) -> String {
    let written = written.trim().to_ascii_lowercase();
    let mut letters = written.chars();
    match letters.next() {
        Some(first) => first.to_uppercase().chain(letters).collect(),
        None => written,
    }
}

/// What the reading pane says with no message open (T232): never blank,
/// and never a dead end -- the key that opens one, and the key that puts
/// messages back over the list, each a shortcut a click runs too. Drawn by
/// the empty inbox's own page, so Focus has one empty pattern.
pub fn empty_pane(keymap: &Keymap) -> EmptyInbox {
    let key = |command| crate::hints::key(keymap, command).map(|key| crate::hints::short(&key));
    EmptyInbox {
        next_digest: None,
        heading: "No message open".to_owned(),
        detail: Some("Messages open here, beside the list.".to_owned()),
        shortcuts: vec![
            (
                key(CommandId::OpenMessage),
                "open".to_owned(),
                CommandId::OpenMessage,
            ),
            (
                key(CommandId::ToggleReadingPane),
                "read over the list".to_owned(),
                CommandId::ToggleReadingPane,
            ),
        ],
    }
}

#[cfg(test)]
mod tests {
    use chrono::{FixedOffset, TimeZone, Utc};

    use super::*;

    fn account(n: i64) -> AccountId {
        AccountId::new(n)
    }

    fn facts(n: i64) -> AccountFacts {
        AccountFacts {
            id: account(n),
            server: format!("imap{n}.example.com"),
            address: format!("ada{n}@example.com"),
        }
    }

    fn status(state: ConnectionState, progress: Option<(u32, u32)>) -> SyncStatus {
        SyncStatus {
            state,
            progress,
            ..SyncStatus::default()
        }
    }

    const AUTH: ConnectionState = ConnectionState::Failing {
        reason: FailureReason::Auth,
    };

    #[test]
    fn a_rejected_password_comes_first_then_offline_then_a_first_sync() {
        let accounts = [facts(1), facts(2), facts(3)];
        let everything = [
            (
                account(1),
                status(ConnectionState::Online, Some((12_408, 18_204))),
            ),
            (account(2), status(ConnectionState::Offline, None)),
            (account(3), status(AUTH, None)),
        ];
        assert_eq!(
            banner(&everything, &accounts),
            Some(Banner::SignIn {
                server: "imap3.example.com".into(),
                address: "ada3@example.com".into(),
            })
        );
        assert_eq!(banner(&everything[..2], &accounts), Some(Banner::Offline));
        assert_eq!(
            banner(&everything[..1], &accounts),
            Some(Banner::FirstSync {
                done: 12_408,
                total: 18_204
            })
        );
    }

    #[test]
    fn nothing_to_say_says_nothing() {
        let accounts = [facts(1)];
        let calm = [(account(1), status(ConnectionState::Online, None))];
        assert_eq!(banner(&calm, &accounts), None);
        // Backoff retries on its own: nothing for the person to do, and every
        // launch passes through it.
        let connecting = [(account(1), status(ConnectionState::Connecting, None))];
        assert_eq!(banner(&connecting, &accounts), None);
    }

    #[test]
    fn a_sync_after_the_first_raises_no_banner() {
        let accounts = [facts(1)];
        let again = SyncStatus {
            last_sync: Some(std::time::Instant::now()),
            ..status(ConnectionState::Online, Some((3, 40)))
        };
        assert_eq!(banner(&[(account(1), again)], &accounts), None);
    }

    #[test]
    fn a_config_failure_is_not_a_password() {
        let accounts = [facts(1)];
        let config = [(
            account(1),
            status(
                ConnectionState::Failing {
                    reason: FailureReason::Config,
                },
                None,
            ),
        )];
        assert_eq!(banner(&config, &accounts), None);
    }

    #[test]
    fn each_banner_reads_as_the_contract_says() {
        let sign_in = Banner::SignIn {
            server: "imap.example.com".into(),
            address: "ada@example.com".into(),
        };
        assert_eq!(
            sign_in.title(),
            "Can't sign in to imap.example.com. The server rejected the password for \
             ada@example.com. Mail on this computer is still available."
        );
        assert_eq!(
            sign_in.action(),
            Some(("Update password\u{2026}", BannerAction::UpdatePassword))
        );
        assert!(sign_in.is_error());
        assert_eq!(sign_in.progress(), None);

        assert_eq!(
            Banner::Offline.title(),
            "You're offline. Everything you do is saved here and syncs when you're back."
        );
        assert_eq!(
            Banner::Offline.action(),
            Some(("Retry now", BannerAction::Retry))
        );
        assert!(!Banner::Offline.is_error());

        let first = Banner::FirstSync {
            done: 12_408,
            total: 18_204,
        };
        assert_eq!(
            first.title(),
            "First sync. 12,408 of 18,204 messages, newest first. You can read and search \
             what's here."
        );
        assert_eq!(first.action(), None);
        let progress = first.progress().expect("a first sync has progress");
        assert!((progress - 12_408.0 / 18_204.0).abs() < 1e-9);
    }

    #[test]
    fn each_banner_is_a_heading_and_a_sentence() {
        let sign_in = Banner::SignIn {
            server: "imap.example.com".into(),
            address: "ada@example.com".into(),
        };
        assert_eq!(sign_in.heading(), "Can't sign in to imap.example.com");
        assert_eq!(
            sign_in.sentence(),
            "The server rejected the password for ada@example.com. Mail on this computer is \
             still available."
        );
        assert_eq!(Banner::Offline.heading(), "You're offline");
        let first = Banner::FirstSync { done: 5, total: 9 };
        assert_eq!(first.heading(), "First sync");
        assert_eq!(
            first.sentence(),
            "5 of 9 messages, newest first. You can read and search what's here."
        );
        // The one line is the two, joined.
        for banner in [sign_in, Banner::Offline, first] {
            assert_eq!(
                banner.title(),
                format!("{}. {}", banner.heading(), banner.sentence())
            );
        }
    }

    #[test]
    fn the_label_says_the_same_in_a_word_or_two() {
        let zone = FixedOffset::east_opt(0).expect("UTC");
        let synced = Utc.with_ymd_and_hms(2026, 9, 26, 16, 9, 0).single();
        let label = |state, progress| {
            sync_label(&[(account(1), status(state, progress))], synced, &zone).text
        };
        assert_eq!(label(AUTH, None), "Sync failed");
        assert_eq!(label(ConnectionState::Offline, None), "Offline");
        assert_eq!(
            label(ConnectionState::Online, Some((12_408, 18_204))),
            "Syncing 12,408 of 18,204"
        );
        assert_eq!(label(ConnectionState::Online, None), "Synced 16:09");
        // Reconnecting after a sync: when mail last arrived is still true.
        assert_eq!(label(ConnectionState::Connecting, None), "Synced 16:09");
        let never = sync_label(
            &[(account(1), status(ConnectionState::Connecting, None))],
            None,
            &zone,
        );
        assert_eq!(never.text, "Connecting");
        let idle = sync_label(
            &[(account(1), status(ConnectionState::Online, None))],
            None,
            &zone,
        );
        assert_eq!(idle.text, "Not synced yet");
    }

    fn rule(name: &str, cadence: &str, day: Option<&str>, at: &str) -> postio_config::DigestRule {
        let day = day.map(|day| match day.parse::<i64>() {
            Ok(number) => format!("day = {number}\n"),
            Err(_) => format!("day = \"{day}\"\n"),
        });
        let text = format!(
            "[[focus.digests]]\nname = \"{name}\"\nqueries = [\"from:news@example.com\"]\n\
             cadence = \"{cadence}\"\n{}at = \"{at}\"\n",
            day.unwrap_or_default()
        );
        postio_config::Config::from_toml_str(&text)
            .expect("a digest rule")
            .focus
            .digests
            .remove(0)
    }

    fn wednesday() -> DateTime<FixedOffset> {
        FixedOffset::east_opt(0)
            .expect("UTC")
            .with_ymd_and_hms(2026, 9, 23, 10, 0, 0)
            .single()
            .expect("a date")
    }

    #[test]
    fn the_empty_inbox_names_only_what_exists() {
        let keymap = Keymap::defaults();
        let plain = FocusConfig {
            filtering: false,
            ..FocusConfig::default()
        };
        let empty = empty_inbox(&plain, 0, keymap, &wednesday());
        assert_eq!(empty.next_digest, None, "no digests, no next digest");
        assert_eq!(
            empty.shortcuts,
            vec![
                (
                    Some("g r".to_owned()),
                    "archive".to_owned(),
                    CommandId::GoToArchive
                ),
                (
                    Some("c".to_owned()),
                    "compose".to_owned(),
                    CommandId::Compose
                ),
            ],
            "no filtering, no filtered count"
        );

        let everything = FocusConfig {
            filtering: true,
            digests: vec![
                rule("Receipts", "monthly", Some("15"), "09:00"),
                rule("Newsletters", "weekly", Some("saturday"), "16:00"),
                rule("Broken", "fortnightly", None, "08:00"),
            ],
            ..FocusConfig::default()
        };
        let empty = empty_inbox(&everything, 186, keymap, &wednesday());
        assert_eq!(
            empty.next_digest.as_deref(),
            Some("Next digest: Weekly \u{b7} Newsletters, Saturday 16:00"),
            "the soonest rule that names a time"
        );
        assert_eq!(
            empty.shortcuts[0],
            (
                Some("g f".to_owned()),
                "186 filtered today".to_owned(),
                CommandId::GoToFiltered
            )
        );
        assert_eq!(empty.shortcuts.len(), 3);
    }

    #[test]
    fn a_digest_more_than_a_week_off_names_its_date() {
        let focus = FocusConfig {
            digests: vec![rule("Receipts", "monthly", Some("15"), "09:00")],
            ..FocusConfig::default()
        };
        let empty = empty_inbox(&focus, 0, Keymap::defaults(), &wednesday());
        assert_eq!(
            empty.next_digest.as_deref(),
            Some("Next digest: Monthly \u{b7} Receipts, Thursday 15 October 09:00")
        );
    }

    fn at(hour: u32) -> Option<DateTime<chrono::Utc>> {
        chrono::TimeZone::with_ymd_and_hms(&chrono::Utc, 2026, 10, 1, hour, 0, 0).single()
    }

    #[test]
    fn an_inbox_is_empty_only_once_a_pass_has_finished() {
        let heard = status(ConnectionState::Online, Some((120, 300)));
        assert_eq!(
            inbox_saying(&[(account(1), heard)], None),
            InboxSaying::Syncing {
                progress: Some((120, 300))
            }
        );
        assert_eq!(
            inbox_saying(&[], None),
            InboxSaying::Syncing { progress: None },
            "before anything has been heard"
        );
        let done = status(ConnectionState::Online, None);
        assert_eq!(
            inbox_saying(&[(account(1), done)], at(9)),
            InboxSaying::Empty { last_synced: at(9) }
        );
    }

    #[test]
    fn a_first_sync_that_cannot_run_says_so() {
        assert_eq!(
            inbox_saying(&[(account(1), status(AUTH, None))], None),
            InboxSaying::Failed
        );
        let mut offline = status(ConnectionState::Offline, None);
        offline.state = ConnectionState::Offline;
        assert_eq!(
            inbox_saying(&[(account(1), offline)], None),
            InboxSaying::Offline
        );
    }

    #[test]
    fn the_words_say_why_there_are_no_rows() {
        let keymap = Keymap::defaults();
        let zone = FixedOffset::east_opt(0).expect("UTC");
        let page = || empty_inbox(&FocusConfig::default(), 0, keymap, &wednesday());
        let syncing = page().saying(
            &InboxSaying::Syncing {
                progress: Some((120, 300)),
            },
            keymap,
            &zone,
        );
        assert_eq!(syncing.heading, "Syncing your inbox\u{2026}");
        assert_eq!(
            syncing.detail.as_deref(),
            Some("120 of 300 messages, newest first. Mail shows here as it arrives.")
        );
        let empty = page().saying(&InboxSaying::Empty { last_synced: at(9) }, keymap, &zone);
        assert_eq!(empty.heading, "Inbox is empty");
        assert_eq!(empty.detail.as_deref(), Some("Synced 09:00"));
        let failed = page().saying(&InboxSaying::Failed, keymap, &zone);
        assert_eq!(failed.heading, "Your inbox hasn't synced yet");
        assert!(failed.detail.expect("a detail").starts_with("Sync failed."));
    }

    #[test]
    fn the_label_puts_the_worst_account_first() {
        let zone = FixedOffset::east_opt(0).expect("UTC");
        let label = sync_label(
            &[
                (account(1), status(ConnectionState::Online, None)),
                (account(2), status(AUTH, None)),
            ],
            None,
            &zone,
        );
        assert_eq!(label.text, "Sync failed");
        assert_eq!(label.icon, "dialog-warning-symbolic");
    }

    #[test]
    fn the_empty_pane_names_what_opens_a_message_and_the_way_back() {
        let said = empty_pane(Keymap::defaults());
        assert_eq!(said.heading, "No message open");
        assert_eq!(
            said.detail.as_deref(),
            Some("Messages open here, beside the list.")
        );
        assert_eq!(
            said.shortcuts,
            vec![
                (Some("\u{21b5}".to_owned()), "open".to_owned(), CommandId::OpenMessage),
                (
                    Some("F8".to_owned()),
                    "read over the list".to_owned(),
                    CommandId::ToggleReadingPane
                ),
            ]
        );
        assert_eq!(said.next_digest, None);
    }
}
