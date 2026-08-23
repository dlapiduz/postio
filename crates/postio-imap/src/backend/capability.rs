//! What a server can do, as reported by the server.
//!
//! Postio never assumes an extension is present. Everything that depends on
//! one — QRESYNC resync, `MOVE`, `APPENDUID`, `IDLE` — is gated on this set,
//! and this set comes from the capability list read **after** authentication.
//! iCloud does not advertise CONDSTORE, QRESYNC, UIDPLUS or IDLE in its
//! pre-auth banner; see ADR 0001, Q3.

use std::collections::BTreeSet;
use std::fmt;

use super::{BackendError, BackendResult};

/// An IMAP extension Postio knows how to use.
///
/// Anything the server advertises that is not in this list is kept verbatim in
/// [`Capabilities`] rather than discarded — a name we do not model today is
/// still evidence in a bug report.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum Capability {
    /// `IMAP4REV1` — the base protocol.
    Imap4Rev1,
    /// `ENABLE` (RFC 5161) — the prerequisite for CONDSTORE and QRESYNC.
    Enable,
    /// `CONDSTORE` (RFC 7162) — per-message `MODSEQ`.
    CondStore,
    /// `QRESYNC` (RFC 7162) — `SELECT` returns changes *and* vanishes.
    QResync,
    /// `IDLE` (RFC 2177) — the server pushes instead of us polling.
    Idle,
    /// `UIDPLUS` (RFC 4315) — `APPENDUID`/`COPYUID`, so an appended message
    /// can be recognized without a search.
    UidPlus,
    /// `MOVE` (RFC 6851) — one round trip instead of copy + store + expunge.
    Move,
    /// `SPECIAL-USE` (RFC 6154) — the server names its own archive and trash.
    SpecialUse,
    /// `LIST-EXTENDED` (RFC 5258) — subscription state in one `LIST`.
    ListExtended,
    /// `NAMESPACE` (RFC 2342).
    Namespace,
    /// `UNSELECT` (RFC 3691) — leave a mailbox without expunging it.
    Unselect,
    /// `BINARY` (RFC 3516) — server-side decoding of `base64` parts.
    Binary,
    /// `COMPRESS=DEFLATE` (RFC 4978).
    Compress,
    /// `SORT` (RFC 5256).
    Sort,
    /// `THREAD` (RFC 5256).
    Thread,
    /// `ESEARCH` (RFC 4731).
    ESearch,
    /// `ID` (RFC 2971).
    Id,
    /// `LITERAL+` (RFC 7888) — send a literal without waiting for the
    /// continuation.
    LiteralPlus,
}

impl Capability {
    /// Every capability Postio models, in declaration order.
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

    /// The name as it appears on the wire, upper case.
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

    /// Looks a capability up by wire name, ignoring case.
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

/// The capability set a server reported.
///
/// Names Postio does not model are kept in a second set rather than thrown
/// away, so [`names`](Self::names) round-trips what the server actually said.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Capabilities {
    known: BTreeSet<Capability>,
    unknown: BTreeSet<String>,
}

impl Capabilities {
    /// An empty set.
    ///
    /// Only ever legitimate before a connection exists: a server that
    /// authenticated us and then advertised nothing is a bug, not a server
    /// with no features. See [`BackendError::EmptyCapabilities`].
    pub fn new() -> Self {
        Self::default()
    }

    /// Builds a set from the names a server advertised.
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

    /// Records one advertised name.
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

    /// Whether the server advertised `capability`.
    pub fn contains(&self, capability: Capability) -> bool {
        self.known.contains(&capability)
    }

    /// Whether the server advertised `name`, modelled or not.
    pub fn has_name(&self, name: &str) -> bool {
        match Capability::from_name(name) {
            Some(capability) => self.contains(capability),
            None => self
                .unknown
                .iter()
                .any(|known| known.eq_ignore_ascii_case(name.trim())),
        }
    }

    /// Fails with [`BackendError::Unsupported`] when `capability` is absent.
    ///
    /// The point of a named error rather than a silent fallback is that "the
    /// server cannot do this" and "we forgot to ask" look identical in a log
    /// otherwise.
    pub fn require(&self, capability: Capability) -> BackendResult<()> {
        if self.contains(capability) {
            Ok(())
        } else {
            Err(BackendError::Unsupported { capability })
        }
    }

    /// Whether nothing at all was advertised.
    pub fn is_empty(&self) -> bool {
        self.known.is_empty() && self.unknown.is_empty()
    }

    /// The modelled capabilities, in declaration order.
    pub fn iter(&self) -> impl Iterator<Item = Capability> + '_ {
        self.known.iter().copied()
    }

    /// Every advertised name, modelled or not, sorted — for logs and for the
    /// live capability assertion test.
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

    /// Whether this server can do QRESYNC-based incremental sync.
    ///
    /// Both halves are needed: CONDSTORE supplies `MODSEQ` and QRESYNC
    /// supplies the vanished set. With only one of them, the sync engine has
    /// to fall back to comparing full UID listings.
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
