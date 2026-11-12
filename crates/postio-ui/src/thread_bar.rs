//! The message window's thread bar and the words around it (spec 013 US3,
//! research R8).
//!
//! The window shows one message of a thread at a time. These are the pure
//! parts of what it adds: which chips fit in the bar, the kicker above the
//! subject, the older-message banner, the sender block's Reply to row and
//! the label column's count. Both frontends draw what they return, so the
//! Mac and GTK say identical words.

use chrono::{DateTime, Local};
use postio_core::{CommandId, Keymap};
use postio_model::MessageId;
use postio_model::thread_view::{Line, ThreadMember, ThreadView};

use crate::{focus_row, hints, row};

/// One chip of the bar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Chip {
    /// A message: avatar over "Name · time".
    Message {
        /// Its place among the members, from 0.
        index: u32,
        /// The avatar's letters.
        initials: String,
        /// "Ben · Thu"; "You · 15:22" for the account's own.
        label: String,
        /// The message on screen: ringed.
        current: bool,
        /// Not read yet: dotted.
        unread: bool,
        /// A search matched it: a yellow square.
        matched: bool,
    },
    /// "… N more": members `from..=to` (indexes) that did not fit.
    More {
        /// How many it stands for.
        count: u32,
        /// The first member it stands for.
        from: u32,
        /// The last member it stands for.
        to: u32,
    },
}

/// The chips of `members` that fit in `room` chips, the member at `current`
/// on screen, `matches` the messages a search matched, `now` for the times.
pub fn chips(
    members: &[ThreadMember],
    current: usize,
    matches: &[MessageId],
    room: u32,
    now: DateTime<Local>,
) -> Vec<Chip> {
    let count = members.len();
    let chip = |index: usize| {
        let member = &members[index];
        let who = match (&member.from, member.own) {
            (_, true) => "You",
            (Some(from), false) => focus_row::first_name(from),
            (None, false) => "?",
        };
        Chip::Message {
            index: index as u32,
            initials: row::initials(member.from.as_ref()),
            label: format!("{who} \u{b7} {}", row::timestamp(member.at, now)),
            current: index == current,
            unread: !member.seen,
            matched: matches.contains(&member.id),
        }
    };
    if count as u32 <= room {
        return (0..count).map(chip).collect();
    }
    // What stays whatever the room: the first, the one on screen and every
    // unread. The latest that fit are added to it, newest first, for as
    // long as the chips and the "more" chips between them are within room.
    let mut kept = vec![false; count];
    kept[0] = true;
    if let Some(at) = kept.get_mut(current) {
        *at = true;
    }
    for (index, member) in members.iter().enumerate() {
        if !member.seen {
            kept[index] = true;
        }
    }
    for index in (0..count).rev() {
        if kept[index] {
            continue;
        }
        kept[index] = true;
        if cost(&kept) > room {
            kept[index] = false;
            break;
        }
    }
    let mut bar = Vec::new();
    let mut index = 0;
    while index < count {
        if kept[index] {
            bar.push(chip(index));
            index += 1;
            continue;
        }
        let from = index;
        while index < count && !kept[index] {
            index += 1;
        }
        // A gap of one costs what its message does, so it shows the message.
        if index - from == 1 {
            bar.push(chip(from));
        } else {
            bar.push(Chip::More {
                count: (index - from) as u32,
                from: from as u32,
                to: (index - 1) as u32,
            });
        }
    }
    bar
}

/// What `kept` costs in chips: each kept message, and one "more" chip for
/// every run of the others between and after them.
fn cost(kept: &[bool]) -> u32 {
    let mut chips = 0;
    let mut gap = false;
    for &keep in kept {
        if keep {
            chips += 1;
            gap = false;
        } else if !gap {
            chips += 1;
            gap = true;
        }
    }
    chips
}

/// Everything the bar draws: the label column, the chips and the button.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadBarView {
    /// The label column's first line: "Thread".
    pub title: String,
    /// Its second line: "6 · 2 new".
    pub count_line: String,
    /// The chips, left to right. A thread of one message has none: draw no
    /// bar.
    pub chips: Vec<Chip>,
    /// The button on the right, which opens the thread menu: "Thread".
    pub menu_label: String,
}

/// The bar of `view` with the member at `current` on screen.
pub fn bar(
    view: &ThreadView,
    current: usize,
    matches: &[MessageId],
    room: u32,
    now: DateTime<Local>,
) -> ThreadBarView {
    let many = view.members.len() > 1;
    ThreadBarView {
        title: "Thread".to_owned(),
        count_line: count_line(view),
        chips: if many {
            chips(&view.members, current, matches, room, now)
        } else {
            Vec::new()
        },
        menu_label: "Thread".to_owned(),
    }
}

