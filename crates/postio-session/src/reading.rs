//! Reading a message's body, and resolving the parts it references (#608). `load_body_or_reason`
//! tells apart situations that look identical in the columns (draft, another client's draft, not
//! fetched, empty, undecodable; #70).
//! `resolve_cid` resolves a `Content-ID` only within its own message, so one sender cannot address
//! another's parts.

use postio_core::Event;
use postio_core::bridge::{EventSink, EventStream};
use postio_model::MessageBody;
use postio_model::fetch::PayloadAsk;
use postio_model::ids::MessageId;
use postio_runtime::Engine;
use postio_storage::repository::{DraftRepository, MessageRepository};
use postio_storage::{BlobStore, Store};
use postio_ui::reader::document::Absent;
use postio_ui::reader::parts::{self, Node};

/// A message's text and HTML, if downloaded; absent, not an error, for a `Partial` message.
pub async fn load_body(connection: &postio_storage::Checkout, id: MessageId) -> MessageBody {
    let Ok(Some(stored)) = MessageRepository::new(connection).body(id).await else {
        return MessageBody::default();
    };
    MessageBody {
        text: stored.text,
        html: stored.html,
    }
}

pub enum Body {
    Ready {
        body: postio_model::MessageBody,
        /// Whether those words are a guess rather than what was sent (#901).
        encoding_problems: bool,
        /// The header block the body was stored with, unparsed: what `sender_warning` reads, so
        /// judging the sender costs no second decompression (#1928).
        headers: Option<String>,
    },
    Absent(Absent),
}

/// As `load_body`, but distinguishing the ways a body can be missing.
///
/// The message's `BodyState` says whether a body was ever fetched: an empty row means both "not
/// downloaded" and "downloaded, no body". `is_offline` tells `Absent::Offline` from
/// `Absent::Partial`; the caller reads it off the engine's `ConnectionState`.
pub async fn load_body_or_reason(
    connection: &postio_storage::Checkout,
    id: MessageId,
    is_offline: bool,
) -> Body {
    load_with_row(connection, id, is_offline).await.0
}

/// `load_body_or_reason`, and the message row it read; `None` for a draft this machine owns.
pub async fn load_with_row(
    connection: &postio_storage::Checkout,
    id: MessageId,
    is_offline: bool,
) -> (Body, Option<postio_model::Message>) {
    use Absent;

    // A draft's body is in `drafts.body_text`, inline and uncompressed for autosave, not in
    // `messages`; reading the row would say "still downloading" (#166).
    if let Ok(Some(draft)) = DraftRepository::new(connection).by_message(id).await {
        return (
            Body::Ready {
                body: draft.body,
                encoding_problems: false,
                headers: None,
            },
            None,
        );
    }

    let repository = MessageRepository::new(connection);

    // Has anything been downloaded for this message at all?
    let row = match repository.get(id).await {
        // `\Draft` with no local buffer is another client's draft: nothing here this machine can
        // edit (#175).
        Ok(Some(message)) if message.flags.is_draft() => {
            return (Body::Absent(Absent::ForeignDraft), Some(message));
        }
        Ok(Some(message)) if !message.sync.body_state.has_body() => {
            // Asked only on the way to "not yet": a server's refusal outranks `Offline` (#1918).
            let failed = repository.body_failed(id).await.unwrap_or(false);
            let reason = postio_ui::reader::document::unfetched(failed, is_offline);
            return (Body::Absent(reason), Some(message));
        }
        Ok(Some(message)) => message,
        Ok(None) => return (Body::Absent(Absent::Missing), None),
        Err(error) => {
            tracing::warn!(message = id.get(), %error, "cannot read a message row");
            return (Body::Absent(Absent::Missing), None);
        }
    };

    let stored = match repository.body(id).await {
        Ok(Some(stored)) => stored,
        // The row went between the two reads above and here.
        Ok(None) => return (Body::Absent(Absent::Missing), Some(row)),
        Err(error) => {
            tracing::warn!(message = id.get(), %error, "cannot read a message's body");
            return (Body::Absent(Absent::Missing), Some(row));
        }
    };

    let declared = row.content_type.clone().or_else(|| {
        stored
            .headers
            .as_deref()
            .map(postio_model::headers::parse_block)
            .and_then(|headers| {
                headers
                    .get("Content-Type")
                    .and_then(|value| value.split(';').next())
                    .map(str::to_owned)
            })
    });
    if declared
        .as_deref()
        .is_some_and(postio_ui::reader::document::is_encrypted)
    {
        return (
            Body::Ready {
                encoding_problems: false,
                headers: stored.headers,
                body: postio_model::MessageBody {
                    text: Some(postio_ui::reader::document::ENCRYPTED.to_owned()),
                    html: None,
                },
            },
            Some(row),
        );
    }

    // "No body" only when the parse says nothing went wrong: a guessed structure can come back with
    // no text part (#287).
    if stored.text.is_none() && stored.html.is_none() && !stored.encoding_problems {
        return (Body::Absent(Absent::Empty), Some(row));
    }

    (
        Body::Ready {
            encoding_problems: stored.encoding_problems,
            headers: stored.headers,
            body: postio_model::MessageBody {
                text: stored.text,
                html: stored.html,
            },
        },
        Some(row),
    )
}

