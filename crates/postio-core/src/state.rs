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

use postio_model::{AccountId, DraftId, MailboxId, MessageId, OperationRange, ThreadId};
use serde::{Deserialize, Serialize};

use crate::bridge::EventSink;
use crate::{ConnectionState, Context, Event, MessageTarget};

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

/// What an action will hit.
///
/// Deliberately not a `Vec<MessageId>`. spec.md §18 says a mailbox is never
/// loaded into memory, and "select all" in a 100,000-message folder would
/// defeat that with one keystroke if selecting meant naming every row. So the
/// whole-mailbox case is a *predicate* — everything the list is showing, minus
/// whatever has been taken back out — and the storage layer resolves it in one
/// statement when an action finally lands. Bulk archive of 50,000 messages is
/// then one `UPDATE` and one queued operation, not 50,000 of each.
///
/// The predicate is relative to the mailbox and query in view. It does not
/// survive a change of either: [`AppState::open_mailbox`] clears it, because
/// "everything" means something different the moment the list does.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Selection {
    /// These messages specifically. Empty means nothing is selected.
    These(Vec<MessageId>),
    /// Everything the list is showing, except these.
    Everything {
        /// Rows taken back out of the selection since it was made.
        except: Vec<MessageId>,
    },
}

impl Default for Selection {
    fn default() -> Self {
        Selection::These(Vec::new())
    }
}

impl Selection {
    /// Whether an action would hit nothing.
    ///
    /// [`Selection::Everything`] is never empty by this measure, even in an
    /// empty mailbox: whether it covers any rows is a question for the store,
    /// and answering it here would mean counting the thing the predicate
    /// exists to avoid counting.
    pub fn is_empty(&self) -> bool {
        matches!(self, Selection::These(messages) if messages.is_empty())
    }

    /// Whether this is the whole-mailbox predicate.
    pub fn is_everything(&self) -> bool {
        matches!(self, Selection::Everything { .. })
    }

    /// The messages, when they can be named.
    ///
    /// `None` for [`Selection::Everything`] — which is the point. A caller
    /// that needs the rows behind a predicate has to ask the store for them,
    /// which is the only place that can answer without loading a mailbox.
    pub fn ids(&self) -> Option<&[MessageId]> {
        match self {
            Selection::These(messages) => Some(messages),
            Selection::Everything { .. } => None,
        }
    }

    /// Whether `message` is in the selection.
    pub fn contains(&self, message: MessageId) -> bool {
        match self {
            Selection::These(messages) => messages.contains(&message),
            Selection::Everything { except } => !except.contains(&message),
        }
    }

    /// Add `message` if it is out, take it out if it is in.
    pub fn toggle(&mut self, message: MessageId) {
        match self {
            Selection::These(messages) => match messages.iter().position(|id| *id == message) {
                Some(index) => {
                    messages.remove(index);
                }
                None => messages.push(message),
            },
            // Inside the predicate, toggling edits the exceptions rather than
            // giving the predicate up: taking one row out of forty thousand
            // must not turn into naming the other 39,999.
            Selection::Everything { except } => match except.iter().position(|id| *id == message) {
                Some(index) => {
                    except.remove(index);
                }
                None => except.push(message),
            },
        }
    }

    /// Add `message`, leaving it alone if it is already in.
    pub fn insert(&mut self, message: MessageId) {
        if !self.contains(message) {
            self.toggle(message);
        }
    }
}

/// What a [`MessageTarget`] came out as, once app state had its say.
///
/// The predicate stays a predicate: a handler that receives
/// [`Resolved::Everything`] hands it to the store to turn into affected rows
/// in one statement, rather than being given a hundred thousand ids that
/// something had to enumerate first. That is the whole reason this is not
/// simply `Vec<MessageId>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolved {
    /// These messages, named.
    Messages(Vec<MessageId>),
    /// Every message in a mailbox, less the ones taken back out.
    Everything {
        /// The mailbox the predicate is about.
        mailbox: MailboxId,
        /// Rows the user removed from the selection.
        except: Vec<MessageId>,
    },
    /// Every message in a thread.
    Thread(ThreadId),
    /// Every message a run of queue rows named, and where they are now.
    ///
    /// The other half of the predicate story: [`Resolved::Everything`] is how a
    /// whole-mailbox action reaches the store, and this is how *undoing* one
    /// gets back. Both stay queries the whole way down.
    Batch {
        /// The queue rows the bulk action wrote.
        range: OperationRange,
        /// The mailbox those messages are in now.
        from: MailboxId,
    },
}

