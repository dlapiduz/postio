//! The places a person can go: folders, views and saved searches. What the app reads of them is
//! here; their order and names are `postio_ui::places`'.

use postio_model::mailbox::Mailbox;
use postio_model::{Account, AccountId, ListScope};
use postio_ui::sidebar::{ViewCounts, display_name};
use postio_ui::terminal::SafeText;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Places {
    pub accounts: Vec<Account>,
    pub folders: Vec<Mailbox>,
    pub counts: Vec<(AccountId, ViewCounts)>,
    pub saved: Vec<Saved>,
    pub features: Features,
}

/// A strip count shows only while its feature is in use.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Features {
    pub filtering: bool,
    pub digest_rules: usize,
    pub digests: Rules,
    pub rule_owners: Vec<(String, String)>,
    pub reading: postio_config::Reading,
    pub after_unsubscribe: postio_config::AfterUnsubscribe,
    pub capture: bool,
    pub like_this: bool,
    pub section: Section,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Section(pub Box<postio_config::FocusConfig>);

impl Section {
    pub fn new(focus: postio_config::FocusConfig) -> Section {
        Section(Box::new(focus))
    }
}

impl Eq for Section {}

/// Equal when written the same: no rule holds a number unequal to itself.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Rules(pub Vec<postio_config::DigestRule>);

impl Eq for Rules {}

impl Features {
    pub fn rule_owner(&self, name: &str) -> Option<&str> {
        self.rule_owners
            .iter()
            .find(|(rule, _)| rule.trim() == name.trim())
            .map(|(_, owner)| owner.as_str())
    }

    pub fn focus(&self) -> postio_config::FocusConfig {
        let mut focus = (*self.section.0).clone();
        focus.filtering = self.filtering;
        focus.digests = self.digests.0.clone();
        focus.reading = self.reading;
        focus.after_unsubscribe = self.after_unsubscribe;
        if self.capture && focus.vault.is_none() {
            focus.vault = Some(Default::default());
        }
        focus
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Saved {
    /// Its key in `[saved_searches]`, which a rename leaves alone.
    pub key: String,
    pub name: String,
    pub query: String,
}

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
        ListScope::Thread(_) => "Thread".to_owned(),
    };
    SafeText::new(&name)
}
