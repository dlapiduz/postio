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
use crate::query::Field;

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
    let words = words(text);
    let lowering = Lowering {
        words: &words,
        today,
        names,
    };
    let mut out = Vec::new();
    let mut at = 0;
    while at < words.len() {
        at = lowering.step(at, &mut out);
    }
    crate::parse(&out.join(" "), today)
}

/// One word, as typed.
struct Word {
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
    let mut quoted = false;
    for c in text.chars() {
        if c == '"' {
            quoted = !quoted;
        }
        if c.is_whitespace() && !quoted {
            if !current.is_empty() {
                out.push(word(std::mem::take(&mut current)));
            }
        } else {
            current.push(c);
        }
    }
    if !current.is_empty() {
        out.push(word(current));
    }
    out
}

fn word(raw: String) -> Word {
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
                let period = match month_from_name(key) {
                    Some(month) => self.month(month)?,
                    None => self.year(key)?,
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
                let weekday = weekday_from_name(next).filter(|_| back)?;
                return Some((at + 2, Period::day(self.latest(weekday, true)?)?));
            }
            _ => {}
        }
        let period = if let Some(weekday) = weekday_from_name(key) {
            Period::day(self.latest(weekday, false)?)?
        } else if let Some(month) = month_from_name(key) {
            self.month(month)?
        } else if let Some(year) = self.year(key) {
            year
        } else {
            Period::day(parse_date(key, self.today)?)?
        };
        Some((at + 1, period))
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
