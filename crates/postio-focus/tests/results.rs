//! The results view, from the controller's side (spec 010 step 3, US2,
//! FR-020 to FR-024, D17, D18, design §3 and screens 06 and 07).
//!
//! ⌘↩ turns the main window into a search's results: a mode the
//! controller holds, which asks the engine for its pages and their
//! passages, groups them into Top hits and months, edits its query term by
//! term, and gives way to the inbox on Esc's last rung. The Mac draws what
//! it is told; GTK never enters it (`results_view` off).

mod search_support;

use postio_config::paths::Platform;
use postio_core::CommandId;
use postio_focus::{
    Effect, FilterButton, FocusController, Input, Intent, Policy, Request, SurfaceKind, TermEdit,
};
use postio_model::MessageId;
use postio_search::results::ConversationOrder;
use postio_ui::keymap::KeyContext;
use postio_ui::search_view::FilterKind;
use search_support::*;

fn text(runs: &[postio_focus::Run]) -> String {
    runs.iter().map(|run| run.text.as_str()).collect()
}

fn lit(runs: &[postio_focus::Run]) -> Vec<String> {
    runs.iter()
        .filter(|run| run.highlighted)
        .map(|run| run.text.clone())
        .collect()
}

fn button(view: &postio_focus::QueryView, kind: FilterKind) -> FilterButton {
    view.buttons
        .iter()
        .find(|button| button.kind == kind)
        .cloned()
        .unwrap_or_else(|| panic!("no {kind:?} button in {view:?}"))
}

fn results_reads(effects: &[Effect]) -> Vec<(ConversationOrder, u32, u32)> {
    asked(effects)
        .into_iter()
        .filter_map(|request| match request {
            Request::ResultsPage {
                order,
                offset,
                limit,
                ..
            } => Some((order, offset, limit)),
            _ => None,
        })
        .collect()
}

#[test]
fn show_all_enters_the_results_with_the_query() {
    let rows = List::of(3);
    let mut focus = mac();
    let effects = show_all(&mut focus, "atlas budget", &rows);

    let intents = shown(&effects);
    assert!(
        intents.contains(&Intent::CloseSurface(SurfaceKind::Bar)),
        "the dropdown goes: {intents:?}"
    );
    assert!(
        results_view(&effects).is_some(),
        "the main window turns into results"
    );
    let query = query_view(&effects).expect("the field's chips and words");
    assert_eq!(query.words, "atlas budget");
    assert!(query.chips.is_empty());
    assert_eq!(query.hint, "/ to edit");
    assert!(focus.in_results());
    assert_eq!(focus.key_context(), KeyContext::Results);
    // Best match, the default with words: its Top hits, and the first page
    // of the month groups, asked together.
    assert_eq!(
        results_reads(&effects),
        [
            (ConversationOrder::BestMatch, 0, 3),
            (ConversationOrder::Newest, 0, 50)
        ]
    );
}

#[test]
fn entering_the_results_remembers_the_query() {
    let rows = List::of(3);
    let mut focus = mac();
    let effects = search(&mut focus, "atlas budget", &rows);
    let remembered: Vec<Request> = asked(&effects)
        .into_iter()
        .filter(|request| matches!(request, Request::RememberSearch { .. }))
        .collect();
    assert_eq!(
        remembered,
        [Request::RememberSearch {
            query: "atlas budget".to_owned(),
            hits: CONVERSATIONS as u64,
        }],
        "once, with what it found"
    );
}

