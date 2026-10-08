//! The pickers at the row and the toast's policy, from the controller's side
//! (specs/009-focus-macos T088; research R2, slice 9).
//!
//! What GTK's window and pickers did -- `open_when`, `open_labels`,
//! `open_moves`, the picker chosen becoming a sent command, and the toast's
//! rules in `postio-widgets` -- asserted here so both frontends get it: `s`,
//! `h`, `l` and `m` open their picker at the cursor's row, or at the open
//! message; a number key picks a numbered row; a typed date reads the same
//! on either platform; Space puts a label on or takes it off, and a name
//! nobody has makes one; Move offers the last two destinations as `1` and
//! `2`; what is chosen goes where the picker aimed when it opened; and the
//! toast stays eight seconds, a new one replaces the old, and a queued
//! send's own Undo is the one Undo runs first.

use chrono::{DateTime, Local, TimeZone, Utc};
use postio_config::paths::Platform;
use postio_core::{Command, CommandId, Event, MessageTarget};
use postio_focus::{
    Anchor, Effect, FocusController, FoldersRead, Input, Intent, LabelsRead, PickerField,
    PickerKind, PickerView, Policy, Reply, Request, RowFacts, Rows, SurfaceKind, Ticket, ToastKind,
};
use postio_model::{
    AccountId, DraftId, Label, LabelId, Mailbox, MailboxId, MailboxRole, MessageId, ThreadId,
};
use postio_ui::keymap::KeyContext;
use postio_ui::pickers;

/// A list of conversations, each from Ada about "Subject n".
struct List {
    rows: Vec<RowFacts>,
}

