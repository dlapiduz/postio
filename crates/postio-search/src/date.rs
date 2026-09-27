//! Forgiving date parsing, in two directions.
//!
//! * **Looking back**, for `before:` and `after:`: `parse_date`, private to
//!   the parser, which reads a day and never a time. Below.
//! * **Looking forward**, for a picker's typed time and a due date:
//!   [`parse_when`], public, which reads a day *and* a time of day and
//!   resolves them to an instant in a zone. Its rules are on the function.
//!
//! They share the month names and nothing else, because they answer
//! opposite questions: `sep 1` typed into `after:` in late September is the
//! one just gone, and typed into a snooze picker it is next year's.
//!
//! # `before:` and `after:`
//!
//! Three families are accepted, in this order:
//!
//! 1. **Relative** — `today`, `yesterday`, `last week`, `last-quarter`,
//!    `3 days ago`, `7d`, `2w`, `3m`, `1y`.
//! 2. **Loose calendar** — `aug1`, `Aug 1`, `1aug`, `aug1,2025`, `august`,
//!    `8/1`, `1/2/2026`.
//! 3. **ISO-ish** — `2026-01-01`, `2026/01/01`, `2026.1.1`, `20260101`.
//!
//! Everything is resolved against a caller-supplied `today`, never the clock:
//! that is what keeps the parser pure and the relative-date tests deterministic.
//! Anything unrecognized returns `None`, which the parser turns into a
//! [`crate::query::Partial`] rather than an error — the user is probably still
//! typing.
//!
//! A date without a year resolves to its most recent occurrence that is not in
//! the future, so in August 2026 `aug1` is this year and `sep1` is last year.

use chrono::{DateTime, Datelike, Days, Months, NaiveDate, TimeZone};

/// Parses a date value against a reference date. Returns `None` for anything
/// not (yet) recognizable.
pub(crate) fn parse_date(value: &str, today: NaiveDate) -> Option<NaiveDate> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    relative(value, today).or_else(|| calendar(value, today))
}

/// The instant a typed time means, looking forward from `now`: what a
/// snooze or remind picker's date field reads (research R6), and what the
/// needs-action detector reads a due date with (R10).
///
/// `tue 9am`, `thu 2pm`, `tomorrow 8`, `in 2 days`, `oct 3 14:00`,
/// `Tue 29 Sep 09:00`, `tonight`, `in 3 hours`, `next week`. Case, commas
/// and the words `at`, `on`, `by`, `this` and `next` do not matter.
///
/// **Forward-looking.** Whatever the text leaves open resolves to the first
/// instant after `now` that fits it: `tue` on a Tuesday afternoon is next
/// week's, `9am` in the evening is tomorrow's, and `sep 1` in late September
/// is next year's. A time the text pins to the past -- `today 9am` at noon,
/// a date with its year gone by -- is `None`, because nothing can be snoozed
/// into it.
///
/// **What fills a gap.** A day with no time is its morning, 08:00, the time
/// the snooze presets' mornings use (`postio_ui::schedule`). A span of days
/// (`in 2 days`) keeps the time of day it was typed at, as a wall clock
/// does; a span of hours or minutes is exact. A bare number after a day is
/// an hour on the 24-hour clock (`tomorrow 8` is 08:00); `am` and `pm`, or
/// `a` and `p`, make it the 12-hour one. A numeric date is month first
/// (`10/3` is 3 October), as `after:` reads one.
///
/// **Daylight saving.** A wall-clock time the clocks skip lands where the
/// clocks do, pushed forward by the gap (02:30 becomes 03:30), and one they
/// repeat is its first occurrence. Either way the answer is a real instant,
/// never a panic -- which is why `now`'s zone is a parameter rather than
/// `Local`: it is `Local` for every caller, and a zone with transitions for
/// the tests that prove this.
///
/// Anything else -- a word it does not know, a time given twice, a weekday
/// that contradicts its date, 13pm -- is `None`. Pure: it reads no clock.
pub fn parse_when<Tz: TimeZone>(text: &str, now: DateTime<Tz>) -> Option<DateTime<Tz>> {
    let pieces = pieces(text)?;
    let when = When::read(&pieces)?;
    when.resolve(now)
}

/// When a day is named with no time: the presets' morning.
const MORNING: (u32, u32) = (8, 0);
/// `afternoon`.
const AFTERNOON: (u32, u32) = (13, 0);
/// `evening` and `tonight`: the presets' "Later today".
const EVENING: (u32, u32) = (18, 0);
/// `noon` and `midday`.
const NOON: (u32, u32) = (12, 0);

/// One lexical piece of a typed time.
#[derive(Debug, Clone, PartialEq)]
enum Piece {
    /// Letters, lowercased: `tue`, `am`, `october`, `rd`.
    Word(String),
    /// Digits alone: `9`, `3`, `2027`.
    Number(u32),
    /// `9:30`, `14:00`.
    Clock(u32, u32),
    /// `2026-10-03`, `10/3`, `10/3/26`.
    Numeric(Vec<u32>),
}

