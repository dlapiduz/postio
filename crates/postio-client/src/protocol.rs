//! What a frontend asks of the store's host, and what it answers.
//!
//! `specs/005-tui-frontend/contracts/protocol.md` is the contract. The host
//! is in the frontend's own process (ADR 0041), so a request is a value
//! handed over a channel and never encoded: these are the vocabulary of
//! [`crate::Client`] and `postio-host`, nothing more.
//!
//! **Nothing here waits on the network.** Every [`Req`] is answered from the
//! host's local state; anything remote is a [`Command`], whose outcome arrives
//! later as events (Principle I).

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

/// What kind of frontend is connecting: labels the connection in the host's
/// log.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ClientKind {
    /// The GTK desktop app.
    Gtk,
    /// `postio-tui`.
    Tui,
    /// Postio Focus, the other desktop app (spec 007).
    Focus,
    /// The macOS frontend, through `postio-ffi`.
    Ffi,
    /// A test.
    Test,
}

/// What Focus's header strip counts (spec 007 FR-018; data-model
/// `FocusCounts`): the rows of its inbox, how many are unread, and how many
/// draw a marker -- the has-action toggle's number. The filtered-today and
/// digest-rule counts join them with their features.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FocusCounts {
    /// The conversations in Focus's inbox.
    pub conversations: u32,
    /// Of those, the ones with unread mail.
    pub unread: u32,
    /// Of those, the ones that draw a marker.
    pub has_action: u32,
    /// Messages filed away since local midnight: "186 filtered today".
    pub filtered_today: u32,
}

/// One row of Focus's Filtered view (screen 21): the message, why it was
/// filtered and by whom, and when.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilteredRow {
    /// The message, as a list row.
    pub message: postio_model::listing::MessageSummary,
    /// Why, as the store spells it: "notification".
    pub reason: String,
    /// Who it came from, shown after the reason: "Forge".
    pub source: Option<String>,
    /// When it was filed away.
    pub at: chrono::DateTime<chrono::Utc>,
}

/// The host's name for one connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ClientId(pub u64);

