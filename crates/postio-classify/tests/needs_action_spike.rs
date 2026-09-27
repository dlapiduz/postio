//! Spike S4 (`specs/007-postio-focus` T010, research R10): the labelled
//! needs-action dataset, and a rules-only prototype of the detector measured
//! against it.
//!
//! The prototype is test code on purpose. The detector is T116's, in the
//! library, under SC-013's gate; this only answers the question that decides
//! T116's shape: can rules alone reach a precision of 0.9, or does the
//! detector need the small table of weights FR-165 allows? When T116 lands
//! its own detector, delete the prototype and keep the dataset.
//!
//! Two rule sets are measured, so the answer is not flattered by rules
//! written after reading the data:
//!
//! - **`as_written`** is R10 as the research states it: own text only,
//!   sentences split at `;` and `—`, a question ends in `?` and is put in the
//!   second person, a to-do is "please", "can/could/would you", "let me
//!   know", "I need you to" or an imperative opening, pleasantries and
//!   rhetorical questions excluded, one marker per message.
//! - **`with_fixes`** adds four general rules R10's lists leave out:
//!   boilerplate that reads as a request ("Let me know if you have any
//!   questions"), an ask put to somebody else by name ("Tove, can you..."),
//!   text addressed to an assistant, and a greeting in front of an imperative
//!   ("Hi Ada, ping me...").
//!
//! Both were written before either was run, with generic phrase lists rather
//! than phrases copied from the dataset. But the dataset and the rules have
//! one author, so read the numbers as a best case: real mail will do worse.
//!
//! Run it with `cargo test -p postio-classify --test needs_action_spike --
//! --nocapture` to see the report.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{Datelike, Duration, NaiveDate, Weekday};
use serde::Deserialize;

const DATASET: &str = include_str!("data/needs_action.toml");

// --- The dataset ----------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct Dataset {
    sent: String,
    user: User,
    message: Vec<Item>,
}

#[derive(Debug, Deserialize)]
struct User {
    name: String,
    address: String,
}

#[derive(Debug, Deserialize)]
struct Item {
    id: String,
    case: String,
    addressing: String,
    text: String,
    label: String,
    quote: Option<String>,
    due: Option<String>,
    #[serde(default)]
    also: Vec<Also>,
}

/// Another genuine ask in the same message: not the marker R10's precedence
/// picks, but not a wrong one either.
#[derive(Debug, Deserialize)]
struct Also {
    label: String,
    quote: String,
}

fn dataset() -> Dataset {
    toml::from_str(DATASET).expect("needs_action.toml parses")
}

fn date(text: &str) -> NaiveDate {
    NaiveDate::parse_from_str(text, "%Y-%m-%d").expect("a YYYY-MM-DD date")
}

/// Whitespace runs collapsed and the sentence's closing `.`/`!` dropped: a
/// quote is compared by its words, not by how the text was wrapped.
fn normalized(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim_end_matches(['.', '!'])
        .to_owned()
}

// --- The dataset's own invariants --------------------------------------------------

#[test]
fn the_dataset_is_big_enough_and_covers_every_case_it_promises() {
    let data = dataset();
    assert!(data.message.len() >= 150, "{} items", data.message.len());

    let mut cases: BTreeMap<&str, usize> = BTreeMap::new();
    for item in &data.message {
        *cases.entry(item.case.as_str()).or_default() += 1;
    }
    for case in [
        "question",
        "todo",
        "pleasantry",
        "rhetorical",
        "quoted",
        "signature",
        "list",
        "copied",
        "automated",
        "instruction",
    ] {
        assert!(
            cases.get(case).copied().unwrap_or(0) >= 5,
            "at least five `{case}` items: {cases:?}"
        );
    }
    for addressing in ["direct", "copied", "list", "automated"] {
        assert!(
            data.message
                .iter()
                .any(|item| item.addressing == addressing),
            "an item addressed `{addressing}`"
        );
    }
}

