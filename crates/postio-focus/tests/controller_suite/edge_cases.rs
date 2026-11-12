//! The edge cases of spec 013 that no story owns (T085): a thread merged
//! under an expansion, a child with no body, an unsent reply opened from its
//! child, a list that lets go of its pages for a moment, the thread bar's
//! chips, a window whose thread cannot be walked, and a sync pass that
//! changes many threads.

use crate::search_support::{asks, shown};
use crate::threads::{
    List, answer, cursor_of, expanded, focus, member_id, read_asked, run, splice_of, unspliced,
    view, window_of,
};
use postio_core::{CommandId, Event};
use postio_focus::{ComposerKind, Effect, Input, Intent, Reply, Request, ToastKind};
use postio_model::{AccountId, DraftId, MessageId, ThreadId};
use postio_ui::thread_bar::Chip;

fn changed(thread: i64, added: &[i64]) -> Input {
    Input::Event(Event::ThreadChanged {
        account: AccountId::new(1),
        thread: ThreadId::new(thread),
        members_added: added.iter().map(|id| MessageId::new(*id)).collect(),
    })
}

// ---- A merge while expanded -------------------------------------------------

/// Harbor (thread 500, anchor message 100) is expanded when a late parent,
/// message 9001, links it to thread 501: 501 survives, and the one row that
/// stands for both is now message 9001's.
#[test]
fn a_merge_while_expanded_keeps_the_expansion_around_the_merged_thread() {
    let (mut focus, rows) = expanded(&[6, 4, 1, 3]);
    let survivor = ThreadId::new(501);

    // Only the survivor is announced. Whether Harbor is still its own
    // thread is a question for the store.
    let effects = focus.handle_on(changed(501, &[9001]), &rows);
    let (asked_ticket, _) = asks(&effects)
        .into_iter()
        .find(|(_, request)| *request == Request::ThreadOf(MessageId::new(100)))
        .expect("the anchor's thread is asked");

    // The list lands before the answer: the row the expansion hung from is
    // gone, and the merged row is at its place. Nothing folds yet.
    let merged = rows.with_row(0, MessageId::new(9001), survivor, 11);
    let effects = focus.landed(&merged, false);
    assert!(unspliced(&effects).is_empty(), "{effects:?}");

    // It is in 501 now: that thread is read.
    let effects = focus.handle_on(
        Input::Reply(
            asked_ticket,
            Reply::ThreadOf {
                message: MessageId::new(100),
                thread: Some(survivor),
            },
        ),
        &merged,
    );
    let (ticket, asked) = read_asked(&effects).expect("the merged thread is read");
    assert_eq!(asked, survivor);

    // Eleven messages, the newest the late parent, and the unsent reply.
    let mut whole = view(survivor, 11, 1, true);
    whole.members[10].id = MessageId::new(9001);
    let effects = answer(&mut focus, ticket, whole, &merged);
    let (anchor, children) = splice_of(&effects).expect("the children are drawn around it");
    assert_eq!((anchor, children.len()), (0, 12));
    assert!(unspliced(&effects).is_empty(), "{effects:?}");

    // And it is the expansion still: j walks into it, and a change to the
    // merged thread reads it again.
    let effects = run(&mut focus, CommandId::NextMessage, &merged);
    assert_eq!(cursor_of(&effects), Some(1), "the first child");
    let effects = focus.handle_on(changed(501, &[9002]), &merged);
    assert_eq!(
        read_asked(&effects).map(|(_, thread)| thread),
        Some(survivor)
    );
}

// ---- A child whose body is not here ----------------------------------------

