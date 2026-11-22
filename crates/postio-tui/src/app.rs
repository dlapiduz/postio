//! The terminal frontend's state, and the one function that changes it.
//!
//! `update` takes an [`Input`] and returns the [`Effect`]s it asks for --
//! a request of the host, a redraw, quitting -- and does no I/O itself.
//! That is what lets every behaviour be driven by synthetic input in a test,
//! with no terminal and no host (research R11).
//!
//! # The list is a window
//!
//! A mailbox is never loaded (Principle V). The list is
//! `postio_ui::list::ListWindow`, the desktop app's and the macOS frontend's
//! own window, paged by `postio_ui::paging::Paging`; after every input,
//! [`App`] walks only the rows in view and asks for whichever of their pages
//! is not already here or on its way.

use crossterm::event::KeyEvent;
use postio_model::ListScope;
use postio_ui::keymap::{KeyContext, Outcome};
use postio_ui::list::ListWindow;

mod capture;
mod digest;
mod filtered;
mod find;
mod list;
mod open;
mod rules;
mod surface;

pub use find::Find;
pub use open::{Menu, MenuAction, MenuItem, Raw};

use crate::focus::Held;
use crate::input::Keys;
use crate::row::Row;
use crate::view::list::{Heading, Visible};
use postio_body::replying::ReplyKind;

/// What the mouse did, resolved against the last frame drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pointer {
    /// A left click.
    Click {
        /// What was under it.
        hit: crate::view::hit::Hit,
        /// Whether Ctrl was held: toggle, rather than move.
        ctrl: bool,
        /// Whether Shift was held: extend, rather than move.
        shift: bool,
    },
    /// A turn of the wheel.
    Wheel {
        /// What was under the pointer.
        hit: crate::view::hit::Hit,
        /// Towards the end, rather than the start.
        down: bool,
    },
}

/// Something that happened, from the terminal or from the host.
#[derive(Debug, Clone, PartialEq)]
pub enum Input {
    /// The terminal is now this many columns and rows.
    Resize(u16, u16),
    /// The clipboard answered an [`Effect::ReadClipboardImage`]: a PNG,
    /// no image, or why it could not be read.
    ClipboardImage(Result<Option<Vec<u8>>, String>),
    /// The host stored an [`Effect::InlineImage`]: the inline part, or
    /// nothing when it could not.
    InlineStored(Option<postio_model::Attachment>),
    /// `config.toml` came back from the person's editor, and was read again.
    ConfigEdited(Result<(), String>),
    /// The external editor exited: what it saved, or why not.
    Edited {
        /// Which composition.
        generation: u64,
        /// The body it saved.
        edited: Result<String, String>,
    },
    /// The host answered an [`Effect::BeginOAuth`]: where the sign-in
    /// waits for the person, or why it could not begin.
    Consent(Result<postio_ui::onboarding::BrowserSignIn, String>),
    /// The host answered an [`Effect::Discover`].
    Discovered(Result<postio_ui::onboarding::Status, String>),
    /// The host answered an [`Effect::AddAccount`]: saved, or the sentence
    /// for why not.
    AccountAdded(Result<(), String>),
    /// The mouse did something over what was drawn.
    Pointer(Pointer),
    /// Text was pasted, or files were dropped: a drop arrives as a paste of
    /// their paths.
    Paste(String),
    /// The host answered an [`Effect::Attach`]: the stored attachment, or
    /// nothing when the file could not be read.
    Attached {
        /// What was asked to be attached.
        path: std::path::PathBuf,
        /// The attachment.
        attached: Option<postio_model::Attachment>,
    },
    /// A key was pressed.
    Key(KeyEvent),
    /// The controller's request was answered.
    FocusReply(postio_focus::Ticket, postio_focus::Reply),
    /// A timer the controller armed ran out.
    FocusTimer(u64),
    /// The host said something happened.
    Host(postio_core::Event),
    /// A conversation asked for by [`Effect::ReadConversation`] arrived.
    Conversation {
        /// Which.
        thread: postio_model::ThreadId,
        /// Its messages, oldest first, or why there are none.
        members: Result<Vec<postio_model::listing::MessageSummary>, String>,
    },
    /// Whom a message asked for by [`Effect::ReadBody`] was written to, for
    /// the open message's header. Arrives just before its body, from the
    /// same read.
    Addressed {
        /// Whose.
        message: postio_model::MessageId,
        /// Whom it was sent to.
        to: Vec<postio_model::EmailAddress>,
        /// Who was copied.
        cc: Vec<postio_model::EmailAddress>,
        /// The one line about a sender who may not be who they say, if any
        /// (#1928).
        warning: Option<String>,
    },
    /// A body asked for by [`Effect::ReadBody`] arrived.
    Body {
        /// Whose.
        message: postio_model::MessageId,
        /// The body, or why there is none.
        answer: Result<postio_client::protocol::Body, String>,
    },
    /// The host answered an [`Effect::Ask`].
    Answer(crate::ask::Answer),
    /// The host answered an [`Effect::Unsubscribe`]: the list's name, or
    /// why not.
    Unsubscribed {
        /// The message `M` was pressed over.
        message: postio_model::MessageId,
        /// The list left, or why not.
        answer: Result<String, String>,
    },
    /// The host answered an [`Effect::ReadListMail`] (#1931).
    ListMail {
        /// The message the unsubscribe was from.
        message: postio_model::MessageId,
        /// The list or sender it left, as the confirmation names it.
        name: String,
        /// What the list left behind, or why it could not be read.
        found: Result<postio_client::protocol::ListMailFound, String>,
    },
    /// The host answered an [`Effect::ReadSource`]: the bytes, or why not.
    Source {
        /// Whose.
        message: postio_model::MessageId,
        /// What came off the wire.
        raw: Result<Vec<u8>, String>,
    },
    /// An [`Effect::Settle`]'s time is up: paint the open message as it is.
    Settled {
        /// Which hold it was asked for.
        generation: u64,
    },
    /// An [`Effect::ArmDwell`]'s time is up.
    DwellDue {
        /// Which clock.
        generation: u64,
        /// The message it was armed for.
        message: postio_model::MessageId,
    },
    /// An [`Effect::ExpireNotice`]'s time is up.
    NoticeDue {
        /// Which notice it was asked for.
        generation: u64,
    },
    /// An [`Effect::Autosave`]'s time is up.
    AutosaveDue {
        /// Which composition asked.
        generation: u64,
        /// How many edits it had when it asked.
        edit: u64,
    },
    /// A signature was saved or removed, or why it could not be: the
    /// store's sentence, which is for the person who typed.
    SignatureSaved(Result<(), String>),
    /// The privacy pane's log, as the store has it now.
    Privacy {
        /// Unsubscribes and read-receipt requests.
        log: postio_client::protocol::PrivacyLog,
        /// The newest outbound connections, newest first.
        connections: Vec<postio_model::egress::EgressEvent>,
    },
    /// The host answered an [`Effect::SaveDraft`]: the draft's id, or why
    /// it could not be saved.
    DraftSaved {
        /// Which composition.
        generation: u64,
        /// Its id, or the sentence for the status line.
        saved: Result<postio_model::DraftId, String>,
    },
    /// New mail worth telling the person about, as the host decided it.
    Notified(postio_ui::notify::Notification),
    /// What a label picker lists, read ([`Effect::ReadLabelPicker`]).
    LabelPicker {
        /// The account the labels are of.
        account: postio_model::AccountId,
        /// Its labels.
        labels: Vec<postio_model::Label>,
        /// How many conversations carry each.
        counts: Vec<(postio_model::LabelId, u32)>,
        /// Which every conversation the picker acts on carries.
        applied: std::collections::BTreeSet<postio_model::LabelId>,
    },
    /// The host answered an [`Effect::CreateLabel`]: the new label.
    LabelMade {
        /// The label, or nothing when it could not be made.
        label: Option<postio_model::Label>,
        /// Whether the picker closes now.
        close: bool,
    },
    /// The last folders mail was moved to ([`Effect::ReadRecentMoves`]).
    RecentMoves(Vec<postio_model::MailboxId>),
    /// The host answered an [`Effect::BarSearch`].
    BarFound {
        /// Which question it answers.
        sequence: u64,
        /// What matched, nothing when the store could not be read, or why
        /// the host could not be asked.
        found: Result<Option<postio_client::protocol::Hits>, String>,
        /// Which of the hits a digest holds: the message, the rule and
        /// whether it has been delivered.
        held: Vec<(postio_model::MessageId, String, bool)>,
    },
    /// The host answered an [`Effect::BarFolder`].
    BarFolder {
        /// Which question it answers.
        sequence: u64,
        /// How many conversations the folder holds.
        count: u32,
        /// The newest of them.
        rows: Vec<crate::bar::ResultRow>,
    },
    /// The labels and correspondents of every account, read for the bar
    /// ([`Effect::ReadPlaceDetails`]).
    PlaceDetails(crate::places::PlaceDetails),
    /// The host answered an [`Effect::Recipients`].
    Recipients {
        /// What was looked up.
        prefix: String,
        /// Who it could be, best first.
        found: Vec<postio_model::contact_group::RecipientCandidate>,
    },
    /// The host answered an [`Effect::QueueSend`].
    Queued {
        /// When it was scheduled for, if it was.
        at: Option<chrono::DateTime<chrono::Utc>>,
        /// Whether it was queued, or why not.
        queued: Result<(), String>,
    },
    /// The host answered an [`Effect::Resume`]: the draft behind the row,
    /// taken back from the Outbox if it was queued; nothing when there is no
    /// local draft or its send has already started.
    Resumed {
        /// The draft, or nothing.
        found: Option<Box<postio_model::Draft>>,
        /// Why its last send failed, for a draft that did.
        failure: Option<String>,
    },
    /// The host answered an [`Effect::ReplySource`]: the message and its
    /// account, or nothing when it could not be read.
    ReplySource {
        /// Which draft to start.
        kind: ReplyKind,
        /// The message and its account.
        found: Option<Box<(postio_model::Message, postio_model::Account)>>,
    },
    /// A message's parts arrived.
    Parts {
        /// Whose.
        message: postio_model::MessageId,
        /// Its parts, or why there are none.
        parts: Result<Vec<postio_model::Attachment>, String>,
    },
    /// A part was written, to be opened or just kept.
    PartWritten {
        /// Where, or why not.
        written: Result<std::path::PathBuf, String>,
        /// Whether it was written to be opened.
        open: bool,
    },
    /// What the places hold, read afresh.
    Places(crate::places::Places),
}

/// Something `update` asks the loop to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    /// Draw a frame.
    Redraw,
    /// Leave.
    Quit,
    /// Ask the host something for a Focus surface and answer with
    /// [`Input::Answer`].
    Ask(crate::ask::Ask),
    /// Read a conversation and answer with [`Input::Conversation`].
    ReadConversation(postio_model::ThreadId),
    /// Read a body and answer with [`Input::Body`].
    ReadBody(postio_model::MessageId),
    /// Read the places again and answer with [`Input::Places`].
    RefreshPlaces,
    /// Leave the list this message came from; answer with
    /// [`Input::Unsubscribed`].
    Unsubscribe(postio_model::MessageId),
    /// Read what the list this message came from has sent, and answer with
    /// [`Input::ListMail`]: the offer after an unsubscribe (#1931). `name` is
    /// what the confirmation says it left.
    ReadListMail {
        /// The message `M` was pressed over.
        message: postio_model::MessageId,
        /// The list or sender left.
        name: String,
    },
    /// Read a message's parts and answer with [`Input::Parts`].
    ReadParts(postio_model::MessageId),
    /// Hand a new-mail notice to the desktop's notification service, where
    /// one is reachable.
    DesktopNotify {
        /// What it says first.
        title: String,
        /// The rest.
        body: String,
    },
    /// Read a label picker's labels, their counts, and which the
    /// conversations carry; the answer comes back as [`Input::LabelPicker`].
    ReadLabelPicker {
        /// The message whose account's labels are offered.
        message: postio_model::MessageId,
        /// The account to offer when the message's cannot be found.
        account: postio_model::AccountId,
        /// The conversations whose labels are shown as applied.
        threads: Vec<postio_model::ThreadId>,
    },
    /// Make a label; the answer comes back as [`Input::LabelMade`].
    CreateLabel {
        /// Whose.
        account: postio_model::AccountId,
        /// Its name.
        name: String,
        /// Whether the picker closes once it is made.
        close: bool,
    },
    /// Read the folders mail was last moved to; the answer comes back as
    /// [`Input::RecentMoves`].
    ReadRecentMoves,
    /// Remember a move, for the picker's Recent.
    NoteMove(postio_model::MailboxId),
    /// Write a part to a file, and answer with [`Input::PartWritten`].
    SavePart {
        /// Whose.
        message: postio_model::MessageId,
        /// Which.
        attachment: postio_model::ids::AttachmentId,
        /// Where; `None` for a private copy to open.
        to: Option<std::path::PathBuf>,
    },
    /// Hand a file to the system's opener.
    Launch(std::path::PathBuf),
    /// Write the remote-image allow list, which the desktop app reads too.
    SaveAllowlist(postio_ui::allowlist::RemoteImageAllowList),
    /// Write `[focus] reading` to `config.toml`: where messages open.
    SetReading(postio_config::Reading),
    /// Read a message's source and answer with [`Input::Source`].
    ReadSource(postio_model::MessageId),
    /// Ask for [`Input::Settled`] after `after`: the longest the paint is
    /// held for a message's reads ([`App::holds_paint`]).
    Settle {
        /// Which hold.
        generation: u64,
        /// How long.
        after: std::time::Duration,
    },
    /// Ask for [`Input::DwellDue`] after `after`: how long the message has
    /// to stay open to count as read.
    ArmDwell {
        /// Which clock.
        generation: u64,
        /// The message on screen.
        message: postio_model::MessageId,
        /// How long.
        after: std::time::Duration,
    },
    /// Ask for [`Input::NoticeDue`] after `after`: the toast's time.
    ExpireNotice {
        /// Which notice, so a newer one is not taken down by an older timer.
        generation: u64,
        /// How long it stays.
        after: std::time::Duration,
    },
    /// Ask again for [`Input::AutosaveDue`] after [`AUTOSAVE`].
    Autosave {
        /// Which composition.
        generation: u64,
        /// How many edits it has now.
        edit: u64,
    },
    /// Save a draft through the host's draft writer.
    SaveDraft {
        /// Which composition.
        generation: u64,
        /// The draft.
        draft: Box<postio_model::Draft>,
    },
    /// A composition closed with nothing worth keeping: drop its row.
    DiscardDraft {
        /// Which composition.
        generation: u64,
        /// Its id, when it has one.
        known: Option<postio_model::DraftId>,
    },
    /// Queue a draft to send, through the same writer as its saves, so it
    /// goes after the last of them.
    QueueSend {
        /// Which composition.
        generation: u64,
        /// The draft.
        draft: Box<postio_model::Draft>,
        /// When, for a scheduled send.
        at: Option<chrono::DateTime<chrono::Utc>>,
    },
    /// Hand the body to the person's own editor; the loop suspends the
    /// screen while it runs.
    EditExternally {
        /// Which composition.
        generation: u64,
        /// The body, as typed.
        markdown: String,
    },
    /// Read an image off the clipboard: on the paste key, never otherwise.
    ReadClipboardImage,
    /// Store pasted image bytes as an inline part of the draft.
    InlineImage {
        /// The image.
        bytes: Vec<u8>,
        /// Its type.
        mime_type: String,
    },
    /// Store the file at this path as an attachment of the draft.
    Attach(std::path::PathBuf),
    /// Save this query as a saved search in `config.toml`, as the desktop's
    /// Ctrl+S does.
    SaveSearch(String),
    /// Run the bar's search; its answer comes back as [`Input::BarFound`].
    BarSearch(crate::bar::Ask),
    /// List a folder's conversations for the bar; the answer comes back as
    /// [`Input::BarFolder`].
    BarFolder {
        /// Which question this is.
        sequence: u64,
        /// The folder.
        mailbox: postio_model::MailboxId,
    },
    /// Read every account's labels and correspondents; the answer comes back
    /// as [`Input::PlaceDetails`].
    ReadPlaceDetails,
    /// Look up who a recipient being typed could be.
    Recipients {
        /// Whose contacts.
        account: postio_model::AccountId,
        /// What has been typed.
        prefix: String,
    },
    /// Open the local draft behind a Drafts or Outbox row.
    Resume(postio_model::MessageId),
    /// Edit `config.toml` in the person's own editor, at a section.
    EditConfig(Option<postio_ui::settings::Section>),
    /// Change an account, as the settings' account commands do.
    Account(postio_client::protocol::AccountOp),
    /// Begin a browser sign-in for a new account.
    BeginOAuth(Box<postio_ui::onboarding::Submission>),
    /// Wait for the sign-in for this address to end.
    FinishOAuth(String),
    /// Give up the sign-in for this address.
    CancelOAuth(String),
    /// Put this text on the clipboard of the terminal's own machine.
    CopyText(String),
    /// Look up a new account's servers.
    Discover(String),
    /// Prove and save a new account.
    AddAccount(Box<postio_ui::onboarding::Submission>),
    /// Save how far back the first sync reaches.
    SaveSyncWindow(postio_ui::onboarding::SyncWindow),
    /// Read the privacy pane's log: what left this machine.
    ReadPrivacy,
    /// Hand a signature's text to the person's editor, then save what it
    /// wrote under `name`: a new signature when `signature` is `None`.
    EditSignature {
        /// Whose.
        account: postio_model::AccountId,
        /// Which, or a new one.
        signature: Option<postio_model::SignatureId>,
        /// What it is called.
        name: String,
        /// Its text before the edit.
        text: String,
    },
    /// Save a signature as it is given, as a rename does.
    SaveSignature {
        /// Whose.
        account: postio_model::AccountId,
        /// Which, or a new one.
        signature: Option<postio_model::SignatureId>,
        /// What it is called.
        name: String,
        /// Its text.
        text: String,
    },
    /// Remove a signature.
    DeleteSignature(postio_model::SignatureId),
    /// Open a link with the system's opener, the person having clicked it
    /// twice.
    OpenLink(String),
    /// Read the message a reply or forward starts from, and its account.
    ReplySource {
        /// Which draft to start.
        kind: ReplyKind,
        /// The message.
        message: postio_model::MessageId,
    },
    /// Send a command to the host, aimed with [`App::state`].
    Send(postio_core::Command),
    /// Ask the controller's request of the host and answer with
    /// [`Input::FocusReply`].
    Focus {
        ticket: postio_focus::Ticket,
        request: postio_focus::Request,
    },
    /// Run the controller's timer.
    FocusTimer {
        token: u64,
        after: std::time::Duration,
    },
}