/// A request a frontend makes of the host. Each is answered by exactly one
/// [`Resp`] with the same id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Req {
    /// Run a command, aimed with the frontend's own selection; its effects
    /// arrive as events.
    Send(Command, StateSnapshot),
    /// The same, learning the id its events will carry.
    SendTracked(Command, StateSnapshot),
    /// One page of a list.
    Page(PageRequest),
    /// How many rows a list would show.
    Count(ListScope),
    /// The rows for these messages, in the order given.
    Rows(Vec<MessageId>),
    /// The rows a scope's list shows for these messages.
    RowsIn(ListScope, Vec<MessageId>),
    /// These messages left a mailbox; said before the list re-reads.
    NoteRemoved(MailboxId, Vec<MessageId>),
    /// An account's folders.
    Mailboxes(AccountId),
    /// What the sidebar draws beside Drafts and the Outbox.
    DraftCounts(AccountId),
    /// Every account, in the sidebar's order.
    Accounts,
    /// A message's body, or why there is none yet.
    Body(MessageId),
    /// A conversation's messages, oldest first, as list rows.
    Conversation(postio_model::ThreadId),
    /// Leave the list this message came from: record the activation, and
    /// answer the list's name. Only ever on a person's deliberate act.
    Unsubscribe(MessageId),
    /// A message's parts: its attachments, and inline parts.
    Parts(MessageId),
    /// Write one part to `to`, fetching it first if it was never downloaded.
    SavePart {
        /// Whose.
        message: MessageId,
        /// Which part.
        attachment: postio_model::ids::AttachmentId,
        /// Where; the frontend chose it.
        to: std::path::PathBuf,
    },
    /// Write one part to a private temporary file for the system to open,
    /// and answer where.
    OpenPart {
        /// Whose.
        message: MessageId,
        /// Which part.
        attachment: postio_model::ids::AttachmentId,
    },
    /// Everything a reading pane draws of each of these messages -- body,
    /// row and send state -- read on one turn of the store, in the order
    /// asked. `offline` is the frontend's: it decides whether a body not
    /// here yet is "downloading" or "offline".
    Readings {
        /// Which messages.
        messages: Vec<MessageId>,
        /// Whether the frontend has no connection at all right now.
        offline: bool,
    },
    /// [`Req::Readings`] for a conversation's first `limit` members, oldest
    /// first: what a frontend prepares before the conversation is opened.
    ThreadReadings {
        /// Which conversation.
        thread: postio_model::ThreadId,
        /// The most members read.
        limit: u32,
        /// As [`Req::Readings`].
        offline: bool,
    },
    /// One inline part of `message`, by its `Content-ID`, when its bytes are
    /// on this machine. Never fetched: a remote read here would be the
    /// tracking pixel the reader blocks.
    InlinePart {
        /// Whose; a `Content-ID` means nothing outside its own message.
        message: MessageId,
        /// The `cid:` it was asked for by.
        content_id: String,
    },
    /// Write each part to its path, fetching what was never downloaded, and
    /// answer how many could not be written. One batch, one answer: `S`.
    SaveParts {
        /// Whose.
        message: MessageId,
        /// Which parts, and where each goes; the frontend chose them.
        parts: Vec<(postio_model::ids::AttachmentId, std::path::PathBuf)>,
    },
    /// A message's stored words, or none: what a search preview draws under
    /// the excerpt it already has. Never fetched, and no reason given -- a
    /// preview with no body keeps the excerpt.
    StoredBody(MessageId),
    /// Write each message's raw source to its path, fetching what was never
    /// downloaded, and answer the paths in the order asked. The first that
    /// cannot be written ends the export: a drop of fewer files than were
    /// dragged must say so.
    ExportMessages(Vec<(MessageId, std::path::PathBuf)>),
    /// Autosave a draft as composition `generation`. Answered in order with
    /// every other draft write from this client (see `DraftWriter`).
    SaveDraft {
        /// Which composition: a second save of one before the first's id
        /// came back still updates the same row.
        generation: u64,
        /// The draft as it stands.
        draft: Box<Draft>,
    },
    /// Queue a draft to send, now or at `at`.
    QueueSend {
        /// Which composition.
        generation: u64,
        /// The draft as it stands.
        draft: Box<Draft>,
        /// When, for a scheduled send.
        at: Option<DateTime<Utc>>,
    },
    /// A composition closed with nothing worth keeping: its row goes.
    DiscardDraft {
        /// Which composition.
        generation: u64,
        /// Its id, when the frontend knows it and the host may not.
        known: Option<DraftId>,
    },
    /// Recipient completion for what has been typed so far.
    Recipients {
        /// Whose contacts.
        account: AccountId,
        /// What has been typed.
        prefix: String,
    },
    /// The account's correspondents, for the finder's `@`.
    Correspondents(AccountId),
    /// Everything recipient completion can offer for an account, read once
    /// so each keystroke is answered from memory rather than by a query.
    RecipientDirectory(AccountId),
    /// The account's labels, for the finder's `+` and the label picker.
    Labels(AccountId),
    /// How many conversations carry each of the account's labels, for
    /// Focus's label picker (spec 007 US5, screen 13).
    LabelCounts(AccountId),
    /// Make a label the account does not have yet, or answer the one it
    /// has by that name in any case: the label picker's "Create label".
    CreateLabel {
        /// Whose label.
        account: AccountId,
        /// Its name, as typed.
        name: String,
    },
    /// The labels on each of these conversations, for a page of Focus's
    /// list: its label pills (spec 007 T043).
    ThreadLabels(Vec<postio_model::ThreadId>),
    /// Focus's header strip: its conversations, unread and has-action
    /// counts (spec 007 FR-018, T048).
    FocusCounts,
    /// The message a reply or forward is built from, and its account.
    ReplySource(MessageId),
    /// The local draft behind a Drafts row, if there is one.
    DraftBehind(MessageId),
    /// Take a queued draft back to edit it, if it has not started sending.
    CancelSend(DraftId),
    /// Why a draft's last send attempt gave up.
    SendFailure(DraftId),
    /// The signature a new draft starts with.
    DefaultSignature {
        /// Which account.
        account: AccountId,
        /// The mailbox selected, whose own signature overrides the account's.
        selected: Option<MailboxId>,
    },
    /// Store the file at this path as an attachment.
    Attach {
        /// Where the file is.
        path: std::path::PathBuf,
        /// Its type, when the frontend can sniff one better than the host's
        /// fallback: the desktop asks shared-mime-info, which the terminal
        /// may not have.
        mime_type: Option<String>,
    },
    /// The bytes of a part the composer holds, by the blob they were stored
    /// under: what an inline image in a draft draws.
    AttachmentBytes(postio_model::ids::BlobId),
    /// Record that a frontend's session began, and answer the draft
    /// `account` was still writing when the last session died without
    /// ending cleanly (#491). Nothing after a clean exit, or for a draft
    /// holding nothing worth keeping. Asked once per process: the ask itself
    /// marks the session open.
    RecoverDraft(AccountId),
    /// Search, as the desktop's search bar does.
    Search(Search),
    /// The desktop search's hits, as its surfaces draw them: the parsed
    /// query the box built, excerpts for the first `snippets` hits, the
    /// sender and folder of each, and the suggestion for a query that
    /// found nothing.
    SearchHits {
        /// Which accounts.
        account: postio_model::AccountScope,
        /// The query, as the box parsed it.
        query: postio_search::ParsedQuery,
        /// The standing rescope from the results' left column.
        scope: postio_search::facets::Scope,
        /// Ranked, or newest first.
        order: postio_search::ResultOrder,
        /// How many hits, from the best, get an excerpt.
        snippets: u32,
    },
    /// What the results' columns say about a search: its count in every
    /// scope and the refinements worth offering. A read of its own, after
    /// the hits, so the number being watched never waits for these.
    Facets {
        /// Which accounts: the scope the hits were counted under.
        account: postio_model::AccountScope,
        /// The query, as the box parsed it.
        query: postio_search::ParsedQuery,
        /// The scope on screen.
        scope: postio_search::facets::Scope,
    },
    /// Change an account the way the settings' account commands do.
    Account(AccountOp),
    /// Look up the servers for a new account's address.
    Discover {
        /// The address as typed.
        address: String,
        /// How the frontend stops the probe: the person typed another
        /// address, pressed Connect, or walked away from the form (#57).
        stop: Stop,
    },
    /// Begin a browser sign-in for a new account; answered with the consent
    /// URL, which nothing opens: the frontend shows it, and opens it only
    /// when asked.
    BeginOAuth {
        /// What the form held.
        submission: Box<postio_ui::onboarding::Submission>,
        /// What the host does once the account is saved.
        then: AfterSave,
    },
    /// Wait for the sign-in for this address to finish, and the account to
    /// be saved.
    FinishOAuth(String),
    /// Give up the sign-in for this address.
    CancelOAuth(String),
    /// Prove a new account's credentials and save it (the password goes to
    /// the keyring and nowhere else).
    AddAccount {
        /// What the form held.
        submission: Box<postio_ui::onboarding::Submission>,
        /// What the host does once the account is saved.
        then: AfterSave,
    },
    /// Every account the settings show -- all but those being removed --
    /// each with its folders and role map, and, when `weights`, what its
    /// mail weighs. One read for the whole panel.
    AccountSettings {
        /// Whether to measure each account's mail: a scan of every message
        /// it holds, asked for only while the panel is on screen.
        weights: bool,
    },
    /// Change one field of an account's row.
    EditAccount(AccountId, AccountField),
    /// Write a signature, new or edited; an edit keeps the rich variant this
    /// form does not show. A refusal's sentence is for the person who typed.
    SaveSignature {
        /// Whose.
        account: AccountId,
        /// Which, for an edit; `None` for a new one.
        signature: Option<postio_model::SignatureId>,
        /// Its name.
        name: String,
        /// Its text.
        text: String,
    },
    /// Remove a signature.
    DeleteSignature(postio_model::SignatureId),
    /// Rebuild an account's local search index and answer when it is over;
    /// progress arrives as events meanwhile. [`AccountOp::RebuildIndex`]
    /// starts the same rebuild and answers at once.
    RebuildIndex(AccountId),
    /// The newest outbound connections the egress log holds, at most this
    /// many (#151).
    EgressLog(u32),
    /// The privacy pane's figures: every unsubscribe activation, newest
    /// first, and how many messages asked for a read receipt.
    PrivacyLog,
    /// Skip or resume a folder's background backfill, and answer its
    /// account's folders as they now stand.
    SetBackfillExcluded {
        /// Which folder.
        mailbox: MailboxId,
        /// Whether it is skipped.
        excluded: bool,
    },
    /// Whether some earlier run already showed the keyboard orientation.
    OrientationSeen,
    /// Write down that this installation is done with the orientation.
    RetireOrientation,
    /// Where mail was last moved, newest first: the move picker's Recent
    /// (spec 007 T098).
    MoveRecent,
    /// Each filter reason with how many messages it keeps filtered: the
    /// Filtered view's tabs (spec 007 T124).
    FilteredTabs,
    /// What a digest delivery holds, as list rows, newest first: the
    /// digest window's plain list (spec 007 T137).
    DeliveryMessages(postio_model::DeliveryId),
    /// How many messages each named rule holds now, waiting for its next
    /// delivery: the `g d` list's "holds N" (spec 007 T139).
    DigestWaiting(Vec<String>),
    /// Which of these messages a digest holds, and for which rule: what a
    /// search result says instead of its folder (spec 007 T140).
    Held(Vec<MessageId>),
    /// The account a message is in: whose labels its picker offers (spec
    /// 007 T170).
    AccountOf(MessageId),
    /// A page of the Filtered view, newest first.
    Filtered {
        /// One reason, as the store spells it, or every reason.
        reason: Option<String>,
        /// The first row.
        offset: u32,
        /// How many rows.
        limit: u32,
    },
    /// Put this folder first in the move picker's Recent.
    NoteMove(MailboxId),
    /// Fetch this message's body ahead of the backfill: a person opened it.
    /// Posted; the body arrives as `BodyLoaded`.
    FetchBody(MessageId),
    /// `[storage] max_bytes` changed: bring the blob store under it.
    StorageCeiling(Option<u64>),
    /// Save an account whose credentials a frontend already proved: the
    /// password to the keyring first, then the row, as the desktop's
    /// first-run screen writes them (`postio_session::onboarding::persist`).
    SaveAccount {
        /// What the form held.
        submission: Box<postio_ui::onboarding::Submission>,
        /// Which backend the proof signed in with.
        backend: postio_model::account::Backend,
    },
    /// Save an account a frontend's browser sign-in already proved: its
    /// tokens to the keyring, then the row
    /// (`postio_session::onboarding::persist_oauth`).
    SaveOAuthAccount(Box<OAuthGrant>),
    /// Store pasted image bytes as an inline part.
    InlineImage {
        /// The image.
        bytes: Vec<u8>,
        /// Its type, `image/…`.
        mime_type: String,
    },
    /// How many messages a sweep of the inbox would file away now, by
    /// Focus's filtering rules (spec 007 FR-118): what the sweep says before
    /// it moves anything. Answered as a count.
    SweepPreview,
    /// A message's raw RFC 822 source, as `view_source` shows it: read from
    /// the blob store, or fetched from the server on this request when it
    /// was never downloaded (spec 007 FR-033). Answered as bytes.
    RawSource(MessageId),
    /// The rows Focus's inbox surfaces among its conversations: the digests
    /// delivered and not archived, and the reminders that fired, each with
    /// its time and position (spec 007, contracts/engine.md).
    Surfaced,
    /// What the capture sheet reads from the vault for a message with
    /// `subject` (spec 007 US15): its projects, the one suggested, and the
    /// tasks Postio captured. Fails with a sentence when no `[focus.vault]`
    /// is configured.
    Vault {
        /// The message's subject, which the suggestion is read from.
        subject: String,
    },
    /// Append `task` to `project`'s note, or the tasks note with none.
    CaptureTask {
        /// The project chosen.
        project: Option<postio_vault::Project>,
        /// The task.
        task: postio_vault::Task,
    },
    /// Append `entry` to `note`, relative to the vault.
    CaptureNote {
        /// The note.
        note: std::path::PathBuf,
        /// The entry.
        entry: postio_vault::NoteEntry,
    },
    /// A digest's summary, with every reference resolved again as it is
    /// read: `None` when none is written, or none is left to show (spec 007
    /// FR-172 to FR-175).
    DigestSummary(postio_model::DeliveryId),
    /// "Digest mail like this" for a message (spec 007 FR-171): the rule
    /// the person's model picks from the queries Postio builds for it, with
    /// its preview. Answered `None` when no model with `like_this` on is
    /// configured -- the command is absent -- or the model picked none.
    DigestLikeThis(MessageId),
    /// What a digest rule matching `queries` would have caught since
    /// `since`: the rule dialog's preview, through the executor (spec 007
    /// FR-120, FR-127).
    DigestPreview {
        /// The rule's queries, in the one query language; any matching holds.
        queries: Vec<String>,
        /// How far back: the last 90 days, in the dialog.
        since: DateTime<Utc>,
    },
    /// Write a digest rule to `config.toml`: a new one, or `replacing` the
    /// rule of that name where it stands (spec 007 FR-120).
    SaveDigestRule {
        /// The rule being edited, by its name as it was.
        replacing: Option<String>,
        /// The rule as the dialog says it.
        rule: DigestRuleDraft,
    },
    /// Take a digest rule out of `config.toml` and release what it held into
    /// the inbox (spec 007 FR-126); answered with how many it released.
    DeleteDigestRule(String),
}

