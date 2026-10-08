//! What Focus says about its own state: the banner under the header strip,
//! the sync label, and the page an empty list shows (research R2, slice 10;
//! screens 16 to 19).
//!
//! Moved from `postio-gtk`'s window: `hear_sync`, `note_account`,
//! `show_state` and `show_empty_or_list`. The words are
//! `postio_ui::focus_state`'s; the controller keeps where every account
//! stands (`postio_ui::status::Trackers`), who each account is, when mail
//! last synced, and what it last said, so a frontend draws a banner, a label
//! and an empty page only when one of them changes.

use std::collections::BTreeSet;

use chrono::{DateTime, Utc};
use postio_config::FocusConfig;
use postio_core::{CommandId, Event};
use postio_model::{AccountId, FocusScope, ListScope};
use postio_ui::focus_state::{
    self, AccountFacts, Banner, BannerAction, EmptyInbox, EmptyPlace, SyncLabel,
};
use postio_ui::status::Trackers;

use crate::feed::Step;
use crate::{FocusController, Intent, Request};

/// The banner under the header strip, in words, drawable as it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BannerView {
    /// The heading, in bold: "You're offline".
    pub heading: String,
    /// The sentence after it.
    pub sentence: String,
    /// Its button, when it has one.
    pub button: Option<BannerButton>,
    /// How far a first sync has come: messages fetched, and expected.
    pub progress: Option<(u32, u32)>,
    /// Drawn in the error colour: a refused password, an account that
    /// cannot sync.
    pub error: bool,
    /// The account a refused password is for: whose credential the
    /// button's sheet asks for.
    pub account: Option<AccountId>,
}

/// A banner's button.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BannerButton {
    /// "Update password…", "Retry now".
    pub label: String,
    /// What a click runs: `UpdateCredential`, or `Refresh`.
    pub command: CommandId,
    /// The command's key, as the keymap in force spells it.
    pub key: Option<String>,
}

/// The answer to [`Request::Accounts`]: who each enabled account is, and
/// when mail last synced before this run.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AccountsRead {
    /// Each enabled account: where it signs in, as whom, and its name.
    pub facts: Vec<AccountFacts>,
    /// The newest of every enabled account's folders' last completed sync.
    pub last_synced: Option<DateTime<Utc>>,
}

/// What the controller keeps of the app's state.
#[derive(Debug, Default)]
pub(crate) struct States {
    /// Where every account stands, from the sync's own events.
    trackers: Trackers,
    /// The accounts heard about, in the order they were.
    tracked: Vec<AccountId>,
    /// Who each enabled account is.
    facts: Vec<AccountFacts>,
    /// The accounts already asked about.
    asked: BTreeSet<AccountId>,
    /// When mail last synced.
    last_synced: Option<DateTime<Utc>>,
    /// `[focus]`: the digests the empty inbox names, and whether Focus
    /// files mail away.
    config: FocusConfig,
    /// What the header strip names: the folder an empty page is about.
    place: String,
    /// The banner last drawn, once one has been.
    banner: Option<Option<BannerView>>,
    /// The sync label last drawn.
    label: Option<SyncLabel>,
    /// The empty page drawn in the list's place, while one is: the list is
    /// what a frontend draws until told otherwise.
    empty: Option<EmptyInbox>,
}

impl FocusController {
    /// What sync said about an account: the banner and the label follow.
    /// The first word about an account is news even when it changes
    /// nothing in its tracker -- a tracker starts at Offline, so an account
    /// whose first report is Offline would otherwise never be drawn as such
    /// -- and it asks who the account is, once.
    pub(crate) fn hear_sync(&mut self, event: &Event) -> Vec<Step> {
        let mut steps = Vec::new();
        let first = match event {
            Event::ConnectionChanged { account, .. }
            | Event::SyncProgress { account, .. }
            | Event::BackfillProgress { account, .. } => {
                let account = *account;
                if !self.states.facts.iter().any(|facts| facts.id == account)
                    && self.states.asked.insert(account)
                {
                    steps.push(Step::Ask(Request::Accounts));
                }
                self.note_account(account)
            }
            _ => false,
        };
        let changed = self.states.trackers.apply(event, None);
        if !(first || changed) {
            return steps;
        }
        if let Event::SyncProgress { done, total, .. } = event
            && done >= total
        {
            self.states.last_synced = Some(self.now().to_utc());
        }
        steps.extend(self.show_state());
        steps
    }

    /// Follow `account`'s sync from here on; whether it is new.
    fn note_account(&mut self, account: AccountId) -> bool {
        let tracked = &mut self.states.tracked;
        let new = !tracked.contains(&account);
        if new {
            tracked.push(account);
        }
        new
    }

    /// Who the accounts are, and when mail last synced, as read: a banner
    /// waiting on a name can say it now.
    pub(crate) fn accounts_read(&mut self, read: Result<AccountsRead, String>) -> Vec<Step> {
        let read = match read {
            Ok(read) => read,
            Err(error) => {
                tracing::debug!(%error, "Focus could not read who its accounts are");
                return Vec::new();
            }
        };
        self.states.facts = read.facts;
        if self.states.last_synced.is_none() {
            self.states.last_synced = read.last_synced;
        }
        self.show_state()
    }

    /// `[focus]` as it stands now: the empty inbox names its digests and
    /// offers Filtered only while Focus files mail away.
    pub(crate) fn set_config(&mut self, config: FocusConfig) -> Vec<Step> {
        self.states.config = config;
        self.said_again()
    }

