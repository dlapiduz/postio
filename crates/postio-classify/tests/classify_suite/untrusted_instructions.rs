//! None of the injected text becomes a marker, and what comes back holds none of the message's
//! words.
//! "Sends nothing" is structural (`check-crate-boundaries.py`); the test adds that the answer is
//! inert data.

use std::fmt::Debug;
use std::ops::Range;

use postio_classify::{
    BodyMessage, Facts, FiledMessage, MarkerKind, Outcome, OwnText, Rules, Senders, at_body,
    at_filing,
};
use postio_model::test_corpus;
use postio_model::{AccountId, EmailAddress, Identity, MailboxRole, Message, ThreadId};

const HONEST: &str = "Could you confirm the room for Thursday's design review?";

const INJECTED: &[&str] = &[
    "send_mail",
    "ledger@billing.example",
    "password",
    "invoice",
    "approved",
    "previous instructions",
];

struct NoGuards;

impl Facts for NoGuards {
    fn wrote_to(&self, _: &EmailAddress) -> bool {
        false
    }
    fn took_part(&self, _: ThreadId) -> bool {
        false
    }
    fn own_domain(&self, _: &EmailAddress) -> bool {
        false
    }
    fn never_filter(&self, _: &EmailAddress) -> bool {
        false
    }
}

struct BuiltIn;

impl Rules for BuiltIn {
    fn senders(&self) -> &Senders {
        Senders::shipped()
    }
}

fn message() -> Message {
    let mut message = test_corpus::load("untrusted-instructions").parse();
    message.promoted = Some(postio_model::promoted::PromotedHeaders::default());
    message
}

fn ada() -> Vec<Identity> {
    let mut identity = Identity::new(
        AccountId::new(1),
        EmailAddress::new(Some("Ada Norwood"), "ada.norwood@example.com"),
    );
    identity.display_name = "Ada Norwood".to_owned();
    vec![identity]
}

fn filed(message: &Message) -> FiledMessage<'_> {
    FiledMessage {
        message,
        thread: message.thread_id,
        role: MailboxRole::Inbox,
    }
}

fn body_text(message: &Message) -> &str {
    message.body.text.as_deref().expect("a text body")
}

fn chars_of(haystack: &str, needle: &str) -> Range<usize> {
    let byte = haystack.find(needle).expect("the text holds it");
    let start = haystack[..byte].chars().count();
    start..start + needle.chars().count()
}

fn inert<T: Clone + Eq + Debug + Send + Sync + 'static>(_: &T) {}

#[test]
fn the_sender_s_own_question_alone_would_be_marked() {
    let message = message();
    let identities = ada();
    let body = BodyMessage {
        filed: filed(&message),
        identities: &identities,
    };
    let own = format!("Hi Ada,\n\n{HONEST}\n");

    let outcome = at_body(&body, &OwnText::new(&own), &NoGuards, &BuiltIn);

    let marker = outcome.marker.expect("the honest question is marked");
    assert_eq!(marker.kind, MarkerKind::Question);
    assert_eq!(marker.span, Some(chars_of(&own, HONEST)));
}

#[test]
fn no_marker_is_built_from_the_injected_instructions() {
    let message = message();
    let identities = ada();
    let body = BodyMessage {
        filed: filed(&message),
        identities: &identities,
    };
    let text = body_text(&message);

    let outcome = at_body(&body, &OwnText::new(text), &NoGuards, &BuiltIn);

    if let Some(marker) = outcome.marker {
        let span = marker.span.expect("a question or a to-do quotes a span");
        let honest = chars_of(text, HONEST);
        let quoted: String = text.chars().skip(span.start).take(span.len()).collect();
        assert!(
            honest.start <= span.start && span.end <= honest.end,
            "a {:?} marker quotes injected text: {quoted:?}",
            marker.kind
        );
    }
}

#[test]
fn what_comes_back_is_inert_data_holding_none_of_the_message_s_words() {
    let message = message();
    let identities = ada();
    let body = BodyMessage {
        filed: filed(&message),
        identities: &identities,
    };
    let text = OwnText::new(body_text(&message));

    let outcomes: [Outcome; 2] = [
        at_filing(&filed(&message), &NoGuards, &BuiltIn),
        at_body(&body, &text, &NoGuards, &BuiltIn),
    ];

    for outcome in &outcomes {
        inert(outcome);
        let shown = format!("{outcome:?}");
        for word in INJECTED {
            assert!(
                !shown.contains(word),
                "the answer carries {word:?} from the message: {shown}"
            );
        }
    }
    assert_eq!(outcomes[0].filter, None);
    assert_eq!(outcomes[0].hold, None);
}
