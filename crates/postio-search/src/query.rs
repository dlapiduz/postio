//! The structured result of parsing a search query.
//!
//! A [`ParsedQuery`] is a flat, ordered list of [`Token`]s. That shape is
//! deliberate: the query executor wants the *filters* and the *free text*
//! separated, while the search bar wants the *tokens in source order with their
//! spans* so it can draw one chip per token. Both views come off the same list — see [`ParsedQuery::filters`]
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
    /// `has:` — a structural property: `has:attach`, or `has:action`.
    Has,
    /// `is:` — a flag state, `is:unread`, `is:read`, `is:flagged`, or what a
    /// message's promoted headers say, `is:bulk`, `is:automated`.
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
    /// `header:x-mailer=mutt` — an arbitrary RFC 5322 field, by name and
    /// optionally by what its value contains.
    ///
    /// The general, late-answering operator for everything the envelope does
    /// not carry (ADR 0025). A header that must be matchable *before* the
    /// body arrives is promoted to an operator of its own instead — `list:`
    /// is one that was.
    Header,
    /// `label:` — a label the message carries, by name, case-insensitively.
    ///
    /// Spec 010 (S2): one operator of the shared language, so it means the
    /// same thing in every interface. A name with spaces is quoted, as a
    /// `subject:` phrase is: `label:"Q3 close"`.
    Label,
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
        Field::Label,
        // Last, because the popup is ordered by how often an operator is
        // reached for and this is the one you type when none of the others
        // will do.
        Field::Header,
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
            Field::Header => "header",
            Field::Label => "label",
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
            "header" => Some(Field::Header),
            "label" => Some(Field::Label),
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
                | Field::Header
                | Field::Label
        )
    }
}

impl Field {
    /// Whether the field takes a braced set of values, `from:{ada tomas}`
    /// (spec 010, D26): every field whose value is a name. `header:` has a
    /// grammar of its own inside its value, and a flag, a date or a size
    /// asked for "either" is better asked once.
    pub fn takes_set(&self) -> bool {
        self.takes_free_text() && *self != Field::Header
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
    /// `is:bulk` — the message offers `List-Unsubscribe`, or says
    /// `Precedence: bulk`, `list` or `junk` (spec 007, research R8).
    Bulk,
    /// `is:automated` — the message says `Auto-Submitted` other than `no`.
    Automated,
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
    /// search in `[saved_searches]` is the string the user typed, and it has to keep
    /// meaning the same thing after an account is removed and re-added under
    /// a new id.
    Account(String),
    /// `group:family` — a contact group by name, unresolved.
    ///
    /// Stays text for the same reason `Account` does: resolving it to
    /// member addresses needs the store, which this crate does not have.
    /// `postio-index` does the resolving.
    Group(String),
    /// `header:x-mailer` — presence — or `header:x-mailer=mutt`.
    ///
    /// `name` is already lowercased and `value` already normalized, both by
    /// `postio_model::headers`, because the column this is matched against
    /// holds exactly that form. Doing it here rather than at the executor is
    /// what lets the in-memory matcher (#479) and the index agree without
    /// each writing its own: a `Filter::Header` means the same comparison
    /// wherever it is evaluated.
    ///
    /// `value: None` is `header:x-mailer` — "the message has such a field" —
    /// and is deliberately not `Some("")`: presence and "contains the empty
    /// string" happen to select the same rows today, and would stop doing so
    /// the moment either side grew a notion of an empty value.
    Header {
        /// The field name, lowercased. Matched exactly, never as a substring.
        name: String,
        /// What the value must contain, or `None` to ask only for presence.
        value: Option<String>,
    },
    /// `has:attach`
    HasAttachment,
    /// `label:atlas` — a label by name, unresolved.
    ///
    /// Text for the reason `Account` is: a saved search is the string the
    /// user typed, and a label's id changes when it is re-created.
    /// `postio-index` resolves it against `labels.name`, case-insensitively.
    Label(String),
    /// `has:action` — the message carries an open marker (an undismissed
    /// `markers` row).
    HasAction,
    /// `is:unread`, `is:read`, `is:flagged`, `is:bulk`, `is:automated`
    Is(State),
    /// `after:2026-01-01` — on or after this date, inclusive.
    After(NaiveDate),
    /// `before:2026-02-01` — strictly before this date.
    Before(NaiveDate),
    /// `larger:1M` — size in bytes, inclusive.
    Larger(u64),
    /// `smaller:1M` — size in bytes, inclusive.
    Smaller(u64),
    /// `from:{ada tomas}` — either of several values of one field (spec
    /// 010, D26): holds when any member does, and negated when none does.
    AnyOf(AnyOf),
}

/// Two or more filters of one field that takes a set ([`Field::takes_set`]),
/// none of them a set itself: what `from:{ada tomas}` asks for.
///
/// Its members are private and [`AnyOf::new`] is the only way in, so every
/// reader -- the executor, the conversation search, the matcher, the chips
/// -- can rely on one field and no nesting without checking for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnyOf {
    members: Vec<Filter>,
}

