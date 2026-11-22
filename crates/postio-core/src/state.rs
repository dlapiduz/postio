//! The account, mailbox, selection and view live here and nowhere else; widgets repaint from the
//! events these mutations return.
//!
//! Mutators return events by diffing the state, so a change that emits nothing is impossible. The
//! back stack restores the exact prior position on `Esc`.

use std::sync::{Arc, Mutex};

use postio_model::{AccountId, DraftId, MailboxId, MessageId, OperationRange, ThreadId};
use serde::{Deserialize, Serialize};

use crate::bridge::EventSink;
use crate::{Context, Event, MessageTarget};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ViewMode {
    #[default]
    List,
    /// Was `Thread` before the conversation moved into the reading pane (#1003).
    Conversation {
        thread: ThreadId,
    },
    Reader {
        message: MessageId,
    },
    Composer {
        draft: DraftId,
    },
}

impl ViewMode {
    pub fn context(&self) -> Context {
        match self {
            ViewMode::List => Context::List,
            ViewMode::Conversation { .. } => Context::Conversation,
            ViewMode::Reader { .. } => Context::Reader,
            ViewMode::Composer { .. } => Context::Composer,
        }
    }
}

/// A predicate, not a `Vec<MessageId>`: a mailbox is never loaded into memory (docs/PRODUCT.md
/// §18), so the store resolves "select all" in one statement. `open_mailbox` clears it because
/// "everything" changes with the list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Selection {
    /// These messages specifically. Empty means nothing is selected.
    These(Vec<MessageId>),
    Everything {
        except: Vec<MessageId>,
    },
}

impl Default for Selection {
    fn default() -> Self {
        Selection::These(Vec::new())
    }
}

impl Selection {
    /// `Everything` is never empty by this measure: whether it covers rows is the store's question.
    pub fn is_empty(&self) -> bool {
        matches!(self, Selection::These(messages) if messages.is_empty())
    }

    pub fn is_everything(&self) -> bool {
        matches!(self, Selection::Everything { .. })
    }

    /// `None` for `Everything`: ask the store, which can answer without loading a mailbox.
    pub fn ids(&self) -> Option<&[MessageId]> {
        match self {
            Selection::These(messages) => Some(messages),
            Selection::Everything { .. } => None,
        }
    }

    pub fn contains(&self, message: MessageId) -> bool {
        match self {
            Selection::These(messages) => messages.contains(&message),
            Selection::Everything { except } => !except.contains(&message),
        }
    }

    pub fn toggle(&mut self, message: MessageId) {
        match self {
            Selection::These(messages) => match messages.iter().position(|id| *id == message) {
                Some(index) => {
                    messages.remove(index);
                }
                None => messages.push(message),
            },
            // Toggling inside the predicate edits the exceptions rather than naming every other
            // row.
            Selection::Everything { except } => match except.iter().position(|id| *id == message) {
                Some(index) => {
                    except.remove(index);
                }
                None => except.push(message),
            },
        }
    }

    pub fn insert(&mut self, message: MessageId) {
        if !self.contains(message) {
            self.toggle(message);
        }
    }
}

/// A handler hands `Everything` to the store to turn into rows in one statement, not enumerated
/// ids.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolved {
    Messages(Vec<MessageId>),
    Everything {
        scope: ViewScope,
        except: Vec<MessageId>,
    },
    Threads(Vec<ThreadId>),
    Thread(ThreadId),
    /// How undoing a whole-mailbox action gets back: a query the whole way down.
    Batch {
        range: OperationRange,
        account: AccountId,
        from: Option<MailboxId>,
    },
}

/// Until #52 only a folder could be "everything"; `mailbox` answers `None` for a smart folder
/// because nothing can be filed there.
///
/// `Unified` carries the accounts it could show, fixed when the gesture is made, so an account
/// reconnecting between `Ctrl+A` and the verb cannot join a selection the user was never shown
/// (#811, ADR 0005 Q10). That costs the type its `Copy`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ViewScope {
    Mailbox(MailboxId),
    Flagged(AccountId),
    Unified {
        /// Sidebar order; never empty.
        accounts: Vec<AccountId>,
    },
    /// Its own scope, not `Unified`: mail held for a digest is not in it, and each row folds a
    /// conversation across accounts, so taking a row out takes out every copy.
    Focus {
        accounts: Vec<AccountId>,
    },
}

