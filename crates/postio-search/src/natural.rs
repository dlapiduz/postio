//! Plain English, lowered into the one query language (spec FR-062,
//! research R5).
//!
//! The command bar takes "the invoice Ada sent last month" and shows it as
//! chips: `invoice from:ada after:2026-08-01 before:2026-09-01`. The chips
//! are not a reading of the sentence alongside some other evaluation of it
//! -- **they are the query** (constitution III). [`lower`] returns exactly
//! what [`crate::parse`] makes of the lowered text, so running the chips
//! returns what typing those operators by hand returns, and there is no
//! second language for the sentence to mean something else in.
//!
//! # The rules
//!
//! Deterministic, local, and small enough to state:
//!
//! * **Correspondents**, when the words around a name say so: after `from`
//!   or `by` (or `sent by`), or before `sent` or `wrote`, a name becomes
//!   `from:`; after `to` (or `sent to`), `to:`. A name is only a name when
//!   the caller's address book knows it -- the `names` closure, tried on the
//!   longest run of up to three words first -- or when it is an address.
//! * **Dates**, as bounds: `today`, `yesterday`, `this week`, `last month`,
//!   `last year`, `last Monday` become an `after:`/`before:` pair; `in
//!   August` and `in 2025` too. `since`, `before` and `after` take any of
//!   those, a weekday, a month, or a date `after:` would read, and become
//!   one bound. `the last 30 days` and `past week` are an `after:` alone. A
//!   week starts on Monday, as the pickers' "Monday morning" does.
//! * **Ago**: `2 weeks ago`, `a month ago`, `3 days ago` are the calendar
//!   day, week, month or year that held the date that far back -- `2 weeks
//!   ago` on a Saturday is the Monday-to-Sunday week a fortnight back.
//! * **Seasons**, meteorological and northern: spring is March to May,
//!   summer June to August, autumn (or fall) September to November, winter
//!   December to February. `last spring` is the latest spring that has
//!   ended; `since winter` and `in summer` take the latest that has begun,
//!   as a month does.
//! * **Structure and state**: `with attachments` is `has:attach` (and
//!   `without attachments` its negation), `unread` is `is:unread`, `flagged`
//!   is `is:flagged`, and `in` a mailbox's role -- `in archive`, `in spam` --
//!   is `in:`.
//! * **Operators, quoted phrases and negations pass through**: typing the
//!   language is always allowed.
//! * **Stop words are dropped** -- articles, pronouns, prepositions, and
//!   "email" or "message", which say what is being searched rather than
//!   what for.
//! * **Every other word stays free text**, which already searches senders,
//!   subjects and bodies.
//!
//! # What it deliberately does not lower
//!
//! A bare name, a bare month or a bare weekday ("Ada", "August invoices")
//! stays free text: without a word that says it is a sender or a date, it is
//! as likely to be a subject. `in` a folder that is not a role ("in
//! Receipts") stays free text too: `in:` naming a folder that does not exist
//! selects nothing rather than everything, so guessing wrong would hide every
//! result. The bar's own `in:` completion is how a folder is named.
//!
//! Pure: no clock (`today` is a parameter), no network, no model.

use chrono::{Datelike, Days, Months, NaiveDate, Weekday};

use crate::ParsedQuery;
use crate::date::{month_from_name, parse_date, weekday_from_name};
use crate::query::{Field, Span};

/// The longest run of words tried as one name: "Ada Moreno", "Mary Ann
/// Evans".
const NAME_WORDS: usize = 3;

/// Words that say nothing about which mail is meant.
const STOP_WORDS: &[&str] = &[
    "a", "about", "after", "all", "an", "and", "any", "are", "at", "be", "been", "before", "by",
    "email", "emails", "find", "for", "from", "get", "give", "i", "in", "is", "it", "its", "list",
    "mail", "mails", "me", "message", "messages", "my", "of", "on", "or", "our", "please",
    "search", "show", "since", "some", "that", "the", "these", "this", "those", "to", "us", "was",
    "we", "were", "which", "who", "whose", "with", "you", "your",
];

/// `text`, read as plain English and lowered into the query language, with
/// `today` as the day relative dates count from and `names` as the address
/// book: given a name as typed ("Ada", "Ada Moreno"), what `from:` or `to:`
/// should say for that correspondent, or `None` for a name it does not know.
///
/// The result is the parse of the lowered text, so its tokens are the chips
/// and its input is what the bar's entry holds while the chips are edited.
/// See the module for the rules.
pub fn lower(text: &str, today: NaiveDate, names: &dyn Fn(&str) -> Option<String>) -> ParsedQuery {
    lower_with_origins(text, today, names).query
}

