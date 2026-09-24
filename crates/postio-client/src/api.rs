//! The client a frontend holds.
//!
//! Commands go down, events come up, and list reads are answered by the
//! store's one owner. [`Client`] implements [`MailStore`], so a frontend
//! holds it behind the same `Arc<dyn MailStore>` it held when the store was
//! in-process, and a read site cannot tell the difference (ADR 0041).
//!
//! The transport is a trait: the socket in production, channels into an
//! in-process host for `postio-ffi` and the integration suites, a fake in
//! the tests below.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use postio_core::{Command, EventEnvelope, InvocationId, SharedState};
use postio_model::ListScope;
use postio_model::contact_group::RecipientCandidate;
use postio_model::ids::{AccountId, MailboxId, MessageId};
use postio_model::listing::{
    DraftCounts, ListPage, ListRows, MailStore, MessageSummary, PageRequest, Read, StoreError,
};
use postio_model::mailbox::Mailbox;
use postio_model::{Draft, DraftId};

use crate::counting::Counts;
use crate::protocol::{Req, Resp};

/// The host is not answering: it exited, or the connection broke.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("Postio's background service is not answering.")]
pub struct Disconnected;

/// A command that was not taken.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SendError {
    /// The host's runtime has stopped.
    #[error("Postio is shutting down and did not take that.")]
    Stopped,
    /// The host is not answering.
    #[error(transparent)]
    Disconnected(#[from] Disconnected),
}

/// One request's answer, awaited.
pub type Call<'a> = Pin<Box<dyn Future<Output = Result<Resp, Disconnected>> + Send + 'a>>;

/// How requests reach the host and events come back.
pub trait Transport: Send + Sync + 'static {
    /// Ask, and await the one answer.
    ///
    /// The request is handed over by this call, not when the answer is first
    /// polled: two calls made in order reach the host in that order however
    /// their answers are awaited. Hence `'static` -- the answer borrows
    /// nothing, so it can be awaited on another task.
    fn call(&self, request: Req) -> Call<'static>;

    /// Ask without waiting for the answer. For facts a later read acts on,
    /// said from a synchronous caller.
    fn post(&self, request: Req);

    /// Every event the host sends this client, in order.
    fn events(&self) -> async_channel::Receiver<EventEnvelope>;
}

impl Req {
    /// The family a round trip is counted under.
    pub fn family(&self) -> &'static str {
        match self {
            Req::Send(..) => "Send",
            Req::SendTracked(..) => "SendTracked",
            Req::Page(_) => "Page",
            Req::Count(_) => "Count",
            Req::Rows(_) => "Rows",
            Req::RowsIn(..) => "RowsIn",
            Req::NoteRemoved(..) => "NoteRemoved",
            Req::Mailboxes(_) => "Mailboxes",
            Req::DraftCounts(_) => "DraftCounts",
            Req::Accounts => "Accounts",
            Req::Body(_) => "Body",
            Req::Conversation(_) => "Conversation",
            Req::Unsubscribe(_) => "Unsubscribe",
            Req::Parts(_) => "Parts",
            Req::SavePart { .. } => "SavePart",
            Req::OpenPart { .. } => "OpenPart",
            Req::Readings { .. } => "Readings",
            Req::ThreadReadings { .. } => "ThreadReadings",
            Req::InlinePart { .. } => "InlinePart",
            Req::SaveParts { .. } => "SaveParts",
            Req::SaveDraft { .. } => "SaveDraft",
            Req::QueueSend { .. } => "QueueSend",
            Req::DiscardDraft { .. } => "DiscardDraft",
            Req::Recipients { .. } => "Recipients",
            Req::Correspondents(_) => "Correspondents",
            Req::Labels(_) => "Labels",
            Req::ReplySource(_) => "ReplySource",
            Req::DraftBehind(_) => "DraftBehind",
            Req::CancelSend(_) => "CancelSend",
            Req::SendFailure(_) => "SendFailure",
            Req::DefaultSignature { .. } => "DefaultSignature",
            Req::Attach { .. } => "Attach",
            Req::AttachmentBytes(_) => "AttachmentBytes",
            Req::RecoverDraft(_) => "RecoverDraft",
            Req::InlineImage { .. } => "InlineImage",
            Req::Search(_) => "Search",
            Req::SearchHits { .. } => "SearchHits",
            Req::Facets { .. } => "Facets",
            Req::StoredBody(_) => "StoredBody",
            Req::ExportMessages(_) => "ExportMessages",
            Req::Diagnose(_) => "Diagnose",
            Req::Account(_) => "Account",
            Req::Discover(_) => "Discover",
            Req::BeginOAuth(_) => "BeginOAuth",
            Req::FinishOAuth(_) => "FinishOAuth",
            Req::CancelOAuth(_) => "CancelOAuth",
            Req::AddAccount(_) => "AddAccount",
            Req::AccountSettings { .. } => "AccountSettings",
            Req::EditAccount(..) => "EditAccount",
            Req::SaveSignature { .. } => "SaveSignature",
            Req::DeleteSignature(_) => "DeleteSignature",
            Req::RebuildIndex(_) => "RebuildIndex",
            Req::EgressLog(_) => "EgressLog",
            Req::PrivacyLog => "PrivacyLog",
            Req::SetBackfillExcluded { .. } => "SetBackfillExcluded",
            Req::OrientationSeen => "OrientationSeen",
            Req::RetireOrientation => "RetireOrientation",
            Req::SaveAccount { .. } => "SaveAccount",
            Req::SaveOAuthAccount(_) => "SaveOAuthAccount",
        }
    }
}

