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
    /// One conversation, stacked in the reading pane.
    ///
    /// Was `Thread`, when a thread meant a column that replaced the list
    /// (#1003). The list is never anything but the list now; what changes is
    /// what the reading pane holds — one message, or all of a
    /// conversation's.
    Conversation {
        /// The conversation on screen.
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
            ViewMode::Conversation { .. } => Context::Conversation,
            ViewMode::Reader { .. } => Context::Reader,
            ViewMode::Search { .. } => Context::Search,
            ViewMode::Composer { .. } => Context::Composer,
        }
    }
}

/// What an action will hit.
///
/// Deliberately not a `Vec<MessageId>`. docs/PRODUCT.md §18 says a mailbox is never
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
    /// Every message the list is showing, less the ones taken back out.
    Everything {
        /// What the list is a view of — a folder, or a smart folder.
        scope: ViewScope,
        /// Rows the user removed from the selection.
        except: Vec<MessageId>,
    },
    /// Every message in each of these threads — a unified group (#184).
    Threads(Vec<ThreadId>),
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
        /// The account those messages belong to.
        account: AccountId,
        /// The mailbox those messages are in now, when they are all in one.
        from: Option<MailboxId>,
    },
}

/// What the message list is a view of.
///
/// A real folder and a smart folder are both things "everything" can be
/// relative to, and until #52 only the first could be: app state held an
/// `Option<MailboxId>`, so `Ctrl+A` in Flagged resolved to nothing and every
/// bulk verb rejected with "Nothing selected".
///
/// The asymmetry that made that happen is still here and is still right —
/// [`ViewScope::mailbox`] answers `None` for a smart folder, because a smart
/// folder is not somewhere a message can be *put*. What changed is that the
/// scope itself survives, so a predicate can be about it.
///
/// # Why the aggregate carries a list and the others do not
///
/// `Unified` is the one variant whose identity is not a single id, because
/// the aggregate list can be showing fewer accounts than are configured: an
/// account Postio cannot currently reach is still *drawn* — its synced mail
/// is real mail, and ADR 0005 Q10 is emphatic that hiding it would be the
/// worse lie — but a whole-view selection made while it was away is not
/// about it. So the accounts the view could actually show are part of what
/// the scope **is**, fixed at the moment the gesture was made rather than
/// looked up when the verb runs (#811).
///
/// That is what makes two aggregates over different account sets different
/// views, which is the property that stops an account reconnecting between
/// the `Ctrl+A` and the `a` from silently joining a selection the user was
/// never shown. It also costs this type its `Copy`, which is the price of
/// the guarantee.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ViewScope {
    /// One folder, as the server has it.
    Mailbox(MailboxId),
    /// Everything flagged in an account, wherever it is filed.
    Flagged(AccountId),
    /// Every account the aggregate view could show when it was asked.
    Unified {
        /// Those accounts, in the sidebar's order. Never empty: a view with
        /// nothing in it is not something a selection can be relative to.
        accounts: Vec<AccountId>,
    },
}

impl ViewScope {
    /// The folder this scope names, when it names one.
    ///
    /// `None` for a smart folder, and load-bearing: a destination has to be a
    /// real mailbox, and a view assembled by a predicate is not one.
    pub fn mailbox(&self) -> Option<MailboxId> {
        match self {
            ViewScope::Mailbox(mailbox) => Some(*mailbox),
            ViewScope::Flagged(_) | ViewScope::Unified { .. } => None,
        }
    }

    /// The account this scope is within.
    ///
    /// `None` for a mailbox, whose account is the store's to answer — a
    /// `MailboxId` does not carry one — and `None` for the aggregate, which
    /// is within several and whose callers all want *the* one.
    /// [`ViewScope::accounts`] is the question the aggregate can answer.
    pub fn account(&self) -> Option<AccountId> {
        match self {
            ViewScope::Mailbox(_) | ViewScope::Unified { .. } => None,
            ViewScope::Flagged(account) => Some(*account),
        }
    }

