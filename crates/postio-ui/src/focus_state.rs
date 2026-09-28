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
use postio_core::{ConnectionState, FailureReason};
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
}