impl AnyOf {
    /// A set of `members`, or `None` when they are fewer than two, of
    /// different fields, of a field that takes no set, or a set among them.
    pub fn new(members: Vec<Filter>) -> Option<AnyOf> {
        let field = members.first()?.field();
        let fits = members.len() >= 2
            && field.takes_set()
            && members
                .iter()
                .all(|member| !matches!(member, Filter::AnyOf(_)) && member.field() == field);
        fits.then_some(AnyOf { members })
    }

    /// The values, as filters, in the order they were written.
    pub fn members(&self) -> &[Filter] {
        &self.members
    }

    /// The one field every member is of.
    pub fn field(&self) -> Field {
        self.members[0].field()
    }
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
            Filter::Header { .. } => Field::Header,
            Filter::HasAttachment => Field::Has,
            Filter::Label(_) => Field::Label,
            Filter::HasAction => Field::Has,
            Filter::Is(_) => Field::Is,
            Filter::After(_) => Field::After,
            Filter::Before(_) => Field::Before,
            Filter::Larger(_) => Field::Larger,
            Filter::Smaller(_) => Field::Smaller,
            Filter::AnyOf(set) => set.field(),
        }
    }

    /// What the filter accepts any one of: a set's members, or the filter
    /// itself. How a reader that lists values -- highlights, chips, a
    /// button's label -- reads a set without matching on it.
    pub fn alternatives(&self) -> &[Filter] {
        match self {
            Filter::AnyOf(set) => set.members(),
            other => std::slice::from_ref(other),
        }
    }

    /// `members` as one filter: the member itself when there is one, a set
    /// when there are more ([`AnyOf::new`]'s rules), `None` otherwise.
    pub fn any_of(mut members: Vec<Filter>) -> Option<Filter> {
        if members.len() == 1 {
            return members
                .pop()
                .filter(|only| !matches!(only, Filter::AnyOf(_)));
        }
        AnyOf::new(members).map(Filter::AnyOf)
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
    /// Whether it was typed in quotes: this word, exactly. Every term is
    /// matched exactly either way; what the quotes add is that the search
    /// box never answers one that found nothing with a different word.
    pub quoted: bool,
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
    /// label.
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
}

/// A parsed query: everything the executor and the search bar need, and nothing
/// that depends on the clock, the database or the network.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ParsedQuery {
    pub(crate) input: String,
    pub(crate) tokens: Vec<Token>,
    /// Whether a near word counts too: see [`forgiving`](Self::forgiving).
    pub(crate) forgiving: bool,
}

impl ParsedQuery {
    /// This query, asked the forgiving way: after the words it says, the
    /// words near them -- a plural, an unfinished word, a misspelling --
    /// ranked below every exact match (ADR 0037, as amended).
    ///
    /// For a person searching, never for a rule. A query *string* is always
    /// exact, so a saved search, a virtual folder and a rule mean exactly
    /// what they say however they are run; only the caller that is showing
    /// results to somebody watching marks the query forgiving, and nothing
    /// typed can.
    #[must_use]
    pub fn forgiving(mut self) -> Self {
        self.forgiving = true;
        self
    }

    /// Whether near words count: [`forgiving`](Self::forgiving).
    pub fn is_forgiving(&self) -> bool {
        self.forgiving
    }

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

