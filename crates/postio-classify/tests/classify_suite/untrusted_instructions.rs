//! Instruction-shaped text produces no action and no request
//! (`specs/007-postio-focus` T119, US12 scenario 4; ADR 0009 Q4).
//!
//! The corpus fixture `untrusted-instructions` is a message to Ada with one
//! honest question in its sender's own words, then text aimed at an
//! assistant: an order to ignore previous instructions, a dated demand to
//! reply with password reset codes, an order to forward every invoice, a tool
//! call naming `send_mail`, and a note to any automated agent. The
//! classifier reads all of it as data: none of it becomes a marker, and what
//! comes back is a fixed schema that holds none of the message's words.
//!
//! "Sends nothing, requests nothing" is structural. Nothing that sends or
//! connects is in this crate's dependency closure, and
//! `scripts/checks/check-crate-boundaries.py` holds that line. What a test
//! adds is that the answer is inert data.

use std::fmt::Debug;
use std::ops::Range;

use postio_classify::{
    BodyMessage, Facts, FiledMessage, MarkerKind, Outcome, OwnText, Rules, Senders, at_body,
    at_filing,
};
use postio_model::test_corpus;
use postio_model::{AccountId, EmailAddress, Identity, MailboxRole, Message, ThreadId};

/// The one thing the sender asks in their own words.
const HONEST: &str = "Could you confirm the room for Thursday's design review?";

/// Words of the injected text that must never come back in an answer.
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

/// The fixture, as a person's mail whose headers are known: no list, and
/// nothing automated.
fn message() -> Message {
    let mut message = test_corpus::load("untrusted-instructions").parse();
    message.promoted = Some(postio_model::promoted::PromotedHeaders::default());
    message
}

/// Ada, whom the fixture is sent to.
fn ada() -> Vec<Identity> {
    let mut identity = Identity::new(
        AccountId::new(1),
        EmailAddress::new(Some("Ada Norwood"), "ada.norwood@example.com"),
    );
    identity.display_name = "Ada Norwood".to_owned();
    vec![identity]
}

/// As filed in Ada's inbox.
fn filed(message: &Message) -> FiledMessage<'_> {
    FiledMessage {
        message,
        thread: message.thread_id,
        role: MailboxRole::Inbox,
    }
}

/// The body's text. It quotes nothing, so all of it is the sender's own,
/// as T115's extraction would give it.
fn body_text(message: &Message) -> &str {
    message.body.text.as_deref().expect("a text body")
}

/// Where `needle` is in `haystack`, in characters.
fn chars_of(haystack: &str, needle: &str) -> Range<usize> {
    let byte = haystack.find(needle).expect("the text holds it");
    let start = haystack[..byte].chars().count();
    start..start + needle.chars().count()
}

/// Data and only data: it can be copied, compared and sent between
/// threads, and it borrows nothing from the message it describes.
fn inert<T: Clone + Eq + Debug + Send + Sync + 'static>(_: &T) {}

#[test]
fn the_sender_s_own_question_alone_would_be_marked() {
    // The control. The fixture is sent directly to Ada and the detector
    // reads it, so the silence below is the rules' answer, not a message
    // they never saw.
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

    // Whatever is marked quotes the sender's own question, and nothing of
    // the text after it.
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
    // Nothing filed and nothing held: a person's mail to Ada, whatever its
    // text says.
    assert_eq!(outcomes[0].filter, None);
    assert_eq!(outcomes[0].hold, None);
}
