//! A reply arriving while the window reads (spec 013 US4, FR-018): the
//! reader is never moved. The bar gains a chip, a banner says who replied,
//! and `}` is the way to it.

use crate::search_support::shown;
use crate::threads::{
    List, answer, member_id, read_asked, ringed, run, view, window_of, window_on_harbor,
};
use chrono::{TimeZone, Utc};
use postio_core::{CommandId, Event};
use postio_focus::{Effect, FocusController, Input, Intent};
use postio_model::thread_view::{Line, ThreadMember, ThreadView};
use postio_model::{AccountId, EmailAddress, MessageId, ThreadId};
use postio_ui::thread_bar::{BannerView, Chip};

const THREAD: ThreadId = ThreadId::new(500);

/// Harbor's read with `replies` more members after its six: each from Ana,
/// unread, a minute after the one before.
fn with_replies(replies: usize) -> ThreadView {
    let mut read = view(THREAD, 6, 2, false);
    for index in 0..replies {
        read.members.push(ThreadMember {
            id: member_id(THREAD, 6 + index),
            from: Some(EmailAddress::new(Some("Ana Ruiz"), "ana@example.com")),
            own: false,
            at: Utc
                .with_ymd_and_hms(2026, 9, 21, 10, index as u32, 0)
                .unwrap(),
            seen: false,
            has_attachments: false,
            line: Line::Own(format!("+1 to Ben on exports, number {index}.")),
            answers: None,
            marker: false,
        });
    }
    read
}

fn changed(thread: ThreadId, added: &[usize]) -> Input {
    Input::Event(Event::ThreadChanged {
        account: AccountId::new(1),
        thread,
        members_added: added.iter().map(|at| member_id(thread, *at)).collect(),
    })
}

/// The window's thread changes by `added`, and the client reads it as `read`.
fn arrives(
    focus: &mut FocusController,
    rows: &List,
    added: &[usize],
    read: ThreadView,
) -> Vec<Effect> {
    let effects = focus.handle_on(changed(THREAD, added), rows);
    let (ticket, asked) = read_asked(&effects).expect("the window reads its thread again");
    assert_eq!(asked, THREAD);
    answer(focus, ticket, read, rows)
}

fn new_reply(effects: &[Effect]) -> Option<Option<BannerView>> {
    shown(effects).into_iter().find_map(|intent| match intent {
        Intent::NewReply(banner) => Some(banner),
        _ => None,
    })
}

fn unread_chips(bar: &postio_ui::thread_bar::ThreadBarView) -> Vec<u32> {
    bar.chips
        .iter()
        .filter_map(|chip| match chip {
            Chip::Message { index, unread, .. } if *unread => Some(*index),
            _ => None,
        })
        .collect()
}

fn opened(effects: &[Effect]) -> Option<MessageId> {
    window_of(effects).shown
}

#[test]
fn a_reply_never_moves_the_reader_and_the_banner_names_who_wrote() {
    let (mut focus, rows, _) = window_on_harbor(2);
    let effects = arrives(&mut focus, &rows, &[6], with_replies(1));
    assert_eq!(opened(&effects), None, "no message is put up: {effects:?}");
    let banner = new_reply(&effects)
        .expect("the banner is said")
        .expect("and it is up");
    assert!(banner.text.contains("Ana"), "{banner:?}");
    assert!(
        banner.text.contains("+1 to Ben on exports, number 0."),
        "it quotes the first line: {banner:?}"
    );
    assert_eq!(banner.action, "Read it");
    assert_eq!(banner.key.as_deref(), Some("}"));

    let window = window_of(&effects);
    let bar = window.bar.expect("the bar is drawn again");
    assert_eq!(bar.chips.len(), 7, "one more chip");
    assert_eq!(ringed(&bar), Some(4), "the ring stays on the message shown");
    assert_eq!(unread_chips(&bar), vec![4, 5, 6], "the new one is dotted");
    assert_eq!(bar.count_line, "7 \u{b7} 3 new");
    assert!(
        window
            .kicker
            .as_deref()
            .is_some_and(|k| k.starts_with("Message 5 of 7")),
        "{:?}",
        window.kicker
    );
}

#[test]
fn two_arrivals_name_the_latest_and_add_both_chips() {
    let (mut focus, rows, _) = window_on_harbor(2);
    let _ = arrives(&mut focus, &rows, &[6], with_replies(1));
    let effects = arrives(&mut focus, &rows, &[7], with_replies(2));
    assert_eq!(opened(&effects), None);
    let banner = new_reply(&effects).flatten().expect("a banner");
    assert!(banner.text.contains("number 1"), "the latest: {banner:?}");
    let bar = window_of(&effects).bar.expect("bar");
    assert_eq!(bar.chips.len(), 8);
    assert_eq!(unread_chips(&bar), vec![4, 5, 6, 7]);
    assert_eq!(ringed(&bar), Some(4));
}

#[test]
fn latest_shows_the_reply_and_takes_the_banner_away() {
    let (mut focus, rows, _) = window_on_harbor(2);
    let _ = arrives(&mut focus, &rows, &[6], with_replies(1));
    let effects = run(&mut focus, CommandId::LatestInConversation, &rows);
    assert_eq!(opened(&effects), Some(member_id(THREAD, 6)));
    assert_eq!(new_reply(&effects), Some(None), "the banner goes");
    assert_eq!(ringed(&window_of(&effects).bar.expect("bar")), Some(6));
}

#[test]
fn the_row_behind_the_window_is_read_again() {
    let (mut focus, rows, _) = window_on_harbor(2);
    let effects = focus.handle_on(changed(THREAD, &[6]), &rows);
    assert!(
        shown(&effects)
            .iter()
            .any(|intent| matches!(intent, Intent::RefreshList)),
        "the list re-reads, and its row shows the new count, people, badge and time: {effects:?}"
    );
}

#[test]
fn another_threads_change_changes_nothing_in_the_window() {
    let (mut focus, rows, _) = window_on_harbor(2);
    let effects = focus.handle_on(changed(ThreadId::new(501), &[0]), &rows);
    assert!(read_asked(&effects).is_none(), "{effects:?}");
    assert!(new_reply(&effects).is_none(), "{effects:?}");
    assert!(window_of(&effects).bar.is_none(), "{effects:?}");
}
