//! Counts and message ids only, never a source name: it is a sender's or a list's.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use postio_model::MessageId;

use crate::needs_action::Asked;
use crate::outcome::{MarkedBy, MarkerKind, Outcome};

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Tally {
    examined: usize,
    filed: BTreeMap<(&'static str, &'static str), Vec<MessageId>>,
    held: BTreeMap<String, Vec<MessageId>>,
    considered: Vec<MessageId>,
    noticed: Vec<MessageId>,
    not_considered: usize,
    no_body: usize,
    marked: BTreeMap<(&'static str, &'static str), Vec<MessageId>>,
}

impl Tally {
    pub fn new() -> Self {
        Self::default()
    }

    /// A hold wins over a reason, as the pass carries it out.
    pub fn filing(&mut self, id: MessageId, outcome: &Outcome) {
        self.examined += 1;
        if let Some(rule) = &outcome.hold {
            self.held
                .entry(rule.as_str().to_owned())
                .or_default()
                .push(id);
        } else if let Some(reason) = &outcome.filter {
            self.filed
                .entry((reason.kind.as_str(), reason.layer.as_str()))
                .or_default()
                .push(id);
        }
    }

    /// `None` with a guard that asks is a body this machine does not hold.
    pub fn body(&mut self, id: MessageId, asked: Asked, outcome: Option<&Outcome>) {
        match asked {
            Asked::Person => self.considered.push(id),
            Asked::Notice => self.noticed.push(id),
            Asked::Neither => {
                self.not_considered += 1;
                return;
            }
        }
        let Some(outcome) = outcome else {
            self.no_body += 1;
            return;
        };
        if let Some(marker) = &outcome.marker {
            self.marked
                .entry((kind_name(marker.kind), by_name(marker.by)))
                .or_default()
                .push(id);
        }
    }

    pub fn render(&self) -> String {
        let mut out = String::new();
        let filed: usize = self.filed.values().map(Vec::len).sum();
        let held: usize = self.held.values().map(Vec::len).sum();
        let marked: usize = self.marked.values().map(Vec::len).sum();
        let _ = writeln!(out, "examined {}", self.examined);
        let _ = writeln!(out, "filed {filed}");
        for ((reason, layer), group) in &self.filed {
            let _ = write!(out, "  {reason} {layer} {}:", group.len());
            ids(&mut out, group);
        }
        let _ = writeln!(out, "held {held}");
        for (rule, group) in &self.held {
            let _ = write!(out, "  {rule} {}:", group.len());
            ids(&mut out, group);
        }
        let _ = writeln!(out, "left in the inbox {}", self.examined - filed - held);
        let _ = write!(out, "considered {}:", self.considered.len());
        ids(&mut out, &self.considered);
        let _ = write!(out, "noticed {}:", self.noticed.len());
        ids(&mut out, &self.noticed);
        let _ = writeln!(out, "not considered {}", self.not_considered);
        let _ = writeln!(out, "no body here {}", self.no_body);
        let _ = writeln!(out, "marked {marked}");
        for ((kind, by), group) in &self.marked {
            let _ = write!(out, "  {kind} {by} {}:", group.len());
            ids(&mut out, group);
        }
        out
    }
}

fn kind_name(kind: MarkerKind) -> &'static str {
    match kind {
        MarkerKind::Invite => "invite",
        MarkerKind::Question => "question",
        MarkerKind::Todo => "todo",
    }
}

fn by_name(by: MarkedBy) -> &'static str {
    match by {
        MarkedBy::Calendar => "calendar",
        MarkedBy::Detector => "detector",
        MarkedBy::Model => "model",
        MarkedBy::Notice => "notice",
    }
}

fn ids(out: &mut String, ids: &[MessageId]) {
    for id in ids {
        let _ = write!(out, " {}", id.get());
    }
    out.push('\n');
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::outcome::{Layer, MarkerCandidate, Reason, ReasonKind, RuleName, SourceName};

    fn id(n: i64) -> MessageId {
        MessageId::new(n)
    }

    fn filed(kind: ReasonKind, layer: Layer) -> Outcome {
        Outcome {
            filter: Some(Reason {
                kind,
                source: Some(SourceName("Ledger Weekly".into())),
                layer,
            }),
            ..Outcome::default()
        }
    }

    fn marked(kind: MarkerKind, by: MarkedBy) -> Outcome {
        Outcome {
            marker: Some(MarkerCandidate {
                kind,
                span: Some(0..10),
                starts_at: None,
                ends_at: None,
                due_at: None,
                invite: None,
                by,
            }),
            ..Outcome::default()
        }
    }

    #[test]
    fn filings_are_counted_by_reason_and_layer_with_their_ids() {
        let mut tally = Tally::new();
        tally.filing(id(3), &filed(ReasonKind::Promotion, Layer::Header));
        tally.filing(id(9), &filed(ReasonKind::Promotion, Layer::Header));
        tally.filing(id(4), &filed(ReasonKind::Notification, Layer::Senders));
        tally.filing(id(5), &Outcome::default());

        let text = tally.render();

        assert!(text.contains("examined 4\n"), "{text}");
        assert!(text.contains("filed 3\n"), "{text}");
        assert!(text.contains("  promotion header 2: 3 9\n"), "{text}");
        assert!(text.contains("  notification senders 1: 4\n"), "{text}");
        assert!(text.contains("left in the inbox 1\n"), "{text}");
    }

    #[test]
    fn a_hold_is_counted_by_its_rule_and_wins_over_a_reason() {
        let mut tally = Tally::new();
        let mut held = filed(ReasonKind::Promotion, Layer::Header);
        held.hold = Some(RuleName("Ledger".into()));
        tally.filing(id(7), &held);

        let text = tally.render();

        assert!(text.contains("held 1\n  Ledger 1: 7\n"), "{text}");
        assert!(text.contains("filed 0\n"), "{text}");
    }

    #[test]
    fn the_guard_and_the_markers_are_counted_by_kind_and_source() {
        let mut tally = Tally::new();
        tally.body(id(1), Asked::Neither, None);
        tally.body(id(2), Asked::Person, None);
        tally.body(id(3), Asked::Person, Some(&Outcome::default()));
        tally.body(
            id(4),
            Asked::Person,
            Some(&marked(MarkerKind::Question, MarkedBy::Detector)),
        );
        tally.body(
            id(6),
            Asked::Person,
            Some(&marked(MarkerKind::Todo, MarkedBy::Model)),
        );
        tally.body(
            id(8),
            Asked::Notice,
            Some(&marked(MarkerKind::Todo, MarkedBy::Notice)),
        );

        let text = tally.render();

        assert!(text.contains("considered 4: 2 3 4 6\n"), "{text}");
        assert!(text.contains("not considered 1\n"), "{text}");
        assert!(text.contains("no body here 1\n"), "{text}");
        assert!(text.contains("noticed 1: 8\n"), "{text}");
        assert!(text.contains("marked 3\n"), "{text}");
        assert!(text.contains("  todo notice 1: 8\n"), "{text}");
        assert!(text.contains("  question detector 1: 4\n"), "{text}");
        assert!(text.contains("  todo model 1: 6\n"), "{text}");
    }

    #[test]
    fn a_source_name_never_prints() {
        let mut tally = Tally::new();
        tally.filing(id(3), &filed(ReasonKind::Promotion, Layer::Header));

        assert!(!tally.render().contains("Ledger Weekly"));
    }
}
