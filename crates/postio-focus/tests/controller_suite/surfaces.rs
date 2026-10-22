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
    Effect, FocusController, Host, Input, Intent, Policy, ReaderVerb, Request, RowFacts, Rows,
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
            host: Host::Own,
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
        vec![Intent::Composer {
            kind: postio_focus::ComposerKind::Draft,
            message: Some(MessageId::new(100))
        }]
    );
    // The composer is the controller's surface now (slice 7).
    assert_eq!(
        focus.key_context(),
        KeyContext::Composer,
        "no reader opened"
    );
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
    // message does not own goes on to the list's rules, exactly as it would
    // from the list -- which since slice 12 are the controller's: capture
    // (here with no vault, so it says how to name one) and the rule dialog.
    let rows = List::of(2, &[]);
    let mut focus = mac();
    let _ = reading(&mut focus, &rows);
    for command in [CommandId::CaptureTask, CommandId::CaptureNote] {
        assert!(focus.answers(command), "{command:?} is the list's rule");
        assert!(
            shown(&run(&mut focus, command, &rows))
                .iter()
                .any(|intent| matches!(
                    intent,
                    Intent::Toast { text, .. } if text == postio_ui::capture::NO_VAULT
                )),
            "{command:?} in the message does what it does on the list"
        );
        assert_eq!(focus.key_context(), KeyContext::Reader);
    }
    assert!(focus.answers(CommandId::DigestRule));
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
        vec![
            Intent::CloseSurface(SurfaceKind::KeyMap),
            Intent::KeyboardHome
        ]
    );
    let _ = focus.handle_on(Input::SurfaceOpened(SurfaceKind::Capture), &rows);
    assert_eq!(
        shown(&run(&mut focus, CommandId::Back, &rows)),
        vec![Intent::CloseSurface(SurfaceKind::Capture)]
    );
    // The composer's Back is the controller's since slice 7: it closes the
    // composer, keeping the draft.
    let _ = focus.handle_on(Input::SurfaceOpened(SurfaceKind::Composer), &rows);
    assert!(focus.answers(CommandId::Back));
    assert_eq!(
        shown(&run(&mut focus, CommandId::Back, &rows)),
        vec![
            Intent::CloseSurface(SurfaceKind::Composer),
            Intent::KeyboardHome
        ]
    );
}

// ---------------------------------------------------------------------------
// Slice 6 (T058, T059): the reader, as GTK adopts it.
// ---------------------------------------------------------------------------

/// The timer an effect list sets, when it sets one.
fn timer_of(effects: &[Effect]) -> Option<u64> {
    effects.iter().find_map(|effect| match effect {
        Effect::Timer { token, .. } => Some(*token),
        _ => None,
    })
}

#[test]
fn stepping_past_waits_for_the_list_and_gives_up_on_its_timer() {
    // An archive sets one timer to give up on. Landings that still hold the
    // row (the archive has not reached the store) are only waited out: the
    // step is the list letting the row go, not a count of re-reads.
    let rows = List::of(3, &[]);
    let mut focus = linux();
    let _ = reading(&mut focus, &rows);
    let effects = run(&mut focus, CommandId::Archive, &rows);
    assert!(timer_of(&effects).is_some(), "the give-up is a timer");
    for _ in 0..8 {
        assert_eq!(opened(&focus.landed(&rows, false)), None);
    }
    let effects = focus.landed(&rows.without(100), false);
    assert_eq!(
        opened(&effects),
        Some(MessageId::new(101)),
        "many landings later the row went, and the next one shows"
    );
}

#[test]
fn a_step_past_that_timed_out_leaves_the_message_where_it_was() {
    let rows = List::of(3, &[]);
    let mut focus = linux();
    let _ = reading(&mut focus, &rows);
    let token = timer_of(&run(&mut focus, CommandId::Archive, &rows)).expect("a give-up timer");
    let _ = focus.handle_on(Input::Timer(token), &rows);
    let effects = focus.landed(&rows.without(100), false);
    assert_eq!(opened(&effects), None, "the archive never landed in time");
    assert_eq!(focus.key_context(), KeyContext::Reader, "still reading");
}

