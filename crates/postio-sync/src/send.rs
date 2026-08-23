//! Sending a [`Draft`](postio_model::Draft): build the message, hand it to
//! SMTP, file the local Sent copy.
//!
//! # Two phases, and why they are split
//!
//! [`resolve`] does everything a database read can answer: load the draft,
//! its account and identity, its attachments' bytes from the blob store, the
//! message it replies to if any, and build the outgoing RFC 5322 bytes. None
//! of that touches the network, so it can fail cheaply and locally — a
//! missing draft, an account with no identity to send as — before anything
//! is opened. [`send`] is what actually talks to a server: it is `async`,
//! `resolve` is not, and [`crate::drain::Drainer`] calls them from the two
//! different places in a drain pass that already made that split for every
//! other operation.
//!
//! # Once it is sent, it is sent
//!
//! The one rule this module exists to get right: after the SMTP transaction
//! is accepted, the message has reached the recipient's server and cannot be
//! recalled. Nothing past that point — the `APPEND` to Sent, writing the
//! local row, deleting the draft — may turn into [`Outcome::Failed`] or
//! [`Outcome::Retry`], because a caller that retries a "failed" send after
//! the transaction already succeeded delivers the message twice. Those steps
//! are best-effort bookkeeping instead: a failure among them is a resync's
//! job to reconcile, not a reason to send again. See [`send`].
//!
//! # A known gap
//!
//! A crash between the SMTP transaction succeeding and this module recording
//! that — vanishingly narrow, but real — can still leave a draft that
//! resends on the next drain. No transport-level idempotency key exists to
//! close it completely; the mitigation here is to do as little as possible
//! in that window; see [`send`].

use chrono::Utc;
use postio_imap::backend::{AppendMessage, MailBackend};
use postio_imap::secret::{AccountKey, SecretStore};
use postio_model::ids::{AccountId, DraftId};
use postio_model::{
    Attachment, Flag, FlagSet, MailboxId, MailboxRole, OutgoingAttachment, mime, outgoing,
};
use postio_smtp::cancel::CancelToken;
use postio_smtp::session::SmtpSession;
use postio_smtp::settings::ConnectionSettings;
use postio_smtp::transport::SmtpConnector;
use postio_storage::BlobStore;
use postio_storage::repository::{
    AccountRepository, BodyBlobs, DraftRepository, MailboxRepository, MessageRepository,
    ThreadingRepository,
};
use rusqlite::Connection;
use secrecy::SecretString;
use std::collections::BTreeSet;

use crate::backfill::put_text;
use crate::drain::{Outcome, Result};

/// What sending needs beyond the [`MailBackend`] every other operation
/// already has: an SMTP transport, the account's credentials, and the blob
/// store an attachment's bytes are read from and the sent copy is written
/// to.
#[derive(Debug)]
pub struct SmtpContext<'a> {
    /// Opens the connection a send is carried over.
    pub connector: &'a dyn SmtpConnector,
    /// Where the account's password is kept — the same one IMAP uses; see
    /// `postio_smtp::settings`.
    pub secrets: &'a dyn SecretStore,
    /// Where attachment bytes are read from, and the sent copy is written to.
    pub blobs: &'a BlobStore,
}

/// Everything [`send`] needs, resolved from local storage in [`resolve`] so
/// that nothing async is left to look up once the network is involved.
#[derive(Debug, Clone)]
pub(crate) struct SendJob {
    draft: DraftId,
    account: AccountId,
    account_address: String,
    outgoing: ConnectionSettings,
    from: String,
    recipients: Vec<String>,
    bcc: Vec<postio_model::EmailAddress>,
    attachments: Vec<Attachment>,
    raw: Vec<u8>,
    sent_mailbox: MailboxId,
    sent_mailbox_path: String,
}

/// What resolving a `Send` operation against local storage found.
pub(crate) enum ResolvedSend {
    /// Ready to hand to [`send`].
    Ready(Box<SendJob>),
    /// Nothing to send any more — the draft is gone.
    Obsolete(String),
    /// Cannot be sent and retrying will not change that.
    Impossible(String),
}