/// What a frontend holds.
#[derive(Clone)]
pub struct Client {
    transport: Arc<dyn Transport>,
    counts: Arc<Counts>,
    state: SharedState,
}

impl std::fmt::Debug for Client {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Client").finish_non_exhaustive()
    }
}

impl Client {
    /// A client over `transport`.
    pub fn new(transport: Arc<dyn Transport>) -> Self {
        Client {
            transport,
            counts: Arc::new(Counts::default()),
            state: SharedState::default(),
        }
    }

    /// The same client, aiming its commands with `state`: the frontend's
    /// own selection, focus and view, which the host adopts before running
    /// each command (ADR 0041).
    pub fn with_state(mut self, state: SharedState) -> Self {
        self.state = state;
        self
    }

    /// The round trips this client has made.
    pub fn counts(&self) -> &Counts {
        &self.counts
    }

    /// Every event the host sends this client.
    pub fn events(&self) -> async_channel::Receiver<EventEnvelope> {
        self.transport.events()
    }

    async fn call(&self, request: Req) -> Result<Resp, Disconnected> {
        self.counts.record(request.family());
        self.transport.call(request).await
    }

    /// A read: the answer, or the host's sentence for why there is none.
    async fn read<T>(
        &self,
        request: Req,
        asked: &str,
        take: impl FnOnce(Resp) -> Option<T>,
    ) -> Result<T, StoreError> {
        match self.call(request).await {
            Ok(Resp::Failed(error)) => Err(error),
            Ok(answer) => take(answer).ok_or_else(|| unexpected(asked)),
            Err(disconnected) => Err(StoreError::new(disconnected.to_string())),
        }
    }

    /// Every account, in the sidebar's order.
    pub async fn accounts(&self) -> Result<Vec<postio_model::Account>, StoreError> {
        self.read(Req::Accounts, "the accounts", |answer| match answer {
            Resp::Accounts(accounts) => Some(accounts),
            _ => None,
        })
        .await
    }

    /// A message's body, or why there is none yet.
    pub async fn body(&self, message: MessageId) -> Result<crate::protocol::Body, StoreError> {
        self.read(Req::Body(message), "a body", |answer| match answer {
            Resp::Body(body) => Some(body),
            _ => None,
        })
        .await
    }

    /// A conversation's messages, oldest first.
    pub async fn conversation(
        &self,
        thread: postio_model::ThreadId,
    ) -> Result<Vec<MessageSummary>, StoreError> {
        self.read(
            Req::Conversation(thread),
            "a conversation",
            |answer| match answer {
                Resp::Rows(rows) => Some(rows),
                _ => None,
            },
        )
        .await
    }

    /// Leave the list `message` came from; the answer is the list's name.
    pub async fn unsubscribe(&self, message: MessageId) -> Result<String, StoreError> {
        self.read(
            Req::Unsubscribe(message),
            "an unsubscribe",
            |answer| match answer {
                Resp::Unsubscribed(list) => Some(list),
                _ => None,
            },
        )
        .await
    }

    /// A message's parts.
    pub async fn parts(
        &self,
        message: MessageId,
    ) -> Result<Vec<postio_model::Attachment>, StoreError> {
        self.read(Req::Parts(message), "the parts", |answer| match answer {
            Resp::Parts(parts) => Some(parts),
            _ => None,
        })
        .await
    }

    /// Write one part to `to`, fetching it first if it has to be.
    pub async fn save_part(
        &self,
        message: MessageId,
        attachment: postio_model::ids::AttachmentId,
        to: std::path::PathBuf,
    ) -> Result<std::path::PathBuf, StoreError> {
        let request = Req::SavePart {
            message,
            attachment,
            to,
        };
        self.read(request, "a saved part", |answer| match answer {
            Resp::Saved(path) => Some(path),
            _ => None,
        })
        .await
    }

    /// Write one part to a private temporary file to open, and say where.
    pub async fn open_part(
        &self,
        message: MessageId,
        attachment: postio_model::ids::AttachmentId,
    ) -> Result<std::path::PathBuf, StoreError> {
        let request = Req::OpenPart {
            message,
            attachment,
        };
        self.read(request, "a part to open", |answer| match answer {
            Resp::Saved(path) => Some(path),
            _ => None,
        })
        .await
    }

