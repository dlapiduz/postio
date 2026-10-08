//! The digest summariser (spec 007 FR-172 to FR-175, US13, research R16):
//! statements with references, each reference resolved by excerpt.
//!
//! The model is shown the digest's messages, numbered, inside the fence, and
//! answers in a fixed schema: rows of a topic, a statement, the number of the
//! message it came from, and a passage of that message copied as written.
//!
//! **A statement is kept only if its reference resolves**: its number names
//! one of the messages, and its passage is in that message's text byte for
//! byte. Anything else is dropped (FR-173). What is kept is plain text --
//! whitespace made single spaces, control characters taken out, and nothing
//! read as markup: a `<a href=…>` the model wrote stays those characters
//! (FR-174). The summariser gets no tools and has no send path.

use std::sync::Arc;

use postio_model::summary::{DigestSummary, SummaryReference, SummaryStatement};
use postio_model::{AccountId, MessageId};
use serde_json::{Map, Value};

use crate::client::{AiError, Client, Question};
use crate::schema::{Field, Kind, Schema};

/// The most messages one summary is written from; the rest are in the
/// plain list.
pub const MAX_SOURCES: usize = 40;

/// How much of each message the model is shown, in characters: a
/// newsletter's first screens, which is where its news is.
pub const SOURCE_CHARS: usize = 4_000;

/// One message a summary is written from.
#[derive(Debug, Clone, Copy)]
pub struct Source<'a> {
    /// Which message.
    pub message: MessageId,
    /// Its account, for the egress log.
    pub account: AccountId,
    /// Who sent it, as the digest shows them.
    pub sender: &'a str,
    /// Its subject.
    pub subject: &'a str,
    /// Its own text: what a reference's passage must be found in.
    pub text: &'a str,
}

/// The person's model, as the digest summariser.
#[derive(Debug, Clone)]
pub struct Summariser {
    client: Arc<Client>,
}

impl Summariser {
    /// Ask `client`'s model.
    pub fn new(client: Arc<Client>) -> Self {
        Self { client }
    }

