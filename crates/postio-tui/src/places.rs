//! The places a person can go: folders, views and saved searches.
//!
//! What they are, in what order, and what each is called is
//! `postio_ui::places`'. What this holds is what the app reads of them: the
//! accounts and folders, the view counts, the pinned saved searches and
//! which of Focus's features are in use.

use postio_model::mailbox::Mailbox;
use postio_model::{Account, AccountId, ListScope};
use postio_ui::sidebar::{ViewCounts, display_name};
use postio_ui::terminal::SafeText;

/// Everything the places are built from.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Places {
    /// Every account, in the settings' order.
    pub accounts: Vec<Account>,
    /// Every account's folders.
    pub folders: Vec<Mailbox>,
    /// Each account's view counts.
    pub counts: Vec<(AccountId, ViewCounts)>,
    /// The saved searches pinned in `config.toml`.
    pub saved: Vec<Saved>,
    /// Which of Focus's features `config.toml` has in use.
    pub features: Features,
}

/// Which of Focus's features are in use, for the strip's counts: a count
/// shows only while its feature is (C10).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Features {
    /// Whether filtering is on, so "N filtered today" means something.
    pub filtering: bool,
    /// How many digest rules there are.
    pub digest_rules: usize,
    /// The digest rules, for when the next digest comes.
    pub digests: Rules,
    /// For each digest rule that is a person's, the rule's name and the
    /// person's: the rules list names whose it is.
    pub rule_owners: Vec<(String, String)>,
    /// Where `[focus] reading` opens a message.
    pub reading: postio_config::Reading,
    /// What the notice after `M` offers to do with the list's mail (#1952).
    pub after_unsubscribe: postio_config::AfterUnsubscribe,
    /// Whether `[focus.vault]` is configured, so a to-do offers Task.
    pub capture: bool,
    /// Whether `[focus.model]` has `like_this` on, so the rule dialog offers
    /// "Digest mail like this".
    pub like_this: bool,
}

/// The digest rules as the file has them. Equal when they are written the
/// same: no rule holds a number that is not equal to itself.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Rules(pub Vec<postio_config::DigestRule>);

impl Eq for Rules {}

impl Features {
    /// The person whose digest the rule called `name` is, if it is a
    /// person's and not the file's.
    pub fn rule_owner(&self, name: &str) -> Option<&str> {
        self.rule_owners
            .iter()
            .find(|(rule, _)| rule.trim() == name.trim())
            .map(|(_, owner)| owner.as_str())
    }

    /// What the empty inbox is told of `[focus]`.
    pub fn focus(&self) -> postio_config::FocusConfig {
        postio_config::FocusConfig {
            filtering: self.filtering,
            digests: self.digests.0.clone(),
            reading: self.reading,
            ..Default::default()
        }
    }
}

/// What the bar and the folders box read of every enabled account beyond the
/// folders: labels with their counts, correspondents, and what waits in each
/// Outbox.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PlaceDetails {
    /// Every account's labels.
    pub labels: Vec<postio_model::Label>,
    /// How many conversations carry each label.
    pub label_counts: Vec<(postio_model::LabelId, u32)>,
    /// Who a typed name can mean.
    pub correspondents: Vec<postio_model::Contact>,
    /// How many drafts wait in each account's Outbox.
    pub outbox: Vec<(AccountId, u32)>,
}

/// A saved search from `config.toml`'s `[saved_searches]`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Saved {
    /// Its key in `[saved_searches]`, which a rename leaves alone.
    pub key: String,
    /// What it is called.
    pub name: String,
    /// The query it runs.
    pub query: String,
}

/// What the strip calls the place `scope` is: `Inbox`, a folder's display
/// name, `Flagged`.
pub fn name_of(places: &Places, scope: ListScope) -> SafeText {
    let name = match scope {
        ListScope::Mailbox(id) => places
            .folders
            .iter()
            .find(|folder| folder.id == id)
            .map(|folder| display_name(folder, &places.folders))
            .unwrap_or_default(),
        ListScope::Flagged(_) => "Flagged".to_owned(),
        ListScope::Snoozed(_) => "Snoozed".to_owned(),
        ListScope::Outbox(_) => "Outbox".to_owned(),
        ListScope::Account(account) => places
            .accounts
            .iter()
            .find(|candidate| candidate.id == account)
            .map(|account| account.address.address.clone())
            .unwrap_or_default(),
        ListScope::Unified | ListScope::Focus(_) => "Inbox".to_owned(),
        ListScope::Thread(_) => "Conversation".to_owned(),
    };
    SafeText::new(&name)
}
