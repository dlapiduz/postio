//! Thread ▾'s menu, the outline, mute and split (spec 013 US5, FR-019).
//!
//! Driven as a frontend would: a fake list, the keys the registry binds,
//! and the replies a client would give.

use crate::search_support::{asks, shown};
use chrono::{TimeZone, Utc};
use postio_config::paths::Platform;
use postio_core::{Command, CommandId, MessageTarget};
use postio_focus::{
    Effect, FocusController, Input, Intent, OutlineInput, Policy, Reply, Request, RowFacts, Rows,
    SurfaceKind, Ticket, ToastKind,
};
use postio_model::listing::MessageSummary;
use postio_model::thread_view::{Line, ThreadMember, ThreadView};
use postio_model::{EmailAddress, MessageId, ThreadId};
use postio_ui::focus_list::{FocusRow, lone};
use postio_ui::thread_outline::{OutlineView, ThreadMenuView};

struct List {
    /// Row `n` is message `100 + n` of thread `500 + n`, six messages.
    muted: Vec<Option<u32>>,
}

impl Rows for List {
    fn len(&self) -> u32 {
        self.muted.len() as u32
    }
    fn facts(&self, position: u32) -> Option<RowFacts> {
        (position < self.len()).then(|| RowFacts {
            id: MessageId::new(100 + i64::from(position)),
            digest: false,
            threads: vec![ThreadId::new(500 + i64::from(position))],
            writes: false,
        })
    }
    fn position_of(&self, message: MessageId) -> Option<u32> {
        let at = message.get() - 100;
        (0..self.len() as i64).contains(&at).then_some(at as u32)
    }
    fn row(&self, position: u32) -> Option<FocusRow> {
        let muted = *self.muted.get(position as usize)?;
        let mut summary = lone(MessageSummary {
            id: MessageId::new(100 + i64::from(position)),
            thread: Some(ThreadId::new(500 + i64::from(position))),
            from: Some(EmailAddress::new(Some("Ben Ortiz"), "ben@example.com")),
            subject: Some("Harbor API draft v3".into()),
            preview: None,
            received_at: Utc.with_ymd_and_hms(2026, 9, 21, 9, 14, 0).unwrap(),
            seen: true,
            flagged: false,
            answered: false,
            send_state: None,
            send_at: None,
            has_attachments: false,
            thread_count: 6,
            to: Vec::new(),
        });
        summary.message_count = 6;
        summary.muted = muted;
        Some(FocusRow::conversation(summary))
    }
}

fn member_id(thread: ThreadId, index: usize) -> MessageId {
    MessageId::new(thread.get() * 100 + index as i64)
}

/// Six messages, Ana's at even places and Ben's at odd ones, the last two
/// unread.
fn view(thread: ThreadId) -> ThreadView {
    let at = Utc.with_ymd_and_hms(2026, 9, 21, 9, 0, 0).unwrap();
    ThreadView {
        thread,
        subject: Some("Harbor API draft v3".into()),
        members: (0..6)
            .map(|index| ThreadMember {
                id: member_id(thread, index),
                from: Some(if index % 2 == 0 {
                    EmailAddress::new(Some("Ana Ruiz"), "ana@example.com")
                } else {
                    EmailAddress::new(Some("Ben Adeyemi"), "ben@example.com")
                }),
                own: false,
                at: at + chrono::Duration::minutes(index as i64),
                seen: index < 4,
                has_attachments: false,
                line: Line::Own(format!("Message {index} says its own thing.")),
                answers: None,
                marker: false,
            })
            .collect(),
        draft: None,
        muted: None,
        labels: Vec::new(),
    }
}

fn focus() -> FocusController {
    FocusController::new(Policy::for_platform(Platform::Apple))
}

fn list() -> List {
    List {
        muted: vec![None, None],
    }
}

fn run(focus: &mut FocusController, command: CommandId, rows: &List) -> Vec<Effect> {
    focus.handle_on(Input::Command(command), rows)
}

fn read_asked(effects: &[Effect]) -> Option<(Ticket, ThreadId)> {
    asks(effects)
        .into_iter()
        .find_map(|(ticket, request)| match request {
            Request::ReadThread(thread) => Some((ticket, thread)),
            _ => None,
        })
}

