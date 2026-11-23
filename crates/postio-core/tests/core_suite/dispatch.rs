use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use postio_core::bridge::{Bridge, EventStream};
use postio_core::dispatch::{CommandError, Dispatcher};
use postio_core::{Command, CommandId, Context, Event, MessageTarget, registry};
use postio_model::MessageId;

fn run(dispatcher: Dispatcher, commands: impl IntoIterator<Item = Command>) -> Vec<Event> {
    let (bridge, events) = Bridge::new(dispatcher).expect("the runtime starts");
    for command in commands {
        bridge.commands().send(command).expect("running");
    }
    bridge.shutdown();
    drain(&events)
}

fn drain(events: &EventStream) -> Vec<Event> {
    std::iter::from_fn(|| events.try_next()).collect()
}

fn message(id: i64) -> MessageId {
    MessageId::new(id)
}

#[test]
fn a_command_reaches_the_handler_registered_for_it() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let recorder = Arc::clone(&seen);

    let dispatcher = Dispatcher::builder()
        .on(CommandId::Archive, move |invocation| {
            let recorder = Arc::clone(&recorder);
            async move {
                recorder.lock().unwrap().push(invocation.command.id());
                invocation.emit(Event::ActionCompleted {
                    description: "Archived".into(),
                    undoable: true,
                });
                Ok(())
            }
        })
        .build();

    let events = run(dispatcher, [Command::default_for(CommandId::Archive)]);

    assert_eq!(*seen.lock().unwrap(), vec![CommandId::Archive]);
    assert!(matches!(
        events.as_slice(),
        [Event::ActionCompleted { undoable: true, .. }]
    ));
}

#[test]
fn key_palette_and_menu_paths_converge_on_one_handler() {
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&calls);

    let dispatcher = Dispatcher::builder()
        .on(CommandId::Archive, move |_invocation| {
            let counter = Arc::clone(&counter);
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        })
        .build();

    let from_key = Command::default_for(
        registry::lookup_binding(Context::List, "e")
            .expect("`e` is archive")
            .id,
    );
    let from_palette = Command::default_for(
        registry::all()
            .find(|spec| spec.title == "Archive")
            .expect("the palette lists archive")
            .id,
    );
    let from_menu = Command::default_for(
        registry::for_context(Context::Reader)
            .find(|spec| spec.id == CommandId::Archive)
            .expect("the menu offers archive")
            .id,
    );

    assert_eq!(from_key, from_palette);
    assert_eq!(from_palette, from_menu);

    run(dispatcher, [from_key, from_palette, from_menu]);
    assert_eq!(calls.load(Ordering::SeqCst), 3);
}

#[test]
fn one_handler_can_serve_several_commands() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let recorder = Arc::clone(&seen);

    let dispatcher = Dispatcher::builder()
        .on_each(
            [CommandId::Archive, CommandId::Delete, CommandId::Flag],
            move |invocation| {
                let recorder = Arc::clone(&recorder);
                async move {
                    recorder.lock().unwrap().push(invocation.command.id());
                    Ok(())
                }
            },
        )
        .build();

    run(
        dispatcher,
        [
            Command::default_for(CommandId::Archive),
            Command::default_for(CommandId::Delete),
            Command::default_for(CommandId::Flag),
        ],
    );

    assert_eq!(
        *seen.lock().unwrap(),
        vec![CommandId::Archive, CommandId::Delete, CommandId::Flag]
    );
}

#[test]
fn a_command_with_no_handler_is_rejected_out_loud() {
    let events = run(
        Dispatcher::builder().build(),
        [Command::default_for(CommandId::Undo)],
    );

    match events.as_slice() {
        [Event::CommandRejected { command, reason }] => {
            assert_eq!(*command, CommandId::Undo.into());
            assert!(!reason.is_empty(), "the rejection explains itself");
        }
        other => panic!("expected a rejection, got {other:?}"),
    }
}