/// A sentence lowered, with what each of its tokens came from: the
/// "Understood as" bar's tiles (spec 010 US7), each "from ‘last month’".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lowered {
    /// Exactly [`lower`]'s answer.
    pub query: ParsedQuery,
    /// One a token of `query`, in the same order.
    pub origins: Vec<Origin>,
}

/// Where one token of a [`Lowered`] query came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Origin {
    /// The token's index in `query.tokens()`.
    pub token: usize,
    /// The bytes of the English sentence it was lowered from, cue words
    /// included ("from ada" for `from:ada`), punctuation around them not;
    /// `None` when the person typed the token in the language itself.
    pub from: Option<Span>,
    /// The words, as typed: the sentence's text at `from`, or the token
    /// itself when it was typed as is.
    pub words: String,
}

/// [`lower`], and for each token the words of `text` it came from.
///
/// Every token comes from one step of the lowering, which reads a run of
/// words and writes one or two operators or a word: those operators' origin
/// is that run. A date pair (`after:` and `before:`) shares its origin. Stop
/// words write nothing and so are nobody's origin.
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
    // Each piece of the lowered text, and the words it came from.
    let mut out = Vec::new();
    let mut sources: Vec<(usize, usize)> = Vec::new();
    let mut at = 0;
    while at < words.len() {
        let written = out.len();
        let next = lowering.step(at, &mut out);
        sources.extend(std::iter::repeat_n((at, next), out.len() - written));
        at = next;
    }

    // Where each piece lands in the joined text, so a token is traced back
    // by its span whatever the parser made of the piece.
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

/// One word, as typed.
struct Word {
    /// Where its text sits in the sentence, without the punctuation around
    /// it: what an origin points at.
    core: Span,
    /// Exactly as typed: what passes through when it is the language already.
    raw: String,
    /// Without the punctuation around it, in the case it was typed in.
    bare: String,
    /// `bare`, lowercased: what the rules compare.
    key: String,
    /// An operator, a quoted phrase or a negation: the language already.
    verbatim: bool,
}

/// Splits `text` at whitespace, keeping a quoted phrase in one word.
fn words(text: &str) -> Vec<Word> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut start = 0;
    let mut quoted = false;
    for (offset, c) in text.char_indices() {
        if c == '"' {
            quoted = !quoted;
        }
        if c.is_whitespace() && !quoted {
            if !current.is_empty() {
                out.push(word(std::mem::take(&mut current), start));
            }
        } else {
            if current.is_empty() {
                start = offset;
            }
            current.push(c);
        }
    }
    if !current.is_empty() {
        out.push(word(current, start));
    }
    out
}

/// The word `raw`, which starts at byte `start` of the sentence.
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

/// A run of whole days: `from` inclusive and `until` exclusive, the way
/// `after:` and `before:` read their dates.
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

/// A calendar unit a period can be "this" or "last" of.
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
    /// Lowers what starts at word `at` into `out`, returning the index of the
    /// next word not yet read.
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

    /// The comparable form of word `at`, if it is plain English.
    fn key(&self, at: usize) -> Option<&str> {
        self.words
            .get(at)
            .filter(|word| !word.verbatim)
            .map(|word| word.key.as_str())
    }

    /// `with attachments`, `without attachments`.
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

    /// `in archive`, `in spam`: a mailbox by its role, which every account
    /// has whatever its folders are called.
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

    /// `from Ada`, `by Ada`, `to Ada`, `sent to Ada`, `Ada sent`, `Ada
    /// wrote`.
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

    /// A name after the cue at `at`: `from`, `by` or `to`.
    fn after_cue(&self, at: usize) -> Option<(usize, String)> {
        let field = match self.key(at)? {
            "from" | "by" => "from",
            "to" => "to",
            _ => return None,
        };
        if let Some(found) = (1..=NAME_WORDS)
            .rev()
            .find_map(|len| Some((at + 1 + len, operator(field, &self.name(at + 1, len)?))))
        {
            return Some(found);
        }
        // An address needs no address book.
        let word = self.words.get(at + 1).filter(|word| !word.verbatim)?;
        let (local, domain) = word.bare.split_once('@')?;
        (!local.is_empty() && !domain.is_empty()).then(|| (at + 2, operator(field, &word.bare)))
    }

    /// What the address book says for the `len` words at `at`, as one name.
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

    /// A date phrase at `at`, as the operators it lowers to.
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

    /// The days a phrase at `at` names: `today`, `yesterday`, `this week`,
    /// `last month`, `last Monday`, a weekday, a month, a year, or a date.
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

    /// `2 weeks ago`, `a month ago`: the calendar unit that held the date
    /// that far back.
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

    /// The week (from Monday), month or year `date` falls in.
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

    /// The latest season starting in month `first` that has ended by today
    /// (`ended`), or that has begun.
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

    /// This or last week, month or year.
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

    /// The last `weekday` on or before today, or strictly before it.
    fn latest(&self, weekday: Weekday, strictly: bool) -> Option<NaiveDate> {
        let today = self.today;
        let mut back =
            (today.weekday().num_days_from_monday() + 7 - weekday.num_days_from_monday()) % 7;
        if strictly && back == 0 {
            back = 7;
        }
        today.checked_sub_days(Days::new(u64::from(back)))
    }

    /// The last `month` that has begun: in September, August is this year's
    /// and December last year's.
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

    /// A year that has begun, written as four digits.
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

    /// `the last 30 days`, `past week`: from that far back until now.
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

