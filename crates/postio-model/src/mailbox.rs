//! Mailboxes (folders) and their special-use roles.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::ids::{AccountId, Generation, MailboxId, ModSeq, SignatureId, Uid};

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
    /// The sidebar's "Snoozed" view — client-only, the same as [`Self::Flagged`]
    /// wearing a folder's clothes; no `SPECIAL-USE` attribute names it.
    Snoozed,
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

    /// The inverse of [`MailboxRole::as_str`].
    ///
    /// Parses the stable identifier a role is *stored* as. Distinct from
    /// [`MailboxRole::guess_from_name`], which guesses a role from the folder
    /// name a server reported; this one only ever accepts what `as_str` emits.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "inbox" => Some(Self::Inbox),
            "archive" => Some(Self::Archive),
            "sent" => Some(Self::Sent),
            "drafts" => Some(Self::Drafts),
            "trash" => Some(Self::Trash),
            "junk" => Some(Self::Junk),
            "flagged" => Some(Self::Flagged),
            "snoozed" => Some(Self::Snoozed),
            "regular" => Some(Self::Regular),
            _ => None,
        }
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
            Self::Snoozed => "snoozed",
            Self::Regular => "regular",
        }
    }
}

/// Folder paths the user has assigned a role to by hand.
///
/// A third tier above the two [`MailboxRole::resolve`] already has, and the
/// one that makes the other two admit they can be wrong. `SPECIAL-USE` covers
/// servers that advertise it and name-matching covers the spellings Postio has
/// been taught; neither covers a self-hosted server that advertises nothing
/// and names its folders in a language nobody added to the list. On that
/// server every role-driven verb — archive, delete, junk, file as sent —
/// refuses permanently, and refusing is the honest answer only for as long as
/// there is no way to say what the folder actually is (#164).
///
/// Keyed by the **path the server reports**, delimiters and all, rather than
/// by the leaf name: two folders can both be called `Old` under different
/// parents, and a leaf match would silently pick one of them.
///
/// The default is empty, and an empty set resolves exactly as
/// [`MailboxRole::resolve`] does. That is a property worth stating because
/// every account that never writes `[mailboxes]` goes through this path.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RoleOverrides {
    /// Path to role. Inverted from the `role = "path"` config writes, because
    /// resolution asks "what is this folder for", never "where is archive".
    by_path: BTreeMap<String, MailboxRole>,
}

impl RoleOverrides {
    /// Builds a set from `role = path` pairs, as `[mailboxes]` spells them.
    ///
    /// One folder per role: the last pair for a role wins, mirroring what a
    /// TOML table does with a repeated key. Roles are looked up *by role*
    /// downstream — `by_role(account, Archive)` returns one mailbox — so two
    /// folders wearing one role is a state nothing can act on.
    pub fn from_pairs<I, S>(pairs: I) -> Self
    where
        I: IntoIterator<Item = (MailboxRole, S)>,
        S: Into<String>,
    {
        let mut by_role: BTreeMap<MailboxRole, String> = BTreeMap::new();
        for (role, path) in pairs {
            by_role.insert(role, path.into());
        }
        Self {
            by_path: by_role
                .into_iter()
                .map(|(role, path)| (path, role))
                .collect(),
        }
    }

    /// Whether anything has been overridden at all.
    pub fn is_empty(&self) -> bool {
        self.by_path.is_empty()
    }

    /// Every mapping, as `(role, path)` pairs in role order.
    pub fn pairs(&self) -> impl Iterator<Item = (MailboxRole, &str)> {
        let mut pairs: Vec<(MailboxRole, &str)> = self
            .by_path
            .iter()
            .map(|(path, role)| (*role, path.as_str()))
            .collect();
        pairs.sort_by_key(|(role, _)| *role);
        pairs.into_iter()
    }

    /// These mappings with `pairs` laid over them: a role `pairs` names is
    /// theirs, every other role stays as it was.
    ///
    /// How an account's own map (ADR 0025) sits on `[mailboxes]`: the file is
    /// one table for every account and the store holds what the user said
    /// about this one, so the store wins where it speaks and the file fills in
    /// where it does not. One folder per role survives, because
    /// [`from_pairs`](Self::from_pairs) keeps the last pair for a role.
    pub fn over<I, S>(&self, pairs: I) -> Self
    where
        I: IntoIterator<Item = (MailboxRole, S)>,
        S: Into<String>,
    {
        Self::from_pairs(
            self.pairs()
                .map(|(role, path)| (role, path.to_owned()))
                .chain(pairs.into_iter().map(|(role, path)| (role, path.into()))),
        )
    }

