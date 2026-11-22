//! Requests are values handed over a channel, never encoded (ADR 0041). Nothing here waits on the
//! network: every `Req` is answered from local state; anything remote is a `Command` whose outcome
//! arrives as events.

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Req {
    Send(Command, StateSnapshot),
    SendTracked(Command, StateSnapshot),
    /// A predicate the host walks with the results' own match, in order with every other command,
    /// so one
    /// undo takes the whole back.
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
    ReplyToLatest {
        draft: Box<Draft>,
    },
    /// Only ever on a person's deliberate act.
    Unsubscribe(MessageId),
    ListMail(MessageId),
    Parts(MessageId),
    SavePart {
        message: MessageId,
        attachment: postio_model::ids::AttachmentId,
        to: std::path::PathBuf,
    },
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
    /// Never fetched, and no reason given: a preview with no body keeps the excerpt.
    StoredBody(MessageId),
    /// The first that cannot be written ends the export: a drop of fewer files than were dragged
    /// must say so.
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
    ContactForAddress(String),
    ContactGlance(String),
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
    /// A dry run, which writes nothing anywhere.
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
    /// Read once, so each keystroke is answered from memory rather than by a query.
    RecipientDirectory(AccountId),
    Labels(AccountId),
    LabelCounts(AccountId),
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
        /// The mailbox selected, whose own signature overrides the account's.
        selected: Option<MailboxId>,
    },
    Attach {
        path: std::path::PathBuf,
        /// Sniffed by the frontend when it can do better than the host's fallback (the desktop asks
        /// shared-mime-info).
        mime_type: Option<String>,
    },
    AttachmentBytes(postio_model::ids::BlobId),
    /// Nothing after a clean exit, or for a draft holding nothing worth keeping. Asked once per
    /// process: the ask itself marks the session open (#491).
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
    /// A read of its own, after the conversations. Cancellable.
    Passages {
        query: postio_search::ParsedQuery,
        hits: Vec<(MessageId, Vec<postio_search::results::Source>)>,
        first_line: postio_search::passage::FirstLine,
    },
    /// Cancellable.
    Files {
        account: postio_model::AccountScope,
        query: postio_search::ParsedQuery,
        offset: u32,
        limit: u32,
    },
    /// Cancellable.
    People {
        account: postio_model::AccountScope,
        query: postio_search::ParsedQuery,
        offset: u32,
        limit: u32,
    },
    /// Never fetched: an attachment whose bytes are not here answers nothing.
    AttachmentCopy {
        attachment: postio_model::ids::AttachmentId,
        /// Where the copy goes; nothing is written outside it.
        dir: std::path::PathBuf,
    },
    /// Cancellable.
    ConversationMatches {
        query: postio_search::ParsedQuery,
        key: postio_search::results::ConversationKey,
    },
    /// Cancellable.
    Relaxations {
        account: postio_model::AccountScope,
        query: postio_search::ParsedQuery,
        today: chrono::NaiveDate,
    },
    /// Cancellable.
    Suggest {
        account: postio_model::AccountScope,
        prefix: String,
        field: Option<postio_search::query::Field>,
    },
    /// Cancellable.
    RecentSearches,
    RememberSearch {
        query: String,
        hits: u64,
        /// The search stopped at its walk's cap: `hits` is a floor.
        capped: bool,
    },
    ForgetSearch(String),
    /// Cancellable.
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
    /// Answered with the consent URL, which nothing opens: the frontend shows it, and opens it only
    /// when asked.
    BeginOAuth {
        submission: Box<postio_ui::onboarding::Submission>,
        then: AfterSave,
    },
    FinishOAuth(String),
    CancelOAuth(String),
    AddAccount {
        submission: Box<postio_ui::onboarding::Submission>,
        then: AfterSave,
    },
    AccountSettings {
        /// A scan of every message the account holds, asked for only while the panel is on screen.
        weights: bool,
    },
    EditAccount(AccountId, AccountField),
    /// An edit keeps the rich variant this form does not show.
    SaveSignature {
        account: AccountId,
        /// Which, for an edit; `None` for a new one.
        signature: Option<postio_model::SignatureId>,
        name: String,
        text: String,
    },
    DeleteSignature(postio_model::SignatureId),
    /// Progress arrives as events meanwhile; `AccountOp::RebuildIndex` starts the same rebuild and
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
    FetchBody(MessageId),
    /// Sent only for a forward: a reply carries nothing, and nothing leaves the machine the person
    /// did not ask for.
    /// Answered with whether they are coming.
    FetchCarried {
        message: MessageId,
        parts: Vec<String>,
    },
    StorageCeiling(Option<u64>),
    /// The password goes to the keyring first, then the row, as the first-run screen writes them.
    SaveAccount {
        submission: Box<postio_ui::onboarding::Submission>,
        backend: postio_model::account::Backend,
    },
    InlineImage {
        bytes: Vec<u8>,
        mime_type: String,
    },
    SweepPreview,
    /// Read from the blob store, or fetched from the server on this request when never downloaded.
    RawSource(MessageId),
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
    /// Every reference is resolved again as it is read: `None` when none is written or none is left
    /// to show.
    DigestSummary(postio_model::DeliveryId),
    /// Answered `None` when no model with `like_this` on is configured, or the model picked none.
    DigestLikeThis(MessageId),
    /// Through the executor, so it means what the same queries mean in search.
    DigestPreview {
        queries: Vec<String>,
        since: DateTime<Utc>,
    },
    SaveDigestRule {
        replacing: Option<String>,
        rule: DigestRuleDraft,
    },
    /// Answered with how many it released.
    DeleteDigestRule(String),
    /// Whichever home holds it -- the file, or the person it names -- is where it goes from.
    TakeBackFilter {
        entry: String,
        /// `question` or `todo` for a marker; `None` for a pin.
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
    /// What the digest is called; unique among the rules.
    pub name: String,
    /// The rule holds a message when any of them matches.
    pub queries: Vec<String>,
    pub cadence: postio_model::listing::Cadence,
    /// A weekday for a weekly digest, a day of the month for a monthly one, none for a daily one.
    pub day: Option<RuleDay>,
    pub at: chrono::NaiveTime,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleDay {
    Weekday(chrono::Weekday),
    OfMonth(u32),
}

impl DigestRuleDraft {
    /// The one place a schedule becomes a cadence, a day and a time; every frontend's rule dialog
    /// ends here.
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
    Bytes(Option<Vec<u8>>),
    Found(Option<Found>),
    Hits(Option<Box<Hits>>),
    Facets(Option<postio_search::facets::Facets>),
    Conversations(Option<Box<Conversations>>),
    Passages(Vec<(MessageId, Vec<postio_search::results::Match>)>),
    Matches(Vec<postio_search::results::ConversationMatch>),
    Files(Vec<postio_search::results::FileHit>),
    People(Vec<postio_search::suggest::Person>),
    Path(Option<std::path::PathBuf>),
    Relaxed(
        Vec<(
            postio_search::relax::Relaxation,
            postio_search::results::Counted,
        )>,
    ),
    Suggestions(Box<postio_search::suggest::Suggestions>),
    Recent(Vec<RecentSearch>),
    SavedCounts(Vec<SavedCount>),
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
    Vault(VaultPicture),
    Captured(postio_vault::Captured),
    Failed(StoreError),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Body {
    /// The words are here.
    Ready {
        /// The text and HTML parts.
        body: postio_model::MessageBody,
        /// Whether those words are a guess rather than what was sent.
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
    /// `None` when the message is gone.
    pub row: Option<Box<postio_model::Message>>,
    /// `None` for ordinary mail.
    pub send_state: Option<postio_model::DraftState>,
    /// The one quiet line when the sender may not be who they say: failed authentication, or a
    /// display
    /// name that names another address (#1928). `None` for ordinary mail.
    pub sender_warning: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountOp {
    SetEnabled {
        account: AccountId,
        enabled: bool,
    },
    /// Marked for deletion, and undone by `Restore` until the deletion is carried out.
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

/// The terminal's first run syncs at once. A desktop app starts the sync itself: a second engine
/// for
/// one account, or one started under the wrong sync window, is what `Wait` keeps out.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum AfterSave {
    #[default]
    Sync,
    Wait,
}

/// A discovery probe opens connections to servers the person has not named yet, so one they moved
/// on
/// from must stop at once rather than hold a socket open (#57, ADR 0012 Q3). Shared, not sent:
/// hooks
/// run inside `stop` on the frontend's own thread before it returns.
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

/// A wrapper only for `Resp`'s `Eq`: a hit's rank is an `f64`, which has no total equality; it is
/// never NaN.
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
    /// Zero before it was first viewed, and a floor past the badge's cap.
    pub new: postio_search::results::Counted,
}

/// A wrapper only for `Resp`'s `Eq`, as `Hits` is: a score is an `f64`, never NaN.
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

/// Flagged mail and mail already in Trash, Junk, Sent or Drafts is not in it (#1931).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListMailFound {
    pub name: String,
    pub ids: Vec<MessageId>,
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
