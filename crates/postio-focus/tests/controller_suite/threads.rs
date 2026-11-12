//! Expanding a thread in place (spec 013 US1, research R7).
//!
//! The controller holds one `Expansion` and the frontend holds the rows: the
//! controller reads them through `Rows` (row positions) and speaks in
//! display positions, which count the expanded thread's children. These
//! cases drive it as a frontend would: a fake list, keys, and the replies a
//! client would give.

use crate::search_support::{asks, shown};
use chrono::{TimeZone, Utc};
use postio_config::paths::Platform;
use postio_core::{CommandId, Event};
use postio_focus::{
    Effect, FocusController, Input, Intent, Policy, Reply, Request, RowFacts, Rows, Ticket,
};
use postio_model::listing::MessageSummary;
use postio_model::thread_view::{Line, ThreadDraft, ThreadMember, ThreadView};
use postio_model::{AccountId, DraftId, EmailAddress, MessageId, ThreadId};
use postio_ui::focus_list::{FocusRow, lone};
use postio_ui::thread_list::ChildRowView;

/// One row of the fake list: a conversation of `count` messages.
#[derive(Clone)]
struct Row {
    id: MessageId,
    thread: ThreadId,
    count: u32,
}

pub(crate) struct List {
    rows: Vec<Row>,
}

impl List {
    /// Rows 0.. with the message counts given; row `n` is message `100 + n`
    /// of thread `500 + n`.
    pub(crate) fn of(counts: &[u32]) -> Self {
        List {
            rows: counts
                .iter()
                .enumerate()
                .map(|(at, count)| Row {
                    id: MessageId::new(100 + at as i64),
                    thread: ThreadId::new(500 + at as i64),
                    count: *count,
                })
                .collect(),
        }
    }

