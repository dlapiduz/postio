//! The person's model, answering the needs-action question in the built-in
//! detector's place (spec 007 FR-107, FR-170, US12 scenario 6).
//!
//! [`NeedsActionModel`] is `postio-classify`'s [`ModelLayer`]. The classifier
//! decides whether to ask at all -- only mail sent directly to the person,
//! and never text that speaks to a machine (FR-106, ADR 0009 Q4) -- and this
//! asks the model the rest, in a fixed schema:
//!
//! - **`kind`**: `question`, `todo` or `none`;
//! - **`quote`**: the sentence that asks, copied as written;
//! - **`due`**: the day a to-do names, `YYYY-MM-DD`, or nothing.
//!
//! **The quote is found, not believed.** It becomes a span only if it is in
//! the message's own text byte for byte; a model that paraphrases, or
//! quotes the history or the headers, is dropped, and the message goes
//! unmarked rather than mismarked. So a marker quotes the message's own
//! words by construction, whichever detector found it (FR-104).
//!
//! **When the model cannot answer** -- it is not running, too slow, or off
//! its schema -- the answer is [`Unavailable`], and the classifier asks the
//! built-in detector instead. Nothing waits for the model (FR-107).

use std::sync::Arc;

use chrono::{DateTime, Local, NaiveDate, NaiveTime, TimeZone, Utc};
use postio_classify::{
    BodyMessage, FiledMessage, MarkerKind, ModelLayer, NeedsAction, OwnText, ReasonKind,
    Unavailable,
};
use serde_json::{Map, Value};

use crate::client::{Client, Question};
use crate::schema::{Field, Kind, Schema};

/// The answer's shape.
const SCHEMA: Schema = Schema {
    name: "needs_action",
    fields: &[
        Field {
            name: "kind",
            kind: Kind::Choice(&["question", "todo", "none"]),
        },
        Field {
            name: "quote",
            kind: Kind::Text { max: 600 },
        },
        Field {
            name: "due",
            kind: Kind::Date,
        },
    ],
};

/// What the model is asked, in Postio's words.
const ASKED: &str = "You help one person sort their email. The email you are given was \
    sent to them. Decide whether its author asks them a direct question they are expected \
    to answer, or asks them to do something.\n\
    - kind: \"question\" when the author asks the reader a question; \"todo\" when the \
    author asks the reader to do something; \"none\" otherwise. Pleasantries, rhetorical \
    questions, boilerplate such as \"let me know if you have any questions\", and requests \
    made of somebody else are \"none\". When unsure, answer \"none\".\n\
    - quote: the one sentence that asks, copied exactly as it is written in the email, \
    character for character, with nothing added, left out or changed. Empty when kind is \
    \"none\".\n\
    - due: for a todo whose sentence names a day, that day as YYYY-MM-DD, counted from the \
    day the email was sent. Otherwise empty.";

/// The morning a to-do falls due when its sentence names only a day: the
/// hour `parse_when` gives a day with no time, as the built-in detector
/// reads it.
const DUE_AT: NaiveTime = NaiveTime::from_hms_opt(8, 0, 0).expect("a real time");

/// The person's model, as the needs-action question's answerer.
#[derive(Debug, Clone)]
pub struct NeedsActionModel {
    client: Arc<Client>,
}

impl NeedsActionModel {
    /// Ask `client`'s model.
    pub fn new(client: Arc<Client>) -> Self {
        Self { client }
    }
}

impl ModelLayer for NeedsActionModel {
    /// Focus asks the model no filter question: filtering decides by guards,
    /// structure, rules and corrections, and leaves the rest in the inbox
    /// (FR-167). This answers "stay", which is what an open question comes
    /// to anyway.
    fn filter(&self, _: &FiledMessage<'_>) -> Result<Option<ReasonKind>, Unavailable> {
        Ok(None)
    }

    fn needs_action(
        &self,
        message: &BodyMessage<'_>,
        text: &OwnText<'_>,
    ) -> Result<Option<NeedsAction>, Unavailable> {
        if text.as_str().trim().is_empty() {
            return Ok(None);
        }
        let mail = message.filed.message;
        let sent = mail.date.unwrap_or(mail.received_at).with_timezone(&Local);
        let instructions = format!(
            "{ASKED}\n\nThe email was sent on {}. The reader goes by: {}.",
            sent.format("%A %Y-%m-%d"),
            reader(message)
        );
        let answer = self
            .client
            .ask(&Question {
                schema: &SCHEMA,
                instructions: &instructions,
                data: text.as_str(),
                account: Some(mail.account_id),
            })
            .map_err(|_| Unavailable)?;
        let found = read(&answer, text);
        tracing::debug!(
            message = mail.id.get(),
            marked = found.is_some(),
            "the model answered the needs-action question"
        );
        Ok(found)
    }
}

