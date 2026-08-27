//! The undo stack: *Archived 12 messages — Undo*.
//!
//! docs/PRODUCT.md §16 and canvas 3b promise that a destructive action is reversible
//! from a toast, bound to `u`. Two properties make that promise honest.
//!
//! **A burst is one unit.** Archiving twelve messages with twelve keystrokes is
//! one gesture as far as the user is concerned, so it is one entry here — one
//! toast that says twelve, and one `u` that takes all twelve back. Actions
//! coalesce while they are the same kind, land inside the coalescing window,
//! and touch messages the unit has not touched yet; that last condition is what
//! keeps every action in a unit independent, and independence is what makes
//! replaying the inverses in recorded order correct.
//!
//! **Undo is local.** An entry carries its inverse as [`Command`]s — the same
//! vocabulary the bus already speaks — and the undo handler applies them
//! local-first: change SQLite now, enqueue the remote half, emit the event.
//! Nothing awaits the network, so undo works on a train with no signal and the
//! server catches up later.
//!
//! Inverses are applied *directly* rather than sent back through the bus. A
//! replayed command would record an undo of the undo, and `u` `u` would toggle
//! instead of walking back through history.
//!
//! # Bounds
//!
//! The stack forgets, on purpose: it holds [`UndoStack::MAX_DEPTH`] units, and
//! a unit older than [`UndoStack::EXPIRY`] is gone. Putting back something
//! archived an hour ago is a surprise rather than a mercy, and by then the
//! server has almost certainly moved on.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use postio_model::MessageId;
use serde::{Deserialize, Serialize};

use crate::Command;

/// What kind of action an undo entry takes back.
///
/// Only actions of the same kind coalesce, so "Archived 3 messages" can never
/// quietly include a delete.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UndoKind {
    /// Messages were archived.
    Archive,
    /// Messages were moved to the trash.
    Delete,
    /// Messages were moved to another mailbox.
    Move,
    /// Messages were flagged.
    Flag,
    /// Messages were unflagged.
    Unflag,
    /// Messages were marked read.
    MarkRead,
    /// Messages were marked unread.
    MarkUnread,
    /// A label was attached to messages.
    Label,
    /// Messages were snoozed.
    Snooze,
    /// Messages were unsnoozed.
    Unsnooze,
}

impl UndoKind {
    /// The toast for `count` messages, already phrased for the user.
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
            UndoKind::Snooze => format!("Snoozed {count} {messages}"),
            UndoKind::Unsnooze => format!("Unsnoozed {count} {messages}"),
        }
    }
}

/// One undoable unit: what happened, and what takes it back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UndoEntry {
    kind: UndoKind,
    messages: Vec<MessageId>,
    /// How many messages the unit covers.
    ///
    /// Equal to `messages.len()` for a unit that names its rows, and larger
    /// for a whole-mailbox one, which knows its size from a `count(*)` and
    /// deliberately does not know its members. See [`UndoEntry::bulk`].
    count: usize,
    inverse: Vec<Command>,
}

impl UndoEntry {
    /// Record that `kind` happened to `messages`, and that `inverse` undoes it.
    ///
    /// The inverse is expressed in commands so the undo handler can apply it
    /// with the same local-first machinery the original action used.
    pub fn new(kind: UndoKind, messages: Vec<MessageId>, inverse: Vec<Command>) -> Self {
        UndoEntry {
            kind,
            count: messages.len(),
            messages,
            inverse,
        }
    }

    /// Record that `kind` happened to `count` messages that this entry does
    /// not name, and that `inverse` takes it back.
    ///
    /// The whole-mailbox case. *Archived 81,717 messages* needs the number and
    /// nothing else needs the rows: the inverse is
    /// [`MessageTarget::Batch`](crate::MessageTarget::Batch), which is a
    /// predicate the store resolves in one statement. Naming the rows here
    /// would be the mailbox-sized read the predicate exists to avoid, arriving
    /// through the undo stack instead of through the selection.
    pub fn bulk(kind: UndoKind, count: usize, inverse: Vec<Command>) -> Self {
        UndoEntry {
            kind,
            messages: Vec::new(),
            count,
            inverse,
        }
    }

    /// Whether this unit covers rows it cannot name.
    pub fn is_bulk(&self) -> bool {
        self.count != self.messages.len()
    }

    /// What sort of action this was.
    pub fn kind(&self) -> UndoKind {
        self.kind
    }

    /// The messages it applied to, in the order it applied to them.
    pub fn messages(&self) -> &[MessageId] {
        &self.messages
    }

    /// What to do to take it back, in the order to do it.
    pub fn inverse(&self) -> &[Command] {
        &self.inverse
    }

    /// The toast: *Archived 12 messages*.
    ///
    /// Derived from the kind and the count rather than stored, so a unit that
    /// grows by coalescing always describes itself correctly.
    pub fn description(&self) -> String {
        self.kind.describe(self.count)
    }