#[test]
fn a_handler_that_declines_says_why() {
    let dispatcher = Dispatcher::builder()
        .on(CommandId::Undo, |_invocation| async move {
            Err(CommandError::rejected("nothing to undo"))
        })
        .build();

    let events = run(dispatcher, [Command::Undo]);

    match events.as_slice() {
        [Event::CommandRejected { command, reason }] => {
            assert_eq!(*command, CommandId::Undo.into());
            assert_eq!(reason, "nothing to undo");
        }
        other => panic!("expected a rejection, got {other:?}"),
    }
}

#[test]
fn a_handler_that_fails_surfaces_an_error_event() {
    let dispatcher = Dispatcher::builder()
        .on(CommandId::Send, |_invocation| async move {
            Err(CommandError::failed("the outbox is unwritable"))
        })
        .build();

    let events = run(dispatcher, [Command::Send]);

    match events.as_slice() {
        [Event::Error { message, .. }] => assert_eq!(message, "the outbox is unwritable"),
        other => panic!("expected an error, got {other:?}"),
    }
}

#[test]
fn a_panicking_handler_does_not_take_the_bus_down_with_it() {
    let recovered = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&recovered);

    let dispatcher = Dispatcher::builder()
        .on(CommandId::Send, |_invocation| async move {
            panic!("a bug in the send handler");
        })
        .on(CommandId::Refresh, move |_invocation| {
            let counter = Arc::clone(&counter);
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        })
        .build();

    let events = run(dispatcher, [Command::Send, Command::Refresh]);

    assert_eq!(
        counter_of(&events, |event| matches!(event, Event::Error { .. })),
        1,
        "the panic was reported: {events:?}"
    );
    assert_eq!(recovered.load(Ordering::SeqCst), 1, "the bus kept running");
}

fn counter_of(events: &[Event], predicate: impl Fn(&Event) -> bool) -> usize {
    events.iter().filter(|event| predicate(event)).count()
}

#[derive(Debug, Default)]
struct FakeMail {
    inbox: Mutex<Vec<MessageId>>,
    archive: Mutex<Vec<MessageId>>,
    selection: Mutex<Vec<MessageId>>,
    queued: Mutex<Vec<String>>,
}

impl FakeMail {
    fn with_inbox(ids: &[i64]) -> Arc<Self> {
        Arc::new(FakeMail {
            inbox: Mutex::new(ids.iter().copied().map(message).collect()),
            ..FakeMail::default()
        })
    }

    fn resolve(&self, target: &MessageTarget) -> Vec<MessageId> {
        match target {
            MessageTarget::Selection => self.selection.lock().unwrap().clone(),
            MessageTarget::Messages(messages) => messages.clone(),
            MessageTarget::Thread(_) | MessageTarget::Threads(_) => {
                self.inbox.lock().unwrap().clone()
            }
            MessageTarget::Batch { .. } => Vec::new(),
        }
    }
}

fn mail_dispatcher(mail: Arc<FakeMail>) -> Dispatcher {
    let select = Arc::clone(&mail);
    let archive = Arc::clone(&mail);
    let undo = Arc::clone(&mail);

    Dispatcher::builder()
        .on(CommandId::OpenMessage, move |invocation| {
            let mail = Arc::clone(&select);
            async move {
                let Command::OpenMessage { message: Some(id) } = invocation.command else {
                    return Err(CommandError::rejected("nothing focused"));
                };
                *mail.selection.lock().unwrap() = vec![id];
                invocation.emit(Event::SelectionChanged {
                    selection: postio_core::state::Selection::These(vec![id]),
                });
                Ok(())
            }
        })
        .on(CommandId::Archive, move |invocation| {
            let mail = Arc::clone(&archive);
            async move {
                let Command::Archive { ref target } = invocation.command else {
                    unreachable!("the bus routes by id")
                };
                let moved = mail.resolve(target);
                if moved.is_empty() {
                    return Err(CommandError::rejected("no messages selected"));
                }
                mail.inbox.lock().unwrap().retain(|id| !moved.contains(id));
                mail.archive.lock().unwrap().extend(moved.iter().copied());
                mail.queued
                    .lock()
                    .unwrap()
                    .push(format!("archive {}", moved.len()));
                invocation.emit(Event::MessagesRemoved {
                    account: postio_model::AccountId::new(1),
                    mailbox: postio_model::MailboxId::new(1),
                    messages: moved.clone(),
                });
                invocation.emit(Event::ActionCompleted {
                    description: format!("Archived {} messages", moved.len()),
                    undoable: true,
                });
                Ok(())
            }
        })
        .on(CommandId::Undo, move |invocation| {
            let mail = Arc::clone(&undo);
            async move {
                let restored: Vec<MessageId> = mail.archive.lock().unwrap().drain(..).collect();
                if restored.is_empty() {
                    return Err(CommandError::rejected("nothing to undo"));
                }
                mail.inbox.lock().unwrap().extend(restored.iter().copied());
                mail.queued
                    .lock()
                    .unwrap()
                    .push(format!("unarchive {}", restored.len()));
                invocation.emit(Event::UndoPerformed {
                    description: format!("Restored {} messages", restored.len()),
                });
                Ok(())
            }
        })
        .build()
}

