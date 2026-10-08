//! The surfaces over the list and the open message, from the controller's
//! side (specs/009-focus-macos T054, T056, T058; contract invariants 5-7).
//!
//! What GTK's window did in `handle_key`, `key_context` and `reading_key`,
//! asserted here so both frontends get it: the keys are the top surface's,
//! Back closes what is on top before it touches the list, `j`/`k` in the
//! open message step the list behind it, a key the message does not own is
//! the list's (#1754), and the Mac keeps one secondary window at a time.

use postio_config::paths::Platform;
use postio_core::{Command, CommandId, MessageTarget};
use postio_focus::{
    Effect, FocusController, Input, Intent, Policy, ReaderVerb, Request, RowFacts, Rows,
    SurfaceKind,
};
use postio_model::{MessageId, ThreadId};
use postio_ui::keymap::KeyContext;

/// A list of conversations; `drafts` are rows that open in the composer.
struct List {
    rows: Vec<RowFacts>,
}

impl List {
    fn of(len: i64, drafts: &[i64]) -> Self {
        List {
            rows: (0..len)
                .map(|at| RowFacts {
                    id: MessageId::new(100 + at),
                    digest: false,
                    threads: vec![ThreadId::new(500 + at)],
                    writes: drafts.contains(&at),
                })
                .collect(),
        }
    }

