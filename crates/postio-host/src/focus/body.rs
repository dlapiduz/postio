//! Focus's body stage: once a body is here, finds invitations (from the calendar part) and
//! questions or to-dos (from its own words).
//! Reads only bodies already on this machine, never fetches one, and only recent inbox mail;
//! catches up at start, then hears `BodyLoaded`.
//! Every message taken is recorded at the classifier's version, so a failed one is not retried
//! endlessly.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use std::collections::HashMap;

use chrono::{DateTime, Local, TimeDelta, TimeZone, Utc};
use postio_ai::NeedsActionModel;
use postio_calendar::{Invitation, Method};
use postio_classify::{
    BodyMessage, Facts, FiledMessage, MarkerCandidate, ModelLayer, OwnText, Rules, Senders,
};
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

use super::model::Models;
use crate::Inner;

const WINDOW: TimeDelta = TimeDelta::days(30);

const BATCH: u32 = 50;

const DEBOUNCE: Duration = Duration::from_millis(500);

/// Background work: a person's own reads and writes go first.
const BREATHER: Duration = Duration::from_millis(20);

type Failure = postio_storage::Error;

#[derive(Clone)]
struct Reading {
    filter: FocusFilter,
    model: Option<Arc<NeedsActionModel>>,
}

/// `caught_up` is set once the catch-up has taken everything it found. With events going to a
/// single
/// reader rather than a hub, nothing is heard: the stage catches up and ends.
pub(super) fn spawn(
    inner: &Inner,
    config: Arc<RwLock<FocusConfig>>,
    models: Arc<Models>,
    caught_up: Arc<AtomicBool>,
    said: Arc<tokio::sync::Notify>,
) -> tokio::task::AbortHandle {
    let events = inner.hub.subscribe("focus:body-stage");
    let database = inner.wiring.database.clone();
    let blobs = inner.wiring.blobs.clone();
    // Re-read per batch so a stopped kind or a changed model reaches the next message.
    let corrections = move || {
        let config = config.read().expect("never poisoned");
        Reading {
            filter: config.filter.clone(),
            model: models.needs_action(&config),
        }
    };
    inner
        .runtime()
        .spawn(async move {
            let version = postio_classify::VERSION;
            if let Err(error) = catch_up(&database, &blobs, version, &corrections).await {
                tracing::warn!(%error, "Focus's body stage could not catch up: {error}");
            }
            caught_up.store(true, Ordering::Release);
            said.notify_one();
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

fn note(landed: &mut Vec<MessageId>, event: &postio_core::Event) {
    if let postio_core::Event::BodyLoaded { message, .. } = event {
        landed.push(*message);
    }
}

fn since() -> DateTime<Utc> {
    Utc::now() - WINDOW
}

async fn catch_up(
    database: &Store,
    blobs: &BlobStore,
    version: u32,
    corrections: &(dyn Fn() -> Reading + Send + Sync),
) -> Result<usize, Failure> {
    let since = since();
    let inboxes = {
        let reader = database.read().await?;
        ThreadRepository::new(&reader).unified_inboxes().await?
    };
    let mut taken = 0;
    for (_, inbox) in inboxes {
        loop {
            super::yield_to_reads(database).await;
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

async fn landed_bodies(
    database: &Store,
    blobs: &BlobStore,
    landed: &[MessageId],
    version: u32,
    corrections: &(dyn Fn() -> Reading + Send + Sync),
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

enum Found {
    /// An invitation's marker comes from the calendar, never from the words around it.
    Invitation(Invitation),
    Asks(Marker),
}

/// Reads first, with no write permit held, then every write in one background transaction.
async fn classify(
    connection: &Checkout,
    blobs: &BlobStore,
    batch: &[MessageId],
    version: u32,
    corrections: &Reading,
) -> Result<(), Failure> {
    if batch.is_empty() {
        return Ok(());
    }
    let now = Utc::now();
    let mut people: HashMap<AccountId, Arc<Vec<Identity>>> = HashMap::new();
    let mut invited = Vec::new();
    let mut asks = Vec::new();
    let mut bounced = Vec::new();
    let mut noticed = Vec::new();
    let mut lines = Vec::new();
    // Whether to read signatures and forwarding headers; read once per batch.
    let suggestions = postio_storage::repository::ContactSuggestionRepository::new(connection)
        .get()
        .await
        .unwrap_or_default();
    for &message in batch {
        if let Ok(Some(row)) = MessageRepository::new(connection).get(message).await
            && let Some(bounce) = bounce_of(blobs, &row).await
        {
            bounced.push((bounce, row.received_at));
        }
        if suggestions.is_on(postio_model::Suggestion::SuggestMerges)
            && let Ok(Some(stored)) = MessageRepository::new(connection).body(message).await
        {
            let found = postio_classify::duplicates::noticed(
                stored.text.as_deref().unwrap_or(""),
                stored.headers.as_deref().unwrap_or(""),
                suggestions.is_on(postio_model::Suggestion::ReadSignatures),
            );
            if !found.is_empty() {
                noticed.push((message, found));
            }
        }
    }
    for &message in batch {
        if let Some(line) = own_line_of(connection, message).await {
            lines.push((message, line));
        }
    }
    for &message in batch {
        match found_in(connection, blobs, message, &mut people, corrections).await {
            Ok(Some(Found::Invitation(invitation))) => invited.push((message, invitation)),
            Ok(Some(Found::Asks(marker))) => asks.push(marker),
            Ok(None) => {}
            // Recorded all the same so it is not taken again; logged by id and outcome, never
            // content.
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
        // One at a time: an update read in this batch must see the marker its request was just
        // given.
        for (message, invitation) in &invited {
            mark_invitation(&transaction, blobs, *message, invitation, now).await?;
            // An invitation is never held; filing sees a calendar part only when the server
            // described the structure.
            digests.release_waiting(*message).await?;
        }
        for (message, line) in &lines {
            MessageRepository::new(&transaction)
                .set_own_line(*message, line)
                .await?;
        }
        let written = MarkerRepository::new(&transaction);
        for marker in &asks {
            written.insert(marker).await?;
            // Mail that asks something is not held; filing held it on its headers alone.
            digests.release_waiting(marker.message).await?;
        }
        for (message, found) in &noticed {
            postio_storage::repository::ContactRepository::new(&transaction)
                .record_evidence(*message, found)
                .await?;
        }
        for (bounce, at) in &bounced {
            postio_storage::repository::ContactRepository::new(&transaction)
                .mark_bounced(&bounce.address, *at)
                .await?;
        }
        FocusClassifiedRepository::new(&transaction)
            .record(batch, FocusStage::Body, version)
            .await
    })
    .await
}

/// `None` when no body is here, which leaves the column NULL. Cutting parses and sanitises HTML, so
/// it runs off the runtime's threads.
async fn own_line_of(connection: &Checkout, message: MessageId) -> Option<String> {
    let stored = MessageRepository::new(connection)
        .body(message)
        .await
        .ok()??;
    let body = MessageBody {
        text: stored.text,
        html: stored.html,
    };
    tokio::task::spawn_blocking(move || postio_body::own_line(&body))
        .await
        .ok()
}

async fn bounce_of(blobs: &BlobStore, row: &Message) -> Option<postio_classify::bounce::Bounce> {
    let parts: Vec<_> = row
        .attachments
        .iter()
        .filter(|part| {
            part.mime_type
                .eq_ignore_ascii_case("message/delivery-status")
        })
        .filter_map(|part| part.blob_id.clone())
        .collect();
    if parts.is_empty() {
        return None;
    }
    let blobs = blobs.clone();
    tokio::task::spawn_blocking(move || {
        parts.iter().find_map(|part| {
            let status = blobs.get(part).ok()?;
            postio_classify::bounce::detect(
                Some("multipart/report; report-type=delivery-status"),
                None,
                &String::from_utf8_lossy(&status),
            )
        })
    })
    .await
    .ok()
    .flatten()
}

async fn found_in(
    connection: &Checkout,
    blobs: &BlobStore,
    message: MessageId,
    people: &mut HashMap<AccountId, Arc<Vec<Identity>>>,
    corrections: &Reading,
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

/// A part that does not parse, or answers nothing (a `PUBLISH`, somebody's `REPLY`), is no
/// invitation and never an error.
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

/// Marks every message carrying the same event (same `UID` and occurrence) with the newest word,
/// per
/// `postio_calendar::supersedes`, so read order does not matter: a cancellation read before its
/// request still cancels it.
/// Invitations are re-read from each calendar part because the store keeps no `RECURRENCE-ID`.
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

/// Floating and all-day times are placed in `zone`. A cancelled event, or one that is over, offers
/// no answer.
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

/// Reads the body only for mail sent directly to the user, from this machine, never fetched.
async fn needs_action(
    connection: &Checkout,
    row: Message,
    people: &mut HashMap<AccountId, Arc<Vec<Identity>>>,
    corrections: &Reading,
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
        corrections: corrections.filter.clone(),
    };
    let model = corrections.model.clone();
    if !postio_classify::considered(&asked, &rules) {
        return Ok(None);
    }
    let Some(stored) = MessageRepository::new(connection).body(message).await? else {
        return Ok(None);
    };
    // Parses and sanitises HTML and reads every clause: off the runtime's threads.
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
        // The model answers in the detector's place, the detector when it cannot; on a blocking
        // thread so waiting holds up only this batch.
        let model = model.as_deref().map(|model| model as &dyn ModelLayer);
        postio_classify::at_body_with(&asked, &text, &NoGuards, &rules, model)
            .marker
            .and_then(|candidate| detected(message, &text, candidate))
    })
    .await
    .unwrap_or(None);
    Ok(marker)
}

fn filed_in_the_inbox(row: &Message) -> FiledMessage<'_> {
    FiledMessage {
        message: row,
        thread: row.thread_id,
        role: MailboxRole::Inbox,
    }
}

/// Includes the account's own address when no identity carries it, since its mail is sent there.
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

/// An invitation is not the detector's to find; it comes from the calendar part.
fn detected(message: MessageId, text: &OwnText<'_>, candidate: MarkerCandidate) -> Option<Marker> {
    let kind = match candidate.kind {
        postio_classify::MarkerKind::Question => MarkerKind::Question,
        postio_classify::MarkerKind::Todo => MarkerKind::Todo,
        postio_classify::MarkerKind::Invite => return None,
    };
    let span = candidate.span?;
    let source = match candidate.by {
        postio_classify::MarkedBy::Model => MarkerSource::Model,
        postio_classify::MarkedBy::Detector => MarkerSource::Detector,
        postio_classify::MarkedBy::Calendar => MarkerSource::Calendar,
    };
    Some(Marker {
        message,
        kind,
        source,
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
            // An invitation is never the detector's to find or the person's to stop.
            postio_classify::MarkerKind::Invite => return false,
        };
        self.corrections.stops(sender, kind)
    }
}

/// Asks nothing of the guards: nothing this stage does files or holds mail.
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
