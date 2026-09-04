//! What a conversation looks like, decided without a toolkit.
//!
//! The conversation surfaces — the list row that stands for a thread, and the
//! pane that stacks its messages (ADR 0015) — draw the same facts twice, and
//! decided them twice: who is in a conversation was worked out inside a
//! `postio-gtk` widget, which made it unreachable from a second frontend and
//! provable only with a display.
//!
//! These are rules, not pixels, so they live here. What the widgets keep is
//! the drawing.

use std::ops::Range;

use chrono::{DateTime, Datelike, Local};
use postio_model::address::EmailAddress;

/// How many names fit before the line starts eliding.
const NAMES_SHOWN: usize = 3;

/// The people in a conversation, short and newest-biased.
///
/// Short names, because every surface that draws this gives it one line
/// beside something else: a subject and a snippet in the list, a message
/// count and a date span in the conversation header. A conversation between
/// four people spelled out in full would push the rest off. First name where
/// there is a display name, the address's local part where there is not —
/// the same information a person uses to recognise a thread at a glance.
///
/// **Newest-biased when it elides.** Participants arrive in first-seen order,
/// so the interesting end is the far one: who started the conversation still
/// identifies it, and who spoke most recently is what changed since you last
/// looked. Both survive; the middle is what goes.
pub fn participants(participants: &[EmailAddress]) -> String {
    // Distinct first, on the full name: one person writing five times is one
    // name, and two different people can share a first name.
    let mut distinct: Vec<&EmailAddress> = Vec::new();
    for address in participants {
        if !distinct
            .iter()
            .any(|seen| seen.display() == address.display())
        {
            distinct.push(address);
        }
    }

    // One participant is not a crowd, and shortening it loses information for
    // nothing: "Site Office" becomes "Site" and an address with no display
    // name becomes half of itself. A conversation with one voice reads
    // exactly like the message row it replaced, which is what the canvas
    // draws.
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

/// One participant, shortened to the token that identifies them.
///
/// A display name is "Ada Norwood"; an address falls back to its local part,
/// which is what a sender without a name has to identify them. Trailing
/// punctuation goes with the quotes. "Bergstrom, Tove" is what Exchange and
/// most directories write, and its first token is `Bergstrom,` — which the
/// join above would then turn into "Bergstrom,, Jonas" (#826). Trimmed rather
/// than split on, because the comma is the only part that is noise: the
/// surname before it is exactly the short name this wants.
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
    // Nothing usable was left — a display name that is only punctuation, or
    // none at all. The local part is what identifies a sender who has not
    // given a name, which is the same answer the `@` branch above reaches
    // for; falling back to the raw display here would put the punctuation
    // straight back into the row.
    address
        .local_part()
        .map(str::to_owned)
        .unwrap_or_else(|| display.clone())
}

/// How many messages a conversation holds, said out loud.
///
/// Singular at one. A pane that says "1 messages" reads as a bug in
/// everything else it is telling you.
pub fn message_count(messages: usize) -> String {
    match messages {
        1 => "1 message".to_owned(),
        n => format!("{n} messages"),
    }
}