/// Harbor (row 0) opened with Return: the window is on message 5 (index 4),
/// the first unread, and its thread is walked.
fn window() -> (FocusController, List) {
    let rows = list();
    let mut focus = focus();
    let _ = focus.open(postio_model::ListScope::Focus(
        postio_model::FocusScope::Inbox,
    ));
    let _ = focus.landed(&rows, true);
    let effects = run(&mut focus, CommandId::OpenMessage, &rows);
    let (ticket, thread) = read_asked(&effects).expect("opening a thread reads it");
    let _ = focus.handle_on(
        Input::Reply(
            ticket,
            Reply::ThreadRead {
                thread,
                view: Ok(Some(view(thread))),
            },
        ),
        &rows,
    );
    (focus, rows)
}

fn menu_of(effects: &[Effect]) -> Option<ThreadMenuView> {
    shown(effects).into_iter().find_map(|intent| match intent {
        Intent::ThreadMenu(menu) => Some(*menu),
        _ => None,
    })
}

fn outline_of(effects: &[Effect]) -> Option<OutlineView> {
    shown(effects)
        .into_iter()
        .rev()
        .find_map(|intent| match intent {
            Intent::Outline(outline) => Some(*outline),
            _ => None,
        })
}

fn requests(effects: &[Effect]) -> Vec<Request> {
    asks(effects)
        .into_iter()
        .map(|(_, request)| request)
        .collect()
}

fn closed(effects: &[Effect], kind: SurfaceKind) -> bool {
    shown(effects)
        .iter()
        .any(|intent| matches!(intent, Intent::CloseSurface(closed) if *closed == kind))
}

fn opened_message(effects: &[Effect]) -> Option<MessageId> {
    shown(effects).into_iter().find_map(|intent| match intent {
        Intent::OpenMessage { message, .. } => Some(message),
        _ => None,
    })
}

#[test]
fn the_thread_menu_opens_over_the_window_in_two_groups() {
    let (mut focus, rows) = window();
    let effects = run(&mut focus, CommandId::ThreadMenu, &rows);
    let menu = menu_of(&effects).expect("the menu opens");
    let titles: Vec<_> = menu
        .groups
        .iter()
        .map(|group| group.title.as_str())
        .collect();
    assert_eq!(titles, ["Whole thread", "This message only"]);
    assert_eq!(menu.groups[0].entries[3].title, "Mute thread");
    assert!(menu.groups[1].entries[0].enabled, "message 5 can be split");
    assert_eq!(
        focus.key_context(),
        postio_ui::keymap::KeyContext::Reader,
        "the menu is the window's keys"
    );
    let effects = run(&mut focus, CommandId::Back, &rows);
    assert!(
        closed(&effects, SurfaceKind::ThreadMenu),
        "Escape closes it"
    );
}

#[test]
fn the_menu_on_the_first_message_says_split_is_not_offered() {
    let (mut focus, rows) = window();
    let _ = run(&mut focus, CommandId::FirstInConversation, &rows);
    let effects = run(&mut focus, CommandId::ThreadMenu, &rows);
    let menu = menu_of(&effects).expect("the menu opens");
    assert!(!menu.groups[1].entries[0].enabled);
}

#[test]
fn choosing_split_in_the_menu_splits_the_message_shown() {
    let (mut focus, rows) = window();
    let _ = run(&mut focus, CommandId::ThreadMenu, &rows);
    let effects = focus.handle_on(Input::ThreadMenuPick(CommandId::SplitThread), &rows);
    assert!(closed(&effects, SurfaceKind::ThreadMenu));
    let thread = ThreadId::new(500);
    assert!(
        requests(&effects).contains(&Request::Post(Command::SplitThread {
            message: Some(member_id(thread, 4)),
            rejoin: None,
        })),
        "split posts the shown message: {effects:?}"
    );
}

#[test]
fn split_on_the_first_message_is_refused_with_a_notice_and_asks_nothing() {
    let (mut focus, rows) = window();
    let _ = run(&mut focus, CommandId::FirstInConversation, &rows);
    let effects = run(&mut focus, CommandId::SplitThread, &rows);
    assert!(
        requests(&effects).is_empty(),
        "nothing is sent: {effects:?}"
    );
    let said = shown(&effects).into_iter().find_map(|intent| match intent {
        Intent::Toast {
            text,
            kind: ToastKind::Notice,
        } => Some(text),
        _ => None,
    });
    let said = said.expect("it says why");
    assert!(said.contains("first message"), "{said}");
}