#[test]
fn every_label_is_well_formed_and_quotes_the_text_verbatim() {
    let data = dataset();
    let mut ids = BTreeSet::new();
    for item in &data.message {
        assert!(ids.insert(item.id.as_str()), "{}: duplicate id", item.id);
        assert!(
            ["direct", "copied", "list", "automated"].contains(&item.addressing.as_str()),
            "{}: addressing {}",
            item.id,
            item.addressing
        );
        match item.label.as_str() {
            "question" | "todo" => {
                assert_eq!(
                    item.addressing, "direct",
                    "{}: only direct mail is marked (FR-106)",
                    item.id
                );
                let quote = item.quote.as_deref().expect("a marked item has a quote");
                assert!(
                    normalized(&item.text).contains(&normalized(quote)),
                    "{}: the quote is not verbatim in the text",
                    item.id
                );
            }
            "none" => assert!(
                item.quote.is_none() && item.due.is_none(),
                "{}: an unmarked item quotes nothing",
                item.id
            ),
            other => panic!("{}: label {other}", item.id),
        }
        if let Some(due) = &item.due {
            assert_eq!(item.label, "todo", "{}: only a to-do is due", item.id);
            assert!(
                date(due) >= date(&data.sent),
                "{}: due before sent",
                item.id
            );
        }
        for also in &item.also {
            assert!(
                item.label != "none" && ["question", "todo"].contains(&also.label.as_str()),
                "{}: an alternative belongs to a marked item",
                item.id
            );
            assert!(
                normalized(&item.text).contains(&normalized(&also.quote)),
                "{}: an alternative quote is not verbatim in the text",
                item.id
            );
        }
    }
}

#[test]
fn every_address_in_the_dataset_is_reserved() {
    let data = dataset();
    assert!(data.user.address.ends_with("@example.com"));
    for item in &data.message {
        for (at, _) in item.text.match_indices('@') {
            let domain: String = item.text[at + 1..]
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '.' || *c == '-')
                .collect();
            let domain = domain.trim_end_matches('.');
            let reserved = domain.ends_with(".example")
                || domain.ends_with(".test")
                || domain.ends_with(".invalid")
                || ["example.com", "example.net", "example.org"]
                    .iter()
                    .any(|reserved| {
                        domain == *reserved || domain.ends_with(&format!(".{reserved}"))
                    });
            assert!(reserved, "{}: {domain} is not a reserved domain", item.id);
        }
    }
}

// --- The prototype ----------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Rules {
    AsWritten,
    WithFixes,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Question,
    Todo,
}