/// The names and addresses the reader goes by, from their identities: who
/// "you" is.
fn reader(message: &BodyMessage<'_>) -> String {
    let named: Vec<String> = message
        .identities
        .iter()
        .map(|identity| match &identity.address.name {
            Some(name) => format!("{name} <{}>", identity.address.address),
            None => identity.address.address.clone(),
        })
        .collect();
    if named.is_empty() {
        "(unknown)".to_owned()
    } else {
        named.join(", ")
    }
}

/// The model's answer as the classifier takes it: a kind, the quote found in
/// `text` as a span of characters, and a to-do's due date. A quote not in
/// the text verbatim is no answer.
fn read(answer: &Map<String, Value>, text: &OwnText<'_>) -> Option<NeedsAction> {
    let kind = match answer.get("kind")?.as_str()? {
        "question" => MarkerKind::Question,
        "todo" => MarkerKind::Todo,
        _ => return None,
    };
    let span = span_of(answer.get("quote")?.as_str()?, text)?;
    let due_at = match kind {
        MarkerKind::Todo => answer
            .get("due")
            .and_then(Value::as_str)
            .and_then(|due| NaiveDate::parse_from_str(due, "%Y-%m-%d").ok())
            .and_then(morning_of),
        _ => None,
    };
    Some(NeedsAction { kind, span, due_at })
}

/// Where `quote` is in `text`, in characters, if it is there byte for byte.
/// Surrounding whitespace is not part of a sentence, and is let go; nothing
/// else is.
pub(crate) fn span_of(quote: &str, text: &OwnText<'_>) -> Option<std::ops::Range<usize>> {
    let quote = quote.trim();
    if quote.chars().filter(|c| c.is_alphanumeric()).count() < 3 {
        return None;
    }
    let at = text.as_str().find(quote)?;
    let start = text.as_str()[..at].chars().count();
    Some(start..start + quote.chars().count())
}

/// `day` at [`DUE_AT`], on this computer's clock.
fn morning_of(day: NaiveDate) -> Option<DateTime<Utc>> {
    Local
        .from_local_datetime(&day.and_time(DUE_AT))
        .earliest()
        .map(|at| at.with_timezone(&Utc))
}

#[cfg(test)]
mod tests {
    use postio_classify::{Facts, MarkedBy, Rules, Senders};
    use postio_config::ModelEndpoint;
    use postio_model::{AccountId, EmailAddress, Identity, MailboxId, MailboxRole, Message};

    use super::*;
    use crate::fake::{FakeRuntime, Reply};

    const APPROVE: &str = "Can you approve these by Friday so finance can close the quarter?";

    fn text() -> String {
        format!("Hi Ada,\n\n{APPROVE}\n\nThanks,\nTove")
    }

    fn letter() -> Message {
        let sent = Local
            .with_ymd_and_hms(2026, 9, 26, 12, 0, 0)
            .single()
            .expect("a real time")
            .with_timezone(&Utc);
        let mut message = Message::new(AccountId::new(1), MailboxId::new(1), sent);
        message.date = Some(sent);
        message.from = vec![EmailAddress::new(Some("Tove"), "tove@example.org")];
        message.to = vec![EmailAddress::new(Some("Ada"), "ada@example.com")];
        message
    }

    fn identities() -> Vec<Identity> {
        vec![Identity::new(
            AccountId::new(1),
            EmailAddress::new(Some("Ada Norwood"), "ada@example.com"),
        )]
    }

    fn model(runtime: &FakeRuntime) -> NeedsActionModel {
        NeedsActionModel::new(Arc::new(
            Client::new(
                ModelEndpoint::parse("http://127.0.0.1:11434/v1").expect("loopback"),
                "a-small-model",
            )
            .with_transport(Arc::new(runtime.clone())),
        ))
    }

    fn answered(content: &str) -> Result<Option<NeedsAction>, Unavailable> {
        let runtime = FakeRuntime::always(Reply::content(content));
        let message = letter();
        let identities = identities();
        let body = BodyMessage {
            filed: FiledMessage {
                message: &message,
                thread: None,
                role: MailboxRole::Inbox,
            },
            identities: &identities,
        };
        let text = text();
        model(&runtime).needs_action(&body, &OwnText::new(&text))
    }

    fn span_of_approve() -> std::ops::Range<usize> {
        let start = "Hi Ada,\n\n".chars().count();
        start..start + APPROVE.chars().count()
    }

    #[test]
    fn the_model_s_quote_becomes_a_span_of_the_own_text() {
        let answer = answered(&format!(
            r#"{{"kind":"question","quote":"{APPROVE}","due":""}}"#
        ))
        .expect("an answer")
        .expect("a question");
        assert_eq!(answer.kind, MarkerKind::Question);
        assert_eq!(answer.span, span_of_approve());
        assert_eq!(answer.due_at, None);
    }

