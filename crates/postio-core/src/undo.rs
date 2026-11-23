//! The undo stack: *Archived 12 messages - Undo*, bound to `u`.
//!
//! A burst coalesces into one unit; entries carry inverses as `Command`s applied directly, since
//! replaying through the bus would record an undo of the undo.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use postio_model::MessageId;
use serde::{Deserialize, Serialize};

use crate::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UndoKind {
    Archive,
    Delete,
    Move,
    Flag,
    Unflag,
    MarkRead,
    MarkUnread,
    Label,
    Snooze,
    Unsnooze,
    Remind,
    Unremind,
    Accept,
    Decline,
    DismissMarker,
    UndismissMarker,
    Restore,
    Refilter,
    Sweep,
    StopDigesting,
    ResumeDigesting,
    MarkedSent,
    RetriedSend,
    CancelledSend,
    MapMailboxRole,
    JoinContacts,
    ContactArrival,
    ContactAddress,
    DetachAddress,
    PreferredAddress,
    CreateContact,
    EditContact,
    DeleteContact,
    RestoreContact,
    CreateGroup,
    RenameGroup,
    DeleteGroup,
    GroupMembers,
    Mute,
    Unmute,
    Split,
    Unsplit,
    AcceptKey,
    UnacceptKey,
    ForgetKey,
    RestoreKey,
}

impl UndoKind {
    fn describe(self, count: usize) -> String {
        let messages = if count == 1 { "message" } else { "messages" };
        match self {
            UndoKind::Archive => format!("Archived {count} {messages}"),
            UndoKind::Delete => format!("Deleted {count} {messages}"),
            UndoKind::Move => format!("Moved {count} {messages}"),
            UndoKind::Flag => format!("Flagged {count} {messages}"),
            UndoKind::Unflag => format!("Unflagged {count} {messages}"),
            UndoKind::MarkRead => format!("Marked {count} {messages} as read"),
            UndoKind::MarkUnread => format!("Marked {count} {messages} as unread"),
            UndoKind::Label => format!("Labelled {count} {messages}"),
            // Never plural: it settles one draft.
            UndoKind::MarkedSent => "Marked as sent".to_owned(),
            UndoKind::RetriedSend => "Sending again".to_owned(),
            // Says where the message went, so the person knows the reply was not thrown away.
            UndoKind::CancelledSend => "Send cancelled \u{2014} back in Drafts".to_owned(),
            UndoKind::MapMailboxRole => "Changed a folder's role".to_owned(),
            UndoKind::JoinContacts => "Merged contacts".to_owned(),
            UndoKind::ContactArrival => "Changed where mail arrives".to_owned(),
            UndoKind::ContactAddress => "Added an address".to_owned(),
            UndoKind::DetachAddress => "Detached an address".to_owned(),
            UndoKind::PreferredAddress => "Changed the preferred address".to_owned(),
            UndoKind::CreateContact => "Added a contact".to_owned(),
            UndoKind::EditContact => "Edited a contact".to_owned(),
            UndoKind::DeleteContact => "Deleted a contact".to_owned(),
            UndoKind::RestoreContact => "Restored a contact".to_owned(),
            UndoKind::CreateGroup => "Made a group".to_owned(),
            UndoKind::RenameGroup => "Renamed a group".to_owned(),
            UndoKind::DeleteGroup => "Deleted a group".to_owned(),
            UndoKind::GroupMembers => "Changed a group".to_owned(),
            UndoKind::Mute => "Muted the thread".to_owned(),
            UndoKind::Unmute => "Unmuted the thread".to_owned(),
            UndoKind::Split => "Split into its own thread".to_owned(),
            UndoKind::Unsplit => "Joined the thread again".to_owned(),
            UndoKind::AcceptKey => "Accepted the key".to_owned(),
            UndoKind::UnacceptKey => "Took back accepting the key".to_owned(),
            UndoKind::ForgetKey => "Forgot the key".to_owned(),
            UndoKind::RestoreKey => "Brought the key back".to_owned(),
            UndoKind::Snooze => format!("Snoozed {count} {messages}"),
            UndoKind::Unsnooze => format!("Unsnoozed {count} {messages}"),
            // Counted in conversations: a reminder waits on a reply to the conversation.
            UndoKind::Remind => format!("Reminder set on {count} {}", threads(count)),
            UndoKind::Unremind => format!("Reminder cleared on {count} {}", threads(count)),
            UndoKind::Accept => "Accepted".to_owned(),
            UndoKind::Decline => "Declined".to_owned(),
            UndoKind::DismissMarker => format!("Dismissed {count} {}", markers(count)),
            UndoKind::UndismissMarker => format!("Brought back {count} {}", markers(count)),
            UndoKind::Restore => format!("Restored {count} {messages}"),
            UndoKind::Refilter => format!("Filtered {count} {messages} again"),
            UndoKind::Sweep => format!("Filtered {count} {messages} out of the inbox"),
            UndoKind::StopDigesting => format!("Stopped digesting {count} {}", senders(count)),
            UndoKind::ResumeDigesting => format!("Digesting {count} {} again", senders(count)),
        }
    }
}

