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
//! # The commit point, and what it replaced
//!
//! That rule protects a caller who is *told* the send failed. It said nothing
//! about a process that is never told anything, and this module's own docs
//! used to call that gap "vanishingly narrow". It was not.
//!
//! The durable fact that stopped a resend was the **deletion of the draft
//! row**, because [`resolve`] reads a missing draft as obsolete. That deletion
//! is the second-to-last thing [`confirm_sent_copy`] does — behind `QUIT`, an
//! `APPEND` of the whole message to the Sent mailbox, a blob write, a
//! `messages.create`, threading and a body write. On a slow link the `APPEND`
//! alone is seconds. It was the largest single piece of network work in the
//! send path, and every moment of it was a window in which a crash meant the
//! recipient got the message twice.
//!
//! ADR 0021 moves the commit ahead of all of it. [`send`] takes two marks:
//!
//! * **`DraftState::Sending`**, committed immediately before the transaction
//!   opens — after connect and auth, which are ordinarily retryable and leave
//!   the draft `Queued`.
//! * **`DraftState::Sent`**, committed the instant `send_message` returns
//!   `Ok`, ahead of `QUIT` and everything else.
//!
//! [`resolve`] reads both back, which is what makes them a guarantee rather
//! than bookkeeping: `Sent` is obsolete and is never rebuilt, and `Sending`
//! is refused rather than retried, because nothing can know whether the
//! payload reached the server and a duplicate cannot be recalled while a
//! message that needs sending again costs three seconds.
//!
//! # Why a retry is recognisable at all
//!
//! Every attempt at one draft now carries the same `Message-ID`. It is minted
//! once, by `DraftRepository::queue_send`, in the transaction that enqueues
//! the operation, and lives on [`Draft::rfc_message_id`]; `outgoing::build`
//! uses it instead of generating one per build. That matters less for
//! receiver-side deduplication — unreliable, and not something Postio may
//! promise — than for Postio recognising *its own* message when it comes back
//! in a Sent-folder sync, which is what #674 uses to resolve an interrupted
//! send without asking the user anything.
//!
//! SMTP has no transport-level idempotency key, so none of this is a proof of
//! delivery. What it is, is at-most-once submission: no path here submits a
//! message that may already have been submitted.
//!
//! [`Draft::rfc_message_id`]: postio_model::Draft::rfc_message_id

use chrono::Utc;
use postio_account::auth::{TokenSource, with_credential};
use postio_account::backend::{AppendMessage, MailBackend};
use postio_account::secret::AccountKey;
use postio_model::ids::{AccountId, DraftId};
use postio_model::{
    Attachment, DraftState, Flag, FlagSet, MailboxId, MailboxRole, Message, OutgoingAttachment,
    mime, outgoing,
};
use postio_smtp::cancel::CancelToken;
use postio_smtp::session::SmtpSession;
use postio_smtp::settings::ConnectionSettings;
use postio_smtp::transport::SmtpConnector;
use postio_storage::BlobStore;
use postio_storage::repository::{
    AccountRepository, DraftRepository, MailboxRepository, MessageRepository, StoredBody,
    ThreadingRepository,
};
use rusqlite::Connection;
use secrecy::SecretString;
use std::collections::BTreeSet;

use crate::backfill::stored_text;
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
    drafts_copy: Option<(MailboxId, String, postio_model::RemoteId)>,
}

