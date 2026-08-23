//! Application state: one coherent answer to "what is the user looking at".
//!
//! Widgets hold no authoritative state of their own — they render what these
//! accessors say and repaint from the events these mutations return. The
//! sharpest requirement is canvas 3a's `t`/`Esc` round trip: drilling into a
//! thread and coming back must restore the *exact* prior position, which only
//! works if the position lives here rather than in whichever widget happened to
//! have it last.

use std::sync::Arc;

use postio_core::bridge::Bridge;
use postio_core::dispatch::{CommandError, Dispatcher};
use postio_core::state::{AppState, SharedState, ViewMode};
use postio_core::{Command, CommandId, ConnectionState, Context, Event};
use postio_model::{AccountId, DraftId, MailboxId, MessageId, ThreadId};

fn message(id: i64) -> MessageId {
    MessageId::new(id)
}

fn selection(state: &AppState) -> (Vec<MessageId>, Option<MessageId>) {
    (state.selection().to_vec(), state.focus())
}

/// A state parked in the inbox with three messages selected and the middle one
/// focused: the position a `t`/`Esc` round trip has to bring back intact.
fn in_the_inbox() -> AppState {
    let mut state = AppState::new();
    state.open_account(AccountId::new(1));
    state.open_mailbox(MailboxId::new(7));
    state.select(vec![message(2), message(3), message(4)], Some(message(3)));
    state
}

#[test]
fn a_fresh_state_is_an_empty_list() {
    let state = AppState::new();

    assert_eq!(*state.view(), ViewMode::List);
    assert_eq!(state.context(), Context::List);
    assert!(state.selection().is_empty());
    assert_eq!(state.focus(), None);
    assert_eq!(state.account(), None);
    assert_eq!(state.mailbox(), None);
    assert_eq!(state.search_query(), None);
}

#[test]
fn opening_a_mailbox_tells_the_list_to_reload() {
    let mut state = AppState::new();

    let events = state.open_mailbox(MailboxId::new(7));

    assert_eq!(state.mailbox(), Some(MailboxId::new(7)));
    assert!(
        events.contains(&Event::MessageListChanged {
            mailbox: MailboxId::new(7)
        }),
        "{events:?}"
    );
}

#[test]
fn switching_mailboxes_drops_a_selection_that_is_no_longer_visible() {
    // The selected rows belong to the mailbox that is going away; keeping them
    // would archive messages the user can no longer see.
    let mut state = in_the_inbox();

    let events = state.open_mailbox(MailboxId::new(9));

    assert!(state.selection().is_empty());
    assert_eq!(state.focus(), None);
    assert!(
        events.contains(&Event::SelectionChanged { messages: vec![] }),
        "{events:?}"
    );
}

#[test]
fn switching_accounts_reloads_the_mailbox_tree() {
    let mut state = in_the_inbox();

    let events = state.open_account(AccountId::new(2));

    assert_eq!(state.account(), Some(AccountId::new(2)));
    assert_eq!(state.mailbox(), None, "the old account's mailbox is gone");
    assert!(
        events.contains(&Event::MailboxesChanged {
            account: AccountId::new(2)
        }),
        "{events:?}"
    );
}

// -- The t / Esc round trip --------------------------------------------------

#[test]
fn drilling_into_a_thread_and_back_restores_the_exact_position() {
    let mut state = in_the_inbox();
    let before = selection(&state);

    let opened = state.open_thread(ThreadId::new(42));
    assert_eq!(
        *state.view(),
        ViewMode::Thread {
            thread: ThreadId::new(42)
        }
    );
    assert_eq!(state.context(), Context::Thread);
    assert!(
        opened.contains(&Event::ContextChanged {
            context: Context::Thread
        }),
        "{opened:?}"
    );

    // Move around inside the thread, so coming back has something to restore.
    state.select(vec![message(11)], Some(message(11)));

    let back = state.back();

    assert_eq!(*state.view(), ViewMode::List);
    assert_eq!(selection(&state), before, "the position came back exactly");
    assert!(
        back.contains(&Event::SelectionChanged {
            messages: before.0.clone()
        }),
        "{back:?}"
    );
}

