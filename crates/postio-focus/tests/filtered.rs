//! The Filtered view, from the controller's side (specs/009-focus-macos
//! T108; spec 007 US9, screen 21).
//!
//! What GTK's `filtered.rs` and the window's Filtered handlers did, asserted
//! here so the Mac's view behaves the same: `g f` shows it with its seven
//! reason tabs and their counts, a number key narrows it to one reason, `R`
//! restores the focused message and stops filtering its sender, and nothing
//! in it is ever deleted (C4) -- no key here sends a deletion.

use chrono::{TimeZone, Utc};
use postio_client::protocol::FilteredRow;
use postio_config::paths::Platform;
use postio_core::{Command, CommandId, MessageTarget};
use postio_focus::{
    Effect, FilteredView, FocusController, Host, Input, Intent, Policy, Reply, Request, RowFacts,
    Rows, SurfaceKind, Ticket,
};
use postio_model::listing::MessageSummary;
use postio_model::{EmailAddress, MessageId, ThreadId};
use postio_ui::keymap::KeyContext;

/// Three conversations behind the view.
struct List;

impl Rows for List {
    fn len(&self) -> u32 {
        3
    }
    fn facts(&self, position: u32) -> Option<RowFacts> {
        (position < 3).then(|| RowFacts {
            id: MessageId::new(100 + i64::from(position)),
            digest: false,
            threads: vec![ThreadId::new(500 + i64::from(position))],
            writes: false,
        })
    }
    fn position_of(&self, message: MessageId) -> Option<u32> {
        let at = message.get() - 100;
        (0..3).contains(&at).then_some(at as u32)
    }
}