/// A guard, green on arrival: a member the store has only a preview of (or
/// nothing) is a quiet placeholder, and the controller asks for no body.
#[test]
fn a_child_whose_body_is_not_local_shows_a_placeholder_and_asks_for_nothing() {
    let rows = List::of(&[6, 4]);
    let mut focus = focus();
    let _ = focus.open(postio_model::ListScope::Focus(
        postio_model::FocusScope::Inbox,
    ));
    let _ = focus.landed(&rows, true);
    let effects = run(&mut focus, CommandId::ExpandThread, &rows);
    let (ticket, thread) = read_asked(&effects).expect("a read");
    let mut whole = view(thread, 6, 0, false);
    whole.members[2].line = postio_model::thread_view::Line::Preview(String::new());
    let effects = answer(&mut focus, ticket, whole, &rows);
    let (_, children) = splice_of(&effects).expect("spliced");
    assert!(children[2].placeholder, "{:?}", children[2]);
    assert_eq!(children[2].line, postio_ui::thread_list::NO_PREVIEW);
    assert!(!children[1].placeholder, "its neighbours say their words");
    assert!(asks(&effects).is_empty(), "no fetch: {effects:?}");

    // Standing on it fetches nothing either.
    for _ in 0..3 {
        let effects = run(&mut focus, CommandId::NextMessage, &rows);
        assert!(asks(&effects).is_empty(), "{effects:?}");
    }
}

// ---- (a) Return on the draft child -----------------------------------------

#[test]
fn return_on_the_draft_child_opens_that_draft_in_the_composer() {
    let (mut focus, rows) = expanded(&[6, 4]);
    // Six members, then the unsent reply: display 7.
    for _ in 0..7 {
        let _ = run(&mut focus, CommandId::NextMessage, &rows);
    }
    assert_eq!(focus.cursor(), Some(7), "on the draft");
    let effects = run(&mut focus, CommandId::OpenMessage, &rows);
    let intents = shown(&effects);
    assert!(
        intents.contains(&Intent::Composer {
            kind: ComposerKind::ThreadDraft {
                draft: DraftId::new(7)
            },
            message: None,
        }),
        "{intents:?}"
    );
    assert!(
        !intents
            .iter()
            .any(|intent| matches!(intent, Intent::OpenMessage { .. })),
        "a draft is written, not read: {intents:?}"
    );
}

// ---- (b) A refresh that empties the landed pages ---------------------------

#[test]
fn an_expansion_survives_a_refresh_that_briefly_lets_go_of_the_landed_pages() {
    let (mut focus, rows) = expanded(&[6, 4, 1, 3]);
    // The refresh drops every page before it reads them again.
    let effects = focus.landed(&List::of(&[]), false);
    assert!(
        unspliced(&effects).is_empty(),
        "an absent page is not a page that lost the row: {effects:?}"
    );
    // The pages come back as they were: still expanded.
    let effects = focus.landed(&rows, false);
    assert!(unspliced(&effects).is_empty(), "{effects:?}");
    let effects = run(&mut focus, CommandId::NextMessage, &rows);
    assert_eq!(cursor_of(&effects), Some(1), "into the children");
}

#[test]
fn a_page_that_is_there_and_does_not_hold_the_row_folds_it() {
    let (mut focus, rows) = expanded(&[6, 4, 1, 3]);
    let rest = rows.with_row(0, MessageId::new(777), ThreadId::new(900), 1);
    let effects = focus.landed(&rest, false);
    assert_eq!(unspliced(&effects), vec![0], "{effects:?}");
}

// ---- (d) A click on a chip -------------------------------------------------

#[test]
fn a_click_on_a_chip_shows_that_message() {
    let (mut focus, rows, _) = crate::threads::window_on_harbor(2);
    let thread = ThreadId::new(500);
    let effects = focus.handle_on(Input::ThreadBarChipClicked(1), &rows);
    let window = window_of(&effects);
    assert_eq!(window.shown, Some(member_id(thread, 1)));
    assert_eq!(crate::threads::ringed(&window.bar.expect("bar")), Some(1));
    assert!(
        crate::threads::marked_read(&effects).is_empty(),
        "it was read"
    );

    // An unread one is marked read by showing it, as `]` does.
    let effects = focus.handle_on(Input::ThreadBarChipClicked(5), &rows);
    assert_eq!(window_of(&effects).shown, Some(member_id(thread, 5)));
    assert_eq!(
        crate::threads::marked_read(&effects),
        vec![member_id(thread, 5)]
    );

    // The one on screen, or one that is not there, does nothing.
    let effects = focus.handle_on(Input::ThreadBarChipClicked(5), &rows);
    assert!(shown(&effects).is_empty(), "{effects:?}");
    let effects = focus.handle_on(Input::ThreadBarChipClicked(40), &rows);
    assert!(shown(&effects).is_empty(), "{effects:?}");
}

