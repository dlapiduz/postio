//! The query parser itself: a pure function from `(&str, today)` to a
//! [`ParsedQuery`].
//!
//! The grammar is intentionally tiny and total — *every* input parses, because
//! results update on every keystroke and a half-typed query is the normal case,
//! not an error case:
//!
//! ```text
//! query   := token*
//! token   := '-'? ( operator | text )
//! operator:= keyword ':' value          (keyword is one Postio knows)
//! text    := word | '"' phrase '"'
//! ```
//!
//! A word runs to the next whitespace, except that a `"` suspends that rule so
//! `subject:"quarterly report"` is one token. An unterminated quote simply runs
//! to the end of the input — the user is still typing it.
//!
//! Anything that is not a keyword Postio knows stays free text, so `foo:bar`
//! and `https://example.com` search for themselves instead of erroring.

use chrono::NaiveDate;

use crate::date::parse_date;
use crate::query::{
    Clause, Field, Filter, ParsedQuery, Partial, Span, State, TextTerm, Token, TokenKind,
};
use crate::size::parse_size;

/// Parses a search query.
///
/// `today` is the reference date every relative date (`after:yesterday`,
/// `after:"last quarter"`) resolves against. It is a parameter rather than a
/// clock read on purpose: this function is pure, which is what makes it
/// exhaustively testable, and what lets the search bar re-parse on every
/// keystroke without side effects.
///
/// This never fails. Half-typed operators become [`Partial`]s, unknown
/// operators become free text, and an empty input becomes an empty query.
pub fn parse(input: &str, today: NaiveDate) -> ParsedQuery {
    let mut tokens = Vec::new();
    let mut cursor = 0usize;

    while let Some(start) = next_word_start(input, cursor) {
        let end = word_end(input, start);
        cursor = end;
        let raw = &input[start..end];
        if let Some(kind) = classify(raw, today) {
            tokens.push(Token {
                span: Span::new(start, end),
                raw: raw.to_string(),
                kind,
            });
        }
    }

    ParsedQuery {
        input: input.to_string(),
        tokens,
    }
}

/// Byte offset of the next non-whitespace character at or after `from`.
fn next_word_start(input: &str, from: usize) -> Option<usize> {
    input[from..]
        .char_indices()
        .find(|(_, ch)| !ch.is_whitespace())
        .map(|(offset, _)| from + offset)
}

/// Byte offset just past the word starting at `start`.
///
/// Whitespace ends a word, unless it sits inside quotes. An unclosed quote
/// swallows the rest of the input, which is exactly what a user mid-phrase
/// expects to see highlighted.
fn word_end(input: &str, start: usize) -> usize {
    let mut in_quotes = false;
    for (offset, ch) in input[start..].char_indices() {
        if ch == '"' {
            in_quotes = !in_quotes;
        } else if !in_quotes && ch.is_whitespace() {
            return start + offset;
        }
    }
    input.len()
}

/// Turns one raw word into a token, or `None` if it carries no meaning at all
/// (a lone `-`, a lone `"`), in which case it is dropped.
fn classify(raw: &str, today: NaiveDate) -> Option<TokenKind> {
    // Punctuation on its own is someone mid-keystroke, not a search term.
    if raw.chars().all(|ch| ch == '-' || ch == '"') {
        return None;
    }
    let (negated, body) = match raw.strip_prefix('-') {
        // A bare `-` is not a negation of anything yet.
        Some(rest) if !rest.is_empty() => (true, rest),
        _ => (false, raw),
    };

    if let Some((keyword, value)) = split_operator(body)
        && let Some(field) = Field::parse(keyword)
    {
        return Some(operator(negated, field, unquote(value), today));
    }

    let value = unquote(body);
    if value.is_empty() {
        return None;
    }
    Some(TokenKind::Text(TextTerm { negated, value }))
}

/// Splits `keyword:value` at the first colon, provided no quote opened before
/// it — `"re: invoice"` is a phrase, not an operator.
fn split_operator(body: &str) -> Option<(&str, &str)> {
    let colon = body.char_indices().find_map(|(offset, ch)| match ch {
        '"' => Some(Err(())),
        ':' => Some(Ok(offset)),
        _ => None,
    })?;
    let colon = colon.ok()?;
    Some((&body[..colon], &body[colon + 1..]))
}

