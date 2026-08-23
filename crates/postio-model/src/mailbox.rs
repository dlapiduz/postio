//! Mailboxes (folders) and their special-use roles.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::ids::{AccountId, MailboxId, ModSeq, Uid, UidValidity};

/// What a mailbox is *for*, independent of what the server calls it.
///
/// Postio routes archive, trash, junk and sent by role, never by name, because
/// the names differ per provider: iCloud reports `Sent Messages` and
/// `Deleted Messages` and advertises no RFC 6154 `SPECIAL-USE` attributes for
/// them. Use [`MailboxRole::resolve`], which trusts the server attribute when
/// there is one and falls back to name matching when there is not.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
pub enum MailboxRole {
    /// The account's inbox.
    Inbox,
    /// Where `a` / `A` archive to.
    Archive,
    /// Where sent mail is filed.
    Sent,
    /// Where unsent drafts live.
    Drafts,
    /// Where deleted mail is moved.
    Trash,
    /// Junk / spam.
    Junk,
    /// A server-side flagged/starred view.
    Flagged,
    /// An ordinary user folder.
    #[default]
    Regular,
}

impl MailboxRole {
    /// Maps an RFC 6154 `SPECIAL-USE` attribute onto a role.
    ///
    /// Returns `None` for attributes that are not a role Postio models —
    /// `\All`, `\HasChildren`, `\Noselect` and friends.
    pub fn from_special_use(attribute: &str) -> Option<Self> {
        let attribute = attribute
            .trim()
            .trim_start_matches('\\')
            .to_ascii_lowercase();
        match attribute.as_str() {
            "inbox" => Some(Self::Inbox),
            "archive" => Some(Self::Archive),
            "sent" => Some(Self::Sent),
            "drafts" => Some(Self::Drafts),
            "trash" => Some(Self::Trash),
            "junk" => Some(Self::Junk),
            "flagged" => Some(Self::Flagged),
            _ => None,
        }
    }

    /// Guesses a role from a folder name, for servers that advertise no
    /// `SPECIAL-USE` — notably iCloud.
    ///
    /// Matches the full path first, then the leaf segment, so `INBOX/Drafts`
    /// resolves but `Projects/Postio` stays [`MailboxRole::Regular`].
    pub fn guess_from_name(name: &str) -> Self {
        let name = name.trim();
        if name.eq_ignore_ascii_case("inbox") {
            return Self::Inbox;
        }
        if let Some(role) = Self::match_name(name) {
            return role;
        }
        let leaf = name.rsplit(['/', '.', '\\']).next().unwrap_or(name).trim();
        Self::match_name(leaf).unwrap_or(Self::Regular)
    }

    fn match_name(name: &str) -> Option<Self> {
        match name.to_ascii_lowercase().as_str() {
            // iCloud uses "Sent Messages"; Outlook/Exchange use "Sent Items".
            "sent" | "sent messages" | "sent items" | "sent mail" => Some(Self::Sent),
            // iCloud uses "Deleted Messages"; Gmail's UI language uses "Bin".
            "trash" | "deleted messages" | "deleted items" | "bin" | "deleted" => Some(Self::Trash),
            "junk" | "spam" | "junk e-mail" | "junk email" | "bulk mail" => Some(Self::Junk),
            "archive" | "archives" | "archived" => Some(Self::Archive),
            "drafts" | "draft" => Some(Self::Drafts),
            "flagged" | "starred" => Some(Self::Flagged),
            _ => None,
        }
    }

    /// Resolves a role from server attributes with a name-based fallback.
    ///
    /// The server attribute always wins — a folder literally named
    /// `Sent Messages` that the server marks `\Archive` is an archive.
    pub fn resolve<I, S>(attributes: I, name: &str) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        for attribute in attributes {
            if let Some(role) = Self::from_special_use(attribute.as_ref()) {
                return role;
            }
        }
        Self::guess_from_name(name)
    }

    /// Whether this is a special-use role rather than an ordinary folder.
    pub fn is_special(self) -> bool {
        !matches!(self, Self::Regular)
    }

    /// A stable lowercase identifier, for storage and config.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Inbox => "inbox",
            Self::Archive => "archive",
            Self::Sent => "sent",
            Self::Drafts => "drafts",
            Self::Trash => "trash",
            Self::Junk => "junk",
            Self::Flagged => "flagged",
            Self::Regular => "regular",
        }
    }
}

/// Cached message counts for a mailbox, so the sidebar never counts rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct MailboxCounts {
    /// Total messages.
    pub total: u32,
    /// Messages without `\Seen`.
    pub unread: u32,
    /// Messages with `\Flagged`.
    pub flagged: u32,
}

/// A folder on the server, mirrored locally.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mailbox {
    /// Local id.
    pub id: MailboxId,
    /// Owning account.
    pub account_id: AccountId,
    /// Parent folder, when the hierarchy is known.
    pub parent_id: Option<MailboxId>,
    /// Leaf name, for display.
    pub name: String,
    /// Full server path, including hierarchy delimiters.
    pub path: String,
    /// Hierarchy delimiter reported by the server, if any.
    pub delimiter: Option<char>,
    /// What this folder is for.
    pub role: MailboxRole,
    /// Whether the folder can hold messages (`\Noselect` folders cannot).
    pub selectable: bool,
    /// Whether the account is subscribed to it.
    pub subscribed: bool,
    /// Cached counts.
    pub counts: MailboxCounts,
    /// Generation of the mailbox's UID space; a change invalidates every UID.
    pub uid_validity: Option<UidValidity>,
    /// The UID the server will assign next.
    pub uid_next: Option<Uid>,
    /// Highest modification sequence seen, for incremental resync.
    pub highest_mod_seq: Option<ModSeq>,
    /// When this mailbox last completed a sync.
    pub last_synced_at: Option<DateTime<Utc>>,
}

impl Mailbox {
    /// Builds an unpersisted mailbox, deriving its leaf name and role from `path`.
    pub fn new(account_id: AccountId, path: impl Into<String>, delimiter: Option<char>) -> Self {
        let path = path.into();
        let name = match delimiter {
            Some(delimiter) => path.rsplit(delimiter).next().unwrap_or(&path).to_owned(),
            None => path.clone(),
        };
        let role = MailboxRole::guess_from_name(&path);
        Self {
            id: MailboxId::UNASSIGNED,
            account_id,
            parent_id: None,
            name,
            path,
            delimiter,
            role,
            selectable: true,
            subscribed: true,
            counts: MailboxCounts::default(),
            uid_validity: None,
            uid_next: None,
            highest_mod_seq: None,
            last_synced_at: None,
        }
    }

    /// Whether every UID cached for this mailbox is stale under `observed`.
    ///
    /// A `UIDVALIDITY` change means the server reused or renumbered the UID
    /// space and the mailbox must be resynchronized from scratch.
    pub fn uid_validity_changed(&self, observed: UidValidity) -> bool {
        matches!(self.uid_validity, Some(known) if known != observed)
    }
}
