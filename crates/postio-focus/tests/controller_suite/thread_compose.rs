//! Replying to an older message, forwarding a thread (spec 013 US7,
//! FR-022, FR-023; decision T3).
//!
//! Driven as a frontend would: a fake list, the keys the registry binds,
//! the draft the composer holds when the controller asks for it, and the
//! replies a client would give.

use crate::search_support::{asks, shown};
use chrono::{TimeZone, Utc};
use postio_config::paths::Platform;
use postio_core::CommandId;
use postio_focus::{
    ComposerKind, DraftEdit, Effect, FocusController, Input, Intent, Policy, Reply, Request,
    RowFacts, Rows, Ticket, ToastKind,
};
use postio_model::listing::MessageSummary;
use postio_model::thread_view::{Line, ThreadMember, ThreadView};
use postio_model::{AccountId, Draft, EmailAddress, ForwardForm, Forwarded, MessageId, ThreadId};
use postio_ui::compose::{ChooserView, OlderReplyView};
use postio_ui::focus_list::{FocusRow, lone};

struct List {
    /// How many messages each row's thread holds.
    messages: u32,
}

impl Rows for List {
    fn len(&self) -> u32 {
        2
    }
    fn facts(&self, position: u32) -> Option<RowFacts> {
        (position < 2).then(|| RowFacts {
            id: MessageId::new(100 + i64::from(position)),
            digest: false,
            threads: vec![ThreadId::new(500 + i64::from(position))],
            writes: false,
        })
    }
    fn position_of(&self, message: MessageId) -> Option<u32> {
        let at = message.get() - 100;
        (0..2).contains(&at).then_some(at as u32)
    }
    fn row(&self, position: u32) -> Option<FocusRow> {
        (position < 2).then(|| {
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
                thread_count: self.messages,
                to: Vec::new(),
            });
            summary.message_count = self.messages;
            FocusRow::conversation(summary)
        })
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

fn run(focus: &mut FocusController, command: CommandId, rows: &List) -> Vec<Effect> {
    focus.handle_on(Input::Command(command), rows)
}

/// The inbox, landed, its cursor on Harbor.
fn inbox(messages: u32) -> (FocusController, List) {
    let rows = List { messages };
    let mut focus = focus();
    let _ = focus.open(postio_model::ListScope::Focus(
        postio_model::FocusScope::Inbox,
    ));
    let _ = focus.landed(&rows, true);
    (focus, rows)
}