/// Strips the quotes a user typed around a value. Quotes only ever group
/// characters; they are never part of the value.
fn unquote(value: &str) -> String {
    value.trim_matches('"').to_string()
}

/// Builds the token for a recognized operator, falling back to a [`Partial`]
/// whenever the value is not usable yet.
fn operator(negated: bool, field: Field, value: String, today: NaiveDate) -> TokenKind {
    let partial = |value: String| {
        TokenKind::Partial(Partial {
            negated,
            field,
            value,
        })
    };
    let filter = |filter: Filter| TokenKind::Filter(Clause { negated, filter });

    if value.is_empty() {
        return partial(value);
    }

    match field {
        Field::From => filter(Filter::From(value)),
        Field::To => filter(Filter::To(value)),
        Field::Subject => filter(Filter::Subject(value)),
        Field::In => filter(Filter::In(value)),
        Field::Filename => filter(Filter::Filename(value)),
        Field::List => filter(Filter::List(value)),
        Field::Has => match value.to_ascii_lowercase().as_str() {
            "attach" | "attachment" | "attachments" | "file" | "files" => {
                filter(Filter::HasAttachment)
            }
            _ => partial(value),
        },
        Field::Is => match value.to_ascii_lowercase().as_str() {
            "unread" | "new" => filter(Filter::Is(State::Unread)),
            "read" | "seen" => filter(Filter::Is(State::Read)),
            // An earlier brief wrote `is:starred`; the canvas renamed it to
            // Flagged, and docs/PRODUCT.md §7 keeps the old spelling as an alias.
            "flagged" | "starred" | "star" => filter(Filter::Is(State::Flagged)),
            _ => partial(value),
        },
        Field::After => match parse_date(&value, today) {
            Some(date) => filter(Filter::After(date)),
            None => partial(value),
        },
        Field::Before => match parse_date(&value, today) {
            Some(date) => filter(Filter::Before(date)),
            None => partial(value),
        },
        Field::Larger => match parse_size(&value) {
            Some(bytes) => filter(Filter::Larger(bytes)),
            None => partial(value),
        },
        Field::Smaller => match parse_size(&value) {
            Some(bytes) => filter(Filter::Smaller(bytes)),
            None => partial(value),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 8, 22).unwrap()
    }

    #[test]
    fn word_end_respects_quotes() {
        let input = r#"subject:"a b" c"#;
        assert_eq!(word_end(input, 0), 13);
        assert_eq!(word_end(input, 14), input.len());
    }

    #[test]
    fn word_end_runs_to_the_end_on_an_unclosed_quote() {
        let input = r#"subject:"a b"#;
        assert_eq!(word_end(input, 0), input.len());
    }

    #[test]
    fn split_operator_ignores_colons_inside_quotes() {
        assert_eq!(split_operator("from:a"), Some(("from", "a")));
        assert_eq!(split_operator(r#""re: x""#), None);
        assert_eq!(split_operator("plain"), None);
        assert_eq!(split_operator(":"), Some(("", "")));
    }

    #[test]
    fn unquote_strips_only_quotes() {
        assert_eq!(unquote(r#""a b""#), "a b");
        assert_eq!(unquote(r#""a b"#), "a b");
        assert_eq!(unquote(r#""""#), "");
        assert_eq!(unquote("a"), "a");
    }

    #[test]
    fn classify_drops_meaningless_words() {
        assert_eq!(classify("-", today()), None);
        assert_eq!(classify("\"", today()), None);
        assert_eq!(classify("\"\"", today()), None);
        assert_eq!(classify("----", today()), None);
        assert_eq!(classify("\"\"\"\"", today()), None);
    }

    #[test]
    fn negation_needs_something_to_negate() {
        let Some(TokenKind::Text(term)) = classify("-x", today()) else {
            panic!("expected negated text");
        };
        assert!(term.negated);
        assert_eq!(term.value, "x");
    }
}
