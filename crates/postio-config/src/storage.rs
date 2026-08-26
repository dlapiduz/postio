//! `[storage]` — how much disk the local store may use.
//!
//! ```toml
//! [storage]
//! max_bytes = 20_000_000_000   # omit for no limit
//! ```
//!
//! # Why there is a ceiling at all
//!
//! ADR 0016 backfills every message's text by default, so the store holds a
//! complete replica of a mailbox rather than a recent slice of it. On the
//! account ADR 0017 measured that is around 1.4 GB of text — and attachment
//! payloads, fetched on demand, accumulate on top of it with nothing bounding
//! them. Postio had no upper bound of any kind on what it would write to
//! somebody's disk.
//!
//! # Why exceeding it is safe
//!
//! ADR 0014 puts it plainly: everything except drafts and the operation queue
//! can be re-synced. So the store is a cache, and a cache may evict. What
//! eviction takes is only ever *refetchable* — raw message source first, then
//! attachment payloads, oldest mail first — and never the message text that
//! search is made of. Passing the ceiling costs a round trip later; it never
//! costs mail.
//!
//! # Why the default is no limit
//!
//! A number here is a promise about somebody else's disk, and Postio does not
//! know how big theirs is. A default that binds surprises the user by throwing
//! away an attachment they wanted offline; a default set high enough never to
//! bind is decoration. Neither is worth the confusion, and ADR 0016's whole
//! posture is that Postio downloads what it needs and says so.
//!
//! So: unset means unbounded, the mechanism is one line away, and the surface
//! that tells a user what their mail actually costs is #383's.

use serde::{Deserialize, Serialize};

use crate::Extras;

/// The `[storage]` section.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct StorageConfig {
    /// Ceiling on the blob store, in bytes. `None` means unbounded.
    ///
    /// See the [module documentation](self) for why unbounded is the default
    /// and why exceeding this is safe.
    #[serde(default)]
    pub max_bytes: Option<u64>,
    /// Keys in `[storage]` this version of Postio does not know.
    #[serde(flatten)]
    pub extras: Extras,
}

impl StorageConfig {
    /// Whether a store of `bytes` is over the ceiling.
    ///
    /// `false` when no ceiling is set, which is the default.
    pub fn is_over(&self, bytes: u64) -> bool {
        self.max_bytes.is_some_and(|budget| bytes > budget)
    }
}

#[cfg(test)]
mod tests {
    use crate::Config;

    #[test]
    fn no_storage_section_means_no_ceiling() {
        // The default, and the one every existing `config.toml` gets. A
        // ceiling nobody asked for would start deleting attachments somebody
        // downloaded on purpose.
        let config: Config = toml::from_str("").expect("parse");
        assert_eq!(config.storage.max_bytes, None);
        assert!(!config.storage.is_over(u64::MAX));
    }

    #[test]
    fn a_ceiling_is_read_and_compared_against() {
        let config: Config = toml::from_str("[storage]\nmax_bytes = 1000\n").expect("parse");
        assert_eq!(config.storage.max_bytes, Some(1000));
        assert!(
            !config.storage.is_over(1000),
            "at the ceiling is not over it"
        );
        assert!(config.storage.is_over(1001));
    }

    #[test]
    fn an_unknown_storage_key_survives_a_round_trip() {
        // The rule the whole config obeys: a key this build does not know is
        // a key a newer build might, and dropping it on save would silently
        // undo somebody's setting.
        let config: Config =
            toml::from_str("[storage]\nmax_bytes = 1000\nevict_when = \"never\"\n").expect("parse");
        let written = toml::to_string(&config).expect("serialize");
        assert!(written.contains("evict_when"));
    }
}
