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
//! actions = ["archive", "move", "label"]   # optional; none by default
//! ```

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{Extras, Result};

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
    /// What an agent may do to the shared mail besides read it and write
    /// drafts: any of `"archive"`, `"move"` and `"label"`. Empty -- the
    /// default -- allows none of them.
    #[serde(default)]
    pub actions: Vec<String>,
    /// Keys this version of Postio does not know, preserved verbatim.
    #[serde(flatten)]
    pub extra: Extras,
}

/// `text` with the sharing of `address` taken out: its
/// `[mcp.accounts."<address>"]` table and nothing else, so every comment and
/// every other key stays where the person put it. An address that is not
/// shared leaves the text as it was. A table left empty by the removal goes
/// with it (ADR 0010 Q6: revoking takes effect on the next call, because the
/// server reads this section again on every call).
pub fn revoke_mcp_account(text: &str, address: &str) -> Result<String> {
    let mut doc = crate::splice::parse(text)?;
    if let Some(accounts) = doc
        .get_mut("mcp")
        .and_then(|mcp| mcp.get_mut("accounts"))
        .and_then(|accounts| accounts.as_table_like_mut())
    {
        accounts.remove(address);
    }
    Ok(tidy(doc).to_string())
}

/// `text` with every account's sharing taken out: the agent-access kill
/// switch. Keys under `[mcp]` this version does not know are kept.
pub fn revoke_all_mcp(text: &str) -> Result<String> {
    let mut doc = crate::splice::parse(text)?;
    if let Some(mcp) = doc.get_mut("mcp").and_then(|mcp| mcp.as_table_like_mut()) {
        mcp.remove("accounts");
    }
    Ok(tidy(doc).to_string())
}

/// Drop `[mcp.accounts]` and `[mcp]` when a removal left them empty, so a
/// file that shared one account and now shares none reads as it did before
/// anything was shared.
fn tidy(mut doc: toml_edit::DocumentMut) -> toml_edit::DocumentMut {
    let accounts_empty = doc
        .get("mcp")
        .and_then(|mcp| mcp.get("accounts"))
        .and_then(|accounts| accounts.as_table_like())
        .is_some_and(|accounts| accounts.is_empty());
    if accounts_empty && let Some(mcp) = doc.get_mut("mcp").and_then(|mcp| mcp.as_table_like_mut())
    {
        mcp.remove("accounts");
    }
    let mcp_empty = doc
        .get("mcp")
        .and_then(|mcp| mcp.as_table_like())
        .is_some_and(|mcp| mcp.is_empty());
    if mcp_empty {
        doc.as_table_mut().remove("mcp");
    }
    doc
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
    fn an_account_allows_no_actions_until_it_names_them() {
        let config =
            Config::from_toml_str("[mcp.accounts.\"ada@example.com\"]\nmailboxes = [\"*\"]\n")
                .expect("parses");
        assert!(config.mcp.accounts["ada@example.com"].actions.is_empty());
        let config = Config::from_toml_str(
            "[mcp.accounts.\"ada@example.com\"]\nmailboxes = [\"*\"]\nactions = [\"archive\", \"label\"]\n",
        )
        .expect("parses");
        assert_eq!(
            config.mcp.accounts["ada@example.com"].actions,
            ["archive", "label"]
        );
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

    // -- Revoking (#1832) ----------------------------------------------------

    use super::{revoke_all_mcp, revoke_mcp_account};

    const SHARED: &str = "\
# my settings
[ui]
theme = \"dark\"

[mcp.accounts.\"ada@example.com\"]
mailboxes = [\"INBOX\"]

[mcp.accounts.\"grace@example.com\"]
mailboxes = [\"*\"]
actions = [\"archive\"]
";

    #[test]
    fn revoking_one_account_leaves_the_other_shared_and_the_file_alone() {
        let after = revoke_mcp_account(SHARED, "ada@example.com").expect("revokes");
        let config = Config::from_toml_str(&after).expect("still parses");
        assert!(
            !config.mcp.accounts.contains_key("ada@example.com"),
            "{after}"
        );
        assert_eq!(
            config.mcp.accounts["grace@example.com"].actions,
            ["archive"]
        );
        assert!(
            after.contains("# my settings") && after.contains("theme = \"dark\""),
            "{after}"
        );
    }

    #[test]
    fn revoking_the_last_account_removes_the_mcp_table_altogether() {
        let one =
            "[ui]\ntheme = \"dark\"\n\n[mcp.accounts.\"ada@example.com\"]\nmailboxes = [\"*\"]\n";
        let after = revoke_mcp_account(one, "ada@example.com").expect("revokes");
        assert!(!after.contains("mcp"), "{after}");
        assert!(after.contains("theme = \"dark\""), "{after}");
    }

    #[test]
    fn revoking_an_account_that_is_not_shared_changes_nothing() {
        let after = revoke_mcp_account(SHARED, "nobody@example.com").expect("revokes");
        assert_eq!(after, SHARED);
        assert_eq!(revoke_mcp_account("", "ada@example.com").expect("ok"), "");
    }

    #[test]
    fn revoking_everything_shares_nothing_and_keeps_unknown_mcp_keys() {
        let text = format!(
            "[mcp]\nfuture = 1\n\n{}",
            &SHARED[SHARED.find("[mcp.accounts").unwrap()..]
        );
        let after = revoke_all_mcp(&text).expect("revokes");
        let config = Config::from_toml_str(&after).expect("parses");
        assert!(config.mcp.accounts.is_empty(), "{after}");
        assert!(after.contains("future = 1"), "{after}");
    }

    #[test]
    fn a_file_that_will_not_parse_is_refused_rather_than_rewritten() {
        assert!(revoke_mcp_account("[mcp", "ada@example.com").is_err());
        assert!(revoke_all_mcp("[mcp").is_err());
    }
}