/// How long the typing has to pause before a draft is saved: the desktop
/// composer's autosave interval.
pub const AUTOSAVE: std::time::Duration = std::time::Duration::from_millis(1500);

/// How long a toast stays on the bottom line.
pub const TOAST: std::time::Duration =
    std::time::Duration::from_secs(postio_ui::focus_target::TOAST_SECONDS as u64);

/// How many lines one turn of the wheel scrolls.
const WHEEL: isize = 3;

/// Everything the terminal frontend knows.
pub struct App {
    size: (u16, u16),
    keys: Keys,
    list: ListWindow<Held>,
    ctl: postio_focus::FocusController,
    selection_words: Option<String>,
    /// The row the keyboard is on.
    cursor: u32,
    /// The first row in view.
    top: u32,
    /// What the host aims this frontend's commands with; the client sends
    /// a snapshot of it with each one (ADR 0041).
    state: postio_core::SharedState,
    /// What is marked.
    selection: postio_ui::selection::SelectionState,
    /// What the strip counts, once read.
    counts: Option<postio_client::protocol::FocusCounts>,
    /// Which of Focus's features `config.toml` has in use.
    features: crate::places::Features,
    /// The accounts whose connection has been heard of.
    tracked: Vec<postio_model::AccountId>,
    /// When mail last arrived, as far as is known.
    last_synced: Option<chrono::DateTime<chrono::Utc>>,
    /// The list being shown.
    scope: Option<ListScope>,
    /// What the status line says about the last thing done: the undo offer,
    /// a refusal, an error.
    notice: Option<String>,
    /// What kind of news the notice is, for its mark and colour.
    notice_tone: Tone,
    /// The key the notice offers undo on, when it offers it.
    notice_undo: Option<String>,
    /// The one button the notice offers, when it offers one (#1952): its
    /// label and the command a click runs.
    notice_offer: Option<(String, postio_core::Command)>,
    /// Which notice is on the line: a timer is for one of them.
    notice_generation: u64,
    /// An answer to an invitation was sent, and its toast is the next one:
    /// it lasts as long as the reply waits.
    answering: bool,
    /// Where the keyboard is.
    focus: Focus,
    /// The folders, views and saved searches the finder and `g l` read.
    places: crate::places::Places,
    /// The privacy pane's log, once read.
    privacy: Option<Privacy>,
    /// What the palette is naming, while it is.
    renaming: Option<Renaming>,
    /// Each account's sync status, folded from the host's events.
    trackers: postio_ui::status::Trackers,
    /// Which account the list on screen belongs to.
    account: Option<postio_model::AccountId>,
    /// Every folder, to find the account a list belongs to.
    folders: Vec<postio_model::mailbox::Mailbox>,
    /// The message the reader shows, and how.
    reading: Option<crate::conversation::Reading>,
    /// The first reader line in view.
    reader_top: usize,
    /// What the open message holds besides the message.
    open: open::Open,
    /// Where saved parts go.
    downloads: std::path::PathBuf,
    /// Senders whose remote images are always allowed, shared with the
    /// desktop app (`postio_ui::allowlist`).
    allowlist: postio_ui::allowlist::RemoteImageAllowList,
    /// The draft being written, which takes the reading pane while it is.
    composer: Option<crate::composer::Composer>,
    /// How many compositions this frontend has started: each is named by the
    /// next, for the host's draft writer.
    compositions: u64,
    /// Every account, for the addresses a composer can send as.
    accounts: Vec<postio_model::Account>,
    /// The schedule-send picker's times, while it is open.
    scheduling: Option<[(&'static str, chrono::DateTime<chrono::Local>); 4]>,
    /// The composer's edit count when "send it anyway?" was asked: sending
    /// again with nothing changed is the answer.
    asked_at: Option<u64>,
    /// The path being typed to attach a file (FR-027), while it is.
    path_prompt: Option<tui_input::Input>,
    /// How the composer shows its preview (`[tui].preview`).
    preview: postio_config::Preview,
    /// Whether the preview is showing.
    previewing: bool,
    /// When the draft was last saved on this machine, for the subtitle.
    saved_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Where the keyboard goes when the composer closes: the message it was
    /// started from, or the list.
    composed_from: Focus,
    /// Whether the draft is in a tab of its own rather than the reading
    /// pane: the desktop's composer window (FR-003).
    detached: bool,
    /// The command bar, while it is open.
    bar: Option<crate::bar::Bar>,
    /// The folders popover, while it is open.
    places_box: Option<crate::folders::Folders>,
    /// The snooze, remind, label or move picker, while it is open.
    picker: Option<crate::pickers::Picker>,
    /// The palette, while it is open.
    palette: Option<PaletteState>,
    /// Whether the terminal speaks the kitty keyboard protocol, so every
    /// chord arrives; otherwise only what a legacy terminal can send does.
    enhanced_keys: bool,
    /// The key map, while it is open.
    sheet: Option<crate::sheet::Sheet>,
    /// A link clicked once: shown in full, and opened by a second click
    /// on it (US2 scenario 4).
    armed_link: Option<String>,
    /// Whether the mouse is listened to (`[tui].mouse`).
    mouse: bool,
    /// The first run, while there is no account yet.
    first_run: Option<crate::first_run::FirstRun>,
    /// The settings, while they are open.
    settings: Option<crate::settings::Settings>,
    /// What Focus's surfaces hold: Filtered and its sweep.
    surfaces: crate::surface::Surfaces,
}

/// What the strip says, with no drawing in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Strip {
    /// The place's name.
    pub place: postio_ui::terminal::SafeText,
    /// `312 · 41 unread`.
    pub counts: String,
    /// The has-action toggle, in Focus's inbox.
    pub toggle: Option<Toggle>,
    /// `Showing 7 of 312 · ! again to show all`, while the filter is on.
    pub showing: Option<String>,
    /// `186 filtered today`, while filtering is on and has filed anything.
    pub filtered: Option<String>,
    /// `4 digest rules`, while there are rules.
    pub rules: Option<String>,
}

/// The has-action toggle: its words and whether it is on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Toggle {
    /// `Has action · 7`.
    pub label: String,
    /// Whether the filter is on.
    pub on: bool,
}

/// What fills the window's body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Front {
    /// Day headings and rows.
    List,
    /// The open message.
    Reader,
    /// The draft being written.
    Composer,
}

/// What kind of news a notice is: the status line marks each with more
/// than a colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    /// Something to know.
    Plain,
    /// Something done.
    Worked,
    /// Something refused or failed.
    Failed,
}

/// What a name typed in the palette is for.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Renaming {
    /// A signature of `account`: `signature`, or a new one, whose text is
    /// `text`.
    Signature {
        account: postio_model::AccountId,
        signature: Option<postio_model::SignatureId>,
        text: String,
    },
}

/// What the privacy pane shows that is not in `config.toml`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Privacy {
    /// Unsubscribes and read-receipt requests.
    pub log: postio_client::protocol::PrivacyLog,
    /// The newest outbound connections, newest first.
    pub connections: Vec<postio_model::egress::EgressEvent>,
}

/// Which of the finder's modes the palette is in (`postio_ui::finder`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Finding {
    /// `M` in the settings: which of an account's roles to point somewhere.
    Roles(postio_model::AccountId),
    /// Then which folder that role is, or automatic.
    RoleFolder(postio_model::AccountId, postio_model::mailbox::MailboxRole),
    /// A new name for the saved search in `App::renaming`.
    Rename,
}

/// A role's name in a sentence.
fn role_name(role: postio_model::mailbox::MailboxRole) -> &'static str {
    use postio_model::mailbox::MailboxRole;
    match role {
        MailboxRole::Inbox => "Inbox",
        MailboxRole::Sent => "Sent",
        MailboxRole::Drafts => "Drafts",
        MailboxRole::Archive => "Archive",
        MailboxRole::Trash => "Trash",
        MailboxRole::Junk => "Junk",
        MailboxRole::Flagged => "Flagged",
        _ => "such",
    }
}

/// The palette: what is typed, which row is chosen, and where it was opened
/// from, which is where what it runs runs.
#[derive(Debug)]
struct PaletteState {
    input: tui_input::Input,
    selected: usize,
    from: Focus,
    finding: Finding,
}

/// The palette as it is drawn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaletteView {
    /// The finder mode's marker: `>`, `#`, `+` or `@`.
    pub marker: &'static str,
    /// What is typed.
    pub query: String,
    /// The rows, best first.
    pub rows: Vec<PaletteRow>,
    /// The chosen row.
    pub selected: usize,
}

/// One palette row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaletteRow {
    /// What it says.
    pub title: String,
    /// What the row says at its right: the key that does the same, as this
    /// terminal can send it.
    pub chord: Option<String>,
    /// Which characters of the title the query matched.
    pub positions: Vec<usize>,
}

/// What a palette row does.
enum PaletteAction {
    /// Ask which folder this role is.
    PickRole(postio_model::AccountId, postio_model::mailbox::MailboxRole),
    /// Point the role at this folder's path, or back to automatic.
    MapRole(
        postio_model::AccountId,
        postio_model::mailbox::MailboxRole,
        Option<String>,
    ),
    /// Call the saved search being renamed this.
    Rename(String),
}

/// Rows offered by title, matched against `query` with the palette's
/// matcher, best first; an empty query keeps the order they were offered in.
fn best_first(
    query: &str,
    offered: impl Iterator<Item = (String, PaletteAction)>,
) -> Vec<(PaletteRow, PaletteAction)> {
    let mut found: Vec<(i32, PaletteRow, PaletteAction)> = offered
        .filter_map(|(title, action)| {
            let matched = postio_ui::palette::score(query.trim(), &title)?;
            Some((
                matched.score,
                PaletteRow {
                    title,
                    chord: None,
                    positions: matched.positions,
                },
                action,
            ))
        })
        .collect();
    found.sort_by_key(|(score, ..)| std::cmp::Reverse(*score));
    found
        .into_iter()
        .map(|(_, row, action)| (row, action))
        .collect()
}

/// Which pane the keyboard is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Focus {
    /// The message list.
    #[default]
    List,
    /// The reading pane.
    Reader,
    /// The composer, in the reading pane.
    Composer,
    /// The command bar.
    Bar,
    /// The folders popover.
    Folders,
    /// A picker over the focused row.
    Picker,
    /// The key map.
    Keys,
    /// The first run, while there is no account.
    FirstRun,
    /// The settings.
    Settings,
    /// The command palette, or another of the finder's modes.
    Palette,
    /// The Filtered view, which takes the window's body.
    Filtered,
    /// A digest's window, in the message frame.
    Digest,
    /// The digest rules, which take the window's body.
    Rules,
    /// The rule dialog, over whatever opened it.
    RuleDialog,
    /// The capture sheet, over whatever opened it.
    Capture,
}

impl std::fmt::Debug for App {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("App")
            .field("size", &self.size)
            .field("cursor", &self.cursor)
            .field("total", &self.list.total())
            .finish_non_exhaustive()
    }
}

impl App {
    /// A frontend in a terminal of `size`, resolving keys with `keys`.
    pub fn new(size: (u16, u16), keys: Keys) -> App {
        App {
            size,
            keys,
            list: ListWindow::new(),
            ctl: postio_focus::FocusController::new(crate::focus::policy()),
            selection_words: None,
            cursor: 0,
            top: 0,
            state: postio_core::SharedState::default(),
            selection: postio_ui::selection::SelectionState::new(),
            counts: None,
            features: crate::places::Features::default(),
            tracked: Vec::new(),
            last_synced: None,
            scope: None,
            notice: None,
            notice_tone: Tone::Plain,
            notice_undo: None,
            notice_offer: None,
            notice_generation: 0,
            answering: false,
            focus: Focus::List,
            places: crate::places::Places::default(),
            privacy: None,
            renaming: None,
            trackers: postio_ui::status::Trackers::default(),
            account: None,
            folders: Vec::new(),
            reading: None,
            reader_top: 0,
            open: open::Open::default(),
            allowlist: postio_ui::allowlist::RemoteImageAllowList::default(),
            downloads: std::path::PathBuf::from("."),
            composer: None,
            compositions: 0,
            accounts: Vec::new(),
            scheduling: None,
            asked_at: None,
            path_prompt: None,
            preview: postio_config::Preview::default(),
            previewing: false,
            detached: false,
            saved_at: None,
            composed_from: Focus::List,
            bar: None,
            places_box: None,
            picker: None,
            palette: None,
            enhanced_keys: false,
            sheet: None,
            armed_link: None,
            mouse: true,
            first_run: None,
            settings: None,
            surfaces: crate::surface::Surfaces::default(),
        }
    }

    /// The same app, saving parts into `downloads`.
    pub fn with_downloads(mut self, downloads: std::path::PathBuf) -> App {
        self.downloads = downloads;
        self
    }

    /// The same app, honouring `allowlist`.
    pub fn with_allowlist(mut self, allowlist: postio_ui::allowlist::RemoteImageAllowList) -> App {
        self.allowlist = allowlist;
        self
    }

    /// The list position of the first row in view.
    pub fn top(&self) -> u32 {
        self.top
    }

    /// The first reader line in view.
    pub fn reader_top(&self) -> usize {
        self.reader_top
    }

    /// The row for `message`, if it is resident.
    pub fn row(&self, message: postio_model::MessageId) -> Option<&Row> {
        self.list.row_of(message).map(|held| &held.row)
    }

    /// What the reader shows: the message, and its rendered body.
    pub fn reading(&self) -> Option<&crate::conversation::Reading> {
        self.reading.as_ref()
    }

    /// Which account `scope` belongs to.
    fn account_of(&self, scope: ListScope) -> Option<postio_model::AccountId> {
        match scope {
            ListScope::Mailbox(mailbox) => self
                .folders
                .iter()
                .find(|folder| folder.id == mailbox)
                .map(|folder| folder.account_id),
            ListScope::Account(account)
            | ListScope::Flagged(account)
            | ListScope::Snoozed(account)
            | ListScope::Outbox(account) => Some(account),
            ListScope::Unified | ListScope::Thread(_) | ListScope::Focus(_) => None,
        }
    }

    /// Where the keyboard is.
    pub fn focus(&self) -> Focus {
        self.focus
    }

    /// The same app, showing the composer's preview as `preview` says.
    pub fn with_preview(mut self, preview: postio_config::Preview) -> App {
        self.preview = preview;
        self
    }

    /// How the preview is shown, when it is: instead of the text, or beside
    /// it; `None` when it is not showing.
    pub fn preview_shown(&self) -> Option<postio_config::Preview> {
        self.previewing.then_some(self.preview)
    }

    /// Whether the draft is in a tab of its own.
    pub fn composer_detached(&self) -> bool {
        self.detached
    }

    /// What fills the window's body: the list, or, in front of it, the
    /// message being read or the draft being written.
    pub fn front(&self) -> Front {
        let at = self.palette.as_ref().map_or(self.focus, |open| open.from);
        match at {
            Focus::Composer if self.composer.is_some() => Front::Composer,
            Focus::Reader if self.reading.is_some() => Front::Reader,
            _ => Front::List,
        }
    }

    /// The draft being written, if one is.
    pub fn composer(&self) -> Option<&crate::composer::Composer> {
        self.composer.as_ref()
    }

    /// Start writing `draft`: the composer takes the reading pane and the
    /// keyboard.
    pub fn compose(&mut self, draft: postio_model::Draft) -> Vec<Effect> {
        self.compositions += 1;
        let identities = self
            .accounts
            .iter()
            .find(|account| account.id == draft.account_id)
            .map(|account| account.identities.clone())
            .unwrap_or_default();
        self.composer = Some(
            crate::composer::Composer::new(self.compositions, draft).with_identities(identities),
        );
        self.composed_from = if self.focus == Focus::Reader && self.reading.is_some() {
            Focus::Reader
        } else {
            Focus::List
        };
        self.saved_at = None;
        self.focus = Focus::Composer;
        self.detached = false;
        // Side by side is shown from the start; the toggle starts on the text.
        self.previewing = self.preview == postio_config::Preview::Split;
        vec![Effect::Redraw]
    }

    /// Leave the composer, back to the list where it was: saving what was
    /// written, and dropping a composition nothing was written in, by the
    /// desktop's own rule (`postio_model::draft::closing`).
    fn close_composer(&mut self) -> Vec<Effect> {
        let mut effects = Vec::new();
        if let Some(composer) = self.composer.take() {
            let generation = composer.generation();
            let draft = composer.draft();
            match postio_model::draft::closing(&draft) {
                postio_model::draft::Closing::Keep => effects.push(Effect::SaveDraft {
                    generation,
                    draft: Box::new(draft),
                }),
                postio_model::draft::Closing::Drop => effects.push(Effect::DiscardDraft {
                    generation,
                    known: draft.id.is_assigned().then_some(draft.id),
                }),
            }
        }
        self.detached = false;
        self.focus = self.after_composing();
        effects.push(Effect::Redraw);
        effects
    }

    /// Where the keyboard goes once the composer is done with it.
    fn after_composing(&self) -> Focus {
        if self.composed_from == Focus::Reader && self.reading.is_some() {
            Focus::Reader
        } else {
            Focus::List
        }
    }

    /// The subtitle's note on the draft: when it was last saved here.
    pub fn saved_note(&self) -> Option<String> {
        self.saved_at.map(postio_ui::compose::saved_at)
    }

    /// A save is due, if nothing was typed since it was asked for.
    fn autosave_due(&mut self, generation: u64, edit: u64) -> Vec<Effect> {
        match &self.composer {
            Some(composer) if composer.generation() == generation && composer.edits() == edit => {
                vec![Effect::SaveDraft {
                    generation,
                    draft: Box::new(composer.draft()),
                }]
            }
            _ => Vec::new(),
        }
    }

    /// Whether the list on screen is where drafts are: the Drafts folder, or
    /// the Outbox that is a view of it.
    fn listing_drafts(&self) -> bool {
        match self.scope {
            Some(ListScope::Outbox(_)) => true,
            Some(ListScope::Mailbox(mailbox)) => self.folders.iter().any(|folder| {
                folder.id == mailbox && folder.role == postio_model::mailbox::MailboxRole::Drafts
            }),
            _ => false,
        }
    }

