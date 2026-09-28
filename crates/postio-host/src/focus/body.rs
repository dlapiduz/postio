//! Focus's body stage (spec 007 T103, `contracts/engine.md`, "Focus mode in
//! the host"): what Focus does with a message once its body is here --
//! invitations from its calendar part, questions and to-dos from its own
//! words.
//!
//! # Two ways in
//!
//! - **As bodies land.** The stage hears every `BodyLoaded`, lets a burst
//!   finish -- a backfill announces bodies by the hundred -- and classifies
//!   the ones it names, as the body indexer does.
//! - **At start.** Mail whose body was here before Focus ran, or that
//!   another app brought in, is caught up on: every inbox's mail from the
//!   last 30 days with a body here and no record at this classifier's
//!   version, newest first, a batch at a time, in the background (FR-134,
//!   FR-141). A record left by an older classifier is not this one's, so a
//!   newer classifier runs it all again.
//!
//! Either way it reads only bodies already on this machine and never fetches
//! one to classify it, and it takes only recent inbox mail: FR-141's bound on
//! what the needs-action detector may read.
//!
//! # What it finds
//!
//! - **Questions and to-dos** (FR-104 to FR-106, T117), in mail sent
//!   directly to the user: the built-in detector reads the newest message's
//!   own words and the marker quotes its sentence verbatim, cut to a plain
//!   prefix of 200 characters. With no model configured nothing else is
//!   asked, and nothing connects anywhere. A message that asks something is
//!   not held for a digest: filing held it on its headers, and the stage
//!   lets it go while it still waits (FR-122).
//!
//! # Records
//!
//! Every message the stage takes is recorded at the classifier's version
//! (`focus_classified`), whatever came of it, so a message whose
//! classification failed is not taken again and again. A failure is logged
//! by ids and outcome, never by content.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use std::collections::HashMap;

use chrono::{DateTime, TimeDelta, Utc};
use postio_classify::{BodyMessage, Facts, FiledMessage, MarkerCandidate, OwnText, Rules, Senders};
use postio_model::listing::MarkerKind;
use postio_model::{
    AccountId, EmailAddress, Identity, MailboxRole, Message, MessageBody, MessageId, ThreadId,
};
use postio_storage::repository::{
    AccountRepository, DigestRepository, FocusClassifiedRepository, FocusStage, Marker,
    MarkerRepository, MarkerSource, MessageRepository, ThreadRepository,
};
use postio_storage::{Checkout, Store, WritePriority};

use crate::Inner;

/// How far back the stage reads: the inbox's last 30 days (FR-141).
const WINDOW: TimeDelta = TimeDelta::days(30);

/// How many messages one batch takes, holding one pooled connection.
const BATCH: u32 = 50;

/// How long a burst of `BodyLoaded` must be quiet before the stage reads
/// what it named: the body indexer's wait, for the same bursts.
const DEBOUNCE: Duration = Duration::from_millis(500);

/// The pause between two batches of the catch-up: it is background work,
/// and a person's own reads and writes go first.
const BREATHER: Duration = Duration::from_millis(20);

/// Why the stage could not read or record what it meant to.
type Failure = postio_storage::Error;

/// Start the body stage over `inner`'s store: the catch-up, then every body
/// that lands. `caught_up` is set once the catch-up has taken everything it
/// found.
///
/// Over a wiring whose events go to a single reader rather than a hub,
/// there is nothing to hear: the stage catches up and ends.
pub(super) fn spawn(inner: &Inner, caught_up: Arc<AtomicBool>) -> tokio::task::AbortHandle {
    let events = inner.hub.subscribe("focus:body-stage");
    let database = inner.wiring.database.clone();
    inner
        .runtime()
        .spawn(async move {
            let version = postio_classify::VERSION;
            if let Err(error) = catch_up(&database, version).await {
                tracing::warn!(%error, "Focus's body stage could not catch up: {error}");
            }
            caught_up.store(true, Ordering::Release);
            let Some(events) = events else {
                return;
            };
            loop {
                let Some(event) = events.next().await else {
                    return;
                };
                let mut landed = Vec::new();
                note(&mut landed, &event);
                let quiet = tokio::time::sleep(DEBOUNCE);
                tokio::pin!(quiet);
                loop {
                    tokio::select! {
                        () = &mut quiet => break,
                        next = events.next() => match next {
                            None => return,
                            Some(event) => note(&mut landed, &event),
                        },
                    }
                }
                if landed.is_empty() {
                    continue;
                }
                landed.sort_unstable();
                landed.dedup();
                if let Err(error) = landed_bodies(&database, &landed, version).await {
                    tracing::warn!(
                        bodies = landed.len(),
                        %error,
                        "Focus's body stage could not classify what landed: {error}"
                    );
                }
            }
        })
        .abort_handle()
}

