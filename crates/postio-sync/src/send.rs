//! Sends a `Draft`: `resolve` is sync and local (fails cheaply before anything is opened), `send`
//! talks to the server.
//! Once SMTP accepts, nothing may become `Failed` or `Retry`: a retry would deliver twice. Later
//! steps are best-effort and a resync reconciles.
//! ADR 0021 commit point: `Sending` is committed before the transaction opens and `Sent` the
//! instant `send_message` returns `Ok`; `resolve` reads both back.
//! Every attempt carries the `Message-ID` minted by `DraftRepository::queue_send`, so Postio
//! recognises its own message when it comes back (#674). At-most-once submission.

use chrono::Utc;
use postio_account::auth::{TokenSource, with_credential};
use postio_account::backend::{AppendMessage, MailBackend};
use postio_account::secret::AccountKey;
use postio_model::ids::{AccountId, DraftId, LabelId, MessageId};
use postio_model::{
    Attachment, DraftState, Flag, FlagSet, MailboxId, MailboxRole, Message, Operation,
    OperationTarget, OutgoingAttachment, mime, outgoing,
};
use postio_smtp::cancel::CancelToken;
use postio_smtp::session::SmtpSession;
use postio_smtp::settings::ConnectionSettings;
use postio_smtp::transport::SmtpConnector;
use postio_storage::BlobStore;
use postio_storage::Connection;
use postio_storage::repository::{
    AccountRepository, ContactRepository, ContactSuggestionRepository, CorrespondentRepository,
    DraftRepository, FlagSource, LabelRepository, MailboxRepository, MessageRepository,
    OperationQueueRepository, StoredBody, ThreadRepository, ThreadingRepository,
};
use secrecy::SecretString;
use std::collections::BTreeSet;

use crate::backfill::stored_text;
use crate::drain::{Outcome, Result};

#[derive(Debug)]
pub struct SmtpContext<'a> {
    pub connector: &'a dyn SmtpConnector,
    /// The same instance the IMAP pool holds (ADR 0006 Q5): one per account, so a rejection is seen
    /// by both sides and two refreshes cannot race on a provider that rotates refresh tokens. A
    /// `TokenSource` because an OAuth account's keyring entry is a refresh token.
    pub tokens: &'a dyn TokenSource,
    pub blobs: &'a BlobStore,
}

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
    /// The draft's Drafts-folder copy, resolved here because the local row that knew it is deleted
    /// after the send.
    drafts_copy: Option<(MailboxId, String, postio_model::RemoteId)>,
    /// Chosen labels and their keyword names, resolved here because the draft is deleted by the
    /// time they are applied.
    labels: Vec<(LabelId, String)>,
    /// When the "remind if no reply" reminder falls due, set as the sent copy is filed.
    remind_at: Option<chrono::DateTime<Utc>>,
}

pub(crate) enum ResolvedSend {
    Ready(Box<SendJob>),
    Obsolete(String),
    Impossible(String),
    /// A fresh process found a draft `Sending` or `Unconfirmed`: the previous attempt died inside
    /// the window ADR 0021 cannot close, and retrying risks a duplicate.
    Uncertain(String),
}

