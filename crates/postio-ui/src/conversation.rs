//! Conversation rules without a toolkit: who is in a thread, its date span, its order and what
//! opens expanded (ADR 0015).

use chrono::{DateTime, Datelike, Local, Utc};
use postio_model::address::EmailAddress;

/// Shared with the header's participant chips so the faces and the names agree about who was
/// elided.
pub const NAMES_SHOWN: usize = 3;

/// First names (local part for a bare address), newest-biased when eliding: the first and
/// last speakers survive and the middle goes.
pub fn participants(participants: &[EmailAddress]) -> String {
    // Distinct on the full name: one person writing five times is one name; two people can share a
    // first name.
    let mut distinct: Vec<&EmailAddress> = Vec::new();
    for address in participants {
        if !distinct
            .iter()
            .any(|seen| seen.display() == address.display())
        {
            distinct.push(address);
        }
    }

    // One participant is not shortened: "Site Office" must not become "Site".
    if distinct.len() == 1 {
        return distinct[0].display().to_string();
    }

    let mut names: Vec<String> = Vec::new();
    for address in distinct {
        let name = short_name(address);
        if !name.is_empty() && !names.contains(&name) {
            names.push(name);
        }
    }
    match names.len() {
        0 => String::new(),
        n if n <= NAMES_SHOWN => names.join(", "),
        _ => {
            let last = names.len();
            format!(
                "{} .. {}",
                names[0],
                names[last - (NAMES_SHOWN - 1)..].join(", ")
            )
        }
    }
}

/// First token of the display name, trailing punctuation trimmed ("Bergstrom, Tove" would
/// join as "Bergstrom,, Jonas", #826); the local part for an address.
fn short_name(address: &EmailAddress) -> String {
    let display = address.display().to_string();
    let name = display
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .trim_matches(|c: char| c == '"' || c == '\'')
        .trim_end_matches([',', ';'])
        .to_string();
    if name.contains('@') {
        return name.split('@').next().unwrap_or(&name).to_string();
    }
    if !name.is_empty() {
        return name;
    }
    // Nothing usable left (a punctuation-only name): the local part, not the raw display.
    address
        .local_part()
        .map(str::to_owned)
        .unwrap_or_else(|| display.clone())
}

pub fn message_count(messages: usize) -> String {
    match messages {
        1 => "1 message".to_owned(),
        n => format!("{n} messages"),
    }
}

/// `10-25 Aug`, `24 Aug`, `28 Dec 2025 - 3 Jan 2026`: one day says one date, a shared month
/// is written once, a year boundary carries both years, and `now` omits the current year.
pub fn date_span(first: DateTime<Local>, last: DateTime<Local>, now: DateTime<Local>) -> String {
    // Defensive: out-of-order dates (a bad `Date` header) must not draw `25-10 Aug`.
    let (first, last) = if first <= last {
        (first, last)
    } else {
        (last, first)
    };

    let same_year = first.year() == last.year();
    let this_year = same_year && first.year() == now.year();

    if first.date_naive() == last.date_naive() {
        return if this_year {
            format!("{} {}", first.day(), month(first))
        } else {
            format!("{} {} {}", first.day(), month(first), first.year())
        };
    }
    if !same_year {
        return format!(
            "{} {} {} - {} {} {}",
            first.day(),
            month(first),
            first.year(),
            last.day(),
            month(last),
            last.year()
        );
    }
    let span = if first.month() == last.month() {
        format!("{}-{} {}", first.day(), last.day(), month(last))
    } else {
        format!(
            "{} {} - {} {}",
            first.day(),
            month(first),
            last.day(),
            month(last)
        )
    };
    if this_year {
        span
    } else {
        format!("{span} {}", last.year())
    }
}

