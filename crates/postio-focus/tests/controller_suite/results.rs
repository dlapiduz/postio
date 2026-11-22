use crate::search_support::*;
use postio_config::paths::Platform;
use postio_core::CommandId;
use postio_focus::{
    Effect, FilterButton, FocusController, Input, Intent, Policy, PopoverView, Request,
    SurfaceKind, TermEdit,
};
use postio_model::MessageId;
use postio_search::results::{ConversationOrder, Counted};
use postio_ui::keymap::KeyContext;
use postio_ui::search_view::FilterKind;

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
            capped: false,
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
    assert!(view.sortable, "the conversations can be ranked or dated");
    assert_eq!(view.count_line, "120 threads");
    assert_eq!(view.sub_line, "12 files · 6 people · last 12 months");
    assert_eq!(view.footer, "120 threads · local index · 41 ms");
    assert_eq!(view.months.len(), 12);
    assert_eq!(view.months[11].label, "Sep");
    assert_eq!(view.tabs[0].label, "Threads");
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
    assert_eq!(top.count, "");
    assert_eq!(
        top.note.as_deref(),
        Some("why each one ranked is under the sender")
    );
    let september = &view.groups[1];
    assert_eq!(september.title, "September 2026");
    assert_eq!(september.first, 3);
    assert_eq!(september.rows, 13);
    assert_eq!(september.count, "13");
    assert_eq!(september.note.as_deref(), Some("newest first"));
    assert_eq!(september.accessible, "September 2026 · 13");
    assert_eq!(top.accessible, "Top hits · 3");
    assert_eq!(view.groups[2].title, "August 2026");
    assert_eq!(view.groups[2].first, 16);
    assert_eq!(view.rows, 3 + CONVERSATIONS as u64);
    assert_eq!(focus.result_count(), view.rows);

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
    let newest = focus.result_row(3).expect("the newest");
    assert_eq!(newest.message, MessageId::new(1000));
    assert!(!newest.top_hit);
    assert_eq!(newest.reason, None);
    assert_eq!(newest.group, 1);
    assert_eq!(newest.sender, "Ada Moreno");
    assert!(newest.unread);
    assert_eq!(text(&newest.subject), "Atlas budget, part 0");
    assert_eq!(lit(&newest.subject), ["Atlas", "budget"]);
    assert_eq!(
        newest.labels,
        [postio_focus::LabelPill {
            name: "Atlas".to_owned(),
            color: Some("#c08a2e".to_owned()),
        }],
        "the label's colour comes with its name"
    );
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

    let effects = focus.handle_on(Input::ResultsOrder(ConversationOrder::BestMatch), &rows);
    let effects = settle(&mut focus, effects, &rows);
    assert!(results_view(&effects).expect("again").groups[0].top_hits);
}

/// A page that lands before the top hits keeps its passages (#2089): the rows have no place
/// until the top hits land, but what was read for them is theirs.
#[test]
fn passages_that_land_before_the_top_hits_are_kept() {
    let rows = List::of(3);
    let mut focus = mac();
    let effects = show_all(&mut focus, "atlas budget", &rows);
    let (top, months): (Vec<_>, Vec<_>) = asks(&effects).into_iter().partition(|(_, request)| {
        matches!(
            request,
            Request::ResultsPage {
                order: ConversationOrder::BestMatch,
                ..
            }
        )
    });
    assert!(!top.is_empty() && !months.is_empty(), "{effects:?}");
    let mut early = Vec::new();
    for (ticket, request) in months {
        let reply = reply_to(&request).expect("a page");
        early.extend(focus.handle_on(Input::Reply(ticket, reply), &rows));
    }
    for (ticket, request) in asks(&early) {
        if matches!(request, Request::ResultsPassages { .. }) {
            let reply = reply_to(&request).expect("passages");
            let _ = focus.handle_on(Input::Reply(ticket, reply), &rows);
        }
    }
    let mut late = Vec::new();
    for (ticket, request) in top {
        let reply = reply_to(&request).expect("the top hits");
        late.extend(focus.handle_on(Input::Reply(ticket, reply), &rows));
    }
    let _ = settle(&mut focus, late, &rows);
    let top_len = 3;
    let tops: Vec<MessageId> = (0..top_len)
        .filter_map(|at| focus.result_row(at).map(|row| row.message))
        .collect();
    let bare: Vec<u64> = (top_len..top_len + 10)
        .filter(|at| {
            focus
                .result_row(*at)
                .is_some_and(|row| !tops.contains(&row.message) && row.passage.is_empty())
        })
        .collect();
    assert!(
        bare.is_empty(),
        "month rows without their passage: {bare:?}"
    );
}

#[test]
fn rows_page_through_a_window() {
    let rows = List::of(3);
    let mut focus = mac();
    let _ = search(&mut focus, "atlas budget", &rows);
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
    let quoted = focus.result_row(7).expect("conversation 4");
    assert_eq!(quoted.message, MessageId::new(1004));
    assert_eq!(quoted.source_tag, "quoted text");
}

#[test]
fn an_in_chip_calls_the_inbox_inbox() {
    let rows = List::of(3);
    let mut focus = mac();
    let effects = search(&mut focus, "atlas in:INBOX -in:inbox", &rows);
    let query = query_view(&effects).expect("the field");
    assert_eq!(
        query
            .chips
            .iter()
            .map(|chip| (chip.operator.as_str(), chip.value.as_str()))
            .collect::<Vec<_>>(),
        [("in:", "Inbox"), ("in:", "Inbox")],
        "the chip says the folder's name; the query keeps the store's"
    );
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
        "60 threads"
    );

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

    let effects = run(&mut focus, CommandId::Search, &rows);
    assert!(shown(&effects).iter().any(|intent| matches!(
        intent,
        Intent::OpenBar { text, .. } if text == "atlas budget"
    )));
    assert_eq!(focus.key_context(), KeyContext::Search);

    let effects = run(&mut focus, CommandId::Back, &rows);
    assert!(shown(&effects).contains(&Intent::CloseSurface(SurfaceKind::Bar)));
    assert!(focus.in_results());
    assert_eq!(focus.key_context(), KeyContext::Results);
    assert_eq!(focus.result_row(0).map(|row| row.checked), Some(true));

    let effects = run(&mut focus, CommandId::Back, &rows);
    assert!(focus.in_results());
    assert_eq!(results_view(&effects).expect("redrawn").selected, 0);
    assert_eq!(focus.result_row(0).map(|row| row.checked), Some(false));

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
fn linux_enters_the_results_as_the_mac_does() {
    let rows = List::of(3);
    let mut focus = FocusController::new(Policy::for_platform(Platform::Freedesktop));
    let effects = show_all(&mut focus, "atlas budget", &rows);
    assert!(results_view(&effects).is_some());
    assert!(focus.in_results());
    assert!(focus.answers(CommandId::HistoryBack));
}

fn open_popover(focus: &mut FocusController, kind: FilterKind, rows: &List) -> PopoverView {
    let effects = focus.handle_on(Input::SearchPopover(kind), rows);
    let query = query_view(&effects).expect("the buttons say which is open");
    assert!(button(&query, kind).open, "{kind:?} is ringed while open");
    popover_view(&effects)
        .flatten()
        .unwrap_or_else(|| panic!("the {kind:?} popover"))
}

fn titles(view: &PopoverView) -> Vec<(String, u64)> {
    view.rows
        .iter()
        .map(|row| (row.title.clone(), row.count))
        .collect()
}

#[test]
fn from_lists_the_people_in_the_results_with_their_counts() {
    let rows = List::of(3);
    let mut focus = mac();
    let _ = search(&mut focus, "atlas budget", &rows);
    let view = open_popover(&mut focus, FilterKind::From, &rows);
    assert_eq!(view.kind, FilterKind::From);
    assert_eq!(view.placeholder, "Filter people in these results");
    assert_eq!(
        titles(&view),
        [
            ("Ada Moreno".to_owned(), 24),
            ("Tom\u{e1}s Reyes".to_owned(), 24)
        ],
        "only the senders the results' facets hold"
    );
    let ada = &view.rows[0];
    assert_eq!(ada.detail.as_deref(), Some("ada@example.com"));
    assert_eq!(ada.initials.as_deref(), Some("AM"));
    assert!((ada.share - 1.0).abs() < f64::EPSILON);
    assert!(!ada.checked && !ada.excluded);
    assert_eq!(
        view.hints
            .iter()
            .map(|hint| (hint.key.as_str(), hint.label.as_str()))
            .collect::<Vec<_>>(),
        [
            ("space", "toggle"),
            ("alt", "-click excludes"),
            ("Return", "apply")
        ]
    );
}

#[test]
fn the_filter_popovers_open_by_their_keys() {
    let rows = List::of(3);
    let mut focus = mac();
    let effects = search(&mut focus, "atlas budget", &rows);
    let buttons = query_view(&effects).expect("the field").buttons;
    let keyed: Vec<(FilterKind, Option<String>)> = buttons
        .iter()
        .map(|button| (button.kind, button.key.clone()))
        .collect();
    for (kind, command) in [
        (FilterKind::From, CommandId::FilterFrom),
        (FilterKind::To, CommandId::FilterTo),
        (FilterKind::Date, CommandId::FilterDate),
        (FilterKind::Anywhere, CommandId::FilterAnywhere),
        (FilterKind::Label, CommandId::FilterLabel),
    ] {
        assert!(
            keyed.contains(&(kind, focus.keymap().binding(command).map(str::to_owned))),
            "{kind:?} shows no key: {keyed:?}"
        );
        assert!(focus.answers(command), "{command:?} is not the results'");
        let effects = run(&mut focus, command, &rows);
        let view = popover_view(&effects).flatten().expect("a popover");
        assert_eq!(view.kind, kind);
        let _ = focus.handle_on(Input::PopoverDone { apply: false }, &rows);
    }
}

#[test]
fn from_lists_you_only_when_the_query_already_asks_for_you() {
    let rows = List::of(3);
    let mut focus = mac();
    let _ = search(&mut focus, "atlas budget from:you@example.com", &rows);
    let effects = focus.handle_on(Input::SearchPopover(FilterKind::From), &rows);
    let request = asked(&effects)
        .into_iter()
        .find(|request| matches!(request, Request::Facets { .. }))
        .expect("the applied field's facets are read");
    let ticket = facets_asked(&effects)[0].0;
    let reply = reply_to(&request).expect("answered");
    let effects = focus.handle_on(Input::Reply(ticket, reply), &rows);
    let view = popover_view(&effects).flatten().expect("redrawn");
    let listed: Vec<(String, bool)> = view
        .rows
        .iter()
        .map(|row| (row.title.clone(), row.checked))
        .collect();
    assert_eq!(
        listed,
        [
            ("You".to_owned(), true),
            ("Ada Moreno".to_owned(), false),
            ("Tom\u{e1}s Reyes".to_owned(), false)
        ]
    );
}

#[test]
fn checking_a_person_previews_live_and_esc_restores_the_query_exactly() {
    let rows = List::of(3);
    let mut focus = mac();
    let _ = search(&mut focus, "atlas budget", &rows);
    let view = open_popover(&mut focus, FilterKind::From, &rows);
    let ada = view.rows[0].token;

    let effects = focus.handle_on(
        Input::PopoverToggle {
            token: ada,
            exclude: false,
        },
        &rows,
    );
    assert!(!queries_asked(&effects).is_empty());
    assert!(
        queries_asked(&effects)
            .iter()
            .all(|query| query == "atlas budget from:ada@example.com"),
        "{:?}",
        queries_asked(&effects)
    );
    let query = query_view(&effects).expect("the field follows the check");
    assert!(button(&query, FilterKind::From).applied);
    assert!(button(&query, FilterKind::From).open);
    let effects = settle(&mut focus, effects, &rows);
    let view = results_view(&effects).expect("the preview");
    assert_eq!(view.count_line, "60 threads");
    assert_eq!(
        view.sub_line,
        "previewing Ada Moreno \u{b7} \u{21a9} applies"
    );
    let popover = popover_view(&effects).flatten().expect("redrawn");
    assert!(popover.rows[0].checked);
    assert_eq!(
        titles(&popover),
        [
            ("Ada Moreno".to_owned(), 24),
            ("Tom\u{e1}s Reyes".to_owned(), 24)
        ],
        "the people it listed stay, with the counts it opened with"
    );

    let effects = focus.handle_on(Input::PopoverDone { apply: false }, &rows);
    assert_eq!(popover_view(&effects), Some(None), "the popover closes");
    let query = query_view(&effects).expect("the field restored");
    assert!(query.chips.is_empty());
    assert!(!button(&query, FilterKind::From).applied);
    assert!(!button(&query, FilterKind::From).open);
    assert!(
        queries_asked(&effects)
            .iter()
            .all(|query| query == "atlas budget")
    );
    let effects = settle(&mut focus, effects, &rows);
    let view = results_view(&effects).expect("as before");
    assert_eq!(view.count_line, "120 threads");
    assert_eq!(
        view.sub_line,
        "12 files \u{b7} 6 people \u{b7} last 12 months"
    );
}

