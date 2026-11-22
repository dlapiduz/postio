//! A contact is a person, not an address: a person owns addresses, an address belongs to at most
//! one person.
//! Mail statistics stay with the address; name, organisation and note belong to the person.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::address::EmailAddress;
use crate::ids::{AddressId, ContactGroupId, ContactId};

/// Provenance: how the person first appeared. Changes only by promotion `mail` -> `user`, in place,
/// never by inserting a second person.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ContactSource {
    #[default]
    Mail,
    User,
    Import,
}

impl ContactSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Mail => "mail",
            Self::User => "user",
            Self::Import => "import",
        }
    }

    /// `None` for an unknown name: a corrupt row must not be guessed at.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "mail" => Some(Self::Mail),
            "user" => Some(Self::User),
            "import" => Some(Self::Import),
            _ => None,
        }
    }
}

/// One mechanism for deletion, restore and exact undo. A `Deleted` person keeps its addresses so
/// new mail
/// does not bring them back; a `Merged` person keeps its fields and memberships so the join can be
/// undone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ContactState {
    #[default]
    Live,
    Deleted,
    Merged,
}

impl ContactState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Live => "live",
            Self::Deleted => "deleted",
            Self::Merged => "merged",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "live" => Some(Self::Live),
            "deleted" => Some(Self::Deleted),
            "merged" => Some(Self::Merged),
            _ => None,
        }
    }
}

/// Detected from the mail's own headers unless the user has said otherwise.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum ContactKind {
    #[default]
    Person,
    Company,
    List,
    Newsletter,
    Automated,
}

impl ContactKind {
    pub const ALL: [Self; 5] = [
        Self::Person,
        Self::Company,
        Self::List,
        Self::Newsletter,
        Self::Automated,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Person => "person",
            Self::Company => "company",
            Self::List => "list",
            Self::Newsletter => "newsletter",
            Self::Automated => "automated",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.as_str() == name)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum KindSource {
    /// The detector, which a newer detector may overrule.
    #[default]
    Detected,
    User,
}

impl KindSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Detected => "detected",
            Self::User => "user",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "detected" => Some(Self::Detected),
            "user" => Some(Self::User),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum ShowAs {
    #[default]
    Full,
    First,
    LastFirst,
    Custom,
}

impl ShowAs {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::First => "first",
            Self::LastFirst => "last_first",
            Self::Custom => "custom",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "full" => Some(Self::Full),
            "first" => Some(Self::First),
            "last_first" => Some(Self::LastFirst),
            "custom" => Some(Self::Custom),
            _ => None,
        }
    }
}

/// Counts over the newest messages a sender sent: the evidence for detecting a `ContactKind`.
/// Zero means no evidence, never "no".
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct KindSignals {
    pub address: String,
    pub name: Option<String>,
    pub messages: u32,
    pub with_list_id: u32,
    pub with_list_precedence: u32,
    pub with_unsubscribe: u32,
    pub with_bulk: u32,
    pub with_junk: u32,
    pub with_auto_generated: u32,
    pub with_auto_replied: u32,
    /// A reply is evidence of a human on the other end.
    pub written_to: u32,
    pub people_at_domain: u32,
}

/// `year * 12 + month - 1`, the key of `address_months`.
pub fn month_key(at: DateTime<Utc>) -> i64 {
    use chrono::Datelike as _;
    i64::from(at.year()) * 12 + i64::from(at.month0())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecentMessage {
    pub message: crate::MessageId,
    pub at: DateTime<Utc>,
    pub outgoing: bool,
}

/// Summed over a person's addresses from maintained tables, so cost follows address count, never
/// mail volume.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ContactStats {
    pub messages: u32,
    pub from_you: u32,
    pub opened: u32,
    pub last: Option<DateTime<Utc>>,
    /// `(month key, messages)` for the months asked, oldest first, with no
    /// entry for a month with no mail.
    pub months: Vec<(i64, u32)>,
    pub recent: Vec<RecentMessage>,
}

impl ContactStats {
    /// How long they typically take to answer the user: the median, over
    /// the ring, of the time from a message the user sent to the first
    /// message back. `None` with fewer than [`MIN_REPLIES`] answered
    /// messages, and a gap longer than [`MAX_REPLY_GAP`] is a conversation
    /// picked up again, not an answer.
    pub fn reply_median(&self) -> Option<chrono::Duration> {
        let mut ring: Vec<&RecentMessage> = self.recent.iter().collect();
        ring.sort_by_key(|message| message.at);
        let mut gaps: Vec<chrono::Duration> = Vec::new();
        let mut asked: Option<DateTime<Utc>> = None;
        for message in ring {
            if message.outgoing {
                asked = Some(message.at);
            } else if let Some(at) = asked.take() {
                let gap = message.at - at;
                if gap >= chrono::Duration::zero() && gap <= MAX_REPLY_GAP {
                    gaps.push(gap);
                }
            }
        }
        if gaps.len() < MIN_REPLIES {
            return None;
        }
        gaps.sort();
        Some(gaps[gaps.len() / 2])
    }
}