/// `Mon 25 Aug at 12:00`. Longer than the list's timestamp on purpose; today and yesterday
/// are words, and another year carries the year.
pub fn message_when(at: DateTime<Local>, now: DateTime<Local>) -> String {
    let time = at.format("%H:%M");
    let days = now
        .date_naive()
        .signed_duration_since(at.date_naive())
        .num_days();
    if days == 0 {
        return format!("Today at {time}");
    }
    if days == 1 {
        return format!("Yesterday at {time}");
    }
    let weekday = at.format("%a");
    if at.year() == now.year() {
        format!("{weekday} {} {} at {time}", at.day(), month(at))
    } else {
        format!(
            "{weekday} {} {} {} at {time}",
            at.day(),
            month(at),
            at.year()
        )
    }
}

fn month(at: DateTime<Local>) -> String {
    at.format("%b").to_string()
}

/// A trait because the frontends carry different rows (the FFI's `RowFfi` crosses a C ABI).
pub trait ConversationMessage {
    /// What a conversation opens expanded, and what `unread_only` keeps.
    fn seen(&self) -> bool;

    fn received_at(&self) -> DateTime<Utc>;

    /// Tie-break for two messages in the same second: the local id, stable and unique.
    fn ordinal(&self) -> i64;
}

/// Each expanded message is a web view; expanding every unread one would break the interaction and
/// memory budgets.
pub const EAGER_EXPANSION_CAP: usize = 3;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Order {
    #[default]
    Oldest,
    Newest,
}

pub fn arrange<T: ConversationMessage + Clone>(
    rows: &[T],
    order: Order,
    unread_only: bool,
) -> Vec<T> {
    let mut rows: Vec<T> = rows
        .iter()
        .filter(|row| !unread_only || !row.seen())
        .cloned()
        .collect();
    // By id after the timestamp so same-second messages do not swap between redraws.
    rows.sort_by_key(|row| (row.received_at(), row.ordinal()));
    if order == Order::Newest {
        rows.reverse();
    }
    rows
}

/// The newest message, always: the pane shows every body, so a reply is aimed at the newest (ADR
/// 0015 amended).
/// `None` only for an empty conversation; `messages` is oldest first.
pub fn opening_focus<T: ConversationMessage>(messages: &[T]) -> Option<usize> {
    messages.len().checked_sub(1)
}

/// Read messages collapse; the focused message always expands, then unread ones behind it
/// up to `cap`, since each expansion costs a web view. Walks backwards from the focus,
/// which is the newest message. The stacked pane's rule; GTK draws one document (ADR 0032).
pub fn expanded_on_open<T: ConversationMessage>(
    messages: &[T],
    focus: usize,
    cap: usize,
) -> Vec<bool> {
    if messages.is_empty() {
        return Vec::new();
    }
    let mut expanded = vec![false; messages.len()];
    let mut spent = 0;
    for index in (0..=focus.min(messages.len() - 1)).rev() {
        if spent >= cap {
            break;
        }
        if index == focus || !messages[index].seen() {
            expanded[index] = true;
            spent += 1;
        }
    }
    expanded
}

#[cfg(test)]
mod tests {
    use super::*;

    fn people(names: &[(&str, &str)]) -> Vec<EmailAddress> {
        names
            .iter()
            .map(|(name, address)| EmailAddress::new(Some(*name), *address))
            .collect()
    }

    #[test]
    fn one_participant_keeps_their_whole_name() {
        assert_eq!(
            participants(&people(&[("Ada Norwood", "ada@example.com")])),
            "Ada Norwood"
        );
        assert_eq!(
            participants(&people(&[("Site Office", "site@example.com")])),
            "Site Office"
        );
    }

    #[test]
    fn a_last_first_display_name_does_not_leave_its_comma_behind() {
        // "Bergstrom, Tove" must not join as "Bergstrom,, Jonas" (#826).
        assert_eq!(
            participants(&people(&[
                ("Quinn Abara", "quinn@example.net"),
                ("Bergstrom, Tove", "tove@example.com"),
                ("Jonas Vek", "jonas@example.org"),
            ])),
            "Quinn, Bergstrom, Jonas"
        );
    }

