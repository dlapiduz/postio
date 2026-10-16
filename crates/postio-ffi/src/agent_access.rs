//! Agent access across the boundary (ADR 0010 Q6, #1832): who may read what,
//! what the agents did, and taking it back.
//!
//! The decisions are `postio_ui::agent_access`'s and the file edits are
//! `postio_config`'s; this carries finished values. Swift parses no TOML and
//! no log line (ADR 0031).

use postio_config::Config;
use postio_ui::agent_access as words;

use crate::settings::SettingsError;

/// One account an agent may read.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SharedAgentAccountFfi {
    /// The address, and what revoking names.
    pub address: String,
    /// Which folders: `All folders`, `INBOX, Receipts`, `No folders`.
    pub folders: String,
    /// `Read and draft only`, or `Also archive and label`.
    pub abilities: String,
}

/// One recent call, already a sentence.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct AgentCallFfi {
    /// `5 min ago`.
    pub when: String,
    /// `Read 1 message "Receipt" in INBOX · ada@example.com`.
    pub line: String,
    /// Whether the call was turned away or failed, for the row's emphasis.
    pub refused: bool,
}

/// The activity list.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct AgentActivityFfi {
    /// `12 calls · last 5 min ago`, or that nothing has happened.
    pub summary: String,
    /// Newest first.
    pub calls: Vec<AgentCallFfi>,
}

/// The pane's fixed words.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct AgentWordsFfi {
    /// Heading over the shared accounts.
    pub shared_heading: String,
    /// Said when nothing is shared.
    pub nothing_shared: String,
    /// Heading over the activity.
    pub activity_heading: String,
    /// What revoking does and does not do.
    pub revoke_note: String,
    /// The per-account button.
    pub revoke_one: String,
    /// The revoke-everything button.
    pub revoke_all: String,
}

/// The pane's fixed words.
#[uniffi::export]
pub fn settings_agent_words() -> AgentWordsFfi {
    AgentWordsFfi {
        shared_heading: words::SHARED_HEADING.to_owned(),
        nothing_shared: words::NOTHING_SHARED.to_owned(),
        activity_heading: words::ACTIVITY_HEADING.to_owned(),
        revoke_note: words::REVOKE_NOTE.to_owned(),
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

/// `text` with nothing shared.
#[uniffi::export]
pub fn settings_patch_revoke_all_agents(text: String) -> Result<String, SettingsError> {
    postio_config::revoke_all_mcp(&text).map_err(|err| SettingsError::Invalid {
        message: err.to_string(),
    })
}

/// The recent calls in `log`, with ids resolved by `names`.
pub(crate) fn activity(
    names: &words::Names,
    calls: &[words::AgentCall],
    now: chrono::DateTime<chrono::Utc>,
) -> AgentActivityFfi {
    AgentActivityFfi {
        summary: words::activity_summary(calls, now),
        calls: calls
            .iter()
            .map(|call| AgentCallFfi {
                when: words::when_label(call.at, now),
                line: words::call_line(call, names),
                refused: call.outcome != words::CallOutcome::Answered,
            })
            .collect(),
    }
}
