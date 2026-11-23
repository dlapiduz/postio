//! Agent access as Settings shows it: shared accounts from `[mcp]` and the access log (ADR 0010
//! Q6).
//! Revoking only removes the account's table; a call in flight finishes, the next sees it gone.
//! The log holds ids only, resolved by the caller (ARCHITECTURE.md §11).

use std::collections::{BTreeSet, HashMap};

use chrono::{DateTime, Utc};
use postio_config::McpConfig;
use serde_json::Value;

pub const TITLE: &str = "Agents";

pub const DESCRIPTION: &str =
    "Which accounts an AI agent may read through postio-mcp, and what it has done.";

pub const STORED_IN: &str = "[mcp] in config.toml · read again on every agent call";

pub const SHARED_HEADING: &str = "Shared with agents";

pub const ACTIVITY_HEADING: &str = "Recent activity";

pub const NOTHING_SHARED: &str =
    "No agent can see any account. Sharing is off until you turn it on in config.toml.";

pub const NO_ACTIVITY: &str = "No agent has used Postio yet.";

pub const REVOKE_NOTE: &str =
    "Revoking takes effect on the agent's next call. A call already running finishes.";

pub const ENTRY_HEADING: &str = "Point your agent at";

pub const REVOKE_ONE: &str = "Revoke";

pub const REVOKE_ALL: &str = "Revoke all";

pub const RECENT: usize = 50;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SharedAccount {
    pub address: String,
    pub folders: String,
    pub abilities: String,
}

