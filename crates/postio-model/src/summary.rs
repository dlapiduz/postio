//! A digest's summary: statements, each ending in a reference to the message
//! it came from, pinned to a passage of that message (spec 007 FR-172 to
//! FR-175, research R16).
//!
//! It is the one place Focus shows text a model wrote, so the shape carries
//! the guarantees:
//!
//! - **Plain text.** A statement is a `String` shown as characters. Nothing
//!   here can hold markup, a link or an image, so a model talked into
//!   writing `<a href=…>` produces those characters and nothing live
//!   (FR-174, ADR 0009 Q4).
//! - **Every statement has a reference.** A reference is a message and an
//!   excerpt of that message's own text, verbatim, found by a byte search
//!   when the summary is written and again when it is shown. A statement
//!   whose reference does not resolve is not kept (FR-173).
//! - **Numbered by the digest.** A reference's number is its message's place
//!   among the messages the summary was written from -- the digest's, oldest
//!   first, those whose bodies were here -- so "cited as 6" names the same
//!   email wherever it appears in one summary.

use serde::{Deserialize, Serialize};

use crate::ids::MessageId;

/// A digest's summary, as it is stored with its delivery and shown.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DigestSummary {
    /// The statements, in the order they are read, grouped by topic.
    pub statements: Vec<SummaryStatement>,
    /// How many messages it was written from.
    pub messages: u32,
    /// How many people sent them.
    pub senders: u32,
}

/// One statement of a summary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SummaryStatement {
    /// The topic it is grouped under: "Your town".
    pub topic: String,
    /// What it says, as plain characters.
    pub text: String,
    /// Where it came from.
    pub reference: SummaryReference,
}

/// What a statement cites: a message, and a passage of it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SummaryReference {
    /// Its number: the message's place among those the summary was written
    /// from, oldest first, from 1.
    pub number: u32,
    /// The message.
    pub message: MessageId,
    /// The cited passage, verbatim from the message's own text: what the
    /// email view highlights.
    pub excerpt: String,
}

impl DigestSummary {
    /// Whether it says anything: a summary with no statement left is no
    /// summary, and the digest opens on its plain list (FR-175).
    pub fn is_empty(&self) -> bool {
        self.statements.is_empty()
    }

    /// The topics, each once, in the order they are first read.
    pub fn topics(&self) -> Vec<&str> {
        let mut topics: Vec<&str> = Vec::new();
        for statement in &self.statements {
            let topic = statement.topic.as_str();
            if !topic.is_empty() && !topics.contains(&topic) {
                topics.push(topic);
            }
        }
        topics
    }

    /// The digest row's first line (FR-124): "Summary of 14 messages from
    /// 6 senders: rail funding vote, …" -- the topics, in order, after the
    /// counts. `None` when there is nothing to summarise.
    pub fn line(&self) -> Option<String> {
        if self.is_empty() {
            return None;
        }
        let plural = |count: u32, one: &str, many: &str| {
            if count == 1 {
                format!("1 {one}")
            } else {
                format!("{count} {many}")
            }
        };
        let topics = self
            .topics()
            .iter()
            .map(|topic| topic.to_lowercase())
            .collect::<Vec<_>>()
            .join(", ");
        Some(format!(
            "Summary of {} from {}: {topics}",
            plural(self.messages, "message", "messages"),
            plural(self.senders, "sender", "senders"),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn statement(topic: &str, number: u32) -> SummaryStatement {
        SummaryStatement {
            topic: topic.to_owned(),
            text: format!("Something about {topic}."),
            reference: SummaryReference {
                number,
                message: MessageId::new(i64::from(number)),
                excerpt: "a passage".to_owned(),
            },
        }
    }

    #[test]
    fn the_row_s_line_is_the_counts_and_the_topics_in_order() {
        let summary = DigestSummary {
            statements: vec![
                statement("Rail funding vote", 1),
                statement("Library hours", 2),
                statement("Rail funding vote", 3),
            ],
            messages: 14,
            senders: 6,
        };
        assert_eq!(
            summary.line().as_deref(),
            Some("Summary of 14 messages from 6 senders: rail funding vote, library hours")
        );
        let one = DigestSummary {
            statements: vec![statement("Sync", 1)],
            messages: 1,
            senders: 1,
        };
        assert_eq!(
            one.line().as_deref(),
            Some("Summary of 1 message from 1 sender: sync")
        );
    }

    #[test]
    fn a_summary_with_nothing_left_is_no_summary() {
        let empty = DigestSummary {
            statements: Vec::new(),
            messages: 3,
            senders: 2,
        };
        assert!(empty.is_empty());
        assert_eq!(empty.line(), None);
    }

    #[test]
    fn it_round_trips_as_the_json_the_delivery_keeps() {
        let summary = DigestSummary {
            statements: vec![statement("<a href=\"https://example.com\">x</a>", 2)],
            messages: 2,
            senders: 1,
        };
        let json = serde_json::to_string(&summary).expect("json");
        assert_eq!(
            serde_json::from_str::<DigestSummary>(&json).expect("back"),
            summary
        );
    }
}