    /// A key while the composer has the keyboard: a composer command if it
    /// names one, and otherwise typed (FR-022a). The keymap is asked in
    /// text entry, so a plain letter is handed back rather than resolved.
    fn composer_key(&mut self, key: &KeyEvent) -> Vec<Effect> {
        if self.scheduling.is_some() {
            return self.schedule_key(key);
        }
        if self.path_prompt.is_some() {
            return self.path_key(key);
        }
        if let Some(composer) = self.composer.as_mut() {
            let before = composer.edits();
            if composer.completion_key(*key) {
                let mut effects = vec![Effect::Redraw];
                if composer.edits() != before {
                    effects.push(Effect::Autosave {
                        generation: composer.generation(),
                        edit: composer.edits(),
                    });
                }
                return effects;
            }
        }
        match self.keys.press(key, KeyContext::Composer, true) {
            Outcome::Command(id) => self.composer_command(&id),
            Outcome::Pending(_) => Vec::new(),
            Outcome::Unhandled => {
                let Some(composer) = self.composer.as_mut() else {
                    return Vec::new();
                };
                let before = composer.edits();
                let mut effects = Vec::new();
                if composer.type_key(*key) {
                    effects.push(Effect::Redraw);
                }
                if composer.edits() != before {
                    // Saved once the typing pauses: each edit asks for a
                    // timer, and only the newest one's still matches.
                    effects.push(Effect::Autosave {
                        generation: composer.generation(),
                        edit: composer.edits(),
                    });
                }
                if let Some(prefix) = composer.wants_completion() {
                    effects.push(Effect::Recipients {
                        account: composer.draft().account_id,
                        prefix,
                    });
                }
                effects
            }
        }
    }

    /// The path being typed to attach a file, while it is.
    pub fn path_prompt(&self) -> Option<&str> {
        self.path_prompt.as_ref().map(tui_input::Input::value)
    }

    /// A key in the path prompt: Enter attaches, Tab completes from the
    /// disk, Escape goes back to writing.
    fn path_key(&mut self, key: &KeyEvent) -> Vec<Effect> {
        use crossterm::event::KeyCode;
        use tui_input::backend::crossterm::EventHandler;
        let Some(prompt) = self.path_prompt.as_mut() else {
            return Vec::new();
        };
        match key.code {
            KeyCode::Esc => self.path_prompt = None,
            KeyCode::Enter => {
                let typed = prompt.value().trim().to_owned();
                self.path_prompt = None;
                if !typed.is_empty() {
                    return vec![Effect::Attach(crate::paths::expand(&typed)), Effect::Redraw];
                }
            }
            KeyCode::Tab => {
                let completed = crate::paths::complete(prompt.value());
                *prompt = tui_input::Input::default().with_value(completed);
            }
            _ => {
                prompt.handle_event(&crossterm::event::Event::Key(*key));
            }
        }
        vec![Effect::Redraw]
    }

    /// A paste, or a drop, while writing: each file named is attached, a
    /// file that cannot be is named with why, and anything else is typed
    /// (US3 scenarios 8, 10, 11).
    fn paste(&mut self, pasted: &str) -> Vec<Effect> {
        if let Some(prompt) = self.path_prompt.as_mut() {
            let joined = format!("{}{}", prompt.value(), pasted.trim());
            *prompt = tui_input::Input::default().with_value(joined);
            return vec![Effect::Redraw];
        }
        if self.focus != Focus::Composer {
            return Vec::new();
        }
        let mut effects = Vec::new();
        for item in postio_ui::paste::classify(pasted) {
            match item {
                postio_ui::paste::PasteItem::File(path) => effects.push(Effect::Attach(path)),
                postio_ui::paste::PasteItem::Unreadable { path, reason } => {
                    let name = crate::paths::name_of(&path);
                    effects.extend(self.say(&format!("Could not attach {name}: {reason}")));
                }
                postio_ui::paste::PasteItem::Text(text) => {
                    if let Some(composer) = self.composer.as_mut() {
                        composer.insert(&text);
                        effects.push(Effect::Autosave {
                            generation: composer.generation(),
                            edit: composer.edits(),
                        });
                    }
                }
            }
        }
        effects.push(Effect::Redraw);
        effects
    }

    /// Take up the keys `config.toml` binds now, after it was edited.
    pub fn rekey(&mut self, keys: Keys) {
        self.keys = keys;
    }

    /// The settings, while they are open.
    pub fn settings(&self) -> Option<&crate::settings::Settings> {
        self.settings.as_ref()
    }

    /// The privacy pane's log, once read.
    pub fn privacy(&self) -> Option<&Privacy> {
        self.privacy.as_ref()
    }

    /// The senders allowed remote images, which the privacy pane lists.
    pub fn allowlist(&self) -> &postio_ui::allowlist::RemoteImageAllowList {
        &self.allowlist
    }

    /// The accounts the settings list, in the order the host gives them.
    pub fn accounts(&self) -> &[postio_model::Account] {
        &self.accounts
    }