/// What resolving a `Send` operation against local storage found.
pub(crate) enum ResolvedSend {
    /// Ready to hand to [`send`].
    Ready(Box<SendJob>),
    /// Nothing to send any more — the draft is gone.
    Obsolete(String),
    /// Cannot be sent and retrying will not change that.
    Impossible(String),
    /// It may already have been sent, and there is no way to find out.
    ///
    /// A fresh process finding a draft still marked `Sending`, or one
    /// already left `Unconfirmed`: the previous attempt died inside the
    /// window ADR 0021 cannot close. Retrying would risk a duplicate.
    Uncertain(String),
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
    // The commit point, read back (ADR 0021). `Sent` is written the instant
    // SMTP accepts, ahead of `QUIT`, the `APPEND` and every local write that
    // follows, so a process that died anywhere in that window comes back to
    // this and stops here instead of delivering the message a second time.
    if draft.state == DraftState::Sent {
        return Ok(ResolvedSend::Obsolete(
            "the submission server already accepted this message".to_owned(),
        ));
    }
    // And the window nothing can see into: the transaction was open when the
    // process stopped. Whether the payload reached the server is not knowable
    // from here and never will be, so this does not retry — a duplicate is
    // delivered to somebody else's inbox and cannot be recalled, while a
    // message that needs saying "send it again" costs three seconds to a
    // person who has been told what happened.
    //
    // Its own outcome rather than a failure (#674): "failed" claims more
    // than is known, and this is the one case where nothing can be known.
    // The draft carries it too, so the answer survives the process that
    // could not find it out.
    if draft.state == DraftState::Sending || draft.state == DraftState::Unconfirmed {
        // Promote the mark on the way past. `Sending` is a *transient* state
        // -- it means "a submission is happening right now" -- and finding it
        // here means the process that was doing it is gone. Leaving it would
        // make every later read say a send is in flight that nothing is
        // flying, and the Drafts list would show a spinner for ever.
        // `Unconfirmed` is the resting state for the same fact, and it is
        // what the user is shown (#674).
        if draft.state == DraftState::Sending {
            DraftRepository::new(connection).set_state(draft.id, DraftState::Unconfirmed)?;
        }
        return Ok(ResolvedSend::Uncertain(INDETERMINATE.to_owned()));
    }
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
        // `.with_auth`, or the stored mechanism dies here the way it died
        // in the IMAP engine (#533): SMTP has spoken XOAUTH2 since #193 and
        // was never told when to.
        outgoing: ConnectionSettings::from_server_config(&account.outgoing).with_auth(account.auth),
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
    // Local-first, like every other mutating verb (#942). The row is in Sent
    // before the socket is opened, so the message is somewhere the user can
    // see it for the whole time it is on its way rather than only afterwards.
    let filed = file_sent_locally(connection, smtp, job);
    let outcome = submit(connection, backend, smtp, resync, job, filed.clone()).await;
    // `Failed` is the one outcome ADR 0021 lets us say "nothing was delivered"
    // about. A `Retry` is still in progress and an `Uncertain` may have gone,
    // and a Sent row is the honest thing to show for both.
    if matches!(outcome, Outcome::Failed { .. }) {
        unfile_sent_copy(connection, filed.as_ref());
    }
    outcome
}