/// One step of the back stack: where the user was, and where they were in it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Frame {
    view: ViewMode,
    selected: Selection,
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
    selected: Selection,
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

    /// What an action would hit.
    pub fn selection(&self) -> &Selection {
        &self.selected
    }

    /// The focused row — the one carrying the key hints, and the position a
    /// round trip through a thread has to restore.
    pub fn focus(&self) -> Option<MessageId> {
        self.focus
    }

    /// What a command aimed at `target` would actually act on.
    ///
    /// # Why an empty selection is not "nothing"
    ///
    /// [`MessageTarget::Selection`] is the registry's default for every
    /// message action, because a keystroke says only which verb it meant. But
    /// the selection is *deliberate* — it is what the user marked with `x`,
    /// Ctrl-click or `Ctrl+A` — and reading mail one message at a time never
    /// marks anything. Pressing `a` there has to archive the message being
    /// read, which is the row the cursor is on.
    ///
    /// So an empty selection falls back to the focus. Without that fallback
    /// the daily case — click a message, press `a` — would archive nothing at
    /// all, silently, which is the single most likely way this whole design
    /// fails. The frontends depend on it: `postio-gtk`'s list deliberately
    /// clears the selection on a plain click for exactly this reason.
    ///
    /// Returns `None` when there is genuinely nothing to act on — no
    /// selection, no focused row — which a handler reports as
    /// `CommandError::rejected` rather than doing nothing quietly.
    pub fn resolve(&self, target: &MessageTarget) -> Option<Resolved> {
        match target {
            MessageTarget::Thread(thread) => Some(Resolved::Thread(*thread)),
            // Taken at its word for the same reason a named list of messages
            // is: undo built this, and it names exactly what it moved.
            MessageTarget::Batch { range, from } => Some(Resolved::Batch {
                range: *range,
                from: *from,
            }),
            MessageTarget::Messages(messages) if messages.is_empty() => None,
            MessageTarget::Messages(messages) => Some(Resolved::Messages(messages.clone())),
            MessageTarget::Selection => match &self.selected {
                Selection::Everything { except } => Some(Resolved::Everything {
                    mailbox: self.mailbox?,
                    except: except.clone(),
                }),
                Selection::These(messages) if !messages.is_empty() => {
                    Some(Resolved::Messages(messages.clone()))
                }
                // Nothing marked: the row being read is what "this" means.
                Selection::These(_) => self.focus.map(|focus| Resolved::Messages(vec![focus])),
            },
        }
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
            state.selected = Selection::These(messages);
            state.focus = focus;
        })
    }

    /// Add `message` to the selection, or take it out again.
    ///
    /// The `x` key. Leaves the focus alone: toggling is about what an action
    /// hits, not about where the keyboard is.
    pub fn toggle_selection(&mut self, message: MessageId) -> Vec<Event> {
        self.commit(|state| state.selected.toggle(message))
    }

    /// Extend the selection onto `message` and move the focus with it.
    ///
    /// `Shift+J` and `Shift+K`. Which row is next is the frontend's to know —
    /// it is the one holding the list order — so this takes the row rather
    /// than a direction.
    pub fn extend_selection_to(&mut self, message: MessageId) -> Vec<Event> {
        self.commit(|state| {
            state.selected.insert(message);
            state.focus = Some(message);
        })
    }

    /// Select everything the list is showing, without naming any of it.
    pub fn select_all(&mut self) -> Vec<Event> {
        self.commit(|state| state.selected = Selection::Everything { except: Vec::new() })
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
            state.selected = Selection::These(vec![message]);
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
        self.selected = Selection::default();
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
                selection: next.selected.clone(),
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
    fn an_empty_selection_means_the_row_the_cursor_is_on() {
        // The daily case, and the one that would fail silently: click a
        // message, press `a`. Nothing was marked, so "the selection" has to
        // mean the message being read or the archive hits nothing at all.
        let mut state = AppState::new();
        state.focus_on(Some(MessageId::new(9)));

        assert_eq!(
            state.resolve(&MessageTarget::Selection),
            Some(Resolved::Messages(vec![MessageId::new(9)]))
        );
    }

    #[test]
    fn a_deliberate_selection_wins_over_the_cursor() {
        let mut state = AppState::new();
        state.select(vec![MessageId::new(1), MessageId::new(2)], None);
        state.focus_on(Some(MessageId::new(9)));

        assert_eq!(
            state.resolve(&MessageTarget::Selection),
            Some(Resolved::Messages(vec![
                MessageId::new(1),
                MessageId::new(2)
            ])),
            "what the user marked, not where they happen to be looking"
        );
    }

    #[test]
    fn select_all_stays_a_predicate_all_the_way_to_the_handler() {
        // Resolving must not be the moment a hundred thousand ids appear:
        // the store turns this into affected rows in one statement.
        let mut state = AppState::new();
        state.open_mailbox(MailboxId::new(4));
        state.select_all();
        state.toggle_selection(MessageId::new(7));

        assert_eq!(
            state.resolve(&MessageTarget::Selection),
            Some(Resolved::Everything {
                mailbox: MailboxId::new(4),
                except: vec![MessageId::new(7)],
            })
        );
    }

    #[test]
    fn nothing_selected_and_nothing_focused_resolves_to_nothing() {
        // Which a handler reports as a rejection — a quiet hint — rather
        // than doing nothing and looking like a bug.
        assert_eq!(AppState::new().resolve(&MessageTarget::Selection), None);
        assert_eq!(
            AppState::new().resolve(&MessageTarget::Messages(Vec::new())),
            None
        );
    }

    #[test]
    fn a_named_target_is_taken_at_its_word() {
        // A hover action or a drop names its own rows, and app state must not
        // second-guess it: the user pointed at that message.
        let mut state = AppState::new();
        state.select(vec![MessageId::new(1)], Some(MessageId::new(1)));

        assert_eq!(
            state.resolve(&MessageTarget::Messages(vec![MessageId::new(42)])),
            Some(Resolved::Messages(vec![MessageId::new(42)]))
        );
        assert_eq!(
            state.resolve(&MessageTarget::Thread(ThreadId::new(5))),
            Some(Resolved::Thread(ThreadId::new(5)))
        );
    }

    #[test]
    fn a_predicate_with_no_mailbox_open_resolves_to_nothing() {
        // "Everything" is relative to the list in view. Without one there is
        // no query to hand the store, and inventing one would be a guess
        // about which mailbox the user meant.
        let mut state = AppState::new();
        state.select_all();

        assert_eq!(state.resolve(&MessageTarget::Selection), None);
    }

    #[test]
    fn the_reader_selects_what_it_shows() {
        let mut state = AppState::new();
        state.select(vec![MessageId::new(1)], Some(MessageId::new(1)));

        state.open_message(MessageId::new(9));

        assert_eq!(state.selection().ids(), Some(&[MessageId::new(9)][..]));
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
