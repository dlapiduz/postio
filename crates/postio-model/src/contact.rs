//! Contacts: the people the user corresponds with, and the addresses they own.
//!
//! A contact is a *person*, not an address (specs/005-contacts). A person owns
//! one or more addresses, and an address belongs to at most one person. What
//! the mail tells us — how often and how recently an address was seen — is a
//! fact about the address and stays with it; the name the user gives, the
//! organisation and the note belong to the person. Every address seen in mail
//! starts as a person of its own, and the user joins them.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::address::EmailAddress;
use crate::ids::{AddressId, ContactGroupId, ContactId};

/// How a person first came to exist.
///
/// This is provenance, not a current judgement: it names how the person
/// *first* appeared, and never changes once it does except by the one
/// deliberate promotion `mail` -> `user`. A person known only from mail whom
/// the user edits or joins is promoted in place, never by inserting a second
/// one (specs/005-contacts FR-022).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ContactSource {
    /// Accumulated from a message's headers.
    #[default]
    Mail,
    /// Created or promoted by the user directly.
    User,
    /// Brought in from an imported vCard.
    Import,
}

impl ContactSource {
    /// A stable lowercase identifier, for storage.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Mail => "mail",
            Self::User => "user",
            Self::Import => "import",
        }
    }

    /// The inverse of [`ContactSource::as_str`].
    ///
    /// `None` for anything else: a value that is not one of these came from a
    /// corrupt row, and guessing at it would be worse than saying so.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "mail" => Some(Self::Mail),
            "user" => Some(Self::User),
            "import" => Some(Self::Import),
            _ => None,
        }
    }
}

/// Whether a person is shown, hidden, or folded into another.
///
/// One mechanism serves deletion, restore and exact undo (specs/005-contacts
/// research R4). A deleted person keeps its addresses, so mail from them keeps
/// counting toward someone hidden — which is what stops the next message from
/// bringing them back — and restoring returns them whole. A person absorbed by
/// a join is `Merged`: it keeps its fields and memberships so the join can be
/// undone exactly, and is never shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ContactState {
    /// Shown, offered in completion, and used for names in the mail.
    #[default]
    Live,
    /// Deleted by the user: offered nowhere, restorable.
    Deleted,
    /// Absorbed by a join into another person.
    Merged,
}

impl ContactState {
    /// A stable lowercase identifier, for storage.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Live => "live",
            Self::Deleted => "deleted",
            Self::Merged => "merged",
        }
    }

    /// The inverse of [`ContactState::as_str`]; `None` for anything else.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "live" => Some(Self::Live),
            "deleted" => Some(Self::Deleted),
            "merged" => Some(Self::Merged),
            _ => None,
        }
    }
}

/// What sort of correspondent a contact is (specs/011-focus-contacts).
///
/// Decided by `postio_classify::kind::detect` from the mail's own headers
/// unless the user has said otherwise ([`KindSource::User`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum ContactKind {
    /// A human being.
    #[default]
    Person,
    /// A business writing as itself (`info@`, `billing@`).
    Company,
    /// A mailing list.
    List,
    /// A newsletter or other bulk sender.
    Newsletter,
    /// A robot: notifications, receipts, bounces.
    Automated,
}

impl ContactKind {
    /// Every kind, in the order the Contacts tabs list them.
    pub const ALL: [Self; 5] = [
        Self::Person,
        Self::Company,
        Self::List,
        Self::Newsletter,
        Self::Automated,
    ];

    /// A stable lowercase identifier, for storage.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Person => "person",
            Self::Company => "company",
            Self::List => "list",
            Self::Newsletter => "newsletter",
            Self::Automated => "automated",
        }
    }

    /// The inverse of [`ContactKind::as_str`]; `None` for anything else.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.as_str() == name)
    }
}

/// Who decided a contact's [`ContactKind`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum KindSource {
    /// The detector, which a newer detector may overrule.
    #[default]
    Detected,
    /// The user, whose word is final.
    User,
}

impl KindSource {
    /// A stable lowercase identifier, for storage.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Detected => "detected",
            Self::User => "user",
        }
    }

    /// The inverse of [`KindSource::as_str`]; `None` for anything else.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "detected" => Some(Self::Detected),
            "user" => Some(Self::User),
            _ => None,
        }
    }
}

/// How a contact's name is shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum ShowAs {
    /// The name as set.
    #[default]
    Full,
    /// The given name alone.
    First,
    /// "Family, Given".
    LastFirst,
    /// The name exactly as the user typed it.
    Custom,
}