#[test]
fn return_keeps_the_previewed_query() {
    let rows = List::of(3);
    let mut focus = mac();
    let _ = search(&mut focus, "atlas budget", &rows);
    let view = open_popover(&mut focus, FilterKind::From, &rows);
    let effects = focus.handle_on(
        Input::PopoverToggle {
            token: view.rows[0].token,
            exclude: false,
        },
        &rows,
    );
    let _ = settle(&mut focus, effects, &rows);
    let effects = focus.handle_on(Input::PopoverDone { apply: true }, &rows);
    assert_eq!(popover_view(&effects), Some(None));
    assert!(queries_asked(&effects).is_empty(), "nothing to ask again");
    let query = query_view(&effects).expect("the field");
    assert!(button(&query, FilterKind::From).applied);
    assert!(!button(&query, FilterKind::From).open);
    let view = results_view(&effects).expect("the frame, no longer a preview");
    assert_eq!(view.sub_line, "from Ada \u{b7} 12 files");
    assert_eq!(view.count_line, "60 threads");
}

#[test]
fn alt_click_excludes_and_again_takes_it_out() {
    let rows = List::of(3);
    let mut focus = mac();
    let _ = search(&mut focus, "atlas budget", &rows);
    let view = open_popover(&mut focus, FilterKind::From, &rows);
    let ada = view.rows[0].token;
    let effects = focus.handle_on(
        Input::PopoverToggle {
            token: ada,
            exclude: true,
        },
        &rows,
    );
    let query = query_view(&effects).expect("the field");
    assert_eq!(
        query
            .chips
            .iter()
            .map(|chip| (chip.operator.as_str(), chip.value.as_str(), chip.excluded))
            .collect::<Vec<_>>(),
        [("from:", "Ada Moreno", true)],
        "struck through, and named"
    );
    let effects = settle(&mut focus, effects, &rows);
    assert_eq!(
        results_view(&effects).expect("the preview").count_line,
        "60 threads"
    );
    let row = &popover_view(&effects).flatten().expect("redrawn").rows[0];
    assert!(row.excluded && !row.checked);

    let effects = focus.handle_on(
        Input::PopoverToggle {
            token: ada,
            exclude: false,
        },
        &rows,
    );
    assert_eq!(
        queries_asked(&effects).first().map(String::as_str),
        Some("atlas budget from:ada@example.com")
    );
    let effects = focus.handle_on(
        Input::PopoverToggle {
            token: ada,
            exclude: true,
        },
        &rows,
    );
    assert_eq!(
        queries_asked(&effects).first().map(String::as_str),
        Some("atlas budget -from:ada@example.com")
    );
    let effects = focus.handle_on(
        Input::PopoverToggle {
            token: ada,
            exclude: true,
        },
        &rows,
    );
    assert_eq!(
        queries_asked(&effects).first().map(String::as_str),
        Some("atlas budget")
    );
}

fn chips(query: &postio_focus::QueryView) -> Vec<(String, String, bool)> {
    query
        .chips
        .iter()
        .map(|chip| (chip.operator.clone(), chip.value.clone(), chip.excluded))
        .collect()
}

#[test]
fn checking_a_second_person_is_either_of_them() {
    let rows = List::of(3);
    let mut focus = mac();
    let _ = search(&mut focus, "atlas budget", &rows);
    let view = open_popover(&mut focus, FilterKind::From, &rows);
    let (ada, tomas) = (view.rows[0].token, view.rows[1].token);
    let check = |focus: &mut FocusController, token| {
        focus.handle_on(
            Input::PopoverToggle {
                token,
                exclude: false,
            },
            &rows,
        )
    };

    let effects = check(&mut focus, ada);
    let _ = settle(&mut focus, effects, &rows);
    let effects = check(&mut focus, tomas);
    assert_eq!(
        queries_asked(&effects).first().map(String::as_str),
        Some("atlas budget from:{ada@example.com tomas@example.com}"),
        "either, not both"
    );
    let query = query_view(&effects).expect("the field follows the check");
    assert_eq!(
        chips(&query),
        [(
            "from:".to_owned(),
            "Ada Moreno, Tom\u{e1}s Reyes".to_owned(),
            false
        )],
        "one chip naming both"
    );
    assert_eq!(
        button(&query, FilterKind::From).label,
        "From: Ada Moreno +1"
    );
    let effects = settle(&mut focus, effects, &rows);
    let popover = popover_view(&effects).flatten().expect("redrawn");
    assert!(
        popover.rows[0].checked && popover.rows[1].checked,
        "both are checked"
    );

    let effects = check(&mut focus, ada);
    assert_eq!(
        queries_asked(&effects).first().map(String::as_str),
        Some("atlas budget from:tomas@example.com")
    );
    let effects = settle(&mut focus, effects, &rows);
    let popover = popover_view(&effects).flatten().expect("redrawn");
    assert!(!popover.rows[0].checked && popover.rows[1].checked);

    let effects = focus.handle_on(Input::PopoverDone { apply: false }, &rows);
    assert_eq!(popover_view(&effects), Some(None));
    assert!(
        queries_asked(&effects)
            .iter()
            .all(|query| query == "atlas budget")
    );
    assert!(query_view(&effects).expect("restored").chips.is_empty());
}

fn facets_asked(effects: &[Effect]) -> Vec<(postio_focus::Ticket, String)> {
    asks(effects)
        .into_iter()
        .filter_map(|(ticket, request)| match request {
            Request::Facets { query, .. } => Some((ticket, query.input().to_owned())),
            _ => None,
        })
        .collect()
}

#[test]
fn a_popover_lists_without_its_own_fields_terms() {
    let rows = List::of(3);
    let mut focus = mac();
    let _ = search(&mut focus, "atlas budget from:ada@example.com", &rows);

    let effects = focus.handle_on(Input::SearchPopover(FilterKind::To), &rows);
    assert!(facets_asked(&effects).is_empty(), "To is not applied");
    let _ = focus.handle_on(Input::PopoverDone { apply: false }, &rows);

    let effects = focus.handle_on(Input::SearchPopover(FilterKind::From), &rows);
    let reads = facets_asked(&effects);
    assert_eq!(reads.len(), 1, "one read for the applied field");
    assert_eq!(reads[0].1, "atlas budget", "the query without its from:");
    let ticket = reads[0].0;
    let request = asked(&effects)
        .into_iter()
        .find(|request| matches!(request, Request::Facets { .. }))
        .expect("asked");
    assert_eq!(request.lane(), Some(postio_focus::Lane::Facets));
    let view = popover_view(&effects).flatten().expect("open at once");
    assert!(view.rows.is_empty(), "nothing is listed until it is read");

    let reply = reply_to(&request).expect("answered");
    let effects = focus.handle_on(Input::Reply(ticket, reply), &rows);
    let view = popover_view(&effects).flatten().expect("redrawn");
    assert_eq!(
        titles(&view),
        [
            ("Ada Moreno".to_owned(), 24),
            ("Tom\u{e1}s Reyes".to_owned(), 24)
        ],
        "counts without the from: term"
    );
    assert!(view.rows[0].checked && !view.rows[1].checked);

    let effects = focus.handle_on(
        Input::PopoverToggle {
            token: view.rows[1].token,
            exclude: false,
        },
        &rows,
    );
    assert_eq!(
        queries_asked(&effects).first().map(String::as_str),
        Some("atlas budget from:{ada@example.com tomas@example.com}")
    );
    let effects = settle(&mut focus, effects, &rows);
    let view = popover_view(&effects).flatten().expect("redrawn");
    assert!(view.rows[0].checked && view.rows[1].checked);
    assert_eq!(view.rows.len(), 2);

    let effects = focus.handle_on(Input::PopoverDone { apply: false }, &rows);
    assert_eq!(popover_view(&effects), Some(None));
    assert!(
        queries_asked(&effects)
            .iter()
            .all(|query| query == "atlas budget from:ada@example.com"),
        "{:?}",
        queries_asked(&effects)
    );
}

#[test]
fn a_set_applied_opens_with_every_member_checked() {
    let rows = List::of(3);
    let mut focus = mac();
    let _ = search(
        &mut focus,
        "atlas budget from:{ada@example.com tomas@example.com}",
        &rows,
    );
    let effects = focus.handle_on(Input::SearchPopover(FilterKind::From), &rows);
    let effects = settle(&mut focus, effects, &rows);
    let view = popover_view(&effects)
        .flatten()
        .expect("the From popover, once its facets are read");
    assert!(view.rows.iter().all(|row| row.checked && !row.excluded));
}

#[test]
fn alt_click_on_a_second_person_excludes_both() {
    let rows = List::of(3);
    let mut focus = mac();
    let _ = search(&mut focus, "atlas budget", &rows);
    let view = open_popover(&mut focus, FilterKind::From, &rows);
    let (ada, tomas) = (view.rows[0].token, view.rows[1].token);
    let exclude = |focus: &mut FocusController, token| {
        focus.handle_on(
            Input::PopoverToggle {
                token,
                exclude: true,
            },
            &rows,
        )
    };
    let effects = exclude(&mut focus, ada);
    let _ = settle(&mut focus, effects, &rows);
    let effects = exclude(&mut focus, tomas);
    assert_eq!(
        queries_asked(&effects).first().map(String::as_str),
        Some("atlas budget -from:{ada@example.com tomas@example.com}"),
        "neither"
    );
    let query = query_view(&effects).expect("the field");
    assert_eq!(
        chips(&query),
        [(
            "from:".to_owned(),
            "Ada Moreno, Tom\u{e1}s Reyes".to_owned(),
            true
        )]
    );
    let effects = settle(&mut focus, effects, &rows);
    let popover = popover_view(&effects).flatten().expect("redrawn");
    assert!(popover.rows.iter().all(|row| row.excluded && !row.checked));
    let effects = exclude(&mut focus, ada);
    assert_eq!(
        queries_asked(&effects).first().map(String::as_str),
        Some("atlas budget -from:tomas@example.com")
    );
}

#[test]
fn the_popovers_own_filter_narrows_its_rows_without_asking() {
    let rows = List::of(3);
    let mut focus = mac();
    let _ = search(&mut focus, "atlas budget", &rows);
    let _ = open_popover(&mut focus, FilterKind::From, &rows);
    let effects = focus.handle_on(Input::PopoverFilter("TOM".to_owned()), &rows);
    assert!(
        asked(&effects).is_empty(),
        "narrowed here, not by the engine"
    );
    let view = popover_view(&effects).flatten().expect("redrawn");
    assert_eq!(view.filter, "TOM");
    assert_eq!(titles(&view), [("Tom\u{e1}s Reyes".to_owned(), 24)]);
    let effects = focus.handle_on(Input::PopoverFilter("ada@".to_owned()), &rows);
    let view = popover_view(&effects).flatten().expect("redrawn");
    assert_eq!(titles(&view), [("Ada Moreno".to_owned(), 24)]);
    let effects = focus.handle_on(
        Input::PopoverToggle {
            token: view.rows[0].token,
            exclude: false,
        },
        &rows,
    );
    assert_eq!(
        queries_asked(&effects).first().map(String::as_str),
        Some("atlas budget from:ada@example.com")
    );
}

