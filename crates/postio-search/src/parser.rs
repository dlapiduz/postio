//! The query parser: a pure function from `(&str, today)` to a [`ParsedQuery`]. The grammar is
//! total: every input parses, because a half-typed query is the normal case.
//! A word runs to the next whitespace, except that a `"` suspends that rule; an unterminated quote
//! runs to the end of input. Anything that is not a known keyword stays free text, so `foo:bar` and
//! `https://example.com` search for themselves.

use chrono::NaiveDate;

use crate::date::parse_date;
use crate::query::{
    Clause, Field, Filter, ParsedQuery, Partial, Span, State, TextTerm, Token, TokenKind,
};
use crate::size::parse_size;

/// `today` is a parameter, not a clock read, so the function is pure and the bar can re-parse on
/// every keystroke. Never fails: half-typed operators become [`Partial`]s, unknown operators free
/// text.
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
        forgiving: false,
        contact_aware: false,
    }
}

fn next_word_start(input: &str, from: usize) -> Option<usize> {
    input[from..]
        .char_indices()
        .find(|(_, ch)| !ch.is_whitespace())
        .map(|(offset, _)| from + offset)
}

/// Whitespace ends a word unless inside quotes or a set's braces (D26); an unclosed quote or set
/// swallows the rest of the input, which is what a user mid-phrase expects highlighted.
pub(crate) fn word_end(input: &str, start: usize) -> usize {
    scan(&input[start..])
        .0
        .map_or(input.len(), |end| start + end)
}

/// A `{` opens a set only straight after an operator's colon (`from:{`, `-label:{`); inside a set a
/// quote suspends the closing brace.
fn scan(word: &str) -> (Option<usize>, bool, bool) {
    let mut in_quotes = false;
    let mut in_set = false;
    for (offset, ch) in word.char_indices() {
        match ch {
            '"' => in_quotes = !in_quotes,
            '{' if !in_quotes && !in_set && opens_set(&word[..offset]) => in_set = true,
            '}' if !in_quotes && in_set => in_set = false,
            ch if ch.is_whitespace() && !in_quotes && !in_set => {
                return (Some(offset), in_quotes, in_set);
            }
            _ => {}
        }
    }
    (None, in_quotes, in_set)
}

pub(crate) fn leaves_set_open(word: &str) -> bool {
    scan(word).2
}

fn opens_set(before: &str) -> bool {
    let body = before.strip_prefix('-').unwrap_or(before);
    body.strip_suffix(':')
        .is_some_and(|keyword| !keyword.contains([':', '"']) && Field::parse(keyword).is_some())
}

fn classify(raw: &str, today: NaiveDate) -> Option<TokenKind> {
    if raw.chars().all(|ch| ch == '-' || ch == '"') {
        return None;
    }
    let (negated, body) = match raw.strip_prefix('-') {
        Some(rest) if !rest.is_empty() => (true, rest),
        _ => (false, raw),
    };

    if let Some((keyword, value)) = split_operator(body)
        && let Some(field) = Field::parse(keyword)
    {
        return Some(operator(negated, field, value, today));
    }

    let value = unquote(body);
    if value.is_empty() {
        return None;
    }
    let quoted = body.starts_with('"');
    Some(TokenKind::Text(TextTerm {
        negated,
        value,
        quoted,
    }))
}

/// Splits at the first colon, provided no quote opened before it: `"re: invoice"` is a phrase.
fn split_operator(body: &str) -> Option<(&str, &str)> {
    let colon = body.char_indices().find_map(|(offset, ch)| match ch {
        '"' => Some(Err(())),
        ':' => Some(Ok(offset)),
        _ => None,
    })?;
    let colon = colon.ok()?;
    Some((&body[..colon], &body[colon + 1..]))
}

fn unquote(value: &str) -> String {
    value.trim_matches('"').to_string()
}

