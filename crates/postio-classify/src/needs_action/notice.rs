//! Strong patterns only: automated mail is full of amounts and dates that ask nothing.

use chrono::{DateTime, Local, TimeZone, Utc};

use super::{
    contains_word, deadline, ends_in_question_mark, lexicon, sentences, starts_with_words,
    straightened,
};
use crate::input::{BodyMessage, OwnText};
use crate::outcome::{MarkedBy, MarkerCandidate, MarkerKind};

pub(crate) fn notice(message: &BodyMessage<'_>, text: &OwnText<'_>) -> Option<MarkerCandidate> {
    let mail = message.filed.message;
    let sent = mail.date.unwrap_or(mail.received_at).with_timezone(&Local);
    let whole = flat(text.as_str());
    if lexicon::NOTICE_NO_ACTION
        .iter()
        .any(|phrase| whole.contains(phrase))
    {
        return None;
    }
    for clause in sentences::clauses(text.as_str()) {
        let lower = flat(&clause.text);
        if lexicon::FOOTERS.iter().any(|footer| lower.contains(footer)) {
            break;
        }
        if ends_in_question_mark(&lower)
            || starts_with_words(&lower, "if")
            || lexicon::NOTICE_SETTLED
                .iter()
                .any(|word| contains_word(&lower, word))
        {
            continue;
        }
        if let Some(due) = obligation(&lower, &sent) {
            return Some(MarkerCandidate {
                kind: MarkerKind::Todo,
                span: Some(clause.span.clone()),
                starts_at: None,
                ends_at: None,
                due_at: due.map(|due| due.with_timezone(&Utc)),
                invite: None,
                by: MarkedBy::Notice,
            });
        }
    }
    None
}

fn flat(text: &str) -> String {
    straightened(text)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// `Some(None)`: owed with no day, which only an overdue amount is.
fn obligation<Tz: TimeZone>(lower: &str, sent: &DateTime<Tz>) -> Option<Option<DateTime<Tz>>> {
    let day = || deadline::deadline_after(lower, sent, lexicon::NOTICE_DAYS);
    let has = |phrases: &[&str]| phrases.iter().any(|phrase| lower.contains(phrase));
    let amount = amount(lower);
    if amount && (contains_word(lower, "overdue") || lower.contains("past due")) {
        return Some(day());
    }
    let owed = amount
        && lexicon::NOTICE_PAYMENT
            .iter()
            .any(|word| contains_word(lower, word));
    let forms = has(lexicon::NOTICE_FORMS) && contains_word(lower, "appointment");
    if owed || forms || has(lexicon::NOTICE_RENEWALS) {
        return day().map(Some);
    }
    None
}

fn amount(lower: &str) -> bool {
    let chars: Vec<char> = lower.chars().collect();
    let digit_after = |at: usize| {
        chars[at..]
            .iter()
            .find(|c| !c.is_whitespace())
            .is_some_and(char::is_ascii_digit)
    };
    let signed = chars
        .iter()
        .enumerate()
        .any(|(at, c)| lexicon::CURRENCY_SIGNS.contains(c) && digit_after(at + 1));
    let words: Vec<&str> = lower.split_whitespace().collect();
    signed
        || words.windows(2).any(|pair| {
            lexicon::CURRENCY_CODES.contains(&pair[0])
                && pair[1].starts_with(|c: char| c.is_ascii_digit())
        })
}