#[test]
fn to_anywhere_and_label_list_their_own_facets() {
    let rows = List::of(3);
    let mut focus = mac();
    let _ = search(&mut focus, "atlas budget", &rows);

    let to = open_popover(&mut focus, FilterKind::To, &rows);
    assert_eq!(titles(&to), [("Ben Adeyemi".to_owned(), 7)]);
    assert_eq!(to.placeholder, "Filter people in these results");
    let _ = focus.handle_on(Input::PopoverDone { apply: false }, &rows);

    let anywhere = open_popover(&mut focus, FilterKind::Anywhere, &rows);
    assert_eq!(
        titles(&anywhere),
        [("Inbox".to_owned(), 117), ("Archive".to_owned(), 3)]
    );
    assert_eq!(anywhere.placeholder, "Filter folders");
    assert_eq!(anywhere.rows[0].initials, None);
    let effects = focus.handle_on(
        Input::PopoverToggle {
            token: anywhere.rows[1].token,
            exclude: false,
        },
        &rows,
    );
    assert_eq!(
        queries_asked(&effects).first().map(String::as_str),
        Some("atlas budget in:Archive")
    );
    let effects = focus.handle_on(Input::PopoverDone { apply: false }, &rows);
    assert_eq!(
        queries_asked(&effects).first().map(String::as_str),
        Some("atlas budget")
    );
    let _ = settle(&mut focus, effects, &rows);

    let label = open_popover(&mut focus, FilterKind::Label, &rows);
    assert_eq!(titles(&label), [("Atlas".to_owned(), 1)]);
    assert_eq!(label.rows[0].color.as_deref(), Some("#c08a2e"));
    assert_eq!(label.placeholder, "Filter labels");
    let effects = focus.handle_on(
        Input::PopoverToggle {
            token: label.rows[0].token,
            exclude: false,
        },
        &rows,
    );
    assert_eq!(
        queries_asked(&effects).first().map(String::as_str),
        Some("atlas budget label:Atlas")
    );
}

#[test]
fn esc_closes_an_open_popover_before_anything_else() {
    let rows = List::of(3);
    let mut focus = mac();
    let _ = search(&mut focus, "atlas budget", &rows);
    let view = open_popover(&mut focus, FilterKind::From, &rows);
    let _ = focus.handle_on(
        Input::PopoverToggle {
            token: view.rows[0].token,
            exclude: false,
        },
        &rows,
    );
    let effects = run(&mut focus, CommandId::Back, &rows);
    assert_eq!(popover_view(&effects), Some(None));
    assert!(focus.in_results(), "the popover's rung, and only it");
    assert!(
        queries_asked(&effects)
            .iter()
            .all(|query| query == "atlas budget")
    );
    let effects = focus.handle_on(Input::SearchPopover(FilterKind::Unread), &rows);
    assert_eq!(popover_view(&effects), None);
}

fn selected_months(view: &postio_focus::ResultsView) -> Vec<usize> {
    view.months
        .iter()
        .enumerate()
        .filter(|(_, bar)| bar.selected)
        .map(|(at, _)| at)
        .collect()
}

#[test]
fn months_set_after_and_before_and_come_back_marked() {
    let rows = List::of(3);
    let mut focus = mac();
    let effects = search(&mut focus, "atlas budget", &rows);
    let view = results_view(&effects).expect("the results");
    assert!(selected_months(&view).is_empty());
    assert_eq!(
        view.timeline_hint,
        "Matches by month \u{b7} drag across months to narrow"
    );
    assert_eq!(view.timeline_step, None);

    let effects = focus.handle_on(
        Input::SearchEdit(TermEdit::SetMonths { first: 5, last: 3 }),
        &rows,
    );
    assert_eq!(
        queries_asked(&effects).first().map(String::as_str),
        Some("atlas budget after:2026-01-01 before:2026-04-01")
    );
    let query = query_view(&effects).expect("the field");
    assert_eq!(
        button(&query, FilterKind::Date).label,
        "January \u{2013} March"
    );
    let effects = settle(&mut focus, effects, &rows);
    let view = results_view(&effects).expect("narrowed");
    assert_eq!(selected_months(&view), [3, 4, 5]);
    assert_eq!(view.months[11].conversations, 13);
    assert!(view.months[11].height > 0.0);
    assert_eq!(
        view.timeline_hint,
        "Jan \u{2013} Mar selected \u{b7} drag to change"
    );
    assert_eq!(
        view.timeline_step,
        Some(postio_ui::hints::Hint {
            key: "alt+Left/alt+Right".to_owned(),
            label: "steps a month".to_owned(),
        })
    );
}

#[test]
fn alt_arrows_step_the_range_by_a_month() {
    let rows = List::of(3);
    let mut focus = mac();
    let _ = search(&mut focus, "atlas budget", &rows);
    assert_eq!(focus.key_context(), KeyContext::Results);

    let effects = run(&mut focus, CommandId::StepRangeForward, &rows);
    assert!(queries_asked(&effects).is_empty());
    let effects = run(&mut focus, CommandId::StepRangeBack, &rows);
    assert_eq!(
        queries_asked(&effects).first().map(String::as_str),
        Some("atlas budget after:2026-09-01 before:2026-10-01")
    );
    let effects = settle(&mut focus, effects, &rows);
    assert_eq!(
        selected_months(&results_view(&effects).expect("this month")),
        [11]
    );
    let effects = run(&mut focus, CommandId::StepRangeForward, &rows);
    assert!(queries_asked(&effects).is_empty());

    let effects = focus.handle_on(
        Input::SearchEdit(TermEdit::SetMonths { first: 3, last: 5 }),
        &rows,
    );
    let _ = settle(&mut focus, effects, &rows);
    let effects = run(&mut focus, CommandId::StepRangeForward, &rows);
    assert_eq!(
        queries_asked(&effects).first().map(String::as_str),
        Some("atlas budget after:2026-02-01 before:2026-05-01"),
        "both bounds a month later, in place"
    );
    let _ = settle(&mut focus, effects, &rows);
    let _ = run(&mut focus, CommandId::StepRangeBack, &rows);
    let effects = run(&mut focus, CommandId::StepRangeBack, &rows);
    assert_eq!(
        queries_asked(&effects).first().map(String::as_str),
        Some("atlas budget after:2025-12-01 before:2026-03-01")
    );
    let effects = settle(&mut focus, effects, &rows);
    assert_eq!(
        selected_months(&results_view(&effects).expect("stepped")),
        [2, 3, 4]
    );

    let effects = focus.handle_on(Input::SearchEdit(TermEdit::ClearFilters), &rows);
    let _ = settle(&mut focus, effects, &rows);
    let effects = focus.handle_on(
        Input::SearchEdit(TermEdit::Add {
            field: "after".to_owned(),
            value: "2026-07-01".to_owned(),
            negated: false,
        }),
        &rows,
    );
    let _ = settle(&mut focus, effects, &rows);
    let effects = run(&mut focus, CommandId::StepRangeBack, &rows);
    assert_eq!(
        queries_asked(&effects).first().map(String::as_str),
        Some("atlas budget after:2026-06-01 before:2026-09-01")
    );
}

#[test]
fn the_date_popover_turns_words_into_dates_and_its_presets_carry_counts() {
    let rows = List::of(3);
    let mut focus = mac();
    let _ = search(&mut focus, "atlas budget", &rows);
    let view = open_popover(&mut focus, FilterKind::Date, &rows);
    assert_eq!(
        view.presets
            .iter()
            .map(|preset| (
                preset.label.as_str(),
                preset.count.as_deref(),
                preset.selected
            ))
            .collect::<Vec<_>>(),
        [
            ("Any time", Some("120"), true),
            ("Last 7 days", Some("2"), false),
            ("Last 30 days", Some("6"), false),
            ("This quarter", Some("12"), false),
            ("This year", Some("17"), false),
            ("Custom\u{2026}", None, false),
        ]
    );
    assert_eq!(
        view.words_hint,
        "Type a date in plain words, or drag across the months."
    );
    assert_eq!(view.months.len(), 12);
    assert!(view.rows.is_empty());
    assert_eq!(view.parsed, None);

    let effects = focus.handle_on(Input::DateWords("since july".to_owned()), &rows);
    assert_eq!(
        queries_asked(&effects).first().map(String::as_str),
        Some("atlas budget after:2026-07-01")
    );
    let popover = popover_view(&effects).flatten().expect("redrawn");
    assert_eq!(popover.words, "since july");
    assert_eq!(popover.parsed.as_deref(), Some("\u{2192} after:2026-07-01"));
    let effects = settle(&mut focus, effects, &rows);
    let popover = popover_view(&effects).flatten().expect("again, counted");
    assert_eq!(
        popover
            .presets
            .iter()
            .filter(|preset| preset.selected)
            .map(|preset| preset.label.as_str())
            .collect::<Vec<_>>(),
        ["This quarter"]
    );
    assert_eq!(popover.result.as_deref(), Some("44 of 120"));
    assert_eq!(popover.range.as_deref(), Some("Jul \u{2013} Sep 2026"));
    let selected: Vec<usize> = popover
        .months
        .iter()
        .enumerate()
        .filter(|(_, bar)| bar.selected)
        .map(|(at, _)| at)
        .collect();
    assert_eq!(selected, [9, 10, 11]);
    assert!(popover.months[0..9].iter().any(|bar| bar.conversations > 0));
    let view = results_view(&effects).expect("the preview");
    assert_eq!(view.count_line, "44 threads");
    assert_eq!(
        view.sub_line,
        "previewing Jul \u{2013} Sep \u{b7} \u{21a9} applies"
    );

    let effects = focus.handle_on(Input::DateWords("budget".to_owned()), &rows);
    assert!(queries_asked(&effects).is_empty());
    assert_eq!(
        popover_view(&effects).flatten().expect("redrawn").parsed,
        None
    );
    let effects = focus.handle_on(Input::DateWords(String::new()), &rows);
    assert_eq!(
        queries_asked(&effects).first().map(String::as_str),
        Some("atlas budget")
    );

    let effects = focus.handle_on(Input::DatePreset(1), &rows);
    assert_eq!(
        queries_asked(&effects).first().map(String::as_str),
        Some("atlas budget after:2026-09-19")
    );
    let effects = focus.handle_on(Input::DatePreset(0), &rows);
    assert_eq!(
        queries_asked(&effects).first().map(String::as_str),
        Some("atlas budget")
    );
    let effects = focus.handle_on(Input::DatePreset(5), &rows);
    assert!(queries_asked(&effects).is_empty());

    let _ = focus.handle_on(Input::DateWords("since july".to_owned()), &rows);
    let effects = focus.handle_on(Input::PopoverDone { apply: true }, &rows);
    let query = query_view(&effects).expect("the field");
    assert_eq!(button(&query, FilterKind::Date).label, "Since July");
    assert!(button(&query, FilterKind::Date).applied);
}

#[test]
fn leaving_the_results_takes_an_open_popover_down() {
    let rows = List::of(3);
    let mut focus = mac();
    let _ = search(&mut focus, "atlas budget", &rows);
    let _ = open_popover(&mut focus, FilterKind::From, &rows);
    let effects = run(&mut focus, CommandId::HistoryBack, &rows);
    assert!(!focus.in_results());
    assert_eq!(
        popover_view(&effects),
        Some(None),
        "no popover is left hanging over the inbox"
    );
}