/// A digest rule as the rule dialog writes it (spec 007 screen 24): what
/// becomes one `[[focus.digests]]` entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DigestRuleDraft {
    /// What the digest is called; unique among the rules.
    pub name: String,
    /// Queries in the one query language; the rule holds a message when
    /// any of them matches. The dialog writes `from:<address>`.
    pub queries: Vec<String>,
    /// How often it comes.
    pub cadence: postio_model::listing::Cadence,
    /// On which day: a weekday for a weekly digest, a day of the month for a
    /// monthly one, none for a daily one.
    pub day: Option<RuleDay>,
    /// At what time, on this machine's clock.
    pub at: chrono::NaiveTime,
}

/// The day a digest comes on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleDay {
    /// A weekday, for a weekly digest.
    Weekday(chrono::Weekday),
    /// A day of the month, 1 to 28, for a monthly digest.
    OfMonth(u32),
}

/// The rule "Digest mail like this" proposes (spec 007 FR-171): queries in
/// the one language, never text a model wrote, and what they would have
/// caught.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LikeThisRule {
    /// The rule's queries, for `[[focus.digests]] match`.
    pub queries: Vec<String>,
    /// What it would have caught in the last 90 days.
    pub preview: DigestPreview,
}

/// What a digest rule would have caught (spec 007 screen 24): "Would have
/// caught 9 messages in the last 90 days", then the first four.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DigestPreview {
    /// How many messages its queries match in the window.
    pub count: u32,
    /// The newest of them, at most four.
    pub first: Vec<MessageSummary>,
}

