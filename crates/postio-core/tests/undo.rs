//! The undo stack: docs/PRODUCT.md §16 and canvas 3b — *Archived 12 messages — Undo*,
//! bound to `u`.
//!
//! Two things make that toast honest. Archiving twelve messages with twelve
//! keystrokes has to be *one* undoable unit, or `u` twelve times is the only
//! way back; and undo has to be instant and local, so it works on a train with
//! no signal and the server catches up later.
//!
//! Time is passed in rather than slept through: the coalescing window and the
//! expiry policy are exactly the things a test must be able to step across.

use std::sync::Arc;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use postio_core::bridge::Bridge;
use postio_core::dispatch::{CommandError, Dispatcher};
use postio_core::undo::{UndoEntry, UndoKind, UndoStack};
use postio_core::{AppState, Command, CommandId, ConnectionState, Event, MessageTarget};
use postio_model::{AccountId, MailboxId, MessageId};

const INBOX: MailboxId = MailboxId::new(7);

fn message(id: i64) -> MessageId {
    MessageId::new(id)
}

/// The undo of an archive: put it back where it came from.
fn archived(id: i64) -> UndoEntry {
    UndoEntry::new(
        UndoKind::Archive,
        vec![message(id)],
        vec![Command::Move {
            target: MessageTarget::Messages(vec![message(id)]),
            to: Some(INBOX),
        }],
    )
}

fn deleted(id: i64) -> UndoEntry {
    UndoEntry::new(
        UndoKind::Delete,
        vec![message(id)],
        vec![Command::Move {
            target: MessageTarget::Messages(vec![message(id)]),
            to: Some(INBOX),
        }],
    )
}

// -- Coalescing --------------------------------------------------------------

#[test]
fn one_archive_is_one_message() {
    let mut stack = UndoStack::new();
    stack.record(archived(1));

    let entry = stack.peek().expect("something to undo");
    assert_eq!(entry.description(), "Archived 1 message");
    assert_eq!(stack.depth(), 1);
}

#[test]
fn a_burst_of_archives_is_one_undoable_unit() {
    // Twelve keystrokes, one `u`. This is the toast docs/PRODUCT.md §16 asks for.
    let mut stack = UndoStack::new();
    let start = Instant::now();

    for (step, id) in (1..=12).enumerate() {
        stack.record_at(
            archived(id),
            start + Duration::from_millis(40 * step as u64),
        );
    }

    assert_eq!(stack.depth(), 1, "the burst coalesced");
    let entry = stack.peek().expect("something to undo");
    assert_eq!(entry.description(), "Archived 12 messages");
    assert_eq!(entry.messages().len(), 12);
    assert_eq!(entry.inverse().len(), 12, "every message has its way back");
}

#[test]
fn a_pause_ends_the_burst() {
    let mut stack = UndoStack::new();
    let start = Instant::now();

    stack.record_at(archived(1), start);
    stack.record_at(archived(2), start + UndoStack::COALESCE_WINDOW / 2);
    // Long enough that the user has moved on to a new thought.
    stack.record_at(
        archived(3),
        start + UndoStack::COALESCE_WINDOW * 2 + Duration::from_millis(1),
    );

    assert_eq!(stack.depth(), 2);
    assert_eq!(stack.peek().unwrap().description(), "Archived 1 message");
}

#[test]
fn the_window_runs_from_the_last_action_not_the_first() {
    // Holding `a` down for a minute is still one gesture.
    let mut stack = UndoStack::new();
    let start = Instant::now();
    let step = UndoStack::COALESCE_WINDOW / 2;

    for id in 1..=20 {
        stack.record_at(archived(id), start + step * id as u32);
    }

    assert_eq!(stack.depth(), 1);
    assert_eq!(stack.peek().unwrap().messages().len(), 20);
}

#[test]
fn touching_the_same_message_twice_starts_a_new_unit() {
    // Within one unit every action is independent, which is what makes
    // replaying the inverses in recorded order correct. A second action on a
    // message already in the unit breaks that, so it starts a new one.
    let mut stack = UndoStack::new();
    let now = Instant::now();

    stack.record_at(archived(1), now);
    stack.record_at(archived(2), now);
    stack.record_at(archived(1), now);

    assert_eq!(stack.depth(), 2);
    assert_eq!(stack.peek().unwrap().messages(), [message(1)]);
}