    /// What has been said, said again under what changed -- the keys a
    /// button and a shortcut name, `[focus]` -- when it reads differently.
    /// Nothing is said that had not been.
    pub(crate) fn said_again(&mut self) -> Vec<Step> {
        let mut steps = Vec::new();
        if self.states.banner.is_some() {
            steps.extend(self.show_state());
        } else if self.states.empty.is_some() {
            steps.extend(self.show_empty());
        }
        steps
    }

    /// The header strip names `name` now.
    pub(crate) fn note_place(&mut self, name: &str) {
        name.clone_into(&mut self.states.place);
    }

    /// The banner and the sync label for where every account stands, and
    /// the empty page, which follows the same news.
    fn show_state(&mut self) -> Vec<Step> {
        let statuses = self.states.trackers.statuses(&self.states.tracked);
        let banner = focus_state::banner(&statuses, &self.states.facts);
        let account = match banner {
            Some(Banner::SignIn { .. }) => refused(&statuses, &self.states.facts),
            _ => None,
        };
        let banner = banner.map(|banner| self.banner_view(&banner, account));
        let mut steps = Vec::new();
        if self.states.banner.as_ref() != Some(&banner) {
            self.states.banner = Some(banner.clone());
            steps.push(Step::Show(Intent::Banner(banner)));
        }
        let label = focus_state::sync_label(&statuses, self.states.last_synced, &chrono::Local);
        if self.states.label.as_ref() != Some(&label) {
            self.states.label = Some(label.clone());
            steps.push(Step::Show(Intent::SyncLabel(label)));
        }
        steps.extend(self.show_empty());
        steps
    }

    /// `banner` in words, its button naming its key under the keymap in
    /// force.
    fn banner_view(&self, banner: &Banner, account: Option<AccountId>) -> BannerView {
        BannerView {
            heading: banner.heading(),
            sentence: banner.sentence(),
            button: banner.action().map(|(label, action)| {
                let command = banner_command(action);
                BannerButton {
                    label: label.to_owned(),
                    command,
                    // Only a key that runs it from the list the banner is
                    // over: `update_credential`'s `c` is the settings
                    // window's, and over the list `c` composes.
                    key: postio_core::registry::get(command)
                        .available_in(postio_core::Context::List)
                        .then(|| postio_ui::hints::key(self.keymap(), command))
                        .flatten(),
                }
            }),
            progress: match banner {
                Banner::FirstSync { done, total } => Some((*done, *total)),
                _ => None,
            },
            error: banner.is_error(),
            account,
        }
    }

    /// The empty page, or the list, when that has changed (screen 16).
    pub(crate) fn show_empty(&mut self) -> Vec<Step> {
        let empty = self.empty_page();
        if self.states.empty == empty {
            return Vec::new();
        }
        self.states.empty.clone_from(&empty);
        vec![Step::Show(Intent::Empty(empty))]
    }

    /// What the list shows in its place, if anything: a place with nothing
    /// in it says why once its first page has landed (a list still waiting
    /// for one is loading, not empty), and Focus's inbox with no
    /// conversations says whether a pass has found nothing or none has
    /// finished yet. The has-action filter showing nothing is the list's
    /// own business.
    fn empty_page(&self) -> Option<EmptyInbox> {
        let keymap = self.keymap();
        let place = match self.feed.scope() {
            // The window starts on its inbox.
            None | Some(ListScope::Focus(FocusScope::Inbox)) => None,
            Some(ListScope::Focus(FocusScope::Snoozed)) => Some(EmptyPlace::Snoozed),
            Some(ListScope::Focus(FocusScope::Flagged)) => Some(EmptyPlace::Flagged),
            Some(ListScope::Outbox(_)) => Some(EmptyPlace::Outbox),
            Some(ListScope::Mailbox(_)) => Some(EmptyPlace::Folder(self.states.place.clone())),
            Some(_) => return None,
        };
        if let Some(place) = place {
            return (self.feed.has_landed() && self.feed.total() == 0)
                .then(|| focus_state::empty_place(&place, keymap));
        }
        if self.cursor.has_action() {
            return None;
        }
        let counts = self.counts.filter(|counts| counts.conversations == 0)?;
        let statuses = self.states.trackers.statuses(&self.states.tracked);
        let saying = focus_state::inbox_saying(&statuses, self.states.last_synced);
        Some(
            focus_state::empty_inbox(
                &self.states.config,
                counts.filtered_today,
                keymap,
                &self.now(),
            )
            .saying(&saying, keymap, &chrono::Local),
        )
    }

    /// The controller's clock: stopped by [`crate::Input::Clock`], or the
    /// machine's.
    pub(crate) fn now(&self) -> chrono::DateTime<chrono::Local> {
        self.pickers.clock.unwrap_or_else(postio_ui::clock::now)
    }
}

/// The account whose password was refused, or is missing: the one
/// `focus_state::banner` names, the first such that is known.
fn refused(
    statuses: &[(AccountId, postio_ui::status::SyncStatus)],
    facts: &[AccountFacts],
) -> Option<AccountId> {
    use postio_core::{ConnectionState, FailureReason};
    statuses
        .iter()
        .find_map(|(account, status)| match status.state {
            ConnectionState::Failing {
                reason: FailureReason::Auth | FailureReason::NoPassword,
            } if facts.iter().any(|known| known.id == *account) => Some(*account),
            _ => None,
        })
}

/// The command a banner's button runs.
fn banner_command(action: BannerAction) -> CommandId {
    match action {
        BannerAction::Retry => CommandId::Refresh,
        BannerAction::UpdatePassword => CommandId::UpdateCredential,
    }
}