#[test]
fn a_timer_that_is_not_the_step_pasts_does_nothing() {
    let rows = List::of(3, &[]);
    let mut focus = linux();
    let _ = reading(&mut focus, &rows);
    let token = timer_of(&run(&mut focus, CommandId::Archive, &rows)).expect("a timer");
    let _ = focus.handle_on(Input::Timer(token + 1), &rows);
    let effects = focus.landed(&rows.without(100), false);
    assert_eq!(opened(&effects), Some(MessageId::new(101)));
}

#[test]
fn opening_another_message_cancels_the_step_past() {
    // A key is never lost: `j` pressed while the archive settles shows the
    // next message, and the step does not drag the reader back.
    let rows = List::of(4, &[]);
    let mut focus = linux();
    let _ = reading(&mut focus, &rows);
    let _ = run(&mut focus, CommandId::Archive, &rows);
    let effects = run(&mut focus, CommandId::NextMessage, &rows);
    assert_eq!(opened(&effects), Some(MessageId::new(101)));
    let effects = focus.landed(&rows.without(100), false);
    assert_eq!(opened(&effects), None, "the reader moved on by itself");
}

#[test]
fn u_in_the_message_goes_back_to_the_list() {
    // Gmail's `u` (and `Left`): the message closes and the list is back,
    // even with the More menu or find up -- unlike Esc, it is not a rung.
    for platform in [mac(), linux()] {
        let rows = List::of(3, &[]);
        let mut focus = platform;
        let _ = reading(&mut focus, &rows);
        assert!(focus.answers(CommandId::PrevView));
        assert_eq!(
            focus.route(CommandId::PrevView),
            postio_focus::Route::Controller
        );
        let _ = focus.handle_on(
            Input::ReaderState {
                more_open: true,
                finding: true,
            },
            &rows,
        );
        let effects = run(&mut focus, CommandId::PrevView, &rows);
        assert!(
            shown(&effects).contains(&Intent::CloseSurface(SurfaceKind::Message)),
            "{:?}",
            shown(&effects)
        );
        assert_eq!(focus.key_context(), KeyContext::List);
    }
}

#[test]
fn u_on_the_list_is_not_the_controllers() {
    let rows = List::of(3, &[]);
    let mut focus = linux();
    let _ = focus.landed(&rows, true);
    assert!(shown(&run(&mut focus, CommandId::PrevView, &rows)).is_empty());
}

#[test]
fn first_and_last_in_the_message_show_the_first_and_last_message() {
    let rows = List::of(5, &[]);
    let mut focus = linux();
    let _ = reading(&mut focus, &rows);
    let effects = run(&mut focus, CommandId::LastMessage, &rows);
    assert_eq!(opened(&effects), Some(MessageId::new(104)));
    assert!(
        shown(&effects).contains(&Intent::Cursor {
            position: 4,
            to_top: false
        }),
        "the list's cursor goes with it"
    );
    assert_eq!(
        opened(&run(&mut focus, CommandId::LastMessage, &rows)),
        None,
        "already on the last one"
    );
    let effects = run(&mut focus, CommandId::FirstMessage, &rows);
    assert_eq!(opened(&effects), Some(MessageId::new(100)));
    assert_eq!(focus.key_context(), KeyContext::Reader, "still reading");
}