pub const MIN_REPLIES: usize = 3;

pub const MAX_REPLY_GAP: chrono::Duration = chrono::Duration::days(30);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OpenSide {
    WaitingOnYou,
    WaitingOnThem,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenItem {
    pub side: OpenSide,
    pub text: String,
    pub message: crate::MessageId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContactGlance {
    pub person: Contact,
    pub stats: ContactStats,
    pub open: Vec<OpenItem>,
    /// `None` means the default rules apply.
    pub arrival: Option<crate::ContactArrival>,
    pub default_arrival: crate::ContactArrival,
    /// For a sender not yet saved; `None` for someone saved, ignored or not a person.
    pub introduction: Option<crate::contact_form::Introduction>,
}

pub const GLANCE_OPEN: u32 = 2;

pub const RECENT: usize = 50;

/// Counts are summed across accounts; the store keeps them per account as evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContactAddress {
    pub id: AddressId,
    pub address: EmailAddress,
    pub times_seen: u32,
    pub last_seen_at: Option<DateTime<Utc>>,
    pub written: u32,
}

impl ContactAddress {
    pub fn new(id: AddressId, address: EmailAddress) -> Self {
        Self {
            id,
            address,
            times_seen: 0,
            last_seen_at: None,
            written: 0,
        }
    }

    pub fn record_seen(&mut self, at: DateTime<Utc>) {
        self.times_seen = self.times_seen.saturating_add(1);
        if self.last_seen_at.is_none_or(|last| at > last) {
            self.last_seen_at = Some(at);
        }
    }
}

/// Counts here are aggregates over the person's addresses, kept so the list can order and filter
/// without adding them up.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Contact {
    pub id: ContactId,
    /// Never written by sync.
    pub name: Option<String>,
    pub organization: Option<String>,
    pub note: Option<String>,
    pub source: ContactSource,
    pub state: ContactState,
    /// One of `addresses`; a stale id falls back to the first.
    pub preferred: AddressId,
    pub addresses: Vec<ContactAddress>,
    pub seen_name: Option<String>,
    pub times_seen: u32,
    pub last_seen_at: Option<DateTime<Utc>>,
    /// More than zero puts the person in the Contacts list's default view.
    pub written: u32,
    pub kind: ContactKind,
    pub kind_source: KindSource,
    /// `None` for someone known only from mail, however often they write.
    pub saved_at: Option<DateTime<Utc>>,
    pub given_name: Option<String>,
    pub family_name: Option<String>,
    pub show_as: ShowAs,
    pub title: Option<String>,
}

impl Contact {
    pub fn new(address: ContactAddress) -> Self {
        Self {
            id: ContactId::UNASSIGNED,
            name: None,
            organization: None,
            note: None,
            source: ContactSource::default(),
            state: ContactState::default(),
            preferred: address.id,
            seen_name: address.address.name.clone(),
            times_seen: address.times_seen,
            last_seen_at: address.last_seen_at,
            written: address.written,
            kind: ContactKind::default(),
            kind_source: KindSource::default(),
            saved_at: None,
            given_name: None,
            family_name: None,
            show_as: ShowAs::default(),
            title: None,
            addresses: vec![address],
        }
    }

    pub fn preferred_address(&self) -> Option<&ContactAddress> {
        self.addresses
            .iter()
            .find(|a| a.id == self.preferred)
            .or_else(|| self.addresses.first())
    }

    /// The user-set name, else the display name last seen, else the
    /// preferred address.
    pub fn display_name(&self) -> &str {
        fn filled(value: Option<&str>) -> Option<&str> {
            value.filter(|v| !v.trim().is_empty())
        }
        filled(self.name.as_deref())
            .or_else(|| filled(self.seen_name.as_deref()))
            .or_else(|| self.preferred_address().map(|a| a.address.address.as_str()))
            .unwrap_or("")
    }

    pub fn is_saved(&self) -> bool {
        self.saved_at.is_some()
    }