/// Remember the message a `BodyLoaded` names; ignore every other event.
fn note(landed: &mut Vec<MessageId>, event: &postio_core::Event) {
    if let postio_core::Event::BodyLoaded { message, .. } = event {
        landed.push(*message);
    }
}

/// The oldest a message may be filed and still be the stage's to read.
fn since() -> DateTime<Utc> {
    Utc::now() - WINDOW
}

/// Classify every inbox's recent mail that has a body here and no record at
/// `version`, newest first, a batch at a time. Answers how many it took.
pub(crate) async fn catch_up(database: &Store, version: u32) -> Result<usize, Failure> {
    let since = since();
    let inboxes = {
        let reader = database.read().await?;
        ThreadRepository::new(&reader).unified_inboxes().await?
    };
    let mut taken = 0;
    for (_, inbox) in inboxes {
        loop {
            let connection = database.connect_background().await?;
            let batch = FocusClassifiedRepository::new(&connection)
                .pending_bodies(inbox, since, version, BATCH)
                .await?;
            if batch.is_empty() {
                break;
            }
            classify(&connection, &batch, version).await?;
            taken += batch.len();
            drop(connection);
            tokio::time::sleep(BREATHER).await;
        }
    }
    if taken > 0 {
        tracing::info!(taken, "Focus's body stage caught up");
    }
    Ok(taken)
}

/// Classify the bodies in `landed` that are the stage's to read.
async fn landed_bodies(
    database: &Store,
    landed: &[MessageId],
    version: u32,
) -> Result<(), Failure> {
    for chunk in landed.chunks(BATCH as usize) {
        let connection = database.connect_background().await?;
        let batch = FocusClassifiedRepository::new(&connection)
            .bodies_to_classify(chunk, since(), version)
            .await?;
        classify(&connection, &batch, version).await?;
    }
    Ok(())
}

/// Classify `batch` and record it at `version`: the reads first, with no
/// write permit held, then every write in one background transaction.
async fn classify(connection: &Checkout, batch: &[MessageId], version: u32) -> Result<(), Failure> {
    if batch.is_empty() {
        return Ok(());
    }
    let mut people: HashMap<AccountId, Arc<Vec<Identity>>> = HashMap::new();
    let mut markers = Vec::new();
    for &message in batch {
        match needs_action(connection, message, &mut people).await {
            Ok(Some(marker)) => markers.push(marker),
            Ok(None) => {}
            // Recorded all the same, so it is not taken again and again;
            // said by id and outcome, never by what it says.
            Err(error) => tracing::warn!(
                message = message.get(),
                %error,
                "Focus's body stage could not classify a message: {error}"
            ),
        }
    }
    let _permit = connection
        .write_gate()
        .acquire(WritePriority::Background)
        .await;
    postio_storage::transaction(connection, |transaction| async move {
        let written = MarkerRepository::new(&transaction);
        let digests = DigestRepository::new(&transaction);
        for marker in &markers {
            written.insert(marker).await?;
            // FR-122: mail that asks something of the user is not held.
            // Filing held it on its headers alone; its body says otherwise.
            digests.release_waiting(marker.message).await?;
        }
        FocusClassifiedRepository::new(&transaction)
            .record(batch, FocusStage::Body, version)
            .await
    })
    .await
}

