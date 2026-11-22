//! A conversation as the pane draws it (ADR 0015 Q4, ADR 0032): rows and where it opens, and the
//! thread
//! as one page. Swift computes neither.

use chrono::{DateTime, Local, TimeZone, Utc};

use crate::RowFfi;

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct ConversationFfi {
    /// So a pane that has moved on can drop a late read.
    pub thread: i64,
    /// The oldest message's subject: a thread named by its newest `Re: Re: Fwd:` would rename
    /// itself.
    pub subject: String,
    /// The line under it: `8 messages · Tessa, Mara, Pinepoint Radon · 10-25 Aug`.
    pub meta: String,
    /// Every message in the thread, oldest first, wherever it is filed.
    pub rows: Vec<RowFfi>,
    /// Which message the pane opens on — the first unread, else the newest.
    /// `None` only for an empty conversation.
    pub focus: Option<u32>,
    pub expanded: Vec<bool>,
}

pub fn fold(thread: i64, rows: Vec<RowFfi>, now: DateTime<Local>) -> ConversationFfi {
    let rows =
        postio_ui::conversation::arrange(&rows, postio_ui::conversation::Order::Oldest, false);
    let focus = postio_ui::conversation::opening_focus(&rows);
    let expanded = focus
        .map(|focus| {
            postio_ui::conversation::expanded_on_open(
                &rows,
                focus,
                postio_ui::conversation::EAGER_EXPANSION_CAP,
            )
        })
        .unwrap_or_default();

    ConversationFfi {
        thread,
        subject: subject(&rows),
        meta: meta(&rows, now),
        focus: focus.map(|focus| focus as u32),
        expanded,
        rows,
    }
}

fn subject(rows: &[RowFfi]) -> String {
    rows.iter()
        .find_map(|row| row.subject.clone())
        .unwrap_or_default()
}

/// `8 messages · Tessa, Mara, Pinepoint Radon · 10-25 Aug`. An empty conversation says nothing, not
/// `0 messages · ·`.
fn meta(rows: &[RowFfi], now: DateTime<Local>) -> String {
    if rows.is_empty() {
        return String::new();
    }
    let senders: Vec<postio_model::address::EmailAddress> = rows
        .iter()
        .filter_map(|row| {
            let address = row.from_address.clone()?;
            Some(postio_model::address::EmailAddress::new(
                row.from.as_deref(),
                address,
            ))
        })
        .collect();

    let mut parts = vec![postio_ui::conversation::message_count(rows.len())];
    let who = postio_ui::conversation::participants(&senders);
    if !who.is_empty() {
        parts.push(who);
    }
    let first = local(rows[0].received_at);
    let last = local(rows[rows.len() - 1].received_at);
    parts.push(postio_ui::conversation::date_span(first, last, now));
    parts.join(" · ")
}

/// A row's timestamp, in the timezone a person is reading it in.
fn local(seconds: i64) -> DateTime<Local> {
    Local.from_utc_datetime(
        &Utc.timestamp_opt(seconds, 0)
            .single()
            .unwrap_or_else(|| Utc.timestamp_nanos(0))
            .naive_utc(),
    )
}

/// When one message arrived, as its own header says it: `Mon 25 Aug at 12:00`. The wording is
/// `postio_ui::conversation`'s so both frontends read the same (ADR 0019 Q6).
#[uniffi::export]
pub fn message_when(received_at: i64) -> String {
    postio_ui::conversation::message_when(local(received_at), Local::now())
}

/// A conversation drawn as one document (ADR 0032, #1595): the page, and where each message is in
/// it.
#[derive(Debug, Clone, PartialEq, Eq, Default, uniffi::Record)]
pub struct ThreadDocumentFfi {
    pub html: String,
    /// The messages in the order the page stacks them, oldest first.
    pub messages: Vec<ThreadAnchorFfi>,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ThreadAnchorFfi {
    pub message: i64,
    /// The element id `J`, `K` and the rail scroll to
    /// (`postio_ui::reader::thread::message_anchor`).
    pub anchor: String,
    /// Its sender's address: what the in-page `Show` link grants, since the
    /// link names the message and the decision is per sender.
    pub address: String,
    /// Native chrome above the page, so it crosses beside it; from the body load the page already
    /// paid for (#1589).
    pub caveat: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ThreadVerbKindFfi {
    Reply,
    Forward,
    Continue,
    Allow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct ThreadVerbFfi {
    pub kind: ThreadVerbKindFfi,
    pub message: i64,
}

/// `None` for anything else, including a verb naming something that is not a message id.
#[uniffi::export]
pub fn thread_verb(url: String) -> Option<ThreadVerbFfi> {
    use postio_ui::reader::thread::MessageVerb;
    let (verb, scope) = postio_ui::reader::thread::verb_of(&url)?;
    Some(ThreadVerbFfi {
        kind: match verb {
            MessageVerb::Reply => ThreadVerbKindFfi::Reply,
            MessageVerb::Forward => ThreadVerbKindFfi::Forward,
            MessageVerb::Continue => ThreadVerbKindFfi::Continue,
            MessageVerb::Allow => ThreadVerbKindFfi::Allow,
        },
        message: scope.parse().ok()?,
    })
}

#[uniffi::export]
pub fn thread_scroll_script(anchor: String) -> String {
    postio_ui::reader::thread::scroll_script(&anchor)
}

#[uniffi::export]
pub fn thread_toggle_script(anchor: String) -> String {
    postio_ui::reader::thread::toggle_script(&anchor)
}

#[uniffi::export]
pub fn thread_expand_all_script() -> String {
    postio_ui::reader::thread::expand_all_script()
}

/// The host script that reports which message fills the pane to
/// `window.webkit.messageHandlers.<handler>`.
#[uniffi::export]
pub fn thread_observer_script(handler: String) -> String {
    postio_ui::reader::thread::observer_script(&handler)
}