/// The host's answer to one [`Req`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resp {
    /// Done; nothing to report.
    Done,
    /// A tracked send was queued; its events carry this id.
    Tracked(InvocationId),
    /// The runtime has stopped and took no command.
    Stopped,
    /// A page.
    Page(ListPage),
    /// A count.
    Count(u32),
    /// Rows.
    Rows(Vec<MessageSummary>),
    /// Rows, in the scope's shape.
    ListRows(ListRows),
    /// Folders.
    Mailboxes(Vec<Mailbox>),
    /// Draft counts.
    DraftCounts(DraftCounts),
    /// The accounts.
    Accounts(Vec<Account>),
    /// A body.
    Body(Body),
    /// The list a message was unsubscribed from.
    Unsubscribed(String),
    /// A message's parts.
    Parts(Vec<postio_model::Attachment>),
    /// Where a part was written.
    Saved(std::path::PathBuf),
    /// What a reading pane draws of each message asked for.
    Readings(Vec<Reading>),
    /// An inline part's bytes and type, or nothing when they are not here.
    InlinePart(Option<(Vec<u8>, String)>),
    /// How many parts of a batch could not be saved.
    SavedParts(u32),
    /// The id a saved draft has.
    DraftSaved(DraftId),
    /// A draft was queued; the Drafts folder whose list moved, if one did.
    Queued(Option<MailboxId>),
    /// Recipient suggestions, best first.
    Recipients(Vec<RecipientCandidate>),
    /// Correspondents, most often seen first.
    Correspondents(Vec<postio_model::Contact>),
    /// What recipient completion can offer.
    RecipientDirectory(RecipientDirectory),
    /// Labels, by name.
    Labels(Vec<postio_model::Label>),
    /// Folders, newest first: the move picker's Recent.
    MoveRecent(Vec<MailboxId>),
    /// Each filter reason with its count, in the tabs' order.
    FilteredTabs(Vec<(String, u32)>),
    /// Counts, in the order asked for.
    Counts(Vec<u32>),
    /// Each held message, its rule, and whether its digest was delivered.
    Held(Vec<(MessageId, String, bool)>),
    /// An account, or none for a message not in the store.
    AccountOf(Option<AccountId>),
    /// A page of filtered mail.
    Filtered(Vec<FilteredRow>),
    /// Each label with how many conversations carry it; a label nothing
    /// carries is left out.
    LabelCounts(Vec<(postio_model::LabelId, u32)>),
    /// One label, or none when it could not be made.
    Label(Option<postio_model::Label>),
    /// Each conversation's labels, in the order they were made.
    ThreadLabels(Vec<(postio_model::ThreadId, postio_model::Label)>),
    /// Focus's counts.
    FocusCounts(FocusCounts),
    /// A reply's source message and its account.
    ReplySource(Option<Box<(postio_model::Message, Account)>>),
    /// A draft, or none.
    Draft(Option<Box<Draft>>),
    /// Why a send failed, if it did.
    SendFailure(Option<String>),
    /// A signature, or none.
    Signature(Option<postio_model::SignatureId>),
    /// A stored attachment, or none when the file could not be read.
    Attached(Option<postio_model::Attachment>),
    /// Stored bytes, or none when they are not here.
    Bytes(Option<Vec<u8>>),
    /// What a search found, or nothing when the store could not be read.
    Found(Option<Found>),
    /// The desktop search's hits, or nothing when the store could not be
    /// read.
    Hits(Option<Box<Hits>>),
    /// A search's facet counts, or nothing when they did not run.
    Facets(Option<postio_search::facets::Facets>),
    /// A message's stored words; empty when there are none here.
    StoredBody(postio_model::MessageBody),
    /// Where each exported message was written, in the order asked.
    Exported(Vec<std::path::PathBuf>),
    /// The accounts the settings show, with their folders and weights.
    AccountSettings(Vec<AccountSettings>),
    /// Outbound connections, newest first.
    Egress(Vec<postio_model::egress::EgressEvent>),
    /// The privacy pane's figures.
    Privacy(PrivacyLog),
    /// Whether the orientation was seen before.
    Seen(bool),
    /// What discovery found, as the first-run screen shows it.
    Onboarding(Box<postio_ui::onboarding::Status>),
    /// Where the browser sign-in waits for the person.
    Consent(Box<postio_ui::onboarding::BrowserSignIn>),
    /// The rows Focus's inbox surfaces, newest first.
    Surfaced(Vec<postio_model::listing::Surfaced>),
    /// A digest rule's preview.
    DigestPreview(DigestPreview),
    /// The rule "Digest mail like this" proposes, if any.
    DigestLikeThis(Option<LikeThisRule>),
    /// A digest's summary, if it has one to show.
    DigestSummary(Option<postio_model::summary::DigestSummary>),
    /// A message's raw source, every byte as the server sent it.
    RawSource(Vec<u8>),
    /// What the capture sheet reads from the vault.
    Vault(VaultPicture),
    /// What a capture appended.
    Captured(postio_vault::Captured),
    /// The read could not be answered; the sentence is for the user.
    Failed(StoreError),
}