    /// A key in the settings: the arrows walk the sections, Enter edits
    /// the one under the cursor, Tab goes into the section's list -- the
    /// accounts, where the registry's account commands act on the account
    /// under the cursor, or the senders allowed remote images.
    fn settings_key(&mut self, key: &KeyEvent) -> Vec<Effect> {
        use crossterm::event::KeyCode;
        use postio_ui::settings::Section;
        let Some(settings) = self.settings.as_mut() else {
            return Vec::new();
        };
        if let Some(account) = settings.signatures_of() {
            return self.signatures_key(account, key);
        }
        let count = match settings.current() {
            Section::Privacy => self.allowlist.senders().count(),
            _ => self.accounts.len(),
        };
        if !settings.in_list() {
            let was = settings.current();
            match key.code {
                KeyCode::Up | KeyCode::Char('k') => settings.step(-1),
                KeyCode::Down | KeyCode::Char('j') => settings.step(1),
                KeyCode::Tab => settings.set_in_list(true),
                KeyCode::Enter if matches!(was, Section::Accounts | Section::Privacy) => {
                    settings.set_in_list(true);
                }
                KeyCode::Enter => return vec![Effect::EditConfig(Some(was))],
                _ => {
                    if let Outcome::Command(id) = self.keys.press(key, KeyContext::Global, false) {
                        return self.settings_command(&id);
                    }
                }
            }
            // The privacy log is read as the pane comes up, as the desktop
            // reads it when its panel is shown: what it says is now.
            if settings.current() == Section::Privacy && was != Section::Privacy {
                return vec![Effect::ReadPrivacy, Effect::Redraw];
            }
            return vec![Effect::Redraw];
        }
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => settings.step_row(-1, count),
            KeyCode::Down | KeyCode::Char('j') => settings.step_row(1, count),
            KeyCode::Tab | KeyCode::BackTab => settings.set_in_list(false),
            // The account's signatures, as the desktop's account form
            // drills into them.
            KeyCode::Char('s') if settings.current() == Section::Accounts => {
                let account = self
                    .accounts
                    .get(settings.row(count))
                    .map(|account| account.id);
                settings.show_signatures(account);
            }
            _ => {
                return match self.keys.press(key, KeyContext::Accounts, false) {
                    Outcome::Command(id) => self.settings_command(&id),
                    Outcome::Pending(_) | Outcome::Unhandled => Vec::new(),
                };
            }
        }
        vec![Effect::Redraw]
    }

    /// A command in the settings: the account commands on the account under
    /// the cursor, `undo` for a removal, Escape to leave.
    fn settings_command(&mut self, id: &str) -> Vec<Effect> {
        use postio_client::protocol::AccountOp;
        let Some(settings) = self.settings.as_mut() else {
            return Vec::new();
        };
        if settings.current() == postio_ui::settings::Section::Privacy && settings.in_list() {
            // The one thing to do to an allowed sender is what the desktop's
            // trash button does: ask again. Removing is the account list's
            // remove key here as it is for an account.
            if id == "remove_account" {
                let row = settings.row(self.allowlist.senders().count());
                let sender = self.allowlist.senders().nth(row).map(str::to_owned);
                if let Some(sender) = sender {
                    self.allowlist.revoke(&sender);
                    let mut effects = self.say(&format!(
                        "Remote images from {sender} will be asked for again"
                    ));
                    effects.insert(0, Effect::SaveAllowlist(self.allowlist.clone()));
                    return effects;
                }
                return Vec::new();
            }
            if id != "back" {
                return self.command(id);
            }
        }
        let account = self
            .accounts
            .get(settings.row(self.accounts.len()))
            .cloned();
        let op = match (id, &account) {
            ("back", _) => {
                self.settings = None;
                self.focus = Focus::List;
                return vec![Effect::Redraw];
            }
            ("undo", _) => settings.take_removed().map(AccountOp::Restore),
            ("toggle_account_enabled", Some(account)) => Some(AccountOp::SetEnabled {
                account: account.id,
                enabled: !account.enabled,
            }),
            ("remove_account", Some(account)) => {
                settings.removed(account.id);
                // The key undo has, not a letter typed here: the one keymap
                // moved it (specs/007-postio-focus contracts/keymap.md).
                let sentence = match self.keys.key_for(KeyContext::Accounts, "undo") {
                    Some(key) => format!("{} removed — {key} to undo", account.display_name),
                    None => format!("{} removed", account.display_name),
                };
                let mut effects = self.say(&sentence);
                effects.push(Effect::Account(AccountOp::Remove(account.id)));
                return effects;
            }
            ("set_default_account", Some(account)) => Some(AccountOp::SetDefault(account.id)),
            ("rebuild_account_index", Some(account)) => Some(AccountOp::RebuildIndex(account.id)),
            ("map_mailbox_role", Some(account)) => {
                let account = account.id;
                return self.open_palette(Finding::Roles(account));
            }
            ("update_credential", Some(account)) => {
                self.first_run = Some(crate::first_run::FirstRun::repair(account));
                self.focus = Focus::FirstRun;
                return vec![Effect::Redraw];
            }
            _ => return self.command(id),
        };
        match op {
            Some(op) => vec![Effect::Account(op), Effect::Redraw],
            None => Vec::new(),
        }
    }

    /// The first run, while there is no account yet.
    pub fn first_run(&self) -> Option<&crate::first_run::FirstRun> {
        self.first_run.as_ref()
    }

    /// A key in the first run.
    fn first_run_key(&mut self, key: &KeyEvent) -> Vec<Effect> {
        let Some(first_run) = self.first_run.as_mut() else {
            return Vec::new();
        };
        // Escape goes back to the mail there is, except while a browser
        // sign-in waits: there it gives the sign-in up, one step back.
        if key.code == crossterm::event::KeyCode::Esc
            && first_run.leavable()
            && *first_run.status() != postio_ui::onboarding::Status::WaitingForBrowser
        {
            self.first_run = None;
            self.focus = if self.settings.is_some() {
                Focus::Settings
            } else {
                Focus::List
            };
            return vec![Effect::Redraw];
        }
        match first_run.key(*key) {
            crate::first_run::Asked::Nothing => vec![Effect::Redraw],
            crate::first_run::Asked::Discover(address) => {
                vec![Effect::Discover(address), Effect::Redraw]
            }
            crate::first_run::Asked::Add(submission) => {
                vec![Effect::AddAccount(submission), Effect::Redraw]
            }
            crate::first_run::Asked::BeginOAuth(submission) => {
                vec![Effect::BeginOAuth(submission), Effect::Redraw]
            }
            crate::first_run::Asked::Open(url) => {
                let mut effects = self.say("Opening the sign-in in your browser");
                effects.push(Effect::OpenLink(url));
                effects
            }
            crate::first_run::Asked::Copy(url) => {
                let mut effects = self.say("The sign-in address is on the clipboard");
                effects.push(Effect::CopyText(url));
                effects
            }
            crate::first_run::Asked::CancelOAuth(address) => {
                vec![Effect::CancelOAuth(address), Effect::Redraw]
            }
            crate::first_run::Asked::Start(window) => {
                self.first_run = None;
                self.focus = Focus::List;
                vec![
                    Effect::SaveSyncWindow(window),
                    Effect::RefreshPlaces,
                    Effect::Redraw,
                ]
            }
        }
    }

    /// The same app, listening to the mouse or not (`[tui].mouse`). Off, a
    /// click does nothing at all, and the terminal's own selection works.
    pub fn with_mouse(mut self, mouse: bool) -> App {
        self.mouse = mouse;
        self
    }

    /// Put `reading` in the reader, as opening a message would.
    #[cfg(test)]
    pub(crate) fn set_reading_for_tests(&mut self, reading: crate::conversation::Reading) {
        self.reading = Some(reading);
        self.focus = Focus::Reader;
    }

    /// What is marked.
    pub fn selection(&self) -> &postio_ui::selection::SelectionState {
        &self.selection
    }

    /// What the bulk bar says is selected, when anything is.
    pub fn selection_summary(&self) -> Option<String> {
        self.selection_words.clone()
    }

    /// A mouse event, on what it landed on: the same things the keys do.
    fn pointer(&mut self, pointer: Pointer) -> Vec<Effect> {
        use crate::view::hit::Target;
        // A menu over the message holds the pointer as it holds the keyboard.
        if self.open.menu.is_some()
            && !matches!(pointer, Pointer::Click { hit, .. } if matches!(hit.target, Target::MenuRow(_)))
        {
            return Vec::new();
        }
        if self.sheet.is_some() {
            match pointer {
                Pointer::Click { hit, .. } if hit.target != Target::Command("cheat_sheet") => {
                    return Vec::new();
                }
                Pointer::Wheel { down, .. } => {
                    let (max, _) = self.sheet_limits();
                    if let Some(sheet) = self.sheet.as_mut() {
                        sheet.scroll_by(if down { WHEEL } else { -WHEEL }, max);
                    }
                    return vec![Effect::Redraw];
                }
                Pointer::Click { .. } => {}
            }
        }
        if self.picker.is_some() {
            match pointer {
                Pointer::Click { hit, .. }
                    if !matches!(
                        hit.target,
                        Target::PickRow(_) | Target::Command("picker_type_date")
                    ) =>
                {
                    return Vec::new();
                }
                Pointer::Wheel { down, .. } => {
                    if let Some(picker) = self.picker.as_mut() {
                        picker.wheel(down);
                    }
                    return vec![Effect::Redraw];
                }
                Pointer::Click { .. } => {}
            }
        }
        if self.places_box.is_some() {
            match pointer {
                Pointer::Click { hit, .. } if !matches!(hit.target, Target::PlaceRow(_)) => {
                    return Vec::new();
                }
                Pointer::Wheel { down, .. } => {
                    self.with_folders(|folders, _, reach| folders.wheel(down, reach));
                    return vec![Effect::Redraw];
                }
                Pointer::Click { .. } => {}
            }
        }
        // The bar holds the pointer as it holds the keyboard: a click outside
        // it does nothing, and the wheel scrolls its lines.
        if self.bar.is_some() {
            match pointer {
                Pointer::Click { hit, .. }
                    if !matches!(
                        hit.target,
                        Target::BarRow(_)
                            | Target::BarSaved(_)
                            | Target::BarChip(_)
                            | Target::BarRun
                            | Target::BarCommands
                            | Target::Command("save_search" | "back")
                    ) =>
                {
                    return Vec::new();
                }
                Pointer::Wheel { down, .. } => {
                    self.with_bar(|bar, _, ctx| bar.wheel(down, ctx));
                    return vec![Effect::Redraw];
                }
                Pointer::Click { .. } => {}
            }
        }
        match pointer {
            Pointer::Click { hit, ctrl, shift } => match hit.target {
                Target::Row(position) => {
                    // Beside the list a click is the pointer's `j`: the
                    // message stays open and follows.
                    let beside = self.focus == Focus::Reader && self.pane().is_some();
                    if !beside {
                        self.focus = Focus::List;
                    }
                    let mut effects = if ctrl || shift {
                        self.tell(postio_focus::Input::Pick {
                            position,
                            range: shift && !ctrl,
                        })
                    } else {
                        self.move_to(position);
                        self.tell(postio_focus::Input::Point(position))
                    };
                    effects.push(Effect::Redraw);
                    effects
                }
                Target::BarRow(index) => self.bar_click(index),
                Target::BarSaved(index) => self.bar_saved(index),
                Target::BarRun => match self.with_bar(|bar, _, ctx| bar.enter(ctx)) {
                    Some(step) => self.bar_step(step),
                    None => Vec::new(),
                },
                Target::BarCommands => {
                    match self.bar.as_mut().map(crate::bar::Bar::commands_only) {
                        Some(step) => self.bar_step(step),
                        None => Vec::new(),
                    }
                }
                Target::BarChip(index) => self.bar_chip(index),
                Target::PlaceRow(index) => self.folders_click(index),
                Target::PickRow(index) => self.picker_click(index),
                Target::Reader(line) => {
                    if self.reading.is_none() {
                        return Vec::new();
                    }
                    self.focus = Focus::Reader;
                    match line {
                        Some(line) => self.click_reader(line),
                        None => vec![Effect::Redraw],
                    }
                }
                Target::ComposerBody => {
                    if let Some(composer) = self.composer.as_mut() {
                        self.focus = Focus::Composer;
                        composer.click_body(hit.row, hit.column);
                    }
                    vec![Effect::Redraw]
                }
                Target::ComposerField(field) => {
                    if let Some(composer) = self.composer.as_mut() {
                        self.focus = Focus::Composer;
                        composer.focus_field(field);
                    }
                    vec![Effect::Redraw]
                }
                Target::Overlay => Vec::new(),
                Target::Surface(part, index) => self.surface_click(part, index),
                // A row of the menu over the message is chosen.
                Target::MenuRow(at) => self.choose(at),
                // The notice's one button (#1952).
                Target::NoticeOffer => self.take_up_offer(),
                // A row's drawn answer is its key, for that row: an answer
                // to an invitation is about the invitation wherever the
                // cursor is; the rest take the cursor there first.
                Target::RowAction(position, id) => {
                    if self.row_at(position).is_none() {
                        return Vec::new();
                    }
                    self.focus = Focus::List;
                    self.move_to(position);
                    let mut effects = self.tell(postio_focus::Input::Point(position));
                    effects.extend(self.command(id));
                    effects
                }
                Target::Command("picker_type_date") if self.picker.is_some() => {
                    let step = self
                        .picker
                        .as_mut()
                        .map(|picker| picker.command(postio_core::CommandId::PickerTypeDate));
                    step.map_or_else(Vec::new, |step| self.picker_step(step))
                }
                // The bar's own save and close are its keys'.
                Target::Command("save_search") if self.bar.is_some() => self.bar_save(),
                Target::Command("back") if self.bar.is_some() => self.close_bar(),
                // A control is its command, the same as its key.
                Target::Command(id) => self.command(id),
                // A button is its command, the same as its key.
                Target::ComposerAction(id) => {
                    if self.composer.is_none() {
                        return Vec::new();
                    }
                    self.focus = Focus::Composer;
                    self.composer_command(id)
                }
            },
            Pointer::Wheel { hit, down } => {
                let lines: isize = if down { WHEEL } else { -WHEEL };
                match hit.target {
                    Target::Row(_) => self.scroll_list(lines),
                    Target::Reader(_) => self.scroll_open_lines(lines),
                    Target::ComposerBody => match self.composer.as_mut() {
                        Some(composer) => composer.scroll(lines),
                        None => return Vec::new(),
                    },
                    Target::Surface(..) | Target::Overlay if self.surface_wheel(lines) => {}
                    _ => return Vec::new(),
                }
                vec![Effect::Redraw]
            }
        }
    }

    /// A click on line `line` of what is being read: what the keys do there.
    fn click_reader(&mut self, line: usize) -> Vec<Effect> {
        use crate::conversation::At;
        let at = self.line_at(line);
        let Some(reading) = self.reading.as_mut() else {
            return Vec::new();
        };
        if !matches!(at, At::Link(_)) {
            self.armed_link = None;
        }
        match at {
            At::Nothing => vec![Effect::Redraw],
            At::Fold { member, block } => {
                reading.toggle_fold(member, block);
                vec![Effect::Redraw]
            }
            // Nothing leaves this machine on one click: the first shows the
            // whole destination, and only a second on the same link opens it.
            At::Link(target) => {
                if self.armed_link.as_deref() == Some(target.as_str()) {
                    self.armed_link = None;
                    let mut effects = self.say(&format!("Opening {target}"));
                    effects.push(Effect::OpenLink(target));
                    effects
                } else {
                    let effects = self.say(&format!("{target} — click again to open it"));
                    self.armed_link = Some(target);
                    effects
                }
            }
            // A click on an attachment's line opens it with the system's
            // opener.
            At::Part { member, part } => {
                let Some(member) = reading.members.get(member) else {
                    return Vec::new();
                };
                let Some(attachment) = member.attachments().get(part).map(|part| part.id) else {
                    return Vec::new();
                };
                vec![Effect::SavePart {
                    message: member.id,
                    attachment,
                    to: None,
                }]
            }
        }
    }

    /// Scroll the list by `lines`, keeping the cursor on a row in view.
    fn scroll_list(&mut self, lines: isize) {
        let last_top = self.top_for_bottom(self.list.total().saturating_sub(1));
        let top = i64::from(self.top) + lines as i64;
        self.top = u32::try_from(top.clamp(0, i64::from(last_top))).unwrap_or(0);
        let shown = self.fit_from(self.top).max(1);
        self.cursor = self.cursor.clamp(self.top, self.top + shown - 1);
    }

    /// The same app, knowing the terminal delivers every chord.
    pub fn with_enhanced_keys(mut self, enhanced: bool) -> App {
        self.enhanced_keys = enhanced;
        self
    }

    /// Open the palette in one of its modes, from wherever the keyboard is.
    fn open_palette(&mut self, finding: Finding) -> Vec<Effect> {
        let from = match self.focus {
            Focus::Palette => Focus::List,
            other => other,
        };
        self.palette = Some(PaletteState {
            input: tui_input::Input::default(),
            selected: 0,
            from,
            finding,
        });
        self.focus = Focus::Palette;
        vec![Effect::Redraw]
    }

    // -- The command bar (src/bar.rs) ------------------------------------

    /// The bar and what it needs to draw, while it is open.
    pub fn bar(&self) -> Option<(&crate::bar::Bar, crate::bar::Ctx<'_>)> {
        let bar = self.bar.as_ref()?;
        Some((bar, self.bar_ctx(bar.from(), self.keys.keymap())))
    }

    /// What the bar needs of the app, for a bar opened over `from`.
    fn bar_ctx<'a>(&self, from: Focus, keymap: &'a postio_core::Keymap) -> crate::bar::Ctx<'a> {
        crate::bar::Ctx {
            keymap,
            context: Self::context_of(from),
            state: self.availability(),
            enhanced: self.enhanced_keys,
        }
    }

    /// What is typed in the bar, while it is open.
    pub fn bar_typed(&self) -> Option<&str> {
        self.bar.as_ref().map(crate::bar::Bar::typed)
    }

    /// Open the bar holding `typed`, over where the keyboard is.
    fn open_bar(&mut self, typed: &str) -> Vec<Effect> {
        let from = match (&self.bar, self.focus) {
            (Some(bar), _) => bar.from(),
            (None, Focus::Palette) => Focus::List,
            (None, focus) => focus,
        };
        let folders: Vec<&postio_model::Mailbox> = self
            .folders
            .iter()
            .filter(|folder| folder.selectable)
            .collect();
        let sources = crate::bar::Sources {
            places: folders
                .iter()
                .map(|folder| postio_ui::places::mailbox_place(folder))
                .collect(),
            folders: folders
                .iter()
                .map(|folder| (folder.id, postio_ui::places::place_name(folder)))
                .collect(),
            saved: self
                .places
                .saved
                .iter()
                .map(|saved| (saved.name.clone(), saved.query.clone()))
                .collect(),
            digesting: self.features.digest_rules > 0,
        };
        let (bar, step) = crate::bar::Bar::open(from, sources, typed);
        self.bar = Some(bar);
        self.focus = Focus::Bar;
        let mut effects = self.bar_step(step);
        effects.push(Effect::ReadPlaceDetails);
        effects
    }

    /// Run `f` on the bar with the keys and what it needs.
    fn with_bar<T>(
        &mut self,
        f: impl FnOnce(&mut crate::bar::Bar, &mut Keys, &crate::bar::Ctx<'_>) -> T,
    ) -> Option<T> {
        let mut bar = self.bar.take()?;
        let keymap = self.keys.keymap().clone();
        let ctx = self.bar_ctx(bar.from(), &keymap);
        let out = f(&mut bar, &mut self.keys, &ctx);
        self.bar = Some(bar);
        Some(out)
    }

    fn bar_key(&mut self, key: &KeyEvent) -> Vec<Effect> {
        match self.with_bar(|bar, keys, ctx| bar.key(key, keys, ctx)) {
            Some(step) => self.bar_step(step),
            None => Vec::new(),
        }
    }

    fn bar_click(&mut self, index: usize) -> Vec<Effect> {
        match self.with_bar(|bar, _, ctx| bar.click(index, ctx)) {
            Some(step) => self.bar_step(step),
            None => Vec::new(),
        }
    }

    fn bar_saved(&mut self, index: usize) -> Vec<Effect> {
        match self.bar.as_mut().map(|bar| bar.open_saved(index)) {
            Some(step) => self.bar_step(step),
            None => Vec::new(),
        }
    }

    fn bar_save(&mut self) -> Vec<Effect> {
        match self.bar.as_ref().map(crate::bar::Bar::save) {
            Some(step) => self.bar_step(step),
            None => Vec::new(),
        }
    }

    fn bar_chip(&mut self, index: usize) -> Vec<Effect> {
        match self.bar.as_mut().map(|bar| bar.edit_chip(index)) {
            Some(step) => self.bar_step(step),
            None => Vec::new(),
        }
    }

    /// What a key or a click in the bar asks for.
    fn bar_step(&mut self, step: crate::bar::Step) -> Vec<Effect> {
        use crate::bar::Step;
        match step {
            Step::Stay => vec![Effect::Redraw],
            Step::Close => self.close_bar(),
            Step::Search(ask) => vec![Effect::BarSearch(ask), Effect::Redraw],
            Step::Folder(mailbox, sequence) => {
                vec![Effect::BarFolder { sequence, mailbox }, Effect::Redraw]
            }
            Step::Save(query) => {
                let mut effects = self.say(&postio_ui::focus_target::search_saved(&query));
                effects.insert(0, Effect::SaveSearch(query));
                effects
            }
            Step::Act(action) => self.bar_act(action),
        }
    }

    fn close_bar(&mut self) -> Vec<Effect> {
        if let Some(bar) = self.bar.take() {
            self.focus = bar.from();
        }
        vec![Effect::Redraw]
    }

    /// Close the bar and do what its row asked, where it opened.
    fn bar_act(&mut self, action: postio_ui::command_bar::BarAction) -> Vec<Effect> {
        use postio_ui::command_bar::BarAction;
        let Some(bar) = self.bar.take() else {
            return Vec::new();
        };
        let from = bar.from();
        self.focus = from;
        match action {
            BarAction::Open { message, .. } => self.open_found(message, bar.result_of(message)),
            BarAction::Command(id) => {
                if postio_ui::command_bar::ACCOUNT_VERBS.contains(&id) && from != Focus::Settings {
                    // Account verbs act on the account row Settings has.
                    self.settings = Some(crate::settings::Settings::default());
                    self.focus = Focus::Settings;
                    return self.say("Pick an account in Settings, then run it again");
                }
                match from {
                    Focus::Composer => self.composer_command(id.as_str()),
                    Focus::Settings => self.settings_command(id.as_str()),
                    _ => self.command(id.as_str()),
                }
            }
            BarAction::Go { destination, name } => self.go_destination(destination, &name),
        }
    }

    /// Show `destination` in the list: a label is its search, as Focus lists
    /// no label on its own.
    fn go_destination(
        &mut self,
        destination: postio_ui::finder::Destination,
        name: &str,
    ) -> Vec<Effect> {
        use postio_ui::finder::Destination;
        match destination {
            Destination::Mailbox(mailbox) => {
                let inbox = self.folders.iter().any(|folder| {
                    folder.id == mailbox && folder.role == postio_model::mailbox::MailboxRole::Inbox
                });
                self.open_scope(if inbox {
                    ListScope::Focus(postio_model::FocusScope::Inbox)
                } else {
                    ListScope::Mailbox(mailbox)
                })
            }
            Destination::Label(_) => self.open_bar(&format!("label:\"{name}\"")),
            Destination::Search(query) => self.open_bar(&query),
            Destination::Outbox(account) => self.open_scope(ListScope::Outbox(account)),
        }
    }

    /// Open a message a search found, as a row opens it: the one in the list
    /// when it is there, otherwise on its own.
    fn open_found(
        &mut self,
        message: postio_model::MessageId,
        found: Option<&crate::bar::ResultRow>,
    ) -> Vec<Effect> {
        // The message itself, wherever it is: the list's row when this list
        // has one, else what the bar says of it. Its verbs aim at it.
        let row = self
            .list
            .row_of(message)
            .map(|held| held.row.clone())
            .or_else(|| {
                found.map(|found| Row {
                    id: message,
                    thread: None,
                    is_thread: false,
                    kind: crate::row::Kind::Message,
                    from: found.sender.clone(),
                    address: None,
                    subject: found.subject.clone(),
                    preview: found.snippet.clone(),
                    when: found.at,
                    unread: false,
                    attachment: false,
                    count: 1,
                    marker: None,
                    labels: Vec::new(),
                    send_state: None,
                })
            });
        let Some(row) = row else {
            return Vec::new();
        };
        let back = match self.focus {
            Focus::Filtered => Focus::Filtered,
            _ => Focus::List,
        };
        self.open_for_itself(row, back)
    }

    fn bar_found(
        &mut self,
        sequence: u64,
        found: Result<Option<postio_client::protocol::Hits>, String>,
        held: &[(postio_model::MessageId, String, bool)],
    ) -> Vec<Effect> {
        if let Ok(Some(postio_client::protocol::Hits(results))) = found {
            self.with_bar(|bar, _, ctx| bar.found(ctx, sequence, results, held));
        }
        vec![Effect::Redraw]
    }

    fn bar_listed(
        &mut self,
        sequence: u64,
        count: u32,
        rows: Vec<crate::bar::ResultRow>,
    ) -> Vec<Effect> {
        self.with_bar(|bar, _, ctx| bar.listed(ctx, sequence, count, rows));
        vec![Effect::Redraw]
    }

    /// The labels and correspondents arrived: labels are places, and a typed
    /// name is looked up among the correspondents.
    fn place_details(&mut self, details: crate::places::PlaceDetails) -> Vec<Effect> {
        let labels = details
            .labels
            .iter()
            .map(postio_ui::places::label_place)
            .collect();
        let names = postio_ui::names::Names::new(&details.correspondents);
        if let Some(folders) = self.places_box.as_mut() {
            folders.learn(details.clone());
        }
        let step = self.bar.as_mut().map(|bar| {
            bar.learn(labels, names);
            bar.places_known()
        });
        match step {
            Some(step) => self.bar_step(step),
            None => Vec::new(),
        }
    }

    // -- The pickers (src/pickers.rs) --------------------------------------

    /// The picker and its keymap, while one is open.
    pub fn picker(&self) -> Option<(&crate::pickers::Picker, &postio_core::Keymap)> {
        Some((self.picker.as_ref()?, self.keys.keymap()))
    }

    /// The rows a verb acts on: the message open for itself, else those
    /// selected, or the cursor's.
    fn aimed_rows(&self) -> Vec<&Row> {
        if let Some(own) = self
            .reading
            .as_ref()
            .and_then(|reading| reading.own.as_ref())
        {
            return vec![&own.row];
        }
        let picked = match self.selection.selection() {
            postio_core::Selection::These(picked) => picked,
            postio_core::Selection::Everything { .. } => return Vec::new(),
        };
        let rows: Vec<&Row> = picked
            .iter()
            .filter_map(|message| self.row(*message))
            .collect();
        if rows.is_empty() {
            self.cursor_message()
                .and_then(|message| self.row(message))
                .into_iter()
                .collect()
        } else {
            rows
        }
    }

    /// What a picker names as its target: the conversation under the cursor,
    /// or how many are selected.
    fn picker_target(&self) -> Option<String> {
        if let postio_core::Selection::Everything { .. } = self.selection.selection() {
            return Some(postio_ui::pickers::EVERY_THREAD.to_owned());
        }
        let rows = self.aimed_rows();
        let first = rows.first()?;
        Some(postio_ui::pickers::target(
            rows.len(),
            first.from.as_str(),
            first.subject.as_str(),
        ))
    }

    /// `b`, `h`, `l` or `v`: the picker, over the row the keyboard is on.
    fn open_picker(&mut self, kind: crate::pickers::Kind) -> Vec<Effect> {
        use crate::pickers::{Kind, Picker};
        let Some(target) = self.picker_target() else {
            return Vec::new();
        };
        let from = match self.focus {
            Focus::Picker => Focus::List,
            focus => focus,
        };
        let now = chrono::Local::now();
        let mut effects = vec![Effect::Redraw];
        let picker = match kind {
            Kind::Snooze | Kind::Remind => Picker::when(kind, &target, from, now),
            Kind::Label => {
                let Some(account) = self.account_here() else {
                    return Vec::new();
                };
                let rows = self.aimed_rows();
                let threads = rows.iter().filter_map(|row| row.thread).collect();
                let message = rows
                    .first()
                    .map_or(postio_model::MessageId::new(0), |row| row.id);
                effects.push(Effect::ReadLabelPicker {
                    message,
                    account,
                    threads,
                });
                Picker::labelling(&target, from, account)
            }
            Kind::Move => {
                let enabled: Vec<postio_model::mailbox::Mailbox> = self
                    .folders
                    .iter()
                    .filter(|folder| {
                        self.accounts
                            .iter()
                            .any(|account| account.enabled && account.id == folder.account_id)
                    })
                    .cloned()
                    .collect();
                effects.push(Effect::ReadRecentMoves);
                Picker::moving(&target, from, enabled)
            }
        };
        self.picker = Some(picker);
        self.focus = Focus::Picker;
        effects
    }

    /// Open the remind picker for the draft being written, over the
    /// composer. What it chooses goes on the draft as `remind_at`, and is
    /// sent with it.
    pub fn open_remind_picker_for_draft(&mut self, target: &str) -> Vec<Effect> {
        let from = match self.focus {
            Focus::Picker => Focus::Composer,
            focus => focus,
        };
        self.picker = Some(crate::pickers::Picker::for_draft(
            target,
            from,
            chrono::Local::now(),
        ));
        self.focus = Focus::Picker;
        vec![Effect::Redraw]
    }

    fn picker_key(&mut self, key: &KeyEvent) -> Vec<Effect> {
        let Some(mut picker) = self.picker.take() else {
            return Vec::new();
        };
        let step = picker.key(key, &mut self.keys);
        self.picker = Some(picker);
        self.picker_step(step)
    }

    fn picker_click(&mut self, index: usize) -> Vec<Effect> {
        let step = self.picker.as_mut().map(|picker| picker.choose(index));
        match step {
            Some(step) => self.picker_step(step),
            None => Vec::new(),
        }
    }

    fn picker_step(&mut self, step: crate::pickers::Step) -> Vec<Effect> {
        use crate::pickers::Step;
        match step {
            Step::Stay => vec![Effect::Redraw],
            Step::Close => self.close_picker(),
            Step::Keep(pick) => self.picker_pick(pick, false),
            Step::Choose(pick) => self.picker_pick(pick, true),
        }
    }

    fn close_picker(&mut self) -> Vec<Effect> {
        if let Some(picker) = self.picker.take() {
            self.focus = picker.from();
        }
        vec![Effect::Redraw]
    }

    /// What a choice sends, aimed where the picker opened.
    fn picker_pick(&mut self, pick: crate::pickers::Pick, close: bool) -> Vec<Effect> {
        use crate::pickers::{Kind, Pick};
        use postio_core::{Command, CommandId};
        let Some(picker) = self.picker.as_ref() else {
            return Vec::new();
        };
        let (kind, draft, account) = (picker.kind(), picker.is_for_draft(), picker.account());
        if close {
            self.close_picker();
        }
        match pick {
            Pick::When(at) if draft => match self.composer.as_mut() {
                Some(composer) => {
                    composer.set_remind_at(Some(at.with_timezone(&chrono::Utc)));
                    vec![
                        Effect::Autosave {
                            generation: composer.generation(),
                            edit: composer.edits(),
                        },
                        Effect::Redraw,
                    ]
                }
                None => vec![Effect::Redraw],
            },
            Pick::When(at) => {
                let at = at.with_timezone(&chrono::Utc);
                let (id, set): (CommandId, fn(&mut Command, chrono::DateTime<chrono::Utc>)) =
                    if kind == Kind::Snooze {
                        (CommandId::Snooze, |command, at| {
                            if let Command::Snooze { until, .. } = command {
                                *until = Some(at);
                            }
                        })
                    } else {
                        (CommandId::RemindIfNoReply, |command, when| {
                            if let Command::RemindIfNoReply { at, .. } = command {
                                *at = Some(when);
                            }
                        })
                    };
                self.send_answered(id, |command| set(command, at))
            }
            Pick::Label { label, on } => self.send_answered(CommandId::AddLabel, |command| {
                if let Command::AddLabel {
                    label: chosen,
                    on: state,
                    ..
                } = command
                {
                    *chosen = Some(label);
                    *state = Some(on);
                }
            }),
            Pick::Create { name, close } => match account {
                Some(account) => vec![Effect::CreateLabel {
                    account,
                    name,
                    close,
                }],
                None => Vec::new(),
            },
            Pick::Move(to) => {
                let mut effects = self.send_answered(CommandId::Move, |command| {
                    if let Command::Move { to: chosen, .. } = command {
                        *chosen = Some(to);
                    }
                });
                effects.push(Effect::NoteMove(to));
                effects
            }
        }
    }

    /// A label was made for the picker: it is put on what the picker acts on.
    fn label_made(&mut self, label: Option<postio_model::Label>, close: bool) -> Vec<Effect> {
        let Some(mut label) = label else {
            return self.say("The label could not be made");
        };
        label.name = postio_ui::terminal::SafeText::new(&label.name)
            .as_str()
            .to_owned();
        let id = label.id;
        if let Some(picker) = self.picker.as_mut() {
            picker.made(label, close);
        }
        if close {
            self.close_picker();
        }
        self.send_answered(postio_core::CommandId::AddLabel, |command| {
            if let postio_core::Command::AddLabel { label, on, .. } = command {
                *label = Some(id);
                *on = Some(true);
            }
        })
    }

    // -- The folders popover (src/folders.rs) -----------------------------

    /// The popover and what it lists from, while it is open.
    pub fn folders(&self) -> Option<(&crate::folders::Folders, crate::folders::Reach<'_>)> {
        Some((self.places_box.as_ref()?, self.folders_reach()))
    }

    /// Whether filtering is on, and how many were filtered today when the
    /// count is in: what Settings' Filtering section says.
    pub fn filtering(&self) -> (bool, Option<u32>) {
        (
            self.features.filtering,
            self.counts.map(|counts| counts.filtered_today),
        )
    }

    fn folders_reach(&self) -> crate::folders::Reach<'_> {
        crate::folders::Reach {
            folders: &self.folders,
            filtered_today: self
                .features
                .filtering
                .then(|| self.counts.map_or(0, |counts| counts.filtered_today)),
        }
    }

    /// `g l`, or a click on the strip's place.
    fn open_folders(&mut self) -> Vec<Effect> {
        let from = match (&self.places_box, self.focus) {
            (Some(open), _) => open.from(),
            (None, Focus::Palette) => Focus::List,
            (None, focus) => focus,
        };
        self.places_box = Some(crate::folders::Folders::open(from));
        self.focus = Focus::Folders;
        vec![Effect::ReadPlaceDetails, Effect::Redraw]
    }

    fn with_folders<T>(
        &mut self,
        f: impl FnOnce(&mut crate::folders::Folders, &mut Keys, &crate::folders::Reach<'_>) -> T,
    ) -> Option<T> {
        let mut open = self.places_box.take()?;
        let reach = crate::folders::Reach {
            folders: &self.folders,
            filtered_today: self
                .features
                .filtering
                .then(|| self.counts.map_or(0, |counts| counts.filtered_today)),
        };
        let out = f(&mut open, &mut self.keys, &reach);
        self.places_box = Some(open);
        Some(out)
    }

    fn folders_key(&mut self, key: &KeyEvent) -> Vec<Effect> {
        match self.with_folders(|folders, keys, reach| folders.key(key, keys, reach)) {
            Some(step) => self.folders_step(step),
            None => Vec::new(),
        }
    }

    fn folders_click(&mut self, index: usize) -> Vec<Effect> {
        let step = self
            .places_box
            .as_ref()
            .map(|open| open.go(index, &self.folders_reach()));
        match step {
            Some(step) => self.folders_step(step),
            None => Vec::new(),
        }
    }

    fn folders_step(&mut self, step: crate::folders::Step) -> Vec<Effect> {
        use crate::folders::Step;
        match step {
            Step::Stay => vec![Effect::Redraw],
            Step::Close => self.close_folders(),
            Step::Go(destination, name) => {
                self.close_folders();
                self.go_destination(destination, &name)
            }
            Step::Run(command) => {
                self.close_folders();
                self.command(command.as_str())
            }
        }
    }

    fn close_folders(&mut self) -> Vec<Effect> {
        if let Some(open) = self.places_box.take() {
            self.focus = open.from();
        }
        vec![Effect::Redraw]
    }

    /// The registry's context for where the keyboard is.
    fn context_of(focus: Focus) -> postio_core::Context {
        match focus {
            Focus::List | Focus::Palette | Focus::FirstRun => postio_core::Context::List,
            Focus::Settings => postio_core::Context::Accounts,
            Focus::Bar | Focus::Folders => postio_core::Context::Search,
            Focus::Picker => postio_core::Context::Picker,
            Focus::Keys => postio_core::Context::List,
            Focus::Reader => postio_core::Context::Reader,
            Focus::Composer => postio_core::Context::Composer,
            Focus::Filtered => postio_core::Context::Filtered,
            Focus::Digest => postio_core::Context::Digest,
            Focus::Rules => postio_core::Context::Filtered,
            Focus::RuleDialog => postio_core::Context::Picker,
            Focus::Capture => postio_core::Context::Capture,
        }
    }

    /// What can run here, given what is on screen.
    fn availability(&self) -> postio_core::Availability {
        postio_core::Availability {
            // The composer's `$EDITOR` and preview are this frontend's own.
            frontend: postio_core::Frontend::Terminal,
            ..postio_core::Availability::open(
                self.account
                    .map_or(postio_core::Scope::Unified, postio_core::Scope::Account),
            )
        }
    }

    // -- The key map (src/sheet.rs) ----------------------------------------

    /// The key map and its columns, while it is open.
    pub fn key_map(&self) -> Option<(&crate::sheet::Sheet, Vec<Vec<crate::sheet::Line>>)> {
        let sheet = self.sheet.as_ref()?;
        let width = crate::view::sheet::body(self.screen()).width;
        Some((
            sheet,
            crate::sheet::columns(self.keys.keymap(), self.enhanced_keys, width),
        ))
    }

    fn screen(&self) -> ratatui::layout::Rect {
        ratatui::layout::Rect::new(0, 0, self.size.0, self.size.1)
    }

    /// How far the key map can scroll, and how far a page is.
    fn sheet_limits(&self) -> (usize, usize) {
        let body = crate::view::sheet::body(self.screen());
        let columns = crate::sheet::columns(self.keys.keymap(), self.enhanced_keys, body.width);
        let page = usize::from(body.height);
        (crate::sheet::height(&columns).saturating_sub(page), page)
    }

    pub(crate) fn open_keys(&mut self) -> Vec<Effect> {
        let from = match (&self.sheet, self.focus) {
            (Some(open), _) => open.from(),
            (None, focus) => focus,
        };
        self.sheet = Some(crate::sheet::Sheet::open(from));
        self.focus = Focus::Keys;
        vec![Effect::Redraw]
    }

    fn keys_key(&mut self, key: &KeyEvent) -> Vec<Effect> {
        let (max, page) = self.sheet_limits();
        let Some(sheet) = self.sheet.as_mut() else {
            return Vec::new();
        };
        match sheet.key(key, &mut self.keys, page, max) {
            crate::sheet::Step::Stay => vec![Effect::Redraw],
            crate::sheet::Step::Close => self.close_keys(),
        }
    }

    fn close_keys(&mut self) -> Vec<Effect> {
        if let Some(sheet) = self.sheet.take() {
            self.focus = sheet.from();
        }
        vec![Effect::Redraw]
    }

    /// The palette as it is drawn, while it is open.
    pub fn palette(&self) -> Option<PaletteView> {
        let state = self.palette.as_ref()?;
        let query = state.input.value().to_owned();
        let rows = self
            .palette_rows(state)
            .into_iter()
            .map(|(row, _)| row)
            .collect();
        Some(PaletteView {
            marker: match state.finding {
                Finding::Roles(_) => "Role",
                Finding::RoleFolder(..) => "Folder",
                Finding::Rename => "Name",
            },
            query,
            rows,
            selected: state.selected,
        })
    }

    /// The rows for what is typed, each with what choosing it does.
    fn palette_rows(&self, state: &PaletteState) -> Vec<(PaletteRow, PaletteAction)> {
        let query = state.input.value();
        match state.finding {
            // ADR 0035's role map: the desktop's roles in its order, then the
            // account's folders with "Automatic" first, as its picker lists
            // them.
            Finding::Roles(account) => {
                let offered = postio_ui::settings::MAPPABLE_ROLES
                    .iter()
                    .map(|(role, title)| {
                        ((*title).to_owned(), PaletteAction::PickRole(account, *role))
                    });
                best_first(query, offered)
            }
            Finding::Rename => {
                let name = query.trim();
                let shown = postio_ui::terminal::SafeText::new(name);
                let title = match (&self.renaming, name.is_empty()) {
                    (Some(Renaming::Signature { .. }), true) => {
                        "A signature needs a name".to_owned()
                    }
                    (
                        Some(Renaming::Signature {
                            signature: None, ..
                        }),
                        false,
                    ) => {
                        format!("Call it “{shown}”, and write it")
                    }
                    (_, true) => "Go back to its first name".to_owned(),
                    (_, false) => format!("Rename to “{shown}”"),
                };
                vec![(
                    PaletteRow {
                        title,
                        chord: None,
                        positions: Vec::new(),
                    },
                    PaletteAction::Rename(name.to_owned()),
                )]
            }
            Finding::RoleFolder(account, role) => {
                let offered = std::iter::once((
                    "Automatic".to_owned(),
                    PaletteAction::MapRole(account, role, None),
                ))
                .chain(
                    self.folders
                        .iter()
                        .filter(|folder| folder.account_id == account && folder.selectable)
                        .map(|folder| {
                            (
                                postio_ui::terminal::SafeText::new(&folder.path).to_string(),
                                PaletteAction::MapRole(account, role, Some(folder.path.clone())),
                            )
                        }),
                );
                best_first(query, offered)
            }
        }
    }

    /// A key in the palette: typed into its query, the arrows choose, Enter
    /// runs the chosen row where the palette was opened, Escape closes.
    fn palette_key(&mut self, key: &KeyEvent) -> Vec<Effect> {
        use crossterm::event::KeyCode;
        use tui_input::backend::crossterm::EventHandler;
        let Some(state) = self.palette.as_mut() else {
            return Vec::new();
        };
        let from = state.from;
        match key.code {
            KeyCode::Esc => {
                self.palette = None;
                self.focus = from;
            }
            KeyCode::Down => state.selected = state.selected.saturating_add(1),
            KeyCode::Up => state.selected = state.selected.saturating_sub(1),
            KeyCode::Enter => {
                let chosen = self.palette.take().and_then(|state| {
                    let mut rows = self.palette_rows(&state);
                    let index = state.selected.min(rows.len().saturating_sub(1));
                    (!rows.is_empty()).then(|| rows.swap_remove(index).1)
                });
                self.focus = from;
                return match chosen {
                    Some(PaletteAction::Rename(name)) => self.named(name),
                    Some(PaletteAction::PickRole(account, role)) => {
                        self.open_palette(Finding::RoleFolder(account, role))
                    }
                    Some(PaletteAction::MapRole(account, role, path)) => vec![
                        Effect::Send(postio_core::Command::MapMailboxRole {
                            account: Some(account),
                            role: Some(role),
                            path,
                        }),
                        Effect::Redraw,
                    ],
                    None => vec![Effect::Redraw],
                };
            }
            _ => {
                if state
                    .input
                    .handle_event(&crossterm::event::Event::Key(*key))
                    .is_some()
                {
                    state.selected = 0;
                }
            }
        }
        if let Some(state) = self.palette.as_ref() {
            let count = self.palette_rows(state).len();
            if let Some(state) = self.palette.as_mut() {
                state.selected = state.selected.min(count.saturating_sub(1));
            }
        }
        vec![Effect::Redraw]
    }

    /// The schedule-send picker's times, while it is open.
    pub fn scheduling(&self) -> Option<&[(&'static str, chrono::DateTime<chrono::Local>)]> {
        self.scheduling.as_ref().map(|times| times.as_slice())
    }

    /// A key while the schedule picker is open: a number picks, Escape
    /// goes back to writing.
    fn schedule_key(&mut self, key: &KeyEvent) -> Vec<Effect> {
        use crossterm::event::KeyCode;
        let Some(times) = self.scheduling else {
            return Vec::new();
        };
        match key.code {
            KeyCode::Esc => {
                self.scheduling = None;
                vec![Effect::Redraw]
            }
            KeyCode::Char(digit @ '1'..='4') => {
                let index = usize::from(digit as u8 - b'1');
                self.scheduling = None;
                self.send_draft(Some(times[index].1.with_timezone(&chrono::Utc)))
            }
            _ => Vec::new(),
        }
    }

    /// Send what is being written, now or at `at`, unless something stops
    /// it: nobody to send to, or a question to ask first (FR-018, FR-057),
    /// asked once and answered by sending again.
    fn send_draft(&mut self, at: Option<chrono::DateTime<chrono::Utc>>) -> Vec<Effect> {
        let Some(composer) = self.composer.as_ref() else {
            return Vec::new();
        };
        let draft = composer.draft();
        if !draft.is_sendable() {
            return self.say(if draft.has_recipients() {
                postio_ui::sending::ALREADY_QUEUED
            } else {
                postio_ui::sending::NO_RECIPIENTS
            });
        }
        let concerns = postio_ui::sending::send_concerns(&draft);
        if !concerns.is_empty() && self.asked_at != Some(composer.edits()) {
            self.asked_at = Some(composer.edits());
            let key = self
                .keys
                .key_for(KeyContext::Composer, "send")
                .unwrap_or_else(|| "send".to_owned());
            let question = postio_ui::sending::join_with_and(&concerns);
            return self.say(&format!(
                "Send it anyway? {question}. {key} again sends it."
            ));
        }
        let generation = composer.generation();
        // The send is the write: the composer closes with nothing to save or
        // discard, and the draft is the queue's now.
        self.composer = None;
        self.asked_at = None;
        self.focus = self.after_composing();
        vec![
            Effect::QueueSend {
                generation,
                draft: Box::new(draft),
                at,
            },
            Effect::Redraw,
        ]
    }

    /// A command run with the composer in front.
    fn composer_command(&mut self, id: &str) -> Vec<Effect> {
        match id {
            "send" => self.send_draft(None),
            "toggle_preview" => {
                self.previewing = !self.previewing;
                vec![Effect::Redraw]
            }
            "edit_externally" => match &self.composer {
                Some(composer) => vec![Effect::EditExternally {
                    generation: composer.generation(),
                    markdown: composer.markdown(),
                }],
                None => Vec::new(),
            },
            // The desktop's Insert image opens a chooser; a terminal's own
            // paste carries text only, so this is where an image is asked
            // for -- the one place the clipboard is read (FR-026).
            "insert_image" => vec![Effect::ReadClipboardImage],
            "attach_file" => {
                self.path_prompt = Some(tui_input::Input::default());
                vec![Effect::Redraw]
            }
            "schedule_send" => {
                self.scheduling = Some(postio_ui::schedule::schedule_presets(chrono::Local::now()));
                vec![Effect::Redraw]
            }
            // Escape. The draft is not lost by leaving: it is autosaved, a row
            // in Drafts, as the desktop's Esc parks one.
            // A draft that has the whole screen gives it back first.
            "back" if self.detached => {
                self.detached = false;
                vec![Effect::Redraw]
            }
            "remind_if_no_reply" => {
                let subject = self
                    .composer
                    .as_ref()
                    .map(|composer| composer.draft().subject)
                    .unwrap_or_default();
                self.open_remind_picker_for_draft(&subject)
            }
            "back" | "discard_draft" => self.close_composer(),
            // The desktop moves its composer into a window of its own; here
            // it is a tab, and the reading pane goes back to the reader.
            "detach_composer" => {
                self.detached = !self.detached;
                vec![Effect::Redraw]
            }
            "save_draft" => self
                .composer
                .as_ref()
                .map(|composer| {
                    vec![Effect::SaveDraft {
                        generation: composer.generation(),
                        draft: Box::new(composer.draft()),
                    }]
                })
                .unwrap_or_default(),
            "bold" | "italic" | "heading_1" | "bullet_list" | "numbered_list" | "quote_block"
            | "insert_link" => {
                use crate::composer::Format;
                let format = match id {
                    "bold" => Format::Bold,
                    "italic" => Format::Italic,
                    "heading_1" => Format::Heading1,
                    "bullet_list" => Format::BulletList,
                    "numbered_list" => Format::NumberedList,
                    "quote_block" => Format::Quote,
                    _ => Format::Link,
                };
                let Some(composer) = self.composer.as_mut() else {
                    return Vec::new();
                };
                if !composer.format(format) {
                    return self.say("Formatting is for the message's body");
                }
                vec![
                    Effect::Redraw,
                    Effect::Autosave {
                        generation: composer.generation(),
                        edit: composer.edits(),
                    },
                ]
            }
            "copy_fields" => {
                if let Some(composer) = self.composer.as_mut() {
                    composer.toggle_copy_fields();
                }
                vec![Effect::Redraw]
            }
            // Everything else the composer context reaches -- quitting, the
            // palette -- means what it means anywhere.
            _ => self.command(id),
        }
    }

    /// The key that runs `command`, as this terminal can send it, for a
    /// hint; `None` when nothing it can send is bound.
    pub fn hint(&self, command: postio_core::CommandId) -> Option<String> {
        postio_ui::terminal::deliverable_binding(self.keys.keymap(), command, self.enhanced_keys)
    }

    /// The keymap context the keyboard is in.
    fn key_context(&self) -> KeyContext {
        match self.focus {
            Focus::List => KeyContext::List,
            Focus::Reader => KeyContext::Reader,
            Focus::Composer => KeyContext::Composer,
            Focus::Bar | Focus::Folders => KeyContext::Search,
            Focus::Picker => KeyContext::Picker,
            Focus::Keys => KeyContext::List,
            Focus::Palette => KeyContext::Palette,
            Focus::FirstRun => KeyContext::Global,
            Focus::Settings => KeyContext::Accounts,
            Focus::Filtered => KeyContext::Filtered,
            Focus::Digest => KeyContext::Digest,
            Focus::Rules => KeyContext::Filtered,
            Focus::RuleDialog => KeyContext::Picker,
            Focus::Capture => KeyContext::Capture,
        }
    }

    /// What the status line says about the last thing done.
    pub fn notice(&self) -> Option<&str> {
        self.notice.as_deref()
    }

    /// Put `sentence` on the status line. Through `SafeText`: these are
    /// Postio's own sentences, but an error can quote a folder name, and a
    /// folder name came from a server.
    fn say(&mut self, sentence: &str) -> Vec<Effect> {
        self.say_as(Tone::Plain, sentence, None)
    }

    /// [`App::say`], as news of `tone`, offering undo on `undo`.
    fn say_as(&mut self, tone: Tone, sentence: &str, undo: Option<String>) -> Vec<Effect> {
        self.say_for(tone, sentence, undo, TOAST)
    }

    /// [`App::say_as`], staying `after` rather than the toast's own time.
    fn say_for(
        &mut self,
        tone: Tone,
        sentence: &str,
        undo: Option<String>,
        after: std::time::Duration,
    ) -> Vec<Effect> {
        self.notice = Some(postio_ui::terminal::SafeText::new(sentence).to_string());
        self.notice_tone = tone;
        self.notice_undo = undo;
        self.notice_offer = None;
        self.notice_generation += 1;
        vec![
            Effect::ExpireNotice {
                generation: self.notice_generation,
                after,
            },
            Effect::Redraw,
        ]
    }

    /// What kind of news the status line's notice is.
    pub fn notice_tone(&self) -> Tone {
        self.notice_tone
    }

    /// The key the notice offers undo on, when it does.
    pub fn notice_undo(&self) -> Option<&str> {
        self.notice_undo.as_deref()
    }

    /// The label of the one button the notice offers, when it offers one.
    pub fn notice_offer(&self) -> Option<&str> {
        self.notice_offer.as_ref().map(|(label, _)| label.as_str())
    }

    /// The same app, mirroring into `state`: the one the client snapshots.
    pub fn with_state(mut self, state: postio_core::SharedState) -> App {
        self.state = state;
        self
    }

    /// The state the client snapshots with each command.
    pub fn state(&self) -> postio_core::SharedState {
        self.state.clone()
    }

    /// Whether the has-action filter is on.
    pub fn has_action(&self) -> bool {
        self.scope == Some(ListScope::Focus(postio_model::FocusScope::HasAction))
    }

    /// What the strip says about the place on screen.
    pub fn strip(&self) -> Strip {
        let focus = matches!(
            self.scope,
            Some(ListScope::Focus(
                postio_model::FocusScope::Inbox | postio_model::FocusScope::HasAction
            ))
        );
        let counts = match (self.scope, self.counts) {
            (Some(ListScope::Focus(_)), Some(counts)) => {
                postio_ui::focus_row::strip_counts(counts.conversations, counts.unread)
            }
            (Some(ListScope::Mailbox(id)), _) => {
                let unread = self
                    .places
                    .folders
                    .iter()
                    .find(|folder| folder.id == id)
                    .map_or(0, |folder| folder.counts.unread);
                postio_ui::focus_row::strip_counts(self.list.total(), unread)
            }
            _ => postio_ui::focus_row::strip_counts(self.list.total(), 0),
        };
        let has_action = self.counts.map(|counts| counts.has_action);
        let toggle = focus.then(|| Toggle {
            label: postio_ui::focus_row::has_action_label(has_action),
            on: self.has_action(),
        });
        let showing = match (self.has_action(), self.counts) {
            (true, Some(counts)) => Some(postio_ui::focus_row::showing(
                counts.has_action,
                counts.conversations,
                self.hint(postio_core::CommandId::ToggleHasAction)
                    .as_deref(),
            )),
            _ => None,
        };
        let filtered = self
            .counts
            .filter(|_| focus && self.features.filtering)
            .map(|counts| counts.filtered_today)
            .filter(|count| *count > 0)
            .map(postio_ui::filtered::today);
        let rules = (focus && self.features.digest_rules > 0)
            .then(|| postio_ui::focus_row::digest_rules(self.features.digest_rules));
        Strip {
            place: self.place_name(),
            counts,
            toggle,
            showing,
            filtered,
            rules,
        }
    }

    /// What the top bar's sync label says: where every account whose
    /// connection has been heard of stands, and when mail last arrived.
    pub fn sync_label(&self) -> postio_ui::focus_state::SyncLabel {
        let folders: Vec<(postio_model::MailboxId, String)> = self
            .folders
            .iter()
            .map(|folder| (folder.id, postio_ui::places::place_name(folder)))
            .collect();
        postio_ui::focus_state::sync_label_here(
            &self.trackers.statuses(&self.tracked),
            self.last_synced,
            &folders,
        )
    }

    /// The one banner under the strip, when there is one: a refused
    /// password, an account that cannot sync, no network, or a first sync.
    pub fn banner(&self) -> Option<postio_ui::focus_state::Banner> {
        let facts: Vec<postio_ui::focus_state::AccountFacts> = self
            .accounts
            .iter()
            .filter(|account| account.enabled)
            .map(|account| postio_ui::focus_state::AccountFacts {
                id: account.id,
                server: account.incoming.host.clone(),
                address: account.address.address.clone(),
                name: if account.display_name.is_empty() {
                    account.address.address.clone()
                } else {
                    account.display_name.clone()
                },
            })
            .collect();
        postio_ui::focus_state::banner(&self.trackers.statuses(&self.tracked), &facts)
    }

    /// What the list's place says while Focus's inbox has no conversations
    /// and the has-action filter is off; nothing otherwise.
    pub fn empty_inbox(
        &self,
        now: chrono::DateTime<chrono::Local>,
    ) -> Option<postio_ui::focus_state::EmptyInbox> {
        if self.scope != Some(ListScope::Focus(postio_model::FocusScope::Inbox))
            || self.list.total() > 0
        {
            return None;
        }
        let filtered = self.counts.map_or(0, |counts| counts.filtered_today);
        let saying = postio_ui::focus_state::inbox_saying(
            &self.trackers.statuses(&self.tracked),
            self.last_synced,
        );
        Some(
            postio_ui::focus_state::empty_inbox(
                &self.features.focus(),
                filtered,
                self.keys.keymap(),
                &now,
            )
            .saying(&saying, self.keys.keymap(), &chrono::Local),
        )
    }

    /// What the strip calls the place on screen.
    pub fn place_name(&self) -> postio_ui::terminal::SafeText {
        match self.scope {
            Some(scope) => crate::places::name_of(&self.places, scope),
            None => postio_ui::terminal::SafeText::new(""),
        }
    }

    /// Whether the terminal has room for the window; when it has not, the
    /// screen says so and draws nothing else.
    pub fn fits(&self) -> bool {
        crate::layout::fits(self.size.0, self.size.1)
    }

    /// The window's rows at this size.
    pub fn window(&self) -> crate::layout::Window {
        crate::layout::window(
            ratatui::layout::Rect::new(0, 0, self.size.0, self.size.1),
            self.banner().is_some(),
        )
    }

    /// The row the keyboard is on.
    pub fn cursor(&self) -> u32 {
        self.cursor
    }

    /// How many rows the list has.
    pub fn total(&self) -> u32 {
        self.list.total()
    }

    /// The lines the list may use.
    fn list_lines(&self) -> u16 {
        self.window().list.height
    }

    /// The row at `position`, when its page is here.
    pub fn row_at(&self, position: u32) -> Option<&Row> {
        self.list.resident_at(position).map(|held| &held.row)
    }

    /// The heading that starts at `position` in a view whose first position
    /// is `top`: the day its mail arrived on, where that is not the day of
    /// the row before it. The first row in view always has one. Search
    /// results are ranked, not dated, and have none.
    fn heading_at(&self, position: u32, top: u32) -> Option<Heading> {
        let row = self.row_at(position)?;
        if self.has_action() {
            // One heading over the whole list, not a day's.
            return (position == 0).then(|| {
                Heading::Text(postio_ui::focus_row::has_action_label(
                    self.counts.map(|counts| counts.has_action),
                ))
            });
        }
        let day = row.day();
        let starts = position == top
            || position
                .checked_sub(1)
                .and_then(|before| self.row_at(before))
                .is_some_and(|before| before.day() != day);
        starts.then_some(Heading::Day(day))
    }

    /// How many lines the row at `position` takes with the heading that
    /// starts there, as the view from `top` draws them. A row whose page is
    /// still on its way is one line.
    fn lines_at(&self, position: u32, top: u32) -> u16 {
        crate::view::list::lines_of(self.row_at(position))
            + u16::from(self.heading_at(position, top).is_some())
    }

    /// How many rows, from position `top`, fit in the list: at least one.
    /// Only the rows in view are read.
    fn fit_from(&self, top: u32) -> u32 {
        let room = self.list_lines();
        let mut used = 0u16;
        let mut shown = 0u32;
        for position in top..self.list.total() {
            let lines = self.lines_at(position, top);
            if used.saturating_add(lines) > room {
                break;
            }
            used += lines;
            shown += 1;
        }
        shown.max(1)
    }

    /// The first position of a view that ends with `cursor` at its foot, as
    /// well as the rows here say: walked back from the cursor, each row with
    /// the heading it draws inside the view -- one where its day begins --
    /// and the one heading the view's first row always has.
    fn top_for_bottom(&self, cursor: u32) -> u32 {
        let room = self.list_lines();
        // The day heading `position` draws when it is not the first in view.
        let day = |position: u32| u16::from(position > 0 && self.heading_at(position, 0).is_some());
        let mut top = cursor;
        let mut used = crate::view::list::lines_of(self.row_at(cursor)) + day(cursor);
        while top > 0 {
            let before = top - 1;
            let lines = crate::view::list::lines_of(self.row_at(before));
            // With `before` first, its heading is drawn whatever the day.
            if used.saturating_add(lines + 1) > room {
                break;
            }
            used += lines + day(before);
            top = before;
        }
        top
    }

    /// The rows in view, for drawing. Reads only what is resident.
    pub fn visible(&self) -> Vec<Visible<'_>> {
        let end = (self.top + self.fit_from(self.top)).min(self.list.total());
        (self.top..end)
            .map(|position| {
                let row = self.row_at(position);
                Visible {
                    row,
                    cursor: position == self.cursor,
                    selected: row.is_some_and(|row| self.selection.contains(row.id)),
                    heading: self.heading_at(position, self.top),
                }
            })
            .collect()
    }

    /// Move the cursor to `position`, keeping it in view.
    fn move_to(&mut self, position: u32) {
        let last = self.list.total().saturating_sub(1);
        self.cursor = position.min(last);
        if self.cursor < self.top {
            self.top = self.cursor;
        } else if self.cursor >= self.top + self.fit_from(self.top) {
            self.top = self.top_for_bottom(self.cursor);
        }
        self.reveal();
    }

    /// Scroll just far enough that the cursor's row is in view: rows that
    /// landed since may be taller than the guess the view was placed by.
    fn reveal(&mut self) {
        let last = self.list.total().saturating_sub(1);
        self.cursor = self.cursor.min(last);
        if self.cursor < self.top {
            self.top = self.cursor;
            return;
        }
        while self.top < self.cursor && self.cursor >= self.top + self.fit_from(self.top) {
            self.top += 1;
        }
    }

    /// The message the cursor is on, if its row is here.
    fn cursor_message(&self) -> Option<postio_model::MessageId> {
        self.list.peek(self.cursor)
    }

    /// Run a command the keymap resolved to.
    ///
    /// Moving and marking are this frontend's own: they change what is on
    /// screen and nothing in the store. Everything else is aimed by
    /// `postio_core::aim` -- the rule every frontend shares for what a verb
    /// acts on -- mirrored into [`App::state`], and sent.
    /// Leave Postio, from wherever the person is. What is being written is
    /// saved on the way out.
    fn quit(&mut self) -> Vec<Effect> {
        let mut effects = if self.composer.is_some() {
            self.close_composer()
        } else {
            Vec::new()
        };
        effects.push(Effect::Quit);
        effects
    }

    fn command(&mut self, id: &str) -> Vec<Effect> {
        if self.focus == Focus::Filtered
            && let Some(effects) = self.filtered_command(id)
        {
            return effects;
        }
        if self.focus == Focus::Digest
            && let Some(effects) = self.digest_command(id)
        {
            return effects;
        }
        if self.focus == Focus::Rules
            && let Some(effects) = self.rules_command(id)
        {
            return effects;
        }
        if self.focus == Focus::Capture
            && let Some(effects) = self.capture_command(id)
        {
            return effects;
        }
        if self.focus == Focus::List
            && let Ok(command) = id.parse::<postio_core::CommandId>()
            && Self::the_controllers(command)
        {
            return self.tell(postio_focus::Input::Command(command));
        }
        match id {
            "find_in_message" => return self.open_find(),
            "find_next" => return self.find_step(true),
            "find_previous" => return self.find_step(false),
            "next_message" if self.focus == Focus::Reader => return self.step_open(1),
            "prev_message" if self.focus == Focus::Reader => return self.step_open(-1),
            "quit" => return self.quit(),
            "reply" | "reply_all" | "forward" => {
                let kind = match id {
                    "reply" => ReplyKind::Reply,
                    "reply_all" => ReplyKind::ReplyAll,
                    _ => ReplyKind::Forward,
                };
                // What is being read, as the desktop's reply reads what its
                // reading pane shows (#325); the row under the cursor when
                // nothing is open yet.
                let message = self
                    .reading
                    .as_ref()
                    .and_then(|reading| reading.members.get(reading.current))
                    .map(|member| member.id)
                    .or_else(|| self.cursor_message());
                if let Some(message) = message {
                    return vec![Effect::ReplySource { kind, message }];
                }
            }
            // A draft being written is resumed, from its row or its open
            // message; one on its way or stopped is read.
            // A digest opens in its own window, not as a message.
            "open_message" if self.focus == Focus::List && self.cursor_is_digest() => {
                return self.open_digest();
            }
            "open_message" if self.focus == Focus::Reader && self.open_draft_offers_edit() => {
                if let Some(message) = self.reading.as_ref().map(|reading| reading.row) {
                    let mut effects = self.close_message();
                    effects.push(Effect::Resume(message));
                    return effects;
                }
            }
            "open_message"
                if self.listing_drafts()
                    && self.row_at(self.cursor).is_none_or(|row| {
                        !postio_ui::focus_dialog::opens_to_read(row.send_state)
                    }) =>
            {
                if let Some(message) = self.cursor_message() {
                    return vec![Effect::Resume(message)];
                }
            }
            // Opening is the reader's own business, not a store verb: the
            // row under the cursor is read if it is not already, and the
            // keyboard goes into it.
            "open_message" => {
                return self.open_at_cursor();
            }
            // One composition at a time, as the desktop's `c` does with a
            // composer already open: it goes back to it.
            "compose" if self.composer.is_some() => {
                self.focus = Focus::Composer;
                return vec![Effect::Redraw];
            }
            "compose" => {
                if let Some(account) = self.account_here() {
                    return self.compose(postio_model::Draft::new(account));
                }
            }
            "go_to_folders" => return self.open_folders(),
            "search" => return self.open_bar(""),
            "command_palette" => {
                return self.open_bar(&postio_ui::finder::COMMANDS_ONLY.to_string());
            }
            "cheat_sheet" if self.sheet.is_some() => return self.close_keys(),
            "cheat_sheet" => return self.open_keys(),
            "settings" => {
                self.settings = Some(crate::settings::Settings::default());
                self.focus = Focus::Settings;
            }
            "edit_config" => return vec![Effect::EditConfig(None)],
            "add_account" => {
                self.first_run = Some(crate::first_run::FirstRun::another());
                self.focus = Focus::FirstRun;
            }
            // The bar's own keys: it resolves them itself while it is open, and
            // there is nothing for them to do anywhere else.
            "toggle_result_order" | "save_search" | "back_to_words" => {}
            "back" if self.focus == Focus::Reader => return self.back_from_message(),
            // A picker resolves its own keys while it is open.
            "picker_choose_1" | "picker_choose_2" | "picker_choose_3" | "picker_choose_4"
            | "picker_type_date" | "picker_toggle" | "picker_confirm" => {}
            "back" if self.focus != Focus::List => self.focus = Focus::List,
            "expand_all" => self.toggle_folds(),
            "show_images" => return self.allow_images(false),
            "always_show_images" => return self.allow_images(true),
            // The notice's own button, from the keyboard (#1953): the offer
            // on the status line if there is one, else a sentence saying so.
            "clear_list_mail" => {
                if self.notice_offer.is_some() {
                    return self.take_up_offer();
                }
                return self.say(postio_ui::unsubscribe::NO_OFFER);
            }
            "unsubscribe" => {
                let reading = self.reading.as_ref();
                return reading
                    .and_then(|reading| reading.members.get(reading.current))
                    .map(|member| vec![Effect::Unsubscribe(member.id)])
                    .unwrap_or_default();
            }
            "next_in_thread" => return self.walk_conversation(1),
            "prev_in_thread" => return self.walk_conversation(-1),
            "toggle_reading_pane" => return self.toggle_reading_pane(),
            "view_source" => return self.view_source(),
            "open_attachment_or_link" => return self.offer_choices(),
            "more_actions" => return self.more_actions(),
            "dismiss_marker" if self.focus == Focus::Reader => {
                return self.dismiss_open_marker();
            }
            "toggle_read" => {
                self.cancel_dwell();
                return self.send("toggle_read");
            }
            "scroll_reader_down" => self.scroll_reader(1),
            "scroll_reader_up" => self.scroll_reader(-1),
            // One toggle in a terminal: reader view is the readable form of
            // bulk mail here, and both commands move between it and the
            // sender's own markup (spec 006 FR-031).
            "view_original" | "toggle_reader_view" => return self.view_original(),
            // One message is shown at a time, so there is none to fold to
            // its header.
            "toggle_fold" => {
                // The keys are whatever the keymap has them on, Gmail's
                // `n` and `p` by default.
                let key = |command| {
                    self.keys
                        .key_for(KeyContext::Reader, command)
                        .unwrap_or_else(|| command.to_owned())
                };
                let sentence = format!(
                    "The open message shows one message; {} and {} step through the thread",
                    key("next_in_thread"),
                    key("prev_in_thread")
                );
                return self.say(&sentence);
            }
            // The invitation on the open message, or on the cursor's row:
            // the host queues the reply for its window (FR-102).
            "accept_invite" | "decline_invite" => {
                let message = self
                    .reading
                    .as_ref()
                    .and_then(|reading| reading.members.get(reading.current))
                    .map(|member| member.id)
                    .or_else(|| self.cursor_message());
                if let Some(message) = message {
                    let command = if id == "accept_invite" {
                        postio_core::CommandId::AcceptInvite
                    } else {
                        postio_core::CommandId::DeclineInvite
                    };
                    return self.answer(message, command);
                }
            }
            // The banner's button: sign in again to the account it names.
            "update_credential" => {
                if let Some(postio_ui::focus_state::Banner::SignIn { address, .. }) = self.banner()
                    && let Some(account) = self
                        .accounts
                        .iter()
                        .find(|account| account.address.address.eq_ignore_ascii_case(&address))
                {
                    self.first_run = Some(crate::first_run::FirstRun::repair(account));
                    self.focus = Focus::FirstRun;
                }
            }
            "go_to_filtered" => return self.go_to_filtered(),
            "go_to_digest_rules" => return self.go_to_digest_rules(),
            "digest_rule" => return self.digest_rule(),
            "digest_like_this" => return self.digest_like_this(),
            "capture_task" => return self.open_capture(postio_ui::capture::Mode::Task),
            "capture_note" => return self.open_capture(postio_ui::capture::Mode::Note),
            // The sheet's own, with no sheet open.
            "capture_change_project" | "capture_use_subject" | "capture_write" => {
                return Vec::new();
            }
            "sweep_inbox" => return self.ask_sweep(),
            "go_to_inbox" => return self.go_to(postio_model::mailbox::MailboxRole::Inbox),
            "go_to_sent" => return self.go_to(postio_model::mailbox::MailboxRole::Sent),
            "go_to_drafts" => return self.go_to(postio_model::mailbox::MailboxRole::Drafts),
            "go_to_flagged" => return self.go_to(postio_model::mailbox::MailboxRole::Flagged),
            // The one keymap's two new destinations (specs/007-postio-focus
            // T162), by role like the others, as the desktop reaches them.
            "go_to_archive" => return self.go_to(postio_model::mailbox::MailboxRole::Archive),
            "go_to_snoozed" => return self.go_to(postio_model::mailbox::MailboxRole::Snoozed),
            "go_to_outbox" => return self.go_to(postio_model::mailbox::MailboxRole::Outbox),
            "go_to_junk" => return self.go_to(postio_model::mailbox::MailboxRole::Junk),
            "go_to_trash" => return self.go_to(postio_model::mailbox::MailboxRole::Trash),
            "saved_search_1" => return self.pinned_search(0),
            "saved_search_2" => return self.pinned_search(1),
            "saved_search_3" => return self.pinned_search(2),
            "saved_search_4" => return self.pinned_search(3),
            // Gmail's `u`, back to the list: from an open message that is
            // closing it, as `Escape` does, not the view before this one.
            "prev_view" if self.focus == Focus::Reader => return self.back_from_message(),
            "prev_view" => {
                return self.tell(postio_focus::Input::Command(
                    postio_core::CommandId::HistoryBack,
                ));
            }
            "next_scope" => return self.next_scope(),
            other => return self.send(other),
        }
        vec![Effect::Redraw]
    }

    /// The message on screen in the sender's own markup, or back in reader
    /// view: redrawn from the body it arrived with, nothing asked again.
    fn view_original(&mut self) -> Vec<Effect> {
        let Some(member) = self
            .reading
            .as_mut()
            .and_then(|reading| reading.members.get_mut(reading.current))
        else {
            return Vec::new();
        };
        let Some(body) = member.source.clone() else {
            return self.say("This message has no markup of its own to show");
        };
        if !member.reader_view && !member.original {
            return self.say("This message is already shown as its sender wrote it");
        }
        member.original = member.reader_view;
        let message = member.id;
        self.show(
            message,
            Ok(postio_client::protocol::Body::Ready {
                body,
                encoding_problems: false,
            }),
        )
    }

    /// The account the go-to keys and the scope cycle start from: the one
    /// on screen, or the first.
    fn account_here(&self) -> Option<postio_model::AccountId> {
        self.account.or_else(|| {
            self.accounts
                .iter()
                .find(|account| account.enabled)
                .map(|account| account.id)
        })
    }

    /// The account's folder with `role`, or its Flagged view, as the
    /// desktop's `g` keys open them.
    fn go_to(&mut self, role: postio_model::mailbox::MailboxRole) -> Vec<Effect> {
        use postio_model::mailbox::MailboxRole;
        let Some(account) = self.account_here() else {
            return Vec::new();
        };
        let scope = match role {
            // Views, not folders (ADR 0036): opened by their role.
            MailboxRole::Flagged => Some(ListScope::Flagged(account)),
            MailboxRole::Snoozed => Some(ListScope::Snoozed(account)),
            // A view over Drafts, not a folder.
            MailboxRole::Outbox => Some(ListScope::Outbox(account)),
            // Focus's inbox is every account's, as one.
            MailboxRole::Inbox => Some(ListScope::Focus(postio_model::FocusScope::Inbox)),
            role => self
                .folders
                .iter()
                .find(|folder| folder.account_id == account && folder.role == role)
                .map(|folder| ListScope::Mailbox(folder.id)),
        };
        match scope {
            Some(scope) => self.open_scope(scope),
            None => self.say(&format!("This account has no {} folder", role_name(role))),
        }
    }

    /// The saved search pinned `index`th, run as the finder runs it:
    /// `alt+1`...`alt+4`, as the desktop's are (specs/007-postio-focus
    /// T162). A place with nothing pinned in it is said.
    fn pinned_search(&mut self, index: usize) -> Vec<Effect> {
        let Some(query) = self
            .places
            .saved
            .get(index)
            .map(|saved| saved.query.clone())
        else {
            return self.say(&postio_ui::focus_target::no_saved_search(index));
        };
        self.open_bar(&query)
    }

    /// Each account's inbox in turn, then every account at once when there
    /// is more than one -- the desktop's `next_scope`.
    fn next_scope(&mut self) -> Vec<Effect> {
        use postio_model::mailbox::MailboxRole;
        let mut scopes: Vec<ListScope> = self
            .accounts
            .iter()
            .filter(|account| account.enabled)
            .filter_map(|account| {
                self.folders
                    .iter()
                    .find(|folder| {
                        folder.account_id == account.id && folder.role == MailboxRole::Inbox
                    })
                    .map(|folder| ListScope::Mailbox(folder.id))
            })
            .collect();
        if scopes.len() > 1 {
            scopes.push(ListScope::Unified);
        }
        if scopes.is_empty() {
            return Vec::new();
        }
        let here = match self.scope {
            Some(ListScope::Unified) => {
                scopes.iter().position(|scope| *scope == ListScope::Unified)
            }
            Some(scope) => {
                let account = self.account_of(scope);
                scopes.iter().position(|candidate| {
                    *candidate != ListScope::Unified && self.account_of(*candidate) == account
                })
            }
            None => None,
        };
        let next = here.map_or(0, |at| (at + 1) % scopes.len());
        self.open_scope(scopes[next])
    }

    /// Answer the invitation `message` carries, as `id` says: through the
    /// host, which queues the reply for the answer's window.
    fn answer(
        &mut self,
        message: postio_model::MessageId,
        id: postio_core::CommandId,
    ) -> Vec<Effect> {
        let message = Some(message);
        self.answering = true;
        vec![Effect::Send(match id {
            postio_core::CommandId::DeclineInvite => {
                postio_core::Command::DeclineInvite { message }
            }
            _ => postio_core::Command::AcceptInvite { message },
        })]
    }

    /// Aim a verb at what the user is looking at, and send it.
    ///
    /// A move with no folder and a label with no label are half a request:
    /// `None` means "ask", and this is the terminal asking, as the desktop's
    /// window opens its finder for them.
    fn send(&mut self, id: &str) -> Vec<Effect> {
        let Ok(id) = id.parse::<postio_core::CommandId>() else {
            return Vec::new();
        };
        match id {
            postio_core::CommandId::Move => self.open_picker(crate::pickers::Kind::Move),
            postio_core::CommandId::AddLabel => self.open_picker(crate::pickers::Kind::Label),
            postio_core::CommandId::Snooze => self.open_picker(crate::pickers::Kind::Snooze),
            postio_core::CommandId::RemindIfNoReply => {
                self.open_picker(crate::pickers::Kind::Remind)
            }
            id => self.send_aimed(id),
        }
    }

    /// Send `id` aimed as [`App::send`] aims it, with the answer to what it
    /// asked filled in.
    fn send_answered(
        &mut self,
        id: postio_core::CommandId,
        answer: impl Fn(&mut postio_core::Command),
    ) -> Vec<Effect> {
        let mut effects = self.send_aimed(id);
        for effect in &mut effects {
            if let Effect::Send(command) = effect {
                answer(command);
            }
        }
        effects.push(Effect::Redraw);
        effects
    }

    /// Aim a verb at what the user is looking at, and send it as it is.
    fn send_aimed(&mut self, id: postio_core::CommandId) -> Vec<Effect> {
        let reachable: Vec<postio_model::AccountId> = self
            .accounts
            .iter()
            .filter(|account| account.enabled)
            .map(|account| account.id)
            .collect();
        let (quiet, _) = postio_core::bridge::event_channel();
        // A message opened for itself is what the verb is about; the list's
        // cursor and marks are somewhere else.
        if let Some(own) = self.own_aim() {
            let nothing = postio_core::Selection::These(Vec::new());
            let aim = postio_core::aim::Aim {
                scope: None,
                selection: &nothing,
                cursor: Some(own),
                rows: &self.list,
            };
            let command = postio_core::aim::command_for(id, &aim);
            postio_core::aim::mirror(&self.state, &quiet, &aim);
            return vec![Effect::Send(command)];
        }
        let selection = self.selection.selection();
        let aim = postio_core::aim::Aim {
            scope: self
                .scope
                .and_then(|scope| postio_core::aim::view_scope(scope, &reachable)),
            selection: &selection,
            cursor: self.cursor_message(),
            rows: &self.list,
        };
        let command = postio_core::aim::command_for(id, &aim);
        postio_core::aim::mirror(&self.state, &quiet, &aim);
        // What was selected has been acted on: the selection lets go.
        self.selection.clear();
        vec![Effect::Send(command)]
    }

    /// Expand every fold in what is being read, or fold them all again.
    fn toggle_folds(&mut self) {
        if let Some(reading) = self.reading.as_mut() {
            reading.toggle_folds();
        }
    }

    /// Allow the current message's remote images: this once, or from its
    /// sender always -- which is written to the allow list the desktop app
    /// reads too, so the sender is trusted in both.
    fn allow_images(&mut self, always: bool) -> Vec<Effect> {
        let Some(reading) = self.reading.as_mut() else {
            return Vec::new();
        };
        let Some(member) = reading.members.get_mut(reading.current) else {
            return Vec::new();
        };
        let nothing_held = member.held_back.remote_images + member.held_back.trackers == 0;
        if nothing_held && !always {
            return Vec::new();
        }
        member.images_allowed = true;
        if !always {
            return vec![Effect::Redraw];
        }
        let Some(address) = member.address.clone() else {
            return vec![Effect::Redraw];
        };
        self.allowlist.allow(&address);
        for member in &mut reading.members {
            if member.address.as_deref() == Some(address.as_str()) {
                member.images_allowed = true;
            }
        }
        vec![
            Effect::Redraw,
            Effect::SaveAllowlist(self.allowlist.clone()),
        ]
    }

    /// Open `message` for reading: its body is read now, and its
    /// conversation when it has several messages (FR-195). Nothing is read
    /// by the cursor passing over a row.
    pub fn open_reading(&mut self, message: postio_model::MessageId) -> Vec<Effect> {
        let shown = self.reading.as_ref().map(|reading| reading.row) == Some(message);
        if shown {
            return Vec::new();
        }
        let Some(row) = self.list.row_of(message) else {
            return Vec::new();
        };
        if row.kind == crate::row::Kind::Digest {
            // A digest opens in its own window (T323), not as a message.
            return Vec::new();
        }
        match row.thread {
            Some(thread) if row.is_thread && row.count > 1 => {
                self.reading = Some(crate::conversation::Reading {
                    row: message,
                    members: Vec::new(),
                    current: 0,
                    own: None,
                });
                vec![Effect::ReadConversation(thread)]
            }
            _ => {
                self.reading = Some(crate::conversation::Reading {
                    row: message,
                    members: vec![crate::conversation::Member::from_row(row)],
                    current: 0,
                    own: None,
                });
                self.reader_top = 0;
                vec![Effect::Redraw, Effect::ReadBody(message)]
            }
        }
    }

    /// A conversation's members arrived: read each one's body, and open on
    /// the newest.
    fn conversation(
        &mut self,
        thread: postio_model::ThreadId,
        members: Result<Vec<postio_model::listing::MessageSummary>, String>,
    ) -> Vec<Effect> {
        let Some(reading) = self.reading.as_mut() else {
            return Vec::new();
        };
        let wanted = self.list.row_of(reading.row).and_then(|row| row.thread) == Some(thread);
        if !wanted || !reading.members.is_empty() {
            return Vec::new();
        }
        let Ok(members) = members else {
            return Vec::new();
        };
        reading.members = members
            .iter()
            .map(crate::conversation::Member::from_summary)
            .collect();
        reading.current = reading.members.len().saturating_sub(1);
        // Only the message shown is read; the others when they are stepped
        // to (FR-195).
        let mut effects = vec![Effect::Redraw];
        if let Some(member) = reading.members.get_mut(reading.current) {
            member.asked = true;
            effects.push(Effect::ReadBody(member.id));
        }
        self.reader_top = 0;
        effects.extend(self.arm_dwell());
        effects
    }

    /// A body arrived: put it on its member, if it is still being read.
    fn show(
        &mut self,
        message: postio_model::MessageId,
        answer: Result<postio_client::protocol::Body, String>,
    ) -> Vec<Effect> {
        use postio_client::protocol::Body;
        use postio_ui::reader::document::{
            Absent, Rendering, absent_html, body_html, suits_reader_view,
        };
        let absent = |state| crate::reader::from_html(&absent_html(state));
        let mut held_back = postio_ui::reader::document::HeldBack::default();
        // Asked for by `view_original`: this message's own markup, whatever
        // the rule would have chosen.
        let asked_original = self
            .reading
            .as_ref()
            .and_then(|reading| reading.members.iter().find(|member| member.id == message))
            .is_some_and(|member| member.original);
        let mut source = None;
        let mut reader_view = false;
        let rendered = match answer {
            Ok(Body::Ready { body, .. }) if body.html.is_some() => {
                // The rule every reader applies: reader view for bulk mail,
                // the sender's own markup (sanitised, folded) otherwise.
                let rendering = if !asked_original && suits_reader_view(&body) {
                    Rendering::Reader
                } else {
                    Rendering::Original
                };
                reader_view = rendering == Rendering::Reader;
                source = Some(body.clone());
                // Blocked always: a terminal draws no image, so allowing them
                // changes what the notice says and never what is fetched.
                let drawn = body_html(&body, postio_body::RemoteImages::Blocked, rendering);
                held_back = drawn.held_back;
                crate::reader::from_html(&drawn.html)
            }
            Ok(Body::Ready { body, .. }) => {
                crate::reader::from_text(body.text.as_deref().unwrap_or(""))
            }
            Ok(Body::Partial) => absent(Absent::Partial),
            Ok(Body::Offline) => absent(Absent::Offline),
            Ok(Body::Missing) => absent(Absent::Missing),
            Ok(Body::Empty) => absent(Absent::Empty),
            Ok(Body::ForeignDraft) => absent(Absent::ForeignDraft),
            Ok(Body::Failed) => absent(Absent::Failed),
            Err(reason) => crate::reader::from_text(&reason),
        };
        let Some(member) = self.reading.as_mut().and_then(|reading| {
            reading
                .members
                .iter_mut()
                .find(|member| member.id == message)
        }) else {
            return Vec::new();
        };
        member.body = Some(rendered);
        member.source = source;
        member.reader_view = reader_view;
        let ask_for_parts = member.has_attachments && member.parts.is_empty();
        member.held_back = held_back;
        member.images_allowed = member
            .address
            .as_deref()
            .is_some_and(|address| self.allowlist.is_allowed(address));
        let mut effects = vec![Effect::Redraw];
        if ask_for_parts {
            effects.push(Effect::ReadParts(message));
        }
        effects
    }

    /// Take in the places, keeping the account the list on screen belongs
    /// to.
    fn fill_places(&mut self, contents: &crate::places::Places) -> Vec<Effect> {
        self.places = contents.clone();
        if let Some(bar) = self.bar.as_mut() {
            bar.set_saved(
                contents
                    .saved
                    .iter()
                    .map(|saved| (saved.name.clone(), saved.query.clone()))
                    .collect(),
            );
        }
        self.features = contents.features.clone();
        self.folders = contents.folders.clone();
        self.accounts = contents.accounts.clone();
        let reach = self
            .accounts
            .iter()
            .filter(|account| account.enabled)
            .map(|account| account.id)
            .collect();
        self.tell(postio_focus::Input::Accounts(reach));
        if self.last_synced.is_none() {
            self.last_synced = contents
                .folders
                .iter()
                .filter(|folder| {
                    contents
                        .accounts
                        .iter()
                        .any(|account| account.enabled && account.id == folder.account_id)
                })
                .filter_map(|folder| folder.last_synced_at)
                .max();
        }
        for account in &contents.accounts {
            self.trackers.note_last_sync(account.id, &contents.folders);
        }
        if let Some(scope) = self.scope {
            self.account = self.account_of(scope);
        }
        if contents.accounts.is_empty() {
            // No account: the first screen offers to add one rather than
            // showing an empty shell (US7 scenario 1).
            if self.first_run.is_none() {
                self.first_run = Some(crate::first_run::FirstRun::default());
                self.focus = Focus::FirstRun;
            }
            return vec![Effect::Redraw];
        }
        // An account arrived from elsewhere -- added on the desktop, say --
        // while the first run was still asking for one: it is done, unless
        // it is this run's own account and the last question is still open,
        // or the run was asked for from the mail, which it leaves by itself.
        if self.first_run.as_ref().is_some_and(|run| {
            !run.leavable() && *run.status() != postio_ui::onboarding::Status::SyncWindow
        }) {
            self.first_run = None;
            self.focus = Focus::List;
        }
        let mut effects = vec![Effect::Redraw];
        if self.scope.is_none()
            && self.first_run.is_none()
            && contents
                .folders
                .iter()
                .any(|folder| folder.role == postio_model::mailbox::MailboxRole::Inbox)
        {
            // Every account's inbox, as one, once there is some to show.
            effects.extend(self.open_scope(ListScope::Focus(postio_model::FocusScope::Inbox)));
        }
        effects
    }

    /// A name typed in the palette, for what asked for it.
    fn named(&mut self, name: String) -> Vec<Effect> {
        let effect = match self.renaming.take() {
            // The picker shows the name, so a signature without one is
            // refused here, as the desktop's form refuses it.
            Some(Renaming::Signature { .. }) if name.is_empty() => {
                return self.say("A signature needs a name");
            }
            Some(Renaming::Signature {
                account,
                signature: None,
                text,
            }) => Effect::EditSignature {
                account,
                signature: None,
                name,
                text,
            },
            Some(Renaming::Signature {
                account,
                signature,
                text,
            }) => Effect::SaveSignature {
                account,
                signature,
                name,
                text,
            },
            None => return vec![Effect::Redraw],
        };
        vec![effect, Effect::Redraw]
    }

    /// A key in an account's signatures: Enter writes the one under the
    /// cursor in the person's editor, `n` starts one, `r` renames, the
    /// keymap's remove key (`remove_account`, as the privacy pane's allowed
    /// senders use it) deletes once asked twice, and Escape goes back to
    /// the accounts.
    fn signatures_key(&mut self, account: postio_model::AccountId, key: &KeyEvent) -> Vec<Effect> {
        use crossterm::event::KeyCode;
        // Asked of the keymap only for a key the list does not use itself,
        // so a letter it moves or names with never reaches a chord.
        let listed = matches!(
            key.code,
            KeyCode::Up
                | KeyCode::Down
                | KeyCode::Char('k' | 'j' | 'n' | 'r')
                | KeyCode::Esc
                | KeyCode::Tab
                | KeyCode::BackTab
                | KeyCode::Enter
        );
        let deleting = !listed
            && matches!(
                self.keys.press(key, KeyContext::Accounts, false),
                Outcome::Command(id) if id == "remove_account"
            );
        let remove_key = self.keys.key_for(KeyContext::Accounts, "remove_account");
        let signatures = self
            .accounts
            .iter()
            .find(|candidate| candidate.id == account)
            .map(|candidate| candidate.signatures.clone())
            .unwrap_or_default();
        let Some(settings) = self.settings.as_mut() else {
            return Vec::new();
        };
        let here = signatures
            .get(settings.signature(signatures.len()))
            .cloned();
        if !deleting {
            settings.keep();
        }
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => settings.step_signature(-1, signatures.len()),
            KeyCode::Down | KeyCode::Char('j') => {
                settings.step_signature(1, signatures.len());
            }
            KeyCode::Esc | KeyCode::Tab | KeyCode::BackTab => settings.show_signatures(None),
            KeyCode::Enter => {
                if let Some(signature) = here {
                    return vec![Effect::EditSignature {
                        account,
                        signature: Some(signature.id),
                        name: signature.name,
                        text: signature.text,
                    }];
                }
            }
            KeyCode::Char('n') => {
                self.renaming = Some(Renaming::Signature {
                    account,
                    signature: None,
                    text: String::new(),
                });
                return self.open_palette(Finding::Rename);
            }
            KeyCode::Char('r') => {
                if let Some(signature) = here {
                    self.renaming = Some(Renaming::Signature {
                        account,
                        signature: Some(signature.id),
                        text: signature.text,
                    });
                    let effects = self.open_palette(Finding::Rename);
                    if let Some(palette) = self.palette.as_mut() {
                        palette.input = tui_input::Input::default().with_value(signature.name);
                    }
                    return effects;
                }
            }
            _ if deleting => {
                if let Some(signature) = here {
                    if settings.confirm_delete(signature.id) {
                        return vec![Effect::DeleteSignature(signature.id), Effect::Redraw];
                    }
                    let name = postio_ui::terminal::SafeText::new(&signature.name);
                    let again = remove_key.unwrap_or_else(|| "it".to_owned());
                    return self.say(&format!(
                        "Delete the signature “{name}”? Press {again} again to delete it; \
                         anything else keeps it"
                    ));
                }
            }
            _ => {}
        }
        vec![Effect::Redraw]
    }

    /// The host said something happened.
    ///
    /// What a command said about itself goes on the status line; what
    /// changed in the store goes through the same paging plan the desktop
    /// list follows, so the two lists react to one event the same way.
    fn hear(&mut self, event: &postio_core::Event) -> Vec<Effect> {
        use postio_core::Event;
        // Every event is offered to the status line first: an error is both
        // something to say and the reason a failing account's line gives.
        let moved = self.trackers.apply(event, self.account);
        if let Event::ConnectionChanged { account, .. }
        | Event::SyncProgress { account, .. }
        | Event::BackfillProgress { account, .. } = event
            && !self.tracked.contains(account)
        {
            self.tracked.push(*account);
        }
        if let Event::SyncProgress { done, total, .. } = event
            && done >= total
        {
            self.last_synced = Some(chrono::Utc::now());
        }
        match event {
            Event::Error { message, .. } => return self.say_as(Tone::Failed, message, None),
            // The words of a message on screen just landed (#1788): read that
            // one again, in place. Only one already drawn: a read still on
            // its way will see them anyway, and the list is never re-read.
            Event::BodyLoaded { message, .. }
            | Event::BodyFailed { message, .. }
            | Event::BodyReachable { message, .. } => {
                let drawn = self.reading.as_ref().is_some_and(|reading| {
                    reading
                        .members
                        .iter()
                        .any(|member| member.id == *message && member.body.is_some())
                });
                return if drawn {
                    vec![Effect::ReadBody(*message)]
                } else {
                    Vec::new()
                };
            }
            _ => {}
        }
        let mut effects = Vec::new();
        if matches!(event, Event::MailboxesChanged { .. }) {
            effects.push(Effect::RefreshPlaces);
        }
        effects.extend(self.tell(postio_focus::Input::Event(event.clone())));
        if moved {
            effects.push(Effect::Redraw);
        }
        effects
    }
}

