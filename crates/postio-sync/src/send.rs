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
use postio_imap::auth::{TokenSource, with_credential};
use postio_imap::backend::{AppendMessage, MailBackend};
use postio_imap::secret::AccountKey;
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
    /// Where the account's credential comes from — **the same instance the
    /// IMAP pool holds** (ADR 0006 Q5).
    ///
    /// One per account, not one per connection. Sharing is what makes a
    /// rejection seen by one side visible to the other, and what stops two
    /// simultaneous refreshes on a provider that rotates its refresh token —
    /// where the second one invalidates the first.
    ///
    /// A `TokenSource` rather than a `SecretStore` because for an OAuth
    /// account the keyring holds no password at all under the account's own
    /// key: it holds a refresh token under a different one, and the
    /// credential to present is minted from it.
    pub tokens: &'a dyn TokenSource,
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
    /// The draft's copy in the Drafts mailbox, when it reached the server —
    /// resolved here because after the send the local row that knew about it
    /// is deleted, and a draft left behind in Drafts is the user's sent
    /// message showing up as still unfinished.
    drafts_copy: Option<(MailboxId, String, crate::drafts::ServerCopy)>,
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

    // Resolved before the send, like everything else here: after the SMTP
    // transaction nothing may fail, so nothing may still need looking up.
    let drafts_copy = match crate::drafts::server_copy(&draft) {
        Some(copy) => MailboxRepository::new(connection)
            .by_role(account.id, MailboxRole::Drafts)?
            .map(|mailbox| (mailbox.id, mailbox.path, copy)),
        None => None,
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
        drafts_copy,
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
    // The same invalidate-and-try-once-more the IMAP pool keeps, from the
    // same place (ADR 0006 Q5). An access token that expired between the last
    // IMAP command and this send is the ordinary case, not the edge — and
    // this side is where the *user* is watching, because a send that failed
    // is a message that did not go.
    let opened = with_credential(
        smtp.tokens,
        &key,
        postio_smtp::error::SmtpError::is_authentication_failure,
        |credential| async move {
            // Exactly one `to_owned`, for the reason `postio_imap`'s own
            // credential copy gives: `SecretString::from` goes through
            // `String::into_boxed_str`, which reallocates whenever capacity
            // exceeds length and frees the buffer holding the password
            // without overwriting it. `str::to_owned` allocates exactly
            // `len`, so this moves instead. #144.
            let password = SecretString::from(credential.expose().to_owned());
            SmtpSession::open(&job.outgoing, &password, smtp.connector).await
        },
    )
    .await;

    let mut session = match opened {
        Ok(Ok(session)) => session,
        Ok(Err(error)) => return outcome_from_smtp_error(error),
        Err(error) => {
            return Outcome::Failed {
                reason: format!(
                    "could not read the credential for {}: {error}",
                    job.account_address
                ),
            };
        }
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
    // No recount needed here: migration 0003's `messages_count_insert`
    // trigger already updated Sent's cached counts the instant `create`
    // inserted the row. A call here would only redo what the trigger just
    // did -- see `MailboxRepository::recount`'s own docs. postio-qhz.8.

    let blobs = BodyBlobs {
        text: put_text(smtp.blobs, message.body.text.as_deref()).unwrap_or_default(),
        html: put_text(smtp.blobs, message.body.html.as_deref()).unwrap_or_default(),
        headers: None,
    };
    let _ = messages.set_body_blobs(message.id, &blobs, postio_model::BodyState::Full);

    let _ = DraftRepository::new(connection).delete(job.draft);
    remove_drafts_copy(backend, resync, job).await;
}

/// Takes the draft's copy out of the Drafts mailbox now that it has been sent.
///
/// Best-effort like everything else after the transaction: a copy left behind
/// is the user's sent message still showing as unfinished on their phone,
/// which a resync of that folder reconciles — and which is a great deal better
/// than a second delivery.
async fn remove_drafts_copy(backend: &dyn MailBackend, resync: &mut BTreeSet<i64>, job: &SendJob) {
    let Some((mailbox, path, copy)) = &job.drafts_copy else {
        return;
    };
    let Ok(capabilities) = backend.capabilities().await else {
        resync.insert(mailbox.get());
        return;
    };
    match crate::drafts::remove(backend, &capabilities, path, *copy).await {
        // Removed, or somebody else got there first.
        Ok(crate::drafts::Removal::Removed | crate::drafts::Removal::Gone) => {}
        _ => {
            resync.insert(mailbox.get());
        }
    }
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