pub fn shared_accounts(config: &McpConfig) -> Vec<SharedAccount> {
    config
        .accounts
        .iter()
        .map(|(address, shared)| {
            let folders = if shared.mailboxes.iter().any(|folder| folder == "*") {
                "All folders".to_owned()
            } else if shared.mailboxes.is_empty() {
                "No folders".to_owned()
            } else {
                shared.mailboxes.join(", ")
            };
            let actions: Vec<&str> = ["archive", "move", "label"]
                .into_iter()
                .filter(|name| shared.actions.iter().any(|action| action == name))
                .collect();
            let abilities = match actions.as_slice() {
                [] => "Read and draft only".to_owned(),
                [only] => format!("Also {only}"),
                [rest @ .., last] => format!("Also {} and {last}", rest.join(", ")),
            };
            SharedAccount {
                address: address.clone(),
                folders,
                abilities,
            }
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallOutcome {
    Answered,
    Refused,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentCall {
    pub at: DateTime<Utc>,
    pub tool: String,
    pub accounts: Vec<i64>,
    pub mailboxes: Vec<i64>,
    pub messages: Vec<i64>,
    pub drafts: Vec<i64>,
    pub labels: Vec<i64>,
    pub outcome: CallOutcome,
}

/// Skips lines that are not calls (torn write, future format): the log is append-only and
/// read while it is written.
pub fn parse_log(log: &str, limit: usize) -> Vec<AgentCall> {
    log.lines()
        .rev()
        .filter_map(parse_line)
        .take(limit)
        .collect()
}

fn parse_line(line: &str) -> Option<AgentCall> {
    let value: Value = serde_json::from_str(line).ok()?;
    let ids = |key: &str| -> Vec<i64> {
        value
            .get(key)
            .and_then(Value::as_array)
            .map(|ids| ids.iter().filter_map(Value::as_i64).collect())
            .unwrap_or_default()
    };
    Some(AgentCall {
        at: DateTime::parse_from_rfc3339(value.get("at")?.as_str()?)
            .ok()?
            .with_timezone(&Utc),
        tool: value.get("tool")?.as_str()?.to_owned(),
        accounts: ids("accounts"),
        mailboxes: ids("mailboxes"),
        messages: ids("messages"),
        drafts: ids("drafts"),
        labels: ids("labels"),
        outcome: match value.get("outcome")?.as_str()? {
            "answered" => CallOutcome::Answered,
            "refused" => CallOutcome::Refused,
            _ => CallOutcome::Failed,
        },
    })
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Wanted {
    pub accounts: BTreeSet<i64>,
    pub mailboxes: BTreeSet<i64>,
    /// Message ids. Only a call that touched one message names it.
    pub messages: BTreeSet<i64>,
}

pub fn wanted(calls: &[AgentCall]) -> Wanted {
    let mut wanted = Wanted::default();
    for call in calls {
        wanted.accounts.extend(&call.accounts);
        wanted.mailboxes.extend(&call.mailboxes);
        if let [only] = call.messages.as_slice() {
            wanted.messages.insert(*only);
        }
    }
    wanted
}

/// What the ids resolved to. Missing entries are ids that no longer exist.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Names {
    pub accounts: HashMap<i64, String>,
    pub mailboxes: HashMap<i64, String>,
    pub messages: HashMap<i64, String>,
}

/// What a tool did, as a verb phrase. A call that did not answer did
/// nothing, so it says what was tried instead of what was done.
fn verb(tool: &str, call: &AgentCall) -> String {
    if call.outcome != CallOutcome::Answered {
        return format!("Tried to {}", attempt(tool));
    }
    let n = call.messages.len();
    let messages = |n: usize| match n {
        0 => "no messages".to_owned(),
        1 => "1 message".to_owned(),
        n => format!("{n} messages"),
    };
    match tool {
        "list_accounts" => "Listed the shared accounts".to_owned(),
        "list_mailboxes" => "Listed folders".to_owned(),
        "search_mail" => format!("Searched mail, {} found", messages(n)),
        "read_message" => format!("Read {}", messages(n)),
        "read_thread" => format!("Read a thread of {}", messages(n)),
        "create_draft" => "Wrote a draft".to_owned(),
        "archive_messages" => format!("Archived {}", messages(n)),
        "move_messages" => format!("Moved {}", messages(n)),
        "add_label" => format!("Labelled {}", messages(n)),
        "remove_label" => format!("Unlabelled {}", messages(n)),
        other => format!("Called {other}"),
    }
}

fn attempt(tool: &str) -> String {
    match tool {
        "list_accounts" => "list the shared accounts".to_owned(),
        "list_mailboxes" => "list folders".to_owned(),
        "search_mail" => "search mail".to_owned(),
        "read_message" => "read a message".to_owned(),
        "read_thread" => "read a thread".to_owned(),
        "create_draft" => "write a draft".to_owned(),
        "archive_messages" => "archive messages".to_owned(),
        "move_messages" => "move messages".to_owned(),
        "add_label" => "label messages".to_owned(),
        "remove_label" => "unlabel messages".to_owned(),
        other => format!("call {other}"),
    }
}

/// The sentence for one call: `Read 1 message "Receipt" in INBOX · ada@example.com`,
/// with ` · refused` or ` · failed` when it did not answer.
pub fn call_line(call: &AgentCall, names: &Names) -> String {
    let mut line = verb(&call.tool, call);
    if let [only] = call.messages.as_slice()
        && let Some(subject) = names.messages.get(only)
    {
        line.push_str(&format!(" \u{201c}{subject}\u{201d}"));
    }
    let folders: Vec<&str> = call
        .mailboxes
        .iter()
        .filter_map(|id| names.mailboxes.get(id).map(String::as_str))
        .collect();
    match folders.as_slice() {
        [] => {}
        [one] => line.push_str(&format!(" in {one}")),
        [first, rest @ ..] => line.push_str(&format!(" in {first} and {} more", rest.len())),
    }
    let accounts: Vec<&str> = call
        .accounts
        .iter()
        .filter_map(|id| names.accounts.get(id).map(String::as_str))
        .collect();
    if !accounts.is_empty() {
        line.push_str(&format!(" \u{b7} {}", accounts.join(", ")));
    }
    match call.outcome {
        CallOutcome::Answered => {}
        CallOutcome::Refused => line.push_str(" \u{b7} refused"),
        CallOutcome::Failed => line.push_str(" \u{b7} failed"),
    }
    line
}

/// `just now`, `5 min ago`, `3 h ago`, `2 days ago`; a date past a week.
pub fn when_label(at: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let seconds = (now - at).num_seconds().max(0);
    match seconds {
        0..=59 => "just now".to_owned(),
        60..=3599 => format!("{} min ago", seconds / 60),
        3600..=86_399 => format!("{} h ago", seconds / 3600),
        86_400..=604_799 => match seconds / 86_400 {
            1 => "1 day ago".to_owned(),
            days => format!("{days} days ago"),
        },
        _ => at.format("%-d %b %Y").to_string(),
    }
}

/// The line over the activity list: `12 calls · last 5 min ago`.
pub fn activity_summary(calls: &[AgentCall], now: DateTime<Utc>) -> String {
    match calls.first() {
        None => NO_ACTIVITY.to_owned(),
        Some(latest) => {
            let count = match calls.len() {
                1 => "1 call".to_owned(),
                n => format!("{n} calls"),
            };
            format!("{count} \u{b7} last {}", when_label(latest.at, now))
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallRow {
    pub when: String,
    pub line: String,
    pub refused: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Activity {
    pub summary: String,
    pub calls: Vec<CallRow>,
}

pub fn activity(calls: &[AgentCall], names: &Names, now: DateTime<Utc>) -> Activity {
    Activity {
        summary: activity_summary(calls, now),
        calls: calls
            .iter()
            .map(|call| CallRow {
                when: when_label(call.at, now),
                line: call_line(call, names),
                refused: call.outcome != CallOutcome::Answered,
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use postio_config::Config;

    use super::*;

    fn at(text: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(text)
            .unwrap()
            .with_timezone(&Utc)
    }

    #[test]
    fn nothing_shared_reads_as_off_and_says_how_to_turn_it_on() {
        let config = Config::from_toml_str("").unwrap();
        assert!(shared_accounts(&config.mcp).is_empty());
        assert!(NOTHING_SHARED.contains("off"));
    }

    #[test]
    fn a_shared_account_says_its_folders_and_what_the_agent_may_do() {
        let config = Config::from_toml_str(
            "[mcp.accounts.\"ada@example.com\"]\nmailboxes = [\"INBOX\", \"Receipts\"]\n\
             [mcp.accounts.\"grace@example.com\"]\nmailboxes = [\"*\"]\nactions = [\"label\", \"archive\"]\n\
             [mcp.accounts.\"nobody@example.com\"]\n",
        )
        .unwrap();
        let shared = shared_accounts(&config.mcp);
        let row = |address: &str| shared.iter().find(|row| row.address == address).unwrap();
        assert_eq!(row("ada@example.com").folders, "INBOX, Receipts");
        assert_eq!(row("ada@example.com").abilities, "Read and draft only");
        assert_eq!(row("grace@example.com").folders, "All folders");
        assert_eq!(row("grace@example.com").abilities, "Also archive and label");
        assert_eq!(row("nobody@example.com").folders, "No folders");
    }

    const LOG: &str = concat!(
        r#"{"at":"2026-10-16T10:00:00+00:00","tool":"search_mail","accounts":[1],"mailboxes":[7],"messages":[11,12],"drafts":[],"labels":[],"outcome":"answered"}"#,
        "\n",
        "not json at all\n",
        r#"{"at":"2026-10-16T11:00:00+00:00","tool":"read_message","accounts":[1],"mailboxes":[7],"messages":[11],"drafts":[],"labels":[],"outcome":"answered"}"#,
        "\n",
        r#"{"at":"2026-10-16T11:30:00+00:00","tool":"archive_messages","accounts":[2],"mailboxes":[],"messages":[],"drafts":[],"labels":[],"outcome":"refused"}"#,
        "\n",
    );

    #[test]
    fn the_log_reads_newest_first_and_skips_a_line_that_is_not_a_call() {
        let calls = parse_log(LOG, 10);
        let tools: Vec<&str> = calls.iter().map(|call| call.tool.as_str()).collect();
        assert_eq!(tools, ["archive_messages", "read_message", "search_mail"]);
        assert_eq!(parse_log(LOG, 2).len(), 2, "limit keeps the newest");
        assert!(parse_log("", 10).is_empty());
    }

    #[test]
    fn a_call_reads_as_a_sentence_over_what_its_ids_resolved_to() {
        let calls = parse_log(LOG, 10);
        let names = Names {
            accounts: HashMap::from([(1, "ada@example.com".to_owned())]),
            mailboxes: HashMap::from([(7, "INBOX".to_owned())]),
            messages: HashMap::from([(11, "Your receipt".to_owned())]),
        };
        assert_eq!(
            call_line(&calls[1], &names),
            "Read 1 message \u{201c}Your receipt\u{201d} in INBOX \u{b7} ada@example.com"
        );
        assert_eq!(
            call_line(&calls[2], &names),
            "Searched mail, 2 messages found in INBOX \u{b7} ada@example.com"
        );
    }

    #[test]
    fn the_activity_is_rows_ready_to_draw_newest_first() {
        let calls = parse_log(LOG, 10);
        let shown = activity(&calls, &Names::default(), at("2026-10-16T11:35:00+00:00"));
        assert_eq!(shown.summary, "3 calls \u{b7} last 5 min ago");
        let rows: Vec<(&str, bool)> = shown
            .calls
            .iter()
            .map(|row| (row.when.as_str(), row.refused))
            .collect();
        assert_eq!(
            rows,
            [
                ("5 min ago", true),
                ("35 min ago", false),
                ("1 h ago", false)
            ]
        );
        assert_eq!(
            shown.calls[0].line,
            "Tried to archive messages \u{b7} refused"
        );
    }

    #[test]
    fn reading_a_thread_says_thread() {
        // The vocabulary table says Thread, never Conversation (#2171).
        let log = concat!(
            r#"{"at":"2026-10-16T10:00:00+00:00","tool":"read_thread","accounts":[],"mailboxes":[],"messages":[11,12],"drafts":[],"labels":[],"outcome":"answered"}"#,
            "\n",
            r#"{"at":"2026-10-16T11:00:00+00:00","tool":"read_thread","accounts":[],"mailboxes":[],"messages":[],"drafts":[],"labels":[],"outcome":"refused"}"#,
            "\n",
        );
        let lines: Vec<String> = parse_log(log, 10)
            .iter()
            .map(|call| call_line(call, &Names::default()))
            .collect();
        assert_eq!(
            lines,
            [
                "Tried to read a thread \u{b7} refused",
                "Read a thread of 2 messages"
            ]
        );
    }

    #[test]
    fn a_refused_call_says_so_and_an_id_that_is_gone_is_not_named() {
        let calls = parse_log(LOG, 10);
        let line = call_line(&calls[0], &Names::default());
        assert_eq!(line, "Tried to archive messages \u{b7} refused");
    }

    #[test]
    fn only_the_ids_a_page_draws_are_asked_for() {
        let wanted = wanted(&parse_log(LOG, 10));
        assert_eq!(wanted.accounts, BTreeSet::from([1, 2]));
        assert_eq!(wanted.mailboxes, BTreeSet::from([7]));
        assert_eq!(wanted.messages, BTreeSet::from([11]));
    }

    #[test]
    fn when_is_relative_for_a_week_and_a_date_after() {
        let now = at("2026-10-16T12:00:00+00:00");
        assert_eq!(when_label(at("2026-10-16T11:59:30+00:00"), now), "just now");
        assert_eq!(
            when_label(at("2026-10-16T11:30:00+00:00"), now),
            "30 min ago"
        );
        assert_eq!(when_label(at("2026-10-16T09:00:00+00:00"), now), "3 h ago");
        assert_eq!(
            when_label(at("2026-10-14T12:00:00+00:00"), now),
            "2 days ago"
        );
        assert_eq!(
            when_label(at("2026-09-01T12:00:00+00:00"), now),
            "1 Sep 2026"
        );
    }

    #[test]
    fn the_summary_counts_calls_and_dates_the_latest() {
        let now = at("2026-10-16T12:00:00+00:00");
        assert_eq!(
            activity_summary(&parse_log(LOG, 10), now),
            "3 calls \u{b7} last 30 min ago"
        );
        assert_eq!(activity_summary(&[], now), NO_ACTIVITY);
    }

    #[test]
    fn the_revoke_note_promises_the_next_call_and_not_the_one_running() {
        assert!(REVOKE_NOTE.contains("next call") && REVOKE_NOTE.contains("already running"));
    }
}