    /// Everything a reading pane draws of each of `messages`, in one call.
    pub async fn readings(
        &self,
        messages: Vec<MessageId>,
        offline: bool,
    ) -> Result<Vec<crate::protocol::Reading>, StoreError> {
        let request = Req::Readings { messages, offline };
        self.read(request, "a reading", |answer| match answer {
            Resp::Readings(readings) => Some(readings),
            _ => None,
        })
        .await
    }

    /// [`Client::readings`] for a conversation's first `limit` members.
    pub async fn thread_readings(
        &self,
        thread: postio_model::ThreadId,
        limit: u32,
        offline: bool,
    ) -> Result<Vec<crate::protocol::Reading>, StoreError> {
        let request = Req::ThreadReadings {
            thread,
            limit,
            offline,
        };
        self.read(request, "a conversation's reading", |answer| match answer {
            Resp::Readings(readings) => Some(readings),
            _ => None,
        })
        .await
    }

    /// One inline part of `message` by its `Content-ID`: its bytes and type,
    /// when they are on this machine.
    pub async fn inline_part(
        &self,
        message: MessageId,
        content_id: String,
    ) -> Result<Option<(Vec<u8>, String)>, StoreError> {
        let request = Req::InlinePart {
            message,
            content_id,
        };
        self.read(request, "an inline part", |answer| match answer {
            Resp::InlinePart(found) => Some(found),
            _ => None,
        })
        .await
    }

    /// Write each of `parts` to its path; the answer is how many could not
    /// be written.
    pub async fn save_parts(
        &self,
        message: MessageId,
        parts: Vec<(postio_model::ids::AttachmentId, std::path::PathBuf)>,
    ) -> Result<usize, StoreError> {
        let request = Req::SaveParts { message, parts };
        self.read(request, "saved parts", |answer| match answer {
            Resp::SavedParts(failed) => Some(failed as usize),
            _ => None,
        })
        .await
    }

