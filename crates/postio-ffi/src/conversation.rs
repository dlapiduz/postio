//! A conversation, folded, as the pane draws it (ADR 0015 Q4, #1263).
//!
//! The macOS reading pane stacks a whole thread: read messages as one line,
//! the latest and the unread ones open. **Which** ones open is not a drawing
//! decision — it is what an opened conversation costs, one web view per
//! expanded message — so it is decided in `postio_ui::conversation` and
//! crosses the boundary already made. Swift draws the answer; it does not
//! compute it.

use chrono::{DateTime, Local, TimeZone, Utc};

use crate::RowFfi;

/// One conversation, ready to draw.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct ConversationFfi {
    /// The thread this is, so a pane that has moved on can drop a late read.
    pub thread: i64,
    /// The subject, largest thing in the pane: the conversation's own, which
    /// is the oldest message's — a thread named by its newest `Re: Re: Fwd:`
    /// would rename itself as it grew.
    pub subject: String,
    /// The line under it: `8 messages · Tessa, Mara, Pinepoint Radon · 10-25 Aug`.
    pub meta: String,
    /// Every message in the thread, oldest first, wherever it is filed.
    pub rows: Vec<RowFfi>,
    /// Which message the pane opens on — the first unread, else the newest.
    /// `None` only for an empty conversation.
    pub focus: Option<u32>,
    /// Which messages open expanded, one flag per row of `rows`.
    pub expanded: Vec<bool>,
}

/// Fold `rows` into the conversation `thread` — the whole policy, in one
/// place, applied before anything crosses.
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

/// The conversation's subject: the oldest message's, and the oldest message
/// that has one if it does not.
fn subject(rows: &[RowFfi]) -> String {
    rows.iter()
        .find_map(|row| row.subject.clone())
        .unwrap_or_default()
}

/// `8 messages · Tessa, Mara, Pinepoint Radon · 10-25 Aug`.
///
/// Every part of it is `postio_ui::conversation`'s, including the eliding and
/// the three shapes a date span takes. What this adds is the joining, and the
/// fact that an empty conversation says nothing rather than `0 messages · ·`.
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
    // Oldest and newest: `rows` is already stacked, so the ends are the span.
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

/// A run of collapsed messages, folded behind one divider.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct RunFfi {
    /// The first message of the run, as an index into the conversation's rows.
    pub start: u32,
    /// How many messages it hides.
    pub count: u32,
    /// What the divider says: `5 earlier messages · Tessa, Mara`.
    pub summary: String,
}

/// Which runs of collapsed messages are long enough to fold into a divider.
///
/// A free function rather than session state, because the answer changes on
/// every expand and collapse — this is asked as someone reads, not once when
/// the conversation opens. It is still not the frontend's to decide: the
/// three-in-a-row minimum, and the eliding of the names on the divider, are
/// `postio_ui::conversation`'s, and a Swift reimplementation would be a
/// second rule to keep in step.
///
/// `expanded` is one flag per row of `rows`, in the same order.
#[uniffi::export]
pub fn conversation_runs(rows: Vec<RowFfi>, expanded: Vec<bool>) -> Vec<RunFfi> {
    let collapsed: Vec<bool> = expanded.iter().map(|open| !open).collect();
    postio_ui::conversation::collapsed_runs(&collapsed, postio_ui::conversation::RUN_MINIMUM)
        .into_iter()
        .map(|range| {
            let senders: Vec<postio_model::address::EmailAddress> = rows
                .get(range.clone())
                .unwrap_or_default()
                .iter()
                .filter_map(|row| {
                    let address = row.from_address.clone()?;
                    Some(postio_model::address::EmailAddress::new(
                        row.from.as_deref(),
                        address,
                    ))
                })
                .collect();
            RunFfi {
                start: range.start as u32,
                count: range.len() as u32,
                summary: postio_ui::conversation::run_summary(range.len(), &senders),
            }
        })
        .collect()
}

/// When one message arrived, as its own header says it: `Mon 25 Aug at 12:00`.
///
/// A free function for the same reason [`conversation_runs`] is: it is a
/// rendering of a timestamp, not a question about a session. The wording is
/// `postio_ui::conversation`'s, so both frontends' message headers read the
/// same — a header is where "which Tuesday was that" gets answered, and two
/// platforms answering it differently is exactly the drift ADR 0019 Q6 is
/// about.
#[uniffi::export]
pub fn message_when(received_at: i64) -> String {
    postio_ui::conversation::message_when(local(received_at), Local::now())
}

/// A conversation drawn as one document (ADR 0032, #1595): the page, and
/// where each message is in it.
///
/// The page is `postio_ui::reader::thread::compose`'s, the same function
/// GTK's pane calls, so a conversation reads the same on either platform.
#[derive(Debug, Clone, PartialEq, Eq, Default, uniffi::Record)]
pub struct ThreadDocumentFfi {
    /// The whole hardened document.
    pub html: String,
    /// The messages in the order the page stacks them, oldest first.
    pub messages: Vec<ThreadAnchorFfi>,
}

/// One message's place in a [`ThreadDocumentFfi`].
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ThreadAnchorFfi {
    /// The message.
    pub message: i64,
    /// The element id it carries, which `J`, `K` and the rail scroll to --
    /// `postio_ui::reader::thread::message_anchor`, so the id and the
    /// fragment that finds it cannot disagree.
    pub anchor: String,
    /// Its sender's address: what the in-page `Show` link grants, since the
    /// link names the message and the decision is per sender.
    pub address: String,
}

/// Which verb a message offers inside the document. See [`thread_verb`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ThreadVerbKindFfi {
    /// Reply to this message rather than to the latest (FR-009).
    Reply,
    /// Forward this message.
    Forward,
    /// Resume the composer on this draft (#1212).
    Continue,
    /// Allow this message's sender's remote images.
    Allow,
}

/// A verb and the message it is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct ThreadVerbFfi {
    /// What was asked.
    pub kind: ThreadVerbKindFfi,
    /// Of which message.
    pub message: i64,
}

/// What a navigation inside a conversation document asks for, if it is one
/// of the page's own verbs (#1595).
///
/// `postio_ui::reader::thread::verb_of`'s parse, the rule GTK's reader
/// applies, so the Mac never matches these schemes itself. `None` for
/// anything else -- including a verb naming something that is not a message
/// id, which a page this boundary wrote never contains.
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

/// The host script that scrolls a conversation document to `anchor`.
/// `postio_ui::reader::thread::scroll_script`, for `J`, `K` and the rail.
#[uniffi::export]
pub fn thread_scroll_script(anchor: String) -> String {
    postio_ui::reader::thread::scroll_script(&anchor)
}

/// The host script that folds or unfolds the message at `anchor` (`z`).
#[uniffi::export]
pub fn thread_toggle_script(anchor: String) -> String {
    postio_ui::reader::thread::toggle_script(&anchor)
}

/// The host script that opens every message (*Expand all*).
#[uniffi::export]
pub fn thread_expand_all_script() -> String {
    postio_ui::reader::thread::expand_all_script()
}
