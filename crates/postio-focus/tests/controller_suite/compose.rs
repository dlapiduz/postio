//! Writing and replying, from the controller's side (specs/009-focus-macos
//! T072; spec 007 US3, screens 05 and 06).
//!
//! What GTK's window and composer did -- `refuses_reply`, the draft opened
//! from a row, `offered_on_open_draft`, `settle_send`, the add-account
//! offer, and the composer's debounced autosave -- asserted here so the
//! Mac's composer behaves the same. The draft's words stay the toolkit's:
//! the controller decides when the composer opens, what it answers, when
//! what is written is saved, and what is said when it closes.

use std::time::Duration;

use chrono::{NaiveDate, TimeZone, Utc};
use postio_config::paths::Platform;
use postio_core::{Command, CommandId};
use postio_focus::{
    ComposerKind, Effect, FocusController, Input, Intent, Policy, Reply, Request, RowFacts, Rows,
    SurfaceKind, Ticket, ToastKind,
};
use postio_model::listing::{MessageSummary, ThreadSummary};
use postio_model::{AccountId, DraftId, DraftState, EmailAddress, MessageId, ThreadId};
use postio_ui::focus_list::FocusRow;
use postio_ui::keymap::KeyContext;

/// A list of conversations, each with the send state its latest message is
/// in: `None` for received mail.
struct List {
    states: Vec<Option<DraftState>>,
}

impl List {
    fn of(states: &[Option<DraftState>]) -> Self {
        List {
            states: states.to_vec(),
        }
    }

    /// `len` conversations of received mail.
    fn received(len: usize) -> Self {
        List {
            states: vec![None; len],
        }
    }
}

impl Rows for List {
    fn len(&self) -> u32 {
        self.states.len() as u32
    }
    fn facts(&self, position: u32) -> Option<RowFacts> {
        self.row(position).map(|row| RowFacts::of(&row))
    }
    fn position_of(&self, id: MessageId) -> Option<u32> {
        let at = id.get().checked_sub(100)?;
        (at >= 0 && (at as usize) < self.states.len()).then_some(at as u32)
    }
    fn row(&self, position: u32) -> Option<FocusRow> {
        let state = *self.states.get(position as usize)?;
        let at = Utc.with_ymd_and_hms(2026, 9, 28, 9, 0, 0).unwrap();
        let id = MessageId::new(100 + i64::from(position));
        let thread = ThreadId::new(500 + i64::from(position));
        Some(FocusRow::conversation(ThreadSummary {
            id: Some(thread),
            representative: MessageSummary {
                id,
                thread: Some(thread),
                from: Some(EmailAddress::new(Some("Ada"), "ada@example.com")),
                subject: Some("Quarterly review".to_owned()),
                preview: None,
                received_at: at,
                seen: false,
                flagged: false,
                answered: false,
                send_state: state,
                send_at: None,
                has_attachments: false,
                thread_count: 1,
                to: Vec::new(),
            },
            subject: Some("Quarterly review".to_owned()),
            participants: Vec::new(),
            message_count: 1,
            unread_count: 1,
            flagged: false,
            has_attachments: false,
            last_at: at,
            marker: None,
            copies: Vec::new(),
        }))
    }
}

/// A controller under `platform`, its clock stopped at 16:12 on a Monday.
fn controller(platform: Platform) -> FocusController {
    let mut focus = FocusController::new(Policy::for_platform(platform));
    let at = chrono::Local
        .from_local_datetime(
            &NaiveDate::from_ymd_opt(2026, 9, 28)
                .unwrap()
                .and_hms_opt(16, 12, 0)
                .unwrap(),
        )
        .unwrap();
    let _ = focus.handle(Input::Clock(Some(at)));
    focus
}

fn mac() -> FocusController {
    controller(Platform::Apple)
}

fn linux() -> FocusController {
    controller(Platform::Freedesktop)
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

fn asked(effects: &[Effect]) -> Vec<(Ticket, Request)> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Ask(ticket, request) => Some((*ticket, request.clone())),
            _ => None,
        })
        .collect()
}

/// The timers set, as `(token, after)`.
fn timers(effects: &[Effect]) -> Vec<(u64, Duration)> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Timer { token, after } => Some((*token, *after)),
            _ => None,
        })
        .collect()
}

/// The composition an `Intent::SaveDraft` names, if one was said.
fn saved(effects: &[Effect]) -> Option<u64> {
    shown(effects).into_iter().find_map(|intent| match intent {
        Intent::SaveDraft { composition } => Some(composition),
        _ => None,
    })
}