    #[test]
    fn a_last_first_name_survives_the_elision_too() {
        assert_eq!(
            participants(&people(&[
                ("Quinn Abara", "quinn@example.net"),
                ("Ada Norwood", "ada@example.com"),
                ("Priya Raman", "priya@example.org"),
                ("Bergstrom, Tove", "tove@example.com"),
                ("Jonas Vek", "jonas@example.org"),
            ])),
            "Quinn .. Bergstrom, Jonas"
        );
    }

    #[test]
    fn a_trailing_comma_is_not_mistaken_for_the_whole_name() {
        // The trim must not eat a punctuation-only name.
        assert_eq!(
            participants(&people(&[
                (",", "odd@example.com"),
                ("Jonas Vek", "jonas@example.org"),
            ])),
            "odd, Jonas"
        );
    }

    #[test]
    fn a_short_conversation_names_everyone_in_it() {
        assert_eq!(
            participants(&people(&[
                ("Ada Norwood", "ada@example.com"),
                ("Quinn Abara", "quinn@example.net"),
                ("Tove Bergstrom", "tove@example.com"),
            ])),
            "Ada, Quinn, Tove"
        );
    }

    #[test]
    fn a_long_conversation_keeps_both_ends_and_drops_the_middle() {
        assert_eq!(
            participants(&people(&[
                ("Ada Norwood", "ada@example.com"),
                ("Quinn Abara", "quinn@example.net"),
                ("Jonas Vek", "jonas@example.org"),
                ("Tove Bergstrom", "tove@example.com"),
                ("Priya Raman", "priya@example.org"),
            ])),
            "Ada .. Tove, Priya"
        );
    }

    #[test]
    fn one_person_writing_repeatedly_is_named_once() {
        assert_eq!(
            participants(&people(&[
                ("Ada Norwood", "ada@example.com"),
                ("Ada Norwood", "ada@example.com"),
                ("Quinn Abara", "quinn@example.net"),
            ])),
            "Ada, Quinn"
        );
    }

    #[test]
    fn one_person_writing_repeatedly_and_alone_is_not_a_crowd() {
        // Dedupe before the one-voice test, or a thread of one person replying to themselves reads
        // as two.
        assert_eq!(
            participants(&people(&[
                ("Site Office", "site@example.com"),
                ("Site Office", "site@example.com"),
            ])),
            "Site Office"
        );
    }

    #[test]
    fn a_lone_sender_with_no_display_name_keeps_their_whole_address() {
        assert_eq!(
            participants(&[EmailAddress::new(None::<&str>, "ada.norwood@example.com")]),
            "ada.norwood@example.com"
        );
    }

    #[test]
    fn a_crowd_with_no_display_names_is_named_by_local_parts() {
        assert_eq!(
            participants(&[
                EmailAddress::new(None::<&str>, "ada@example.com"),
                EmailAddress::new(None::<&str>, "quinn@example.net"),
            ]),
            "ada, quinn"
        );
    }

    #[test]
    fn no_participants_is_empty_rather_than_a_placeholder() {
        assert_eq!(participants(&[]), "");
    }

    use chrono::TimeZone;

    fn at(year: i32, month: u32, day: u32) -> DateTime<Local> {
        Local
            .with_ymd_and_hms(year, month, day, 12, 0, 0)
            .single()
            .expect("a real local time")
    }

    #[test]
    fn one_message_is_not_messages() {
        assert_eq!(message_count(1), "1 message");
        assert_eq!(message_count(8), "8 messages");
        assert_eq!(message_count(0), "0 messages");
    }

    #[test]
    fn a_conversation_had_in_one_day_says_one_date() {
        let day = at(2026, 8, 24);
        assert_eq!(date_span(day, day, at(2026, 9, 3)), "24 Aug");
    }

    #[test]
    fn a_span_inside_one_month_writes_the_month_once() {
        assert_eq!(
            date_span(at(2026, 8, 10), at(2026, 8, 25), at(2026, 9, 3)),
            "10-25 Aug",
            "the canvas's own header line"
        );
    }

    #[test]
    fn a_span_across_two_months_writes_both() {
        assert_eq!(
            date_span(at(2026, 7, 28), at(2026, 8, 3), at(2026, 9, 3)),
            "28 Jul - 3 Aug"
        );
    }