    /// The role the user assigned to `path`, if they assigned one.
    pub fn role_for(&self, path: &str) -> Option<MailboxRole> {
        self.by_path.get(path).copied()
    }

    /// The full precedence: **override, then `SPECIAL-USE`, then the name.**
    ///
    /// Above `SPECIAL-USE` and not merely above the name guess. A server that
    /// marks a folder `\Junk` is usually right, and "usually" is exactly what
    /// an override is for — the user has looked at their own server and
    /// disagreed, and there is no reading of that in which Postio knows
    /// better.
    ///
    /// One function with three tiers rather than a second code path, which is
    /// what keeps the precedence stateable in one place. It is called from
    /// folder reconciliation rather than from the IMAP edge where `resolve`
    /// runs, because the edge parses what the *server* said and has no
    /// business reading the user's configuration.
    pub fn resolve<I, S>(&self, attributes: I, path: &str) -> MailboxRole
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self.settle(MailboxRole::resolve(attributes, path), path)
    }

    /// The same precedence, over a role the backend has already settled.
    ///
    /// A listing arrives with its contested roles resolved -- the server's
    /// claim first, then the shallowest name, then the alphabet -- and that
    /// verdict is what the user's mapping goes on top of. Re-deriving the
    /// role from the name here would hand a look-alike its role straight
    /// back, which is how one account ended up with two `sent` folders
    /// (#943).
    pub fn settle(&self, natural: MailboxRole, path: &str) -> MailboxRole {
        if let Some(role) = self.role_for(path) {
            return role;
        }
        // A role the user has pinned to some *other* folder is spoken for, so
        // this one cannot also claim it. Without this, pointing `archive` at a
        // new folder on a server that already has one called `Archive` leaves
        // two rows wearing the role, and `by_role` returns whichever the query
        // reaches first -- so archiving would go to an arbitrary one of them,
        // and which one could change between runs. Demoted to `Regular`,
        // which is what the folder is once it is not the archive.
        if self.claims(natural) {
            return MailboxRole::Regular;
        }
        natural
    }

    /// Whether some folder has been pinned to `role`.
    fn claims(&self, role: MailboxRole) -> bool {
        self.by_path.values().any(|pinned| *pinned == role)
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
    /// Messages currently snoozed (`snoozed_until` in the future).
    pub snoozed: u32,
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
    /// A signature that overrides the account's own default when composing
    /// from this folder (#394) — see [`crate::signature_default::resolve`]. Local
    /// preference, never server state: unlike every field below these two,
    /// nothing in a sync pass ever sets or reads it.
    #[serde(default)]
    pub signature_id: Option<SignatureId>,
    /// Excludes this folder from the background backfill lane — both axes
    /// ADR 0017 split it into, headers/text and attachment payloads (ADR
    /// 0016, #350). The same local-preference shape as `signature_id`
    /// immediately above: `postio-sync::discover::reconcile` clones the
    /// existing row and copies across only what the server's `LIST` said, so
    /// this survives a resync exactly the way `signature_id` does. Turning
    /// it on does not delete or expire anything already pulled, and does not
    /// stop an interactive, on-open fetch — the same distinction
    /// `postio-sync`'s `BackfillPolicy::background` draws for its
    /// account-wide knob; this is that knob, narrowed to one folder.
    #[serde(default)]
    pub backfill_excluded: bool,
    /// Cached counts.
    pub counts: MailboxCounts,
    /// Generation of the mailbox's UID space; a change invalidates every UID.
    pub generation: Option<Generation>,
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
            signature_id: None,
            backfill_excluded: false,
            counts: MailboxCounts::default(),
            generation: None,
            uid_next: None,
            highest_mod_seq: None,
            last_synced_at: None,
        }
    }

    /// Whether every UID cached for this mailbox is stale under `observed`.
    ///
    /// A `UIDVALIDITY` change means the server reused or renumbered the UID
    /// space and the mailbox must be resynchronized from scratch.
    pub fn generation_changed(&self, observed: Generation) -> bool {
        matches!(self.generation, Some(known) if known != observed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_role_round_trips_through_its_stored_identifier() {
        for role in [
            MailboxRole::Inbox,
            MailboxRole::Archive,
            MailboxRole::Sent,
            MailboxRole::Drafts,
            MailboxRole::Trash,
            MailboxRole::Junk,
            MailboxRole::Flagged,
            MailboxRole::Regular,
        ] {
            assert_eq!(MailboxRole::from_name(role.as_str()), Some(role));
        }
        assert_eq!(MailboxRole::from_name("Inbox"), None, "spelling is exact");
        assert_eq!(MailboxRole::from_name("nonsense"), None);
    }
}