    /// Falls back rather than inventing a missing part: to the family name, then `display_name`.
    pub fn shown_as(&self) -> std::borrow::Cow<'_, str> {
        use std::borrow::Cow;
        fn filled(value: &Option<String>) -> Option<&str> {
            value.as_deref().map(str::trim).filter(|v| !v.is_empty())
        }
        let (given, family) = (filled(&self.given_name), filled(&self.family_name));
        match (self.show_as, given, family) {
            (ShowAs::First, Some(given), _) => Cow::Borrowed(given),
            (ShowAs::LastFirst, Some(given), Some(family)) => {
                Cow::Owned(format!("{family}, {given}"))
            }
            (ShowAs::LastFirst, None, Some(family)) => Cow::Borrowed(family),
            _ => Cow::Borrowed(self.display_name()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum ContactView {
    /// A sender the user only received mail from is not here, though it still completes in the
    /// composer.
    #[default]
    Written,
    Everyone,
    Deleted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContactListRow {
    pub id: ContactId,
    pub name: String,
    pub preferred: Option<String>,
    pub address_count: u32,
    pub last_seen_at: Option<DateTime<Utc>>,
    pub source: ContactSource,
    pub state: ContactState,
    pub sort_key: String,
    pub kind: ContactKind,
    pub messages: u32,
    pub saved: bool,
    pub arrival: Option<crate::ContactArrival>,
    pub notify: bool,
    pub html_original: bool,
    pub digest_cadence: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum ContactTab {
    #[default]
    All,
    People,
    Companies,
    Lists,
    Automated,
}

impl ContactTab {
    pub const ALL: [Self; 5] = [
        Self::All,
        Self::People,
        Self::Companies,
        Self::Lists,
        Self::Automated,
    ];

    pub fn kinds(self) -> &'static [ContactKind] {
        match self {
            Self::All => &ContactKind::ALL,
            Self::People => &[ContactKind::Person],
            Self::Companies => &[ContactKind::Company],
            Self::Lists => &[ContactKind::List, ContactKind::Newsletter],
            Self::Automated => &[ContactKind::Automated],
        }
    }

    pub fn next(self) -> Self {
        Self::ALL[(Self::ALL.iter().position(|tab| *tab == self).unwrap_or(0) + 1).min(4)]
    }

    pub fn previous(self) -> Self {
        Self::ALL[Self::ALL
            .iter()
            .position(|tab| *tab == self)
            .unwrap_or(0)
            .saturating_sub(1)]
    }
}

/// Each count is capped at `REVIEW_CAP`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ReviewSummary {
    pub duplicates: u32,
    pub bounced: u32,
    pub frequent_unsaved: u32,
}

pub const REVIEW_CAP: u32 = 99;

impl ReviewSummary {
    pub fn is_empty(&self) -> bool {
        self.duplicates == 0 && self.bounced == 0 && self.frequent_unsaved == 0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileChip {
    pub name: String,
    pub mime: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueRead {
    pub message: crate::MessageId,
    pub subject: String,
    pub at: DateTime<Utc>,
    pub opened: bool,
}

pub const CARD_RECENT: u32 = 3;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContactCardRead {
    pub person: Contact,
    pub groups: Vec<String>,
    pub stats: ContactStats,
    pub open: Vec<OpenItem>,
    pub settings: crate::ContactSettings,
    pub default_arrival: crate::ContactArrival,
    pub files: Vec<FileChip>,
    pub issues: Vec<IssueRead>,
    pub address_notes: Vec<crate::contact_form::AddressNote>,
}

/// `before` is the sort key of the row before the first, which says whether the first starts a new
/// letter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContactsPageRead {
    pub rows: Vec<ContactListRow>,
    /// `None` for the first row of the list.
    pub before: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ContactsOverview {
    pub total: u32,
    pub saved: u32,
    pub tabs: Vec<(ContactTab, u32)>,
    pub review: ReviewSummary,
    /// `(made, of)` while addresses with mail still wait for a person.
    pub building: Option<(u32, u32)>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContactDetail {
    pub person: Contact,
    pub groups: Vec<String>,
    pub messages: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SuggestionReason {
    SameName,
    Replied { replies: u32 },
}

/// Offered, never acted on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JoinSuggestion {
    pub people: [Contact; 2],
    pub reason: SuggestionReason,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MergeGroup {
    /// Two or more records, the best one to keep first.
    pub people: Vec<Contact>,
    pub evidence: Vec<String>,
}

/// Addresses change one at a time, with their own verbs, because each is a claim about who owns
/// what.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersonEdit {
    /// The name the user gave them; `None` or blank leaves them unnamed.
    pub name: Option<String>,
    pub organization: Option<String>,
    pub note: Option<String>,
}

/// Precise enough to put the move back.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AddressMove {
    pub previous: Option<ContactId>,
    pub emptied: Option<ContactState>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersonFields {
    pub name: Option<String>,
    pub organization: Option<String>,
    pub note: Option<String>,
    pub source: ContactSource,
    pub preferred: AddressId,
}

/// Precise enough to undo a join exactly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JoinReceipt {
    pub into: ContactId,
    pub restore: Vec<(ContactId, Vec<AddressId>)>,
    pub prior: PersonFields,
    /// Filled from an absorbed person when the survivor had none.
    #[serde(default)]
    pub prior_title: Option<String>,
    pub added_memberships: Vec<ContactGroupId>,
    /// What the survivor had decided before the join; the join fills its gaps from the absorbed
    /// people.
    #[serde(default)]
    pub prior_settings: Box<crate::ContactSettings>,
}

#[cfg(test)]
mod tests {

    #[test]
    fn the_tabs_walk_and_stop_at_the_ends_and_each_holds_its_kinds() {
        assert_eq!(ContactTab::All.previous(), ContactTab::All);
        assert_eq!(ContactTab::All.next(), ContactTab::People);
        assert_eq!(ContactTab::Automated.next(), ContactTab::Automated);
        for tab in ContactTab::ALL {
            assert!(!tab.kinds().is_empty());
        }
        assert_eq!(ContactTab::Lists.kinds().len(), 2);
        let none = ReviewSummary::default();
        assert!(none.is_empty());
        assert!(!ReviewSummary { bounced: 1, ..none }.is_empty());
    }
    use super::*;

    fn address(id: i64, name: Option<&str>, addr: &str) -> ContactAddress {
        ContactAddress::new(AddressId::new(id), EmailAddress::new(name, addr))
    }

    #[test]
    fn kind_source_and_show_as_round_trip_and_refuse_the_unknown() {
        for kind in ContactKind::ALL {
            assert_eq!(ContactKind::from_name(kind.as_str()), Some(kind));
        }
        assert_eq!(ContactKind::from_name("bot"), None);
        for source in [KindSource::Detected, KindSource::User] {
            assert_eq!(KindSource::from_name(source.as_str()), Some(source));
        }
        assert_eq!(KindSource::from_name("guessed"), None);
        for style in [
            ShowAs::Full,
            ShowAs::First,
            ShowAs::LastFirst,
            ShowAs::Custom,
        ] {
            assert_eq!(ShowAs::from_name(style.as_str()), Some(style));
        }
        assert_eq!(ShowAs::from_name("shouting"), None);
    }

    #[test]
    fn a_new_contact_is_a_detected_person_who_is_not_saved() {
        let contact = Contact::new(address(1, None, "ada@example.com"));
        assert_eq!(contact.kind, ContactKind::Person);
        assert_eq!(contact.kind_source, KindSource::Detected);
        assert_eq!(contact.show_as, ShowAs::Full);
        assert!(contact.saved_at.is_none());
        assert!(!contact.is_saved());
    }

    #[test]
    fn show_as_styles_the_name_and_never_invents_one() {
        let mut contact = Contact::new(address(1, None, "ada@example.com"));
        contact.name = Some("Ada King Lovelace".into());
        contact.given_name = Some("Ada".into());
        contact.family_name = Some("Lovelace".into());

        assert_eq!(contact.shown_as(), "Ada King Lovelace");
        contact.show_as = ShowAs::First;
        assert_eq!(contact.shown_as(), "Ada");
        contact.show_as = ShowAs::LastFirst;
        assert_eq!(contact.shown_as(), "Lovelace, Ada");
        contact.show_as = ShowAs::Custom;
        assert_eq!(contact.shown_as(), "Ada King Lovelace");

        contact.given_name = None;
        contact.show_as = ShowAs::First;
        assert_eq!(contact.shown_as(), "Ada King Lovelace");
        contact.show_as = ShowAs::LastFirst;
        assert_eq!(contact.shown_as(), "Lovelace");
        contact.name = None;
        contact.family_name = None;
        assert_eq!(contact.shown_as(), "ada@example.com");
    }

    #[test]
    fn contact_source_round_trips_through_its_stored_identifier() {
        for source in [
            ContactSource::Mail,
            ContactSource::User,
            ContactSource::Import,
        ] {
            assert_eq!(ContactSource::from_name(source.as_str()), Some(source));
        }
        assert_eq!(ContactSource::from_name("scraped"), None);
    }

    #[test]
    fn contact_state_round_trips_and_refuses_what_it_does_not_know() {
        for state in [
            ContactState::Live,
            ContactState::Deleted,
            ContactState::Merged,
        ] {
            assert_eq!(ContactState::from_name(state.as_str()), Some(state));
        }
        assert_eq!(ContactState::from_name("archived"), None);
        assert_eq!(ContactState::from_name(""), None);
    }

    #[test]
    fn a_new_person_is_live_from_mail_and_owns_the_one_address_it_was_built_from() {
        let contact = Contact::new(address(4, Some("Ada"), "ada@example.com"));
        assert_eq!(contact.source, ContactSource::Mail);
        assert_eq!(contact.state, ContactState::Live);
        assert_eq!(contact.addresses.len(), 1);
        assert_eq!(contact.preferred, AddressId::new(4));
        assert_eq!(
            contact
                .preferred_address()
                .map(|a| a.address.address.as_str()),
            Some("ada@example.com")
        );
    }

    #[test]
    fn the_displayed_name_is_the_users_then_the_mails_then_the_preferred_address() {
        let mut contact = Contact::new(address(1, None, "ada@work.example"));
        contact.addresses.push(address(2, None, "ada@home.example"));
        contact.preferred = AddressId::new(2);
        assert_eq!(contact.display_name(), "ada@home.example");

        contact.seen_name = Some("A. Lovelace".into());
        assert_eq!(contact.display_name(), "A. Lovelace");

        contact.name = Some("Ada Lovelace".into());
        assert_eq!(contact.display_name(), "Ada Lovelace");
    }

    #[test]
    fn a_blank_name_is_no_name_at_either_level() {
        let mut contact = Contact::new(address(1, None, "ada@example.com"));
        contact.name = Some("   ".into());
        contact.seen_name = Some("\t".into());
        assert_eq!(contact.display_name(), "ada@example.com");
    }

    #[test]
    fn a_preferred_id_that_is_not_one_of_the_persons_own_falls_back_to_the_first() {
        let mut contact = Contact::new(address(1, None, "ada@example.com"));
        contact.preferred = AddressId::new(99);
        assert_eq!(contact.display_name(), "ada@example.com");
    }

    #[test]
    fn an_address_counts_its_sightings_and_keeps_the_latest_date() {
        let mut addr = address(1, Some("Ada"), "ada@example.com");
        let later = DateTime::from_timestamp(20, 0).unwrap();
        let earlier = DateTime::from_timestamp(10, 0).unwrap();
        addr.record_seen(later);
        addr.record_seen(earlier);
        assert_eq!(addr.times_seen, 2);
        assert_eq!(addr.last_seen_at, Some(later));
    }
}

#[cfg(test)]
mod reply_time_tests {
    use chrono::TimeZone;

    use super::*;

    fn ring(events: &[(i64, bool)]) -> ContactStats {
        let base = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        ContactStats {
            recent: events
                .iter()
                .enumerate()
                .map(|(n, (minutes, outgoing))| RecentMessage {
                    message: crate::MessageId::new(n as i64 + 1),
                    at: base + chrono::Duration::minutes(*minutes),
                    outgoing: *outgoing,
                })
                .rev()
                .collect(),
            ..ContactStats::default()
        }
    }

    #[test]
    fn the_median_of_the_gaps_from_what_you_sent_to_what_came_back() {
        // Sent at 0, answered at 60; sent at 100, answered at 280; sent at
        // 300, answered at 420: gaps 60, 180 and 120 minutes.
        let stats = ring(&[
            (0, true),
            (60, false),
            (100, true),
            (280, false),
            (300, true),
            (420, false),
        ]);
        assert_eq!(stats.reply_median(), Some(chrono::Duration::minutes(120)));
    }

    #[test]
    fn too_few_answers_say_nothing_and_a_revival_is_not_an_answer() {
        let few = ring(&[(0, true), (60, false), (100, true), (160, false)]);
        assert_eq!(few.reply_median(), None);
        let day = 60 * 24;
        let stale = ring(&[
            (0, true),
            (60, false),
            (100, true),
            (160, false),
            (200, true),
            (200 + 40 * day, false),
        ]);
        assert_eq!(stale.reply_median(), None, "two answers and a revival");
    }

    #[test]
    fn mail_they_sent_unprompted_is_no_answer() {
        let stats = ring(&[(0, false), (10, false), (20, true), (80, false)]);
        assert_eq!(stats.reply_median(), None);
    }
}