/// Take in one input; say what should happen next.
pub fn update(app: &mut App, input: Input) -> Vec<Effect> {
    let mut effects = match input {
        Input::Resize(width, height) => {
            app.size = (width, height);
            vec![Effect::Redraw]
        }
        // Quit's key means the same on every surface, a text field's and a
        // frame's included, so no surface gets to answer it first.
        Input::Key(key) if app.keys.is_primary(&key, postio_core::CommandId::Quit) => app.quit(),
        Input::Key(key) if app.focus == Focus::Keys => app.keys_key(&key),
        Input::Key(key) if app.open.menu.is_some() && app.focus == Focus::Reader => {
            app.menu_key(&key)
        }
        Input::Key(key) if app.open.find.is_some() && app.focus == Focus::Reader => {
            app.find_key(&key)
        }
        Input::Key(key) if app.surfaces.sweep.is_some() => app.sweep_key(&key),
        Input::Key(key) if app.focus == Focus::FirstRun => app.first_run_key(&key),
        Input::Key(key) if app.focus == Focus::Settings => app.settings_key(&key),
        Input::Key(key) if app.focus == Focus::Composer => app.composer_key(&key),
        Input::Key(key) if app.focus == Focus::Bar => app.bar_key(&key),
        Input::Key(key) if app.focus == Focus::Folders => app.folders_key(&key),
        Input::Key(key) if app.focus == Focus::Picker => app.picker_key(&key),
        Input::Key(key) if app.focus == Focus::Palette => app.palette_key(&key),
        Input::Key(key) if app.focus == Focus::Filtered => app.filtered_key(&key),
        Input::Key(key) if app.focus == Focus::Digest => app.digest_key(&key),
        Input::Key(key) if app.focus == Focus::Rules => app.rules_key(&key),
        Input::Key(key) if app.focus == Focus::RuleDialog => app.rule_key(&key),
        Input::Key(key) if app.focus == Focus::Capture => app.capture_key(&key),
        Input::Key(key) => match app.keys.press(&key, app.key_context(), false) {
            Outcome::Command(id) => app.command(&id),
            Outcome::Pending(_) | Outcome::Unhandled => Vec::new(),
        },
        Input::FocusReply(ticket, reply) => app.tell(postio_focus::Input::Reply(ticket, reply)),
        Input::FocusTimer(token) => app.tell(postio_focus::Input::Timer(token)),
        Input::Host(event) => {
            let mut effects = app.hear(&event);
            effects.extend(app.filtered_hears(&event));
            effects
        }
        Input::Answer(answer) => app.answered(answer),
        Input::Places(contents) => {
            let mut effects = app.fill_places(&contents);
            effects.extend(app.rules_reread());
            effects
        }
        Input::Parts { message, parts } => {
            if let (Ok(parts), Some(reading)) = (parts, app.reading.as_mut())
                && let Some(member) = reading
                    .members
                    .iter_mut()
                    .find(|member| member.id == message)
            {
                member.parts = parts;
            }
            vec![Effect::Redraw]
        }
        Input::PartWritten { written, open } => match written {
            Ok(path) if open => {
                let mut effects = app.say(&format!("Opening {}", path.display()));
                effects.push(Effect::Launch(path));
                effects
            }
            Ok(path) => app.say(&format!("Saved {}", path.display())),
            Err(reason) => app.say(&reason),
        },
        Input::NoticeDue { generation } => {
            if generation == app.notice_generation && app.notice.take().is_some() {
                app.notice_undo = None;
                app.notice_offer = None;
                vec![Effect::Redraw]
            } else {
                Vec::new()
            }
        }
        Input::Source { message, raw } => app.source_read(message, raw),
        Input::DwellDue {
            generation,
            message,
        } => app.dwelt(generation, message),
        Input::AutosaveDue { generation, edit } => app.autosave_due(generation, edit),
        Input::SignatureSaved(saved) => match saved {
            // The account list carries the signatures; read it again.
            Ok(()) => vec![Effect::RefreshPlaces, Effect::Redraw],
            Err(reason) => app.say(&reason),
        },
        Input::Privacy { log, connections } => {
            app.privacy = Some(Privacy { log, connections });
            vec![Effect::Redraw]
        }
        Input::DraftSaved { generation, saved } => match saved {
            Ok(id) => {
                if let Some(composer) = app.composer.as_mut()
                    && composer.generation() == generation
                {
                    composer.adopt_id(id);
                    app.saved_at = Some(chrono::Utc::now());
                }
                Vec::new()
            }
            Err(reason) => app.say(&reason),
        },
        Input::Edited { generation, edited } => match (edited, app.composer.as_mut()) {
            (Ok(markdown), Some(composer)) if composer.generation() == generation => {
                composer.replace_body(&markdown);
                vec![
                    Effect::Autosave {
                        generation,
                        edit: composer.edits(),
                    },
                    Effect::Redraw,
                ]
            }
            (Ok(_), _) => app.say("The draft closed while it was in the editor"),
            (Err(reason), _) => app.say(&format!("The editor did not save: {reason}")),
        },
        Input::ClipboardImage(read) => match read {
            Ok(Some(bytes)) => vec![Effect::InlineImage {
                bytes,
                mime_type: "image/png".to_owned(),
            }],
            Ok(None) => app.say("There is no image on the clipboard"),
            Err(reason) => app.say(&reason),
        },
        Input::InlineStored(stored) => match (stored, app.composer.as_mut()) {
            (Some(image), Some(composer)) => {
                let label = image
                    .content_id
                    .as_deref()
                    .map(|id| format!("![image](cid:{id})"));
                composer.attach(image);
                if let Some(label) = label {
                    composer.insert(&label);
                }
                vec![
                    Effect::Autosave {
                        generation: composer.generation(),
                        edit: composer.edits(),
                    },
                    Effect::Redraw,
                ]
            }
            (Some(_), None) => app.say("The draft closed before the image was stored"),
            (None, _) => app.say("The image could not be stored"),
        },
        Input::Discovered(found) => {
            if let Some(first_run) = app.first_run.as_mut() {
                match found {
                    Ok(status) => first_run.discovered(status),
                    // Asking failed: type the servers, as when nothing was found.
                    Err(_) => first_run
                        .discovered(postio_ui::onboarding::Status::Manual { suggestion: None }),
                }
            }
            vec![Effect::Redraw]
        }
        Input::Consent(consent) => {
            let Some(first_run) = app.first_run.as_mut() else {
                return vec![Effect::Redraw];
            };
            match consent {
                Ok(sign_in) => {
                    first_run.consent(sign_in);
                    // Its end comes back as the account being added, or not.
                    vec![Effect::FinishOAuth(first_run.address()), Effect::Redraw]
                }
                Err(sentence) => {
                    first_run.failed(sentence);
                    vec![Effect::Redraw]
                }
            }
        }
        Input::AccountAdded(added) => {
            if let Some(first_run) = app.first_run.as_mut() {
                match added {
                    Ok(()) if first_run.repairing() => {
                        // Signed in again: back where it was asked from.
                        app.first_run = None;
                        app.focus = if app.settings.is_some() {
                            Focus::Settings
                        } else {
                            Focus::List
                        };
                        return app.say("Signed in again");
                    }
                    Ok(()) => first_run.added(),
                    Err(sentence) => first_run.failed(sentence),
                }
            }
            vec![Effect::Redraw]
        }
        Input::Pointer(pointer) if app.mouse => app.pointer(pointer),
        Input::Pointer(_) => Vec::new(),
        Input::ConfigEdited(edited) => {
            let mut effects = match edited {
                Ok(()) => app.say("Saved — config.toml is read again"),
                Err(reason) => app.say(&format!("The editor did not save: {reason}")),
            };
            effects.push(Effect::RefreshPlaces);
            effects
        }
        Input::Paste(pasted) => app.paste(&pasted),
        Input::Attached { path, attached } => match (attached, app.composer.as_mut()) {
            (Some(attachment), Some(composer)) => {
                composer.attach(attachment);
                vec![
                    Effect::Autosave {
                        generation: composer.generation(),
                        edit: composer.edits(),
                    },
                    Effect::Redraw,
                ]
            }
            (Some(_), None) => app.say("The draft closed before the file was attached"),
            (None, _) => app.say(&format!(
                "Could not attach {}: it could not be read",
                crate::paths::name_of(&path)
            )),
        },
        Input::BarFound {
            sequence,
            found,
            held,
        } => app.bar_found(sequence, found, &held),
        Input::BarFolder {
            sequence,
            count,
            rows,
        } => app.bar_listed(sequence, count, rows),
        Input::PlaceDetails(details) => app.place_details(details),
        Input::Notified(notification) => {
            let safe = |text: &str| postio_ui::terminal::SafeText::new(text).to_string();
            let (title, body) = (safe(&notification.title), safe(&notification.body));
            let mut effects = app.say(&format!("New mail — {title}: {body}"));
            effects.push(Effect::DesktopNotify { title, body });
            effects
        }
        // Outside text, made safe to draw once, here, as the list's rows are.
        Input::LabelPicker {
            account,
            labels,
            counts,
            applied,
        } => {
            let labels = labels
                .into_iter()
                .map(|mut label| {
                    label.name = postio_ui::terminal::SafeText::new(&label.name)
                        .as_str()
                        .to_owned();
                    label
                })
                .collect();
            if let Some(picker) = app.picker.as_mut() {
                picker.learn_labels(account, labels, counts, applied.into_iter().collect());
            }
            vec![Effect::Redraw]
        }
        Input::LabelMade { label, close } => app.label_made(label, close),
        Input::RecentMoves(recent) => {
            if let Some(picker) = app.picker.as_mut() {
                picker.learn_recent(recent);
            }
            vec![Effect::Redraw]
        }
        Input::Recipients { prefix, found } => {
            if let Some(composer) = app.composer.as_mut() {
                composer.offer(&prefix, found);
            }
            vec![Effect::Redraw]
        }
        Input::Queued { at, queued } => match (queued, at) {
            (Ok(()), None) => app.say("Sending — it is in the Outbox until it leaves"),
            (Ok(()), Some(at)) => app.say(&format!(
                "Scheduled for {}",
                at.with_timezone(&chrono::Local).format("%a %-d %b, %H:%M")
            )),
            (Err(reason), _) => app.say(&format!(
                "Not sent: {reason}. The draft is still in Drafts."
            )),
        },
        Input::Resumed { found, failure } => match found {
            Some(draft) => {
                let mut effects = app.compose(*draft);
                if let Some(reason) = failure {
                    effects.extend(app.say(&format!("Not sent — {reason}")));
                }
                effects
            }
            None => app.say("That draft is not on this device, or is already sending"),
        },
        Input::ReplySource { kind, found } => match found.map(|found| *found) {
            Some((message, account)) => {
                app.compose(postio_body::replying::reply_draft(kind, &message, &account))
            }
            None => app.say("That message could not be read to reply to"),
        },
        Input::Unsubscribed { message, answer } => match answer {
            // Said once, with the offer if the list left mail behind: a
            // local read, so the one notice is not late.
            Ok(name) => vec![Effect::ReadListMail { message, name }],
            Err(reason) => app.say(&reason),
        },
        Input::ListMail { name, found, .. } => app.confirm_unsubscribed(&name, found.ok()),
        Input::Addressed {
            message,
            to,
            cc,
            warning,
        } => {
            let member = app.reading.as_mut().and_then(|reading| {
                reading
                    .members
                    .iter_mut()
                    .find(|member| member.id == message)
            });
            match member {
                Some(member) => {
                    // A header is attacker-controlled like any other.
                    let safe = |people: &[postio_model::EmailAddress]| -> Vec<_> {
                        people
                            .iter()
                            .map(|address| postio_ui::terminal::SafeText::new(address.display()))
                            .collect()
                    };
                    member.to = safe(&to);
                    member.cc = safe(&cc);
                    member.warning = warning.as_deref().map(postio_ui::terminal::SafeText::new);
                    vec![Effect::Redraw]
                }
                None => Vec::new(),
            }
        }
        Input::Body { message, answer } => {
            let current = app
                .reading
                .as_ref()
                .is_some_and(|reading| reading.members.iter().any(|member| member.id == message));
            if current {
                app.release_paint();
            }
            app.show(message, answer)
        }
        Input::Conversation { thread, members } => {
            if members.is_err() {
                app.release_paint();
            }
            app.conversation(thread, members)
        }
        Input::Settled { generation } => app.settled(generation),
    };
    // A message open follows the cursor wherever it went: `j`, a click, or
    // the row it was opened from leaving.
    effects.extend(app.follow_cursor());
    effects.extend(app.fetches());
    effects.extend(app.filtered_fetches());
    effects.extend(app.hold_paint(&effects));
    effects
}