#[test]
fn the_cursor_moving_under_an_open_message_shows_its_row() {
    // Beside the list, a click moves the cursor and the pane follows.
    let rows = List::of(4, &[2]);
    let mut focus = linux();
    let _ = reading(&mut focus, &rows);
    let _ = focus.handle_on(Input::Point(1), &rows);
    let effects = focus.handle_on(Input::Follow, &rows);
    assert_eq!(opened(&effects), Some(MessageId::new(101)));
    // Already showing: nothing to read again.
    assert!(shown(&focus.handle_on(Input::Follow, &rows)).is_empty());
    // A draft is written, not read: passing over one closes the pane's
    // message and opens no composer.
    let _ = focus.handle_on(Input::Point(2), &rows);
    let effects = focus.handle_on(Input::Follow, &rows);
    assert!(
        shown(&effects).contains(&Intent::CloseSurface(SurfaceKind::Message)),
        "{:?}",
        shown(&effects)
    );
    assert!(
        !shown(&effects)
            .iter()
            .any(|intent| matches!(intent, Intent::Composer { .. }))
    );
}

#[test]
fn the_cursor_moving_with_nothing_open_reads_nothing() {
    let rows = List::of(3, &[]);
    let mut focus = linux();
    let _ = focus.landed(&rows, true);
    let _ = focus.handle_on(Input::Point(1), &rows);
    assert!(shown(&focus.handle_on(Input::Follow, &rows)).is_empty());
    assert_eq!(focus.reading(), None);
}

#[test]
fn a_message_the_frontend_showed_itself_follows_the_cursor_too() {
    // A hit or a digest's email is no list row: the controller was told the
    // surface, not the message. `j` past the end of what it walks moves the
    // list, and the follow shows the cursor's row.
    let rows = List::of(3, &[]);
    let mut focus = linux();
    let _ = focus.landed(&rows, true);
    let _ = focus.handle_on(Input::SurfaceOpened(SurfaceKind::Message), &rows);
    assert_eq!(focus.reading(), None);
    let _ = run(&mut focus, CommandId::NextMessage, &rows);
    let effects = focus.handle_on(Input::Follow, &rows);
    assert_eq!(opened(&effects), Some(MessageId::new(101)));
    assert_eq!(focus.reading(), Some(MessageId::new(101)));
}

#[test]
fn archive_and_delete_in_a_message_shown_apart_close_it_not_step_past() {
    // A hit or a digest reference is on no row that will leave: the message
    // surface closes with the verb, and no step-past timer waits on a row
    // that never goes. A hit the list holds, but the cursor is not on, is
    // the same.
    for (command, message) in [
        (CommandId::Archive, 999),
        (CommandId::Delete, 999),
        (CommandId::Archive, 101),
        (CommandId::Delete, 101),
    ] {
        let rows = List::of(3, &[]);
        let mut focus = linux();
        let _ = focus.landed(&rows, true);
        let _ = focus.handle_on(Input::SurfaceOpened(SurfaceKind::Message), &rows);
        let _ = focus.handle_on(Input::Shown(Some(MessageId::new(message))), &rows);
        let effects = run(&mut focus, command, &rows);
        assert!(
            asked(&effects).iter().any(|request| matches!(
                request,
                Request::Send {
                    command: Command::Archive { .. } | Command::Delete { .. },
                    ..
                }
            )),
            "{command:?} on {message} is sent"
        );
        assert!(
            shown(&effects).contains(&Intent::CloseSurface(SurfaceKind::Message)),
            "{command:?} on {message} closes the message"
        );
        assert_eq!(timer_of(&effects), None, "no step-past to wait on");
    }
}

#[test]
fn dismissing_the_marker_in_the_message_is_for_the_message() {
    let rows = List::of(3, &[]);
    let mut focus = linux();
    let _ = focus.landed(&rows, true);
    let _ = run(&mut focus, CommandId::ToggleSelection, &rows);
    let _ = run(&mut focus, CommandId::NextMessage, &rows);
    let _ = run(&mut focus, CommandId::OpenMessage, &rows);
    let effects = run(&mut focus, CommandId::DismissMarker, &rows);
    let posted: Vec<_> = asked(&effects)
        .into_iter()
        .filter_map(|request| match request {
            Request::Post(Command::DismissMarker { target, dismissed }) => {
                Some((target, dismissed))
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        posted,
        vec![(MessageTarget::Messages(vec![MessageId::new(101)]), true)],
        "the open message, not the selected first row"
    );
}