/// The question or to-do `message` puts to the user, if the built-in
/// detector finds one (FR-104 to FR-106).
///
/// Its body is read only when FR-106's gate would let the detector consider
/// it -- mail sent directly to the user -- and it is read from this machine,
/// never fetched (FR-141). What the detector reads is the newest message's
/// own words (`postio_body::own_text`), and the marker quotes a span of them.
async fn needs_action(
    connection: &Checkout,
    message: MessageId,
    people: &mut HashMap<AccountId, Arc<Vec<Identity>>>,
) -> Result<Option<Marker>, Failure> {
    let messages = MessageRepository::new(connection);
    let Some(row) = messages.get(message).await? else {
        return Ok(None);
    };
    let identities = match people.get(&row.account_id) {
        Some(identities) => Arc::clone(identities),
        None => {
            let identities = Arc::new(identities_of(connection, row.account_id).await?);
            people.insert(row.account_id, Arc::clone(&identities));
            identities
        }
    };
    let asked = BodyMessage {
        filed: filed_in_the_inbox(&row),
        identities: &identities,
    };
    if !postio_classify::considered(&asked, &Shipped) {
        return Ok(None);
    }
    let Some(stored) = messages.body(message).await? else {
        return Ok(None);
    };
    // Cutting the own text parses and sanitises HTML, and the detector reads
    // every clause: work for one core, off the runtime's own threads.
    let body = MessageBody {
        text: stored.text,
        html: stored.html,
    };
    let marker = tokio::task::spawn_blocking(move || {
        let own = postio_body::own_text(&body);
        let text = OwnText::new(&own);
        let asked = BodyMessage {
            filed: filed_in_the_inbox(&row),
            identities: &identities,
        };
        postio_classify::at_body(&asked, &text, &NoGuards, &Shipped)
            .marker
            .and_then(|candidate| detected(message, &text, candidate))
    })
    .await
    .unwrap_or(None);
    Ok(marker)
}

/// `row` as the classifier is handed it: filed in the inbox, which is all
/// the body stage takes.
fn filed_in_the_inbox(row: &Message) -> FiledMessage<'_> {
    FiledMessage {
        message: row,
        thread: row.thread_id,
        role: MailboxRole::Inbox,
    }
}

/// Who "you" are on `account`: its identities, and its own address when no
/// identity carries it, since that is where its mail is sent.
async fn identities_of(
    connection: &Checkout,
    account: AccountId,
) -> Result<Vec<Identity>, Failure> {
    let Some(account) = AccountRepository::new(connection).get(account).await? else {
        return Ok(Vec::new());
    };
    let mut identities = account.identities.clone();
    if !identities
        .iter()
        .any(|identity| identity.address.same_address(&account.address))
    {
        identities.push(Identity::new(account.id, account.address.clone()));
    }
    Ok(identities)
}

/// The marker a detected question or to-do becomes: its sentence as
/// offsets into the own text, and the excerpt cut from it (research R2).
/// An invitation is not the detector's to find: it comes from the calendar
/// part.
fn detected(message: MessageId, text: &OwnText<'_>, candidate: MarkerCandidate) -> Option<Marker> {
    let kind = match candidate.kind {
        postio_classify::MarkerKind::Question => MarkerKind::Question,
        postio_classify::MarkerKind::Todo => MarkerKind::Todo,
        postio_classify::MarkerKind::Invite => return None,
    };
    let span = candidate.span?;
    Some(Marker {
        message,
        kind,
        source: MarkerSource::Detector,
        span: Some((
            u32::try_from(span.start).ok()?,
            u32::try_from(span.end).ok()?,
        )),
        excerpt: Some(text.excerpt(&span)),
        starts_at: None,
        ends_at: None,
        due_at: candidate.due_at,
        invite: None,
        invite_state: None,
        answer: None,
        dismissed_at: None,
    })
}

/// What the detector decides by: the automated-senders table Postio ships,
/// as data (FR-114).
struct Shipped;

impl Rules for Shipped {
    fn senders(&self) -> &Senders {
        Senders::shipped()
    }
}

/// The body stage asks nothing of the guards: nothing it does files or
/// holds mail. A classifier that asked anyway would be told what a store
/// that cannot answer says.
struct NoGuards;

impl Facts for NoGuards {
    fn wrote_to(&self, _: &EmailAddress) -> bool {
        true
    }
    fn took_part(&self, _: ThreadId) -> bool {
        true
    }
    fn own_domain(&self, _: &EmailAddress) -> bool {
        true
    }
    fn never_filter(&self, _: &EmailAddress) -> bool {
        true
    }
}