fn toasts(effects: &[Effect]) -> Vec<(String, ToastKind)> {
    shown(effects)
        .into_iter()
        .filter_map(|intent| match intent {
            Intent::Toast { text, kind } => Some((text, kind)),
            _ => None,
        })
        .collect()
}

fn run(focus: &mut FocusController, command: CommandId, rows: &List) -> Vec<Effect> {
    focus.handle_on(Input::Command(command), rows)
}

/// The list landed with the cursor on its first row.
fn landed(focus: &mut FocusController, rows: &List) {
    let _ = focus.landed(rows, true);
}

/// `c` on a landed list: the composer is open, with nothing written yet.
fn composing(focus: &mut FocusController, rows: &List) -> Vec<Effect> {
    landed(focus, rows);
    run(focus, CommandId::Compose, rows)
}

/// The first composition a controller opens.
const FIRST: u64 = 1;

#[test]
fn c_opens_a_new_message_in_the_composer() {
    for mut focus in [mac(), linux()] {
        let rows = List::received(2);
        let effects = composing(&mut focus, &rows);
        assert_eq!(
            shown(&effects),
            vec![Intent::Composer {
                kind: ComposerKind::New,
                message: None,
            }]
        );
        assert_eq!(focus.key_context(), KeyContext::Composer);
        assert!(focus.answers(CommandId::Back), "Esc is the controller's");
    }
}

#[test]
fn e_cap_e_and_f_answer_the_cursors_row() {
    for (command, kind) in [
        (CommandId::Reply, ComposerKind::Reply),
        (CommandId::ReplyAll, ComposerKind::ReplyAll),
        (CommandId::Forward, ComposerKind::Forward),
    ] {
        let rows = List::received(3);
        let mut focus = mac();
        landed(&mut focus, &rows);
        let _ = run(&mut focus, CommandId::NextMessage, &rows);
        assert!(focus.answers(command), "{command} is the controller's");
        let effects = run(&mut focus, command, &rows);
        assert_eq!(
            shown(&effects),
            vec![Intent::Composer {
                kind,
                message: Some(MessageId::new(101)),
            }],
            "{command} answers the row the cursor is on"
        );
    }
}

#[test]
fn from_the_open_message_they_answer_it_and_on_the_mac_replace_its_window() {
    let rows = List::received(3);
    let mut focus = mac();
    landed(&mut focus, &rows);
    let _ = run(&mut focus, CommandId::NextMessage, &rows);
    let _ = run(&mut focus, CommandId::OpenMessage, &rows);
    let effects = run(&mut focus, CommandId::ReplyAll, &rows);
    assert_eq!(
        shown(&effects),
        vec![
            Intent::CloseSurface(SurfaceKind::Message),
            Intent::Composer {
                kind: ComposerKind::ReplyAll,
                message: Some(MessageId::new(101)),
            },
        ],
        "one secondary window at a time (M4)"
    );
    assert_eq!(focus.key_context(), KeyContext::Composer);
    assert_eq!(focus.reading(), None, "the message is no longer open");
}

#[test]
fn on_linux_the_composer_stacks_over_the_open_message() {
    let rows = List::received(2);
    let mut focus = linux();
    landed(&mut focus, &rows);
    let _ = run(&mut focus, CommandId::OpenMessage, &rows);
    let effects = run(&mut focus, CommandId::Reply, &rows);
    assert_eq!(
        shown(&effects),
        vec![Intent::Composer {
            kind: ComposerKind::Reply,
            message: Some(MessageId::new(100)),
        }]
    );
    // Esc leaves the composer and the message is still there under it.
    let _ = run(&mut focus, CommandId::Back, &rows);
    assert_eq!(focus.key_context(), KeyContext::Reader);
}

#[test]
fn a_reply_to_mail_still_on_its_way_out_is_refused_in_the_shared_words() {
    for state in [
        DraftState::Editing,
        DraftState::Queued,
        DraftState::Sending,
        DraftState::Failed,
        DraftState::Unconfirmed,
    ] {
        for command in [CommandId::Reply, CommandId::ReplyAll] {
            let rows = List::of(&[Some(state)]);
            let mut focus = mac();
            landed(&mut focus, &rows);
            let effects = run(&mut focus, command, &rows);
            assert_eq!(
                toasts(&effects),
                vec![(
                    postio_ui::focus_target::NO_REPLY_TO_OUTGOING.to_owned(),
                    ToastKind::Notice
                )],
                "{command} on mail that is {state:?}"
            );
            assert!(
                !shown(&effects)
                    .iter()
                    .any(|intent| matches!(intent, Intent::Composer { .. })),
                "no composer opens"
            );
            assert_eq!(focus.key_context(), KeyContext::List);
        }
    }
}