/// Splits `text` into [`Piece`]s: whitespace and commas separate words, and
/// within a word letters and digits separate too, so `9am` is `9` and `am`
/// and `oct3` is `oct` and `3`. `None` when a character belongs to none of
/// them.
fn pieces(text: &str) -> Option<Vec<Piece>> {
    let mut out = Vec::new();
    for word in text.split(|c: char| c.is_whitespace() || c == ',') {
        // A sentence's full stop, typed or pasted, is not part of a time.
        let word = word.trim_end_matches('.').to_lowercase();
        let mut chars = word.chars().peekable();
        while let Some(&first) = chars.peek() {
            let mut run = String::new();
            if first.is_ascii_digit() {
                while let Some(&c) = chars.peek() {
                    if !(c.is_ascii_digit() || matches!(c, ':' | '/' | '-')) {
                        break;
                    }
                    run.push(c);
                    chars.next();
                }
                out.push(numeric_piece(&run)?);
            } else if first.is_alphabetic() {
                while let Some(&c) = chars.peek() {
                    if !c.is_alphabetic() {
                        break;
                    }
                    run.push(c);
                    chars.next();
                }
                out.push(Piece::Word(run));
            } else {
                return None;
            }
        }
    }
    Some(out)
}

/// A run of digits and separators: a number, a clock time or a date.
fn numeric_piece(run: &str) -> Option<Piece> {
    let number = |part: &str| -> Option<u32> {
        (!part.is_empty() && part.len() <= 4)
            .then(|| part.parse().ok())
            .flatten()
    };
    if let Some((hour, minute)) = run.split_once(':') {
        if minute.len() != 2 {
            return None;
        }
        return Some(Piece::Clock(number(hour)?, number(minute)?));
    }
    for separator in ['-', '/'] {
        if run.contains(separator) {
            let parts: Option<Vec<u32>> = run.split(separator).map(number).collect();
            return parts.map(Piece::Numeric);
        }
    }
    number(run).map(Piece::Number)
}

/// A weekday by any of the names people type for it.
fn weekday_from_name(name: &str) -> Option<chrono::Weekday> {
    use chrono::Weekday;
    Some(match name {
        "mon" | "monday" => Weekday::Mon,
        "tue" | "tues" | "tuesday" => Weekday::Tue,
        "wed" | "weds" | "wednesday" => Weekday::Wed,
        "thu" | "thur" | "thurs" | "thursday" => Weekday::Thu,
        "fri" | "friday" => Weekday::Fri,
        "sat" | "saturday" => Weekday::Sat,
        "sun" | "sunday" => Weekday::Sun,
        _ => return None,
    })
}

/// Whether `piece` is an ordinal's suffix: the `rd` of `3rd`.
fn is_ordinal(piece: Option<&Piece>) -> bool {
    matches!(piece, Some(Piece::Word(w)) if matches!(w.as_str(), "st" | "nd" | "rd" | "th"))
}

/// `am` or `pm` (or `a` or `p`), as whether the hour is after noon.
fn meridiem(piece: Option<&Piece>) -> Option<bool> {
    match piece {
        Some(Piece::Word(w)) => match w.as_str() {
            "am" | "a" => Some(false),
            "pm" | "p" => Some(true),
            _ => None,
        },
        _ => None,
    }
}

/// A clock time, on the 12-hour clock when `after_noon` says which half.
fn clock(hour: u32, minute: u32, after_noon: Option<bool>) -> Option<chrono::NaiveTime> {
    let hour = match after_noon {
        None => hour,
        Some(_) if !(1..=12).contains(&hour) => return None,
        Some(false) => hour % 12,
        Some(true) => hour % 12 + 12,
    };
    chrono::NaiveTime::from_hms_opt(hour, minute, 0)
}

fn at_clock((hour, minute): (u32, u32)) -> chrono::NaiveTime {
    chrono::NaiveTime::from_hms_opt(hour, minute, 0).expect("a constant clock time")
}

/// Which day a typed time names, before it is resolved against a clock.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Day {
    Today,
    Tomorrow,
    /// `next week`: this weekday, seven days on.
    NextWeek,
    /// `oct 3`, `2026-10-03`: a calendar date, the year perhaps left open.
    Date {
        month: u32,
        day: u32,
        year: Option<i32>,
    },
    /// `in 2 days`: whole days on, keeping the time of day.
    DaysOn(u64),
    /// `in 2 months`.
    MonthsOn(u32),
}