impl Kind {
    fn label(self) -> &'static str {
        match self {
            Kind::Question => "question",
            Kind::Todo => "todo",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Marker {
    kind: Kind,
    excerpt: String,
    due: Option<NaiveDate>,
}

struct Prototype {
    rules: Rules,
    first_name: String,
    sent: NaiveDate,
}

const WEEKDAYS: [(&str, Weekday); 7] = [
    ("monday", Weekday::Mon),
    ("tuesday", Weekday::Tue),
    ("wednesday", Weekday::Wed),
    ("thursday", Weekday::Thu),
    ("friday", Weekday::Fri),
    ("saturday", Weekday::Sat),
    ("sunday", Weekday::Sun),
];

const MONTHS: [&str; 12] = [
    "january",
    "february",
    "march",
    "april",
    "may",
    "june",
    "july",
    "august",
    "september",
    "october",
    "november",
    "december",
];

/// A line that closes the message: what follows is the signature.
const CLOSINGS: &[&str] = &[
    "thanks",
    "thank you",
    "many thanks",
    "thanks!",
    "best",
    "best wishes",
    "best regards",
    "kind regards",
    "regards",
    "warm regards",
    "warmly",
    "cheers",
    "love",
    "all the best",
];

/// Openings that R10 counts as an imperative: a verb asking the reader to act.
const IMPERATIVES: &[&str] = &[
    "send", "review", "sign", "approve", "confirm", "fill", "complete", "submit", "update", "book",
    "bring", "check", "read", "add", "call", "reply", "forward", "upload", "share", "schedule",
    "return", "register", "rsvp", "pay", "renew", "finish", "prepare", "draft", "email", "ping",
    "pick", "leave", "move", "fix", "test", "look", "verify",
];

/// Openings R10's "imperative" covers that are phrases rather than one verb.
const IMPERATIVE_PHRASES: &[&str] = &["don't forget", "make sure", "remember to", "be sure to"];

/// Small talk, which R10 excludes by name.
const PLEASANTRIES: &[&str] = &[
    "how are you",
    "how are things",
    "how's it going",
    "how is it going",
    "how have you been",
    "how's life",
    "how was your weekend",
    "hope you're well",
    "hope you are well",
    "hope all is well",
    "hope you had a good",
    "hope you had a nice",
    "did you have a good weekend",
];

/// Questions that expect no answer, which R10 excludes by name.
const RHETORICAL: &[&str] = &[
    "who knew",
    "can you believe",
    "can you imagine",
    "what could go wrong",
    "what could possibly go wrong",
    "right?",
    "isn't it",
    "don't you think",
    "guess what",
    "why not",
    "who would have thought",
    "you know?",
];

/// Boilerplate that reads as a request and is not one (`with_fixes` only).
const BOILERPLATE: &[&str] = &[
    "let me know if you have any questions",
    "let me know if you need anything",
    "please find attached",
    "please see attached",
    "please see below",
    "see below",
    "don't hesitate",
    "feel free",
    "please note",
    "please ignore",
    "please consider the environment",
    "received it in error",
    "received this in error",
];

/// Words that open a greeting, not a name to address.
const GREETINGS: &[&str] = &[
    "hi",
    "hey",
    "hello",
    "dear",
    "morning",
    "hiya",
    "evening",
    "afternoon",
    "thanks",
    "also",
    "so",
    "anyway",
    "ok",
    "okay",
    "well",
    "sorry",
    "yes",
    "no",
    "sure",
    "great",
    "first",
    "second",
    "quick",
    "one",
    "oh",
];

/// Who text is addressed to when it is not the person reading
/// (`with_fixes` only).
const ASSISTANT_WORDS: &[&str] = &[
    "assistant",
    "ai",
    "language model",
    "llm",
    "automated agent",
    "previous instructions",
    "system prompt",
];

impl Prototype {
    fn detect(&self, addressing: &str, text: &str) -> Option<Marker> {
        // FR-106: only mail sent directly to the user.
        if addressing != "direct" {
            return None;
        }
        let own = own_text(text);
        let mut questions = Vec::new();
        let mut todos = Vec::new();
        for sentence in sentences(&own) {
            if self.excluded(&sentence) {
                continue;
            }
            if self.is_question(&sentence) {
                questions.push(sentence);
            } else if self.is_todo(&sentence) {
                let due = self.deadline(&sentence);
                todos.push((sentence, due));
            }
        }
        // R10: the first to-do with a deadline, else the first question, else
        // the first to-do.
        if let Some((sentence, due)) = todos.iter().find(|(_, due)| due.is_some()) {
            return Some(Marker {
                kind: Kind::Todo,
                excerpt: sentence.clone(),
                due: *due,
            });
        }
        if let Some(sentence) = questions.first() {
            return Some(Marker {
                kind: Kind::Question,
                excerpt: sentence.clone(),
                due: None,
            });
        }
        todos.first().map(|(sentence, due)| Marker {
            kind: Kind::Todo,
            excerpt: sentence.clone(),
            due: *due,
        })
    }

    fn excluded(&self, sentence: &str) -> bool {
        let lower = sentence.to_lowercase();
        if PLEASANTRIES.iter().any(|p| lower.contains(p))
            || (lower.ends_with('?') && RHETORICAL.iter().any(|r| lower.contains(r)))
        {
            return true;
        }
        if self.rules == Rules::AsWritten {
            return false;
        }
        BOILERPLATE.iter().any(|b| lower.contains(b))
            || self.addressed_to_someone_else(sentence)
            || ASSISTANT_WORDS
                .iter()
                .any(|word| contains_word(&lower, word))
    }

    /// "Tove, can you...": a vocative that is not the reader.
    fn addressed_to_someone_else(&self, sentence: &str) -> bool {
        let Some((head, _)) = sentence.split_once([',', '—']) else {
            return false;
        };
        let head = head.trim();
        let single_name = !head.contains(' ')
            && head.chars().next().is_some_and(char::is_uppercase)
            && head.chars().all(char::is_alphabetic);
        single_name
            && !head.eq_ignore_ascii_case(&self.first_name)
            && !GREETINGS.contains(&head.to_lowercase().as_str())
    }

    /// R10: ends in `?` and is put to the reader in the second person.
    fn is_question(&self, sentence: &str) -> bool {
        let trimmed = sentence.trim_end_matches(['"', ')', '\'']);
        trimmed.ends_with('?') && second_person(&trimmed.to_lowercase())
    }

    /// R10: "please", "can/could/would you", "let me know", "I need you to",
    /// or an imperative opening -- and not a question.
    fn is_todo(&self, sentence: &str) -> bool {
        if sentence.trim_end().ends_with('?') {
            return false;
        }
        let lower = sentence.to_lowercase();
        let body = if self.rules == Rules::WithFixes {
            strip_greeting(&lower, &self.first_name)
        } else {
            lower.clone()
        };
        let body = body.trim_start();
        contains_word(&lower, "please")
            || ["can you ", "could you ", "would you "]
                .iter()
                .any(|opening| body.starts_with(opening))
            || lower.contains("let me know")
            || lower.contains("i need you to")
            || IMPERATIVE_PHRASES
                .iter()
                .any(|phrase| body.starts_with(phrase))
            || body
                .split(|c: char| !c.is_alphanumeric() && c != '\'')
                .next()
                .is_some_and(|first| IMPERATIVES.contains(&first))
    }

    /// A deadline phrase, resolved against the day the message was sent.
    fn deadline(&self, sentence: &str) -> Option<NaiveDate> {
        let lower = sentence.to_lowercase();
        for keyword in ["by ", "before ", "no later than ", "until "] {
            for (at, _) in lower.match_indices(keyword) {
                if at > 0 && lower.as_bytes()[at - 1].is_ascii_alphanumeric() {
                    continue;
                }
                let window: String = lower[at + keyword.len()..].chars().take(40).collect();
                if let Some(day) = self.resolve(&window) {
                    return Some(day);
                }
            }
        }
        None
    }

    fn resolve(&self, window: &str) -> Option<NaiveDate> {
        let words: Vec<&str> = window
            .split(|c: char| !c.is_alphanumeric())
            .filter(|word| !word.is_empty())
            .collect();
        for (index, word) in words.iter().enumerate() {
            match *word {
                "today" | "tonight" | "eod" | "cob" => return Some(self.sent),
                "end" if words.get(index + 1..index + 3) == Some(&["of", "day"]) => {
                    return Some(self.sent);
                }
                "end" if words.get(index + 1..index + 4) == Some(&["of", "the", "month"]) => {
                    let first_of_next =
                        NaiveDate::from_ymd_opt(self.sent.year(), self.sent.month() + 1, 1)?;
                    return Some(first_of_next - Duration::days(1));
                }
                "tomorrow" => return Some(self.sent + Duration::days(1)),
                _ => {}
            }
            if let Some((_, weekday)) = WEEKDAYS.iter().find(|(name, _)| name == word) {
                // Unless a date follows ("Monday, 28 September").
                if let Some(day) = words
                    .get(index + 1..index + 3)
                    .and_then(|next| self.day_month(next))
                {
                    return Some(day);
                }
                let mut day = self.sent + Duration::days(1);
                while day.weekday() != *weekday {
                    day += Duration::days(1);
                }
                return Some(day);
            }
            if let Some(day) = words
                .get(index..index + 2)
                .and_then(|pair| self.day_month(pair))
            {
                return Some(day);
            }
        }
        None
    }

    /// "28 September" or "September 28".
    fn day_month(&self, pair: &[&str]) -> Option<NaiveDate> {
        let (day, month) = match (pair[0].parse::<u32>(), pair[1].parse::<u32>()) {
            (Ok(day), Err(_)) => (day, pair[1]),
            (Err(_), Ok(day)) => (day, pair[0]),
            _ => return None,
        };
        let month = MONTHS.iter().position(|name| *name == month)? as u32 + 1;
        let this_year = NaiveDate::from_ymd_opt(self.sent.year(), month, day)?;
        if this_year >= self.sent {
            Some(this_year)
        } else {
            NaiveDate::from_ymd_opt(self.sent.year() + 1, month, day)
        }
    }
}

fn contains_word(haystack: &str, word: &str) -> bool {
    haystack.match_indices(word).any(|(at, _)| {
        let before = haystack[..at].chars().next_back();
        let after = haystack[at + word.len()..].chars().next();
        !before.is_some_and(char::is_alphanumeric) && !after.is_some_and(char::is_alphanumeric)
    })
}

fn second_person(lower: &str) -> bool {
    [
        "you", "your", "yours", "you're", "you've", "you'll", "you'd",
    ]
    .iter()
    .any(|word| contains_word(lower, word))
}

/// "hi ada, ping me..." -> "ping me...".
fn strip_greeting(lower: &str, first_name: &str) -> String {
    let mut rest = lower.trim_start().to_owned();
    loop {
        let Some((head, tail)) = rest.split_once(',') else {
            return rest;
        };
        let words: Vec<&str> = head.split_whitespace().collect();
        let greeting = words
            .first()
            .is_some_and(|first| GREETINGS.contains(first) || *first == first_name.to_lowercase())
            && words.len() <= 3;
        if !greeting {
            return rest;
        }
        rest = tail.trim_start().to_owned();
    }
}

/// R10's own text, approximated: quoted lines, forwarded and original
/// messages, the signature and everything after a closing are dropped.
fn own_text(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let mut kept = Vec::new();
    let mut index = 0;
    while index < lines.len() {
        let line = lines[index];
        let trimmed = line.trim();
        let lower = trimmed.to_lowercase();
        let separator = lower.starts_with("-----original message")
            || (lower.starts_with("---") && lower.contains("forwarded message"))
            || (trimmed.len() >= 10 && trimmed.chars().all(|c| c == '_'))
            || lower.starts_with("sent from my")
            || trimmed == "--"
            || line == "-- "
            || CLOSINGS.contains(&lower.trim_end_matches([',', '!', '.']))
            || (lower.starts_with("from: ")
                && lines
                    .get(index + 1)
                    .is_some_and(|next| next.starts_with("Sent:") || next.starts_with("Date:")));
        if separator {
            break;
        }
        if lower.starts_with("on ") && lower.ends_with("wrote:") {
            // Top-posted over a quote: skip the attribution and let the quoted
            // lines drop one by one. Over unquoted text, stop here.
            let next = lines[index + 1..]
                .iter()
                .find(|next| !next.trim().is_empty());
            if next.is_some_and(|next| next.trim_start().starts_with('>')) {
                index += 1;
                continue;
            }
            break;
        }
        if !trimmed.starts_with('>') {
            kept.push(line);
        }
        index += 1;
    }
    kept.join("\n")
}

/// Paragraphs, then sentences at `.`, `?`, `!`, then clauses at `;` and `—`.
/// A paragraph indented as code is not prose.
fn sentences(own: &str) -> Vec<String> {
    let mut out = Vec::new();
    for paragraph in own.split("\n\n") {
        if paragraph
            .lines()
            .all(|line| line.starts_with("    ") || line.trim().is_empty())
        {
            continue;
        }
        let flat = paragraph.split_whitespace().collect::<Vec<_>>().join(" ");
        let mut start = 0;
        let chars: Vec<(usize, char)> = flat.char_indices().collect();
        for (position, (at, character)) in chars.iter().enumerate() {
            let end_of_sentence = matches!(character, '.' | '?' | '!')
                && chars
                    .get(position + 1)
                    .is_none_or(|(_, next)| next.is_whitespace());
            if end_of_sentence {
                out.extend(clauses(&flat[start..at + character.len_utf8()]));
                start = at + character.len_utf8();
            }
        }
        out.extend(clauses(&flat[start..]));
    }
    out.into_iter()
        .filter(|sentence| !sentence.is_empty())
        .collect()
}

fn clauses(sentence: &str) -> Vec<String> {
    sentence
        .split([';', '—'])
        .map(|clause| clause.trim().to_owned())
        .collect()
}

// --- The measurement -----------------------------------------------------------------

#[derive(Debug, Default)]
struct Score {
    predicted: usize,
    right: usize,
    /// Right, or one of the item's other genuine asks.
    right_or_also: usize,
    right_kind: usize,
    labelled: usize,
    due_expected: usize,
    due_right: usize,
    errors: Vec<String>,
    false_positives_by_case: BTreeMap<String, usize>,
}

impl Score {
    fn precision(&self) -> f64 {
        self.right as f64 / self.predicted.max(1) as f64
    }
    fn lenient_precision(&self) -> f64 {
        self.right_or_also as f64 / self.predicted.max(1) as f64
    }
    fn kind_precision(&self) -> f64 {
        self.right_kind as f64 / self.predicted.max(1) as f64
    }
    fn recall(&self) -> f64 {
        self.right as f64 / self.labelled.max(1) as f64
    }
}

fn measure(rules: Rules) -> Score {
    let data = dataset();
    let prototype = Prototype {
        rules,
        first_name: data
            .user
            .name
            .split_whitespace()
            .next()
            .expect("a first name")
            .to_owned(),
        sent: date(&data.sent),
    };
    let mut score = Score::default();
    for item in &data.message {
        let got = prototype.detect(&item.addressing, &item.text);
        let marked = item.label != "none";
        if marked {
            score.labelled += 1;
        }
        let expected = || {
            format!(
                "{} {:?}{}",
                item.label,
                item.quote.as_deref().unwrap_or(""),
                item.due
                    .as_deref()
                    .map(|due| format!(" due {due}"))
                    .unwrap_or_default()
            )
        };
        let Some(marker) = got else {
            if marked {
                score.errors.push(format!(
                    "MISS  {:<34} [{}] expected {}",
                    item.id,
                    item.case,
                    expected()
                ));
            }
            continue;
        };
        score.predicted += 1;
        let kind_right = marker.kind.label() == item.label;
        let quote_right = item
            .quote
            .as_deref()
            .is_some_and(|quote| normalized(quote) == normalized(&marker.excerpt));
        if kind_right {
            score.right_kind += 1;
        }
        let got_text = format!(
            "{} {:?}{}",
            marker.kind.label(),
            marker.excerpt,
            marker
                .due
                .map(|due| format!(" due {due}"))
                .unwrap_or_default()
        );
        let an_also = item.also.iter().any(|also| {
            also.label == marker.kind.label()
                && normalized(&also.quote) == normalized(&marker.excerpt)
        });
        if (kind_right && quote_right) || an_also {
            score.right_or_also += 1;
        }
        if kind_right && quote_right {
            score.right += 1;
            if let Some(due) = item.due.as_deref() {
                score.due_expected += 1;
                if marker.due == Some(date(due)) {
                    score.due_right += 1;
                } else {
                    score.errors.push(format!(
                        "DUE   {:<34} [{}] expected {} got {}",
                        item.id,
                        item.case,
                        expected(),
                        got_text
                    ));
                }
            }
            continue;
        }
        if !marked {
            *score
                .false_positives_by_case
                .entry(item.case.clone())
                .or_default() += 1;
        }
        let tag = if an_also {
            "ALSO "
        } else if marked {
            "WRONG"
        } else {
            "FALSE"
        };
        score.errors.push(format!(
            "{tag} {:<34} [{}] expected {} got {}",
            item.id,
            item.case,
            expected(),
            got_text
        ));
    }
    score
}

fn report(name: &str, score: &Score) {
    println!("== {name} ==");
    println!(
        "markers made {}, right {} -> precision {:.3} (another genuine ask counted right: {:.3}; right kind, any sentence: {:.3})",
        score.predicted,
        score.right,
        score.precision(),
        score.lenient_precision(),
        score.kind_precision()
    );
    println!(
        "labelled {} -> recall {:.3}; due dates right {}/{}",
        score.labelled,
        score.recall(),
        score.due_right,
        score.due_expected
    );
    println!(
        "false positives by case: {:?}",
        score.false_positives_by_case
    );
    for error in &score.errors {
        println!("  {error}");
    }
    println!();
}

#[test]
fn rules_alone_measured_on_the_dataset() {
    let as_written = measure(Rules::AsWritten);
    let with_fixes = measure(Rules::WithFixes);
    report("R10 as written", &as_written);
    report("R10 with four general fixes", &with_fixes);

    // The spike's answer, where a change to the dataset or the rules shows
    // it moving: R10 as written falls short of SC-013's 0.9, and the four
    // general fixes reach it, by one marker. Should a later dataset push the
    // fixed rules under 0.9, R10's fallback applies and T116 adds the small
    // weights table.
    assert!(
        as_written.precision() < 0.9,
        "R10 as written now reaches {:.3}: the fixes are no longer what gets it there",
        as_written.precision()
    );
    assert!(
        with_fixes.precision() >= 0.9,
        "rules alone reach only {:.3} on the dataset: T116 needs the weights table (R10)",
        with_fixes.precision()
    );
}