/// Resolves a `Send` operation for `draft_id`: loads the draft, its account
/// and identity, its attachments' bytes, and the message it replies to (if
/// any), then builds the outgoing message. Nothing here is async — every
/// input is a database row or a blob store read.
pub(crate) fn resolve(
    connection: &Connection,
    smtp: Option<&SmtpContext<'_>>,
    draft_id: DraftId,
) -> Result<ResolvedSend> {
    let Some(smtp) = smtp else {
        return Ok(ResolvedSend::Impossible(
            "no SMTP transport is configured".to_owned(),
        ));
    };

    let Some(draft) = DraftRepository::new(connection).get(draft_id)? else {
        return Ok(ResolvedSend::Obsolete(
            "the draft is no longer in the local store".to_owned(),
        ));
    };
    if !draft.has_recipients() {
        return Ok(ResolvedSend::Impossible(
            "the draft has no recipients".to_owned(),
        ));
    }

    let Some(account) = AccountRepository::new(connection).get(draft.account_id)? else {
        return Ok(ResolvedSend::Impossible(
            "the account is no longer in the local store".to_owned(),
        ));
    };

    let identity = draft
        .identity_id
        .and_then(|id| account.identities.iter().find(|identity| identity.id == id))
        .or_else(|| account.default_identity());
    let Some(identity) = identity else {
        return Ok(ResolvedSend::Impossible(
            "the account has no identity to send as".to_owned(),
        ));
    };

    let mut buffers = Vec::with_capacity(draft.attachments.len());
    for attachment in &draft.attachments {
        let Some(blob_id) = &attachment.blob_id else {
            return Ok(ResolvedSend::Impossible(format!(
                "the attachment {:?} has not finished uploading to the local store",
                attachment.display_name()
            )));
        };
        let content = smtp.blobs.get(blob_id)?;
        buffers.push((attachment.clone(), content));
    }
    let outgoing_attachments: Vec<OutgoingAttachment<'_>> = buffers
        .iter()
        .map(|(attachment, content)| OutgoingAttachment {
            attachment,
            content,
        })
        .collect();

    let parent = match draft.in_reply_to {
        Some(id) => MessageRepository::new(connection).get(id)?,
        None => None,
    };

    let built = outgoing::build(&draft, identity, &outgoing_attachments, parent.as_ref());

    let Some(sent) = MailboxRepository::new(connection).by_role(account.id, MailboxRole::Sent)?
    else {
        return Ok(ResolvedSend::Impossible(
            "this account has no Sent mailbox yet".to_owned(),
        ));
    };

    let recipients = draft
        .all_recipients()
        .map(|address| address.address.clone())
        .collect();
    let attachments = buffers
        .into_iter()
        .map(|(mut attachment, _)| {
            // A copy for the message this becomes, not the draft's own row.
            attachment.id = postio_model::ids::AttachmentId::UNASSIGNED;
            attachment.message_id = postio_model::ids::MessageId::UNASSIGNED;
            attachment
        })
        .collect();

    Ok(ResolvedSend::Ready(Box::new(SendJob {
        draft: draft_id,
        account: account.id,
        account_address: account.address.address.clone(),
        outgoing: ConnectionSettings::from_server_config(&account.outgoing),
        from: identity.address.address.clone(),
        recipients,
        bcc: draft.bcc.clone(),
        attachments,
        raw: built.raw,
        sent_mailbox: sent.id,
        sent_mailbox_path: sent.path,
    })))
}