fn threads(count: usize) -> &'static str {
    if count == 1 { "thread" } else { "threads" }
}

fn senders(count: usize) -> &'static str {
    if count == 1 { "sender" } else { "senders" }
}

fn markers(count: usize) -> &'static str {
    if count == 1 { "marker" } else { "markers" }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UndoEntry {
    kind: UndoKind,
    /// Shorter than the stack's expiry for a send that can be cancelled only until it leaves.
    #[serde(default)]
    window: Option<Duration>,
    messages: Vec<MessageId>,
    /// Larger than `messages.len()` for a whole-mailbox unit, which knows its size from a
    /// `count(*)` and not its members.
    count: usize,
    /// Set when the rows acted on are not the messages they hold: a three-message conversation is
    /// one row.
    #[serde(default)]
    rows: Option<usize>,
    inverse: Vec<Command>,
}

impl UndoEntry {
    pub fn new(kind: UndoKind, messages: Vec<MessageId>, inverse: Vec<Command>) -> Self {
        UndoEntry {
            kind,
            window: None,
            count: messages.len(),
            rows: None,
            messages,
            inverse,
        }
    }

    /// The whole-mailbox case: the inverse is a `MessageTarget::Batch` predicate, so naming the
    /// rows would be the mailbox-sized read.
    pub fn bulk(kind: UndoKind, count: usize, inverse: Vec<Command>) -> Self {
        UndoEntry {
            kind,
            window: None,
            messages: Vec::new(),
            count,
            rows: None,
            inverse,
        }
    }

    /// The toast counts what the person chose (three rows), not the messages inside them; every
    /// message still has its way back.
    #[must_use]
    pub fn acting_on(mut self, rows: usize) -> Self {
        self.rows = Some(rows);
        self
    }

    fn shown(&self) -> usize {
        self.rows.unwrap_or(self.count)
    }

    /// An invitation answer's reply waits seconds in the outbox, so the entry goes with its window
    /// or it shadows the archive beneath with "too late". It never folds into another unit.
    #[must_use]
    pub fn lasting(mut self, window: Duration) -> Self {
        self.window = Some(window);
        self
    }

    pub fn is_bulk(&self) -> bool {
        self.count != self.messages.len()
    }

    pub fn kind(&self) -> UndoKind {
        self.kind
    }

    pub fn messages(&self) -> &[MessageId] {
        &self.messages
    }

    pub fn inverse(&self) -> &[Command] {
        &self.inverse
    }

    /// Derived, not stored, so a coalesced unit describes itself correctly.
    pub fn description(&self) -> String {
        self.kind.describe(self.shown())
    }

    pub fn undone_description(&self) -> String {
        format!("{}, undone", self.description())
    }

    /// A whole-mailbox unit always overlaps: it cannot name its rows, and folding the next archive
    /// into it would put separately acted rows behind one `u`.
    fn overlaps(&self, other: &UndoEntry) -> bool {
        self.is_bulk()
            || other.is_bulk()
            || other
                .messages
                .iter()
                .any(|message| self.messages.contains(message))
    }