impl ShowAs {
    /// A stable lowercase identifier, for storage.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::First => "first",
            Self::LastFirst => "last_first",
            Self::Custom => "custom",
        }
    }

    /// The inverse of [`ShowAs::as_str`]; `None` for anything else.
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

/// What the mail has said about one sender, counted over the newest messages
/// it sent: the evidence `postio_classify::kind::detect` decides a
/// [`ContactKind`] from. Plain counts and the address's own words, so
/// detection never rereads mail and cannot reach the network.
///
/// A count of messages carrying a signal is "no evidence" at zero, never
/// "no": a header not yet known counts as absent.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct KindSignals {
    /// The sender's address, which the local part and domain are read from.
    pub address: String,
    /// The display name its mail carries.
    pub name: Option<String>,
    /// Messages counted.
    pub messages: u32,
    /// Of them, those carrying a `List-Id`.
    pub with_list_id: u32,
    /// Those with `Precedence: list`.
    pub with_list_precedence: u32,
    /// Those offering `List-Unsubscribe`.
    pub with_unsubscribe: u32,
    /// Those with `Precedence: bulk`.
    pub with_bulk: u32,
    /// Those with `Precedence: junk`.
    pub with_junk: u32,
    /// Those with `Auto-Submitted` other than `no` or `auto-replied`.
    pub with_auto_generated: u32,
    /// Those with `Auto-Submitted: auto-replied`.
    pub with_auto_replied: u32,
    /// Messages the user has sent to this person: a reply is evidence of a
    /// human on the other end.
    pub written_to: u32,
    /// Other people at this address's domain among the user's contacts.
    pub people_at_domain: u32,
}

/// The key of the month `at` falls in: `year * 12 + month - 1`, what
/// `address_months` is keyed by (specs/011-focus-contacts S5).
pub fn month_key(at: DateTime<Utc>) -> i64 {
    use chrono::Datelike as _;
    i64::from(at.year()) * 12 + i64::from(at.month0())
}

/// A message in an address's ring of the newest fifty.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecentMessage {
    /// The message.
    pub message: crate::MessageId,
    /// When it was sent or received.
    pub at: DateTime<Utc>,
    /// Whether the user sent it to the person.
    pub outgoing: bool,
}

/// What the mail says about a person, summed over their addresses: the tiles
/// of a card, its twelve-month strip and what is recent between them and the
/// user (specs/011-focus-contacts S5). Read from maintained tables, so what
/// it costs follows the number of addresses and never the amount of mail.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ContactStats {
    /// Messages any of their addresses appeared on.
    pub messages: u32,
    /// Messages the user sent to any of them.
    pub from_you: u32,
    /// Messages from them that have been read.
    pub opened: u32,
    /// The newest message.
    pub last: Option<DateTime<Utc>>,
    /// `(month key, messages)` for the months asked, oldest first, with no
    /// entry for a month with no mail.
    pub months: Vec<(i64, u32)>,
    /// The newest messages across their addresses, newest first, at most
    /// [`RECENT`].
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

/// How many answered messages it takes to say how fast someone replies.
pub const MIN_REPLIES: usize = 3;

/// The longest a gap can be and still be an answer.
pub const MAX_REPLY_GAP: chrono::Duration = chrono::Duration::days(30);

/// Whose move it is on something open between the user and a person.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OpenSide {
    /// The person is waiting on the user: an action marker on their mail that
    /// the user has not dismissed.
    WaitingOnYou,
    /// The user is waiting on the person: a sent message with a reminder
    /// that no reply has settled.
    WaitingOnThem,
}

/// One thing open between the user and a person, with the message behind it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenItem {
    /// Whose move.
    pub side: OpenSide,
    /// What it is: the sentence a marker called out, or the sent message's
    /// subject.
    pub text: String,
    /// The message it hangs on.
    pub message: crate::MessageId,
}

