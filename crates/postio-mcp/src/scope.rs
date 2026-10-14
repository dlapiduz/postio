//! What the person has shared: `[mcp]`, as the tools ask it.
//!
//! Read again for every call ([`Scope::load`]), so taking an account or a
//! folder out of `config.toml` takes effect on the next call (ADR 0010 Q6).
//! Everything not named is invisible, and anything that cannot be read --
//! a missing file, one that does not parse -- shares nothing.

use std::path::Path;

use postio_config::McpConfig;

/// Every folder of an account, as `[mcp]` spells it.
pub const EVERY_FOLDER: &str = "*";

/// The accounts and folders an agent may read.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Scope {
    accounts: Vec<(String, Folders)>,
}

/// Which of an account's folders are shared.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Folders {
    /// All of them.
    Every,
    /// Only these paths.
    Only(Vec<String>),
}

impl Folders {
    /// Whether the folder at `path` is shared.
    ///
    /// Paths are compared as written, except `INBOX`, which IMAP names
    /// case-insensitively (RFC 3501 §5.1).
    pub fn allows(&self, path: &str) -> bool {
        match self {
            Folders::Every => true,
            Folders::Only(paths) => paths.iter().any(|shared| {
                shared == path
                    || (shared.eq_ignore_ascii_case("INBOX") && path.eq_ignore_ascii_case("INBOX"))
            }),
        }
    }
}

impl Scope {
    /// Nothing shared.
    pub fn none() -> Scope {
        Scope::default()
    }

    /// What `config` shares. An account whose table names no folder is left
    /// out: it shares nothing, so it is not shown either.
    pub fn from_config(config: &McpConfig) -> Scope {
        let accounts = config
            .accounts
            .iter()
            .filter_map(|(address, shared)| {
                let paths: Vec<String> = shared
                    .mailboxes
                    .iter()
                    .map(|path| path.trim().to_owned())
                    .filter(|path| !path.is_empty())
                    .collect();
                if paths.is_empty() {
                    return None;
                }
                let folders = if paths.iter().any(|path| path == EVERY_FOLDER) {
                    Folders::Every
                } else {
                    Folders::Only(paths)
                };
                Some((address.trim().to_ascii_lowercase(), folders))
            })
            .collect();
        Scope { accounts }
    }

    /// What the file at `path` shares now; nothing when there is no file or
    /// it does not parse.
    pub fn load(path: Option<&Path>) -> Scope {
        let Some(path) = path else {
            return Scope::none();
        };
        match postio_config::Config::load_from_path(path) {
            Ok(config) => Scope::from_config(&config.mcp),
            Err(error) => {
                tracing::warn!(%error, "config.toml did not load; sharing nothing with the agent");
                Scope::none()
            }
        }
    }

    /// Whether nothing is shared at all.
    pub fn is_empty(&self) -> bool {
        self.accounts.is_empty()
    }

    /// The folders shared of the account at `address`, or `None` when the
    /// account is not shared. Addresses compare without regard to case.
    pub fn account(&self, address: &str) -> Option<&Folders> {
        let address = address.trim().to_ascii_lowercase();
        self.accounts
            .iter()
            .find(|(shared, _)| *shared == address)
            .map(|(_, folders)| folders)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scope(toml: &str) -> Scope {
        Scope::from_config(
            &postio_config::Config::from_toml_str(toml)
                .expect("parses")
                .mcp,
        )
    }

    #[test]
    fn an_empty_file_shares_nothing() {
        let scope = scope("");
        assert!(scope.is_empty());
        assert_eq!(scope.account("ada@example.com"), None);
    }

    #[test]
    fn only_the_named_account_is_shared() {
        let scope = scope("[mcp.accounts.\"ada@example.com\"]\nmailboxes = [\"INBOX\"]\n");
        assert!(scope.account("ada@example.com").is_some());
        assert!(scope.account("Ada@Example.com").is_some());
        assert_eq!(scope.account("grace@example.org"), None);
    }

    #[test]
    fn an_account_without_folders_is_not_shared() {
        let scope = scope("[mcp.accounts.\"ada@example.com\"]\nmailboxes = []\n");
        assert!(scope.is_empty());
        assert_eq!(scope.account("ada@example.com"), None);
    }

    #[test]
    fn only_the_named_folders_are_shared() {
        let scope = scope("[mcp.accounts.\"ada@example.com\"]\nmailboxes = [\"inbox\", \"Receipts\"]\n");
        let folders = scope.account("ada@example.com").expect("shared");
        assert!(folders.allows("INBOX"));
        assert!(folders.allows("Receipts"));
        assert!(!folders.allows("receipts"));
        assert!(!folders.allows("Archive"));
        assert!(!folders.allows("Receipts/2026"));
    }

    #[test]
    fn a_star_shares_every_folder() {
        let scope = scope("[mcp.accounts.\"ada@example.com\"]\nmailboxes = [\"*\"]\n");
        let folders = scope.account("ada@example.com").expect("shared");
        assert!(folders.allows("Archive"));
        assert!(folders.allows("Anything/at all"));
    }

    #[test]
    fn a_file_that_does_not_parse_shares_nothing() {
        let directory = tempfile::tempdir().expect("a directory");
        let path = directory.path().join("config.toml");
        std::fs::write(
            &path,
            "[mcp.accounts.\"ada@example.com\"]\nmailboxes = [\"*\"\n",
        )
        .expect("written");
        assert!(Scope::load(Some(&path)).is_empty());
    }

    #[test]
    fn a_missing_file_shares_nothing() {
        let directory = tempfile::tempdir().expect("a directory");
        assert!(Scope::load(Some(&directory.path().join("config.toml"))).is_empty());
        assert!(Scope::load(None).is_empty());
    }
}