#[test]
fn different_actions_never_coalesce() {
    // "Archived 3 messages" must never quietly include a delete.
    let mut stack = UndoStack::new();
    let now = Instant::now();

    stack.record_at(archived(1), now);
    stack.record_at(deleted(2), now);
    stack.record_at(archived(3), now);

    assert_eq!(stack.depth(), 3);
    assert_eq!(stack.peek().unwrap().description(), "Archived 1 message");
}

// -- Undoing -----------------------------------------------------------------

#[test]
fn undoing_hands_back_the_inverse_in_recorded_order() {
    let mut stack = UndoStack::new();
    let now = Instant::now();
    stack.record_at(archived(1), now);
    stack.record_at(archived(2), now);

    let entry = stack.undo_at(now).expect("something to undo");

    assert_eq!(entry.description(), "Archived 2 messages");
    assert_eq!(
        entry.inverse(),
        [
            Command::Move {
                target: MessageTarget::Messages(vec![message(1)]),
                to: Some(INBOX),
            },
            Command::Move {
                target: MessageTarget::Messages(vec![message(2)]),
                to: Some(INBOX),
            },
        ]
    );
    assert!(stack.is_empty(), "the unit came off the stack");
}

#[test]
fn undo_takes_one_unit_at_a_time() {
    let mut stack = UndoStack::new();
    let now = Instant::now();
    stack.record_at(archived(1), now);
    stack.record_at(deleted(2), now);

    assert_eq!(stack.undo_at(now).unwrap().kind(), UndoKind::Delete);
    assert_eq!(stack.undo_at(now).unwrap().kind(), UndoKind::Archive);
    assert_eq!(stack.undo_at(now), None);
}

#[test]
fn there_is_nothing_to_undo_on_an_empty_stack() {
    let mut stack = UndoStack::new();
    assert!(stack.is_empty());
    assert_eq!(stack.undo(), None);
    assert_eq!(stack.peek(), None);
}

// -- Bounds and expiry -------------------------------------------------------

#[test]
fn the_stack_is_bounded_and_forgets_the_oldest() {
    let mut stack = UndoStack::new();
    let now = Instant::now();
    let spaced = |step: usize| now + UndoStack::COALESCE_WINDOW * 2 * (step as u32 + 1);

    for step in 0..UndoStack::MAX_DEPTH + 5 {
        stack.record_at(archived(step as i64), spaced(step));
    }

    assert_eq!(stack.depth(), UndoStack::MAX_DEPTH);
    // What survived is the recent end: undo walks back through the newest.
    let newest = stack.undo_at(spaced(UndoStack::MAX_DEPTH + 5)).unwrap();
    assert_eq!(
        newest.messages(),
        [message(UndoStack::MAX_DEPTH as i64 + 4)]
    );
}

#[test]
fn an_undo_the_user_forgot_about_expires() {
    // Putting back something archived an hour ago is a surprise, not a mercy,
    // and by then the server has almost certainly moved on.
    let mut stack = UndoStack::new();
    let now = Instant::now();
    stack.record_at(archived(1), now);

    let later = now + UndoStack::EXPIRY + Duration::from_secs(1);
    assert_eq!(stack.undo_at(later), None, "the entry aged out");
    assert!(stack.is_empty());
}

#[test]
fn a_fresh_entry_survives_an_expired_one_underneath_it() {
    let mut stack = UndoStack::new();
    let now = Instant::now();
    stack.record_at(archived(1), now);

    let later = now + UndoStack::EXPIRY + Duration::from_secs(1);
    stack.record_at(archived(2), later);

    assert_eq!(stack.depth(), 1, "the stale one was pruned on the way in");
    assert_eq!(stack.undo_at(later).unwrap().messages(), [message(2)]);
}

#[test]
fn the_policy_is_adjustable_for_the_tests_that_need_it() {
    let mut stack = UndoStack::with_policy(Duration::from_millis(10), Duration::from_secs(60), 2);
    let now = Instant::now();

    stack.record_at(archived(1), now);
    stack.record_at(archived(2), now + Duration::from_millis(20));
    stack.record_at(archived(3), now + Duration::from_millis(40));

    assert_eq!(stack.depth(), 2, "bounded at the depth we asked for");
}

// -- Through the bus, offline ------------------------------------------------

/// The local half of the world: an inbox, an archive, and the queue of
/// operations the sync engine will replay against the server when it can.
#[derive(Debug, Default)]
struct FakeMail {
    inbox: Mutex<Vec<MessageId>>,
    archive: Mutex<Vec<MessageId>>,
    queued: Mutex<Vec<Command>>,
    undo: Mutex<UndoStack>,
}