/// A message's body as the store holds it, or which kind of "no body" this
/// is. The frontend applies the reader's rules to it -- sanitising, reader
/// view, folding -- with the same shared code every reader uses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Body {
    /// The words are here.
    Ready {
        /// The text and HTML parts.
        body: postio_model::MessageBody,
        /// Whether those words are a guess rather than what was sent.
        encoding_problems: bool,
    },
    /// Headers are here; the body has not been fetched yet.
    Partial,
    /// Not fetched, and nothing is fetching: offline.
    Offline,
    /// Recorded, but the bytes are not in the store.
    Missing,
    /// Fetched, and there is no text or HTML part.
    Empty,
    /// A draft written by another client: nothing here to edit.
    ForeignDraft,
}

/// Everything a reading pane draws of one message, read on one turn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reading {
    /// Which message.
    pub message: MessageId,
    /// Its body, or which kind of "no body" this is.
    pub body: Body,
    /// Its row -- the header, the parts, the list it came from -- or `None`
    /// when the message is gone.
    pub row: Option<Box<postio_model::Message>>,
    /// Whether it is a message being sent, and in which state; `None` for
    /// ordinary mail.
    pub send_state: Option<postio_model::DraftState>,
}

/// What the settings' account commands do to an account.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountOp {
    /// Sync it, or stop.
    SetEnabled {
        /// Which.
        account: AccountId,
        /// Whether it should sync.
        enabled: bool,
    },
    /// Remove it: marked for deletion, and undone by [`AccountOp::Restore`]
    /// until the deletion is carried out.
    Remove(AccountId),
    /// Take back a removal.
    Restore(AccountId),
    /// Make it the account new mail is written from.
    SetDefault(AccountId),
    /// Rebuild its local search index; progress arrives as events.
    RebuildIndex(AccountId),
}

