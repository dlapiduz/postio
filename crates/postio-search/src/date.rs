//! Forgiving date parsing: `parse_date` (private) reads a day for `before:`/`after:`;
//! [`parse_when`] reads a day and time forward from now. They share month names and nothing else:
//! `sep 1` in late September is last year's for `after:` and next year's for a snooze picker.
//! Everything resolves against a caller-supplied `today`, never the clock; anything unrecognized is
//! `None`, which the parser turns into a [`crate::query::Partial`] rather than an error.
//! A date without a year resolves to its most recent occurrence that is not in the future.

use chrono::{DateTime, Datelike, Days, Months, NaiveDate, TimeZone};

pub(crate) fn parse_date(value: &str, today: NaiveDate) -> Option<NaiveDate> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    relative(value, today).or_else(|| calendar(value, today))
}

/// Resolves a typed time forward from `now`: what the text leaves open is the first instant after
/// `now` that fits (`tue` on a Tuesday afternoon is next week's; `sep 1` in late September is next
/// year's), and a time pinned to the past is `None`. A day with no time is 08:00; `in N days` keeps
/// the typed time of day; a bare number after a day is a 24-hour hour; numeric dates are month
/// first. A wall-clock time the clocks skip is pushed forward by the gap and a repeated one is its
/// first occurrence, never a panic; `now`'s zone is a parameter so tests can use one with
/// transitions. Pure.
pub fn parse_when<Tz: TimeZone>(text: &str, now: DateTime<Tz>) -> Option<DateTime<Tz>> {
    let pieces = pieces(text)?;
    let when = When::read(&pieces)?;
    when.resolve(now)
}

const MORNING: (u32, u32) = (8, 0);
const AFTERNOON: (u32, u32) = (13, 0);
const EVENING: (u32, u32) = (18, 0);
const NOON: (u32, u32) = (12, 0);

#[derive(Debug, Clone, PartialEq)]
enum Piece {
    Word(String),
    Number(u32),
    Clock(u32, u32),
    Numeric(Vec<u32>),
}

