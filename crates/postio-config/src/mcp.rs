//! `[mcp]` — what an AI agent may read through `postio-mcp` (ADR 0010 Q6).
//!
//! Off by default and opt-in per account and per folder: an account with no
//! table here is invisible to every tool, `list_accounts` included, and an
//! account whose table names no folder shares nothing. The server reads this
//! section again on every tool call, so taking an account out takes effect
//! on the next call rather than the next restart.
//!
//! ```toml
//! [mcp.accounts."ada@example.com"]
//! mailboxes = ["INBOX", "Receipts"]   # folder paths; "*" shares every one
//! ```

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::Extras;

/// The `[mcp]` section.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct McpConfig {
    /// The accounts an agent may read, keyed by the account's address.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub accounts: BTreeMap<String, McpAccount>,
    /// Keys this version of Postio does not know, preserved verbatim.
    #[serde(flatten)]
    pub extra: Extras,
}

/// One shared account: `[mcp.accounts."<address>"]`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct McpAccount {
    /// The folders an agent may read, by their path on the server; `"*"`
    /// shares every folder. Empty shares nothing.
    #[serde(default)]
    pub mailboxes: Vec<String>,
    /// Keys this version of Postio does not know, preserved verbatim.
    #[serde(flatten)]
    pub extra: Extras,
}

#[cfg(test)]
mod tests {
    use crate::Config;

    #[test]
    fn nothing_is_shared_until_the_file_says_so() {
        let config = Config::from_toml_str("").expect("an empty file parses");
        assert!(config.mcp.accounts.is_empty());
    }

    #[test]
    fn an_account_is_shared_by_its_address_with_the_folders_named() {
        let config = Config::from_toml_str(
            "[mcp.accounts.\"ada@example.com\"]\nmailboxes = [\"INBOX\", \"Receipts\"]\n",
        )
        .expect("parses");
        let shared = &config.mcp.accounts["ada@example.com"];
        assert_eq!(shared.mailboxes, ["INBOX", "Receipts"]);
    }

    #[test]
    fn an_account_named_without_folders_shares_none() {
        let config = Config::from_toml_str("[mcp.accounts.\"ada@example.com\"]\n").expect("parses");
        assert!(config.mcp.accounts["ada@example.com"].mailboxes.is_empty());
    }

    #[test]
    fn an_unshared_file_writes_no_mcp_keys() {
        let text = toml::to_string(&Config::default()).expect("serialises");
        assert!(!text.contains("accounts"), "{text}");
    }
}
