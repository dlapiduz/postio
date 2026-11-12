//! Search answers in threads, from the controller's side (spec 013 US6,
//! research R11, FR-020, FR-021).
//!
//! The engine here knows three conversations: Harbor (six messages, the
//! third, fifth and sixth match "rate limit headers"), a one-message
//! "Gateway defaults", and "Re: Platform SLOs" (eight messages, only the
//! fifth matches). Ids: message `n` of Harbor is `100 + n`, of the SLOs
//! thread `300 + n`.

use crate::search_support::*;
use chrono::TimeZone;
use postio_core::CommandId;
use postio_focus::{Effect, FocusController, Input, Intent, ReaderVerb, Reply, Request, ToastKind};
use postio_model::thread_view::{Line, ThreadMember, ThreadView};
use postio_model::{EmailAddress, MessageId, ThreadId};
use postio_search::results::{
    ConversationKey, ConversationMatch, ConversationOrder, Match, Passage, RankReason, Source,
};
use postio_ui::search_view::ResultChild;

const HARBOR: i64 = 7;
const GATEWAY: i64 = 8;
const SLOS: i64 = 9;

fn member(thread: i64, number: i64, who: &EmailAddress) -> ThreadMember {
    ThreadMember {
        id: MessageId::new(if thread == SLOS { 300 } else { 100 } + number),
        from: Some(who.clone()),
        own: false,
        at: chrono::Utc
            .with_ymd_and_hms(2026, 9, 20 + number as u32, 12, 0, 0)
            .unwrap(),
        seen: true,
        has_attachments: false,
        line: Line::Own(format!("message {number}")),
        answers: None,
        marker: false,
    }
}

/// Messages from Tomás on the odd numbers and Ben on the even.
fn view_of(thread: i64, count: i64) -> ThreadView {
    ThreadView {
        thread: ThreadId::new(thread),
        subject: Some(match thread {
            HARBOR => "Harbor API draft v3".to_owned(),
            SLOS => "Re: Platform SLOs".to_owned(),
            _ => "Gateway defaults".to_owned(),
        }),
        members: (1..=count)
            .map(|n| member(thread, n, &if n % 2 == 0 { ben() } else { tomas() }))
            .collect(),
        draft: None,
        muted: None,
        labels: Vec::new(),
    }
}

fn read_view(thread: ThreadId) -> Option<ThreadView> {
    match thread.get() {
        HARBOR => Some(view_of(HARBOR, 6)),
        GATEWAY => Some(view_of(GATEWAY, 1)),
        SLOS => Some(view_of(SLOS, 8)),
        _ => None,
    }
}

fn hit(
    thread: i64,
    best: i64,
    messages: u32,
    matched: u32,
    subject: &str,
) -> postio_search::results::ConversationHit {
    let mut hit = conversation(0);
    hit.key = ConversationKey::Thread(ThreadId::new(thread));
    hit.best = MessageId::new(best);
    hit.messages = messages;
    hit.subject = Some(subject.to_owned());
    hit.unread = false;
    hit.has_attachments = false;
    hit.labels = Vec::new();
    hit.reasons = vec![RankReason::Matches(matched)];
    hit
}

fn found(message: i64) -> ConversationMatch {
    ConversationMatch {
        message: Some(MessageId::new(message)),
        from: Some(tomas()),
        found: Match {
            source: Source::Body,
            passage: Some(passage_for(MessageId::new(message))),
            when: None,
        },
        to: Vec::new(),
    }
}

fn subject_match() -> ConversationMatch {
    ConversationMatch {
        message: None,
        from: None,
        found: Match {
            source: Source::Subject,
            passage: Some(Passage {
                text: "Harbor API draft v3".to_owned(),
                ranges: Vec::new(),
                elided_start: false,
                elided_end: false,
            }),
            when: None,
        },
        to: Vec::new(),
    }
}

fn matches_of(key: ConversationKey) -> Vec<ConversationMatch> {
    match key {
        ConversationKey::Thread(thread) if thread.get() == HARBOR => {
            vec![found(103), found(105), found(106), subject_match()]
        }
        ConversationKey::Thread(thread) if thread.get() == SLOS => vec![found(305)],
        _ => vec![found(201)],
    }
}

