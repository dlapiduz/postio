//! Forgiving date parsing for `before:` and `after:`.
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

use chrono::{Datelike, Days, Months, NaiveDate};

/// Parses a date value against a reference date. Returns `None` for anything
/// not (yet) recognizable.
pub(crate) fn parse_date(value: &str, today: NaiveDate) -> Option<NaiveDate> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    relative(value, today).or_else(|| calendar(value, today))
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