/// One field of an account's row, as the settings' detail view edits it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccountField {
    /// The name shown in the sidebar.
    DisplayName(String),
    /// The incoming server's hostname.
    ImapHost(String),
    /// The incoming server's port.
    ImapPort(u16),
    /// The outgoing server's hostname.
    SmtpHost(String),
    /// The outgoing server's port.
    SmtpPort(u16),
    /// Which signature the composer starts on, or none of them.
    DefaultSignature(Option<postio_model::SignatureId>),
}

/// One account as the settings panel shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountSettings {
    /// The row.
    pub account: Account,
    /// Every folder it can open, by server path, in listing order.
    pub folders: Vec<String>,
    /// The roles pointed somewhere by hand, and where.
    pub chosen: Vec<(postio_model::MailboxRole, String)>,
    /// What each role resolves to right now, mapped or not.
    pub resolved: Vec<(postio_model::MailboxRole, String)>,
    /// The roles the server refused a folder for, and what it said.
    pub refused: Vec<(postio_model::MailboxRole, String)>,
    /// What its mail weighs, when asked for and measurable.
    pub weight: Option<postio_core::event::MailFootprint>,
}

/// What the privacy pane draws.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PrivacyLog {
    /// Every account's unsubscribe activations, newest first.
    pub activations: Vec<postio_model::UnsubscribeActivation>,
    /// How many messages, in every account, asked for a read receipt.
    pub read_receipts: u64,
}

