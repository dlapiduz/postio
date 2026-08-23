//! What the user is looking at: one coherent, authoritative answer.
//!
//! The account, the mailbox, the selection, the view and the per-account
//! connection state live here and nowhere else. Widgets render from these
//! accessors and repaint from the events these mutations return; a widget that
//! kept its own copy would be one refresh away from disagreeing with the
//! database.
//!
//! # Every change is an event
//!
//! Mutators return the [`Event`]s the change produced, and they return them by
//! *diffing* the state rather than by remembering to emit — so a change that
//! emits nothing is impossible, and a no-op that emits something is too. That
//! is what makes "the UI repaints only from events" a rule the frontend can
//! rely on rather than a habit.
//!
//! # The back stack is why `t` and `Esc` round-trip
//!
//! Canvas 3a asks that drilling into a thread with `t` and coming back with
//! `Esc` restore the *exact* prior position. Each drill-in pushes the view and
//! the selection it left; [`AppState::back`] pops them. Because the position is
//! here and not in the list widget, it survives the widget being rebuilt,
//! rewindowed or scrolled somewhere else entirely.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use postio_model::{AccountId, DraftId, MailboxId, MessageId, ThreadId};
use serde::{Deserialize, Serialize};

use crate::bridge::EventSink;
use crate::{ConnectionState, Context, Event};

/// Which surface the reading pane is showing.
///
/// Not a widget and not a window: compose takes over the reading pane rather
/// than opening a window of its own, and search is a view the user can leave
/// with `Esc`, so both are modes of the same pane.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ViewMode {
    /// The message list.
    #[default]
    List,
    /// One thread, drilled into from the list.
    Thread {
        /// The thread on screen.
        thread: ThreadId,
    },
    /// One message in the reading pane.
    Reader {
        /// The message on screen.
        message: MessageId,
    },
    /// Search results for a query.
    Search {
        /// The query as the user typed it, in `postio-search`'s syntax.
        query: String,
    },
    /// The composer, which has taken the reading pane over.
    Composer {
        /// The draft being edited.
        draft: DraftId,
    },
}

impl ViewMode {
    /// The keyboard context this view owns.
    ///
    /// This is what the palette, the `?` cheat sheet and the focused-row key
    /// hints filter on, so the mapping has to live next to the view rather
    /// than being re-derived by each surface.
    pub fn context(&self) -> Context {
        match self {
            ViewMode::List => Context::List,
            ViewMode::Thread { .. } => Context::Thread,
            ViewMode::Reader { .. } => Context::Reader,
            ViewMode::Search { .. } => Context::Search,
            ViewMode::Composer { .. } => Context::Composer,
        }
    }
}

/// One step of the back stack: where the user was, and where they were in it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Frame {
    view: ViewMode,
    selected: Vec<MessageId>,
    focus: Option<MessageId>,
}

/// The application's view of itself.
///
/// Mutated only from command handlers — the bus is the single writer, which is
/// what lets every mutation be a total order without a lock held across an
/// `.await`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AppState {
    account: Option<AccountId>,
    mailbox: Option<MailboxId>,
    selected: Vec<MessageId>,
    focus: Option<MessageId>,
    view: ViewMode,
    back: Vec<Frame>,
    connections: BTreeMap<AccountId, ConnectionState>,
}

impl AppState {
    /// How deep the drill-in stack goes before the oldest step is forgotten.
    ///
    /// Deep enough that no real navigation reaches it, bounded so a session
    /// that hops between search, threads and messages for an hour does not
    /// grow without limit.
    pub const MAX_BACK_DEPTH: usize = 32;

    /// An empty state: the message list, nothing selected.
    pub fn new() -> Self {
        AppState::default()
    }

    // -- What the user is looking at -------------------------------------

    /// The account in the sidebar, if one has been opened.
    pub fn account(&self) -> Option<AccountId> {
        self.account
    }

    /// The mailbox the list is showing, if one has been opened.
    pub fn mailbox(&self) -> Option<MailboxId> {
        self.mailbox
    }

    /// The selected messages, in list order.
    pub fn selection(&self) -> &[MessageId] {
        &self.selected
    }

    /// The focused row — the one carrying the key hints, and the position a
    /// round trip through a thread has to restore.
    pub fn focus(&self) -> Option<MessageId> {
        self.focus
    }

    /// The current view.
    pub fn view(&self) -> &ViewMode {
        &self.view
    }

    /// The keyboard context, derived from the view.
    pub fn context(&self) -> Context {
        self.view.context()
    }

    /// The active search query, if the user is in search.
    pub fn search_query(&self) -> Option<&str> {
        match &self.view {
            ViewMode::Search { query } => Some(query),
            _ => None,
        }
    }