pub(crate) async fn resolve(
    connection: &Connection,
    smtp: Option<&SmtpContext<'_>>,
    draft_id: DraftId,
) -> Result<ResolvedSend> {
    let Some(smtp) = smtp else {
        return Ok(ResolvedSend::Impossible(
            "no SMTP transport is configured".to_owned(),
        ));
    };

    let Some(draft) = DraftRepository::new(connection).get(draft_id).await? else {
        return Ok(ResolvedSend::Obsolete(
            "the draft is no longer in the local store".to_owned(),
        ));
    };
    // The commit point read back (ADR 0021): a process that died after SMTP accepted stops here
    // instead of delivering twice.
    if draft.state == DraftState::Sent {
        return Ok(ResolvedSend::Obsolete(
            "the submission server already accepted this message".to_owned(),
        ));
    }
    // The transaction was open when the process stopped: whether the payload arrived is unknowable,
    // and a duplicate cannot be recalled.
    // Its own outcome, not a failure (#674): "failed" claims more than is known. The draft carries
    // it so the answer survives.
    if draft.state == DraftState::Sending || draft.state == DraftState::Unconfirmed {
        // Promote the transient `Sending` mark on the way past: its process is gone, and leaving it
        // would show a spinner in Drafts for ever. `Unconfirmed` is the resting state (#674).
        if draft.state == DraftState::Sending {
            DraftRepository::new(connection)
                .set_state(draft.id, DraftState::Unconfirmed)
                .await?;
        }
        return Ok(ResolvedSend::Uncertain(INDETERMINATE.to_owned()));
    }
    if !draft.has_recipients() {
        return Ok(ResolvedSend::Impossible(
            "the draft has no recipients".to_owned(),
        ));
    }

    let Some(account) = AccountRepository::new(connection)
        .get(draft.account_id)
        .await?
    else {
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
            // A forward's carried part was fetched before this (`crate::carry`), so what is missing
            // is a file the composer never finished writing.
            return Ok(ResolvedSend::Impossible(format!(
                "the attachment {} was never finished being written to this machine",
                crate::carry::describe(attachment)
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
        Some(id) => MessageRepository::new(connection).get(id).await?,
        None => None,
    };

    let calendar = draft
        .calendar_reply
        .as_deref()
        .map(|ics| outgoing::CalendarPart {
            method: outgoing::CalendarMethod::Reply,
            ics: ics.as_bytes(),
        });
    let built = outgoing::build(
        &draft,
        identity,
        &outgoing_attachments,
        parent.as_ref(),
        calendar,
    );

    let Some(sent) = MailboxRepository::new(connection)
        .by_role(account.id, MailboxRole::Sent)
        .await?
    else {
        return Ok(ResolvedSend::Impossible(
            "this account has no Sent mailbox yet".to_owned(),
        ));
    };

    // Resolved before the send: after the SMTP transaction nothing may fail, so nothing may still
    // need looking up.
    let drafts_copy = match crate::drafts::server_copy(&draft) {
        Some(copy) => MailboxRepository::new(connection)
            .by_role(account.id, MailboxRole::Drafts)
            .await?
            .map(|mailbox| (mailbox.id, mailbox.path, copy)),
        None => None,
    };

    let mut labels = Vec::with_capacity(draft.labels.len());
    for id in &draft.labels {
        if let Some(label) = LabelRepository::new(connection).get(*id).await?
            && label.account_id == account.id
        {
            labels.push((label.id, label.name));
        }
    }

    let recipients = draft
        .all_recipients()
        .map(|address| address.address.clone())
        .collect();
    let attachments = buffers
        .into_iter()
        .map(|(mut attachment, _)| {
            attachment.id = postio_model::ids::AttachmentId::UNASSIGNED;
            attachment.message_id = postio_model::ids::MessageId::UNASSIGNED;
            attachment
        })
        .collect();

    Ok(ResolvedSend::Ready(Box::new(SendJob {
        draft: draft_id,
        account: account.id,
        account_address: account.address.address.clone(),
        // `.with_auth`, or the stored mechanism dies here as it did in the IMAP engine (#533).
        outgoing: ConnectionSettings::from_server_config(&account.outgoing).with_auth(account.auth),
        from: identity.address.address.clone(),
        recipients,
        bcc: draft.bcc.clone(),
        attachments,
        raw: built.raw,
        sent_mailbox: sent.id,
        sent_mailbox_path: sent.path,
        drafts_copy,
        labels,
        remind_at: draft.remind_at,
    })))
}

/// Sends `job` over SMTP, then files the local Sent copy. After the SMTP transaction accepts this
/// always returns `Applied`; `resync` only flags Sent for reconciliation.
pub(crate) async fn send(
    connection: &Connection,
    backend: &dyn MailBackend,
    smtp: &SmtpContext<'_>,
    resync: &mut BTreeSet<i64>,
    threaded: &mut crate::threaded::ThreadArrivals,
    job: &SendJob,
) -> Outcome {
    // Local-first (#942): the Sent row is written before the socket opens, so the message is
    // visible while on its way.
    let filed = file_sent_locally(connection, smtp, job).await;
    let outcome = submit(
        connection,
        backend,
        smtp,
        resync,
        threaded,
        job,
        filed.clone(),
    )
    .await;
    // `Failed` is the one outcome ADR 0021 lets say "nothing was delivered"; `Retry` is in progress
    // and `Uncertain` may have gone, so Sent keeps the row for both.
    if matches!(outcome, Outcome::Failed { .. }) {
        unfile_sent_copy(connection, filed.as_ref()).await;
    } else if let Some(sent) = &filed
        && let Some(thread) = sent.thread_id
    {
        threaded.add(thread, sent.id);
    }
    if matches!(outcome, Outcome::Applied)
        && let Some(copy) = filed.as_ref()
    {
        save_who_was_written_to(connection, job.account, copy).await;
    }
    outcome
}

async fn submit(
    connection: &Connection,
    backend: &dyn MailBackend,
    smtp: &SmtpContext<'_>,
    resync: &mut BTreeSet<i64>,
    threaded: &mut crate::threaded::ThreadArrivals,
    job: &SendJob,
    filed: Option<Message>,
) -> Outcome {
    let key = AccountKey::new(&job.account_address);
    // The IMAP pool's invalidate-and-retry-once (ADR 0006 Q5): an access token that expired since
    // the last IMAP command is the ordinary case.
    let opened = with_credential(
        smtp.tokens,
        &key,
        postio_smtp::error::SmtpError::is_authentication_failure,
        |credential| async move {
            // Exactly one `to_owned`: `SecretString::from` reallocates when capacity exceeds length
            // and frees the password buffer unwiped; `str::to_owned` allocates exactly `len`
            // (#144).
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

    // ADR 0021's first mark, taken here: connect and auth are retryable and leave `Queued`, while
    // everything past this may put a payload on the wire.
    if let Err(error) = mark(connection, job, DraftState::Sending).await {
        // Nothing submitted yet, so safe to retry; it must be a refusal, since sending without the
        // mark reopens the crash window.
        return Outcome::Retry {
            reason: format!("could not record that the send had started: {error}"),
            after: None,
        };
    }

    let cancel = CancelToken::new();
    if let Err(error) = session
        .send_message(&job.from, &job.recipients, &job.raw, &cancel)
        .await
    {
        if error.submission_is_indeterminate() {
            // The server went silent once the payload was going out: leave the mark where it is,
            // since "try again" and "you already sent it" are indistinguishable.
            // The draft says so durably (`Unconfirmed`); a failed write is recorded, never a reason
            // to retry the send.
            if let Err(error) = mark(connection, job, DraftState::Unconfirmed).await {
                tracing::error!(
                    %error,
                    "could not record that a send was left unconfirmed; the \
                     draft stays in Sending, which still refuses to resend it: \
                     {error}"
                );
            }
            return Outcome::Uncertain {
                reason: INDETERMINATE.to_owned(),
            };
        }
        // The server refused, so nothing was accepted: the mark comes off or `resolve` would refuse
        // the backoff's retry.
        release(connection, job).await;
        return outcome_from_smtp_error(error);
    }

    // Delivered; the second mark is the next thing, ahead of `QUIT`, the `APPEND` and the local
    // row, so a crash window is one local commit (ADR 0021).
    // A failure here can only become `Applied`: the message has gone.
    let _ = mark(connection, job, DraftState::Sent).await;

    let _ = session.quit().await;
    confirm_sent_copy(connection, backend, smtp, resync, threaded, job, filed).await;
    Outcome::Applied
}

/// Shown for a send nobody can determine, by `send` and by `resolve` on a dead mark, so the two
/// cannot describe it differently. Leaves the question open: it is not a failure.
pub(crate) const INDETERMINATE: &str = "this send was interrupted while it was being submitted, \
     so it may or may not have arrived — Postio will not send it again on its own";

/// The calls in `send` are ADR 0021's second decision: the only writes here whose ordering is the
/// guarantee.
async fn mark(
    connection: &Connection,
    job: &SendJob,
    state: DraftState,
) -> postio_storage::Result<()> {
    DraftRepository::new(connection)
        .set_state(job.draft, state)
        .await
}

/// Best-effort and safe: a failed release leaves `Sending`, which `resolve` refuses rather than
/// resends.
async fn release(connection: &Connection, job: &SendJob) {
    if let Err(error) = mark(connection, job, DraftState::Queued).await {
        tracing::warn!(%error, "could not return an unsent draft to the queue");
    }
}

/// Writes the Sent row before anything touches the network (#942): send used to write it after SMTP
/// and `APPEND`, so Sent was empty in between.
/// The row has no server identity until `confirm_sent_copy` attaches one; the storage identity
/// fallback lets a Sent resync adopt it.
/// Best-effort: on failure `confirm_sent_copy` writes the row at the end.
async fn file_sent_locally(
    connection: &Connection,
    smtp: &SmtpContext<'_>,
    job: &SendJob,
) -> Option<Message> {
    let mut message = mime::parse(&job.raw).into_message(job.account, job.sent_mailbox, Utc::now());
    message.flags = job.sent_flags();
    message.labels = job.label_ids();
    message.bcc = job.bcc.clone();
    message.attachments = job.attachments.clone();
    message.raw_blob_id = smtp.blobs.put(&job.raw).ok();

    let messages = MessageRepository::new(connection);
    if messages.create(&mut message).await.is_err() {
        return None;
    }
    // Counted as written to now, before the network (spec 007, T075); `unfile_sent_copy` takes it
    // back on failure.
    record_correspondents(connection, job.account, message.id).await;
    // The thread id must come back onto the struct (#1488): `confirm_sent_copy` later calls
    // `update` from it, and a copy captured before threading would un-thread the reply.
    match ThreadingRepository::new(connection, job.account)
        .thread(&message)
        .await
    {
        Ok(threaded) => message.thread_id = Some(threaded.thread_id),
        // Not fatal, but logged: the silence let #1488 go unnoticed.
        Err(error) => tracing::warn!(%error, "could not thread the sent copy"),
    }
    label_the_conversation(connection, job, &message).await;
    remind_on_the_conversation(connection, job, &message).await;

    // The header block of what was sent, so `header:` finds mail this client composed (#884).
    let block = postio_model::headers::block_of(&job.raw);
    let body = StoredBody {
        text: stored_text(message.body.text.as_deref()),
        html: stored_text(message.body.html.as_deref()),
        headers: block.as_ref().map(|block| block.text.clone()),
        headers_truncated: block.as_ref().is_some_and(|block| block.truncated),
        encoding_problems: false,
    };
    let _ = messages
        .set_body(message.id, &body, postio_model::BodyState::Full)
        .await;
    Some(message)
}

impl SendJob {
    fn sent_flags(&self) -> FlagSet {
        std::iter::once(Flag::Seen)
            .chain(
                self.labels
                    .iter()
                    .map(|(_, name)| Flag::Keyword(name.clone())),
            )
            .collect()
    }

    fn label_ids(&self) -> Vec<LabelId> {
        self.labels.iter().map(|(id, _)| *id).collect()
    }
}

/// Labels the rest of the sent copy's conversation. Best-effort: a failed label never fails or
/// repeats a send.
async fn label_the_conversation(connection: &Connection, job: &SendJob, copy: &Message) {
    if job.labels.is_empty() {
        return;
    }
    let Some(thread) = copy.thread_id else {
        return;
    };
    if let Err(error) = label_members(connection, job, thread, copy.id).await {
        tracing::warn!(%error, "could not label a sent message's conversation");
    }
}

/// Sets the reminder the draft asked for (FR-044) on the sent copy's conversation, locally before
/// the network. Best-effort.
async fn remind_on_the_conversation(connection: &Connection, job: &SendJob, copy: &Message) {
    let (Some(due), Some(thread)) = (job.remind_at, copy.thread_id) else {
        return;
    };
    if let Err(error) = postio_storage::repository::ReminderRepository::new(connection)
        .set(thread, copy.id, due, Utc::now())
        .await
    {
        tracing::warn!(%error, "could not set a sent message's reminder");
    }
}

async fn label_members(
    connection: &Connection,
    job: &SendJob,
    thread: postio_model::ThreadId,
    copy: MessageId,
) -> postio_storage::Result<()> {
    let members = ThreadRepository::new(connection).member_ids(thread).await?;
    let messages = MessageRepository::new(connection);
    let labels = LabelRepository::new(connection);
    let queue = OperationQueueRepository::new(connection);
    let at = Utc::now();
    for member in members.into_iter().filter(|member| *member != copy) {
        let Some(message) = messages.get(member).await? else {
            continue;
        };
        let carried = labels.for_message(member).await?;
        let mut flags = message.flags.clone();
        let mut added = FlagSet::new();
        for (label, name) in &job.labels {
            if carried.contains(label) {
                continue;
            }
            labels.attach(member, *label).await?;
            let keyword = Flag::Keyword(name.clone());
            flags.insert(keyword.clone());
            added.insert(keyword);
        }
        if added.is_empty() {
            continue;
        }
        messages
            .set_flags(member, &flags, FlagSource::Local)
            .await?;
        queue
            .enqueue(
                job.account,
                OperationTarget::Message(member),
                &Operation::SetFlags { flags: added },
                at,
            )
            .await?;
    }
    Ok(())
}

/// Logged and not returned: a failure here never fails or repeats a send.
async fn record_correspondents(connection: &Connection, account: AccountId, message: MessageId) {
    if let Err(error) = CorrespondentRepository::new(connection)
        .record_sent(account, &[message])
        .await
    {
        tracing::warn!(%error, "could not record who a sent message went to");
    }
}

/// With "Save people I write to, automatically" on, saves everyone `copy` went to (To, Cc and Bcc)
/// as contacts; off, does nothing, not even a count.
/// Logged and not returned.
async fn save_who_was_written_to(connection: &Connection, account: AccountId, copy: &Message) {
    let on = match ContactSuggestionRepository::new(connection).get().await {
        Ok(suggestions) => suggestions.is_on(postio_model::Suggestion::SaveAutomatically),
        Err(error) => {
            tracing::warn!(%error, "could not read the contact switches");
            return;
        }
    };
    if !on {
        return;
    }
    let Ok(Some(account)) = AccountRepository::new(connection).get(account).await else {
        return;
    };
    if let Err(error) = crate::contacts::record(connection, &account, copy).await {
        tracing::warn!(%error, "could not record the people a send wrote to");
        return;
    }
    match ContactRepository::new(connection).save_written_to().await {
        Ok(saved) if saved > 0 => tracing::info!(saved, "saved the people a send wrote to"),
        Ok(_) => {}
        Err(error) => tracing::warn!(%error, "could not save the people a send wrote to"),
    }
}

/// Takes back the Sent row after a failed send. Only for `Outcome::Failed` (ADR 0021): `Retry`
/// keeps it, and `Uncertain` keeps it because it may have gone (#674).
async fn unfile_sent_copy(connection: &Connection, filed: Option<&Message>) {
    let Some(message) = filed else {
        return;
    };
    if let Err(error) = CorrespondentRepository::new(connection)
        .unrecord_sent(message.account_id, message.id)
        .await
    {
        tracing::warn!(
            %error,
            "could not take back who a failed send was counted as written to"
        );
    }
    if let Err(error) = MessageRepository::new(connection)
        .delete(&[message.id])
        .await
    {
        tracing::warn!(
            %error,
            "could not remove the Sent copy of a send that failed; it will \
             show as sent until the folder is resynced"
        );
    }
}

/// Attaches the server's identity from the `APPEND` to the row `file_sent_locally` wrote, or writes
/// the row if the early write failed.
async fn confirm_sent_copy(
    connection: &Connection,
    backend: &dyn MailBackend,
    smtp: &SmtpContext<'_>,
    resync: &mut BTreeSet<i64>,
    threaded: &mut crate::threaded::ThreadArrivals,
    job: &SendJob,
    filed: Option<Message>,
) {
    let flags = job.sent_flags();

    let append = AppendMessage::new(job.raw.clone()).with_flags(flags.clone());
    let mapping = backend
        .append(&job.sent_mailbox_path, &append)
        .await
        .ok()
        .flatten();
    if mapping.is_none() {
        resync.insert(job.sent_mailbox.get());
    }

    let messages = MessageRepository::new(connection);
    match filed {
        Some(mut message) => {
            if let Some(mapping) = mapping {
                message.server.uid = Some(mapping.destination);
                message.server.uid_validity = Some(mapping.uid_validity);
                message.server.remote_id = Some(mapping.destination_remote_id());
                if let Err(error) = messages.update(&mut message).await {
                    tracing::warn!(
                        %error,
                        "could not record where the Sent copy landed; a resync \
                         of Sent adopts the row by its Message-ID instead"
                    );
                    resync.insert(job.sent_mailbox.get());
                }
            }
        }
        None => {
            let mut message =
                mime::parse(&job.raw).into_message(job.account, job.sent_mailbox, Utc::now());
            message.flags = flags;
            message.labels = job.label_ids();
            message.bcc = job.bcc.clone();
            message.attachments = job.attachments.clone();
            message.raw_blob_id = smtp.blobs.put(&job.raw).ok();
            if let Some(mapping) = mapping {
                message.server.uid = Some(mapping.destination);
                message.server.uid_validity = Some(mapping.uid_validity);
                message.server.remote_id = Some(mapping.destination_remote_id());
            }
            if messages.create(&mut message).await.is_err() {
                return;
            }
            record_correspondents(connection, job.account, message.id).await;
            if let Ok(joined) = ThreadingRepository::new(connection, job.account)
                .thread(&message)
                .await
            {
                message.thread_id = Some(joined.thread_id);
                threaded.add(joined.thread_id, message.id);
            }
            label_the_conversation(connection, job, &message).await;
            remind_on_the_conversation(connection, job, &message).await;
            let block = postio_model::headers::block_of(&job.raw);
            let body = StoredBody {
                text: stored_text(message.body.text.as_deref()),
                html: stored_text(message.body.html.as_deref()),
                headers: block.as_ref().map(|block| block.text.clone()),
                headers_truncated: block.as_ref().is_some_and(|block| block.truncated),
                encoding_problems: false,
            };
            let _ = messages
                .set_body(message.id, &body, postio_model::BodyState::Full)
                .await;
        }
    }

    let _ = DraftRepository::new(connection).delete(job.draft).await;
    remove_drafts_copy(backend, resync, job).await;
}

/// Best-effort: a copy left behind is reconciled by a resync of that folder, which beats a second
/// delivery.
async fn remove_drafts_copy(backend: &dyn MailBackend, resync: &mut BTreeSet<i64>, job: &SendJob) {
    let Some((mailbox, path, copy)) = &job.drafts_copy else {
        return;
    };
    let Ok(capabilities) = backend.capabilities().await else {
        resync.insert(mailbox.get());
        return;
    };
    match crate::drafts::remove(backend, &capabilities, path, copy).await {
        Ok(crate::drafts::Removal::Removed | crate::drafts::Removal::Gone) => {}
        _ => {
            resync.insert(mailbox.get());
        }
    }
}

/// Classified as `Outcome::from_error` does: `4xx` and transport trouble retry, everything else
/// does not.
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

/// Resolves every `Unconfirmed` draft whose message has since turned up (ADR 0021 Decision 3,
/// #674): the message carries the `Message-ID` the draft reserved (#461).
/// Run after any pass, since the server chooses where a copy is filed; the cost is one indexed read
/// per unconfirmed draft.
/// Returns the drafts resolved. A draft with no reserved id is left alone: guessing from subject
/// and recipients names the wrong message.
pub async fn confirm_unconfirmed(
    connection: &Connection,
    account: AccountId,
) -> Result<Vec<(DraftId, postio_model::ids::MessageId)>> {
    let drafts = DraftRepository::new(connection);
    let messages = MessageRepository::new(connection);
    let mut resolved = Vec::new();

    for draft in drafts.by_state(DraftState::Unconfirmed).await? {
        if draft.account_id != account {
            continue;
        }
        let Some(reserved) = draft.rfc_message_id.as_ref() else {
            continue;
        };
        // Scoped to the account: the same `Message-ID` in another account is not evidence this
        // submission succeeded.
        let Some(message) = messages
            .ids_by_rfc_message_id(account, reserved)
            .await?
            .into_iter()
            .next()
        else {
            continue;
        };
        drafts.set_state(draft.id, DraftState::Sent).await?;
        drafts.set_synced_message(draft.id, message).await?;
        tracing::info!(
            draft = draft.id.get(),
            "an unconfirmed send turned up in a sync; it did arrive"
        );
        resolved.push((draft.id, message));
    }
    Ok(resolved)
}
