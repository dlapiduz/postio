//! Focus's filing pass (spec 007 T102, `contracts/engine.md`): each arrival
//! classified in the transaction that filed it, and what the classifier
//! decided carried out there, through the storage verbs that take the
//! caller's transaction.
//!
//! # What it does with a decision
//!
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

use chrono::Utc;
use postio_classify::{Facts, Layer, Outcome, ReasonKind, Rules, Senders};
use postio_config::{FocusConfig, FocusFilter};
use postio_model::{AccountId, EmailAddress, MailboxId, MailboxRole, MessageId};
use postio_storage::Connection;
use postio_storage::actions::{self, Relocation};
use postio_storage::repository::{
    CorrespondentRepository, FilterDecision, FilterDecisionRepository, FilterLayer, FilterReason,
    IdentityRepository, MailboxRepository, ThreadRepository,
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
/// senders Postio ships.
#[derive(Debug, Clone, Copy, Default)]
pub struct BuiltIn;

impl Classifier for BuiltIn {
    fn at_filing(&self, message: &FiledMessage<'_>, facts: &dyn Facts) -> Outcome {
        postio_classify::at_filing(message, facts, &Shipped)
    }
}

/// The rules as shipped: the automated-senders table, as data.
struct Shipped;

impl Rules for Shipped {
    fn senders(&self) -> &Senders {
        Senders::shipped()
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
    /// nothing is written -- filtering on, nobody pinned.
    fn default() -> Self {
        FocusFiling::with_classifier(Arc::new(BuiltIn))
    }
}

impl FocusFiling {
    /// Focus's own pass, as `config` sets it.
    pub fn from_config(config: &FocusConfig) -> Self {
        FocusFiling::default().configured(config)
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

    /// Whether `outcome` asks this pass to do anything: a reason to file
    /// the message away, while filtering is on.
    fn acts(&self, outcome: &Outcome) -> bool {
        self.filtering && outcome.filter.is_some()
    }

    /// The SQL of every read the pass can issue, so a test can ask the
    /// planner about each (`contracts/engine.md`: no scans).
    pub fn reads() -> Vec<String> {
        vec![
            IdentityRepository::explain_own_addresses().to_owned(),
            CorrespondentRepository::explain_written_to(1),
            ThreadRepository::explain_took_part().to_owned(),
            MailboxRepository::explain_by_role(),
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

#[async_trait::async_trait]
impl FilingPass for FocusFiling {
    async fn file(
        &self,
        transaction: &Connection,
        filed: &[FiledMessage<'_>],
    ) -> Result<FilingEffects, SyncError> {
        let now = Utc::now();
        let mut own: Option<Arc<BTreeSet<String>>> = None;
        let mut archives: BTreeMap<AccountId, Option<MailboxId>> = BTreeMap::new();
        // Filtered arrivals by account and the folder they leave: one move
        // per folder, as a multi-select archive is.
        let mut leaving: BTreeMap<AccountId, BTreeMap<MailboxId, Vec<MessageId>>> = BTreeMap::new();
        let mut effects = FilingEffects::default();

        for message in filed {
            let outcome = self.decide(transaction, message, &mut own).await?;
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
            FilterDecisionRepository::new(transaction)
                .record(&FilterDecision {
                    message: message.message.id,
                    reason: stored_reason(reason.kind),
                    source: reason.source.as_ref().map(|name| name.as_str().to_owned()),
                    layer: stored_layer(reason.layer),
                    decided_at: now,
                })
                .await?;
            leaving
                .entry(account)
                .or_default()
                .entry(message.message.mailbox_id)
                .or_default()
                .push(message.message.id);
            effects.filtered.push(message.message.id);
        }

        for (account, by_source) in &leaving {
            let Some(Some(archive)) = archives.get(account) else {
                continue;
            };
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
        if !effects.filtered.is_empty() {
            tracing::debug!(
                arrivals = filed.len(),
                filtered = effects.filtered.len(),
                "Focus filed arrivals away"
            );
        }
        Ok(effects)
    }
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
