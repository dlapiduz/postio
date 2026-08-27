//! The structured result of parsing a search query.
//!
//! A [`ParsedQuery`] is a flat, ordered list of [`Token`]s. That shape is
//! deliberate: the query executor wants the *filters* and the *free text*
//! separated, while the search bar wants the *tokens in source order with their
//! spans* so it can draw one chip per token and pop the chip under the caret
//! with Backspace. Both views come off the same list — see [`ParsedQuery::filters`]
//! and [`ParsedQuery::tokens`].

use chrono::NaiveDate;

/// A byte range inside the original query string.
///
/// Always lands on `char` boundaries, so `&input[span.start..span.end]` is safe
/// for the input the query was parsed from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Span {
    /// Byte offset of the first character of the token.
    pub start: usize,
    /// Byte offset one past the last character of the token.
    pub end: usize,
}

impl Span {
    /// Builds a span.
    pub fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }

    /// Length of the span in bytes.
    pub fn len(&self) -> usize {
        self.end.saturating_sub(self.start)
    }

    /// Whether the span covers no text.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Whether `offset` falls inside the span, counting both edges so a caret
    /// resting against either end of a chip still selects it.
    pub fn contains(&self, offset: usize) -> bool {
        offset >= self.start && offset <= self.end
    }
}

/// The operator keywords Postio understands.
///
/// The spellings come from the design canvas (artboard 2b) and are recorded in
/// `docs/PRODUCT.md` §7: it is `has:attach` and `is:flagged`, with the older
/// `has:attachment` and `is:starred` accepted as aliases.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Field {
    /// `from:` — sender address or display name.
    From,
    /// `to:` — recipient address or display name (To, Cc or Bcc).
    To,
    /// `subject:` — subject line.
    Subject,
    /// `has:` — a structural property, currently only `has:attach`.
    Has,
    /// `is:` — a flag state: `is:unread`, `is:read`, `is:flagged`.
    Is,
    /// `before:` — messages strictly older than a date.
    Before,
    /// `after:` — messages on or after a date.
    After,
    /// `in:` — mailbox name or role.
    In,
    /// `filename:` — attachment filename.
    Filename,
    /// `larger:` — message size floor.
    Larger,
    /// `smaller:` — message size ceiling.
    Smaller,
    /// `list:` — `List-Id` mailing list.
    List,
    /// `account:` — which account's mail, by name or address.
    ///
    /// Orthogonal to the tri-tab's role scope rather than a fourth value of
    /// it (#186): "this account's inbox" and "every account's inbox" are both
    /// things to be able to ask for, so account and role compose.
    Account,
    /// `group:` — a named contact group, by name.
    ///
    /// ADR 0007 Q3: a group answers *which people*, not which messages, so
    /// unlike every other field here it cannot be expressed any other way
    /// in this language — it composes with the rest rather than replacing
    /// them, resolved by `postio-index` to the member address set.
    Group,
}

impl Field {
    /// Every field, in the order the search bar's completion popup offers them.
    pub const ALL: &'static [Field] = &[
        Field::From,
        Field::To,
        Field::Subject,
        Field::Has,
        Field::Is,
        Field::Before,
        Field::After,
        Field::In,
        Field::Filename,
        Field::Larger,
        Field::Smaller,
        Field::List,
        Field::Account,
        Field::Group,
    ];

    /// The canonical keyword, without the trailing colon.
    pub fn keyword(&self) -> &'static str {
        match self {
            Field::From => "from",
            Field::To => "to",
            Field::Subject => "subject",
            Field::Has => "has",
            Field::Is => "is",
            Field::Before => "before",
            Field::After => "after",
            Field::In => "in",
            Field::Filename => "filename",
            Field::Larger => "larger",
            Field::Smaller => "smaller",
            Field::List => "list",
            Field::Account => "account",
            Field::Group => "group",
        }
    }

    /// Resolves a keyword, case-insensitively. Unknown keywords are not
    /// operators at all — the caller treats them as free text.
    pub fn parse(keyword: &str) -> Option<Field> {
        match keyword.to_ascii_lowercase().as_str() {
            "from" => Some(Field::From),
            "to" => Some(Field::To),
            "subject" | "title" => Some(Field::Subject),
            "has" => Some(Field::Has),
            "is" => Some(Field::Is),
            "before" => Some(Field::Before),
            "after" | "since" => Some(Field::After),
            "in" | "folder" | "mailbox" => Some(Field::In),
            "filename" | "file" | "attachment" => Some(Field::Filename),
            "larger" | "bigger" | "size" => Some(Field::Larger),
            "smaller" => Some(Field::Smaller),
            "list" => Some(Field::List),
            "account" => Some(Field::Account),
            "group" => Some(Field::Group),
            _ => None,
        }
    }

    /// Whether the field takes a free-form value that is useful the moment the
    /// first character is typed (`from:al` already narrows), as opposed to one
    /// drawn from a fixed vocabulary (`is:`) or needing a full parse (`after:`).
    pub fn takes_free_text(&self) -> bool {
        matches!(
            self,
            Field::From
                | Field::To
                | Field::Subject
                | Field::In
                | Field::Filename
                | Field::List
                | Field::Account
                | Field::Group
        )
    }
}