/// A typed time, read but not yet resolved. Every slot is filled at most
/// once: `tue tue` and `9am 10am` say two things and mean neither.
#[derive(Debug, Default)]
struct When {
    day: Option<Day>,
    weekday: Option<chrono::Weekday>,
    time: Option<chrono::NaiveTime>,
    /// `morning`, `evening`: a time that an exact one overrides.
    part: Option<chrono::NaiveTime>,
    /// `in 3 hours`: an exact span from now, and nothing else.
    span: Option<chrono::Duration>,
}

/// Fills an empty slot, or fails if it was already filled.
fn fill<T>(slot: &mut Option<T>, value: T) -> Option<()> {
    match slot {
        Some(_) => None,
        None => {
            *slot = Some(value);
            Some(())
        }
    }
}

impl When {
    fn read(pieces: &[Piece]) -> Option<When> {
        let mut when = When::default();
        let mut i = 0;
        while i < pieces.len() {
            i = when.take(pieces, i)?;
        }
        Some(when)
    }

    /// Reads the piece at `i` (and whatever belongs with it), returning the
    /// index of the next unread piece.
    fn take(&mut self, pieces: &[Piece], i: usize) -> Option<usize> {
        let next = pieces.get(i + 1);
        match &pieces[i] {
            Piece::Word(word) => match word.as_str() {
                "next" if matches!(next, Some(Piece::Word(w)) if w == "week") => {
                    fill(&mut self.day, Day::NextWeek)?;
                    Some(i + 2)
                }
                "at" | "on" | "by" | "this" | "next" => Some(i + 1),
                "today" => fill(&mut self.day, Day::Today).map(|()| i + 1),
                "tonight" => {
                    fill(&mut self.day, Day::Today)?;
                    fill(&mut self.part, at_clock(EVENING))?;
                    Some(i + 1)
                }
                "tomorrow" | "tmrw" => fill(&mut self.day, Day::Tomorrow).map(|()| i + 1),
                "morning" => fill(&mut self.part, at_clock(MORNING)).map(|()| i + 1),
                "afternoon" => fill(&mut self.part, at_clock(AFTERNOON)).map(|()| i + 1),
                "evening" => fill(&mut self.part, at_clock(EVENING)).map(|()| i + 1),
                "noon" | "midday" => fill(&mut self.time, at_clock(NOON)).map(|()| i + 1),
                "in" => self.take_span(pieces, i + 1),
                other => {
                    if let Some(weekday) = weekday_from_name(other) {
                        fill(&mut self.weekday, weekday)?;
                        return Some(i + 1);
                    }
                    let month = month_from_name(other)?;
                    // `oct 3`, `oct 3rd`, `oct 3 2027`.
                    let Some(Piece::Number(day)) = next else {
                        return None;
                    };
                    let mut after = i + 2;
                    if is_ordinal(pieces.get(after)) {
                        after += 1;
                    }
                    self.take_date(pieces, month, *day, after)
                }
            },
            Piece::Number(number) => {
                // `3 oct`, `3rd oct`: a day of the month.
                let month_at = if is_ordinal(next) { i + 2 } else { i + 1 };
                if let Some(Piece::Word(word)) = pieces.get(month_at)
                    && let Some(month) = month_from_name(word)
                {
                    return self.take_date(pieces, month, *number, month_at + 1);
                }
                // Otherwise an hour: `9am`, `tomorrow 8`.
                let after_noon = meridiem(next);
                fill(&mut self.time, clock(*number, 0, after_noon)?)?;
                Some(i + 1 + usize::from(after_noon.is_some()))
            }
            Piece::Clock(hour, minute) => {
                let after_noon = meridiem(next);
                fill(&mut self.time, clock(*hour, *minute, after_noon)?)?;
                Some(i + 1 + usize::from(after_noon.is_some()))
            }
            Piece::Numeric(parts) => {
                let day = match parts.as_slice() {
                    [year, month, day] if *year >= 1000 => Day::Date {
                        month: *month,
                        day: *day,
                        year: Some(*year as i32),
                    },
                    [month, day] => Day::Date {
                        month: *month,
                        day: *day,
                        year: None,
                    },
                    [month, day, year] => Day::Date {
                        month: *month,
                        day: *day,
                        year: Some(full_year(*year)),
                    },
                    _ => return None,
                };
                fill(&mut self.day, day).map(|()| i + 1)
            }
        }
    }

    /// A calendar date whose day and month are read, with a year perhaps at
    /// `at`.
    fn take_date(&mut self, pieces: &[Piece], month: u32, day: u32, at: usize) -> Option<usize> {
        let (year, next) = match pieces.get(at) {
            Some(Piece::Number(year)) if *year >= 1000 => (Some(*year as i32), at + 1),
            _ => (None, at),
        };
        fill(&mut self.day, Day::Date { month, day, year })?;
        Some(next)
    }