#[test]
fn a_sent_message_and_a_forward_are_not_refused() {
    let rows = List::of(&[Some(DraftState::Sent)]);
    let mut focus = mac();
    landed(&mut focus, &rows);
    let effects = run(&mut focus, CommandId::Reply, &rows);
    assert!(
        matches!(
            shown(&effects).as_slice(),
            [Intent::Composer {
                kind: ComposerKind::Reply,
                ..
            }]
        ),
        "replying to your own sent mail answers its recipients"
    );

    let rows = List::of(&[Some(DraftState::Queued)]);
    let mut focus = mac();
    landed(&mut focus, &rows);
    let effects = run(&mut focus, CommandId::Forward, &rows);
    assert!(
        matches!(
            shown(&effects).as_slice(),
            [Intent::Composer {
                kind: ComposerKind::Forward,
                ..
            }]
        ),
        "a forward has a recipient of its own to find"
    );
}

#[test]
fn with_no_account_to_write_from_c_offers_add_account() {
    let rows = List::received(0);
    let mut focus = mac();
    let _ = focus.handle(Input::Accounts(Vec::new()));
    let effects = run(&mut focus, CommandId::Compose, &rows);
    assert_eq!(
        toasts(&effects),
        vec![(
            postio_ui::focus_target::NO_ACCOUNT_TO_WRITE_FROM.to_owned(),
            ToastKind::Offer {
                label: postio_ui::focus_target::ADD_ACCOUNT.to_owned(),
                command: CommandId::AddAccount,
            }
        )]
    );
    assert_eq!(focus.key_context(), KeyContext::List, "no composer opened");

    // With an account, `c` writes.
    let _ = focus.handle(Input::Accounts(vec![AccountId::new(1)]));
    let effects = run(&mut focus, CommandId::Compose, &rows);
    assert!(matches!(
        shown(&effects).as_slice(),
        [Intent::Composer {
            kind: ComposerKind::New,
            ..
        }]
    ));
}

#[test]
fn edits_within_the_quiet_period_coalesce_into_one_save() {
    let rows = List::received(1);
    let mut focus = mac();
    let _ = composing(&mut focus, &rows);

    let mut tokens = Vec::new();
    for _ in 0..3 {
        let effects = focus.handle(Input::ComposerEdited);
        let set = timers(&effects);
        assert_eq!(set.len(), 1, "every edit re-arms the one timer");
        assert_eq!(set[0].1, Duration::from_millis(1500));
        assert_eq!(saved(&effects), None, "an edit saves nothing yet");
        tokens.push(set[0].0);
    }
    assert!(
        tokens.windows(2).all(|pair| pair[0] != pair[1]),
        "each arming is a timer of its own"
    );
    for stale in &tokens[..2] {
        assert!(
            focus.handle(Input::Timer(*stale)).is_empty(),
            "a timer an edit re-armed saves nothing"
        );
    }
    let effects = focus.handle(Input::Timer(tokens[2]));
    assert_eq!(saved(&effects), Some(FIRST), "the last edit's timer saves");
    assert_eq!(
        shown(&effects).len(),
        1,
        "and says nothing else: autosave is quiet"
    );
    assert!(
        focus.handle(Input::Timer(tokens[2])).is_empty(),
        "a timer fires once"
    );
    // The save landing changes nothing on screen.
    assert!(
        focus
            .handle(Input::DraftSaved {
                composition: FIRST,
                saved: Ok(true),
            })
            .is_empty()
    );
}

#[test]
fn esc_saves_at_once_closes_the_composer_and_says_the_draft_was_kept() {
    let rows = List::received(2);
    let mut focus = mac();
    let _ = composing(&mut focus, &rows);
    let pending = timers(&focus.handle(Input::ComposerEdited))[0].0;

    let effects = run(&mut focus, CommandId::Back, &rows);
    let said = shown(&effects);
    assert!(said.contains(&Intent::CloseSurface(SurfaceKind::Composer)));
    assert!(
        said.contains(&Intent::KeyboardHome),
        "the keyboard goes home"
    );
    assert_eq!(
        saved(&effects),
        Some(FIRST),
        "saved now, not after the wait"
    );
    assert!(toasts(&effects).is_empty(), "said once the save has landed");
    assert_eq!(focus.key_context(), KeyContext::List);
    assert!(
        focus.handle(Input::Timer(pending)).is_empty(),
        "the wait it cut short saves nothing more"
    );

    let effects = focus.handle(Input::DraftSaved {
        composition: FIRST,
        saved: Ok(true),
    });
    assert_eq!(
        toasts(&effects),
        vec![(
            "Draft saved locally 16:12".to_owned(),
            ToastKind::Completed {
                undoable: false,
                seconds: None,
            }
        )]
    );
}