    #[test]
    fn a_span_across_a_year_carries_both_years() {
        assert_eq!(
            date_span(at(2025, 12, 28), at(2026, 1, 3), at(2026, 9, 3)),
            "28 Dec 2025 - 3 Jan 2026"
        );
    }

    #[test]
    fn a_conversation_from_a_past_year_says_which() {
        assert_eq!(
            date_span(at(2025, 8, 10), at(2025, 8, 25), at(2026, 9, 3)),
            "10-25 Aug 2025"
        );
        assert_eq!(
            date_span(at(2025, 8, 24), at(2025, 8, 24), at(2026, 9, 3)),
            "24 Aug 2025"
        );
    }

    #[test]
    fn a_conversation_this_year_does_not_repeat_the_year() {
        assert!(!date_span(at(2026, 8, 10), at(2026, 8, 25), at(2026, 9, 3)).contains("2026"));
    }

    #[test]
    fn ends_the_wrong_way_round_still_read_forwards() {
        assert_eq!(
            date_span(at(2026, 8, 25), at(2026, 8, 10), at(2026, 9, 3)),
            "10-25 Aug"
        );
    }

    #[test]
    fn a_message_from_today_does_not_name_the_day() {
        let now = at(2026, 8, 25);
        let arrived = now - chrono::Duration::hours(2);
        assert_eq!(message_when(arrived, now), "Today at 10:00");
    }

    #[test]
    fn yesterday_is_a_word_rather_than_a_date() {
        let now = at(2026, 8, 25);
        let arrived = at(2026, 8, 24);
        assert_eq!(message_when(arrived, now), "Yesterday at 12:00");
    }

    #[test]
    fn anything_older_carries_its_weekday() {
        let now = at(2026, 8, 25);
        let arrived = at(2026, 8, 10);
        assert_eq!(message_when(arrived, now), "Mon 10 Aug at 12:00");
    }

    #[test]
    fn a_message_from_another_year_says_which() {
        let now = at(2026, 1, 3);
        let arrived = at(2025, 12, 28);
        assert_eq!(message_when(arrived, now), "Sun 28 Dec 2025 at 12:00");
    }

    #[derive(Clone, Debug, PartialEq)]
    struct Msg {
        id: i64,
        seen: bool,
        at: DateTime<Utc>,
    }

    impl ConversationMessage for Msg {
        fn seen(&self) -> bool {
            self.seen
        }

        fn received_at(&self) -> DateTime<Utc> {
            self.at
        }

        fn ordinal(&self) -> i64 {
            self.id
        }
    }

    fn message(id: i64, seen: bool) -> Msg {
        Msg {
            id,
            seen,
            at: Utc.timestamp_opt(1_770_000_000 + id, 0).single().unwrap(),
        }
    }

    fn arrived(id: i64, second: i64) -> Msg {
        Msg {
            at: Utc
                .timestamp_opt(1_770_000_000 + second, 0)
                .single()
                .unwrap(),
            ..message(id, true)
        }
    }

    #[test]
    fn a_conversation_stacks_oldest_first() {
        let stacked = arrange(
            &[message(3, true), message(1, true), message(2, true)],
            Order::Oldest,
            false,
        );
        assert_eq!(
            stacked.iter().map(|row| row.id).collect::<Vec<_>>(),
            vec![1, 2, 3],
            "a conversation is stacked the way it was had"
        );
    }

    #[test]
    fn two_messages_in_the_same_second_do_not_swap_between_redraws() {
        // Same-second messages need a tie-break or the pane reorders under the reader.
        let first = arrange(&[arrived(9, 40), arrived(4, 40)], Order::Oldest, false);
        let again = arrange(&[arrived(4, 40), arrived(9, 40)], Order::Oldest, false);
        assert_eq!(
            first.iter().map(|row| row.id).collect::<Vec<_>>(),
            vec![4, 9]
        );
        assert_eq!(
            first, again,
            "the same conversation must stack the same way"
        );
    }