    /// `in 3 hours`, `in 2 days`, `in a week`: the count and unit at `at`.
    fn take_span(&mut self, pieces: &[Piece], at: usize) -> Option<usize> {
        let count = match pieces.get(at)? {
            Piece::Number(count) => *count,
            Piece::Word(word) if word == "a" || word == "an" => 1,
            _ => return None,
        };
        let Piece::Word(unit) = pieces.get(at + 1)? else {
            return None;
        };
        let count64 = u64::from(count);
        match unit.as_str() {
            "min" | "mins" | "minute" | "minutes" => {
                fill(&mut self.span, chrono::Duration::minutes(i64::from(count)))?
            }
            "h" | "hr" | "hrs" | "hour" | "hours" => {
                fill(&mut self.span, chrono::Duration::hours(i64::from(count)))?
            }
            "d" | "day" | "days" => fill(&mut self.day, Day::DaysOn(count64))?,
            "w" | "wk" | "wks" | "week" | "weeks" => {
                fill(&mut self.day, Day::DaysOn(count64.checked_mul(7)?))?
            }
            "month" | "months" => fill(&mut self.day, Day::MonthsOn(count))?,
            _ => return None,
        }
        Some(at + 2)
    }

    /// The first instant after `now` that this names, if there is one.
    fn resolve<Tz: TimeZone>(self, now: DateTime<Tz>) -> Option<DateTime<Tz>> {
        if let Some(span) = self.span {
            let alone = self.day.is_none()
                && self.weekday.is_none()
                && self.time.is_none()
                && self.part.is_none();
            let when = now.clone().checked_add_signed(span)?;
            return (alone && when > now).then_some(when);
        }

        let zone = now.timezone();
        let wall = now.naive_local();
        let today = wall.date();
        let named = self.time.or(self.part);
        let time = named.unwrap_or_else(|| at_clock(MORNING));
        let ahead = |date: NaiveDate, time: chrono::NaiveTime| {
            resolve_local(&zone, date.and_time(time), &now).filter(|when| *when > now)
        };

        match (self.day, self.weekday) {
            // A time alone: the next time the clock shows it.
            (None, None) => {
                let time = named?;
                ahead(today, time).or_else(|| ahead(today.succ_opt()?, time))
            }
            (None, Some(weekday)) => (0..=7)
                .filter_map(|n| today.checked_add_days(Days::new(n)))
                .filter(|date| date.weekday() == weekday)
                .find_map(|date| ahead(date, time)),
            (Some(day), weekday) => {
                let date = match day {
                    Day::Today => today,
                    Day::Tomorrow => today.succ_opt()?,
                    Day::NextWeek => today.checked_add_days(Days::new(7))?,
                    Day::DaysOn(days) => today.checked_add_days(Days::new(days))?,
                    Day::MonthsOn(months) => today.checked_add_months(Months::new(months))?,
                    Day::Date {
                        month,
                        day,
                        year: Some(year),
                    } => NaiveDate::from_ymd_opt(year, month, day)?,
                    // The first year in which it is still ahead. Eight, for
                    // the 29th of February.
                    Day::Date {
                        month,
                        day,
                        year: None,
                    } => (today.year()..=today.year() + 8)
                        .filter_map(|year| NaiveDate::from_ymd_opt(year, month, day))
                        .find(|date| ahead(*date, time).is_some())?,
                };
                // `Tue 29 Sep`: the weekday must agree with the date.
                if weekday.is_some_and(|weekday| weekday != date.weekday()) {
                    return None;
                }
                let time = match day {
                    Day::DaysOn(_) | Day::MonthsOn(_) => named.unwrap_or(wall.time()),
                    _ => time,
                };
                ahead(date, time)
            }
        }
    }
}

/// `local` in `zone`, as a real instant even when the clocks skip or repeat
/// it: a skipped time is pushed forward by the gap, and a repeated one is its
/// first occurrence still after `now`.
fn resolve_local<Tz: TimeZone>(
    zone: &Tz,
    local: chrono::NaiveDateTime,
    now: &DateTime<Tz>,
) -> Option<DateTime<Tz>> {
    use chrono::{MappedLocalTime, Offset as _};
    match zone.from_local_datetime(&local) {
        MappedLocalTime::Single(when) => Some(when),
        MappedLocalTime::Ambiguous(earliest, latest) => {
            Some(if earliest > *now { earliest } else { latest })
        }
        MappedLocalTime::None => {
            // Read through the offset in force before the gap, which is
            // where a wall clock that jumped forward would put it.
            let before = zone
                .offset_from_utc_datetime(&(local - chrono::Duration::days(1)))
                .fix();
            let utc = local - chrono::Duration::seconds(i64::from(before.local_minus_utc()));
            Some(zone.from_utc_datetime(&utc))
        }
    }
}