    /// The draft the composer is editing, if it is open.
    pub fn composing(&self) -> Option<DraftId> {
        match &self.view {
            ViewMode::Composer { draft } => Some(*draft),
            _ => None,
        }
    }

    /// How an account stands with its server. An account nothing has been
    /// reported about is [`ConnectionState::Offline`]: working locally.
    pub fn connection(&self, account: AccountId) -> ConnectionState {
        self.connections
            .get(&account)
            .copied()
            .unwrap_or(ConnectionState::Offline)
    }

    /// How many steps `Esc` can still unwind.
    pub fn back_depth(&self) -> usize {
        self.back.len()
    }

    // -- Mutations -------------------------------------------------------

    /// Open an account, which resets the mailbox and the selection with it.
    pub fn open_account(&mut self, account: AccountId) -> Vec<Event> {
        self.commit(|state| {
            if state.account == Some(account) {
                return;
            }
            state.account = Some(account);
            // The old mailbox and rows belong to an account that is no longer
            // on screen; keeping them would let an action land on a message
            // the user cannot see.
            state.mailbox = None;
            state.clear_position();
        })
    }

    /// Open a mailbox in the list, dropping a selection from the old one.
    pub fn open_mailbox(&mut self, mailbox: MailboxId) -> Vec<Event> {
        self.commit(|state| {
            if state.mailbox == Some(mailbox) {
                return;
            }
            state.mailbox = Some(mailbox);
            state.clear_position();
        })
    }

    /// Replace the selection and the focused row.
    pub fn select(&mut self, messages: Vec<MessageId>, focus: Option<MessageId>) -> Vec<Event> {
        self.commit(|state| {
            state.selected = messages;
            state.focus = focus;
        })
    }

    /// Move the focused row without changing the selection.
    ///
    /// `j` and `k` move the focus; the selection follows only when the user
    /// asks for it.
    pub fn focus_on(&mut self, message: Option<MessageId>) -> Vec<Event> {
        self.commit(|state| state.focus = message)
    }

    /// Drop the selection.
    pub fn clear_selection(&mut self) -> Vec<Event> {
        self.commit(AppState::clear_position)
    }

    /// Drill into a thread, remembering where we came from.
    pub fn open_thread(&mut self, thread: ThreadId) -> Vec<Event> {
        self.commit(|state| state.push(ViewMode::Thread { thread }))
    }

    /// Open a message in the reading pane, remembering where we came from.
    pub fn open_message(&mut self, message: MessageId) -> Vec<Event> {
        self.commit(|state| {
            state.push(ViewMode::Reader { message });
            // What the reader is showing *is* the selection: an archive from
            // the reading pane must not land on the row behind it.
            state.selected = vec![message];
            state.focus = Some(message);
        })
    }

    /// Show results for a query.
    ///
    /// Refining a query while already in search replaces the view rather than
    /// pushing another step — typing must not build a stack of `Esc`s.
    pub fn open_search(&mut self, query: impl Into<String>) -> Vec<Event> {
        let query = query.into();
        self.commit(|state| {
            let view = ViewMode::Search { query };
            if matches!(state.view, ViewMode::Search { .. }) {
                state.view = view;
            } else {
                state.push(view);
            }
        })
    }

    /// Open the composer over the reading pane.
    pub fn open_composer(&mut self, draft: DraftId) -> Vec<Event> {
        self.commit(|state| state.push(ViewMode::Composer { draft }))
    }

    /// Go back one step, restoring the exact position that step left.
    ///
    /// Returns no events at the top of the stack: there is nowhere further
    /// out, and the bus turns that into a quiet rejection rather than a beep.
    pub fn back(&mut self) -> Vec<Event> {
        self.commit(|state| {
            if let Some(frame) = state.back.pop() {
                state.view = frame.view;
                state.selected = frame.selected;
                state.focus = frame.focus;
            }
        })
    }

    /// Report an account's connection state for the status line.
    pub fn set_connection(
        &mut self,
        account: AccountId,
        connection: ConnectionState,
    ) -> Vec<Event> {
        self.commit(|state| {
            state.connections.insert(account, connection);
        })
    }

    // -- Internals -------------------------------------------------------

    fn clear_position(&mut self) {
        self.selected.clear();
        self.focus = None;
    }