/// Unquoting happens here because `header:` splits its own value first: `header:x-mailer="mutt
/// 1.5"` opens its quote inside the value, so stripping the outer quotes before the `=` leaves the
/// opening one in.
fn operator(negated: bool, field: Field, raw: &str, today: NaiveDate) -> TokenKind {
    let partial = |value: String| {
        TokenKind::Partial(Partial {
            negated,
            field,
            value,
        })
    };
    let filter = |filter: Filter| TokenKind::Filter(Clause { negated, filter });

    if field == Field::Header {
        return header(negated, raw);
    }
    if raw.starts_with('{') {
        return match set(field, raw, today) {
            Some(filter) => TokenKind::Filter(Clause { negated, filter }),
            None => partial(raw.to_string()),
        };
    }

    let value = unquote(raw);
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
        Field::Label => filter(Filter::Label(value)),
        Field::With => {
            let addresses: Vec<String> = value
                .split(',')
                .map(str::trim)
                .filter(|address| !address.is_empty())
                .map(str::to_owned)
                .collect();
            if addresses.is_empty() {
                partial(value)
            } else {
                filter(Filter::With(addresses))
            }
        }
        Field::Has => match value.to_ascii_lowercase().as_str() {
            "attach" | "attachment" | "attachments" | "file" | "files" => {
                filter(Filter::HasAttachment)
            }
            "action" | "actions" => filter(Filter::HasAction),
            _ => partial(value),
        },
        Field::Is => match value.to_ascii_lowercase().as_str() {
            "unread" | "new" => filter(Filter::Is(State::Unread)),
            "read" | "seen" => filter(Filter::Is(State::Read)),
            // `is:starred` is an alias kept for the old spelling; the canvas says Flagged.
            "flagged" | "starred" | "star" => filter(Filter::Is(State::Flagged)),
            "bulk" => filter(Filter::Is(State::Bulk)),
            "automated" => filter(Filter::Is(State::Automated)),
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
        Field::Header => unreachable!("header: is split before the value is unquoted"),
    }
}

fn set(field: Field, raw: &str, today: NaiveDate) -> Option<Filter> {
    if !field.takes_set() {
        return None;
    }
    let inner = raw.strip_prefix('{')?.strip_suffix('}')?;
    if inner_closes_early(inner) {
        return None;
    }
    let mut members = Vec::new();
    let mut cursor = 0;
    while let Some(start) = next_word_start(inner, cursor) {
        let end = inner_word_end(inner, start);
        cursor = end;
        let value = &inner[start..end];
        if value.starts_with('{') {
            return None;
        }
        match operator(false, field, value, today) {
            TokenKind::Filter(clause) => members.push(clause.filter),
            _ => return None,
        }
    }
    Filter::any_of(members)
}

fn inner_closes_early(inner: &str) -> bool {
    let mut in_quotes = false;
    inner.chars().any(|ch| {
        if ch == '"' {
            in_quotes = !in_quotes;
        }
        ch == '}' && !in_quotes
    })
}

fn inner_word_end(inner: &str, start: usize) -> usize {
    let mut in_quotes = false;
    for (offset, ch) in inner[start..].char_indices() {
        if ch == '"' {
            in_quotes = !in_quotes;
        } else if !in_quotes && ch.is_whitespace() {
            return start + offset;
        }
    }
    inner.len()
}

/// Split at the first `=`: a structured value contains its own (`authentication-results=spf=pass`).
/// Both halves are normalized by `postio_model::headers`, so the index, the in-memory matcher and
/// `Filter::Header` compare the same form.
fn header(negated: bool, raw: &str) -> TokenKind {
    let (name, value) = match raw.split_once('=') {
        Some((name, value)) => (name, Some(value)),
        None => (raw, None),
    };
    let name = postio_model::headers::normalize_name(&unquote(name));
    if name.is_empty() {
        return TokenKind::Partial(Partial {
            negated,
            field: Field::Header,
            value: raw.to_string(),
        });
    }
    // `header:x-mailer=` is presence, never an error, and never
    // `Some("")` -- see `Filter::Header`.
    let value = value
        .map(|value| postio_model::headers::normalize_value(&unquote(value)))
        .filter(|value| !value.is_empty());
    TokenKind::Filter(Clause {
        negated,
        filter: Filter::Header { name, value },
    })
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
