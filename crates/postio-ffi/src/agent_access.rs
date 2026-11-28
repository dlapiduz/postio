//! Agent access across the boundary: finished values, so Swift parses no TOML and no log line (ADR
//! 0031).

use postio_config::Config;
use postio_ui::agent_access as words;

use crate::settings::SettingsError;

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SharedAgentAccountFfi {
    pub address: String,
    /// Which folders: `All folders`, `INBOX, Receipts`, `No folders`.
    pub folders: String,
    /// `Read and draft only`, or `Also archive and label`.
    pub abilities: String,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct AgentCallFfi {
    /// `5 min ago`.
    pub when: String,
    /// `Read 1 message "Receipt" in INBOX · ada@example.com`.
    pub line: String,
    pub refused: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct AgentActivityFfi {
    /// `12 calls · last 5 min ago`, or that nothing has happened.
    pub summary: String,
    /// Newest first.
    pub calls: Vec<AgentCallFfi>,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct AgentWordsFfi {
    pub shared_heading: String,
    pub nothing_shared: String,
    pub activity_heading: String,
    pub revoke_note: String,
    pub entry_heading: String,
    pub revoke_one: String,
    pub revoke_all: String,
}

#[uniffi::export]
pub fn settings_agent_words() -> AgentWordsFfi {
    AgentWordsFfi {
        shared_heading: words::SHARED_HEADING.to_owned(),
        nothing_shared: words::NOTHING_SHARED.to_owned(),
        activity_heading: words::ACTIVITY_HEADING.to_owned(),
        revoke_note: words::REVOKE_NOTE.to_owned(),
        entry_heading: words::ENTRY_HEADING.to_owned(),
        revoke_one: words::REVOKE_ONE.to_owned(),
        revoke_all: words::REVOKE_ALL.to_owned(),
    }
}

/// The accounts `[mcp]` shares, or `None` when the file will not parse (the
/// pane disables itself rather than show a guess, ADR 0029).
#[uniffi::export]
pub fn settings_agent_sharing(text: String) -> Option<Vec<SharedAgentAccountFfi>> {
    let config = Config::from_toml_str(&text).ok()?;
    Some(
        words::shared_accounts(&config.mcp)
            .into_iter()
            .map(|row| SharedAgentAccountFfi {
                address: row.address,
                folders: row.folders,
                abilities: row.abilities,
            })
            .collect(),
    )
}

/// `text` with `address` no longer shared. Takes effect on the agent's next
/// call: the server reads `[mcp]` again on each one.
#[uniffi::export]
pub fn settings_patch_revoke_agent(text: String, address: String) -> Result<String, SettingsError> {
    postio_config::revoke_mcp_account(&text, &address).map_err(|err| SettingsError::Invalid {
        message: err.to_string(),
    })
}

#[uniffi::export]
pub fn settings_patch_revoke_all_agents(text: String) -> Result<String, SettingsError> {
    postio_config::revoke_all_mcp(&text).map_err(|err| SettingsError::Invalid {
        message: err.to_string(),
    })
}

pub(crate) fn from_activity(shown: words::Activity) -> AgentActivityFfi {
    AgentActivityFfi {
        summary: shown.summary,
        calls: shown
            .calls
            .into_iter()
            .map(|row| AgentCallFfi {
                when: row.when,
                line: row.line,
                refused: row.refused,
            })
            .collect(),
    }
}