impl ViewScope {
    /// `None` for a smart folder: a destination has to be a real mailbox.
    pub fn mailbox(&self) -> Option<MailboxId> {
        match self {
            ViewScope::Mailbox(mailbox) => Some(*mailbox),
            ViewScope::Flagged(_) | ViewScope::Unified { .. } | ViewScope::Focus { .. } => None,
        }
    }

    /// `None` for a mailbox (the store knows its account) and for the aggregate; see `accounts`.
    pub fn account(&self) -> Option<AccountId> {
        match self {
            ViewScope::Mailbox(_) | ViewScope::Unified { .. } | ViewScope::Focus { .. } => None,
            ViewScope::Flagged(account) => Some(*account),
        }
    }

    /// One for a smart folder, none for a folder (the store answers from it), the recorded list for
    /// the aggregate.
    pub fn accounts(&self) -> &[AccountId] {
        match self {
            ViewScope::Mailbox(_) => &[],
            ViewScope::Flagged(account) => std::slice::from_ref(account),
            ViewScope::Unified { accounts } | ViewScope::Focus { accounts } => accounts,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefreshTarget {
    Folder(MailboxId),
    Inboxes(Vec<AccountId>),
}

impl ViewScope {
    /// A folder is itself; an aggregate is fed by its accounts' inboxes.
    pub fn refresh_target(&self) -> RefreshTarget {
        match self {
            ViewScope::Mailbox(mailbox) => RefreshTarget::Folder(*mailbox),
            ViewScope::Flagged(_) | ViewScope::Unified { .. } | ViewScope::Focus { .. } => {
                RefreshTarget::Inboxes(self.accounts().to_vec())
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Frame {
    view: ViewMode,
    selected: Selection,
    focus: Option<MessageId>,
}

/// Moved down to `postio-model` (#186) so `postio-index` can share it: `AppState.scope` and
/// `SearchRequest.account` must agree about "unified". Move is unavailable in `Scope::Unified`.
pub use postio_model::AccountScope as Scope;

/// What a verb aims with: scope, view, selection and focus. A frontend carries its own copy to a
/// host in another process (ADR 0041); the back stack stays behind.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StateSnapshot {
    scope: Scope,
    viewing: Option<ViewScope>,
    selected: Selection,
    focus: Option<MessageId>,
    view: ViewMode,
}

/// Mutated only from command handlers: the bus is the single writer, so mutations are totally
/// ordered without a lock across `.await`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AppState {
    scope: Scope,
    viewing: Option<ViewScope>,
    selected: Selection,
    focus: Option<MessageId>,
    view: ViewMode,
    back: Vec<Frame>,
}

impl AppState {
    /// Bounded so a long session does not grow without limit.
    pub const MAX_BACK_DEPTH: usize = 32;

    pub fn new() -> Self {
        AppState::default()
    }

    pub fn snapshot(&self) -> StateSnapshot {
        StateSnapshot {
            scope: self.scope,
            viewing: self.viewing.clone(),
            selected: self.selected.clone(),
            focus: self.focus,
            view: self.view.clone(),
        }
    }

    /// Silent: the host's copy is read by the verb and by nothing that repaints.
    pub fn adopt(&mut self, snapshot: StateSnapshot) {
        let StateSnapshot {
            scope,
            viewing,
            selected,
            focus,
            view,
        } = snapshot;
        self.scope = scope;
        self.viewing = viewing;
        self.selected = selected;
        self.focus = focus;
        self.view = view;
    }

    /// Replaces `account()`, so a consumer must decide what it means when there is not one (#182).
    pub fn scope(&self) -> Scope {
        self.scope
    }

    /// `None` in a smart folder; `viewing` says what the list is scoped to either way.
    pub fn mailbox(&self) -> Option<MailboxId> {
        self.viewing.as_ref().and_then(ViewScope::mailbox)
    }

    pub fn viewing(&self) -> Option<&ViewScope> {
        self.viewing.as_ref()
    }

    pub fn selection(&self) -> &Selection {
        &self.selected
    }

    pub fn focus(&self) -> Option<MessageId> {
        self.focus
    }

    /// An empty selection falls back to the focused row: reading mail marks nothing, so click then
    /// `a` must archive the message being read.
    ///
    /// `None` when there is nothing to act on, which a handler reports as `CommandError::rejected`.
    pub fn resolve(&self, target: &MessageTarget) -> Option<Resolved> {
        match target {
            MessageTarget::Thread(thread) => Some(Resolved::Thread(*thread)),
            MessageTarget::Threads(threads) => Some(Resolved::Threads(threads.clone())),
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
                // The scope, not the mailbox: a smart folder has no mailbox (#52).
                Selection::Everything { except } => Some(Resolved::Everything {
                    scope: self.viewing.clone()?,
                    except: except.clone(),
                }),
                Selection::These(messages) if !messages.is_empty() => {
                    Some(Resolved::Messages(messages.clone()))
                }
                Selection::These(_) => self.focus.map(|focus| Resolved::Messages(vec![focus])),
            },
        }
    }

    pub fn view(&self) -> &ViewMode {
        &self.view
    }

    pub fn context(&self) -> Context {
        self.view.context()
    }

    pub fn composing(&self) -> Option<DraftId> {
        match &self.view {
            ViewMode::Composer { draft } => Some(*draft),
            _ => None,
        }
    }

    pub fn back_depth(&self) -> usize {
        self.back.len()
    }

    pub fn open_mailbox(&mut self, mailbox: MailboxId) -> Vec<Event> {
        self.open_view(ViewScope::Mailbox(mailbox))
    }

    pub fn open_flagged(&mut self, account: AccountId) -> Vec<Event> {
        self.open_view(ViewScope::Flagged(account))
    }

    /// A selection is relative to the list in view; a predicate carried across would name rows the
    /// user cannot see.
    pub fn open_view(&mut self, scope: ViewScope) -> Vec<Event> {
        self.commit(|state| {
            if state.viewing.as_ref() == Some(&scope) {
                return;
            }
            state.viewing = Some(scope);
            state.clear_position();
        })
    }

    pub fn select(&mut self, messages: Vec<MessageId>, focus: Option<MessageId>) -> Vec<Event> {
        self.commit(|state| {
            state.selected = Selection::These(messages);
            state.focus = focus;
        })
    }

    /// Leaves the focus alone: toggling is about what an action hits, not where the keyboard is.
    pub fn toggle_selection(&mut self, message: MessageId) -> Vec<Event> {
        self.commit(|state| state.selected.toggle(message))
    }

    /// The frontend holds the list order, so this takes the row rather than a direction.
    pub fn extend_selection_to(&mut self, message: MessageId) -> Vec<Event> {
        self.commit(|state| {
            state.selected.insert(message);
            state.focus = Some(message);
        })
    }

    pub fn select_all(&mut self) -> Vec<Event> {
        self.commit(|state| state.selected = Selection::Everything { except: Vec::new() })
    }

    pub fn focus_on(&mut self, message: Option<MessageId>) -> Vec<Event> {
        self.commit(|state| state.focus = message)
    }

    pub fn clear_selection(&mut self) -> Vec<Event> {
        self.commit(AppState::clear_position)
    }

    pub fn open_conversation(&mut self, thread: ThreadId) -> Vec<Event> {
        self.commit(|state| state.push(ViewMode::Conversation { thread }))
    }

    pub fn open_message(&mut self, message: MessageId) -> Vec<Event> {
        self.commit(|state| {
            state.push(ViewMode::Reader { message });
            // The reader's message is the selection: an archive from the reading pane must not land
            // on the row behind it.
            state.selected = Selection::These(vec![message]);
            state.focus = Some(message);
        })
    }

    pub fn open_composer(&mut self, draft: DraftId) -> Vec<Event> {
        self.commit(|state| state.push(ViewMode::Composer { draft }))
    }

    /// Returns no events at the top of the stack; the bus turns that into a quiet rejection.
    pub fn back(&mut self) -> Vec<Event> {
        self.commit(|state| {
            if let Some(frame) = state.back.pop() {
                state.view = frame.view;
                state.selected = frame.selected;
                state.focus = frame.focus;
            }
        })
    }

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
            // Forget a middle step: `Esc` must still undo the newest drill-in and reach the oldest,
            // the session's start.
            self.back.remove(1);
        }
    }

    /// Diffing makes "a change always emits an event" true by construction.
    fn commit(&mut self, mutate: impl FnOnce(&mut AppState)) -> Vec<Event> {
        let before = self.clone();
        mutate(self);
        before.diff(self)
    }

    fn diff(&self, next: &AppState) -> Vec<Event> {
        let mut events = Vec::new();

        if self.viewing != next.viewing
            && let Some(mailbox) = next.mailbox()
            // A unified view holds no mailbox, so this emits nothing rather than invent an account.
            && let Some(account) = next.scope.account()
        {
            events.push(Event::MessageListChanged { account, mailbox });
        }

        if self.view != next.view {
            // The hand-over is its own event so the frontend need not interpret a view change.
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

        events
    }
}

/// Handlers clone this into closures. The lock is never held across an `.await`, so it cannot
/// deadlock the bus.
#[derive(Debug, Clone, Default)]
pub struct SharedState(Arc<Mutex<AppState>>);

impl SharedState {
    pub fn new(state: AppState) -> Self {
        SharedState(Arc::new(Mutex::new(state)))
    }

    pub fn read<R>(&self, with: impl FnOnce(&AppState) -> R) -> R {
        with(&self.lock())
    }

    pub fn snapshot(&self) -> AppState {
        self.lock().clone()
    }

    /// Returns how many events were emitted; zero means nothing changed.
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
    fn refreshing_an_aggregate_view_targets_every_account_it_draws_from() {
        let accounts = vec![AccountId::new(1), AccountId::new(2)];
        for scope in [
            ViewScope::Focus {
                accounts: accounts.clone(),
            },
            ViewScope::Unified {
                accounts: accounts.clone(),
            },
        ] {
            assert_eq!(
                scope.refresh_target(),
                RefreshTarget::Inboxes(accounts.clone())
            );
        }
        assert_eq!(
            ViewScope::Flagged(AccountId::new(3)).refresh_target(),
            RefreshTarget::Inboxes(vec![AccountId::new(3)])
        );
        assert_eq!(
            ViewScope::Mailbox(MailboxId::new(9)).refresh_target(),
            RefreshTarget::Folder(MailboxId::new(9))
        );
    }

    #[test]
    fn an_empty_selection_means_the_row_the_cursor_is_on() {
        // Click a message, press `a`: nothing was marked, so the focus must serve or the archive
        // hits nothing.
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
    fn a_snapshot_carries_a_selection_to_another_process_intact() {
        let mut frontend = AppState::new();
        frontend.open_mailbox(MailboxId::new(4));
        frontend.select_all();
        frontend.toggle_selection(MessageId::new(7));
        frontend.focus_on(Some(MessageId::new(8)));

        let wire = serde_json::to_string(&frontend.snapshot()).expect("serialises");
        let mut host = AppState::new();
        host.adopt(serde_json::from_str(&wire).expect("deserialises"));

        assert_eq!(
            host.resolve(&MessageTarget::Selection),
            frontend.resolve(&MessageTarget::Selection)
        );
        assert_eq!(host.focus(), Some(MessageId::new(8)));
        assert_eq!(host.mailbox(), Some(MailboxId::new(4)));
    }

    #[test]
    fn nothing_selected_and_nothing_focused_resolves_to_nothing() {
        assert_eq!(AppState::new().resolve(&MessageTarget::Selection), None);
        assert_eq!(
            AppState::new().resolve(&MessageTarget::Messages(Vec::new())),
            None
        );
    }

    #[test]
    fn a_named_target_is_taken_at_its_word() {
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
        let mut state = AppState::new();
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
        let scope = ViewScope::Unified {
            accounts: vec![AccountId::new(1), AccountId::new(2)],
        };

        assert_eq!(scope.mailbox(), None);
        assert_eq!(scope.account(), None);
    }

    #[test]
    fn the_accounts_a_unified_selection_was_scoped_to_are_part_of_the_view() {
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