/// "Message 4 of 6 · ] newer · [ older", the keys the keymap has.
pub fn kicker(index: usize, count: usize, keymap: &Keymap) -> String {
    let mut said = format!("Message {} of {count}", index + 1);
    for (command, word) in [
        (CommandId::NextInConversation, "newer"),
        (CommandId::PrevInConversation, "older"),
    ] {
        if let Some(key) = hints::key(keymap, command) {
            said.push_str(&format!(" \u{b7} {key} {word}"));
        }
    }
    said
}

/// A quiet banner under the bar: words, and one button.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BannerView {
    /// What it says.
    pub text: String,
    /// The button: "Latest".
    pub action: String,
    /// The button's key, as the keymap has it: "}".
    pub key: Option<String>,
}

/// What the window says when the message on screen left its thread (deleted
/// here or elsewhere) and it moved to `newer`, the next newer message, or
/// the next older one.
pub fn gone_notice(newer: bool) -> &'static str {
    if newer {
        "That message is gone. Showing the next newer one."
    } else {
        "That message is gone. Showing the next older one."
    }
}

/// The banner on a message that is not the latest.
pub fn older_banner(keymap: &Keymap) -> BannerView {
    BannerView {
        text: "This is an older message. Reply and forward act on it; archive, snooze, \
               label and move act on the whole thread."
            .to_owned(),
        action: "Latest".to_owned(),
        key: hints::key(keymap, CommandId::LatestInConversation),
    }
}

/// What the window says when `mod+g` has no match left to go to.
pub fn no_more_matches(forward: bool) -> &'static str {
    if forward {
        "That is the last match in this thread"
    } else {
        "That is the first match in this thread"
    }
}

/// The banner for a reply that arrived while the window was on another
/// message: who wrote, the first line of it, and `}` to read it. The reader
/// stays where they are.
pub fn new_reply_banner(member: &ThreadMember, keymap: &Keymap) -> BannerView {
    let who = match (&member.from, member.own) {
        (_, true) => "You",
        (Some(from), false) => focus_row::first_name(from),
        (None, false) => "Someone",
    };
    let line = match &member.line {
        Line::Own(text) | Line::Preview(text) => text.trim(),
        Line::Quoted => "",
    };
    let quoted = if line.is_empty() {
        String::new()
    } else {
        format!(": \u{201c}{line}\u{201d}")
    };
    BannerView {
        text: format!("{who} replied just now{quoted}. You stay on this message until you move."),
        action: "Read it".to_owned(),
        key: hints::key(keymap, CommandId::LatestInConversation),
    }
}

/// The banner of a window opened from a search: where among the matched
/// messages the one on screen is, and the two ways on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatchBannerView {
    /// "\u{2018}rate limit headers\u{2019} \u{b7} match 1 of 3 in this thread (messages 3, 5 and 6)".
    pub text: String,
    /// The first button: "Next".
    pub next: String,
    /// Its key, as the keymap has it.
    pub next_key: Option<String>,
    /// The second: "Back to results".
    pub back: String,
    /// Its key, as the keymap has it.
    pub back_key: Option<String>,
}

/// The match banner: `query` as typed, the matched message at `at` of
/// `numbers` (each message's place in the thread, from 1) on screen.
pub fn match_banner(query: &str, at: usize, numbers: &[u32], keymap: &Keymap) -> MatchBannerView {
    let listed: Vec<String> = numbers.iter().map(u32::to_string).collect();
    let (last, rest) = listed
        .split_last()
        .map_or(("", &[][..]), |(l, r)| (l.as_str(), r));
    let where_ = match rest {
        [] => format!("message {last}"),
        _ => format!("messages {} and {last}", rest.join(", ")),
    };
    MatchBannerView {
        text: format!(
            "\u{2018}{query}\u{2019} \u{b7} match {} of {} in this thread ({where_})",
            at + 1,
            numbers.len()
        ),
        next: "Next".to_owned(),
        next_key: hints::key(keymap, CommandId::FindNext),
        back: "Back to results".to_owned(),
        back_key: hints::key(keymap, CommandId::Back),
    }
}

/// "Ana Ruiz, Mon 21 Sep (message 1)": the message `index` answers, when
/// it is not the one just before it.
pub fn reply_to_row(view: &ThreadView, index: usize) -> Option<String> {
    let answered = view
        .members
        .get(view.members.get(index)?.answers? as usize)?;
    let who = if answered.own {
        "You"
    } else {
        answered
            .from
            .as_ref()
            .map_or("Unknown sender", |f| f.display())
    };
    let day = answered.at.with_timezone(&Local).format("%a %-d %b");
    let number = view
        .members
        .iter()
        .position(|member| member.id == answered.id)?
        + 1;
    Some(format!("{who}, {day} (message {number})"))
}