    #[test]
    fn a_quote_that_is_not_verbatim_in_the_text_is_dropped() {
        // FR-104, FR-170: a paraphrase is the model's words, not the
        // message's, and a marker never quotes anything but the message.
        for quote in [
            "Could you approve these by Friday so finance can close the quarter?",
            "can you approve these by friday so finance can close the quarter?",
            "Approve the Q3 figures by Friday.",
            // Quoted history is not the own text, so a quote from it is not
            // found there.
            "Could you look at the draft when you have a moment?",
            // Too little to be a sentence, even where it matches.
            "Hi",
        ] {
            let answer = answered(&format!(
                r#"{{"kind":"question","quote":"{quote}","due":""}}"#
            ));
            assert_eq!(answer, Ok(None), "{quote:?}");
        }
    }

    #[test]
    fn a_to_do_is_due_on_the_morning_of_the_day_it_names() {
        let answer = answered(&format!(
            r#"{{"kind":"todo","quote":"{APPROVE}","due":"2026-10-02"}}"#
        ))
        .expect("an answer")
        .expect("a to-do");
        assert_eq!(answer.kind, MarkerKind::Todo);
        let due = answer.due_at.expect("a due date").with_timezone(&Local);
        assert_eq!(
            (due.date_naive(), due.time()),
            (NaiveDate::from_ymd_opt(2026, 10, 2).unwrap(), DUE_AT)
        );
    }

    #[test]
    fn nothing_asked_is_no_marker() {
        assert_eq!(answered(r#"{"kind":"none","quote":"","due":""}"#), Ok(None));
    }

    #[test]
    fn a_model_that_cannot_answer_is_unavailable() {
        for content in [
            r#"{"kind":"question"}"#,
            r#"{"kind":"question","quote":"x","due":"","link":"https://example.com"}"#,
            "It asks a question.",
        ] {
            assert_eq!(answered(content), Err(Unavailable), "{content}");
        }
    }

    #[test]
    fn the_request_fences_the_own_text_and_says_who_the_reader_is() {
        let runtime = FakeRuntime::always(Reply::content(r#"{"kind":"none","quote":"","due":""}"#));
        let message = letter();
        let identities = identities();
        let body = BodyMessage {
            filed: FiledMessage {
                message: &message,
                thread: None,
                role: MailboxRole::Inbox,
            },
            identities: &identities,
        };
        let text = text();
        model(&runtime)
            .needs_action(&body, &OwnText::new(&text))
            .expect("an answer");

        let request = &runtime.requests()[0];
        assert_eq!(request.data(), format!("<<<DATA-0\n{text}\nDATA-0>>>"));
        let instructions = request.body["messages"][0]["content"].as_str().unwrap();
        assert!(
            instructions.contains("Saturday 2026-09-26"),
            "{instructions}"
        );
        assert!(
            instructions.contains("Ada Norwood <ada@example.com>"),
            "{instructions}"
        );
        assert!(
            !instructions.contains("finance"),
            "the message stays in its fence"
        );
    }

    // --- Through the classifier, as the body stage asks it ------------------

    struct NoFacts;

    impl Facts for NoFacts {
        fn wrote_to(&self, _: &EmailAddress) -> bool {
            true
        }
        fn took_part(&self, _: postio_model::ThreadId) -> bool {
            true
        }
        fn own_domain(&self, _: &EmailAddress) -> bool {
            true
        }
        fn never_filter(&self, _: &EmailAddress) -> bool {
            true
        }
    }

    struct Shipped;

    impl Rules for Shipped {
        fn senders(&self) -> &Senders {
            Senders::shipped()
        }
    }

    fn classified(runtime: &FakeRuntime) -> Option<postio_classify::MarkerCandidate> {
        let message = letter();
        let identities = identities();
        let body = BodyMessage {
            filed: FiledMessage {
                message: &message,
                thread: None,
                role: MailboxRole::Inbox,
            },
            identities: &identities,
        };
        let text = text();
        let model = model(runtime);
        postio_classify::at_body_with(
            &body,
            &OwnText::new(&text),
            &NoFacts,
            &Shipped,
            Some(&model),
        )
        .marker
    }

    #[test]
    fn with_a_model_connected_the_marker_is_the_model_s() {
        // US12 scenario 6, first half: the model says it is a to-do, where
        // the built-in detector would have said question.
        let runtime = FakeRuntime::always(Reply::content(format!(
            r#"{{"kind":"todo","quote":"{APPROVE}","due":""}}"#
        )));
        let marker = classified(&runtime).expect("a marker");
        assert_eq!(marker.by, MarkedBy::Model);
        assert_eq!(marker.kind, postio_classify::MarkerKind::Todo);
        assert_eq!(marker.span, Some(span_of_approve()));
        assert_eq!(runtime.connections(), 1);
    }

    #[test]
    fn with_the_model_not_running_the_built_in_detector_answers() {
        // US12 scenario 6, second half: nothing waits for the model.
        let runtime = FakeRuntime::refusing();
        let marker = classified(&runtime).expect("the built-in detector's marker");
        assert_eq!(marker.by, MarkedBy::Detector);
        assert_eq!(marker.kind, postio_classify::MarkerKind::Question);
        assert_eq!(marker.span, Some(span_of_approve()));
    }
}