    fn absorb(&mut self, other: UndoEntry) {
        if self.rows.is_some() || other.rows.is_some() {
            self.rows = Some(self.shown() + other.shown());
        }
        self.messages.extend(other.messages);
        self.count += other.count;
        self.inverse.extend(other.inverse);
    }
}

#[derive(Debug, Clone)]
struct Recorded {
    entry: UndoEntry,
    at: Instant,
}

impl Recorded {
    fn closed(&self, now: Instant) -> bool {
        self.entry
            .window
            .is_some_and(|window| now.saturating_duration_since(self.at) >= window)
    }
}

#[derive(Debug, Clone)]
pub struct UndoStack {
    entries: VecDeque<Recorded>,
    coalesce_within: Duration,
    expire_after: Duration,
    depth: usize,
}

impl Default for UndoStack {
    fn default() -> Self {
        UndoStack::new()
    }
}

impl UndoStack {
    /// A keystroke burst is well inside it; a new thought is not.
    pub const COALESCE_WINDOW: Duration = Duration::from_secs(1);

    /// Past this the local database and the server have both moved on.
    pub const EXPIRY: Duration = Duration::from_secs(600);

    pub const MAX_DEPTH: usize = 50;

    pub fn new() -> Self {
        UndoStack::with_policy(Self::COALESCE_WINDOW, Self::EXPIRY, Self::MAX_DEPTH)
    }

    /// For tests that need to step across the window rather than sleep through it.
    pub fn with_policy(coalesce_within: Duration, expire_after: Duration, depth: usize) -> Self {
        UndoStack {
            entries: VecDeque::new(),
            coalesce_within,
            expire_after,
            depth: depth.max(1),
        }
    }

