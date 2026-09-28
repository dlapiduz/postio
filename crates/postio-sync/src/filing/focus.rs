//! Focus's filing pass (spec 007 T102, `contracts/engine.md`): each arrival
//! classified in the transaction that filed it, and what the classifier
//! decided carried out there, through the storage verbs that take the
//! caller's transaction.
//!
//! # What it does with a decision
//!
//! - **Hold** (FR-120 to FR-122): the message is held under its digest
//!   rule, and stays filed in the inbox, where the other apps and devices
//!   see it; Focus's own inbox leaves it out until its digest is delivered
//!   (`digest_holds`). A rule the user wrote wins over a reason Postio
//!   guessed, so held mail is never also filtered.
//! - **Filter** (FR-110 to FR-117): the decision is recorded with its
//!   reason, its source and the layer that decided, and the message is
//!   archived with the server's move queued, exactly as `a` archives it
//!   (`postio_storage::actions::relocate`). An account with no Archive
//!   folder keeps the mail in its inbox: filtered mail is archived mail,
//!   and a decision on a message still in the inbox would say one thing
//!   while the list said another.
//!
//! # What it costs
//!
//! At most four statements per arrival, plus its writes, and none walks the
//! mail (`contracts/engine.md`). A message no rule would act on costs no
//! read at all ([`super::facts`]). One a rule would act on costs the guard
//! questions its classifier asks: which senders the person wrote to (one
//! seek per message, whatever the number of senders), whether they took part
//! in the conversation (one `EXISTS`), and, once per call, the person's own
//! addresses and the account's Archive folder.

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

/// What decides, for [`FocusFiling`]: a classifier's filing stage.
///
/// [`BuiltIn`] is Postio's own, `postio_classify`'s layers. The seam is
/// here so the pass can be proven apart from the rules, and so milestone 2's
/// model can take a turn without the pass changing.
///
/// **An implementation must be monotone in its facts**: answering `true` to
/// a question can only keep mail where it is. Every fact is a guard, a
/// reason *not* to act (`Facts`), and the pass leans on it: when a
/// classifier acts on nothing while every unread fact is "no", no read is
/// made.
pub trait Classifier: Send + Sync + std::fmt::Debug {
    /// What to do with `message`, given `facts`.
    fn at_filing(&self, message: &FiledMessage<'_>, facts: &dyn Facts) -> Outcome;
}

/// Postio's own classifier: `postio_classify`'s layers, over the automated
/// senders Postio ships and the user's digest rules.
#[derive(Debug, Clone, Default)]
pub struct BuiltIn {
    digests: Digests,
}

impl BuiltIn {
    /// The built-in layers, holding what `[[focus.digests]]` in `config`
    /// asks: the rules that apply, in the file's order (contracts/config.md),
    /// read as of today.
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
    /// The automated-senders table, as data (FR-114).
    fn senders(&self) -> &Senders {
        Senders::shipped()
    }

    fn digests(&self) -> &Digests {
        &self.digests
    }
}

/// Focus's filing pass: what a host in Focus mode hands its engines.
#[derive(Debug, Clone)]
pub struct FocusFiling {
    classifier: Arc<dyn Classifier>,
    /// `[focus] filtering`: whether mail is filed away at all (FR-119).
    filtering: bool,
    /// `[focus.filter]`: the senders never filed away (FR-111, FR-116).
    filter: FocusFilter,
}

impl Default for FocusFiling {
    /// Focus's own: the built-in classifier, with `[focus]` as it is when
    /// nothing is written -- filtering on, nobody pinned, no digests.
    fn default() -> Self {
        FocusFiling::with_classifier(Arc::new(BuiltIn::default()))
    }
}

impl FocusFiling {
    /// Focus's own pass, as `config` sets it: the built-in classifier with
    /// its digest rules, and whether and whom to file away.
    pub fn from_config(config: &FocusConfig) -> Self {
        FocusFiling::with_classifier(Arc::new(BuiltIn::from_config(config))).configured(config)
    }

    /// A pass that decides with `classifier`, with `[focus]` at its
    /// defaults.
    pub fn with_classifier(classifier: Arc<dyn Classifier>) -> Self {
        FocusFiling {
            classifier,
            filtering: true,
            filter: FocusFilter::default(),
        }
    }

    /// This pass, doing what `config` says of filtering: whether to file
    /// anything away, and whom never to.
    pub fn configured(mut self, config: &FocusConfig) -> Self {
        self.filtering = config.filtering;
        self.filter = config.filter.clone();
        self
    }

    /// Whether `outcome` asks this pass to do anything: a digest to hold the
    /// message for, or a reason to file it away while filtering is on.
    fn acts(&self, outcome: &Outcome) -> bool {
        outcome.hold.is_some() || (self.filtering && outcome.filter.is_some())
    }

    /// The SQL of every read the pass can issue, so a test can ask the
    /// planner about each (`contracts/engine.md`: no scans).
    pub fn reads() -> Vec<String> {
        vec![
            IdentityRepository::explain_own_addresses().to_owned(),
            CorrespondentRepository::explain_written_to(1),
            ThreadRepository::explain_took_part().to_owned(),
            MailboxRepository::explain_by_role(),
            ReminderRepository::explain_standing_on(2),
        ]
    }

