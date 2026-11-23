//! What the person has shared: `[mcp]`, read again for every call so a
//! removal takes effect at once; anything unreadable shares nothing.

use std::path::Path;

use postio_config::McpConfig;

pub const EVERY_FOLDER: &str = "*";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Scope {
    accounts: Vec<(String, Folders, Actions)>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Actions {
    pub archive: bool,
    pub move_: bool,
    pub label: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Archive,
    Move,
    Label,
}

impl Action {
    pub fn name(self) -> &'static str {
        match self {
            Action::Archive => "archive",
            Action::Move => "move",
            Action::Label => "label",
        }
    }
}

impl Actions {
    /// The actions `names` lists; a name this build does not know allows
    /// nothing.
    pub fn from_names<'a>(names: impl IntoIterator<Item = &'a String>) -> Actions {
        let mut actions = Actions::default();
        for name in names {
            match name.trim().to_ascii_lowercase().as_str() {
                "archive" => actions.archive = true,
                "move" => actions.move_ = true,
                "label" => actions.label = true,
                _ => {}
            }
        }
        actions
    }

    pub fn allows(self, action: Action) -> bool {
        match action {
            Action::Archive => self.archive,
            Action::Move => self.move_,
            Action::Label => self.label,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Folders {
    Every,
    Only(Vec<String>),
}

impl Folders {
    /// Paths are compared as written, except `INBOX`, which IMAP names case-insensitively.
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
                Some((
                    address.trim().to_ascii_lowercase(),
                    folders,
                    Actions::from_names(&shared.actions),
                ))
            })
            .collect();
        Scope { accounts }
    }

    pub fn load(path: Option<&Path>) -> Scope {
        let Some(path) = path else {
            return Scope::none();
        };
        match postio_config::Config::load_from_path(path) {
            Ok(config) => Scope::from_config(&config.mcp),
            Err(_) => {
                // Not the error itself: a parse error quotes the line, and the
                // line here may be an address.
                tracing::warn!("config.toml did not load; sharing nothing with the agent");
                Scope::none()
            }
        }
    }

    pub fn is_empty(&self) -> bool {
        self.accounts.is_empty()
    }

    /// The folders shared of the account at `address`, or `None` when the
    /// account is not shared. Addresses compare without regard to case.
    pub fn account(&self, address: &str) -> Option<&Folders> {
        let address = address.trim().to_ascii_lowercase();
        self.accounts
            .iter()
            .find(|(shared, ..)| *shared == address)
            .map(|(_, folders, _)| folders)
    }

    pub fn actions(&self, address: &str) -> Actions {
        let address = address.trim().to_ascii_lowercase();
        self.accounts
            .iter()
            .find(|(shared, ..)| *shared == address)
            .map(|(.., actions)| *actions)
            .unwrap_or_default()
    }

    pub fn anywhere(&self, action: Action) -> bool {
        self.accounts
            .iter()
            .any(|(.., actions)| actions.allows(action))
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
        let scope =
            scope("[mcp.accounts.\"ada@example.com\"]\nmailboxes = [\"inbox\", \"Receipts\"]\n");
        let folders = scope.account("ada@example.com").expect("shared");
        assert!(folders.allows("INBOX"));
        assert!(folders.allows("Receipts"));
        assert!(!folders.allows("receipts"));
        assert!(!folders.allows("Archive"));
        assert!(!folders.allows("Receipts/2026"));
    }

    #[test]
    fn actions_are_none_until_named_and_only_those_named() {
        let none = scope("[mcp.accounts.\"ada@example.com\"]\nmailboxes = [\"*\"]\n");
        assert_eq!(none.actions("ada@example.com"), Actions::default());
        assert!(!none.anywhere(Action::Archive));

        let some = scope(
            "[mcp.accounts.\"ada@example.com\"]\nmailboxes = [\"*\"]\n\
             actions = [\"Archive\", \"label\", \"delete\"]\n",
        );
        let actions = some.actions("ada@example.com");
        assert!(actions.allows(Action::Archive));
        assert!(actions.allows(Action::Label));
        assert!(!actions.allows(Action::Move));
        assert_eq!(some.actions("grace@example.org"), Actions::default());
    }

    #[test]
    fn actions_on_an_account_that_shares_no_folder_allow_nothing() {
        let scope = scope("[mcp.accounts.\"ada@example.com\"]\nactions = [\"archive\"]\n");
        assert!(!scope.anywhere(Action::Archive));
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