/// `today`, `yesterday`, `last week`, `3 days ago`, `7d`, ...
fn relative(value: &str, today: NaiveDate) -> Option<NaiveDate> {
    let compact: String = value
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-' && *c != '_')
        .flat_map(|c| c.to_lowercase())
        .collect();

    match compact.as_str() {
        "today" | "now" => return Some(today),
        "yesterday" => return today.checked_sub_days(Days::new(1)),
        _ => {}
    }

    let mut rest = compact.as_str();
    rest = rest.strip_suffix("ago").unwrap_or(rest);
    for prefix in ["last", "past", "previous"] {
        if let Some(stripped) = rest.strip_prefix(prefix) {
            rest = stripped;
            break;
        }
    }

    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    let unit = &rest[digits.len()..];
    let count: u32 = if digits.is_empty() {
        1
    } else {
        digits.parse().ok()?
    };

    match unit {
        "d" | "day" | "days" => today.checked_sub_days(Days::new(u64::from(count))),
        "w" | "week" | "weeks" => today.checked_sub_days(Days::new(u64::from(count) * 7)),
        "m" | "month" | "months" => today.checked_sub_months(Months::new(count)),
        "q" | "quarter" | "quarters" => today.checked_sub_months(Months::new(count * 3)),
        "y" | "year" | "years" => today.checked_sub_months(Months::new(count * 12)),
        _ => None,
    }
}

/// A run of characters of one kind inside a loose date.
enum Run {
    Word(String),
    Number(u32),
}

/// `aug1`, `1 august 2025`, `8/1`, `2026-01-01`, `20260101`, ...
fn calendar(value: &str, today: NaiveDate) -> Option<NaiveDate> {
    let runs = split_runs(value)?;
    let mut words = Vec::new();
    let mut numbers = Vec::new();
    for run in &runs {
        match run {
            Run::Word(word) => words.push(word.as_str()),
            Run::Number(number) => numbers.push(*number),
        }
    }

    match words.len() {
        0 => numeric_date(&numbers, today),
        1 => {
            let month = month_from_name(words[0])?;
            match numbers.len() {
                0 => infer_year(month, 1, today),
                1 if numbers[0] >= 1000 => NaiveDate::from_ymd_opt(numbers[0] as i32, month, 1),
                1 => infer_year(month, numbers[0], today),
                2 => {
                    // `aug1,2025` and `1 august 2025`: the four-digit-ish run is
                    // the year whichever side it landed on.
                    let (day, year) = if numbers[0] > 31 {
                        (numbers[1], numbers[0])
                    } else {
                        (numbers[0], numbers[1])
                    };
                    NaiveDate::from_ymd_opt(full_year(year), month, day)
                }
                _ => None,
            }
        }
        _ => None,
    }
}

/// Dates made only of numbers.
fn numeric_date(numbers: &[u32], today: NaiveDate) -> Option<NaiveDate> {
    match numbers {
        // `20260101`. A bare `2026` or `20` is not a date — the user is typing.
        [packed] if (10_000_101..=99_991_231).contains(packed) => {
            let year = packed / 10_000;
            let month = (packed / 100) % 100;
            let day = packed % 100;
            NaiveDate::from_ymd_opt(year as i32, month, day)
        }
        // `2026-08` is that month; `8/1` is a month and a day.
        [first, second] if *first >= 1000 => NaiveDate::from_ymd_opt(*first as i32, *second, 1),
        [month, day] => infer_year(*month, *day, today),
        // `2026-01-02` or `1/2/2026`.
        [first, second, third] if *first >= 1000 => {
            NaiveDate::from_ymd_opt(*first as i32, *second, *third)
        }
        [month, day, year] => NaiveDate::from_ymd_opt(full_year(*year), *month, *day),
        _ => None,
    }
}

/// Splits a loose date into alphabetic and numeric runs, discarding separators.
/// Returns `None` if anything other than letters, digits and the usual
/// separators shows up.
fn split_runs(value: &str) -> Option<Vec<Run>> {
    let mut runs = Vec::new();
    let mut word = String::new();
    let mut number = String::new();

    for ch in value.chars() {
        if ch.is_ascii_digit() {
            flush_word(&mut word, &mut runs);
            number.push(ch);
        } else if ch.is_alphabetic() {
            flush_number(&mut number, &mut runs)?;
            word.extend(ch.to_lowercase());
        } else if matches!(ch, '-' | '/' | '.' | ',' | ' ' | '\t' | '_') {
            flush_word(&mut word, &mut runs);
            flush_number(&mut number, &mut runs)?;
        } else {
            return None;
        }
        if runs.len() > 4 {
            return None;
        }
    }
    flush_word(&mut word, &mut runs);
    flush_number(&mut number, &mut runs)?;

    if runs.is_empty() { None } else { Some(runs) }
}

fn flush_word(word: &mut String, runs: &mut Vec<Run>) {
    if !word.is_empty() {
        runs.push(Run::Word(std::mem::take(word)));
    }
}

fn flush_number(number: &mut String, runs: &mut Vec<Run>) -> Option<()> {
    if !number.is_empty() {
        let parsed = std::mem::take(number).parse().ok()?;
        runs.push(Run::Number(parsed));
    }
    Some(())
}