#[test]
fn esc_on_a_composer_nothing_was_written_in_saves_nothing_and_says_nothing() {
    let rows = List::received(1);
    let mut focus = mac();
    let _ = composing(&mut focus, &rows);
    let effects = run(&mut focus, CommandId::Back, &rows);
    assert_eq!(
        shown(&effects),
        vec![
            Intent::CloseSurface(SurfaceKind::Composer),
            Intent::KeyboardHome
        ]
    );
}

#[test]
fn a_save_with_nothing_to_keep_is_not_announced() {
    let rows = List::received(1);
    let mut focus = mac();
    let _ = composing(&mut focus, &rows);
    let _ = focus.handle(Input::ComposerEdited);
    let _ = run(&mut focus, CommandId::Back, &rows);
    // Written, then cleared: the frontend found nothing worth keeping.
    let effects = focus.handle(Input::DraftSaved {
        composition: FIRST,
        saved: Ok(false),
    });
    assert!(toasts(&effects).is_empty());
}

#[test]
fn a_save_that_failed_is_said_in_its_own_words() {
    let rows = List::received(1);
    let mut focus = mac();
    let _ = composing(&mut focus, &rows);
    let _ = focus.handle(Input::ComposerEdited);
    let _ = run(&mut focus, CommandId::Back, &rows);
    let effects = focus.handle(Input::DraftSaved {
        composition: FIRST,
        saved: Err("This draft could not be saved.".to_owned()),
    });
    assert_eq!(
        toasts(&effects),
        vec![(
            "This draft could not be saved.".to_owned(),
            ToastKind::Notice
        )]
    );
}

#[test]
fn a_composer_its_window_closed_is_saved_as_esc_saves_it() {
    let rows = List::received(1);
    let mut focus = mac();
    let _ = composing(&mut focus, &rows);
    let _ = focus.handle(Input::ComposerEdited);
    // ⌘W or the close button: the toolkit closed it, and says so.
    let effects = focus.handle_on(Input::SurfaceClosed(SurfaceKind::Composer), &rows);
    assert_eq!(saved(&effects), Some(FIRST));
    assert!(shown(&effects).contains(&Intent::KeyboardHome));
    assert!(
        !shown(&effects).contains(&Intent::CloseSurface(SurfaceKind::Composer)),
        "it is closed already"
    );
}

#[test]
fn a_new_composition_saves_the_one_it_replaces_first() {
    let rows = List::received(2);
    let mut focus = mac();
    let _ = composing(&mut focus, &rows);
    let _ = focus.handle(Input::ComposerEdited);
    let effects = run(&mut focus, CommandId::Reply, &rows);
    assert_eq!(
        shown(&effects),
        vec![
            Intent::SaveDraft { composition: FIRST },
            Intent::Composer {
                kind: ComposerKind::Reply,
                message: Some(MessageId::new(100)),
            },
        ],
        "the words in the composer are saved before it is refilled"
    );
    // The replaced draft is said to be kept, once it is.
    let effects = focus.handle(Input::DraftSaved {
        composition: FIRST,
        saved: Ok(true),
    });
    assert_eq!(toasts(&effects).len(), 1);
    // The new composition's own timer saves the new one.
    let token = timers(&focus.handle(Input::ComposerEdited))[0].0;
    assert_eq!(saved(&focus.handle(Input::Timer(token))), Some(FIRST + 1));
}

#[test]
fn a_draft_row_opens_in_the_composer() {
    let rows = List::of(&[Some(DraftState::Editing), None]);
    let mut focus = mac();
    landed(&mut focus, &rows);
    let effects = run(&mut focus, CommandId::OpenMessage, &rows);
    assert_eq!(
        shown(&effects),
        vec![Intent::Composer {
            kind: ComposerKind::Draft,
            message: Some(MessageId::new(100)),
        }]
    );
    assert_eq!(focus.key_context(), KeyContext::Composer);
}