/// What the reader's header says about who the sender is and whether the content is signed
/// (#1928, spec 016). `headers` is the stored header block; which servers to believe comes from the
/// account's provider (`authserv_ids`). Mail the user sent is never judged.
pub async fn authenticity(
    connection: &postio_storage::Checkout,
    row: &postio_model::Message,
    headers: Option<&str>,
) -> postio_ui::reader::authenticity::HeaderAuthenticity {
    use postio_ui::reader::authenticity::{HeaderAuthenticity, Signature};
    let headers = headers.map(postio_model::headers::parse_block);
    let keys = postio_storage::repository::KeyRepository::new(connection);
    let verdict = keys.verdict(row.id).await.ok().flatten();
    let wants_account = headers
        .as_ref()
        .is_some_and(|headers| headers.contains("Authentication-Results"));
    if verdict.is_none()
        && !wants_account
        && postio_ui::reader::authenticity::impersonation(&row.from).is_none()
    {
        return HeaderAuthenticity::default();
    }
    let account = postio_storage::repository::AccountRepository::new(connection)
        .get(row.account_id)
        .await
        .ok()
        .flatten();
    if let Some(account) = &account
        && std::iter::once(&account.address)
            .chain(account.identities.iter().map(|identity| &identity.address))
            .any(|own| row.from.iter().any(|from| from.same_address(own)))
    {
        return HeaderAuthenticity::default();
    }
    let trusted: &[String] = match (&account, wants_account) {
        (Some(account), true) => {
            postio_account::discovery::authserv_ids_for_imap_host(&account.incoming.host)
        }
        _ => &[],
    };
    let accepted = match (&verdict, row.from.first()) {
        (Some(_), Some(from)) => keys.accepted(from).await.ok().flatten().map(|(key, _)| key),
        _ => None,
    };
    let signature = verdict.map(|verdict| Signature {
        status: verdict.status,
        scope: verdict.scope,
        signer: verdict.fingerprint,
        accepted,
    });
    postio_ui::reader::authenticity::header(
        &headers.unwrap_or_default(),
        &row.from,
        trusted,
        signature.as_ref(),
    )
}

/// One inline part of `message`, by its `Content-ID`, scoped to that message so a sender cannot
/// address another's parts.
///
/// `None` when the bytes are not already on this machine: fetching here would be a tracking pixel
/// through the back door.
pub async fn resolve_cid(
    database: &Store,
    blobs: &BlobStore,
    message: MessageId,
    content_id: &str,
) -> Option<(Vec<u8>, String)> {
    let connection = database.connect().await.ok()?;
    let part = MessageRepository::new(&connection)
        .get(message)
        .await
        .ok()??
        .attachments
        .into_iter()
        .find(|part| part.content_id.as_deref() == Some(content_id))?;
    let bytes = blobs.get(&part.blob_id?).ok()?;
    Some((bytes, part.mime_type))
}

