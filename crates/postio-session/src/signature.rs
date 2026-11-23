use postio_model::{EmailAddress, MessageId};
use postio_signature::{Format, KeyInfo, KeySet, Policy, Reason, SignedScope, Status, Verdict};
use postio_storage::repository::{
    KeyKind, KeyRepository, KeySource, MessageRepository, NewKey, VerdictRow,
};
use postio_storage::{BlobStore, Checkout, Store, WritePriority};

const INLINE: &str = "-----BEGIN PGP SIGNED MESSAGE-----";

pub async fn check_messages(
    database: &Store,
    blobs: &BlobStore,
    messages: &[MessageId],
) -> postio_storage::Result<usize> {
    let connection = database.connect_background().await?;
    let mut changed: Vec<EmailAddress> = Vec::new();
    let mut checked = 0usize;
    for &message in messages {
        if let Some(from) = record_autocrypt(&connection, message).await? {
            changed.push(from);
        }
        checked += usize::from(check_one(&connection, blobs, message).await?);
    }
    changed.sort_by_key(EmailAddress::normalized);
    changed.dedup_by_key(|address| address.normalized());
    for address in &changed {
        let again = KeyRepository::new(&connection)
            .signed_messages_from(address)
            .await?;
        for message in again {
            checked += usize::from(check_one(&connection, blobs, message).await?);
        }
    }
    tracing::debug!(
        messages = messages.len(),
        checked,
        keys_changed = changed.len(),
        "signatures checked"
    );
    Ok(checked)
}

async fn record_autocrypt(
    connection: &Checkout,
    message: MessageId,
) -> postio_storage::Result<Option<EmailAddress>> {
    let messages = MessageRepository::new(connection);
    let Some(row) = messages.get(message).await? else {
        return Ok(None);
    };
    let Some(from) = row.from.first().cloned() else {
        return Ok(None);
    };
    let Some(headers) = messages.headers(message).await? else {
        return Ok(None);
    };
    let Some(info) = headers
        .get("Autocrypt")
        .and_then(|value| postio_signature::autocrypt(value, &from.address))
        .as_ref()
        .and_then(KeyInfo::of)
    else {
        return Ok(None);
    };
    let _permit = connection
        .write_gate()
        .acquire(WritePriority::Background)
        .await;
    KeyRepository::new(connection)
        .record(&NewKey {
            fingerprint: &info.fingerprint,
            kind: KeyKind::OpenPgp,
            keyblock: &info.keyblock,
            primary_uid: info.primary_uid.as_deref(),
            revoked: info.revoked,
            addresses: std::slice::from_ref(&from),
            source: KeySource::Autocrypt,
            message: Some(message),
            at: row.received_at.timestamp(),
        })
        .await?;
    Ok(Some(from))
}

async fn check_one(
    connection: &Checkout,
    blobs: &BlobStore,
    message: MessageId,
) -> postio_storage::Result<bool> {
    let messages = MessageRepository::new(connection);
    let Some(row) = messages.get(message).await? else {
        return Ok(false);
    };
    let Some(from) = row.from.first() else {
        return Ok(false);
    };
    let declared = match row.content_type.clone() {
        Some(kind) => Some(kind),
        None => messages.headers(message).await?.and_then(|headers| {
            headers
                .get("Content-Type")
                .and_then(|value| value.split(';').next())
                .map(|kind| kind.trim().to_ascii_lowercase())
        }),
    };
    let exact = declared.as_deref().is_some_and(needs_exact_bytes);
    let raw = row
        .raw_blob_id
        .as_ref()
        .and_then(|blob| blobs.get(blob).ok());
    let bytes = match (exact, raw) {
        (true, Some(raw)) => raw,
        (true, None) => {
            let absent = VerdictRow {
                format: if declared.as_deref() == Some("multipart/encrypted") {
                    "encrypted".to_owned()
                } else {
                    "pgp_mime".to_owned()
                },
                scope: "whole".to_owned(),
                status: "not_here_yet".to_owned(),
                reason: None,
                fingerprint: None,
                checked_at: chrono::Utc::now().timestamp(),
            };
            write(connection, message, &absent).await?;
            return Ok(true);
        }
        (false, raw) => {
            let text = messages
                .body(message)
                .await?
                .and_then(|stored| stored.text)
                .filter(|text| text.contains(INLINE));
            match (raw, text) {
                (Some(raw), Some(_)) => raw,
                (None, Some(text)) => {
                    format!("Content-Type: text/plain\r\n\r\n{text}").into_bytes()
                }
                (_, None) => return Ok(false),
            }
        }
    };
    let held = KeyRepository::new(connection).for_address(from).await?;
    let keys = KeySet::from_keyblocks(held.iter().map(|key| key.keyblock.as_slice()));
    let Some(verdict) = postio_signature::verify(&bytes, &keys, &Policy) else {
        return Ok(false);
    };
    write(connection, message, &row_of(&verdict)).await?;
    Ok(true)
}