fn mac() -> FocusController {
    FocusController::new(Policy::for_platform(Platform::Apple))
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

fn run(focus: &mut FocusController, command: CommandId) -> Vec<Effect> {
    focus.handle_on(Input::Command(command), &List)
}

fn view(effects: &[Effect]) -> Option<FilteredView> {
    shown(effects)
        .into_iter()
        .rev()
        .find_map(|intent| match intent {
            Intent::Filtered(view) => Some(*view),
            _ => None,
        })
}

/// A filtered message from Ledger, filed `minutes` ago.
fn filed(id: i64, reason: &str, minutes: i64) -> FilteredRow {
    let at =
        Utc.with_ymd_and_hms(2026, 10, 2, 9, 0, 0).unwrap() - chrono::Duration::minutes(minutes);
    FilteredRow {
        message: MessageSummary {
            id: MessageId::new(id),
            thread: Some(ThreadId::new(id + 1000)),
            from: Some(EmailAddress::new(Some("Ledger"), "news@ledger.example")),
            subject: Some(format!("Issue {id}")),
            preview: Some("This week".to_owned()),
            received_at: at,
            seen: true,
            flagged: false,
            answered: false,
            send_state: None,
            send_at: None,
            has_attachments: false,
            thread_count: 1,
            to: Vec::new(),
        },
        reason: reason.to_owned(),
        source: Some("Ledger".to_owned()),
        at,
    }
}

/// `g f`, answered: the tabs' counts, then the first page of All.
fn opened(focus: &mut FocusController, rows: Vec<FilteredRow>) -> Vec<Effect> {
    let asked = asks(&run(focus, CommandId::GoToFiltered));
    let mut effects = Vec::new();
    for (ticket, request) in asked {
        let reply = match request {
            Request::FilteredTabs => Reply::FilteredTabs(Ok(vec![
                ("notification".to_owned(), 88),
                ("spam".to_owned(), 12),
                ("promotion".to_owned(), 41),
            ])),
            Request::Filtered { stamp, offset, .. } => Reply::Filtered {
                stamp,
                offset,
                answer: Ok(rows.clone()),
            },
            other => panic!("g f asked for {other:?}"),
        };
        effects.extend(focus.handle_on(Input::Reply(ticket, reply), &List));
    }
    effects
}

#[test]
fn g_f_shows_filtered_with_seven_reason_tabs_and_their_counts() {
    let mut focus = mac();
    let effects = run(&mut focus, CommandId::GoToFiltered);
    assert_eq!(shown(&effects), vec![Intent::ShowFiltered]);
    let asked: Vec<Request> = asks(&effects).into_iter().map(|(_, asked)| asked).collect();
    assert!(asked.contains(&Request::FilteredTabs), "{asked:?}");
    assert!(
        asked.iter().any(|request| matches!(
            request,
            Request::Filtered {
                reason: None,
                offset: 0,
                ..
            }
        )),
        "the first page of All: {asked:?}"
    );
    assert_eq!(focus.key_context(), KeyContext::Filtered);

    let mut focus = mac();
    let effects = opened(&mut focus, vec![filed(7, "notification", 5)]);
    let view = view(&effects).expect("the view is drawn");
    let tabs: Vec<(String, u32)> = view
        .tabs
        .iter()
        .map(|tab| (tab.name.clone(), tab.count))
        .collect();
    assert_eq!(
        tabs,
        [
            ("All", 141),
            ("Spam", 12),
            ("Promotions", 41),
            ("Notifications", 88),
            ("Receipts", 0),
            ("Shipping", 0),
            ("Social", 0),
        ]
        .map(|(name, count)| (name.to_owned(), count))
    );
    let keys: Vec<Option<String>> = view.tabs.iter().map(|tab| tab.key.clone()).collect();
    assert_eq!(
        keys,
        (1..=7).map(|key| Some(key.to_string())).collect::<Vec<_>>(),
        "tabs `1`-`7`"
    );
    assert!(view.tabs[0].on, "All shows first");
    assert_eq!(view.note, postio_ui::filtered::NOTE, "C4, said on the view");
    assert_eq!(view.rows.len(), 1);
    assert_eq!(view.rows[0].message, MessageId::new(7));
    assert_eq!(view.rows[0].pill, "notification \u{b7} Ledger");
    assert_eq!(view.focused, Some(0), "the first row has the keyboard");
}

#[test]
fn a_number_key_narrows_filtered_to_its_reason() {
    let mut focus = mac();
    let _ = opened(&mut focus, vec![filed(7, "notification", 5)]);
    let effects = run(&mut focus, CommandId::FilteredTab4);
    assert!(
        asks(&effects).iter().any(|(_, request)| matches!(
            request,
            Request::Filtered { reason: Some(reason), offset: 0, .. } if reason == "notification"
        )),
        "the fourth tab is Notifications"
    );
    let view = view(&effects).expect("the tabs are drawn again");
    assert!(view.tabs[3].on && !view.tabs[0].on);
}

#[test]
fn r_restores_the_focused_message_and_nothing_is_ever_deleted() {
    let mut focus = mac();
    let _ = opened(
        &mut focus,
        vec![filed(7, "notification", 5), filed(8, "spam", 9)],
    );
    let effects = run(&mut focus, CommandId::NextMessage);
    assert_eq!(shown(&effects), vec![Intent::FilteredFocus(Some(1))]);
    let effects = run(&mut focus, CommandId::RestoreFiltered);
    assert_eq!(
        asks(&effects)
            .into_iter()
            .map(|(_, asked)| asked)
            .collect::<Vec<_>>(),
        vec![Request::Post(Command::RestoreFiltered {
            target: MessageTarget::Messages(vec![MessageId::new(8)]),
            restored: true,
        })]
    );
    // C4: whatever the keys of the view, none of them deletes.
    for command in postio_ui::filtered::TAB_COMMANDS.into_iter().chain([
        CommandId::RestoreFiltered,
        CommandId::Delete,
        CommandId::Archive,
        CommandId::NextMessage,
        CommandId::PrevMessage,
    ]) {
        let mut focus = mac();
        let _ = opened(&mut focus, vec![filed(7, "notification", 5)]);
        for (_, request) in asks(&run(&mut focus, command)) {
            if let Request::Post(command) | Request::Send { command, .. } = &request {
                assert!(
                    !matches!(command, Command::Delete { .. }),
                    "{command:?} deleted from Filtered"
                );
            }
        }
    }
}

#[test]
fn back_and_g_i_leave_filtered_for_the_list() {
    for leave in [CommandId::Back, CommandId::GoToInbox] {
        let mut focus = mac();
        let _ = opened(&mut focus, vec![filed(7, "notification", 5)]);
        let effects = run(&mut focus, leave);
        let shown = shown(&effects);
        assert!(
            shown.contains(&Intent::CloseSurface(SurfaceKind::Filtered)),
            "{leave:?}: {shown:?}"
        );
        assert!(shown.contains(&Intent::KeyboardHome), "{leave:?}");
        assert_eq!(focus.key_context(), KeyContext::List, "{leave:?}");
    }
}

#[test]
fn return_opens_the_focused_message_over_filtered_and_esc_comes_back() {
    let mut focus = mac();
    let _ = opened(
        &mut focus,
        vec![filed(7, "notification", 5), filed(8, "spam", 9)],
    );
    let effects = run(&mut focus, CommandId::OpenMessage);
    assert!(shown(&effects).contains(&Intent::OpenMessage {
        message: MessageId::new(7),
        index: 0,
        total: 2,
        host: Host::Own,
    }));
    // `j` in it walks Filtered, not the list behind it.
    let effects = run(&mut focus, CommandId::NextMessage);
    assert!(shown(&effects).contains(&Intent::OpenMessage {
        message: MessageId::new(8),
        index: 1,
        total: 2,
        host: Host::Own,
    }));
    let effects = run(&mut focus, CommandId::Back);
    assert_eq!(
        shown(&effects),
        vec![Intent::CloseSurface(SurfaceKind::Message)]
    );
    assert_eq!(focus.key_context(), KeyContext::Filtered);
}

#[test]
fn the_sweep_says_how_many_first_and_moves_only_once_confirmed() {
    let mut focus = mac();
    let _ = opened(&mut focus, Vec::new());
    let effects = run(&mut focus, CommandId::SweepInbox);
    let [(ticket, Request::SweepPreview)] = asks(&effects)[..] else {
        panic!("F counts first: {effects:?}");
    };
    let effects = focus.handle_on(Input::Reply(ticket, Reply::SweepPreview(Ok(2))), &List);
    let confirm = shown(&effects)
        .into_iter()
        .find_map(|intent| match intent {
            Intent::Confirm(confirm) => Some(confirm),
            _ => None,
        })
        .expect("the sweep asks");
    assert_eq!(confirm.heading, postio_ui::filtered::SWEEP_HEADING);
    assert_eq!(confirm.confirm, postio_ui::filtered::sweep_action(2));
    assert!(asks(&effects).is_empty(), "nothing moves before the answer");
    let effects = focus.handle_on(Input::Confirmed(confirm.token), &List);
    assert_eq!(
        asks(&effects)
            .into_iter()
            .map(|(_, asked)| asked)
            .collect::<Vec<_>>(),
        vec![Request::Post(Command::SweepInbox)]
    );
    // A sweep that would move nothing says so instead of asking.
    let effects = run(&mut focus, CommandId::SweepInbox);
    let [(ticket, _)] = asks(&effects)[..] else {
        panic!("counted again");
    };
    let effects = focus.handle_on(Input::Reply(ticket, Reply::SweepPreview(Ok(0))), &List);
    assert!(shown(&effects).iter().any(|intent| matches!(
        intent,
        Intent::Toast { text, .. } if text == postio_ui::filtered::SWEEP_NOTHING
    )));
}

#[test]
fn a_page_for_a_tab_since_left_is_not_drawn() {
    let mut focus = mac();
    let asked = asks(&run(&mut focus, CommandId::GoToFiltered));
    let (ticket, stale) = asked
        .iter()
        .find_map(|(ticket, request)| match request {
            Request::Filtered { stamp, .. } => Some((*ticket, *stamp)),
            _ => None,
        })
        .expect("a page is asked");
    let _ = run(&mut focus, CommandId::FilteredTab2);
    let effects = focus.handle_on(
        Input::Reply(
            ticket,
            Reply::Filtered {
                stamp: stale,
                offset: 0,
                answer: Ok(vec![filed(7, "notification", 5)]),
            },
        ),
        &List,
    );
    assert!(view(&effects).is_none(), "{effects:?}");
}
