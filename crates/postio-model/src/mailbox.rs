use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::ids::{AccountId, Generation, MailboxId, ModSeq, SignatureId, Uid};

/// A view has a name, position and count and nothing else: no path, UIDVALIDITY or sync state, and
/// it cannot receive a moved message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum RoleKind {
    Folder,
    View,
}

/// Routed by role, never name, since names differ per provider (iCloud reports `Sent Messages` with
/// no RFC 6154 `SPECIAL-USE`). Use `resolve`: the server attribute first, name matching as
/// fallback.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
pub enum MailboxRole {
    Inbox,
    Archive,
    Sent,
    Drafts,
    Trash,
    Junk,
    Flagged,
    /// Client-only; no `SPECIAL-USE` attribute names it.
    Snoozed,
    /// Client-only: a message on its way is filed in Drafts on the server.
    Outbox,
    #[default]
    Regular,
}

impl MailboxRole {
    /// `None` for attributes that are not a role Postio models (`\All`, `\HasChildren`,
    /// `\Noselect`).
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

    /// For servers that advertise no `SPECIAL-USE`, notably iCloud. Full path first, then the leaf
    /// segment, so `INBOX/Drafts` resolves but `Projects/Postio` stays `Regular`.
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
            "sent" | "sent messages" | "sent items" | "sent mail" => Some(Self::Sent),
            "trash" | "deleted messages" | "deleted items" | "bin" | "deleted" => Some(Self::Trash),
            "junk" | "spam" | "junk e-mail" | "junk email" | "bulk mail" => Some(Self::Junk),
            "archive" | "archives" | "archived" => Some(Self::Archive),
            "drafts" | "draft" => Some(Self::Drafts),
            "flagged" | "starred" => Some(Self::Flagged),
            _ => None,
        }
    }

    /// The server attribute always wins: a folder named `Sent Messages` that the server marks
    /// `\Archive` is an archive.
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

    pub fn is_special(self) -> bool {
        !matches!(self, Self::Regular)
    }

    /// Accepts only what `as_str` emits, unlike `guess_from_name`.
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
            "outbox" => Some(Self::Outbox),
            "regular" => Some(Self::Regular),
            _ => None,
        }
    }

    /// `Regular` is not among them: an account is owed an Inbox, not an arbitrary folder.
    pub const RESERVED: [Self; 6] = [
        Self::Inbox,
        Self::Archive,
        Self::Sent,
        Self::Drafts,
        Self::Trash,
        Self::Junk,
    ];

    /// Exhaustive with no fallback arm, so a new role fails to compile until classified. A `View`
    /// must never be stored (`MailboxRepository` refuses it; the schema's `CHECK` lists only
    /// folders) nor offered as a destination.
    pub fn kind(self) -> RoleKind {
        match self {
            Self::Inbox
            | Self::Archive
            | Self::Sent
            | Self::Drafts
            | Self::Trash
            | Self::Junk
            | Self::Regular
            // A folder despite being a sidebar view: RFC 6154 defines `\Flagged` as `SPECIAL-USE`
            // (Gmail's Starred);
            // the synthetic row is for servers without one.
            | Self::Flagged => RoleKind::Folder,
            // Neither has a `SPECIAL-USE` attribute, so discovery can never
            // assign one and no server can advertise one.
            Self::Snoozed | Self::Outbox => RoleKind::View,
        }
    }

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
            Self::Outbox => "outbox",
            Self::Regular => "regular",
        }
    }
}

/// Folder paths the user assigned a role by hand: a third tier above `resolve`, for self-hosted
/// servers that advertise nothing and name folders in an unlisted language, where every role verb
/// would refuse permanently (#164). Keyed by the full server path, not the leaf, since two folders
/// can share a leaf. An empty set resolves exactly as `MailboxRole::resolve`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RoleOverrides {
    /// Path to role, inverted from the config's `role = "path"` because resolution asks what a
    /// folder is for.
    by_path: BTreeMap<String, MailboxRole>,
}

impl RoleOverrides {
    /// One folder per role: the last pair for a role wins, as in a TOML table, since two folders on
    /// one role cannot be acted on.
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

    pub fn is_empty(&self) -> bool {
        self.by_path.is_empty()
    }

    pub fn pairs(&self) -> impl Iterator<Item = (MailboxRole, &str)> {
        let mut pairs: Vec<(MailboxRole, &str)> = self
            .by_path
            .iter()
            .map(|(path, role)| (*role, path.as_str()))
            .collect();
        pairs.sort_by_key(|(role, _)| *role);
        pairs.into_iter()
    }

    /// How an account's own map (ADR 0035) sits on `[mailboxes]`: the store wins where it speaks,
    /// the file fills in the rest.
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

    pub fn role_for(&self, path: &str) -> Option<MailboxRole> {
        self.by_path.get(path).copied()
    }