async fn write(
    connection: &Checkout,
    message: MessageId,
    row: &VerdictRow,
) -> postio_storage::Result<()> {
    let _permit = connection
        .write_gate()
        .acquire(WritePriority::Background)
        .await;
    KeyRepository::new(connection)
        .put_verdict(message, row)
        .await
}

fn needs_exact_bytes(content_type: &str) -> bool {
    matches!(
        content_type.to_ascii_lowercase().as_str(),
        "multipart/signed"
            | "multipart/encrypted"
            | "application/pkcs7-mime"
            | "application/x-pkcs7-mime"
            | "application/pkcs7-signature"
    )
}

fn row_of(verdict: &Verdict) -> VerdictRow {
    let (format, scope) = match &verdict.scope {
        SignedScope::WholeMessage { format, .. } => (format_name(*format), "whole"),
        SignedScope::Part { format, .. } => (format_name(*format), "part"),
        SignedScope::Inline { whole: true, .. } => ("inline_pgp", "whole"),
        SignedScope::Inline { whole: false, .. } => ("inline_pgp", "inline"),
        SignedScope::Encrypted => ("encrypted", "encrypted"),
    };
    VerdictRow {
        format: format.to_owned(),
        scope: scope.to_owned(),
        status: match verdict.status {
            Status::Good => "good",
            Status::Bad => "bad",
            Status::CantCheck => "cant_check",
            Status::NotHereYet => "not_here_yet",
            Status::Revoked => "revoked",
        }
        .to_owned(),
        reason: verdict.reason.map(reason_name),
        fingerprint: verdict.signer.clone(),
        checked_at: chrono::Utc::now().timestamp(),
    }
}

fn format_name(format: Format) -> &'static str {
    match format {
        Format::PgpMime => "pgp_mime",
        Format::InlinePgp => "inline_pgp",
        Format::Smime => "smime",
    }
}

fn reason_name(reason: Reason) -> String {
    match reason {
        Reason::NoKey => "no_key".to_owned(),
        Reason::WeakHash(hash) => format!("weak_hash:{hash:?}").to_ascii_lowercase(),
        Reason::WeakKey => "weak_key".to_owned(),
        Reason::Expired => "expired".to_owned(),
        Reason::NotYetValid => "not_yet_valid".to_owned(),
        Reason::Unsupported => "unsupported".to_owned(),
        Reason::Malformed => "malformed".to_owned(),
        Reason::Tampered => "tampered".to_owned(),
    }
}

const CHECK_DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(500);

pub fn spawn_signature_checker(
    database: Store,
    blobs: BlobStore,
    events: Option<postio_core::bridge::EventStream>,
    runtime: &tokio::runtime::Handle,
) -> tokio::task::JoinHandle<()> {
    runtime.spawn(async move {
        let Some(events) = events else {
            return;
        };
        loop {
            let Some(event) = events.next().await else {
                return;
            };
            let mut pending = Vec::new();
            note(&mut pending, &event);
            let quiet = tokio::time::sleep(CHECK_DEBOUNCE);
            tokio::pin!(quiet);
            loop {
                tokio::select! {
                    () = &mut quiet => break,
                    next = events.next() => match next {
                        None => return,
                        Some(event) => note(&mut pending, &event),
                    },
                }
            }
            if pending.is_empty() {
                continue;
            }
            pending.sort_unstable();
            pending.dedup();
            if let Err(error) = check_messages(&database, &blobs, &pending).await {
                tracing::warn!(%error, messages = pending.len(), "the signature check failed: {error}");
            }
        }
    })
}

fn note(pending: &mut Vec<MessageId>, event: &postio_core::Event) {
    if let postio_core::Event::BodyLoaded { message, .. } = event {
        pending.push(*message);
    }
}