    fn without(&self, gone: i64) -> Self {
        List {
            rows: self
                .rows
                .iter()
                .filter(|row| row.id != MessageId::new(gone))
                .cloned()
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

fn mac() -> FocusController {
    FocusController::new(Policy::for_platform(Platform::Apple))
}

fn linux() -> FocusController {
    FocusController::new(Policy::for_platform(Platform::Freedesktop))
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

fn asked(effects: &[Effect]) -> Vec<Request> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Ask(_, request) => Some(request.clone()),
            _ => None,
        })
        .collect()
}

fn opened(effects: &[Effect]) -> Option<MessageId> {
    shown(effects).into_iter().find_map(|intent| match intent {
        Intent::OpenMessage { message, .. } => Some(message),
        _ => None,
    })
}

fn run(focus: &mut FocusController, command: CommandId, rows: &List) -> Vec<Effect> {
    focus.handle_on(Input::Command(command), rows)
}

/// The list landed with the cursor on its first row, and `Return` opened it.
fn reading(focus: &mut FocusController, rows: &List) -> Vec<Effect> {
    let _ = focus.landed(rows, true);
    run(focus, CommandId::OpenMessage, rows)
}

#[test]
fn return_opens_the_cursors_message_where_the_list_has_it() {
    let rows = List::of(3, &[]);
    let mut focus = mac();
    let _ = focus.landed(&rows, true);
    let _ = run(&mut focus, CommandId::NextMessage, &rows);
    let effects = run(&mut focus, CommandId::OpenMessage, &rows);
    assert_eq!(
        shown(&effects),
        vec![Intent::OpenMessage {
            message: MessageId::new(101),
            index: 1,
            total: 3,
        }]
    );
    assert_eq!(focus.key_context(), KeyContext::Reader);
}

#[test]
fn a_draft_opens_to_be_written_not_read() {
    let rows = List::of(2, &[0]);
    let mut focus = mac();
    let effects = reading(&mut focus, &rows);
    assert_eq!(
        shown(&effects),
        vec![Intent::OpenDraft {
            message: MessageId::new(100)
        }]
    );
    assert_eq!(focus.key_context(), KeyContext::List, "no reader opened");
}

#[test]
fn the_key_context_is_the_top_surfaces() {
    let rows = List::of(2, &[]);
    let mut focus = linux();
    assert_eq!(focus.key_context(), KeyContext::List);
    for (kind, context) in [
        (SurfaceKind::Filtered, KeyContext::Filtered),
        (SurfaceKind::Bar, KeyContext::Search),
        (SurfaceKind::Digest, KeyContext::Digest),
        (SurfaceKind::Capture, KeyContext::Capture),
        (SurfaceKind::Composer, KeyContext::Composer),
    ] {
        let _ = focus.handle_on(Input::SurfaceOpened(kind), &rows);
        assert_eq!(focus.key_context(), context, "{kind:?} on top");
    }
    let _ = focus.handle_on(Input::SurfaceClosed(SurfaceKind::Composer), &rows);
    assert_eq!(
        focus.key_context(),
        KeyContext::Capture,
        "back to what was under it"
    );
}

#[test]
fn back_in_the_message_closes_more_then_find_then_the_message() {
    let rows = List::of(3, &[]);
    let mut focus = mac();
    let _ = reading(&mut focus, &rows);
    let _ = focus.handle_on(
        Input::ReaderState {
            more_open: true,
            finding: true,
        },
        &rows,
    );
    assert_eq!(
        shown(&run(&mut focus, CommandId::Back, &rows)),
        vec![Intent::Reader(ReaderVerb::CloseMore)]
    );
    let _ = focus.handle_on(
        Input::ReaderState {
            more_open: false,
            finding: true,
        },
        &rows,
    );
    assert_eq!(
        shown(&run(&mut focus, CommandId::Back, &rows)),
        vec![Intent::Reader(ReaderVerb::CloseFind)]
    );
    let _ = focus.handle_on(
        Input::ReaderState {
            more_open: false,
            finding: false,
        },
        &rows,
    );
    let _ = run(&mut focus, CommandId::ToggleSelection, &rows);
    let effects = run(&mut focus, CommandId::Back, &rows);
    assert_eq!(
        shown(&effects),
        vec![
            Intent::CloseSurface(SurfaceKind::Message),
            Intent::KeyboardHome
        ],
        "the message closes, and the selection is kept: Back took one rung"
    );
    assert_eq!(focus.key_context(), KeyContext::List);
    assert_eq!(
        focus.selection(),
        postio_core::state::Selection::These(vec![MessageId::new(100)]),
        "the selection stays"
    );
}

#[test]
fn j_in_the_message_steps_the_list_without_closing_it() {
    let rows = List::of(3, &[]);
    let mut focus = mac();
    let _ = reading(&mut focus, &rows);
    let effects = run(&mut focus, CommandId::NextMessage, &rows);
    let intents = shown(&effects);
    assert!(
        intents.contains(&Intent::Cursor {
            position: 1,
            to_top: false
        }),
        "the cursor moves behind the message: {intents:?}"
    );
    assert_eq!(opened(&effects), Some(MessageId::new(101)));
    assert!(
        !intents
            .iter()
            .any(|intent| matches!(intent, Intent::CloseSurface(_))),
        "the window is not closed and reopened"
    );
    let _ = run(&mut focus, CommandId::NextMessage, &rows);
    let effects = run(&mut focus, CommandId::NextMessage, &rows);
    assert_eq!(
        opened(&effects),
        None,
        "at the last row there is nothing next"
    );
}

#[test]
fn the_message_owns_its_reader_verbs() {
    let rows = List::of(2, &[]);
    let mut focus = mac();
    let _ = reading(&mut focus, &rows);
    for (command, verb) in [
        (CommandId::FindInMessage, ReaderVerb::FindInMessage),
        (CommandId::FindNext, ReaderVerb::FindNext),
        (CommandId::FindPrevious, ReaderVerb::FindPrevious),
        (CommandId::SwitchTreatment, ReaderVerb::SwitchTreatment),
        (CommandId::ViewSource, ReaderVerb::ViewSource),
        (CommandId::MoreActions, ReaderVerb::ShowMore),
        (CommandId::NextInConversation, ReaderVerb::StepThread(1)),
        (CommandId::PrevInConversation, ReaderVerb::StepThread(-1)),
    ] {
        assert!(focus.answers(command), "{command:?} is the message's");
        assert_eq!(
            shown(&run(&mut focus, command, &rows)),
            vec![Intent::Reader(verb)],
            "{command:?}"
        );
    }
    assert!(
        !mac().answers(CommandId::FindInMessage),
        "with no message open, find is not the list's"
    );
}

#[test]
fn a_key_the_message_does_not_own_is_the_lists() {
    // #1754: `t`, `n` and `d` did nothing in the open message. What the
    // message does not own is not the controller's either: it goes on to
    // the host's table, exactly as it would from the list.
    let rows = List::of(2, &[]);
    let mut focus = mac();
    let _ = reading(&mut focus, &rows);
    for command in [
        CommandId::CaptureTask,
        CommandId::CaptureNote,
        CommandId::DigestRule,
    ] {
        assert!(
            !focus.answers(command),
            "{command:?} falls through to the host, as on the list"
        );
    }
}

#[test]
fn a_verb_in_the_message_is_for_the_message_not_the_selection() {
    let rows = List::of(3, &[]);
    let mut focus = mac();
    let _ = focus.landed(&rows, true);
    let _ = run(&mut focus, CommandId::ToggleSelection, &rows);
    let _ = run(&mut focus, CommandId::NextMessage, &rows);
    let _ = run(&mut focus, CommandId::OpenMessage, &rows);
    let effects = run(&mut focus, CommandId::ToggleRead, &rows);
    let sent: Vec<_> = asked(&effects)
        .into_iter()
        .filter_map(|request| match request {
            Request::Send { aims, .. } => Some(aims),
            _ => None,
        })
        .collect();
    assert_eq!(
        sent,
        vec![vec![MessageTarget::Threads(vec![ThreadId::new(501)])]],
        "the open message's conversation, not the selected first row"
    );
}

#[test]
fn archive_in_the_message_moves_on_to_the_row_that_took_its_place() {
    let rows = List::of(3, &[]);
    let mut focus = mac();
    let _ = reading(&mut focus, &rows);
    let effects = run(&mut focus, CommandId::Archive, &rows);
    assert!(
        asked(&effects).iter().any(|request| matches!(
            request,
            Request::Send {
                command: Command::Archive { .. },
                ..
            }
        )),
        "the archive is sent"
    );
    // The list re-reads without the archived row.
    let after = rows.without(100);
    let effects = focus.landed(&after, false);
    assert_eq!(opened(&effects), Some(MessageId::new(101)), "the next one");
    assert_eq!(focus.key_context(), KeyContext::Reader, "still reading");
}

#[test]
fn archiving_the_last_message_steps_back_and_the_only_one_closes() {
    let rows = List::of(2, &[]);
    let mut focus = mac();
    let _ = reading(&mut focus, &rows);
    let _ = run(&mut focus, CommandId::NextMessage, &rows);
    let _ = run(&mut focus, CommandId::Archive, &rows);
    let effects = focus.landed(&rows.without(101), false);
    assert_eq!(opened(&effects), Some(MessageId::new(100)), "the one above");

    let one = List::of(1, &[]);
    let mut focus = mac();
    let _ = reading(&mut focus, &one);
    let _ = run(&mut focus, CommandId::Archive, &one);
    let effects = focus.landed(&one.without(100), false);
    assert!(
        shown(&effects).contains(&Intent::CloseSurface(SurfaceKind::Message)),
        "nothing left to read"
    );
}

#[test]
fn stepping_past_gives_up_rather_than_waiting_forever() {
    // The row never leaves (the archive failed): after a few landings the
    // message stays open on what it shows, instead of polling.
    let rows = List::of(3, &[]);
    let mut focus = mac();
    let _ = reading(&mut focus, &rows);
    let _ = run(&mut focus, CommandId::Archive, &rows);
    for _ in 0..8 {
        assert_eq!(opened(&focus.landed(&rows, false)), None);
    }
}

#[test]
fn on_the_mac_a_new_secondary_window_replaces_the_one_open() {
    let rows = List::of(2, &[]);
    let mut focus = mac();
    let _ = reading(&mut focus, &rows);
    let effects = focus.handle_on(Input::SurfaceOpened(SurfaceKind::Digest), &rows);
    assert_eq!(
        shown(&effects),
        vec![Intent::CloseSurface(SurfaceKind::Message)],
        "one window at a time (M4)"
    );
    assert_eq!(focus.key_context(), KeyContext::Digest);
}

#[test]
fn on_linux_dialogs_stack() {
    let rows = List::of(2, &[]);
    let mut focus = linux();
    let _ = reading(&mut focus, &rows);
    let effects = focus.handle_on(Input::SurfaceOpened(SurfaceKind::Digest), &rows);
    assert!(shown(&effects).is_empty(), "the message stays under it");
    let _ = focus.handle_on(Input::SurfaceClosed(SurfaceKind::Digest), &rows);
    assert_eq!(focus.key_context(), KeyContext::Reader);
}

#[test]
fn back_closes_a_dialog_and_the_key_map_closes_on_its_own_key() {
    let rows = List::of(2, &[]);
    let mut focus = mac();
    let _ = focus.handle_on(Input::SurfaceOpened(SurfaceKind::KeyMap), &rows);
    assert_eq!(
        shown(&run(&mut focus, CommandId::CheatSheet, &rows)),
        vec![Intent::CloseSurface(SurfaceKind::KeyMap)]
    );
    let _ = focus.handle_on(Input::SurfaceClosed(SurfaceKind::KeyMap), &rows);
    let _ = focus.handle_on(Input::SurfaceOpened(SurfaceKind::Capture), &rows);
    assert_eq!(
        shown(&run(&mut focus, CommandId::Back, &rows)),
        vec![Intent::CloseSurface(SurfaceKind::Capture)]
    );
    // The composer's Back is its own (it keeps the draft): not the
    // controller's to close.
    let _ = focus.handle_on(Input::SurfaceOpened(SurfaceKind::Composer), &rows);
    assert!(!focus.answers(CommandId::Back));
}