/// The first month of a season, by name: meteorological, northern.
fn season_from_name(name: &str) -> Option<u32> {
    match name {
        "spring" => Some(3),
        "summer" => Some(6),
        "autumn" | "fall" => Some(9),
        "winter" => Some(12),
        _ => None,
    }
}

/// `field:value`, quoted when the value has a space in it.
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

    /// Saturday 26 September 2026, the clock screen 07 is drawn at.
    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 9, 26).expect("a real date")
    }

    /// The address book the tests search: two correspondents, one of them
    /// known by a two-word name as well.
    fn names(name: &str) -> Option<String> {
        match name.to_lowercase().as_str() {
            "ada" => Some("ada".to_owned()),
            "ada moreno" => Some("ada.moreno@example.org".to_owned()),
            "grace" => Some("grace".to_owned()),
            _ => None,
        }
    }

    /// Each phrase, and the query a person could have typed by hand instead.
    const LOWERED: &[(&str, &str)] = &[
        // Screen 07's sentence (spec US4 scenario 1). "invoice" stays free
        // text, which already searches subjects; the screen's `subject:`
        // chip is a recorded difference (research R5).
        (
            "the invoice Ada sent last month",
            "invoice from:ada after:2026-08-01 before:2026-09-01",
        ),
        // Correspondents, by the words around their names.
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
        // Dates, as a pair of bounds (or one, when the range is open).
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
        // Structure and state.
        (
            "emails from Grace with attachments",
            "from:grace has:attach",
        ),
        ("invoice without attachments", "invoice -has:attach"),
        ("unread flagged invoices", "is:unread is:flagged invoices"),
        ("invoices in archive", "invoices in:archive"),
        ("in spam", "in:junk"),
        // What the person typed in the language itself passes through.
        (
            "from:ada budget last month",
            "from:ada budget after:2026-08-01 before:2026-09-01",
        ),
        (
            "\"quarterly report\" from Ada",
            "\"quarterly report\" from:ada",
        ),
        // Punctuation is not part of a word.
        ("invoice, from Ada?", "invoice from:ada"),
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
    fn words_it_cannot_lower_stay_free_text() {
        // Spec FR-062 and US4 scenario 8. A name the address book does not
        // know is not a correspondent, whatever word is in front of it; a
        // word after "in" that is not a mailbox's role is not a folder.
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
        // Chips of the one language (constitution III): nothing half-typed,
        // and the query is exactly the parse of its own text.
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

    /// Spec 010 US7: each sentence, the query it lowers to, and for each of
    /// its tokens the words of the sentence it came from -- `None` for a
    /// token the person typed in the language itself.
    #[allow(clippy::type_complexity)]
    const ORIGINS: &[(&str, &str, &[Option<&str>])] = &[
        // Screen 05.
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
        // The design's date examples (design SPEC "Plain English").
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
        // A quoted phrase and an operator are the language already.
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
        // Stop words leave no token and no origin; punctuation is not part
        // of the words a token came from; spans are bytes, and a word
        // before them may be more than one byte a character.
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
        // Spec US4 scenario 8.
        for (text, _) in LOWERED {
            let first = lower(text, today(), &names);
            for _ in 0..3 {
                assert_eq!(lower(text, today(), &names), first, "{text:?}");
            }
        }
    }
}
