//! One JSON line per tool call: ids, counts and outcomes, never content (ADR 0010 Q6).

use std::io::Write as _;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Access {
    pub accounts: Vec<i64>,
    pub mailboxes: Vec<i64>,
    pub messages: Vec<i64>,
    pub drafts: Vec<i64>,
    pub labels: Vec<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Answered,
    Refused,
    Failed,
}

impl Outcome {
    fn as_str(self) -> &'static str {
        match self {
            Outcome::Answered => "answered",
            Outcome::Refused => "refused",
            Outcome::Failed => "failed",
        }
    }
}

/// Where the log is kept; `None` keeps none.
#[derive(Debug, Clone, Default)]
pub struct Audit {
    path: Option<PathBuf>,
}

impl Audit {
    pub fn to_file(path: PathBuf) -> Audit {
        Audit { path: Some(path) }
    }

    pub fn none() -> Audit {
        Audit { path: None }
    }

    pub fn installed() -> Audit {
        Audit::to_file(postio_session::paths::store_path().with_file_name("mcp-access.log"))
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// A log that cannot be written is reported in the process log and does not fail the call.
    pub fn record(&self, tool: &str, access: &Access, outcome: Outcome) {
        tracing::info!(
            tool,
            accounts = access.accounts.len(),
            mailboxes = access.mailboxes.len(),
            messages = access.messages.len(),
            drafts = access.drafts.len(),
            outcome = outcome.as_str(),
            "an agent called a tool"
        );
        let Some(path) = &self.path else {
            return;
        };
        let line = serde_json::json!({
            "at": chrono::Utc::now().to_rfc3339(),
            "tool": tool,
            "accounts": access.accounts,
            "mailboxes": access.mailboxes,
            "messages": access.messages,
            "drafts": access.drafts,
            "labels": access.labels,
            "outcome": outcome.as_str(),
        });
        let written = (|| {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)?;
            writeln!(file, "{line}")
        })();
        if let Err(error) = written {
            tracing::warn!(%error, "the agent access log could not be written: {error}");
        }
    }
}