/// The engine: the conversations above in Best match order, the rest as
/// the results suite's engine has it.
fn engine(request: &Request) -> Option<Reply> {
    match request {
        Request::ResultsPage {
            query,
            order,
            offset,
            limit,
            stamp,
        } => {
            let mut answer = page(query, *order, *offset, *limit);
            if *order == ConversationOrder::BestMatch {
                answer.hits = vec![
                    hit(HARBOR, 106, 6, 3, "Harbor API draft v3"),
                    hit(GATEWAY, 201, 1, 1, "Gateway defaults"),
                    hit(SLOS, 305, 8, 1, "Re: Platform SLOs"),
                ];
            }
            Some(Reply::ResultsPage {
                stamp: *stamp,
                order: *order,
                offset: *offset,
                answer: Ok(Box::new(answer)),
            })
        }
        Request::QuickLookMatches { key, stamp, .. } => Some(Reply::QuickLookMatches {
            stamp: *stamp,
            answer: Ok(matches_of(*key)),
        }),
        Request::ReadThread(thread) => Some(Reply::ThreadRead {
            thread: *thread,
            view: Ok(read_view(*thread)),
        }),
        other => reply_to(other),
    }
}

/// Answer every read among `effects` until none is left; every batch of
/// effects the answers produced, in order, after `effects`.
fn settle_all(focus: &mut FocusController, effects: Vec<Effect>, rows: &List) -> Vec<Vec<Effect>> {
    let mut batches = vec![effects];
    let mut at = 0;
    while at < batches.len() {
        let waiting = batches[at].clone();
        for (ticket, request) in asks(&waiting) {
            if let Some(reply) = engine(&request) {
                batches.push(focus.handle_on(Input::Reply(ticket, reply), rows));
            }
        }
        at += 1;
    }
    batches
}

fn flat(batches: &[Vec<Effect>]) -> Vec<Effect> {
    batches.iter().flatten().cloned().collect()
}

/// Search "rate limit headers": Harbor on top, expanded.
fn harbor_results(rows: &List) -> (FocusController, Vec<Vec<Effect>>) {
    let mut focus = mac();
    let started = show_all(&mut focus, "rate limit headers", rows);
    let batches = settle_all(&mut focus, started, rows);
    (focus, batches)
}

fn last_splice(effects: &[Effect]) -> Option<Option<postio_focus::ResultSplice>> {
    shown(effects)
        .into_iter()
        .rev()
        .find_map(|intent| match intent {
            Intent::ResultSplice(splice) => Some(splice.map(|splice| *splice)),
            _ => None,
        })
}

/// What a child says: its number and tag, or its gap line.
fn said(child: &ResultChild) -> String {
    match child {
        ResultChild::Match { tag, .. } => tag.clone(),
        gap => postio_ui::search_view::gap_line(gap),
    }
}

#[test]
fn the_top_result_with_several_matches_is_expanded_on_arrival() {
    let rows = List::of(3);
    let (_focus, batches) = harbor_results(&rows);
    let splice = last_splice(&flat(&batches))
        .expect("a splice was said")
        .expect("and it is one");
    assert_eq!(splice.anchor, 0, "under the top hit");
    assert_eq!(splice.focused, None);
    assert_eq!(
        splice.children.iter().map(said).collect::<Vec<_>>(),
        [
            "message 3",
            "Message 4 doesn't match \u{b7} Ben, Thu",
            "message 5",
            "message 6"
        ],
        "the subject match is no child, and one line stands for message 4"
    );
}

#[test]
fn the_default_expansion_is_read_with_the_page_and_costs_no_second_trip() {
    let rows = List::of(3);
    let (_focus, batches) = harbor_results(&rows);
    // The batch the top hits landed in asks for their passages, the
    // matches and the thread together.
    let landing = batches
        .iter()
        .find(|batch| {
            asked(batch)
                .iter()
                .any(|request| matches!(request, Request::ResultsPassages { .. }))
        })
        .expect("the top hits' passages were asked for");
    let there = asked(landing);
    assert_eq!(
        there
            .iter()
            .filter(|request| matches!(request, Request::QuickLookMatches { .. }))
            .count(),
        1,
        "{there:?}"
    );
    assert!(
        there.contains(&Request::ReadThread(ThreadId::new(HARBOR))),
        "{there:?}"
    );
    // And nowhere else: one read of the matches and one of the thread, for
    // the one expanded result.
    let all = asked(&flat(&batches));
    assert_eq!(
        all.iter()
            .filter(|request| matches!(request, Request::QuickLookMatches { .. }))
            .count(),
        1
    );
    assert_eq!(
        all.iter()
            .filter(|request| matches!(request, Request::ReadThread(_)))
            .count(),
        1
    );
}

