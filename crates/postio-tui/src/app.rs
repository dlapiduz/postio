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
use postio_ui::focus_list::FocusRow;
use postio_ui::keymap::{KeyContext, Outcome};
use postio_ui::list::ListWindow;
use postio_ui::paging::{Fetch, Page, Paging};
use postio_ui::surfaced::Spliced;

mod capture;
mod digest;
mod filtered;
mod find;
mod open;
mod rules;
mod surface;

pub use find::Find;
pub use open::{Menu, MenuAction, MenuItem, Raw};

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
#[derive(Debug, Clone, PartialEq, Eq)]
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
    /// A list was opened, and has this many rows.
    Opened {
        /// What the list shows.
        scope: ListScope,
        /// How many rows it has.
        total: u32,
    },
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
    Unsubscribed(Result<String, String>),
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
    /// What Focus's inbox surfaces among its conversations, read afresh:
    /// fired reminders and digest deliveries.
    Surfaced(Vec<postio_model::listing::Surfaced>),
    /// What the strip counts, read after a page landed.
    FocusCounts(postio_client::protocol::FocusCounts),
    /// A list was counted again, after an event said it changed.
    Recounted {
        /// Which list.
        scope: ListScope,
        /// How many rows it has now.
        total: u32,
    },
    /// A page asked for by [`Effect::Fetch`] arrived, or failed.
    Page {
        /// The list's generation when it was asked for.
        generation: u64,
        /// Which page.
        page: u32,
        /// The rows, or why there are none.
        rows: Result<Page<Row>, String>,
    },
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
    /// Open a list: count it and answer with [`Input::Opened`].
    Open(ListScope),
    /// Read the places again and answer with [`Input::Places`].
    RefreshPlaces,
    /// Read what Focus's inbox surfaces and answer with [`Input::Surfaced`].
    ReadSurfaced,
    /// Read the strip's counts and answer with [`Input::FocusCounts`].
    ReadFocusCounts,
    /// Count a list again and answer with [`Input::Recounted`].
    Recount(ListScope),
    /// Leave the list this message came from; answer with
    /// [`Input::Unsubscribed`].
    Unsubscribe(postio_model::MessageId),
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
    /// Read a page of the list and answer with [`Input::Page`].
    Fetch {
        /// The list's generation now; a page for an older one is dropped.
        generation: u64,
        /// Which page.
        page: u32,
        /// What to read.
        fetch: Fetch,
        /// Where Focus's surfaced rows sit among the page's positions, when
        /// the list has them.
        placement: Option<Placement>,
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
    list: ListWindow<Row>,
    /// The rows that were on screen when the list was read again, by
    /// position, drawn until their pages land so a list sync touches does
    /// not go blank for a moment each time.
    shown: std::collections::HashMap<u32, Row>,
    paging: Paging,
    /// The row the keyboard is on.
    cursor: u32,
    /// The first row in view.
    top: u32,
    /// What the host aims this frontend's commands with; the client sends
    /// a snapshot of it with each one (ADR 0041).
    state: postio_core::SharedState,
    /// What is marked.
    selection: postio_ui::selection::SelectionState,
    /// The rows Focus's inbox surfaces among its conversations, in the order
    /// the host gave them.
    surfaced: Vec<FocusRow>,
    /// Where they sit.
    spliced: Spliced,
    /// What the strip counts, once read.
    counts: Option<postio_client::protocol::FocusCounts>,
    /// The message the cursor stays on when the has-action filter swaps the
    /// list under it, until the new list's first page lands.
    keep: Option<postio_model::MessageId>,
    /// Which of Focus's features `config.toml` has in use.
    features: crate::places::Features,
    /// The accounts whose connection has been heard of.
    tracked: Vec<postio_model::AccountId>,
    /// When mail last arrived, as far as is known.
    last_synced: Option<chrono::DateTime<chrono::Utc>>,
    /// The list being shown.
    scope: Option<ListScope>,
    /// The lists opened before this one, newest last, for `prev_view`.
    history: Vec<ListScope>,
    /// The next open is `prev_view` going back, not a new step forward.
    going_back: bool,
    /// What the status line says about the last thing done: the undo offer,
    /// a refusal, an error.
    notice: Option<String>,
    /// What kind of news the notice is, for its mark and colour.
    notice_tone: Tone,
    /// The key the notice offers undo on, when it offers it.
    notice_undo: Option<String>,
    /// Which notice is on the line: a timer is for one of them.
    notice_generation: u64,
    /// An answer to an invitation was sent, and its toast is the next one:
    /// it lasts as long as the reply waits.
    answering: bool,
    /// Where the keyboard is.
    focus: Focus,
    /// The folders, views and saved searches the finder and `g o` read.
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

/// The surfaced rows of Focus's inbox and where they sit: what turns the
/// store's conversations into a page of positions.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Placement {
    /// The rows, in the order the host gave them.
    pub surfaced: Vec<FocusRow>,
    /// Their positions.
    pub spliced: Spliced,
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

/// How many lists `prev_view` can go back through.
const HISTORY: usize = 32;

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
            shown: std::collections::HashMap::new(),
            paging: Paging::default(),
            cursor: 0,
            top: 0,
            state: postio_core::SharedState::default(),
            selection: postio_ui::selection::SelectionState::new(),
            surfaced: Vec::new(),
            spliced: Spliced::default(),
            counts: None,
            keep: None,
            features: crate::places::Features::default(),
            tracked: Vec::new(),
            last_synced: None,
            scope: None,
            notice: None,
            notice_tone: Tone::Plain,
            notice_undo: None,
            notice_generation: 0,
            answering: false,
            focus: Focus::List,
            places: crate::places::Places::default(),
            privacy: None,
            renaming: None,
            trackers: postio_ui::status::Trackers::default(),
            account: None,
            folders: Vec::new(),
            history: Vec::new(),
            going_back: false,
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
        self.list.row_of(message)
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
        postio_ui::selection::summary(&self.selection.selection(), Some(self.total()), &[])
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
                    let message = self.list.peek(position);
                    match (ctrl, shift, message) {
                        (true, _, Some(_)) => {
                            if let Some(message) = self.selectable(position) {
                                self.selection.toggle(message);
                            }
                        }
                        (false, true, Some(_)) => {
                            // Every row from the anchor -- where the marking
                            // started, else the cursor -- to the one clicked.
                            let anchor = self
                                .selection
                                .anchor()
                                .and_then(|anchor| self.list.position_of(anchor))
                                .unwrap_or(self.cursor);
                            if self.selection.anchor().is_none()
                                && let Some(start) = self.cursor_message()
                            {
                                self.selection.select_only(start);
                            }
                            let (from, to) = (anchor.min(position), anchor.max(position));
                            let over: Vec<_> =
                                (from..=to).filter_map(|at| self.list.peek(at)).collect();
                            self.selection.extend_over(over);
                        }
                        _ => self.move_to(position),
                    }
                    vec![Effect::Redraw]
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
                // A row's drawn answer is its key, for that row: an answer
                // to an invitation is about the invitation wherever the
                // cursor is; the rest take the cursor there first.
                Target::RowAction(position, id) => {
                    let Some(message) = self.row_at(position).map(|row| row.id) else {
                        return Vec::new();
                    };
                    match id.parse::<postio_core::CommandId>() {
                        Ok(
                            command @ (postio_core::CommandId::AcceptInvite
                            | postio_core::CommandId::DeclineInvite),
                        ) => self.answer(message, command),
                        _ => {
                            self.focus = Focus::List;
                            self.move_to(position);
                            self.command(id)
                        }
                    }
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
                self.open_there(if inbox {
                    ListScope::Focus(postio_model::FocusScope::Inbox)
                } else {
                    ListScope::Mailbox(mailbox)
                })
            }
            Destination::Label(_) => self.open_bar(&format!("label:\"{name}\"")),
            Destination::Search(query) => self.open_bar(&query),
            Destination::Outbox(account) => self.open_there(ListScope::Outbox(account)),
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
        let row = self.list.row_of(message).cloned().or_else(|| {
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
            .filter_map(|message| self.list.row_of(*message))
            .collect();
        if rows.is_empty() {
            self.cursor_message()
                .and_then(|message| self.list.row_of(message))
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
            return Some("Every conversation".to_owned());
        }
        let rows = self.aimed_rows();
        let first = rows.first()?;
        Some(postio_ui::pickers::target(
            rows.len(),
            first.from.as_str(),
            first.subject.as_str(),
        ))
    }

    /// `s`, `h`, `l` or `m`: the picker, over the row the keyboard is on.
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

    fn folders_reach(&self) -> crate::folders::Reach<'_> {
        crate::folders::Reach {
            folders: &self.folders,
            filtered_today: self
                .features
                .filtering
                .then(|| self.counts.map_or(0, |counts| counts.filtered_today)),
        }
    }

    /// `g o`, or a click on the strip's place.
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
            "bold" | "italic" | "bullet_list" | "numbered_list" | "quote_block" | "insert_link" => {
                use crate::composer::Format;
                let format = match id {
                    "bold" => Format::Bold,
                    "italic" => Format::Italic,
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
        postio_ui::focus_state::sync_label_here(
            &self.trackers.statuses(&self.tracked),
            self.last_synced,
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
        self.list
            .resident_at(position)
            .or_else(|| self.shown.get(&position))
    }

    /// Drop what is cached so the rows in view are read again, keeping the
    /// rows on screen to draw until their pages land.
    fn read_again(&mut self) {
        let end = (self.top + self.fit_from(self.top)).min(self.list.total());
        self.shown = (self.top..end)
            .filter_map(|position| self.row_at(position).map(|row| (position, row.clone())))
            .collect();
        self.list.invalidate();
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

    /// The effect that reads `page`, with the surfaced rows it is placed
    /// among when this is Focus's inbox.
    fn fetch_of(&self, generation: u64, page: u32) -> Option<Effect> {
        let fetch = self.paging.fetch_for(page)?;
        let placement = (self.splices() && matches!(fetch, Fetch::Scope(_))).then(|| Placement {
            surfaced: self.surfaced.clone(),
            spliced: self.spliced.clone(),
        });
        Some(Effect::Fetch {
            generation,
            page,
            fetch,
            placement,
        })
    }

    /// Ask for every page in view that is neither here nor on its way.
    fn fetches(&mut self) -> Vec<Effect> {
        let end = (self.top + self.fit_from(self.top)).min(self.list.total());
        let mut wanted = Vec::new();
        for position in self.top..end {
            if let Some(postio_ui::list::Lookup::Missing { request }) = self.list.row_at(position) {
                wanted.extend(request);
            }
        }
        let generation = self.list.generation();
        wanted
            .into_iter()
            .filter_map(|page| self.fetch_of(generation, page))
            .collect()
    }

    /// Whether the list on screen has surfaced rows spliced among its
    /// conversations: Focus's own inbox, and not a search's results.
    fn splices(&self) -> bool {
        self.scope == Some(ListScope::Focus(postio_model::FocusScope::Inbox))
    }

    /// How many conversations the store holds for the list on screen, which
    /// is its length without the rows spliced among them.
    fn stored_total(&self) -> u32 {
        let spliced = if self.splices() {
            self.spliced.len()
        } else {
            0
        };
        self.list.total().saturating_sub(spliced)
    }

    /// The length of the list on screen over `stored` conversations.
    fn total_over(&self, stored: u32) -> u32 {
        if self.splices() {
            self.spliced.total(stored)
        } else {
            stored
        }
    }

    /// Whether the list on screen is Focus's inbox, for what its strip
    /// counts.
    fn in_focus(&self) -> bool {
        matches!(self.scope, Some(ListScope::Focus(_)))
    }

    /// The message at `position`, when its row is here and may be selected:
    /// a digest stands for many messages and selection skips it.
    fn selectable(&self, position: u32) -> Option<postio_model::MessageId> {
        self.row_at(position)
            .filter(|row| row.kind != crate::row::Kind::Digest)
            .map(|row| row.id)
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
        let last = self.list.total().saturating_sub(1);
        match id {
            "find_in_message" => return self.open_find(),
            "find_next" => return self.find_step(true),
            "find_previous" => return self.find_step(false),
            "next_message" if self.focus == Focus::Reader => return self.step_open(1),
            "prev_message" if self.focus == Focus::Reader => return self.step_open(-1),
            "next_message" => self.move_to(self.cursor.saturating_add(1)),
            "prev_message" => self.move_to(self.cursor.saturating_sub(1)),
            "first_message" => self.move_to(0),
            "last_message" => self.move_to(last),
            "toggle_selection" => {
                if let Some(message) = self.selectable(self.cursor) {
                    self.selection.toggle(message);
                }
            }
            "extend_selection_down" | "extend_selection_up" => {
                let anchor = self.cursor_message();
                if self.selection.anchor().is_none()
                    && let Some(anchor) = anchor
                {
                    self.selection.select_only(anchor);
                }
                let next = if id == "extend_selection_down" {
                    self.cursor.saturating_add(1)
                } else {
                    self.cursor.saturating_sub(1)
                };
                self.move_to(next);
                if let Some(message) = self.cursor_message() {
                    self.selection.extend_to(message);
                }
            }
            "select_all" => self
                .selection
                .select_all(postio_ui::selection::Reach::default()),
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
            "toggle_has_action" => return self.toggle_has_action(),
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
            "unsubscribe" => {
                let reading = self.reading.as_ref();
                return reading
                    .and_then(|reading| reading.members.get(reading.current))
                    .map(|member| vec![Effect::Unsubscribe(member.id)])
                    .unwrap_or_default();
            }
            "next_in_conversation" => return self.walk_conversation(1),
            "prev_in_conversation" => return self.walk_conversation(-1),
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
            // Escape clears the selection.
            "back" => self.selection.clear(),
            // One toggle in a terminal: reader view is the readable form of
            // bulk mail here, and both commands move between it and the
            // sender's own markup (spec 006 FR-031).
            "view_original" | "toggle_reader_view" => return self.view_original(),
            // One message is shown at a time, so there is none to fold to
            // its header.
            "toggle_fold" => {
                return self
                    .say("The open message shows one message; [ and ] step through the thread");
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
            "saved_search_1" => return self.pinned_search(0),
            "saved_search_2" => return self.pinned_search(1),
            "saved_search_3" => return self.pinned_search(2),
            "saved_search_4" => return self.pinned_search(3),
            "prev_view" => {
                let Some(scope) = self.history.pop() else {
                    return self.say("There is no earlier view");
                };
                self.going_back = true;
                return self.open_there(scope);
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

    /// `!`: narrow Focus's inbox to the rows with a marker, or back. The
    /// selection goes, since what it named may not be shown; the cursor
    /// stays on the same message when that message is still shown.
    fn toggle_has_action(&mut self) -> Vec<Effect> {
        use postio_model::FocusScope;
        let scope = match self.scope {
            Some(ListScope::Focus(FocusScope::Inbox)) => FocusScope::HasAction,
            Some(ListScope::Focus(FocusScope::HasAction)) => FocusScope::Inbox,
            _ => return Vec::new(),
        };
        self.keep = self.cursor_message();
        self.selection.clear();
        // Narrowing is not going somewhere else: `prev_view` skips it.
        self.going_back = true;
        vec![Effect::Open(ListScope::Focus(scope)), Effect::Redraw]
    }

    /// Open `scope`.
    fn open_there(&mut self, scope: ListScope) -> Vec<Effect> {
        vec![Effect::Open(scope), Effect::Redraw]
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
            // Focus's inbox is every account's, as one.
            MailboxRole::Inbox => Some(ListScope::Focus(postio_model::FocusScope::Inbox)),
            role => self
                .folders
                .iter()
                .find(|folder| folder.account_id == account && folder.role == role)
                .map(|folder| ListScope::Mailbox(folder.id)),
        };
        match scope {
            Some(scope) => self.open_there(scope),
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
        self.open_there(scopes[next])
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
            effects.push(Effect::Open(ListScope::Focus(
                postio_model::FocusScope::Inbox,
            )));
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
            Event::ActionCompleted {
                description,
                undoable,
            } => {
                // An answer's Undo works while its reply waits, so its toast
                // stays exactly that long (FR-102).
                let after = if std::mem::take(&mut self.answering) {
                    postio_session::actions::RSVP_WINDOW
                } else {
                    TOAST
                };
                let key = self
                    .keys
                    .key_for(KeyContext::List, "undo")
                    .filter(|_| *undoable);
                return self.say_for(Tone::Worked, description, key, after);
            }
            Event::UndoPerformed { description } => {
                return self.say_as(Tone::Worked, description, None);
            }
            Event::CommandRejected { reason, .. } => {
                self.answering = false;
                return self.say_as(Tone::Failed, reason, None);
            }
            Event::Error { message, .. } => return self.say_as(Tone::Failed, message, None),
            _ => {}
        }
        if moved {
            return vec![Effect::Redraw];
        }
        if matches!(event, Event::MailboxesChanged { .. }) {
            return vec![Effect::RefreshPlaces];
        }
        // What is surfaced may have moved with the mail -- an archived
        // reminder's row goes with its conversation -- so it is read again
        // whenever the list is.
        let rereads = self.splices()
            && (matches!(event, Event::SurfacedChanged)
                || self.paging.plan(event) != postio_ui::paging::Plan::Ignore);
        let mut effects = match self.paging.plan(event) {
            postio_ui::paging::Plan::Ignore => Vec::new(),
            postio_ui::paging::Plan::InsertAtTop(count) => {
                if self.list.inserted_at_top(count) {
                    // The rows under the cursor moved down; follow them.
                    self.cursor = self.cursor.saturating_add(count);
                    self.top = self.top.saturating_add(count);
                }
                vec![Effect::Redraw]
            }
            postio_ui::paging::Plan::Refetch(messages) => {
                let generation = self.list.generation();
                let pages = self.list.pages_holding(messages);
                let pending: Vec<u32> = pages
                    .into_iter()
                    .filter(|page| self.list.note_pending(*page))
                    .collect();
                pending
                    .into_iter()
                    .filter_map(|page| self.fetch_of(generation, page))
                    .collect()
            }
            postio_ui::paging::Plan::Reload => self
                .scope
                .map(|scope| vec![Effect::Recount(scope)])
                .unwrap_or_default(),
        };
        if rereads {
            effects.push(Effect::ReadSurfaced);
        }
        effects
    }

    /// Focus's surfaced rows were read: place them, and read the list again
    /// under them.
    fn surfaced_read(&mut self, read: &[postio_model::listing::Surfaced]) -> Vec<Effect> {
        let stored = self.stored_total();
        let mut rows = Vec::new();
        let mut positions = Vec::new();
        for surfaced in read {
            if let Some(row) = FocusRow::surfaced(surfaced) {
                positions.push(surfaced.position());
                rows.push(row);
            }
        }
        self.surfaced = rows;
        self.spliced = Spliced::new(&positions);
        if !self.splices() {
            // Read for the inbox that is about to open.
            return Vec::new();
        }
        self.read_again();
        let _ = self.list.set_total(self.spliced.total(stored));
        self.reveal();
        vec![Effect::Redraw]
    }

    /// A list was counted again after it changed: keep the scroll, drop what
    /// is cached, and let the rows in view be read again.
    fn recounted(&mut self, scope: ListScope, total: u32) -> Vec<Effect> {
        if self.scope != Some(scope) {
            return Vec::new();
        }
        self.read_again();
        let _ = self.list.set_total(self.total_over(total));
        self.move_to(self.cursor);
        let mut effects = vec![Effect::Redraw];
        effects.extend(self.in_focus().then_some(Effect::ReadFocusCounts));
        effects
    }

    /// A list opened: show it from the top.
    fn open(&mut self, scope: ListScope, total: u32) -> Vec<Effect> {
        self.paging.open(scope);
        // Another list's rows are not this one's, however briefly.
        self.shown.clear();
        if let Some(previous) = self.scope.filter(|previous| *previous != scope) {
            if std::mem::take(&mut self.going_back) {
                // Back is a step back, not another one forward.
            } else {
                self.history.push(previous);
                if self.history.len() > HISTORY {
                    self.history.remove(0);
                }
            }
        }
        self.scope = Some(scope);
        self.account = self.account_of(scope);
        // A selection is relative to the list it was made in.
        self.selection.clear();
        if !self.splices() {
            self.surfaced.clear();
            self.spliced = Spliced::default();
        }
        self.list.reset(self.total_over(total));
        self.cursor = 0;
        self.top = 0;
        let mut effects = vec![Effect::Redraw];
        effects.extend(self.in_focus().then_some(Effect::ReadFocusCounts));
        effects
    }

    /// A page arrived, or did not.
    fn page(&mut self, generation: u64, page: u32, rows: Result<Page<Row>, String>) -> Vec<Effect> {
        match rows {
            Ok(rows) => {
                // The count travels with every page: the rows and the total
                // are one read, so each page corrects the total the list was
                // opened with -- for its own generation only.
                if generation == self.list.generation() {
                    let _ = self.list.set_total(rows.total);
                }
                let delivered = self.list.deliver(generation, page, rows.rows);
                let (list, total) = (&self.list, self.list.total());
                self.shown.retain(|position, _| {
                    *position < total && list.resident_at(*position).is_none()
                });
                if delivered.stale {
                    Vec::new()
                } else {
                    // The cursor goes back to the message `!` left it on.
                    if let Some(message) = self.keep.take()
                        && let Some(position) = self.list.position_of(message)
                    {
                        self.move_to(position);
                    }
                    // Rows that landed may be taller than the guess the view
                    // was placed by.
                    self.reveal();
                    let mut effects = vec![Effect::Redraw];
                    effects.extend(self.in_focus().then_some(Effect::ReadFocusCounts));
                    effects
                }
            }
            Err(reason) => {
                tracing::debug!(page, "a page did not arrive: {reason}");
                self.list.abandon(generation, page);
                Vec::new()
            }
        }
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
        Input::Opened { scope, total } => app.open(scope, total),
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
        Input::Unsubscribed(answer) => app.say(&match answer {
            Ok(list) => format!("Asked to leave {list}"),
            Err(reason) => reason,
        }),
        Input::Addressed { message, to, cc } => {
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
        Input::Surfaced(read) => app.surfaced_read(&read),
        Input::FocusCounts(counts) => {
            app.counts = Some(counts);
            vec![Effect::Redraw]
        }
        Input::Recounted { scope, total } => app.recounted(scope, total),
        Input::Page {
            generation,
            page,
            rows,
        } => app.page(generation, page, rows),
    };
    // A message open follows the cursor wherever it went: `j`, a click, or
    // the row it was opened from leaving.
    effects.extend(app.follow_cursor());
    effects.extend(app.fetches());
    effects.extend(app.filtered_fetches());
    effects.extend(app.hold_paint(&effects));
    effects
}

#[cfg(test)]
pub(crate) mod tests {
    use chrono::{TimeZone, Utc};
    use crossterm::event::{KeyCode, KeyEventKind, KeyEventState, KeyModifiers};
    use postio_model::{MailboxId, MessageId};

    use super::*;
    use crate::test_support::{
        alt, app, click, key, open_list, places, press, reader_text, row, serve, type_text, wheel,
    };

    #[test]
    fn a_resize_changes_what_is_shown_and_asks_the_host_nothing() {
        let mut app = app((160, 40));

        let effects = update(&mut app, Input::Resize(40, 30));

        assert_eq!(effects, vec![Effect::Redraw], "a redraw and nothing else");
        assert!(!app.fits(), "below 50 columns the window does not fit");

        update(&mut app, Input::Resize(160, 40));
        assert!(app.fits(), "widening brings it back");
    }

    #[test]
    fn the_list_holds_as_many_rows_as_fit_between_the_strip_and_the_bottom_line() {
        // 42 rows: the top bar, the strip and the bottom line take three,
        // the first day's heading one, and each plain row is one line.
        let mut app = app((160, 42));
        let opening = open_list(&mut app, 100);
        serve(&mut app, opening);
        assert_eq!(
            app.visible().len(),
            38,
            "the rows drawn are the rows that fit"
        );
    }

    #[test]
    fn rows_with_a_marker_take_two_lines_and_fewer_of_them_fit() {
        use crate::test_support::{conversation, local, marked, show_focus};
        use postio_model::listing::{MarkerKind, MarkerSummary};
        use postio_ui::focus_list::FocusRow;
        let marker = MarkerSummary {
            kind: MarkerKind::Question,
            when: None,
            excerpt: Some("Can you?".into()),
            answer: None,
            cancelled: false,
        };
        let mut app = app((60, 12));
        let rows = (0..20)
            .map(|id| {
                FocusRow::conversation(marked(
                    conversation(id + 1, "Ada", "Question", "", local(23, 9, 0)),
                    marker.clone(),
                ))
            })
            .collect();
        show_focus(&mut app, rows);
        // Nine lines for the list: a heading, then two lines to a row.
        assert_eq!(app.visible().len(), 4);
        for _ in 0..10 {
            update(&mut app, press('j'));
        }
        assert_eq!(app.cursor(), 10);
        let visible = app.visible();
        assert!(
            visible.iter().any(|row| row.cursor),
            "the cursor stays in view as the view scrolls by rows of two lines"
        );
    }

    fn focus_inbox() -> ListScope {
        ListScope::Focus(postio_model::FocusScope::Inbox)
    }

    fn surfaced_digest(position: u32) -> postio_model::listing::Surfaced {
        postio_model::listing::Surfaced::Digest {
            delivery: postio_model::ids::DeliveryId::new(1),
            rule: "Newsletters".into(),
            cadence: None,
            count: 14,
            senders: Vec::new(),
            summary_line: None,
            at: Utc.with_ymd_and_hms(2026, 9, 20, 9, 0, 0).unwrap(),
            position,
        }
    }

    #[test]
    fn the_inbox_reads_its_counts_when_it_opens_and_when_a_page_lands() {
        let mut app = app((120, 30));
        let opened = update(
            &mut app,
            Input::Opened {
                scope: focus_inbox(),
                total: 3,
            },
        );
        assert!(opened.contains(&Effect::ReadFocusCounts), "{opened:?}");
        let fetch = opened
            .iter()
            .find_map(|effect| match effect {
                Effect::Fetch {
                    generation, page, ..
                } => Some((*generation, *page)),
                _ => None,
            })
            .expect("the first page is asked for");
        let landed = update(
            &mut app,
            Input::Page {
                generation: fetch.0,
                page: fetch.1,
                rows: Ok(Page {
                    total: 3,
                    rows: vec![row(0), row(1), row(2)],
                }),
            },
        );
        assert!(landed.contains(&Effect::ReadFocusCounts), "{landed:?}");
        // A folder is not Focus's inbox: nothing to count.
        let folder = open_list(&mut app, 3);
        assert!(!folder.contains(&Effect::ReadFocusCounts), "{folder:?}");
    }

    #[test]
    fn surfaced_rows_are_placed_among_the_conversations_and_ride_with_each_fetch() {
        let mut app = app((120, 30));
        update(
            &mut app,
            Input::Opened {
                scope: focus_inbox(),
                total: 2,
            },
        );
        assert_eq!(app.total(), 2);
        let effects = update(&mut app, Input::Surfaced(vec![surfaced_digest(1)]));
        assert_eq!(app.total(), 3, "the digest is a row of the list");
        let placement = effects.iter().find_map(|effect| match effect {
            Effect::Fetch { placement, .. } => placement.clone(),
            _ => None,
        });
        assert_eq!(
            placement.map(|placement| placement.spliced.len()),
            Some(1),
            "the page is read with the digest's place: {effects:?}"
        );
        // A folder is not spliced into.
        let folder = open_list(&mut app, 4);
        assert_eq!(app.total(), 4);
        assert!(
            folder.iter().all(|effect| !matches!(
                effect,
                Effect::Fetch {
                    placement: Some(_),
                    ..
                }
            )),
            "{folder:?}"
        );
    }

    #[test]
    fn surfaced_rows_are_read_again_only_for_the_inbox_that_shows_them() {
        use postio_core::Event;
        let mut app = app((120, 30));
        update(
            &mut app,
            Input::Opened {
                scope: focus_inbox(),
                total: 2,
            },
        );
        let effects = update(&mut app, Input::Host(Event::SurfacedChanged));
        assert!(effects.contains(&Effect::ReadSurfaced), "{effects:?}");
        open_list(&mut app, 2);
        let effects = update(&mut app, Input::Host(Event::SurfacedChanged));
        assert!(!effects.contains(&Effect::ReadSurfaced), "{effects:?}");
    }

    #[test]
    fn selection_skips_a_digest() {
        let mut app = app((120, 30));
        update(
            &mut app,
            Input::Opened {
                scope: focus_inbox(),
                total: 1,
            },
        );
        let effects = update(&mut app, Input::Surfaced(vec![surfaced_digest(0)]));
        let digest = postio_ui::focus_list::FocusRow::surfaced(&surfaced_digest(0)).unwrap();
        crate::test_support::serve_with(&mut app, effects, |position| match position {
            0 => crate::row::Row::from(digest.clone()),
            other => row(other),
        });
        assert_eq!(
            app.row_at(0).map(|row| row.kind),
            Some(crate::row::Kind::Digest)
        );
        update(&mut app, press('x'));
        assert!(
            app.selection().selection().is_empty(),
            "x on a digest marks nothing"
        );
        update(&mut app, press('j'));
        update(&mut app, press('x'));
        assert!(
            !app.selection().selection().is_empty(),
            "x on a message marks it"
        );
    }

    #[test]
    fn opening_a_list_asks_only_for_the_pages_in_view() {
        let mut app = app((120, 30));
        let effects = open_list(&mut app, 100_000);
        let pages: Vec<u32> = effects
            .iter()
            .filter_map(|effect| match effect {
                Effect::Fetch { page, .. } => Some(*page),
                _ => None,
            })
            .collect();
        assert!(!pages.is_empty(), "the first rows are asked for");
        assert!(
            pages.iter().all(|page| *page <= 1),
            "only the top: {pages:?}"
        );
    }

    #[test]
    fn walking_a_hundred_thousand_rows_reads_only_what_passes_through_view() {
        // Principle V as a count. A keystroke costs at most one page and the
        // page after it -- `ListWindow`'s read-ahead, which the desktop list
        // shares, so a fast scroll does not stall at a boundary -- and
        // walking 500 rows reads the pages those rows are on and no more.
        let mut app = app((120, 30));
        let opening = open_list(&mut app, 100_000);
        serve(&mut app, opening);
        let mut reads = 0;
        for _ in 0..500 {
            let effects = update(&mut app, press('j'));
            let fetches = effects
                .iter()
                .filter(|effect| matches!(effect, Effect::Fetch { .. }))
                .count();
            assert!(fetches <= 2, "one keystroke, {fetches} reads");
            reads += serve(&mut app, effects);
        }
        assert_eq!(app.cursor(), 500);
        // 500 rows and a screenful are eleven pages of fifty, plus the one
        // read ahead.
        assert!(reads <= 12, "{reads} page reads for 500 rows");
    }

    #[test]
    fn the_cursor_stops_at_either_end() {
        let mut app = app((120, 30));
        let opening = open_list(&mut app, 3);
        serve(&mut app, opening);
        update(&mut app, press('k'));
        assert_eq!(app.cursor(), 0);
        for _ in 0..10 {
            update(&mut app, press('j'));
        }
        assert_eq!(app.cursor(), 2);
        update(&mut app, press('g'));
        update(&mut app, press('g'));
        assert_eq!(app.cursor(), 0, "g g is the first row");
        update(&mut app, press('G'));
        assert_eq!(app.cursor(), 2, "G is the last");
    }

    fn mark(app: &mut App, position: u32) {
        while app.cursor() < position {
            update(app, press('j'));
        }
        update(app, press('x'));
    }

    #[test]
    fn a_verb_acts_on_what_is_marked_not_where_the_cursor_is() {
        // US1 scenario 3: three marked, the cursor on a fourth.
        let mut app = app((120, 30));
        let opening = open_list(&mut app, 10);
        serve(&mut app, opening);
        for position in [1, 2, 3] {
            mark(&mut app, position);
        }
        update(&mut app, press('j'));
        assert_eq!(app.cursor(), 4);

        let effects = update(&mut app, press('a'));

        let sent: Vec<_> = effects
            .iter()
            .filter(|effect| matches!(effect, Effect::Send(_)))
            .collect();
        assert_eq!(sent.len(), 1, "{effects:?}");
        let marked: Vec<MessageId> = [1, 2, 3].map(|position| row(position).id).to_vec();
        assert_eq!(
            app.state()
                .read(|state| state.resolve(&postio_core::MessageTarget::Selection)),
            Some(postio_core::Resolved::Messages(marked)),
            "the host would archive the three marked rows"
        );
    }

    #[test]
    fn with_nothing_marked_a_verb_acts_on_the_cursor_row() {
        let mut app = app((120, 30));
        let opening = open_list(&mut app, 10);
        serve(&mut app, opening);
        update(&mut app, press('j'));
        update(&mut app, press('a'));
        assert_eq!(
            app.state()
                .read(|state| state.resolve(&postio_core::MessageTarget::Selection)),
            Some(postio_core::Resolved::Messages(vec![row(1).id]))
        );
    }

    #[test]
    fn marked_rows_are_drawn_as_marked() {
        let mut app = app((120, 30));
        let opening = open_list(&mut app, 10);
        serve(&mut app, opening);
        mark(&mut app, 2);
        let visible = app.visible();
        assert!(visible[2].selected);
        assert!(!visible[1].selected);
    }

    #[test]
    fn an_undoable_action_is_announced_and_ctrl_z_sends_undo() {
        let mut app = app((120, 30));
        let opening = open_list(&mut app, 10);
        serve(&mut app, opening);
        update(
            &mut app,
            Input::Host(postio_core::Event::ActionCompleted {
                description: "Archived 12 messages".into(),
                undoable: true,
            }),
        );
        assert_eq!(app.notice(), Some("Archived 12 messages"));
        assert_eq!(app.notice_undo(), Some("ctrl+z"));

        let effects = update(&mut app, key(KeyCode::Char('z'), KeyModifiers::CONTROL));
        assert!(
            effects.contains(&Effect::Send(postio_core::Command::Undo)),
            "{effects:?}"
        );
        update(
            &mut app,
            Input::Host(postio_core::Event::UndoPerformed {
                description: "Unarchived 12 messages".into(),
            }),
        );
        assert_eq!(app.notice(), Some("Unarchived 12 messages"));
    }

    fn expiring(effects: &[Effect]) -> Vec<(u64, std::time::Duration)> {
        effects
            .iter()
            .filter_map(|effect| match effect {
                Effect::ExpireNotice { generation, after } => Some((*generation, *after)),
                _ => None,
            })
            .collect()
    }

    fn archived(app: &mut App, description: &str) -> Vec<Effect> {
        update(
            app,
            Input::Host(postio_core::Event::ActionCompleted {
                description: description.into(),
                undoable: true,
            }),
        )
    }

    #[test]
    fn a_toast_stays_eight_seconds_and_ctrl_z_undoes_after_it_has_gone() {
        let mut app = app((120, 30));
        let opening = open_list(&mut app, 10);
        serve(&mut app, opening);
        let effects = archived(&mut app, "Archived 3 messages");
        let [(generation, after)] = expiring(&effects)[..] else {
            panic!("one timer for the toast: {effects:?}");
        };
        assert_eq!(after, std::time::Duration::from_secs(8));
        assert_eq!(
            after.as_secs(),
            u64::from(postio_ui::focus_target::TOAST_SECONDS)
        );

        update(&mut app, Input::NoticeDue { generation });
        assert_eq!(app.notice(), None, "gone after its time");
        let effects = update(&mut app, key(KeyCode::Char('z'), KeyModifiers::CONTROL));
        assert!(
            effects.contains(&Effect::Send(postio_core::Command::Undo)),
            "the host keeps the stack: {effects:?}"
        );
    }

    #[test]
    fn a_newer_toast_replaces_the_one_before_and_the_old_timer_does_not_take_it_down() {
        let mut app = app((120, 30));
        let opening = open_list(&mut app, 10);
        serve(&mut app, opening);
        let first = expiring(&archived(&mut app, "Archived 1 message"))[0].0;
        let second = expiring(&archived(&mut app, "Archived 2 messages"))[0].0;
        assert_ne!(first, second);
        update(&mut app, Input::NoticeDue { generation: first });
        assert_eq!(app.notice(), Some("Archived 2 messages"));
        update(&mut app, Input::NoticeDue { generation: second });
        assert_eq!(app.notice(), None);
    }

    #[test]
    fn an_answer_to_an_invitation_keeps_its_undo_for_as_long_as_the_reply_waits() {
        let mut app = app((120, 30));
        let opening = open_list(&mut app, 10);
        serve(&mut app, opening);
        let sent = update(&mut app, press('y'));
        assert!(
            sent.iter().any(|effect| matches!(effect, Effect::Send(_))),
            "{sent:?}"
        );
        let effects = archived(&mut app, "Accepted Harbor design review");
        assert_eq!(
            expiring(&effects)[0].1,
            postio_session::actions::RSVP_WINDOW,
            "{effects:?}"
        );
        // The next toast is an ordinary one again.
        let effects = archived(&mut app, "Archived 1 message");
        assert_eq!(expiring(&effects)[0].1, std::time::Duration::from_secs(8));
    }

    #[test]
    fn a_list_the_host_changed_is_counted_again_and_reread() {
        let mut app = app((120, 30));
        let opening = open_list(&mut app, 10);
        serve(&mut app, opening);
        let scope = ListScope::Mailbox(MailboxId::new(1));
        let effects = update(
            &mut app,
            Input::Host(postio_core::Event::MessageListChanged {
                account: postio_model::AccountId::new(1),
                mailbox: MailboxId::new(1),
            }),
        );
        assert!(effects.contains(&Effect::Recount(scope)), "{effects:?}");

        let effects = update(&mut app, Input::Recounted { scope, total: 7 });
        assert_eq!(app.total(), 7);
        assert!(
            effects
                .iter()
                .any(|effect| matches!(effect, Effect::Fetch { page: 0, .. })),
            "the rows in view are read again: {effects:?}"
        );
    }

    #[test]
    fn in_the_composer_a_letter_is_typed_not_run() {
        // T053: `a` is Archive in the list and a letter in the composer.
        let mut app = app((160, 40));
        let opening = open_list(&mut app, 3);
        serve(&mut app, opening);
        app.compose(postio_model::Draft::new(postio_model::AccountId::new(1)));
        assert_eq!(app.focus(), Focus::Composer);

        let effects = update(&mut app, press('a'));
        assert!(
            !effects
                .iter()
                .any(|effect| matches!(effect, Effect::Send(_))),
            "a letter ran a command: {effects:?}"
        );
        let composer = app.composer().expect("still composing");
        assert_eq!(composer.value(crate::composer::Field::To), "a");

        update(&mut app, key(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.focus(), Focus::List, "Escape leaves the composer");
        assert!(app.composer().is_none());
    }

    #[test]
    fn a_reply_opens_filled_and_escape_goes_back_to_the_same_row() {
        // US3 scenario 1.
        let mut app = app((160, 40));
        let opening = open_list(&mut app, 3);
        serve(&mut app, opening);
        update(&mut app, press('j'));
        let row = app.cursor();

        let effects = update(&mut app, press('e'));
        let asked = effects.iter().find_map(|effect| match effect {
            Effect::ReplySource { kind, message } => Some((*kind, *message)),
            _ => None,
        });
        assert_eq!(
            asked,
            Some((postio_body::replying::ReplyKind::Reply, MessageId::new(2))),
            "{effects:?}"
        );

        let found = crate::composer::tests::a_message_and_its_account();
        update(
            &mut app,
            Input::ReplySource {
                kind: postio_body::replying::ReplyKind::Reply,
                found: Some(Box::new(found)),
            },
        );
        assert_eq!(app.focus(), Focus::Composer);
        let composer = app.composer().expect("composing");
        assert_eq!(
            composer.value(crate::composer::Field::To),
            "Ada <ada@example.com>"
        );
        assert_eq!(
            composer.value(crate::composer::Field::Subject),
            "Re: Tide gate"
        );

        update(&mut app, key(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.focus(), Focus::List);
        assert_eq!(app.cursor(), row, "back where they were");
    }

    #[test]
    fn a_new_message_starts_empty_from_the_account_on_screen() {
        let mut app = app((160, 40));
        update(&mut app, Input::Places(places()));
        let opening = open_list(&mut app, 3);
        serve(&mut app, opening);
        update(&mut app, press('c'));
        let composer = app.composer().expect("composing");
        assert_eq!(composer.field(), crate::composer::Field::To);
        assert_eq!(composer.draft().account_id, postio_model::AccountId::new(1));
    }

    #[test]
    fn copy_fields_raises_cc_and_bcc_in_the_composer() {
        let mut app = app((160, 40));
        composing(&mut app);
        app.composer_command("copy_fields");
        assert!(app.composer().unwrap().shows_extra_recipients());
    }

    #[test]
    fn new_mail_worth_telling_is_said_and_offered_to_the_desktop() {
        // T022: with no desktop app open the terminal is the one told; it
        // says so on the status line, and hands the same words to the
        // desktop's notification service where there is one.
        let mut app = app((160, 40));
        let effects = update(
            &mut app,
            Input::Notified(postio_ui::notify::Notification {
                identifier: "inbox-1".into(),
                title: "Grace Hopper".into(),
                body: "Tide gate\u{1b}[2J report".into(),
                mailbox: MailboxId::new(1),
                message: None,
            }),
        );
        let notice = app.notice().expect("said on the status line");
        assert!(notice.contains("Grace Hopper"), "{notice}");
        assert!(
            !notice.contains('\u{1b}'),
            "outside text is made safe: {notice:?}"
        );
        assert!(
            effects.iter().any(|effect| matches!(
                effect,
                Effect::DesktopNotify { title, .. } if title == "Grace Hopper"
            )),
            "{effects:?}"
        );
    }

    #[test]
    fn open_message_reads_the_row_and_puts_the_keyboard_in_the_reader() {
        // It fell through to the dispatcher, which answered that it was not
        // wired up; opening is the reader's own business.
        let mut app = app((160, 40));
        let opening = open_list(&mut app, 3);
        serve(&mut app, opening);
        let effects = app.command("open_message");
        assert!(
            !effects
                .iter()
                .any(|effect| matches!(effect, Effect::Send(_))),
            "{effects:?}"
        );
        assert!(
            effects.contains(&Effect::ReadBody(MessageId::new(1)))
                || app
                    .reading()
                    .is_some_and(|reading| reading.row == MessageId::new(1)),
            "the message under the cursor is what the reader shows: {effects:?}"
        );
        assert_eq!(app.focus(), Focus::Reader);
    }

    fn opens(effects: &[Effect]) -> Vec<ListScope> {
        effects
            .iter()
            .filter_map(|effect| match effect {
                Effect::Open(scope) => Some(*scope),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn the_go_to_keys_open_the_accounts_folder_with_that_role() {
        let mut app = app((160, 40));
        update(&mut app, Input::Places(places()));
        let opening = open_list(&mut app, 3);
        serve(&mut app, opening);
        let account = postio_model::AccountId::new(1);
        assert_eq!(
            opens(&app.command("go_to_flagged")),
            vec![ListScope::Flagged(account)]
        );
        update(
            &mut app,
            Input::Opened {
                scope: ListScope::Flagged(account),
                total: 0,
            },
        );
        assert_eq!(
            opens(&app.command("go_to_inbox")),
            vec![ListScope::Focus(postio_model::FocusScope::Inbox)],
            "the inbox is every account's, as one"
        );
        // No Sent folder in this account: said, not sent to nothing.
        let effects = app.command("go_to_sent");
        assert!(opens(&effects).is_empty());
        assert!(
            app.notice().is_some_and(|notice| notice.contains("Sent")),
            "{:?}",
            app.notice()
        );
    }

    #[test]
    fn g_r_and_g_z_open_the_archive_and_the_snoozed_view_as_the_desktop_does() {
        // specs/007-postio-focus T162: the one keymap's two new
        // destinations, by role, as the classic app's `act` reaches them.
        let mut app = app((160, 40));
        update(&mut app, Input::Places(places()));
        let opening = open_list(&mut app, 3);
        serve(&mut app, opening);
        let account = postio_model::AccountId::new(1);

        update(&mut app, press('g'));
        let effects = update(&mut app, press('r'));
        assert_eq!(opens(&effects), vec![ListScope::Mailbox(MailboxId::new(2))]);
        update(
            &mut app,
            Input::Opened {
                scope: ListScope::Mailbox(MailboxId::new(2)),
                total: 0,
            },
        );

        update(&mut app, press('g'));
        let effects = update(&mut app, press('z'));
        assert_eq!(opens(&effects), vec![ListScope::Snoozed(account)]);
    }

    #[test]
    fn previous_view_goes_back_where_the_list_was() {
        let mut app = app((160, 40));
        update(&mut app, Input::Places(places()));
        let opening = open_list(&mut app, 3);
        serve(&mut app, opening);
        let account = postio_model::AccountId::new(1);
        update(
            &mut app,
            Input::Opened {
                scope: ListScope::Flagged(account),
                total: 0,
            },
        );
        assert_eq!(
            opens(&app.command("prev_view")),
            vec![ListScope::Mailbox(MailboxId::new(1))]
        );
    }

    #[test]
    fn next_scope_walks_each_account_then_all_of_them() {
        use postio_model::mailbox::{Mailbox, MailboxRole};
        let mut contents = places();
        let mut second = contents.accounts[0].clone();
        second.id = postio_model::AccountId::new(2);
        second.address = postio_model::EmailAddress::new(None::<String>, "bea@example.com");
        let mut inbox = Mailbox::new(second.id, "INBOX", None);
        inbox.id = MailboxId::new(9);
        inbox.role = MailboxRole::Inbox;
        inbox.selectable = true;
        contents.accounts.push(second);
        contents.folders.push(inbox);
        let mut app = app((160, 40));
        update(&mut app, Input::Places(contents));
        let opening = open_list(&mut app, 3);
        serve(&mut app, opening);
        let mut walked = Vec::new();
        for _ in 0..3 {
            let next = opens(&app.command("next_scope"));
            assert_eq!(next.len(), 1, "{next:?}");
            update(
                &mut app,
                Input::Opened {
                    scope: next[0],
                    total: 0,
                },
            );
            walked.push(next[0]);
        }
        assert_eq!(
            walked,
            vec![
                ListScope::Mailbox(MailboxId::new(9)),
                ListScope::Unified,
                ListScope::Mailbox(MailboxId::new(1)),
            ]
        );
    }

    #[test]
    fn save_draft_saves_now_and_formatting_reaches_the_body() {
        let mut app = app((160, 40));
        addressed(&mut app, "Tide gate");
        assert_eq!(saves(&app.composer_command("save_draft")).len(), 1);
        app.composer
            .as_mut()
            .unwrap()
            .focus_field(crate::composer::Field::Body);
        let effects = app.composer_command("quote_block");
        assert_eq!(app.composer().unwrap().markdown(), "> Looking now.");
        assert!(
            effects
                .iter()
                .any(|effect| matches!(effect, Effect::Autosave { .. })),
            "{effects:?}"
        );
    }

    #[test]
    fn view_original_leaves_reader_view_for_the_senders_markup_and_back() {
        let mut campaign = String::from("<table><tr><td><table><tr><td>");
        for index in 0..14 {
            campaign.push_str(&format!(
                r##"<a href="https://example.com/{index}" style="color:#06c">shop</a>"##
            ));
        }
        campaign.push_str("</td></tr></table></td></tr></table>");
        let mut app = app((160, 40));
        let opening = open_list(&mut app, 3);
        serve(&mut app, opening);
        update(&mut app, key(KeyCode::Enter, KeyModifiers::NONE));
        let message = MessageId::new(1);
        update(
            &mut app,
            Input::Body {
                message,
                answer: Ok(postio_client::protocol::Body::Ready {
                    body: postio_model::MessageBody {
                        text: None,
                        html: Some(campaign),
                    },
                    encoding_problems: false,
                }),
            },
        );
        let drawn = |app: &App| app.reading().unwrap().members[0].reader_view;
        assert!(drawn(&app), "bulk mail opens in reader view");
        app.command("view_original");
        assert!(!drawn(&app), "the sender's own markup");
        app.command("view_original");
        assert!(drawn(&app), "and back");
    }

    /// Run `id` where its surface is, in an app with mail in the list, the
    /// first message open and, for the composer's commands, a draft.
    fn run_anywhere(id: &str, spec: &postio_core::registry::CommandSpec) -> Vec<Effect> {
        use postio_core::Context;
        let mut app = app((160, 40));
        update(&mut app, Input::Places(places()));
        let opening = open_list(&mut app, 3);
        serve(&mut app, opening);
        update(&mut app, key(KeyCode::Enter, KeyModifiers::NONE));
        if spec.contexts == Context::Composer.as_set() {
            addressed(&mut app, "Parity");
            return app.composer_command(id);
        }
        if spec.contexts == Context::Filtered.as_set() {
            app.go_to_filtered();
            return app.command(id);
        }
        if spec.contexts == Context::Digest.as_set() {
            // A digest's window, over the inbox's first row as a digest.
            let rows = vec![crate::test_support::digest_row(1, "Newsletters", 3, 1)];
            crate::test_support::show_focus(&mut app, rows);
            app.command("open_message");
            return app.command(id);
        }
        if spec.contexts == Context::Accounts.as_set() {
            update(&mut app, key(KeyCode::Char(','), KeyModifiers::ALT));
            update(&mut app, key(KeyCode::Tab, KeyModifiers::NONE));
            return app.settings_command(id);
        }
        app.command(id)
    }

    #[test]
    fn every_command_is_answered_here_or_by_the_dispatcher() {
        // The registry-parity suite proves every command has a key this
        // terminal can send and a palette row. This proves pressing it does
        // something: a command the terminal neither handles nor passes to a
        // handler reaches the dispatcher, which answers "not wired up".
        let mut wired: Vec<postio_core::CommandId> = postio_session::actions::WIRED.to_vec();
        wired.push(postio_core::CommandId::Refresh);
        let mut unanswered = Vec::new();
        let terminal = postio_core::Availability {
            frontend: postio_core::Frontend::Terminal,
            ..postio_core::Availability::open(postio_core::Scope::Unified)
        };
        for spec in postio_core::registry::all() {
            let id = spec.id.as_str();
            // Never offered here (`Requirement::Graphical`): owed no answer.
            if !spec.requires.met_by(terminal) {
                continue;
            }
            let effects = run_anywhere(id, spec);
            let dropped = effects
                .iter()
                .any(|effect| matches!(effect, Effect::Send(sent) if !wired.contains(&sent.id())));
            if dropped {
                unanswered.push(id);
            }
        }
        assert!(
            unanswered.is_empty(),
            "commands this terminal offers and sends to nothing: {unanswered:?}"
        );
    }

    fn composing(app: &mut App) {
        app.compose(postio_model::Draft::new(postio_model::AccountId::new(1)));
    }

    fn saves(effects: &[Effect]) -> Vec<postio_model::Draft> {
        effects
            .iter()
            .filter_map(|effect| match effect {
                Effect::SaveDraft { draft, .. } => Some((**draft).clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_draft_is_saved_once_the_typing_pauses() {
        // T056: each edit asks for a timer; only the newest one saves.
        let mut app = app((160, 40));
        composing(&mut app);
        let first = update(&mut app, press('a'));
        let second = update(&mut app, press('b'));
        let due = |effects: &[Effect]| {
            effects.iter().find_map(|effect| match effect {
                Effect::Autosave { generation, edit } => Some((*generation, *edit)),
                _ => None,
            })
        };
        let (generation, early) = due(&first).expect("a timer for the first edit");
        let (_, late) = due(&second).expect("and for the second");
        assert_eq!(super::AUTOSAVE, std::time::Duration::from_millis(1500));

        let stale = update(
            &mut app,
            Input::AutosaveDue {
                generation,
                edit: early,
            },
        );
        assert!(
            saves(&stale).is_empty(),
            "a later edit is coming: {stale:?}"
        );
        let fresh = update(
            &mut app,
            Input::AutosaveDue {
                generation,
                edit: late,
            },
        );
        let saved = saves(&fresh);
        assert_eq!(saved.len(), 1, "{fresh:?}");
        assert_eq!(saved[0].to[0].address, "ab");
    }

    #[test]
    fn escape_keeps_what_was_written_and_drops_what_was_not() {
        let mut app = app((160, 40));
        composing(&mut app);
        update(&mut app, press('a'));
        let effects = update(&mut app, key(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(saves(&effects).len(), 1, "written, so kept: {effects:?}");

        composing(&mut app);
        let effects = update(&mut app, key(KeyCode::Esc, KeyModifiers::NONE));
        assert!(saves(&effects).is_empty(), "{effects:?}");
        assert!(
            effects
                .iter()
                .any(|effect| matches!(effect, Effect::DiscardDraft { .. })),
            "untouched, so dropped: {effects:?}"
        );
    }

    #[test]
    fn quitting_while_writing_saves_the_draft_first() {
        // US3 scenario 5: closed without sending, the draft is in Drafts.
        let mut app = app((160, 40));
        composing(&mut app);
        update(&mut app, press('a'));
        let effects = update(&mut app, key(KeyCode::Char('q'), KeyModifiers::CONTROL));
        let saved = effects
            .iter()
            .position(|effect| matches!(effect, Effect::SaveDraft { .. }));
        let quit = effects.iter().position(|effect| *effect == Effect::Quit);
        assert!(
            matches!((saved, quit), (Some(saved), Some(quit)) if saved < quit),
            "{effects:?}"
        );
    }

    #[test]
    fn a_saved_draft_keeps_the_id_the_host_gave_it() {
        let mut app = app((160, 40));
        composing(&mut app);
        update(&mut app, press('a'));
        let generation = app.composer().expect("composing").generation();
        let id = postio_model::DraftId::new(41);
        update(
            &mut app,
            Input::DraftSaved {
                generation,
                saved: Ok(id),
            },
        );
        assert_eq!(app.composer().expect("composing").draft().id, id);

        // An answer for a composition that has since closed is not this one's.
        update(
            &mut app,
            Input::DraftSaved {
                generation: generation + 7,
                saved: Ok(postio_model::DraftId::new(99)),
            },
        );
        assert_eq!(app.composer().expect("composing").draft().id, id);
    }

    #[test]
    fn enter_on_a_row_in_drafts_reopens_the_draft() {
        use postio_model::mailbox::{Mailbox, MailboxRole};
        let mut app = app((160, 40));
        let mut contents = places();
        let mut drafts = Mailbox::new(postio_model::AccountId::new(1), "Drafts", None);
        drafts.id = MailboxId::new(9);
        drafts.role = MailboxRole::Drafts;
        drafts.selectable = true;
        contents.folders.push(drafts);
        update(&mut app, Input::Places(contents));
        let opening = update(
            &mut app,
            Input::Opened {
                scope: ListScope::Mailbox(MailboxId::new(9)),
                total: 2,
            },
        );
        crate::test_support::serve_with(&mut app, opening, |position| {
            let mut draft = row(position);
            draft.send_state = Some(postio_model::DraftState::Editing);
            draft
        });

        let effects = update(&mut app, key(KeyCode::Enter, KeyModifiers::NONE));
        assert!(
            effects.contains(&Effect::Resume(MessageId::new(1))),
            "{effects:?}"
        );
        let mut draft = postio_model::Draft::new(postio_model::AccountId::new(1));
        draft.id = postio_model::DraftId::new(5);
        draft.body_markdown = Some("Half **written**".into());
        update(
            &mut app,
            Input::Resumed {
                found: Some(Box::new(draft)),
                failure: None,
            },
        );
        let composer = app.composer().expect("composing");
        assert_eq!(composer.markdown(), "Half **written**");
        assert_eq!(composer.draft().id, postio_model::DraftId::new(5));
    }

    #[test]
    fn a_failed_draft_reopens_saying_why_it_was_not_sent() {
        // FR-066, as the desktop says it (#1487): the person has come back to
        // do something about it, so they are told what went wrong.
        let mut app = app((160, 40));
        let mut draft = postio_model::Draft::new(postio_model::AccountId::new(1));
        draft.state = postio_model::DraftState::Failed;
        update(
            &mut app,
            Input::Resumed {
                found: Some(Box::new(draft)),
                failure: Some("the server refused the recipient".into()),
            },
        );
        assert!(app.composer().is_some());
        assert_eq!(
            app.notice(),
            Some("Not sent — the server refused the recipient")
        );
    }

    fn addressed(app: &mut App, subject: &str) {
        let mut draft = postio_model::Draft::new(postio_model::AccountId::new(1));
        draft.to = vec![postio_model::EmailAddress::new(
            None::<String>,
            "grace@example.net",
        )];
        draft.subject = subject.into();
        draft.body_markdown = Some("Looking now.".into());
        app.compose(draft);
    }

    fn sends(effects: &[Effect]) -> Vec<Option<chrono::DateTime<Utc>>> {
        effects
            .iter()
            .filter_map(|effect| match effect {
                Effect::QueueSend { at, .. } => Some(*at),
                _ => None,
            })
            .collect()
    }

    fn ctrl_return() -> Input {
        key(KeyCode::Enter, KeyModifiers::CONTROL)
    }

    #[test]
    fn the_composers_send_button_and_alt_s_both_send() {
        let mut app = app((160, 40));
        addressed(&mut app, "Tide gate");
        let clicked = update(
            &mut app,
            click(
                crate::view::hit::Target::ComposerAction("send"),
                false,
                false,
            ),
        );
        assert_eq!(sends(&clicked), vec![None], "{clicked:?}");

        let mut app = app_with_draft_for_alt_s();
        let pressed = update(&mut app, key(KeyCode::Char('s'), KeyModifiers::ALT));
        assert_eq!(sends(&pressed), vec![None], "{pressed:?}");
    }

    fn app_with_draft_for_alt_s() -> App {
        let mut app = app((160, 40));
        addressed(&mut app, "Tide gate");
        app
    }

    #[test]
    fn sending_queues_the_draft_and_closes_without_discarding_it() {
        let mut app = app((160, 40));
        addressed(&mut app, "Tide gate");
        let effects = update(&mut app, ctrl_return());
        assert_eq!(sends(&effects), vec![None], "{effects:?}");
        assert!(
            !effects.iter().any(|effect| matches!(
                effect,
                Effect::DiscardDraft { .. } | Effect::SaveDraft { .. }
            )),
            "the send is the write: {effects:?}"
        );
        assert!(app.composer().is_none());
        assert_eq!(app.focus(), Focus::List);
    }

    #[test]
    fn the_legacy_alternate_sends_too() {
        let mut app = app((160, 40));
        addressed(&mut app, "Tide gate");
        let effects = update(&mut app, key(KeyCode::Enter, KeyModifiers::ALT));
        assert_eq!(sends(&effects), vec![None], "{effects:?}");
    }

    #[test]
    fn a_message_to_nobody_is_not_sent() {
        let mut app = app((160, 40));
        composing(&mut app);
        let effects = update(&mut app, ctrl_return());
        assert!(sends(&effects).is_empty());
        assert_eq!(app.notice(), Some(postio_ui::sending::NO_RECIPIENTS));
        assert!(app.composer().is_some(), "still writing");
    }

    #[test]
    fn a_message_with_no_subject_asks_once_then_sends() {
        let mut app = app((160, 40));
        addressed(&mut app, "");
        let asked = update(&mut app, ctrl_return());
        assert!(sends(&asked).is_empty(), "{asked:?}");
        let notice = app.notice().expect("a question").to_owned();
        assert!(notice.contains("no subject"), "{notice}");
        assert!(app.composer().is_some());

        let effects = update(&mut app, ctrl_return());
        assert_eq!(
            sends(&effects),
            vec![None],
            "asked and answered: {effects:?}"
        );
    }

    #[test]
    fn a_scheduled_send_offers_the_desktops_times() {
        let mut app = app((160, 40));
        addressed(&mut app, "Tide gate");
        update(
            &mut app,
            key(KeyCode::Enter, KeyModifiers::CONTROL | KeyModifiers::SHIFT),
        );
        let offered = app.scheduling().expect("the picker").to_vec();
        assert_eq!(offered.len(), 4);
        assert_eq!(offered[2].0, "Tomorrow morning");

        let effects = update(&mut app, press('3'));
        let at = offered[2].1.with_timezone(&Utc);
        assert_eq!(sends(&effects), vec![Some(at)], "{effects:?}");
        assert!(app.scheduling().is_none());
        assert!(app.composer().is_none());
    }

    #[test]
    fn escape_from_the_schedule_picker_keeps_writing() {
        let mut app = app((160, 40));
        addressed(&mut app, "Tide gate");
        update(
            &mut app,
            key(KeyCode::Enter, KeyModifiers::CONTROL | KeyModifiers::SHIFT),
        );
        update(&mut app, key(KeyCode::Esc, KeyModifiers::NONE));
        assert!(app.scheduling().is_none());
        assert!(app.composer().is_some());
    }

    #[test]
    fn the_status_line_says_where_a_send_went() {
        let mut app = app((160, 40));
        update(
            &mut app,
            Input::Queued {
                at: None,
                queued: Ok(()),
            },
        );
        assert_eq!(
            app.notice(),
            Some("Sending — it is in the Outbox until it leaves")
        );
        update(
            &mut app,
            Input::Queued {
                at: None,
                queued: Err("the store is busy".into()),
            },
        );
        assert_eq!(
            app.notice(),
            Some("Not sent: the store is busy. The draft is still in Drafts.")
        );
    }

    fn ada() -> postio_model::contact_group::RecipientCandidate {
        postio_model::contact_group::RecipientCandidate::Contact(postio_model::EmailAddress::new(
            Some("Ada"),
            "ada@example.com",
        ))
    }

    #[test]
    fn typing_a_recipient_offers_the_contacts_it_could_be() {
        // T055, at the desktop's threshold of four characters (#424).
        let mut app = app((160, 40));
        composing(&mut app);
        let effects = type_text(&mut app, "ada@");
        let asked: Vec<_> = effects
            .iter()
            .filter_map(|effect| match effect {
                Effect::Recipients { prefix, .. } => Some(prefix.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(asked, vec!["ada@".to_owned()], "three letters ask nothing");

        update(
            &mut app,
            Input::Recipients {
                prefix: "ada@".into(),
                found: vec![ada()],
            },
        );
        let composer = app.composer().expect("composing");
        assert_eq!(composer.suggestions(), &[ada()]);

        update(&mut app, key(KeyCode::Tab, KeyModifiers::NONE));
        let composer = app.composer().expect("composing");
        assert_eq!(
            composer.value(crate::composer::Field::To),
            "Ada <ada@example.com>, "
        );
        assert!(composer.suggestions().is_empty());
        assert_eq!(
            composer.field(),
            crate::composer::Field::To,
            "room for another"
        );
    }

    #[test]
    fn an_answer_for_what_is_no_longer_typed_is_not_offered() {
        let mut app = app((160, 40));
        composing(&mut app);
        type_text(&mut app, "ada@e");
        update(
            &mut app,
            Input::Recipients {
                prefix: "ada@".into(),
                found: vec![ada()],
            },
        );
        assert!(app.composer().expect("composing").suggestions().is_empty());
    }

    #[test]
    fn escape_puts_suggestions_away_before_it_leaves() {
        let mut app = app((160, 40));
        composing(&mut app);
        type_text(&mut app, "ada@");
        update(
            &mut app,
            Input::Recipients {
                prefix: "ada@".into(),
                found: vec![ada()],
            },
        );
        update(&mut app, key(KeyCode::Esc, KeyModifiers::NONE));
        let composer = app.composer().expect("still composing");
        assert!(composer.suggestions().is_empty());
    }

    fn attaching(effects: &[Effect]) -> Vec<std::path::PathBuf> {
        effects
            .iter()
            .filter_map(|effect| match effect {
                Effect::Attach(path) => Some(path.clone()),
                _ => None,
            })
            .collect()
    }

    fn in_the_body(app: &mut App) {
        update(app, key(KeyCode::Tab, KeyModifiers::NONE));
        update(app, key(KeyCode::Tab, KeyModifiers::NONE));
    }

    #[test]
    fn dropping_files_attaches_each_and_leaves_the_body_alone() {
        // US3 scenario 8: a drop arrives as a paste of the files' paths.
        let dir = tempfile::tempdir().unwrap();
        let (one, two) = (
            dir.path().join("plan.pdf"),
            dir.path().join("site photo.jpg"),
        );
        std::fs::write(&one, b"%PDF-").unwrap();
        std::fs::write(&two, b"\xff\xd8\xff").unwrap();
        let mut app = app((160, 40));
        composing(&mut app);
        in_the_body(&mut app);
        type_text(&mut app, "See attached");

        let dropped = format!("{} '{}'", one.display(), two.display());
        let effects = update(&mut app, Input::Paste(dropped));
        assert_eq!(attaching(&effects), vec![one, two], "{effects:?}");
        assert_eq!(app.composer().unwrap().markdown(), "See attached");
    }

    #[test]
    fn pasted_words_are_typed() {
        // US3 scenario 10: not a path to a file, so it is text.
        let mut app = app((160, 40));
        composing(&mut app);
        in_the_body(&mut app);
        let effects = update(&mut app, Input::Paste("see /etc/ for details".into()));
        assert!(attaching(&effects).is_empty());
        assert_eq!(app.composer().unwrap().markdown(), "see /etc/ for details");
    }

    #[test]
    fn a_path_that_cannot_be_read_is_named_and_changes_nothing() {
        // US3 scenario 11.
        let dir = tempfile::tempdir().unwrap();
        let mut app = app((160, 40));
        composing(&mut app);
        in_the_body(&mut app);
        let before = app.composer().unwrap().draft();
        let effects = update(&mut app, Input::Paste(dir.path().display().to_string()));
        assert!(attaching(&effects).is_empty());
        let notice = app.notice().expect("told").to_owned();
        assert!(
            notice.contains(
                &dir.path()
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            ),
            "{notice}"
        );
        let after = app.composer().unwrap().draft();
        assert_eq!(after.body, before.body);
        assert_eq!(after.attachments, before.attachments);
    }

    #[test]
    fn a_stored_file_is_listed_on_the_draft() {
        let mut app = app((160, 40));
        composing(&mut app);
        let mut stored =
            postio_model::Attachment::new(MessageId::UNASSIGNED, "application/pdf", 12_288);
        stored.filename = Some("fixture.pdf".into());
        let effects = update(
            &mut app,
            Input::Attached {
                path: "fixture.pdf".into(),
                attached: Some(stored.clone()),
            },
        );
        let composer = app.composer().unwrap();
        assert_eq!(composer.draft().attachments, vec![stored]);
        assert!(
            effects
                .iter()
                .any(|effect| matches!(effect, Effect::Autosave { .. })),
            "an attachment is an edit: {effects:?}"
        );

        update(
            &mut app,
            Input::Attached {
                path: "/gone/away.pdf".into(),
                attached: None,
            },
        );
        assert!(
            app.notice().unwrap().contains("away.pdf"),
            "{:?}",
            app.notice()
        );
    }

    #[test]
    fn a_file_can_be_attached_by_typing_its_path() {
        // T064 (FR-027): a path prompt, with Tab completing from the disk.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("fixture.pdf"), b"%PDF-").unwrap();
        let mut app = app((160, 40));
        composing(&mut app);
        update(&mut app, key(KeyCode::Char('a'), KeyModifiers::ALT));
        assert_eq!(app.path_prompt(), Some(""), "the prompt is open");

        type_text(&mut app, &format!("{}/fix", dir.path().display()));
        update(&mut app, key(KeyCode::Tab, KeyModifiers::NONE));
        let completed = dir.path().join("fixture.pdf").display().to_string();
        assert_eq!(app.path_prompt(), Some(completed.as_str()));

        let effects = update(&mut app, key(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(attaching(&effects), vec![dir.path().join("fixture.pdf")]);
        assert_eq!(app.path_prompt(), None);
        assert_eq!(
            app.composer().unwrap().markdown(),
            "",
            "the path was not typed into the draft"
        );
    }

    #[test]
    fn the_body_goes_to_the_editor_and_comes_back_with_nothing_else_changed() {
        // US3 scenario 6.
        let mut app = app((160, 40));
        addressed(&mut app, "Tide gate");
        let before = app.composer().unwrap().draft();
        let effects = update(&mut app, key(KeyCode::Char('e'), KeyModifiers::ALT));
        let (generation, handed) = effects
            .iter()
            .find_map(|effect| match effect {
                Effect::EditExternally {
                    generation,
                    markdown,
                } => Some((*generation, markdown.clone())),
                _ => None,
            })
            .expect("handed to the editor");
        assert_eq!(handed, "Looking now.");

        let effects = update(
            &mut app,
            Input::Edited {
                generation,
                edited: Ok("Looking now.\n\nFound it.".into()),
            },
        );
        let after = app.composer().unwrap().draft();
        assert_eq!(
            after.body_markdown.as_deref(),
            Some("Looking now.\n\nFound it.")
        );
        assert_eq!((after.to, after.subject), (before.to, before.subject));
        assert!(
            effects
                .iter()
                .any(|effect| matches!(effect, Effect::Autosave { .. })),
            "{effects:?}"
        );

        update(
            &mut app,
            Input::Edited {
                generation,
                edited: Err("vi exited with 1".into()),
            },
        );
        assert!(app.notice().unwrap().contains("vi exited with 1"));
        assert_eq!(
            app.composer().unwrap().markdown(),
            "Looking now.\n\nFound it.",
            "a failed edit changes nothing"
        );
    }

    #[test]
    fn a_detached_draft_keeps_its_id_and_escape_brings_the_frame_back() {
        // FR-196: Detach gives the composer the whole screen.
        let mut app = app((160, 40));
        let opening = open_list(&mut app, 3);
        serve(&mut app, opening);
        let mut draft = postio_model::Draft::new(postio_model::AccountId::new(1));
        draft.id = postio_model::DraftId::new(5);
        draft.to = vec![postio_model::EmailAddress::new(
            None::<String>,
            "grace@example.net",
        )];
        app.compose(draft);
        let generation = app.composer().unwrap().generation();

        update(&mut app, alt('o'));
        assert!(app.composer_detached());
        assert_eq!(app.focus(), Focus::Composer);

        let effects = update(&mut app, key(KeyCode::Esc, KeyModifiers::NONE));
        assert!(!app.composer_detached(), "Escape brings it back");
        assert_eq!(app.focus(), Focus::Composer, "still writing");
        assert!(saves(&effects).is_empty(), "{effects:?}");
        let composer = app.composer().expect("the draft is still open");
        assert_eq!(composer.draft().id, postio_model::DraftId::new(5));
        assert_eq!(composer.generation(), generation);
        assert_eq!(app.front(), Front::Composer);

        update(&mut app, alt('o'));
        update(&mut app, alt('o'));
        assert!(
            !app.composer_detached(),
            "the same key detaches and attaches"
        );
    }

    fn reads_the_clipboard(effects: &[Effect]) -> usize {
        effects
            .iter()
            .filter(|effect| matches!(effect, Effect::ReadClipboardImage))
            .count()
    }

    #[test]
    fn typing_never_reads_the_clipboard() {
        // T062 / FR-026: only the paste key does.
        let mut app = app((160, 40));
        composing(&mut app);
        in_the_body(&mut app);
        let effects = type_text(&mut app, "Here is the photo: ");
        assert_eq!(reads_the_clipboard(&effects), 0);
        let effects = update(&mut app, Input::Paste("some words".into()));
        assert_eq!(
            reads_the_clipboard(&effects),
            0,
            "a text paste is not a read"
        );
    }

    #[test]
    fn a_pasted_image_goes_in_where_the_cursor_is() {
        // US3 scenario 9.
        let mut app = app((160, 40));
        composing(&mut app);
        in_the_body(&mut app);
        type_text(&mut app, "Photo: ");
        let effects = update(&mut app, alt('g'));
        assert_eq!(reads_the_clipboard(&effects), 1, "{effects:?}");

        let png = b"\x89PNG\r\n\x1a\nrest".to_vec();
        let effects = update(&mut app, Input::ClipboardImage(Ok(Some(png.clone()))));
        assert!(
            effects.contains(&Effect::InlineImage {
                bytes: png,
                mime_type: "image/png".into()
            }),
            "{effects:?}"
        );

        let mut stored = postio_model::Attachment::new(MessageId::UNASSIGNED, "image/png", 12);
        stored.disposition = postio_model::attachment::Disposition::Inline;
        stored.content_id = Some("abc@postio.invalid".into());
        update(&mut app, Input::InlineStored(Some(stored.clone())));
        let composer = app.composer().unwrap();
        assert_eq!(
            composer.markdown(),
            "Photo: ![image](cid:abc@postio.invalid)"
        );
        assert_eq!(composer.attachments(), &[stored]);
        let html = composer.draft().body.html.expect("an image is HTML");
        assert!(html.contains("cid:abc@postio.invalid"), "{html}");
    }

    #[test]
    fn no_clipboard_says_so_and_no_image_says_so() {
        let mut app = app((160, 40));
        composing(&mut app);
        update(
            &mut app,
            Input::ClipboardImage(Err(crate::clipboard::UNAVAILABLE.into())),
        );
        assert_eq!(app.notice(), Some(crate::clipboard::UNAVAILABLE));
        update(&mut app, Input::ClipboardImage(Ok(None)));
        assert_eq!(app.notice(), Some("There is no image on the clipboard"));
    }

    #[test]
    fn a_click_moves_the_cursor_and_ctrl_or_shift_select_without_moving_the_reader() {
        // US5 scenario 1.
        use crate::view::hit::Target;
        let mut app = app((160, 40));
        let opening = open_list(&mut app, 8);
        serve(&mut app, opening);

        let effects = update(&mut app, click(Target::Row(2), false, false));
        assert_eq!(app.cursor(), 2);
        assert_eq!(app.focus(), Focus::List);
        assert_eq!(reads(&effects), 0, "a click on a row reads nothing");

        update(&mut app, click(Target::Row(4), false, true));
        assert_eq!(
            app.cursor(),
            2,
            "shift-click leaves the cursor, and the reader, alone"
        );
        for position in [2, 3, 4] {
            assert!(
                app.selection().contains(MessageId::new(position + 1)),
                "shift-click extends from the cursor over {position}"
            );
        }

        update(&mut app, click(Target::Row(6), true, false));
        assert_eq!(
            app.cursor(),
            2,
            "ctrl-click leaves the cursor, and the reader, alone"
        );
        assert!(app.selection().contains(MessageId::new(7)));
        assert!(
            app.selection().contains(MessageId::new(3)),
            "and keeps what was marked"
        );
    }

    #[test]
    fn the_wheel_scrolls_the_pane_under_the_pointer_and_nothing_else() {
        // US5 scenario 2.
        use crate::view::hit::Target;
        let mut app = app((160, 20));
        let opening = open_list(&mut app, 200);
        serve(&mut app, opening);
        let reading = crate::conversation::Reading {
            row: MessageId::new(1),
            members: vec![crate::conversation::tests::member_with_lines(1, 120)],
            current: 0,
            own: None,
        };
        app.reading = Some(reading);

        update(&mut app, wheel(Target::Reader(Some(0)), true));
        assert!(app.reader_top() > 0, "the reader scrolled");
        assert_eq!(app.top(), 0, "the list did not");

        let reader_top = app.reader_top();
        update(&mut app, wheel(Target::Row(0), true));
        assert!(app.top() > 0, "the list scrolled");
        assert_eq!(app.reader_top(), reader_top, "the reader did not");
    }

    fn reading_of(body: crate::reader::Rendered) -> crate::conversation::Reading {
        let mut member = crate::conversation::tests::member_with_lines(1, 0);
        member.body = Some(body);
        crate::conversation::Reading {
            row: MessageId::new(1),
            members: vec![member],
            current: 0,
            own: None,
        }
    }

    fn line_of(app: &App, wanted: &str) -> usize {
        reader_text(app)
            .lines()
            .position(|line| line.contains(wanted))
            .unwrap_or_else(|| panic!("no line with {wanted}"))
    }

    #[test]
    fn a_click_on_a_fold_marker_expands_it() {
        // T073.
        use crate::view::hit::Target;
        let mut app = app((160, 40));
        app.reading = Some(reading_of(crate::reader::from_text(
            "Sounds good.\n> earlier\n> words",
        )));
        let marker = line_of(&app, "quoted line");
        update(&mut app, click(Target::Reader(Some(marker)), false, false));
        let text = reader_text(&app);
        assert!(text.contains("earlier"), "{text}");
    }

    #[test]
    fn a_link_opens_only_on_a_second_click() {
        // US2 scenario 4 and T045: the first click shows where it goes.
        use crate::view::hit::Target;
        let mut app = app((160, 40));
        app.reading = Some(reading_of(crate::reader::from_html(
            "<p>See <a href=\"https://example.com/report\">the report</a>.</p>",
        )));
        let at = line_of(&app, "example.com/report");
        let first = update(&mut app, click(Target::Reader(Some(at)), false, false));
        assert!(
            !first
                .iter()
                .any(|effect| matches!(effect, Effect::OpenLink(_))),
            "{first:?}"
        );
        assert!(
            app.notice().unwrap().contains("https://example.com/report"),
            "the whole destination is shown first: {:?}",
            app.notice()
        );
        let second = update(&mut app, click(Target::Reader(Some(at)), false, false));
        assert!(
            second.contains(&Effect::OpenLink("https://example.com/report".into())),
            "{second:?}"
        );
    }

    #[test]
    fn a_click_in_the_body_puts_the_cursor_there() {
        // T075: column 5 of body line 2.
        let mut app = app((160, 40));
        let mut draft = postio_model::Draft::new(postio_model::AccountId::new(1));
        draft.body_markdown = Some("First line\nSecond line here\nThird".into());
        app.compose(draft);
        update(
            &mut app,
            Input::Pointer(Pointer::Click {
                hit: crate::view::hit::Hit {
                    target: crate::view::hit::Target::ComposerBody,
                    column: 5,
                    row: 1,
                },
                ctrl: false,
                shift: false,
            }),
        );
        let composer = app.composer().unwrap();
        assert_eq!(composer.field(), crate::composer::Field::Body);
        assert_eq!(composer.body().cursor(), (1, 5));
    }

    #[test]
    fn with_the_mouse_off_clicks_do_nothing_and_every_key_still_works() {
        // US5 scenario 3.
        use crate::view::hit::Target;
        let mut app = app((160, 40)).with_mouse(false);
        let opening = open_list(&mut app, 8);
        serve(&mut app, opening);
        let effects = update(&mut app, click(Target::Row(4), false, false));
        assert!(effects.is_empty(), "{effects:?}");
        assert_eq!(app.cursor(), 0);
        update(&mut app, wheel(Target::Row(0), true));
        assert_eq!(app.top(), 0);

        update(&mut app, press('j'));
        update(&mut app, press('j'));
        assert_eq!(app.cursor(), 2, "the keys are untouched");
        update(&mut app, press('x'));
        assert!(app.selection().contains(MessageId::new(3)));
    }

    fn an_empty_store() -> crate::places::Places {
        crate::places::Places::default()
    }

    fn discovered_settings() -> postio_ui::onboarding::Settings {
        postio_ui::onboarding::Settings {
            imap: postio_ui::onboarding::Server {
                host: "imap.example.test".into(),
                port: 993,
                security: postio_model::TransportSecurity::Tls,
            },
            smtp: postio_ui::onboarding::Server {
                host: "smtp.example.test".into(),
                port: 465,
                security: postio_model::TransportSecurity::Tls,
            },
            login: "ada@example.test".into(),
            source: "Fastmail".into(),
            ..Default::default()
        }
    }

    #[test]
    fn with_no_account_the_first_screen_offers_to_add_one() {
        // US7 scenario 1.
        let mut app = app((160, 40));
        update(&mut app, Input::Places(an_empty_store()));
        assert_eq!(app.focus(), Focus::FirstRun);
        assert!(app.first_run().is_some());
    }

    #[test]
    fn an_account_is_found_proved_and_saved_from_the_first_run() {
        // US7 and T085.
        use postio_ui::onboarding::{Status, SyncWindow};
        let mut app = app((160, 40));
        update(&mut app, Input::Places(an_empty_store()));
        type_text(&mut app, "ada@example.test");
        let effects = update(&mut app, key(KeyCode::Enter, KeyModifiers::NONE));
        assert!(
            effects.contains(&Effect::Discover("ada@example.test".into())),
            "{effects:?}"
        );
        assert_eq!(app.first_run().unwrap().status(), &Status::Probing);

        update(
            &mut app,
            Input::Discovered(Ok(Status::Found(discovered_settings()))),
        );
        type_text(&mut app, "correct horse");
        let effects = update(&mut app, key(KeyCode::Enter, KeyModifiers::NONE));
        let submitted = effects
            .iter()
            .find_map(|effect| match effect {
                Effect::AddAccount(submission) => Some((**submission).clone()),
                _ => None,
            })
            .expect("submitted");
        assert_eq!(submitted.address, "ada@example.test");
        assert_eq!(submitted.password, "correct horse");
        assert_eq!(submitted.settings, discovered_settings());
        assert_eq!(app.first_run().unwrap().status(), &Status::Connecting);

        // Refused: said as the desktop says it, and the password can be typed
        // again.
        update(
            &mut app,
            Input::AccountAdded(Err("The server rejected that.".into())),
        );
        assert_eq!(
            app.first_run().unwrap().status().message(),
            Some("The server rejected that.")
        );
        type_text(&mut app, "!");
        let effects = update(&mut app, key(KeyCode::Enter, KeyModifiers::NONE));
        assert!(
            effects
                .iter()
                .any(|effect| matches!(effect, Effect::AddAccount(_)))
        );

        update(&mut app, Input::AccountAdded(Ok(())));
        assert_eq!(app.first_run().unwrap().status(), &Status::SyncWindow);
        let effects = update(&mut app, press('1'));
        assert!(
            effects.contains(&Effect::SaveSyncWindow(SyncWindow::LastMonth)),
            "{effects:?}"
        );
        assert!(effects.contains(&Effect::RefreshPlaces), "{effects:?}");
        assert!(app.first_run().is_none(), "on to the mail");
    }

    #[test]
    fn once_there_is_an_account_its_inbox_opens() {
        let mut app = app((160, 40));
        update(&mut app, Input::Places(an_empty_store()));
        let effects = update(&mut app, Input::Places(places()));
        assert!(
            effects.contains(&Effect::Open(ListScope::Focus(
                postio_model::FocusScope::Inbox
            ))),
            "{effects:?}"
        );
        assert!(app.first_run().is_none());
    }

    #[test]
    fn a_second_account_is_added_from_the_mail_and_can_be_left() {
        use postio_ui::onboarding::Status;
        let mut app = app((160, 40));
        update(&mut app, Input::Places(places()));
        let opening = open_list(&mut app, 3);
        serve(&mut app, opening);

        update(&mut app, key(KeyCode::Char('n'), KeyModifiers::ALT));
        assert_eq!(app.focus(), Focus::FirstRun);
        let run = app.first_run().expect("asking for the account");
        assert_eq!(run.status(), &Status::Idle);
        assert!(!run.repairing(), "a new account, not the one there");

        // Mail keeps arriving while the address is typed; it does not close
        // what was asked for.
        type_text(&mut app, "grace@example.test");
        update(&mut app, Input::Places(places()));
        assert_eq!(app.focus(), Focus::FirstRun, "still asking");

        update(&mut app, key(KeyCode::Esc, KeyModifiers::NONE));
        assert!(app.first_run().is_none());
        assert_eq!(app.focus(), Focus::List);
    }

    fn in_settings(app: &mut App) {
        update(app, Input::Places(places()));
        let opening = open_list(app, 3);
        serve(app, opening);
        update(app, key(KeyCode::Char(','), KeyModifiers::ALT));
        assert_eq!(app.focus(), Focus::Settings);
    }

    #[test]
    fn the_privacy_section_reads_its_log_and_revokes_an_allowed_sender() {
        use postio_ui::settings::Section;
        let mut app = app((160, 40));
        in_settings(&mut app);
        app.allowlist.allow("news@example.com");
        let mut effects = Vec::new();
        while app.settings().expect("open").current() != Section::Privacy {
            effects = update(&mut app, key(KeyCode::Down, KeyModifiers::NONE));
        }
        assert!(effects.contains(&Effect::ReadPrivacy), "{effects:?}");
        update(
            &mut app,
            Input::Privacy {
                log: postio_client::protocol::PrivacyLog {
                    activations: Vec::new(),
                    read_receipts: 2,
                },
                connections: Vec::new(),
            },
        );
        assert_eq!(app.privacy().expect("read").log.read_receipts, 2);

        update(&mut app, key(KeyCode::Tab, KeyModifiers::NONE));
        assert!(app.settings().unwrap().in_list(), "on the senders");
        let effects = update(&mut app, key(KeyCode::Delete, KeyModifiers::NONE));
        assert!(
            effects.iter().any(|effect| matches!(
                effect,
                Effect::SaveAllowlist(list) if !list.is_allowed("news@example.com")
            )),
            "{effects:?}"
        );
        assert!(!app.allowlist.is_allowed("news@example.com"));
    }

    #[test]
    fn an_accounts_signatures_are_edited_added_renamed_and_deleted() {
        use postio_model::{Signature, SignatureId};
        let mut app = app((160, 40));
        let mut contents = places();
        let account = contents.accounts[0].id;
        let mut work = Signature::new("Work", "Ada\nThe Engine Room");
        work.id = SignatureId::new(5);
        contents.accounts[0].signatures = vec![work];
        update(&mut app, Input::Places(contents));
        let opening = open_list(&mut app, 3);
        serve(&mut app, opening);
        update(&mut app, key(KeyCode::Char(','), KeyModifiers::ALT));
        update(&mut app, key(KeyCode::Tab, KeyModifiers::NONE));

        update(&mut app, press('s'));
        assert_eq!(app.settings().unwrap().signatures_of(), Some(account));

        // Enter: the text, in the person's editor.
        let effects = update(&mut app, key(KeyCode::Enter, KeyModifiers::NONE));
        assert!(
            effects.contains(&Effect::EditSignature {
                account,
                signature: Some(SignatureId::new(5)),
                name: "Work".into(),
                text: "Ada\nThe Engine Room".into(),
            }),
            "{effects:?}"
        );

        // n: a name, then the editor on nothing yet.
        update(&mut app, press('n'));
        assert_eq!(app.focus(), Focus::Palette);
        type_text(&mut app, "Home");
        let effects = update(&mut app, key(KeyCode::Enter, KeyModifiers::NONE));
        assert!(
            effects.contains(&Effect::EditSignature {
                account,
                signature: None,
                name: "Home".into(),
                text: String::new(),
            }),
            "{effects:?}"
        );
        assert_eq!(app.focus(), Focus::Settings);

        // r: the name it has, changed; the text stays.
        update(&mut app, press('r'));
        assert_eq!(app.palette().expect("asking").query, "Work");
        for _ in 0.."Work".len() {
            update(&mut app, key(KeyCode::Backspace, KeyModifiers::NONE));
        }
        type_text(&mut app, "Office");
        let effects = update(&mut app, key(KeyCode::Enter, KeyModifiers::NONE));
        assert!(
            effects.contains(&Effect::SaveSignature {
                account,
                signature: Some(SignatureId::new(5)),
                name: "Office".into(),
                text: "Ada\nThe Engine Room".into(),
            }),
            "{effects:?}"
        );

        // A refusal is said, in the store's own sentence for a person.
        update(
            &mut app,
            Input::SignatureSaved(Err("There is already a signature called Office".into())),
        );
        assert_eq!(
            app.notice(),
            Some("There is already a signature called Office")
        );

        // The keymap's remove key asks first, naming itself, and deletes
        // when pressed again (T162: it was a hard-coded `d`, which the
        // keymap could neither rebind nor show). A letter deletes nothing.
        let effects = update(&mut app, press('d'));
        let effects = [effects, update(&mut app, press('d'))].concat();
        assert!(!effects.contains(&Effect::DeleteSignature(SignatureId::new(5))));
        let effects = update(&mut app, key(KeyCode::Delete, KeyModifiers::NONE));
        assert!(!effects.contains(&Effect::DeleteSignature(SignatureId::new(5))));
        assert!(
            app.notice()
                .is_some_and(|notice| notice.contains("Press Delete again")),
            "{:?}",
            app.notice()
        );
        let effects = update(&mut app, key(KeyCode::Delete, KeyModifiers::NONE));
        assert!(
            effects.contains(&Effect::DeleteSignature(SignatureId::new(5))),
            "{effects:?}"
        );

        // Escape goes back to the accounts, still in the settings.
        update(&mut app, key(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.settings().unwrap().signatures_of(), None);
        assert!(app.settings().unwrap().in_list());
    }

    #[test]
    fn settings_list_every_section_the_desktop_does() {
        // T087: enumerated from postio_ui::settings, not listed by hand.
        let mut app = app((160, 40));
        in_settings(&mut app);
        let shown: Vec<&str> = app
            .settings()
            .expect("open")
            .sections()
            .iter()
            .map(|section| section.label())
            .collect();
        let expected: Vec<&str> = postio_ui::settings::Section::ALL
            .iter()
            .map(|section| section.label())
            .collect();
        assert_eq!(shown, expected);
    }

    #[test]
    fn enter_on_a_section_edits_the_file_there() {
        let mut app = app((160, 40));
        in_settings(&mut app);
        update(&mut app, key(KeyCode::Down, KeyModifiers::NONE));
        let effects = update(&mut app, key(KeyCode::Enter, KeyModifiers::NONE));
        assert!(
            effects.contains(&Effect::EditConfig(Some(
                postio_ui::settings::Section::Filters
            ))),
            "{effects:?}"
        );
    }

    #[test]
    fn map_mailbox_role_asks_which_role_and_which_folder() {
        // ADR 0035's role map, from the terminal: `M` asks rather than
        // sending half a command, the roles are the desktop's, in its order,
        // and "Automatic" is first among the folders as it is there.
        use postio_model::mailbox::MailboxRole;
        let mut app = app((160, 40));
        in_settings(&mut app);
        update(&mut app, key(KeyCode::Tab, KeyModifiers::NONE));
        let effects = update(&mut app, press('M'));
        assert!(
            !effects
                .iter()
                .any(|effect| matches!(effect, Effect::Send(_))),
            "{effects:?}"
        );
        let roles: Vec<String> = app
            .palette()
            .expect("the role picker")
            .rows
            .into_iter()
            .map(|row| row.title)
            .collect();
        assert_eq!(roles, ["Sent", "Archive", "Drafts", "Trash", "Junk"]);

        type_text(&mut app, "arch");
        update(&mut app, key(KeyCode::Enter, KeyModifiers::NONE));
        let folders: Vec<String> = app
            .palette()
            .expect("the folder picker")
            .rows
            .into_iter()
            .map(|row| row.title)
            .collect();
        assert_eq!(folders[0], "Automatic");
        assert!(folders.contains(&"Archive".to_owned()), "{folders:?}");

        type_text(&mut app, "archive");
        let effects = update(&mut app, key(KeyCode::Enter, KeyModifiers::NONE));
        assert!(
            effects.contains(&Effect::Send(postio_core::Command::MapMailboxRole {
                account: Some(postio_model::AccountId::new(1)),
                role: Some(MailboxRole::Archive),
                path: Some("Archive".into()),
            })),
            "{effects:?}"
        );
        assert_eq!(app.focus(), Focus::Settings, "back where it was asked");
    }

    #[test]
    fn the_account_commands_change_the_account_under_the_cursor() {
        use postio_client::protocol::AccountOp;
        let mut app = app((160, 40));
        in_settings(&mut app);
        let account = postio_model::AccountId::new(1);
        // Into the account list.
        update(&mut app, key(KeyCode::Tab, KeyModifiers::NONE));
        let ops = |effects: Vec<Effect>| -> Vec<AccountOp> {
            effects
                .into_iter()
                .filter_map(|effect| match effect {
                    Effect::Account(op) => Some(op),
                    _ => None,
                })
                .collect()
        };
        assert_eq!(
            ops(update(&mut app, key(KeyCode::Enter, KeyModifiers::NONE))),
            vec![AccountOp::SetEnabled {
                account,
                enabled: false
            }]
        );
        assert_eq!(
            ops(update(&mut app, press('m'))),
            vec![AccountOp::SetDefault(account)]
        );
        assert_eq!(
            ops(update(&mut app, press('r'))),
            vec![AccountOp::RebuildIndex(account)]
        );
        assert_eq!(
            ops(update(&mut app, key(KeyCode::Delete, KeyModifiers::NONE))),
            vec![AccountOp::Remove(account)]
        );
        assert_eq!(
            app.notice(),
            Some("ada removed — ctrl+z to undo"),
            "the key the removal names is the key undo has"
        );
        assert_eq!(
            ops(update(
                &mut app,
                key(KeyCode::Char('z'), KeyModifiers::CONTROL)
            )),
            vec![AccountOp::Restore(account)],
            "undo takes the removal back"
        );

        update(&mut app, press('c'));
        let repair = app.first_run().expect("the sign-in, again");
        assert!(matches!(
            repair.status(),
            postio_ui::onboarding::Status::Reauthenticate(_)
        ));
        assert_eq!(
            repair.value(crate::first_run::Field::Address),
            "ada@example.com"
        );
    }

    #[test]
    fn a_browser_sign_in_shows_its_url_and_opens_it_only_when_asked() {
        // US7 scenario 2.
        use postio_ui::onboarding::{BrowserSignIn, Status};
        let mut app = app((160, 40));
        update(&mut app, Input::Places(an_empty_store()));
        type_text(&mut app, "ada@example.test");
        update(&mut app, key(KeyCode::Enter, KeyModifiers::NONE));
        let mut settings = discovered_settings();
        settings.oauth_sign_in = true;
        update(&mut app, Input::Discovered(Ok(Status::Found(settings))));
        assert_eq!(
            app.first_run().unwrap().field(),
            crate::first_run::Field::ClientId
        );
        type_text(&mut app, "postio-test");
        let effects = update(&mut app, key(KeyCode::Enter, KeyModifiers::NONE));
        let begun = effects
            .iter()
            .find_map(|effect| match effect {
                Effect::BeginOAuth(submission) => Some((**submission).clone()),
                _ => None,
            })
            .expect("begun");
        assert_eq!(begun.oauth_client.unwrap().client_id, "postio-test");

        let url = "https://login.example.test/authorize?client_id=postio-test";
        let effects = update(
            &mut app,
            Input::Consent(Ok(BrowserSignIn {
                provider: "Example".into(),
                scopes: vec!["offline_access".into()],
                redirect_uri: "http://127.0.0.1:41337/".into(),
                authorize_url: url.into(),
            })),
        );
        assert!(
            effects.contains(&Effect::FinishOAuth("ada@example.test".into())),
            "waits for the end at once: {effects:?}"
        );
        assert!(
            !effects
                .iter()
                .any(|effect| matches!(effect, Effect::OpenLink(_))),
            "nothing is opened until the person acts: {effects:?}"
        );
        assert_eq!(
            app.first_run().unwrap().status(),
            &Status::WaitingForBrowser
        );

        let effects = update(&mut app, press('y'));
        assert!(
            effects.contains(&Effect::CopyText(url.into())),
            "{effects:?}"
        );
        let effects = update(&mut app, key(KeyCode::Enter, KeyModifiers::NONE));
        assert!(
            effects.contains(&Effect::OpenLink(url.into())),
            "{effects:?}"
        );

        update(&mut app, Input::AccountAdded(Ok(())));
        assert_eq!(app.first_run().unwrap().status(), &Status::SyncWindow);
    }

    #[test]
    fn escape_gives_up_a_browser_sign_in() {
        use postio_ui::onboarding::{BrowserSignIn, Status};
        let mut app = app((160, 40));
        update(&mut app, Input::Places(an_empty_store()));
        type_text(&mut app, "ada@example.test");
        update(&mut app, key(KeyCode::Enter, KeyModifiers::NONE));
        let mut settings = discovered_settings();
        settings.oauth_sign_in = true;
        update(&mut app, Input::Discovered(Ok(Status::Found(settings))));
        type_text(&mut app, "postio-test");
        update(&mut app, key(KeyCode::Enter, KeyModifiers::NONE));
        update(&mut app, Input::Consent(Ok(BrowserSignIn::default())));
        let effects = update(&mut app, key(KeyCode::Esc, KeyModifiers::NONE));
        assert!(
            effects.contains(&Effect::CancelOAuth("ada@example.test".into())),
            "{effects:?}"
        );
    }

    #[test]
    fn the_composer_fills_the_body_and_the_list_returns_when_it_closes() {
        let mut app = app((70, 30));
        app.compose(postio_model::Draft::new(postio_model::AccountId::new(1)));
        assert_eq!(app.front(), Front::Composer);
        update(&mut app, key(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.front(), Front::List);
    }

    #[test]
    fn a_refusal_is_said_quietly() {
        let mut app = app((120, 30));
        update(
            &mut app,
            Input::Host(postio_core::Event::CommandRejected {
                command: postio_core::CommandId::Archive.into(),
                reason: "Nothing selected".into(),
            }),
        );
        assert_eq!(app.notice(), Some("Nothing selected"));
    }

    #[test]
    fn the_sync_label_follows_what_the_host_says_of_the_accounts() {
        use postio_core::{ConnectionState, Event};
        let account = postio_model::AccountId::new(1);
        let mut app = app((160, 40));
        update(&mut app, Input::Places(places()));
        assert_eq!(app.sync_label().text, "Not synced yet");
        update(
            &mut app,
            Input::Host(Event::ConnectionChanged {
                account,
                state: ConnectionState::Offline,
            }),
        );
        assert_eq!(app.sync_label().text, "Offline");
        update(
            &mut app,
            Input::Host(Event::ConnectionChanged {
                account,
                state: ConnectionState::Online,
            }),
        );
        update(
            &mut app,
            Input::Host(Event::SyncProgress {
                account,
                done: 3,
                total: 9,
            }),
        );
        assert_eq!(app.sync_label().text, "Syncing 3 of 9");
        update(
            &mut app,
            Input::Host(Event::SyncProgress {
                account,
                done: 9,
                total: 9,
            }),
        );
        assert!(
            app.sync_label().text.starts_with("Synced "),
            "{:?}",
            app.sync_label()
        );
    }

    fn reads(effects: &[Effect]) -> usize {
        effects
            .iter()
            .filter(|effect| matches!(effect, Effect::ReadBody(_)))
            .count()
    }

    #[test]
    fn scrolling_reads_no_body_and_opening_reads_one() {
        let mut app = app((160, 40));
        let opening = open_list(&mut app, 50);
        serve(&mut app, opening);
        for _ in 0..10 {
            let effects = update(&mut app, press('j'));
            assert_eq!(reads(&effects), 0, "a keystroke reads no body");
        }
        let effects = update(&mut app, key(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(
            effects
                .iter()
                .filter(|effect| matches!(effect, Effect::ReadBody(_)))
                .count(),
            1,
            "only the row opened is read: {effects:?}"
        );
    }

    #[test]
    fn a_body_that_arrives_is_drawn_in_the_reader() {
        let mut app = app((160, 40));
        let opening = open_list(&mut app, 5);
        serve(&mut app, opening);
        let message = row(0).id;
        app.open_reading(message);
        update(
            &mut app,
            Input::Body {
                message,
                answer: Ok(postio_client::protocol::Body::Ready {
                    body: postio_model::MessageBody {
                        text: Some("Hello Ada,\n\n> old words\n".into()),
                        html: None,
                    },
                    encoding_problems: false,
                }),
            },
        );
        let reading = app.reading().expect("the reader shows it");
        assert_eq!(reading.row, message);
        let drawn = reader_text(&app);
        assert!(drawn.contains("Hello Ada,"), "{drawn}");
        assert!(drawn.contains("quoted line"), "{drawn}");
    }

    #[test]
    fn a_body_for_a_row_already_left_is_not_drawn() {
        let mut app = app((160, 40));
        let opening = open_list(&mut app, 5);
        serve(&mut app, opening);
        update(&mut app, press('j'));
        update(
            &mut app,
            Input::Body {
                message: row(0).id,
                answer: Ok(postio_client::protocol::Body::Partial),
            },
        );
        assert!(
            app.reading()
                .is_none_or(|reading| reading.members.iter().all(|member| member.body.is_none())),
            "nothing drawn for a row already left"
        );
    }

    fn reading_a_long_quoted_reply(app: &mut App) {
        let opening = open_list(app, 5);
        serve(app, opening);
        let message = row(0).id;
        app.open_reading(message);
        let mut text = String::from("Top line\n");
        for n in 0..80 {
            text.push_str(&format!("line {n}\n"));
        }
        text.push_str("> quoted words\n");
        update(
            app,
            Input::Body {
                message,
                answer: Ok(postio_client::protocol::Body::Ready {
                    body: postio_model::MessageBody {
                        text: Some(text),
                        html: None,
                    },
                    encoding_problems: false,
                }),
            },
        );
    }

    #[test]
    fn in_the_reader_o_expands_the_quoted_history_and_folds_it_again() {
        let mut app = app((160, 40));
        reading_a_long_quoted_reply(&mut app);
        assert!(!reader_text(&app).contains("quoted words"));
        update(&mut app, key(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.focus(), Focus::Reader);
        update(&mut app, press('O'));
        assert!(reader_text(&app).contains("quoted words"), "expanded");
        update(&mut app, press('O'));
        assert!(!reader_text(&app).contains("quoted words"), "folded again");
    }

    #[test]
    fn space_scrolls_the_reader_a_screen_at_a_time() {
        let mut app = app((160, 40));
        reading_a_long_quoted_reply(&mut app);
        assert_eq!(app.reader_top(), 0);
        update(&mut app, press(' '));
        assert!(app.reader_top() > 20, "a screenful: {}", app.reader_top());
        update(&mut app, key(KeyCode::PageUp, KeyModifiers::NONE));
        assert_eq!(app.reader_top(), 0);
    }

    fn summary(id: i64, from: &str) -> postio_model::listing::MessageSummary {
        postio_model::listing::MessageSummary {
            id: MessageId::new(id),
            thread: Some(postio_model::ThreadId::new(9)),
            from: Some(postio_model::EmailAddress::new(None::<String>, from)),
            subject: Some("Plans".into()),
            preview: None,
            received_at: Utc.with_ymd_and_hms(2026, 9, 20, 9, 0, 0).unwrap(),
            seen: true,
            flagged: false,
            answered: false,
            send_state: None,
            send_at: None,
            has_attachments: false,
            thread_count: 3,
        }
    }

    /// A list of one conversation row, three messages long, being read.
    fn reading_a_conversation(app: &mut App) -> Vec<Effect> {
        let effects = open_list(app, 1);
        let (generation, page) = effects
            .iter()
            .find_map(|effect| match effect {
                Effect::Fetch {
                    generation, page, ..
                } => Some((*generation, *page)),
                _ => None,
            })
            .unwrap();
        let mut conversation = row(0);
        conversation.id = MessageId::new(3);
        conversation.thread = Some(postio_model::ThreadId::new(9));
        conversation.is_thread = true;
        conversation.count = 3;
        update(
            app,
            Input::Page {
                generation,
                page,
                rows: Ok(Page {
                    total: 1,
                    rows: vec![conversation],
                }),
            },
        );
        let effects = app.open_reading(MessageId::new(3));
        assert!(
            effects.contains(&Effect::ReadConversation(postio_model::ThreadId::new(9))),
            "a conversation row asks for its messages: {effects:?}"
        );
        update(
            app,
            Input::Conversation {
                thread: postio_model::ThreadId::new(9),
                members: Ok(vec![
                    summary(1, "ada@example.com"),
                    summary(2, "bea@example.com"),
                    summary(3, "cy@example.com"),
                ]),
            },
        )
    }

    fn ready(message: MessageId) -> Input {
        Input::Body {
            message,
            answer: Ok(postio_client::protocol::Body::Ready {
                body: postio_model::MessageBody {
                    text: Some("words".into()),
                    html: None,
                },
                encoding_problems: false,
            }),
        }
    }

    fn settle_of(effects: &[Effect]) -> Option<(u64, std::time::Duration)> {
        effects.iter().find_map(|effect| match effect {
            Effect::Settle { generation, after } => Some((*generation, *after)),
            _ => None,
        })
    }

    fn varied(position: u32) -> Row {
        let when = chrono::TimeZone::with_ymd_and_hms(&chrono::Utc, 2026, 9, 28, 9, 0, 0).unwrap()
            - chrono::Duration::hours(i64::from(position) * 5);
        let mut row = crate::test_support::row_from(
            i64::from(position) + 1,
            &format!("Sender {position}"),
            &format!("Subject {position}"),
            "",
            when,
        );
        if position.is_multiple_of(3) {
            row.marker = Some(postio_model::listing::MarkerSummary {
                kind: postio_model::listing::MarkerKind::Question,
                when: None,
                excerpt: Some("Can you?".into()),
                answer: None,
                cancelled: false,
            });
        }
        row
    }

    fn senders_shown(drawn: &str) -> Vec<u32> {
        drawn
            .lines()
            .filter_map(|line| {
                let at = line.find("Sender ")?;
                line[at + 7..]
                    .split(|c: char| !c.is_ascii_digit())
                    .next()?
                    .parse()
                    .ok()
            })
            .collect()
    }

    #[test]
    fn scrolling_past_the_foot_moves_the_list_one_row_not_a_screenful() {
        use crate::test_support::{screen, serve_with};
        let mut app = app((120, 36));
        let opening = open_list(&mut app, 400);
        serve_with(&mut app, opening, varied);
        let mut last = senders_shown(&screen(120, 36, &app));
        for _ in 0..150 {
            let effects = update(&mut app, press('j'));
            serve_with(&mut app, effects, varied);
            let shown = senders_shown(&screen(120, 36, &app));
            if shown.first() != last.first() {
                let below = shown.len()
                    - 1
                    - shown
                        .iter()
                        .position(|position| *position == app.cursor)
                        .expect("the cursor is in view");
                assert!(
                    below <= 1,
                    "the cursor stays at the foot as the list moves under it, \
                     with at most a row's slack where row heights differ: \
                     {last:?} became {shown:?}"
                );
                let moved = shown[0] - last[0];
                assert!(
                    moved <= 2,
                    "one step moves the list by about a row, not {moved}: \
                     {last:?} became {shown:?}"
                );
            }
            last = shown;
        }
    }

    #[test]
    fn a_list_read_again_keeps_its_rows_on_screen_until_they_land() {
        use crate::test_support::{row_from, screen, serve_with};
        let mut app = app((120, 36));
        let opening = open_list(&mut app, 3);
        serve(&mut app, opening);
        assert!(screen(120, 36, &app).contains("Message 1"));
        // Sync moved something: the list is counted and read again.
        let effects = update(
            &mut app,
            Input::Recounted {
                scope: ListScope::Mailbox(MailboxId::new(1)),
                total: 3,
            },
        );
        assert!(
            effects
                .iter()
                .any(|effect| matches!(effect, Effect::Fetch { .. })),
            "the rows in view are read again: {effects:?}"
        );
        let drawn = screen(120, 36, &app);
        assert!(
            drawn.contains("Message 1") && drawn.contains("Message 2"),
            "what was on screen stays until its page lands, not a blank list:\n{drawn}"
        );
        serve_with(&mut app, effects, |position| {
            row_from(
                i64::from(position) + 1,
                "Bea",
                &format!("Updated {position}"),
                "",
                chrono::TimeZone::with_ymd_and_hms(&chrono::Utc, 2026, 9, 20, 9, 0, 0).unwrap(),
            )
        });
        let drawn = screen(120, 36, &app);
        assert!(
            drawn.contains("Updated 1") && !drawn.contains("Message 1"),
            "then the page read again replaces it:\n{drawn}"
        );
    }

    #[test]
    fn opening_a_message_holds_the_paint_until_its_body_lands() {
        let mut app = app((120, 36));
        let opening = crate::test_support::open_list(&mut app, 3);
        crate::test_support::serve(&mut app, opening);
        assert!(!app.holds_paint(), "a list is painted as it comes");
        let effects = update(&mut app, key(KeyCode::Enter, KeyModifiers::NONE));
        let message = effects
            .iter()
            .find_map(|effect| match effect {
                Effect::ReadBody(message) => Some(*message),
                _ => None,
            })
            .expect("a body is asked for");
        let (_, after) = settle_of(&effects).expect("a deadline for the hold");
        assert!(
            after <= std::time::Duration::from_millis(16),
            "never longer than a frame: {after:?}"
        );
        assert!(app.holds_paint(), "the frame waits for what it will hold");
        update(
            &mut app,
            Input::Addressed {
                message,
                to: vec![postio_model::EmailAddress::new(
                    None::<String>,
                    "bea@example.com",
                )],
                cc: Vec::new(),
            },
        );
        assert!(
            app.holds_paint(),
            "the recipients alone would move the body down a row a moment later"
        );
        let effects = update(&mut app, ready(message));
        assert!(!app.holds_paint(), "the whole message is painted at once");
        assert!(effects.contains(&Effect::Redraw), "{effects:?}");
    }

    #[test]
    fn a_conversation_holds_the_paint_across_both_of_its_reads() {
        let mut app = app((160, 40));
        let effects = open_list(&mut app, 1);
        let (generation, page) = effects
            .iter()
            .find_map(|effect| match effect {
                Effect::Fetch {
                    generation, page, ..
                } => Some((*generation, *page)),
                _ => None,
            })
            .unwrap();
        let mut conversation = row(0);
        conversation.id = MessageId::new(3);
        conversation.thread = Some(postio_model::ThreadId::new(9));
        conversation.is_thread = true;
        conversation.count = 3;
        update(
            &mut app,
            Input::Page {
                generation,
                page,
                rows: Ok(Page {
                    total: 1,
                    rows: vec![conversation],
                }),
            },
        );
        let opened = update(&mut app, key(KeyCode::Enter, KeyModifiers::NONE));
        assert!(
            opened.contains(&Effect::ReadConversation(postio_model::ThreadId::new(9))),
            "{opened:?}"
        );
        assert!(settle_of(&opened).is_some(), "{opened:?}");
        let members = update(
            &mut app,
            Input::Conversation {
                thread: postio_model::ThreadId::new(9),
                members: Ok(vec![
                    summary(1, "ada@example.com"),
                    summary(3, "cy@example.com"),
                ]),
            },
        );
        assert!(
            members.contains(&Effect::ReadBody(MessageId::new(3))),
            "{members:?}"
        );
        assert!(
            app.holds_paint(),
            "its members are known and its body is not yet"
        );
        assert!(
            settle_of(&members).is_none(),
            "one deadline from the open, not a fresh one per read: {members:?}"
        );
        update(&mut app, ready(MessageId::new(3)));
        assert!(!app.holds_paint());
    }

    #[test]
    fn a_slow_body_is_painted_without_it_after_a_frame_and_a_closed_one_at_once() {
        let mut app = app((120, 36));
        let opening = crate::test_support::open_list(&mut app, 3);
        crate::test_support::serve(&mut app, opening);
        let effects = update(&mut app, key(KeyCode::Enter, KeyModifiers::NONE));
        let (generation, _) = settle_of(&effects).expect("a deadline");
        update(
            &mut app,
            Input::Settled {
                generation: generation + 1,
            },
        );
        assert!(
            app.holds_paint(),
            "an older deadline does not end a newer hold"
        );
        let effects = update(&mut app, Input::Settled { generation });
        assert!(!app.holds_paint(), "the frame is painted as it is");
        assert!(effects.contains(&Effect::Redraw), "{effects:?}");

        update(&mut app, key(KeyCode::Esc, KeyModifiers::NONE));
        update(&mut app, key(KeyCode::Down, KeyModifiers::NONE));
        update(&mut app, key(KeyCode::Enter, KeyModifiers::NONE));
        assert!(app.holds_paint());
        update(&mut app, key(KeyCode::Esc, KeyModifiers::NONE));
        assert!(!app.holds_paint(), "closing is painted at once");
    }

    #[test]
    fn a_conversation_row_reads_the_message_shown_and_the_others_when_stepped_to() {
        let mut app = app((160, 40));
        let effects = reading_a_conversation(&mut app);
        let reads: Vec<MessageId> = effects
            .iter()
            .filter_map(|effect| match effect {
                Effect::ReadBody(message) => Some(*message),
                _ => None,
            })
            .collect();
        assert_eq!(
            reads,
            [3].map(MessageId::new).to_vec(),
            "only the newest, which is shown (FR-195)"
        );
        update(&mut app, key(KeyCode::Enter, KeyModifiers::NONE));
        let stepped = update(&mut app, press('['));
        assert!(
            stepped.contains(&Effect::ReadBody(MessageId::new(2))),
            "{stepped:?}"
        );
        let again = update(&mut app, press(']'));
        assert!(
            !again.iter().any(|e| matches!(e, Effect::ReadBody(_))),
            "read once: {again:?}"
        );
        update(&mut app, press('['));
        update(&mut app, press('['));
        for id in 1..=3 {
            update(
                &mut app,
                Input::Body {
                    message: MessageId::new(id),
                    answer: Ok(postio_client::protocol::Body::Ready {
                        body: postio_model::MessageBody {
                            text: Some(format!("words of message {id}")),
                            html: None,
                        },
                        encoding_problems: false,
                    }),
                },
            );
        }
        // One message at a time, the one stepped to, under its sender.
        for (id, who) in [
            (1, "ada@example.com"),
            (2, "bea@example.com"),
            (3, "cy@example.com"),
        ] {
            let at = app.reading().unwrap().current;
            let drawn = reader_text(&app);
            assert!(
                drawn.contains(&format!("words of message {}", at + 1)),
                "{drawn}"
            );
            assert!(
                !drawn.contains(&format!("words of message {}", (at + 1) % 3 + 1)),
                "{drawn}"
            );
            let _ = (id, who);
            update(&mut app, press(']'));
        }
    }

    #[test]
    fn brackets_in_the_reader_walk_the_conversation() {
        let mut app = app((160, 40));
        reading_a_conversation(&mut app);
        update(&mut app, key(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.reading().unwrap().current, 2, "it opens on the newest");
        update(&mut app, press('['));
        assert_eq!(app.reading().unwrap().current, 1);
        update(&mut app, press('['));
        assert_eq!(app.reading().unwrap().current, 0);
        update(&mut app, press('['));
        assert_eq!(
            app.reading().unwrap().current,
            0,
            "nothing before the first"
        );
        update(&mut app, press(']'));
        assert_eq!(app.reading().unwrap().current, 1);
    }

    #[test]
    fn blocked_remote_images_are_counted_and_i_a_trusts_the_sender_everywhere() {
        let mut app = app((160, 40));
        let opening = open_list(&mut app, 5);
        serve(&mut app, opening);
        let message = row(0).id;
        app.open_reading(message);
        update(
            &mut app,
            Input::Body {
                message,
                answer: Ok(postio_client::protocol::Body::Ready {
                    body: postio_model::MessageBody {
                        text: None,
                        html: Some(
                            "<p>Hi</p><img src=\"https://cdn.example.org/a.png\" alt=\"Hero\">"
                                .into(),
                        ),
                    },
                    encoding_problems: false,
                }),
            },
        );
        let drawn = reader_text(&app);
        assert!(drawn.contains("1 remote image blocked"), "{drawn}");
        assert!(
            drawn.contains("i i"),
            "the key that shows them is named: {drawn}"
        );

        update(&mut app, key(KeyCode::Tab, KeyModifiers::NONE));
        update(&mut app, press('i'));
        let effects = update(&mut app, press('a'));
        let saved = effects.iter().find_map(|effect| match effect {
            Effect::SaveAllowlist(list) => Some(list.clone()),
            _ => None,
        });
        let saved = saved.expect("the allow list, shared with the desktop, is saved");
        assert!(saved.is_allowed("ada@example.com"), "{saved:?}");
        let drawn = reader_text(&app);
        assert!(!drawn.contains("remote image blocked"), "{drawn}");
        assert!(drawn.contains("allowed"), "{drawn}");
    }

    #[test]
    fn shift_u_asks_to_leave_the_list_of_the_message_being_read() {
        let mut app = app((160, 40));
        reading_a_conversation(&mut app);
        update(&mut app, key(KeyCode::Tab, KeyModifiers::NONE));
        let effects = update(&mut app, key(KeyCode::Char('U'), KeyModifiers::SHIFT));
        assert!(
            effects.contains(&Effect::Unsubscribe(MessageId::new(3))),
            "the message being read, the newest: {effects:?}"
        );
        update(&mut app, Input::Unsubscribed(Ok("news.example.com".into())));
        assert_eq!(app.notice(), Some("Asked to leave news.example.com"));
    }

    #[test]
    fn a_messages_parts_are_listed_and_a_written_one_is_launched() {
        let mut app =
            app((160, 40)).with_downloads(std::path::PathBuf::from("/home/ada/Downloads"));
        let effects = open_list(&mut app, 1);
        let (generation, page) = effects
            .iter()
            .find_map(|effect| match effect {
                Effect::Fetch {
                    generation, page, ..
                } => Some((*generation, *page)),
                _ => None,
            })
            .unwrap();
        let mut with_a_file = row(0);
        with_a_file.attachment = true;
        update(
            &mut app,
            Input::Page {
                generation,
                page,
                rows: Ok(Page {
                    total: 1,
                    rows: vec![with_a_file],
                }),
            },
        );
        let message = row(0).id;
        app.open_reading(message);
        let effects = update(
            &mut app,
            Input::Body {
                message,
                answer: Ok(postio_client::protocol::Body::Ready {
                    body: postio_model::MessageBody {
                        text: Some("See attached.".into()),
                        html: None,
                    },
                    encoding_problems: false,
                }),
            },
        );
        assert!(effects.contains(&Effect::ReadParts(message)), "{effects:?}");

        let mut report = postio_model::Attachment::new(message, "application/pdf", 2_048);
        report.id = postio_model::ids::AttachmentId::new(4);
        report.filename = Some("report.pdf".into());
        update(
            &mut app,
            Input::Parts {
                message,
                parts: Ok(vec![report]),
            },
        );
        let drawn = reader_text(&app);
        assert!(drawn.contains("report.pdf"), "{drawn}");
        assert!(
            drawn.contains("2.0 KB") || drawn.contains("2 KB"),
            "{drawn}"
        );

        let effects = update(
            &mut app,
            Input::PartWritten {
                written: Ok(std::path::PathBuf::from(
                    "/run/user/1000/postio/parts/1-4/report.pdf",
                )),
                open: true,
            },
        );
        assert!(effects.contains(&Effect::Launch(std::path::PathBuf::from(
            "/run/user/1000/postio/parts/1-4/report.pdf"
        ))));
    }

    #[test]
    fn the_quit_command_quits() {
        let mut app = app((120, 30));
        let effects = update(
            &mut app,
            Input::Key(KeyEvent {
                code: KeyCode::Char('q'),
                modifiers: KeyModifiers::CONTROL,
                kind: KeyEventKind::Press,
                state: KeyEventState::NONE,
            }),
        );
        assert!(effects.contains(&Effect::Quit), "{effects:?}");
    }

    #[test]
    fn a_page_that_failed_is_asked_for_again() {
        let mut app = app((120, 30));
        let effects = open_list(&mut app, 100);
        let Some(Effect::Fetch {
            generation, page, ..
        }) = effects
            .into_iter()
            .find(|effect| matches!(effect, Effect::Fetch { page: 0, .. }))
        else {
            panic!("the first page was asked for");
        };
        let effects = update(
            &mut app,
            Input::Page {
                generation,
                page,
                rows: Err("busy".into()),
            },
        );
        assert!(
            effects
                .iter()
                .any(|effect| matches!(effect, Effect::Fetch { page: 0, .. })),
            "{effects:?}"
        );
    }
}