/// The submission itself, from opening the connection to filing the copy.
async fn submit(
    connection: &Connection,
    backend: &dyn MailBackend,
    smtp: &SmtpContext<'_>,
    resync: &mut BTreeSet<i64>,
    job: &SendJob,
    filed: Option<Message>,
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
            // Exactly one `to_owned`, for the reason `postio_account`'s own
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

    // The first of ADR 0021's two marks, and it is taken *here* rather than
    // earlier on purpose: connecting and authenticating are ordinarily
    // retryable and leave the draft `Queued`, while everything past this line
    // may put a payload on the wire. A process that dies from here on comes
    // back to `Sending`, which `resolve` refuses to submit again.
    if let Err(error) = mark(connection, job, DraftState::Sending) {
        // Nothing has been submitted yet, so this is safe to retry — and it
        // must be a refusal rather than a shrug: sending without the mark is
        // sending with the crash window wide open again.
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
            // The client stopped hearing from the server once the payload was
            // already going out (#673 is what makes that knowable, rather than
            // any lost connection anywhere in the session). Leave the mark
            // exactly where it is: this is the one failure that must not
            // become a retry, because "try again" and "you already sent it"
            // are indistinguishable from here.
            // The draft says so too, and it is the durable half: the queue
            // row is settled and gone, while `Unconfirmed` is what the user
            // finds in Drafts ten minutes later. A failure to write it is
            // not a reason to retry the *send* -- that is the one thing this
            // branch exists to prevent -- so it is recorded and the outcome
            // stands.
            if let Err(error) = mark(connection, job, DraftState::Unconfirmed) {
                tracing::error!(
                    %error,
                    "could not record that a send was left unconfirmed; the \
                     draft stays in Sending, which still refuses to resend it"
                );
            }
            return Outcome::Uncertain {
                reason: INDETERMINATE.to_owned(),
            };
        }
        // The server answered and refused, so nothing was accepted and the
        // mark has to come back off — otherwise `resolve` would refuse the
        // retry the ordinary backoff is about to schedule, and a 4xx from a
        // rate-limited server would end the message's life.
        release(connection, job);
        return outcome_from_smtp_error(error);
    }

    // Delivered, and the second mark is the very next thing that happens —
    // ahead of `QUIT`, the `APPEND`, the blob write and the local row. Before
    // ADR 0021 the fact that stopped a resend was the draft's *deletion* at
    // the end of `confirm_sent_copy`, which put a whole IMAP round trip inside
    // the window a crash could reopen. Now the window is one local commit.
    //
    // A failure here cannot become anything but `Applied`, for the same
    // reason nothing else below can: the message has gone.
    let _ = mark(connection, job, DraftState::Sent);

    let _ = session.quit().await;
    confirm_sent_copy(connection, backend, smtp, resync, job, filed).await;
    Outcome::Applied
}

/// What the user is told about a send whose outcome nobody can determine.
///
/// One string, used by [`send`] when it witnesses the failure and by
/// [`resolve`] when a later process finds the mark a dead one left behind, so
/// the two cannot describe the same situation differently. Worded to leave
/// the question open: this is not a failure, and saying it failed would be a
/// claim nothing supports.
pub(crate) const INDETERMINATE: &str = "this send was interrupted while it was being submitted, \
     so it may or may not have arrived — Postio will not send it again on its own";

/// Commits `state` onto the draft this job is sending.
///
/// Its own function because the calls in [`send`] are the whole of ADR 0021's
/// second decision, and they are the only writes in this module whose
/// *ordering* — not merely their success — is the guarantee.
fn mark(connection: &Connection, job: &SendJob, state: DraftState) -> postio_storage::Result<()> {
    DraftRepository::new(connection).set_state(job.draft, state)
}

/// Puts the draft back to `Queued` after a refusal the client witnessed.
///
/// Best-effort, and safe as such in the one direction that matters: a release
/// that fails leaves the draft `Sending`, which [`resolve`] refuses rather
/// than resends. The cost of losing this write is a message that needs asking
/// about, never one that goes twice.
fn release(connection: &Connection, job: &SendJob) {
    if let Err(error) = mark(connection, job, DraftState::Queued) {
        tracing::warn!(%error, "could not return an unsent draft to the queue");
    }
}