#[test]
fn a_persons_chip_reads_as_their_name_and_the_query_keeps_the_address() {
    let rows = List::of(3);
    let mut focus = mac();
    let _ = search(&mut focus, "atlas budget", &rows);
    let effects = focus.handle_on(
        Input::SearchEdit(TermEdit::Add {
            field: "from".to_owned(),
            value: "ada@example.com".to_owned(),
            negated: false,
        }),
        &rows,
    );
    let effects = settle(&mut focus, effects, &rows);
    let query = query_view(&effects).expect("the field");
    let chips: Vec<(&str, &str)> = query
        .chips
        .iter()
        .map(|chip| (chip.operator.as_str(), chip.value.as_str()))
        .collect();
    assert_eq!(chips, [("from:", "Ada Moreno")]);
    assert!(
        queries_asked(&effects)
            .iter()
            .all(|asked| asked == "atlas budget from:ada@example.com"),
        "the query is the address: {:?}",
        queries_asked(&effects)
    );

    let effects = focus.handle_on(
        Input::SearchEdit(TermEdit::Add {
            field: "to".to_owned(),
            value: "nobody@example.com".to_owned(),
            negated: false,
        }),
        &rows,
    );
    let query = query_view(&effects).expect("the field");
    assert_eq!(
        query
            .chips
            .iter()
            .map(|chip| chip.value.as_str())
            .collect::<Vec<_>>(),
        ["Ada Moreno", "nobody@example.com"],
        "no name, the value as typed"
    );
}

fn looked(effects: &[Effect]) -> postio_focus::QuickLookView {
    quick_look_view(effects)
        .flatten()
        .expect("Quick Look, drawn")
}

fn matches_asked(effects: &[Effect]) -> Vec<postio_search::results::ConversationKey> {
    asked(effects)
        .into_iter()
        .filter_map(|request| match request {
            Request::QuickLookMatches { key, .. } => Some(key),
            _ => None,
        })
        .collect()
}

#[test]
fn quick_look_opens_on_the_focused_result_and_reads_its_matches() {
    let rows = List::of(3);
    let mut focus = mac();
    let _ = search(&mut focus, "atlas budget", &rows);
    let effects = run(&mut focus, CommandId::QuickLook, &rows);

    let view = looked(&effects);
    assert_eq!(view.title, "Quick Look");
    assert_eq!(view.position, "1 of 120");
    assert_eq!(text(&view.subject), "Atlas budget, part 7");
    assert_eq!(lit(&view.subject), ["Atlas", "budget"]);
    assert!(text(&view.sender).starts_with("Tom\u{e1}s Reyes tomas@example.com"));
    assert!(
        text(&view.sender).contains(" \u{b7} Sat 12 Sep, "),
        "{}",
        text(&view.sender)
    );
    assert_eq!(
        view.walk
            .as_ref()
            .map(|hint| (hint.key.as_str(), hint.label.as_str())),
        Some(("j/k", "moves through results while it stays open"))
    );
    assert_eq!(
        view.actions
            .iter()
            .map(|hint| (hint.label.as_str(), hint.key.as_str()))
            .collect::<Vec<_>>(),
        [("Open", "Return"), ("Archive", "e"), ("Close", "space")]
    );
    assert_eq!(
        matches_asked(&effects),
        [postio_search::results::ConversationKey::Thread(
            postio_model::ThreadId::new(2007)
        )]
    );
    assert!(focus.in_results());
    assert_eq!(
        focus.key_context(),
        KeyContext::Results,
        "the results keep the keys"
    );

    let effects = settle(&mut focus, effects, &rows);
    let view = looked(&effects);
    assert_eq!(view.matches_line, "3 matches in this thread");
    assert_eq!(
        view.matches_hint
            .as_ref()
            .map(|hint| (hint.key.as_str(), hint.label.as_str())),
        Some(("]/[", "jump between them"))
    );
    assert_eq!(
        view.cards
            .iter()
            .map(|card| (card.place.as_str(), card.when.as_str()))
            .collect::<Vec<_>>(),
        [
            ("Body", "Ben · 11 Sep"),
            ("Body", "Tomás · 12 Sep"),
            ("Subject", "")
        ]
    );
    assert_eq!(lit(&view.cards[1].passage), ["Atlas", "budget"]);
    assert_eq!(view.current, Some(1), "the result's own message");
    assert!(
        text(&view.sender).starts_with(
            "Tom\u{e1}s Reyes tomas@example.com \u{b7} to you, Finance team \u{b7} Sat 12 Sep, "
        ),
        "{}",
        text(&view.sender)
    );
}

#[test]
fn j_and_k_move_the_results_and_quick_look_follows_in_place() {
    let rows = List::of(3);
    let mut focus = mac();
    let _ = search(&mut focus, "atlas budget", &rows);
    let effects = run(&mut focus, CommandId::QuickLook, &rows);
    let _ = settle(&mut focus, effects, &rows);

    let effects = run(&mut focus, CommandId::NextMessage, &rows);
    assert!(shown(&effects).contains(&Intent::ResultsCursor(1)));
    let view = looked(&effects);
    assert_eq!(view.position, "2 of 120");
    assert_eq!(text(&view.subject), "Atlas budget, part 2");
    assert_eq!(
        matches_asked(&effects),
        [postio_search::results::ConversationKey::Thread(
            postio_model::ThreadId::new(2002)
        )]
    );
    let effects = settle(&mut focus, effects, &rows);
    assert_eq!(looked(&effects).cards.len(), 3);

    let effects = run(&mut focus, CommandId::PrevMessage, &rows);
    assert_eq!(looked(&effects).position, "1 of 120");

    let mut said = Vec::new();
    for _ in 0..6 {
        let effects = run(&mut focus, CommandId::NextMessage, &rows);
        let view = looked(&effects);
        said.push((view.position, text(&view.subject)));
    }
    let said: Vec<(&str, &str)> = said
        .iter()
        .map(|(at, subject)| (at.as_str(), subject.as_str()))
        .collect();
    assert_eq!(
        said,
        [
            ("2 of 120", "Atlas budget, part 2"),
            ("3 of 120", "Atlas budget, part 30"),
            ("4 of 120", "Atlas budget, part 0"),
            ("5 of 120", "Atlas budget, part 1"),
            ("2 of 120", "Atlas budget, part 2"),
            ("6 of 120", "Atlas budget, part 3"),
        ]
    );
}

#[test]
fn brackets_move_the_ring_between_the_cards_while_it_is_open() {
    let rows = List::of(3);
    let mut focus = mac();
    let _ = search(&mut focus, "atlas budget", &rows);
    assert!(
        quick_look_view(&run(&mut focus, CommandId::NextMatch, &rows)).is_none(),
        "] does nothing while Quick Look is closed"
    );
    let effects = run(&mut focus, CommandId::QuickLook, &rows);
    let _ = settle(&mut focus, effects, &rows);

    let effects = run(&mut focus, CommandId::NextMatch, &rows);
    assert_eq!(looked(&effects).current, Some(2));
    assert!(
        quick_look_view(&run(&mut focus, CommandId::NextMatch, &rows)).is_none(),
        "the last card stays ringed: nothing to redraw"
    );
    let effects = run(&mut focus, CommandId::PrevMatch, &rows);
    assert_eq!(looked(&effects).current, Some(1));
    let effects = run(&mut focus, CommandId::PrevMatch, &rows);
    assert_eq!(looked(&effects).current, Some(0));
    assert!(
        !shown(&effects)
            .iter()
            .any(|intent| matches!(intent, Intent::ResultsCursor(_))),
        "the results' ring does not move"
    );
}

#[test]
fn a_ring_moved_before_the_matches_land_stays_on_its_card() {
    let rows = List::of(3);
    let mut focus = mac();
    let _ = search(&mut focus, "atlas budget", &rows);
    let opened = run(&mut focus, CommandId::QuickLook, &rows);
    let places = |view: &postio_focus::QuickLookView| -> Vec<String> {
        view.cards.iter().map(|card| card.place.clone()).collect()
    };
    let view = looked(&opened);
    assert_eq!(places(&view), ["Body", "Subject"], "the row's own cards");
    assert_eq!(view.current, Some(0));

    let stepped = looked(&run(&mut focus, CommandId::NextMatch, &rows));
    assert_eq!(stepped.current, Some(1), "the subject's card");

    let landed = looked(&settle(&mut focus, opened, &rows));
    assert_eq!(places(&landed), ["Body", "Body", "Subject"]);
    assert_eq!(
        landed.current,
        Some(2),
        "the conversation's matches took the ring off the subject's card, \
         where the person had moved it"
    );
}

#[test]
fn return_opens_the_message_and_quick_look_goes() {
    let rows = List::of(3);
    let mut focus = mac();
    let _ = search(&mut focus, "atlas budget", &rows);
    let _ = run(&mut focus, CommandId::QuickLook, &rows);
    let effects = run(&mut focus, CommandId::OpenMessage, &rows);
    assert_eq!(quick_look_view(&effects), Some(None));
    let intents = shown(&effects);
    let closed = intents
        .iter()
        .position(|intent| *intent == Intent::QuickLook(None))
        .expect("closed");
    let opened = intents
        .iter()
        .position(|intent| {
            matches!(
                intent,
                Intent::OpenMessage { message, .. } if *message == MessageId::new(1007)
            )
        })
        .expect("the message window opens");
    assert!(closed < opened, "the panel goes before the window comes");
}

#[test]
fn archive_moves_quick_look_to_the_next_result_and_closes_after_the_last() {
    let rows = List::of(3);
    let mut focus = mac();
    let _ = search(&mut focus, "atlas budget", &rows);
    let _ = run(&mut focus, CommandId::QuickLook, &rows);
    let effects = run(&mut focus, CommandId::Archive, &rows);
    assert!(
        asked(&effects).iter().any(|request| matches!(
            request,
            Request::Send {
                command: postio_core::Command::Archive { .. },
                ..
            }
        )),
        "the conversation is archived: {:?}",
        asked(&effects)
    );
    assert!(shown(&effects).contains(&Intent::ResultsCursor(1)));
    assert_eq!(looked(&effects).position, "2 of 120");

    let effects = run(&mut focus, CommandId::Undo, &rows);
    assert_eq!(asked(&effects), [Request::Post(postio_core::Command::Undo)]);
    assert_eq!(quick_look_view(&effects), None, "nothing closed");

    let _ = run(&mut focus, CommandId::QuickLook, &rows);
    let _ = run(&mut focus, CommandId::LastMessage, &rows);
    let last = focus.result_count() - 1;
    let wanted = focus.results_wanted(last);
    let _ = settle(&mut focus, wanted, &rows);
    let effects = run(&mut focus, CommandId::QuickLook, &rows);
    assert_eq!(looked(&effects).position, "120 of 120");
    let effects = run(&mut focus, CommandId::Archive, &rows);
    assert_eq!(quick_look_view(&effects), Some(None));
    assert!(focus.in_results());
}

#[test]
fn space_or_esc_closes_quick_look_and_nothing_else() {
    let rows = List::of(3);
    let mut focus = mac();
    let _ = search(&mut focus, "atlas budget", &rows);
    let _ = run(&mut focus, CommandId::NextMessage, &rows);
    let _ = run(&mut focus, CommandId::ToggleSelection, &rows);

    let _ = run(&mut focus, CommandId::QuickLook, &rows);
    let effects = run(&mut focus, CommandId::QuickLook, &rows);
    assert_eq!(
        quick_look_view(&effects),
        Some(None),
        "Space again closes it"
    );

    let _ = run(&mut focus, CommandId::QuickLook, &rows);
    let effects = run(&mut focus, CommandId::Back, &rows);
    assert_eq!(quick_look_view(&effects), Some(None), "so does Esc");
    assert!(focus.in_results());
    let view = results_view(&effects);
    assert!(
        view.is_none_or(|view| view.selected == 1),
        "the checked row is still checked: Esc's rung was Quick Look's"
    );
    let effects = run(&mut focus, CommandId::Back, &rows);
    assert_eq!(
        results_view(&effects).map(|view| view.selected),
        Some(0),
        "the next Esc clears the selection"
    );
}

