//! Plain English lowered into the one query language: the chips are the query, since [`lower`]
//! returns exactly what [`crate::parse`] makes of the lowered text. Deterministic, local, pure
//! (`today` is a parameter).
//! A bare name, month or weekday stays free text, and so does `in` a folder that is not a role:
//! `in:` naming a missing folder selects nothing, so guessing wrong would hide every result.

use chrono::{Datelike, Days, Months, NaiveDate, Weekday};

use crate::ParsedQuery;
use crate::date::{month_from_name, parse_date, weekday_from_name};
use crate::query::{Field, Span};

const NAME_WORDS: usize = 3;

const STOP_WORDS: &[&str] = &[
    "a", "about", "after", "all", "an", "and", "any", "are", "at", "be", "been", "before", "by",
    "email", "emails", "find", "for", "from", "get", "give", "i", "in", "is", "it", "its", "list",
    "mail", "mails", "me", "message", "messages", "my", "of", "on", "or", "our", "please",
    "search", "show", "since", "some", "that", "the", "these", "this", "those", "to", "us", "was",
    "we", "were", "which", "who", "whose", "with", "you", "your",
];

/// `names` is the address book: given a name as typed, what `from:` or `to:` should say, or `None`
/// for an unknown name.
pub fn lower(text: &str, today: NaiveDate, names: &dyn Fn(&str) -> Option<String>) -> ParsedQuery {
    lower_with_origins(text, today, names).query
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lowered {
    pub query: ParsedQuery,
    pub origins: Vec<Origin>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Origin {
    pub token: usize,
    /// `None` when the person typed the token in the language itself.
    pub from: Option<Span>,
    pub words: String,
}

pub fn lower_with_origins(
    text: &str,
    today: NaiveDate,
    names: &dyn Fn(&str) -> Option<String>,
) -> Lowered {
    let words = words(text);
    let lowering = Lowering {
        words: &words,
        today,
        names,
    };
    let mut out = Vec::new();
    let mut sources: Vec<(usize, usize)> = Vec::new();
    let mut at = 0;
    while at < words.len() {
        let written = out.len();
        let next = lowering.step(at, &mut out);
        sources.extend(std::iter::repeat_n((at, next), out.len() - written));
        at = next;
    }

    let mut joined = String::new();
    let mut placed = Vec::with_capacity(out.len());
    for piece in &out {
        if !joined.is_empty() {
            joined.push(' ');
        }
        let start = joined.len();
        joined.push_str(piece);
        placed.push(start..joined.len());
    }
    let query = crate::parse(&joined, today);

    let origins = query
        .tokens()
        .iter()
        .enumerate()
        .map(|(token, parsed)| {
            let piece = placed
                .iter()
                .position(|range| range.contains(&parsed.span.start))
                .unwrap_or(0);
            let (first, next) = sources[piece];
            if words[first].verbatim {
                return Origin {
                    token,
                    from: None,
                    words: parsed.raw.clone(),
                };
            }
            let span = Span::new(words[first].core.start, words[next - 1].core.end);
            Origin {
                token,
                from: Some(span),
                words: text[span.start..span.end].to_owned(),
            }
        })
        .collect();
    Lowered { query, origins }
}

struct Word {
    core: Span,
    raw: String,
    bare: String,
    key: String,
    verbatim: bool,
}

fn words(text: &str) -> Vec<Word> {
    let mut out = Vec::new();
    let mut cursor = 0;
    while let Some(start) = text[cursor..]
        .char_indices()
        .find(|(_, c)| !c.is_whitespace())
        .map(|(offset, _)| cursor + offset)
    {
        let end = crate::parser::word_end(text, start);
        out.push(word(text[start..end].to_owned(), start));
        cursor = end;
    }
    out
}

fn word(raw: String, start: usize) -> Word {
    let punctuation = |c: char| !c.is_alphanumeric();
    let core = match raw.trim_start_matches(punctuation).len() {
        0 => Span::new(start, start),
        rest => {
            let from = start + raw.len() - rest;
            Span::new(from, start + raw.trim_end_matches(punctuation).len())
        }
    };
    let verbatim = raw.starts_with('"')
        || (raw.len() > 1 && raw.starts_with('-'))
        || raw
            .split_once(':')
            .is_some_and(|(keyword, _)| Field::parse(keyword).is_some());
    let bare: String = raw
        .trim_matches(|c: char| !c.is_alphanumeric())
        .chars()
        .filter(|c| *c != '"')
        .collect();
    Word {
        core,
        key: bare.to_lowercase(),
        bare,
        raw,
        verbatim,
    }
}

#[derive(Debug, Clone, Copy)]
struct Period {
    from: NaiveDate,
    until: NaiveDate,
}

impl Period {
    fn day(date: NaiveDate) -> Option<Self> {
        Some(Self {
            from: date,
            until: date.succ_opt()?,
        })
    }

    fn bounds(self) -> Vec<String> {
        vec![
            format!("after:{}", self.from),
            format!("before:{}", self.until),
        ]
    }
}

#[derive(Debug, Clone, Copy)]
enum Unit {
    Week,
    Month,
    Year,
}

struct Lowering<'a> {
    words: &'a [Word],
    today: NaiveDate,
    names: &'a dyn Fn(&str) -> Option<String>,
}

impl Lowering<'_> {
    fn step(&self, at: usize, out: &mut Vec<String>) -> usize {
        let word = &self.words[at];
        if word.verbatim {
            out.push(word.raw.clone());
            return at + 1;
        }
        if let Some((next, operators)) = self.dates(at) {
            out.extend(operators);
            return next;
        }
        if let Some((next, operator)) = self
            .attachments(at)
            .or_else(|| self.folder(at))
            .or_else(|| self.correspondent(at))
        {
            out.push(operator);
            return next;
        }
        match word.key.as_str() {
            "unread" => out.push("is:unread".to_owned()),
            "flagged" | "starred" => out.push("is:flagged".to_owned()),
            key if key.is_empty() || STOP_WORDS.contains(&key) => {}
            _ => out.push(word.bare.clone()),
        }
        at + 1
    }

    fn key(&self, at: usize) -> Option<&str> {
        self.words
            .get(at)
            .filter(|word| !word.verbatim)
            .map(|word| word.key.as_str())
    }

    fn attachments(&self, at: usize) -> Option<(usize, String)> {
        let negated = match self.key(at)? {
            "with" | "has" | "having" => false,
            "without" => true,
            _ => return None,
        };
        if !matches!(self.key(at + 1)?, "attachment" | "attachments") {
            return None;
        }
        let operator = if negated { "-has:attach" } else { "has:attach" };
        Some((at + 2, operator.to_owned()))
    }

    /// A mailbox by its role, which every account has whatever its folders are called.
    fn folder(&self, at: usize) -> Option<(usize, String)> {
        if self.key(at)? != "in" {
            return None;
        }
        let role = match self.key(at + 1)? {
            "inbox" => "inbox",
            "archive" | "archived" => "archive",
            "sent" => "sent",
            "draft" | "drafts" => "drafts",
            "trash" | "bin" | "deleted" => "trash",
            "junk" | "spam" => "junk",
            "outbox" => "outbox",
            "snoozed" => "snoozed",
            _ => return None,
        };
        Some((at + 2, format!("in:{role}")))
    }

    fn correspondent(&self, at: usize) -> Option<(usize, String)> {
        if matches!(self.key(at)?, "sent" | "wrote" | "written")
            && let Some(found) = self.after_cue(at + 1)
        {
            return Some(found);
        }
        if let Some(found) = self.after_cue(at) {
            return Some(found);
        }
        (1..=NAME_WORDS).rev().find_map(|len| {
            let verb = self.key(at + len)?;
            if !matches!(verb, "sent" | "wrote") {
                return None;
            }
            let value = self.name(at, len)?;
            Some((at + len + 1, operator("from", &value)))
        })
    }

    fn after_cue(&self, at: usize) -> Option<(usize, String)> {
        let field = match self.key(at)? {
            "from" | "by" => "from",
            "to" => "to",
            _ => return None,
        };
        let (mut next, first) = self.correspondent_at(at + 1)?;
        let mut values = vec![first];
        while self.key(next) == Some("or")
            && let Some((after, value)) = self.correspondent_at(next + 1)
        {
            values.push(value);
            next = after;
        }
        Some((next, set_operator(field, &values)))
    }

    fn correspondent_at(&self, at: usize) -> Option<(usize, String)> {
        if let Some(found) = (1..=NAME_WORDS)
            .rev()
            .find_map(|len| Some((at + len, self.name(at, len)?)))
        {
            return Some(found);
        }
        let word = self.words.get(at).filter(|word| !word.verbatim)?;
        let (local, domain) = word.bare.split_once('@')?;
        (!local.is_empty() && !domain.is_empty()).then(|| (at + 1, word.bare.clone()))
    }

    fn name(&self, at: usize, len: usize) -> Option<String> {
        let words = self.words.get(at..at + len)?;
        if words
            .iter()
            .any(|word| word.verbatim || word.bare.is_empty())
        {
            return None;
        }
        let name = words
            .iter()
            .map(|word| word.bare.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        (self.names)(&name).filter(|value| !value.trim().is_empty())
    }

    fn dates(&self, at: usize) -> Option<(usize, Vec<String>)> {
        if let Some((next, period)) = self.ago(at) {
            return Some((next, period.bounds()));
        }
        match self.key(at)? {
            "since" => {
                let (next, period) = self.period(at + 1)?;
                Some((next, vec![format!("after:{}", period.from)]))
            }
            "before" => {
                let (next, period) = self.period(at + 1)?;
                Some((next, vec![format!("before:{}", period.from)]))
            }
            "after" => {
                let (next, period) = self.period(at + 1)?;
                Some((next, vec![format!("after:{}", period.until)]))
            }
            "in" => {
                let key = self.key(at + 1)?;
                let period = match (month_from_name(key), season_from_name(key)) {
                    (Some(month), _) => self.month(month)?,
                    (None, Some(season)) => self.season(season, false)?,
                    (None, None) => self.year(key)?,
                };
                Some((at + 2, period.bounds()))
            }
            "on" => {
                let key = self.key(at + 1)?;
                let date = match weekday_from_name(key) {
                    Some(weekday) => self.latest(weekday, false)?,
                    None => parse_date(key, self.today)?,
                };
                Some((at + 2, Period::day(date)?.bounds()))
            }
            "today" | "yesterday" | "this" | "last" => match self.period(at) {
                Some((next, period)) => Some((next, period.bounds())),
                None => self.rolling(at),
            },
            "past" => self.rolling(at),
            _ => None,
        }
    }

    fn period(&self, at: usize) -> Option<(usize, Period)> {
        let key = self.key(at)?;
        match key {
            "today" => return Some((at + 1, Period::day(self.today)?)),
            "yesterday" => return Some((at + 1, Period::day(self.today.pred_opt()?)?)),
            "this" | "last" => {
                let back = key == "last";
                let next = self.key(at + 1)?;
                let unit = match next {
                    "week" => Some(Unit::Week),
                    "month" => Some(Unit::Month),
                    "year" => Some(Unit::Year),
                    _ => None,
                };
                if let Some(unit) = unit {
                    return Some((at + 2, self.calendar(unit, back)?));
                }
                if let Some(season) = season_from_name(next).filter(|_| back) {
                    return Some((at + 2, self.season(season, true)?));
                }
                let weekday = weekday_from_name(next).filter(|_| back)?;
                return Some((at + 2, Period::day(self.latest(weekday, true)?)?));
            }
            _ => {}
        }
        let period = if let Some(weekday) = weekday_from_name(key) {
            Period::day(self.latest(weekday, false)?)?
        } else if let Some(month) = month_from_name(key) {
            self.month(month)?
        } else if let Some(season) = season_from_name(key) {
            self.season(season, false)?
        } else if let Some(year) = self.year(key) {
            year
        } else {
            Period::day(parse_date(key, self.today)?)?
        };
        Some((at + 1, period))
    }

    fn ago(&self, at: usize) -> Option<(usize, Period)> {
        let count: u32 = match self.key(at)? {
            "a" | "an" | "one" => 1,
            number => number.parse().ok()?,
        };
        if self.key(at + 2)? != "ago" {
            return None;
        }
        let today = self.today;
        let days = |n: u32| today.checked_sub_days(Days::new(u64::from(n)));
        let period = match self.key(at + 1)? {
            "day" | "days" => Period::day(days(count)?)?,
            "week" | "weeks" => Self::containing(Unit::Week, days(count.checked_mul(7)?)?)?,
            "month" | "months" => {
                Self::containing(Unit::Month, today.checked_sub_months(Months::new(count))?)?
            }
            "year" | "years" => Self::containing(
                Unit::Year,
                today.checked_sub_months(Months::new(count.checked_mul(12)?))?,
            )?,
            _ => return None,
        };
        Some((at + 3, period))
    }

    fn containing(unit: Unit, date: NaiveDate) -> Option<Period> {
        let from = match unit {
            Unit::Week => {
                date.checked_sub_days(Days::new(u64::from(date.weekday().num_days_from_monday())))?
            }
            Unit::Month => date.with_day(1)?,
            Unit::Year => NaiveDate::from_ymd_opt(date.year(), 1, 1)?,
        };
        let until = match unit {
            Unit::Week => from.checked_add_days(Days::new(7))?,
            Unit::Month => from.checked_add_months(Months::new(1))?,
            Unit::Year => from.checked_add_months(Months::new(12))?,
        };
        Some(Period { from, until })
    }

    fn season(&self, first: u32, ended: bool) -> Option<Period> {
        let today = self.today;
        let mut year = today.year();
        loop {
            let from = NaiveDate::from_ymd_opt(year, first, 1)?;
            let until = from.checked_add_months(Months::new(3))?;
            let mark = if ended { until } else { from };
            if mark <= today {
                return Some(Period { from, until });
            }
            year = year.checked_sub(1)?;
            if year < today.year() - 2 {
                return None;
            }
        }
    }

    fn calendar(&self, unit: Unit, back: bool) -> Option<Period> {
        let today = self.today;
        let (start, step) = match unit {
            Unit::Week => {
                let since_monday = u64::from(today.weekday().num_days_from_monday());
                let monday = today.checked_sub_days(Days::new(since_monday))?;
                let next = monday.checked_add_days(Days::new(7))?;
                let last = monday.checked_sub_days(Days::new(7))?;
                return Some(if back {
                    Period {
                        from: last,
                        until: monday,
                    }
                } else {
                    Period {
                        from: monday,
                        until: next,
                    }
                });
            }
            Unit::Month => (today.with_day(1)?, Months::new(1)),
            Unit::Year => (
                NaiveDate::from_ymd_opt(today.year(), 1, 1)?,
                Months::new(12),
            ),
        };
        Some(if back {
            Period {
                from: start.checked_sub_months(step)?,
                until: start,
            }
        } else {
            Period {
                from: start,
                until: start.checked_add_months(step)?,
            }
        })
    }

    fn latest(&self, weekday: Weekday, strictly: bool) -> Option<NaiveDate> {
        let today = self.today;
        let mut back =
            (today.weekday().num_days_from_monday() + 7 - weekday.num_days_from_monday()) % 7;
        if strictly && back == 0 {
            back = 7;
        }
        today.checked_sub_days(Days::new(u64::from(back)))
    }

    /// In September, August is this year's and December last year's.
    fn month(&self, month: u32) -> Option<Period> {
        let year = if month <= self.today.month() {
            self.today.year()
        } else {
            self.today.year() - 1
        };
        let from = NaiveDate::from_ymd_opt(year, month, 1)?;
        Some(Period {
            from,
            until: from.checked_add_months(Months::new(1))?,
        })
    }

    fn year(&self, key: &str) -> Option<Period> {
        if key.len() != 4 || !key.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
        let year: i32 = key.parse().ok()?;
        if !(1970..=self.today.year()).contains(&year) {
            return None;
        }
        Some(Period {
            from: NaiveDate::from_ymd_opt(year, 1, 1)?,
            until: NaiveDate::from_ymd_opt(year + 1, 1, 1)?,
        })
    }

    fn rolling(&self, at: usize) -> Option<(usize, Vec<String>)> {
        let (count, unit_at) = match self.key(at + 1)?.parse::<u32>() {
            Ok(count) => (count, at + 2),
            Err(_) => (1, at + 1),
        };
        let today = self.today;
        let from = match self.key(unit_at)? {
            "day" | "days" => today.checked_sub_days(Days::new(u64::from(count)))?,
            "week" | "weeks" => today.checked_sub_days(Days::new(u64::from(count) * 7))?,
            "month" | "months" => today.checked_sub_months(Months::new(count))?,
            "year" | "years" => today.checked_sub_months(Months::new(count.checked_mul(12)?))?,
            _ => return None,
        };
        Some((unit_at + 1, vec![format!("after:{from}")]))
    }
}

fn season_from_name(name: &str) -> Option<u32> {
    match name {
        "spring" => Some(3),
        "summer" => Some(6),
        "autumn" | "fall" => Some(9),
        "winter" => Some(12),
        _ => None,
    }
}

fn set_operator(field: &str, values: &[String]) -> String {
    match values {
        [one] => operator(field, one),
        many => {
            let values: Vec<String> = many
                .iter()
                .map(|value| {
                    let spelled = operator(field, value);
                    spelled[field.len() + 1..].to_owned()
                })
                .collect();
            format!("{field}:{{{}}}", values.join(" "))
        }
    }
}

fn operator(field: &str, value: &str) -> String {
    let value: String = value.chars().filter(|c| *c != '"').collect();
    if value.chars().any(char::is_whitespace) {
        format!("{field}:\"{value}\"")
    } else {
        format!("{field}:{value}")
    }
}

#[cfg(test)]
mod tests {
    use chrono::NaiveDate;

    use super::lower;
    use crate::parse;
    use crate::query::TokenKind;

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 9, 26).expect("a real date")
    }

    fn names(name: &str) -> Option<String> {
        match name.to_lowercase().as_str() {
            "ada" => Some("ada".to_owned()),
            "ada moreno" => Some("ada.moreno@example.org".to_owned()),
            "grace" => Some("grace".to_owned()),
            _ => None,
        }
    }

    const LOWERED: &[(&str, &str)] = &[
        (
            "the invoice Ada sent last month",
            "invoice from:ada after:2026-08-01 before:2026-09-01",
        ),
        ("invoice from Ada", "invoice from:ada"),
        ("budget by Grace", "budget from:grace"),
        ("notes Grace wrote", "notes from:grace"),
        ("messages to Ada", "to:ada"),
        ("emails sent to Grace", "to:grace"),
        (
            "the report Ada Moreno sent",
            "report from:ada.moreno@example.org",
        ),
        ("from ada@example.com", "from:ada@example.com"),
        ("today", "after:2026-09-26 before:2026-09-27"),
        ("yesterday", "after:2026-09-25 before:2026-09-26"),
        ("this week", "after:2026-09-21 before:2026-09-28"),
        ("last week", "after:2026-09-14 before:2026-09-21"),
        ("this month", "after:2026-09-01 before:2026-10-01"),
        ("last year", "after:2025-01-01 before:2026-01-01"),
        ("since Monday", "after:2026-09-21"),
        ("since August", "after:2026-08-01"),
        (
            "receipts in August",
            "receipts after:2026-08-01 before:2026-09-01",
        ),
        ("in December", "after:2025-12-01 before:2026-01-01"),
        ("in 2025", "after:2025-01-01 before:2026-01-01"),
        ("last Monday", "after:2026-09-21 before:2026-09-22"),
        ("before August", "before:2026-08-01"),
        ("after August", "after:2026-09-01"),
        ("the last 30 days", "after:2026-08-27"),
        (
            "emails from Grace with attachments",
            "from:grace has:attach",
        ),
        ("invoice without attachments", "invoice -has:attach"),
        ("unread flagged invoices", "is:unread is:flagged invoices"),
        ("invoices in archive", "invoices in:archive"),
        ("in spam", "in:junk"),
        (
            "from:ada budget last month",
            "from:ada budget after:2026-08-01 before:2026-09-01",
        ),
        (
            "\"quarterly report\" from Ada",
            "\"quarterly report\" from:ada",
        ),
        ("invoice, from Ada?", "invoice from:ada"),
        (
            "from:{ada@example.com tomas@example.com} budget last month",
            "from:{ada@example.com tomas@example.com} budget after:2026-08-01 before:2026-09-01",
        ),
        (
            r#"-label:{atlas "Q3 close"} invoices"#,
            r#"-label:{atlas "Q3 close"} invoices"#,
        ),
    ];

    #[test]
    fn plain_english_lowers_to_what_could_have_been_typed_by_hand() {
        for (text, typed) in LOWERED {
            assert_eq!(
                lower(text, today(), &names),
                parse(typed, today()),
                "{text:?} should lower to {typed:?}"
            );
        }
    }

    #[test]
    fn or_joins_people_only_after_a_cue_and_only_known_ones() {
        for (text, typed) in [
            ("from ada or grace", "from:{ada grace}"),
            (
                "from ada or grace or ada moreno",
                "from:{ada grace ada.moreno@example.org}",
            ),
            ("from ada or nobody", "from:ada nobody"),
            ("ada or grace", "ada grace"),
            ("from ada or", "from:ada"),
        ] {
            assert_eq!(
                lower(text, today(), &names),
                parse(typed, today()),
                "{text:?} should lower to {typed:?}"
            );
        }
    }

    #[test]
    fn words_it_cannot_lower_stay_free_text() {
        for (text, typed) in [
            ("quarterly budget review", "quarterly budget review"),
            ("from Zork", "Zork"),
            ("Zork sent it", "Zork sent"),
            ("Ada", "Ada"),
            ("the meeting in person", "meeting person"),
            ("August invoices", "August invoices"),
        ] {
            assert_eq!(
                lower(text, today(), &names),
                parse(typed, today()),
                "{text:?} should lower to {typed:?}"
            );
        }
    }

    #[test]
    fn a_lowering_is_only_ever_complete_operators_and_free_text() {
        for (text, _) in LOWERED {
            let lowered = lower(text, today(), &names);
            assert!(!lowered.is_empty(), "{text:?} lowered to nothing");
            assert_eq!(lowered, parse(lowered.input(), today()), "{text:?}");
            for token in lowered.tokens() {
                assert!(
                    !matches!(token.kind, TokenKind::Partial(_)),
                    "{text:?} lowered to a half-typed {:?}",
                    token.raw
                );
            }
        }
    }

    #[allow(clippy::type_complexity)]
    const ORIGINS: &[(&str, &str, &[Option<&str>])] = &[
        (
            "invoices from ada last month",
            "invoices from:ada after:2026-08-01 before:2026-09-01",
            &[
                Some("invoices"),
                Some("from ada"),
                Some("last month"),
                Some("last month"),
            ],
        ),
        ("since july", "after:2026-07-01", &[Some("since july")]),
        (
            "2 weeks ago",
            "after:2026-09-07 before:2026-09-14",
            &[Some("2 weeks ago"), Some("2 weeks ago")],
        ),
        (
            "budget a month ago",
            "budget after:2026-08-01 before:2026-09-01",
            &[Some("budget"), Some("a month ago"), Some("a month ago")],
        ),
        (
            "last spring",
            "after:2026-03-01 before:2026-06-01",
            &[Some("last spring"), Some("last spring")],
        ),
        ("since winter", "after:2025-12-01", &[Some("since winter")]),
        (
            "receipts with attachments",
            "receipts has:attach",
            &[Some("receipts"), Some("with attachments")],
        ),
        (
            "unread from ada",
            "is:unread from:ada",
            &[Some("unread"), Some("from ada")],
        ),
        (
            "the report Ada Moreno sent",
            "report from:ada.moreno@example.org",
            &[Some("report"), Some("Ada Moreno sent")],
        ),
        (
            "\"quarterly report\" from Ada",
            "\"quarterly report\" from:ada",
            &[None, Some("from Ada")],
        ),
        (
            "from:ada budget since July",
            "from:ada budget after:2026-07-01",
            &[None, Some("budget"), Some("since July")],
        ),
        (
            "show me the invoices, from Ada?",
            "invoices from:ada",
            &[Some("invoices"), Some("from Ada")],
        ),
        (
            "café receipts from Ada",
            "café receipts from:ada",
            &[Some("café"), Some("receipts"), Some("from Ada")],
        ),
        (
            "invoices from ada or grace",
            "invoices from:{ada grace}",
            &[Some("invoices"), Some("from ada or grace")],
        ),
        (
            "to Ada Moreno or grace@example.com last month",
            "to:{ada.moreno@example.org grace@example.com} after:2026-08-01 before:2026-09-01",
            &[
                Some("to Ada Moreno or grace@example.com"),
                Some("last month"),
                Some("last month"),
            ],
        ),
    ];

    #[test]
    fn every_token_says_which_words_it_came_from() {
        for (text, typed, words) in ORIGINS {
            let lowered = super::lower_with_origins(text, today(), &names);
            assert_eq!(
                lowered.query,
                parse(typed, today()),
                "{text:?} should lower to {typed:?}"
            );
            assert_eq!(
                lowered.query,
                lower(text, today(), &names),
                "{text:?}: the origins come with the same query `lower` gives"
            );
            assert_eq!(
                lowered.origins.len(),
                lowered.query.tokens().len(),
                "{text:?}: one origin a token"
            );
            for (index, (origin, expected)) in lowered.origins.iter().zip(*words).enumerate() {
                assert_eq!(origin.token, index, "{text:?}: in token order");
                let token = &lowered.query.tokens()[index].raw;
                match (origin.from, expected) {
                    (Some(span), Some(expected)) => {
                        assert_eq!(
                            &text[span.start..span.end],
                            *expected,
                            "{text:?}: {token} came from the sentence's own words"
                        );
                        assert_eq!(origin.words, *expected, "{text:?}: {token}");
                    }
                    (None, None) => {
                        assert_eq!(&origin.words, token, "{text:?}: typed as is");
                    }
                    (from, expected) => {
                        panic!("{text:?}: {token} came from {from:?}, expected {expected:?}")
                    }
                }
            }
        }
    }

    #[test]
    fn the_same_words_always_give_the_same_chips() {
        for (text, _) in LOWERED {
            let first = lower(text, today(), &names);
            for _ in 0..3 {
                assert_eq!(lower(text, today(), &names), first, "{text:?}");
            }
        }
    }
}
