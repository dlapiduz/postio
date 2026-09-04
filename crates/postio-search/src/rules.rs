//! When each rule can be answered, and which rules answer at a given point.
//!
//! ADR 0008 Q3. Postio syncs headers newest-first and backfills bodies
//! lazily — `BodyState` exists precisely because a message is listed,
//! threaded and header-searchable long before its body is local — so a rule
//! containing `body:` *cannot* be evaluated when the message arrives. Both
//! tempting answers are wrong: fetching the body eagerly throws away the
//! backfill design and makes first sync slow on a large mailbox, and
//! evaluating against an absent body silently makes `body:invoice` false and
//! files mail in the wrong place.
//!
//! So each rule declares nothing and the engine derives what it needs.
//! [`Stage`] is that derivation, computed from the fields a query actually
//! uses by [`needs_body`](crate::needs_body), and a rule is evaluated at
//! exactly one of the two points.
//!
//! # Exactly once, by construction
//!
//! "A message is evaluated against a given rule exactly once" is not
//! bookkeeping here, it is the shape: [`Stage`] is one value per rule, the
//! two points ask for disjoint stages, and each point runs once per message.
//! There is no table of what has already been evaluated because there is
//! nothing for one to prevent. What the *callers* have to get right is the
//! other half — arrival fires for a message that was inserted rather than
//! re-seen, and the body point fires when a body first becomes local — and
//! that is asserted where those calls are.
//!
//! # What this does not do
//!
//! It does not carry actions out, and it does not honour `stop`: both are
//! #481, which owns the action vocabulary and says `stop` halts evaluation
//! "on that pass". This answers only which rules the pass should consider,
//! in the order the file lists them.

use chrono::NaiveDate;
use postio_model::rule::Rule;

use crate::matcher::{Subject, matches, needs_body};
use crate::query::ParsedQuery;

/// Which of the two evaluation points a rule belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    /// Answerable from the headers alone, so it runs in the sync pass that
    /// inserts the message — before the user ever sees it in the Inbox.
    OnArrival,
    /// Touches the body, so it runs when that message's backfill completes.
    /// The message is in the Inbox in between, which is honest: it *is* in
    /// the Inbox until Postio knows enough to move it.
    OnBody,
}

/// One rule with its query parsed and its stage derived.
#[derive(Debug, Clone)]
pub struct Staged {
    /// The rule as configured.
    pub rule: Rule,
    /// Where it can be answered.
    pub stage: Stage,
    query: ParsedQuery,
}

impl Staged {
    /// The parsed query, for a caller that wants to explain the rule.
    pub fn query(&self) -> &ParsedQuery {
        &self.query
    }
}

/// Every configured rule, parsed once and filed by stage.
///
/// Parsed once because a rule's query is fixed for as long as the config is:
/// re-parsing per message would be per *message*, on the sync pass's hot
/// path, for a string that has not changed.
#[derive(Debug, Clone, Default)]
pub struct RuleSet {
    staged: Vec<Staged>,
}

impl RuleSet {
    /// Parse and classify `rules`, keeping the file's order.
    ///
    /// Disabled rules are dropped here rather than skipped at every
    /// evaluation: `enabled = false` is how a rule is turned off, and a
    /// caller that has to remember to check it is a caller that eventually
    /// does not.
    pub fn compile(rules: &[Rule], today: NaiveDate) -> Self {
        let staged = rules
            .iter()
            .filter(|rule| rule.enabled)
            .map(|rule| {
                let query = crate::parse(&rule.query, today);
                let stage = if needs_body(&query) {
                    Stage::OnBody
                } else {
                    Stage::OnArrival
                };
                Staged {
                    rule: rule.clone(),
                    stage,
                    query,
                }
            })
            .collect();
        RuleSet { staged }
    }

    /// Whether any rule at all runs at `stage`.
    ///
    /// What the sync pass asks before doing any work: with no body-requiring
    /// rules configured, the backfill point should not read a body back out
    /// of the store to match nothing against.
    pub fn has(&self, stage: Stage) -> bool {
        self.staged.iter().any(|staged| staged.stage == stage)
    }

    /// Every rule, in file order.
    pub fn rules(&self) -> &[Staged] {
        &self.staged
    }

    /// The rules that run at `stage` and match `subject`, in file order.
    pub fn matching<'a>(&'a self, stage: Stage, subject: &Subject<'_>) -> Vec<&'a Rule> {
        self.staged
            .iter()
            .filter(|staged| staged.stage == stage)
            .filter(|staged| matches(&staged.query, subject))
            .map(|staged| &staged.rule)
            .collect()
    }
}