pub const BODY_WAIT: std::time::Duration = std::time::Duration::from_secs(30);

pub enum PartSource {
    Payload(postio_model::ids::BlobId),
    /// The whole raw message, from which the part is cut: rows fetched before the payload axis, or
    /// with no recorded `BODYSTRUCTURE`.
    Raw(postio_model::ids::BlobId),
}

/// A message's MIME tree, ready to draw; root and nodes travel together so a frontend cannot draw
/// one message's parts under another's type.
pub struct MessageParts {
    pub root: String,
    pub nodes: Vec<Node>,
}

/// What a message is made of, read from the store and from nothing else: it fetches nothing, and
/// `Node::downloaded` says which parts are here.
pub async fn message_parts(database: &Store, message: MessageId) -> Result<MessageParts, String> {
    let connection = database
        .connect()
        .await
        .map_err(|error| error.to_string())?;
    let stored = MessageRepository::new(&connection)
        .get(message)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "That message is no longer here".to_string())?;
    let body = load_body(&connection, message).await;

    let root = parts::root_type(stored.content_type.as_deref(), &body, &stored.attachments);
    let nodes = parts::tree(&root, &stored.attachments);
    Ok(MessageParts { root, nodes })
}

/// One part's bytes, fetched first if they are not on this machine yet (ADR 0017).
///
/// This may reach the network because the user named these bytes (save, drag, "Open with…"); the
/// reading pane reaches only for the words of the message opened (#1788). Returns `Err`, not an
/// empty file, when the bytes cannot be had.
pub async fn part_bytes_at(
    database: &Store,
    blobs: &BlobStore,
    engine: Option<Engine>,
    events: Option<&EventSink>,
    message: MessageId,
    part_id: &str,
) -> Result<Vec<u8>, String> {
    let source = match locate_part(database, message, part_id).await? {
        Some(source) => source,
        None => {
            let engine =
                engine.ok_or("This account is not syncing, so that part cannot be fetched")?;
            // Subscribed before the ask so a fast refusal is not missed (#1987).
            let failures = events.and_then(|events| events.subscribe("part wait"));
            // `request_payloads` returns once queued; `true` means there was something to fetch,
            // not that it landed.
            let asked = engine
                .request_payloads(message, vec![part_id.to_owned()])
                .await
                .map_err(|error| error.message().to_string())?;
            tracing::debug!(
                message = message.get(),
                ?asked,
                "asked the engine for a part"
            );
            match asked {
                PayloadAsk::Queued => {
                    wait_for_part(database, message, part_id, failures.as_ref(), BODY_WAIT).await?
                }
                // "Here already" may be the background lane having fetched it between the look and
                // the answer (#109); one re-read settles it.
                PayloadAsk::Local => locate_part(database, message, part_id)
                    .await?
                    .ok_or("There is nothing to fetch for that part")?,
                refused => return Err(not_fetched(refused).into()),
            }
        }
    };

    match source {
        PartSource::Payload(blob) => blobs.get(&blob).map_err(|error| error.to_string()),
        PartSource::Raw(blob) => {
            let bytes = blobs.get(&blob).map_err(|error| error.to_string())?;
            postio_model::mime::parse(&bytes)
                .parts
                .into_iter()
                .find(|part| part.attachment.part_id.as_deref() == Some(part_id))
                .map(|part| part.content)
                .ok_or_else(|| "That part is not in the message the server sent".into())
        }
    }
}

/// `part_bytes_at`, for a caller holding an attachment row. The id becomes a MIME path before
/// anything is fetched, because a whole-message fetch replaces the attachment rows and the id would
/// not survive it.
pub async fn part_bytes(
    database: &Store,
    blobs: &BlobStore,
    engine: Option<Engine>,
    events: Option<&EventSink>,
    message: MessageId,
    attachment: postio_model::ids::AttachmentId,
) -> Result<Vec<u8>, String> {
    let part_id = part_path(database, message, attachment)
        .await?
        .ok_or("That part has no place in the message to read it from")?;
    part_bytes_at(database, blobs, engine, events, message, &part_id).await
}

