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
    /// The macOS frontend, through `postio-ffi`.
    Ffi,
    /// A test.
    Test,
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
    Discover(String),
    /// Begin a browser sign-in for a new account; answered with the consent
    /// URL, which nothing opens: the frontend shows it, and opens it only
    /// when asked.
    BeginOAuth(Box<postio_ui::onboarding::Submission>),
    /// Wait for the sign-in for this address to finish, and the account to
    /// be saved.
    FinishOAuth(String),
    /// Give up the sign-in for this address.
    CancelOAuth(String),
    /// Prove a new account's credentials and save it (the password goes to
    /// the keyring and nowhere else).
    AddAccount(Box<postio_ui::onboarding::Submission>),
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
    /// Contacts, best first.
    pub contacts: Vec<postio_model::Contact>,
}