/// Sends `job` over SMTP, then files the local Sent copy.
///
/// See the [module docs](self) for why nothing past the SMTP transaction
/// accepting may become [`Outcome::Failed`] or [`Outcome::Retry`] — from
/// there on this always returns [`Outcome::Applied`], and `resync` is used
/// only to flag the Sent mailbox for reconciliation, never to fail the step.
pub(crate) async fn send(
    connection: &Connection,
    backend: &dyn MailBackend,
    smtp: &SmtpContext<'_>,
    resync: &mut BTreeSet<i64>,
    job: &SendJob,
) -> Outcome {
    let key = AccountKey::new(&job.account_address);
    let password = match smtp.secrets.retrieve(&key).await {
        Ok(password) => password,
        Err(error) => {
            return Outcome::Failed {
                reason: format!(
                    "could not read the password for {}: {error}",
                    job.account_address
                ),
            };
        }
    };
    let password = SecretString::from(password.expose().to_owned());

    let mut session = match SmtpSession::open(&job.outgoing, &password, smtp.connector).await {
        Ok(session) => session,
        Err(error) => return outcome_from_smtp_error(error),
    };

    let cancel = CancelToken::new();
    if let Err(error) = session
        .send_message(&job.from, &job.recipients, &job.raw, &cancel)
        .await
    {
        return outcome_from_smtp_error(error);
    }

    // Delivered. Everything from here is best-effort: it may not turn this
    // outcome into anything but Applied.
    let _ = session.quit().await;
    file_sent_copy(connection, backend, smtp, resync, job).await;
    Outcome::Applied
}

/// Records the send locally: appends to the server's Sent folder, writes the
/// local row and its blobs, threads it, and drops the draft.
///
/// Every step tolerates the previous one failing, because none of them may
/// cause a resend. An `APPEND` failure flags the Sent mailbox for resync
/// instead — the message still gets a local row, so nothing is lost from
/// the user's own view of Postio even if the row has no server `UID` until
/// a later sync reconciles it.
async fn file_sent_copy(
    connection: &Connection,
    backend: &dyn MailBackend,
    smtp: &SmtpContext<'_>,
    resync: &mut BTreeSet<i64>,
    job: &SendJob,
) {
    let mut flags = FlagSet::new();
    flags.insert(Flag::Seen);

    let append = AppendMessage::new(job.raw.clone()).with_flags(flags.clone());
    let mapping = backend
        .append(&job.sent_mailbox_path, &append)
        .await
        .ok()
        .flatten();
    if mapping.is_none() {
        resync.insert(job.sent_mailbox.get());
    }

    let mut message = mime::parse(&job.raw).into_message(job.account, job.sent_mailbox, Utc::now());
    message.flags = flags;
    message.bcc = job.bcc.clone();
    message.attachments = job.attachments.clone();
    message.raw_blob_id = smtp.blobs.put(&job.raw).ok();
    if let Some(mapping) = mapping {
        message.server.uid = Some(mapping.destination);
        message.server.uid_validity = Some(mapping.uid_validity);
    }

    let messages = MessageRepository::new(connection);
    if messages.create(&mut message).is_err() {
        return;
    }
    let _ = ThreadingRepository::new(connection, job.account).thread(&message);
    // The list and the sidebar read this cached count rather than counting
    // rows, so a message filed without recomputing it would stay invisible
    // in Sent until an unrelated resync happened to recompute it.
    let _ = MailboxRepository::new(connection).recount(job.sent_mailbox);

    let blobs = BodyBlobs {
        text: put_text(smtp.blobs, message.body.text.as_deref()).unwrap_or_default(),
        html: put_text(smtp.blobs, message.body.html.as_deref()).unwrap_or_default(),
        headers: None,
    };
    let _ = messages.set_body_blobs(message.id, &blobs, postio_model::BodyState::Full);

    let _ = DraftRepository::new(connection).delete(job.draft);
}

/// Classifies an [`SmtpError`](postio_smtp::error::SmtpError) the same way
/// [`crate::drain::Outcome::from_error`] classifies a `BackendError`: `4xx`
/// and transport trouble is worth retrying, everything else is not.
fn outcome_from_smtp_error(error: postio_smtp::error::SmtpError) -> Outcome {
    let reason = error.to_string();
    if error.is_transient() {
        return Outcome::Retry {
            reason,
            after: None,
        };
    }
    Outcome::Failed { reason }
}