impl FakeMail {
    fn archive_now(&self, messages: &[MessageId]) {
        self.inbox
            .lock()
            .unwrap()
            .retain(|id| !messages.contains(id));
        self.archive
            .lock()
            .unwrap()
            .extend(messages.iter().copied());
        self.queued.lock().unwrap().push(Command::Archive {
            target: MessageTarget::Messages(messages.to_vec()),
        });
    }

    /// Applying an inverse is the same local-first shape: change the local
    /// truth now, enqueue the remote half, never await the network.
    fn apply_inverse(&self, command: &Command) {
        let Command::Move { target, .. } = command else {
            unreachable!("archive inverts to a move")
        };
        let MessageTarget::Messages(messages) = target else {
            unreachable!("the recorded inverse names its messages")
        };
        self.archive
            .lock()
            .unwrap()
            .retain(|id| !messages.contains(id));
        self.inbox.lock().unwrap().extend(messages.iter().copied());
        self.queued.lock().unwrap().push(command.clone());
    }
}

#[test]
fn archiving_twelve_messages_then_undoing_restores_all_twelve() {
    let mail = Arc::new(FakeMail {
        inbox: Mutex::new((1..=12).map(message).collect()),
        ..FakeMail::default()
    });
    // Nothing here reaches a server, and nothing awaits one: undo is a local
    // operation with a remote tail, so it works offline by construction.
    assert_eq!(
        AppState::new().connection(AccountId::new(1)),
        ConnectionState::Offline
    );

    let archiving = Arc::clone(&mail);
    let undoing = Arc::clone(&mail);
    let dispatcher = Dispatcher::builder()
        .on(CommandId::Archive, move |invocation| {
            let mail = Arc::clone(&archiving);
            async move {
                let Command::Archive { ref target } = invocation.command else {
                    unreachable!()
                };
                let MessageTarget::Messages(messages) = target else {
                    return Err(CommandError::rejected("no selection"));
                };
                mail.archive_now(messages);
                let entry = UndoEntry::new(
                    UndoKind::Archive,
                    messages.clone(),
                    vec![Command::Move {
                        target: MessageTarget::Messages(messages.clone()),
                        to: Some(INBOX),
                    }],
                );
                let mut undo = mail.undo.lock().unwrap();
                undo.record(entry);
                invocation.emit(Event::ActionCompleted {
                    description: undo.peek().expect("just recorded").description(),
                    undoable: true,
                });
                Ok(())
            }
        })
        .on(CommandId::Undo, move |invocation| {
            let mail = Arc::clone(&undoing);
            async move {
                let Some(entry) = mail.undo.lock().unwrap().undo() else {
                    return Err(CommandError::rejected("nothing to undo"));
                };
                // Inverses are applied directly rather than sent back through
                // the bus: replaying them as commands would record an undo of
                // the undo, and `u` `u` would toggle instead of walking back.
                for command in entry.inverse() {
                    mail.apply_inverse(command);
                }
                invocation.emit(Event::UndoPerformed {
                    description: entry.description(),
                });
                Ok(())
            }
        })
        .build();

    let (bridge, events) = Bridge::new(dispatcher).expect("the runtime starts");
    for id in 1..=12 {
        bridge
            .commands()
            .send(Command::Archive {
                target: MessageTarget::Messages(vec![message(id)]),
            })
            .expect("running");
    }
    bridge.commands().send(Command::Undo).expect("running");
    bridge.shutdown();

    let seen: Vec<Event> = std::iter::from_fn(|| events.try_next()).collect();

    // Locally: everything is back in the inbox.
    assert_eq!(mail.archive.lock().unwrap().len(), 0);
    assert_eq!(mail.inbox.lock().unwrap().len(), 12);
    // Remotely: every archive and every inverse is queued for the server.
    let queued = mail.queued.lock().unwrap();
    assert_eq!(queued.len(), 24, "12 archives and 12 ways back");
    assert!(
        queued[12..]
            .iter()
            .all(|command| matches!(command, Command::Move { .. })),
        "the inverses are queued for replay: {queued:?}"
    );

    // And the user was told, once, about all twelve.
    assert!(
        seen.contains(&Event::UndoPerformed {
            description: "Archived 12 messages".into(),
        }),
        "{seen:?}"
    );
    assert!(
        seen.iter().any(|event| matches!(
            event,
            Event::ActionCompleted {
                description,
                undoable: true,
            } if description == "Archived 12 messages"
        )),
        "the last toast counted the whole burst: {seen:?}"
    );
}
