//! Digest summaries (spec 007 T154, FR-172 to FR-175, US13): written in the
//! background by the person's own model once a digest is delivered, kept
//! with the delivery, and resolved again each time they are read.
//!
//! # Written
//!
//! Each tick of the due timer hands the open deliveries with no summary to
//! the summariser, one at a time, on a task of its own, so nothing waits on
//! the model: the digest row is there with its senders from the moment it
//! is delivered, and gains its summary's line when one is written (FR-142,
//! FR-175). Only when `[focus.model]` names a model with `digest_summary`
//! on; otherwise nothing is asked and nothing connects (FR-166).
//!
//! - The summary is written from the messages' own text, read from this
//!   machine; a message whose body is not here is left out, and a delivery
//!   with none is tried again on a later tick.
//! - A model that is not running leaves the delivery for later; the client
//!   leaves the runtime alone for a while, so a later tick costs nothing.
//! - A model that answers off its schema, or whose every statement fails to
//!   resolve, leaves an empty summary, which is not asked for again: the
//!   digest opens on its plain list.
//!
//! # Read
//!
//! [`digest_summary`] resolves every reference again against its message's
//! own text, byte for byte, and drops a statement whose passage is not
//! there any more (research R16). Nothing here logs a word of a summary or
//! a message: ids and counts only.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use postio_ai::{AiError, Source, Summariser};
use postio_config::{FocusConfig, ModelFeature};
use postio_model::listing::StoreError;
use postio_model::summary::DigestSummary;
use postio_model::{AccountId, DeliveryId, MessageBody, MessageId};
use postio_storage::repository::{DigestRepository, MessageRepository};
use postio_storage::{Connection, Store, WritePriority};

use super::model::Models;
use crate::Inner;

/// How many deliveries one pass takes.
const PASS: u32 = 4;

/// A message a summary is written from, read from the store.
struct Read {
    message: MessageId,
    account: AccountId,
    sender: String,
    sender_address: String,
    subject: String,
    text: String,
}

/// Summarise what is delivered and waiting for a summary, on a task of its
/// own, unless the last pass is still running. The due timer calls it each
/// tick.
pub(crate) fn summarise_in_background(
    inner: &Arc<Inner>,
    models: &Arc<Models>,
    config: &FocusConfig,
    busy: &Arc<AtomicBool>,
) {
    if config.model_for(ModelFeature::DigestSummary).is_none() {
        return;
    }
    if busy.swap(true, Ordering::AcqRel) {
        return;
    }
    let (inner, models, config, busy) = (
        Arc::clone(inner),
        Arc::clone(models),
        config.clone(),
        Arc::clone(busy),
    );
    inner.runtime().clone().spawn(async move {
        match summarise_open(&inner.wiring.database, &models, &config).await {
            Ok(0) => {}
            Ok(written) => {
                tracing::debug!(written, "Focus wrote digest summaries");
                inner.hub.emit(postio_core::Event::SurfacedChanged);
            }
            Err(error) => {
                tracing::warn!(%error, "Focus could not write its digest summaries: {error}");
            }
        }
        busy.store(false, Ordering::Release);
    });
}

/// Write a summary for each open delivery that has none, as far as the
/// model answers. Answers how many it wrote.
async fn summarise_open(
    database: &Store,
    models: &Models,
    config: &FocusConfig,
) -> Result<usize, postio_storage::Error> {
    let Some(client) = models.client(config, ModelFeature::DigestSummary) else {
        return Ok(0);
    };
    let waiting = {
        let reader = database.read().await?;
        DigestRepository::new(&reader).unsummarised(PASS).await?
    };
    let mut written = 0;
    for delivery in waiting {
        let sources = {
            let reader = database.read().await?;
            sources_of(&reader, delivery).await?
        };
        if sources.is_empty() {
            continue;
        }
        let summariser = Summariser::new(Arc::clone(&client));
        let answered = tokio::task::spawn_blocking(move || {
            let senders = sources
                .iter()
                .map(|source| source.sender_address.to_lowercase())
                .collect::<HashSet<_>>()
                .len();
            let borrowed: Vec<Source<'_>> = sources
                .iter()
                .map(|source| Source {
                    message: source.message,
                    account: source.account,
                    sender: &source.sender,
                    subject: &source.subject,
                    text: &source.text,
                })
                .collect();
            summariser.summarise(&borrowed, u32::try_from(senders).unwrap_or(u32::MAX))
        })
        .await
        .unwrap_or(Err(AiError::Down));
        let summary = match answered {
            Ok(summary) => summary,
            // Off its schema: nothing to show, and not asked again.
            Err(AiError::Malformed(_)) => DigestSummary {
                statements: Vec::new(),
                messages: 0,
                senders: 0,
            },
            // Not running, or would not take the request: later.
            Err(_) => break,
        };
        let json = serde_json::to_string(&summary).unwrap_or_else(|_| "{}".to_owned());
        let connection = database.connect_background().await?;
        let _permit = connection
            .write_gate()
            .acquire(WritePriority::Background)
            .await;
        DigestRepository::new(&connection)
            .set_summary(delivery, &json, chrono::Utc::now())
            .await?;
        tracing::debug!(
            delivery = delivery.get(),
            statements = summary.statements.len(),
            "Focus kept a digest summary"
        );
        written += 1;
    }
    Ok(written)
}