/// A message flag state, for `is:`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum State {
    /// `is:unread` — `\Seen` is absent.
    Unread,
    /// `is:read` — `\Seen` is present.
    Read,
    /// `is:flagged` — `\Flagged` is present. The canvas says "Flagged", never
    /// "Starred", but `is:starred` is accepted on input.
    Flagged,
}

/// One structured constraint, with its value already parsed.
///
/// Values stay as plain data — no `MailboxId`, no `Flag`, no SQL. Resolving
/// `in:archive` against the account's folders and turning dates into timestamps
/// is the executor's job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Filter {
    /// `from:alice`
    From(String),
    /// `to:bob`
    To(String),
    /// `subject:invoice`
    Subject(String),
    /// `in:archive` — a mailbox name, path or role name.
    In(String),
    /// `filename:contract.pdf`
    Filename(String),
    /// `list:lkml`
    List(String),
    /// `account:work` — an account by name or address, unresolved.
    ///
    /// Deliberately still text. Resolving it to an `AccountId` needs the
    /// store, which this crate does not have and must not grow: a saved
    /// search in `[filters]` is the string the user typed, and it has to keep
    /// meaning the same thing after an account is removed and re-added under
    /// a new id.
    Account(String),
    /// `group:family` — a contact group by name, unresolved.
    ///
    /// Stays text for the same reason `Account` does: resolving it to
    /// member addresses needs the store, which this crate does not have.
    /// `postio-index` does the resolving.
    Group(String),
    /// `has:attach`
    HasAttachment,
    /// `is:unread`, `is:read`, `is:flagged`
    Is(State),
    /// `after:2026-01-01` — on or after this date, inclusive.
    After(NaiveDate),
    /// `before:2026-02-01` — strictly before this date.
    Before(NaiveDate),
    /// `larger:1M` — size in bytes, inclusive.
    Larger(u64),
    /// `smaller:1M` — size in bytes, inclusive.
    Smaller(u64),
}

impl Filter {
    /// The operator this filter came from, for chip labels and completion.
    pub fn field(&self) -> Field {
        match self {
            Filter::From(_) => Field::From,
            Filter::To(_) => Field::To,
            Filter::Subject(_) => Field::Subject,
            Filter::In(_) => Field::In,
            Filter::Filename(_) => Field::Filename,
            Filter::List(_) => Field::List,
            Filter::Account(_) => Field::Account,
            Filter::Group(_) => Field::Group,
            Filter::HasAttachment => Field::Has,
            Filter::Is(_) => Field::Is,
            Filter::After(_) => Field::After,
            Filter::Before(_) => Field::Before,
            Filter::Larger(_) => Field::Larger,
            Filter::Smaller(_) => Field::Smaller,
        }
    }
}

/// A [`Filter`] plus whether it was negated with a leading `-`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Clause {
    /// `-from:bob` excludes rather than includes.
    pub negated: bool,
    /// The constraint itself.
    pub filter: Filter,
}

/// A recognized operator whose value is not usable yet.
///
/// This is what keeps as-you-type search from erroring: `is:` and `is:unr` and
/// `after:2026-` are all perfectly ordinary intermediate states. A partial
/// constrains nothing — the executor ignores it — but it carries enough for the
/// search bar to draw a pending chip and offer completions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Partial {
    /// Whether a leading `-` was typed.
    pub negated: bool,
    /// The operator that was recognized.
    pub field: Field,
    /// Whatever has been typed after the colon so far, possibly empty.
    pub value: String,
}

/// A free-text term, destined for the FTS5 `MATCH` expression.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextTerm {
    /// `-docker` excludes the term.
    pub negated: bool,
    /// The term with any surrounding quotes removed. A quoted term keeps its
    /// spaces and is matched as an FTS5 phrase.
    pub value: String,
}

/// What a [`Token`] turned out to be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenKind {
    /// A complete operator that constrains results.
    Filter(Clause),
    /// A recognized operator that is still being typed.
    Partial(Partial),
    /// Free text.
    Text(TextTerm),
}

/// One chip's worth of query: a slice of the input and what it means.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    /// Where the token sits in the original query string.
    pub span: Span,
    /// The exact source text, quotes, leading `-` and all. This is the chip's
    /// label and what [`ParsedQuery::remove_token`] deletes.
    pub raw: String,
    /// The parsed meaning.
    pub kind: TokenKind,
}

impl Token {
    /// The operator this token belongs to, or `None` for free text.
    pub fn field(&self) -> Option<Field> {
        match &self.kind {
            TokenKind::Filter(clause) => Some(clause.filter.field()),
            TokenKind::Partial(partial) => Some(partial.field),
            TokenKind::Text(_) => None,
        }
    }

    /// Whether the token was negated with a leading `-`.
    pub fn negated(&self) -> bool {
        match &self.kind {
            TokenKind::Filter(clause) => clause.negated,
            TokenKind::Partial(partial) => partial.negated,
            TokenKind::Text(term) => term.negated,
        }
    }