/// What the known-sender popover is read from: the person at an address, what
/// the mail says about them, what is open between them and the user, and
/// where their mail arrives (specs/011-focus-contacts S6).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContactGlance {
    /// The person, made from the stored mail if the catch-up had not reached
    /// them.
    pub person: Contact,
    /// Their counts.
    pub stats: ContactStats,
    /// Up to two things open between them and the user.
    pub open: Vec<OpenItem>,
    /// Where their mail arrives; `None` for by the default rules.
    pub arrival: Option<crate::ContactArrival>,
    /// Where the default rules put it, as the engine last decided for their
    /// newest mail: what [`ContactGlance::arrival`] of `None` means.
    pub default_arrival: crate::ContactArrival,
    /// For a sender not yet saved: how they reached the user and what their
    /// signature says (specs/011-focus-contacts S9). `None` for someone
    /// saved, ignored or not a person.
    pub introduction: Option<crate::contact_form::Introduction>,
}

/// How many open things the popover shows.
pub const GLANCE_OPEN: u32 = 2;

/// How many recent messages an address keeps, and a person's stats show.
pub const RECENT: usize = 50;

/// One address a person owns, with what the mail says about it.
///
/// The counts are summed across the accounts the address was seen through;
/// the store keeps them per account as evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContactAddress {
    /// The store's id for this address.
    pub id: AddressId,
    /// The address, carrying the display name the mail most recently gave it.
    pub address: EmailAddress,
    /// How many messages this address has been seen on.
    pub times_seen: u32,
    /// When it was last seen.
    pub last_seen_at: Option<DateTime<Utc>>,
    /// How many messages the user sent to it.
    pub written: u32,
}

impl ContactAddress {
    /// An address with no sightings yet.
    pub fn new(id: AddressId, address: EmailAddress) -> Self {
        Self {
            id,
            address,
            times_seen: 0,
            last_seen_at: None,
            written: 0,
        }
    }

    /// Records one more sighting at `at`, advancing `last_seen_at` monotonically.
    pub fn record_seen(&mut self, at: DateTime<Utc>) {
        self.times_seen = self.times_seen.saturating_add(1);
        if self.last_seen_at.is_none_or(|last| at > last) {
            self.last_seen_at = Some(at);
        }
    }
}

/// A person the user corresponds with.
///
/// Built from the mail with no setup, and edited, joined and deleted by the
/// user. The counts at this level are aggregates over the person's addresses,
/// kept so the list can order and filter people without adding them up.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Contact {
    /// Local id.
    pub id: ContactId,
    /// The name the user set or picked on a join. Never written by sync.
    pub name: Option<String>,
    /// Organisation, as the user entered it.
    pub organization: Option<String>,
    /// A free-text note.
    pub note: Option<String>,
    /// How this person first came to exist.
    pub source: ContactSource,
    /// Shown, deleted, or folded into another person.
    pub state: ContactState,
    /// The address offered first in completion and used by "write to". One of
    /// [`Contact::addresses`]; a stale id falls back to the first.
    pub preferred: AddressId,
    /// Every address this person owns.
    pub addresses: Vec<ContactAddress>,
    /// The display name most recently seen on any of the person's addresses.
    pub seen_name: Option<String>,
    /// Messages any of the person's addresses has been seen on.
    pub times_seen: u32,
    /// When any of them was last seen.
    pub last_seen_at: Option<DateTime<Utc>>,
    /// Messages the user sent to any of them. More than zero puts the person
    /// in the Contacts list's default view.
    pub written: u32,
    /// What sort of correspondent this is.
    pub kind: ContactKind,
    /// Who decided [`Contact::kind`].
    pub kind_source: KindSource,
    /// When the user saved this person; `None` for someone only known from
    /// mail, however often they write.
    pub saved_at: Option<DateTime<Utc>>,
    /// The given name, when the person has one apart from [`Contact::name`].
    pub given_name: Option<String>,
    /// The family name, likewise.
    pub family_name: Option<String>,
    /// How [`Contact::shown_as`] styles the name.
    pub show_as: ShowAs,
    /// A job title.
    pub title: Option<String>,
}

impl Contact {
    /// Builds an unpersisted person owning one address.
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

    /// The preferred address, or the first one when the preferred id is not
    /// among the person's own.
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

    /// Whether the user saved this person.
    pub fn is_saved(&self) -> bool {
        self.saved_at.is_some()
    }

    /// The name in the style the person asked for ([`Contact::show_as`]).
    ///
    /// A style that needs a part the person lacks falls back rather than
    /// inventing one: to the family name, then [`Contact::display_name`].
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

/// Which people the Contacts list shows (specs/005-contacts FR-005, FR-023a).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum ContactView {
    /// People the user made or imported, and people the user has written to.
    /// A sender the user only ever received mail from is not here, though it
    /// still completes in the composer.
    #[default]
    Written,
    /// Every live person, however they came to be known.
    Everyone,
    /// People the user deleted, for restoring.
    Deleted,
}