/// What the host does with a new account once it is saved.
///
/// The terminal's first run syncs at once. A desktop app starts the sync
/// itself: its first run asks how far back to sync after the account is
/// saved and before any mail is fetched, its add-account dialog brings the
/// new account into a window that is already running, and a credential
/// update is over an account whose sync is running already. A second engine
/// for one account, or one started under the wrong sync window, is what
/// [`AfterSave::Wait`] keeps out.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum AfterSave {
    /// Start the account's sync.
    #[default]
    Sync,
    /// Save it and nothing more: the frontend starts what it needs.
    Wait,
}

/// A frontend's way to stop work it asked the host for, where it stands.
///
/// A discovery probe opens connections to servers the person has not named
/// yet, so one the person has moved on from -- another address typed,
/// Connect pressed, the form closed -- must stop at once rather than hold a
/// socket open for an answer nobody reads (#57, ADR 0012 Q3). The host is in
/// the frontend's process (ADR 0041), so this is shared, not sent: whatever
/// the host hangs on [`Stop::on_stop`] runs inside [`Stop::stop`], on the
/// frontend's own thread, before `stop` returns.
#[derive(Clone, Default)]
pub struct Stop(std::sync::Arc<std::sync::Mutex<Stopping>>);

/// Whether a [`Stop`] has fired, and what runs when it does.
#[derive(Default)]
struct Stopping {
    stopped: bool,
    hooks: Vec<Box<dyn FnOnce() + Send>>,
}