#[test]
fn the_outline_opens_on_the_message_shown_with_every_message_and_chip() {
    let (mut focus, rows) = window();
    let effects = run(&mut focus, CommandId::ThreadOutline, &rows);
    let outline = outline_of(&effects).expect("the outline opens");
    assert_eq!(outline.rows.len(), 6);
    assert_eq!(outline.cursor, 4);
    assert!(outline.rows[4].current);
    assert_eq!(
        outline.people,
        [("Ana Ruiz".to_owned(), 3), ("Ben Adeyemi".to_owned(), 3)]
    );
    assert_eq!(focus.key_context(), postio_ui::keymap::KeyContext::Outline);
}

#[test]
fn typing_and_person_chips_narrow_the_outline() {
    let (mut focus, rows) = window();
    let _ = run(&mut focus, CommandId::ThreadOutline, &rows);
    let effects = focus.handle_on(Input::Outline(OutlineInput::Typed("ben".into())), &rows);
    let outline = outline_of(&effects).expect("redrawn");
    assert_eq!(
        outline
            .rows
            .iter()
            .map(|row| row.number)
            .collect::<Vec<_>>(),
        [2, 4, 6]
    );
    let effects = focus.handle_on(Input::Outline(OutlineInput::Typed(String::new())), &rows);
    assert_eq!(outline_of(&effects).unwrap().rows.len(), 6);
    let effects = focus.handle_on(
        Input::Outline(OutlineInput::Person(Some("Ana Ruiz".into()))),
        &rows,
    );
    let outline = outline_of(&effects).unwrap();
    assert_eq!(outline.person.as_deref(), Some("Ana Ruiz"));
    assert_eq!(
        outline
            .rows
            .iter()
            .map(|row| row.number)
            .collect::<Vec<_>>(),
        [1, 3, 5]
    );
}

#[test]
fn slash_puts_the_keyboard_in_the_filter() {
    let (mut focus, rows) = window();
    let _ = run(&mut focus, CommandId::ThreadOutline, &rows);
    let effects = focus.handle_on(Input::Outline(OutlineInput::FocusFilter), &rows);
    assert!(
        shown(&effects)
            .iter()
            .any(|intent| matches!(intent, Intent::OutlineFocusFilter))
    );
    let effects = run(&mut focus, CommandId::Search, &rows);
    assert!(
        shown(&effects)
            .iter()
            .any(|intent| matches!(intent, Intent::OutlineFocusFilter)),
        "the Search key does the same"
    );
}

#[test]
fn up_and_down_move_the_outline_s_cursor_and_stop_at_the_ends() {
    let (mut focus, rows) = window();
    let _ = run(&mut focus, CommandId::ThreadOutline, &rows);
    let effects = focus.handle_on(Input::Outline(OutlineInput::Move(1)), &rows);
    assert_eq!(outline_of(&effects).unwrap().cursor, 5);
    let effects = focus.handle_on(Input::Outline(OutlineInput::Move(1)), &rows);
    assert_eq!(outline_of(&effects).unwrap().cursor, 5, "the end stays");
    let effects = focus.handle_on(Input::Outline(OutlineInput::Move(-9)), &rows);
    assert_eq!(outline_of(&effects).unwrap().cursor, 0);
}

#[test]
fn u_goes_to_the_first_unread_inside_the_outline() {
    let (mut focus, rows) = window();
    let _ = run(&mut focus, CommandId::ThreadOutline, &rows);
    let _ = focus.handle_on(Input::Outline(OutlineInput::Move(-9)), &rows);
    let effects = run(&mut focus, CommandId::OutlineFirstUnread, &rows);
    assert_eq!(outline_of(&effects).unwrap().cursor, 4);
}