impl List {
    fn of(len: i64) -> Self {
        List {
            rows: (0..len)
                .map(|at| RowFacts {
                    id: MessageId::new(100 + at),
                    digest: false,
                    threads: vec![ThreadId::new(500 + at)],
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
    fn said(&self, position: u32) -> Option<(String, String)> {
        (position < self.len()).then(|| ("Ada Example".to_owned(), format!("Subject {position}")))
    }
}

/// Saturday 26 September 2026, 16:09 here: the clock every test stops.
fn saturday_afternoon() -> DateTime<Local> {
    local_at(2026, 9, 26, 16, 9)
}

fn local_at(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> DateTime<Local> {
    Local
        .with_ymd_and_hms(year, month, day, hour, minute, 0)
        .single()
        .expect("an unambiguous local time")
}

fn controller(platform: Platform) -> FocusController {
    let mut focus = FocusController::new(Policy::for_platform(platform));
    focus.handle(Input::Clock(Some(saturday_afternoon())));
    focus
}

fn mac() -> FocusController {
    controller(Platform::Apple)
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

fn asks(effects: &[Effect]) -> Vec<(Ticket, Request)> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Ask(ticket, request) => Some((*ticket, request.clone())),
            _ => None,
        })
        .collect()
}

fn asked(effects: &[Effect]) -> Vec<Request> {
    asks(effects)
        .into_iter()
        .map(|(_, request)| request)
        .collect()
}

/// The picker as it was opened among `effects`.
fn opened(effects: &[Effect]) -> PickerView {
    shown(effects)
        .into_iter()
        .find_map(|intent| match intent {
            Intent::OpenPicker(view) => Some(view),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no picker opened in {effects:?}"))
}

/// The picker's last view among `effects`: opened or redrawn.
fn view(effects: &[Effect]) -> PickerView {
    shown(effects)
        .into_iter()
        .rev()
        .find_map(|intent| match intent {
            Intent::OpenPicker(view) | Intent::PickerRows(view) => Some(view),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no picker drawn in {effects:?}"))
}

/// The token of the row named `name`.
fn token(view: &PickerView, name: &str) -> u64 {
    view.rows
        .iter()
        .find(|row| row.name == name)
        .unwrap_or_else(|| panic!("no row {name:?} in {:?}", view.rows))
        .token
}

fn run(focus: &mut FocusController, command: CommandId, rows: &List) -> Vec<Effect> {
    focus.handle_on(Input::Command(command), rows)
}

/// The one command sent among `effects`, with where it went.
fn sent(effects: &[Effect]) -> (Command, Vec<MessageTarget>) {
    let sends: Vec<(Command, Vec<MessageTarget>)> = asked(effects)
        .into_iter()
        .filter_map(|request| match request {
            Request::Send { command, aims, .. } => Some((command, aims)),
            _ => None,
        })
        .collect();
    assert_eq!(sends.len(), 1, "one send in {effects:?}");
    sends[0].clone()
}

fn closes_the_picker(effects: &[Effect]) -> bool {
    shown(effects).contains(&Intent::CloseSurface(SurfaceKind::Picker))
}

fn label(id: i64, name: &str) -> Label {
    let mut label = Label::new(AccountId::new(1), name);
    label.id = LabelId::new(id);
    label
}

fn folder(id: i64, name: &str, role: MailboxRole) -> Mailbox {
    let mut folder = Mailbox::new(AccountId::new(1), name, None);
    folder.id = MailboxId::new(id);
    folder.role = role;
    folder
}

/// The labels one account has: Atlas on both of the first two
/// conversations, Harbor on none.
fn labels_read() -> LabelsRead {
    LabelsRead {
        account: AccountId::new(1),
        labels: vec![label(1, "Atlas"), label(2, "Harbor")],
        counts: vec![(LabelId::new(1), 4)],
        carried: vec![
            (ThreadId::new(500), label(1, "Atlas")),
            (ThreadId::new(501), label(1, "Atlas")),
        ],
    }
}

/// Answer the one picker read among `effects` with `reply`.
fn answer(
    focus: &mut FocusController,
    effects: &[Effect],
    rows: &List,
    reply: impl FnOnce(u64) -> Reply,
) -> Vec<Effect> {
    let (ticket, stamp) = asks(effects)
        .into_iter()
        .find_map(|(ticket, request)| match request {
            Request::Labels { stamp, .. }
            | Request::Folders { stamp }
            | Request::CreateLabel { stamp, .. } => Some((ticket, stamp)),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no picker read in {effects:?}"));
    focus.handle_on(Input::Reply(ticket, reply(stamp)), rows)
}

#[test]
fn s_h_l_and_m_open_their_picker_at_the_cursors_row() {
    let rows = List::of(3);
    for (command, kind, title) in [
        (CommandId::Snooze, PickerKind::Snooze, pickers::SNOOZE_TITLE),
        (
            CommandId::RemindIfNoReply,
            PickerKind::Remind,
            pickers::REMIND_TITLE,
        ),
        (CommandId::AddLabel, PickerKind::Label, pickers::LABEL_TITLE),
        (CommandId::Move, PickerKind::Move, pickers::MOVE_TITLE),
    ] {
        let mut focus = mac();
        focus.handle_on(Input::Point(1), &rows);
        assert!(focus.answers(command), "{command:?} is the controller's");
        let effects = run(&mut focus, command, &rows);
        let view = opened(&effects);
        assert_eq!(view.kind, kind);
        assert_eq!(view.anchor, Anchor::Row(1), "hung from the cursor's row");
        assert_eq!(view.title, title);
        assert_eq!(view.target, "Ada Example \u{b7} Subject 1");
        assert_eq!(focus.key_context(), KeyContext::Picker, "{kind:?}");

        // Back closes it, and the keyboard goes home.
        let back = run(&mut focus, CommandId::Back, &rows);
        assert!(closes_the_picker(&back), "{back:?}");
        assert!(shown(&back).contains(&Intent::KeyboardHome));
        assert_eq!(focus.key_context(), KeyContext::List);
    }
}

#[test]
fn the_label_and_move_pickers_read_what_they_list_when_they_open() {
    let rows = List::of(3);
    let mut focus = mac();
    focus.handle_on(Input::Point(1), &rows);
    let effects = run(&mut focus, CommandId::AddLabel, &rows);
    assert_eq!(opened(&effects).field, PickerField::Filter);
    assert_eq!(opened(&effects).placeholder, pickers::LABEL_FILTER);
    assert!(
        asked(&effects).iter().any(|request| matches!(
            request,
            Request::Labels { message: Some(message), threads, .. }
                if *message == MessageId::new(101) && *threads == [ThreadId::new(501)]
        )),
        "the cursor's message names the account; its conversations what is applied: {effects:?}"
    );

    let mut focus = mac();
    focus.handle_on(Input::Point(1), &rows);
    let effects = run(&mut focus, CommandId::Move, &rows);
    assert_eq!(opened(&effects).placeholder, pickers::MOVE_FILTER);
    assert!(
        asked(&effects)
            .iter()
            .any(|request| matches!(request, Request::Folders { .. })),
        "{effects:?}"
    );
}

#[test]
fn from_the_open_message_a_picker_hangs_from_it_and_aims_at_the_message() {
    let rows = List::of(3);
    let mut focus = mac();
    // Row 0 is selected; the message open is row 2's.
    focus.handle_on(Input::Point(0), &rows);
    run(&mut focus, CommandId::ToggleSelection, &rows);
    focus.handle_on(Input::Point(2), &rows);
    run(&mut focus, CommandId::OpenMessage, &rows);
    assert_eq!(focus.reading(), Some(MessageId::new(102)));

    let effects = run(&mut focus, CommandId::Snooze, &rows);
    let view = opened(&effects);
    assert_eq!(view.anchor, Anchor::OpenMessage);
    assert_eq!(view.target, "Ada Example \u{b7} Subject 2");
    let effects = run(&mut focus, CommandId::PickerChoose1, &rows);
    let (_, aims) = sent(&effects);
    assert_eq!(
        aims,
        vec![MessageTarget::Threads(vec![ThreadId::new(502)])],
        "the message, not the selection"
    );
    assert!(closes_the_picker(&effects));
    assert!(
        !shown(&effects).contains(&Intent::KeyboardHome),
        "the message is still open under it"
    );
    assert_eq!(focus.key_context(), KeyContext::Reader);
}

#[test]
fn a_number_key_picks_a_numbered_preset() {
    let rows = List::of(3);
    let mut focus = mac();
    focus.handle_on(Input::Point(0), &rows);
    let effects = run(&mut focus, CommandId::Snooze, &rows);
    let view = opened(&effects);
    let now = saturday_afternoon();
    let presets = postio_ui::schedule::snooze_presets(now);
    let drawn: Vec<(&str, &str, Option<&str>)> = view
        .rows
        .iter()
        .map(|row| (row.name.as_str(), row.detail.as_str(), row.key.as_deref()))
        .collect();
    let wanted: Vec<(String, String, Option<&str>)> = presets
        .iter()
        .zip(["1", "2", "3", "4"])
        .map(|((name, at), key)| ((*name).to_owned(), pickers::when_label(*at, now), Some(key)))
        .collect();
    assert_eq!(
        drawn,
        wanted
            .iter()
            .map(|(name, detail, key)| (name.as_str(), detail.as_str(), *key))
            .collect::<Vec<_>>(),
        "the four presets, numbered in the controller"
    );
    assert_eq!(view.field, PickerField::Date);
    assert_eq!(view.placeholder, pickers::DATE_PLACEHOLDER);
    assert_eq!(view.hint.as_deref(), Some(pickers::TYPE_A_DATE));

    let effects = run(&mut focus, CommandId::PickerChoose2, &rows);
    assert_eq!(
        sent(&effects),
        (
            Command::Snooze {
                target: MessageTarget::Selection,
                until: Some(presets[1].1.to_utc()),
            },
            vec![MessageTarget::Threads(vec![ThreadId::new(500)])],
        ),
        "`2` is tomorrow morning"
    );
    assert!(closes_the_picker(&effects));
    assert!(shown(&effects).contains(&Intent::KeyboardHome));
}

#[test]
fn tab_then_tue_9am_is_the_same_instant_on_both_platforms() {
    let rows = List::of(1);
    let mut chosen = Vec::new();
    for platform in [Platform::Freedesktop, Platform::Apple] {
        let mut focus = controller(platform);
        focus.handle_on(Input::Point(0), &rows);
        run(&mut focus, CommandId::Snooze, &rows);
        assert_eq!(
            shown(&run(&mut focus, CommandId::PickerTypeDate, &rows)),
            vec![Intent::PickerField],
            "Tab puts the keyboard in the date field"
        );
        let effects = focus.handle_on(
            Input::PickerTyped {
                text: "receipts".to_owned(),
            },
            &rows,
        );
        assert_eq!(view(&effects).hint.as_deref(), Some(pickers::NOT_A_DATE));
        assert!(
            asked(&run(&mut focus, CommandId::PickerConfirm, &rows)).is_empty(),
            "words that are no date send nothing"
        );

        let effects = focus.handle_on(
            Input::PickerTyped {
                text: "tue 9am".to_owned(),
            },
            &rows,
        );
        assert_eq!(view(&effects).typed, "tue 9am");
        assert_eq!(view(&effects).hint.as_deref(), Some("Tue 29 Sep, 09:00"));
        let effects = run(&mut focus, CommandId::PickerConfirm, &rows);
        let (command, _) = sent(&effects);
        assert!(closes_the_picker(&effects));
        chosen.push(command);
    }
    let tuesday = local_at(2026, 9, 29, 9, 0).to_utc();
    assert_eq!(
        chosen,
        vec![
            Command::Snooze {
                target: MessageTarget::Selection,
                until: Some(tuesday),
            };
            2
        ],
        "both platforms read it through the one parser"
    );
}

#[test]
fn remind_offers_its_presets_and_sets_a_reminder() {
    let rows = List::of(1);
    let mut focus = mac();
    focus.handle_on(Input::Point(0), &rows);
    let effects = run(&mut focus, CommandId::RemindIfNoReply, &rows);
    let view = opened(&effects);
    let names: Vec<&str> = view.rows.iter().map(|row| row.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "Tomorrow",
            "In 2 working days",
            "End of the week",
            "In a week"
        ]
    );
    assert_eq!(
        view.footnote,
        pickers::remind_footnote(saturday_afternoon())
    );
    let effects = focus.handle_on(Input::PickerChoose(token(&view, "Tomorrow")), &rows);
    assert_eq!(
        sent(&effects).0,
        Command::RemindIfNoReply {
            target: MessageTarget::Selection,
            at: Some(local_at(2026, 9, 27, 9, 0).to_utc()),
        },
        "a click chooses a row as its number would"
    );
}

#[test]
fn space_toggles_a_label_on_the_aim_and_return_closes() {
    let rows = List::of(3);
    let mut focus = mac();
    focus.handle_on(Input::Point(0), &rows);
    run(&mut focus, CommandId::ToggleSelection, &rows);
    run(&mut focus, CommandId::ExtendSelectionDown, &rows);
    assert_eq!(focus.cursor(), Some(1));

    let effects = run(&mut focus, CommandId::AddLabel, &rows);
    assert_eq!(opened(&effects).target, "2 conversations");
    assert!(
        asked(&effects).iter().any(|request| matches!(
            request,
            Request::Labels { threads, .. }
                if *threads == [ThreadId::new(500), ThreadId::new(501)]
        )),
        "{effects:?}"
    );
    let effects = answer(&mut focus, &effects, &rows, |stamp| Reply::Labels {
        stamp,
        answer: Ok(labels_read()),
    });
    let listed = view(&effects);
    let atlas = listed.rows.iter().find(|row| row.name == "Atlas").unwrap();
    assert!(atlas.applied && atlas.dot);
    assert_eq!(atlas.detail, pickers::APPLIED);
    let harbor = listed.rows.iter().find(|row| row.name == "Harbor").unwrap();
    assert!(!harbor.applied);
    assert_eq!(harbor.detail, "0");
    assert!(harbor.key.is_none(), "labels are not numbered");

    // Space on Harbor puts it on both conversations; the picker stays.
    let effects = focus.handle_on(Input::PickerToggle(token(&listed, "Harbor")), &rows);
    let (command, aims) = sent(&effects);
    assert_eq!(
        command,
        Command::AddLabel {
            target: MessageTarget::Selection,
            label: Some(LabelId::new(2)),
            on: Some(true),
        }
    );
    assert_eq!(
        aims,
        vec![MessageTarget::Threads(vec![
            ThreadId::new(500),
            ThreadId::new(501)
        ])]
    );
    assert!(!closes_the_picker(&effects));
    let listed = view(&effects);
    assert!(
        listed
            .rows
            .iter()
            .find(|row| row.name == "Harbor")
            .unwrap()
            .applied
    );

    // Space on Atlas takes it off.
    let effects = focus.handle_on(Input::PickerToggle(token(&listed, "Atlas")), &rows);
    assert_eq!(
        sent(&effects).0,
        Command::AddLabel {
            target: MessageTarget::Selection,
            label: Some(LabelId::new(1)),
            on: Some(false),
        }
    );

    // Return closes, and the selection it acted on goes.
    let effects = run(&mut focus, CommandId::PickerConfirm, &rows);
    assert!(closes_the_picker(&effects));
    assert!(asked(&effects).is_empty(), "Return sends nothing more");
    assert_eq!(focus.selection(), postio_core::state::Selection::default());
    assert_eq!(focus.key_context(), KeyContext::List);
}

#[test]
fn a_name_nobody_has_makes_a_label_and_puts_it_on() {
    let rows = List::of(1);
    let mut focus = mac();
    focus.handle_on(Input::Point(0), &rows);
    let effects = run(&mut focus, CommandId::AddLabel, &rows);
    answer(&mut focus, &effects, &rows, |stamp| Reply::Labels {
        stamp,
        answer: Ok(labels_read()),
    });
    let effects = focus.handle_on(
        Input::PickerTyped {
            text: "Receipts ".to_owned(),
        },
        &rows,
    );
    let listed = view(&effects);
    let create = &listed.rows[0];
    assert!(create.create, "{:?}", listed.rows);
    assert_eq!(create.name, pickers::create_label("Receipts"));

    let effects = focus.handle_on(Input::PickerToggle(create.token), &rows);
    assert!(
        asked(&effects).iter().any(|request| matches!(
            request,
            Request::CreateLabel { account, name, .. }
                if *account == AccountId::new(1) && name == "Receipts"
        )),
        "{effects:?}"
    );
    let effects = answer(&mut focus, &effects, &rows, |stamp| Reply::LabelCreated {
        stamp,
        answer: Ok(label(3, "Receipts")),
    });
    assert_eq!(
        sent(&effects).0,
        Command::AddLabel {
            target: MessageTarget::Selection,
            label: Some(LabelId::new(3)),
            on: Some(true),
        }
    );
    let listed = view(&effects);
    assert_eq!(listed.typed, "", "the filter empties for the next");
    let names: Vec<&str> = listed.rows.iter().map(|row| row.name.as_str()).collect();
    assert_eq!(names, ["Atlas", "Harbor", "Receipts"]);
    assert!(listed.rows[2].applied);
}

#[test]
fn move_offers_the_recent_folders_as_one_and_two() {
    let rows = List::of(1);
    let mut focus = mac();
    focus.handle_on(Input::Point(0), &rows);
    let effects = run(&mut focus, CommandId::Move, &rows);
    let effects = answer(&mut focus, &effects, &rows, |stamp| Reply::Folders {
        stamp,
        answer: Ok(FoldersRead {
            folders: vec![
                folder(1, "INBOX", MailboxRole::Inbox),
                folder(2, "Archive", MailboxRole::Archive),
                folder(3, "Travel", MailboxRole::Regular),
                folder(7, "Receipts", MailboxRole::Regular),
            ],
            recent: vec![MailboxId::new(3), MailboxId::new(7)],
        }),
    });
    let listed = view(&effects);
    let drawn: Vec<(Option<&str>, &str, Option<&str>)> = listed
        .rows
        .iter()
        .map(|row| {
            (
                row.section.as_deref(),
                row.name.as_str(),
                row.key.as_deref(),
            )
        })
        .collect();
    assert_eq!(
        drawn,
        [
            (Some(pickers::RECENT_HEADING), "Travel", Some("1")),
            (None, "Receipts", Some("2")),
            (Some(pickers::ALL_FOLDERS_HEADING), "Archive", None),
        ],
        "the inbox is where the mail is, and is no destination"
    );

    let effects = run(&mut focus, CommandId::PickerChoose2, &rows);
    assert_eq!(
        sent(&effects).0,
        Command::Move {
            target: MessageTarget::Selection,
            to: Some(MailboxId::new(7)),
        }
    );
    assert!(
        asked(&effects).contains(&Request::NoteMove(MailboxId::new(7))),
        "kept as a recent move"
    );
    assert!(closes_the_picker(&effects));
}

#[test]
fn what_is_chosen_goes_where_the_picker_aimed_when_it_opened() {
    let rows = List::of(3);
    let mut focus = mac();
    focus.handle_on(Input::Point(0), &rows);
    let effects = run(&mut focus, CommandId::Move, &rows);
    let reply = |stamp| Reply::Folders {
        stamp,
        answer: Ok(FoldersRead {
            folders: vec![folder(7, "Receipts", MailboxRole::Regular)],
            recent: Vec::new(),
        }),
    };
    let effects = answer(&mut focus, &effects, &rows, reply);
    // A click elsewhere in the list moves the cursor behind the picker.
    focus.handle_on(Input::Point(2), &rows);
    let effects = focus.handle_on(
        Input::PickerChoose(token(&view(&effects), "Receipts")),
        &rows,
    );
    assert_eq!(
        sent(&effects).1,
        vec![MessageTarget::Threads(vec![ThreadId::new(500)])],
        "row 0, as when it opened"
    );
}

#[test]
fn an_answer_for_a_picker_since_closed_changes_nothing() {
    let rows = List::of(1);
    let mut focus = mac();
    focus.handle_on(Input::Point(0), &rows);
    let effects = run(&mut focus, CommandId::AddLabel, &rows);
    // Closed however it was: Esc in its field, a click outside.
    focus.handle_on(Input::SurfaceClosed(SurfaceKind::Picker), &rows);
    assert_eq!(focus.key_context(), KeyContext::List);
    let late = answer(&mut focus, &effects, &rows, |stamp| Reply::Labels {
        stamp,
        answer: Ok(labels_read()),
    });
    assert!(late.is_empty(), "{late:?}");
    assert!(
        focus.handle_on(Input::PickerToggle(1), &rows).is_empty(),
        "and its rows run nothing"
    );
}

#[test]
fn a_picker_with_nothing_to_aim_at_does_not_open() {
    let mut focus = mac();
    let empty = List::of(0);
    assert!(run(&mut focus, CommandId::Snooze, &empty).is_empty());
    assert_eq!(focus.key_context(), KeyContext::List);
}

#[test]
fn the_pickers_keys_are_the_controllers_even_with_no_picker_up() {
    let focus = mac();
    for command in [
        CommandId::PickerChoose1,
        CommandId::PickerChoose2,
        CommandId::PickerChoose3,
        CommandId::PickerChoose4,
        CommandId::PickerTypeDate,
        CommandId::PickerToggle,
        CommandId::PickerConfirm,
    ] {
        assert!(focus.answers(command), "{command:?}");
    }
    let mut focus = mac();
    assert!(run(&mut focus, CommandId::PickerChoose1, &List::of(1)).is_empty());
}

#[test]
fn a_toast_stays_eight_seconds_unless_its_undo_lasts_longer() {
    assert_eq!(postio_ui::focus_target::TOAST_SECONDS, 8);
    let mut focus = mac();
    let effects = focus.handle(Input::Event(Event::ActionCompleted {
        description: "Archived 1 message".to_owned(),
        undoable: true,
    }));
    let Some(Intent::Toast { kind, .. }) = shown(&effects).into_iter().next() else {
        panic!("a toast: {effects:?}");
    };
    assert_eq!(kind.seconds(), 8);
    assert_eq!(ToastKind::Undone.seconds(), 8);
    assert_eq!(ToastKind::Notice.seconds(), 8);
    assert_eq!(
        ToastKind::Completed {
            undoable: true,
            seconds: Some(300),
        }
        .seconds(),
        300,
        "an answer's lasts as long as its Undo works"
    );
}

#[test]
fn a_queued_sends_own_undo_runs_first_until_a_new_toast_replaces_it() {
    let rows = List::of(1);
    let mut focus = mac();
    let draft = DraftId::new(9);
    let effects = focus.handle(Input::SendQueued { draft, at: None });
    assert_eq!(
        shown(&effects),
        vec![Intent::Toast {
            text: postio_ui::sending::QUEUED_TO_SEND.to_owned(),
            kind: ToastKind::Completed {
                undoable: true,
                seconds: None,
            },
        }]
    );
    let effects = run(&mut focus, CommandId::Undo, &rows);
    assert_eq!(
        asked(&effects),
        vec![Request::Post(Command::CancelSend { draft: Some(draft) })],
        "Undo takes the send back, not the stack's last action"
    );
    assert_eq!(
        asked(&run(&mut focus, CommandId::Undo, &rows)),
        vec![Request::Post(Command::Undo)],
        "once: the next Undo is the stack's"
    );

    // A scheduled send says so; a toast after it takes its Undo away.
    let effects = focus.handle(Input::SendQueued {
        draft,
        at: Some(Utc::now()),
    });
    assert!(
        matches!(
            shown(&effects).as_slice(),
            [Intent::Toast { text, .. }] if text == postio_ui::sending::SEND_SCHEDULED
        ),
        "{effects:?}"
    );
    focus.handle(Input::Event(Event::ActionCompleted {
        description: "Archived 1 message".to_owned(),
        undoable: true,
    }));
    assert_eq!(
        asked(&run(&mut focus, CommandId::Undo, &rows)),
        vec![Request::Post(Command::Undo)],
        "the new toast replaced the old, and its Undo with it"
    );
}