    /// Hand `request` over now, in the order this is called, and answer
    /// when the host does: for writes that must land in the order they were
    /// made however their answers are awaited.
    fn hand_over<T: Send + 'static>(
        &self,
        request: Req,
        asked: &'static str,
        take: fn(Resp) -> Option<T>,
    ) -> impl Future<Output = Result<T, StoreError>> + Send + 'static {
        self.counts.record(request.family());
        let answer = self.transport.call(request);
        async move {
            match answer.await {
                Ok(Resp::Failed(error)) => Err(error),
                Ok(answer) => take(answer).ok_or_else(|| unexpected(asked)),
                Err(disconnected) => Err(StoreError::new(disconnected.to_string())),
            }
        }
    }

    /// Autosave `draft` as composition `generation`; the answer is its id.
    ///
    /// Handed over at the call, like the other draft writes: see
    /// [`Transport::call`].
    pub fn save_draft(
        &self,
        generation: u64,
        draft: Draft,
    ) -> impl Future<Output = Result<DraftId, StoreError>> + Send + 'static {
        let request = Req::SaveDraft {
            generation,
            draft: Box::new(draft),
        };
        self.hand_over(request, "a saved draft", |answer| match answer {
            Resp::DraftSaved(id) => Some(id),
            _ => None,
        })
    }

    /// Queue `draft` to send, now or at `at`; handed over at the call.
    pub fn queue_send(
        &self,
        generation: u64,
        draft: Draft,
        at: Option<DateTime<Utc>>,
    ) -> impl Future<Output = Result<Option<MailboxId>, StoreError>> + Send + 'static {
        let request = Req::QueueSend {
            generation,
            draft: Box::new(draft),
            at,
        };
        self.hand_over(request, "a queued send", |answer| match answer {
            Resp::Queued(moved) => Some(moved),
            _ => None,
        })
    }

    /// Composition `generation` closed with nothing worth keeping; handed
    /// over at the call.
    pub fn discard_draft(
        &self,
        generation: u64,
        known: Option<DraftId>,
    ) -> impl Future<Output = Result<(), StoreError>> + Send + 'static {
        let request = Req::DiscardDraft { generation, known };
        self.hand_over(request, "a discard", |answer| match answer {
            Resp::Done => Some(()),
            _ => None,
        })
    }

    /// Recipient completion for `prefix`.
    pub async fn recipients(
        &self,
        account: AccountId,
        prefix: String,
    ) -> Result<Vec<RecipientCandidate>, StoreError> {
        let request = Req::Recipients { account, prefix };
        self.read(request, "recipients", |answer| match answer {
            Resp::Recipients(found) => Some(found),
            _ => None,
        })
        .await
    }

    /// The account's correspondents, for the finder's `@`.
    pub async fn correspondents(
        &self,
        account: AccountId,
    ) -> Result<Vec<postio_model::Contact>, StoreError> {
        self.read(
            Req::Correspondents(account),
            "correspondents",
            |answer| match answer {
                Resp::Correspondents(found) => Some(found),
                _ => None,
            },
        )
        .await
    }

    /// The account's labels, for the finder's `+`.
    pub async fn labels(&self, account: AccountId) -> Result<Vec<postio_model::Label>, StoreError> {
        self.read(Req::Labels(account), "labels", |answer| match answer {
            Resp::Labels(found) => Some(found),
            _ => None,
        })
        .await
    }

    /// The message a reply to `message` is built from, and its account.
    pub async fn reply_source(
        &self,
        message: MessageId,
    ) -> Result<Option<(postio_model::Message, postio_model::Account)>, StoreError> {
        self.read(
            Req::ReplySource(message),
            "a reply source",
            |answer| match answer {
                Resp::ReplySource(found) => Some(found.map(|found| *found)),
                _ => None,
            },
        )
        .await
    }

    /// The local draft behind a Drafts row.
    pub async fn draft_behind(&self, message: MessageId) -> Result<Option<Draft>, StoreError> {
        self.read(
            Req::DraftBehind(message),
            "a draft",
            |answer| match answer {
                Resp::Draft(found) => Some(found.map(|found| *found)),
                _ => None,
            },
        )
        .await
    }

    /// Take a queued draft back to edit, if it has not started sending.
    pub async fn cancel_send(&self, draft: DraftId) -> Result<Option<Draft>, StoreError> {
        self.read(
            Req::CancelSend(draft),
            "a cancelled send",
            |answer| match answer {
                Resp::Draft(found) => Some(found.map(|found| *found)),
                _ => None,
            },
        )
        .await
    }

    /// Why a draft's last send attempt gave up.
    pub async fn send_failure(&self, draft: DraftId) -> Result<Option<String>, StoreError> {
        self.read(
            Req::SendFailure(draft),
            "a send failure",
            |answer| match answer {
                Resp::SendFailure(why) => Some(why),
                _ => None,
            },
        )
        .await
    }

    /// The signature a new draft for `account` starts with.
    pub async fn default_signature(
        &self,
        account: AccountId,
        selected: Option<MailboxId>,
    ) -> Result<Option<postio_model::SignatureId>, StoreError> {
        let request = Req::DefaultSignature { account, selected };
        self.read(request, "a signature", |answer| match answer {
            Resp::Signature(found) => Some(found),
            _ => None,
        })
        .await
    }

    /// Store the file at `path` as an attachment, of the type the host
    /// guesses for it.
    pub async fn attach(
        &self,
        path: std::path::PathBuf,
    ) -> Result<Option<postio_model::Attachment>, StoreError> {
        self.attach_as(path, None).await
    }

    /// Store the file at `path` as an attachment of `mime_type`, when the
    /// frontend sniffed one; the host guesses otherwise.
    pub async fn attach_as(
        &self,
        path: std::path::PathBuf,
        mime_type: Option<String>,
    ) -> Result<Option<postio_model::Attachment>, StoreError> {
        let request = Req::Attach { path, mime_type };
        self.read(request, "an attachment", |answer| match answer {
            Resp::Attached(found) => Some(found),
            _ => None,
        })
        .await
    }

    /// The bytes stored under `blob`, or none when they are not here.
    pub async fn attachment_bytes(
        &self,
        blob: postio_model::ids::BlobId,
    ) -> Result<Option<Vec<u8>>, StoreError> {
        self.read(
            Req::AttachmentBytes(blob),
            "an attachment's bytes",
            |answer| match answer {
                Resp::Bytes(found) => Some(found),
                _ => None,
            },
        )
        .await
    }

    /// Say this frontend's session began, and take back the draft `account`
    /// was writing when the last one died, if it did.
    pub async fn recover_draft(&self, account: AccountId) -> Result<Option<Draft>, StoreError> {
        self.read(
            Req::RecoverDraft(account),
            "a recovered draft",
            |answer| match answer {
                Resp::Draft(found) => Some(found.map(|found| *found)),
                _ => None,
            },
        )
        .await
    }

    /// Store pasted image bytes as an inline part.
    pub async fn inline_image(
        &self,
        bytes: Vec<u8>,
        mime_type: String,
    ) -> Result<Option<postio_model::Attachment>, StoreError> {
        let request = Req::InlineImage { bytes, mime_type };
        self.read(request, "an inline image", |answer| match answer {
            Resp::Attached(found) => Some(found),
            _ => None,
        })
        .await
    }

    /// One of `postio-diag`'s reports, run by the daemon on its own
    /// connection.
    pub async fn diagnose(&self, report: String) -> Result<String, StoreError> {
        self.read(Req::Diagnose(report), "a report", |answer| match answer {
            Resp::Diagnosis(text) => Some(text),
            _ => None,
        })
        .await
    }

    /// Change an account as the settings' account commands do.
    pub async fn account(&self, op: crate::protocol::AccountOp) -> Result<(), StoreError> {
        self.read(
            Req::Account(op),
            "an account change",
            |answer| match answer {
                Resp::Done => Some(()),
                _ => None,
            },
        )
        .await
    }

    /// What discovery finds for a new account's `address`.
    pub async fn discover(
        &self,
        address: String,
    ) -> Result<postio_ui::onboarding::Status, StoreError> {
        self.read(Req::Discover(address), "discovery", |answer| match answer {
            Resp::Onboarding(status) => Some(*status),
            _ => None,
        })
        .await
    }

    /// Begin a browser sign-in; the answer is the consent URL, unopened.
    pub async fn begin_oauth(
        &self,
        submission: postio_ui::onboarding::Submission,
    ) -> Result<postio_ui::onboarding::BrowserSignIn, StoreError> {
        let request = Req::BeginOAuth(Box::new(submission));
        self.read(request, "a sign-in", |answer| match answer {
            Resp::Consent(consent) => Some(*consent),
            _ => None,
        })
        .await
    }

    /// Wait for the sign-in for `address` to finish and the account to be
    /// saved; the error is the sentence for the screen.
    pub async fn finish_oauth(&self, address: String) -> Result<(), StoreError> {
        self.read(
            Req::FinishOAuth(address),
            "a finished sign-in",
            |answer| match answer {
                Resp::Done => Some(()),
                _ => None,
            },
        )
        .await
    }

    /// Give up the sign-in for `address`.
    pub async fn cancel_oauth(&self, address: String) -> Result<(), StoreError> {
        self.read(
            Req::CancelOAuth(address),
            "a cancelled sign-in",
            |answer| match answer {
                Resp::Done => Some(()),
                _ => None,
            },
        )
        .await
    }

    /// Prove and save a new account. The error is the sentence the first-run
    /// screen shows.
    pub async fn add_account(
        &self,
        submission: postio_ui::onboarding::Submission,
    ) -> Result<(), StoreError> {
        self.read(
            Req::AddAccount(Box::new(submission)),
            "a new account",
            |answer| match answer {
                Resp::Done => Some(()),
                _ => None,
            },
        )
        .await
    }

    /// Search; `None` when the store could not be read.
    pub async fn search(
        &self,
        search: crate::protocol::Search,
    ) -> Result<Option<crate::protocol::Found>, StoreError> {
        self.read(Req::Search(search), "a search", |answer| match answer {
            Resp::Found(found) => Some(found),
            _ => None,
        })
        .await
    }

    /// The desktop search's hits for `query`, with excerpts for the first
    /// `snippets`; `None` when the store could not be read.
    pub async fn search_hits(
        &self,
        account: postio_model::AccountScope,
        query: postio_search::ParsedQuery,
        scope: postio_search::facets::Scope,
        order: postio_search::ResultOrder,
        snippets: u32,
    ) -> Result<Option<postio_search::SearchResults>, StoreError> {
        let request = Req::SearchHits {
            account,
            query,
            scope,
            order,
            snippets,
        };
        self.read(request, "a search", |answer| match answer {
            Resp::Hits(found) => Some(found.map(|hits| hits.0)),
            _ => None,
        })
        .await
    }

    /// What the results' columns say about `query` under `account`; `None`
    /// when the counts did not run.
    pub async fn facets(
        &self,
        account: postio_model::AccountScope,
        query: postio_search::ParsedQuery,
        scope: postio_search::facets::Scope,
    ) -> Result<Option<postio_search::facets::Facets>, StoreError> {
        let request = Req::Facets {
            account,
            query,
            scope,
        };
        self.read(request, "the facet counts", |answer| match answer {
            Resp::Facets(found) => Some(found),
            _ => None,
        })
        .await
    }

    /// `message`'s stored words; empty when none are here.
    pub async fn stored_body(
        &self,
        message: MessageId,
    ) -> Result<postio_model::MessageBody, StoreError> {
        self.read(Req::StoredBody(message), "a body", |answer| match answer {
            Resp::StoredBody(body) => Some(body),
            _ => None,
        })
        .await
    }

    /// Write each message's raw source to its path; the answer is the paths
    /// written, in the order asked.
    pub async fn export_messages(
        &self,
        messages: Vec<(MessageId, std::path::PathBuf)>,
    ) -> Result<Vec<std::path::PathBuf>, StoreError> {
        self.read(
            Req::ExportMessages(messages),
            "an export",
            |answer| match answer {
                Resp::Exported(paths) => Some(paths),
                _ => None,
            },
        )
        .await
    }

    /// Every account the settings show, with its folders and role map, and
    /// what its mail weighs when `weights`.
    pub async fn account_settings(
        &self,
        weights: bool,
    ) -> Result<Vec<crate::protocol::AccountSettings>, StoreError> {
        let request = Req::AccountSettings { weights };
        self.read(request, "the account settings", |answer| match answer {
            Resp::AccountSettings(accounts) => Some(accounts),
            _ => None,
        })
        .await
    }

    /// Change one field of `account`'s row.
    pub async fn edit_account(
        &self,
        account: AccountId,
        field: crate::protocol::AccountField,
    ) -> Result<(), StoreError> {
        self.done(Req::EditAccount(account, field), "an account edit")
            .await
    }

    /// Write a signature: `signature` for an edit, `None` for a new one.
    /// A refusal's sentence is for the person who typed it.
    pub async fn save_signature(
        &self,
        account: AccountId,
        signature: Option<postio_model::SignatureId>,
        name: String,
        text: String,
    ) -> Result<(), StoreError> {
        let request = Req::SaveSignature {
            account,
            signature,
            name,
            text,
        };
        self.done(request, "a saved signature").await
    }

    /// Remove a signature.
    pub async fn delete_signature(
        &self,
        signature: postio_model::SignatureId,
    ) -> Result<(), StoreError> {
        self.done(Req::DeleteSignature(signature), "a removed signature")
            .await
    }

    /// Rebuild `account`'s local search index; answers when it is over.
    pub async fn rebuild_index(&self, account: AccountId) -> Result<(), StoreError> {
        self.done(Req::RebuildIndex(account), "a rebuilt index")
            .await
    }

    /// The newest `limit` outbound connections, newest first.
    pub async fn egress_log(
        &self,
        limit: u32,
    ) -> Result<Vec<postio_model::egress::EgressEvent>, StoreError> {
        self.read(
            Req::EgressLog(limit),
            "the egress log",
            |answer| match answer {
                Resp::Egress(entries) => Some(entries),
                _ => None,
            },
        )
        .await
    }

    /// The privacy pane's figures.
    pub async fn privacy_log(&self) -> Result<crate::protocol::PrivacyLog, StoreError> {
        self.read(Req::PrivacyLog, "the privacy log", |answer| match answer {
            Resp::Privacy(log) => Some(log),
            _ => None,
        })
        .await
    }

    /// Skip or resume `mailbox`'s backfill; the answer is its account's
    /// folders as they now stand.
    pub async fn set_backfill_excluded(
        &self,
        mailbox: MailboxId,
        excluded: bool,
    ) -> Result<Vec<Mailbox>, StoreError> {
        let request = Req::SetBackfillExcluded { mailbox, excluded };
        self.read(request, "a folder's backfill", |answer| match answer {
            Resp::Mailboxes(mailboxes) => Some(mailboxes),
            _ => None,
        })
        .await
    }

    /// Whether some earlier run already showed the keyboard orientation.
    pub async fn orientation_seen(&self) -> Result<bool, StoreError> {
        self.read(
            Req::OrientationSeen,
            "the orientation",
            |answer| match answer {
                Resp::Seen(seen) => Some(seen),
                _ => None,
            },
        )
        .await
    }

    /// Write down that the orientation is done with, for every later run.
    pub async fn retire_orientation(&self) -> Result<(), StoreError> {
        self.done(Req::RetireOrientation, "the orientation").await
    }

    /// Save an account whose credentials were proved: the password to the
    /// keyring, then the row. The error is the first-run screen's sentence.
    pub async fn save_account(
        &self,
        submission: postio_ui::onboarding::Submission,
        backend: postio_model::account::Backend,
    ) -> Result<(), StoreError> {
        let request = Req::SaveAccount {
            submission: Box::new(submission),
            backend,
        };
        self.done(request, "a saved account").await
    }

    /// Save an account a browser sign-in proved: its tokens to the keyring,
    /// then the row.
    pub async fn save_oauth_account(
        &self,
        grant: crate::protocol::OAuthGrant,
    ) -> Result<(), StoreError> {
        self.done(Req::SaveOAuthAccount(Box::new(grant)), "a saved account")
            .await
    }

    /// A write answered with [`Resp::Done`].
    async fn done(&self, request: Req, asked: &str) -> Result<(), StoreError> {
        self.read(request, asked, |answer| match answer {
            Resp::Done => Some(()),
            _ => None,
        })
        .await
    }

    /// Run a command. Its effects arrive as events.
    pub async fn send(&self, command: Command) -> Result<(), SendError> {
        let aim = self.state.read(|state| state.snapshot());
        match self.call(Req::Send(command, aim)).await? {
            Resp::Stopped => Err(SendError::Stopped),
            _ => Ok(()),
        }
    }

    /// Run a command and learn the id its events will carry.
    pub async fn send_tracked(&self, command: Command) -> Result<InvocationId, SendError> {
        let aim = self.state.read(|state| state.snapshot());
        match self.call(Req::SendTracked(command, aim)).await? {
            Resp::Tracked(id) => Ok(id),
            _ => Err(SendError::Stopped),
        }
    }
}