    /// What the classifier decides for `filed`, reading from the store only
    /// the facts it asks for, each at most once.
    ///
    /// `own` is the person's own domains, read by the first arrival in the
    /// call that needs them and shared by the rest.
    async fn decide(
        &self,
        transaction: &Connection,
        filed: &FiledMessage<'_>,
        own: &mut Option<Arc<BTreeSet<String>>>,
    ) -> Result<Outcome, SyncError> {
        let never = |address: &EmailAddress| self.filter.never_filters(address);
        let senders = &filed.message.from;
        let mut known = Known::new(senders, filed.thread, &never);
        // Nothing acts while every unread guard says "no": then nothing
        // acts at all, and there is nothing to read (`super::facts`).
        let optimistic = self.classifier.at_filing(filed, known.assuming(false));
        if !self.acts(&optimistic) {
            return Ok(optimistic);
        }
        // Asked again, with doubt for what is unread, until it asks nothing
        // unread. Each question is read once, so this turns at most once
        // per question.
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

/// The domains the person sends from, lowercased: every account's address
/// and every identity's.
async fn own_domains(transaction: &Connection) -> Result<BTreeSet<String>, SyncError> {
    Ok(IdentityRepository::new(transaction)
        .own_addresses()
        .await?
        .iter()
        .filter_map(EmailAddress::domain)
        .map(str::to_ascii_lowercase)
        .collect())
}

/// Which of `senders` the person wrote to, normalised.
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

/// What the pass decided for a call's messages, read and decided with
/// nothing written yet.
#[derive(Default)]
struct Plan<'m> {
    /// Held for a digest rule: the message, and the rule.
    holds: Vec<(MessageId, postio_classify::RuleName)>,
    /// Filed away: the message, why, and its account's Archive.
    filings: Vec<(
        &'m postio_model::Message,
        postio_classify::Reason,
        MailboxId,
    )>,
}

impl FocusFiling {
    /// Focus's pass as a sweep of the inbox runs it (spec 007 FR-118):
    /// `config`'s filtering, with every guard, and no digest rule -- a rule
    /// holds what arrives after it, not what is already in the inbox.
    pub fn sweeping(config: &FocusConfig) -> Self {
        let mut config = config.clone();
        config.digests.clear();
        FocusFiling::from_config(&config)
    }

    /// Which of `filed` this pass would file away, and nothing written: what
    /// a sweep's preview counts. It decides exactly as [`Self::file_away`]
    /// does, so the count is what the sweep then moves.
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

    /// File `filed` away by this pass's reasons, in `transaction`, and hold
    /// nothing and answer no reminder: what a sweep of mail already in the
    /// inbox does, where [`FilingPass::file`] is for arrivals.
    pub async fn file_away(
        &self,
        transaction: &Connection,
        filed: &[FiledMessage<'_>],
    ) -> Result<FilingEffects, SyncError> {
        let plan = self.plan(transaction, filed).await?;
        self.carry_out(transaction, plan, false, Utc::now()).await
    }

    /// What this pass would do with `filed`: read from the store only what
    /// the classifier asks, each at most once, and decide.
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
            // A rule the user wrote wins over a reason Postio guessed: held
            // mail stays filed in the inbox, out of Focus's own (FR-121).
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

    /// Write what `plan` decided, as of `now`: its holds when `holding`,
    /// and its filings -- each decision with its reason, and the archive
    /// with the server's move queued, one move per folder left, as a
    /// multi-select archive is.
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
        for (message, reason, archive) in plan.filings {
            FilterDecisionRepository::new(transaction)
                .record(&FilterDecision {
                    message: message.id,
                    reason: stored_reason(reason.kind),
                    source: reason.source.as_ref().map(|name| name.as_str().to_owned()),
                    layer: stored_layer(reason.layer),
                    decided_at: now,
                })
                .await?;
            leaving
                .entry((message.account_id, archive))
                .or_default()
                .entry(message.mailbox_id)
                .or_default()
                .push(message.id);
            effects.filtered.push(message.id);
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

/// Whether `message` can answer a reminder: a reply -- it names what it
/// answers, by `In-Reply-To` or `References` -- filed somewhere other than
/// the person's own folders, and not in Junk, where a stranger's mail
/// that threads in by its subject would otherwise count as an answer.
///
/// A message that names nothing starts a conversation of its own, and no
/// reminder can stand on that, so a notification costs no read. A reply
/// threaded by its subject alone is not missed: the due timer asks who
/// has written in the conversation before it fires a reminder.
fn may_answer(message: &FiledMessage<'_>) -> bool {
    let replies = message.message.in_reply_to.is_some() || !message.message.references.is_empty();
    replies
        && !matches!(
            message.role,
            MailboxRole::Sent | MailboxRole::Drafts | MailboxRole::Outbox | MailboxRole::Junk
        )
}

/// A reply from somebody else cancels the reminder waiting on it, and
/// settles one that had already surfaced (spec 007 FR-044): the person was
/// waiting to hear, and has.
///
/// One read per call, and only when a reply arrived: whether a reminder
/// stands on any of the conversations the replies joined. Only when one
/// does is the person's own list of addresses read, to tell their own
/// follow-up -- which leaves the reminder waiting -- from somebody else's
/// answer.
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

/// A classifier's reason as the store spells it (`filter_decisions`). One
/// vocabulary, held in two crates so the store never depends on what
/// decides; this is where they meet, and a test holds them equal.
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

/// A classifier's layer as the store spells it.
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
        // One vocabulary (FR-113), two crates: every reason the classifier
        // can give has a stored spelling, the same one, and every stored
        // reason is one the classifier can give.
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
        // The CHECK on `filter_decisions` is the vocabulary's last word: a
        // reason it refuses is never stored. Each of the classifier's, as
        // the pass records it, is kept and read back as itself.
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