#[test]
fn j_walks_into_the_children_and_skips_the_gap() {
    let rows = List::of(3);
    let (mut focus, _) = harbor_results(&rows);
    let mut at = Vec::new();
    for _ in 0..4 {
        let effects = run(&mut focus, CommandId::NextMessage, &rows);
        let shown = shown(&effects);
        at.push((
            shown.iter().rev().find_map(|intent| match intent {
                Intent::ResultChildFocus(child) => Some(*child),
                _ => None,
            }),
            shown.iter().rev().find_map(|intent| match intent {
                Intent::ResultsCursor(position) => Some(*position),
                _ => None,
            }),
        ));
    }
    assert_eq!(
        at,
        [
            (Some(Some(0)), None),
            (Some(Some(2)), None),
            (Some(Some(3)), None),
            (Some(None), Some(1)),
        ],
        "message 3, 5, 6, then the next result"
    );
}

#[test]
fn right_and_left_expand_and_collapse_other_results() {
    let rows = List::of(3);
    let (mut focus, _) = harbor_results(&rows);
    // Past Harbor's three matching children to Gateway, one message.
    for _ in 0..4 {
        let _ = run(&mut focus, CommandId::NextMessage, &rows);
    }
    let effects = run(&mut focus, CommandId::ExpandThread, &rows);
    assert!(asked(&effects).is_empty(), "{:?}", asked(&effects));
    // The SLOs thread is the last row.
    let _ = run(&mut focus, CommandId::NextMessage, &rows);
    let effects = run(&mut focus, CommandId::ExpandThread, &rows);
    let batches = settle_all(&mut focus, effects, &rows);
    let splice = last_splice(&flat(&batches))
        .expect("said")
        .expect("expanded");
    assert_eq!(splice.anchor, 2);
    assert_eq!(
        splice.children.iter().map(said).collect::<Vec<_>>(),
        ["message 5"]
    );
    // One at a time: Harbor's folded when the other opened.
    let effects = run(&mut focus, CommandId::CollapseThread, &rows);
    assert_eq!(last_splice(&effects), Some(None));
}

/// `j` to message 3 and Return: the window opens on it.
fn opened_at_message_3(rows: &List) -> (FocusController, Vec<Effect>) {
    let (mut focus, _) = harbor_results(rows);
    let _ = run(&mut focus, CommandId::NextMessage, rows);
    let effects = run(&mut focus, CommandId::OpenMessage, rows);
    let batches = settle_all(&mut focus, effects, rows);
    (focus, flat(&batches))
}

#[test]
fn return_on_a_match_opens_the_window_at_that_message_with_every_match_marked() {
    let rows = List::of(3);
    let (_focus, effects) = opened_at_message_3(&rows);
    let shown = shown(&effects);
    assert!(
        shown.iter().any(|intent| matches!(
            intent,
            Intent::OpenMessage { message, .. } if *message == MessageId::new(103)
        )),
        "{shown:?}"
    );
    let bar = shown
        .iter()
        .rev()
        .find_map(|intent| match intent {
            Intent::ThreadBar(bar) => Some(bar.clone()),
            _ => None,
        })
        .expect("a thread bar");
    let marked: Vec<u32> = bar
        .chips
        .iter()
        .filter_map(|chip| match chip {
            postio_ui::thread_bar::Chip::Message {
                index,
                matched: true,
                ..
            } => Some(*index),
            _ => None,
        })
        .collect();
    assert_eq!(marked, [2, 4, 5], "messages 3, 5 and 6 are marked");
    let kicker = shown.iter().rev().find_map(|intent| match intent {
        Intent::Kicker(kicker) => Some(kicker.clone()),
        _ => None,
    });
    assert!(kicker.expect("a kicker").starts_with("Message 3 of 6"));
    let banner = shown
        .iter()
        .rev()
        .find_map(|intent| match intent {
            Intent::MatchBanner(banner) => Some(banner.clone()),
            _ => None,
        })
        .expect("a banner decision")
        .expect("a match banner");
    assert_eq!(
        banner.text,
        "\u{2018}rate limit headers\u{2019} \u{b7} match 1 of 3 in this thread (messages 3, 5 and 6)"
    );
    assert_eq!(banner.next, "Next");
    assert_eq!(banner.back, "Back to results");
}

fn banner_text(effects: &[Effect]) -> Option<String> {
    shown(effects)
        .into_iter()
        .rev()
        .find_map(|intent| match intent {
            Intent::MatchBanner(Some(banner)) => Some(banner.text),
            _ => None,
        })
}