/// Harbor opened with Return: the window is on message 5 (index 4), the
/// first unread, and its thread is walked.
fn window() -> (FocusController, List) {
    let (mut focus, rows) = inbox(6);
    let effects = run(&mut focus, CommandId::OpenMessage, &rows);
    let (ticket, thread) = asks(&effects)
        .into_iter()
        .find_map(|(ticket, request)| match request {
            Request::ReadThread(thread) => Some((ticket, thread)),
            _ => None,
        })
        .expect("opening a thread reads it");
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

/// The window on message 4 of 6, Ben's.
fn window_on_message_4() -> (FocusController, List) {
    let (mut focus, rows) = window();
    let _ = run(&mut focus, CommandId::PrevInConversation, &rows);
    (focus, rows)
}

fn intents(effects: &[Effect]) -> Vec<Intent> {
    shown(effects)
}

fn requests(effects: &[Effect]) -> Vec<(Ticket, Request)> {
    asks(effects)
}

fn chooser_of(effects: &[Effect]) -> Option<ChooserView> {
    intents(effects)
        .into_iter()
        .rev()
        .find_map(|intent| match intent {
            Intent::ForwardChooser(view) => Some(*view),
            _ => None,
        })
}

fn older_reply_of(effects: &[Effect]) -> Option<Option<OlderReplyView>> {
    intents(effects)
        .into_iter()
        .rev()
        .find_map(|intent| match intent {
            Intent::OlderReply(view) => Some(view.map(|view| *view)),
            _ => None,
        })
}

fn composer_of(effects: &[Effect]) -> Option<(ComposerKind, Option<MessageId>)> {
    intents(effects)
        .into_iter()
        .find_map(|intent| match intent {
            Intent::Composer { kind, message } => Some((kind, message)),
            _ => None,
        })
}

fn draft(subject: &str) -> Draft {
    let mut draft = Draft::new(AccountId::new(1));
    draft.subject = subject.to_owned();
    draft.body_markdown = Some("typed by the person".to_owned());
    draft.forwarded_from = Some(MessageId::new(50003));
    draft
}

/// Press `command` with the composer on top and give the controller the
/// draft it asks for; the effects after the draft arrived.
fn with_draft(
    focus: &mut FocusController,
    rows: &List,
    command: CommandId,
    held: &Draft,
) -> (Vec<Effect>, Vec<Effect>) {
    let asked = run(focus, command, rows);
    let edit = intents(&asked)
        .into_iter()
        .find_map(|intent| match intent {
            Intent::ComposerDraftWanted(edit) => Some(edit),
            _ => None,
        })
        .expect("the controller asks the composer for its draft");
    let given = focus.handle_on(Input::ComposerDraft(edit, Box::new(held.clone())), rows);
    (asked, given)
}

fn answer(
    focus: &mut FocusController,
    rows: &List,
    effects: &[Effect],
    reply: Reply,
) -> Vec<Effect> {
    let (ticket, _) = requests(effects).into_iter().next().expect("a request");
    focus.handle_on(Input::Reply(ticket, reply), rows)
}

// ---- The forward chooser ----

#[test]
fn f_in_a_thread_of_six_opens_the_composer_with_the_chooser_on_this_message() {
    let (mut focus, rows) = window_on_message_4();
    let effects = run(&mut focus, CommandId::Forward, &rows);
    assert_eq!(
        composer_of(&effects),
        Some((
            ComposerKind::Forward {
                form: ForwardForm::This
            },
            Some(member_id(ThreadId::new(500), 3))
        ))
    );
    let chooser = chooser_of(&effects).expect("a thread of six asks what to forward");
    assert_eq!(chooser.selected, ForwardForm::This);
    let numbers: Vec<_> = chooser.cards.iter().map(|card| card.key.clone()).collect();
    assert_eq!(
        numbers,
        ["1", "2", "3"].map(|n| Some(n.to_owned())),
        "three cards, each with its number key"
    );
    assert_eq!(
        chooser.cards[0].detail,
        "Only Ben\u{2019}s Monday message, quoted as usual."
    );
    assert!(chooser.cards[1].detail.contains("All 6 messages"));
}

#[test]
fn f_on_a_collapsed_row_of_a_thread_asks_the_same_question() {
    let (mut focus, rows) = inbox(6);
    let effects = run(&mut focus, CommandId::Forward, &rows);
    assert!(
        chooser_of(&effects).is_some(),
        "the row says six messages: {:?}",
        intents(&effects)
    );
}

#[test]
fn f_in_a_one_message_thread_shows_no_chooser() {
    let (mut focus, rows) = inbox(1);
    let effects = run(&mut focus, CommandId::Forward, &rows);
    assert_eq!(
        composer_of(&effects),
        Some((
            ComposerKind::Forward {
                form: ForwardForm::This
            },
            Some(MessageId::new(100))
        ))
    );
    assert_eq!(chooser_of(&effects), None);
    // And the chooser's keys have nothing to choose.
    let two = run(&mut focus, CommandId::ForwardWhat2, &rows);
    assert!(intents(&two).is_empty() && requests(&two).is_empty());
}

#[test]
fn two_switches_to_the_whole_thread_through_set_forward_form() {
    let (mut focus, rows) = window_on_message_4();
    let _ = run(&mut focus, CommandId::Forward, &rows);
    let held = draft("Fwd: Harbor API draft v3");
    let (asked, given) = with_draft(&mut focus, &rows, CommandId::ForwardWhat2, &held);
    assert!(
        requests(&asked).is_empty(),
        "the draft is the composer's: nothing is asked of the client before it arrives"
    );
    let all = requests(&given);
    assert_eq!(
        all.iter().map(|(_, r)| r.clone()).collect::<Vec<_>>(),
        vec![Request::SetForwardForm {
            draft: Box::new(held.clone()),
            form: ForwardForm::Thread
        }]
    );
    let mut whole = draft("Fwd: Harbor API draft v3 (6 messages)");
    whole.forwarded_from = held.forwarded_from;
    let done = answer(
        &mut focus,
        &rows,
        &given,
        Reply::Forwarded {
            form: ForwardForm::Thread,
            answer: Ok(Forwarded::Draft(Box::new(whole.clone()))),
        },
    );
    assert_eq!(
        chooser_of(&done).map(|c| c.selected),
        Some(ForwardForm::Thread)
    );
    let placed = intents(&done)
        .into_iter()
        .find_map(|intent| match intent {
            Intent::ComposerDraft(draft) => Some(*draft),
            _ => None,
        })
        .expect("the composer is given the draft in its new form");
    assert_eq!(placed.subject, "Fwd: Harbor API draft v3 (6 messages)");
    assert_eq!(placed.body_markdown.as_deref(), Some("typed by the person"));
}

#[test]
fn three_switches_to_attachments_and_one_back_to_this_message() {
    let (mut focus, rows) = window_on_message_4();
    let _ = run(&mut focus, CommandId::Forward, &rows);
    let held = draft("Fwd: Harbor API draft v3");
    let (_, given) = with_draft(&mut focus, &rows, CommandId::ForwardWhat3, &held);
    assert!(matches!(
        requests(&given).as_slice(),
        [(
            _,
            Request::SetForwardForm {
                form: ForwardForm::Attachments,
                ..
            }
        )]
    ));
    let done = answer(
        &mut focus,
        &rows,
        &given,
        Reply::Forwarded {
            form: ForwardForm::Attachments,
            answer: Ok(Forwarded::Draft(Box::new(held.clone()))),
        },
    );
    assert_eq!(
        chooser_of(&done).map(|c| c.selected),
        Some(ForwardForm::Attachments)
    );
    let (_, back) = with_draft(&mut focus, &rows, CommandId::ForwardWhat1, &held);
    assert!(matches!(
        requests(&back).as_slice(),
        [(
            _,
            Request::SetForwardForm {
                form: ForwardForm::This,
                ..
            }
        )]
    ));
}

#[test]
fn choosing_the_card_already_chosen_asks_for_nothing() {
    let (mut focus, rows) = window_on_message_4();
    let _ = run(&mut focus, CommandId::Forward, &rows);
    let effects = run(&mut focus, CommandId::ForwardWhat1, &rows);
    assert!(intents(&effects).is_empty() && requests(&effects).is_empty());
}

#[test]
fn a_click_on_a_card_is_the_same_as_its_key() {
    let (mut focus, rows) = window_on_message_4();
    let _ = run(&mut focus, CommandId::Forward, &rows);
    let effects = focus.handle_on(Input::ForwardFormClicked(ForwardForm::Thread), &rows);
    assert!(
        intents(&effects).contains(&Intent::ComposerDraftWanted(DraftEdit::ForwardForm(
            ForwardForm::Thread
        )))
    );
}

#[test]
fn a_thread_with_messages_not_here_says_how_many_and_stays_as_it_was() {
    let (mut focus, rows) = window_on_message_4();
    let _ = run(&mut focus, CommandId::Forward, &rows);
    let held = draft("Fwd: Harbor API draft v3");
    let (_, given) = with_draft(&mut focus, &rows, CommandId::ForwardWhat2, &held);
    let refused = answer(
        &mut focus,
        &rows,
        &given,
        Reply::Forwarded {
            form: ForwardForm::Thread,
            answer: Ok(Forwarded::NotLocal(vec![
                MessageId::new(50000),
                MessageId::new(50001),
            ])),
        },
    );
    let notice = intents(&refused)
        .into_iter()
        .find_map(|intent| match intent {
            Intent::Toast { text, kind } => Some((text, kind)),
            _ => None,
        })
        .expect("the refusal is said");
    assert!(notice.0.starts_with("2 messages"), "{}", notice.0);
    assert_eq!(notice.1, ToastKind::Notice);
    assert!(
        !intents(&refused)
            .iter()
            .any(|i| matches!(i, Intent::ComposerDraft(_) | Intent::ForwardChooser(_))),
        "the draft and the chooser stay as they were"
    );
    assert!(requests(&refused).is_empty(), "nothing is fetched");
    // The form is still This: choosing it again is nothing.
    let again = run(&mut focus, CommandId::ForwardWhat1, &rows);
    assert!(intents(&again).is_empty() && requests(&again).is_empty());
}

// ---- Replying to an older message ----

#[test]
fn replying_to_message_4_of_6_says_it_is_not_the_latest() {
    let (mut focus, rows) = window_on_message_4();
    let effects = run(&mut focus, CommandId::Reply, &rows);
    assert_eq!(
        composer_of(&effects),
        Some((ComposerKind::Reply, Some(member_id(ThreadId::new(500), 3))))
    );
    let bar = older_reply_of(&effects)
        .flatten()
        .expect("message 4 of 6 is not the latest");
    assert_eq!(
        bar.title,
        "You\u{2019}re replying to message 4 of 6, not the latest"
    );
    assert!(
        bar.detail.starts_with("Two newer messages: Ana at "),
        "{}",
        bar.detail
    );
    assert!(bar.detail.contains(" and Ben at "), "{}", bar.detail);
    assert!(
        bar.detail
            .ends_with("Your reply quotes Ben\u{2019}s Monday message and threads under it."),
        "{}",
        bar.detail
    );
    let keymap = focus.keymap();
    assert_eq!(bar.actions[0].label, "Show them");
    assert_eq!(
        bar.actions[0].key,
        postio_ui::hints::key(keymap, CommandId::ShowNewer)
    );
    assert_eq!(bar.actions[1].label, "Reply to latest");
    assert_eq!(
        bar.actions[1].key,
        postio_ui::hints::key(keymap, CommandId::ReplyToLatest)
    );
}

#[test]
fn replying_to_the_latest_shows_no_bar() {
    let (mut focus, rows) = window();
    let _ = run(&mut focus, CommandId::NextInConversation, &rows);
    let effects = run(&mut focus, CommandId::Reply, &rows);
    assert!(composer_of(&effects).is_some());
    assert_eq!(older_reply_of(&effects), None);
}

#[test]
fn a_reply_to_all_to_an_older_message_shows_the_bar_too() {
    let (mut focus, rows) = window_on_message_4();
    let effects = run(&mut focus, CommandId::ReplyAll, &rows);
    assert!(older_reply_of(&effects).flatten().is_some());
}

#[test]
fn alt_e_in_the_composer_switches_to_the_latest_and_keeps_what_was_typed() {
    let (mut focus, rows) = window_on_message_4();
    let _ = run(&mut focus, CommandId::Reply, &rows);
    let mut held = draft("Re: Harbor API draft v3");
    held.forwarded_from = None;
    held.in_reply_to = Some(member_id(ThreadId::new(500), 3));
    let (_, given) = with_draft(&mut focus, &rows, CommandId::ReplyToLatest, &held);
    assert_eq!(
        requests(&given)
            .into_iter()
            .map(|(_, r)| r)
            .collect::<Vec<_>>(),
        vec![Request::ReplyToLatest {
            draft: Box::new(held.clone())
        }],
        "the typed text goes with the draft"
    );
    let mut latest = held.clone();
    latest.in_reply_to = Some(member_id(ThreadId::new(500), 5));
    let done = answer(
        &mut focus,
        &rows,
        &given,
        Reply::RepliedToLatest(Ok(Box::new(latest.clone()))),
    );
    let placed = intents(&done)
        .into_iter()
        .find_map(|intent| match intent {
            Intent::ComposerDraft(draft) => Some(*draft),
            _ => None,
        })
        .expect("the composer is given the reply to message 6");
    assert_eq!(placed.in_reply_to, Some(member_id(ThreadId::new(500), 5)));
    assert_eq!(placed.body_markdown.as_deref(), Some("typed by the person"));
    assert_eq!(older_reply_of(&done), Some(None), "the bar goes");
    // Nothing older to leave now.
    let again = run(&mut focus, CommandId::ReplyToLatest, &rows);
    assert!(intents(&again).is_empty());
}

#[test]
fn alt_e_from_the_window_replies_to_the_latest_message_directly() {
    let (mut focus, rows) = window_on_message_4();
    let effects = run(&mut focus, CommandId::ReplyToLatest, &rows);
    assert_eq!(
        composer_of(&effects),
        Some((ComposerKind::Reply, Some(member_id(ThreadId::new(500), 5))))
    );
    assert_eq!(older_reply_of(&effects), None, "it answers the latest");
}

#[test]
fn alt_e_from_a_list_row_replies_to_its_latest_message() {
    let (mut focus, rows) = inbox(6);
    let effects = run(&mut focus, CommandId::ReplyToLatest, &rows);
    assert_eq!(
        composer_of(&effects),
        Some((ComposerKind::Reply, Some(MessageId::new(100))))
    );
}

#[test]
fn show_them_toggles_the_newer_messages_read_only() {
    let (mut focus, rows) = window_on_message_4();
    let _ = run(&mut focus, CommandId::Reply, &rows);
    let thread = ThreadId::new(500);
    let shown_now = run(&mut focus, CommandId::ShowNewer, &rows);
    assert!(
        intents(&shown_now).contains(&Intent::NewerMessages(Some(vec![
            member_id(thread, 4),
            member_id(thread, 5)
        ])))
    );
    assert_eq!(
        older_reply_of(&shown_now)
            .flatten()
            .map(|b| b.actions[0].label.clone()),
        Some("Hide them".to_owned())
    );
    let hidden = run(&mut focus, CommandId::ShowNewer, &rows);
    assert!(intents(&hidden).contains(&Intent::NewerMessages(None)));
    assert_eq!(
        older_reply_of(&hidden)
            .flatten()
            .map(|b| b.actions[0].label.clone()),
        Some("Show them".to_owned())
    );
}

#[test]
fn show_them_does_nothing_in_a_composer_with_no_newer_messages() {
    let (mut focus, rows) = inbox(6);
    let _ = run(&mut focus, CommandId::Compose, &rows);
    let effects = run(&mut focus, CommandId::ShowNewer, &rows);
    assert!(intents(&effects).is_empty());
}

#[test]
fn expand_quoted_toggles_the_collapsed_rows() {
    let (mut focus, rows) = window_on_message_4();
    let _ = run(&mut focus, CommandId::Forward, &rows);
    let open = run(&mut focus, CommandId::ExpandQuoted, &rows);
    assert!(intents(&open).contains(&Intent::QuotedExpanded(true)));
    let shut = run(&mut focus, CommandId::ExpandQuoted, &rows);
    assert!(intents(&shut).contains(&Intent::QuotedExpanded(false)));
}

#[test]
fn a_new_composition_starts_collapsed_and_without_the_old_bar() {
    let (mut focus, rows) = window_on_message_4();
    let _ = run(&mut focus, CommandId::Reply, &rows);
    let _ = run(&mut focus, CommandId::ExpandQuoted, &rows);
    let _ = run(&mut focus, CommandId::Back, &rows);
    let _ = run(&mut focus, CommandId::Compose, &rows);
    let effects = run(&mut focus, CommandId::ExpandQuoted, &rows);
    assert!(
        intents(&effects).contains(&Intent::QuotedExpanded(true)),
        "the first press of a new composition expands: {:?}",
        intents(&effects)
    );
}

#[test]
fn the_menus_forward_the_whole_thread_opens_the_chooser_on_whole_thread() {
    let (mut focus, rows) = window_on_message_4();
    let _ = run(&mut focus, CommandId::ThreadMenu, &rows);
    let effects = focus.handle_on(Input::ThreadMenuPick(CommandId::ForwardWhat2), &rows);
    assert_eq!(
        composer_of(&effects).map(|(kind, _)| kind),
        Some(ComposerKind::Forward {
            form: ForwardForm::Thread
        })
    );
    assert_eq!(
        chooser_of(&effects).map(|c| c.selected),
        Some(ForwardForm::Thread)
    );
}

#[test]
fn a_whole_thread_open_asks_for_the_thread_forward_and_a_this_open_does_not() {
    let (mut focus, rows) = window_on_message_4();
    let plain = run(&mut focus, CommandId::Forward, &rows);
    assert!(requests(&plain).is_empty(), "f builds nothing itself");
    let (mut focus, rows) = window_on_message_4();
    let _ = run(&mut focus, CommandId::ThreadMenu, &rows);
    let opened = focus.handle_on(Input::ThreadMenuPick(CommandId::ForwardWhat2), &rows);
    assert_eq!(
        requests(&opened)
            .into_iter()
            .map(|(_, r)| r)
            .collect::<Vec<_>>(),
        vec![Request::ForwardThread {
            thread: ThreadId::new(500),
            form: ForwardForm::Thread
        }]
    );
    let whole = draft("Fwd: Harbor API draft v3 (6 messages)");
    let done = answer(
        &mut focus,
        &rows,
        &opened,
        Reply::Forwarded {
            form: ForwardForm::Thread,
            answer: Ok(Forwarded::Draft(Box::new(whole.clone()))),
        },
    );
    assert!(intents(&done).contains(&Intent::ComposerDraft(Box::new(whole))));
}

#[test]
fn a_whole_thread_open_refused_for_messages_not_here_goes_back_to_this_message() {
    let (mut focus, rows) = window_on_message_4();
    let _ = run(&mut focus, CommandId::ThreadMenu, &rows);
    let opened = focus.handle_on(Input::ThreadMenuPick(CommandId::ForwardWhat2), &rows);
    let refused = answer(
        &mut focus,
        &rows,
        &opened,
        Reply::Forwarded {
            form: ForwardForm::Thread,
            answer: Ok(Forwarded::NotLocal(vec![MessageId::new(50000)])),
        },
    );
    assert_eq!(
        chooser_of(&refused).map(|c| c.selected),
        Some(ForwardForm::This)
    );
    assert!(
        intents(&refused)
            .iter()
            .any(|i| matches!(i, Intent::Toast { .. }))
    );
}
