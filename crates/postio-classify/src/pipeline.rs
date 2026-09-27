//! The layers, and the rule that binds them: an earlier layer's decision
//! stands (FR-130).

use chrono::{DateTime, Utc};

use crate::facts::Facts;
use crate::input::{BodyMessage, FiledMessage, OwnText};
use crate::outcome::{
    Layer, MarkerCandidate, MarkerKind, Outcome, Reason, ReasonKind, RuleName, Span,
};
use crate::rules::Rules;

/// Classifies a message as it is filed, before its body: whether to filter
/// it and whether to hold it (`contracts/engine.md`).
///
/// The layers run in FR-130's order -- guards, corrections, structure and
/// rules -- and each decides only what no earlier layer has. With no rules
/// yet, nothing is decided, and the outcome is empty.
pub fn at_filing(message: &FiledMessage<'_>, facts: &dyn Facts, rules: &dyn Rules) -> Outcome {
    Pipeline::built_in().at_filing(message, facts, rules)
}

/// Classifies a message once its body has arrived: what it asks of the user,
/// quoted from `text`, its own words.
pub fn at_body(
    message: &BodyMessage<'_>,
    text: &OwnText<'_>,
    facts: &dyn Facts,
    rules: &dyn Rules,
) -> Outcome {
    Pipeline::built_in().at_body(message, text, facts, rules)
}

/// Layer 4: the user's own model, when they have brought one (milestone 2,
/// FR-165, FR-170). `postio-ai` implements it.
///
/// It is asked only what layers 1-3 left open, and it can answer only in the
/// fixed schema: a category, or a kind with a span and a due date. There is
/// no field in either answer for words of its own (FR-132), and what it
/// returns is checked before it is believed.
pub trait ModelLayer {
    /// A filter reason for a message the earlier layers left open, or `None`
    /// to leave it in the inbox.
    fn filter(&self, message: &FiledMessage<'_>) -> Option<ReasonKind>;

    /// Whether the message's own text asks something of the user, and where.
    fn needs_action(&self, message: &BodyMessage<'_>, text: &OwnText<'_>) -> Option<NeedsAction>;
}

/// A model's needs-action answer: what kind, which characters of the own
/// text, and when it is due. Nothing else.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NeedsAction {
    /// A question or a to-do. An invitation is never the model's to find:
    /// it comes from the calendar part (FR-100).
    pub kind: MarkerKind,
    /// The sentence, as character offsets into the [`OwnText`].
    pub span: Span,
    /// The due date the sentence names, if it names one.
    pub due_at: Option<DateTime<Utc>>,
}

/// One question's answer so far, as the layers hand it down.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) enum Slot<T> {
    /// No layer has decided it.
    #[default]
    Open,
    /// A layer decided it: an answer, or `None` for "not this", which closes
    /// the question as firmly as an answer does. That is how a guard keeps
    /// mail in the inbox against every rule after it.
    Decided(Option<T>),
}

impl<T> Slot<T> {
    /// Decides the question, unless an earlier layer already has: its
    /// decision stands (FR-130).
    pub(crate) fn decide(&mut self, answer: Option<T>) {
        if self.is_open() {
            *self = Slot::Decided(answer);
        }
    }

    /// No layer has decided it yet.
    pub(crate) fn is_open(&self) -> bool {
        matches!(self, Slot::Open)
    }

    fn answer(self) -> Option<T> {
        match self {
            Slot::Open => None,
            Slot::Decided(answer) => answer,
        }
    }
}

/// The three questions, as the layers pass them down.
#[derive(Debug, Default)]
pub(crate) struct Decisions {
    pub(crate) filter: Slot<Reason>,
    pub(crate) hold: Slot<RuleName>,
    pub(crate) marker: Slot<MarkerCandidate>,
}

impl Decisions {
    /// What was decided. A question no layer answered is answered `None`:
    /// when in doubt, mail goes to the inbox, unheld and unmarked.
    fn outcome(self) -> Outcome {
        Outcome {
            filter: self.filter.answer(),
            hold: self.hold.answer(),
            marker: self.marker.answer(),
        }
    }
}