fn opened(effects: &[Effect]) -> Option<MessageId> {
    shown(effects)
        .into_iter()
        .rev()
        .find_map(|intent| match intent {
            Intent::OpenMessage { message, .. } => Some(message),
            _ => None,
        })
}

#[test]
fn find_next_steps_in_the_message_then_on_to_the_next_matched_message() {
    let rows = List::of(3);
    let (mut focus, _) = opened_at_message_3(&rows);
    // The message's own find goes first.
    let effects = run(&mut focus, CommandId::FindNext, &rows);
    assert!(
        shown(&effects).contains(&Intent::Reader(ReaderVerb::NextMatch)),
        "{:?}",
        shown(&effects)
    );
    assert_eq!(opened(&effects), None, "the window has not moved");
    // It has no more: on to message 5 (4 does not match).
    let effects = focus.handle_on(Input::MatchEdge { forward: true }, &rows);
    assert_eq!(opened(&effects), Some(MessageId::new(105)));
    assert!(
        banner_text(&effects)
            .expect("the banner")
            .contains("match 2 of 3")
    );
    let effects = focus.handle_on(Input::MatchEdge { forward: true }, &rows);
    assert_eq!(opened(&effects), Some(MessageId::new(106)));
    assert!(banner_text(&effects).unwrap().contains("match 3 of 3"));
}

#[test]
fn on_the_last_match_it_stays_and_says_so() {
    let rows = List::of(3);
    let (mut focus, _) = opened_at_message_3(&rows);
    let _ = focus.handle_on(Input::MatchEdge { forward: true }, &rows);
    let _ = focus.handle_on(Input::MatchEdge { forward: true }, &rows);
    let effects = focus.handle_on(Input::MatchEdge { forward: true }, &rows);
    assert_eq!(opened(&effects), None, "it stays on message 6");
    let said = shown(&effects).into_iter().find_map(|intent| match intent {
        Intent::Toast {
            text,
            kind: ToastKind::Notice,
        } => Some(text),
        _ => None,
    });
    assert_eq!(
        said.as_deref(),
        Some("That is the last match in this thread")
    );
    // And back: find previous steps to message 5.
    let effects = run(&mut focus, CommandId::FindPrevious, &rows);
    assert!(shown(&effects).contains(&Intent::Reader(ReaderVerb::PrevMatch)));
    let effects = focus.handle_on(Input::MatchEdge { forward: false }, &rows);
    assert_eq!(opened(&effects), Some(MessageId::new(105)));
}

#[test]
fn escape_returns_to_the_results_with_the_ring_on_the_child_it_opened_from() {
    let rows = List::of(3);
    let (mut focus, _) = opened_at_message_3(&rows);
    // mod+g twice reaches message 6.
    let _ = focus.handle_on(Input::MatchEdge { forward: true }, &rows);
    let _ = focus.handle_on(Input::MatchEdge { forward: true }, &rows);
    let effects = run(&mut focus, CommandId::Back, &rows);
    let shown = shown(&effects);
    assert!(
        shown.contains(&Intent::CloseSurface(postio_focus::SurfaceKind::Message)),
        "{shown:?}"
    );
    assert!(
        shown.contains(&Intent::ResultChildFocus(Some(0))),
        "{shown:?}"
    );
    assert!(focus.in_results(), "the results are still up");
}

#[test]
fn a_window_not_opened_from_a_search_has_no_match_banner() {
    let rows = List::of(3);
    let mut focus = mac();
    let _ = search(&mut focus, "atlas budget", &rows);
    let _ = run(&mut focus, CommandId::OpenMessage, &rows);
    let effects = run(&mut focus, CommandId::FindNext, &rows);
    assert!(
        !shown(&effects).contains(&Intent::Reader(ReaderVerb::NextMatch)),
        "the window's own find stays the frontend's"
    );
}

#[test]
fn a_multi_match_thread_says_how_many_of_its_messages_match_and_a_single_match_does_not() {
    let rows = List::of(3);
    let (focus, _) = harbor_results(&rows);
    assert_eq!(
        focus.result_row(0).expect("harbor").summary.as_deref(),
        Some("3 of 6 messages match")
    );
    assert_eq!(focus.result_row(1).expect("gateway").summary, None);
    assert_eq!(
        focus.result_row(2).expect("slos").summary,
        None,
        "one match in eight keeps its snippet"
    );
}
