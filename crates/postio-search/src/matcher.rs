//! The second evaluator: a query, one message, in memory (ADR 0008 Q1).
//!
//! # Why there are two
//!
//! `postio-index` answers a [`ParsedQuery`] by compiling it to SQL over the
//! store's full-text index: the search bar, a saved search, a digest rule's
//! preview. A digest rule (spec FR-127, research R13) is also asked of mail
//! as the sync pass files it, inside the pass's transaction, before the
//! message is anywhere a query could see it -- so it is answered here, from
//! the message itself. One language, one parser, two evaluators.
//!
//! # Agreement is the feature
//!
//! If the two disagreed, a rule's preview would show one answer and the rule
//! would do another. `postio-index`'s `digest_matcher` test runs a list of
//! queries through both over the whole `.eml` corpus and asserts identical
//! answers. Everything below that looks like a fussy detail is the executor's
//! own behaviour, reproduced on purpose:
//!
//! * **The row is the index's row.** [`Document`] is the text the store's
//!   triggers write into `search_documents` for a message -- each address as
//!   its name, a space, and its address -- and the test holds the two equal,
//!   column for column.
//! * **Words, not substrings.** The index splits text into runs of letters
//!   and digits and lowercases them (tantivy's `default` analyzer), so
//!   `from:ad` does not find `ada@example.com`, and `from:francoise` does not
//!   find "Françoise": the metadata columns are not folded for diacritics.
//! * **Two conditions, as the executor asks them.** `from:v` holds when
//!   (1) `v`'s words appear, in order and adjacent, in *any* of the row's five
//!   columns -- the executor's index lookup -- and (2) *any one* of `v`'s
//!   words appears in the sender -- its per-row check, which the engine
//!   answers word by word, not as a phrase. `to:`, `subject:`, `filename:`
//!   and `list:` are the same with their own column in place of the sender
//!   (`fts_column_condition`, all five).
//!
//! That last point has a consequence worth saying plainly: `from:` a full
//! address also selects mail sent **to** that address by anyone whose own
//! address shares one of its words (`example`, `com`). That is what search
//! answers today, so it is what a rule answers too; the day the executor
//! checks the column as a phrase (#1699), this module follows it, and the
//! differential test is what says so.
//!
//! # What it evaluates
//!
//! The part of the language digest rules are written in: `from:` (milestone
//! 1's sender rules), and `list:`, `to:`, `subject:` and `filename:`
//! (milestone 2's list and query rules) -- what is known of a message as it
//! is filed, before its body -- each possibly negated, any number of them
//! together. Anything else is [`Unsupported`], named, so a rule that asks
//! more than this can be refused where it is configured rather than
//! silently holding the wrong mail. Free text is among it: the executor
//! also searches bodies, which filing does not have.

use postio_model::{EmailAddress, Message};

use crate::ParsedQuery;
use crate::query::{Filter, TokenKind};

/// A message's row in the executor's metadata index, built in memory: the
/// five columns of `search_documents`, as `postio-index`'s triggers write
/// them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Document {
    /// The `From` addresses, each as its name, a space, and its address.
    pub sender: String,
    /// The `Bcc`, `Cc` and `To` addresses, the same way, in the order the
    /// trigger's aggregate reads them: through the index on `(message_id,
    /// kind, position)`, so by kind, alphabetically, then by position.
    pub recipients: String,
    /// The subject, or nothing.
    pub subject: String,
    /// Attachment filenames, space-separated.
    pub filenames: String,
    /// The `List-Id`'s identifier, or nothing.
    pub list_id: String,
}

impl Document {
    /// `message`'s row.
    pub fn of(message: &Message) -> Self {
        Self {
            sender: addresses(&message.from),
            recipients: addresses(message.bcc.iter().chain(&message.cc).chain(&message.to)),
            subject: message.subject.clone().unwrap_or_default(),
            filenames: message
                .attachments
                .iter()
                .filter_map(|attachment| attachment.filename.as_deref())
                .collect::<Vec<_>>()
                .join(" "),
            list_id: message.list_id.clone().unwrap_or_default(),
        }
    }

    fn columns(&self) -> [&str; 5] {
        [
            &self.sender,
            &self.recipients,
            &self.subject,
            &self.filenames,
            &self.list_id,
        ]
    }
}

/// `coalesce(name, '') || ' ' || address`, joined by spaces.
fn addresses<'a>(addresses: impl IntoIterator<Item = &'a EmailAddress>) -> String {
    addresses
        .into_iter()
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

/// Why a query cannot be matched in memory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unsupported {
    /// The query constrains nothing: a rule over it would hold every message.
    Empty,
    /// A token outside `from:`, `to:`, `subject:`, `filename:` and `list:`
    /// -- free text, another operator, or one still being typed -- as it was
    /// written.
    Token(String),
}

/// A query, ready to be asked of one message after another.
#[derive(Debug, Clone)]
pub struct Matcher {
    clauses: Vec<Condition>,
}

/// One column condition, with its words worked out once.
#[derive(Debug, Clone)]
struct Condition {
    negated: bool,
    /// Which column the per-row check reads.
    column: Column,
    /// The value's words as the index holds them, with their positions.
    phrase: Vec<(usize, String)>,
    /// The value's words as the per-row check compares them.
    words: Vec<String>,
}