/// A built-in layer: it decides what it can and leaves the rest open.
pub(crate) trait Stage {
    fn at_filing(
        &self,
        _message: &FiledMessage<'_>,
        _facts: &dyn Facts,
        _rules: &dyn Rules,
        _decisions: &mut Decisions,
    ) {
    }

    fn at_body(
        &self,
        _message: &BodyMessage<'_>,
        _text: &OwnText<'_>,
        _facts: &dyn Facts,
        _rules: &dyn Rules,
        _decisions: &mut Decisions,
    ) {
    }
}

/// Layer 1, the guards (FR-111): the user wrote to the sender, took part in
/// the conversation, shares its domain, or pinned it. Through [`Facts`];
/// T101 gives it its checks.
struct Guards;

impl Stage for Guards {}

/// Layer 2, the user's corrections (FR-108, FR-116). They rank above the
/// rules because a restore must beat the rule that filtered the message;
/// T118 and T125 give it what it reads.
struct Corrections;

impl Stage for Corrections {}

/// Layer 3, structure and rules: calendar parts, list and bulk headers, the
/// automated-senders table, the user's digest rules, and the built-in
/// needs-action detector (T105, T110, T116, T122, T133).
struct StructureAndRules;

impl Stage for StructureAndRules {}

/// Layers 1-3, in FR-130's order.
const BUILT_IN: &[&dyn Stage] = &[&Guards, &Corrections, &StructureAndRules];

/// The layers a classification runs through, and the model after them when
/// the user has brought one.
pub(crate) struct Pipeline<'a> {
    stages: &'a [&'a dyn Stage],
    model: Option<&'a dyn ModelLayer>,
}

impl Pipeline<'static> {
    /// Layers 1-3, and no model: milestone 1.
    pub(crate) fn built_in() -> Self {
        Pipeline {
            stages: BUILT_IN,
            model: None,
        }
    }
}

impl<'a> Pipeline<'a> {
    #[cfg(test)]
    pub(crate) fn new(stages: &'a [&'a dyn Stage], model: Option<&'a dyn ModelLayer>) -> Self {
        Pipeline { stages, model }
    }

    pub(crate) fn at_filing(
        &self,
        message: &FiledMessage<'_>,
        facts: &dyn Facts,
        rules: &dyn Rules,
    ) -> Outcome {
        let mut decisions = Decisions::default();
        for stage in self.stages {
            stage.at_filing(message, facts, rules, &mut decisions);
        }
        if let Some(model) = self.model
            && decisions.filter.is_open()
        {
            let reason = model.filter(message).map(|kind| Reason {
                kind,
                source: None,
                layer: Layer::Model,
            });
            decisions.filter.decide(reason);
        }
        decisions.outcome()
    }

    pub(crate) fn at_body(
        &self,
        message: &BodyMessage<'_>,
        text: &OwnText<'_>,
        facts: &dyn Facts,
        rules: &dyn Rules,
    ) -> Outcome {
        let mut decisions = Decisions::default();
        for stage in self.stages {
            stage.at_body(message, text, facts, rules, &mut decisions);
        }
        if let Some(model) = self.model
            && decisions.marker.is_open()
        {
            let marker = model
                .needs_action(message, text)
                .filter(|answer| believable(answer, text))
                .map(|answer| MarkerCandidate {
                    kind: answer.kind,
                    span: Some(answer.span),
                    starts_at: None,
                    ends_at: None,
                    due_at: answer.due_at,
                    invite: None,
                });
            decisions.marker.decide(marker);
        }
        decisions.outcome()
    }
}