/// The span a conversation covers: `10-25 Aug`, `24 Aug`, `28 Dec 2025 - 3 Jan
/// 2026`.
///
/// Three shapes, and each one exists because the shorter version would be
/// wrong rather than merely terse:
///
/// * **One day** says one date. `24-24 Aug` is not a span, it is a rendering
///   fault.
/// * **Within a year**, the month is written once when both ends share it
///   (`10-25 Aug`) and twice when they do not (`28 Jul - 3 Aug`).
/// * **Across a year boundary**, both ends carry their year. A conversation
///   that ran from December into January is exactly the case where "28 Dec -
///   3 Jan" invites the reader to guess, and guessing wrong makes a
///   two-week thread look like an eleven-month one.
///
/// `now` decides whether a year is worth printing at all: this year is the
/// ordinary case and saying so on every conversation is noise.
pub fn date_span(first: DateTime<Local>, last: DateTime<Local>, now: DateTime<Local>) -> String {
    // Defensive rather than a contract: the caller sorts oldest-first, and a
    // pane that drew `25-10 Aug` because one message had a bad Date header
    // would be reporting the header rather than the conversation.
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

/// The three-letter month, in the canvas's own casing.
fn month(at: DateTime<Local>) -> String {
    at.format("%b").to_string()
}

/// Runs of consecutive collapsed messages long enough to fold into one
/// divider (canvas turn 8a).
///
/// `collapsed[i]` says whether message `i` is drawn as a one-line header. The
/// answer is the ranges worth replacing with `5 earlier messages · Ada, Bo`.
///
/// **Three, not two.** A divider hides its messages behind a click, so it has
/// to save more lines than it costs. Two collapsed rows become one divider
/// plus nothing — no saving, and a gesture where there was none. Three
/// become one, which is the first point the trade is worth making.
pub fn collapsed_runs(collapsed: &[bool], minimum: usize) -> Vec<Range<usize>> {
    let mut runs = Vec::new();
    let mut start = None;
    for (index, folded) in collapsed.iter().enumerate() {
        match (folded, start) {
            (true, None) => start = Some(index),
            (false, Some(from)) => {
                if index - from >= minimum {
                    runs.push(from..index);
                }
                start = None;
            }
            _ => {}
        }
    }
    if let Some(from) = start
        && collapsed.len() - from >= minimum
    {
        runs.push(from..collapsed.len());
    }
    runs
}

/// How many consecutive collapsed messages earn a divider.
pub const RUN_MINIMUM: usize = 3;

/// What a folded run says: `5 earlier messages · Ada, Bo`.
///
/// The senders are the run's own, deduped in order of first appearance and
/// shortened the way [`participants`] shortens a thread's — one vocabulary
/// for "who is in this", whether the "this" is a conversation or five lines
/// of it.
pub fn run_summary(count: usize, senders: &[EmailAddress]) -> String {
    let who = participants(senders);
    if who.is_empty() {
        return format!("{count} earlier messages");
    }
    format!("{count} earlier messages · {who}")
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- the participants line (ADR 0015 Q2, #307) ------------------------

    fn people(names: &[(&str, &str)]) -> Vec<EmailAddress> {
        names
            .iter()
            .map(|(name, address)| EmailAddress::new(Some(*name), *address))
            .collect()
    }

    #[test]
    fn one_participant_keeps_their_whole_name() {
        // A conversation with one voice has to read exactly like the message
        // row it replaced — every row in a folder is a thread row now, so
        // shortening here would shorten the ordinary case. "Site Office"
        // must not become "Site".
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
        // "Bergstrom, Tove" is what Exchange and most directories put in a
        // display name, and shortening it to the first whitespace token kept
        // the comma -- so joining the names produced "Bergstrom,, Jonas",
        // which reads as a rendering fault rather than as two people (#826).
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
        // The elided branch joins with ", " as well, so it has the same
        // problem and needs the same proof -- this is the shape the shot in
        // #826 actually showed.
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
        // The trim must not eat a name that is only punctuation, or a sender
        // whose display name is a stray comma would vanish from the row
        // rather than being named by their address.
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
        // Newest-biased: whoever started it still identifies the
        // conversation, and whoever spoke last is what changed since you
        // looked. The middle is what nobody scans for.
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
        // Deduplication happens before the "is this one voice" test, or a
        // thread where one person replied to themselves would read as two.
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
        // Several addresses do not fit; their local parts do, and are what
        // distinguishes them.
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
        // A message row has none, and `initials_source` falls back to the
        // sender rather than drawing something that looks like a thread.
        assert_eq!(participants(&[]), "");
    }

    // -- the header's own line (canvas turn 8a, #1004) --------------------

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
        // `24-24 Aug` is not a span, it is a rendering fault.
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
        // The case where the short form invites a guess, and a wrong guess
        // turns a two-week thread into an eleven-month one.
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
        // Noise on every conversation, for the ordinary case.
        assert!(!date_span(at(2026, 8, 10), at(2026, 8, 25), at(2026, 9, 3)).contains("2026"));
    }

    #[test]
    fn ends_the_wrong_way_round_still_read_forwards() {
        // A bad `Date` header should not make the pane report the header
        // instead of the conversation.
        assert_eq!(
            date_span(at(2026, 8, 25), at(2026, 8, 10), at(2026, 9, 3)),
            "10-25 Aug"
        );
    }

    // -- folded runs (canvas turn 8a, #1005) ------------------------------

    #[test]
    fn a_run_of_three_folds_and_a_run_of_two_does_not() {
        // Two collapsed rows become one divider plus nothing: no saving, and
        // a gesture where there was none.
        assert_eq!(collapsed_runs(&[true, true], RUN_MINIMUM), Vec::new());
        assert_eq!(collapsed_runs(&[true, true, true], RUN_MINIMUM), vec![0..3]);
    }

    #[test]
    fn the_canvas_shape_folds_only_its_middle() {
        // 8 messages: one collapsed at the top, five collapsed in the middle,
        // one collapsed, one expanded at the end. The top one is a run of
        // one and stays as itself.
        let collapsed = [true, false, true, true, true, true, true, false];
        assert_eq!(collapsed_runs(&collapsed, RUN_MINIMUM), vec![2..7]);
    }

    #[test]
    fn a_run_that_reaches_the_end_still_counts() {
        // The loop has to close an open run when the slice ends, or a
        // conversation whose tail is collapsed folds nothing.
        assert_eq!(collapsed_runs(&[false, true, true, true], RUN_MINIMUM), vec![1..4]);
    }

    #[test]
    fn two_runs_are_two_dividers() {
        let collapsed = [true, true, true, false, true, true, true, true];
        assert_eq!(collapsed_runs(&collapsed, RUN_MINIMUM), vec![0..3, 4..8]);
    }

    #[test]
    fn nothing_collapsed_folds_nothing() {
        assert_eq!(collapsed_runs(&[false, false, false], RUN_MINIMUM), Vec::new());
        assert_eq!(collapsed_runs(&[], RUN_MINIMUM), Vec::new());
    }

    #[test]
    fn a_divider_names_its_count_and_who_is_in_it() {
        let senders = people(&[
            ("Ada Norwood", "ada@example.com"),
            ("Bo Ferris", "bo@example.com"),
            ("Ada Norwood", "ada@example.com"),
        ]);
        assert_eq!(run_summary(5, &senders), "5 earlier messages · Ada, Bo");
    }

    #[test]
    fn a_divider_with_no_senders_still_says_how_many() {
        assert_eq!(run_summary(4, &[]), "4 earlier messages");
    }
}