#[test]
fn quick_look_goes_with_the_results_it_looked_into() {
    let rows = List::of(3);
    let mut focus = mac();
    let _ = search(&mut focus, "atlas budget", &rows);
    let _ = run(&mut focus, CommandId::QuickLook, &rows);
    let effects = focus.handle_on(
        Input::SearchEdit(TermEdit::Toggle {
            field: "has".to_owned(),
            value: "attachment".to_owned(),
        }),
        &rows,
    );
    assert_eq!(quick_look_view(&effects), Some(None), "a new query");

    let _ = settle(&mut focus, effects, &rows);
    let _ = run(&mut focus, CommandId::QuickLook, &rows);
    let effects = run(&mut focus, CommandId::HistoryBack, &rows);
    assert!(!focus.in_results());
    assert_eq!(quick_look_view(&effects), Some(None), "back to the inbox");
}

#[test]
fn an_answer_for_a_result_since_left_is_dropped() {
    let rows = List::of(3);
    let mut focus = mac();
    let _ = search(&mut focus, "atlas budget", &rows);
    let first = run(&mut focus, CommandId::QuickLook, &rows);
    let second = run(&mut focus, CommandId::NextMessage, &rows);
    let mut late = Vec::new();
    for (ticket, request) in asks(&first) {
        if let Some(reply) = reply_to(&request) {
            late.extend(focus.handle_on(Input::Reply(ticket, reply), &rows));
        }
    }
    assert_eq!(quick_look_view(&late), None, "nothing redrawn for it");
    let effects = settle(&mut focus, second, &rows);
    assert_eq!(text(&looked(&effects).subject), "Atlas budget, part 2");
}

fn labels(hints: &[postio_ui::hints::Hint]) -> Vec<&str> {
    hints.iter().map(|hint| hint.label.as_str()).collect()
}

#[test]
fn x_checks_results_and_the_footer_becomes_the_bulk_bar() {
    let rows = List::of(3);
    let mut focus = mac();
    let effects = search(&mut focus, "atlas budget", &rows);
    let view = results_view(&effects).expect("the results");
    assert_eq!(view.selected, 0);
    assert!(view.bulk.is_empty());
    assert_eq!(view.select_all, None, "nothing checked, no bulk bar");

    let effects = run(&mut focus, CommandId::ToggleSelection, &rows);
    let view = results_view(&effects).expect("redrawn");
    assert_eq!(view.selected, 1);
    assert_eq!(
        labels(&view.bulk),
        ["Archive", "Snooze", "Mark read", "Label", "Move", "Delete"]
    );
    let all = view.select_all.expect("`* a` on the bar's right");
    assert_eq!(
        all.label, "select all 120",
        "every conversation, not every row"
    );
    assert_eq!(all.key, "* a");

    let _ = run(&mut focus, CommandId::NextMessage, &rows);
    let effects = run(&mut focus, CommandId::ToggleSelection, &rows);
    assert_eq!(results_view(&effects).expect("redrawn").selected, 2);
}

#[test]
fn capital_x_checks_every_conversation_the_query_matches() {
    let rows = List::of(3);
    let mut focus = mac();
    let _ = search(&mut focus, "atlas budget", &rows);
    let effects = run(&mut focus, CommandId::SelectAll, &rows);
    let view = results_view(&effects).expect("redrawn");
    assert_eq!(view.selected, 120, "the whole match, not the rows read");
    assert_eq!(view.select_all, None, "everything is selected already");
    assert!(!view.bulk.is_empty());
    assert!(
        shown(&effects).contains(&Intent::ResultsPage {
            first: 0,
            count: view.rows
        }),
        "every row redraws checked"
    );
    assert_eq!(focus.result_row(0).map(|row| row.checked), Some(true));
    assert_eq!(focus.result_row(40).map(|row| row.checked), Some(true));

    let effects = run(&mut focus, CommandId::ToggleSelection, &rows);
    let view = results_view(&effects).expect("redrawn");
    assert_eq!(view.selected, 119);
    let best = focus.result_row(0).expect("a row").message;
    assert_eq!(focus.result_row(0).map(|row| row.checked), Some(false));
    let again = (3..focus.result_count())
        .find(|at| focus.result_row(*at).is_some_and(|row| row.message == best))
        .expect("the conversation in its month");
    assert_eq!(focus.result_row(again).map(|row| row.checked), Some(false));
    assert!(view.select_all.is_some(), "not everything any more");
}

#[test]
fn a_verb_on_everything_checked_aims_at_the_query_as_a_predicate() {
    let rows = List::of(3);
    let mut focus = mac();
    let _ = search(&mut focus, "atlas budget", &rows);
    let _ = run(&mut focus, CommandId::SelectAll, &rows);
    let _ = run(&mut focus, CommandId::ToggleSelection, &rows);
    let effects = run(&mut focus, CommandId::Archive, &rows);

    let sent: Vec<Request> = asked(&effects)
        .into_iter()
        .filter(|request| matches!(request, Request::Send { .. }))
        .collect();
    let [
        Request::Send {
            command: postio_core::Command::Archive { .. },
            aims,
            everything: Some(everything),
        },
    ] = sent.as_slice()
    else {
        panic!("one archive, at a predicate: {sent:?}");
    };
    assert!(aims.is_empty(), "no rows named: {aims:?}");
    assert_eq!(
        everything.query.as_ref().map(|query| query.input()),
        Some("atlas budget"),
        "every conversation the query matches"
    );
    assert_eq!(
        everything.except,
        [MessageId::new(1007)],
        "but the one taken back"
    );
    let view = results_view(&effects).expect("redrawn");
    assert_eq!(view.selected, 0, "what was checked has been acted on");
    assert!(view.bulk.is_empty());
}

#[test]
fn a_verb_on_checked_rows_aims_at_their_conversations() {
    let rows = List::of(3);
    let mut focus = mac();
    let _ = search(&mut focus, "atlas budget", &rows);
    let _ = run(&mut focus, CommandId::ToggleSelection, &rows);
    let _ = run(&mut focus, CommandId::NextMessage, &rows);
    let _ = run(&mut focus, CommandId::ToggleSelection, &rows);
    let _ = run(&mut focus, CommandId::NextMessage, &rows);
    let effects = run(&mut focus, CommandId::ToggleRead, &rows);

    let sent: Vec<Request> = asked(&effects)
        .into_iter()
        .filter(|request| matches!(request, Request::Send { .. }))
        .collect();
    assert_eq!(
        sent,
        [Request::Send {
            command: postio_core::Command::ToggleRead {
                target: postio_core::MessageTarget::Selection,
                unread: None,
            },
            aims: vec![postio_core::MessageTarget::Threads(vec![
                postio_model::ThreadId::new(2007),
                postio_model::ThreadId::new(2002),
            ])],
            everything: None,
        }],
        "the two checked, not the focused third"
    );
    assert_eq!(results_view(&effects).expect("redrawn").selected, 0);
}

#[test]
fn a_picker_in_the_results_opens_over_what_is_checked() {
    let rows = List::of(3);
    let mut focus = mac();
    let _ = search(&mut focus, "atlas budget", &rows);
    let _ = run(&mut focus, CommandId::ToggleSelection, &rows);
    let _ = run(&mut focus, CommandId::NextMessage, &rows);
    let _ = run(&mut focus, CommandId::ToggleSelection, &rows);
    let effects = run(&mut focus, CommandId::Move, &rows);
    let opened = shown(&effects)
        .into_iter()
        .find_map(|intent| match intent {
            Intent::OpenPicker(view) => Some(view),
            _ => None,
        })
        .expect("m opens the move picker");
    assert_eq!(opened.anchor, postio_focus::Anchor::Result(1));
    assert_eq!(opened.target, "2 threads");
    assert!(
        asked(&effects)
            .iter()
            .any(|request| matches!(request, Request::Folders { .. })),
        "its folders are read"
    );
}

#[test]
fn the_results_are_asked_again_once_a_verb_on_them_lands() {
    let rows = List::of(3);
    let mut focus = mac();
    let _ = search(&mut focus, "atlas budget", &rows);
    let _ = run(&mut focus, CommandId::Archive, &rows);
    let effects = focus.handle_on(
        Input::Event(postio_core::Event::ActionCompleted {
            description: "Archived".to_owned(),
            undoable: true,
        }),
        &rows,
    );
    assert_eq!(
        results_reads(&effects),
        [
            (ConversationOrder::BestMatch, 0, 3),
            (ConversationOrder::Newest, 0, 50)
        ],
        "an archived result is in another folder now: the rows say so"
    );
    let _ = settle(&mut focus, effects, &rows);
    assert_eq!(focus.result_count(), 123, "and the ring stays where it was");
}

#[test]
fn the_results_are_asked_again_once_a_message_is_read_under_them() {
    let rows = List::of(3);
    let mut focus = mac();
    let _ = search(&mut focus, "atlas budget", &rows);
    let effects = focus.handle_on(
        Input::Event(postio_core::Event::MessagesChanged {
            account: postio_model::AccountId::new(1),
            messages: vec![postio_model::MessageId::new(5)],
        }),
        &rows,
    );
    assert_eq!(
        results_reads(&effects),
        [
            (ConversationOrder::BestMatch, 0, 3),
            (ConversationOrder::Newest, 0, 50)
        ],
        "the results are read again"
    );
}

#[test]
fn a_burst_of_changes_reads_the_results_again_once_more_not_once_each() {
    let rows = List::of(3);
    let mut focus = mac();
    let _ = search(&mut focus, "atlas budget", &rows);
    let changed = |message: i64| {
        Input::Event(postio_core::Event::MessagesChanged {
            account: postio_model::AccountId::new(1),
            messages: vec![postio_model::MessageId::new(message)],
        })
    };
    let first = focus.handle_on(changed(5), &rows);
    assert_eq!(
        results_reads(&first).len(),
        2,
        "the first change reads them again"
    );
    for message in [6, 7] {
        let more = focus.handle_on(changed(message), &rows);
        assert_eq!(
            results_reads(&more),
            [],
            "a change while a read is out waits for it"
        );
    }
    let landed = settle(&mut focus, first, &rows);
    assert_eq!(
        results_reads(&landed),
        [
            (ConversationOrder::BestMatch, 0, 3),
            (ConversationOrder::Newest, 0, 50),
            (ConversationOrder::BestMatch, 0, 3),
            (ConversationOrder::Newest, 0, 50)
        ],
        "one read for the first change, one for the two behind it"
    );
    assert_eq!(focus.result_count(), 123, "and the results are all there");
}

fn screen_twelve(focus: &mut FocusController, rows: &List) {
    let _ = search(focus, "atlas budget", rows);
    for (field, value) in [("from", "ada@example.com"), ("after", "2026-07-01")] {
        let effects = focus.handle_on(
            Input::SearchEdit(TermEdit::Add {
                field: field.to_owned(),
                value: value.to_owned(),
                negated: false,
            }),
            rows,
        );
        let _ = settle(focus, effects, rows);
    }
}

fn save_popover(effects: &[Effect]) -> Option<Option<postio_focus::SaveView>> {
    shown(effects)
        .into_iter()
        .rev()
        .find_map(|intent| match intent {
            Intent::SavePopover(view) => Some(view.map(|view| *view)),
            _ => None,
        })
}

#[test]
fn cmd_s_opens_the_save_popover_named_from_the_query() {
    let rows = List::of(3);
    let mut focus = mac();
    let saved = |key: &str| postio_ui::saved_search::SavedSearch {
        key: key.to_owned(),
        name: key.to_owned(),
        query: format!("subject:{key}"),
        notify: false,
    };
    let _ = focus.handle(Input::SavedSearches(vec![saved("a"), saved("b")]));
    screen_twelve(&mut focus, &rows);
    let effects = run(&mut focus, CommandId::SaveSearch, &rows);
    let view = save_popover(&effects)
        .flatten()
        .expect("⌘S opens the Save popover");
    assert_eq!(view.title, "Save as a saved search");
    assert_eq!(view.name, "Atlas budget from Ada");
    assert_eq!(
        view.chips,
        ["from:Ada Moreno", "after:2026-07-01", "atlas budget"],
        "the terms, read-only, a person by name"
    );
    assert!(view.pin, "pinned unless said");
    assert_eq!(view.pin_note, "Appears at the top of search as");
    assert_eq!(
        view.pin_key.as_deref(),
        Some("alt+3"),
        "the next free number"
    );
    assert!(!view.notify);
    assert_eq!(view.notify_note, "A quiet badge, not a banner");
    assert!(!view.rolling);
    assert_eq!(
        view.rolling_note.as_deref(),
        Some("Off: always since 1 July. On: always the last 87 days"),
        "what each way means for this query's date"
    );
    assert!(
        asked(&effects).is_empty(),
        "nothing is written until Save: {:?}",
        asked(&effects)
    );
}

