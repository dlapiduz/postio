use chrono::NaiveDate;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

impl Span {
    pub fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }

    pub fn len(&self) -> usize {
        self.end.saturating_sub(self.start)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Counts both edges so a caret resting against either end of a chip still selects it.
    pub fn contains(&self, offset: usize) -> bool {
        offset >= self.start && offset <= self.end
    }
}

/// Spellings come from the design canvas (`docs/PRODUCT.md` §7): `has:attach` and `is:flagged`,
/// with `has:attachment` and `is:starred` accepted as aliases.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Field {
    From,
    To,
    Subject,
    Has,
    Is,
    Before,
    After,
    In,
    Filename,
    Larger,
    Smaller,
    List,
    /// Orthogonal to the role scope (#186): "this account's inbox" and "every account's inbox" are
    /// both things to ask for.
    Account,
    With,
    /// A group answers which people, not which messages, so it composes with the rest;
    /// `postio-index` resolves it to the member address set.
    Group,
    /// The late-answering operator for everything the envelope does not carry (ADR 0025); a header
    /// that must match before the body arrives is promoted to its own operator (`list:`).
    Header,
    /// A name with spaces is quoted, as a `subject:` phrase is.
    Label,
}

impl Field {
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
        Field::With,
        Field::Group,
        Field::Label,
        // Last, because the popup is ordered by how often an operator is
        // reached for and this is the one you type when none of the others
        // will do.
        Field::Header,
    ];

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
            Field::With => "with",
            Field::Group => "group",
            Field::Header => "header",
            Field::Label => "label",
        }
    }

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
            "with" => Some(Field::With),
            "group" => Some(Field::Group),
            "header" => Some(Field::Header),
            "label" => Some(Field::Label),
            _ => None,
        }
    }

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
                | Field::With
                | Field::Group
                | Field::Header
                | Field::Label
        )
    }
}

