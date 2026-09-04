//! The second evaluator: one message, in memory, no SQL.
//!
//! # Why there are two
//!
//! [`postio_index`] executes a [`ParsedQuery`] by compiling it to SQL and an
//! FTS5 `MATCH`, which is the right shape for the search bar: bulk queries
//! over mail that is already indexed. A **rule** is the other shape. It fires
//! on one message as it arrives, while the sync pass is holding a
//! [`Message`] it has just parsed and deciding what to do with it *before* it
//! is committed anywhere a query could see. There is no row to run SQL
//! against (ADR 0008 Q1).
//!
//! So this crate — pure, `postio-model` and `chrono`, no SQL and no toolkit —
//! holds the matcher, and `postio-index` holds the executor, and both consume
//! [`ParsedQuery::tree`]. One language, one parser, two evaluators.
//!
//! # The agreement is the feature
//!
//! Two evaluators of one language that disagree is the worst outcome
//! available here — worse than not having rules at all — because a dry-run
//! would show one answer and the rule would then do another. Everything below
//! that looks like a fussy detail is a detail the executor also has, written
//! the same way on purpose:
//!
//! * **Tokens, not substrings.** `from:ad` does not find `ada@example.com`,
//!   because FTS5 indexes tokens and the executor asks FTS5. [`tokens`] is
//!   this side of that.
//! * **Phrases are sequences.** `subject:"quarterly report"` is a contiguous
//!   run of tokens, which is what an FTS5 quoted phrase means.
//! * **`header:` values are substrings, and capped.** ADR 0025 Q6 makes the
//!   value a `LIKE '%…%'`, and Q3 caps the stored value — so a matcher
//!   holding the whole value would find messages the index cannot. Both
//!   sides run `postio_model::headers::normalize_value`, which is why the
//!   parser normalizes once rather than each evaluator normalizing its own.
//! * **An unresolvable name matches nothing.** `in:`, `account:` and
//!   `group:` name things this crate cannot resolve, so the caller supplies
//!   the names. Supplying none means the operator selects nothing — never
//!   everything, which is what dropping an unanswerable predicate would
//!   silently mean.
//!
//! `postio-index`'s `differential.rs` is what holds all of that to account:
//! it runs one query list through both evaluators over the whole `.eml`
//! corpus and asserts the two result sets are identical.
//!
//! # What it cannot answer on arrival
//!
//! A message is listed, threaded and header-searchable long before its body
//! is local — that is what `BodyState` is for — so a query touching the body
//! cannot be answered when the message arrives. [`needs_body`] is how the
//! engine knows to wait for the backfill instead of evaluating against an
//! absent body and filing mail on `false` (ADR 0008 Q3).

use chrono::NaiveDate;
use postio_model::Message;

use crate::query::{Clause, Field, Filter, ParsedQuery, QueryTree, State, TextTerm};

/// One message, plus the few facts about it that do not live on it.
///
/// `in:`, `account:` and `group:` name things only the store can resolve —
/// a mailbox's name, path and role; an account's display name and address;
/// which contact groups a message's correspondents belong to. The executor
/// resolves them in SQL. This crate has no store, so the caller passes the
/// names in, and an operator with no names to match against selects nothing.
#[derive(Debug, Clone, Copy)]
pub struct Subject<'a> {
    message: &'a Message,
    body: Option<&'a str>,
    mailbox: &'a [&'a str],
    account: &'a [&'a str],
    groups: &'a [&'a str],
}

impl<'a> Subject<'a> {
    /// A message with none of the facts from outside it.
    ///
    /// Enough for every operator the message itself answers, which is most of
    /// them. `in:`, `account:` and `group:` select nothing until they are
    /// given names, and a body-touching query selects nothing until it is
    /// given a body.
    pub fn new(message: &'a Message) -> Self {
        Self {
            message,
            body: None,
            mailbox: &[],
            account: &[],
            groups: &[],
        }
    }

    /// The message's indexable text, when it is local.
    ///
    /// `None` is "not fetched yet", which is why [`needs_body`] exists: the
    /// engine asks that first rather than letting an absent body answer
    /// `false`.
    pub fn with_body(mut self, body: Option<&'a str>) -> Self {
        self.body = body;
        self
    }