    #[test]
    fn newest_first_is_the_list_order_reversed() {
        let stacked = arrange(
            &[message(1, true), message(2, true), message(3, true)],
            Order::Newest,
            false,
        );
        assert_eq!(
            stacked.iter().map(|row| row.id).collect::<Vec<_>>(),
            vec![3, 2, 1]
        );
    }

    #[test]
    fn unread_only_leaves_the_read_ones_out() {
        let stacked = arrange(
            &[message(1, true), message(2, false), message(3, true)],
            Order::Oldest,
            true,
        );
        assert_eq!(
            stacked.iter().map(|row| row.id).collect::<Vec<_>>(),
            vec![2]
        );
    }

    #[test]
    fn a_conversation_opens_on_its_most_recent_message() {
        let messages = [
            message(1, true),
            message(2, true),
            message(3, false),
            message(4, false),
        ];
        assert_eq!(opening_focus(&messages), Some(3));
    }

    #[test]
    fn a_conversation_read_all_the_way_through_opens_on_its_newest() {
        let messages = [message(1, true), message(2, true), message(3, true)];
        assert_eq!(opening_focus(&messages), Some(2));
    }

    #[test]
    fn a_wholly_unread_conversation_still_opens_at_the_end() {
        let messages = [message(1, false), message(2, false), message(3, false)];
        assert_eq!(opening_focus(&messages), Some(2));
    }

    #[test]
    fn read_state_does_not_move_the_opening_focus_at_all() {
        let unread_early = [message(1, true), message(2, false), message(3, true)];
        let all_read = [message(1, true), message(2, true), message(3, true)];
        assert_eq!(opening_focus(&unread_early), opening_focus(&all_read));
    }

    #[test]
    fn an_empty_conversation_has_nowhere_to_focus() {
        assert_eq!(opening_focus::<Msg>(&[]), None);
    }

    #[test]
    fn everything_after_the_focus_stays_collapsed() {
        let messages = [
            message(1, true),
            message(2, true),
            message(3, false),
            message(4, false),
        ];
        let expanded = expanded_on_open(&messages, 2, EAGER_EXPANSION_CAP);
        assert_eq!(
            expanded,
            vec![false, false, true, false],
            "the focus opens, the read ones behind it stay shut, and the \
             unread one *after* it is not part of what was landed on"
        );
    }

    #[test]
    fn a_long_unread_conversation_does_not_expand_all_of_it() {
        let messages: Vec<Msg> = (0..30).map(|id| message(id, false)).collect();
        let expanded = expanded_on_open(&messages, 29, EAGER_EXPANSION_CAP);

        assert_eq!(
            expanded.iter().filter(|open| **open).count(),
            EAGER_EXPANSION_CAP,
            "opening a conversation must not cost a web view per message"
        );
        assert!(
            expanded[30 - EAGER_EXPANSION_CAP..]
                .iter()
                .all(|open| *open),
            "the ones that do expand are the focus and the ones behind it"
        );
    }

    #[test]
    fn the_focused_message_expands_even_when_it_has_been_read() {
        let messages = [message(1, true), message(2, true), message(3, true)];
        let expanded = expanded_on_open(&messages, 2, EAGER_EXPANSION_CAP);
        assert_eq!(expanded, vec![false, false, true]);
    }

    #[test]
    fn a_read_message_behind_the_focus_stays_collapsed() {
        let messages = [message(1, false), message(2, true), message(3, false)];
        let expanded = expanded_on_open(&messages, 2, EAGER_EXPANSION_CAP);
        assert_eq!(expanded, vec![true, false, true]);
    }

    #[test]
    fn a_cap_of_one_opens_only_what_is_focused() {
        let messages: Vec<Msg> = (0..5).map(|id| message(id, false)).collect();
        let expanded = expanded_on_open(&messages, 1, 1);
        assert_eq!(expanded, vec![false, true, false, false, false]);
    }

    #[test]
    fn an_empty_conversation_expands_nothing() {
        assert!(expanded_on_open::<Msg>(&[], 0, EAGER_EXPANSION_CAP).is_empty());
    }
}
