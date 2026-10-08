//! A to-do's deadline: "by Friday", "before 15 October", "no later than
//! Thursday", read through `parse_when`, the reading the snooze and remind
//! pickers give a typed date (research R6, R10).

use chrono::{DateTime, Datelike, NaiveDate, TimeZone};
use postio_search::date::parse_when;

use super::lexicon::{DEADLINE_KEYWORDS, EVENT_OPENERS, IDIOMS, Idiom};

/// How many words after a keyword can hold its date: "before the release
/// on Friday", "by Monday, 28 September".
const WINDOW: usize = 8;

/// The longest date `parse_when` is offered: "Tue 29 Sep 09:00".
const LONGEST: usize = 4;

/// How far into the window an event's name can reach before its day:
/// "before our quarterly call on Tuesday".
const EVENT_WORDS: usize = 5;

/// The deadline a clause names, as the first instant it can mean after
/// `sent`, the moment the message was written; `None` when it names none,
/// or one already past.
///
/// `lower` is the clause lowercased, with its apostrophes straightened. The
/// first keyword that names a day wins.
pub(super) fn deadline<Tz: TimeZone>(lower: &str, sent: &DateTime<Tz>) -> Option<DateTime<Tz>> {
    let mut keywords: Vec<(usize, &str)> = DEADLINE_KEYWORDS
        .iter()
        .flat_map(|keyword| lower.match_indices(keyword))
        .filter(|(at, keyword)| {
            let before = lower[..*at].chars().next_back();
            let after = lower[at + keyword.len()..].chars().next();
            !before.is_some_and(char::is_alphanumeric) && after.is_some_and(char::is_whitespace)
        })
        .collect();
    keywords.sort_unstable();
    keywords.into_iter().find_map(|(at, keyword)| {
        let words: Vec<&str> = lower[at + keyword.len()..]
            .split_whitespace()
            .take(WINDOW)
            .collect();
        after_keyword(&words, sent)
    })
}

/// The day the words after a keyword name, if they name one.
fn after_keyword<Tz: TimeZone>(words: &[&str], sent: &DateTime<Tz>) -> Option<DateTime<Tz>> {
    let bare: Vec<&str> = words.iter().map(|word| bare(word)).collect();
    if let Some(idiom) = idiom(&bare) {
        return parse_when(&idiom_phrase(idiom, sent), sent.clone());
    }
    if let Some(found) = phrase_at(words, 0, sent) {
        return found;
    }
    // "before the release on Friday": an event, named, then the day it is.
    if EVENT_OPENERS.contains(bare.first()?) {
        let on = bare
            .iter()
            .take(EVENT_WORDS)
            .position(|word| matches!(*word, "on" | "at"))?;
        return phrase_at(words, on + 1, sent).flatten();
    }
    None
}

/// The longest run of words from `start` that `parse_when` reads, resolved
/// from `sent`: `Some(None)` when it reads one but the time has gone by,
/// and `None` when no run from here is a date at all. A bare number is not
/// a date: "by 10 percent" is not ten o'clock.
fn phrase_at<Tz: TimeZone>(
    words: &[&str],
    start: usize,
    sent: &DateTime<Tz>,
) -> Option<Option<DateTime<Tz>>> {
    let start_of_day = sent
        .timezone()
        .from_local_datetime(&sent.date_naive().and_hms_opt(0, 0, 0)?)
        .earliest()
        .unwrap_or_else(|| sent.clone());
    let last = words.len().min(start + LONGEST);
    (start + 1..=last).rev().find_map(|end| {
        let phrase = words[start..end]
            .iter()
            .map(|word| word.trim_end_matches(['.', ',', ';', ':', '!', '?', ')', '"', '\'']))
            .collect::<Vec<_>>()
            .join(" ");
        if phrase
            .chars()
            .all(|c| c.is_ascii_digit() || c.is_whitespace())
        {
            return None;
        }
        // Read from the start of the day, so a phrase whose time has gone
        // by is still known for a date, and is not cut down to a shorter
        // one that means something else.
        parse_when(&phrase, start_of_day.clone())?;
        Some(parse_when(&phrase, sent.clone()))
    })
}

/// A word without the punctuation around it.
fn bare(word: &str) -> &str {
    word.trim_matches(|c: char| !c.is_alphanumeric())
}

/// The idiom the words open with, after an optional "the".
fn idiom(bare: &[&str]) -> Option<&'static Idiom> {
    let words = match bare.first() {
        Some(&"the") => &bare[1..],
        _ => bare,
    };
    IDIOMS
        .iter()
        .find(|(phrase, _)| words.starts_with(phrase))
        .map(|(_, idiom)| idiom)
}

