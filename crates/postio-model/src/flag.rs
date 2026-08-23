//! Message flags and keywords.

use std::cmp::Ordering;
use std::collections::BTreeSet;
use std::fmt;
use std::hash::{Hash, Hasher};

use serde::{Deserialize, Serialize};

/// A flag on a message.
///
/// # Invariants
///
/// * **One canonical representation per flag.** [`Flag::parse`] folds every
///   spelling of a well-known flag onto its variant, so `\Seen`, `\SEEN` and
///   `\seen` are the same value, and there is no `Keyword("\\Seen")`.
/// * **Keywords are case-insensitive** (RFC 3501 keywords are), so a
///   [`FlagSet`] can never hold both `Work` and `work`. The case that was first
///   seen is preserved for display.
/// * **[`Flag::Recent`] is transient.** It is a per-session server signal and is
///   never persisted; strip it with [`FlagSet::persistable`] before storing.
/// * **[`Flag::Deleted`] is the IMAP `\Deleted` flag**, meaning "marked for
///   expunge on the server" — it is not Postio's local delete, which is
///   [`LocalSyncState::deleted_locally`](crate::LocalSyncState::deleted_locally).
#[derive(Debug, Clone)]
pub enum Flag {
    /// `\Seen` — the message has been read.
    Seen,
    /// `\Answered` — the message has been replied to.
    Answered,
    /// `\Flagged` — "Flagged" in the sidebar (not "Starred").
    Flagged,
    /// `\Deleted` — marked for expunge on the server.
    Deleted,
    /// `\Draft` — an unsent draft.
    Draft,
    /// `\Recent` — transient, never persisted.
    Recent,
    /// `$Forwarded` — the message has been forwarded.
    Forwarded,
    /// `$Junk` — classified as junk.
    Junk,
    /// `$NotJunk` — explicitly classified as not junk.
    NotJunk,
    /// Any other server keyword. Compared case-insensitively.
    Keyword(String),
}

impl Flag {
    /// Parses a wire flag or keyword into its canonical variant.
    ///
    /// Never fails: anything unrecognized becomes a [`Flag::Keyword`].
    pub fn parse(raw: impl AsRef<str>) -> Self {
        let raw = raw.as_ref().trim();
        match raw.to_ascii_lowercase().as_str() {
            "\\seen" => Self::Seen,
            "\\answered" => Self::Answered,
            "\\flagged" => Self::Flagged,
            "\\deleted" => Self::Deleted,
            "\\draft" => Self::Draft,
            "\\recent" => Self::Recent,
            "$forwarded" => Self::Forwarded,
            "$junk" => Self::Junk,
            "$notjunk" => Self::NotJunk,
            _ => Self::Keyword(raw.to_owned()),
        }
    }

    /// The canonical wire spelling.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Seen => "\\Seen",
            Self::Answered => "\\Answered",
            Self::Flagged => "\\Flagged",
            Self::Deleted => "\\Deleted",
            Self::Draft => "\\Draft",
            Self::Recent => "\\Recent",
            Self::Forwarded => "$Forwarded",
            Self::Junk => "$Junk",
            Self::NotJunk => "$NotJunk",
            Self::Keyword(keyword) => keyword,
        }
    }

    /// Whether this flag is a per-session server signal that must not be stored.
    pub fn is_transient(&self) -> bool {
        matches!(self, Self::Recent)
    }

    /// Whether this flag is one of the RFC 3501 system flags.
    pub fn is_system(&self) -> bool {
        !matches!(self, Self::Keyword(_))
    }

    /// Sort rank, so system flags order ahead of keywords deterministically.
    fn rank(&self) -> u8 {
        match self {
            Self::Seen => 0,
            Self::Answered => 1,
            Self::Flagged => 2,
            Self::Deleted => 3,
            Self::Draft => 4,
            Self::Recent => 5,
            Self::Forwarded => 6,
            Self::Junk => 7,
            Self::NotJunk => 8,
            Self::Keyword(_) => 9,
        }
    }
}

impl PartialEq for Flag {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Keyword(a), Self::Keyword(b)) => a.eq_ignore_ascii_case(b),
            _ => self.rank() == other.rank(),
        }
    }
}

impl Eq for Flag {}

impl Ord for Flag {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Self::Keyword(a), Self::Keyword(b)) => {
                a.to_ascii_lowercase().cmp(&b.to_ascii_lowercase())
            }
            _ => self.rank().cmp(&other.rank()),
        }
    }
}

impl PartialOrd for Flag {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Hash for Flag {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.rank().hash(state);
        if let Self::Keyword(keyword) = self {
            keyword.to_ascii_lowercase().hash(state);
        }
    }
}

impl fmt::Display for Flag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Serialize for Flag {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for Flag {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(Self::parse(String::deserialize(deserializer)?))
    }
}

/// The set of flags on a message.
///
/// A set, not a list: insertion order carries no meaning, duplicates collapse,
/// and iteration is in a stable canonical order so two equal sets always
/// serialize identically.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct FlagSet(BTreeSet<Flag>);

impl FlagSet {
    /// An empty set.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a flag; returns whether it was not already present.
    pub fn insert(&mut self, flag: Flag) -> bool {
        self.0.insert(flag)
    }

    /// Removes a flag; returns whether it was present.
    pub fn remove(&mut self, flag: &Flag) -> bool {
        self.0.remove(flag)
    }

    /// Whether the flag is present.
    pub fn contains(&self, flag: &Flag) -> bool {
        self.0.contains(flag)
    }

    /// Iterates in canonical order.
    pub fn iter(&self) -> std::collections::btree_set::Iter<'_, Flag> {
        self.0.iter()
    }

    /// The number of distinct flags.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether no flags are set.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The same set with every transient flag removed, ready to persist.
    pub fn persistable(&self) -> Self {
        Self(
            self.0
                .iter()
                .filter(|flag| !flag.is_transient())
                .cloned()
                .collect(),
        )
    }

    /// Whether `\Seen` is set.
    pub fn is_seen(&self) -> bool {
        self.contains(&Flag::Seen)
    }

    /// Whether `\Seen` is absent.
    pub fn is_unread(&self) -> bool {
        !self.is_seen()
    }

    /// Whether `\Flagged` is set.
    pub fn is_flagged(&self) -> bool {
        self.contains(&Flag::Flagged)
    }

    /// Whether `\Answered` is set.
    pub fn is_answered(&self) -> bool {
        self.contains(&Flag::Answered)
    }

    /// Whether `\Draft` is set.
    pub fn is_draft(&self) -> bool {
        self.contains(&Flag::Draft)
    }

    /// Whether `\Deleted` is set (marked for expunge on the server).
    pub fn is_deleted(&self) -> bool {
        self.contains(&Flag::Deleted)
    }
}

impl FromIterator<Flag> for FlagSet {
    fn from_iter<I: IntoIterator<Item = Flag>>(iter: I) -> Self {
        Self(iter.into_iter().collect())
    }
}

impl<'a> IntoIterator for &'a FlagSet {
    type Item = &'a Flag;
    type IntoIter = std::collections::btree_set::Iter<'a, Flag>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

impl Extend<Flag> for FlagSet {
    fn extend<I: IntoIterator<Item = Flag>>(&mut self, iter: I) {
        self.0.extend(iter);
    }
}
