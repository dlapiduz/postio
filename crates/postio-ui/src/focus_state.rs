//! What Focus says about its own state: the one banner under the header strip (the one that
//! asks something of the person first) and the sync label in the top bar.

use chrono::{DateTime, Local, TimeZone};
use postio_config::FocusConfig;
use postio_core::{CommandId, ConnectionState, FailureReason, Keymap};
use postio_model::{AccountId, MailboxId};

use crate::status::SyncStatus;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountFacts {
    pub id: AccountId,
    pub server: String,
    pub address: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Banner {
    SignIn {
        server: String,
        address: String,
        missing: bool,
    },
    Failing {
        account: String,
        reason: String,
    },
    Offline,
    FirstSync {
        done: u32,
        total: u32,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BannerAction {
    UpdatePassword,
    Retry,
}

impl Banner {
    pub fn heading(&self) -> String {
        match self {
            Banner::SignIn { server, .. } => format!("Can't sign in to {server}"),
            Banner::Failing { account, .. } => format!("{account} can't sync"),
            Banner::Offline => "You're offline".to_owned(),
            Banner::FirstSync { .. } => "First sync".to_owned(),
        }
    }

    pub fn sentence(&self) -> String {
        match self {
            Banner::SignIn {
                address,
                missing: true,
                ..
            } => format!(
                "Postio has no password saved for {address}. Mail on this computer is \
                 still available."
            ),
            Banner::SignIn { address, .. } => format!(
                "The server rejected the password for {address}. Mail on this computer is \
                 still available."
            ),
            Banner::Failing { reason, .. } => format!(
                "{reason} Mail already on this computer, from every account, is still here."
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

    pub fn title(&self) -> String {
        format!("{}. {}", self.heading(), self.sentence())
    }

    pub fn action(&self) -> Option<(&'static str, BannerAction)> {
        match self {
            Banner::SignIn { .. } => {
                Some(("Update password\u{2026}", BannerAction::UpdatePassword))
            }
            Banner::Failing { .. } | Banner::Offline => Some(("Retry now", BannerAction::Retry)),
            Banner::FirstSync { .. } => None,
        }
    }

    pub fn progress(&self) -> Option<f64> {
        match self {
            Banner::FirstSync { done, total } if *total > 0 => {
                Some((f64::from(*done) / f64::from(*total)).clamp(0.0, 1.0))
            }
            _ => None,
        }
    }

    pub fn is_error(&self) -> bool {
        matches!(self, Banner::SignIn { .. } | Banner::Failing { .. })
    }
}

/// `None` for a pass with nothing to reach, and once it has finished.
fn running(status: &SyncStatus) -> Option<(u32, u32)> {
    match status.progress {
        Some((done, total)) if total > 0 && done < total => Some((done, total)),
        _ => None,
    }
}

fn count(value: u32) -> String {
    crate::selection::count(value)
}

/// Only a refused or missing password is a sign-in banner: retrying a rejected credential is
/// how an account gets locked. Only no network is offline (`Connecting` is backoff that
/// retries itself). Only a pass with no sync behind it is a first sync.
pub fn banner(statuses: &[(AccountId, SyncStatus)], accounts: &[AccountFacts]) -> Option<Banner> {
    let refused = statuses.iter().find_map(|(account, status)| {
        let ConnectionState::Failing { reason } = status.state else {
            return None;
        };
        let missing = reason == FailureReason::NoPassword;
        (reason == FailureReason::Auth || missing)
            .then(|| accounts.iter().find(|facts| facts.id == *account))
            .flatten()
            .map(|facts| (facts, missing))
    });
    if let Some((facts, missing)) = refused {
        return Some(Banner::SignIn {
            server: facts.server.clone(),
            address: facts.address.clone(),
            missing,
        });
    }
    let failing = statuses.iter().find_map(|(account, status)| {
        let ConnectionState::Failing { reason } = status.state else {
            return None;
        };
        let facts = accounts.iter().find(|facts| facts.id == *account)?;
        Some(Banner::Failing {
            account: facts.name.clone(),
            // The sync's own words, else the words for the kind of failure (ADR 0005 Q10).
            reason: status
                .detail
                .clone()
                .unwrap_or_else(|| failure_words(reason).to_owned()),
        })
    });
    if failing.is_some() {
        return failing;
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

fn failure_words(reason: FailureReason) -> &'static str {
    match reason {
        FailureReason::Auth => "The server rejected the password.",
        FailureReason::NoPassword => "Postio has no password saved for this account.",
        FailureReason::Network => "The server can't be reached. It will try again on its own.",
        FailureReason::Server => "The server is refusing the work. It will try again, slower.",
        FailureReason::Config => "The account's settings are wrong. Check them in settings.",
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncLabel {
    pub text: String,
    pub icon: &'static str,
    pub detail: Option<String>,
}

impl SyncLabel {
    fn plain(text: impl Into<String>, icon: &'static str) -> Self {
        SyncLabel {
            text: text.into(),
            icon,
            detail: None,
        }
    }
}

pub fn syncing(done: u32, total: u32) -> String {
    format!("Syncing {} of {}", count(done), count(total))
}

/// The link speaks first (failing, then offline), then a running pass, then a folder whose
/// last pass failed (#1731), and only then when mail last synced: "Synced 16:09" claims a
/// comparison a failed folder never had.
pub fn sync_label<Tz: TimeZone>(
    statuses: &[(AccountId, SyncStatus)],
    last_synced: Option<DateTime<chrono::Utc>>,
    folders: &[(MailboxId, String)],
    zone: &Tz,
) -> SyncLabel
where
    Tz::Offset: std::fmt::Display,
{
    let states = || statuses.iter().map(|(_, status)| status.state);
    if states().any(|state| matches!(state, ConnectionState::Failing { .. })) {
        return SyncLabel::plain("Sync failed", "dialog-warning-symbolic");
    }
    if states().any(|state| state == ConnectionState::Offline) {
        return SyncLabel::plain("Offline", "network-offline-symbolic");
    }
    let passes: Vec<(u32, u32)> = statuses
        .iter()
        .filter_map(|(_, status)| running(status).or_else(|| status.backfill_running()))
        .collect();
    if !passes.is_empty() {
        let done = passes.iter().map(|(done, _)| done).sum();
        let total = passes.iter().map(|(_, total)| total).sum();
        return SyncLabel::plain(syncing(done, total), "emblem-synchronizing-symbolic");
    }
    if let Some(label) = cannot_sync(statuses, folders) {
        return label;
    }
    if let Some(at) = last_synced {
        return SyncLabel::plain(
            format!("Synced {}", at.with_timezone(zone).format("%H:%M")),
            "emblem-ok-symbolic",
        );
    }
    let text = if states().any(|state| state == ConnectionState::Connecting) {
        "Connecting"
    } else {
        "Not synced yet"
    };
    SyncLabel::plain(text, "emblem-synchronizing-symbolic")
}

fn cannot_sync(
    statuses: &[(AccountId, SyncStatus)],
    folders: &[(MailboxId, String)],
) -> Option<SyncLabel> {
    let failing: Vec<(Option<&str>, &str)> = statuses
        .iter()
        .flat_map(|(_, status)| &status.failing)
        .map(|(mailbox, reason)| {
            let name = folders
                .iter()
                .find(|(each, _)| each == mailbox)
                .map(|(_, name)| name.as_str());
            (name, reason.as_str())
        })
        .collect();
    let text = match failing.as_slice() {
        [] => return None,
        [(Some(name), _)] => format!("{name} can't sync"),
        [(None, _)] => "A folder can't sync".to_owned(),
        many => format!("{} folders can't sync", many.len()),
    };
    let detail = failing
        .iter()
        .map(|(name, reason)| match name {
            Some(name) => format!("{name}: {reason}"),
            None => (*reason).to_owned(),
        })
        .collect::<Vec<_>>()
        .join("\n");
    Some(SyncLabel {
        text,
        icon: "dialog-warning-symbolic",
        detail: Some(detail),
    })
}

pub fn sync_label_here(
    statuses: &[(AccountId, SyncStatus)],
    last_synced: Option<DateTime<chrono::Utc>>,
    folders: &[(MailboxId, String)],
) -> SyncLabel {
    sync_label(statuses, last_synced, folders, &Local)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmptyInbox {
    pub next_digest: Option<String>,
    pub heading: String,
    pub detail: Option<String>,
    pub shortcuts: Vec<(Option<String>, String, CommandId)>,
}

/// "Empty" claims a pass finished and found nothing; before that, no rows only means mail has not
/// arrived.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InboxSaying {
    Empty {
        last_synced: Option<DateTime<chrono::Utc>>,
    },
    Syncing {
        progress: Option<(u32, u32)>,
    },
    Offline,
    Failed,
}

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
        self.next_digest = None;
        self.shortcuts
            .retain(|(_, _, command)| *command == CommandId::Compose);
        self
    }
}

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

fn cadence(written: &str) -> String {
    let written = written.trim().to_ascii_lowercase();
    let mut letters = written.chars();
    match letters.next() {
        Some(first) => first.to_uppercase().chain(letters).collect(),
        None => written,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EmptyPlace {
    Snoozed,
    Flagged,
    Drafts,
    Outbox,
    Folder(String),
}

/// Why a place is empty and the way out, never a blank page that looks like a failed load.
/// The same page as the empty inbox, so Focus has one empty pattern.
pub fn empty_place(place: &EmptyPlace, keymap: &Keymap) -> EmptyInbox {
    let key = |command| crate::hints::key(keymap, command);
    let press = |command, what: &str| match key(command) {
        Some(key) => format!("Press {} {what}", crate::hints::short(&key)),
        None => {
            let mut said = what.to_owned();
            said.get_mut(..1).map(str::make_ascii_uppercase);
            said
        }
    };
    let (heading, detail, extra) = match place {
        EmptyPlace::Snoozed => (
            "Nothing snoozed".to_owned(),
            press(
                CommandId::Snooze,
                "on a message in the list to snooze it; it comes back when you said.",
            ),
            None,
        ),
        EmptyPlace::Flagged => (
            "Nothing flagged".to_owned(),
            press(
                CommandId::Flag,
                "on a message in the list to flag it, and it waits here.",
            ),
            None,
        ),
        EmptyPlace::Drafts => (
            "No drafts".to_owned(),
            "A message you start and leave is kept here until you send it.".to_owned(),
            Some(CommandId::Compose),
        ),
        EmptyPlace::Outbox => (
            "Nothing waiting to send".to_owned(),
            "A message you send waits here until it is on its way; a cancelled send goes back to Drafts."
                .to_owned(),
            Some(CommandId::Compose),
        ),
        EmptyPlace::Folder(name) => (
            format!("{name} is empty"),
            "Mail filed here shows up as it arrives.".to_owned(),
            None,
        ),
    };
    let mut shortcuts = vec![(
        key(CommandId::GoToInbox),
        "back to the inbox".to_owned(),
        CommandId::GoToInbox,
    )];
    if let Some(command) = extra {
        shortcuts.push((key(command), "compose".to_owned(), command));
    }
    EmptyInbox {
        heading,
        detail: Some(detail),
        next_digest: None,
        shortcuts,
    }
}

/// Never blank and never a dead end: the key that opens a message and the key that puts
/// messages back over the list. Drawn by the empty inbox's page.
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

pub const CANT_OPEN_MAIL: &str = "Postio can\u{2019}t open your mail";

pub const TRY_AGAIN: &str = "Try again";

pub const STORE_FROM_ANOTHER_VERSION: &str = "Your mail store is from another version of Postio";

pub const START_OVER: &str = "This version of Postio can\u{2019}t read the store an earlier \
     build wrote, and no update carries it forward, so trying again won\u{2019}t help. \
     A fresh store keeps your accounts and settings and syncs your mail again from the \
     server. Snoozes, reminders, Focus\u{2019}s filing history, and drafts or changes \
     not yet sent stay in the old store, which is set aside, not deleted.";

pub const STORE_UNDER_ANOTHER_KEY: &str = "Your mail store is locked with another key";

/// Nothing in the old store can be read, so unlike `START_OVER` it promises no accounts.
pub const START_OVER_UNDER_ANOTHER_KEY: &str = "This store was encrypted with a key \
     this installation doesn\u{2019}t have \u{2014} another installation\u{2019}s, or one \
     the keyring replaced \u{2014} so trying again won\u{2019}t help. A fresh store asks \
     for your accounts again and syncs your mail again from the server. The old store, \
     with its snoozes, reminders, and unsent drafts, is set aside, not deleted.";

pub const START_A_FRESH_STORE: &str = "Start a fresh store";

pub const STARTING_A_FRESH_STORE: &str = "Starting a fresh store\u{2026}";

pub fn started_over(set_aside: &str) -> String {
    format!("Started a fresh store. The old one is in {set_aside}")
}

#[cfg(test)]
mod tests {

    #[test]
    fn a_store_started_over_says_where_the_old_one_is() {
        assert_eq!(
            started_over("/stores/set-aside/when"),
            "Started a fresh store. The old one is in /stores/set-aside/when"
        );
        assert!(START_OVER.contains("set aside, not deleted"));
    }

    #[test]
    fn a_store_under_another_key_promises_no_accounts() {
        assert!(START_OVER_UNDER_ANOTHER_KEY.contains("set aside, not deleted"));
        assert!(START_OVER_UNDER_ANOTHER_KEY.contains("won\u{2019}t help"));
        assert!(!START_OVER_UNDER_ANOTHER_KEY.contains("keeps your accounts"));
    }

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
            name: format!("Work {n}"),
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
    fn syncing_counts_with_separators() {
        assert_eq!(syncing(3, 9), "Syncing 3 of 9");
        assert_eq!(syncing(1_200, 8_400), "Syncing 1,200 of 8,400");
    }

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
                missing: false,
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
    fn one_account_failing_for_another_reason_is_named_with_its_reason() {
        let accounts = [facts(1), facts(2)];
        let failing = SyncStatus {
            detail: Some("The server's certificate does not verify.".into()),
            ..status(
                ConnectionState::Failing {
                    reason: FailureReason::Config,
                },
                None,
            )
        };
        let statuses = [
            (account(1), status(ConnectionState::Online, None)),
            (account(2), failing),
        ];
        let shown = banner(&statuses, &accounts).expect("a failing account is said");
        assert_eq!(
            shown,
            Banner::Failing {
                account: "Work 2".into(),
                reason: "The server's certificate does not verify.".into(),
            }
        );
        assert_eq!(shown.heading(), "Work 2 can't sync");
        assert_eq!(
            shown.sentence(),
            "The server's certificate does not verify. Mail already on this computer, \
             from every account, is still here."
        );
        assert_eq!(
            shown.action(),
            Some(("Retry now", BannerAction::Retry)),
            "a failure that is not the password offers Retry"
        );
        assert!(shown.is_error());
        let both = [
            (
                account(1),
                status(
                    ConnectionState::Failing {
                        reason: FailureReason::Auth,
                    },
                    None,
                ),
            ),
            statuses[1].clone(),
        ];
        assert!(matches!(
            banner(&both, &accounts),
            Some(Banner::SignIn { .. })
        ));
    }

    #[test]
    fn a_failure_with_no_words_still_says_so() {
        let accounts = [facts(1)];
        let silent = [(
            account(1),
            status(
                ConnectionState::Failing {
                    reason: FailureReason::Server,
                },
                None,
            ),
        )];
        let Some(Banner::Failing { reason, .. }) = banner(&silent, &accounts) else {
            panic!("a failing account is said");
        };
        assert_eq!(
            reason,
            "The server is refusing the work. It will try again, slower."
        );
    }

    #[test]
    fn nothing_to_say_says_nothing() {
        let accounts = [facts(1)];
        let calm = [(account(1), status(ConnectionState::Online, None))];
        assert_eq!(banner(&calm, &accounts), None);
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
        assert!(!matches!(
            banner(&config, &accounts),
            Some(Banner::SignIn { .. })
        ));
    }

    #[test]
    fn each_banner_reads_as_the_contract_says() {
        let sign_in = Banner::SignIn {
            server: "imap.example.com".into(),
            address: "ada@example.com".into(),
            missing: false,
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
    fn a_missing_password_is_told_apart_from_a_rejected_one() {
        let accounts = [facts(1)];
        let none = ConnectionState::Failing {
            reason: FailureReason::NoPassword,
        };
        let shown = banner(&[(account(1), status(none, None))], &accounts)
            .expect("a missing password shows the sign-in banner");
        assert_eq!(
            shown.sentence(),
            "Postio has no password saved for ada1@example.com. Mail on this computer is \
             still available."
        );
        assert_eq!(shown.heading(), "Can't sign in to imap1.example.com");
        assert_eq!(
            shown.action(),
            Some(("Update password\u{2026}", BannerAction::UpdatePassword))
        );
        let refused = banner(&[(account(1), status(AUTH, None))], &accounts).expect("a banner");
        assert!(
            refused
                .sentence()
                .starts_with("The server rejected the password")
        );
    }

    #[test]
    fn each_banner_is_a_heading_and_a_sentence() {
        let sign_in = Banner::SignIn {
            server: "imap.example.com".into(),
            address: "ada@example.com".into(),
            missing: false,
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
            sync_label(&[(account(1), status(state, progress))], synced, &[], &zone).text
        };
        assert_eq!(label(AUTH, None), "Sync failed");
        assert_eq!(label(ConnectionState::Offline, None), "Offline");
        assert_eq!(
            label(ConnectionState::Online, Some((12_408, 18_204))),
            "Syncing 12,408 of 18,204"
        );
        assert_eq!(label(ConnectionState::Online, None), "Synced 16:09");
        let backfill = |done, total| {
            let status = SyncStatus {
                state: ConnectionState::Online,
                backfill: Some((done, total)),
                ..SyncStatus::default()
            };
            sync_label(&[(account(1), status)], synced, &[], &zone).text
        };
        assert_eq!(backfill(12_400, 81_744), "Syncing 12,400 of 81,744");
        assert_eq!(backfill(81_744, 81_744), "Synced 16:09");
        assert_eq!(label(ConnectionState::Connecting, None), "Synced 16:09");
        let never = sync_label(
            &[(account(1), status(ConnectionState::Connecting, None))],
            None,
            &[],
            &zone,
        );
        assert_eq!(never.text, "Connecting");
        let idle = sync_label(
            &[(account(1), status(ConnectionState::Online, None))],
            None,
            &[],
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
                    Some("g a".to_owned()),
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
    fn the_label_names_a_folder_that_cannot_sync() {
        let zone = FixedOffset::east_opt(0).expect("UTC");
        let synced = Utc.with_ymd_and_hms(2026, 9, 26, 16, 9, 0).single();
        let folders = [
            (MailboxId::new(3), "Inbox".to_owned()),
            (MailboxId::new(7), "Sent Messages".to_owned()),
        ];
        let failing = |mailboxes: &[i64], progress| SyncStatus {
            failing: mailboxes
                .iter()
                .map(|mailbox| (MailboxId::new(*mailbox), format!("Refused {mailbox}.")))
                .collect(),
            ..status(ConnectionState::Online, progress)
        };
        let label = |status| sync_label(&[(account(1), status)], synced, &folders, &zone);

        let one = label(failing(&[7], None));
        assert_eq!(one.text, "Sent Messages can't sync");
        assert_eq!(one.icon, "dialog-warning-symbolic");
        assert_eq!(one.detail.as_deref(), Some("Sent Messages: Refused 7."));

        let two = label(failing(&[3, 7], None));
        assert_eq!(two.text, "2 folders can't sync");
        assert_eq!(
            two.detail.as_deref(),
            Some("Inbox: Refused 3.\nSent Messages: Refused 7.")
        );

        let unnamed = label(failing(&[9], None));
        assert_eq!(unnamed.text, "A folder can't sync");
        assert_eq!(unnamed.detail.as_deref(), Some("Refused 9."));

        assert_eq!(
            label(failing(&[7], Some((4, 9)))).text,
            "Syncing 4 of 9",
            "progress outranks a folder's mark"
        );
        let offline = SyncStatus {
            state: ConnectionState::Offline,
            ..failing(&[7], None)
        };
        assert_eq!(label(offline).text, "Offline");
        let fine = label(status(ConnectionState::Online, None));
        assert_eq!(fine.text, "Synced 16:09");
        assert_eq!(fine.detail, None);
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
            &[],
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
                (
                    Some("\u{21b5}".to_owned()),
                    "open".to_owned(),
                    CommandId::OpenMessage
                ),
                (
                    Some("F8".to_owned()),
                    "read over the list".to_owned(),
                    CommandId::ToggleReadingPane
                ),
            ]
        );
        assert_eq!(said.next_digest, None);
    }

    #[test]
    fn every_empty_place_says_why_and_offers_the_inbox() {
        let places = [
            (EmptyPlace::Snoozed, "Nothing snoozed"),
            (EmptyPlace::Flagged, "Nothing flagged"),
            (EmptyPlace::Drafts, "No drafts"),
            (EmptyPlace::Outbox, "Nothing waiting to send"),
            (EmptyPlace::Folder("Travel".into()), "Travel is empty"),
        ];
        for (place, heading) in places {
            let said = empty_place(&place, Keymap::defaults());
            assert_eq!(said.heading, heading);
            assert!(
                said.detail
                    .as_deref()
                    .is_some_and(|detail| !detail.is_empty()),
                "{place:?} says why it is empty"
            );
            assert!(
                said.shortcuts
                    .iter()
                    .any(|(_, _, command)| *command == CommandId::GoToInbox),
                "{place:?} is not a dead end"
            );
            assert_eq!(said.next_digest, None);
        }
    }

    #[test]
    fn an_empty_snoozed_place_names_the_key_that_snoozes() {
        let said = empty_place(&EmptyPlace::Snoozed, Keymap::defaults());
        let snooze = crate::hints::key(Keymap::defaults(), CommandId::Snooze)
            .expect("snooze is bound by default");
        assert!(
            said.detail.as_deref().unwrap().contains(&snooze),
            "{said:?} names {snooze}"
        );
    }
}