/// Put one part's bytes at exactly `path`. Replaces rather than appends: the save dialog asked
/// about overwriting.
pub async fn save_part(
    database: &Store,
    blobs: &BlobStore,
    engine: Option<Engine>,
    events: Option<&EventSink>,
    message: MessageId,
    part_id: &str,
    path: &std::path::Path,
) -> Result<(), String> {
    let bytes = part_bytes_at(database, blobs, engine, events, message, part_id).await?;
    std::fs::write(path, &bytes).map_err(|error| format!("Could not save that part: {error}"))?;
    // Not the filename: the sender chose it.
    tracing::debug!(message = %message, part = part_id, bytes = bytes.len(), "part saved");
    Ok(())
}

pub struct SavedParts {
    pub saved: usize,
    pub failed: usize,
}

/// Write every part that holds bytes into `directory`, named by `save_names` (which suffixes
/// duplicates). Containers are skipped, not failed; one failing part does not abandon the rest.
pub async fn save_all_parts(
    database: &Store,
    blobs: &BlobStore,
    engine: Option<Engine>,
    events: Option<&EventSink>,
    message: MessageId,
    directory: &std::path::Path,
) -> Result<SavedParts, String> {
    let leaves: Vec<Node> = message_parts(database, message)
        .await?
        .nodes
        .into_iter()
        .filter(Node::is_leaf)
        .collect();
    let names = parts::save_names(&leaves);
    std::fs::create_dir_all(directory).map_err(|error| error.to_string())?;

    let mut outcome = SavedParts {
        saved: 0,
        failed: 0,
    };
    for (node, name) in leaves.iter().zip(names) {
        let written = match part_bytes_at(
            database,
            blobs,
            engine.clone(),
            events,
            message,
            &node.part_id,
        )
        .await
        {
            Ok(bytes) => std::fs::write(directory.join(name), &bytes).is_ok(),
            Err(_) => false,
        };
        if written {
            outcome.saved += 1;
        } else {
            outcome.failed += 1;
        }
    }
    tracing::debug!(
        message = %message,
        saved = outcome.saved,
        failed = outcome.failed,
        "every part saved"
    );
    Ok(outcome)
}

pub fn not_fetched(asked: PayloadAsk) -> &'static str {
    match asked {
        PayloadAsk::Unreachable => {
            "That attachment cannot be downloaded right now \u{2014} this \
             account is not connected. It will download once the \
             connection is back, so try again then"
        }
        PayloadAsk::NotOnServer => {
            "That attachment cannot be downloaded yet \u{2014} its message \
             has only just moved and the server has not placed it. Try \
             again after the next sync"
        }
        PayloadAsk::Never => {
            "Attachments are set never to download, so that one stays on \
             the server. Change it in Settings, under Sync & storage"
        }
        PayloadAsk::Gone => "That attachment is no longer in this message",
        PayloadAsk::Queued | PayloadAsk::Local => "There is nothing to fetch for that part",
    }
}