    /// The spellings `in:` may use: the mailbox's name, its path, its role.
    pub fn in_mailbox(mut self, names: &'a [&'a str]) -> Self {
        self.mailbox = names;
        self
    }

    /// The spellings `account:` may use: display name and address.
    pub fn in_account(mut self, names: &'a [&'a str]) -> Self {
        self.account = names;
        self
    }

    /// The contact groups this message's correspondents belong to.
    pub fn with_groups(mut self, groups: &'a [&'a str]) -> Self {
        self.groups = groups;
        self
    }

    /// The message under test.
    pub fn message(&self) -> &'a Message {
        self.message
    }
}

/// Whether `query` selects `subject`.
///
/// An empty query selects everything, which is what `All(vec![])` means and
/// what an empty `WHERE` means. A half-typed operator constrains nothing —
/// [`ParsedQuery::tree`] drops partials — so results narrow as a query is
/// typed rather than flickering to nothing between keystrokes.
pub fn matches(query: &ParsedQuery, subject: &Subject<'_>) -> bool {
    evaluate(&query.tree(), subject)
}

/// Whether answering `query` needs the message's body.
///
/// ADR 0008 Q3's fact classification, computed from the fields a query uses
/// rather than declared by whoever wrote it. `true` means the query cannot be
/// answered when the message arrives and belongs on the backfill completion
/// instead; `false` means it can run in the same transaction as the insert,
/// so the user never sees the mail land in the Inbox first.
///
/// **`header:` is on the `true` side**, however its name reads. ADR 0025 Q4:
/// the header block arrives with the body, so a message whose body is not
/// local has no block to match. Classifying it by its name produces rules
/// that fire on arrival against an empty block and file mail on `false`,
/// which is the failure ADR 0008 Q3 was written to prevent.
///
/// Free text is on the `true` side too: it reaches the body index as well as
/// the metadata one, so a message could match on words that are not local
/// yet.
pub fn needs_body(query: &ParsedQuery) -> bool {
    query.tokens().iter().any(|token| match &token.kind {
        crate::query::TokenKind::Text(_) => true,
        crate::query::TokenKind::Filter(clause) => field_needs_body(clause.filter.field()),
        crate::query::TokenKind::Partial(partial) => field_needs_body(partial.field),
        _ => false,
    })
}

fn field_needs_body(field: Field) -> bool {
    match field {
        Field::Body | Field::Header => true,
        Field::From
        | Field::To
        | Field::Subject
        | Field::Has
        | Field::Is
        | Field::Before
        | Field::After
        | Field::In
        | Field::Filename
        | Field::Larger
        | Field::Smaller
        | Field::List
        | Field::Account
        | Field::Group => false,
    }
}

fn evaluate(tree: &QueryTree, subject: &Subject<'_>) -> bool {
    match tree {
        QueryTree::All(children) => children.iter().all(|child| evaluate(child, subject)),
        QueryTree::Any(children) => children.iter().any(|child| evaluate(child, subject)),
        QueryTree::Filter(clause) => clause.negated ^ filter_matches(clause, subject),
        QueryTree::Text(term) => term.negated ^ text_matches(term, subject),
    }
}

