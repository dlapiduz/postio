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
    fn call(&self, request: Req) -> Call<'_>;

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
            Req::SaveDraft { .. } => "SaveDraft",
            Req::QueueSend { .. } => "QueueSend",
            Req::DiscardDraft { .. } => "DiscardDraft",
            Req::Recipients { .. } => "Recipients",
            Req::ReplySource(_) => "ReplySource",
            Req::DraftBehind(_) => "DraftBehind",
            Req::CancelSend(_) => "CancelSend",
            Req::SendFailure(_) => "SendFailure",
            Req::DefaultSignature { .. } => "DefaultSignature",
            Req::Attach(_) => "Attach",
            Req::InlineImage { .. } => "InlineImage",
            Req::Search(_) => "Search",
            Req::Diagnose(_) => "Diagnose",
            Req::Discover(_) => "Discover",
            Req::AddAccount(_) => "AddAccount",
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

    /// Autosave `draft` as composition `generation`; the answer is its id.
    pub async fn save_draft(&self, generation: u64, draft: Draft) -> Result<DraftId, StoreError> {
        let request = Req::SaveDraft {
            generation,
            draft: Box::new(draft),
        };
        self.read(request, "a saved draft", |answer| match answer {
            Resp::DraftSaved(id) => Some(id),
            _ => None,
        })
        .await
    }

    /// Queue `draft` to send, now or at `at`.
    pub async fn queue_send(
        &self,
        generation: u64,
        draft: Draft,
        at: Option<DateTime<Utc>>,
    ) -> Result<Option<MailboxId>, StoreError> {
        let request = Req::QueueSend {
            generation,
            draft: Box::new(draft),
            at,
        };
        self.read(request, "a queued send", |answer| match answer {
            Resp::Queued(moved) => Some(moved),
            _ => None,
        })
        .await
    }

    /// Composition `generation` closed with nothing worth keeping.
    pub async fn discard_draft(
        &self,
        generation: u64,
        known: Option<DraftId>,
    ) -> Result<(), StoreError> {
        let request = Req::DiscardDraft { generation, known };
        self.read(request, "a discard", |answer| match answer {
            Resp::Done => Some(()),
            _ => None,
        })
        .await
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

    /// Store the file at `path` as an attachment.
    pub async fn attach(
        &self,
        path: std::path::PathBuf,
    ) -> Result<Option<postio_model::Attachment>, StoreError> {
        self.read(Req::Attach(path), "an attachment", |answer| match answer {
            Resp::Attached(found) => Some(found),
            _ => None,
        })
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
        fn call(&self, request: Req) -> Call<'_> {
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
}