#[derive(Debug, Clone, Copy)]
enum Column {
    Sender,
    Recipients,
    Subject,
    Filenames,
    ListId,
}

impl Matcher {
    /// `query`, if every token in it is a `from:`, `to:`, `subject:`,
    /// `filename:` or `list:`.
    pub fn new(query: &ParsedQuery) -> Result<Self, Unsupported> {
        if query.is_empty() {
            return Err(Unsupported::Empty);
        }
        let clauses = query
            .tokens()
            .iter()
            .map(|token| {
                let TokenKind::Filter(clause) = &token.kind else {
                    return Err(Unsupported::Token(token.raw.clone()));
                };
                let (column, value) = match &clause.filter {
                    Filter::From(value) => (Column::Sender, value),
                    Filter::To(value) => (Column::Recipients, value),
                    Filter::Subject(value) => (Column::Subject, value),
                    Filter::Filename(value) => (Column::Filenames, value),
                    Filter::List(value) => (Column::ListId, value),
                    _ => return Err(Unsupported::Token(token.raw.clone())),
                };
                Ok(Condition {
                    negated: clause.negated,
                    column,
                    phrase: indexed(value),
                    words: scalar(value).collect(),
                })
            })
            .collect::<Result<_, _>>()?;
        Ok(Self { clauses })
    }

    /// Whether the query selects `message`.
    pub fn matches(&self, message: &Message) -> bool {
        self.matches_document(&Document::of(message))
    }

    /// Whether the query selects the message `document` is the row of.
    pub fn matches_document(&self, document: &Document) -> bool {
        self.clauses
            .iter()
            .all(|condition| condition.negated ^ condition.holds(document))
    }
}

impl Condition {
    fn holds(&self, document: &Document) -> bool {
        let column = match self.column {
            Column::Sender => &document.sender,
            Column::Recipients => &document.recipients,
            Column::Subject => &document.subject,
            Column::Filenames => &document.filenames,
            Column::ListId => &document.list_id,
        };
        let in_the_row = document
            .columns()
            .iter()
            .any(|text| phrase_in(text, &self.phrase));
        let in_the_column = scalar(column).any(|word| self.words.contains(&word));
        in_the_row && in_the_column
    }
}

/// Words of this many bytes or more are not indexed: tantivy's
/// `RemoveLongFilter::limit(40)` in the `default` analyzer.
const LONGEST_WORD: usize = 40;

/// Maximal runs of letters and digits: tantivy's `SimpleTokenizer`.
fn runs(text: &str) -> impl Iterator<Item = &str> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|run| !run.is_empty())
}

/// tantivy's `LowerCaser`: ASCII in place, anything else char by char.
fn lowercase(word: &str) -> String {
    if word.is_ascii() {
        word.to_ascii_lowercase()
    } else {
        word.chars().flat_map(char::to_lowercase).collect()
    }
}

/// `text`'s words as the index holds them: lowercased, a long word dropped
/// but its position still counted, as the index counts it.
fn indexed(text: &str) -> Vec<(usize, String)> {
    runs(text)
        .enumerate()
        .filter(|(_, run)| run.len() < LONGEST_WORD)
        .map(|(position, run)| (position, lowercase(run)))
        .collect()
}

/// `text`'s words as the engine's scalar `fts_match` compares them: the same
/// runs, lowercased, with no limit on length.
fn scalar(text: &str) -> impl Iterator<Item = String> + '_ {
    runs(text).map(lowercase)
}

