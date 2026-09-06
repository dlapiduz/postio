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
