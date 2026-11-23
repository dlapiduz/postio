//! The second evaluator: a query against one message in memory (ADR 0008 Q1), for digest rules
//! asked as the sync pass files mail, before it is anywhere a query could see it.
//! It must agree with `postio-index`'s executor, which `digest_matcher` checks over the `.eml`
//! corpus: [`Document`] is the row the store's triggers write, words not substrings (tantivy's
//! `default` analyzer), `from:v` a phrase in one column.
//! Only `from:`, `to:`, `subject:`, `filename:` and `list:` (negatable, sets allowed) are
//! evaluated; anything else is [`Unsupported`], so a rule asking more is refused where configured.

use postio_model::{EmailAddress, Message};

use crate::ParsedQuery;
use crate::query::{Filter, TokenKind};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Document {
    pub sender: String,
    /// The `Bcc`, `Cc` and `To` addresses, the same way, in the order the
    /// trigger's aggregate reads them: through the index on `(message_id,
    /// kind, position)`, so by kind, alphabetically, then by position.
    pub recipients: String,
    pub subject: String,
    pub filenames: String,
    pub list_id: String,
}

impl Document {
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
}

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unsupported {
    Empty,
    Token(String),
}

#[derive(Debug, Clone)]
pub struct Matcher {
    clauses: Vec<Condition>,
}

#[derive(Debug, Clone)]
struct Condition {
    negated: bool,
    alternatives: Vec<Alternative>,
}

#[derive(Debug, Clone)]
struct Alternative {
    column: Column,
    phrase: Vec<(usize, String)>,
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
                let alternatives = clause
                    .filter
                    .alternatives()
                    .iter()
                    .map(|filter| {
                        let (column, value) = match filter {
                            Filter::From(value) => (Column::Sender, value),
                            Filter::To(value) => (Column::Recipients, value),
                            Filter::Subject(value) => (Column::Subject, value),
                            Filter::Filename(value) => (Column::Filenames, value),
                            Filter::List(value) => (Column::ListId, value),
                            _ => return Err(Unsupported::Token(token.raw.clone())),
                        };
                        Ok(Alternative {
                            column,
                            phrase: indexed(value),
                        })
                    })
                    .collect::<Result<_, _>>()?;
                Ok(Condition {
                    negated: clause.negated,
                    alternatives,
                })
            })
            .collect::<Result<_, _>>()?;
        Ok(Self { clauses })
    }

    pub fn matches(&self, message: &Message) -> bool {
        self.matches_document(&Document::of(message))
    }

    pub fn matches_document(&self, document: &Document) -> bool {
        self.clauses
            .iter()
            .all(|condition| condition.negated ^ condition.holds(document))
    }
}

impl Condition {
    fn holds(&self, document: &Document) -> bool {
        self.alternatives
            .iter()
            .any(|alternative| alternative.holds(document))
    }
}

impl Alternative {
    fn holds(&self, document: &Document) -> bool {
        let column = match self.column {
            Column::Sender => &document.sender,
            Column::Recipients => &document.recipients,
            Column::Subject => &document.subject,
            Column::Filenames => &document.filenames,
            Column::ListId => &document.list_id,
        };
        phrase_in(column, &self.phrase)
    }
}

/// Words of this many bytes or more are not indexed: tantivy's
/// `RemoveLongFilter::limit(40)` in the `default` analyzer.
const LONGEST_WORD: usize = 40;

fn runs(text: &str) -> impl Iterator<Item = &str> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|run| !run.is_empty())
}

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
    fn a_full_address_is_found_in_the_sender_and_nowhere_else() {
        let reply = row(
            "Quinn Abara quinn.abara@example.net",
            "Ada Norwood ada.norwood@example.com",
            "",
        );
        assert!(!matcher("from:ada.norwood@example.com").matches_document(&reply));
        assert!(matcher("to:ada.norwood@example.com").matches_document(&reply));
        assert!(!matcher("to:quinn.abara@example.net").matches_document(&reply));
        assert!(!matcher("from:\"example.net quinn\"").matches_document(&reply));
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
        assert!(!matcher("to:quinn").matches_document(&minutes));
        minutes.subject = "Minutes and invite".to_owned();
        assert!(!matcher("subject:invite.ics").matches_document(&minutes));
        assert!(matcher("subject:minutes").matches_document(&minutes));
    }

    #[test]
    fn a_set_holds_when_any_value_does_and_negated_when_none_does() {
        let ada = row(
            "Ada Norwood ada.norwood@example.com",
            "",
            "harbour.example.org",
        );
        let grace = row("Grace Hopper grace@navy.test", "", "");
        let quinn = row("Quinn Abara quinn.abara@example.net", "", "");
        let either = matcher("from:{ada grace}");
        assert!(either.matches_document(&ada));
        assert!(either.matches_document(&grace));
        assert!(!either.matches_document(&quinn));
        let neither = matcher("-from:{ada grace}");
        assert!(!neither.matches_document(&ada));
        assert!(!neither.matches_document(&grace));
        assert!(neither.matches_document(&quinn));
        assert!(matcher("from:{grace quinn} -list:harbour").matches_document(&quinn));
        assert!(!matcher("from:{ada quinn} list:{weekly nothing}").matches_document(&ada));
        assert!(matcher(r#"from:{"Ada Norwood" zed}"#).matches_document(&ada));
    }

    #[test]
    fn a_set_of_what_the_matcher_refuses_is_refused() {
        for (query, refused) in [
            ("label:{atlas harbour}", "label:{atlas harbour}"),
            ("from:ada in:{inbox archive}", "in:{inbox archive}"),
        ] {
            assert_eq!(
                Matcher::new(&parse(query, today())).err(),
                Some(Unsupported::Token(refused.to_owned())),
                "{query:?}"
            );
        }
    }

    #[test]
    fn a_word_too_long_to_index_is_a_gap_the_phrase_cannot_close() {
        let long = "x".repeat(LONGEST_WORD);
        let sender = format!("Ada {long} norwood@example.com");
        let document = row(&sender, "", "");
        assert!(!matcher("from:\"ada norwood\"").matches_document(&document));
        assert!(matcher("from:norwood@example.com").matches_document(&document));
    }
}
