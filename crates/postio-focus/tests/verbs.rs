//! Verbs, their aim, and where the cursor goes when mail leaves or comes
//! back (specs/009-focus-macos T041; contract invariant 4).
//!
//! What GTK's window did around `send`, asserted here so both frontends get
//! it: a verb goes to the selection when there is one and the cursor's row
//! otherwise, an archive hands the cursor to the survivor below, the
//! selection goes once acted on, toasts say what the host said, and an
//! undo puts the cursor on the row it brought back.

use postio_config::paths::Platform;
use postio_core::state::Selection;
use postio_core::{Command, CommandId, Event, MessageTarget};
use postio_focus::{
    Effect, FocusController, Input, Intent, Policy, Request, RowFacts, Rows, ToastKind,
};
use postio_model::{AccountId, MessageId, ThreadId};

struct List {
    rows: Vec<RowFacts>,
}

impl List {
    /// `ids` in order, each a conversation of its own unless `0` (a digest).
    fn of(ids: &[i64]) -> Self {
        List {
            rows: ids
                .iter()
                .map(|&id| RowFacts {
                    id: MessageId::new(id),
                    digest: id == 0,
                    threads: if id == 0 {
                        Vec::new()
                    } else {
                        vec![ThreadId::new(1000 + id)]
                    },
                    writes: false,
                })
                .collect(),
        }
    }
}

impl Rows for List {
    fn len(&self) -> u32 {
        self.rows.len() as u32
    }
    fn facts(&self, position: u32) -> Option<RowFacts> {
        self.rows.get(position as usize).cloned()
    }
    fn position_of(&self, message: MessageId) -> Option<u32> {
        self.rows
            .iter()
            .position(|row| row.id == message)
            .map(|at| at as u32)
    }
}

fn focus_on(rows: &List) -> FocusController {
    let mut focus = FocusController::new(Policy::for_platform(Platform::Apple));
    let _ = focus.handle(Input::Accounts(vec![AccountId::new(1)]));
    let _ = focus.landed(rows, true);
    focus
}

fn run(focus: &mut FocusController, command: CommandId, rows: &List) -> Vec<Effect> {
    focus.handle_on(Input::Command(command), rows)
}

fn shown(effects: &[Effect]) -> Vec<Intent> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Show(intent) => Some(intent.clone()),
            _ => None,
        })
        .collect()
}

fn asks(effects: &[Effect]) -> Vec<Request> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Ask(_, request) => Some(request.clone()),
            _ => None,
        })
        .collect()
}

fn cursor_of(effects: &[Effect]) -> Option<u32> {
    shown(effects)
        .into_iter()
        .rev()
        .find_map(|intent| match intent {
            Intent::Cursor { position, .. } => Some(position),
            _ => None,
        })
}

#[test]
fn a_verb_with_nothing_selected_goes_to_the_cursors_conversation() {
    let rows = List::of(&[1, 2, 3]);
    let mut focus = focus_on(&rows);
    let effects = run(&mut focus, CommandId::Archive, &rows);
    assert_eq!(
        asks(&effects),
        vec![Request::Send {
            command: Command::default_for(CommandId::Archive),
            aims: vec![MessageTarget::Threads(vec![ThreadId::new(1001)])],
            everything: None,
        }]
    );
}

#[test]
fn a_verb_goes_to_the_selection_and_lets_it_go() {
    let rows = List::of(&[1, 2, 3]);
    let mut focus = focus_on(&rows);
    let _ = run(&mut focus, CommandId::ToggleSelection, &rows);
    let _ = run(&mut focus, CommandId::NextMessage, &rows);
    let _ = run(&mut focus, CommandId::NextMessage, &rows);
    let _ = run(&mut focus, CommandId::ToggleSelection, &rows);
    let effects = run(&mut focus, CommandId::ToggleRead, &rows);
    assert_eq!(
        asks(&effects),
        vec![Request::Send {
            command: Command::default_for(CommandId::ToggleRead),
            aims: vec![MessageTarget::Threads(vec![
                ThreadId::new(1001),
                ThreadId::new(1003)
            ])],
            everything: None,
        }]
    );
    assert!(
        shown(&effects).contains(&Intent::Selection {
            selection: Selection::These(Vec::new()),
            summary: None
        }),
        "what it named has been acted on"
    );
}

#[test]
fn everything_selected_is_a_predicate_the_host_resolves() {
    let rows = List::of(&[1, 2, 3]);
    let mut focus = focus_on(&rows);
    let _ = run(&mut focus, CommandId::SelectAll, &rows);
    let effects = run(&mut focus, CommandId::Archive, &rows);
    let sent = asks(&effects);
    let [Request::Send { everything, .. }] = sent.as_slice() else {
        panic!("one send: {effects:?}");
    };
    assert_eq!(
        everything.as_ref().map(|all| all.accounts.clone()),
        Some(vec![AccountId::new(1)])
    );
}