    /// The free-text terms worth matching: [`text_terms`](Self::text_terms)
    /// without a single unquoted character.
    ///
    /// A single character is too little to search for. It is in most of a
    /// mailbox -- every "Hannah's" says "s" -- so it narrows nothing, and the
    /// bar asks for it on the first keystroke of every word: matching it
    /// over a store of 80,000 messages ran for three minutes and held every
    /// other search behind it. Quoted, it was asked for; a digit is a word;
    /// a negation excludes and costs nothing to keep.
    pub fn searchable_terms(&self) -> impl Iterator<Item = &TextTerm> {
        self.text_terms().filter(|term| {
            term.quoted
                || term.negated
                || term.value.chars().count() >= 2
                || term.value.chars().all(|c| c.is_ascii_digit())
        })
    }

    /// Whether there is anything to search by: a word worth matching or a
    /// filter. A query of single letters alone is not yet a search.
    pub fn is_searchable(&self) -> bool {
        self.searchable_terms().next().is_some() || self.filters().next().is_some()
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
        for term in self.searchable_terms() {
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

/// The canonical text of a clause: the one form term edits write (D13).
///
/// A chip's ✕, a filter button and a relaxation all rewrite the query
/// string, and each would otherwise write whatever spelling came to hand --
/// `has:attach` here, `has:attachments` there. This is the one spelling:
/// `has:attachment`, `is:unread`, `has:action`, ISO dates, sizes in the
/// largest binary unit that divides them exactly, and a value quoted when it
/// holds whitespace (`label:"Q3 close"`). The parser still reads every form
/// it reads today; [`crate::parse`] of the result is `clause` again.
///
/// A value containing a `"` cannot be written in this language at all (a
/// quote only ever groups), so no spelling of it round-trips.
pub fn spell(clause: &Clause) -> String {
    let body = spell_filter(&clause.filter);
    if clause.negated {
        format!("-{body}")
    } else {
        body
    }
}

/// [`spell`] without the negation. A set is its field's keyword and its
/// members' values in braces (D26).
fn spell_filter(filter: &Filter) -> String {
    match filter {
        Filter::AnyOf(set) => {
            let values: Vec<String> = set
                .members()
                .iter()
                .map(|member| {
                    let spelled = spell_filter(member);
                    let (_, value) = spelled.split_once(':').unwrap_or(("", &spelled));
                    value.to_owned()
                })
                .collect();
            format!("{}:{{{}}}", set.field().keyword(), values.join(" "))
        }
        _ => spell_one(filter),
    }
}

/// One value's filter, spelled.
fn spell_one(filter: &Filter) -> String {
    match filter {
        Filter::From(value)
        | Filter::To(value)
        | Filter::Subject(value)
        | Filter::In(value)
        | Filter::Filename(value)
        | Filter::List(value)
        | Filter::Account(value)
        | Filter::Group(value)
        | Filter::Label(value) => {
            format!("{}:{}", filter.field().keyword(), quoted(value))
        }
        Filter::Header { name, value: None } => format!("header:{name}"),
        Filter::Header {
            name,
            value: Some(value),
        } => format!("header:{name}={}", quoted(value)),
        Filter::HasAttachment => "has:attachment".to_owned(),
        Filter::HasAction => "has:action".to_owned(),
        Filter::Is(state) => format!(
            "is:{}",
            match state {
                State::Unread => "unread",
                State::Read => "read",
                State::Flagged => "flagged",
                State::Bulk => "bulk",
                State::Automated => "automated",
            }
        ),
        Filter::After(date) => format!("after:{}", date.format("%Y-%m-%d")),
        Filter::Before(date) => format!("before:{}", date.format("%Y-%m-%d")),
        Filter::Larger(bytes) => format!("larger:{}", spell_size(*bytes)),
        Filter::Smaller(bytes) => format!("smaller:{}", spell_size(*bytes)),
        Filter::AnyOf(_) => spell_filter(filter),
    }
}

/// A value as typed: bare when it is one word, in quotes when it is not --
/// or when a brace in it would read as a set's (D26).
fn quoted(value: &str) -> String {
    if value.is_empty()
        || value
            .chars()
            .any(|c| c.is_whitespace() || c == '{' || c == '}')
    {
        format!("\"{value}\"")
    } else {
        value.to_owned()
    }
}

/// `1048576` as `1M`: the largest binary unit that divides it exactly, so
/// the spelling reads back to the same number of bytes.
fn spell_size(bytes: u64) -> String {
    const UNITS: [(u64, &str); 3] = [(1 << 30, "G"), (1 << 20, "M"), (1 << 10, "K")];
    UNITS
        .iter()
        .find(|(unit, _)| bytes != 0 && bytes.is_multiple_of(*unit))
        .map(|(unit, suffix)| format!("{}{suffix}", bytes / unit))
        .unwrap_or_else(|| bytes.to_string())
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
    fn a_one_character_word_is_not_searched_for() {
        // Typing "southwest" asks for "s" first; and "Hannah's" says "s".
        // A single letter is in most of a mailbox and says nothing, and
        // matching it ran for minutes over a large store.
        let today = chrono::NaiveDate::from_ymd_opt(2026, 10, 8).unwrap();
        let fts = |text: &str| crate::parse(text, today).fts_match();
        assert_eq!(fts("s"), None, "nothing to match yet");
        assert_eq!(
            fts("hannah s invoice"),
            Some(r#""hannah" AND "invoice""#.to_owned())
        );
        assert_eq!(
            fts("\"s\""),
            Some(r#""s""#.to_owned()),
            "asked for in quotes, it is"
        );
        assert_eq!(
            fts("c 104"),
            Some(r#""104""#.to_owned()),
            "a number is a word"
        );
    }

    #[test]
    fn forgiving_is_how_a_query_is_asked_not_what_it_says() {
        let today = chrono::NaiveDate::from_ymd_opt(2026, 10, 8).unwrap();
        let typed = crate::parse("tickt", today);
        assert!(
            !typed.is_forgiving(),
            "a parsed string is exact: a rule's is"
        );
        let asked = typed.clone().forgiving();
        assert!(asked.is_forgiving());
        assert_eq!(
            asked.fts_match(),
            typed.fts_match(),
            "the words are the same words; only the asking differs"
        );
    }

    #[test]
    fn a_letter_alone_is_not_a_search_but_a_filter_is() {
        let today = chrono::NaiveDate::from_ymd_opt(2026, 10, 8).unwrap();
        assert!(!crate::parse("s", today).is_searchable());
        assert!(crate::parse("so", today).is_searchable());
        assert!(crate::parse("s is:unread", today).is_searchable());
    }

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

    /// Every `Filter` variant names the `Field` it actually came from --
    /// checked one at a time rather than trusting the `match`'s exhaustiveness
    /// to mean the mapping is right. The compiler only proves every arm is
    /// present, not that `Filter::List` does not answer `Field::From` by a
    /// copy-paste from the arm above it.
    #[test]
    fn every_filter_names_its_own_field() {
        let cases = [
            (Filter::From("a".into()), Field::From),
            (Filter::To("a".into()), Field::To),
            (Filter::Subject("a".into()), Field::Subject),
            (Filter::In("a".into()), Field::In),
            (Filter::Filename("a".into()), Field::Filename),
            (Filter::List("a".into()), Field::List),
            (Filter::Account("a".into()), Field::Account),
            (Filter::Group("a".into()), Field::Group),
            (Filter::HasAttachment, Field::Has),
            (Filter::Label("a".into()), Field::Label),
            (Filter::HasAction, Field::Has),
            (Filter::Is(State::Unread), Field::Is),
            (
                Filter::After(NaiveDate::from_ymd_opt(2026, 1, 1).unwrap()),
                Field::After,
            ),
            (
                Filter::Before(NaiveDate::from_ymd_opt(2026, 1, 1).unwrap()),
                Field::Before,
            ),
            (Filter::Larger(1024), Field::Larger),
            (Filter::Smaller(1024), Field::Smaller),
        ];
        for (filter, field) in cases {
            assert_eq!(filter.field(), field, "{filter:?} should name {field:?}");
        }
    }

    fn token(kind: TokenKind) -> Token {
        Token {
            span: Span::new(0, 0),
            raw: String::new(),
            kind,
        }
    }

    #[test]
    fn negated_reads_the_matching_variant_not_a_fixed_one() {
        // Each arm reads a different struct's own `negated` field; a token
        // built from the *other* two kinds proves this is not one field
        // three names all happen to resolve to.
        let filter_token = token(TokenKind::Filter(Clause {
            negated: true,
            filter: Filter::HasAttachment,
        }));
        let partial_token = token(TokenKind::Partial(Partial {
            negated: false,
            field: Field::Is,
            value: String::new(),
        }));
        let text_token = token(TokenKind::Text(TextTerm {
            negated: true,
            value: "docker".into(),
            quoted: false,
        }));

        assert!(filter_token.negated());
        assert!(!partial_token.negated());
        assert!(text_token.negated());
    }

    fn every_kind_of_filter() -> Vec<Filter> {
        let day = |y, m, d| NaiveDate::from_ymd_opt(y, m, d).unwrap();
        vec![
            Filter::From("ada".into()),
            Filter::From("Ada Moreno".into()),
            Filter::From("ada@example.com".into()),
            Filter::To("bo".into()),
            Filter::Subject("budget v4".into()),
            Filter::In("Archive".into()),
            Filter::Filename("q3.xlsx".into()),
            Filter::List("dev.example.com".into()),
            Filter::Account("work".into()),
            Filter::Group("family".into()),
            Filter::Label("atlas".into()),
            Filter::Label("Q3 close".into()),
            Filter::Header {
                name: "x-mailer".into(),
                value: None,
            },
            Filter::Header {
                name: "x-mailer".into(),
                value: Some("mutt 1.5".into()),
            },
            Filter::Header {
                name: "authentication-results".into(),
                value: Some("spf=pass".into()),
            },
            Filter::HasAttachment,
            Filter::HasAction,
            Filter::Is(State::Unread),
            Filter::Is(State::Read),
            Filter::Is(State::Flagged),
            Filter::Is(State::Bulk),
            Filter::Is(State::Automated),
            Filter::After(day(2026, 7, 1)),
            Filter::Before(day(2025, 12, 31)),
            Filter::Larger(1024 * 1024),
            Filter::Larger(1500),
            Filter::Smaller(3 * 1024),
            Filter::Smaller(2 * 1024 * 1024 * 1024),
        ]
    }

    #[test]
    fn every_clause_spells_to_text_that_parses_back_to_it() {
        let today = NaiveDate::from_ymd_opt(2026, 10, 8).unwrap();
        for filter in every_kind_of_filter() {
            for negated in [false, true] {
                let clause = Clause {
                    negated,
                    filter: filter.clone(),
                };
                let text = spell(&clause);
                let parsed = crate::parse(&text, today);
                assert_eq!(parsed.tokens().len(), 1, "{text:?} is one token");
                assert_eq!(
                    parsed.tokens()[0].kind,
                    TokenKind::Filter(clause.clone()),
                    "{text:?} reads back as {clause:?}"
                );
            }
        }
    }

    #[test]
    fn spell_writes_one_canonical_form_per_filter() {
        let clause = |filter| Clause {
            negated: false,
            filter,
        };
        let day = |y, m, d| NaiveDate::from_ymd_opt(y, m, d).unwrap();
        assert_eq!(spell(&clause(Filter::HasAttachment)), "has:attachment");
        assert_eq!(spell(&clause(Filter::HasAction)), "has:action");
        assert_eq!(spell(&clause(Filter::Is(State::Unread))), "is:unread");
        assert_eq!(spell(&clause(Filter::Is(State::Flagged))), "is:flagged");
        assert_eq!(
            spell(&clause(Filter::Label("Q3 close".into()))),
            r#"label:"Q3 close""#
        );
        assert_eq!(spell(&clause(Filter::Label("atlas".into()))), "label:atlas");
        assert_eq!(
            spell(&clause(Filter::After(day(2026, 7, 1)))),
            "after:2026-07-01"
        );
        assert_eq!(
            spell(&clause(Filter::Before(day(2026, 3, 9)))),
            "before:2026-03-09"
        );
        assert_eq!(spell(&clause(Filter::Larger(1024 * 1024))), "larger:1M");
        assert_eq!(spell(&clause(Filter::Smaller(1500))), "smaller:1500");
        assert_eq!(
            spell(&clause(Filter::Header {
                name: "x-mailer".into(),
                value: Some("mutt 1.5".into()),
            })),
            r#"header:x-mailer="mutt 1.5""#
        );
        assert_eq!(
            spell(&Clause {
                negated: true,
                filter: Filter::From("ada".into()),
            }),
            "-from:ada"
        );
    }

    fn set(members: Vec<Filter>) -> Filter {
        Filter::any_of(members).expect("a set of one field")
    }

    #[test]
    fn a_set_spells_as_braces_and_one_value_as_the_plain_clause() {
        let clause = |filter| Clause {
            negated: false,
            filter,
        };
        assert_eq!(
            spell(&clause(set(vec![
                Filter::From("ada@example.com".into()),
                Filter::From("tomas@example.com".into()),
            ]))),
            "from:{ada@example.com tomas@example.com}"
        );
        assert_eq!(
            spell(&Clause {
                negated: true,
                filter: set(vec![
                    Filter::Label("Q3 close".into()),
                    Filter::Label("atlas".into()),
                ]),
            }),
            r#"-label:{"Q3 close" atlas}"#
        );
        assert_eq!(
            spell(&clause(set(vec![
                Filter::In("a}b".into()),
                Filter::In("{x".into()),
            ]))),
            r#"in:{"a}b" "{x"}"#,
            "a brace in a value is quoted"
        );
        assert_eq!(
            spell(&clause(Filter::Label("{x".into()))),
            r#"label:"{x""#,
            "a plain value that opens with a brace is quoted too"
        );
    }

    #[test]
    fn every_set_spells_to_text_that_parses_back_to_it() {
        let today = NaiveDate::from_ymd_opt(2026, 10, 8).unwrap();
        let sets = [
            set(vec![
                Filter::From("ada".into()),
                Filter::From("Tomás Reyes".into()),
            ]),
            set(vec![
                Filter::To("a@example.com".into()),
                Filter::To("b@example.com".into()),
                Filter::To("c@example.com".into()),
            ]),
            set(vec![
                Filter::Subject("budget v4".into()),
                Filter::Subject("q3".into()),
            ]),
            set(vec![
                Filter::In("Inbox".into()),
                Filter::In("Archive".into()),
            ]),
            set(vec![
                Filter::Label("Q3 close".into()),
                Filter::Label("{odd}".into()),
            ]),
            set(vec![
                Filter::List("a.example.org".into()),
                Filter::List("b".into()),
            ]),
            set(vec![
                Filter::Filename("q3.xlsx".into()),
                Filter::Filename("v4.pdf".into()),
            ]),
            set(vec![
                Filter::Account("work".into()),
                Filter::Account("home".into()),
            ]),
            set(vec![
                Filter::Group("family".into()),
                Filter::Group("team".into()),
            ]),
        ];
        for filter in sets {
            for negated in [false, true] {
                let clause = Clause {
                    negated,
                    filter: filter.clone(),
                };
                let text = spell(&clause);
                let parsed = crate::parse(&text, today);
                assert_eq!(parsed.tokens().len(), 1, "{text:?} is one token");
                assert_eq!(
                    parsed.tokens()[0].kind,
                    TokenKind::Filter(clause.clone()),
                    "{text:?} reads back as {clause:?}"
                );
            }
        }
    }

    #[test]
    fn a_plain_filter_is_its_own_one_alternative() {
        let ada = Filter::From("ada".into());
        assert_eq!(ada.alternatives(), std::slice::from_ref(&ada));
    }

    #[test]
    fn a_parsed_query_hands_back_the_string_it_was_parsed_from() {
        let parsed = ParsedQuery {
            input: "from:ada docker".to_owned(),
            tokens: Vec::new(),
            forgiving: false,
        };
        assert_eq!(parsed.input(), "from:ada docker");
    }
}
