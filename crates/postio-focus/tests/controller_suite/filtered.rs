//! The Filtered view, from the controller's side (specs/009-focus-macos
//! T108; spec 007 US9, screen 21).
//!
//! What GTK's `filtered.rs` and the window's Filtered handlers did, asserted
//! here so the Mac's view behaves the same: `g f` shows it with its seven
//! reason tabs and their counts, a number key narrows it to one reason, `R`
//! restores the focused message and stops filtering its sender, and nothing
//! in it is deleted but by Delete (C4: never automatically).

use crate::search_support::{asks, shown};
use chrono::{TimeZone, Utc};
use postio_client::protocol::FilteredRow;
use postio_config::paths::Platform;
use postio_core::{Command, CommandId, MessageTarget};
use postio_focus::{
    Effect, FilteredBar, FilteredView, FocusController, Host, Input, Intent, Policy, Reply,
    Request, RowFacts, Rows, SurfaceKind,
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
fn r_restores_the_focused_message() {
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
}

/// What a key sends while Filtered is up, as commands.
fn sent(focus: &mut FocusController, command: CommandId) -> Vec<(Command, Vec<MessageTarget>)> {
    asks(&run(focus, command))
        .into_iter()
        .filter_map(|(_, request)| match request {
            Request::Send { command, aims, .. } => Some((command, aims)),
            _ => None,
        })
        .collect()
}

#[test]
fn delete_flag_and_read_act_on_the_focused_filtered_message_alone() {
    // C4 is "never deleted automatically": Delete is the person's own.
    for (key, made) in [
        (CommandId::Delete, Command::default_for(CommandId::Delete)),
        (CommandId::Flag, Command::default_for(CommandId::Flag)),
        (
            CommandId::ToggleRead,
            Command::default_for(CommandId::ToggleRead),
        ),
    ] {
        let mut focus = mac();
        let _ = opened(
            &mut focus,
            vec![filed(7, "notification", 5), filed(8, "spam", 9)],
        );
        let _ = run(&mut focus, CommandId::NextMessage);
        assert_eq!(
            sent(&mut focus, key),
            vec![(made, vec![MessageTarget::Messages(vec![MessageId::new(8)])])],
            "{key:?}: the filtered message, not its conversation or the list's row"
        );
    }
}

#[test]
fn only_delete_deletes_in_filtered_and_archive_does_nothing() {
    let others = postio_ui::filtered::TAB_COMMANDS.into_iter().chain([
        CommandId::RestoreFiltered,
        CommandId::NextMessage,
        CommandId::PrevMessage,
        CommandId::OpenMessage,
    ]);
    for command in others {
        let mut focus = mac();
        let _ = opened(&mut focus, vec![filed(7, "notification", 5)]);
        for (_, request) in asks(&run(&mut focus, command)) {
            if let Request::Post(made) | Request::Send { command: made, .. } = &request {
                assert!(
                    !matches!(made, Command::Delete { .. }),
                    "{command:?} deleted from Filtered"
                );
            }
        }
    }
    // Filtered mail is archived already.
    let mut focus = mac();
    let _ = opened(&mut focus, vec![filed(7, "notification", 5)]);
    assert_eq!(sent(&mut focus, CommandId::Archive), vec![]);
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

#[test]
fn the_delete_flag_and_read_keys_reach_filtered() {
    use postio_ui::keymap::{Chord, Outcome};
    let mut focus = mac();
    let _ = opened(&mut focus, vec![filed(7, "notification", 5)]);
    for (key, command) in [
        ("#", CommandId::Delete),
        ("s", CommandId::Flag),
        ("U", CommandId::ToggleRead),
    ] {
        let chord: Chord = key.parse().expect("a chord");
        let outcome = focus.press(
            &chord,
            focus.key_context(),
            false,
            std::time::Instant::now(),
        );
        assert_eq!(
            outcome,
            Outcome::Command(command.as_str().to_owned()),
            "{key} in Filtered"
        );
    }
}

// -- Selecting in Filtered, as the list does (the main list's rules) --------

/// Three filtered messages, 7 to 9, newest first.
fn three() -> Vec<FilteredRow> {
    vec![
        filed(7, "notification", 5),
        filed(8, "spam", 9),
        filed(9, "promotion", 20),
    ]
}

/// The last selection Filtered drew in `effects`: the rows boxed, and its
/// bar's count.
fn selection(effects: &[Effect]) -> Option<(Vec<i64>, Option<String>)> {
    shown(effects)
        .into_iter()
        .rev()
        .find_map(|intent| match intent {
            Intent::FilteredSelection { selected, bar } => Some((
                selected.iter().map(|id| id.get()).collect(),
                bar.map(|bar: FilteredBar| bar.summary),
            )),
            _ => None,
        })
}

fn pick(focus: &mut FocusController, position: u32, range: bool) -> Vec<Effect> {
    focus.handle_on(Input::FilteredPick { position, range }, &List)
}

#[test]
fn x_takes_the_focused_row_in_and_out_of_the_selection() {
    let mut focus = mac();
    let _ = opened(&mut focus, three());
    let effects = run(&mut focus, CommandId::ToggleSelection);
    assert_eq!(
        selection(&effects),
        Some((vec![7], Some("1 selected".to_owned()))),
        "{effects:?}"
    );
    let _ = run(&mut focus, CommandId::NextMessage);
    let effects = run(&mut focus, CommandId::ToggleSelection);
    assert_eq!(
        selection(&effects),
        Some((vec![7, 8], Some("2 selected".to_owned())))
    );
    // Out again: the box clears, the bar stays for the one left.
    let effects = run(&mut focus, CommandId::ToggleSelection);
    assert_eq!(
        selection(&effects),
        Some((vec![7], Some("1 selected".to_owned())))
    );
    // Moving the keyboard selects nothing.
    let effects = run(&mut focus, CommandId::NextMessage);
    assert_eq!(selection(&effects), None);
}

#[test]
fn j_and_k_capital_extend_the_selection_as_the_keyboard_walks() {
    let mut focus = mac();
    let _ = opened(&mut focus, three());
    let effects = run(&mut focus, CommandId::ExtendSelectionDown);
    assert_eq!(
        selection(&effects),
        Some((vec![7, 8], Some("2 selected".to_owned())))
    );
    assert_eq!(shown(&effects)[0], Intent::FilteredFocus(Some(1)));
    let effects = run(&mut focus, CommandId::ExtendSelectionDown);
    assert_eq!(selection(&effects).unwrap().0, vec![7, 8, 9]);
    // Up from the last row takes the row it leaves in too, and walks back.
    let effects = run(&mut focus, CommandId::ExtendSelectionUp);
    assert_eq!(shown(&effects)[0], Intent::FilteredFocus(Some(1)));
}

#[test]
fn select_all_takes_every_row_read() {
    let mut focus = mac();
    let _ = opened(&mut focus, three());
    let effects = run(&mut focus, CommandId::SelectAll);
    assert_eq!(
        selection(&effects),
        Some((vec![7, 8, 9], Some("3 selected".to_owned())))
    );
}

#[test]
fn a_click_with_the_toggle_modifier_toggles_and_shift_takes_a_range() {
    let mut focus = mac();
    let _ = opened(&mut focus, three());
    let effects = pick(&mut focus, 1, false);
    assert_eq!(selection(&effects).unwrap().0, vec![8], "{effects:?}");
    assert!(
        shown(&effects).contains(&Intent::FilteredFocus(Some(1))),
        "the keyboard follows the click"
    );
    // Shift from the anchor (row 1) down to row 2.
    let effects = pick(&mut focus, 2, true);
    assert_eq!(selection(&effects).unwrap().0, vec![8, 9]);
    let effects = pick(&mut focus, 1, false);
    assert_eq!(selection(&effects).unwrap().0, vec![9]);
    // A plain click selects nothing.
    let effects = focus.handle_on(Input::FilteredPoint(0), &List);
    assert_eq!(selection(&effects), None);
}

#[test]
fn the_view_boxes_the_selected_rows_and_carries_the_bar() {
    let mut focus = mac();
    let _ = opened(&mut focus, three());
    let _ = run(&mut focus, CommandId::ToggleSelection);
    // A re-read, as mail moving elsewhere causes, keeps the selection.
    let effects = opened_again(&mut focus, three());
    let view = view(&effects).expect("redrawn");
    let boxed: Vec<bool> = view.rows.iter().map(|row| row.selected).collect();
    assert_eq!(boxed, [true, false, false]);
    let bar = view.bar.expect("the bar is up with a selection");
    assert_eq!(bar.summary, "1 selected");
    let verbs: Vec<(CommandId, bool)> = bar
        .actions
        .iter()
        .map(|action| (action.command, action.primary))
        .collect();
    assert_eq!(
        verbs,
        [
            (CommandId::RestoreFiltered, true),
            (CommandId::ToggleRead, false)
        ]
    );
    assert_eq!(
        bar.actions[0].key.as_deref(),
        Some("R"),
        "keys come from the keymap"
    );
    let hints: Vec<String> = bar.hints.iter().map(|hint| hint.label.clone()).collect();
    assert_eq!(hints, ["toggle", "extend", "clear"]);
}

/// The view's own re-read: `Event::MessageListChanged`, answered.
fn opened_again(focus: &mut FocusController, rows: Vec<FilteredRow>) -> Vec<Effect> {
    let effects = focus.handle_on(
        Input::Event(postio_core::Event::MessageListChanged {
            account: postio_model::AccountId::new(1),
            mailbox: postio_model::MailboxId::new(1),
        }),
        &List,
    );
    let mut out = Vec::new();
    for (ticket, request) in asks(&effects) {
        let reply = match request {
            Request::FilteredTabs => Reply::FilteredTabs(Ok(Vec::new())),
            Request::Filtered { stamp, offset, .. } => Reply::Filtered {
                stamp,
                offset,
                answer: Ok(rows.clone()),
            },
            other => panic!("a re-read asked for {other:?}"),
        };
        out.extend(focus.handle_on(Input::Reply(ticket, reply), &List));
    }
    out
}

#[test]
fn r_restores_every_selected_row_in_one_step_and_clears_the_selection() {
    let mut focus = mac();
    let _ = opened(&mut focus, three());
    let _ = run(&mut focus, CommandId::ToggleSelection);
    let _ = run(&mut focus, CommandId::NextMessage);
    let _ = run(&mut focus, CommandId::NextMessage);
    let _ = run(&mut focus, CommandId::ToggleSelection);
    let effects = run(&mut focus, CommandId::RestoreFiltered);
    assert_eq!(
        asks(&effects)
            .into_iter()
            .map(|(_, asked)| asked)
            .collect::<Vec<_>>(),
        vec![Request::Post(Command::RestoreFiltered {
            target: MessageTarget::Messages(vec![MessageId::new(7), MessageId::new(9)]),
            restored: true,
        })],
        "one command, so one Undo takes both back"
    );
    assert_eq!(
        selection(&effects),
        Some((Vec::new(), None)),
        "what the selection named has been acted on"
    );
}

#[test]
fn delete_flag_and_read_act_on_every_selected_row() {
    for (key, made) in [
        (CommandId::Delete, Command::default_for(CommandId::Delete)),
        (CommandId::Flag, Command::default_for(CommandId::Flag)),
        (
            CommandId::ToggleRead,
            Command::default_for(CommandId::ToggleRead),
        ),
    ] {
        let mut focus = mac();
        let _ = opened(&mut focus, three());
        let _ = run(&mut focus, CommandId::ExtendSelectionDown);
        let effects = run(&mut focus, key);
        let sent: Vec<_> = asks(&effects)
            .into_iter()
            .filter_map(|(_, request)| match request {
                Request::Send { command, aims, .. } => Some((command, aims)),
                _ => None,
            })
            .collect();
        assert_eq!(
            sent,
            vec![(
                made,
                vec![MessageTarget::Messages(vec![
                    MessageId::new(7),
                    MessageId::new(8)
                ])]
            )],
            "{key:?}: the two selected messages, not their conversations"
        );
        assert_eq!(selection(&effects).unwrap().0, Vec::<i64>::new());
    }
}

#[test]
fn escape_clears_a_selection_before_it_leaves_filtered() {
    let mut focus = mac();
    let _ = opened(&mut focus, three());
    let _ = run(&mut focus, CommandId::ToggleSelection);
    let effects = run(&mut focus, CommandId::Back);
    assert_eq!(selection(&effects), Some((Vec::new(), None)));
    assert!(
        !shown(&effects).contains(&Intent::CloseSurface(SurfaceKind::Filtered)),
        "the first Escape only clears: {effects:?}"
    );
    assert_eq!(focus.key_context(), KeyContext::Filtered);
    let effects = run(&mut focus, CommandId::Back);
    assert!(shown(&effects).contains(&Intent::CloseSurface(SurfaceKind::Filtered)));
}

#[test]
fn a_tab_is_another_place_so_the_selection_goes() {
    let mut focus = mac();
    let _ = opened(&mut focus, three());
    let _ = run(&mut focus, CommandId::SelectAll);
    let effects = run(&mut focus, CommandId::FilteredTab4);
    let view = view(&effects).expect("the tabs are drawn again");
    assert!(view.bar.is_none(), "{view:?}");
    assert!(view.rows.iter().all(|row| !row.selected));
    // And what was selected is not acted on from the new tab.
    let _ = opened_again(&mut focus, three());
    let sent = sent(&mut focus, CommandId::Delete);
    assert_eq!(
        sent[0].1,
        vec![MessageTarget::Messages(vec![MessageId::new(7)])],
        "the focused row alone"
    );
}

#[test]
fn leaving_filtered_leaves_the_lists_own_selection_alone() {
    let mut focus = mac();
    let _ = run(&mut focus, CommandId::ToggleSelection);
    let before = focus.selection();
    let _ = opened(&mut focus, three());
    let _ = run(&mut focus, CommandId::SelectAll);
    assert_eq!(focus.selection(), before, "Filtered's is its own");
    let _ = run(&mut focus, CommandId::Back);
    let _ = run(&mut focus, CommandId::Back);
    assert_eq!(focus.selection(), before);
}

#[test]
fn the_selection_keys_reach_filtered() {
    use postio_ui::keymap::{Chord, Outcome};
    let mut focus = mac();
    let _ = opened(&mut focus, three());
    for (key, command) in [
        ("x", CommandId::ToggleSelection),
        ("J", CommandId::ExtendSelectionDown),
        ("K", CommandId::ExtendSelectionUp),
        ("X", CommandId::SelectAll),
    ] {
        let chord: Chord = key.parse().expect("a chord");
        let outcome = focus.press(
            &chord,
            focus.key_context(),
            false,
            std::time::Instant::now(),
        );
        assert_eq!(
            outcome,
            Outcome::Command(command.as_str().to_owned()),
            "{key} in Filtered"
        );
        assert!(focus.answers(command), "{command:?} is the controller's");
    }
}
