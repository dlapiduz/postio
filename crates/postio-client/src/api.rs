//! Commands go down, events come up, and list reads are answered by the store's host in the same
//! process (ADR 0041). `Client` implements `MailStore`, so a frontend cannot tell the difference.

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
use crate::protocol::{ListMailFound, Queued, Req, Resp};

macro_rules! only {
    ($variant:path) => {
        |answer| match answer {
            $variant(value) => Some(value),
            _ => None,
        }
    };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("Postio's store is not answering.")]
pub struct Disconnected;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SendError {
    #[error("Postio is shutting down and did not take that.")]
    Stopped,
    #[error(transparent)]
    Disconnected(#[from] Disconnected),
}

pub type Call<'a> = Pin<Box<dyn Future<Output = Result<Resp, Disconnected>> + Send + 'a>>;

pub trait Transport: Send + Sync + 'static {
    /// Handed over by this call, not when first polled: two calls in order reach the host in that
    /// order. Hence `'static`.
    fn call(&self, request: Req) -> Call<'static>;

    fn post(&self, request: Req);

    fn events(&self) -> async_channel::Receiver<EventEnvelope>;
}

impl Req {
    /// True for Focus's search reads, which a keystroke supersedes; a write stopped halfway is
    /// worse than a wasted answer.
    pub fn cancellable(&self) -> bool {
        matches!(
            self,
            Req::Conversations { .. }
                | Req::Passages { .. }
                | Req::ConversationMatches { .. }
                | Req::Files { .. }
                | Req::People { .. }
                | Req::Relaxations { .. }
                | Req::Suggest { .. }
                | Req::RecentSearches
                | Req::SavedCounts { .. }
        )
    }

