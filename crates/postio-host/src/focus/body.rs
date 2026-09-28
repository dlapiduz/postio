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
//! - **Invitations** (FR-100, FR-103, T110), from the calendar part stored
//!   with the body: the event's times, placed on the user's clock when the
//!   calendar left them floating, and whether it is open, cancelled or
//!   over. Every message carrying the same event shows the newest word on
//!   it, whichever the stage read first.
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

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use std::collections::HashMap;

use chrono::{DateTime, Local, TimeDelta, TimeZone, Utc};
use postio_calendar::{Invitation, Method};
use postio_classify::{BodyMessage, Facts, FiledMessage, MarkerCandidate, OwnText, Rules, Senders};
use postio_config::{FocusConfig, FocusFilter};
use postio_model::listing::MarkerKind;
use postio_model::{
    AccountId, EmailAddress, Identity, MailboxRole, Message, MessageBody, MessageId, ThreadId,
};
use postio_storage::repository::{
    AccountRepository, DigestRepository, FocusClassifiedRepository, FocusStage, InviteIdentity,
    InviteState, Marker, MarkerRepository, MarkerSource, MessageRepository, ThreadRepository,
};
use postio_storage::{BlobStore, Checkout, Connection, Store, WritePriority};

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
pub(super) fn spawn(
    inner: &Inner,
    config: Arc<RwLock<FocusConfig>>,
    caught_up: Arc<AtomicBool>,
) -> tokio::task::AbortHandle {
    let events = inner.hub.subscribe("focus:body-stage");
    let database = inner.wiring.database.clone();
    let blobs = inner.wiring.blobs.clone();
    // `[focus.filter]` as it stands when each batch is read: a kind the
    // person stopped reaches the next message classified.
    let corrections = move || config.read().expect("never poisoned").filter.clone();
    inner
        .runtime()
        .spawn(async move {
            let version = postio_classify::VERSION;
            if let Err(error) = catch_up(&database, &blobs, version, &corrections).await {
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
                if let Err(error) =
                    landed_bodies(&database, &blobs, &landed, version, &corrections).await
                {
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
pub(crate) async fn catch_up(
    database: &Store,
    blobs: &BlobStore,
    version: u32,
    corrections: &(dyn Fn() -> FocusFilter + Send + Sync),
) -> Result<usize, Failure> {
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
            classify(&connection, blobs, &batch, version, &corrections()).await?;
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
    blobs: &BlobStore,
    landed: &[MessageId],
    version: u32,
    corrections: &(dyn Fn() -> FocusFilter + Send + Sync),
) -> Result<(), Failure> {
    for chunk in landed.chunks(BATCH as usize) {
        let connection = database.connect_background().await?;
        let batch = FocusClassifiedRepository::new(&connection)
            .bodies_to_classify(chunk, since(), version)
            .await?;
        classify(&connection, blobs, &batch, version, &corrections()).await?;
    }
    Ok(())
}

/// What the stage found in one message: at most one marker's worth.
enum Found {
    /// An invitation, from its calendar part: an invitation's marker comes
    /// from the calendar, never from the words around it (FR-100).
    Invitation(Invitation),
    /// A question or a to-do, from its own words.
    Asks(Marker),
}

/// Classify `batch` and record it at `version`: the reads first, with no
/// write permit held, then every write in one background transaction.
async fn classify(
    connection: &Checkout,
    blobs: &BlobStore,
    batch: &[MessageId],
    version: u32,
    corrections: &FocusFilter,
) -> Result<(), Failure> {
    if batch.is_empty() {
        return Ok(());
    }
    let now = Utc::now();
    let mut people: HashMap<AccountId, Arc<Vec<Identity>>> = HashMap::new();
    let mut invited = Vec::new();
    let mut asks = Vec::new();
    for &message in batch {
        match found_in(connection, blobs, message, &mut people, corrections).await {
            Ok(Some(Found::Invitation(invitation))) => invited.push((message, invitation)),
            Ok(Some(Found::Asks(marker))) => asks.push(marker),
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
        let digests = DigestRepository::new(&transaction);
        // One at a time, in the transaction: an update read in this batch
        // must see the marker its request was given a moment ago.
        for (message, invitation) in &invited {
            mark_invitation(&transaction, blobs, *message, invitation, now).await?;
            // FR-122: an invitation is never held. Filing sees a calendar
            // part only when the server described the structure.
            digests.release_waiting(*message).await?;
        }
        let written = MarkerRepository::new(&transaction);
        for marker in &asks {
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

/// What `message` calls out, if anything: its invitation, when its calendar
/// part is here and is one, and otherwise the question or to-do its own
/// words put to the user.
async fn found_in(
    connection: &Checkout,
    blobs: &BlobStore,
    message: MessageId,
    people: &mut HashMap<AccountId, Arc<Vec<Identity>>>,
    corrections: &FocusFilter,
) -> Result<Option<Found>, Failure> {
    let Some(row) = MessageRepository::new(connection).get(message).await? else {
        return Ok(None);
    };
    if let Some(invitation) = invitation_of(blobs, &row).await {
        return Ok(Some(Found::Invitation(invitation)));
    }
    Ok(needs_action(connection, row, people, corrections)
        .await?
        .map(Found::Asks))
}

/// The invitation `row`'s calendar part carries, when the part is on this
/// machine and reads as one: a request or a cancellation. A part that does
/// not parse, or that answers nothing -- a `PUBLISH`, somebody's `REPLY` --
/// is no invitation, and never an error anybody has to dismiss.
async fn invitation_of(blobs: &BlobStore, row: &Message) -> Option<Invitation> {
    let parts: Vec<_> = row
        .attachments
        .iter()
        .filter(|part| part.mime_type.eq_ignore_ascii_case("text/calendar"))
        .filter_map(|part| part.blob_id.clone())
        .collect();
    if parts.is_empty() {
        return None;
    }
    let blobs = blobs.clone();
    tokio::task::spawn_blocking(move || {
        parts.iter().find_map(|part| {
            let ics = blobs.get(part).ok()?;
            postio_calendar::parse(&ics)
                .ok()
                .filter(|invitation| matches!(invitation.method, Method::Request | Method::Cancel))
        })
    })
    .await
    .ok()
    .flatten()
}

/// Mark `message`'s invitation, and every message carrying the same event:
/// each shows the newest word on it (FR-103).
///
/// The same event is the same `UID` and the same occurrence. Which of them
/// is newest is `postio_calendar::supersedes`'s to say, and it is asked of
/// the invitations themselves, read again from each message's own calendar
/// part: the store keeps no `RECURRENCE-ID`. So the order the stage reads
/// them in does not matter -- a cancellation read before the request it
/// cancels still cancels it. A marker already showing the newest word is
/// left as it is, answer and all.
async fn mark_invitation(
    transaction: &Connection,
    blobs: &BlobStore,
    message: MessageId,
    invitation: &Invitation,
    now: DateTime<Utc>,
) -> Result<(), Failure> {
    let markers = MarkerRepository::new(transaction);
    let mut event = vec![(message, invitation.clone())];
    for other in markers.invitations(&invitation.uid).await? {
        if other.message == message {
            continue;
        }
        let Some(row) = MessageRepository::new(transaction)
            .get(other.message)
            .await?
        else {
            continue;
        };
        if let Some(theirs) = invitation_of(blobs, &row).await
            && theirs.uid == invitation.uid
            && theirs.recurrence_id == invitation.recurrence_id
        {
            event.push((other.message, theirs));
        }
    }
    let newest =
        event
            .iter()
            .map(|(_, candidate)| candidate)
            .fold(invitation, |newest, candidate| {
                if postio_calendar::supersedes(candidate, newest) {
                    candidate
                } else {
                    newest
                }
            });
    for (member, _) in &event {
        let marker = invitation_marker(*member, newest, now, &Local);
        if markers.insert(&marker).await? {
            continue;
        }
        let shows_it = markers.get(*member).await?.is_some_and(|held| {
            held.invite == marker.invite && held.invite_state == marker.invite_state
        });
        if !shows_it {
            markers.replace(&marker).await?;
        }
    }
    Ok(())
}

/// The marker `invitation` gives `message`, as of `now`, with floating and
/// all-day times placed in `zone`, the user's (FR-100).
///
/// A cancelled event says so, and one that is over -- a series once its
/// last occurrence is -- is past; either way it offers no answer (FR-103).
fn invitation_marker<Tz: TimeZone>(
    message: MessageId,
    invitation: &Invitation,
    now: DateTime<Utc>,
    zone: &Tz,
) -> Marker {
    let state = if invitation.cancelled {
        InviteState::Cancelled
    } else if invitation
        .last_end()
        .is_some_and(|end| end.instant_in(zone) <= now)
    {
        InviteState::Past
    } else {
        InviteState::Open
    };
    Marker {
        message,
        kind: MarkerKind::Invite,
        source: MarkerSource::Calendar,
        span: None,
        excerpt: None,
        starts_at: Some(invitation.starts_at.instant_in(zone)),
        ends_at: Some(invitation.ends_at.instant_in(zone)),
        due_at: None,
        invite: Some(InviteIdentity {
            uid: invitation.uid.clone(),
            sequence: i64::from(invitation.sequence),
            stamp: invitation.stamp,
        }),
        invite_state: Some(state),
        answer: None,
        dismissed_at: None,
    }
}

/// The question or to-do `row` puts to the user, if the built-in detector
/// finds one (FR-104 to FR-106).
///
/// Its body is read only when FR-106's gate would let the detector consider
/// it -- mail sent directly to the user -- and it is read from this machine,
/// never fetched (FR-141). What the detector reads is the newest message's
/// own words (`postio_body::own_text`), and the marker quotes a span of them.
async fn needs_action(
    connection: &Checkout,
    row: Message,
    people: &mut HashMap<AccountId, Arc<Vec<Identity>>>,
    corrections: &FocusFilter,
) -> Result<Option<Marker>, Failure> {
    let message = row.id;
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
    let rules = Shipped {
        corrections: corrections.clone(),
    };
    if !postio_classify::considered(&asked, &rules) {
        return Ok(None);
    }
    let Some(stored) = MessageRepository::new(connection).body(message).await? else {
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
        postio_classify::at_body(&asked, &text, &NoGuards, &rules)
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
/// as data (FR-114), and the marker kinds the person stopped for a sender
/// by dismissing them (FR-108).
struct Shipped {
    corrections: FocusFilter,
}

impl Rules for Shipped {
    fn senders(&self) -> &Senders {
        Senders::shipped()
    }

    fn stops(&self, sender: &EmailAddress, kind: postio_classify::MarkerKind) -> bool {
        let kind = match kind {
            postio_classify::MarkerKind::Question => "question",
            postio_classify::MarkerKind::Todo => "todo",
            // An invitation comes from its calendar part, and is never the
            // detector's to find or the person's to stop.
            postio_classify::MarkerKind::Invite => return false,
        };
        self.corrections.stops(sender, kind)
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