/// One row of the Contacts list: what the row draws, and where it sits.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContactListRow {
    /// The person.
    pub id: ContactId,
    /// Their displayed name ([`Contact::display_name`]).
    pub name: String,
    /// Their preferred address, if they have one.
    pub preferred: Option<String>,
    /// How many addresses they own.
    pub address_count: u32,
    /// When any of their addresses was last seen in mail.
    pub last_seen_at: Option<DateTime<Utc>>,
    /// How they came to exist, so the list can mark the ones the user made.
    pub source: ContactSource,
    /// Their state; only the Deleted view shows anything but `Live`.
    pub state: ContactState,
    /// The list's ordering key, which the next page seeks past.
    pub sort_key: String,
    /// What sort of correspondent they are.
    pub kind: ContactKind,
    /// Messages any of their addresses has been seen on.
    pub messages: u32,
    /// Whether the user saved them.
    pub saved: bool,
    /// Where their mail arrives, when the user decided.
    pub arrival: Option<crate::ContactArrival>,
    /// Whether a message from them notifies.
    pub notify: bool,
    /// Whether their HTML is always drawn as sent.
    pub html_original: bool,
    /// The cadence of the digest their mail is held for, when it is.
    pub digest_cadence: Option<String>,
}

/// The Contacts view's kind tabs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum ContactTab {
    /// Everyone.
    #[default]
    All,
    /// People.
    People,
    /// Companies.
    Companies,
    /// Lists and newsletters.
    Lists,
    /// Automated senders.
    Automated,
}

impl ContactTab {
    /// Every tab, left to right.
    pub const ALL: [Self; 5] = [
        Self::All,
        Self::People,
        Self::Companies,
        Self::Lists,
        Self::Automated,
    ];

    /// The kinds the tab holds.
    pub fn kinds(self) -> &'static [ContactKind] {
        match self {
            Self::All => &ContactKind::ALL,
            Self::People => &[ContactKind::Person],
            Self::Companies => &[ContactKind::Company],
            Self::Lists => &[ContactKind::List, ContactKind::Newsletter],
            Self::Automated => &[ContactKind::Automated],
        }
    }

    /// The tab after this one, stopping at the last.
    pub fn next(self) -> Self {
        Self::ALL[(Self::ALL.iter().position(|tab| *tab == self).unwrap_or(0) + 1).min(4)]
    }

    /// The tab before this one, stopping at the first.
    pub fn previous(self) -> Self {
        Self::ALL[Self::ALL
            .iter()
            .position(|tab| *tab == self)
            .unwrap_or(0)
            .saturating_sub(1)]
    }
}

/// What waits in the review queue, each count capped at [`REVIEW_CAP`]:
/// the "Worth a look" line (specs/011-focus-contacts S7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ReviewSummary {
    /// Pairs that may be the same person.
    pub duplicates: u32,
    /// Addresses whose mail bounced.
    pub bounced: u32,
    /// People the user writes to often and has not saved.
    pub frequent_unsaved: u32,
}

/// Where a review count stops counting.
pub const REVIEW_CAP: u32 = 99;

impl ReviewSummary {
    /// Whether there is nothing to review: the line is hidden.
    pub fn is_empty(&self) -> bool {
        self.duplicates == 0 && self.bounced == 0 && self.frequent_unsaved == 0
    }
}

/// A file in a recent message from a person: a chip on their card.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileChip {
    /// Its name.
    pub name: String,
    /// Its media type, which says the chip's letters (`XLSX`, `PDF`).
    pub mime: String,
}

/// One recent issue of a newsletter or list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueRead {
    /// The message.
    pub message: crate::MessageId,
    /// Its subject.
    pub subject: String,
    /// When it arrived.
    pub at: DateTime<Utc>,
    /// Whether the user has read it.
    pub opened: bool,
}

/// How many recent issues and files a card shows.
pub const CARD_RECENT: u32 = 3;