/// The label column's second line: "6 · 2 new", or just "6".
pub fn count_line(view: &ThreadView) -> String {
    let unread = view.members.iter().filter(|member| !member.seen).count();
    match focus_row::new_badge(unread as u32) {
        Some(new) => format!("{} \u{b7} {new}", view.members.len()),
        None => view.members.len().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, TimeZone, Utc};
    use postio_model::{EmailAddress, ThreadId};

    fn now() -> DateTime<Local> {
        Local.with_ymd_and_hms(2026, 9, 26, 16, 41, 0).unwrap()
    }

    fn members(count: usize, unread: &[usize]) -> Vec<ThreadMember> {
        let first = Utc.with_ymd_and_hms(2026, 9, 21, 12, 0, 0).unwrap();
        (0..count)
            .map(|index| ThreadMember {
                id: MessageId::new(index as i64 + 1),
                from: Some(EmailAddress::new(Some("Ben Adeyemi"), "ben@example.net")),
                own: false,
                at: first + Duration::days(index as i64),
                seen: !unread.contains(&index),
                has_attachments: false,
                line: postio_model::thread_view::Line::Quoted,
                answers: None,
                marker: false,
            })
            .collect()
    }

    fn view(members: Vec<ThreadMember>) -> ThreadView {
        ThreadView {
            thread: ThreadId::new(1),
            subject: None,
            members,
            draft: None,
            muted: None,
            labels: Vec::new(),
        }
    }

    /// What a bar says in one line: `3` a message, `3*` the current, `3.`
    /// unread, `[a-b]` a More.
    fn shape(chips: &[Chip]) -> Vec<String> {
        chips
            .iter()
            .map(|chip| match chip {
                Chip::Message {
                    index,
                    current,
                    unread,
                    ..
                } => format!(
                    "{index}{}{}",
                    if *current { "*" } else { "" },
                    if *unread { "." } else { "" }
                ),
                Chip::More { from, to, count } => {
                    assert_eq!(*count, to - from + 1);
                    format!("[{from}-{to}]")
                }
            })
            .collect()
    }

    #[test]
    fn everything_that_fits_is_one_chip_per_message() {
        let all = members(6, &[4, 5]);
        let bar = chips(&all, 3, &[], 8, now());
        assert_eq!(shape(&bar), ["0", "1", "2", "3*", "4.", "5."]);
        // Room for exactly as many as there are is still everything.
        assert_eq!(chips(&all, 3, &[], 6, now()).len(), 6);
    }

    #[test]
    fn a_chip_says_who_and_when_and_what_marks_it() {
        let mut all = members(6, &[5]);
        all[5].at = Local
            .with_ymd_and_hms(2026, 9, 26, 13, 40, 0)
            .unwrap()
            .with_timezone(&Utc);
        all[2].own = true;
        let bar = chips(&all, 5, &[MessageId::new(2)], 8, now());
        let Chip::Message {
            initials,
            label,
            current,
            unread,
            matched,
            ..
        } = &bar[0]
        else {
            panic!("the first is a message");
        };
        assert_eq!(initials, "BA");
        assert_eq!(label, "Ben \u{b7} Mon");
        assert!(!current && !unread && !matched);
        let Chip::Message { label, matched, .. } = &bar[1] else {
            panic!()
        };
        assert_eq!(label, "Ben \u{b7} Tue");
        assert!(*matched, "message 2 is the search's");
        let Chip::Message { label, .. } = &bar[2] else {
            panic!()
        };
        assert!(label.starts_with("You \u{b7} "));
        let Chip::Message {
            label,
            current,
            unread,
            ..
        } = &bar[5]
        else {
            panic!()
        };
        // 13:40 today is a clock time, as every row says it.
        assert_eq!(label, "Ben \u{b7} 13:40");
        assert!(*current && *unread);
    }

    #[test]
    fn forty_members_in_room_for_eight_keep_the_first_a_more_and_the_latest() {
        let all = members(40, &[]);
        let bar = chips(&all, 39, &[], 8, now());
        assert_eq!(
            shape(&bar),
            ["0", "[1-33]", "34", "35", "36", "37", "38", "39*"]
        );
    }

    #[test]
    fn the_current_and_every_unread_are_kept() {
        let all = members(40, &[12, 13]);
        let bar = chips(&all, 3, &[], 8, now());
        let shown = shape(&bar);
        for want in ["0", "3*", "12.", "13."] {
            assert!(shown.contains(&want.to_owned()), "{want} in {shown:?}");
        }
        assert_eq!(shown.first().map(String::as_str), Some("0"));
        // Nothing more than the room, since the kept ones fit in it.
        assert!(bar.len() <= 8, "{shown:?}");
        // Every member is a chip or inside a More, in order, once.
        let mut next = 0;
        for chip in &bar {
            match chip {
                Chip::Message { index, .. } => {
                    assert_eq!(*index, next);
                    next += 1;
                }
                Chip::More { from, to, .. } => {
                    assert_eq!(*from, next);
                    next = to + 1;
                }
            }
        }
        assert_eq!(next, 40);
    }

    #[test]
    fn kept_chips_beyond_the_room_stand_apart_with_a_more_between_each() {
        let all = members(20, &[3, 8, 14]);
        let bar = chips(&all, 3, &[], 5, now());
        assert_eq!(
            shape(&bar),
            [
                "0", "[1-2]", "3*.", "[4-7]", "8.", "[9-13]", "14.", "[15-19]"
            ]
        );
    }

    #[test]
    fn a_gap_of_one_is_the_message_itself() {
        let all = members(10, &[]);
        // Kept: 0, 2 (current) and the latest; message 1 alone is a chip.
        let bar = chips(&all, 2, &[], 4, now());
        assert!(
            !bar.iter()
                .any(|chip| matches!(chip, Chip::More { count: 1, .. })),
            "{:?}",
            shape(&bar)
        );
    }

    #[test]
    fn the_kicker_names_the_place_and_the_keys() {
        assert_eq!(
            kicker(3, 6, Keymap::defaults()),
            "Message 4 of 6 \u{b7} ] newer \u{b7} [ older"
        );
        let mut overrides = postio_config::KeyBindings::default();
        overrides.overrides_mut().insert(
            postio_core::CommandId::NextInConversation
                .as_str()
                .to_owned(),
            "L".to_owned(),
        );
        let rebound = Keymap::resolve(&overrides);
        assert_eq!(
            kicker(0, 2, &rebound),
            "Message 1 of 2 \u{b7} L newer \u{b7} [ older"
        );
    }

    #[test]
    fn the_older_banner_says_what_acts_on_what_and_offers_the_latest() {
        let banner = older_banner(Keymap::defaults());
        assert_eq!(
            banner.text,
            "This is an older message. Reply and forward act on it; archive, snooze, \
             label and move act on the whole thread."
        );
        assert_eq!(banner.action, "Latest");
        assert_eq!(banner.key.as_deref(), Some("}"));
    }

    #[test]
    fn reply_to_names_the_message_answered_unless_it_is_the_one_before() {
        let mut all = members(4, &[]);
        all[0].from = Some(EmailAddress::new(Some("Ana Ruiz"), "ana@example.com"));
        all[3].answers = Some(0);
        let v = view(all);
        assert_eq!(
            reply_to_row(&v, 3).as_deref(),
            Some("Ana Ruiz, Mon 21 Sep (message 1)")
        );
        assert_eq!(reply_to_row(&v, 2), None, "it answers the one before it");
        assert_eq!(reply_to_row(&v, 9), None, "no such message");
    }

    #[test]
    fn the_count_line_is_the_size_and_what_is_new() {
        assert_eq!(count_line(&view(members(6, &[4, 5]))), "6 \u{b7} 2 new");
        assert_eq!(count_line(&view(members(6, &[]))), "6");
    }

    #[test]
    fn the_match_banner_says_which_match_and_where_the_others_are() {
        let banner = match_banner("rate limit headers", 0, &[3, 5, 6], Keymap::defaults());
        assert_eq!(
            banner.text,
            "\u{2018}rate limit headers\u{2019} \u{b7} match 1 of 3 in this thread (messages 3, 5 and 6)"
        );
        assert_eq!(
            (banner.next.as_str(), banner.next_key.as_deref()),
            (
                "Next",
                hints::key(Keymap::defaults(), CommandId::FindNext).as_deref()
            )
        );
        assert!(banner.next_key.is_some(), "find next is bound");
        assert_eq!(
            (banner.back.as_str(), banner.back_key.as_deref()),
            ("Back to results", Some("Escape"))
        );
        let two = match_banner("rate", 1, &[3, 5], Keymap::defaults());
        assert!(
            two.text
                .ends_with("match 2 of 2 in this thread (messages 3 and 5)"),
            "{}",
            two.text
        );
        let one = match_banner("rate", 0, &[3], Keymap::defaults());
        assert!(
            one.text
                .ends_with("match 1 of 1 in this thread (message 3)"),
            "{}",
            one.text
        );
    }
}