    /// Whether this token should be drawn as a chip rather than as plain text.
    pub fn is_operator(&self) -> bool {
        !matches!(self.kind, TokenKind::Text(_))
    }
}

/// A parsed query: everything the executor and the search bar need, and nothing
/// that depends on the clock, the database or the network.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ParsedQuery {
    pub(crate) input: String,
    pub(crate) tokens: Vec<Token>,
}

impl ParsedQuery {
    /// The query string this was parsed from.
    pub fn input(&self) -> &str {
        &self.input
    }

    /// Every token, in source order.
    pub fn tokens(&self) -> &[Token] {
        &self.tokens
    }

    /// Whether the query constrains nothing at all.
    pub fn is_empty(&self) -> bool {
        self.tokens.is_empty()
    }

    /// The structured constraints, in source order.
    pub fn filters(&self) -> impl Iterator<Item = &Clause> {
        self.tokens.iter().filter_map(|token| match &token.kind {
            TokenKind::Filter(clause) => Some(clause),
            _ => None,
        })
    }

    /// Operators that are still being typed. They constrain nothing.
    pub fn partials(&self) -> impl Iterator<Item = &Partial> {
        self.tokens.iter().filter_map(|token| match &token.kind {
            TokenKind::Partial(partial) => Some(partial),
            _ => None,
        })
    }

    /// The free-text terms, in source order.
    pub fn text_terms(&self) -> impl Iterator<Item = &TextTerm> {
        self.tokens.iter().filter_map(|token| match &token.kind {
            TokenKind::Text(term) => Some(term),
            _ => None,
        })
    }

    /// The token under a caret at `offset` bytes into the input.
    ///
    /// Both edges count, so pressing Backspace with the caret against the right
    /// edge of a chip pops that chip.
    pub fn token_at(&self, offset: usize) -> Option<&Token> {
        self.tokens.iter().find(|token| token.span.contains(offset))
    }

    /// The query string with token `index` removed and the surrounding
    /// whitespace tidied — what the search bar sets its entry to when a chip is
    /// popped. An out-of-range index returns the input unchanged.
    pub fn remove_token(&self, index: usize) -> String {
        let Some(token) = self.tokens.get(index) else {
            return self.input.clone();
        };
        let before = self.input[..token.span.start].trim_end();
        let after = self.input[token.span.end..].trim_start();
        match (before.is_empty(), after.is_empty()) {
            (true, _) => after.to_string(),
            (false, true) => before.to_string(),
            (false, false) => format!("{before} {after}"),
        }
    }

    /// The free-text portion as an FTS5 `MATCH` expression, or `None` when
    /// there is nothing positive to match on.
    ///
    /// Every term is emitted as a quoted FTS5 string literal, so words a user
    /// types — `AND`, `OR`, `NEAR`, `*`, `(` — are matched literally instead of
    /// being read as query syntax. Negated terms become an FTS5 `NOT` group,
    /// which needs something on its left; a query whose only free text is
    /// negated therefore yields `None`, and the executor excludes those terms
    /// itself using [`ParsedQuery::text_terms`].
    pub fn fts_match(&self) -> Option<String> {
        let mut positive = Vec::new();
        let mut negative = Vec::new();
        for term in self.text_terms() {
            let literal = fts_literal(&term.value);
            if term.negated {
                negative.push(literal);
            } else {
                positive.push(literal);
            }
        }
        if positive.is_empty() {
            return None;
        }
        let matched = positive.join(" AND ");
        if negative.is_empty() {
            Some(matched)
        } else {
            Some(format!("({matched}) NOT ({})", negative.join(" OR ")))
        }
    }
}

/// Wraps a term as an FTS5 string literal, doubling embedded quotes.
///
/// `pub` rather than private: `postio-index`'s executor needs it too, to
/// build the exclusion `MATCH` it runs itself when
/// [`ParsedQuery::fts_match`] returns `None` for a query that is all negated
/// text (see that method's docs).
pub fn fts_literal(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for ch in value.chars() {
        if ch == '"' {
            out.push('"');
        }
        out.push(ch);
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fts_literal_doubles_quotes() {
        assert_eq!(fts_literal(r#"say "hi""#), r#""say ""hi""""#);
    }

    #[test]
    fn span_contains_both_edges() {
        let span = Span::new(2, 5);
        assert!(span.contains(2));
        assert!(span.contains(5));
        assert!(!span.contains(1));
        assert!(!span.contains(6));
        assert_eq!(span.len(), 3);
        assert!(!span.is_empty());
        assert!(Span::new(4, 4).is_empty());
    }

    #[test]
    fn unknown_keywords_are_not_fields() {
        assert_eq!(Field::parse("nope"), None);
        assert_eq!(Field::parse(""), None);
        assert_eq!(Field::parse("FROM"), Some(Field::From));
    }

    #[test]
    fn only_text_valued_fields_take_free_text() {
        assert!(Field::From.takes_free_text());
        assert!(!Field::Is.takes_free_text());
        assert!(!Field::After.takes_free_text());
    }
}