/// The messages `delivery` holds whose bodies are here, oldest first, with
/// their own text: at most as many as a summary is written from.
async fn sources_of(
    connection: &Connection,
    delivery: DeliveryId,
) -> Result<Vec<Read>, postio_storage::Error> {
    let held = DigestRepository::new(connection)
        .delivery_messages(delivery)
        .await?;
    let messages = MessageRepository::new(connection);
    let mut read = Vec::new();
    for id in held.into_iter().take(postio_ai::MAX_SOURCES) {
        let Some(row) = messages.get(id).await? else {
            continue;
        };
        let Some(body) = messages.body(id).await? else {
            continue;
        };
        let text = own_text(MessageBody {
            text: body.text,
            html: body.html,
        })
        .await;
        if text.trim().is_empty() {
            continue;
        }
        let sender = row.from.first();
        read.push(Read {
            message: id,
            account: row.account_id,
            sender: sender
                .map(|from| from.name.clone().unwrap_or_else(|| from.address.clone()))
                .unwrap_or_default(),
            sender_address: sender.map(|from| from.address.clone()).unwrap_or_default(),
            subject: row.subject.clone().unwrap_or_default(),
            text,
        });
    }
    Ok(read)
}

/// A message's own words, cut off the runtime's threads.
async fn own_text(body: MessageBody) -> String {
    tokio::task::spawn_blocking(move || postio_body::own_text(&body))
        .await
        .unwrap_or_default()
}

/// `delivery`'s summary, as it is shown: every statement whose passage is
/// still in its message's own text, byte for byte. `None` when none is
/// written, or none is left to show (FR-173, FR-175).
///
/// Two statements for the summary and what the digest holds, then one for
/// each message it cites (its body).
pub(crate) async fn digest_summary(
    inner: &Inner,
    delivery: DeliveryId,
) -> Result<Option<DigestSummary>, StoreError> {
    let reader = inner.wiring.database.read().await?;
    let Some(json) = DigestRepository::new(&reader).summary(delivery).await? else {
        return Ok(None);
    };
    let Ok(mut summary) = serde_json::from_str::<DigestSummary>(&json) else {
        return Ok(None);
    };
    let held: HashSet<MessageId> = DigestRepository::new(&reader)
        .delivery_messages(delivery)
        .await?
        .into_iter()
        .collect();
    let messages = MessageRepository::new(&reader);
    let mut texts: HashMap<MessageId, Option<String>> = HashMap::new();
    let mut kept = Vec::with_capacity(summary.statements.len());
    for statement in summary.statements {
        let message = statement.reference.message;
        if !held.contains(&message) {
            continue;
        }
        if let std::collections::hash_map::Entry::Vacant(slot) = texts.entry(message) {
            let text = match messages.body(message).await? {
                Some(body) => Some(
                    own_text(MessageBody {
                        text: body.text,
                        html: body.html,
                    })
                    .await,
                ),
                None => None,
            };
            slot.insert(text);
        }
        let resolves = texts
            .get(&message)
            .and_then(Option::as_deref)
            .is_some_and(|text| {
                postio_ai::resolve(&statement.reference.excerpt, text).as_deref()
                    == Some(statement.reference.excerpt.as_str())
            });
        if resolves {
            kept.push(statement);
        }
    }
    summary.statements = kept;
    Ok((!summary.is_empty()).then_some(summary))
}

/// The digest row's line from a delivery's kept summary: its opening, or
/// `None` when there is none to show (FR-124).
pub(crate) fn line_of(json: Option<&str>) -> Option<String> {
    serde_json::from_str::<DigestSummary>(json?).ok()?.line()
}