#[test]
fn saving_from_the_popover_writes_what_it_says_and_a_notify_search_starts_seen() {
    let rows = List::of(3);
    let mut focus = mac();
    screen_twelve(&mut focus, &rows);
    let _ = run(&mut focus, CommandId::SaveSearch, &rows);
    let effects = focus.handle_on(
        Input::SaveSearchAs {
            name: "Atlas from Ada".to_owned(),
            pin: true,
            notify: true,
            rolling: true,
        },
        &rows,
    );
    assert_eq!(save_popover(&effects), Some(None), "the popover goes");
    let saving = shown(&effects)
        .into_iter()
        .find_map(|intent| match intent {
            Intent::SaveSearch(save) => Some(save),
            _ => None,
        })
        .expect("the save is the frontend's to write");
    assert_eq!(
        saving,
        postio_focus::SaveSearch {
            query: "atlas budget from:ada@example.com after:2026-07-01".to_owned(),
            name: Some("Atlas from Ada".to_owned()),
            pin: true,
            notify: true,
            dates: postio_ui::saved_search::Dates::Rolling {
                today: today().date_naive()
            },
        }
    );

    let written = postio_ui::saved_search::SavedSearch {
        key: "atlas-budget-from-ada-example-com-after-2026-07-01".to_owned(),
        name: "Atlas from Ada".to_owned(),
        query: "atlas budget from:ada@example.com after:87d".to_owned(),
        notify: true,
    };
    let effects = focus.handle_on(
        Input::SearchSaved(Ok(postio_focus::SearchesSaved {
            searches: vec![written.clone()],
            key: Some(written.key.clone()),
        })),
        &rows,
    );
    assert!(shown(&effects).iter().any(|intent| matches!(
        intent,
        Intent::Toast { text, .. } if text.contains("Atlas from Ada")
    )));
    assert!(
        asked(&effects).contains(&Request::MarkSeen { key: written.key }),
        "its badge counts from the moment it was saved"
    );
}

const NOTHING: &str = "from:ada@example.com has:attachment before:2026-03-01 subject:\"budget v4\"";

fn ways_out_asked(effects: &[Effect]) -> Option<(postio_focus::Ticket, String, u64)> {
    asks(effects)
        .into_iter()
        .find_map(|(ticket, request)| match request {
            Request::Relaxations { query, stamp, .. } => {
                let raw: Vec<String> = query
                    .tokens()
                    .iter()
                    .map(|token| token.raw.clone())
                    .collect();
                Some((ticket, raw.join(" "), stamp))
            }
            _ => None,
        })
}

fn ways_out(query: &str, counts: &[u64]) -> Vec<(postio_search::relax::Relaxation, Counted)> {
    let exact: Vec<Counted> = counts
        .iter()
        .map(|&conversations| Counted {
            conversations,
            capped: false,
        })
        .collect();
    ways_out_counted(query, &exact)
}

fn ways_out_counted(
    query: &str,
    counts: &[Counted],
) -> Vec<(postio_search::relax::Relaxation, Counted)> {
    let parsed = postio_search::parse(query, today().date_naive());
    let offered = postio_search::relax::relax(&parsed);
    assert_eq!(offered.len(), counts.len(), "{offered:?}");
    offered.into_iter().zip(counts.iter().copied()).collect()
}

fn nothing_found(
    focus: &mut FocusController,
    query: &str,
    counts: &[u64],
    rows: &List,
) -> Vec<Effect> {
    let exact: Vec<Counted> = counts
        .iter()
        .map(|&conversations| Counted {
            conversations,
            capped: false,
        })
        .collect();
    nothing_found_counted(focus, query, &exact, rows)
}

fn nothing_found_counted(
    focus: &mut FocusController,
    query: &str,
    counts: &[Counted],
    rows: &List,
) -> Vec<Effect> {
    let effects = search(focus, query, rows);
    let (ticket, _, stamp) = ways_out_asked(&effects).expect("the ways out are asked for");
    focus.handle_on(
        Input::Reply(
            ticket,
            postio_focus::Reply::Relaxations {
                stamp,
                answer: Ok(ways_out_counted(query, counts)),
            },
        ),
        rows,
    )
}

#[test]
fn a_way_out_counted_past_its_cap_is_drawn_as_a_floor() {
    let rows = List::of(3);
    let mut focus = mac();
    let floor = |conversations| Counted {
        conversations,
        capped: true,
    };
    let exact = |conversations| Counted {
        conversations,
        capped: false,
    };
    let effects = nothing_found_counted(
        &mut focus,
        NOTHING,
        &[floor(10_000), exact(1), exact(0), exact(4), exact(0)],
        &rows,
    );
    let page = no_results_view(&effects).expect("drawn").expect("up");
    let counts: Vec<&str> = page
        .relaxations
        .iter()
        .map(|way| way.count.as_str())
        .collect();
    assert_eq!(counts, ["10,000+ threads", "4 threads", "1 thread"]);
}

#[test]
fn nothing_found_shows_the_page_at_once_and_asks_for_the_ways_out() {
    let rows = List::of(3);
    let mut focus = mac();
    let effects = search(&mut focus, NOTHING, &rows);

    let view = results_view(&effects).expect("the results");
    assert_eq!(view.rows, 0);
    assert_eq!(view.footer, "Searched 18,204 messages · 41 ms");
    assert_eq!(
        labels(&view.hints),
        ["clear filters"],
        "nothing to loosen by number until the counts land"
    );
    let page = no_results_view(&effects)
        .expect("the page")
        .expect("drawn, not taken away");
    assert_eq!(page.title, "Nothing matches all four filters");
    assert!(page.body.starts_with("Each line below loosens one filter"));
    assert!(page.relaxations.is_empty());
    assert_eq!(page.counting.as_deref(), Some("Counting looser searches…"));
    assert_eq!(
        page.searched,
        "Searched all 18,204 messages on this Mac, including attachment contents."
    );
    let (_, query, _) = ways_out_asked(&effects).expect("the ways out are asked for");
    assert_eq!(query, NOTHING);
    let field = query_view(&effects).expect("the field");
    assert_eq!(field.hint, "clears filters", "D24");
    assert_eq!(field.hint_key.as_deref(), Some("cmd+BackSpace"));
    assert!(field.ringed);
    assert!(focus.answers(CommandId::BackToWords));
}

#[test]
fn the_ways_out_are_most_first_none_empty_and_at_most_four() {
    let rows = List::of(3);
    let mut focus = mac();
    let query =
        "from:ada@example.com has:attachment before:2026-03-01 subject:\"budget v4\" atlas plan";
    let effects = nothing_found(&mut focus, query, &[4, 1, 6, 0, 2, 3, 5], &rows);

    let page = no_results_view(&effects).expect("drawn").expect("up");
    assert_eq!(page.title, "Nothing matches all six filters");
    assert_eq!(page.counting, None);
    let shown: Vec<(u32, &str, &str, &str, bool)> = page
        .relaxations
        .iter()
        .map(|way| {
            (
                way.number,
                way.label.as_str(),
                way.query.as_str(),
                way.count.as_str(),
                way.focused,
            )
        })
        .collect();
    assert_eq!(
        shown,
        [
            (
                1,
                "Remove “before March”",
                "from:ada@example.com has:attachment subject:\"budget v4\" atlas plan",
                "6 threads",
                true
            ),
            (
                2,
                "Remove “plan”",
                "from:ada@example.com has:attachment before:2026-03-01 subject:\"budget v4\" atlas",
                "5 threads",
                false
            ),
            (
                3,
                "Anyone, not just Ada Moreno",
                "has:attachment before:2026-03-01 subject:\"budget v4\" atlas plan",
                "4 threads",
                false
            ),
            (
                4,
                "Remove “atlas”",
                "from:ada@example.com has:attachment before:2026-03-01 subject:\"budget v4\" plan",
                "3 threads",
                false
            ),
        ],
        "most first, none of 0, four at most"
    );
    assert_eq!(page.relaxations[0].key.as_deref(), Some("1"));
    let view = results_view(&effects).expect("the footer, again");
    assert_eq!(labels(&view.hints), ["loosen a filter", "clear filters"]);
    assert_eq!(view.hints[0].key, "1\u{2013}4");

    let field = query_view(&effects).expect("the field");
    let ringed: Vec<&str> = field
        .chips
        .iter()
        .filter(|chip| chip.focused)
        .map(|chip| chip.operator.as_str())
        .collect();
    assert_eq!(ringed, ["before:"]);

    let effects = run(&mut focus, CommandId::NextMessage, &rows);
    let page = no_results_view(&effects).expect("drawn").expect("up");
    assert!(page.relaxations[1].focused && !page.relaxations[0].focused);
    let field = query_view(&effects).expect("the field");
    assert!(field.chips.iter().all(|chip| !chip.focused));
    let effects = run(&mut focus, CommandId::NextMessage, &rows);
    let field = query_view(&effects).expect("the field");
    let ringed: Vec<&str> = field
        .chips
        .iter()
        .filter(|chip| chip.focused)
        .map(|chip| chip.operator.as_str())
        .collect();
    assert_eq!(ringed, ["from:"]);
}

#[test]
fn a_number_runs_its_way_out_and_return_runs_the_focused_one() {
    let rows = List::of(3);
    let mut focus = mac();
    nothing_found(&mut focus, NOTHING, &[0, 0, 0, 7, 3], &rows);

    assert!(
        run(&mut focus, CommandId::PickRelaxation3, &rows).is_empty(),
        "no third way out"
    );
    let effects = run(&mut focus, CommandId::PickRelaxation2, &rows);
    assert_eq!(
        queries_asked(&effects).first().map(String::as_str),
        Some("from:ada@example.com has:attachment before:2026-03-01 \"budget v4\""),
        "the second way out, run"
    );
    assert_eq!(no_results_view(&effects), Some(None), "the page goes");
    let field = query_view(&effects).expect("the field");
    assert_eq!(field.hint, "/ to edit");
    assert_eq!(field.hint_key, None);
    assert!(!field.ringed, "found something: the field rests");

    let mut focus = mac();
    nothing_found(&mut focus, NOTHING, &[0, 0, 0, 7, 3], &rows);
    let effects = run(&mut focus, CommandId::OpenMessage, &rows);
    assert_eq!(
        queries_asked(&effects).first().map(String::as_str),
        Some("from:ada@example.com has:attachment before:2026-03-01"),
        "Return runs the focused one, the first"
    );
}

#[test]
fn cmd_backspace_with_nothing_found_clears_the_filters_and_keeps_the_words() {
    let rows = List::of(3);
    let mut focus = mac();
    let query = "notes from:ada@example.com subject:v4";
    let effects = search(&mut focus, query, &rows);
    assert!(no_results_view(&effects).flatten().is_some());

    let effects = run(&mut focus, CommandId::BackToWords, &rows);
    let field = query_view(&effects).expect("the field");
    assert!(field.chips.is_empty(), "{field:?}");
    assert_eq!(field.words, "notes");
    assert_eq!(
        queries_asked(&effects).first().map(String::as_str),
        Some("notes")
    );
    assert_eq!(no_results_view(&effects), Some(None));

    let effects = settle(&mut focus, effects, &rows);
    assert!(results_view(&effects).is_some_and(|view| view.rows > 0));
    assert!(!focus.answers(CommandId::BackToWords));
}