#[test]
fn a_click_on_the_more_chip_opens_the_outline_on_the_first_message_it_stands_for() {
    let (mut focus, rows, _) = crate::threads::window_on_harbor(2);
    let effects = focus.handle_on(Input::BarRoom(4), &rows);
    let bar = window_of(&effects).bar.expect("the bar");
    let from = bar
        .chips
        .iter()
        .find_map(|chip| match chip {
            Chip::More { from, .. } => Some(*from),
            Chip::Message { .. } => None,
        })
        .expect("a More chip at this width");
    let effects = focus.handle_on(Input::ThreadBarMoreClicked { from }, &rows);
    let outline = shown(&effects)
        .into_iter()
        .find_map(|intent| match intent {
            Intent::Outline(outline) => Some(outline),
            _ => None,
        })
        .expect("the outline opens");
    assert_eq!(
        outline.cursor, from as usize,
        "the ring is on the first message the chip stood for"
    );
}

// ---- (e) A walk that cannot begin leaves no bar behind ----------------------

/// A window on Harbor (row 0 of `counts`), opened with Return.
fn window_over(counts: &[u32]) -> (postio_focus::FocusController, List) {
    let rows = List::of(counts);
    let mut focus = focus();
    let _ = focus.open(postio_model::ListScope::Focus(
        postio_model::FocusScope::Inbox,
    ));
    let _ = focus.landed(&rows, true);
    let effects = run(&mut focus, CommandId::OpenMessage, &rows);
    let (ticket, thread) = read_asked(&effects).expect("a read");
    let effects = answer(&mut focus, ticket, view(thread, 6, 2, false), &rows);
    assert!(window_of(&effects).bar.is_some(), "the bar is drawn");
    (focus, rows)
}

fn cleared(effects: &[Effect]) -> bool {
    let intents = shown(effects);
    let bar_empty = intents.iter().any(|intent| {
        matches!(intent, Intent::ThreadBar(bar) if bar.chips.is_empty() && bar.count_line.is_empty())
    });
    bar_empty
        && intents.contains(&Intent::Kicker(String::new()))
        && intents.contains(&Intent::OlderBanner(None))
        && intents.contains(&Intent::NewReply(None))
        && intents.contains(&Intent::ReplyTo(None))
        && intents.contains(&Intent::MatchBanner(None))
}

#[test]
fn j_onto_a_message_of_no_thread_takes_the_old_bar_away() {
    let (mut focus, rows) = window_over(&[6, 1]);
    let effects = run(&mut focus, CommandId::NextMessage, &rows);
    assert!(
        shown(&effects)
            .iter()
            .any(|intent| matches!(intent, Intent::OpenMessage { .. })),
        "the window moved on: {effects:?}"
    );
    assert!(
        cleared(&effects),
        "Harbor's bar is not left up: {effects:?}"
    );
}

#[test]
fn a_thread_read_that_cannot_begin_the_walk_takes_the_old_bar_away() {
    let (mut focus, rows) = window_over(&[6, 4]);
    let effects = run(&mut focus, CommandId::NextMessage, &rows);
    let (ticket, thread) = read_asked(&effects).expect("the new window reads its thread");
    // The thread the store gave does not hold the message on screen.
    let mut other = view(thread, 4, 0, false);
    for member in &mut other.members {
        member.id = MessageId::new(member.id.get() + 7_000);
    }
    let effects = answer(&mut focus, ticket, other, &rows);
    assert!(cleared(&effects), "{effects:?}");

    // And a read that failed.
    let (mut focus, rows) = window_over(&[6, 4]);
    let effects = run(&mut focus, CommandId::NextMessage, &rows);
    let (ticket, thread) = read_asked(&effects).expect("a read");
    let effects = focus.handle_on(
        Input::Reply(
            ticket,
            Reply::ThreadRead {
                thread,
                view: Err("the store is busy".to_owned()),
            },
        ),
        &rows,
    );
    assert!(cleared(&effects), "{effects:?}");
}

