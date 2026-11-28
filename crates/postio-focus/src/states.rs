//! What Focus says about its own state: the banner under the header strip, the sync label, and
//! the page an empty list shows. The controller keeps where every account stands, who each is,
//! and when mail last synced, so a frontend draws each only when it changes.

use std::collections::BTreeSet;

use chrono::{DateTime, Utc};
use postio_config::FocusConfig;
use postio_core::{CommandId, Event};
use postio_model::{AccountId, FocusScope, ListScope};
use postio_ui::focus_state::{
    self, AccountFacts, Banner, BannerAction, EmptyInbox, EmptyPlace, InboxSaying, SyncLabel,
};
use postio_ui::status::Trackers;

use crate::feed::Step;
use crate::{FocusController, Intent, Request};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BannerView {
    pub heading: String,
    pub sentence: String,
    pub button: Option<BannerButton>,
    pub progress: Option<(u32, u32)>,
    pub error: bool,
    pub account: Option<AccountId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BannerButton {
    pub label: String,
    pub command: CommandId,
    pub key: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AccountsRead {
    pub facts: Vec<AccountFacts>,
    pub last_synced: Option<DateTime<Utc>>,
}

#[derive(Debug, Default)]
pub(crate) struct States {
    trackers: Trackers,
    tracked: Vec<AccountId>,
    facts: Vec<AccountFacts>,
    asked: BTreeSet<AccountId>,
    /// Until they are answered, when mail last synced is not known.
    read: Reading,
    last_synced: Option<DateTime<Utc>>,
    pub(crate) config: FocusConfig,
    place: String,
    banner: Option<Option<BannerView>>,
    label: Option<SyncLabel>,
    empty: Option<EmptyInbox>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum Reading {
    #[default]
    Unasked,
    Asked,
    Read,
}

impl FocusController {
    /// The first word about an account is news even when it changes nothing in its tracker: a
    /// tracker starts at Offline, so an account whose first report is Offline would otherwise never
    /// be drawn as such.
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
                    if self.states.read == Reading::Unasked {
                        self.states.read = Reading::Asked;
                    }
                }
                self.note_account(account)
            }
            _ => false,
        };
        let changed = self.states.trackers.apply(event, None);
        if !(first || changed) {
            return steps;
        }
        // A folder the places have no name for yet is read for, so the label can name it when they
        // land.
        if let Event::MailboxSyncChanged {
            mailbox, failure, ..
        } = event
        {
            steps.push(Step::Show(Intent::PlacesChanged));
            if failure.is_some() && !self.folder_names().iter().any(|(each, _)| each == mailbox) {
                steps.extend(self.bar.places_wanted());
            }
        }
        if let Event::SyncProgress { done, total, .. } = event
            && done >= total
        {
            self.states.last_synced = Some(self.now().to_utc());
        }
        steps.extend(self.show_state());
        steps
    }

    pub(crate) fn failure(&self, mailbox: postio_model::MailboxId) -> Option<&str> {
        self.states.trackers.failure(mailbox)
    }

    fn note_account(&mut self, account: AccountId) -> bool {
        let tracked = &mut self.states.tracked;
        let new = !tracked.contains(&account);
        if new {
            tracked.push(account);
        }
        new
    }

    pub(crate) fn accounts_read(&mut self, read: Result<AccountsRead, String>) -> Vec<Step> {
        self.states.read = Reading::Read;
        let read = match read {
            Ok(read) => read,
            Err(error) => {
                tracing::debug!(%error, "Focus could not read who its accounts are");
                return self.show_empty();
            }
        };
        self.states.facts = read.facts;
        if self.states.last_synced.is_none() {
            self.states.last_synced = read.last_synced;
        }
        self.show_state()
    }

    pub(crate) fn set_config(&mut self, config: FocusConfig) -> Vec<Step> {
        self.states.config = config;
        self.said_again()
    }

    pub(crate) fn said_again(&mut self) -> Vec<Step> {
        let mut steps = Vec::new();
        if self.states.banner.is_some() {
            steps.extend(self.show_state());
        } else if self.states.empty.is_some() {
            steps.extend(self.show_empty());
        }
        steps
    }

    pub fn note_place(&mut self, name: &str) {
        name.clone_into(&mut self.states.place);
    }

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
        steps.extend(self.show_label(&statuses));
        steps.extend(self.show_empty());
        steps
    }

    pub(crate) fn relabel(&mut self) -> Vec<Step> {
        if self.states.label.is_none() {
            return Vec::new();
        }
        let statuses = self.states.trackers.statuses(&self.states.tracked);
        self.show_label(&statuses)
    }

    fn show_label(&mut self, statuses: &[(AccountId, postio_ui::status::SyncStatus)]) -> Vec<Step> {
        let label = focus_state::sync_label(
            statuses,
            self.states.last_synced,
            self.folder_names(),
            &chrono::Local,
        );
        if self.states.label.as_ref() == Some(&label) {
            return Vec::new();
        }
        self.states.label = Some(label.clone());
        vec![Step::Show(Intent::SyncLabel(label))]
    }

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

    /// An empty inbox that would say it is waiting on sync asks when mail last synced first: a
    /// store an earlier run filled has nothing to wait for, and sync may never say so. The page
    /// waits for that answer.
    pub(crate) fn show_empty(&mut self) -> Vec<Step> {
        let (empty, waiting) = match self.empty_page() {
            Page::Waiting => (self.states.empty.clone(), true),
            Page::Shown(page) => (page, false),
        };
        let mut steps = Vec::new();
        if waiting && self.states.read == Reading::Unasked {
            self.states.read = Reading::Asked;
            steps.push(Step::Ask(Request::Accounts));
        }
        if self.states.empty != empty {
            self.states.empty.clone_from(&empty);
            steps.push(Step::Show(Intent::Empty(empty)));
        }
        steps
    }

    /// A list still waiting for its first page is loading, not empty. The has-action filter
    /// showing nothing is the list's own business.
    fn empty_page(&self) -> Page {
        let keymap = self.keymap();
        let place = match self.feed.scope() {
            None | Some(ListScope::Focus(FocusScope::Inbox)) => None,
            Some(ListScope::Focus(FocusScope::Snoozed)) => Some(EmptyPlace::Snoozed),
            Some(ListScope::Focus(FocusScope::Flagged)) => Some(EmptyPlace::Flagged),
            Some(ListScope::Outbox(_)) => Some(EmptyPlace::Outbox),
            Some(ListScope::Mailbox(_)) => Some(EmptyPlace::Folder(self.states.place.clone())),
            Some(_) => return Page::Shown(None),
        };
        if let Some(place) = place {
            return Page::Shown(
                (self.feed.has_landed() && self.feed.total() == 0)
                    .then(|| focus_state::empty_place(&place, keymap)),
            );
        }
        if self.cursor.has_action() {
            return Page::Shown(None);
        }
        let Some(counts) = self.counts.filter(|counts| counts.conversations == 0) else {
            return Page::Shown(None);
        };
        let statuses = self.states.trackers.statuses(&self.states.tracked);
        let saying = focus_state::inbox_saying(&statuses, self.states.last_synced);
        if self.states.read != Reading::Read && !matches!(saying, InboxSaying::Empty { .. }) {
            return Page::Waiting;
        }
        Page::Shown(Some(
            focus_state::empty_inbox(
                &self.states.config,
                counts.filtered_today,
                keymap,
                &self.now(),
            )
            .saying(&saying, keymap, &chrono::Local),
        ))
    }

    pub(crate) fn now(&self) -> chrono::DateTime<chrono::Local> {
        self.pickers.clock.unwrap_or_else(postio_ui::clock::now)
    }
}

enum Page {
    Shown(Option<EmptyInbox>),
    Waiting,
}

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

fn banner_command(action: BannerAction) -> CommandId {
    match action {
        BannerAction::Retry => CommandId::Refresh,
        BannerAction::UpdatePassword => CommandId::UpdateCredential,
    }
}