    /// Precedence: override, then `SPECIAL-USE`, then the name; the user looked at their own server
    /// and disagreed. Called from folder reconciliation, not the IMAP edge, which must not read
    /// user configuration.
    pub fn resolve<I, S>(&self, attributes: I, path: &str) -> MailboxRole
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self.settle(MailboxRole::resolve(attributes, path), path)
    }

    /// Over a role the backend already settled (server claim, shallowest name, alphabet):
    /// re-deriving from the name would hand a look-alike its role back, giving one account two
    /// `sent` folders (#943).
    pub fn settle(&self, natural: MailboxRole, path: &str) -> MailboxRole {
        if let Some(role) = self.role_for(path) {
            return role;
        }
        // A role pinned to another folder is spoken for: otherwise two rows wear it and `by_role`
        // returns an arbitrary one, varying between runs. Demoted to `Regular`.
        if self.claims(natural) {
            return MailboxRole::Regular;
        }
        natural
    }

    fn claims(&self, role: MailboxRole) -> bool {
        self.by_path.values().any(|pinned| *pinned == role)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct MailboxCounts {
    pub total: u32,
    pub unread: u32,
    pub flagged: u32,
    pub snoozed: u32,
    /// Only Drafts has one: failed or unconfirmed sends. Not maintained by the count triggers:
    /// `MailboxRepository::draft_counts` answers it and the sidebar feed fills it in before the row
    /// is drawn.
    pub attention: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mailbox {
    pub id: MailboxId,
    pub account_id: AccountId,
    pub parent_id: Option<MailboxId>,
    pub name: String,
    pub path: String,
    pub delimiter: Option<char>,
    pub role: MailboxRole,
    pub selectable: bool,
    pub subscribed: bool,
    /// Overrides the account's default signature when composing from this folder (#394). Local
    /// preference, never server state.
    #[serde(default)]
    pub signature_id: Option<SignatureId>,
    /// Excludes this folder from the background backfill lane, both axes (ADR 0016, 0017, #350).
    /// Survives a resync because `discover::reconcile` copies across only what `LIST` said. Deletes
    /// nothing already pulled; interactive on-open fetches still run.
    #[serde(default)]
    pub backfill_excluded: bool,
    pub counts: MailboxCounts,
    /// Generation of the mailbox's UID space; a change invalidates every UID.
    pub generation: Option<Generation>,
    pub uid_next: Option<Uid>,
    pub highest_mod_seq: Option<ModSeq>,
    pub last_synced_at: Option<DateTime<Utc>>,
}

impl Mailbox {
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

    /// A change means the server reused or renumbered the UID space: resync from scratch.
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

    #[test]
    fn every_role_is_a_folder_or_a_view() {
        // Exhaustive on purpose, with the count asserted so a variant cannot be dropped from this
        // list.
        let all = [
            MailboxRole::Inbox,
            MailboxRole::Archive,
            MailboxRole::Sent,
            MailboxRole::Drafts,
            MailboxRole::Trash,
            MailboxRole::Junk,
            MailboxRole::Flagged,
            MailboxRole::Snoozed,
            MailboxRole::Outbox,
            MailboxRole::Regular,
        ];
        assert_eq!(all.len(), 10, "a role was added without reaching this test");
        for role in all {
            let _ = role.kind();
        }
    }

    #[test]
    fn a_view_names_no_folder_and_a_folder_does() {
        for role in [MailboxRole::Snoozed, MailboxRole::Outbox] {
            assert_eq!(
                role.kind(),
                RoleKind::View,
                "{role:?} is a saved question about messages filed elsewhere"
            );
        }
        for role in [
            MailboxRole::Inbox,
            MailboxRole::Archive,
            MailboxRole::Sent,
            MailboxRole::Drafts,
            MailboxRole::Trash,
            MailboxRole::Junk,
            MailboxRole::Regular,
            MailboxRole::Flagged,
        ] {
            assert_eq!(
                role.kind(),
                RoleKind::Folder,
                "{role:?} names a folder on the server"
            );
        }
        // `Flagged` is both: RFC 6154 defines it, so a server can have this folder; `Snoozed` and
        // `Outbox` have no attribute.
        assert_eq!(
            MailboxRole::from_special_use("\\Flagged"),
            Some(MailboxRole::Flagged)
        );
        assert_eq!(MailboxRole::from_special_use("\\Snoozed"), None);
        assert_eq!(MailboxRole::from_special_use("\\Outbox"), None);
    }

    #[test]
    fn the_reserved_roles_are_the_six_every_account_must_have() {
        assert_eq!(
            MailboxRole::RESERVED,
            [
                MailboxRole::Inbox,
                MailboxRole::Archive,
                MailboxRole::Sent,
                MailboxRole::Drafts,
                MailboxRole::Trash,
                MailboxRole::Junk,
            ]
        );
        assert_eq!(MailboxRole::Regular.kind(), RoleKind::Folder);
        assert!(!MailboxRole::RESERVED.contains(&MailboxRole::Regular));
        for role in MailboxRole::RESERVED {
            assert_eq!(role.kind(), RoleKind::Folder);
        }
    }

    #[test]
    fn the_outbox_round_trips_through_its_stored_identifier() {
        assert_eq!(MailboxRole::Outbox.as_str(), "outbox");
        assert_eq!(MailboxRole::from_name("outbox"), Some(MailboxRole::Outbox));
    }
}