#[test]
fn ways_out_for_a_search_since_changed_are_dropped() {
    let rows = List::of(3);
    let mut focus = mac();
    let effects = search(&mut focus, NOTHING, &rows);
    let (ticket, _, stamp) = ways_out_asked(&effects).expect("asked");
    let cleared = focus.handle_on(Input::SearchEdit(TermEdit::ClearFilters), &rows);
    let _ = settle(&mut focus, cleared, &rows);
    let effects = focus.handle_on(
        Input::Reply(
            ticket,
            postio_focus::Reply::Relaxations {
                stamp,
                answer: Ok(ways_out(NOTHING, &[1, 2, 3, 4, 5])),
            },
        ),
        &rows,
    );
    assert_eq!(no_results_view(&effects), None, "{:?}", shown(&effects));
    assert!(run(&mut focus, CommandId::PickRelaxation1, &rows).is_empty());
}

#[test]
fn no_way_out_says_so_rather_than_counting_forever() {
    let rows = List::of(3);
    let mut focus = mac();
    let effects = nothing_found(&mut focus, NOTHING, &[0, 0, 0, 0, 0], &rows);
    let page = no_results_view(&effects).expect("drawn").expect("up");
    assert!(page.relaxations.is_empty());
    assert_eq!(page.counting, None);
    assert!(
        page.body
            .starts_with("Loosening any one of them still finds nothing")
    );

    let mut focus = mac();
    let effects = search(&mut focus, NOTHING, &rows);
    let (ticket, _, stamp) = ways_out_asked(&effects).expect("asked");
    let effects = focus.handle_on(
        Input::Reply(
            ticket,
            postio_focus::Reply::Relaxations {
                stamp,
                answer: Err("the index is closed".to_owned()),
            },
        ),
        &rows,
    );
    let page = no_results_view(&effects).expect("drawn").expect("up");
    assert_eq!(page.counting, None, "a failed count stops counting");
}

#[test]
fn the_ways_out_are_a_lane_of_their_own() {
    let rows = List::of(3);
    let mut focus = mac();
    let effects = search(&mut focus, NOTHING, &rows);
    let request = asks(&effects)
        .into_iter()
        .map(|(_, request)| request)
        .find(|request| matches!(request, Request::Relaxations { .. }))
        .expect("asked");
    assert_eq!(request.lane(), Some(postio_focus::Lane::Relaxations));
}

fn files_asked(effects: &[Effect]) -> usize {
    asked(effects)
        .iter()
        .filter(|request| matches!(request, Request::Files { .. }))
        .count()
}

fn copies_asked(
    effects: &[Effect],
) -> Vec<(postio_model::AttachmentId, postio_focus::FilePurpose)> {
    asked(effects)
        .into_iter()
        .filter_map(|request| match request {
            Request::AttachmentCopy {
                attachment,
                purpose,
                ..
            } => Some((attachment, purpose)),
            _ => None,
        })
        .collect()
}

fn copies_shown(effects: &[Effect]) -> Vec<Option<postio_focus::FileCopy>> {
    shown(effects)
        .into_iter()
        .filter_map(|intent| match intent {
            Intent::FileCopy(copy) => Some(copy),
            _ => None,
        })
        .collect()
}

fn removed(effects: &[Effect]) -> Vec<std::path::PathBuf> {
    asked(effects)
        .into_iter()
        .filter_map(|request| match request {
            Request::RemoveCopy(path) => Some(path),
            _ => None,
        })
        .collect()
}

fn on_the_files_tab(focus: &mut FocusController, rows: &List) -> Vec<Effect> {
    let _ = search(focus, "atlas budget", rows);
    let effects = run(focus, CommandId::ResultsFiles, rows);
    assert_eq!(
        files_asked(&effects),
        1,
        "the cards are read when the tab opens"
    );
    settle(focus, effects, rows)
}

#[test]
fn the_files_tab_reads_its_cards_and_draws_them() {
    let rows = List::of(3);
    let mut focus = mac();
    let effects = on_the_files_tab(&mut focus, &rows);
    let view = results_view(&effects).expect("the frame, with the cards counted");
    assert_eq!(view.rows, 3);
    assert_eq!(view.cursor, Some(0), "the first card ringed");
    assert!(view.groups.is_empty(), "a grid, not month groups");
    assert_eq!(view.order, ConversationOrder::Newest);
    assert!(!view.sortable);
    assert_eq!(view.tabs[1].count, "3", "the tab counts the cards");
    assert!(view.tabs[1].selected);
    assert_eq!(view.count_line, "3 files");
    assert_eq!(view.footer, "3 files \u{b7} local index \u{b7} 41 ms");
    let header = view.files.as_ref().expect("the Files tab's header");
    assert_eq!(header.title, "Files whose name or contents match");
    assert!(
        header.note.starts_with("contents are indexed on this Mac"),
        "{}",
        header.note
    );
    assert_eq!(
        view.hints
            .iter()
            .map(|hint| hint.label.as_str())
            .collect::<Vec<_>>(),
        [
            "move",
            "Quick Look",
            "open the message",
            "save file",
            "switch tab"
        ]
    );

    let card = focus.result_file(0).expect("the first card");
    assert_eq!(card.kind, "XLSX");
    assert_eq!(card.preview, postio_focus::FilePreview::Sheet);
    assert_eq!(text(&card.name), "Atlas-Q3-budget.xlsx");
    assert_eq!(lit(&card.name), ["Atlas", "budget"]);
    assert_eq!(
        text(&card.line),
        "Sheet \u{2018}Q3\u{2019}, row 3: Total Atlas budget 1,240,000"
    );
    assert_eq!(lit(&card.line), ["Atlas", "budget"]);
    assert_eq!(
        card.subject,
        "in \u{2018}Re: Atlas Q3 budget, final numbers\u{2019}"
    );
    assert!(card.focused);
    let pdf = focus.result_file(1).expect("the second");
    assert_eq!(pdf.kind, "PDF");
    assert_eq!(pdf.preview, postio_focus::FilePreview::Page);
    assert_eq!(pdf.meta, "Ada Moreno \u{b7} 4 Sep \u{b7} 212 KB");
    assert!(text(&pdf.line).starts_with("Page 2: "));
    let named = focus.result_file(2).expect("the third");
    assert!(
        named.line.is_empty(),
        "matched by its name only: no contents line"
    );
    assert_eq!(focus.result_file(3), None);
}

#[test]
fn space_on_a_card_says_what_the_platform_does_with_it() {
    let rows = List::of(3);
    let mut linux = FocusController::new(Policy::for_platform(Platform::Freedesktop));
    let _ = linux.handle(Input::Clock(Some(today())));
    let effects = on_the_files_tab(&mut linux, &rows);
    let view = results_view(&effects).expect("the cards");
    let space: Vec<&str> = view
        .hints
        .iter()
        .filter(|hint| hint.key == "space")
        .map(|hint| hint.label.as_str())
        .collect();
    assert_eq!(space, ["open the file"]);
    let effects = run(&mut linux, CommandId::ResultsConversations, &rows);
    let view = results_view(&effects).expect("the threads");
    assert!(
        view.hints
            .iter()
            .any(|hint| hint.key == "space" && hint.label == "Quick Look"),
        "{:?}",
        view.hints
    );
}

#[test]
fn the_keys_move_the_ring_over_the_cards() {
    let rows = List::of(3);
    let mut focus = mac();
    let _ = on_the_files_tab(&mut focus, &rows);
    let effects = run(&mut focus, CommandId::NextMessage, &rows);
    assert!(shown(&effects).contains(&Intent::ResultsCursor(1)));
    let effects = focus.handle_on(Input::ResultsPoint(2), &rows);
    assert!(shown(&effects).contains(&Intent::ResultsCursor(2)));
    assert!(focus.result_file(2).is_some_and(|card| card.focused));
    let effects = run(&mut focus, CommandId::PrevMessage, &rows);
    assert!(shown(&effects).contains(&Intent::ResultsCursor(1)));
    let effects = run(&mut focus, CommandId::ResultsConversations, &rows);
    assert_eq!(results_view(&effects).and_then(|view| view.cursor), Some(0));
}

#[test]
fn space_hands_a_copy_of_the_file_to_quick_look_and_removes_it_after() {
    let rows = List::of(3);
    let mut focus = mac();
    let _ = on_the_files_tab(&mut focus, &rows);
    let effects = run(&mut focus, CommandId::QuickLook, &rows);
    assert_eq!(
        copies_asked(&effects),
        [(
            postio_model::AttachmentId::new(71),
            postio_focus::FilePurpose::Preview
        )]
    );
    assert!(
        quick_look_view(&effects).is_none(),
        "not the results' own Quick Look: the system's, on the file"
    );
    let effects = settle(&mut focus, effects, &rows);
    let copy = copy_of(postio_model::AttachmentId::new(71));
    assert_eq!(
        copies_shown(&effects),
        [Some(postio_focus::FileCopy {
            path: copy.clone(),
            name: "Atlas-Q3-budget.xlsx".to_owned(),
            purpose: postio_focus::FilePurpose::Preview,
        })]
    );

    let effects = run(&mut focus, CommandId::QuickLook, &rows);
    assert_eq!(copies_shown(&effects), [None]);
    assert_eq!(removed(&effects), std::slice::from_ref(&copy));

    let asked_for = run(&mut focus, CommandId::QuickLook, &rows);
    let effects = settle(&mut focus, asked_for, &rows);
    assert_eq!(copies_shown(&effects).len(), 1);
    let effects = focus.handle_on(Input::FileCopyDone, &rows);
    assert_eq!(removed(&effects), [copy]);
    assert!(
        copies_shown(&effects).is_empty(),
        "the panel is already gone"
    );
}

#[test]
fn command_down_saves_the_file_and_return_opens_its_message() {
    let rows = List::of(3);
    let mut focus = mac();
    let _ = on_the_files_tab(&mut focus, &rows);
    let effects = run(&mut focus, CommandId::SaveFile, &rows);
    assert_eq!(
        copies_asked(&effects),
        [(
            postio_model::AttachmentId::new(71),
            postio_focus::FilePurpose::Save
        )]
    );
    let effects = settle(&mut focus, effects, &rows);
    assert!(matches!(
        copies_shown(&effects).as_slice(),
        [Some(postio_focus::FileCopy {
            purpose: postio_focus::FilePurpose::Save,
            ..
        })]
    ));
    let effects = focus.handle_on(Input::FileCopyDone, &rows);
    assert_eq!(removed(&effects).len(), 1);

    let _ = focus.handle_on(Input::ResultsPoint(1), &rows);
    let effects = run(&mut focus, CommandId::OpenMessage, &rows);
    assert!(
        shown(&effects).iter().any(|intent| matches!(
            intent,
            Intent::OpenMessage { message, .. } if *message == MessageId::new(1030)
        )),
        "{:?}",
        shown(&effects)
    );
}

#[test]
fn a_file_not_on_this_mac_is_not_previewed_and_says_so() {
    let rows = List::of(3);
    let mut focus = mac();
    let _ = on_the_files_tab(&mut focus, &rows);
    let effects = run(&mut focus, CommandId::QuickLook, &rows);
    let (ticket, request) = asks(&effects)
        .into_iter()
        .find(|(_, request)| matches!(request, Request::AttachmentCopy { .. }))
        .expect("a copy asked for");
    let Request::AttachmentCopy { stamp, purpose, .. } = request else {
        unreachable!()
    };
    let effects = focus.handle_on(
        Input::Reply(
            ticket,
            postio_focus::Reply::AttachmentCopy {
                stamp,
                purpose,
                answer: Ok(None),
            },
        ),
        &rows,
    );
    assert!(copies_shown(&effects).is_empty());
    assert!(
        shown(&effects)
            .iter()
            .any(|intent| matches!(intent, Intent::Toast { .. })),
        "{:?}",
        shown(&effects)
    );
}

#[test]
fn a_new_query_reads_the_cards_again_and_leaving_removes_a_copy() {
    let rows = List::of(3);
    let mut focus = mac();
    let _ = on_the_files_tab(&mut focus, &rows);
    let asked_for = run(&mut focus, CommandId::QuickLook, &rows);
    let effects = settle(&mut focus, asked_for, &rows);
    assert_eq!(copies_shown(&effects).len(), 1);
    let effects = focus.handle_on(
        Input::SearchEdit(TermEdit::Toggle {
            field: "has".to_owned(),
            value: "attachment".to_owned(),
        }),
        &rows,
    );
    assert_eq!(
        files_asked(&effects),
        1,
        "the tab shown reads its cards again"
    );
    assert_eq!(
        copies_shown(&effects),
        [None],
        "the preview goes with the query"
    );
    assert_eq!(removed(&effects).len(), 1);
}

