use std::sync::Arc;

use postio_core::ActionId;
use postio_core::bridge::Bridge;
use postio_core::dispatch::{CommandError, Dispatcher};
use postio_core::state::{AppState, Scope, SharedState, ViewMode};
use postio_core::{Command, CommandId, Context, Event};
use postio_model::{AccountId, DraftId, MailboxId, MessageId, ThreadId};

fn message(id: i64) -> MessageId {
    MessageId::new(id)
}

fn selection(state: &AppState) -> (Vec<MessageId>, Option<MessageId>) {
    (
        state.selection().ids().unwrap_or_default().to_vec(),
        state.focus(),
    )
}

fn on_account(account: i64) -> AppState {
    let mut snapshot =
        serde_json::to_value(AppState::new().snapshot()).expect("a snapshot serialises");
    snapshot["scope"] =
        serde_json::to_value(Scope::Account(AccountId::new(account))).expect("a scope serialises");
    let mut state = AppState::new();
    state.adopt(serde_json::from_value(snapshot).expect("a snapshot deserialises"));
    state
}

fn in_the_inbox() -> AppState {
    let mut state = on_account(1);
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
    assert_eq!(state.scope().account(), None);
    assert_eq!(state.mailbox(), None);
}

#[test]
fn opening_a_mailbox_tells_the_list_to_reload() {
    let mut state = on_account(1);

    let events = state.open_mailbox(MailboxId::new(7));

    assert_eq!(state.mailbox(), Some(MailboxId::new(7)));
    assert!(
        events.contains(&Event::MessageListChanged {
            account: AccountId::new(1),
            mailbox: MailboxId::new(7)
        }),
        "{events:?}"
    );
}

#[test]
fn opening_a_mailbox_with_no_account_open_reloads_nothing() {
    let mut state = AppState::new();

    let events = state.open_mailbox(MailboxId::new(7));

    assert_eq!(state.mailbox(), Some(MailboxId::new(7)));
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, Event::MessageListChanged { .. })),
        "{events:?}"
    );
}

#[test]
fn switching_mailboxes_drops_a_selection_that_is_no_longer_visible() {
    let mut state = in_the_inbox();

    let events = state.open_mailbox(MailboxId::new(9));

    assert!(state.selection().is_empty());
    assert_eq!(state.focus(), None);
    assert!(
        events.contains(&Event::SelectionChanged {
            selection: postio_core::state::Selection::default()
        }),
        "{events:?}"
    );
}

#[test]
fn drilling_into_a_thread_and_back_restores_the_exact_position() {
    let mut state = in_the_inbox();
    let before = selection(&state);

    let opened = state.open_conversation(ThreadId::new(42));
    assert_eq!(
        *state.view(),
        ViewMode::Conversation {
            thread: ThreadId::new(42)
        }
    );
    assert_eq!(state.context(), Context::Conversation);
    assert!(
        opened.contains(&Event::ContextChanged {
            context: Context::Conversation
        }),
        "{opened:?}"
    );

    state.select(vec![message(11)], Some(message(11)));

    let back = state.back();

    assert_eq!(*state.view(), ViewMode::List);
    assert_eq!(selection(&state), before, "the position came back exactly");
    assert!(
        back.contains(&Event::SelectionChanged {
            selection: postio_core::state::Selection::These(before.0.clone())
        }),
        "{back:?}"
    );
}

#[test]
fn back_unwinds_one_step_at_a_time() {
    let mut state = in_the_inbox();
    state.open_conversation(ThreadId::new(42));
    state.select(vec![message(11)], Some(message(11)));
    state.open_message(message(11));

    assert_eq!(state.context(), Context::Reader);

    state.back();
    assert_eq!(
        *state.view(),
        ViewMode::Conversation {
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
    let mut state = in_the_inbox();
    let before = state.clone();

    let events = state.back();

    assert!(events.is_empty(), "{events:?}");
    assert_eq!(state, before);
}

#[test]
fn the_back_stack_is_bounded() {
    let mut state = in_the_inbox();
    for id in 0..1_000 {
        state.open_conversation(ThreadId::new(id));
    }

    assert!(state.back_depth() <= AppState::MAX_BACK_DEPTH);

    while !state.back().is_empty() {}
    assert_eq!(*state.view(), ViewMode::List);
}

#[test]
fn the_composer_takes_over_the_pane_and_gives_it_back() {
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

#[test]
fn the_context_follows_the_view() {
    let cases = [
        (ViewMode::List, Context::List),
        (
            ViewMode::Conversation {
                thread: ThreadId::new(1),
            },
            Context::Conversation,
        ),
        (
            ViewMode::Reader {
                message: message(1),
            },
            Context::Reader,
        ),
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
    type Mutation = fn(&mut AppState) -> Vec<Event>;
    let mutations: Vec<(&str, Mutation)> = vec![
        ("mailbox", |state| state.open_mailbox(MailboxId::new(7))),
        ("select", |state| {
            state.select(vec![message(2)], Some(message(2)))
        }),
        ("focus", |state| state.focus_on(Some(message(2)))),
        ("thread", |state| state.open_conversation(ThreadId::new(42))),
        ("message", |state| state.open_message(message(2))),
        ("composer", |state| state.open_composer(DraftId::new(5))),
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

        if !matches!(name, "thread" | "message" | "composer" | "back") {
            let again = mutate(&mut state);
            assert!(again.is_empty(), "`{name}` repeated itself: {again:?}");
        }
    }
}

#[test]
fn state_changes_arrive_at_the_ui_as_events_from_the_bus() {
    let state = SharedState::new(in_the_inbox());
    let for_open = state.clone();
    let for_back = state.clone();

    let dispatcher = Dispatcher::builder()
        .on(CommandId::OpenMessage, move |invocation| {
            let state = for_open.clone();
            async move {
                state.update(&invocation.events(), |state| {
                    state.open_conversation(ThreadId::new(42))
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
        Command::OpenMessage { message: None },
        Command::Back,
        Command::Back,
    ] {
        bridge.commands().send(command).expect("running");
    }
    bridge.shutdown();

    let seen: Vec<Event> = std::iter::from_fn(|| events.try_next()).collect();
    assert!(
        seen.contains(&Event::ContextChanged {
            context: Context::Conversation
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
                command: ActionId::Builtin(CommandId::Back),
                ..
            }
        )),
        "the second Esc had nowhere to go: {seen:?}"
    );

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

#[test]
fn a_fresh_state_is_unified_over_nothing_rather_than_an_absent_account() {
    let state = AppState::new();

    assert_eq!(
        state.scope(),
        Scope::Unified,
        "there is always a scope; unified over zero accounts is simply empty, \
         which is what a fresh install shows"
    );
    assert_eq!(
        state.scope().account(),
        None,
        "and `None` now means exactly one thing: not a single-account view"
    );
}

#[test]
fn one_account_still_behaves_as_it_always_did() {
    let mut state = on_account(1);
    state.open_mailbox(MailboxId::new(4));

    assert_eq!(state.scope(), Scope::Account(AccountId::new(1)));
    assert_eq!(state.mailbox(), Some(MailboxId::new(4)));
}
