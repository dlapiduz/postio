//! Capabilities come from the list read after authentication: iCloud omits CONDSTORE, QRESYNC,
//! UIDPLUS and IDLE pre-auth (ADR 0001 Q3).

use std::collections::BTreeSet;
use std::fmt;

use super::{BackendError, BackendResult};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum Capability {
    Imap4Rev1,
    Enable,
    CondStore,
    QResync,
    Idle,
    UidPlus,
    Move,
    SpecialUse,
    ListExtended,
    Namespace,
    Unselect,
    Binary,
    Compress,
    Sort,
    Thread,
    ESearch,
    Id,
    LiteralPlus,
}

impl Capability {
    pub const ALL: &'static [Capability] = &[
        Capability::Imap4Rev1,
        Capability::Enable,
        Capability::CondStore,
        Capability::QResync,
        Capability::Idle,
        Capability::UidPlus,
        Capability::Move,
        Capability::SpecialUse,
        Capability::ListExtended,
        Capability::Namespace,
        Capability::Unselect,
        Capability::Binary,
        Capability::Compress,
        Capability::Sort,
        Capability::Thread,
        Capability::ESearch,
        Capability::Id,
        Capability::LiteralPlus,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Imap4Rev1 => "IMAP4REV1",
            Self::Enable => "ENABLE",
            Self::CondStore => "CONDSTORE",
            Self::QResync => "QRESYNC",
            Self::Idle => "IDLE",
            Self::UidPlus => "UIDPLUS",
            Self::Move => "MOVE",
            Self::SpecialUse => "SPECIAL-USE",
            Self::ListExtended => "LIST-EXTENDED",
            Self::Namespace => "NAMESPACE",
            Self::Unselect => "UNSELECT",
            Self::Binary => "BINARY",
            Self::Compress => "COMPRESS=DEFLATE",
            Self::Sort => "SORT",
            Self::Thread => "THREAD",
            Self::ESearch => "ESEARCH",
            Self::Id => "ID",
            Self::LiteralPlus => "LITERAL+",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        let name = name.trim();
        Self::ALL
            .iter()
            .copied()
            .find(|capability| capability.as_str().eq_ignore_ascii_case(name))
    }
}

impl fmt::Display for Capability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Capabilities {
    known: BTreeSet<Capability>,
    unknown: BTreeSet<String>,
}

impl Capabilities {
    /// Empty only before a connection exists; an empty list after authentication is
    /// `EmptyCapabilities`.
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_names<I, S>(names: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut capabilities = Self::new();
        for name in names {
            capabilities.insert(name.as_ref());
        }
        capabilities
    }

    pub fn insert(&mut self, name: &str) {
        let name = name.trim();
        if name.is_empty() {
            return;
        }
        match Capability::from_name(name) {
            Some(capability) => {
                self.known.insert(capability);
            }
            None => {
                self.unknown.insert(name.to_owned());
            }
        }
    }

    pub fn contains(&self, capability: Capability) -> bool {
        self.known.contains(&capability)
    }

    pub fn has_name(&self, name: &str) -> bool {
        match Capability::from_name(name) {
            Some(capability) => self.contains(capability),
            None => self
                .unknown
                .iter()
                .any(|known| known.eq_ignore_ascii_case(name.trim())),
        }
    }

    pub fn require(&self, capability: Capability) -> BackendResult<()> {
        if self.contains(capability) {
            Ok(())
        } else {
            Err(BackendError::Unsupported { capability })
        }
    }

    pub fn is_empty(&self) -> bool {
        self.known.is_empty() && self.unknown.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = Capability> + '_ {
        self.known.iter().copied()
    }

    pub fn names(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .known
            .iter()
            .map(|capability| capability.as_str().to_owned())
            .chain(self.unknown.iter().cloned())
            .collect();
        names.sort();
        names
    }

    /// Needs both: CONDSTORE supplies `MODSEQ`, QRESYNC the vanished set.
    pub fn supports_incremental_sync(&self) -> bool {
        self.contains(Capability::CondStore) && self.contains(Capability::QResync)
    }
}

impl fmt::Display for Capabilities {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.names().join(" "))
    }
}

impl<S: AsRef<str>> FromIterator<S> for Capabilities {
    fn from_iter<I: IntoIterator<Item = S>>(iter: I) -> Self {
        Self::from_names(iter)
    }
}