    pub fn family(&self) -> &'static str {
        match self {
            Req::Send(..) => "Send",
            Req::SendTracked(..) => "SendTracked",
            Req::SendMatching { .. } => "SendMatching",
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
            Req::Thread(_) => "Thread",
            Req::Mute(_) => "Mute",
            Req::Unmute(_) => "Unmute",
            Req::Split(_) => "Split",
            Req::MutedOf(_) => "MutedOf",
            Req::ForwardThread { .. } => "ForwardThread",
            Req::SetForwardForm { .. } => "SetForwardForm",
            Req::ReplyToLatest { .. } => "ReplyToLatest",
            Req::Unsubscribe(_) => "Unsubscribe",
            Req::ListMail(_) => "ListMail",
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
            Req::ContactForAddress(_) => "ContactForAddress",
            Req::ContactGlance(_) => "ContactGlance",
            Req::ContactsPage { .. } => "ContactsPage",
            Req::ContactsFiltered { .. } => "ContactsFiltered",
            Req::ContactsOverview => "ContactsOverview",
            Req::ContactCard(_) => "ContactCard",
            Req::AgendaPage { .. } => "AgendaPage",
            Req::EventDetail(_) => "EventDetail",
            Req::ContactGroups => "ContactGroups",
            Req::ContactsSyncBringIn { .. } => "ContactsSyncBringIn",
            Req::ContactsImport(_) => "ContactsImport",
            Req::ContactsSyncPlan { .. } => "ContactsSyncPlan",
            Req::ContactsExport => "ContactsExport",
            Req::ContactSeen(_) => "ContactSeen",
            Req::ContactSuggestions => "ContactSuggestions",
            Req::ContactSuggestionSet(..) => "ContactSuggestionSet",
            Req::ContactReview(_) => "ContactReview",
            Req::ContactPosition(..) => "ContactPosition",
            Req::ContactMergeOf(_) => "ContactMergeOf",
            Req::RecipientDirectory(_) => "RecipientDirectory",
            Req::Labels(_) => "Labels",
            Req::LabelCounts(_) => "LabelCounts",
            Req::CreateLabel { .. } => "CreateLabel",
            Req::ThreadLabels(_) => "ThreadLabels",
            Req::FocusCounts => "FocusCounts",
            Req::UndoTop => "UndoTop",
            Req::ReplySource(_) => "ReplySource",
            Req::DraftBehind(_) => "DraftBehind",
            Req::DraftById(_) => "DraftById",
            Req::CancelSend(_) => "CancelSend",
            Req::SendFailure(_) => "SendFailure",
            Req::DefaultSignature { .. } => "DefaultSignature",
            Req::Attach { .. } => "Attach",
            Req::AttachmentBytes(_) => "AttachmentBytes",
            Req::RecoverDraft(_) => "RecoverDraft",
            Req::InlineImage { .. } => "InlineImage",
            Req::AttachRemoteImage { .. } => "AttachRemoteImage",
            Req::Search(_) => "Search",
            Req::SearchHits { .. } => "SearchHits",
            Req::Facets { .. } => "Facets",
            Req::Conversations { .. } => "Conversations",
            Req::Passages { .. } => "Passages",
            Req::ConversationMatches { .. } => "ConversationMatches",
            Req::Files { .. } => "Files",
            Req::People { .. } => "People",
            Req::AttachmentCopy { .. } => "AttachmentCopy",
            Req::Relaxations { .. } => "Relaxations",
            Req::Suggest { .. } => "Suggest",
            Req::RecentSearches => "RecentSearches",
            Req::RememberSearch { .. } => "RememberSearch",
            Req::ForgetSearch(_) => "ForgetSearch",
            Req::SavedCounts { .. } => "SavedCounts",
            Req::MarkSeen(_) => "MarkSeen",
            Req::StoredBody(_) => "StoredBody",
            Req::ExportMessages(_) => "ExportMessages",
            Req::Account(_) => "Account",
            Req::Discover { .. } => "Discover",
            Req::BeginOAuth { .. } => "BeginOAuth",
            Req::FinishOAuth(_) => "FinishOAuth",
            Req::CancelOAuth(_) => "CancelOAuth",
            Req::AddAccount { .. } => "AddAccount",
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
            Req::MoveRecent => "MoveRecent",
            Req::FilteredTabs => "FilteredTabs",
            Req::DeliveryMessages(_) => "DeliveryMessages",
            Req::DigestWaiting(_) => "DigestWaiting",
            Req::Held(_) => "Held",
            Req::AccountOf(_) => "AccountOf",
            Req::Filtered { .. } => "Filtered",
            Req::NoteMove(_) => "NoteMove",
            Req::FetchBody(_) => "FetchBody",
            Req::FetchCarried { .. } => "FetchCarried",
            Req::StorageCeiling(_) => "StorageCeiling",
            Req::SaveAccount { .. } => "SaveAccount",
            Req::SweepPreview => "SweepPreview",
            Req::RawSource(_) => "RawSource",
            Req::Surfaced => "Surfaced",
            Req::DigestPreview { .. } => "DigestPreview",
            Req::DigestSummary(_) => "DigestSummary",
            Req::DigestLikeThis(_) => "DigestLikeThis",
            Req::SaveDigestRule { .. } => "SaveDigestRule",
            Req::DeleteDigestRule(_) => "DeleteDigestRule",
            Req::TakeBackFilter { .. } => "TakeBackFilter",
            Req::Treatments => "Treatments",
            Req::SetTreatment { .. } => "SetTreatment",
            Req::Vault { .. } => "Vault",
            Req::CaptureTask { .. } => "CaptureTask",
            Req::CaptureNote { .. } => "CaptureNote",
        }
    }
}

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
    pub fn new(transport: Arc<dyn Transport>) -> Self {
        Client {
            transport,
            counts: Arc::new(Counts::default()),
            state: SharedState::default(),
        }
    }

    pub fn with_state(mut self, state: SharedState) -> Self {
        self.state = state;
        self
    }

    pub fn counts(&self) -> &Counts {
        &self.counts
    }

    pub fn events(&self) -> async_channel::Receiver<EventEnvelope> {
        self.transport.events()
    }

    pub fn fetch_body(&self, message: MessageId) {
        self.counts.record("FetchBody");
        self.transport.post(Req::FetchBody(message));
    }

    /// Asks for the `parts` now rather than when the send drains (#1690).
    pub async fn fetch_carried(
        &self,
        message: MessageId,
        parts: Vec<String>,
    ) -> Result<postio_model::fetch::PayloadAsk, StoreError> {
        if parts.is_empty() {
            return Ok(postio_model::fetch::PayloadAsk::Local);
        }
        self.read(
            Req::FetchCarried { message, parts },
            "a forward's parts",
            only!(Resp::Carried),
        )
        .await
    }

    pub fn storage_ceiling(&self, max_bytes: Option<u64>) {
        self.counts.record("StorageCeiling");
        self.transport.post(Req::StorageCeiling(max_bytes));
    }

    async fn call(&self, request: Req) -> Result<Resp, Disconnected> {
        self.counts.record(request.family());
        self.transport.call(request).await
    }

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

    pub async fn accounts(&self) -> Result<Vec<postio_model::Account>, StoreError> {
        self.read(Req::Accounts, "the accounts", only!(Resp::Accounts))
            .await
    }

    pub async fn body(&self, message: MessageId) -> Result<crate::protocol::Body, StoreError> {
        self.read(Req::Body(message), "a body", only!(Resp::Body))
            .await
    }

    pub async fn conversation(
        &self,
        thread: postio_model::ThreadId,
    ) -> Result<Vec<MessageSummary>, StoreError> {
        self.read(Req::Conversation(thread), "a thread", only!(Resp::Rows))
            .await
    }

    pub async fn thread(
        &self,
        thread: postio_model::ThreadId,
    ) -> Result<Option<postio_model::ThreadView>, StoreError> {
        self.read(Req::Thread(thread), "a thread", |answer| match answer {
            Resp::Thread(view) => Some(view.map(|view| *view)),
            _ => None,
        })
        .await
    }

    /// New replies are archived on arrival, except one addressed to the person alone.
    pub async fn mute(&self, thread: postio_model::ThreadId) -> Result<(), StoreError> {
        self.read(Req::Mute(thread), "a mute", |answer| match answer {
            Resp::Done => Some(()),
            _ => None,
        })
        .await
    }

    pub async fn unmute(&self, thread: postio_model::ThreadId) -> Result<(), StoreError> {
        self.read(Req::Unmute(thread), "an unmute", |answer| match answer {
            Resp::Done => Some(()),
            _ => None,
        })
        .await
    }

    pub async fn split(&self, message: MessageId) -> Result<postio_model::ThreadId, StoreError> {
        self.read(Req::Split(message), "a split", only!(Resp::Split))
            .await
    }

    pub async fn muted_of(
        &self,
        threads: Vec<postio_model::ThreadId>,
    ) -> Result<Vec<(postio_model::ThreadId, u32)>, StoreError> {
        self.read(
            Req::MutedOf(threads),
            "the muted threads",
            only!(Resp::Muted),
        )
        .await
    }

    pub async fn forward_thread(
        &self,
        thread: postio_model::ThreadId,
        form: postio_model::ForwardForm,
    ) -> Result<postio_model::Forwarded, StoreError> {
        self.read(
            Req::ForwardThread { thread, form },
            "a thread forward",
            only!(Resp::Forwarded),
        )
        .await
    }

    pub async fn set_forward_form(
        &self,
        draft: Draft,
        form: postio_model::ForwardForm,
    ) -> Result<postio_model::Forwarded, StoreError> {
        self.read(
            Req::SetForwardForm {
                draft: Box::new(draft),
                form,
            },
            "a forward form",
            only!(Resp::Forwarded),
        )
        .await
    }

    pub async fn reply_to_latest(&self, draft: Draft) -> Result<Draft, StoreError> {
        self.read(
            Req::ReplyToLatest {
                draft: Box::new(draft),
            },
            "a reply to the latest",
            |answer| match answer {
                Resp::Draft(Some(draft)) => Some(*draft),
                _ => None,
            },
        )
        .await
    }

    pub async fn unsubscribe(&self, message: MessageId) -> Result<String, StoreError> {
        self.read(
            Req::Unsubscribe(message),
            "an unsubscribe",
            only!(Resp::Unsubscribed),
        )
        .await
    }

    pub async fn list_mail(&self, message: MessageId) -> Result<ListMailFound, StoreError> {
        self.read(
            Req::ListMail(message),
            "a list's mail",
            only!(Resp::ListMail),
        )
        .await
    }

    pub async fn parts(
        &self,
        message: MessageId,
    ) -> Result<Vec<postio_model::Attachment>, StoreError> {
        self.read(Req::Parts(message), "the parts", only!(Resp::Parts))
            .await
    }

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
        self.read(request, "a saved part", only!(Resp::Saved)).await
    }

    pub async fn open_part(
        &self,
        message: MessageId,
        attachment: postio_model::ids::AttachmentId,
    ) -> Result<std::path::PathBuf, StoreError> {
        let request = Req::OpenPart {
            message,
            attachment,
        };
        self.read(request, "a part to open", only!(Resp::Saved))
            .await
    }

    pub async fn readings(
        &self,
        messages: Vec<MessageId>,
        offline: bool,
    ) -> Result<Vec<crate::protocol::Reading>, StoreError> {
        let request = Req::Readings { messages, offline };
        self.read(request, "a reading", only!(Resp::Readings)).await
    }

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
        self.read(request, "a thread's reading", only!(Resp::Readings))
            .await
    }

    pub async fn inline_part(
        &self,
        message: MessageId,
        content_id: String,
    ) -> Result<Option<(Vec<u8>, String)>, StoreError> {
        let request = Req::InlinePart {
            message,
            content_id,
        };
        self.read(request, "an inline part", only!(Resp::InlinePart))
            .await
    }

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

    /// For writes that must land in the order they were made however their answers are awaited.
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

    pub fn save_draft(
        &self,
        generation: u64,
        draft: Draft,
    ) -> impl Future<Output = Result<DraftId, StoreError>> + Send + 'static {
        let request = Req::SaveDraft {
            generation,
            draft: Box::new(draft),
        };
        self.hand_over(request, "a saved draft", only!(Resp::DraftSaved))
    }

    pub fn queue_send(
        &self,
        generation: u64,
        draft: Draft,
        at: Option<DateTime<Utc>>,
    ) -> impl Future<Output = Result<Queued, StoreError>> + Send + 'static {
        let request = Req::QueueSend {
            generation,
            draft: Box::new(draft),
            at,
        };
        self.hand_over(request, "a queued send", only!(Resp::Queued))
    }

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

    pub async fn recipients(
        &self,
        account: AccountId,
        prefix: String,
    ) -> Result<Vec<RecipientCandidate>, StoreError> {
        let request = Req::Recipients { account, prefix };
        self.read(request, "recipients", only!(Resp::Recipients))
            .await
    }

    pub async fn take_back_filter(
        &self,
        entry: &str,
        kind: Option<&str>,
    ) -> Result<(), StoreError> {
        self.read(
            Req::TakeBackFilter {
                entry: entry.to_owned(),
                kind: kind.map(str::to_owned),
            },
            "taking back a filter entry",
            |answer| match answer {
                Resp::Done => Some(()),
                _ => None,
            },
        )
        .await
    }

    pub async fn treatments(&self) -> Result<Vec<(String, String)>, StoreError> {
        self.read(
            Req::Treatments,
            "the senders' treatments",
            only!(Resp::Treatments),
        )
        .await
    }

    pub async fn set_treatment(
        &self,
        address: &str,
        treatment: Option<&str>,
    ) -> Result<(), StoreError> {
        self.read(
            Req::SetTreatment {
                address: address.to_owned(),
                treatment: treatment.map(str::to_owned),
            },
            "a sender's treatment",
            |answer| match answer {
                Resp::Done => Some(()),
                _ => None,
            },
        )
        .await
    }

    /// `None` for an address the store never saw.
    pub async fn contact_for_address(
        &self,
        address: &str,
    ) -> Result<Option<postio_model::Contact>, StoreError> {
        self.read(
            Req::ContactForAddress(address.to_owned()),
            "the person at an address",
            |answer| match answer {
                Resp::ContactFor(found) => Some(found.map(|person| *person)),
                _ => None,
            },
        )
        .await
    }

    pub async fn contact_glance(
        &self,
        address: &str,
    ) -> Result<Option<postio_model::ContactGlance>, StoreError> {
        self.read(
            Req::ContactGlance(address.to_owned()),
            "the person at an address, at a glance",
            |answer| match answer {
                Resp::ContactGlance(found) => Some(found.map(|glance| *glance)),
                _ => None,
            },
        )
        .await
    }

    pub async fn contacts_page(
        &self,
        tab: postio_model::ContactTab,
        after: Option<(String, postio_model::ContactId)>,
        skip: u32,
        limit: u32,
    ) -> Result<postio_model::ContactsPageRead, StoreError> {
        self.read(
            Req::ContactsPage {
                tab,
                after,
                skip,
                limit,
            },
            "a page of contacts",
            only!(Resp::ContactsPage),
        )
        .await
    }

    pub async fn agenda_page(
        &self,
        anchor: postio_model::calendar::AgendaAnchor,
        limit: u32,
    ) -> Result<Vec<postio_model::calendar::AgendaEntry>, StoreError> {
        self.read(
            Req::AgendaPage { anchor, limit },
            "a page of the agenda",
            only!(Resp::AgendaPage),
        )
        .await
    }

    pub async fn event_detail(
        &self,
        cursor: postio_model::calendar::AgendaCursor,
    ) -> Result<Option<Box<postio_model::calendar::EventDetail>>, StoreError> {
        self.read(
            Req::EventDetail(cursor),
            "an event",
            only!(Resp::EventDetail),
        )
        .await
    }

    pub async fn contacts_filtered(
        &self,
        tab: postio_model::ContactTab,
        text: &str,
        cap: u32,
    ) -> Result<Vec<postio_model::ContactListRow>, StoreError> {
        self.read(
            Req::ContactsFiltered {
                tab,
                text: text.to_owned(),
                cap,
            },
            "the contacts matching a filter",
            only!(Resp::ContactsFiltered),
        )
        .await
    }

    pub async fn contacts_overview(&self) -> Result<postio_model::ContactsOverview, StoreError> {
        self.read(
            Req::ContactsOverview,
            "the contacts overview",
            only!(Resp::ContactsOverview),
        )
        .await
    }

    pub async fn contact_card(
        &self,
        person: postio_model::ContactId,
    ) -> Result<Option<postio_model::ContactCardRead>, StoreError> {
        self.read(
            Req::ContactCard(person),
            "a contact's card",
            |answer| match answer {
                Resp::ContactCard(card) => Some(card.map(|card| *card)),
                _ => None,
            },
        )
        .await
    }

    pub async fn contacts_sync_plan(
        &self,
        records: Vec<postio_contact_sync::ExternalRecord>,
        scope: postio_contact_sync::Scope,
        mode: postio_contact_sync::Mode,
    ) -> Result<postio_contact_sync::Plan, StoreError> {
        self.read(
            Req::ContactsSyncPlan {
                records,
                scope,
                mode,
            },
            "the sync plan",
            |answer| match answer {
                Resp::ContactsSyncPlan(plan) => Some(*plan),
                _ => None,
            },
        )
        .await
    }

    /// Only if `shown` is still the plan's fingerprint.
    pub async fn contacts_sync_bring_in(
        &self,
        records: Vec<postio_contact_sync::ExternalRecord>,
        scope: postio_contact_sync::Scope,
        mode: postio_contact_sync::Mode,
        shown: u64,
    ) -> Result<postio_contact_sync::BroughtIn, StoreError> {
        self.read(
            Req::ContactsSyncBringIn {
                records,
                scope,
                mode,
                shown,
            },
            "bringing contacts in",
            |answer| match answer {
                Resp::ContactsBroughtIn(done) => Some(*done),
                _ => None,
            },
        )
        .await
    }

    pub async fn import_contacts(
        &self,
        bytes: Vec<u8>,
    ) -> Result<postio_model::card::ImportSummary, StoreError> {
        self.read(
            Req::ContactsImport(bytes),
            "an import",
            |answer| match answer {
                Resp::ContactsImported(summary) => Some(*summary),
                _ => None,
            },
        )
        .await
    }

    pub async fn export_contacts(&self) -> Result<(String, usize), StoreError> {
        self.read(Req::ContactsExport, "an export", |answer| match answer {
            Resp::ContactsExported(text, count) => Some((text, count)),
            _ => None,
        })
        .await
    }

    pub async fn contact_review(
        &self,
        first: Vec<postio_model::ContactId>,
    ) -> Result<Vec<postio_model::MergeGroup>, StoreError> {
        self.read(
            Req::ContactReview(first),
            "the review",
            only!(Resp::ContactReview),
        )
        .await
    }

    pub async fn contact_position(
        &self,
        person: postio_model::ContactId,
        tab: postio_model::ContactTab,
    ) -> Result<Option<u32>, StoreError> {
        self.read(
            Req::ContactPosition(person, tab),
            "a person's place in the list",
            only!(Resp::ContactPosition),
        )
        .await
    }

    pub async fn contact_merge_of(
        &self,
        ids: Vec<postio_model::ContactId>,
    ) -> Result<Option<postio_model::MergeGroup>, StoreError> {
        self.read(
            Req::ContactMergeOf(ids),
            "the records to merge",
            |answer| match answer {
                Resp::ContactMergeOf(group) => Some(group.map(|group| *group)),
                _ => None,
            },
        )
        .await
    }

    pub async fn contact_suggestions(
        &self,
    ) -> Result<postio_model::ContactSuggestions, StoreError> {
        self.read(
            Req::ContactSuggestions,
            "the Suggestions switches",
            only!(Resp::ContactSuggestions),
        )
        .await
    }

    pub async fn set_contact_suggestion(
        &self,
        which: postio_model::Suggestion,
        on: bool,
    ) -> Result<postio_model::ContactSuggestions, StoreError> {
        self.read(
            Req::ContactSuggestionSet(which, on),
            "a Suggestions switch",
            only!(Resp::ContactSuggestions),
        )
        .await
    }

    pub async fn contact_seen(
        &self,
        person: postio_model::ContactId,
    ) -> Result<Vec<postio_model::SeenItem>, StoreError> {
        self.read(
            Req::ContactSeen(person),
            "what the mail says",
            only!(Resp::ContactSeen),
        )
        .await
    }

    pub async fn contact_groups(&self) -> Result<Vec<String>, StoreError> {
        self.read(
            Req::ContactGroups,
            "the contact groups",
            only!(Resp::ContactGroups),
        )
        .await
    }

    pub async fn recipient_directory(
        &self,
        account: AccountId,
    ) -> Result<crate::protocol::RecipientDirectory, StoreError> {
        self.read(
            Req::RecipientDirectory(account),
            "the recipient directory",
            only!(Resp::RecipientDirectory),
        )
        .await
    }

    pub async fn correspondents(
        &self,
        account: AccountId,
    ) -> Result<Vec<postio_model::Contact>, StoreError> {
        self.read(
            Req::Correspondents(account),
            "correspondents",
            only!(Resp::Correspondents),
        )
        .await
    }

    pub async fn labels(&self, account: AccountId) -> Result<Vec<postio_model::Label>, StoreError> {
        self.read(Req::Labels(account), "labels", only!(Resp::Labels))
            .await
    }

    pub async fn label_counts(
        &self,
        account: AccountId,
    ) -> Result<Vec<(postio_model::LabelId, u32)>, StoreError> {
        self.read(
            Req::LabelCounts(account),
            "label counts",
            only!(Resp::LabelCounts),
        )
        .await
    }

    pub async fn create_label(
        &self,
        account: AccountId,
        name: String,
    ) -> Result<Option<postio_model::Label>, StoreError> {
        self.read(
            Req::CreateLabel { account, name },
            "a new label",
            only!(Resp::Label),
        )
        .await
    }

    pub async fn thread_labels(
        &self,
        threads: Vec<postio_model::ThreadId>,
    ) -> Result<Vec<(postio_model::ThreadId, postio_model::Label)>, StoreError> {
        self.read(
            Req::ThreadLabels(threads),
            "labels",
            only!(Resp::ThreadLabels),
        )
        .await
    }

    pub async fn focus_counts(&self) -> Result<crate::protocol::FocusCounts, StoreError> {
        self.read(Req::FocusCounts, "Focus's counts", only!(Resp::FocusCounts))
            .await
    }

    pub async fn undo_top(&self) -> Result<Option<String>, StoreError> {
        self.read(Req::UndoTop, "the undo top", only!(Resp::UndoTop))
            .await
    }

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

    pub async fn draft_by_id(&self, draft: DraftId) -> Result<Option<Draft>, StoreError> {
        self.read(Req::DraftById(draft), "a draft", |answer| match answer {
            Resp::Draft(found) => Some(found.map(|found| *found)),
            _ => None,
        })
        .await
    }

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

    pub async fn send_failure(&self, draft: DraftId) -> Result<Option<String>, StoreError> {
        self.read(
            Req::SendFailure(draft),
            "a send failure",
            only!(Resp::SendFailure),
        )
        .await
    }

    pub async fn default_signature(
        &self,
        account: AccountId,
        selected: Option<MailboxId>,
    ) -> Result<Option<postio_model::SignatureId>, StoreError> {
        let request = Req::DefaultSignature { account, selected };
        self.read(request, "a signature", only!(Resp::Signature))
            .await
    }

    pub async fn attach(
        &self,
        path: std::path::PathBuf,
    ) -> Result<Option<postio_model::Attachment>, StoreError> {
        self.attach_as(path, None).await
    }

    pub async fn attach_as(
        &self,
        path: std::path::PathBuf,
        mime_type: Option<String>,
    ) -> Result<Option<postio_model::Attachment>, StoreError> {
        let request = Req::Attach { path, mime_type };
        self.read(request, "an attachment", only!(Resp::Attached))
            .await
    }

    pub async fn attachment_bytes(
        &self,
        blob: postio_model::ids::BlobId,
    ) -> Result<Option<Vec<u8>>, StoreError> {
        self.read(
            Req::AttachmentBytes(blob),
            "an attachment's bytes",
            only!(Resp::Bytes),
        )
        .await
    }

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

    pub async fn attach_remote_image(
        &self,
        draft: postio_model::DraftId,
        href: String,
    ) -> Result<Result<postio_model::Attachment, postio_ui::compose::RemoteRefusal>, StoreError>
    {
        let request = Req::AttachRemoteImage { draft, href };
        self.read(request, "a remote image", |answer| match answer {
            Resp::RemoteImage(outcome) => Some(outcome),
            _ => None,
        })
        .await
    }

    pub async fn inline_image(
        &self,
        bytes: Vec<u8>,
        mime_type: String,
    ) -> Result<Option<postio_model::Attachment>, StoreError> {
        let request = Req::InlineImage { bytes, mime_type };
        self.read(request, "an inline image", only!(Resp::Attached))
            .await
    }

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

    pub async fn discover(
        &self,
        address: String,
    ) -> Result<postio_ui::onboarding::Status, StoreError> {
        self.discover_until(address, crate::protocol::Stop::new())
            .await
    }

    /// A stopped probe answers as one that found nothing.
    pub async fn discover_until(
        &self,
        address: String,
        stop: crate::protocol::Stop,
    ) -> Result<postio_ui::onboarding::Status, StoreError> {
        let request = Req::Discover { address, stop };
        self.read(request, "discovery", |answer| match answer {
            Resp::Onboarding(status) => Some(*status),
            _ => None,
        })
        .await
    }

    pub async fn begin_oauth(
        &self,
        submission: postio_ui::onboarding::Submission,
    ) -> Result<postio_ui::onboarding::BrowserSignIn, StoreError> {
        self.begin_oauth_then(submission, crate::protocol::AfterSave::Sync)
            .await
    }

    pub async fn begin_oauth_then(
        &self,
        submission: postio_ui::onboarding::Submission,
        then: crate::protocol::AfterSave,
    ) -> Result<postio_ui::onboarding::BrowserSignIn, StoreError> {
        let request = Req::BeginOAuth {
            submission: Box::new(submission),
            then,
        };
        self.read(request, "a sign-in", |answer| match answer {
            Resp::Consent(consent) => Some(*consent),
            _ => None,
        })
        .await
    }

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

    pub async fn add_account(
        &self,
        submission: postio_ui::onboarding::Submission,
    ) -> Result<(), StoreError> {
        self.add_account_then(submission, crate::protocol::AfterSave::Sync)
            .await
    }

    pub async fn add_account_then(
        &self,
        submission: postio_ui::onboarding::Submission,
        then: crate::protocol::AfterSave,
    ) -> Result<(), StoreError> {
        self.read(
            Req::AddAccount {
                submission: Box::new(submission),
                then,
            },
            "a new account",
            |answer| match answer {
                Resp::Done => Some(()),
                _ => None,
            },
        )
        .await
    }

    pub async fn search(
        &self,
        search: crate::protocol::Search,
    ) -> Result<Option<crate::protocol::Found>, StoreError> {
        self.read(Req::Search(search), "a search", only!(Resp::Found))
            .await
    }

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
        self.read(request, "the facet counts", only!(Resp::Facets))
            .await
    }

    pub async fn conversations(
        &self,
        account: postio_model::AccountScope,
        query: postio_search::ParsedQuery,
        order: postio_search::results::ConversationOrder,
        offset: u32,
        limit: u32,
    ) -> Result<Option<postio_search::results::ConversationResults>, StoreError> {
        let request = Req::Conversations {
            account,
            query,
            order,
            offset,
            limit,
        };
        self.read(request, "a search", |answer| match answer {
            Resp::Conversations(found) => Some(found.map(|found| found.0)),
            _ => None,
        })
        .await
    }

    pub async fn passages(
        &self,
        query: postio_search::ParsedQuery,
        hits: Vec<(MessageId, Vec<postio_search::results::Source>)>,
        first_line: postio_search::passage::FirstLine,
    ) -> Result<Vec<(MessageId, Vec<postio_search::results::Match>)>, StoreError> {
        self.read(
            Req::Passages {
                query,
                hits,
                first_line,
            },
            "the passages",
            only!(Resp::Passages),
        )
        .await
    }

    pub async fn conversation_matches(
        &self,
        query: postio_search::ParsedQuery,
        key: postio_search::results::ConversationKey,
    ) -> Result<Vec<postio_search::results::ConversationMatch>, StoreError> {
        self.read(
            Req::ConversationMatches { query, key },
            "a thread's matches",
            only!(Resp::Matches),
        )
        .await
    }

    pub async fn files(
        &self,
        account: postio_model::AccountScope,
        query: postio_search::ParsedQuery,
        offset: u32,
        limit: u32,
    ) -> Result<Vec<postio_search::results::FileHit>, StoreError> {
        let request = Req::Files {
            account,
            query,
            offset,
            limit,
        };
        self.read(request, "the files", only!(Resp::Files)).await
    }

    pub async fn people(
        &self,
        account: postio_model::AccountScope,
        query: postio_search::ParsedQuery,
        offset: u32,
        limit: u32,
    ) -> Result<Vec<postio_search::suggest::Person>, StoreError> {
        let request = Req::People {
            account,
            query,
            offset,
            limit,
        };
        self.read(request, "the people", only!(Resp::People)).await
    }

    pub async fn attachment_copy(
        &self,
        attachment: postio_model::ids::AttachmentId,
        dir: std::path::PathBuf,
    ) -> Result<Option<std::path::PathBuf>, StoreError> {
        let request = Req::AttachmentCopy { attachment, dir };
        self.read(request, "a copy of a file", only!(Resp::Path))
            .await
    }

    pub async fn relaxations(
        &self,
        account: postio_model::AccountScope,
        query: postio_search::ParsedQuery,
        today: chrono::NaiveDate,
    ) -> Result<
        Vec<(
            postio_search::relax::Relaxation,
            postio_search::results::Counted,
        )>,
        StoreError,
    > {
        let request = Req::Relaxations {
            account,
            query,
            today,
        };
        self.read(request, "the ways out", only!(Resp::Relaxed))
            .await
    }

    pub async fn suggest(
        &self,
        account: postio_model::AccountScope,
        prefix: String,
        field: Option<postio_search::query::Field>,
    ) -> Result<postio_search::suggest::Suggestions, StoreError> {
        let request = Req::Suggest {
            account,
            prefix,
            field,
        };
        self.read(request, "the suggestions", |answer| match answer {
            Resp::Suggestions(found) => Some(*found),
            _ => None,
        })
        .await
    }

    pub async fn recent_searches(&self) -> Result<Vec<crate::protocol::RecentSearch>, StoreError> {
        self.read(
            Req::RecentSearches,
            "the recent searches",
            only!(Resp::Recent),
        )
        .await
    }

    pub async fn remember_search(
        &self,
        query: String,
        hits: u64,
        capped: bool,
    ) -> Result<(), StoreError> {
        self.read(
            Req::RememberSearch {
                query,
                hits,
                capped,
            },
            "a remembered search",
            |answer| match answer {
                Resp::Done => Some(()),
                _ => None,
            },
        )
        .await
    }

    pub async fn mark_seen(&self, key: String) -> Result<(), StoreError> {
        self.done(Req::MarkSeen(key), "a saved search seen").await
    }

    pub async fn forget_search(&self, query: String) -> Result<(), StoreError> {
        self.read(
            Req::ForgetSearch(query),
            "a forgotten search",
            |answer| match answer {
                Resp::Done => Some(()),
                _ => None,
            },
        )
        .await
    }

    pub async fn saved_counts(
        &self,
        account: postio_model::AccountScope,
        today: chrono::NaiveDate,
        searches: Vec<(String, String)>,
    ) -> Result<Vec<crate::protocol::SavedCount>, StoreError> {
        let request = Req::SavedCounts {
            account,
            today,
            searches,
        };
        self.read(
            request,
            "the saved searches' counts",
            only!(Resp::SavedCounts),
        )
        .await
    }

    pub async fn stored_body(
        &self,
        message: MessageId,
    ) -> Result<postio_model::MessageBody, StoreError> {
        self.read(Req::StoredBody(message), "a body", only!(Resp::StoredBody))
            .await
    }

    pub async fn export_messages(
        &self,
        messages: Vec<(MessageId, std::path::PathBuf)>,
    ) -> Result<Vec<std::path::PathBuf>, StoreError> {
        self.read(
            Req::ExportMessages(messages),
            "an export",
            only!(Resp::Exported),
        )
        .await
    }

    pub async fn account_settings(
        &self,
        weights: bool,
    ) -> Result<Vec<crate::protocol::AccountSettings>, StoreError> {
        let request = Req::AccountSettings { weights };
        self.read(
            request,
            "the account settings",
            only!(Resp::AccountSettings),
        )
        .await
    }

    pub async fn edit_account(
        &self,
        account: AccountId,
        field: crate::protocol::AccountField,
    ) -> Result<(), StoreError> {
        self.done(Req::EditAccount(account, field), "an account edit")
            .await
    }

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

    pub async fn delete_signature(
        &self,
        signature: postio_model::SignatureId,
    ) -> Result<(), StoreError> {
        self.done(Req::DeleteSignature(signature), "a removed signature")
            .await
    }

    pub async fn rebuild_index(&self, account: AccountId) -> Result<(), StoreError> {
        self.done(Req::RebuildIndex(account), "a rebuilt index")
            .await
    }

    pub async fn egress_log(
        &self,
        limit: u32,
    ) -> Result<Vec<postio_model::egress::EgressEvent>, StoreError> {
        self.read(Req::EgressLog(limit), "the egress log", only!(Resp::Egress))
            .await
    }

    pub async fn privacy_log(&self) -> Result<crate::protocol::PrivacyLog, StoreError> {
        self.read(Req::PrivacyLog, "the privacy log", only!(Resp::Privacy))
            .await
    }

    pub async fn set_backfill_excluded(
        &self,
        mailbox: MailboxId,
        excluded: bool,
    ) -> Result<Vec<Mailbox>, StoreError> {
        let request = Req::SetBackfillExcluded { mailbox, excluded };
        self.read(request, "a folder's backfill", only!(Resp::Mailboxes))
            .await
    }

    pub async fn orientation_seen(&self) -> Result<bool, StoreError> {
        self.read(Req::OrientationSeen, "the orientation", only!(Resp::Seen))
            .await
    }

    pub async fn retire_orientation(&self) -> Result<(), StoreError> {
        self.done(Req::RetireOrientation, "the orientation").await
    }

    pub async fn move_recent(&self) -> Result<Vec<MailboxId>, StoreError> {
        self.read(Req::MoveRecent, "recent moves", only!(Resp::MoveRecent))
            .await
    }

    pub async fn account_of(&self, message: MessageId) -> Result<Option<AccountId>, StoreError> {
        self.read(
            Req::AccountOf(message),
            "a message's account",
            only!(Resp::AccountOf),
        )
        .await
    }

    pub async fn held(
        &self,
        messages: Vec<MessageId>,
    ) -> Result<Vec<(MessageId, String, bool)>, StoreError> {
        self.read(Req::Held(messages), "held mail", only!(Resp::Held))
            .await
    }

    pub async fn digest_waiting(&self, rules: Vec<String>) -> Result<Vec<u32>, StoreError> {
        self.read(
            Req::DigestWaiting(rules),
            "what the rules hold",
            only!(Resp::Counts),
        )
        .await
    }

    pub async fn delivery_messages(
        &self,
        delivery: postio_model::DeliveryId,
    ) -> Result<Vec<postio_model::listing::MessageSummary>, StoreError> {
        self.read(
            Req::DeliveryMessages(delivery),
            "a digest's messages",
            only!(Resp::Rows),
        )
        .await
    }

    pub async fn filtered_tabs(&self) -> Result<Vec<(String, u32)>, StoreError> {
        self.read(
            Req::FilteredTabs,
            "the filtered tabs",
            only!(Resp::FilteredTabs),
        )
        .await
    }

    pub async fn filtered(
        &self,
        reason: Option<String>,
        offset: u32,
        limit: u32,
    ) -> Result<Vec<crate::protocol::FilteredRow>, StoreError> {
        self.read(
            Req::Filtered {
                reason,
                offset,
                limit,
            },
            "filtered mail",
            only!(Resp::Filtered),
        )
        .await
    }

    pub async fn note_move(&self, mailbox: MailboxId) -> Result<(), StoreError> {
        self.done(Req::NoteMove(mailbox), "a recent move").await
    }

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

    pub async fn sweep_preview(&self) -> Result<u32, StoreError> {
        self.read(Req::SweepPreview, "a sweep's count", only!(Resp::Count))
            .await
    }

    /// Fetched on this call and no other when not here: the person asked for these bytes by name.
    pub async fn raw_source(&self, message: MessageId) -> Result<Vec<u8>, StoreError> {
        self.read(
            Req::RawSource(message),
            "the message's source",
            only!(Resp::RawSource),
        )
        .await
    }

    pub async fn surfaced(&self) -> Result<Vec<postio_model::listing::Surfaced>, StoreError> {
        self.read(Req::Surfaced, "the surfaced rows", only!(Resp::Surfaced))
            .await
    }

    pub async fn vault(&self, subject: &str) -> Result<crate::protocol::VaultPicture, StoreError> {
        self.read(
            Req::Vault {
                subject: subject.to_owned(),
            },
            "the vault",
            only!(Resp::Vault),
        )
        .await
    }

    /// Appends exactly one line, the note's own bytes untouched.
    pub async fn capture_task(
        &self,
        project: Option<postio_vault::Project>,
        task: postio_vault::Task,
    ) -> Result<postio_vault::Captured, StoreError> {
        self.read(
            Req::CaptureTask { project, task },
            "the task written to the vault",
            only!(Resp::Captured),
        )
        .await
    }

    pub async fn capture_note(
        &self,
        note: std::path::PathBuf,
        entry: postio_vault::NoteEntry,
    ) -> Result<postio_vault::Captured, StoreError> {
        self.read(
            Req::CaptureNote { note, entry },
            "the note written to the vault",
            only!(Resp::Captured),
        )
        .await
    }

    pub async fn digest_like_this(
        &self,
        message: MessageId,
    ) -> Result<Option<crate::protocol::LikeThisRule>, StoreError> {
        self.read(
            Req::DigestLikeThis(message),
            "a rule for mail like this",
            only!(Resp::DigestLikeThis),
        )
        .await
    }

    pub async fn digest_summary(
        &self,
        delivery: postio_model::DeliveryId,
    ) -> Result<Option<postio_model::summary::DigestSummary>, StoreError> {
        self.read(
            Req::DigestSummary(delivery),
            "a digest's summary",
            only!(Resp::DigestSummary),
        )
        .await
    }

    /// Counted through the executor, so it means what the same queries mean in search.
    pub async fn digest_preview(
        &self,
        queries: Vec<String>,
        since: DateTime<Utc>,
    ) -> Result<crate::protocol::DigestPreview, StoreError> {
        let request = Req::DigestPreview { queries, since };
        self.read(request, "a digest's preview", only!(Resp::DigestPreview))
            .await
    }

    pub async fn save_digest_rule(
        &self,
        replacing: Option<String>,
        rule: crate::protocol::DigestRuleDraft,
    ) -> Result<(), StoreError> {
        self.done(
            Req::SaveDigestRule { replacing, rule },
            "a saved digest rule",
        )
        .await
    }

    pub async fn delete_digest_rule(&self, name: String) -> Result<u32, StoreError> {
        self.read(
            Req::DeleteDigestRule(name),
            "a removed digest rule",
            only!(Resp::Count),
        )
        .await
    }

    async fn done(&self, request: Req, asked: &str) -> Result<(), StoreError> {
        self.read(request, asked, |answer| match answer {
            Resp::Done => Some(()),
            _ => None,
        })
        .await
    }

    pub async fn send(&self, command: Command) -> Result<(), SendError> {
        let aim = self.state.read(|state| state.snapshot());
        match self.call(Req::Send(command, aim)).await? {
            Resp::Stopped => Err(SendError::Stopped),
            _ => Ok(()),
        }
    }

    pub async fn send_matching(
        &self,
        command: Command,
        account: postio_model::AccountScope,
        query: postio_search::ParsedQuery,
        except: Vec<MessageId>,
    ) -> Result<(), SendError> {
        match self
            .call(Req::SendMatching {
                command,
                account,
                query,
                except,
            })
            .await?
        {
            Resp::Stopped => Err(SendError::Stopped),
            _ => Ok(()),
        }
    }

    pub async fn send_tracked(&self, command: Command) -> Result<InvocationId, SendError> {
        let aim = self.state.read(|state| state.snapshot());
        match self.call(Req::SendTracked(command, aim)).await? {
            Resp::Tracked(id) => Ok(id),
            _ => Err(SendError::Stopped),
        }
    }
}