/// Records the send locally: appends to the server's Sent folder, writes the
/// local row and its blobs, threads it, and drops the draft.
///
/// Every step tolerates the previous one failing, because none of them may
/// cause a resend. An `APPEND` failure flags the Sent mailbox for resync
/// instead — the message still gets a local row, so nothing is lost from
/// the user's own view of Postio even if the row has no server `UID` until
/// a later sync reconciles it.
/// Writes the Sent row before anything touches the network.
///
/// **The whole point of #942.** Every other mutating verb in Postio is
/// local-first — SQLite write, enqueue, emit, repaint, and the UI never awaits
/// the network (`CLAUDE.md`). Send was the exception: the Sent row was written
/// at the *end* of the job, after SMTP delivery and the IMAP `APPEND` had both
/// come back, so between pressing send and delivery completing there was
/// nothing in Sent at all. A user reported it as mail going "into the ether".
///
/// The row has no server identity yet — the `APPEND` has not happened — which
/// is what [`confirm_sent_copy`] attaches, and what
/// `local_copies_awaiting_identity` lets a resync of Sent adopt rather than
/// duplicate.
///
/// Best-effort like everything else on this path: a row that cannot be written
/// is not a reason to refuse to send. The send proceeds and
/// [`confirm_sent_copy`] writes the row at the end, which is exactly the
/// behaviour this replaced.
fn file_sent_locally(
    connection: &Connection,
    smtp: &SmtpContext<'_>,
    job: &SendJob,
) -> Option<Message> {
    let mut flags = FlagSet::new();
    flags.insert(Flag::Seen);

    let mut message = mime::parse(&job.raw).into_message(job.account, job.sent_mailbox, Utc::now());
    message.flags = flags;
    message.bcc = job.bcc.clone();
    message.attachments = job.attachments.clone();
    message.raw_blob_id = smtp.blobs.put(&job.raw).ok();

    let messages = MessageRepository::new(connection);
    if messages.create(&mut message).is_err() {
        return None;
    }
    let _ = ThreadingRepository::new(connection, job.account).thread(&message);
    // No recount needed: `messages_count_insert` already moved Sent's cached
    // counts when `create` inserted the row.

    // The block of what was actually sent, from the same bytes the raw blob
    // was written from. Mail Postio sent is mail `header:` has to be able to
    // find (#884): a Sent folder that answered "no such message" for a header
    // on a message this client composed would be the most obviously wrong
    // possible answer.
    let block = postio_model::headers::block_of(&job.raw);
    let body = StoredBody {
        text: stored_text(message.body.text.as_deref()),
        html: stored_text(message.body.html.as_deref()),
        headers: block.as_ref().map(|block| block.text.clone()),
        headers_truncated: block.as_ref().is_some_and(|block| block.truncated),
        // Postio composed and encoded these bytes, so there is no decode to
        // have gone wrong. The one honest `false` in the codebase (#901).
        encoding_problems: false,
    };
    let _ = messages.set_body(message.id, &body, postio_model::BodyState::Full);
    Some(message)
}

/// Takes back the Sent row [`file_sent_locally`] wrote, after a failed send.
///
/// Only for [`Outcome::Failed`], which ADR 0021 defines as the outcomes where
/// *nothing was delivered* is true rather than hopeful — the draft becomes
/// editable again and belongs in Drafts, so a copy in Sent would be claiming
/// something that did not happen. A `Retry` keeps the row, because the send
/// has not finished; `Uncertain` keeps it precisely because it may have gone,
/// which is the whole of #674.
fn unfile_sent_copy(connection: &Connection, filed: Option<&Message>) {
    let Some(message) = filed else {
        return;
    };
    if let Err(error) = MessageRepository::new(connection).delete(&[message.id]) {
        tracing::warn!(
            %error,
            "could not remove the Sent copy of a send that failed; it will \
             show as sent until the folder is resynced"
        );
    }
}

