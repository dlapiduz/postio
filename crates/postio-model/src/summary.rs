//! A digest's summary: the one place Focus shows text a model wrote. Statements are plain
//! `String`s, so no markup, link or image can be live (ADR 0009 Q4); each carries a message
//! reference with a verbatim excerpt, found by byte search when written and when shown, and an
//! unresolved statement is not kept. A reference's number is its message's place among the
//! summary's source messages.

use serde::{Deserialize, Serialize};

use crate::ids::MessageId;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DigestSummary {
    pub statements: Vec<SummaryStatement>,
    pub messages: u32,
    pub senders: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SummaryStatement {
    pub topic: String,
    pub text: String,
    pub reference: SummaryReference,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SummaryReference {
    pub number: u32,
    pub message: MessageId,
    pub excerpt: String,
}

impl DigestSummary {
    /// A summary with no statement left is no summary, and the digest opens on its plain list.
    pub fn is_empty(&self) -> bool {
        self.statements.is_empty()
    }

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

    /// The digest row's first line: "Summary of 14 messages from 6 senders: rail funding vote, …".
    /// `None` when there is nothing to summarise.
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
