//! What a frontend asks of the store's host, and what it answers.
//! The host is in the frontend's own process (ADR 0041), so a request is a value handed over a
//! channel, never encoded.
//! Nothing here waits on the network: every `Req` is answered from local state; anything remote is
//! a `Command` whose outcome arrives as events.

use chrono::{DateTime, Utc};
use postio_core::{Command, InvocationId, StateSnapshot};
use postio_model::ListScope;
use postio_model::contact_group::RecipientCandidate;
use postio_model::ids::{AccountId, MailboxId, MessageId};
use postio_model::listing::{
    DraftCounts, ListPage, ListRows, MessageSummary, PageRequest, StoreError,
};
use postio_model::mailbox::Mailbox;
use postio_model::{Account, Draft, DraftId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ClientKind {
    Tui,
    Focus,
    Ffi,
    Mcp,
    Test,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FocusCounts {
    pub conversations: u32,
    pub unread: u32,
    pub has_action: u32,
    pub filtered_today: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilteredRow {
    pub message: postio_model::listing::MessageSummary,
    pub reason: String,
    pub source: Option<String>,
    pub at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ClientId(pub u64);

/// Each is answered by exactly one `Resp` with the same id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Req {
    /// Aimed with the frontend's own selection; its effects arrive as events.
    Send(Command, StateSnapshot),
    SendTracked(Command, StateSnapshot),
    /// Every conversation `query` matches but the rows `except` stands for (each a conversation's
    /// best message):
    /// a predicate the host walks in order with every other command, so one undo takes the whole
    /// back.
    SendMatching {
        command: Command,
        account: postio_model::AccountScope,
        query: postio_search::ParsedQuery,
        except: Vec<MessageId>,
    },
    Page(PageRequest),
    Count(ListScope),
    Rows(Vec<MessageId>),
    RowsIn(ListScope, Vec<MessageId>),
    /// Said before the list re-reads.
    NoteRemoved(MailboxId, Vec<MessageId>),
    Mailboxes(AccountId),
    DraftCounts(AccountId),
    Accounts,
    Body(MessageId),
    Conversation(postio_model::ThreadId),
    Thread(postio_model::ThreadId),
    Mute(postio_model::ThreadId),
    Unmute(postio_model::ThreadId),
    Split(MessageId),
    MutedOf(Vec<postio_model::ThreadId>),
    /// Reads the local store only.
    ForwardThread {
        thread: postio_model::ThreadId,
        form: postio_model::ForwardForm,
    },
    SetForwardForm {
        draft: Box<Draft>,
        form: postio_model::ForwardForm,
    },
    /// The quote and `In-Reply-To` move; the typed text stays.
    ReplyToLatest {
        draft: Box<Draft>,
    },
    /// Only on a person's deliberate act.
    Unsubscribe(MessageId),
    ListMail(MessageId),
    Parts(MessageId),
    /// Fetches the part first if it was never downloaded.
    SavePart {
        message: MessageId,
        attachment: postio_model::ids::AttachmentId,
        to: std::path::PathBuf,
    },
    /// Writes to a private temporary file and answers where.
    OpenPart {
        message: MessageId,
        attachment: postio_model::ids::AttachmentId,
    },
    /// `offline` is the frontend's: it decides whether a body not here yet is "downloading" or
    /// "offline".
    Readings {
        messages: Vec<MessageId>,
        offline: bool,
    },
    ThreadReadings {
        thread: postio_model::ThreadId,
        limit: u32,
        offline: bool,
    },
    /// Never fetched: a remote read here would be the tracking pixel the reader blocks.
    InlinePart {
        message: MessageId,
        content_id: String,
    },
    /// One batch, one answer: how many could not be written.
    SaveParts {
        message: MessageId,
        parts: Vec<(postio_model::ids::AttachmentId, std::path::PathBuf)>,
    },
    /// Never fetched; a preview with no body keeps the excerpt.
    StoredBody(MessageId),
    /// The first path that cannot be written ends the export: a drop of fewer files than were
    /// dragged must say so.
    ExportMessages(Vec<(MessageId, std::path::PathBuf)>),
    /// Answered in order with every other draft write from this client (see `DraftWriter`).
    SaveDraft {
        /// A second save before the first's id came back still updates the same row.
        generation: u64,
        draft: Box<Draft>,
    },
    QueueSend {
        generation: u64,
        draft: Box<Draft>,
        at: Option<DateTime<Utc>>,
    },
    DiscardDraft {
        generation: u64,
        known: Option<DraftId>,
    },
    Recipients {
        account: AccountId,
        prefix: String,
    },
    Correspondents(AccountId),
    /// Made now from the stored mail when the catch-up has not reached them.
    ContactForAddress(String),
    ContactGlance(String),
    /// Rows past `after` (a sort key and a person), then `skip` more, at most `limit`.
    ContactsPage {
        tab: postio_model::ContactTab,
        after: Option<(String, postio_model::ContactId)>,
        skip: u32,
        limit: u32,
    },
    ContactsFiltered {
        tab: postio_model::ContactTab,
        text: String,
        cap: u32,
    },
    ContactsOverview,
    ContactCard(postio_model::ContactId),
    ContactGroups,
    /// A dry run: writes nothing anywhere.
    ContactsSyncPlan {
        records: Vec<postio_contact_sync::ExternalRecord>,
        scope: postio_contact_sync::Scope,
        mode: postio_contact_sync::Mode,
    },
    /// Writes Postio's own database; their side is never written.
    ContactsSyncBringIn {
        records: Vec<postio_contact_sync::ExternalRecord>,
        scope: postio_contact_sync::Scope,
        mode: postio_contact_sync::Mode,
        shown: u64,
    },
    ContactsImport(Vec<u8>),
    ContactsExport,
    ContactSeen(postio_model::ContactId),
    ContactSuggestions,
    ContactSuggestionSet(postio_model::Suggestion, bool),
    ContactReview(Vec<postio_model::ContactId>),
    ContactPosition(postio_model::ContactId, postio_model::ContactTab),
    ContactMergeOf(Vec<postio_model::ContactId>),
    /// Read once so each keystroke is answered from memory, not a query.
    RecipientDirectory(AccountId),
    Labels(AccountId),
    LabelCounts(AccountId),
    /// Answers the existing label of that name, in any case.
    CreateLabel {
        account: AccountId,
        name: String,
    },
    ThreadLabels(Vec<postio_model::ThreadId>),
    FocusCounts,
    UndoTop,
    ReplySource(MessageId),
    DraftBehind(MessageId),
    DraftById(DraftId),
    CancelSend(DraftId),
    SendFailure(DraftId),
    DefaultSignature {
        account: AccountId,
        selected: Option<MailboxId>,
    },
    Attach {
        path: std::path::PathBuf,
        /// Set when the frontend can sniff a type better than the host's fallback (the desktop asks
        /// shared-mime-info).
        mime_type: Option<String>,
    },
    AttachmentBytes(postio_model::ids::BlobId),
    /// Answers the draft `account` was still writing when the last session died unclean (#491);
    /// nothing after a clean exit.
    /// The ask itself marks the session open, so ask once per process.
    RecoverDraft(AccountId),
    Search(Search),
    SearchHits {
        account: postio_model::AccountScope,
        query: postio_search::ParsedQuery,
        scope: postio_search::facets::Scope,
        order: postio_search::ResultOrder,
        snippets: u32,
    },
    /// A read of its own, after the hits, so the number being watched never waits for these.
    Facets {
        account: postio_model::AccountScope,
        query: postio_search::ParsedQuery,
        scope: postio_search::facets::Scope,
    },
    /// Cancellable: dropping the call stops it.
    Conversations {
        account: postio_model::AccountScope,
        query: postio_search::ParsedQuery,
        order: postio_search::results::ConversationOrder,
        offset: u32,
        limit: u32,
    },
    /// A read of its own, after the conversations.
    Passages {
        query: postio_search::ParsedQuery,
        hits: Vec<(MessageId, Vec<postio_search::results::Source>)>,
        first_line: postio_search::passage::FirstLine,
    },
    Files {
        account: postio_model::AccountScope,
        query: postio_search::ParsedQuery,
        offset: u32,
        limit: u32,
    },
    People {
        account: postio_model::AccountScope,
        query: postio_search::ParsedQuery,
        offset: u32,
        limit: u32,
    },
    /// Never fetched: an attachment whose bytes are not here answers nothing.
    AttachmentCopy {
        attachment: postio_model::ids::AttachmentId,
        dir: std::path::PathBuf,
    },
    ConversationMatches {
        query: postio_search::ParsedQuery,
        key: postio_search::results::ConversationKey,
    },
    Relaxations {
        account: postio_model::AccountScope,
        query: postio_search::ParsedQuery,
        today: chrono::NaiveDate,
    },
    Suggest {
        account: postio_model::AccountScope,
        prefix: String,
        field: Option<postio_search::query::Field>,
    },
    RecentSearches,
    RememberSearch {
        query: String,
        hits: u64,
        /// `hits` is a floor when the walk's cap stopped the search.
        capped: bool,
    },
    ForgetSearch(String),
    SavedCounts {
        account: postio_model::AccountScope,
        today: chrono::NaiveDate,
        /// A search whose key is not listed is no longer saved, and its seen-progress is dropped.
        searches: Vec<(String, String)>,
    },
    MarkSeen(String),
    Account(AccountOp),
    Discover {
        address: String,
        stop: Stop,
    },
    /// Answered with the consent URL, which nothing opens: the frontend shows it and opens it only
    /// when asked.
    BeginOAuth {
        submission: Box<postio_ui::onboarding::Submission>,
        then: AfterSave,
    },
    FinishOAuth(String),
    CancelOAuth(String),
    /// The password goes to the keyring and nowhere else.
    AddAccount {
        submission: Box<postio_ui::onboarding::Submission>,
        then: AfterSave,
    },
    AccountSettings {
        /// `weights` scans every message an account holds: ask only while the panel is on screen.
        weights: bool,
    },
    EditAccount(AccountId, AccountField),
    /// An edit keeps the rich variant this form does not show.
    SaveSignature {
        account: AccountId,
        signature: Option<postio_model::SignatureId>,
        name: String,
        text: String,
    },
    DeleteSignature(postio_model::SignatureId),
    /// Answers when the rebuild is over; `AccountOp::RebuildIndex` starts the same rebuild and
    /// answers at once.
    RebuildIndex(AccountId),
    EgressLog(u32),
    PrivacyLog,
    SetBackfillExcluded {
        mailbox: MailboxId,
        excluded: bool,
    },
    OrientationSeen,
    RetireOrientation,
    MoveRecent,
    FilteredTabs,
    DeliveryMessages(postio_model::DeliveryId),
    DigestWaiting(Vec<String>),
    Held(Vec<MessageId>),
    AccountOf(MessageId),
    Filtered {
        reason: Option<String>,
        offset: u32,
        limit: u32,
    },
    NoteMove(MailboxId),
    /// Posted; the body arrives as `BodyLoaded`.
    FetchBody(MessageId),
    /// Sent only for a forward: a reply carries nothing, and nothing leaves the machine the person
    /// did not ask for (#1690).
    FetchCarried {
        message: MessageId,
        parts: Vec<String>,
    },
    StorageCeiling(Option<u64>),
    /// The password goes to the keyring first, then the row.
    SaveAccount {
        submission: Box<postio_ui::onboarding::Submission>,
        backend: postio_model::account::Backend,
    },
    /// The one network request a composer makes, on its press alone. Answered as the attachment, or
    /// why it stays a link.
    AttachRemoteImage {
        draft: postio_model::DraftId,
        href: String,
    },
    InlineImage {
        bytes: Vec<u8>,
        mime_type: String,
    },
    SweepPreview,
    /// Fetched from the server on this request when never downloaded.
    RawSource(MessageId),
    SourceView {
        message: MessageId,
        offline: bool,
    },
    GnupgKeyring {
        on: bool,
    },
    ImportKeys(ImportFrom),
    Surfaced,
    /// Fails with a sentence when no `[focus.vault]` is configured.
    Vault {
        subject: String,
    },
    CaptureTask {
        project: Option<postio_vault::Project>,
        task: postio_vault::Task,
    },
    CaptureNote {
        note: std::path::PathBuf,
        entry: postio_vault::NoteEntry,
    },
    DigestSummary(postio_model::DeliveryId),
    /// `None` when no model with `like_this` on is configured, or the model picked none.
    DigestLikeThis(MessageId),
    DigestPreview {
        queries: Vec<String>,
        since: DateTime<Utc>,
    },
    SaveDigestRule {
        replacing: Option<String>,
        rule: DigestRuleDraft,
    },
    DeleteDigestRule(String),
    TakeBackFilter {
        entry: String,
        kind: Option<String>,
    },
    Treatments,
    /// The treatment is the person's, so it covers every address they own.
    SetTreatment {
        address: String,
        treatment: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DigestRuleDraft {
    pub name: String,
    /// The rule holds a message when any query matches. The dialog writes `from:<address>`.
    pub queries: Vec<String>,
    pub cadence: postio_model::listing::Cadence,
    /// A weekday for weekly, a day of the month for monthly, none for daily.
    pub day: Option<RuleDay>,
    /// On this machine's clock.
    pub at: chrono::NaiveTime,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleDay {
    Weekday(chrono::Weekday),
    /// 1 to 28.
    OfMonth(u32),
}

impl DigestRuleDraft {
    /// The one place a schedule becomes a cadence, a day and a time. `Err` is a sentence for the
    /// person.
    pub fn from_form(
        form: &postio_ui::rule_form::RuleForm,
        schedule: &postio_ui::digest::Schedule,
    ) -> Result<Self, String> {
        use postio_config::Due;
        use postio_model::listing::Cadence;
        let (name, queries, due) = form.written(schedule)?;
        let (cadence, day, at) = match due {
            Due::Daily { at } => (Cadence::Daily, None, at),
            Due::Weekly { day, at } => (Cadence::Weekly, Some(RuleDay::Weekday(day)), at),
            Due::Monthly { day, at } => (Cadence::Monthly, Some(RuleDay::OfMonth(day)), at),
        };
        Ok(DigestRuleDraft {
            name,
            queries,
            cadence,
            day,
            at,
        })
    }
}

/// Queries in the one language, never text a model wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LikeThisRule {
    pub queries: Vec<String>,
    pub preview: DigestPreview,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DigestPreview {
    pub count: u32,
    pub first: Vec<MessageSummary>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Queued {
    pub drafts: Option<MailboxId>,
    pub draft: DraftId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resp {
    Done,
    Tracked(InvocationId),
    /// The runtime has stopped and took no command.
    Stopped,
    Page(ListPage),
    Count(u32),
    Rows(Vec<MessageSummary>),
    Thread(Option<Box<postio_model::ThreadView>>),
    Split(postio_model::ThreadId),
    Forwarded(postio_model::Forwarded),
    Muted(Vec<(postio_model::ThreadId, u32)>),
    ListRows(ListRows),
    Mailboxes(Vec<Mailbox>),
    DraftCounts(DraftCounts),
    Accounts(Vec<Account>),
    Body(Body),
    Unsubscribed(String),
    ListMail(ListMailFound),
    Parts(Vec<postio_model::Attachment>),
    Saved(std::path::PathBuf),
    Readings(Vec<Reading>),
    InlinePart(Option<(Vec<u8>, String)>),
    SavedParts(u32),
    Carried(postio_model::fetch::PayloadAsk),
    DraftSaved(DraftId),
    Queued(Queued),
    Recipients(Vec<RecipientCandidate>),
    Correspondents(Vec<postio_model::Contact>),
    ContactFor(Option<Box<postio_model::Contact>>),
    ContactGlance(Option<Box<postio_model::ContactGlance>>),
    ContactsPage(postio_model::ContactsPageRead),
    ContactsFiltered(Vec<postio_model::ContactListRow>),
    ContactsOverview(postio_model::ContactsOverview),
    ContactCard(Option<Box<postio_model::ContactCardRead>>),
    ContactGroups(Vec<String>),
    ContactsSyncPlan(Box<postio_contact_sync::Plan>),
    ContactsBroughtIn(Box<postio_contact_sync::BroughtIn>),
    ContactsImported(Box<postio_model::card::ImportSummary>),
    /// The file's text and how many people it holds.
    ContactsExported(String, usize),
    ContactSeen(Vec<postio_model::SeenItem>),
    ContactSuggestions(postio_model::ContactSuggestions),
    ContactPosition(Option<u32>),
    ContactReview(Vec<postio_model::MergeGroup>),
    ContactMergeOf(Option<Box<postio_model::MergeGroup>>),
    Treatments(Vec<(String, String)>),
    RecipientDirectory(RecipientDirectory),
    Labels(Vec<postio_model::Label>),
    MoveRecent(Vec<MailboxId>),
    FilteredTabs(Vec<(String, u32)>),
    Counts(Vec<u32>),
    Held(Vec<(MessageId, String, bool)>),
    AccountOf(Option<AccountId>),
    Filtered(Vec<FilteredRow>),
    /// A label nothing carries is left out.
    LabelCounts(Vec<(postio_model::LabelId, u32)>),
    Label(Option<postio_model::Label>),
    ThreadLabels(Vec<(postio_model::ThreadId, postio_model::Label)>),
    FocusCounts(FocusCounts),
    UndoTop(Option<String>),
    ReplySource(Option<Box<(postio_model::Message, Account)>>),
    Draft(Option<Box<Draft>>),
    SendFailure(Option<String>),
    Signature(Option<postio_model::SignatureId>),
    Attached(Option<postio_model::Attachment>),
    RemoteImage(Result<postio_model::Attachment, postio_ui::compose::RemoteRefusal>),
    Bytes(Option<Vec<u8>>),
    Found(Option<Found>),
    Hits(Option<Box<Hits>>),
    Facets(Option<postio_search::facets::Facets>),
    Conversations(Option<Box<Conversations>>),
    Passages(Vec<(MessageId, Vec<postio_search::results::Match>)>),
    /// Empty when the conversation could not be read.
    Matches(Vec<postio_search::results::ConversationMatch>),
    /// Empty when the store could not be read.
    Files(Vec<postio_search::results::FileHit>),
    /// Empty when the store could not be read.
    People(Vec<postio_search::suggest::Person>),
    Path(Option<std::path::PathBuf>),
    /// Empty when none would find anything, or they could not be counted.
    Relaxed(
        Vec<(
            postio_search::relax::Relaxation,
            postio_search::results::Counted,
        )>,
    ),
    /// Empty when the store could not be read.
    Suggestions(Box<postio_search::suggest::Suggestions>),
    Recent(Vec<RecentSearch>),
    SavedCounts(Vec<SavedCount>),
    /// Empty when there are none here.
    StoredBody(postio_model::MessageBody),
    Exported(Vec<std::path::PathBuf>),
    AccountSettings(Vec<AccountSettings>),
    Egress(Vec<postio_model::egress::EgressEvent>),
    Privacy(PrivacyLog),
    Seen(bool),
    Onboarding(Box<postio_ui::onboarding::Status>),
    Consent(Box<postio_ui::onboarding::BrowserSignIn>),
    Surfaced(Vec<postio_model::listing::Surfaced>),
    DigestPreview(DigestPreview),
    DigestLikeThis(Option<LikeThisRule>),
    DigestSummary(Option<postio_model::summary::DigestSummary>),
    RawSource(Vec<u8>),
    SourceView(Box<postio_ui::source::SourceView>),
    GnupgKeyring(postio_ui::settings::GnupgKeyring),
    Imported(postio_ui::source::Imported),
    Vault(VaultPicture),
    Captured(postio_vault::Captured),
    /// The sentence is for the user.
    Failed(StoreError),
}

/// The frontend applies the reader's rules (sanitising, reader view, folding) with the code every
/// reader shares.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Body {
    Ready {
        body: postio_model::MessageBody,
        encoding_problems: bool,
    },
    Partial,
    Offline,
    Missing,
    Empty,
    ForeignDraft,
    /// The server would not give up the body when last asked (#1918); opening the message asks
    /// again.
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reading {
    pub message: MessageId,
    pub body: Body,
    pub row: Option<Box<postio_model::Message>>,
    pub send_state: Option<postio_model::DraftState>,
    pub authenticity: postio_ui::reader::authenticity::HeaderAuthenticity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountOp {
    SetEnabled {
        account: AccountId,
        enabled: bool,
    },
    /// Marked for deletion; undone by `AccountOp::Restore` until the deletion is carried out.
    Remove(AccountId),
    Restore(AccountId),
    SetDefault(AccountId),
    RebuildIndex(AccountId),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccountField {
    DisplayName(String),
    ImapHost(String),
    ImapPort(u16),
    SmtpHost(String),
    SmtpPort(u16),
    DefaultSignature(Option<postio_model::SignatureId>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountSettings {
    pub account: Account,
    pub folders: Vec<String>,
    pub chosen: Vec<(postio_model::MailboxRole, String)>,
    pub resolved: Vec<(postio_model::MailboxRole, String)>,
    pub refused: Vec<(postio_model::MailboxRole, String)>,
    pub weight: Option<postio_core::event::MailFootprint>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PrivacyLog {
    pub activations: Vec<postio_model::UnsubscribeActivation>,
    pub read_receipts: u64,
}

/// A desktop app starts the sync itself (after the first-run window choice, in a running window, or
/// for a credential update);
/// `AfterSave::Wait` keeps out a second engine for one account, or one under the wrong sync window.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum AfterSave {
    #[default]
    Sync,
    /// Save it and nothing more: the frontend starts what it needs.
    Wait,
}

/// A discovery probe opens connections to servers the person has not named yet, so one they moved
/// on from must stop at once (#57, ADR 0012 Q3).
/// Shared, not sent: the `on_stop` hooks run inside `stop`, on the frontend's thread, before it
/// returns.
#[derive(Clone, Default)]
pub struct Stop(std::sync::Arc<std::sync::Mutex<Stopping>>);

#[derive(Default)]
struct Stopping {
    stopped: bool,
    hooks: Vec<Box<dyn FnOnce() + Send>>,
}

impl Stop {
    pub fn new() -> Stop {
        Stop::default()
    }

    /// Every hook runs before this returns; pulling it again does nothing.
    pub fn stop(&self) {
        let hooks = {
            let mut stopping = self
                .0
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            stopping.stopped = true;
            std::mem::take(&mut stopping.hooks)
        };
        for hook in hooks {
            hook();
        }
    }

    pub fn is_stopped(&self) -> bool {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .stopped
    }

    pub fn on_stop(&self, hook: impl FnOnce() + Send + 'static) {
        let mut stopping = self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if stopping.stopped {
            drop(stopping);
            hook();
        } else {
            stopping.hooks.push(Box::new(hook));
        }
    }
}

impl std::fmt::Debug for Stop {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Stop").field(&self.is_stopped()).finish()
    }
}

impl PartialEq for Stop {
    fn eq(&self, other: &Stop) -> bool {
        std::sync::Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for Stop {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Search {
    pub account: postio_model::AccountScope,
    pub query: String,
    pub newest_first: bool,
    pub scope: postio_search::facets::Scope,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub ids: Vec<MessageId>,
    pub hits: u64,
    pub capped: bool,
    pub corpus_complete: bool,
    pub elapsed: std::time::Duration,
}

/// A wrapper only for `Resp`'s `Eq`: a hit's rank is an `f64`, never NaN (the executor folds `bm25`
/// with finite weights).
#[derive(Debug, Clone, PartialEq)]
pub struct Hits(pub postio_search::SearchResults);

impl Eq for Hits {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecentSearch {
    pub query: String,
    pub last_run_at: chrono::DateTime<chrono::Utc>,
    pub hits: u64,
    pub capped: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedCount {
    pub key: String,
    pub total: postio_search::results::Counted,
    /// Zero before it was first viewed.
    pub new: postio_search::results::Counted,
}

/// A wrapper only for [`Resp`]'s `Eq`, as [`Hits`] is: a hit's score is an
/// `f64`, never NaN.
#[derive(Debug, Clone, PartialEq)]
pub struct Conversations(pub postio_search::results::ConversationResults);

impl Eq for Conversations {}

/// A group with no members is left out, since it would expand to nothing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RecipientDirectory {
    pub groups: Vec<(String, Vec<postio_model::EmailAddress>)>,
    pub contacts: Vec<postio_ui::recipients::Correspondent>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct VaultPicture {
    pub projects: Vec<postio_vault::Project>,
    pub suggestion: Option<postio_vault::Suggestion>,
    pub tasks_note: std::path::PathBuf,
    pub tasks: Vec<postio_vault::CapturedTask>,
}

/// Flagged mail and mail already in Trash, Junk, Sent or Drafts is not in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListMailFound {
    pub name: String,
    pub ids: Vec<MessageId>,
    /// More matched than the page held, so `ids.len()` is a floor.
    pub more: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use postio_model::EmailAddress;
    use postio_ui::digest::Schedule;
    use postio_ui::rule_form::RuleForm;

    fn form() -> RuleForm {
        let sender = EmailAddress {
            address: "news@example.com".to_owned(),
            name: None,
        };
        RuleForm::new_rule(&[sender], None)
    }

    #[test]
    fn a_daily_rule_takes_no_day() {
        let mut schedule = Schedule::new_rule();
        schedule.cadence = 0;
        schedule.at = "07:30".to_owned();
        let draft = DigestRuleDraft::from_form(&form(), &schedule).expect("a draft");
        assert_eq!(draft.cadence, postio_model::listing::Cadence::Daily);
        assert_eq!(draft.day, None);
        assert_eq!(draft.at.format("%H:%M").to_string(), "07:30");
        assert_eq!(draft.queries, ["from:news@example.com"]);
    }

    #[test]
    fn a_weekly_rule_names_its_weekday_and_a_monthly_one_its_date() {
        let mut schedule = Schedule::new_rule();
        schedule.cadence = 1;
        schedule.weekday = 2;
        let weekly = DigestRuleDraft::from_form(&form(), &schedule).expect("a draft");
        assert_eq!(weekly.day, Some(RuleDay::Weekday(chrono::Weekday::Wed)));
        schedule.cadence = 2;
        schedule.month_day = 14;
        let monthly = DigestRuleDraft::from_form(&form(), &schedule).expect("a draft");
        assert_eq!(monthly.day, Some(RuleDay::OfMonth(15)));
    }

    #[test]
    fn a_time_that_does_not_read_is_said_not_written() {
        let mut schedule = Schedule::new_rule();
        schedule.at = "soon".to_owned();
        assert_eq!(
            DigestRuleDraft::from_form(&form(), &schedule),
            Err(postio_ui::digest::TIME_ERROR.to_owned())
        );
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportFrom {
    File(std::path::PathBuf),
    Message(MessageId),
}
