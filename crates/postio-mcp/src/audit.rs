//! The access log: what each tool call read and wrote (ADR 0010 Q6).
//!
//! One JSON object per line, appended: when, which tool, the accounts,
//! folders, messages and drafts it touched, by id, and how it ended. Ids,
//! counts and outcomes, never content -- not a subject, not an address, not
//! the words of a search (ARCHITECTURE.md §11). That is enough to answer
//! "what did it read", and nothing in it is mail.

use std::io::Write as _;
use std::path::{Path, PathBuf};

/// What one call touched, by id.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Access {
    /// Accounts.
    pub accounts: Vec<i64>,
    /// Folders.
    pub mailboxes: Vec<i64>,
    /// Messages returned.
    pub messages: Vec<i64>,
    /// Drafts written.
    pub drafts: Vec<i64>,
}

/// How a call ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// It answered.
    Answered,
    /// It asked for something not shared, or something that is not there.
    Refused,
    /// The store could not be reached or read.
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
    /// Append to the file at `path`, creating it and its folder.
    pub fn to_file(path: PathBuf) -> Audit {
        Audit { path: Some(path) }
    }

    /// Keep no log.
    pub fn none() -> Audit {
        Audit { path: None }
    }

    /// Where this installation keeps it: beside the store, as
    /// `mcp-access.log`.
    pub fn installed() -> Audit {
        Audit::to_file(postio_session::paths::store_path().with_file_name("mcp-access.log"))
    }

    /// The file, when there is one.
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// Record one call. A log that cannot be written is said in the
    /// process's own log and does not fail the call.
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