#[test]
fn return_on_a_filtered_row_shows_that_message_and_closes_the_outline() {
    let (mut focus, rows) = window();
    let _ = run(&mut focus, CommandId::ThreadOutline, &rows);
    let _ = focus.handle_on(Input::Outline(OutlineInput::Typed("ben".into())), &rows);
    let _ = focus.handle_on(Input::Outline(OutlineInput::Move(-9)), &rows);
    let effects = run(&mut focus, CommandId::OpenMessage, &rows);
    assert!(closed(&effects, SurfaceKind::Outline));
    assert_eq!(
        opened_message(&effects),
        Some(member_id(ThreadId::new(500), 1)),
        "message 2, the first of Ben's"
    );
    let kicker = shown(&effects).into_iter().find_map(|intent| match intent {
        Intent::Kicker(text) => Some(text),
        _ => None,
    });
    assert!(
        kicker.is_some_and(|text| text.starts_with("Message 2 of 6")),
        "the walk is on message 2"
    );
}

#[test]
fn escape_closes_the_outline_and_the_window_keeps_its_message() {
    let (mut focus, rows) = window();
    let _ = run(&mut focus, CommandId::ThreadOutline, &rows);
    let effects = run(&mut focus, CommandId::Back, &rows);
    assert!(closed(&effects, SurfaceKind::Outline));
    assert!(opened_message(&effects).is_none());
}

#[test]
fn m_in_the_window_mutes_the_thread_and_again_unmutes_it() {
    let (mut focus, rows) = window();
    let thread = ThreadId::new(500);
    let effects = run(&mut focus, CommandId::MuteThread, &rows);
    assert_eq!(
        requests(&effects),
        [Request::Post(Command::MuteThread {
            thread: Some(thread),
            muted: true
        })]
    );
    let effects = run(&mut focus, CommandId::MuteThread, &rows);
    assert_eq!(
        requests(&effects),
        [Request::Post(Command::MuteThread {
            thread: Some(thread),
            muted: false
        })]
    );
    let effects = run(&mut focus, CommandId::ThreadMenu, &rows);
    assert_eq!(
        menu_of(&effects).unwrap().groups[0].entries[3].title,
        "Mute thread"
    );
}

#[test]
fn the_menu_offers_unmute_once_the_thread_is_muted() {
    let (mut focus, rows) = window();
    let _ = run(&mut focus, CommandId::MuteThread, &rows);
    let effects = run(&mut focus, CommandId::ThreadMenu, &rows);
    assert_eq!(
        menu_of(&effects).unwrap().groups[0].entries[3].title,
        "Unmute thread"
    );
}

#[test]
fn m_on_a_list_row_mutes_that_thread_and_a_muted_row_unmutes() {
    let mut rows = list();
    let mut focus = focus();
    let _ = focus.open(postio_model::ListScope::Focus(
        postio_model::FocusScope::Inbox,
    ));
    let _ = focus.landed(&rows, true);
    let effects = run(&mut focus, CommandId::MuteThread, &rows);
    assert_eq!(
        requests(&effects),
        [Request::Post(Command::MuteThread {
            thread: Some(ThreadId::new(500)),
            muted: true
        })]
    );
    rows.muted[0] = Some(1);
    let effects = run(&mut focus, CommandId::MuteThread, &rows);
    assert_eq!(
        requests(&effects),
        [Request::Post(Command::MuteThread {
            thread: Some(ThreadId::new(500)),
            muted: false
        })]
    );
}

#[test]
fn delete_just_this_message_aims_the_message_shown_not_its_thread() {
    let (mut focus, rows) = window();
    let effects = run(&mut focus, CommandId::DeleteThisMessage, &rows);
    let aims: Vec<_> = requests(&effects)
        .into_iter()
        .filter_map(|request| match request {
            Request::Send { aims, .. } => Some(aims),
            _ => None,
        })
        .flatten()
        .collect();
    assert_eq!(
        aims,
        [MessageTarget::Messages(vec![member_id(
            ThreadId::new(500),
            4
        )])]
    );
}

#[test]
fn move_just_this_message_opens_the_folder_picker() {
    let (mut focus, rows) = window();
    let effects = run(&mut focus, CommandId::MoveThisMessage, &rows);
    assert!(
        shown(&effects)
            .iter()
            .any(|intent| matches!(intent, Intent::OpenPicker(_))),
        "{effects:?}"
    );
    assert!(
        requests(&effects)
            .iter()
            .any(|request| matches!(request, Request::Folders { .. }))
    );
}
