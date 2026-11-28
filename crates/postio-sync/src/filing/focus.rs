//! Focus's filing pass (spec 007 T102): classifies each arrival in the transaction that files it,
//! then holds it under its digest or archives it with the move queued.
//! A user-written rule wins over a guessed reason. With no Archive folder, filtered mail stays in
//! the inbox so the decision and the list agree.
//! At most four statements per arrival; a message no rule would act on costs no read.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use chrono::{Local, Utc};
use postio_classify::{Digests, Facts, Layer, Outcome, ReasonKind, Rules, Senders};
use postio_config::{FocusConfig, FocusFilter};
use postio_model::{AccountId, EmailAddress, MailboxId, MailboxRole, MessageId, ThreadId};
use postio_storage::Connection;
use postio_storage::actions::{self, Relocation};
use postio_storage::repository::{
    CorrespondentRepository, DigestRepository, FilterDecision, FilterDecisionRepository,
    FilterLayer, FilterReason, IdentityRepository, MailboxRepository, ReminderRepository,
    ThreadRepository,
};

use super::facts::Known;
use super::{FiledMessage, FilingEffects, FilingPass};
use crate::drain::SyncError;

/// A classifier's filing stage; `BuiltIn` is Postio's own.
/// An implementation must be monotone in its facts: answering `true` to a question can only keep
/// mail where it is. The pass relies on it to skip reads.
pub trait Classifier: Send + Sync + std::fmt::Debug {
    fn at_filing(&self, message: &FiledMessage<'_>, facts: &dyn Facts) -> Outcome;
}

#[derive(Debug, Clone, Default)]
pub struct BuiltIn {
    digests: Digests,
}

impl BuiltIn {
    pub fn from_config(config: &FocusConfig) -> Self {
        let applicable = config.applicable_digests();
        BuiltIn {
            digests: Digests::new(
                applicable
                    .iter()
                    .map(|(rule, _)| (rule.name.as_str(), rule.queries.as_slice())),
                Local::now().date_naive(),
            ),
        }
    }
}

impl Classifier for BuiltIn {
    fn at_filing(&self, message: &FiledMessage<'_>, facts: &dyn Facts) -> Outcome {
        postio_classify::at_filing(message, facts, self)
    }
}

impl Rules for BuiltIn {
    fn senders(&self) -> &Senders {
        Senders::shipped()
    }

    fn digests(&self) -> &Digests {
        &self.digests
    }
}

#[derive(Debug, Clone)]
pub struct FocusFiling {
    classifier: Arc<dyn Classifier>,
    filtering: bool,
    filter: FocusFilter,
}

impl Default for FocusFiling {
    fn default() -> Self {
        FocusFiling::with_classifier(Arc::new(BuiltIn::default()))
    }
}

impl FocusFiling {
    pub fn from_config(config: &FocusConfig) -> Self {
        FocusFiling::with_classifier(Arc::new(BuiltIn::from_config(config))).configured(config)
    }

    pub fn with_classifier(classifier: Arc<dyn Classifier>) -> Self {
        FocusFiling {
            classifier,
            filtering: true,
            filter: FocusFilter::default(),
        }
    }

    pub fn configured(mut self, config: &FocusConfig) -> Self {
        self.filtering = config.filtering;
        self.filter = config.filter.clone();
        self
    }

    fn acts(&self, outcome: &Outcome) -> bool {
        outcome.hold.is_some() || (self.filtering && outcome.filter.is_some())
    }

    pub fn reads() -> Vec<String> {
        vec![
            IdentityRepository::explain_own_addresses().to_owned(),
            CorrespondentRepository::explain_written_to(1),
            ThreadRepository::explain_took_part().to_owned(),
            MailboxRepository::explain_by_role(),
            ReminderRepository::explain_standing_on(2),
        ]
    }