/// English month names and the abbreviations people actually type.
fn month_from_name(name: &str) -> Option<u32> {
    let month = match name {
        "jan" | "january" => 1,
        "feb" | "february" => 2,
        "mar" | "march" => 3,
        "apr" | "april" => 4,
        "may" => 5,
        "jun" | "june" => 6,
        "jul" | "july" => 7,
        "aug" | "august" => 8,
        "sep" | "sept" | "september" => 9,
        "oct" | "october" => 10,
        "nov" | "november" => 11,
        "dec" | "december" => 12,
        _ => return None,
    };
    Some(month)
}

/// The most recent `month`/`day` that is not in the future.
fn infer_year(month: u32, day: u32, today: NaiveDate) -> Option<NaiveDate> {
    let candidate = NaiveDate::from_ymd_opt(today.year(), month, day)?;
    if candidate <= today {
        Some(candidate)
    } else {
        NaiveDate::from_ymd_opt(today.year() - 1, month, day)
    }
}

/// Expands a two-digit year; leaves anything else alone.
fn full_year(year: u32) -> i32 {
    if year < 100 {
        2000 + year as i32
    } else {
        year as i32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 8, 22).unwrap()
    }

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    #[test]
    fn empty_and_garbage_are_not_dates() {
        for value in ["", "   ", "notadate", "au", "2026-", "20", "aug1aug", "🙂"] {
            assert_eq!(parse_date(value, today()), None, "{value}");
        }
    }

    #[test]
    fn impossible_dates_are_rejected() {
        for value in ["2026-13-45", "2026-02-30", "feb30", "0/0", "99999999999"] {
            assert_eq!(parse_date(value, today()), None, "{value}");
        }
    }

    #[test]
    fn relative_units() {
        assert_eq!(parse_date("today", today()), Some(d(2026, 8, 22)));
        assert_eq!(parse_date("now", today()), Some(d(2026, 8, 22)));
        assert_eq!(parse_date("yesterday", today()), Some(d(2026, 8, 21)));
        assert_eq!(parse_date("last week", today()), Some(d(2026, 8, 15)));
        assert_eq!(parse_date("past 2 weeks", today()), Some(d(2026, 8, 8)));
        assert_eq!(parse_date("previous month", today()), Some(d(2026, 7, 22)));
        // "this month" is ambiguous as a bound; we would rather stay partial.
        assert_eq!(parse_date("this month", today()), None);
        assert_eq!(parse_date("last quarter", today()), Some(d(2026, 5, 22)));
        assert_eq!(parse_date("2 quarters ago", today()), Some(d(2026, 2, 22)));
        assert_eq!(parse_date("10y", today()), Some(d(2016, 8, 22)));
    }

    #[test]
    fn month_end_clamping() {
        assert_eq!(
            parse_date("last month", d(2026, 3, 31)),
            Some(d(2026, 2, 28))
        );
        assert_eq!(parse_date("1y", d(2024, 2, 29)), Some(d(2023, 2, 28)));
    }

    #[test]
    fn two_digit_years_expand() {
        assert_eq!(parse_date("1/2/26", today()), Some(d(2026, 1, 2)));
    }

    #[test]
    fn year_and_month_only() {
        assert_eq!(parse_date("2026-03", today()), Some(d(2026, 3, 1)));
        assert_eq!(parse_date("mar2025", today()), Some(d(2025, 3, 1)));
    }

    #[test]
    fn every_month_name_and_abbreviation_parses() {
        for (name, month) in [
            ("jan", 1),
            ("january", 1),
            ("feb", 2),
            ("mar", 3),
            ("apr", 4),
            ("may", 5),
            ("jun", 6),
            ("jul", 7),
            ("aug", 8),
            ("sep", 9),
            ("sept", 9),
            ("september", 9),
            ("oct", 10),
            ("nov", 11),
            ("dec", 12),
        ] {
            let parsed = parse_date(&format!("{name}5"), today()).unwrap();
            assert_eq!(parsed.month(), month, "{name}");
            assert_eq!(parsed.day(), 5, "{name}");
            assert!(parsed <= today(), "{name} resolved into the future");
        }
    }

    #[test]
    fn too_many_runs_is_not_a_date() {
        assert_eq!(parse_date("1-2-3-4-5", today()), None);
    }
}

#[cfg(test)]
mod when_tests {
    //! `parse_when` against a fixed clock, in a zone that has daylight
    //! saving -- which `Local` cannot promise on whatever machine runs this.

    use chrono::{
        DateTime, Duration, FixedOffset, Local, MappedLocalTime, NaiveDate, NaiveDateTime, TimeZone,
    };

    use super::parse_when;