    /// Every account a whole-view selection here is about.
    ///
    /// One for a smart folder, none for a folder — whose account the store
    /// answers from the folder — and the recorded list for the aggregate.
    pub fn accounts(&self) -> &[AccountId] {
        match self {
            ViewScope::Mailbox(_) => &[],
            ViewScope::Flagged(account) => std::slice::from_ref(account),
            ViewScope::Unified { accounts } => accounts,
        }
    }
}

/// One step of the back stack: where the user was, and where they were in it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Frame {
    view: ViewMode,
    selected: Selection,
    focus: Option<MessageId>,
}

/// How many accounts a view is about: one, or all of them.
///
/// The type itself lives in [`postio_model::AccountScope`] and is re-exported
/// here under the name #182 gave it. It moved down to `postio-model` in #186,
/// when search needed the same value and `postio-index` could not depend on
/// this crate: `AppState.scope` and `SearchRequest.account` are meant to be
/// the same answer to the same question, and two enums that must agree about
/// what "unified" means is exactly how they stop being.
///
/// Commands that need somewhere to put a message are unavailable in
/// [`Scope::Unified`] — see [`Requirement`](crate::registry::Requirement).
pub use postio_model::AccountScope as Scope;

/// The application's view of itself.
///
/// Mutated only from command handlers — the bus is the single writer, which is
/// what lets every mutation be a total order without a lock held across an
/// `.await`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AppState {
    scope: Scope,
    viewing: Option<ViewScope>,
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

    /// What the mail on screen belongs to.
    ///
    /// Replaces the old `account()`, deliberately rather than wrapping it:
    /// a consumer that wants "the one account" has to say
    /// `scope().account()` and, in saying it, decide what it means when
    /// there is not one (#182).
    pub fn scope(&self) -> Scope {
        self.scope
    }

    /// The mailbox the list is showing, if it is showing a real one.
    ///
    /// `None` in a smart folder — which is the answer every caller wants,
    /// because they all go on to use it as somewhere a message could be put.
    /// [`AppState::viewing`] is the one that says what the list is scoped to
    /// whether or not that is a folder.
    pub fn mailbox(&self) -> Option<MailboxId> {
        self.viewing.as_ref().and_then(ViewScope::mailbox)
    }

    /// What the list is a view of, folder or smart folder.
    pub fn viewing(&self) -> Option<&ViewScope> {
        self.viewing.as_ref()
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
            MessageTarget::Threads(threads) => Some(Resolved::Threads(threads.clone())),
            // Taken at its word for the same reason a named list of messages
            // is: undo built this, and it names exactly what it moved.
            MessageTarget::Batch {
                range,
                account,
                from,
            } => Some(Resolved::Batch {
                range: *range,
                account: *account,
                from: *from,
            }),
            MessageTarget::Messages(messages) if messages.is_empty() => None,
            MessageTarget::Messages(messages) => Some(Resolved::Messages(messages.clone())),
            MessageTarget::Selection => match &self.selected {
                // The scope, not the mailbox. A smart folder has no mailbox
                // and asking it for one is what made `Ctrl+A` in Flagged
                // resolve to nothing (#52).
                Selection::Everything { except } => Some(Resolved::Everything {
                    scope: self.viewing.clone()?,
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

    /// Every account the application has heard about, in a stable order.
    ///
    /// `connections` is keyed by [`AccountId`] in a `BTreeMap`, so this is
    /// ascending id — the order accounts were created in, which does not
    /// change when one is disabled or when another is added. That stability
    /// is load-bearing for [`next_scope`](Self::next_scope) and for the
    /// per-account hue the sidebar draws: a colour a person has learned must
    /// not move because a later account appeared.
    pub fn accounts(&self) -> Vec<AccountId> {
        self.connections.keys().copied().collect()
    }

    /// Move to the next scope: unified, then each account in turn, and round.
    ///
    /// What `g a` does. Cycling rather than a menu because the set is small
    /// and ordered, and because a keystroke has no argument to name a scope
    /// with — the sidebar's rows are the surface for going somewhere
    /// directly.
    ///
    /// With no accounts, or exactly one, this is a no-op rather than a
    /// pointless flicker between "unified" and the only account there is:
    /// they show the same mail, so switching would be a visible change that
    /// changes nothing.
    pub fn next_scope(&mut self) -> Vec<Event> {
        let accounts = self.accounts();
        if accounts.len() < 2 {
            return Vec::new();
        }
        match self.scope {
            Scope::Unified => self.open_account(accounts[0]),
            Scope::Account(current) => match accounts.iter().position(|id| *id == current) {
                Some(index) if index + 1 < accounts.len() => self.open_account(accounts[index + 1]),
                // Past the last account, or an account that has gone away
                // since the scope was set — both land back at unified, which
                // is the one scope that is always valid.
                _ => self.open_unified(),
            },
        }
    }

    /// How many steps `Esc` can still unwind.
    pub fn back_depth(&self) -> usize {
        self.back.len()
    }

    // -- Mutations -------------------------------------------------------

    /// Open an account, which resets the mailbox and the selection with it.
    pub fn open_account(&mut self, account: AccountId) -> Vec<Event> {
        self.commit(|state| {
            if state.scope == Scope::Account(account) {
                return;
            }
            state.scope = Scope::Account(account);
            // The old mailbox and rows belong to an account that is no longer
            // on screen; keeping them would let an action land on a message
            // the user cannot see.
            state.viewing = None;
            state.clear_position();
        })
    }

    /// Widen the view to every enabled account.
    ///
    /// Drops the mailbox for the same reason [`open_account`](Self::open_account)
    /// does: a folder belongs to one account, so it cannot survive a view
    /// that spans them all, and keeping it would let an action land somewhere
    /// the user can no longer see.
    pub fn open_unified(&mut self) -> Vec<Event> {
        self.commit(|state| {
            if state.scope == Scope::Unified {
                return;
            }
            state.scope = Scope::Unified;
            state.viewing = None;
            state.clear_position();
        })
    }

    /// Open a mailbox in the list, dropping a selection from the old one.
    pub fn open_mailbox(&mut self, mailbox: MailboxId) -> Vec<Event> {
        self.open_view(ViewScope::Mailbox(mailbox))
    }

    /// Open the account's Flagged view, dropping a selection from the old one.
    pub fn open_flagged(&mut self, account: AccountId) -> Vec<Event> {
        self.open_view(ViewScope::Flagged(account))
    }

    /// Point the list at `scope`, dropping a selection from wherever it was.
    ///
    /// A selection is relative to the list in view and does not survive a
    /// change of it — the list does the same, and a predicate carried across
    /// would name rows the user can no longer see.
    pub fn open_view(&mut self, scope: ViewScope) -> Vec<Event> {
        self.commit(|state| {
            if state.viewing.as_ref() == Some(&scope) {
                return;
            }
            state.viewing = Some(scope);
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
    pub fn open_conversation(&mut self, thread: ThreadId) -> Vec<Event> {
        self.commit(|state| state.push(ViewMode::Conversation { thread }))
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

        if self.scope != next.scope
            && let Some(account) = next.scope.account()
        {
            events.push(Event::MailboxesChanged { account });
        }
        if self.viewing != next.viewing
            && let Some(mailbox) = next.mailbox()
            // A mailbox is only ever selected within an account, so the id
            // here is the mailbox's owner. The let-chain keeps the diff total:
            // a unified view holds no mailbox (both `open_unified` and
            // `open_account` clear it), so this emits nothing rather than
            // inventing an account for a folder that spans none.
            && let Some(account) = next.scope.account()
        {
            events.push(Event::MessageListChanged { account, mailbox });
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
                scope: ViewScope::Mailbox(MailboxId::new(4)),
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
    fn select_all_in_a_smart_folder_is_about_the_smart_folder() {
        // #52: `Feed::mailbox()` answers `None` in Flagged on purpose -- a
        // smart folder must not claim to be a mailbox -- and app state used
        // to hold nothing but a mailbox, so the predicate had nowhere to
        // live. `Ctrl+A` resolved to `None` and every bulk verb rejected
        // with "Nothing selected": honest, and not what was asked.
        let mut state = AppState::new();
        state.open_flagged(AccountId::new(3));
        state.select_all();
        state.toggle_selection(MessageId::new(9));

        assert_eq!(
            state.resolve(&MessageTarget::Selection),
            Some(Resolved::Everything {
                scope: ViewScope::Flagged(AccountId::new(3)),
                except: vec![MessageId::new(9)],
            })
        );
    }

    #[test]
    fn a_smart_folder_still_refuses_to_call_itself_a_mailbox() {
        // The asymmetry #52 had to preserve while fixing the rest of it: a
        // scope that can be selected over is still not somewhere a message
        // can be filed, and every caller reading `mailbox()` is asking the
        // second question.
        let mut state = AppState::new();
        state.open_flagged(AccountId::new(3));

        assert_eq!(state.mailbox(), None);
        assert_eq!(
            state.viewing(),
            Some(&ViewScope::Flagged(AccountId::new(3)))
        );
    }

    #[test]
    fn leaving_a_smart_folder_drops_its_selection() {
        // A predicate is relative to the list in view. Carried into a folder
        // it would name rows that are no longer on screen.
        let mut state = AppState::new();
        state.open_flagged(AccountId::new(3));
        state.select_all();
        state.open_mailbox(MailboxId::new(4));

        assert_eq!(
            state.resolve(&MessageTarget::Selection),
            None,
            "the whole-view predicate must not survive the change of view"
        );
    }

    #[test]
    fn select_all_in_the_unified_view_resolves_to_the_accounts_it_could_show() {
        // #811. `open_unified` leaves no `ViewScope` behind, so `Ctrl+A` in
        // the aggregate resolved to nothing and every bulk verb rejected with
        // "Nothing selected" -- a refusal the user did not earn. The
        // frontend mirrors the view it was actually showing, and the accounts
        // it could show travel with it.
        let mut state = AppState::new();
        state.open_unified();
        state.open_view(ViewScope::Unified {
            accounts: vec![AccountId::new(1), AccountId::new(2)],
        });
        state.select_all();

        assert_eq!(
            state.resolve(&MessageTarget::Selection),
            Some(Resolved::Everything {
                scope: ViewScope::Unified {
                    accounts: vec![AccountId::new(1), AccountId::new(2)],
                },
                except: Vec::new(),
            })
        );
    }

    #[test]
    fn a_unified_scope_names_neither_a_folder_nor_one_account() {
        // Both accessors are read as "somewhere a message could be put" and
        // "the one account this is within". An aggregate is neither, and
        // answering either would be a guess a caller would act on.
        let scope = ViewScope::Unified {
            accounts: vec![AccountId::new(1), AccountId::new(2)],
        };

        assert_eq!(scope.mailbox(), None);
        assert_eq!(scope.account(), None);
    }

    #[test]
    fn the_accounts_a_unified_selection_was_scoped_to_are_part_of_the_view() {
        // Two aggregates over different account sets are different views, so
        // one does not inherit the other's selection. That is what stops an
        // account reconnecting mid-gesture from joining a selection the user
        // was never shown.
        let mut state = AppState::new();
        state.open_view(ViewScope::Unified {
            accounts: vec![AccountId::new(1)],
        });
        state.select_all();

        state.open_view(ViewScope::Unified {
            accounts: vec![AccountId::new(1), AccountId::new(2)],
        });

        assert_eq!(
            state.resolve(&MessageTarget::Selection),
            None,
            "the predicate must not survive the view widening under it"
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
            state.open_conversation(ThreadId::new(id));
        }

        assert_eq!(state.back_depth(), AppState::MAX_BACK_DEPTH);
        state.back();
        assert_eq!(
            *state.view(),
            ViewMode::Conversation {
                thread: ThreadId::new(last - 1)
            },
            "Esc undoes the most recent drill-in"
        );

        while !state.back().is_empty() {}
        assert_eq!(*state.view(), ViewMode::List, "Esc always gets you out");
    }
}