impl Stop {
    /// A stop nobody has pulled.
    pub fn new() -> Stop {
        Stop::default()
    }

    /// Stop the work, now: every hook runs before this returns. Pulling it
    /// again does nothing.
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

    /// Whether it has been pulled.
    pub fn is_stopped(&self) -> bool {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .stopped
    }

    /// Run `hook` when the stop is pulled, or now, if it already has been.
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

/// Two stops are equal when they are the same stop.
impl PartialEq for Stop {
    fn eq(&self, other: &Stop) -> bool {
        std::sync::Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for Stop {}

/// A browser sign-in a frontend completed and proved, to be saved.
///
/// The tokens cross to the store's owner, which writes them to the keyring
/// and nowhere else; `Debug` never shows them.
#[derive(Clone, PartialEq, Eq)]
pub struct OAuthGrant {
    /// What the form held.
    pub submission: postio_ui::onboarding::Submission,
    /// Where consent was asked.
    pub authorize_url: String,
    /// Where tokens are minted.
    pub token_url: String,
    /// The scopes granted.
    pub scopes: Vec<String>,
    /// How long the provider keeps a refresh token alive, when it says.
    pub refresh_token_lifetime_days: Option<u32>,
    /// The access token.
    pub access_token: String,
    /// The refresh token, when one was issued.
    pub refresh_token: Option<String>,
    /// How long the access token lives, from when it was issued.
    pub expires_in: Option<std::time::Duration>,
    /// The token type the server named.
    pub token_type: String,
    /// The scope the server said it granted.
    pub scope: Option<String>,
}

impl std::fmt::Debug for OAuthGrant {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OAuthGrant")
            .field("submission", &self.submission)
            .field("scopes", &self.scopes)
            .finish_non_exhaustive()
    }
}

/// A search, as a frontend's search bar asks it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Search {
    /// Which accounts.
    pub account: postio_model::AccountScope,
    /// The query as typed, in Postio's query language; the host parses it.
    pub query: String,
    /// Newest first rather than best match first.
    pub newest_first: bool,
    /// The standing rescope a facet picks, the query left as typed.
    pub scope: postio_search::facets::Scope,
}

/// What a search found: the matching messages, best first, and what the
/// readout says about them (`postio_ui::search::Outcome`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    /// The messages, in the order the list shows them.
    pub ids: Vec<MessageId>,
    /// How many matched.
    pub hits: u64,
    /// Whether `hits` is a floor rather than the true count.
    pub capped: bool,
    /// Whether every message searched had a body to search.
    pub corpus_complete: bool,
    /// How long it took.
    pub elapsed: std::time::Duration,
}

/// What a desktop search found, as its surfaces draw it.
///
/// A wrapper only for [`Resp`]'s `Eq`: a hit's rank is an `f64`, which has
/// no total equality. It is never NaN -- the executor folds `bm25` with
/// finite weights -- so equality here is reflexive in practice.
#[derive(Debug, Clone, PartialEq)]
pub struct Hits(pub postio_search::SearchResults);

impl Eq for Hits {}

/// What recipient completion can offer an account: its groups, each with
/// its members' addresses, and its contacts in the order the store ranks
/// them. A group with no members is left out, since it would expand to
/// nothing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RecipientDirectory {
    /// Named groups with their members' addresses, in the store's order.
    pub groups: Vec<(String, Vec<postio_model::EmailAddress>)>,
    /// Contacts, best first, each with how often the user wrote to it:
    /// what `postio_ui::recipients::suggest` ranks by (spec 007 T076).
    pub contacts: Vec<postio_ui::recipients::Correspondent>,
}

/// What the capture sheet reads from the vault (spec 007 US15, FR-181).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct VaultPicture {
    /// The vault's projects, by name.
    pub projects: Vec<postio_vault::Project>,
    /// The project suggested for the message, and why.
    pub suggestion: Option<postio_vault::Suggestion>,
    /// The note a task goes to with no project, relative to the vault.
    pub tasks_note: std::path::PathBuf,
    /// Every task Postio captured, in the tasks note and the projects'.
    pub tasks: Vec<postio_vault::CapturedTask>,
}