#[test]
fn a_whole_workflow_runs_through_commands_alone() {
    let mail = FakeMail::with_inbox(&[1, 2, 3]);
    let events = run(
        mail_dispatcher(Arc::clone(&mail)),
        [
            Command::OpenMessage {
                message: Some(message(2)),
            },
            Command::Archive {
                target: MessageTarget::Selection,
            },
            Command::Undo,
        ],
    );

    assert_eq!(
        *mail.archive.lock().unwrap(),
        Vec::<MessageId>::new(),
        "undo emptied the archive"
    );
    assert_eq!(mail.inbox.lock().unwrap().len(), 3);
    assert_eq!(
        *mail.queued.lock().unwrap(),
        vec!["archive 1".to_string(), "unarchive 1".to_string()],
        "each local write enqueued its remote counterpart"
    );

    let descriptions: Vec<&Event> = events.iter().collect();
    assert!(
        matches!(descriptions[0], Event::SelectionChanged { .. }),
        "{descriptions:?}"
    );
    assert!(
        descriptions
            .iter()
            .any(|event| matches!(event, Event::ActionCompleted { undoable: true, .. })),
        "the archive offered an undo: {descriptions:?}"
    );
    assert!(
        matches!(descriptions.last(), Some(Event::UndoPerformed { .. })),
        "{descriptions:?}"
    );
}

#[test]
fn a_command_that_cannot_run_yet_is_rejected_rather_than_guessed_at() {
    let mail = FakeMail::with_inbox(&[1, 2, 3]);
    let events = run(
        mail_dispatcher(Arc::clone(&mail)),
        [Command::Archive {
            target: MessageTarget::Selection,
        }],
    );

    assert_eq!(mail.inbox.lock().unwrap().len(), 3, "nothing moved");
    match events.as_slice() {
        [Event::CommandRejected { reason, .. }] => assert_eq!(reason, "no messages selected"),
        other => panic!("expected a rejection, got {other:?}"),
    }
}

#[test]
fn dispatch_is_serialized_so_state_never_races() {
    let order = Arc::new(Mutex::new(Vec::new()));
    let recorder = Arc::clone(&order);

    let dispatcher = Dispatcher::builder()
        .on(CommandId::Flag, move |invocation| {
            let recorder = Arc::clone(&recorder);
            async move {
                let Command::Flag { ref target, .. } = invocation.command else {
                    unreachable!()
                };
                let MessageTarget::Messages(messages) = target else {
                    unreachable!()
                };
                let id = messages[0];
                tokio::task::yield_now().await;
                recorder.lock().unwrap().push(id.get());
                Ok(())
            }
        })
        .build();

    let commands: Vec<Command> = (1..=64)
        .map(|id| Command::Flag {
            target: MessageTarget::Messages(vec![message(id)]),
            flagged: Some(true),
        })
        .collect();
    run(dispatcher, commands);

    let seen = order.lock().unwrap().clone();
    assert_eq!(seen, (1..=64).collect::<Vec<i64>>());
}