    fn push(&mut self, view: ViewMode) {
        self.back.push(Frame {
            view: std::mem::replace(&mut self.view, view),
            selected: self.selected.clone(),
            focus: self.focus,
        });
        if self.back.len() > Self::MAX_BACK_DEPTH {
            // Forget a step from the middle. The newest must survive — `Esc`
            // always undoes the drill-in just made — and so must the oldest,
            // which is where the session started: however deep the user has
            // wandered, enough `Esc`s have to get them back out to the list.
            self.back.remove(1);
        }
    }

    /// Apply a mutation and derive the events it caused.
    ///
    /// Diffing rather than emitting by hand is what makes "a change always
    /// emits an event" true by construction instead of by review.
    fn commit(&mut self, mutate: impl FnOnce(&mut AppState)) -> Vec<Event> {
        let before = self.clone();
        mutate(self);
        before.diff(self)
    }

    fn diff(&self, next: &AppState) -> Vec<Event> {
        let mut events = Vec::new();

        if self.account != next.account
            && let Some(account) = next.account
        {
            events.push(Event::MailboxesChanged { account });
        }
        if self.mailbox != next.mailbox
            && let Some(mailbox) = next.mailbox
        {
            events.push(Event::MessageListChanged { mailbox });
        }

        if self.view != next.view {
            // The composer owns the reading pane while it is open, so the
            // frontend needs the hand-over as its own event, not as a view
            // change it has to interpret.
            if let Some(draft) = self.composing()
                && next.composing() != Some(draft)
            {
                events.push(Event::ComposerClosed { draft });
            }
            if let Some(draft) = next.composing()
                && self.composing() != Some(draft)
            {
                events.push(Event::ComposerOpened { draft });
            }
            events.push(Event::ViewChanged {
                view: next.view.clone(),
            });
            if self.context() != next.context() {
                events.push(Event::ContextChanged {
                    context: next.context(),
                });
            }
        }

        // The focus is part of the selection as far as the UI is concerned:
        // it is the row wearing the key hints.
        if self.selected != next.selected || self.focus != next.focus {
            events.push(Event::SelectionChanged {
                messages: next.selected.clone(),
            });
        }

        for (account, state) in &next.connections {
            if self.connections.get(account) != Some(state) {
                events.push(Event::ConnectionChanged {
                    account: *account,
                    state: *state,
                });
            }
        }

        events
    }
}

/// The state as the command bus holds it: one owner, many handlers.
///
/// Handlers clone this into their closures and mutate through
/// [`update`](SharedState::update), which emits whatever the change produced.
/// The lock is never held across an `.await` — mutations are synchronous — so
/// it cannot deadlock the bus.
#[derive(Debug, Clone, Default)]
pub struct SharedState(Arc<Mutex<AppState>>);

impl SharedState {
    /// Share a state.
    pub fn new(state: AppState) -> Self {
        SharedState(Arc::new(Mutex::new(state)))
    }

    /// Read something out of the state.
    pub fn read<R>(&self, with: impl FnOnce(&AppState) -> R) -> R {
        with(&self.lock())
    }

    /// A copy to hand to something that must not hold the lock.
    pub fn snapshot(&self) -> AppState {
        self.lock().clone()
    }

    /// Mutate the state and emit the events the change produced.
    ///
    /// Returns how many events were emitted; zero means nothing changed,
    /// which is usually a [`CommandError::rejected`](crate::CommandError)
    /// rather than a failure.
    pub fn update(
        &self,
        events: &EventSink,
        mutate: impl FnOnce(&mut AppState) -> Vec<Event>,
    ) -> usize {
        let produced = mutate(&mut self.lock());
        let count = produced.len();
        for event in produced {
            events.emit(event);
        }
        count
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, AppState> {
        // A panicking handler must not take the application's state with it;
        // the bus already reported the panic as an error event.
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_reader_selects_what_it_shows() {
        let mut state = AppState::new();
        state.select(vec![MessageId::new(1)], Some(MessageId::new(1)));

        state.open_message(MessageId::new(9));

        assert_eq!(state.selection(), [MessageId::new(9)]);
        assert_eq!(state.focus(), Some(MessageId::new(9)));
    }

    #[test]
    fn overflowing_the_stack_keeps_both_ends_of_it() {
        let mut state = AppState::new();
        let last = AppState::MAX_BACK_DEPTH as i64 + 5;
        for id in 0..=last {
            state.open_thread(ThreadId::new(id));
        }

        assert_eq!(state.back_depth(), AppState::MAX_BACK_DEPTH);
        state.back();
        assert_eq!(
            *state.view(),
            ViewMode::Thread {
                thread: ThreadId::new(last - 1)
            },
            "Esc undoes the most recent drill-in"
        );

        while !state.back().is_empty() {}
        assert_eq!(*state.view(), ViewMode::List, "Esc always gets you out");
    }
}