#[test]
fn best_match_gives_top_hits_then_month_groups() {
    let rows = List::of(3);
    let mut focus = mac();
    let effects = search(&mut focus, "atlas budget", &rows);
    let view = results_view(&effects).expect("the results");

    assert_eq!(view.order, ConversationOrder::BestMatch);
    assert_eq!(view.count_line, "120 conversations");
    assert_eq!(view.sub_line, "12 files · 6 people · last 12 months");
    assert_eq!(view.footer, "120 conversations · local index · 41 ms");
    assert_eq!(view.months.len(), 12);
    assert_eq!(view.months[11].label, "Sep");
    assert_eq!(view.tabs[0].label, "Conversations");
    assert_eq!(view.tabs[0].count, "120");
    assert!(view.tabs[0].selected);
    assert_eq!(view.tabs[1].count, "12");
    assert_eq!(view.tabs[2].count, "6");

    let top = &view.groups[0];
    assert!(top.top_hits);
    assert_eq!(
        (top.title.as_str(), top.first, top.rows),
        ("Top hits", 0, 3)
    );
    let september = &view.groups[1];
    assert_eq!(september.title, "September 2026");
    assert_eq!(september.first, 3);
    // Every conversation sits in the month of its newest match (D4): the
    // 26th back to the 2nd, every other day.
    assert_eq!(september.rows, 13);
    assert_eq!(september.count, "13");
    assert_eq!(september.note.as_deref(), Some("newest first"));
    assert_eq!(september.accessible, "September 2026 · 13");
    assert_eq!(top.accessible, "Top hits · 3");
    assert_eq!(view.groups[2].title, "August 2026");
    assert_eq!(view.groups[2].first, 16);
    assert_eq!(view.rows, 3 + CONVERSATIONS as u64);
    assert_eq!(focus.result_count(), view.rows);

    // The Top hits say why they ranked; the flagged one says flagged (D19).
    let first = focus.result_row(0).expect("a top hit");
    assert!(first.top_hit);
    assert_eq!(first.message, MessageId::new(1007));
    assert_eq!(first.reason.as_deref(), Some("you replied · 3 matches"));
    let second = focus.result_row(1).expect("a top hit");
    assert_eq!(second.reason.as_deref(), Some("you flagged · 2 matches"));
    assert_eq!(
        focus.result_row(2).and_then(|row| row.reason).as_deref(),
        Some("frequent sender · file name")
    );
    // Then the newest, in September's group.
    let newest = focus.result_row(3).expect("the newest");
    assert_eq!(newest.message, MessageId::new(1000));
    assert!(!newest.top_hit);
    assert_eq!(newest.reason, None);
    assert_eq!(newest.group, 1);
    assert_eq!(newest.sender, "Ada Moreno");
    assert!(newest.unread);
    assert_eq!(text(&newest.subject), "Atlas budget, part 0");
    assert_eq!(lit(&newest.subject), ["Atlas", "budget"]);
    assert_eq!(newest.labels, ["Atlas"]);
    assert!(newest.attachments);
    assert_eq!(newest.count_badge.as_deref(), Some("3"));
    assert_eq!(newest.folder, "in:Inbox");
    assert_eq!(newest.date, "26 Sep");
    assert_eq!(view.cursor, Some(0), "the focus ring on the first row");
}

#[test]
fn newest_has_only_month_groups() {
    let rows = List::of(3);
    let mut focus = mac();
    let _ = search(&mut focus, "atlas budget", &rows);
    let effects = run(&mut focus, CommandId::ToggleResultOrder, &rows);
    assert_eq!(
        results_reads(&effects),
        [(ConversationOrder::Newest, 0, 50)],
        "no Top hits to ask for"
    );
    let effects = settle(&mut focus, effects, &rows);
    let view = results_view(&effects).expect("the results again");
    assert_eq!(view.order, ConversationOrder::Newest);
    assert!(view.groups.iter().all(|group| !group.top_hits));
    assert_eq!(view.groups[0].title, "September 2026");
    assert_eq!(view.groups[0].first, 0);
    assert_eq!(view.rows, CONVERSATIONS as u64);
    assert_eq!(
        focus.result_row(0).map(|row| row.message),
        Some(MessageId::new(1000))
    );

    // And the order chosen from the Sort menu.
    let effects = focus.handle_on(Input::ResultsOrder(ConversationOrder::BestMatch), &rows);
    let effects = settle(&mut focus, effects, &rows);
    assert!(results_view(&effects).expect("again").groups[0].top_hits);
}

#[test]
fn rows_page_through_a_window() {
    let rows = List::of(3);
    let mut focus = mac();
    let _ = search(&mut focus, "atlas budget", &rows);
    // The first page is fifty of the month groups' rows, after Top hits.
    assert!(focus.result_row(52).is_some());
    assert!(focus.result_row(53).is_none(), "not read yet");

    let effects = focus.results_wanted(60);
    assert_eq!(
        results_reads(&effects),
        [(ConversationOrder::Newest, 50, 50)]
    );
    assert!(
        results_reads(&focus.results_wanted(61)).is_empty(),
        "a page on its way is not asked for twice"
    );
    let effects = settle(&mut focus, effects, &rows);
    assert!(
        shown(&effects).contains(&Intent::ResultsPage {
            first: 53,
            count: 50
        }),
        "the page's rows are said to have changed: {:?}",
        shown(&effects)
    );
    assert_eq!(
        focus.result_row(60).map(|row| row.message),
        Some(MessageId::new(1057))
    );
}

