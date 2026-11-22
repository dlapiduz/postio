//! `[storage]`: a disk ceiling on the blob store. Exceeding it is safe: only refetchable data is
//! evicted
//! (raw source, then attachment payloads, oldest first), never message text (ADR 0014, 0016).
//! Unset means unbounded: a default that binds would throw away attachments someone downloaded on
//! purpose.

use serde::{Deserialize, Serialize};

use crate::Extras;

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct StorageConfig {
    /// Ceiling on the blob store, in bytes. `None` means unbounded: a default that binds would
    /// throw away
    /// attachments someone downloaded on purpose.
    #[serde(default)]
    pub max_bytes: Option<u64>,
    #[serde(flatten)]
    pub extras: Extras,
}

impl StorageConfig {
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
        let config: Config =
            toml::from_str("[storage]\nmax_bytes = 1000\nevict_when = \"never\"\n").expect("parse");
        let written = toml::to_string(&config).expect("serialize");
        assert!(written.contains("evict_when"));
    }
}