#[test]
fn back_unwinds_one_step_at_a_time() {
    // list -> thread -> message, then Esc, Esc: the reader returns to the
    // thread it was opened from, not all the way out to the list.
    let mut state = in_the_inbox();
    state.open_thread(ThreadId::new(42));
    state.select(vec![message(11)], Some(message(11)));
    state.open_message(message(11));

    assert_eq!(state.context(), Context::Reader);

    state.back();
    assert_eq!(
        *state.view(),
        ViewMode::Thread {
            thread: ThreadId::new(42)
        }
    );
    assert_eq!(
        state.focus(),
        Some(message(11)),
        "the read message stays put"
    );

    state.back();
    assert_eq!(*state.view(), ViewMode::List);
    assert_eq!(state.focus(), Some(message(3)));
}

#[test]
fn back_from_the_list_changes_nothing() {
    // There is nowhere further out; the bus turns this into a quiet rejection.
    let mut state = in_the_inbox();
    let before = state.clone();

    let events = state.back();

    assert!(events.is_empty(), "{events:?}");
    assert_eq!(state, before);
}

#[test]
fn the_back_stack_is_bounded() {
    // Drill in and out forever without the stack growing forever.
    let mut state = in_the_inbox();
    for id in 0..1_000 {
        state.open_thread(ThreadId::new(id));
    }

    assert!(state.back_depth() <= AppState::MAX_BACK_DEPTH);

    // However deep the wandering went, enough `Esc`s still get the user out.
    while !state.back().is_empty() {}
    assert_eq!(*state.view(), ViewMode::List);
}

// -- Search and the composer -------------------------------------------------

#[test]
fn search_is_a_view_the_user_can_leave() {
    let mut state = in_the_inbox();
    let before = selection(&state);

    let events = state.open_search("from:ana has:attachment");

    assert_eq!(state.search_query(), Some("from:ana has:attachment"));
    assert_eq!(state.context(), Context::Search);
    assert!(
        events.contains(&Event::ContextChanged {
            context: Context::Search
        }),
        "{events:?}"
    );

    state.back();
    assert_eq!(state.search_query(), None);
    assert_eq!(selection(&state), before);
}

#[test]
fn refining_a_query_stays_in_search_and_still_announces_itself() {
    let mut state = in_the_inbox();
    state.open_search("from:a");

    let events = state.open_search("from:an");

    assert_eq!(state.search_query(), Some("from:an"));
    assert!(!events.is_empty(), "a changed query is a changed view");
    assert_eq!(state.back_depth(), 1, "typing does not deepen the stack");
}

#[test]
fn the_composer_takes_over_the_pane_and_gives_it_back() {
    // The canvas is explicit: compose is not a separate window.
    let mut state = in_the_inbox();
    let before = selection(&state);

    let opened = state.open_composer(DraftId::new(5));

    assert_eq!(state.context(), Context::Composer);
    assert!(
        opened.contains(&Event::ComposerOpened {
            draft: DraftId::new(5)
        }),
        "{opened:?}"
    );

    let closed = state.back();

    assert_eq!(*state.view(), ViewMode::List);
    assert_eq!(selection(&state), before);
    assert!(
        closed.contains(&Event::ComposerClosed {
            draft: DraftId::new(5)
        }),
        "{closed:?}"
    );
}

// -- The invariants -----------------------------------------------------------

#[test]
fn the_context_follows_the_view() {
    let cases = [
        (ViewMode::List, Context::List),
        (
            ViewMode::Thread {
                thread: ThreadId::new(1),
            },
            Context::Thread,
        ),
        (
            ViewMode::Reader {
                message: message(1),
            },
            Context::Reader,
        ),
        (ViewMode::Search { query: "a".into() }, Context::Search),
        (
            ViewMode::Composer {
                draft: DraftId::new(1),
            },
            Context::Composer,
        ),
    ];
    for (view, context) in cases {
        assert_eq!(view.context(), context, "{view:?}");
    }
}