/// A read's answer that was not the kind asked for: a bug on one side, said
/// as a sentence rather than a panic in the reader.
fn unexpected(asked: &str) -> StoreError {
    StoreError::new(format!(
        "Postio's background service gave an unexpected answer to {asked}."
    ))
}

impl MailStore for Client {
    fn list_page(&self, request: PageRequest) -> Read<'_, ListPage> {
        Box::pin(
            self.read(Req::Page(request), "a page", |answer| match answer {
                Resp::Page(page) => Some(page),
                _ => None,
            }),
        )
    }

    fn list_count(&self, scope: ListScope) -> Read<'_, u32> {
        Box::pin(
            self.read(Req::Count(scope), "a count", |answer| match answer {
                Resp::Count(count) => Some(count),
                _ => None,
            }),
        )
    }

    fn message_rows(&self, ids: Vec<MessageId>) -> Read<'_, Vec<MessageSummary>> {
        Box::pin(self.read(Req::Rows(ids), "rows", |answer| match answer {
            Resp::Rows(rows) => Some(rows),
            _ => None,
        }))
    }

    fn rows_in(&self, scope: ListScope, ids: Vec<MessageId>) -> Read<'_, ListRows> {
        Box::pin(
            self.read(Req::RowsIn(scope, ids), "rows", |answer| match answer {
                Resp::ListRows(rows) => Some(rows),
                _ => None,
            }),
        )
    }

    fn note_removed(&self, mailbox: MailboxId, messages: Vec<MessageId>) {
        let request = Req::NoteRemoved(mailbox, messages);
        self.counts.record(request.family());
        self.transport.post(request);
    }

    fn mailboxes(&self, account: AccountId) -> Read<'_, Vec<Mailbox>> {
        Box::pin(
            self.read(Req::Mailboxes(account), "folders", |answer| match answer {
                Resp::Mailboxes(mailboxes) => Some(mailboxes),
                _ => None,
            }),
        )
    }

    fn draft_counts(&self, account: AccountId) -> Read<'_, DraftCounts> {
        Box::pin(self.read(
            Req::DraftCounts(account),
            "draft counts",
            |answer| match answer {
                Resp::DraftCounts(counts) => Some(counts),
                _ => None,
            },
        ))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use postio_model::listing::MessagePage;

    use super::*;

    /// Records what it was asked and answers from a script, oldest first.
    struct Fake {
        asked: Mutex<Vec<Req>>,
        posted: Mutex<Vec<Req>>,
        answers: Mutex<Vec<Result<Resp, Disconnected>>>,
        events: async_channel::Receiver<EventEnvelope>,
    }

    fn client(answers: Vec<Result<Resp, Disconnected>>) -> (Client, Arc<Fake>) {
        let (_tx, events) = async_channel::unbounded();
        let fake = Arc::new(Fake {
            asked: Mutex::new(Vec::new()),
            posted: Mutex::new(Vec::new()),
            answers: Mutex::new(answers),
            events,
        });
        (Client::new(fake.clone()), fake)
    }

    impl Transport for Fake {
        fn call(&self, request: Req) -> Call<'static> {
            self.asked.lock().unwrap().push(request);
            let answer = self.answers.lock().unwrap().remove(0);
            Box::pin(async move { answer })
        }
        fn post(&self, request: Req) {
            self.posted.lock().unwrap().push(request);
        }
        fn events(&self) -> async_channel::Receiver<EventEnvelope> {
            self.events.clone()
        }
    }

    fn scope() -> ListScope {
        ListScope::Mailbox(MailboxId::new(2))
    }

    #[tokio::test]
    async fn a_page_is_asked_for_and_handed_back() {
        let page = ListPage::Messages(MessagePage {
            total: 0,
            rows: vec![],
        });
        let (client, fake) = client(vec![Ok(Resp::Page(page.clone()))]);
        let request = PageRequest {
            scope: scope(),
            offset: 20,
            limit: 20,
        };
        assert_eq!(client.list_page(request).await, Ok(page));
        assert_eq!(*fake.asked.lock().unwrap(), vec![Req::Page(request)]);
    }

    #[tokio::test]
    async fn a_refused_read_keeps_the_hosts_sentence() {
        let (client, _) = client(vec![Ok(Resp::Failed(StoreError::new(
            "The store is closed.",
        )))]);
        let error = client.list_count(scope()).await.unwrap_err();
        assert_eq!(error.message(), "The store is closed.");
    }

    #[tokio::test]
    async fn a_silent_host_is_said_as_a_sentence_not_a_panic() {
        let (client, _) = client(vec![Err(Disconnected)]);
        let error = client.mailboxes(AccountId::new(1)).await.unwrap_err();
        assert_eq!(
            error.message(),
            "Postio's background service is not answering."
        );
    }

    #[tokio::test]
    async fn a_command_is_sent_and_a_stopped_runtime_is_reported() {
        let (client, fake) = client(vec![Ok(Resp::Done), Ok(Resp::Stopped)]);
        assert_eq!(client.send(Command::Undo).await, Ok(()));
        assert_eq!(client.send(Command::Undo).await, Err(SendError::Stopped));
        assert_eq!(fake.asked.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn a_command_carries_the_frontends_selection() {
        let state = SharedState::default();
        state.update(&postio_core::bridge::event_channel().0, |app| {
            app.select(vec![MessageId::new(5)], Some(MessageId::new(5)))
        });
        let (client, fake) = client(vec![Ok(Resp::Done)]);
        let client = client.with_state(state.clone());
        client.send(Command::Undo).await.unwrap();
        let Req::Send(_, aim) = &fake.asked.lock().unwrap()[0] else {
            panic!("a send was asked for");
        };
        assert_eq!(*aim, state.read(|app| app.snapshot()));
        assert_ne!(*aim, postio_core::StateSnapshot::default());
    }

    #[tokio::test]
    async fn a_tracked_send_hands_back_the_hosts_id() {
        let id = InvocationId::next();
        let (client, _) = client(vec![Ok(Resp::Tracked(id))]);
        assert_eq!(client.send_tracked(Command::Undo).await, Ok(id));
    }

    #[tokio::test]
    async fn a_removal_is_posted_without_waiting() {
        let (client, fake) = client(vec![]);
        client.note_removed(MailboxId::new(2), vec![MessageId::new(3)]);
        assert_eq!(
            *fake.posted.lock().unwrap(),
            vec![Req::NoteRemoved(MailboxId::new(2), vec![MessageId::new(3)])]
        );
    }

    #[tokio::test]
    async fn the_accounts_are_asked_for_and_handed_back() {
        let (client, fake) = client(vec![Ok(Resp::Accounts(Vec::new()))]);
        assert_eq!(client.accounts().await, Ok(Vec::new()));
        assert_eq!(*fake.asked.lock().unwrap(), vec![Req::Accounts]);
    }

    #[tokio::test]
    async fn a_body_is_asked_for_by_message() {
        let (client, fake) = client(vec![Ok(Resp::Body(crate::protocol::Body::Partial))]);
        assert_eq!(
            client.body(MessageId::new(9)).await,
            Ok(crate::protocol::Body::Partial)
        );
        assert_eq!(
            *fake.asked.lock().unwrap(),
            vec![Req::Body(MessageId::new(9))]
        );
        assert_eq!(client.counts().of("Body"), 1);
    }

    #[tokio::test]
    async fn an_unsubscribe_answers_the_lists_name() {
        let (client, fake) = client(vec![Ok(Resp::Unsubscribed("news.example.com".into()))]);
        assert_eq!(
            client.unsubscribe(MessageId::new(4)).await,
            Ok("news.example.com".to_owned())
        );
        assert_eq!(
            *fake.asked.lock().unwrap(),
            vec![Req::Unsubscribe(MessageId::new(4))]
        );
    }

    #[tokio::test]
    async fn round_trips_are_counted_by_family() {
        let page = ListPage::Messages(MessagePage {
            total: 0,
            rows: vec![],
        });
        let (client, _) = client(vec![
            Ok(Resp::Page(page.clone())),
            Ok(Resp::Page(page)),
            Ok(Resp::Count(4)),
        ]);
        let request = PageRequest {
            scope: scope(),
            offset: 0,
            limit: 20,
        };
        client.list_page(request).await.unwrap();
        client.list_page(request).await.unwrap();
        client.list_count(scope()).await.unwrap();
        assert_eq!(client.counts().of("Page"), 2);
        assert_eq!(client.counts().of("Count"), 1);
        assert_eq!(client.counts().of("Body"), 0);
    }

    #[tokio::test]
    async fn a_draft_write_is_handed_over_when_asked_not_when_awaited() {
        // A composer hands over an autosave, a send and a discard from
        // signal handlers and awaits each answer somewhere else, later. The
        // order they reach the host has to be the order they were made, so
        // each is handed over at the call, before anything polls it.
        let (client, fake) = client(vec![
            Ok(Resp::DraftSaved(DraftId::new(9))),
            Ok(Resp::Queued(None)),
            Ok(Resp::Done),
        ]);
        let draft = Draft::new(AccountId::new(1));
        let saved = client.save_draft(1, draft.clone());
        let queued = client.queue_send(1, draft, None);
        let discarded = client.discard_draft(1, None);
        let families: Vec<&str> = fake.asked.lock().unwrap().iter().map(Req::family).collect();
        assert_eq!(families, ["SaveDraft", "QueueSend", "DiscardDraft"]);
        // And nothing about the answers borrows the client.
        drop(client);
        assert_eq!(saved.await, Ok(DraftId::new(9)));
        assert_eq!(queued.await, Ok(None));
        assert_eq!(discarded.await, Ok(()));
    }
}