    pub fn depth(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn peek(&self) -> Option<&UndoEntry> {
        let now = Instant::now();
        self.entries
            .iter()
            .rev()
            .find(|recorded| !recorded.closed(now))
            .map(|recorded| &recorded.entry)
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    pub fn record(&mut self, entry: UndoEntry) -> &UndoEntry {
        self.record_at(entry, Instant::now())
    }

    pub fn record_at(&mut self, entry: UndoEntry, now: Instant) -> &UndoEntry {
        self.prune(now);

        if let Some(last) = self.entries.back_mut()
            && last.entry.kind == entry.kind
            && last.entry.window.is_none()
            && entry.window.is_none()
            && now.duration_since(last.at) <= self.coalesce_within
            && !last.entry.overlaps(&entry)
        {
            last.entry.absorb(entry);
            // The window runs from the last action, so holding a key down stays one gesture.
            last.at = now;
            return &self.entries.back().expect("just merged into it").entry;
        }

        self.entries.push_back(Recorded { entry, at: now });
        if self.entries.len() > self.depth {
            self.entries.pop_front();
        }
        &self.entries.back().expect("just pushed").entry
    }

    pub fn undo(&mut self) -> Option<UndoEntry> {
        self.undo_at(Instant::now())
    }

    pub fn undo_at(&mut self, now: Instant) -> Option<UndoEntry> {
        self.prune(now);
        self.entries.pop_back().map(|recorded| recorded.entry)
    }

    /// Entries are in time order, so the stack's expiry only looks at the front; an entry whose own
    /// window closed can be anywhere.
    fn prune(&mut self, now: Instant) {
        while let Some(oldest) = self.entries.front() {
            if now.saturating_duration_since(oldest.at) > self.expire_after {
                self.entries.pop_front();
            } else {
                break;
            }
        }
        self.entries.retain(|recorded| !recorded.closed(now));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(kind: UndoKind, id: i64) -> UndoEntry {
        UndoEntry::new(kind, vec![MessageId::new(id)], vec![Command::Undo])
    }

    #[test]
    fn the_toast_counts_and_pluralizes() {
        assert_eq!(
            UndoKind::CancelledSend.describe(1),
            "Send cancelled \u{2014} back in Drafts",
            "a cancelled send says where the message went"
        );
        assert_eq!(UndoKind::Archive.describe(1), "Archived 1 message");
        assert_eq!(UndoKind::Archive.describe(12), "Archived 12 messages");
        assert_eq!(
            UndoKind::MarkUnread.describe(2),
            "Marked 2 messages as unread"
        );
    }

    #[test]
    fn recording_reports_the_unit_it_landed_in() {
        let mut stack = UndoStack::new();
        let now = Instant::now();
        stack.record_at(entry(UndoKind::Flag, 1), now);
        let merged = stack.record_at(entry(UndoKind::Flag, 2), now);

        assert_eq!(merged.description(), "Flagged 2 messages");
    }

    #[test]
    fn a_whole_mailbox_unit_counts_what_it_will_not_name() {
        let entry = UndoEntry::bulk(UndoKind::Archive, 81_717, vec![Command::Undo]);

        assert_eq!(entry.description(), "Archived 81717 messages");
        assert!(entry.messages().is_empty(), "it names none of them");
        assert!(entry.is_bulk());
    }

    #[test]
    fn a_whole_mailbox_unit_never_coalesces_with_anything() {
        let mut stack = UndoStack::new();
        let now = Instant::now();
        stack.record_at(
            UndoEntry::bulk(UndoKind::Archive, 40_000, vec![Command::Undo]),
            now,
        );
        let next = stack.record_at(entry(UndoKind::Archive, 1), now);

        assert_eq!(next.description(), "Archived 1 message");
        assert_eq!(stack.depth(), 2, "two gestures, two units");
        assert_eq!(
            stack.undo_at(now).map(|entry| entry.description()),
            Some("Archived 1 message".into())
        );
        assert_eq!(
            stack.undo_at(now).map(|entry| entry.description()),
            Some("Archived 40000 messages".into()),
            "and the bulk one is still behind it, whole"
        );
    }

    #[test]
    fn the_toast_counts_the_rows_acted_on_not_the_messages_inside_them() {
        let ids = (1..=5).map(MessageId::new).collect();
        let entry = UndoEntry::new(UndoKind::Archive, ids, vec![Command::Undo]).acting_on(3);

        assert_eq!(entry.description(), "Archived 3 messages");
        assert_eq!(
            entry.messages().len(),
            5,
            "every message still has its way back"
        );
        assert!(!entry.is_bulk(), "the rows named are the messages it holds");
    }

    #[test]
    fn a_burst_counts_the_rows_of_every_action_in_it() {
        let mut stack = UndoStack::new();
        let now = Instant::now();
        let two = UndoEntry::new(
            UndoKind::Archive,
            vec![MessageId::new(1), MessageId::new(2)],
            vec![Command::Undo],
        )
        .acting_on(1);
        stack.record_at(two, now);
        let merged = stack.record_at(entry(UndoKind::Archive, 3), now);

        assert_eq!(merged.description(), "Archived 2 messages");
        assert_eq!(merged.messages().len(), 3);
    }

    #[test]
    fn a_cancelled_send_says_the_message_is_a_draft_again() {
        assert_eq!(
            UndoKind::CancelledSend.describe(1),
            "Send cancelled \u{2014} back in Drafts"
        );
    }

    #[test]
    fn an_undone_unit_says_it_was_undone() {
        // The toast after `u` must not repeat the one before it.
        let entry = UndoEntry::new(
            UndoKind::Archive,
            vec![MessageId::new(1), MessageId::new(2)],
            vec![Command::Undo],
        );

        assert_eq!(entry.description(), "Archived 2 messages");
        assert_eq!(entry.undone_description(), "Archived 2 messages, undone");
    }

    #[test]
    fn a_policy_depth_of_zero_still_holds_one_unit() {
        let mut stack = UndoStack::with_policy(Duration::ZERO, Duration::from_secs(1), 0);
        stack.record(entry(UndoKind::Delete, 1));
        assert_eq!(stack.depth(), 1);
    }
}
