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
//! # Records
//!
//! Every message the stage takes is recorded at the classifier's version
//! (`focus_classified`), whatever came of it, so a message whose
//! classification failed is not taken again and again. A failure is logged
//! by ids and outcome, never by content.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use chrono::{DateTime, TimeDelta, Utc};
use postio_model::MessageId;
use postio_storage::repository::{FocusClassifiedRepository, FocusStage, ThreadRepository};
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

/// Classify `batch` and record it at `version`.
async fn classify(connection: &Checkout, batch: &[MessageId], version: u32) -> Result<(), Failure> {
    if batch.is_empty() {
        return Ok(());
    }
    let _permit = connection
        .write_gate()
        .acquire(WritePriority::Background)
        .await;
    FocusClassifiedRepository::new(connection)
        .record(batch, FocusStage::Body, version)
        .await
}