    /// Central European time for 2026, transitions and all: CET (+01:00)
    /// until 29 March 01:00 UTC, CEST (+02:00) until 25 October 01:00 UTC,
    /// then CET again. So 02:30 on 29 March never happens there, and 02:30
    /// on 25 October happens twice.
    #[derive(Debug, Clone, Copy)]
    struct Cet2026;

    fn utc(y: i32, m: u32, d: u32, h: u32, min: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(y, m, d)
            .and_then(|date| date.and_hms_opt(h, min, 0))
            .expect("a real date")
    }

    fn hours(h: i32) -> FixedOffset {
        FixedOffset::east_opt(h * 3600).expect("a real offset")
    }

    impl TimeZone for Cet2026 {
        type Offset = FixedOffset;

        fn from_offset(_: &FixedOffset) -> Self {
            Cet2026
        }

        fn offset_from_utc_datetime(&self, at: &NaiveDateTime) -> FixedOffset {
            if *at >= utc(2026, 3, 29, 1, 0) && *at < utc(2026, 10, 25, 1, 0) {
                hours(2)
            } else {
                hours(1)
            }
        }

        fn offset_from_utc_date(&self, at: &NaiveDate) -> FixedOffset {
            self.offset_from_utc_datetime(&at.and_hms_opt(0, 0, 0).expect("midnight"))
        }

        fn offset_from_local_datetime(
            &self,
            local: &NaiveDateTime,
        ) -> MappedLocalTime<FixedOffset> {
            // A wall-clock time is valid under an offset when reading it
            // back through that offset lands in the offset's own period.
            let fits = |offset: FixedOffset| {
                let at = *local - Duration::seconds(i64::from(offset.local_minus_utc()));
                self.offset_from_utc_datetime(&at) == offset
            };
            match (fits(hours(2)), fits(hours(1))) {
                (true, true) => MappedLocalTime::Ambiguous(hours(2), hours(1)),
                (true, false) => MappedLocalTime::Single(hours(2)),
                (false, true) => MappedLocalTime::Single(hours(1)),
                (false, false) => MappedLocalTime::None,
            }
        }

        fn offset_from_local_date(&self, local: &NaiveDate) -> MappedLocalTime<FixedOffset> {
            self.offset_from_local_datetime(&local.and_hms_opt(0, 0, 0).expect("midnight"))
        }
    }

    /// A wall-clock time in the test zone, which must exist exactly once.
    fn at(y: i32, m: u32, d: u32, h: u32, min: u32) -> DateTime<Cet2026> {
        Cet2026
            .with_ymd_and_hms(y, m, d, h, min, 0)
            .single()
            .expect("an unambiguous wall-clock time")
    }

    /// Saturday 26 September 2026, 16:09: spec US5's clock.
    fn saturday() -> DateTime<Cet2026> {
        at(2026, 9, 26, 16, 9)
    }

    #[test]
    fn a_typed_time_means_the_next_instant_it_names() {
        let now = saturday();
        let table: &[(&str, DateTime<Cet2026>)] = &[
            // The five the picker's footnote and research R6 name.
            ("tue 9am", at(2026, 9, 29, 9, 0)),
            ("thu 2pm", at(2026, 10, 1, 14, 0)),
            ("tomorrow 8", at(2026, 9, 27, 8, 0)),
            ("in 2 days", at(2026, 9, 28, 16, 9)),
            ("oct 3 14:00", at(2026, 10, 3, 14, 0)),
            // Case, spelling, order and connectives do not matter.
            ("Tue 9AM", at(2026, 9, 29, 9, 0)),
            ("tuesday 9:30am", at(2026, 9, 29, 9, 30)),
            ("tue at 9am", at(2026, 9, 29, 9, 0)),
            ("9am tue", at(2026, 9, 29, 9, 0)),
            ("tue 9", at(2026, 9, 29, 9, 0)),
            ("tue 9a", at(2026, 9, 29, 9, 0)),
            ("tomorrow 2p", at(2026, 9, 27, 14, 0)),
            ("next tue 9am", at(2026, 9, 29, 9, 0)),
            // A day with no time is its morning, as the presets' mornings are.
            ("tue", at(2026, 9, 29, 8, 0)),
            ("tomorrow", at(2026, 9, 27, 8, 0)),
            ("oct 3", at(2026, 10, 3, 8, 0)),
            // Today's weekday: today if the time is still ahead, else a week on.
            ("sat 5pm", at(2026, 9, 26, 17, 0)),
            ("sat", at(2026, 10, 3, 8, 0)),
            // A time with no day: the next time the clock shows it.
            ("6pm", at(2026, 9, 26, 18, 0)),
            ("18:30", at(2026, 9, 26, 18, 30)),
            ("9am", at(2026, 9, 27, 9, 0)),
            ("noon", at(2026, 9, 27, 12, 0)),
            ("12pm tomorrow", at(2026, 9, 27, 12, 0)),
            ("12am tomorrow", at(2026, 9, 27, 0, 0)),
            // Parts of the day, at the presets' own times.
            ("tomorrow morning", at(2026, 9, 27, 8, 0)),
            ("monday evening", at(2026, 9, 28, 18, 0)),
            ("tonight", at(2026, 9, 26, 18, 0)),
            ("today 5pm", at(2026, 9, 26, 17, 0)),
            // Spans from now: hours are exact, days keep the time of day.
            ("in 3 hours", at(2026, 9, 26, 19, 9)),
            ("in 90 minutes", at(2026, 9, 26, 17, 39)),
            ("in 2h", at(2026, 9, 26, 18, 9)),
            ("in a week", at(2026, 10, 3, 16, 9)),
            ("in 2 days at 9am", at(2026, 9, 28, 9, 0)),
            ("next week", at(2026, 10, 3, 8, 0)),
            // Calendar dates, looking forward for the year.
            ("3 oct 14:00", at(2026, 10, 3, 14, 0)),
            ("october 3rd, 2pm", at(2026, 10, 3, 14, 0)),
            ("sep 1", at(2027, 9, 1, 8, 0)),
            ("10/3 9:30am", at(2026, 10, 3, 9, 30)),
            ("2026-10-03 14:00", at(2026, 10, 3, 14, 0)),
            ("oct 3 2027", at(2027, 10, 3, 8, 0)),
            // What the pickers print (spec US5) reads back as itself.
            ("Tue 29 Sep 09:00", at(2026, 9, 29, 9, 0)),
            ("Sat 3 Oct 08:00", at(2026, 10, 3, 8, 0)),
        ];
        for (text, expected) in table {
            assert_eq!(
                parse_when(text, now),
                Some(*expected),
                "{text:?} at Saturday 16:09"
            );
        }
    }

