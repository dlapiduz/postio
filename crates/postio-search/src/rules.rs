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
//! [`Stage`] is that derivation, and a rule is evaluated at exactly one of
//! the two points.
//!
//! It is derived from *both* halves of a rule (ADR 0030): the fields the
//! query uses, by [`needs_body`](crate::needs_body), and what the actions
//! need to be carried out, by
//! [`Action::needs_body_to_run`](postio_model::rule::Action::needs_body_to_run).
//! `forward:` is the first action with a requirement of its own — it sends
//! the message on, so it needs the message — and a rule carrying one is
//! `OnBody` however header-answerable its query reads. The stage is the later
//! of the two, and it is the rule's as a whole: the actions never split
//! across the points.
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

use postio_model::rule::Action;

use crate::matcher::{Subject, matches, needs_body};
use crate::query::ParsedQuery;

/// The point a requirement puts a rule at: the body point if it needs a body,
/// the arrival point if it does not.
fn stage_for(needs_body: bool) -> Stage {
    if needs_body {
        Stage::OnBody
    } else {
        Stage::OnArrival
    }
}

/// Which of the two evaluation points a rule belongs to.
///
/// Ordered, because that is how a stage is derived: ADR 0030 makes it the
/// *later* of what the query needs and what the actions need, and `OnArrival`
/// is the earlier of the two.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
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
                // The later of the two requirements (ADR 0030). A rule stages
                // as a whole: its actions never split across the two points,
                // because `stop` would then mean two things for one rule and
                // a `forward:` would fire from a folder the same rule's
                // `move:` had already emptied.
                let stage = stage_for(needs_body(&query)).max(stage_for(
                    rule.actions.iter().any(Action::needs_body_to_run),
                ));
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

    /// The rules that run at `stage` and match `subject`, in file order, up
    /// to and including the first match carrying `stop`.
    ///
    /// `stop` is ADR 0008 Q4 and #481: a match carrying it halts the rules
    /// *below* it for this message. Its own actions still run — it is the
    /// rules after it that do not — so the stopping rule is included in what
    /// this answers rather than replacing it.
    ///
    /// Scoped to this `stage`, and to this call, which is to say to this
    /// message. The two evaluation points are separate passes at separate
    /// times (ADR 0008 Q3), so a `stop` among the header rules says nothing
    /// about the body rules that run when a backfill later completes; and
    /// nothing is remembered between calls, so a `stop` cannot leak onto the
    /// next message of a sync and disable the rule set for the rest of it.
    pub fn matching<'a>(&'a self, stage: Stage, subject: &Subject<'_>) -> Vec<&'a Rule> {
        let mut matched = Vec::new();
        for staged in self.staged.iter().filter(|staged| staged.stage == stage) {
            if !matches(&staged.query, subject) {
                continue;
            }
            matched.push(&staged.rule);
            if staged.rule.stop {
                break;
            }
        }
        matched
    }
}
