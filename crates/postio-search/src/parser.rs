//! The query parser itself: a pure function from `(&str, today)` to a
//! [`ParsedQuery`].
//!
//! The grammar is intentionally tiny and total — *every* input parses, because
//! results update on every keystroke and a half-typed query is the normal case,
//! not an error case:
//!
//! ```text
//! query   := disjunction
//! disjunction := conjunction ( 'OR' conjunction )*
//! conjunction := factor+
//! factor  := '(' disjunction ')' | token
//! token   := '-'? ( operator | text )
//! operator:= keyword ':' value          (keyword is one Postio knows)
//! text    := word | '"' phrase '"'
//! ```
//!
//! `AND` is juxtaposition and binds tighter than `OR`, so
//! `from:ada OR from:grace has:attach` is `ada OR (grace AND attach)` (#478).
//! There is no `AND` keyword: adjacency already means it, and a second
//! spelling of the same thing is a second thing to get wrong.
//!
//! The token vector stays *flat* and lexical — it is what the search bar
//! draws chips from, and chips did not change. The boolean structure is
//! derived on demand by [`ParsedQuery::tree`].
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
        // A parenthesis is punctuation, not part of the word it is touching:
        // `(from:grace` is a group opening on a filter. Peeled off both ends
        // before classifying, and only outside quotes -- `subject:"(draft)"`
        // is a phrase that happens to contain brackets.
        let mut offset = start;
        let raw = &input[start..end];
        if !raw.starts_with('"') {
            while input[offset..end].starts_with('(') {
                tokens.push(punctuation(input, offset, TokenKind::Open));
                offset += 1;
            }
        }
        let mut limit = end;
        let mut trailing = Vec::new();
        if !input[offset..limit].ends_with('"') {
            while limit > offset && input[offset..limit].ends_with(')') {
                limit -= 1;
                trailing.push(punctuation(input, limit, TokenKind::Close));
            }
        }
        let body = &input[offset..limit];
        if !body.is_empty() {
            let kind = if body == OR {
                Some(TokenKind::Or)
            } else {
                classify(body, today)
            };
            if let Some(kind) = kind {
                tokens.push(Token {
                    span: Span::new(offset, limit),
                    raw: body.to_string(),
                    kind,
                });
            }
        }
        trailing.reverse();
        tokens.append(&mut trailing);
    }

    ParsedQuery {
        input: input.to_string(),
        tokens,
    }
}

/// The one spelling of the boolean. See [`TokenKind::Or`] for why it shouts.
const OR: &str = "OR";

/// A one-character structural token at `at`.
fn punctuation(input: &str, at: usize, kind: TokenKind) -> Token {
    Token {
        span: Span::new(at, at + 1),
        raw: input[at..at + 1].to_string(),
        kind,
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
        Field::Account => filter(Filter::Account(value)),
        Field::Group => filter(Filter::Group(value)),
        Field::Body => filter(Filter::Body(value)),
        // `header:x-mailer=mutt` splits on the first `=`; `header:x-mailer`
        // with no `=` asks whether the header is there at all, which is a
        // complete question and so a filter rather than a partial. Names are
        // lowercased because header names are case-insensitive (RFC 5322).
        Field::Header => {
            let (name, header_value) = match value.split_once('=') {
                Some((name, header_value)) => (name, header_value),
                None => (value.as_str(), ""),
            };
            if name.is_empty() {
                return partial(value);
            }
            filter(Filter::Header {
                name: name.to_ascii_lowercase(),
                value: header_value.to_string(),
            })
        }
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