#[test]
fn a_bar_is_cleared_once_and_never_for_a_window_that_drew_none() {
    let (mut focus, rows) = window_over(&[6, 1, 1]);
    let effects = run(&mut focus, CommandId::NextMessage, &rows);
    assert!(cleared(&effects));
    let effects = run(&mut focus, CommandId::NextMessage, &rows);
    assert!(
        !shown(&effects)
            .iter()
            .any(|intent| matches!(intent, Intent::ThreadBar(_))),
        "nothing is up to clear: {effects:?}"
    );
}

// ---- (f) A sync pass that changes many threads -----------------------------

fn refreshes(effects: &[Effect]) -> usize {
    shown(effects)
        .iter()
        .filter(|intent| matches!(intent, Intent::RefreshList))
        .count()
}

fn timers(effects: &[Effect]) -> Vec<u64> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Timer { token, .. } => Some(*token),
            _ => None,
        })
        .collect()
}

#[test]
fn a_sync_pass_that_changes_many_threads_repaints_the_list_once() {
    let rows = List::of(&[6, 4, 1, 3]);
    let mut focus = focus();
    let _ = focus.open(postio_model::ListScope::Focus(
        postio_model::FocusScope::Inbox,
    ));
    let _ = focus.landed(&rows, true);

    let mut armed = Vec::new();
    let mut repainted = 0;
    for thread in 600..650 {
        let effects = focus.handle_on(changed(thread, &[thread * 10]), &rows);
        repainted += refreshes(&effects);
        armed.extend(timers(&effects));
    }
    assert_eq!(repainted, 0, "none of fifty repaints at once");
    assert_eq!(armed.len(), 1, "one wait covers them all: {armed:?}");

    // The quiet period passes (the controller has no clock; the frontend
    // calls back): one repaint.
    let effects = focus.handle_on(Input::Timer(armed[0]), &rows);
    assert_eq!(refreshes(&effects), 1, "{effects:?}");
    // And the same timer firing again repaints nothing.
    let effects = focus.handle_on(Input::Timer(armed[0]), &rows);
    assert_eq!(refreshes(&effects), 0, "{effects:?}");

    // The next change begins a new wait.
    let effects = focus.handle_on(changed(601, &[1]), &rows);
    assert_eq!((refreshes(&effects), timers(&effects).len()), (0, 1));
}

#[test]
fn a_change_to_the_expanded_thread_is_still_read_at_once() {
    let (mut focus, rows) = expanded(&[6, 4]);
    let effects = focus.handle_on(changed(500, &[9001]), &rows);
    assert!(read_asked(&effects).is_some(), "{effects:?}");
    let _ = ToastKind::Notice;
}

/// A reply is the thread's newest message, so the row it stands for is
/// another message's after the list lands: the expansion follows it.
#[test]
fn a_reply_to_the_expanded_thread_moves_its_row_and_the_expansion_follows() {
    let (mut focus, rows) = expanded(&[6, 4, 1, 3]);
    let thread = ThreadId::new(500);
    let effects = focus.handle_on(changed(500, &[9001]), &rows);
    let (ticket, _) = read_asked(&effects).expect("the thread is read again");

    // The list lands first, with the reply standing for the thread.
    let replied = rows.with_row(0, MessageId::new(9001), thread, 7);
    let effects = focus.landed(&replied, false);
    assert!(unspliced(&effects).is_empty(), "{effects:?}");

    let mut whole = view(thread, 7, 1, true);
    whole.members[6].id = MessageId::new(9001);
    let effects = answer(&mut focus, ticket, whole, &replied);
    let (anchor, children) = splice_of(&effects).expect("redrawn");
    assert_eq!((anchor, children.len()), (0, 8), "seven and the draft");
    assert!(unspliced(&effects).is_empty(), "{effects:?}");
}

/// Fifty other threads changing ask where the anchor is once, then once
/// more if any changed while the question was out.
#[test]
fn a_burst_of_other_threads_asks_where_the_anchor_is_once_at_a_time() {
    let (mut focus, rows) = expanded(&[6, 4, 1, 3]);
    let mut asked = 0;
    for thread in 600..650 {
        let effects = focus.handle_on(changed(thread, &[]), &rows);
        asked += asks(&effects)
            .iter()
            .filter(|(_, request)| matches!(request, Request::ThreadOf(_)))
            .count();
    }
    assert_eq!(asked, 1, "one question in flight at a time");
}