#[test]
fn a_change_always_emits_an_event_and_a_no_op_never_does() {
    // Widgets repaint from events only. A silent mutation is a stale pane.
    type Mutation = fn(&mut AppState) -> Vec<Event>;
    let mutations: Vec<(&str, Mutation)> = vec![
        ("account", |state| state.open_account(AccountId::new(1))),
        ("mailbox", |state| state.open_mailbox(MailboxId::new(7))),
        ("select", |state| {
            state.select(vec![message(2)], Some(message(2)))
        }),
        ("focus", |state| state.focus_on(Some(message(2)))),
        ("thread", |state| state.open_thread(ThreadId::new(42))),
        ("message", |state| state.open_message(message(2))),
        ("search", |state| state.open_search("from:ana")),
        ("composer", |state| state.open_composer(DraftId::new(5))),
        ("connection", |state| {
            state.set_connection(AccountId::new(1), ConnectionState::Online)
        }),
        ("back", |state| state.back()),
        ("clear", |state| state.clear_selection()),
    ];

    for (name, mutate) in mutations {
        let mut state = in_the_inbox();

        let before = state.clone();
        let events = mutate(&mut state);
        assert_eq!(
            events.is_empty(),
            state == before,
            "`{name}` emitted {events:?} for a change of {}",
            state != before
        );

        // Applied twice, the second application is a no-op and stays silent —
        // except for the ones that are a step by construction.
        if !matches!(name, "thread" | "message" | "composer" | "back") {
            let again = mutate(&mut state);
            assert!(again.is_empty(), "`{name}` repeated itself: {again:?}");
        }
    }
}

#[test]
fn connection_state_is_per_account() {
    let mut state = AppState::new();
    let first = AccountId::new(1);
    let second = AccountId::new(2);

    let events = state.set_connection(first, ConnectionState::Connecting);
    assert_eq!(
        events,
        vec![Event::ConnectionChanged {
            account: first,
            state: ConnectionState::Connecting,
        }]
    );
    state.set_connection(second, ConnectionState::Failing);

    assert_eq!(state.connection(first), ConnectionState::Connecting);
    assert_eq!(state.connection(second), ConnectionState::Failing);
    assert_eq!(
        state.connection(AccountId::new(3)),
        ConnectionState::Offline,
        "an account we have not heard from is working locally"
    );
}

// -- Only the bus mutates it --------------------------------------------------

#[test]
fn state_changes_arrive_at_the_ui_as_events_from_the_bus() {
    // The wiring the application uses: handlers own the state, widgets own
    // nothing, and every repaint comes off the event stream.
    let state = SharedState::new(in_the_inbox());
    let for_thread = state.clone();
    let for_back = state.clone();

    let dispatcher = Dispatcher::builder()
        .on(CommandId::Thread, move |invocation| {
            let state = for_thread.clone();
            async move {
                state.update(&invocation.events(), |state| {
                    state.open_thread(ThreadId::new(42))
                });
                Ok(())
            }
        })
        .on(CommandId::Back, move |invocation| {
            let state = for_back.clone();
            async move {
                let events = state.update(&invocation.events(), AppState::back);
                if events == 0 {
                    return Err(CommandError::rejected("nowhere to go back to"));
                }
                Ok(())
            }
        })
        .build();

    let (bridge, events) = Bridge::new(dispatcher).expect("the runtime starts");
    for command in [
        Command::Thread { thread: None },
        Command::Back,
        Command::Back,
    ] {
        bridge.commands().send(command).expect("running");
    }
    bridge.shutdown();

    let seen: Vec<Event> = std::iter::from_fn(|| events.try_next()).collect();
    assert!(
        seen.contains(&Event::ContextChanged {
            context: Context::Thread
        }),
        "{seen:?}"
    );
    assert!(
        seen.contains(&Event::ContextChanged {
            context: Context::List
        }),
        "{seen:?}"
    );
    assert!(
        seen.iter().any(|event| matches!(
            event,
            Event::CommandRejected {
                command: CommandId::Back,
                ..
            }
        )),
        "the second Esc had nowhere to go: {seen:?}"
    );

    // And the shared state is where the truth ended up.
    state.read(|state| {
        assert_eq!(*state.view(), ViewMode::List);
        assert_eq!(state.focus(), Some(message(3)));
    });
}

#[test]
fn a_snapshot_can_be_read_without_holding_the_lock() {
    let state = SharedState::new(in_the_inbox());
    let snapshot: Arc<AppState> = Arc::new(state.snapshot());

    state.read(|live| assert_eq!(*live, *snapshot));
}
