//! What Focus says about its own state, at the boundary
//! (specs/009-focus-macos T096, for the Mac's T099).
//!
//! The banner under the header strip, the toolbar's sync label and the page
//! an empty list shows are the controller's (`postio_focus`, ADR 0045),
//! worded by `postio_ui::focus_state`. Each crosses as an event only when it
//! changes: `FocusBanner`, `FocusSyncLabel` and `FocusEmpty`. Swift draws
//! what it is told and runs a button's or a shortcut's command as a menu item
//! would.

/// The banner under the header strip: words, drawable as they are.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct BannerFfi {
    /// The heading, in bold: "You're offline", "Can't sign in to …".
    pub heading: String,
    /// The sentence after it.
    pub sentence: String,
    /// Its button, when it has one.
    pub button: Option<BannerButtonFfi>,
    /// How far a first sync has come, for its progress bar.
    pub progress: Option<BannerProgressFfi>,
    /// Drawn as an error (`systemRed` at a low opacity): a refused
    /// password, or an account that cannot sync.
    pub error: bool,
    /// The account a refused password is for: whose credential "Update
    /// password…" asks for.
    pub account: Option<i64>,
}

/// A banner's button.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct BannerButtonFfi {
    /// "Update password…", "Retry now".
    pub label: String,
    /// The registry command a click runs: `update_credential`, `refresh`.
    pub command: String,
    /// Its key, as the keymap in force spells it.
    pub key: Option<String>,
}

/// How far a first sync has come.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct BannerProgressFfi {
    /// Messages fetched so far.
    pub done: u32,
    /// Messages the pass expects.
    pub total: u32,
}

/// What the sync label's mark is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum SyncMarkFfi {
    /// Mail synced: "Synced 09:30".
    Synced,
    /// On its way: "Syncing 1,200 of 8,400", "Connecting", "Not synced yet".
    Syncing,
    /// No network.
    Offline,
    /// An account's sync is failing.
    Failed,
}

/// The page an empty list shows in its place (screen 16).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct EmptyPageFfi {
    /// The bold line: "Inbox is empty", "Syncing your inbox…", "Nothing
    /// snoozed".
    pub heading: String,
    /// The line under it, when there is one.
    pub detail: Option<String>,
    /// "Next digest: Weekly · Newsletters, Saturday 16:00", when a digest
    /// rule names a time.
    pub next_digest: Option<String>,
    /// The shortcuts, only to what exists.
    pub shortcuts: Vec<EmptyShortcutFfi>,
}

/// One of an empty page's shortcuts: a key, its words, and what a click
/// runs.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct EmptyShortcutFfi {
    /// The key, as the keymap in force spells it.
    pub key: Option<String>,
    /// "compose", "archive", "186 filtered today".
    pub words: String,
    /// The registry command.
    pub command: String,
}

impl From<postio_focus::BannerView> for BannerFfi {
    fn from(banner: postio_focus::BannerView) -> Self {
        BannerFfi {
            heading: banner.heading,
            sentence: banner.sentence,
            button: banner.button.map(|button| BannerButtonFfi {
                label: button.label,
                command: button.command.to_string(),
                key: button.key,
            }),
            progress: banner
                .progress
                .map(|(done, total)| BannerProgressFfi { done, total }),
            error: banner.error,
            account: banner.account.map(|account| account.get()),
        }
    }
}

impl SyncMarkFfi {
    /// The mark for a label `postio_ui::focus_state::sync_label` made, by
    /// the icon it chose: one reading of its four, pinned by a test over
    /// every branch.
    pub(crate) fn of(label: &postio_ui::focus_state::SyncLabel) -> Self {
        match label.icon {
            "emblem-ok-symbolic" => SyncMarkFfi::Synced,
            "network-offline-symbolic" => SyncMarkFfi::Offline,
            "dialog-warning-symbolic" => SyncMarkFfi::Failed,
            _ => SyncMarkFfi::Syncing,
        }
    }
}

impl From<postio_ui::focus_state::EmptyInbox> for EmptyPageFfi {
    fn from(page: postio_ui::focus_state::EmptyInbox) -> Self {
        EmptyPageFfi {
            heading: page.heading,
            detail: page.detail,
            next_digest: page.next_digest,
            shortcuts: page
                .shortcuts
                .into_iter()
                .map(|(key, words, command)| EmptyShortcutFfi {
                    key,
                    words,
                    command: command.to_string(),
                })
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use postio_core::{ConnectionState, FailureReason};
    use postio_model::AccountId;
    use postio_ui::focus_state::sync_label_here;
    use postio_ui::status::SyncStatus;

    use super::*;

    fn standing(state: ConnectionState, progress: Option<(u32, u32)>) -> SyncMarkFfi {
        let status = SyncStatus {
            state,
            progress,
            ..SyncStatus::default()
        };
        SyncMarkFfi::of(&sync_label_here(&[(AccountId::new(1), status)], None))
    }

    #[test]
    fn every_label_has_its_own_mark() {
        assert_eq!(
            standing(
                ConnectionState::Failing {
                    reason: FailureReason::Auth
                },
                None
            ),
            SyncMarkFfi::Failed
        );
        assert_eq!(
            standing(ConnectionState::Offline, None),
            SyncMarkFfi::Offline
        );
        assert_eq!(
            standing(ConnectionState::Online, Some((1, 4))),
            SyncMarkFfi::Syncing
        );
        assert_eq!(
            standing(ConnectionState::Connecting, None),
            SyncMarkFfi::Syncing
        );
        let synced = sync_label_here(
            &[(
                AccountId::new(1),
                SyncStatus {
                    state: ConnectionState::Online,
                    ..SyncStatus::default()
                },
            )],
            Some(chrono::Utc::now()),
        );
        assert_eq!(SyncMarkFfi::of(&synced), SyncMarkFfi::Synced);
    }
}