fn people_asked(effects: &[Effect]) -> Vec<Request> {
    asked(effects)
        .into_iter()
        .filter(|request| matches!(request, Request::People { .. }))
        .collect()
}

fn on_the_people_tab(focus: &mut FocusController, rows: &List) -> Vec<Effect> {
    let _ = search(focus, "atlas", rows);
    let effects = run(focus, CommandId::ResultsPeople, rows);
    let asked = people_asked(&effects);
    assert_eq!(asked.len(), 1, "the rows are read when the tab opens");
    assert_eq!(asked[0].lane(), Some(postio_focus::Lane::People));
    settle(focus, effects, rows)
}

#[test]
fn the_people_tab_lists_who_the_results_are_from_and_to() {
    let rows = List::of(3);
    let mut focus = mac();
    let effects = on_the_people_tab(&mut focus, &rows);
    let view = results_view(&effects).expect("the frame, with the people counted");
    assert_eq!(view.rows, 2);
    assert_eq!(view.cursor, Some(0), "the first person ringed");
    assert!(view.groups.is_empty(), "a list, not month groups");
    assert_eq!(view.tabs[2].count, "2", "the tab counts the rows");
    assert!(
        view.sub_line.contains("2 people"),
        "the line under the count agrees with the tab: {}",
        view.sub_line
    );
    assert!(view.tabs[2].selected);
    assert_eq!(
        view.hints
            .iter()
            .map(|hint| hint.label.as_str())
            .collect::<Vec<_>>(),
        ["move", "search their mail", "switch tab"]
    );

    let ada = focus.result_person(0).expect("the first person");
    assert_eq!(ada.name, "Ada Moreno");
    assert_eq!(ada.address, "ada@example.com");
    assert_eq!(ada.initials, "AM");
    assert_eq!(ada.messages, "3 messages");
    assert_eq!(ada.last, "26 Sep");
    assert!(ada.focused);
    assert_eq!(
        ada.accessible,
        "Ada Moreno, ada@example.com, 3 messages, last 26 Sep"
    );
    let tomas = focus.result_person(1).expect("the second");
    assert_eq!(tomas.messages, "1 message");
    assert!(!tomas.focused);
    assert!(focus.result_person(2).is_none());
}

#[test]
fn return_on_a_person_searches_their_mail_on_the_conversations_tab() {
    let rows = List::of(3);
    let mut focus = mac();
    let _ = on_the_people_tab(&mut focus, &rows);
    let effects = run(&mut focus, CommandId::NextMessage, &rows);
    assert!(
        shown(&effects)
            .iter()
            .any(|intent| matches!(intent, Intent::ResultsCursor(1))),
        "{:?}",
        shown(&effects)
    );
    assert!(focus.result_person(1).expect("Tomás").focused);

    let effects = run(&mut focus, CommandId::OpenMessage, &rows);
    assert!(
        queries_asked(&effects)
            .iter()
            .all(|query| query == "from:tomas@example.com"),
        "{:?}",
        queries_asked(&effects)
    );
    assert!(!queries_asked(&effects).is_empty(), "their mail is read");
    let effects = settle(&mut focus, effects, &rows);
    let view = results_view(&effects).expect("the results again");
    assert!(view.tabs[0].selected, "on the Conversations tab");
    assert!(people_asked(&effects).is_empty(), "no People read there");
    let query = query_view(&effects).expect("the field");
    assert_eq!(query.chips.len(), 1, "the query is the person alone");

    let effects = run(&mut focus, CommandId::HistoryBack, &rows);
    assert_eq!(
        queries_asked(&effects).first().map(String::as_str),
        Some("atlas")
    );
    assert_eq!(people_asked(&effects).len(), 1);
    let effects = settle(&mut focus, effects, &rows);
    assert!(results_view(&effects).expect("the frame").tabs[2].selected);
}

#[test]
fn a_new_query_reads_the_people_again() {
    let rows = List::of(3);
    let mut focus = mac();
    let _ = on_the_people_tab(&mut focus, &rows);
    let effects = focus.handle_on(
        Input::SearchEdit(TermEdit::Toggle {
            field: "has".to_owned(),
            value: "attachment".to_owned(),
        }),
        &rows,
    );
    assert_eq!(people_asked(&effects).len(), 1, "the tab shown reads again");
}

fn settle_capped(focus: &mut FocusController, effects: Vec<Effect>, rows: &List) -> Vec<Effect> {
    use postio_focus::Reply;
    let cap = postio_search::results::CONVERSATION_WALK_CAP;
    let capped = |results: &mut postio_search::results::ConversationResults| {
        results.total = cap;
        results.capped = true;
        results.facets.capped = true;
    };
    let mut all = effects.clone();
    let mut waiting = effects;
    loop {
        let mut next = Vec::new();
        for (ticket, request) in asks(&waiting) {
            let Some(mut reply) = reply_to(&request) else {
                continue;
            };
            match &mut reply {
                Reply::ResultsPage {
                    answer: Ok(results),
                    ..
                }
                | Reply::Facets {
                    answer: Ok(results),
                    ..
                } => capped(results),
                _ => {}
            }
            next.extend(focus.handle_on(Input::Reply(ticket, reply), rows));
        }
        if next.is_empty() {
            return all;
        }
        all.extend(next.clone());
        waiting = next;
    }
}

#[test]
fn a_capped_answer_says_every_count_is_a_floor() {
    let rows = List::of(3);
    let mut focus = mac();
    let effects = show_all(&mut focus, "atlas budget", &rows);
    let effects = settle_capped(&mut focus, effects, &rows);
    let view = results_view(&effects).expect("the results");

    assert_eq!(view.count_line, "5,000+ threads");
    assert_eq!(view.footer, "5,000+ threads · local index · 41 ms");
    assert_eq!(
        view.tabs
            .iter()
            .map(|tab| tab.count.as_str())
            .collect::<Vec<_>>(),
        ["5,000+", "12+", "6+"],
        "the Files and People tabs count the same capped walk"
    );
    assert_eq!(view.sub_line, "12+ files · 6+ people · last 12 months");
    assert!(
        view.months
            .iter()
            .all(|month| month.count == format!("{}+", month.conversations)),
        "every month is a floor: {:?}",
        view.months
    );
    let months: Vec<&str> = view
        .groups
        .iter()
        .filter(|group| !group.top_hits)
        .map(|group| group.count.as_str())
        .collect();
    assert!(
        !months.is_empty() && months.iter().all(|count| count.ends_with('+')),
        "every month group's count is a floor: {months:?}"
    );

    let effects = focus.handle_on(Input::SearchPopover(FilterKind::From), &rows);
    let effects = settle_capped(&mut focus, effects, &rows);
    let from = popover_view(&effects).flatten().expect("the From popover");
    assert_eq!(
        from.rows
            .iter()
            .map(|row| (row.title.as_str(), row.count, row.count_label.as_str()))
            .collect::<Vec<_>>(),
        [("Ada Moreno", 24, "24+"), ("Tom\u{e1}s Reyes", 24, "24+")]
    );
}

#[test]
fn i_in_the_results_asks_about_the_focused_results_sender_and_a_click_on_a_name_too() {
    let rows = List::of(3);
    let mut focus = mac();
    screen_twelve(&mut focus, &rows);
    let row = focus.result_row(0).expect("the first result is read");
    let address = row.from_address.clone().expect("its sender's address");
    let effects = run(&mut focus, CommandId::ContactPopover, &rows);
    let asked: Vec<String> = effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Ask(_, postio_focus::Request::ContactGlance { address, .. }) => {
                Some(address.clone())
            }
            _ => None,
        })
        .collect();
    assert_eq!(asked, [address], "`i` on the focused result");
    assert_eq!(
        focus.key_context(),
        postio_ui::keymap::KeyContext::ContactPopover
    );
}

#[test]
fn a_result_says_where_a_digest_holds_it_and_its_account() {
    use postio_focus::{Input, PlacesRead, Reply};
    let rows = List::of(3);
    let mut focus = mac();
    let effects = run(&mut focus, CommandId::Search, &rows);
    let (ticket, _) = asks(&effects)
        .into_iter()
        .find(|(_, request)| *request == Request::Places)
        .expect("opening the bar reads the places");
    let inbox = postio_model::MailboxId::new(1);
    let _ = focus.handle_on(
        Input::Reply(
            ticket,
            Reply::Places(Ok(PlacesRead {
                folders: vec![(inbox, "Inbox".to_owned())],
                owners: vec![(inbox, "work@example.org".to_owned())],
                ..PlacesRead::default()
            })),
        ),
        &rows,
    );
    let _ = focus.handle_on(
        Input::Typed {
            text: "atlas budget".to_owned(),
        },
        &rows,
    );
    let held = MessageId::new(1007);
    let mut waiting = run(&mut focus, CommandId::ShowAllResults, &rows);
    while !waiting.is_empty() {
        let mut next = Vec::new();
        for (ticket, request) in asks(&waiting) {
            let Some(mut reply) = reply_to(&request) else {
                continue;
            };
            if let Reply::ResultsPage {
                answer: Ok(found), ..
            } = &mut reply
                && found.hits.iter().any(|hit| hit.best == held)
            {
                found.names.held = vec![(held, "Newsletters".to_owned(), false)];
            }
            next.extend(focus.handle_on(Input::Reply(ticket, reply), &rows));
        }
        waiting = next;
    }
    let drawn: Vec<postio_focus::ResultRow> =
        (0..20).filter_map(|at| focus.result_row(at)).collect();
    let place = |message: MessageId| {
        drawn
            .iter()
            .find(|row| row.message == message)
            .map(|row| row.folder.clone())
    };
    assert_eq!(
        place(held).as_deref(),
        Some("held \u{b7} Newsletters \u{b7} work@example.org"),
        "{drawn:?}"
    );
    assert_eq!(
        place(MessageId::new(1002)).as_deref(),
        Some("in:Inbox \u{b7} work@example.org")
    );
}

#[test]
fn a_new_search_after_history_back_from_files_is_not_found_nothing() {
    let rows = List::of(3);
    let mut focus = mac();
    let _ = search(&mut focus, "atlas budget", &rows);
    let effects = run(&mut focus, CommandId::ResultsFiles, &rows);
    let _ = settle(&mut focus, effects, &rows);
    let effects = run(&mut focus, CommandId::HistoryBack, &rows);
    let _ = settle(&mut focus, effects, &rows);
    assert!(!focus.in_results(), "back in the inbox");
    let effects = search(&mut focus, "atlas", &rows);
    let said: Vec<_> = shown(&effects)
        .into_iter()
        .filter_map(|intent| match intent {
            Intent::Relaxations(view) => Some(view.is_some()),
            _ => None,
        })
        .collect();
    assert!(
        !said.contains(&true),
        "a search that found something said it found nothing: {said:?}"
    );
}

#[test]
fn a_result_walks_each_conversation_once() {
    let rows = List::of(3);
    let mut focus = mac();
    let _ = search(&mut focus, "atlas budget", &rows);
    let conversations = focus.result_count();
    let drawn: Vec<MessageId> = (0..conversations)
        .filter_map(|at| focus.result_row(at))
        .map(|row| row.message)
        .collect();
    let mut once = drawn.clone();
    once.sort();
    once.dedup();
    assert!(
        once.len() < drawn.len(),
        "Top hits are drawn twice here: {drawn:?}"
    );
    let effects = run(&mut focus, CommandId::OpenMessage, &rows);
    let total = shown(&effects).iter().find_map(|intent| match intent {
        Intent::OpenMessage { total, .. } => Some(*total),
        _ => None,
    });
    assert_eq!(total, Some(once.len() as u32), "{:?}", shown(&effects));
}