/// An idiom as words `parse_when` knows.
fn idiom_phrase<Tz: TimeZone>(idiom: &Idiom, sent: &DateTime<Tz>) -> String {
    match idiom {
        Idiom::Tonight => "tonight".to_owned(),
        Idiom::Friday => "friday 18:00".to_owned(),
        Idiom::MonthEnd => {
            let today = sent.date_naive();
            let (year, month) = if today.month() == 12 {
                (today.year() + 1, 1)
            } else {
                (today.year(), today.month() + 1)
            };
            let last = NaiveDate::from_ymd_opt(year, month, 1)
                .and_then(|first| first.pred_opt())
                .unwrap_or(today);
            format!("{} 18:00", last.format("%Y-%m-%d"))
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::{FixedOffset, NaiveDate};

    use super::*;

    /// Saturday 26 September 2026 at noon, two hours east of UTC: the day
    /// US12's scenarios are set on.
    fn sent() -> DateTime<FixedOffset> {
        FixedOffset::east_opt(2 * 3600)
            .expect("an offset")
            .with_ymd_and_hms(2026, 9, 26, 12, 0, 0)
            .single()
            .expect("a time")
    }

    fn due(lower: &str) -> Option<NaiveDate> {
        deadline(lower, &sent()).map(|when| when.date_naive())
    }

    fn day(month: u32, day: u32) -> Option<NaiveDate> {
        NaiveDate::from_ymd_opt(2026, month, day)
    }

    #[test]
    fn a_weekday_is_the_next_one_after_the_message_was_sent() {
        // US12 scenario 2: sent on Saturday 26 September, due Wed 30 Sep.
        assert_eq!(due("please leave comments by wednesday"), day(9, 30));
        assert_eq!(
            due("please sign the nda and send it back by monday."),
            day(9, 28)
        );
        assert_eq!(due("please complete it by friday."), day(10, 2));
    }

    #[test]
    fn a_deadline_is_by_before_or_no_later_than() {
        assert_eq!(due("please register before 15 october."), day(10, 15));
        assert_eq!(
            due("please confirm the final headcount no later than thursday."),
            day(10, 1)
        );
        assert_eq!(
            due("please rsvp by 5 october so we can confirm numbers."),
            day(10, 5)
        );
    }

    #[test]
    fn a_date_may_carry_its_weekday() {
        assert_eq!(
            due("please send the signed timesheet by monday, 28 september."),
            day(9, 28)
        );
    }

    #[test]
    fn what_follows_the_date_is_not_part_of_it() {
        assert_eq!(
            due("let me know by tuesday if you can make it."),
            day(9, 29)
        );
        assert_eq!(
            due("please upload your slides by tomorrow so we can merge the decks."),
            day(9, 27)
        );
    }

    #[test]
    fn before_an_event_on_a_day_is_that_day() {
        assert_eq!(
            due("please review it before the release on friday."),
            day(10, 2)
        );
        assert_eq!(
            due("please read the attached memo before our call on tuesday."),
            day(9, 29)
        );
    }

    #[test]
    fn the_end_of_the_day_the_week_and_the_month() {
        assert_eq!(due("please reply to them by end of day."), day(9, 26));
        assert_eq!(due("please send it by eod"), day(9, 26));
        assert_eq!(due("please send it by the end of the week"), day(10, 2));
        assert_eq!(
            due("send your abstracts by the end of the month"),
            day(9, 30)
        );
    }

    #[test]
    fn a_when_is_not_a_deadline() {
        // "On Monday says when, not by when" (the dataset's rule 4).
        assert_eq!(due("don't forget to bring your passport on monday."), None);
        assert_eq!(due("please send it by email on friday."), None);
        assert_eq!(due("please hold it until friday."), None);
    }

    #[test]
    fn a_by_that_names_no_day_is_no_deadline() {
        assert_eq!(due("please return them by then if you've finished."), None);
        assert_eq!(due("please raise the budget by 10 percent."), None);
        assert_eq!(
            due("check the numbers before the board meeting, they look off."),
            None
        );
        assert_eq!(due("make sure you add it before you send it out."), None);
        assert_eq!(due("by the way, send me the file."), None);
        assert_eq!(due("goodbye friday"), None);
    }

    #[test]
    fn a_deadline_already_past_is_none() {
        // Sent at noon: 9am today has gone by, and nothing can be due then.
        assert_eq!(due("please send it by 9am today"), None);
    }
}
