//! "Digest mail like this" (spec 007 FR-171, US14): the person's model
//! checks which mail is alike, and a rule is made from what it picks.
//!
//! The model writes no rule. Postio builds the candidates itself, each a
//! query in the one language that can hold mail as it is filed -- the
//! message's list, its sender, its sender's domain -- and shows the model
//! the example and a sample of the recent mail each candidate would catch.
//! The model answers with one number: the candidate whose mail is like the
//! example, or none. So the rule is Postio's query, previewed through the
//! executor before it is saved like any other, and never text a model
//! wrote.

use std::sync::Arc;

use postio_model::AccountId;
use serde_json::{Map, Value};

use crate::client::{AiError, Client, Question};
use crate::schema::{Field, Kind, Schema};

/// The most candidates one question offers.
pub const MAX_CANDIDATES: usize = 8;

/// The message the person chose.
#[derive(Debug, Clone, Copy)]
pub struct Example<'a> {
    /// Its account, for the egress log.
    pub account: AccountId,
    /// Who sent it.
    pub sender: &'a str,
    /// Its subject.
    pub subject: &'a str,
    /// The start of its own text.
    pub text: &'a str,
}

/// One rule Postio could make, and a sample of what it would catch.
#[derive(Debug, Clone)]
pub struct Candidate<'a> {
    /// The query, in the one language.
    pub query: &'a str,
    /// Recent mail it matches: each one's sender and subject.
    pub sample: Vec<(&'a str, &'a str)>,
}

/// The person's model, as the judge of what is alike.
#[derive(Debug, Clone)]
pub struct LikeThis {
    client: Arc<Client>,
}

impl LikeThis {
    /// Ask `client`'s model.
    pub fn new(client: Arc<Client>) -> Self {
        Self { client }
    }

    /// Which of `candidates` catches mail like `example`: its index, or
    /// `None` when the model says none does.
    pub fn choose(
        &self,
        example: &Example<'_>,
        candidates: &[Candidate<'_>],
    ) -> Result<Option<usize>, AiError> {
        let candidates = &candidates[..candidates.len().min(MAX_CANDIDATES)];
        let mut data = format!(
            "The example:\nFrom: {}\nSubject: {}\n{}\n\nThe rules, each with recent mail it \
             would catch:",
            example.sender,
            example.subject,
            example.text.chars().take(1_500).collect::<String>()
        );
        for (at, candidate) in candidates.iter().enumerate() {
            data.push_str(&format!("\n\n[{}] {}", at + 1, candidate.query));
            for (sender, subject) in &candidate.sample {
                data.push_str(&format!("\n- {sender}: {subject}"));
            }
        }
        let answer = self.client.ask(&Question {
            schema: &SCHEMA,
            instructions: ASKED,
            data: &data,
            account: Some(example.account),
        })?;
        pick(&answer, candidates.len())
    }
}

/// The answer's shape: one number.
const SCHEMA: Schema = Schema {
    name: "like_this",
    fields: &[Field {
        name: "candidate",
        kind: Kind::Integer {
            min: 0,
            max: MAX_CANDIDATES as i64,
        },
    }],
};

/// What the model is asked, in Postio's words.
const ASKED: &str = "You help one person gather mail into a digest. You are given an \
    example email and numbered rules, each with a sample of the recent mail it would catch. \
    Choose the rule whose mail is most like the example -- the same kind of mail, such as \
    the same newsletter or the same sort of notice -- and that catches little else. Answer \
    with its number as candidate, or 0 if no rule's mail is like the example.";

/// The candidate `answer` names, by index.
fn pick(answer: &Map<String, Value>, offered: usize) -> Result<Option<usize>, AiError> {
    let number = answer
        .get("candidate")
        .and_then(Value::as_u64)
        .and_then(|number| usize::try_from(number).ok())
        .ok_or_else(|| AiError::Malformed("no candidate".to_owned()))?;
    match number {
        0 => Ok(None),
        number if number <= offered => Ok(Some(number - 1)),
        _ => Err(AiError::Malformed(
            "a candidate that was not offered".to_owned(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use postio_config::ModelEndpoint;

    use super::*;
    use crate::fake::{FakeRuntime, Reply};

    fn like_this(runtime: &FakeRuntime) -> LikeThis {
        LikeThis::new(Arc::new(
            Client::new(
                ModelEndpoint::parse("http://127.0.0.1:11434/v1").expect("loopback"),
                "a-small-model",
            )
            .with_transport(Arc::new(runtime.clone())),
        ))
    }

    fn example() -> Example<'static> {
        Example {
            account: AccountId::new(1),
            sender: "Local-First Weekly <editor@weekly.example>",
            subject: "Issue 112: Sync without servers",
            text: "This week: local-first apps that merge later.",
        }
    }

    fn candidates() -> Vec<Candidate<'static>> {
        vec![
            Candidate {
                query: "list:weekly.lists.example.org",
                sample: vec![("editor@weekly.example", "Issue 111: CRDTs in practice")],
            },
            Candidate {
                query: "from:editor@weekly.example",
                sample: vec![
                    ("editor@weekly.example", "Issue 111: CRDTs in practice"),
                    ("editor@weekly.example", "Your subscription receipt"),
                ],
            },
        ]
    }

    fn chosen(content: &str) -> Result<Option<usize>, AiError> {
        let runtime = FakeRuntime::always(Reply::content(content));
        like_this(&runtime).choose(&example(), &candidates())
    }

    #[test]
    fn the_model_picks_a_candidate_by_its_number() {
        assert_eq!(chosen(r#"{"candidate":1}"#), Ok(Some(0)));
        assert_eq!(chosen(r#"{"candidate":2}"#), Ok(Some(1)));
    }

    #[test]
    fn none_alike_is_no_rule() {
        assert_eq!(chosen(r#"{"candidate":0}"#), Ok(None));
    }

    #[test]
    fn a_number_that_names_no_candidate_is_not_believed() {
        assert!(matches!(
            chosen(r#"{"candidate":3}"#),
            Err(AiError::Malformed(_))
        ));
        assert!(matches!(
            chosen(r#"{"candidate":"from:anyone@example.com"}"#),
            Err(AiError::Malformed(_))
        ));
    }

    #[test]
    fn the_example_and_the_samples_are_fenced_as_data() {
        let runtime = FakeRuntime::always(Reply::content(r#"{"candidate":0}"#));
        like_this(&runtime)
            .choose(&example(), &candidates())
            .expect("an answer");
        let request = &runtime.requests()[0];
        let data = request.data();
        assert!(data.contains("Issue 112: Sync without servers"), "{data}");
        assert!(data.contains("[1] list:weekly.lists.example.org"), "{data}");
        assert!(data.contains("[2] from:editor@weekly.example"), "{data}");
        assert!(data.contains("Your subscription receipt"), "{data}");
        assert!(request.body.get("tools").is_none());
        let instructions = request.body["messages"][0]["content"].as_str().unwrap();
        assert!(
            !instructions.contains("Sync without servers"),
            "{instructions}"
        );
    }
}