#[test]
fn passages_are_asked_for_a_page_after_it_lands_and_fill_it() {
    let rows = List::of(3);
    let mut focus = mac();
    let effects = show_all(&mut focus, "atlas budget", &rows);
    // Answer the pages, not yet their passages.
    let mut landed = Vec::new();
    for (ticket, request) in asks(&effects) {
        if matches!(request, Request::ResultsPage { .. }) {
            let reply = reply_to(&request).unwrap();
            landed.extend(focus.handle_on(Input::Reply(ticket, reply), &rows));
        }
    }
    let passages: Vec<Request> = asked(&landed)
        .into_iter()
        .filter(|request| matches!(request, Request::ResultsPassages { .. }))
        .collect();
    assert_eq!(passages.len(), 2, "one for Top hits, one for the page");
    let Request::ResultsPassages { hits, .. } = &passages[1] else {
        unreachable!()
    };
    assert_eq!(hits.len(), 50);

    let before = focus.result_row(3).expect("the newest");
    assert!(before.passage.is_empty(), "no passage before it is read");
    assert_eq!(before.source_tag, "body");

    let filled = settle(&mut focus, landed, &rows);
    assert!(
        shown(&filled)
            .iter()
            .any(|intent| matches!(intent, Intent::ResultsPage { .. }))
    );
    let after = focus.result_row(3).expect("the newest");
    assert!(text(&after.passage).starts_with('\u{2026}'));
    assert_eq!(lit(&after.passage), ["Atlas", "budget"]);
    assert!(
        after
            .accessible
            .starts_with("Ada Moreno, Atlas budget, part 0, matched in body: "),
        "{}",
        after.accessible
    );
    // A match in the quoted history says so (US2 scenario 2).
    let quoted = focus.result_row(7).expect("conversation 4");
    assert_eq!(quoted.message, MessageId::new(1004));
    assert_eq!(quoted.source_tag, "quoted text");
}

#[test]
fn a_filter_button_edits_the_query_and_asks_again() {
    let rows = List::of(3);
    let mut focus = mac();
    let _ = search(&mut focus, "atlas budget", &rows);
    let effects = focus.handle_on(
        Input::SearchEdit(TermEdit::Toggle {
            field: "has".to_owned(),
            value: "attachment".to_owned(),
        }),
        &rows,
    );
    let query = query_view(&effects).expect("the field follows the button");
    assert_eq!(query.words, "atlas budget");
    assert_eq!(
        query
            .chips
            .iter()
            .map(|chip| (chip.operator.as_str(), chip.value.as_str()))
            .collect::<Vec<_>>(),
        [("has:", "attachment")]
    );
    assert!(button(&query, FilterKind::Attachment).applied);
    assert!(!button(&query, FilterKind::From).applied);
    let asked_again: Vec<String> = asked(&effects)
        .into_iter()
        .filter_map(|request| match request {
            Request::ResultsPage { query, .. } => Some(query.input().to_owned()),
            _ => None,
        })
        .collect();
    assert!(!asked_again.is_empty());
    assert!(
        asked_again
            .iter()
            .all(|query| query == "atlas budget has:attachment")
    );

    // A person added: the button reads as who, once the answer has the
    // names; Ada's conversations are half of them.
    let effects = focus.handle_on(
        Input::SearchEdit(TermEdit::Add {
            field: "from".to_owned(),
            value: "ada@example.com".to_owned(),
            negated: false,
        }),
        &rows,
    );
    let effects = settle(&mut focus, effects, &rows);
    let query = query_view(&effects).expect("again");
    assert_eq!(button(&query, FilterKind::From).label, "From: Ada Moreno");
    assert!(button(&query, FilterKind::From).applied);
    assert_eq!(
        results_view(&effects).expect("again").count_line,
        "60 conversations"
    );

    // ✕ on a chip takes it out.
    let has = query
        .chips
        .iter()
        .find(|chip| chip.operator == "has:")
        .expect("the has: chip")
        .token;
    let effects = focus.handle_on(Input::SearchEdit(TermEdit::Remove { token: has }), &rows);
    let query = query_view(&effects).expect("again");
    assert!(!button(&query, FilterKind::Attachment).applied);
    assert!(button(&query, FilterKind::From).applied);
}