/// Wait for a queued part to land, or give up saying so. Polls for either shape the bytes arrive in
/// (the part's blob, or the raw message).
///
/// `failures` is subscribed before the fetch was queued: a refused fetch leaves nothing to poll
/// for, only a `BodyFailed`, matched on the part (#1996). Landed bytes win over a failure heard in
/// the same breath. `wait` is `BODY_WAIT` outside tests.
pub async fn wait_for_part(
    database: &Store,
    message: MessageId,
    part_id: &str,
    failures: Option<&EventStream>,
    wait: std::time::Duration,
) -> Result<PartSource, String> {
    let started = std::time::Instant::now();
    let deadline = started + wait;
    loop {
        // A failed read is usually the writer we are waiting for; only the deadline ends this.
        match locate_part(database, message, part_id).await {
            Ok(Some(source)) => {
                tracing::debug!(
                    message = message.get(),
                    after_ms = started.elapsed().as_millis() as u64,
                    "a part asked for arrived"
                );
                return Ok(source);
            }
            Ok(None) => {}
            Err(error) if std::time::Instant::now() >= deadline => return Err(error),
            Err(_) => {}
        }
        if failures.is_some_and(|events| fetch_failed(events, message, part_id)) {
            tracing::debug!(
                message = message.get(),
                after_ms = started.elapsed().as_millis() as u64,
                "the fetch for a part asked for failed"
            );
            return Err("That attachment could not be downloaded — the server \
                        would not send it. Try again, or open the message \
                        on another device"
                .into());
        }
        if std::time::Instant::now() >= deadline {
            tracing::debug!(message = message.get(), "a part asked for never arrived");
            return Err("That part did not arrive in time — it is still \
                        downloading, so try again in a moment"
                .into());
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
}

fn fetch_failed(events: &EventStream, message: MessageId, part_id: &str) -> bool {
    let mut failed = false;
    while let Some(event) = events.try_next() {
        failed |= matches!(
            &event,
            Event::BodyFailed { message: m, sections, .. }
                if *m == message && sections.holds_part(part_id)
        );
    }
    failed
}

async fn part_path(
    database: &Store,
    message: MessageId,
    attachment: postio_model::ids::AttachmentId,
) -> Result<Option<String>, String> {
    Ok(read_message(database, message)
        .await?
        .attachments
        .iter()
        .find(|part| part.id == attachment)
        .and_then(|part| part.part_id.clone()))
}

/// Whether `part_id`'s bytes are on this machine, and in which shape; the part's own blob first, as
/// the exact bytes.
pub async fn locate_part(
    database: &Store,
    message: MessageId,
    part_id: &str,
) -> Result<Option<PartSource>, String> {
    let row = read_message(database, message).await?;
    if let Some(blob) = row
        .attachments
        .iter()
        .find(|part| part.part_id.as_deref() == Some(part_id))
        .and_then(|part| part.blob_id.clone())
    {
        return Ok(Some(PartSource::Payload(blob)));
    }
    Ok(row.raw_blob_id.map(PartSource::Raw))
}

async fn read_message(
    database: &Store,
    message: MessageId,
) -> Result<postio_model::Message, String> {
    let connection = database
        .connect()
        .await
        .map_err(|error| error.to_string())?;
    MessageRepository::new(&connection)
        .get(message)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "That message is no longer here".into())
}

#[cfg(test)]
mod wait_tests {
    use super::*;
    use postio_core::Event;
    use postio_core::bridge::EventHub;
    use postio_core::event::Sections;
    use postio_model::{Attachment, Message};
    use postio_storage::test_support;
    use std::time::{Duration, Instant};

    async fn message_with_an_unfetched_part() -> (Store, MessageId, postio_model::AccountId) {
        let store = test_support::memory().await;
        let connection = store.connect().await.expect("a connection");
        let (account, inbox) = test_support::account_with_inbox(&connection).await;
        let mut message = Message::new(account.id, inbox, chrono::Utc::now());
        let mut report = Attachment::new(MessageId::UNASSIGNED, "application/pdf", 19);
        report.filename = Some("report.pdf".into());
        report.part_id = Some("2".into());
        message.attachments.push(report);
        let id = MessageRepository::new(&connection)
            .create(&mut message)
            .await
            .expect("a message");
        (store, id, account.id)
    }

    #[tokio::test]
    async fn a_failed_fetch_ends_the_wait_at_once_and_says_it_failed() {
        let (store, message, account) = message_with_an_unfetched_part().await;
        let hub = EventHub::new();
        let failures = hub.subscribe("test");
        hub.sink().emit(Event::BodyFailed {
            account,
            message,
            sections: Sections::Parts(vec!["2".to_owned()]),
        });

        let started = Instant::now();
        let said = wait_for_part(
            &store,
            message,
            "2",
            Some(&failures),
            Duration::from_secs(30),
        )
        .await
        .err()
        .expect("a failed fetch is not a part");

        assert!(
            started.elapsed() < Duration::from_secs(5),
            "waited {:?} for a fetch that had already failed",
            started.elapsed()
        );
        assert!(said.contains("could not be downloaded"), "{said}");
        assert!(!said.contains("still downloading"), "{said}");
    }

    #[tokio::test]
    async fn another_messages_failure_does_not_end_the_wait() {
        let (store, message, account) = message_with_an_unfetched_part().await;
        let hub = EventHub::new();
        let failures = hub.subscribe("test");
        hub.sink().emit(Event::BodyFailed {
            account,
            message: MessageId::new(message.get() + 1000),
            sections: Sections::Parts(vec!["2".to_owned()]),
        });

        let said = wait_for_part(
            &store,
            message,
            "2",
            Some(&failures),
            Duration::from_millis(150),
        )
        .await
        .err()
        .expect("nothing arrived");
        assert!(said.contains("still downloading"), "{said}");
    }

    #[tokio::test]
    async fn a_refused_text_fetch_of_the_same_message_does_not_end_a_part_wait() {
        let (store, message, account) = message_with_an_unfetched_part().await;
        let hub = EventHub::new();
        let failures = hub.subscribe("test");
        for sections in [Sections::Text, Sections::Parts(vec!["3".to_owned()])] {
            hub.sink().emit(Event::BodyFailed {
                account,
                message,
                sections,
            });
        }
        let said = wait_for_part(
            &store,
            message,
            "2",
            Some(&failures),
            Duration::from_millis(150),
        )
        .await
        .err()
        .expect("nothing arrived");
        assert!(said.contains("still downloading"), "{said}");

        hub.sink().emit(Event::BodyFailed {
            account,
            message,
            sections: Sections::Whole,
        });
        let said = wait_for_part(
            &store,
            message,
            "2",
            Some(&failures),
            Duration::from_secs(30),
        )
        .await
        .err()
        .expect("a refusal is not a part");
        assert!(said.contains("could not be downloaded"), "{said}");
    }

    #[test]
    fn each_reason_a_part_was_not_queued_is_its_own_sentence() {
        let reasons = [
            PayloadAsk::Unreachable,
            PayloadAsk::NotOnServer,
            PayloadAsk::Never,
            PayloadAsk::Gone,
        ];
        let said: Vec<&str> = reasons.iter().map(|asked| not_fetched(*asked)).collect();
        for (asked, sentence) in reasons.iter().zip(&said) {
            assert!(
                !sentence.contains("still downloading"),
                "{asked:?}: {sentence}"
            );
        }
        assert!(said[0].contains("not connected"), "{}", said[0]);
        assert!(said[2].contains("never to download"), "{}", said[2]);
        let mut distinct = said.clone();
        distinct.sort_unstable();
        distinct.dedup();
        assert_eq!(distinct.len(), said.len(), "{said:?}");
    }

    #[tokio::test]
    async fn a_link_that_returns_does_not_say_the_server_refused() {
        let (store, message, account) = message_with_an_unfetched_part().await;
        let hub = EventHub::new();
        let failures = hub.subscribe("test");
        hub.sink().emit(Event::BodyReachable { account, message });

        let said = wait_for_part(
            &store,
            message,
            "2",
            Some(&failures),
            Duration::from_millis(150),
        )
        .await
        .err()
        .expect("nothing arrived");
        assert!(!said.contains("could not be downloaded"), "{said}");
    }

    #[tokio::test]
    async fn a_part_that_lands_is_returned_even_with_a_failure_heard() {
        let (store, message, account) = message_with_an_unfetched_part().await;
        let hub = EventHub::new();
        let failures = hub.subscribe("test");
        hub.sink().emit(Event::BodyFailed {
            account,
            message,
            sections: Sections::Parts(vec!["2".to_owned()]),
        });
        {
            let connection = store.connect().await.expect("a connection");
            let repository = MessageRepository::new(&connection);
            let mut row = repository.get(message).await.unwrap().unwrap();
            row.raw_blob_id = Some(postio_model::ids::BlobId::new("deadbeef"));
            repository.update(&mut row).await.expect("updated");
        }
        let source = wait_for_part(&store, message, "2", Some(&failures), BODY_WAIT).await;
        assert!(matches!(source, Ok(PartSource::Raw(_))));
    }
}