/// Everything a person's card shows, read in one go: the person with their
/// addresses, what the mail says about them, what is open between them and
/// the user, what the user decided about them, and what came recently
/// (specs/011-focus-contacts S7).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContactCardRead {
    /// The person, with every address and what the mail says about each.
    pub person: Contact,
    /// The groups they are in.
    pub groups: Vec<String>,
    /// The tiles, the 12 months and the ring.
    pub stats: ContactStats,
    /// Up to [`CARD_RECENT`] things open between them and the user.
    pub open: Vec<OpenItem>,
    /// What the user decided about them.
    pub settings: crate::ContactSettings,
    /// Where the default rules put their mail.
    pub default_arrival: crate::ContactArrival,
    /// Files in their recent messages, newest first.
    pub files: Vec<FileChip>,
    /// Their recent messages, newest first: a newsletter's issues.
    pub issues: Vec<IssueRead>,
    /// What the user calls each address, and which a bounce named: what the
    /// Edit form fills its type menus from.
    pub address_notes: Vec<crate::contact_form::AddressNote>,
}

/// A page of the Contacts list: its rows, and the sort key of the row just
/// before the first, which says whether the first starts a new letter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContactsPageRead {
    /// The rows, by name.
    pub rows: Vec<ContactListRow>,
    /// The sort key of the row before them; `None` for the first row of
    /// the list.
    pub before: Option<String>,
}

/// What the Contacts view's strip and footer say: the counts the tabs and
/// the footer show, the review queue, and how far the catch-up has got.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ContactsOverview {
    /// Every live person.
    pub total: u32,
    /// How many of them are saved.
    pub saved: u32,
    /// Each tab's count, in [`ContactTab::ALL`] order.
    pub tabs: Vec<(ContactTab, u32)>,
    /// What waits in the review queue.
    pub review: ReviewSummary,
    /// `(made, of)` while addresses with mail still wait for a person.
    pub building: Option<(u32, u32)>,
}

/// Everything the Contacts detail view shows about one person (FR-006).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContactDetail {
    /// The person, with every address and what the mail says about each.
    pub person: Contact,
    /// The names of the groups they belong to.
    pub groups: Vec<String>,
    /// How many distinct messages involve any of their addresses.
    pub messages: u64,
}

/// Why two people are offered as possibly one (specs/005-contacts R11).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SuggestionReason {
    /// Their names match, ignoring case and spacing.
    SameName,
    /// Mail sent to one of them was answered from the other, this often.
    Replied {
        /// How many answers crossed.
        replies: u32,
    },
}

/// Two people who might be one, and why -- offered, never acted on
/// (FR-019).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JoinSuggestion {
    /// The two, each with their addresses and counts: the evidence.
    pub people: [Contact; 2],
    /// Why they are offered.
    pub reason: SuggestionReason,
}

/// What the editor changes about a person (FR-021): their name,
/// organisation and note. Addresses are changed one at a time, with their own
/// verbs, because each is a claim about who owns what.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersonEdit {
    /// The name the user gave them; `None` or blank leaves them unnamed.
    pub name: Option<String>,
    /// Their organisation.
    pub organization: Option<String>,
    /// The user's note.
    pub note: Option<String>,
}

/// What moving an address did, precisely enough to put it back: who had it,
/// and the state they were in if the move left them with no address at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AddressMove {
    /// Who owned the address before, or nobody.
    pub previous: Option<ContactId>,
    /// `previous`'s state before the move emptied them, when it did.
    pub emptied: Option<ContactState>,
}

/// The fields of a person a join may change, as they were before it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersonFields {
    /// The name the user set, if any.
    pub name: Option<String>,
    /// Organisation.
    pub organization: Option<String>,
    /// Note.
    pub note: Option<String>,
    /// Provenance.
    pub source: ContactSource,
    /// The preferred address.
    pub preferred: AddressId,
}

/// What a join changed, precisely enough to take it back
/// (specs/005-contacts research R8): which addresses came from which person,
/// what the survivor's fields were, and which group memberships the join
/// gave it that it did not already have.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JoinReceipt {
    /// The person the others were joined into.
    pub into: ContactId,
    /// Each absorbed person and the addresses that were theirs.
    pub restore: Vec<(ContactId, Vec<AddressId>)>,
    /// The survivor's fields before the join.
    pub prior: PersonFields,
    /// Groups the survivor joined only because an absorbed person was in them.
    pub added_memberships: Vec<ContactGroupId>,
    /// What the survivor had decided before the join, which the join fills
    /// the gaps of from the absorbed people (specs/011-focus-contacts Q8).
    #[serde(default)]
    pub prior_settings: Box<crate::ContactSettings>,
}

#[cfg(test)]
mod tests {
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

        // A style that needs a part the person does not have falls back to
        // the name, then to what the mail called them, then the address.
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