fn pieces(text: &str) -> Option<Vec<Piece>> {
    let mut out = Vec::new();
    for word in text.split(|c: char| c.is_whitespace() || c == ',') {
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

pub(crate) fn weekday_from_name(name: &str) -> Option<chrono::Weekday> {
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

fn is_ordinal(piece: Option<&Piece>) -> bool {
    matches!(piece, Some(Piece::Word(w)) if matches!(w.as_str(), "st" | "nd" | "rd" | "th"))
}

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

#[derive(Debug, Clone, Copy, PartialEq)]
enum Day {
    Today,
    Tomorrow,
    NextWeek,
    Date {
        month: u32,
        day: u32,
        year: Option<i32>,
    },
    DaysOn(u64),
    MonthsOn(u32),
}

/// A typed time, read but not yet resolved. Every slot is filled at most
/// once: `tue tue` and `9am 10am` say two things and mean neither.
#[derive(Debug, Default)]
struct When {
    day: Option<Day>,
    weekday: Option<chrono::Weekday>,
    time: Option<chrono::NaiveTime>,
    part: Option<chrono::NaiveTime>,
    span: Option<chrono::Duration>,
}

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
                let month_at = if is_ordinal(next) { i + 2 } else { i + 1 };
                if let Some(Piece::Word(word)) = pieces.get(month_at)
                    && let Some(month) = month_from_name(word)
                {
                    return self.take_date(pieces, month, *number, month_at + 1);
                }
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

    fn take_date(&mut self, pieces: &[Piece], month: u32, day: u32, at: usize) -> Option<usize> {
        let (year, next) = match pieces.get(at) {
            Some(Piece::Number(year)) if *year >= 1000 => (Some(*year as i32), at + 1),
            _ => (None, at),
        };
        fill(&mut self.day, Day::Date { month, day, year })?;
        Some(next)
    }

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

pub fn parse_end<Tz: TimeZone>(text: &str, start: DateTime<Tz>) -> Option<DateTime<Tz>> {
    let pieces = pieces(text)?;
    let end = match pieces.first() {
        Some(Piece::Word(word)) if matches!(word.as_str(), "until" | "till" | "to") => {
            until(&pieces[1..], &start)?
        }
        _ => start.clone() + length(&pieces)?,
    };
    (end > start).then_some(end)
}

fn length(pieces: &[Piece]) -> Option<chrono::Duration> {
    let mut total = chrono::Duration::zero();
    let mut rest = pieces;
    while let Some(Piece::Number(amount)) = rest.first() {
        let unit = match rest.get(1) {
            Some(Piece::Word(unit)) => unit.as_str(),
            None if total > chrono::Duration::zero() => "m",
            _ => return None,
        };
        let amount = i64::from(*amount);
        total += match unit {
            "h" | "hr" | "hrs" | "hour" | "hours" => chrono::Duration::hours(amount),
            "m" | "min" | "mins" | "minute" | "minutes" => chrono::Duration::minutes(amount),
            "d" | "day" | "days" => chrono::Duration::days(amount),
            _ => return None,
        };
        rest = rest.get(2..).unwrap_or_default();
    }
    (rest.is_empty() && total > chrono::Duration::zero()).then_some(total)
}

fn until<Tz: TimeZone>(pieces: &[Piece], start: &DateTime<Tz>) -> Option<DateTime<Tz>> {
    let (hour, minute) = match pieces.first()? {
        Piece::Number(hour) => (*hour, 0),
        Piece::Clock(hour, minute) => (*hour, *minute),
        _ => return parse_when(&words(pieces), start.clone()),
    };
    let day = start.naive_local().date();
    let at = |time: chrono::NaiveTime| resolve_local(&start.timezone(), day.and_time(time), start);
    match pieces.get(1) {
        None => [hour, hour + 12]
            .into_iter()
            .filter_map(|hour| clock(hour, minute, None))
            .filter_map(at)
            .find(|end| end > start),
        next if pieces.len() == 2 => at(clock(hour, minute, Some(meridiem(next)?))?),
        _ => parse_when(&words(pieces), start.clone()),
    }
}

fn words(pieces: &[Piece]) -> String {
    pieces
        .iter()
        .map(|piece| match piece {
            Piece::Word(word) => word.clone(),
            Piece::Number(number) => number.to_string(),
            Piece::Clock(hour, minute) => format!("{hour}:{minute:02}"),
            Piece::Numeric(parts) => parts
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("/"),
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// `local` in `zone`, as a real instant even when the clocks skip or repeat it: a skipped time is
/// pushed forward by the gap, a repeated one is its first occurrence still after `now`. Public so
/// everything turning a wall-clock time into an instant does it one way. `Some` always.
pub fn resolve_local<Tz: TimeZone>(
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

enum Run {
    Word(String),
    Number(u32),
}

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

fn numeric_date(numbers: &[u32], today: NaiveDate) -> Option<NaiveDate> {
    match numbers {
        [packed] if (10_000_101..=99_991_231).contains(packed) => {
            let year = packed / 10_000;
            let month = (packed / 100) % 100;
            let day = packed % 100;
            NaiveDate::from_ymd_opt(year as i32, month, day)
        }
        [first, second] if *first >= 1000 => NaiveDate::from_ymd_opt(*first as i32, *second, 1),
        [month, day] => infer_year(*month, *day, today),
        [first, second, third] if *first >= 1000 => {
            NaiveDate::from_ymd_opt(*first as i32, *second, *third)
        }
        [month, day, year] => NaiveDate::from_ymd_opt(full_year(*year), *month, *day),
        _ => None,
    }
}

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

pub(crate) fn month_from_name(name: &str) -> Option<u32> {
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

fn infer_year(month: u32, day: u32, today: NaiveDate) -> Option<NaiveDate> {
    let candidate = NaiveDate::from_ymd_opt(today.year(), month, day)?;
    if candidate <= today {
        Some(candidate)
    } else {
        NaiveDate::from_ymd_opt(today.year() - 1, month, day)
    }
}

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

    use chrono::{
        DateTime, Duration, FixedOffset, Local, MappedLocalTime, NaiveDate, NaiveDateTime, TimeZone,
    };

    use super::parse_when;

    /// CET until 29 March 01:00 UTC, CEST until 25 October 01:00 UTC: 02:30 on 29 March never
    /// happens, 02:30 on 25 October happens twice.
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

    fn at(y: i32, m: u32, d: u32, h: u32, min: u32) -> DateTime<Cet2026> {
        Cet2026
            .with_ymd_and_hms(y, m, d, h, min, 0)
            .single()
            .expect("an unambiguous wall-clock time")
    }

    fn saturday() -> DateTime<Cet2026> {
        at(2026, 9, 26, 16, 9)
    }

    #[test]
    fn a_typed_time_means_the_next_instant_it_names() {
        let now = saturday();
        let table: &[(&str, DateTime<Cet2026>)] = &[
            ("tue 9am", at(2026, 9, 29, 9, 0)),
            ("thu 2pm", at(2026, 10, 1, 14, 0)),
            ("tomorrow 8", at(2026, 9, 27, 8, 0)),
            ("in 2 days", at(2026, 9, 28, 16, 9)),
            ("oct 3 14:00", at(2026, 10, 3, 14, 0)),
            ("Tue 9AM", at(2026, 9, 29, 9, 0)),
            ("tuesday 9:30am", at(2026, 9, 29, 9, 30)),
            ("tue at 9am", at(2026, 9, 29, 9, 0)),
            ("9am tue", at(2026, 9, 29, 9, 0)),
            ("tue 9", at(2026, 9, 29, 9, 0)),
            ("tue 9a", at(2026, 9, 29, 9, 0)),
            ("tomorrow 2p", at(2026, 9, 27, 14, 0)),
            ("next tue 9am", at(2026, 9, 29, 9, 0)),
            ("tue", at(2026, 9, 29, 8, 0)),
            ("tomorrow", at(2026, 9, 27, 8, 0)),
            ("oct 3", at(2026, 10, 3, 8, 0)),
            ("sat 5pm", at(2026, 9, 26, 17, 0)),
            ("sat", at(2026, 10, 3, 8, 0)),
            ("6pm", at(2026, 9, 26, 18, 0)),
            ("18:30", at(2026, 9, 26, 18, 30)),
            ("9am", at(2026, 9, 27, 9, 0)),
            ("noon", at(2026, 9, 27, 12, 0)),
            ("12pm tomorrow", at(2026, 9, 27, 12, 0)),
            ("12am tomorrow", at(2026, 9, 27, 0, 0)),
            ("tomorrow morning", at(2026, 9, 27, 8, 0)),
            ("monday evening", at(2026, 9, 28, 18, 0)),
            ("tonight", at(2026, 9, 26, 18, 0)),
            ("today 5pm", at(2026, 9, 26, 17, 0)),
            ("in 3 hours", at(2026, 9, 26, 19, 9)),
            ("in 90 minutes", at(2026, 9, 26, 17, 39)),
            ("in 2h", at(2026, 9, 26, 18, 9)),
            ("in a week", at(2026, 10, 3, 16, 9)),
            ("in 2 days at 9am", at(2026, 9, 28, 9, 0)),
            ("next week", at(2026, 10, 3, 8, 0)),
            ("3 oct 14:00", at(2026, 10, 3, 14, 0)),
            ("october 3rd, 2pm", at(2026, 10, 3, 14, 0)),
            ("sep 1", at(2027, 9, 1, 8, 0)),
            ("10/3 9:30am", at(2026, 10, 3, 9, 30)),
            ("2026-10-03 14:00", at(2026, 10, 3, 14, 0)),
            ("oct 3 2027", at(2027, 10, 3, 8, 0)),
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
            "today 9am",
            "2026-09-01 10:00",
            "2025-01-01",
        ] {
            assert_eq!(parse_when(text, now), None, "{text:?}");
        }
    }

    #[test]
    fn a_time_the_clocks_skip_lands_on_the_real_instant_after_the_gap() {
        let now = at(2026, 3, 27, 12, 0);
        let when = parse_when("mar 29 2:30", now).expect("a real instant");
        assert_eq!(when, Cet2026.from_utc_datetime(&utc(2026, 3, 29, 1, 30)));
        assert_eq!(when.naive_local(), utc(2026, 3, 29, 3, 30));
    }

    #[test]
    fn a_time_the_clocks_repeat_is_its_first_occurrence() {
        let now = at(2026, 10, 20, 12, 0);
        let when = parse_when("oct 25 2:30", now).expect("a real instant");
        assert_eq!(when, Cet2026.from_utc_datetime(&utc(2026, 10, 25, 0, 30)));
        assert_eq!(when.naive_local(), utc(2026, 10, 25, 2, 30));
    }

    #[test]
    fn days_across_a_change_keep_the_wall_clock_and_hours_keep_the_duration() {
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

#[cfg(test)]
mod end_tests {
    use chrono::{TimeZone, Utc};

    use super::parse_end;

    fn start() -> chrono::DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 24, 15, 0, 0).unwrap()
    }

    fn end(text: &str) -> Option<String> {
        parse_end(text, start()).map(|end| end.format("%a %H:%M").to_string())
    }

    #[test]
    fn a_length_counts_from_the_start() {
        assert_eq!(end("1h").as_deref(), Some("Tue 16:00"));
        assert_eq!(end("90m").as_deref(), Some("Tue 16:30"));
        assert_eq!(end("1h30").as_deref(), Some("Tue 16:30"));
        assert_eq!(end("1h 15m").as_deref(), Some("Tue 16:15"));
        assert_eq!(end("2 hours").as_deref(), Some("Tue 17:00"));
        assert_eq!(end("45 min").as_deref(), Some("Tue 15:45"));
    }

    #[test]
    fn until_a_bare_hour_is_the_first_one_after_the_start() {
        assert_eq!(end("until 4").as_deref(), Some("Tue 16:00"));
        assert_eq!(end("till 16").as_deref(), Some("Tue 16:00"));
        assert_eq!(end("until 4:30pm").as_deref(), Some("Tue 16:30"));
        assert_eq!(end("to thu 5pm").as_deref(), Some("Thu 17:00"));
    }

    #[test]
    fn nothing_that_ends_before_it_starts_or_is_not_a_length() {
        assert_eq!(
            end("until 2"),
            None,
            "02:00 and 14:00 have both passed by 15:00"
        );
        assert_eq!(end("until 2pm"), None);
        assert_eq!(end("0m"), None);
        assert_eq!(end("1x"), None);
        assert_eq!(end("soon"), None);
        assert_eq!(end("30"), None, "a bare number names no unit");
    }
}