fn filter_matches(clause: &Clause, subject: &Subject<'_>) -> bool {
    let message = subject.message;
    match &clause.filter {
        Filter::From(value) => phrase_in(&sender_text(message), value),
        Filter::To(value) => phrase_in(&recipients_text(message), value),
        Filter::Subject(value) => phrase_in(message.subject.as_deref().unwrap_or_default(), value),
        Filter::Filename(value) => phrase_in(&filenames_text(message), value),
        Filter::List(value) => phrase_in(message.list_id.as_deref().unwrap_or_default(), value),
        Filter::Body(value) => phrase_in(subject.body.unwrap_or_default(), value),
        // Exact, case-insensitive, against every spelling the store offers —
        // the same `lower(x) = lower(?)` disjunction the executor compiles.
        // No names is an empty set, and an empty set matches nothing.
        Filter::In(value) => names_contain(subject.mailbox, value),
        Filter::Account(value) => names_contain(subject.account, value),
        Filter::Group(value) => names_contain(subject.groups, value),
        Filter::Header { name, value } => header_matches(message, name, value.as_deref()),
        Filter::HasAttachment => message.has_attachments(),
        Filter::Is(state) => match state {
            State::Unread => !message.flags.is_seen(),
            State::Read => message.flags.is_seen(),
            State::Flagged => message.flags.is_flagged(),
        },
        // Half-open, and on `received_at` rather than `best_date`: the
        // executor compares the column, and the column is the server's
        // receive time.
        Filter::After(date) => message.received_at >= day_start(*date),
        Filter::Before(date) => message.received_at < day_start(*date),
        Filter::Larger(bytes) => message.size >= *bytes,
        Filter::Smaller(bytes) => message.size <= *bytes,
    }
}

/// Free text reaches both indexes, so a hit in either half is a hit.
fn text_matches(term: &TextTerm, subject: &Subject<'_>) -> bool {
    let message = subject.message;
    phrase_in(&sender_text(message), &term.value)
        || phrase_in(&recipients_text(message), &term.value)
        || phrase_in(message.subject.as_deref().unwrap_or_default(), &term.value)
        || phrase_in(&filenames_text(message), &term.value)
        || phrase_in(message.list_id.as_deref().unwrap_or_default(), &term.value)
        || phrase_in(subject.body.unwrap_or_default(), &term.value)
}

/// `header:name` and `header:name=value`, on the rows the index would hold.
///
/// Normalized on both sides and compared the way ADR 0025 Q6 settles it: the
/// name exactly and case-insensitively — `header:x-mail` must not find
/// `X-Mailer` — and the value as a case-insensitive substring of the
/// **normalized** value, which is capped at
/// `postio_model::headers::VALUE_LIMIT`. Matching the uncapped value here
/// would find messages the index physically cannot.
///
/// Any occurrence matching is a match, which is why `Headers` keeps
/// duplicates and why the index has an `ordinal`.
fn header_matches(message: &Message, name: &str, value: Option<&str>) -> bool {
    message.headers.iter().any(|header| {
        if postio_model::headers::normalize_name(&header.name) != name {
            return false;
        }
        match value {
            None => true,
            Some(value) => {
                let stored = postio_model::headers::normalize_value(&header.value);
                contains_ignoring_ascii_case(&stored, value)
            }
        }
    })
}

/// The text the executor's `sender` column holds: `"name address"` per
/// address, joined by spaces.
fn sender_text(message: &Message) -> String {
    address_text(message.from.iter())
}

/// The `recipients` column: To, Cc and Bcc together, as the executor's
/// trigger aggregates them.
fn recipients_text(message: &Message) -> String {
    address_text(message.to.iter().chain(&message.cc).chain(&message.bcc))
}