#[test]
fn archiving_the_cursors_selected_row_hands_the_cursor_to_the_survivor_below() {
    // Rows 1 2 3 4; 2 and 3 selected, cursor on 2: the cursor stands on 4,
    // the first row after its own that stays (#468).
    let rows = List::of(&[1, 2, 3, 4]);
    let mut focus = focus_on(&rows);
    let _ = run(&mut focus, CommandId::NextMessage, &rows);
    let _ = run(&mut focus, CommandId::ExtendSelectionDown, &rows);
    let _ = run(&mut focus, CommandId::PrevMessage, &rows);
    let effects = run(&mut focus, CommandId::Archive, &rows);
    assert_eq!(cursor_of(&effects), Some(3));
}

#[test]
fn a_digest_row_alone_gives_a_mail_verb_nothing_to_aim_at() {
    let rows = List::of(&[0, 2]);
    let mut focus = focus_on(&rows);
    assert!(asks(&run(&mut focus, CommandId::Archive, &rows)).is_empty());
}

#[test]
fn undo_is_sent_as_it_is() {
    let rows = List::of(&[1]);
    let mut focus = focus_on(&rows);
    assert_eq!(
        asks(&run(&mut focus, CommandId::Undo, &rows)),
        vec![Request::Post(Command::Undo)]
    );
}

#[test]
fn accepting_on_a_row_answers_its_invitation_and_the_toast_waits_for_the_reply() {
    let rows = List::of(&[7, 8]);
    let mut focus = focus_on(&rows);
    let effects = run(&mut focus, CommandId::AcceptInvite, &rows);
    assert_eq!(
        asks(&effects),
        vec![Request::Post(Command::AcceptInvite {
            message: Some(MessageId::new(7))
        })]
    );
    let effects = focus.handle(Input::Event(Event::ActionCompleted {
        description: "Accepted".into(),
        undoable: true,
    }));
    let toasts = shown(&effects);
    let [Intent::Toast { kind, .. }] = toasts.as_slice() else {
        panic!("a toast: {effects:?}");
    };
    assert!(
        matches!(
            kind,
            ToastKind::Completed {
                undoable: true,
                seconds: Some(_)
            }
        ),
        "an answer's toast stays as long as its reply can be undone: {kind:?}"
    );
}

#[test]
fn the_host_s_words_are_the_toast() {
    let rows = List::of(&[1]);
    let mut focus = focus_on(&rows);
    let effects = focus.handle(Input::Event(Event::ActionCompleted {
        description: "Archived 3 messages".into(),
        undoable: true,
    }));
    assert_eq!(
        shown(&effects),
        vec![Intent::Toast {
            text: "Archived 3 messages".into(),
            kind: ToastKind::Completed {
                undoable: true,
                seconds: None
            }
        }]
    );
    let effects = focus.handle(Input::Event(Event::CommandRejected {
        command: CommandId::Undo.into(),
        reason: "nothing to undo".into(),
    }));
    assert_eq!(
        shown(&effects),
        vec![Intent::Toast {
            text: "nothing to undo".into(),
            kind: ToastKind::Notice
        }]
    );
}

#[test]
fn an_undo_puts_the_cursor_on_the_row_it_brought_back() {
    let before = List::of(&[1, 2, 3]);
    let mut focus = focus_on(&before);
    let _ = run(&mut focus, CommandId::NextMessage, &before);
    let _ = run(&mut focus, CommandId::Archive, &before);
    let after = List::of(&[1, 3]);
    let _ = focus.landed(&after, false);

    let effects = focus.handle(Input::Event(Event::UndoPerformed {
        description: "Archived 1 message".into(),
    }));
    assert!(matches!(
        shown(&effects).as_slice(),
        [Intent::Toast {
            kind: ToastKind::Undone,
            ..
        }]
    ));
    // The first re-read may not have it yet; the next does.
    assert_eq!(cursor_of(&focus.landed(&after, false)), None);
    let effects = focus.landed(&before, false);
    assert_eq!(cursor_of(&effects), Some(1), "on the row it brought back");
}

#[test]
fn the_controller_says_which_commands_are_its_own() {
    let focus = FocusController::new(Policy::for_platform(Platform::Apple));
    for id in [
        CommandId::NextMessage,
        CommandId::ToggleSelection,
        CommandId::SelectAll,
        CommandId::ToggleHasAction,
        CommandId::Archive,
        CommandId::Delete,
        CommandId::Undo,
        CommandId::AcceptInvite,
        CommandId::DismissMarker,
    ] {
        assert!(focus.answers(id), "{id} is the controller's");
    }
    for id in [CommandId::Reply, CommandId::Compose, CommandId::Settings] {
        assert!(!focus.answers(id), "{id} is a surface's, not the list's");
    }
}