impl Field {
    /// Every field whose value is a name. `header:` has its own grammar, and a flag, date or size
    /// asked for "either" is better asked once.
    pub fn takes_set(&self) -> bool {
        self.takes_free_text() && *self != Field::Header
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum State {
    Unread,
    Read,
    Flagged,
    Bulk,
    Automated,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Filter {
    From(String),
    To(String),
    Subject(String),
    In(String),
    Filename(String),
    List(String),
    /// Deliberately still text: resolving needs the store, and a saved search is the string the
    /// user typed, which must keep meaning the same after an account is removed and re-added.
    Account(String),
    /// Addresses, not a person: a search pinned from it names the addresses as they were and does
    /// not follow a later join.
    With(Vec<String>),
    /// Stays text for the same reason `Account` does; `postio-index` resolves it.
    Group(String),
    /// `name` is lowercased and `value` normalized by `postio_model::headers`, the form the column
    /// holds, so the in-memory matcher and the index agree.
    /// `value: None` is presence, deliberately not `Some("")`: they select the same rows today but
    /// would stop when either side grew a notion of an empty value.
    Header {
        name: String,
        value: Option<String>,
    },
    HasAttachment,
    /// Text for the reason `Account` is: a label's id changes when it is re-created.
    Label(String),
    HasAction,
    Is(State),
    After(NaiveDate),
    Before(NaiveDate),
    Larger(u64),
    Smaller(u64),
    /// Holds when any member does; negated, when none does.
    AnyOf(AnyOf),
}

/// Members are private and [`AnyOf::new`] is the only way in, so every reader can rely on one field
/// and no nesting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnyOf {
    members: Vec<Filter>,
}

impl AnyOf {
    pub fn new(members: Vec<Filter>) -> Option<AnyOf> {
        let field = members.first()?.field();
        let fits = members.len() >= 2
            && field.takes_set()
            && members
                .iter()
                .all(|member| !matches!(member, Filter::AnyOf(_)) && member.field() == field);
        fits.then_some(AnyOf { members })
    }

    pub fn members(&self) -> &[Filter] {
        &self.members
    }

    pub fn field(&self) -> Field {
        self.members[0].field()
    }
}

impl Filter {
    pub fn field(&self) -> Field {
        match self {
            Filter::From(_) => Field::From,
            Filter::To(_) => Field::To,
            Filter::Subject(_) => Field::Subject,
            Filter::In(_) => Field::In,
            Filter::Filename(_) => Field::Filename,
            Filter::List(_) => Field::List,
            Filter::Account(_) => Field::Account,
            Filter::With(_) => Field::With,
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

    pub fn alternatives(&self) -> &[Filter] {
        match self {
            Filter::AnyOf(set) => set.members(),
            other => std::slice::from_ref(other),
        }
    }

    pub fn any_of(mut members: Vec<Filter>) -> Option<Filter> {
        if members.len() == 1 {
            return members
                .pop()
                .filter(|only| !matches!(only, Filter::AnyOf(_)));
        }
        AnyOf::new(members).map(Filter::AnyOf)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Clause {
    pub negated: bool,
    pub filter: Filter,
}

/// Keeps as-you-type search from erroring. A partial constrains nothing; it carries enough for a
/// pending chip and completions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Partial {
    pub negated: bool,
    pub field: Field,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextTerm {
    pub negated: bool,
    pub value: String,
    /// Every term is matched exactly either way; quotes add that the search box never answers one
    /// that found nothing with a different word.
    pub quoted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenKind {
    Filter(Clause),
    Partial(Partial),
    Text(TextTerm),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub span: Span,
    pub raw: String,
    pub kind: TokenKind,
}

impl Token {
    pub fn field(&self) -> Option<Field> {
        match &self.kind {
            TokenKind::Filter(clause) => Some(clause.filter.field()),
            TokenKind::Partial(partial) => Some(partial.field),
            TokenKind::Text(_) => None,
        }
    }

    pub fn negated(&self) -> bool {
        match &self.kind {
            TokenKind::Filter(clause) => clause.negated,
            TokenKind::Partial(partial) => partial.negated,
            TokenKind::Text(term) => term.negated,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ParsedQuery {
    pub(crate) input: String,
    pub(crate) tokens: Vec<Token>,
    pub(crate) forgiving: bool,
    pub(crate) contact_aware: bool,
}

impl ParsedQuery {
    /// A word that names a contact finds the mail of every address they own, `from:@Group` every
    /// member's. An option of the caller, never of the query text: a saved search, a rule and a
    /// frontend that has not adopted people mean what they say.
    #[must_use]
    pub fn contact_aware(mut self) -> Self {
        self.contact_aware = true;
        self
    }

    pub fn is_contact_aware(&self) -> bool {
        self.contact_aware
    }

    /// For a person searching, never a rule: a query string is always exact, so a saved search,
    /// folder or rule means what it says, and nothing typed can mark a query forgiving.
    #[must_use]
    pub fn forgiving(mut self) -> Self {
        self.forgiving = true;
        self
    }

    pub fn is_forgiving(&self) -> bool {
        self.forgiving
    }

    pub fn input(&self) -> &str {
        &self.input
    }

    pub fn tokens(&self) -> &[Token] {
        &self.tokens
    }

    pub fn is_empty(&self) -> bool {
        self.tokens.is_empty()
    }

    pub fn filters(&self) -> impl Iterator<Item = &Clause> {
        self.tokens.iter().filter_map(|token| match &token.kind {
            TokenKind::Filter(clause) => Some(clause),
            _ => None,
        })
    }

    pub fn partials(&self) -> impl Iterator<Item = &Partial> {
        self.tokens.iter().filter_map(|token| match &token.kind {
            TokenKind::Partial(partial) => Some(partial),
            _ => None,
        })
    }

    pub fn text_terms(&self) -> impl Iterator<Item = &TextTerm> {
        self.tokens.iter().filter_map(|token| match &token.kind {
            TokenKind::Text(term) => Some(term),
            _ => None,
        })
    }

    /// A single character is too little: it is in most of a mailbox, and matching it over 80,000
    /// messages ran three minutes. Quoted, it was asked for; a digit is a word; a negation costs
    /// nothing.
    pub fn searchable_terms(&self) -> impl Iterator<Item = &TextTerm> {
        self.text_terms().filter(|term| {
            term.quoted
                || term.negated
                || term.value.chars().count() >= 2
                || term.value.chars().all(|c| c.is_ascii_digit())
        })
    }

    pub fn is_searchable(&self) -> bool {
        self.searchable_terms().next().is_some() || self.filters().next().is_some()
    }

    /// Every term is a quoted FTS5 literal, so `AND`, `NEAR` and `*` match literally. A query whose
    /// only free text is
    /// negated yields `None` (a `NOT` group needs a left side); the executor excludes those terms
    /// itself.
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

/// The one spelling term edits write (D13): `has:attachment`, `is:unread`, ISO dates, sizes in the
/// largest binary unit that divides them, a value quoted when it holds whitespace. [`crate::parse`]
/// of the result is `clause` again.
/// A value containing a `"` cannot be written in this language.
pub fn spell(clause: &Clause) -> String {
    let body = spell_filter(&clause.filter);
    if clause.negated {
        format!("-{body}")
    } else {
        body
    }
}

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
        Filter::With(addresses) => format!("with:{}", quoted(&addresses.join(","))),
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

fn spell_size(bytes: u64) -> String {
    const UNITS: [(u64, &str); 3] = [(1 << 30, "G"), (1 << 20, "M"), (1 << 10, "K")];
    UNITS
        .iter()
        .find(|(unit, _)| bytes != 0 && bytes.is_multiple_of(*unit))
        .map(|(unit, suffix)| format!("{}{suffix}", bytes / unit))
        .unwrap_or_else(|| bytes.to_string())
}

/// `pub` because `postio-index`'s executor needs it for the exclusion `MATCH` when
/// [`ParsedQuery::fts_match`] returns `None`.
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
            (Filter::With(vec!["a".into()]), Field::With),
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
            contact_aware: false,
        };
        assert_eq!(parsed.input(), "from:ada docker");
    }
}