    /// Whether this unit and `other` overlap, which would make replaying their
    /// inverses in recorded order wrong.
    ///
    /// A whole-mailbox unit always answers yes. It cannot say which rows it
    /// covers — that is the point of it — so there is no honest way to check,
    /// and the conservative answer is also the right one for the user:
    /// `Ctrl+A` then `a` is a gesture on its own, and folding the next archive
    /// into it would put a row the user acted on separately behind the same
    /// single `u`.
    fn overlaps(&self, other: &UndoEntry) -> bool {
        self.is_bulk()
            || other.is_bulk()
            || other
                .messages
                .iter()
                .any(|message| self.messages.contains(message))
    }

    fn absorb(&mut self, other: UndoEntry) {
        self.messages.extend(other.messages);
        self.count += other.count;
        self.inverse.extend(other.inverse);
    }
}

/// One entry and when it was last added to.
#[derive(Debug, Clone)]
struct Recorded {
    entry: UndoEntry,
    at: Instant,
}

/// The undo history: bounded, self-pruning, and coalescing.
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
    /// How long after an action another of the same kind still counts as the
    /// same gesture. A keystroke burst is well inside it; a new thought is not.
    pub const COALESCE_WINDOW: Duration = Duration::from_secs(1);

    /// How long an undo stays offered. Past this the local database and the
    /// server have both moved on.
    pub const EXPIRY: Duration = Duration::from_secs(600);

    /// How many units the stack holds before it forgets the oldest.
    pub const MAX_DEPTH: usize = 50;

    /// An empty stack with the application's policy.
    pub fn new() -> Self {
        UndoStack::with_policy(Self::COALESCE_WINDOW, Self::EXPIRY, Self::MAX_DEPTH)
    }

    /// An empty stack with a policy of your own — for tests that need to step
    /// across the window rather than sleep through it.
    pub fn with_policy(coalesce_within: Duration, expire_after: Duration, depth: usize) -> Self {
        UndoStack {
            entries: VecDeque::new(),
            coalesce_within,
            expire_after,
            depth: depth.max(1),
        }
    }

    /// How many units `u` can still walk back through.
    pub fn depth(&self) -> usize {
        self.entries.len()
    }

    /// Whether there is anything to undo.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The unit `u` would take back, without taking it back — this is what the
    /// toast is written from.
    pub fn peek(&self) -> Option<&UndoEntry> {
        self.entries.back().map(|recorded| &recorded.entry)
    }

    /// Forget everything. Used when the account changes out from under us.
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Record an action, coalescing it into the current unit if it belongs.
    pub fn record(&mut self, entry: UndoEntry) -> &UndoEntry {
        self.record_at(entry, Instant::now())
    }

    /// Record an action as of `now`.
    pub fn record_at(&mut self, entry: UndoEntry, now: Instant) -> &UndoEntry {
        self.prune(now);

        if let Some(last) = self.entries.back_mut()
            && last.entry.kind == entry.kind
            && now.duration_since(last.at) <= self.coalesce_within
            // Independence within a unit is what lets the inverses replay in
            // the order they were recorded.
            && !last.entry.overlaps(&entry)
        {
            last.entry.absorb(entry);
            // The window runs from the last action, so holding a key down
            // stays one gesture however long it is held.
            last.at = now;
            return &self.entries.back().expect("just merged into it").entry;
        }

        self.entries.push_back(Recorded { entry, at: now });
        if self.entries.len() > self.depth {
            self.entries.pop_front();
        }
        &self.entries.back().expect("just pushed").entry
    }

    /// Take back the most recent unit.
    pub fn undo(&mut self) -> Option<UndoEntry> {
        self.undo_at(Instant::now())
    }

    /// Take back the most recent unit as of `now`.
    pub fn undo_at(&mut self, now: Instant) -> Option<UndoEntry> {
        self.prune(now);
        self.entries.pop_back().map(|recorded| recorded.entry)
    }

    /// Drop units that have aged out. Entries are in time order, so this only
    /// ever has to look at the front.
    fn prune(&mut self, now: Instant) {
        while let Some(oldest) = self.entries.front() {
            if now.saturating_duration_since(oldest.at) > self.expire_after {
                self.entries.pop_front();
            } else {
                break;
            }
        }
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
        // *Archived 81,717 messages* from a `count(*)` and an inverse that is
        // a predicate. Putting the rows in here would be the mailbox-sized
        // read the selection predicate exists to avoid, arriving by another
        // door.
        let entry = UndoEntry::bulk(UndoKind::Archive, 81_717, vec![Command::Undo]);

        assert_eq!(entry.description(), "Archived 81717 messages");
        assert!(entry.messages().is_empty(), "it names none of them");
        assert!(entry.is_bulk());
    }

    #[test]
    fn a_whole_mailbox_unit_never_coalesces_with_anything() {
        // It cannot say which rows it covers, so there is no honest way to
        // check whether the next action overlaps it — and folding them would
        // put a row the user acted on separately behind the same single `u`.
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
    fn a_policy_depth_of_zero_still_holds_one_unit() {
        let mut stack = UndoStack::with_policy(Duration::ZERO, Duration::from_secs(1), 0);
        stack.record(entry(UndoKind::Delete, 1));
        assert_eq!(stack.depth(), 1);
    }
}