    /// `own` is the person's own domains, read by the first arrival that needs them and shared by
    /// the rest.
    async fn decide(
        &self,
        transaction: &Connection,
        filed: &FiledMessage<'_>,
        own: &mut Option<Arc<BTreeSet<String>>>,
    ) -> Result<Outcome, SyncError> {
        let never = |address: &EmailAddress| self.filter.never_filters(address);
        let senders = &filed.message.from;
        let mut known = Known::new(senders, filed.thread, &never);
        let optimistic = self.classifier.at_filing(filed, known.assuming(false));
        if !self.acts(&optimistic) {
            return Ok(optimistic);
        }
        loop {
            let outcome = self.classifier.at_filing(filed, known.assuming(true));
            let wanted = known.wanted();
            if wanted.is_empty() {
                return Ok(outcome);
            }
            if wanted.own_domain {
                if own.is_none() {
                    *own = Some(Arc::new(own_domains(transaction).await?));
                }
                known.own_domains = own.clone();
            }
            if wanted.wrote_to {
                known.written = Some(written_to(transaction, senders).await?);
            }
            if wanted.took_part
                && let Some(thread) = filed.thread
            {
                known.took_part = Some(ThreadRepository::new(transaction).took_part(thread).await?);
            }
        }
    }
}

async fn own_domains(transaction: &Connection) -> Result<BTreeSet<String>, SyncError> {
    Ok(IdentityRepository::new(transaction)
        .own_addresses()
        .await?
        .iter()
        .filter_map(EmailAddress::domain)
        .map(str::to_ascii_lowercase)
        .collect())
}

async fn written_to(
    transaction: &Connection,
    senders: &[EmailAddress],
) -> Result<BTreeSet<String>, SyncError> {
    Ok(CorrespondentRepository::new(transaction)
        .written_to(senders)
        .await?
        .into_iter()
        .collect())
}

#[derive(Default)]
struct Plan<'m> {
    holds: Vec<(MessageId, postio_classify::RuleName)>,
    filings: Vec<(
        &'m postio_model::Message,
        postio_classify::Reason,
        MailboxId,
    )>,
}

impl FocusFiling {
    /// A sweep of the inbox: filtering with every guard and no digest rule, since a rule holds
    /// later arrivals, not mail already there.
    pub fn sweeping(config: &FocusConfig) -> Self {
        let mut config = config.clone();
        config.digests.clear();
        FocusFiling::from_config(&config)
    }