/// Whether `phrase` occurs in `text` as the index finds a phrase: each word
/// at the same distance from the first as in the phrase. A phrase with no
/// words finds nothing, as an empty index query does.
fn phrase_in(text: &str, phrase: &[(usize, String)]) -> bool {
    let Some(((first_at, first), rest)) = phrase.split_first() else {
        return false;
    };
    let words = indexed(text);
    let word_at = |position: usize| {
        words
            .binary_search_by_key(&position, |(at, _)| *at)
            .ok()
            .map(|index| words[index].1.as_str())
    };
    words.iter().any(|(start, word)| {
        word == first
            && rest
                .iter()
                .all(|(at, want)| word_at(start + (at - first_at)) == Some(want.as_str()))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    fn today() -> chrono::NaiveDate {
        chrono::NaiveDate::from_ymd_opt(2026, 9, 26).expect("a real date")
    }

    fn matcher(query: &str) -> Matcher {
        Matcher::new(&parse(query, today())).expect("a digest rule's query")
    }

    fn row(sender: &str, recipients: &str, list_id: &str) -> Document {
        Document {
            sender: sender.to_owned(),
            recipients: recipients.to_owned(),
            subject: "Minutes".to_owned(),
            filenames: String::new(),
            list_id: list_id.to_owned(),
        }
    }

    #[test]
    fn only_what_is_known_as_mail_is_filed_is_evaluated() {
        for query in [
            "from:ada",
            "list:harbour",
            "-from:ada list:harbour",
            "to:ada",
            "subject:minutes",
            "filename:invite.ics",
            "list:harbour -subject:minutes",
        ] {
            assert!(Matcher::new(&parse(query, today())).is_ok(), "{query:?}");
        }
        assert_eq!(
            Matcher::new(&parse("", today())).err(),
            Some(Unsupported::Empty)
        );
        for (query, refused) in [
            ("invoice", "invoice"),
            ("from:ada is:unread", "is:unread"),
            ("from:", "from:"),
            ("list:harbour has:attachment", "has:attachment"),
            ("in:archive", "in:archive"),
        ] {
            assert_eq!(
                Matcher::new(&parse(query, today())).err(),
                Some(Unsupported::Token(refused.to_owned())),
                "{query:?}"
            );
        }
    }

    #[test]
    fn a_sender_is_matched_by_whole_words_in_order() {
        let ada = row("Ada Norwood ada.norwood@example.com", "", "");
        assert!(matcher("from:ada.norwood@example.com").matches_document(&ada));
        assert!(matcher("from:ADA").matches_document(&ada));
        assert!(matcher("from:\"Ada Norwood\"").matches_document(&ada));
        assert!(!matcher("from:ad").matches_document(&ada));
        assert!(!matcher("from:\"com example\"").matches_document(&ada));
        assert!(!matcher("from:grace@example.com").matches_document(&ada));
        // The name and the address are one run of text in the row, so a
        // phrase may span them, as it does in the index.
        assert!(matcher("from:\"Norwood Ada\"").matches_document(&ada));
    }

    #[test]
    fn diacritics_are_not_folded_in_the_metadata() {
        // The body index is folded (`postio_model::fold`); these columns are
        // not, so the executor finds "Françoise" only as typed.
        let francoise = row("Françoise Lemaître f.lemaitre@fr.example", "", "");
        assert!(matcher("from:Françoise").matches_document(&francoise));
        assert!(!matcher("from:francoise").matches_document(&francoise));
    }

    #[test]
    fn a_full_address_also_finds_mail_to_it_from_a_sender_sharing_a_word() {
        // The executor's meaning today, reproduced so a rule agrees with its
        // preview: the address is found in the recipients, and the sender
        // shares "example" with it.
        let reply = row(
            "Quinn Abara quinn.abara@example.net",
            "Ada Norwood ada.norwood@example.com",
            "",
        );
        assert!(matcher("from:ada.norwood@example.com").matches_document(&reply));
        // Sharing no word with it, the sender is not taken for it.
        let stranger = row(
            "Grace Hopper grace@navy.test",
            "Ada Norwood ada.norwood@example.com",
            "",
        );
        assert!(!matcher("from:ada.norwood@example.com").matches_document(&stranger));
    }

    #[test]
    fn a_list_is_matched_by_its_identifier() {
        let list = row(
            "Ada Norwood ada.norwood@example.com",
            "harbour-dev@lists.example.org",
            "harbour-dev.lists.example.org",
        );
        assert!(matcher("list:harbour-dev.lists.example.org").matches_document(&list));
        assert!(matcher("list:harbour").matches_document(&list));
        assert!(!matcher("list:weekly.news.example.org").matches_document(&list));
        assert!(!matcher("-list:harbour").matches_document(&list));
        assert!(matcher("from:ada list:harbour").matches_document(&list));
        assert!(!matcher("from:grace list:harbour").matches_document(&list));
    }

    #[test]
    fn a_recipient_a_subject_and_a_filename_are_matched_as_their_columns() {
        // US14: a query rule. `to:`, `subject:` and `filename:` are the
        // executor's same two conditions as `from:`, on their own columns.
        let mut minutes = row(
            "Quinn Abara quinn.abara@example.net",
            "Ada Norwood ada.norwood@example.com",
            "",
        );
        minutes.filenames = "notes.pdf invite.ics".to_owned();
        assert!(matcher("to:ada.norwood@example.com").matches_document(&minutes));
        assert!(!matcher("to:ren.ishida@example.net").matches_document(&minutes));
        assert!(matcher("subject:minutes").matches_document(&minutes));
        assert!(!matcher("subject:agenda").matches_document(&minutes));
        assert!(matcher("filename:invite.ics").matches_document(&minutes));
        assert!(!matcher("filename:drawing.svg").matches_document(&minutes));
        assert!(matcher("to:ada subject:minutes").matches_document(&minutes));
        assert!(!matcher("to:ada -subject:minutes").matches_document(&minutes));
        // As with `from:`, the phrase may be anywhere in the row, and one of
        // its words must be in the column: "quinn" is in the row, not in
        // the recipients.
        assert!(!matcher("to:quinn").matches_document(&minutes));
    }

    #[test]
    fn a_word_too_long_to_index_is_a_gap_the_phrase_cannot_close() {
        let long = "x".repeat(LONGEST_WORD);
        let sender = format!("Ada {long} norwood@example.com");
        let document = row(&sender, "", "");
        // The index holds "ada", nothing at the next position, "norwood".
        assert!(!matcher("from:\"ada norwood\"").matches_document(&document));
        assert!(matcher("from:norwood@example.com").matches_document(&document));
    }
}