    /// A summary of `sources`, oldest first, from `senders` people: every
    /// statement the model wrote whose reference resolves. An error when the
    /// model could not answer; the digest then opens on its plain list.
    pub fn summarise(
        &self,
        sources: &[Source<'_>],
        senders: u32,
    ) -> Result<DigestSummary, AiError> {
        let sources = &sources[..sources.len().min(MAX_SOURCES)];
        let data = sources
            .iter()
            .enumerate()
            .map(|(at, source)| {
                let text: String = source.text.chars().take(SOURCE_CHARS).collect();
                format!(
                    "[{}] From: {} -- Subject: {}\n{text}",
                    at + 1,
                    one_line(source.sender),
                    one_line(source.subject)
                )
            })
            .collect::<Vec<_>>()
            .join("\n\n");
        let answer = self.client.ask(&Question {
            schema: &SCHEMA,
            instructions: ASKED,
            data: &data,
            account: sources.first().map(|source| source.account),
        })?;
        let statements = statements(&answer, sources);
        tracing::debug!(
            sources = sources.len(),
            kept = statements.len(),
            "the model summarised a digest"
        );
        Ok(DigestSummary {
            statements,
            messages: u32::try_from(sources.len()).unwrap_or(u32::MAX),
            senders,
        })
    }
}

/// The answer's shape: rows of a topic, a statement, the message's number
/// and a passage of it.
const SCHEMA: Schema = Schema {
    name: "digest_summary",
    fields: &[Field {
        name: "statements",
        kind: Kind::Rows {
            max: 40,
            fields: &[
                Field {
                    name: "topic",
                    kind: Kind::Text { max: 60 },
                },
                Field {
                    name: "text",
                    kind: Kind::Text { max: 400 },
                },
                Field {
                    name: "source",
                    kind: Kind::Integer {
                        min: 1,
                        max: MAX_SOURCES as i64,
                    },
                },
                Field {
                    name: "excerpt",
                    kind: Kind::Text { max: 300 },
                },
            ],
        },
    }],
};

/// What the model is asked, in Postio's words.
const ASKED: &str = "You summarise a digest of newsletters and updates for one person. \
    The messages are numbered [1], [2] and so on. Write short, factual statements of what \
    they say, grouped by topic, and nothing they do not say.\n\
    - topic: a few words naming what the statement is about; statements on one topic share it.\n\
    - text: one plain sentence, with no links, markup or formatting.\n\
    - source: the number of the message the statement comes from.\n\
    - excerpt: a short passage from that message that supports the statement, copied \
    exactly as it is written, character for character.\n\
    Order the statements by topic. Leave out adverts, footers and unsubscribe text.";

/// The rows whose references resolve, as plain text.
fn statements(answer: &Map<String, Value>, sources: &[Source<'_>]) -> Vec<SummaryStatement> {
    let Some(rows) = answer.get("statements").and_then(Value::as_array) else {
        return Vec::new();
    };
    rows.iter()
        .filter_map(|row| {
            let number = u32::try_from(row.get("source")?.as_i64()?).ok()?;
            let source = sources.get(usize::try_from(number).ok()?.checked_sub(1)?)?;
            let excerpt = resolve(row.get("excerpt")?.as_str()?, source.text)?;
            let text = plain(row.get("text")?.as_str()?);
            if text.is_empty() {
                return None;
            }
            Some(SummaryStatement {
                topic: plain(row.get("topic")?.as_str()?),
                text,
                reference: SummaryReference {
                    number,
                    message: source.message,
                    excerpt,
                },
            })
        })
        .collect()
}

/// `excerpt` as it is in `text`, if it is there byte for byte: the passage a
/// reference is pinned to. Surrounding whitespace is let go; nothing else.
pub fn resolve(excerpt: &str, text: &str) -> Option<String> {
    let excerpt = excerpt.trim();
    if excerpt.chars().filter(|c| c.is_alphanumeric()).count() < 3 || !text.contains(excerpt) {
        return None;
    }
    Some(excerpt.to_owned())
}

/// What the model wrote, as one line of plain characters: runs of
/// whitespace are one space, and control characters are gone. Nothing else
/// changes, so markup stays the characters it is.
fn plain(text: &str) -> String {
    text.split_whitespace()
        .map(|word| word.chars().filter(|c| !c.is_control()).collect::<String>())
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// A header's text on one line, for the fence's labels.
fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use postio_config::ModelEndpoint;

    use super::*;
    use crate::fake::{FakeRuntime, Reply};

    const LEDGER: &str = "The council voted 7 to 2 to fund the rail link. \
        Work starts in March, and the station opens in 2028.";
    const WEEKLY: &str = "Issue 112: Sync without servers. \
        Local-first apps keep working offline and merge later.";

    fn sources() -> Vec<Source<'static>> {
        vec![
            Source {
                message: MessageId::new(11),
                account: AccountId::new(1),
                sender: "The Evening Ledger",
                subject: "Tonight's council vote",
                text: LEDGER,
            },
            Source {
                message: MessageId::new(12),
                account: AccountId::new(1),
                sender: "Local-First Weekly",
                subject: "Issue 112",
                text: WEEKLY,
            },
        ]
    }

    fn summariser(runtime: &FakeRuntime) -> Summariser {
        Summariser::new(Arc::new(
            Client::new(
                ModelEndpoint::parse("http://127.0.0.1:11434/v1").expect("loopback"),
                "a-small-model",
            )
            .with_transport(Arc::new(runtime.clone())),
        ))
    }

    fn row(topic: &str, text: &str, source: u32, excerpt: &str) -> Value {
        serde_json::json!({ "topic": topic, "text": text, "source": source, "excerpt": excerpt })
    }

    fn summarised(rows: Vec<Value>) -> Result<DigestSummary, AiError> {
        let runtime = FakeRuntime::always(Reply::content(
            serde_json::json!({ "statements": rows }).to_string(),
        ));
        summariser(&runtime).summarise(&sources(), 2)
    }

    #[test]
    fn every_statement_ends_in_a_reference_to_the_message_it_came_from() {
        // US13 scenario 1, SC-014: each statement cites one of the digest's
        // messages, by its number, at a passage that is in it verbatim.
        let summary = summarised(vec![
            row(
                "Your town",
                "The council funded the rail link.",
                1,
                "voted 7 to 2 to fund the rail link",
            ),
            row(
                "Software",
                "Local-first apps merge later.",
                2,
                "keep working offline and merge later",
            ),
        ])
        .expect("a summary");

        assert_eq!(summary.messages, 2);
        assert_eq!(summary.senders, 2);
        assert_eq!(summary.statements.len(), 2);
        let texts = [LEDGER, WEEKLY];
        for (statement, source) in summary.statements.iter().zip(sources()) {
            let reference = &statement.reference;
            assert_eq!(reference.message, source.message);
            assert!(
                texts[reference.number as usize - 1].contains(&reference.excerpt),
                "{reference:?}"
            );
        }
        assert_eq!(summary.statements[1].reference.number, 2);
        assert_eq!(summary.statements[0].topic, "Your town");
    }

    #[test]
    fn a_statement_whose_reference_does_not_resolve_is_not_kept() {
        // FR-173: a passage that is not in its message, or a number that
        // names no message, and the statement is dropped.
        let summary = summarised(vec![
            row("Your town", "Kept.", 1, "the station opens in 2028"),
            row(
                "Your town",
                "Paraphrased.",
                1,
                "the station will open in 2028",
            ),
            row(
                "Your town",
                "Wrong message.",
                2,
                "the station opens in 2028",
            ),
            row("Software", "No such message.", 3, "Sync without servers"),
            row("Software", "Too little.", 2, "Is"),
        ])
        .expect("a summary");
        let kept: Vec<&str> = summary
            .statements
            .iter()
            .map(|statement| statement.text.as_str())
            .collect();
        assert_eq!(kept, ["Kept."]);
    }

    #[test]
    fn what_the_model_wrote_is_kept_as_plain_characters() {
        // US13 scenario 3, FR-174: markup and a bare URL are characters, and
        // a statement is one line.
        let summary = summarised(vec![row(
            "Your <b>town</b>",
            "Read <a href=\"https://phish.example/\">this</a>\nat https://phish.example/x \u{7}now.",
            1,
            "Work starts in March",
        )])
        .expect("a summary");
        let statement = &summary.statements[0];
        assert_eq!(
            statement.text,
            "Read <a href=\"https://phish.example/\">this</a> at https://phish.example/x now."
        );
        assert_eq!(statement.topic, "Your <b>town</b>");
    }

    #[test]
    fn the_messages_are_fenced_as_data_and_numbered() {
        // US13 scenario 5, ADR 0009 Q4: the digest's mail is data in the
        // fence, one request is all it costs, and it carries no tool.
        let runtime = FakeRuntime::always(Reply::content(r#"{"statements":[]}"#));
        let summary = summariser(&runtime)
            .summarise(&sources(), 2)
            .expect("an answer");
        assert!(summary.is_empty());

        let requests = runtime.requests();
        assert_eq!(requests.len(), 1, "one request for the digest");
        let data = requests[0].data();
        assert!(data.starts_with("<<<DATA-0\n"), "{data}");
        assert!(data.contains("[1] From: The Evening Ledger"), "{data}");
        assert!(data.contains("[2] From: Local-First Weekly"), "{data}");
        assert!(data.contains(LEDGER) && data.contains(WEEKLY));
        assert!(requests[0].body.get("tools").is_none());
        let instructions = requests[0].body["messages"][0]["content"].as_str().unwrap();
        assert!(!instructions.contains("council"), "{instructions}");
    }

    #[test]
    fn a_model_that_is_not_running_writes_no_summary() {
        // US13 scenario 4: the digest opens on its plain list.
        let runtime = FakeRuntime::refusing();
        assert_eq!(
            summariser(&runtime).summarise(&sources(), 2),
            Err(AiError::Down)
        );
    }
}