    /// The same list without the row for `thread`.
    fn without(&self, thread: ThreadId) -> Self {
        List {
            rows: self
                .rows
                .iter()
                .filter(|row| row.thread != thread)
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
        self.rows.get(position as usize).map(|row| RowFacts {
            id: row.id,
            digest: false,
            threads: vec![row.thread],
            writes: false,
        })
    }
    fn position_of(&self, message: MessageId) -> Option<u32> {
        self.rows
            .iter()
            .position(|row| row.id == message)
            .map(|at| at as u32)
    }
    fn row(&self, position: u32) -> Option<FocusRow> {
        let row = self.rows.get(position as usize)?;
        let mut summary = lone(MessageSummary {
            id: row.id,
            thread: Some(row.thread),
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
            thread_count: row.count,
            to: Vec::new(),
        });
        summary.message_count = row.count;
        Some(FocusRow::conversation(summary))
    }
}

/// The member `index` of the thread `thread`.
pub(crate) fn member_id(thread: ThreadId, index: usize) -> MessageId {
    MessageId::new(thread.get() * 100 + index as i64)
}

/// A thread read: `members` messages, the last `unread` of them unread, and
/// an unsent reply when `draft`.
pub(crate) fn view(thread: ThreadId, members: usize, unread: usize, draft: bool) -> ThreadView {
    let at = Utc.with_ymd_and_hms(2026, 9, 21, 9, 0, 0).unwrap();
    ThreadView {
        thread,
        subject: Some("Harbor API draft v3".into()),
        members: (0..members)
            .map(|index| ThreadMember {
                id: member_id(thread, index),
                from: Some(EmailAddress::new(Some("Ben Ortiz"), "ben@example.com")),
                own: index % 3 == 2,
                at: at + chrono::Duration::minutes(index as i64),
                seen: index + unread < members,
                has_attachments: false,
                line: Line::Own(format!("Message {index} says its own thing.")),
                answers: None,
                marker: false,
            })
            .collect(),
        draft: draft.then(|| ThreadDraft {
            id: DraftId::new(7),
            first_words: "Thanks, I will".into(),
            updated_at: at,
        }),
        muted: None,
        labels: Vec::new(),
    }
}

pub(crate) fn focus() -> FocusController {
    FocusController::new(Policy::for_platform(Platform::Apple))
}

pub(crate) fn run(focus: &mut FocusController, command: CommandId, rows: &List) -> Vec<Effect> {
    focus.handle_on(Input::Command(command), rows)
}

/// The thread read the effects ask for, and its ticket.
pub(crate) fn read_asked(effects: &[Effect]) -> Option<(Ticket, ThreadId)> {
    asks(effects)
        .into_iter()
        .find_map(|(ticket, request)| match request {
            Request::ReadThread(thread) => Some((ticket, thread)),
            _ => None,
        })
}

/// Answer a read the way the client would.
pub(crate) fn answer(
    focus: &mut FocusController,
    ticket: Ticket,
    view: ThreadView,
    rows: &List,
) -> Vec<Effect> {
    focus.handle_on(
        Input::Reply(
            ticket,
            Reply::ThreadRead {
                thread: view.thread,
                view: Ok(Some(view)),
            },
        ),
        rows,
    )
}

fn splice_of(effects: &[Effect]) -> Option<(u32, Vec<ChildRowView>)> {
    shown(effects).into_iter().find_map(|intent| match intent {
        Intent::Splice { anchor, children } => Some((anchor, children)),
        _ => None,
    })
}

fn unspliced(effects: &[Effect]) -> Vec<u32> {
    shown(effects)
        .into_iter()
        .filter_map(|intent| match intent {
            Intent::Unsplice { anchor } => Some(anchor),
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

fn opened_message(effects: &[Effect]) -> Option<MessageId> {
    shown(effects).into_iter().find_map(|intent| match intent {
        Intent::OpenMessage { message, .. } => Some(message),
        _ => None,
    })
}

/// Harbor, row 0 (thread 500, six messages and a draft), expanded over a
/// list of `counts`.
fn expanded(counts: &[u32]) -> (FocusController, List) {
    let rows = List::of(counts);
    let mut focus = focus();
    let _ = focus.open(postio_model::ListScope::Focus(
        postio_model::FocusScope::Inbox,
    ));
    let _ = focus.landed(&rows, true);
    let effects = run(&mut focus, CommandId::ExpandThread, &rows);
    let (ticket, thread) = read_asked(&effects).expect("a read");
    let _ = answer(&mut focus, ticket, view(thread, 6, 2, true), &rows);
    (focus, rows)
}

#[test]
fn expanding_harbor_reads_it_and_splices_six_children_and_the_draft() {
    let rows = List::of(&[6, 4, 1, 3]);
    let mut focus = focus();
    let _ = focus.open(postio_model::ListScope::Focus(
        postio_model::FocusScope::Inbox,
    ));
    let _ = focus.landed(&rows, true);

    let effects = run(&mut focus, CommandId::ExpandThread, &rows);
    let (ticket, thread) = read_asked(&effects).expect("ExpandThread reads the thread");
    assert_eq!(thread, ThreadId::new(500));
    assert!(
        splice_of(&effects).is_none(),
        "nothing is drawn until the thread is read"
    );

    let effects = answer(&mut focus, ticket, view(thread, 6, 2, true), &rows);
    let (anchor, children) = splice_of(&effects).expect("the read splices");
    assert_eq!(anchor, 0);
    assert_eq!(children.len(), 7, "six members and the draft");
    assert!(children[..6].iter().all(|child| !child.draft));
    assert!(children[6].draft, "the unsent reply comes last");
    assert!(
        children[4].bold && children[5].bold && !children[3].bold,
        "the two unread are bold"
    );
    assert_eq!(
        children[0]
            .focused_hint
            .as_ref()
            .map(|hint| hint.label.as_str()),
        Some("this one")
    );
    assert_eq!(focus.cursor(), Some(0), "the cursor stays on the row");
}

#[test]
fn expanding_another_thread_collapses_the_first() {
    let (mut focus, rows) = expanded(&[6, 4, 1, 3]);
    // Harbor's children sit between rows 0 and 1: row 1 is display 8.
    let effects = run(&mut focus, CommandId::NextMessage, &rows);
    assert_eq!(cursor_of(&effects), Some(1), "first child");
    let _ = run(&mut focus, CommandId::LastMessage, &rows);
    let _ = run(&mut focus, CommandId::PrevMessage, &rows);
    // Back onto row 2 (a single message), then up to row 1.
    let _ = run(&mut focus, CommandId::PrevMessage, &rows);
    assert_eq!(focus.cursor(), Some(8), "row 1 sits after the 7 children");

    let effects = run(&mut focus, CommandId::ExpandThread, &rows);
    let (ticket, thread) = read_asked(&effects).expect("a read of B");
    assert_eq!(thread, ThreadId::new(501));
    let effects = answer(&mut focus, ticket, view(thread, 4, 0, false), &rows);
    assert_eq!(unspliced(&effects), vec![0], "A goes");
    let (anchor, children) = splice_of(&effects).expect("B comes");
    assert_eq!((anchor, children.len()), (1, 4));
    assert_eq!(
        cursor_of(&effects),
        Some(1),
        "B's row, now that A's children are gone"
    );
    assert_eq!(focus.cursor(), Some(1));
}

#[test]
fn j_and_k_walk_into_and_through_the_children() {
    let (mut focus, rows) = expanded(&[6, 4, 1, 3]);
    let mut seen = Vec::new();
    for _ in 0..8 {
        let effects = run(&mut focus, CommandId::NextMessage, &rows);
        seen.push(cursor_of(&effects));
    }
    // Seven children (display 1..=7), then row 1 (display 8), then row 2,
    // and so on to the end of the list: 12 positions in all.
    assert_eq!(
        seen,
        (1..=8).map(Some).collect::<Vec<_>>(),
        "children are walked one by one, then the next row"
    );
    for want in (0..8).rev() {
        let effects = run(&mut focus, CommandId::PrevMessage, &rows);
        assert_eq!(cursor_of(&effects), Some(want));
    }
    assert_eq!(focus.cursor(), Some(0), "back on the thread's row");
    // The list is 4 rows + 7 children: the last position is 10.
    let effects = run(&mut focus, CommandId::LastMessage, &rows);
    assert_eq!(cursor_of(&effects), Some(10));
}

#[test]
fn collapsing_from_a_child_unsplices_and_returns_to_the_row() {
    let (mut focus, rows) = expanded(&[6, 4, 1, 3]);
    for _ in 0..3 {
        let _ = run(&mut focus, CommandId::NextMessage, &rows);
    }
    assert_eq!(focus.cursor(), Some(3));
    let effects = run(&mut focus, CommandId::CollapseThread, &rows);
    assert_eq!(unspliced(&effects), vec![0]);
    assert_eq!(cursor_of(&effects), Some(0), "the thread's row");
    assert_eq!(focus.cursor(), Some(0));
    // Collapsed: j goes to the next row, not into children.
    let effects = run(&mut focus, CommandId::NextMessage, &rows);
    assert_eq!(cursor_of(&effects), Some(1));
}

#[test]
fn expanding_a_thread_of_one_message_does_nothing() {
    let rows = List::of(&[6, 1]);
    let mut focus = focus();
    let _ = focus.open(postio_model::ListScope::Focus(
        postio_model::FocusScope::Inbox,
    ));
    let _ = focus.landed(&rows, true);
    let _ = run(&mut focus, CommandId::NextMessage, &rows);
    // Positive control: the same key on a thread of six reads it.
    let _ = run(&mut focus, CommandId::PrevMessage, &rows);
    assert!(read_asked(&run(&mut focus, CommandId::ExpandThread, &rows)).is_some());
    let mut focus = self::focus();
    let _ = focus.open(postio_model::ListScope::Focus(
        postio_model::FocusScope::Inbox,
    ));
    let _ = focus.landed(&rows, true);
    let _ = run(&mut focus, CommandId::NextMessage, &rows);
    let effects = run(&mut focus, CommandId::ExpandThread, &rows);
    assert!(effects.is_empty(), "no request, no intent: {effects:?}");
}

#[test]
fn the_anchor_leaving_the_list_unsplices_and_the_cursor_moves_to_the_next_row() {
    let (mut focus, rows) = expanded(&[6, 4, 1, 3]);
    for _ in 0..3 {
        let _ = run(&mut focus, CommandId::NextMessage, &rows);
    }
    // Archived elsewhere: the list no longer holds Harbor.
    let rest = rows.without(ThreadId::new(500));
    let effects = focus.landed(&rest, false);
    assert_eq!(unspliced(&effects), vec![0]);
    assert_eq!(
        cursor_of(&effects),
        Some(0),
        "the row that moved up into Harbor's place"
    );
    assert_eq!(focus.cursor(), Some(0));
    let effects = run(&mut focus, CommandId::NextMessage, &rest);
    assert_eq!(cursor_of(&effects), Some(1), "no children left to walk");
}

#[test]
fn a_changed_thread_is_read_again_and_keeps_the_focused_child() {
    let (mut focus, rows) = expanded(&[6, 4, 1, 3]);
    for _ in 0..3 {
        let _ = run(&mut focus, CommandId::NextMessage, &rows);
    }
    let thread = ThreadId::new(500);
    let focused = member_id(thread, 2);
    assert_eq!(focus.cursor(), Some(3), "the third member");

    // A reply arrives.
    let effects = focus.handle_on(
        Input::Event(Event::ThreadChanged {
            account: AccountId::new(1),
            thread,
            members_added: vec![MessageId::new(9001)],
        }),
        &rows,
    );
    let (ticket, asked) = read_asked(&effects).expect("the expansion reads again");
    assert_eq!(asked, thread);

    // An older message is re-linked ahead of the focused one: the same
    // message is now at index 3, not 2.
    let mut changed = view(thread, 6, 3, true);
    changed
        .members
        .insert(1, view(thread, 7, 0, false).members.remove(6));
    let at = changed
        .members
        .iter()
        .position(|member| member.id == focused)
        .expect("kept");
    assert_ne!(at, 2, "the focused message moved");
    let effects = answer(&mut focus, ticket, changed, &rows);
    let (anchor, children) = splice_of(&effects).expect("the children are redrawn");
    assert_eq!(
        (anchor, children.len()),
        (0, 8),
        "seven members and a draft"
    );
    assert_eq!(
        cursor_of(&effects),
        Some(1 + at as u32),
        "the cursor follows the same message"
    );
    assert_eq!(focus.cursor(), Some(1 + at as u32));

    // A change to another thread reads nothing.
    let effects = focus.handle_on(
        Input::Event(Event::ThreadChanged {
            account: AccountId::new(1),
            thread: ThreadId::new(501),
            members_added: Vec::new(),
        }),
        &rows,
    );
    assert!(read_asked(&effects).is_none(), "{effects:?}");
}

#[test]
fn return_on_a_thread_row_opens_the_first_unread_and_on_a_child_that_child() {
    let rows = List::of(&[6, 4]);
    let mut focus = focus();
    let _ = focus.open(postio_model::ListScope::Focus(
        postio_model::FocusScope::Inbox,
    ));
    let _ = focus.landed(&rows, true);

    // The thread is not loaded: it is read first, then opened where
    // `open_at` says (members 4 and 5 are unread).
    let effects = run(&mut focus, CommandId::OpenMessage, &rows);
    assert_eq!(opened_message(&effects), None, "not before it is read");
    let (ticket, thread) = read_asked(&effects).expect("the thread is read first");
    let effects = answer(&mut focus, ticket, view(thread, 6, 2, true), &rows);
    assert_eq!(opened_message(&effects), Some(member_id(thread, 4)));
    assert!(
        splice_of(&effects).is_none(),
        "opening a row does not expand it"
    );

    // On a child: that child's message.
    let (mut focus, rows) = expanded(&[6, 4]);
    let _ = run(&mut focus, CommandId::NextMessage, &rows);
    let _ = run(&mut focus, CommandId::NextMessage, &rows);
    let effects = run(&mut focus, CommandId::OpenMessage, &rows);
    assert_eq!(
        opened_message(&effects),
        Some(member_id(ThreadId::new(500), 1))
    );

    // Expanded, the row's own Return needs no second read.
    let (mut focus, rows) = expanded(&[6, 4]);
    let effects = run(&mut focus, CommandId::OpenMessage, &rows);
    assert!(read_asked(&effects).is_none(), "{effects:?}");
    assert_eq!(
        opened_message(&effects),
        Some(member_id(ThreadId::new(500), 4))
    );
}

// ---- Other surfaces (T035, FR-009): a folder and Filtered -------------------

/// A folder (not the inbox) is the same list surface with another scope.
/// This is a guard, not a TDD step: it was green on arrival.
#[test]
fn a_folder_list_expands_and_collapses_as_the_inbox_does() {
    let rows = List::of(&[6, 4, 1]);
    let mut focus = focus();
    let _ = focus.open(postio_model::ListScope::Mailbox(
        postio_model::MailboxId::new(7),
    ));
    let _ = focus.landed(&rows, true);

    let effects = run(&mut focus, CommandId::ExpandThread, &rows);
    let (ticket, thread) = read_asked(&effects).expect("a folder row reads its thread");
    let effects = answer(&mut focus, ticket, view(thread, 6, 2, true), &rows);
    let (anchor, children) = splice_of(&effects).expect("the read splices");
    assert_eq!((anchor, children.len()), (0, 7));

    let effects = run(&mut focus, CommandId::CollapseThread, &rows);
    assert_eq!(unspliced(&effects), vec![0]);
}

/// Filtered's rows: messages, the first of three messages in its thread.
fn filed_rows(counts: &[u32]) -> Vec<postio_client::protocol::FilteredRow> {
    let at = Utc.with_ymd_and_hms(2026, 10, 2, 9, 0, 0).unwrap();
    counts
        .iter()
        .enumerate()
        .map(|(index, count)| {
            let id = 100 + index as i64;
            postio_client::protocol::FilteredRow {
                message: MessageSummary {
                    id: MessageId::new(id),
                    thread: Some(ThreadId::new(500 + index as i64)),
                    from: Some(EmailAddress::new(Some("Ledger"), "news@ledger.example")),
                    subject: Some(format!("Issue {id}")),
                    preview: None,
                    received_at: at,
                    seen: true,
                    flagged: false,
                    answered: false,
                    send_state: None,
                    send_at: None,
                    has_attachments: false,
                    thread_count: *count,
                    to: Vec::new(),
                },
                reason: "notification".to_owned(),
                source: None,
                at,
            }
        })
        .collect()
}

fn filtered_splice_of(effects: &[Effect]) -> Option<(u32, Vec<ChildRowView>)> {
    shown(effects).into_iter().find_map(|intent| match intent {
        Intent::FilteredSplice { anchor, children } => Some((anchor, children)),
        _ => None,
    })
}

/// `g f`, answered with `counts`' rows.
fn filtered_over(counts: &[u32]) -> (FocusController, List) {
    let rows = List::of(&[1]);
    let mut focus = focus();
    let _ = focus.landed(&rows, true);
    let page = filed_rows(counts);
    for (ticket, request) in asks(&run(&mut focus, CommandId::GoToFiltered, &rows)) {
        let reply = match request {
            Request::FilteredTabs => Reply::FilteredTabs(Ok(Vec::new())),
            Request::Filtered { stamp, offset, .. } => Reply::Filtered {
                stamp,
                offset,
                answer: Ok(page.clone()),
            },
            other => panic!("g f asked for {other:?}"),
        };
        let _ = focus.handle_on(Input::Reply(ticket, reply), &rows);
    }
    (focus, rows)
}

#[test]
fn filtered_expands_and_collapses_a_thread_row() {
    let (mut focus, rows) = filtered_over(&[6, 1, 3]);
    let effects = run(&mut focus, CommandId::ExpandThread, &rows);
    let (ticket, thread) = read_asked(&effects).expect("Right in Filtered reads the thread");
    assert_eq!(thread, ThreadId::new(500));
    let effects = answer(&mut focus, ticket, view(thread, 6, 2, true), &rows);
    let (anchor, children) = filtered_splice_of(&effects).expect("the read splices over Filtered");
    assert_eq!((anchor, children.len()), (0, 7));

    // j walks into the children, as in the inbox.
    let effects = run(&mut focus, CommandId::NextMessage, &rows);
    assert!(
        shown(&effects)
            .iter()
            .any(|intent| matches!(intent, Intent::FilteredFocus(Some(1)))),
        "the first child has the keyboard: {:?}",
        shown(&effects)
    );

    let effects = run(&mut focus, CommandId::CollapseThread, &rows);
    assert!(
        shown(&effects)
            .iter()
            .any(|intent| matches!(intent, Intent::FilteredUnsplice { anchor: 0 })),
        "{:?}",
        shown(&effects)
    );
}

#[test]
fn filtered_row_of_one_message_does_not_expand() {
    let (mut focus, rows) = filtered_over(&[1, 3]);
    let effects = run(&mut focus, CommandId::ExpandThread, &rows);
    assert!(read_asked(&effects).is_none());
}

#[test]
fn filtered_keeps_its_open_thread_and_the_child_focus_when_the_page_is_read_again() {
    let (mut focus, rows) = filtered_over(&[6, 1, 3]);
    let effects = run(&mut focus, CommandId::ExpandThread, &rows);
    let (ticket, thread) = read_asked(&effects).expect("a read");
    let _ = answer(&mut focus, ticket, view(thread, 6, 2, true), &rows);
    let _ = run(&mut focus, CommandId::NextMessage, &rows);
    let _ = run(&mut focus, CommandId::NextMessage, &rows);

    // Mail moved elsewhere: Filtered reads its first page again.
    let page = filed_rows(&[6, 1, 3]);
    let effects = focus.handle_on(
        Input::Event(Event::MessageListChanged {
            account: AccountId::new(1),
            mailbox: postio_model::MailboxId::new(1),
        }),
        &rows,
    );
    let (ticket, request) = asks(&effects)
        .into_iter()
        .find(|(_, request)| matches!(request, Request::Filtered { .. }))
        .expect("Filtered is read again");
    let Request::Filtered { stamp, offset, .. } = request else {
        unreachable!()
    };
    let effects = focus.handle_on(
        Input::Reply(
            ticket,
            Reply::Filtered {
                stamp,
                offset,
                answer: Ok(page),
            },
        ),
        &rows,
    );
    let view = shown(&effects)
        .into_iter()
        .find_map(|intent| match intent {
            Intent::Filtered(view) => Some(*view),
            _ => None,
        })
        .expect("the view is drawn again");
    assert_eq!(view.expanded.as_ref().map(|x| x.children.len()), Some(7));
    assert_eq!(view.focused, Some(2), "still on the second child");
}

/// D2: Restore moves mail, so on a child it moves the thread (every
/// member), not the child message.
#[test]
fn restore_on_a_filtered_child_aims_the_whole_thread() {
    let (mut focus, rows) = filtered_over(&[6, 1, 3]);
    let effects = run(&mut focus, CommandId::ExpandThread, &rows);
    let (ticket, thread) = read_asked(&effects).expect("a read");
    let _ = answer(&mut focus, ticket, view(thread, 6, 2, true), &rows);
    let _ = run(&mut focus, CommandId::NextMessage, &rows);

    let effects = run(&mut focus, CommandId::RestoreFiltered, &rows);
    let targets: Vec<_> = asks(&effects)
        .into_iter()
        .filter_map(|(_, request)| match request {
            Request::Post(postio_core::Command::RestoreFiltered { target, .. }) => Some(target),
            _ => None,
        })
        .collect();
    assert_eq!(
        targets,
        vec![postio_core::MessageTarget::Threads(vec![thread])],
        "the child's whole thread"
    );
}

// ---------------------------------------------------------------------
// Spec 013 US2: every action says what it acts on (T037, T041).
// ---------------------------------------------------------------------

/// Harbor as the design draws it: message 4 (index 3) is Ben Adeyemi's.
fn harbor(thread: ThreadId) -> ThreadView {
    let mut view = view(thread, 6, 2, true);
    view.members[3].from = Some(EmailAddress::new(Some("Ben Adeyemi"), "ben@example.com"));
    view
}

/// Harbor expanded as [`expanded`] does, but with the design's names.
fn harbor_expanded(counts: &[u32]) -> (FocusController, List) {
    let rows = List::of(counts);
    let mut focus = focus();
    let _ = focus.open(postio_model::ListScope::Focus(
        postio_model::FocusScope::Inbox,
    ));
    let _ = focus.landed(&rows, true);
    let effects = run(&mut focus, CommandId::ExpandThread, &rows);
    let (ticket, thread) = read_asked(&effects).expect("a read");
    let _ = answer(&mut focus, ticket, harbor(thread), &rows);
    (focus, rows)
}

/// The footer an effect list draws last, if any.
fn footer_of(effects: &[Effect]) -> Option<postio_ui::scope::FooterView> {
    shown(effects)
        .into_iter()
        .rev()
        .find_map(|intent| match intent {
            Intent::Footer(view) => Some(view),
            _ => None,
        })
}

/// What `command` pressed on the cursor's row would aim at: the targets of
/// the verb it sends, or of the picker it opens.
fn aimed_by(
    focus: &mut FocusController,
    command: CommandId,
    rows: &List,
) -> Vec<postio_core::MessageTarget> {
    let effects = run(focus, command, rows);
    asks(&effects)
        .into_iter()
        .filter_map(|(_, request)| match request {
            Request::Send { aims, .. } => Some(aims),
            Request::Labels {
                threads, message, ..
            } => Some(match message {
                Some(message) if threads.is_empty() => {
                    vec![postio_core::MessageTarget::Messages(vec![message])]
                }
                _ => vec![postio_core::MessageTarget::Threads(threads)],
            }),
            _ => None,
        })
        .flatten()
        .collect()
}

/// Down to Ben Adeyemi's message 4: the fourth child.
fn on_child_four(focus: &mut FocusController, rows: &List) {
    for _ in 0..4 {
        let _ = run(focus, CommandId::NextMessage, rows);
    }
    assert_eq!(focus.cursor(), Some(4));
}

#[test]
fn reply_on_a_child_answers_that_message_and_on_the_row_the_latest() {
    let (mut focus, rows) = harbor_expanded(&[6, 4, 1, 3]);
    let effects = run(&mut focus, CommandId::Reply, &rows);
    let on_row = shown(&effects).into_iter().find_map(|intent| match intent {
        Intent::Composer { message, .. } => message,
        _ => None,
    });
    assert_eq!(on_row, Some(MessageId::new(100)), "the row's own message");
    let _ = run(&mut focus, CommandId::Back, &rows);

    on_child_four(&mut focus, &rows);
    let effects = run(&mut focus, CommandId::Reply, &rows);
    let on_child = shown(&effects).into_iter().find_map(|intent| match intent {
        Intent::Composer { message, .. } => message,
        _ => None,
    });
    assert_eq!(on_child, Some(member_id(ThreadId::new(500), 3)));
}

#[test]
fn archive_on_a_child_aims_the_whole_thread() {
    let (mut focus, rows) = harbor_expanded(&[6, 4, 1, 3]);
    on_child_four(&mut focus, &rows);
    assert_eq!(
        aimed_by(&mut focus, CommandId::Archive, &rows),
        vec![postio_core::MessageTarget::Threads(vec![ThreadId::new(
            500
        )])]
    );
}

#[test]
fn read_and_unread_on_a_child_is_that_message_s_and_on_the_row_the_thread_s() {
    let (mut focus, rows) = harbor_expanded(&[6, 4, 1, 3]);
    assert_eq!(
        aimed_by(&mut focus, CommandId::ToggleRead, &rows),
        vec![postio_core::MessageTarget::Threads(vec![ThreadId::new(
            500
        )])],
        "on the row: the thread"
    );
    on_child_four(&mut focus, &rows);
    assert_eq!(
        aimed_by(&mut focus, CommandId::ToggleRead, &rows),
        vec![postio_core::MessageTarget::Messages(vec![member_id(
            ThreadId::new(500),
            3
        )])],
        "on a child: this message"
    );
}

#[test]
fn move_or_delete_only_this_is_refused_on_a_collapsed_row() {
    let rows = List::of(&[6, 4]);
    let mut focus = focus();
    let _ = focus.open(postio_model::ListScope::Focus(
        postio_model::FocusScope::Inbox,
    ));
    let _ = focus.landed(&rows, true);
    let _ = run(&mut focus, CommandId::NextMessage, &rows);
    for command in [CommandId::MoveThisMessage, CommandId::DeleteThisMessage] {
        let effects = run(&mut focus, command, &rows);
        assert!(
            asks(&effects).is_empty() && shown(&effects).is_empty(),
            "{command:?} does nothing on a thread's row: {effects:?}"
        );
    }
}

/// D2: on a child, "move only this" opens the Move picker aimed at that
/// message alone, and choosing a folder moves that message.
#[test]
fn move_only_this_on_a_child_opens_the_picker_aimed_at_that_message() {
    let (mut focus, rows) = harbor_expanded(&[6, 4, 1, 3]);
    on_child_four(&mut focus, &rows);
    let effects = run(&mut focus, CommandId::MoveThisMessage, &rows);
    assert!(
        shown(&effects)
            .iter()
            .any(|intent| matches!(intent, Intent::OpenPicker(_))),
        "the Move picker opens: {effects:?}"
    );
    let (ticket, stamp) = asks(&effects)
        .into_iter()
        .find_map(|(ticket, request)| match request {
            Request::Folders { stamp } => Some((ticket, stamp)),
            _ => None,
        })
        .expect("the picker reads the folders");
    let mut travel = postio_model::Mailbox::new(AccountId::new(1), "Travel", None);
    travel.id = postio_model::MailboxId::new(3);
    let _ = focus.handle_on(
        Input::Reply(
            ticket,
            Reply::Folders {
                stamp,
                answer: Ok(postio_focus::FoldersRead {
                    folders: vec![travel],
                    recent: vec![postio_model::MailboxId::new(3)],
                }),
            },
        ),
        &rows,
    );
    let effects = run(&mut focus, CommandId::PickerChoose1, &rows);
    let aims: Vec<_> = asks(&effects)
        .into_iter()
        .filter_map(|(_, request)| match request {
            Request::Send { aims, .. } => Some(aims),
            _ => None,
        })
        .flatten()
        .collect();
    assert_eq!(
        aims,
        vec![postio_core::MessageTarget::Messages(vec![member_id(
            ThreadId::new(500),
            3
        )])],
        "only the child moves"
    );
}

#[test]
fn delete_only_this_on_a_child_deletes_that_message_and_the_cursor_stays_among_the_children() {
    let (mut focus, rows) = harbor_expanded(&[6, 4, 1, 3]);
    on_child_four(&mut focus, &rows);
    assert_eq!(
        aimed_by(&mut focus, CommandId::DeleteThisMessage, &rows),
        vec![postio_core::MessageTarget::Messages(vec![member_id(
            ThreadId::new(500),
            3
        )])],
        "only the child"
    );
    // The thread is read again without it.
    let thread = ThreadId::new(500);
    let effects = focus.handle_on(
        Input::Event(Event::ThreadChanged {
            account: AccountId::new(1),
            thread,
            members_added: Vec::new(),
        }),
        &rows,
    );
    let (ticket, _) = read_asked(&effects).expect("the expansion reads again");
    let mut left = harbor(thread);
    left.members.remove(3);
    let effects = answer(&mut focus, ticket, left, &rows);
    assert_eq!(
        focus.cursor(),
        Some(4),
        "the cursor stays on the child that took its place, not the row: {effects:?}"
    );
}

#[test]
fn the_footer_names_the_thread_on_its_row_and_the_message_on_a_child() {
    let (mut focus, rows) = harbor_expanded(&[6, 4, 1, 3]);
    let effects = run(&mut focus, CommandId::NextMessage, &rows);
    assert_eq!(
        footer_of(&effects).map(|f| f.title).as_deref(),
        Some("Message 1 of 6 \u{b7} Ben Ortiz")
    );
    let effects = run(&mut focus, CommandId::PrevMessage, &rows);
    assert_eq!(
        footer_of(&effects).map(|f| f.title).as_deref(),
        Some("Harbor API draft v3 \u{b7} 6 messages"),
        "the row says its thread and its size"
    );
    on_child_four(&mut focus, &rows);
    let effects = run(&mut focus, CommandId::PrevMessage, &rows);
    let _ = effects;
    let effects = run(&mut focus, CommandId::NextMessage, &rows);
    assert_eq!(
        footer_of(&effects).map(|f| f.title).as_deref(),
        Some("Message 4 of 6 \u{b7} Ben Adeyemi")
    );
}

fn labels(footer: &postio_ui::scope::FooterView) -> Vec<&str> {
    footer
        .keys
        .iter()
        .map(|(_, label)| label.as_str())
        .collect()
}

#[test]
fn the_footer_s_labels_say_this_on_a_child_and_latest_on_a_row() {
    let (mut focus, rows) = harbor_expanded(&[6, 4, 1, 3]);
    let effects = run(&mut focus, CommandId::NextMessage, &rows);
    let child = footer_of(&effects).expect("a footer on the child");
    let said = labels(&child);
    for want in ["reply to this", "forward this", "archive thread"] {
        assert!(said.contains(&want), "{want} in {said:?}");
    }

    let effects = run(&mut focus, CommandId::CollapseThread, &rows);
    let row = footer_of(&effects).expect("a footer on the row");
    let said = labels(&row);
    for want in ["reply to latest", "forward latest", "archive thread"] {
        assert!(said.contains(&want), "{want} in {said:?}");
    }
    assert!(!said.contains(&"reply to this"));
}

#[test]
fn the_footer_s_keycaps_and_note_are_the_keymaps_not_the_designs() {
    let (mut focus, rows) = harbor_expanded(&[6, 4, 1, 3]);
    for _ in 0..3 {
        let _ = run(&mut focus, CommandId::NextMessage, &rows);
    }
    let effects = run(&mut focus, CommandId::NextMessage, &rows);
    let footer = footer_of(&effects).expect("a footer");
    let keymap = focus.keymap().clone();
    let key_of = |footer: &postio_ui::scope::FooterView, label: &str| {
        footer
            .keys
            .iter()
            .find(|(_, said)| said == label)
            .map(|(cap, _)| cap.clone())
    };
    let reply = keymap.binding(CommandId::Reply).expect("reply is bound");
    assert_eq!(key_of(&footer, "reply to this").as_deref(), Some(reply));
    let [archive, snooze, label, mv] = [
        CommandId::Archive,
        CommandId::Snooze,
        CommandId::AddLabel,
        CommandId::Move,
    ]
    .map(|id| keymap.binding(id).expect("bound").to_owned());
    let note = format!(
        "Reply and forward act on this message; {archive}, {snooze}, {label}, {mv} act on the thread"
    );
    assert_eq!(footer.note.as_deref(), Some(note.as_str()));
    assert_ne!(
        footer.note.as_deref(),
        Some("Reply and forward act on this message; a, s, l, m act on the thread"),
        "the design's keys are not the registry's"
    );

    // Rebind reply; the next footer follows.
    let mut overrides = postio_config::KeyBindings::default();
    overrides
        .overrides_mut()
        .insert("reply".to_owned(), "alt+q".to_owned());
    let rebound = postio_core::Keymap::resolve_on(&overrides, Platform::Apple);
    assert_eq!(rebound.binding(CommandId::Reply), Some("alt+q"));
    let _ = focus.handle_on(Input::Keymap(rebound), &rows);
    let _ = run(&mut focus, CommandId::PrevMessage, &rows);
    let effects = run(&mut focus, CommandId::NextMessage, &rows);
    let footer = footer_of(&effects).expect("a footer");
    assert_eq!(key_of(&footer, "reply to this").as_deref(), Some("alt+q"));
}

#[test]
fn a_one_message_row_s_footer_has_no_thread_wording() {
    let (mut focus, rows) = harbor_expanded(&[6, 4, 1, 3]);
    // Row 2 (one message) is display 9 behind Harbor's seven children and
    // row 1.
    let _ = run(&mut focus, CommandId::LastMessage, &rows);
    let effects = run(&mut focus, CommandId::PrevMessage, &rows);
    assert_eq!(focus.cursor(), Some(9));
    let footer = footer_of(&effects).expect("a footer");
    assert_eq!(footer.title, "Harbor API draft v3");
    assert_eq!(footer.note, None);
    for said in labels(&footer) {
        assert!(
            !said.contains("thread") && !said.contains("latest") && !said.contains("this"),
            "{said}"
        );
    }
}

#[test]
fn filtered_s_footer_follows_its_children_too() {
    let (mut focus, rows) = filtered_over(&[6, 1, 3]);
    let effects = run(&mut focus, CommandId::ExpandThread, &rows);
    let (ticket, thread) = read_asked(&effects).expect("a read");
    let _ = answer(&mut focus, ticket, harbor(thread), &rows);
    let effects = run(&mut focus, CommandId::NextMessage, &rows);
    assert_eq!(
        footer_of(&effects).map(|f| f.title).as_deref(),
        Some("Message 1 of 6 \u{b7} Ben Ortiz")
    );
}

/// T041, the guard: a label added from a child is on every member of the
/// thread, and one undo takes it off all of them. The write-through is the
/// engine's (spec 013 D3), so this is green on arrival; it keeps the child's
/// aim from ever narrowing a thread verb to one message.
#[test]
fn a_label_added_from_a_child_aims_the_thread_and_one_undo_undoes_it() {
    let (mut focus, rows) = harbor_expanded(&[6, 4, 1, 3]);
    on_child_four(&mut focus, &rows);
    let effects = run(&mut focus, CommandId::AddLabel, &rows);
    assert!(
        asks(&effects).iter().any(|(_, request)| matches!(
            request,
            Request::Labels { threads, .. } if *threads == [ThreadId::new(500)]
        )),
        "the label picker is for the thread, not message 4: {effects:?}"
    );
    let effects = run(&mut focus, CommandId::Undo, &rows);
    assert!(
        asks(&effects)
            .iter()
            .any(|(_, request)| matches!(request, Request::Post(postio_core::Command::Undo))),
        "one undo is one command: {effects:?}"
    );
}

// ---- Spec 013 US3: walking a thread in the message window ----

/// What the window was told, as one record.
#[derive(Default, Debug)]
pub(crate) struct Window {
    pub(crate) shown: Option<MessageId>,
    pub(crate) bar: Option<postio_ui::thread_bar::ThreadBarView>,
    pub(crate) kicker: Option<String>,
    pub(crate) banner: Option<Option<postio_ui::thread_bar::BannerView>>,
    pub(crate) reply_to: Option<Option<String>>,
}

pub(crate) fn window_of(effects: &[Effect]) -> Window {
    let mut window = Window::default();
    for intent in shown(effects) {
        match intent {
            Intent::OpenMessage { message, .. } => window.shown = Some(message),
            Intent::ThreadBar(bar) => window.bar = Some(bar),
            Intent::Kicker(kicker) => window.kicker = Some(kicker),
            Intent::OlderBanner(banner) => window.banner = Some(banner),
            Intent::ReplyTo(reply_to) => window.reply_to = Some(reply_to),
            _ => {}
        }
    }
    window
}

/// The index of the ringed chip.
pub(crate) fn ringed(bar: &postio_ui::thread_bar::ThreadBarView) -> Option<u32> {
    bar.chips.iter().find_map(|chip| match chip {
        postio_ui::thread_bar::Chip::Message { index, current, .. } if *current => Some(*index),
        _ => None,
    })
}

pub(crate) fn marked_read(effects: &[Effect]) -> Vec<MessageId> {
    asked_requests(effects)
        .into_iter()
        .filter_map(|request| match request {
            Request::Post(postio_core::Command::MarkReadOnDwell { message }) => Some(message),
            _ => None,
        })
        .collect()
}

fn asked_requests(effects: &[Effect]) -> Vec<Request> {
    asks(effects)
        .into_iter()
        .map(|(_, request)| request)
        .collect()
}

/// Harbor (row 0, six messages, the last two unread) opened with Return: the
/// window is on message 4 (index 4), the first unread.
pub(crate) fn window_on_harbor(unread: usize) -> (FocusController, List, Window) {
    let rows = List::of(&[6, 4, 1, 3]);
    let mut focus = focus();
    let _ = focus.open(postio_model::ListScope::Focus(
        postio_model::FocusScope::Inbox,
    ));
    let _ = focus.landed(&rows, true);
    let effects = run(&mut focus, CommandId::OpenMessage, &rows);
    let (ticket, thread) = read_asked(&effects).expect("opening a thread reads it");
    let effects = answer(&mut focus, ticket, view(thread, 6, unread, false), &rows);
    let window = window_of(&effects);
    (focus, rows, window)
}

#[test]
fn opening_a_thread_draws_the_bar_the_kicker_and_the_banner() {
    let (_focus, _rows, window) = window_on_harbor(2);
    assert_eq!(window.shown, Some(member_id(ThreadId::new(500), 4)));
    let bar = window.bar.expect("the bar");
    assert_eq!(bar.count_line, "6 \u{b7} 2 new");
    assert_eq!(bar.chips.len(), 6);
    assert_eq!(ringed(&bar), Some(4));
    assert_eq!(
        window.kicker.as_deref(),
        Some("Message 5 of 6 \u{b7} ] newer \u{b7} [ older")
    );
    assert!(
        matches!(window.banner, Some(Some(_))),
        "message 5 of 6 is not the latest"
    );
    assert_eq!(window.reply_to, Some(None));
}

#[test]
fn bracket_steps_older_and_newer_and_a_newer_unread_is_marked_read() {
    let (mut focus, rows, _) = window_on_harbor(2);
    let thread = ThreadId::new(500);
    let effects = run(&mut focus, CommandId::PrevInConversation, &rows);
    let window = window_of(&effects);
    assert_eq!(window.shown, Some(member_id(thread, 3)), "[ is older");
    assert_eq!(ringed(&window.bar.expect("bar")), Some(3));
    assert_eq!(
        window.kicker.as_deref().map(|k| &k[..14]),
        Some("Message 4 of 6")
    );
    assert!(matches!(window.banner, Some(Some(_))));
    assert!(marked_read(&effects).is_empty(), "message 4 was read");

    let effects = run(&mut focus, CommandId::NextInConversation, &rows);
    assert_eq!(window_of(&effects).shown, Some(member_id(thread, 4)));
    assert_eq!(
        marked_read(&effects),
        vec![member_id(thread, 4)],
        "showing an unread message marks it read (FR-017)"
    );
    let effects = run(&mut focus, CommandId::NextInConversation, &rows);
    let window = window_of(&effects);
    assert_eq!(window.shown, Some(member_id(thread, 5)));
    assert_eq!(marked_read(&effects), vec![member_id(thread, 5)]);
    assert_eq!(
        window.bar.as_ref().map(|bar| bar.count_line.as_str()),
        Some("6"),
        "both unread are read now"
    );
    assert_eq!(window.banner, Some(None), "the latest has no banner");
}

#[test]
fn newer_on_the_latest_does_nothing() {
    let (mut focus, rows, _) = window_on_harbor(2);
    let _ = run(&mut focus, CommandId::LatestInConversation, &rows);
    let effects = run(&mut focus, CommandId::NextInConversation, &rows);
    assert!(shown(&effects).is_empty(), "{effects:?}");
    assert!(asks(&effects).is_empty(), "{effects:?}");
}

#[test]
fn first_and_latest_go_to_the_ends() {
    let (mut focus, rows, _) = window_on_harbor(2);
    let thread = ThreadId::new(500);
    let effects = run(&mut focus, CommandId::FirstInConversation, &rows);
    let window = window_of(&effects);
    assert_eq!(window.shown, Some(member_id(thread, 0)));
    assert_eq!(ringed(&window.bar.expect("bar")), Some(0));
    let effects = run(&mut focus, CommandId::LatestInConversation, &rows);
    let window = window_of(&effects);
    assert_eq!(window.shown, Some(member_id(thread, 5)));
    assert_eq!(window.banner, Some(None));
    assert_eq!(marked_read(&effects), vec![member_id(thread, 5)]);
}

#[test]
fn g_u_goes_to_the_first_unread_and_stays_put_with_none() {
    let (mut focus, rows, _) = window_on_harbor(2);
    let thread = ThreadId::new(500);
    let _ = run(&mut focus, CommandId::FirstInConversation, &rows);
    let effects = run(&mut focus, CommandId::FirstUnreadInConversation, &rows);
    assert_eq!(window_of(&effects).shown, Some(member_id(thread, 4)));

    let (mut focus, rows, window) = window_on_harbor(0);
    assert_eq!(
        window.shown,
        Some(member_id(thread, 5)),
        "nothing unread opens on the latest"
    );
    let effects = run(&mut focus, CommandId::FirstUnreadInConversation, &rows);
    assert!(shown(&effects).is_empty(), "stays put: {effects:?}");
}

#[test]
fn j_in_the_window_still_opens_the_next_thread() {
    let (mut focus, rows, _) = window_on_harbor(2);
    let effects = run(&mut focus, CommandId::NextMessage, &rows);
    assert_eq!(opened_message(&effects), Some(MessageId::new(101)));
    let (_, thread) = read_asked(&effects).expect("the new window reads its thread");
    assert_eq!(thread, ThreadId::new(501));
}

#[test]
fn the_room_the_window_has_decides_how_many_chips_are_drawn() {
    let (mut focus, rows, window) = window_on_harbor(2);
    assert_eq!(window.bar.expect("bar").chips.len(), 6);
    let effects = focus.handle_on(Input::BarRoom(4), &rows);
    let bar = window_of(&effects).bar.expect("the bar is composed again");
    assert!(
        bar.chips.len() <= 5
            && bar
                .chips
                .iter()
                .any(|chip| matches!(chip, postio_ui::thread_bar::Chip::More { .. })),
        "{bar:?}"
    );
    assert_eq!(ringed(&bar), Some(4), "the message on screen stays kept");
    let effects = focus.handle_on(Input::BarRoom(8), &rows);
    assert_eq!(window_of(&effects).bar.expect("bar").chips.len(), 6);
}

#[test]
fn closing_the_window_forgets_the_walk() {
    let (mut focus, rows, _) = window_on_harbor(2);
    let _ = run(&mut focus, CommandId::Back, &rows);
    let effects = focus.handle_on(Input::BarRoom(4), &rows);
    assert!(shown(&effects).is_empty(), "no window, no bar: {effects:?}");
}

/// How many thread reads the effects ask for.
fn reads_asked(effects: &[Effect]) -> usize {
    asked_requests(effects)
        .iter()
        .filter(|request| matches!(request, Request::ReadThread(_)))
        .count()
}

#[test]
fn a_hundred_opens_of_a_thread_ask_for_it_once_each_and_its_answer_asks_for_nothing() {
    let rows = List::of(&[6, 4, 1, 3]);
    let mut focus = focus();
    let _ = focus.open(postio_model::ListScope::Focus(
        postio_model::FocusScope::Inbox,
    ));
    let _ = focus.landed(&rows, true);
    for _ in 0..100 {
        let effects = run(&mut focus, CommandId::OpenMessage, &rows);
        assert_eq!(reads_asked(&effects), 1, "an open reads its thread once");
        let (ticket, thread) = read_asked(&effects).expect("the read");
        let effects = answer(&mut focus, ticket, view(thread, 6, 2, false), &rows);
        assert!(
            asks(&effects).is_empty(),
            "the answer to a read asks for nothing: {effects:?}"
        );
        // The frontend's own echo of what it was shown must not begin the
        // window again.
        let echo = focus.handle_on(
            Input::Shown(Some(window_of(&effects).shown.unwrap())),
            &rows,
        );
        assert_eq!(reads_asked(&echo), 0, "{echo:?}");
        let effects = run(&mut focus, CommandId::Back, &rows);
        assert_eq!(reads_asked(&effects), 0, "closing reads nothing");
    }
}

#[test]
fn a_row_of_one_message_asks_for_no_thread_read() {
    let rows = List::of(&[1, 4]);
    let mut focus = focus();
    let _ = focus.open(postio_model::ListScope::Focus(
        postio_model::FocusScope::Inbox,
    ));
    let _ = focus.landed(&rows, true);
    let effects = run(&mut focus, CommandId::OpenMessage, &rows);
    assert_eq!(
        opened_message(&effects),
        Some(MessageId::new(100)),
        "{effects:?}"
    );
    assert_eq!(reads_asked(&effects), 0, "{effects:?}");
}