    #[test]
    fn nonsense_and_the_past_are_none() {
        let now = saturday();
        for text in [
            "",
            "   ",
            "banana",
            "tu",
            "tue 25:00",
            "tue 9xm",
            "13pm",
            "0am",
            "feb 30",
            "tue tue",
            "tue 9am 10am",
            "in days",
            "in 2 fortnights",
            "in 0 days",
            "yesterday",
            // A weekday that contradicts its date is a typo, not a choice.
            "mon 29 sep",
            // Named, and already behind the clock.
            "today 9am",
            "2026-09-01 10:00",
            "2025-01-01",
        ] {
            assert_eq!(parse_when(text, now), None, "{text:?}");
        }
    }

    #[test]
    fn a_time_the_clocks_skip_lands_on_the_real_instant_after_the_gap() {
        // 02:30 on 29 March never happens in this zone: 02:00 CET is
        // followed by 03:00 CEST. Pushed forward by the gap, as a wall clock
        // would be, it is 03:30 CEST, 01:30 UTC.
        let now = at(2026, 3, 27, 12, 0);
        let when = parse_when("mar 29 2:30", now).expect("a real instant");
        assert_eq!(when, Cet2026.from_utc_datetime(&utc(2026, 3, 29, 1, 30)));
        assert_eq!(when.naive_local(), utc(2026, 3, 29, 3, 30));
    }

    #[test]
    fn a_time_the_clocks_repeat_is_its_first_occurrence() {
        // 02:30 on 25 October happens twice: at 00:30 UTC in CEST, and an
        // hour later in CET. The first is the one a person means.
        let now = at(2026, 10, 20, 12, 0);
        let when = parse_when("oct 25 2:30", now).expect("a real instant");
        assert_eq!(when, Cet2026.from_utc_datetime(&utc(2026, 10, 25, 0, 30)));
        assert_eq!(when.naive_local(), utc(2026, 10, 25, 2, 30));
    }

    #[test]
    fn days_across_a_change_keep_the_wall_clock_and_hours_keep_the_duration() {
        // Friday 23 October 16:09 CEST; the clocks go back on Sunday.
        let now = at(2026, 10, 23, 16, 9);
        let days = parse_when("in 2 days", now).expect("a real instant");
        assert_eq!(days.naive_local(), utc(2026, 10, 25, 16, 9));
        assert_eq!(days - now, Duration::hours(49));

        let hours = parse_when("in 48 hours", now).expect("a real instant");
        assert_eq!(hours - now, Duration::hours(48));
        assert_eq!(hours.naive_local(), utc(2026, 10, 25, 15, 9));
    }

    #[test]
    fn the_local_zone_is_what_callers_pass() {
        // What the picker hands it; every other case pins a zone instead.
        let now = Local
            .with_ymd_and_hms(2026, 9, 26, 16, 9, 0)
            .single()
            .expect("an unambiguous local time");
        let expected = Local
            .with_ymd_and_hms(2026, 9, 27, 8, 0, 0)
            .single()
            .expect("an unambiguous local time");
        assert_eq!(parse_when("tomorrow 8", now), Some(expected));
    }
}