fn unexpected(asked: &str) -> StoreError {
    StoreError::new(format!(
        "Postio's store gave an unexpected answer to {asked}."
    ))
}

impl MailStore for Client {
    fn list_page(&self, request: PageRequest) -> Read<'_, ListPage> {
        Box::pin(self.read(Req::Page(request), "a page", only!(Resp::Page)))
    }

    fn list_count(&self, scope: ListScope) -> Read<'_, u32> {
        Box::pin(self.read(Req::Count(scope), "a count", only!(Resp::Count)))
    }

    fn message_rows(&self, ids: Vec<MessageId>) -> Read<'_, Vec<MessageSummary>> {
        Box::pin(self.read(Req::Rows(ids), "rows", only!(Resp::Rows)))
    }

    fn rows_in(&self, scope: ListScope, ids: Vec<MessageId>) -> Read<'_, ListRows> {
        Box::pin(self.read(Req::RowsIn(scope, ids), "rows", only!(Resp::ListRows)))
    }

    fn note_removed(&self, mailbox: MailboxId, messages: Vec<MessageId>) {
        let request = Req::NoteRemoved(mailbox, messages);
        self.counts.record(request.family());
        self.transport.post(request);
    }

    fn mailboxes(&self, account: AccountId) -> Read<'_, Vec<Mailbox>> {
        Box::pin(self.read(Req::Mailboxes(account), "folders", only!(Resp::Mailboxes)))
    }

    fn draft_counts(&self, account: AccountId) -> Read<'_, DraftCounts> {
        Box::pin(self.read(
            Req::DraftCounts(account),
            "draft counts",
            only!(Resp::DraftCounts),
        ))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use postio_model::listing::MessagePage;

    use super::*;

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
        assert_eq!(error.message(), "Postio's store is not answering.");
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
    async fn a_page_s_labels_are_asked_for_by_conversation() {
        let label = postio_model::Label::new(AccountId::new(1), "Atlas");
        let thread = postio_model::ThreadId::new(4);
        let (client, fake) = client(vec![Ok(Resp::ThreadLabels(vec![(thread, label.clone())]))]);
        assert_eq!(
            client.thread_labels(vec![thread]).await,
            Ok(vec![(thread, label)])
        );
        assert_eq!(
            *fake.asked.lock().unwrap(),
            vec![Req::ThreadLabels(vec![thread])]
        );
        assert_eq!(client.counts().of("ThreadLabels"), 1);
    }

    #[tokio::test]
    async fn focus_s_counts_are_one_request() {
        let counts = crate::protocol::FocusCounts {
            conversations: 312,
            unread: 41,
            has_action: 7,
            filtered_today: 186,
        };
        let (client, fake) = client(vec![Ok(Resp::FocusCounts(counts))]);
        assert_eq!(client.focus_counts().await, Ok(counts));
        assert_eq!(*fake.asked.lock().unwrap(), vec![Req::FocusCounts]);
        assert_eq!(client.counts().of("FocusCounts"), 1);
    }

    #[tokio::test]
    async fn the_undo_top_is_one_request() {
        let (client, fake) = client(vec![Ok(Resp::UndoTop(Some("Archived 3 messages".into())))]);
        assert_eq!(
            client.undo_top().await,
            Ok(Some("Archived 3 messages".to_owned()))
        );
        assert_eq!(*fake.asked.lock().unwrap(), vec![Req::UndoTop]);
        assert_eq!(client.counts().of("UndoTop"), 1);
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
        // The autosave, send and discard must reach the host in the order they were made, so each
        // is handed over at the call.
        let (client, fake) = client(vec![
            Ok(Resp::DraftSaved(DraftId::new(9))),
            Ok(Resp::Queued(Queued {
                drafts: None,
                draft: DraftId::new(9),
            })),
            Ok(Resp::Done),
        ]);
        let draft = Draft::new(AccountId::new(1));
        let saved = client.save_draft(1, draft.clone());
        let queued = client.queue_send(1, draft, None);
        let discarded = client.discard_draft(1, None);
        let families: Vec<&str> = fake.asked.lock().unwrap().iter().map(Req::family).collect();
        assert_eq!(families, ["SaveDraft", "QueueSend", "DiscardDraft"]);
        drop(client);
        assert_eq!(saved.await, Ok(DraftId::new(9)));
        assert_eq!(queued.await.map(|queued| queued.draft), Ok(DraftId::new(9)));
        assert_eq!(discarded.await, Ok(()));
    }
}