#[test]
fn edit_on_an_open_draft_on_its_way_opens_it_in_the_composer() {
    for state in [
        DraftState::Queued,
        DraftState::Failed,
        DraftState::Unconfirmed,
    ] {
        let rows = List::of(&[Some(state)]);
        let mut focus = mac();
        landed(&mut focus, &rows);
        let _ = run(&mut focus, CommandId::OpenMessage, &rows);
        assert_eq!(
            focus.reading(),
            Some(MessageId::new(100)),
            "it opens to be read"
        );
        let effects = run(&mut focus, CommandId::OpenMessage, &rows);
        assert_eq!(
            shown(&effects),
            vec![
                Intent::CloseSurface(SurfaceKind::Message),
                Intent::Composer {
                    kind: ComposerKind::Draft,
                    message: Some(MessageId::new(100)),
                },
            ],
            "Edit, on mail that is {state:?}"
        );
    }
}

/// `command` on the cursor's row asks which draft is behind it, and the
/// answer settles that draft's send.
fn settles(command: CommandId, expected: Command) {
    let rows = List::of(&[Some(DraftState::Unconfirmed)]);
    let mut focus = mac();
    landed(&mut focus, &rows);
    let effects = run(&mut focus, command, &rows);
    let [(ticket, request)] = asked(&effects).try_into().expect("one request");
    assert_eq!(
        request,
        Request::DraftBehind {
            message: MessageId::new(100),
            command,
        }
    );
    let effects = focus.handle(Input::Reply(
        ticket,
        Reply::DraftBehind {
            message: MessageId::new(100),
            command,
            draft: Some(DraftId::new(9)),
        },
    ));
    assert_eq!(
        asked(&effects)
            .into_iter()
            .map(|(_, request)| request)
            .collect::<Vec<_>>(),
        vec![Request::Post(expected)]
    );
}

#[test]
fn cancel_retry_and_mark_sent_settle_the_draft_behind_the_row() {
    let draft = Some(DraftId::new(9));
    settles(CommandId::CancelSend, Command::CancelSend { draft });
    settles(CommandId::RetrySend, Command::RetrySend { draft });
    settles(CommandId::MarkSent, Command::MarkSent { draft });
}

#[test]
fn settling_a_message_that_is_no_draft_says_so() {
    let rows = List::received(1);
    let mut focus = mac();
    landed(&mut focus, &rows);
    let effects = run(&mut focus, CommandId::CancelSend, &rows);
    let [(ticket, _)] = asked(&effects).try_into().expect("one request");
    let effects = focus.handle(Input::Reply(
        ticket,
        Reply::DraftBehind {
            message: MessageId::new(100),
            command: CommandId::CancelSend,
            draft: None,
        },
    ));
    assert_eq!(
        toasts(&effects),
        vec![(
            postio_ui::focus_target::NOT_BEING_SENT.to_owned(),
            ToastKind::Notice
        )]
    );
    assert!(asked(&effects).is_empty());
}

#[test]
fn settling_the_open_draft_closes_it_and_the_keyboard_goes_home() {
    let rows = List::of(&[Some(DraftState::Failed), None]);
    let mut focus = mac();
    landed(&mut focus, &rows);
    let _ = run(&mut focus, CommandId::OpenMessage, &rows);
    let effects = run(&mut focus, CommandId::RetrySend, &rows);
    let said = shown(&effects);
    assert!(said.contains(&Intent::CloseSurface(SurfaceKind::Message)));
    assert!(said.contains(&Intent::KeyboardHome));
    assert_eq!(
        asked(&effects)
            .into_iter()
            .map(|(_, request)| request)
            .collect::<Vec<_>>(),
        vec![Request::DraftBehind {
            message: MessageId::new(100),
            command: CommandId::RetrySend,
        }]
    );
    assert_eq!(focus.key_context(), KeyContext::List);
}

#[test]
fn a_composer_the_frontend_opened_itself_is_saved_on_closing_too() {
    // A `mailto:` link: the frontend made the draft and opened the window.
    let rows = List::received(1);
    let mut focus = mac();
    landed(&mut focus, &rows);
    let _ = focus.handle_on(Input::SurfaceOpened(SurfaceKind::Composer), &rows);
    assert_eq!(focus.key_context(), KeyContext::Composer);
    let _ = focus.handle(Input::ComposerEdited);
    let effects = run(&mut focus, CommandId::Back, &rows);
    assert_eq!(saved(&effects), Some(FIRST));
}

#[test]
fn edits_with_no_composer_open_set_no_timer() {
    let mut focus = mac();
    assert!(focus.handle(Input::ComposerEdited).is_empty());
}