/// Whether a model's needs-action answer can be taken as one: a question or
/// a to-do (an invitation is the calendar part's to say), quoting a
/// non-empty run of characters that are really in `text`. Anything else is
/// no answer at all, and the message goes unmarked rather than mismarked.
fn believable(answer: &NeedsAction, text: &OwnText<'_>) -> bool {
    matches!(answer.kind, MarkerKind::Question | MarkerKind::Todo)
        && answer.span.start < answer.span.end
        && answer.span.end <= text.len_chars()
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use postio_model::{AccountId, EmailAddress, MailboxId, MailboxRole, Message, ThreadId};

    use super::*;

    /// No guard applies: the user never wrote to anybody, took part in
    /// nothing, owns no domain and pinned nobody.
    struct NoFacts;

    impl Facts for NoFacts {
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

    struct NoRules;

    impl Rules for NoRules {}

    fn message() -> Message {
        let mut message = Message::new(AccountId::new(1), MailboxId::new(1), Utc::now());
        message.from = vec![EmailAddress::new(Some("Forge"), "notify@forge.example.com")];
        message.to = vec![EmailAddress::new(
            Some("Ada Norwood"),
            "ada.norwood@example.com",
        )];
        message
    }

    fn filed(message: &Message) -> FiledMessage<'_> {
        FiledMessage {
            message,
            mailbox: Some(MailboxRole::Inbox),
            unsubscribe_offered: Some(true),
            automation: Some(8),
            has_calendar: false,
        }
    }

    const TEXT: &str = "Can you approve these by Friday so finance can close the quarter?";

    fn reason(kind: ReasonKind, layer: Layer) -> Reason {
        Reason {
            kind,
            source: None,
            layer,
        }
    }

    // --- Layers that decide one thing each ----------------------------------

    /// Files everything, for `kind`.
    struct Files(ReasonKind);

    impl Stage for Files {
        fn at_filing(&self, _: &FiledMessage<'_>, _: &dyn Facts, _: &dyn Rules, d: &mut Decisions) {
            d.filter.decide(Some(reason(self.0, Layer::Header)));
        }
    }

    /// A guard: keeps everything in the inbox.
    struct Keeps;

    impl Stage for Keeps {
        fn at_filing(&self, _: &FiledMessage<'_>, _: &dyn Facts, _: &dyn Rules, d: &mut Decisions) {
            d.filter.decide(None);
        }
    }

    /// Holds everything under one digest rule.
    struct Holds(&'static str);

    impl Stage for Holds {
        fn at_filing(&self, _: &FiledMessage<'_>, _: &dyn Facts, _: &dyn Rules, d: &mut Decisions) {
            d.hold.decide(Some(RuleName(self.0.to_owned())));
        }
    }

    /// Marks every body with an invitation.
    struct Invites;

    impl Stage for Invites {
        fn at_body(
            &self,
            _: &BodyMessage<'_>,
            _: &OwnText<'_>,
            _: &dyn Facts,
            _: &dyn Rules,
            d: &mut Decisions,
        ) {
            d.marker.decide(Some(MarkerCandidate {
                kind: MarkerKind::Invite,
                span: None,
                starts_at: None,
                ends_at: None,
                due_at: None,
                invite: None,
            }));
        }
    }

    /// A model that answers what it is told to, and counts the questions.
    #[derive(Default)]
    struct Model {
        filter: Option<ReasonKind>,
        needs_action: Option<NeedsAction>,
        asked: Cell<u32>,
    }

    impl ModelLayer for Model {
        fn filter(&self, _: &FiledMessage<'_>) -> Option<ReasonKind> {
            self.asked.set(self.asked.get() + 1);
            self.filter
        }
        fn needs_action(&self, _: &BodyMessage<'_>, _: &OwnText<'_>) -> Option<NeedsAction> {
            self.asked.set(self.asked.get() + 1);
            self.needs_action.clone()
        }
    }

    fn classify_filing(stages: &[&dyn Stage], model: Option<&dyn ModelLayer>) -> Outcome {
        let message = message();
        Pipeline::new(stages, model).at_filing(&filed(&message), &NoFacts, &NoRules)
    }

    fn classify_body(stages: &[&dyn Stage], model: Option<&dyn ModelLayer>) -> Outcome {
        let message = message();
        let body = BodyMessage {
            filed: filed(&message),
        };
        Pipeline::new(stages, model).at_body(&body, &OwnText::new(TEXT), &NoFacts, &NoRules)
    }

    // --- The built-in pipeline ----------------------------------------------

    #[test]
    fn with_no_rules_yet_the_built_in_pipeline_decides_nothing() {
        // Mail that every later rule would have something to say about -- a
        // notifier's address, List-Unsubscribe, Auto-Submitted, a question --
        // stays in the inbox, unheld and unmarked, until a rule says so.
        let message = message();
        let body = BodyMessage {
            filed: filed(&message),
        };

        assert_eq!(
            at_filing(&filed(&message), &NoFacts, &NoRules),
            Outcome::default()
        );
        assert_eq!(
            at_body(&body, &OwnText::new(TEXT), &NoFacts, &NoRules),
            Outcome::default()
        );
    }

    // --- An earlier layer's decision stands (FR-130) -------------------------

    #[test]
    fn an_earlier_layer_s_decision_stands() {
        let outcome = classify_filing(
            &[&Files(ReasonKind::Spam), &Files(ReasonKind::Promotion)],
            None,
        );

        assert_eq!(
            outcome.filter,
            Some(reason(ReasonKind::Spam, Layer::Header))
        );
    }

    #[test]
    fn a_guard_s_no_closes_the_question_against_every_rule_after_it() {
        let outcome = classify_filing(&[&Keeps, &Files(ReasonKind::Promotion)], None);

        assert_eq!(outcome.filter, None);
    }

    #[test]
    fn each_question_is_decided_on_its_own() {
        let outcome = classify_filing(&[&Keeps, &Holds("Newsletters")], None);

        assert_eq!(outcome.filter, None);
        assert_eq!(outcome.hold, Some(RuleName("Newsletters".to_owned())));
        assert_eq!(outcome.marker, None);
    }

    // --- The model seam (milestone 2) -----------------------------------------

    #[test]
    fn the_model_answers_what_the_layers_left_open() {
        let model = Model {
            filter: Some(ReasonKind::Notification),
            ..Model::default()
        };
        let outcome = classify_filing(&[], Some(&model));

        assert_eq!(
            outcome.filter,
            Some(reason(ReasonKind::Notification, Layer::Model))
        );
        assert_eq!(model.asked.get(), 1);
    }

    #[test]
    fn the_model_is_not_asked_what_a_layer_already_decided() {
        let model = Model {
            filter: Some(ReasonKind::Notification),
            ..Model::default()
        };
        let outcome = classify_filing(&[&Keeps], Some(&model));

        assert_eq!(outcome.filter, None, "the guard's no stands");
        assert_eq!(model.asked.get(), 0, "and the model never saw the message");
    }

    #[test]
    fn a_marker_a_layer_made_is_not_the_model_s_to_replace() {
        let model = Model {
            needs_action: Some(NeedsAction {
                kind: MarkerKind::Question,
                span: 0..TEXT.chars().count(),
                due_at: None,
            }),
            ..Model::default()
        };
        let outcome = classify_body(&[&Invites], Some(&model));

        assert_eq!(
            outcome.marker.map(|marker| marker.kind),
            Some(MarkerKind::Invite)
        );
        assert_eq!(model.asked.get(), 0);
    }

    #[test]
    fn the_model_s_marker_is_a_span_of_the_text_and_nothing_else() {
        let whole = 0..TEXT.chars().count();
        let model = Model {
            needs_action: Some(NeedsAction {
                kind: MarkerKind::Question,
                span: whole.clone(),
                due_at: None,
            }),
            ..Model::default()
        };
        let outcome = classify_body(&[], Some(&model));

        assert_eq!(
            outcome.marker,
            Some(MarkerCandidate {
                kind: MarkerKind::Question,
                span: Some(whole),
                starts_at: None,
                ends_at: None,
                due_at: None,
                invite: None,
            })
        );
    }

    #[test]
    fn a_model_span_outside_the_text_is_no_answer() {
        // Past the end, empty, and backwards.
        let backwards = std::ops::Range { start: 30, end: 10 };
        for span in [10..500, 20..20, backwards] {
            let model = Model {
                needs_action: Some(NeedsAction {
                    kind: MarkerKind::Todo,
                    span: span.clone(),
                    due_at: None,
                }),
                ..Model::default()
            };

            assert_eq!(classify_body(&[], Some(&model)).marker, None, "{span:?}");
        }
    }

    #[test]
    fn a_model_cannot_make_an_invitation() {
        // Invitations come from the message's own calendar part, with no
        // model involved (FR-100).
        let model = Model {
            needs_action: Some(NeedsAction {
                kind: MarkerKind::Invite,
                span: 0..10,
                due_at: None,
            }),
            ..Model::default()
        };

        assert_eq!(classify_body(&[], Some(&model)).marker, None);
    }
}