#[test]
fn j_and_k_move_the_focus_ring_and_return_opens_the_result() {
    let rows = List::of(3);
    let mut focus = mac();
    let _ = search(&mut focus, "atlas budget", &rows);
    let effects = run(&mut focus, CommandId::NextMessage, &rows);
    assert!(shown(&effects).contains(&Intent::ResultsCursor(1)));
    assert!(
        !shown(&effects)
            .iter()
            .any(|intent| matches!(intent, Intent::Cursor { .. })),
        "the inbox behind does not move"
    );
    let effects = run(&mut focus, CommandId::OpenMessage, &rows);
    assert!(
        shown(&effects).iter().any(|intent| matches!(
            intent,
            Intent::OpenMessage { message, .. } if *message == MessageId::new(1002)
        )),
        "{:?}",
        shown(&effects)
    );
    // j in the message walks the results, and the ring follows.
    let effects = run(&mut focus, CommandId::NextMessage, &rows);
    assert!(shown(&effects).iter().any(|intent| matches!(
        intent,
        Intent::OpenMessage { message, .. } if *message == MessageId::new(1030)
    )));
    assert!(shown(&effects).contains(&Intent::ResultsCursor(2)));
}

#[test]
fn esc_closes_the_dropdown_then_clears_the_selection_then_leaves_for_the_inbox() {
    let rows = List::of(3);
    let mut focus = mac();
    let _ = focus.landed(&rows, true);
    let _ = run(&mut focus, CommandId::NextMessage, &rows);
    let _ = run(&mut focus, CommandId::ToggleSelection, &rows);
    let inbox = focus.selection();
    assert_eq!(focus.cursor(), Some(1));

    let _ = search(&mut focus, "atlas budget", &rows);
    let effects = run(&mut focus, CommandId::ToggleSelection, &rows);
    assert_eq!(results_view(&effects).expect("redrawn").selected, 1);
    assert_eq!(focus.result_row(0).map(|row| row.checked), Some(true));

    // `/` over the results: the dropdown, on the results' query.
    let effects = run(&mut focus, CommandId::Search, &rows);
    assert!(shown(&effects).iter().any(|intent| matches!(
        intent,
        Intent::OpenBar { text, .. } if text == "atlas budget"
    )));
    assert_eq!(focus.key_context(), KeyContext::Search);

    // Esc: the dropdown, and only it.
    let effects = run(&mut focus, CommandId::Back, &rows);
    assert!(shown(&effects).contains(&Intent::CloseSurface(SurfaceKind::Bar)));
    assert!(focus.in_results());
    assert_eq!(focus.key_context(), KeyContext::Results);
    assert_eq!(focus.result_row(0).map(|row| row.checked), Some(true));

    // Esc: the selection.
    let effects = run(&mut focus, CommandId::Back, &rows);
    assert!(focus.in_results());
    assert_eq!(results_view(&effects).expect("redrawn").selected, 0);
    assert_eq!(focus.result_row(0).map(|row| row.checked), Some(false));

    // Esc: the inbox, as it was.
    let effects = run(&mut focus, CommandId::Back, &rows);
    assert!(!focus.in_results());
    assert_eq!(focus.key_context(), KeyContext::List);
    let intents = shown(&effects);
    assert_eq!(intents.first(), Some(&Intent::LeaveResults));
    assert!(intents.contains(&Intent::Cursor {
        position: 1,
        to_top: false
    }));
    assert!(intents.iter().any(|intent| matches!(
        intent,
        Intent::Selection { selection, .. } if *selection == inbox
    )));
    assert_eq!(focus.selection(), inbox);
}

#[test]
fn an_answer_for_a_query_since_edited_is_dropped() {
    let rows = List::of(3);
    let mut focus = mac();
    let first = show_all(&mut focus, "atlas budget", &rows);
    let _ = focus.handle_on(
        Input::SearchEdit(TermEdit::Toggle {
            field: "is".to_owned(),
            value: "unread".to_owned(),
        }),
        &rows,
    );
    let mut stale = Vec::new();
    for (ticket, request) in asks(&first) {
        if let Some(reply) = reply_to(&request) {
            stale.extend(focus.handle_on(Input::Reply(ticket, reply), &rows));
        }
    }
    assert!(
        stale.is_empty(),
        "the old query's answer draws and asks nothing: {stale:?}"
    );
}

#[test]
fn gtk_never_enters_the_results() {
    let rows = List::of(3);
    let mut focus = FocusController::new(Policy::for_platform(Platform::Freedesktop));
    let effects = show_all(&mut focus, "atlas budget", &rows);
    assert!(results_view(&effects).is_none());
    assert!(!focus.in_results());
    assert!(!focus.answers(CommandId::HistoryBack));
}