    /// Which of `filed` this pass would file away, writing nothing; decides exactly as `file_away`,
    /// so the preview count is what the sweep moves.
    pub async fn would_file_away(
        &self,
        connection: &Connection,
        filed: &[FiledMessage<'_>],
    ) -> Result<Vec<MessageId>, SyncError> {
        let plan = self.plan(connection, filed).await?;
        Ok(plan
            .filings
            .iter()
            .map(|(message, _, _)| message.id)
            .collect())
    }

    pub async fn outcomes(
        &self,
        connection: &Connection,
        filed: &[FiledMessage<'_>],
    ) -> Result<Vec<Outcome>, SyncError> {
        let mut own = None;
        let mut outcomes = Vec::with_capacity(filed.len());
        for message in filed {
            outcomes.push(self.decide(connection, message, &mut own).await?);
        }
        Ok(outcomes)
    }

    pub async fn file_away(
        &self,
        transaction: &Connection,
        filed: &[FiledMessage<'_>],
    ) -> Result<FilingEffects, SyncError> {
        let plan = self.plan(transaction, filed).await?;
        self.carry_out(transaction, plan, false, Utc::now()).await
    }

    async fn plan<'m>(
        &self,
        transaction: &Connection,
        filed: &[FiledMessage<'m>],
    ) -> Result<Plan<'m>, SyncError> {
        let mut own: Option<Arc<BTreeSet<String>>> = None;
        let mut archives: BTreeMap<AccountId, Option<MailboxId>> = BTreeMap::new();
        let mut plan = Plan::default();
        for message in filed {
            let outcome = self.decide(transaction, message, &mut own).await?;
            // A user-written rule wins over a guessed reason (FR-121).
            if let Some(rule) = outcome.hold {
                plan.holds.push((message.message.id, rule));
                continue;
            }
            let Some(reason) = outcome.filter.filter(|_| self.filtering) else {
                continue;
            };
            let account = message.message.account_id;
            let archive = match archives.get(&account) {
                Some(archive) => *archive,
                None => {
                    let archive = MailboxRepository::new(transaction)
                        .by_role(account, MailboxRole::Archive)
                        .await?
                        .map(|mailbox| mailbox.id);
                    archives.insert(account, archive);
                    archive
                }
            };
            let Some(archive) = archive else {
                continue;
            };
            if message.message.mailbox_id == archive {
                continue;
            }
            plan.filings.push((message.message, reason, archive));
        }
        Ok(plan)
    }

    /// One move is queued per folder left, as a multi-select archive does.
    async fn carry_out(
        &self,
        transaction: &Connection,
        plan: Plan<'_>,
        holding: bool,
        now: chrono::DateTime<Utc>,
    ) -> Result<FilingEffects, SyncError> {
        let mut effects = FilingEffects::default();
        if holding {
            for (message, rule) in plan.holds {
                DigestRepository::new(transaction)
                    .hold(message, rule.as_str(), now)
                    .await?;
                effects.held.push(message);
            }
        }
        let mut leaving: BTreeMap<(AccountId, MailboxId), BTreeMap<MailboxId, Vec<MessageId>>> =
            BTreeMap::new();
        let mut marking: BTreeMap<(AccountId, FilterReason), Vec<&postio_model::Message>> =
            BTreeMap::new();
        for (message, reason, archive) in plan.filings {
            let stored = stored_reason(reason.kind);
            FilterDecisionRepository::new(transaction)
                .record(&FilterDecision {
                    message: message.id,
                    reason: stored,
                    source: reason.source.as_ref().map(|name| name.as_str().to_owned()),
                    layer: stored_layer(reason.layer),
                    decided_at: now,
                })
                .await?;
            marking
                .entry((message.account_id, stored))
                .or_default()
                .push(message);
            leaving
                .entry((message.account_id, archive))
                .or_default()
                .entry(message.mailbox_id)
                .or_default()
                .push(message.id);
            effects.filtered.push(message.id);
        }
        // Queued before the moves so the server moves messages that already say why (#2076).
        for ((account, reason), messages) in &marking {
            actions::mark_filtered(transaction, *account, messages, Some(*reason), now).await?;
        }
        for ((account, archive), by_source) in &leaving {
            actions::relocate(
                transaction,
                *account,
                by_source,
                *archive,
                Relocation::Move,
                now,
            )
            .await?;
        }
        Ok(effects)
    }
}

#[async_trait::async_trait]
impl FilingPass for FocusFiling {
    async fn file(
        &self,
        transaction: &Connection,
        filed: &[FiledMessage<'_>],
    ) -> Result<FilingEffects, SyncError> {
        let now = Utc::now();
        let plan = self.plan(transaction, filed).await?;
        let effects = self.carry_out(transaction, plan, true, now).await?;
        answer_reminders(transaction, filed, now).await?;
        if !effects.filtered.is_empty() || !effects.held.is_empty() {
            tracing::debug!(
                arrivals = filed.len(),
                filtered = effects.filtered.len(),
                held = effects.held.len(),
                "Focus filed arrivals"
            );
        }
        Ok(effects)
    }
}

/// A reply names what it answers (`In-Reply-To` or `References`), is not in the person's own
/// folders and not in Junk, where a stranger's mail threaded by subject would count.
/// A subject-threaded reply is not missed: the due timer asks before it fires.
fn may_answer(message: &FiledMessage<'_>) -> bool {
    let replies = message.message.in_reply_to.is_some() || !message.message.references.is_empty();
    replies
        && !matches!(
            message.role,
            MailboxRole::Sent | MailboxRole::Drafts | MailboxRole::Outbox | MailboxRole::Junk
        )
}

/// A reply from somebody else cancels the reminder waiting on it, and settles one that had surfaced
/// (spec 007 FR-044).
/// One read per call, and the person's own addresses only when a reminder stands, to tell their own
/// follow-up from an answer.
async fn answer_reminders(
    transaction: &Connection,
    filed: &[FiledMessage<'_>],
    now: chrono::DateTime<Utc>,
) -> Result<(), SyncError> {
    let mut threads: Vec<ThreadId> = filed
        .iter()
        .filter(|message| may_answer(message))
        .filter_map(|message| message.thread)
        .collect();
    threads.sort_unstable();
    threads.dedup();
    if threads.is_empty() {
        return Ok(());
    }
    let reminders = ReminderRepository::new(transaction);
    let standing = reminders.standing_on(&threads).await?;
    if standing.is_empty() {
        return Ok(());
    }
    let own: BTreeSet<String> = IdentityRepository::new(transaction)
        .own_addresses()
        .await?
        .iter()
        .map(EmailAddress::normalized)
        .collect();
    for reminder in standing {
        let replied = filed.iter().any(|message| {
            message.thread == Some(reminder.thread)
                && may_answer(message)
                && message
                    .message
                    .from
                    .iter()
                    .any(|from| !own.contains(&from.normalized()))
        });
        if !replied {
            continue;
        }
        if reminder.fired_at.is_some() {
            reminders.settle(reminder.id, now).await?;
        } else {
            reminders.cancel(reminder.id, now).await?;
        }
    }
    Ok(())
}

/// A classifier's reason as the store spells it (`filter_decisions`); one vocabulary in two crates,
/// held equal by a test.
pub(crate) fn stored_reason(kind: ReasonKind) -> FilterReason {
    match kind {
        ReasonKind::Spam => FilterReason::Spam,
        ReasonKind::Promotion => FilterReason::Promotion,
        ReasonKind::Notification => FilterReason::Notification,
        ReasonKind::Receipt => FilterReason::Receipt,
        ReasonKind::Shipping => FilterReason::Shipping,
        ReasonKind::Social => FilterReason::Social,
    }
}

pub(crate) fn stored_layer(layer: Layer) -> FilterLayer {
    match layer {
        Layer::Header => FilterLayer::Header,
        Layer::Senders => FilterLayer::Senders,
        Layer::Server => FilterLayer::Server,
        Layer::Model => FilterLayer::Model,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_store_spells_every_reason_the_classifier_gives_as_the_classifier_does() {
        for kind in ReasonKind::ALL {
            assert_eq!(stored_reason(kind).as_str(), kind.as_str(), "{kind:?}");
        }
        let mut reached: Vec<&str> = ReasonKind::ALL
            .into_iter()
            .map(|kind| stored_reason(kind).as_str())
            .collect();
        reached.sort_unstable();
        let mut stored: Vec<&str> = FilterReason::ALL
            .into_iter()
            .map(FilterReason::as_str)
            .collect();
        stored.sort_unstable();
        assert_eq!(reached, stored);
    }

    #[test]
    fn the_store_spells_every_layer_the_classifier_names_as_the_classifier_does() {
        for layer in Layer::ALL {
            assert_eq!(stored_layer(layer).as_str(), layer.as_str(), "{layer:?}");
        }
        let mut reached: Vec<&str> = Layer::ALL
            .into_iter()
            .map(|layer| stored_layer(layer).as_str())
            .collect();
        reached.sort_unstable();
        let mut stored: Vec<&str> = FilterLayer::ALL
            .into_iter()
            .map(FilterLayer::as_str)
            .collect();
        stored.sort_unstable();
        assert_eq!(reached, stored);
    }

    #[tokio::test]
    async fn every_reason_the_classifier_gives_is_one_the_store_keeps() {
        let database = postio_storage::test_support::memory().await;
        let connection = database.connect().await.expect("checkout");
        let (account, inbox) = postio_storage::test_support::account_with_inbox(&connection).await;
        let decisions = FilterDecisionRepository::new(&connection);
        for kind in ReasonKind::ALL {
            let mut message = postio_model::Message::new(account.id, inbox, Utc::now());
            let id = postio_storage::repository::MessageRepository::new(&connection)
                .create(&mut message)
                .await
                .expect("a message");
            decisions
                .record(&FilterDecision {
                    message: id,
                    reason: stored_reason(kind),
                    source: None,
                    layer: stored_layer(Layer::Header),
                    decided_at: Utc::now(),
                })
                .await
                .unwrap_or_else(|error| panic!("{kind:?} was refused: {error}"));
            let kept = decisions.get(id).await.expect("a read").expect("kept");
            assert_eq!(kept.reason.as_str(), kind.as_str());
        }
    }
}