/// Attaches the server's identity to the row [`file_sent_locally`] wrote.
///
/// The `APPEND` is what gives the sent copy a place on the server. Until it
/// lands the local row stands on its own, which is deliberate — see
/// [`file_sent_locally`].
///
/// Writes the row here after all if the early write failed, so the message is
/// filed either way.
async fn confirm_sent_copy(
    connection: &Connection,
    backend: &dyn MailBackend,
    smtp: &SmtpContext<'_>,
    resync: &mut BTreeSet<i64>,
    job: &SendJob,
    filed: Option<Message>,
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

    let messages = MessageRepository::new(connection);
    match filed {
        // The ordinary path: the row is already in Sent and has been since
        // before the connection opened. Only the server's coordinates are new.
        Some(mut message) => {
            if let Some(mapping) = mapping {
                message.server.uid = Some(mapping.destination);
                message.server.uid_validity = Some(mapping.uid_validity);
                message.server.remote_id = Some(mapping.destination_remote_id());
                if let Err(error) = messages.update(&mut message) {
                    tracing::warn!(
                        %error,
                        "could not record where the Sent copy landed; a resync \
                         of Sent adopts the row by its Message-ID instead"
                    );
                    resync.insert(job.sent_mailbox.get());
                }
            }
        }
        // The early write did not happen, so this is the old behaviour:
        // file it now, with whatever identity the append produced.
        None => {
            let mut message =
                mime::parse(&job.raw).into_message(job.account, job.sent_mailbox, Utc::now());
            message.flags = flags;
            message.bcc = job.bcc.clone();
            message.attachments = job.attachments.clone();
            message.raw_blob_id = smtp.blobs.put(&job.raw).ok();
            if let Some(mapping) = mapping {
                message.server.uid = Some(mapping.destination);
                message.server.uid_validity = Some(mapping.uid_validity);
                message.server.remote_id = Some(mapping.destination_remote_id());
            }
            if messages.create(&mut message).is_err() {
                return;
            }
            let _ = ThreadingRepository::new(connection, job.account).thread(&message);
            let block = postio_model::headers::block_of(&job.raw);
            let body = StoredBody {
                text: stored_text(message.body.text.as_deref()),
                html: stored_text(message.body.html.as_deref()),
                headers: block.as_ref().map(|block| block.text.clone()),
                headers_truncated: block.as_ref().is_some_and(|block| block.truncated),
                // Postio composed and encoded these bytes, so there is no
                // decode to have gone wrong (#901).
                encoding_problems: false,
            };
            let _ = messages.set_body(message.id, &body, postio_model::BodyState::Full);
        }
    }

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
    match crate::drafts::remove(backend, &capabilities, path, copy).await {
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

/// Resolve every `Unconfirmed` draft whose message has since turned up.
///
/// The half of ADR 0021 Decision 3 that answers the question instead of
/// asking it (#674). A send interrupted mid-submission leaves a draft nobody
/// can settle — but the message, if it went, carries the `Message-ID` the
/// draft reserved before the first attempt (#461), and many submission
/// servers file the sender's copy themselves. So the next sync that brings
/// that message down *is* the confirmation, and it arrives without asking
/// the user anything and without a single extra request.
///
/// Run after a pass rather than only after a Sent pass: where a copy is
/// filed is the server's choice, not ours, and the cost of looking is one
/// indexed read per unconfirmed draft — of which there are normally none.
///
/// Returns the drafts it resolved, so the caller can tell the panes. A draft
/// with no reserved id predates #461 and is left alone: without one there is
/// nothing to recognise it by, and guessing from subject and recipients is
/// how the wrong message gets called the sent one.
pub fn confirm_unconfirmed(
    connection: &Connection,
    account: AccountId,
) -> Result<Vec<(DraftId, postio_model::ids::MessageId)>> {
    let drafts = DraftRepository::new(connection);
    let messages = MessageRepository::new(connection);
    let mut resolved = Vec::new();

    for draft in drafts.by_state(DraftState::Unconfirmed)? {
        if draft.account_id != account {
            continue;
        }
        let Some(reserved) = draft.rfc_message_id.as_ref() else {
            continue;
        };
        // The account is part of the lookup: the same `Message-ID` arriving
        // in a *different* account is somebody else's copy of a conversation,
        // not evidence that this account's submission succeeded.
        let Some(message) = messages
            .ids_by_rfc_message_id(account, reserved)?
            .into_iter()
            .next()
        else {
            continue;
        };
        drafts.set_state(draft.id, DraftState::Sent)?;
        drafts.set_synced_message(draft.id, message)?;
        tracing::info!(
            draft = draft.id.get(),
            "an unconfirmed send turned up in a sync; it did arrive"
        );
        resolved.push((draft.id, message));
    }
    Ok(resolved)
}