fn address_text<'a>(addresses: impl Iterator<Item = &'a postio_model::EmailAddress>) -> String {
    addresses
        .map(|address| {
            format!(
                "{} {}",
                address.name.as_deref().unwrap_or_default(),
                address.address
            )
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn filenames_text(message: &Message) -> String {
    message
        .attachments
        .iter()
        .filter_map(|attachment| attachment.filename.as_deref())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Whether any of `names` equals `value`, ASCII case folded.
fn names_contain(names: &[&str], value: &str) -> bool {
    names.iter().any(|name| name.eq_ignore_ascii_case(value))
}

/// Whether `haystack` contains `needle` as a contiguous run of tokens.
///
/// This is what an FTS5 quoted phrase means, and it is why `from:ad` does not
/// find `ada@example.com`: the executor asks FTS5, which has only tokens to
/// answer with. A needle that tokenizes to nothing — punctuation somebody is
/// mid-typing — matches nothing rather than everything, the same way an empty
/// `MATCH` finds no rows.
fn phrase_in(haystack: &str, needle: &str) -> bool {
    let needle = tokens(needle);
    if needle.is_empty() {
        return false;
    }
    let haystack = tokens(haystack);
    haystack
        .windows(needle.len())
        .any(|window| window == needle.as_slice())
}

/// The tokens FTS5 would index, as closely as a pure crate can say.
///
/// `unicode61 remove_diacritics 2` splits on everything that is not a letter
/// or a digit and folds case; the diacritic half is [`fold`]'s.
fn tokens(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    for character in text.chars() {
        if character.is_alphanumeric() {
            current.extend(fold(character));
        } else if !current.is_empty() {
            tokens.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

/// One character, lowercased and stripped of its diacritic if it has one.
///
/// `remove_diacritics 2` is why `café` and `cafe` are one token to the
/// executor, and a matcher that skipped it would disagree with the index on
/// every accented name in a mailbox. The mapping covers Latin-1 and Latin
/// Extended-A, which is what mail addresses and display names are written in;
/// anything else is passed through lowercased, and the differential test over
/// the corpus is what says whether that is enough.
fn fold(character: char) -> impl Iterator<Item = char> {
    const FOLDED: &str = "aaaaaaaceeeeiiiidnoooooxouuuuypsaaaaaaaceeeeiiiidnooooo/ouuuuypy";
    let folded = match character as u32 {
        // Latin-1 Supplement, À (0xC0) through ÿ (0xFF), minus the two
        // multiplication/division signs the table above keeps in place.
        code @ 0xC0..=0xFF => FOLDED.chars().nth((code - 0xC0) as usize),
        _ => None,
    };
    match folded {
        Some(folded) if folded.is_alphanumeric() => {
            Box::new(std::iter::once(folded)) as Box<dyn Iterator<Item = char>>
        }
        _ => Box::new(character.to_lowercase()),
    }
}

/// Whether `haystack` contains `needle`, folding ASCII case.
///
/// `LIKE '%' || ? || '%'` in the executor, which folds ASCII case and nothing
/// else — SQLite's own `lower()` is ASCII-only too, so there is nothing to
/// gain here by folding more, and something to lose: the two would disagree.
fn contains_ignoring_ascii_case(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    let haystack = haystack.to_ascii_lowercase();
    let needle = needle.to_ascii_lowercase();
    haystack.contains(&needle)
}

fn day_start(date: NaiveDate) -> chrono::DateTime<chrono::Utc> {
    date.and_hms_opt(0, 0, 0)
        .expect("midnight always exists")
        .and_utc()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_phrase_is_a_contiguous_run_of_tokens() {
        assert!(phrase_in(
            "the quarterly report is late",
            "quarterly report"
        ));
        assert!(!phrase_in("the report is quarterly", "quarterly report"));
        assert!(phrase_in("ada@example.com", "ada"));
        assert!(!phrase_in("ada@example.com", "ad"));
    }

    #[test]
    fn punctuation_alone_matches_nothing_rather_than_everything() {
        // An empty FTS5 `MATCH` finds no rows; a matcher that treated an
        // empty needle as "always true" would fire a rule on every message.
        assert!(!phrase_in("anything at all", "..."));
        assert!(!phrase_in("anything at all", ""));
    }

    #[test]
    fn a_diacritic_folds_the_way_the_tokenizer_folds_it() {
        // `remove_diacritics 2`. Without this the index finds `café` for
        // `cafe` and the matcher does not, on every accented name in a
        // mailbox.
        assert_eq!(tokens("Café"), vec!["cafe".to_string()]);
        assert_eq!(
            tokens("Ådne Ünal"),
            vec!["adne".to_string(), "unal".to_string()]
        );
        assert!(phrase_in("réunion at noon", "reunion"));
        assert!(phrase_in("reunion at noon", "réunion"));
    }

    #[test]
    fn a_header_value_is_a_substring_and_a_name_is_not() {
        let mut message = Message::new(
            postio_model::AccountId::new(1),
            postio_model::MailboxId::new(1),
            chrono::Utc::now(),
        );
        message.headers = [("X-Mailer", "Mutt 1.5.24")].into_iter().collect();
        assert!(header_matches(&message, "x-mailer", Some("1.5")));
        assert!(header_matches(&message, "x-mailer", None));
        assert!(!header_matches(&message, "x-mail", None));
    }
}
