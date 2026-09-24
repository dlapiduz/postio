//! The terminal frontend's state, and the one function that changes it.
//!
//! `update` takes an [`Input`] and returns the [`Effect`]s it asks for --
//! a request of the daemon, a redraw, quitting -- and does no I/O itself.
//! That is what lets every behaviour be driven by synthetic input in a test,
//! with no terminal and no daemon (research R11).
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
use postio_ui::paging::{Fetch, Page, Paging};

use crate::input::Keys;
use crate::layout::{self, Requested, Shown};
use crate::row::Row;
use crate::view::list::Visible;
use postio_body::replying::ReplyKind;

/// Something that happened, from the terminal or from the daemon.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Input {
    /// The terminal is now this many columns and rows.
    Resize(u16, u16),
    /// The clipboard answered an [`Effect::ReadClipboardImage`]: a PNG,
    /// no image, or why it could not be read.
    ClipboardImage(Result<Option<Vec<u8>>, String>),
    /// The daemon stored an [`Effect::InlineImage`]: the inline part, or
    /// nothing when it could not.
    InlineStored(Option<postio_model::Attachment>),
    /// The external editor exited: what it saved, or why not.
    Edited {
        /// Which composition.
        generation: u64,
        /// The body it saved.
        edited: Result<String, String>,
    },
    /// Text was pasted, or files were dropped: a drop arrives as a paste of
    /// their paths.
    Paste(String),
    /// The daemon answered an [`Effect::Attach`]: the stored attachment, or
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
    /// The daemon said something happened.
    Host(postio_core::Event),
    /// The cursor has rested on `message` since [`Effect::Rest`] asked.
    Rested(postio_model::MessageId),
    /// A conversation asked for by [`Effect::ReadConversation`] arrived.
    Conversation {
        /// Which.
        thread: postio_model::ThreadId,
        /// Its messages, oldest first, or why there are none.
        members: Result<Vec<postio_model::listing::MessageSummary>, String>,
    },
    /// A body asked for by [`Effect::ReadBody`] arrived.
    Body {
        /// Whose.
        message: postio_model::MessageId,
        /// The body, or why there is none.
        answer: Result<postio_client::protocol::Body, String>,
    },
    /// The daemon answered an [`Effect::Unsubscribe`]: the list's name, or
    /// why not.
    Unsubscribed(Result<String, String>),
    /// An [`Effect::Autosave`]'s time is up.
    AutosaveDue {
        /// Which composition asked.
        generation: u64,
        /// How many edits it had when it asked.
        edit: u64,
    },
    /// The daemon answered an [`Effect::SaveDraft`]: the draft's id, or why
    /// it could not be saved.
    DraftSaved {
        /// Which composition.
        generation: u64,
        /// Its id, or the sentence for the status line.
        saved: Result<postio_model::DraftId, String>,
    },
    /// The daemon answered an [`Effect::Search`].
    Found {
        /// Which question it answers.
        sequence: u64,
        /// What matched, nothing when the store could not be read, or why
        /// the daemon could not be asked.
        found: Result<Option<postio_client::protocol::Found>, String>,
    },
    /// The daemon answered an [`Effect::Recipients`].
    Recipients {
        /// What was looked up.
        prefix: String,
        /// Who it could be, best first.
        found: Vec<postio_model::contact_group::RecipientCandidate>,
    },
    /// The daemon answered an [`Effect::QueueSend`].
    Queued {
        /// When it was scheduled for, if it was.
        at: Option<chrono::DateTime<chrono::Utc>>,
        /// Whether it was queued, or why not.
        queued: Result<(), String>,
    },
    /// The daemon answered an [`Effect::Resume`]: the draft behind the row,
    /// taken back from the Outbox if it was queued; nothing when there is no
    /// local draft or its send has already started.
    Resumed(Option<Box<postio_model::Draft>>),
    /// The daemon answered an [`Effect::ReplySource`]: the message and its
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
    /// What the sidebar holds, read afresh.
    Sidebar(crate::sidebar::Contents),
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
    /// Wait [`READ_REST`], then answer with [`Input::Rested`].
    Rest(postio_model::MessageId),
    /// Read a conversation and answer with [`Input::Conversation`].
    ReadConversation(postio_model::ThreadId),
    /// Read a body and answer with [`Input::Body`].
    ReadBody(postio_model::MessageId),
    /// Open a list: count it and answer with [`Input::Opened`].
    Open(ListScope),
    /// Read the sidebar's contents again and answer with [`Input::Sidebar`].
    RefreshSidebar,
    /// Count a list again and answer with [`Input::Recounted`].
    Recount(ListScope),
    /// Leave the list this message came from; answer with
    /// [`Input::Unsubscribed`].
    Unsubscribe(postio_model::MessageId),
    /// Read a message's parts and answer with [`Input::Parts`].
    ReadParts(postio_model::MessageId),
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
    /// Ask again for [`Input::AutosaveDue`] after [`AUTOSAVE`].
    Autosave {
        /// Which composition.
        generation: u64,
        /// How many edits it has now.
        edit: u64,
    },
    /// Save a draft through the daemon's draft writer.
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
    /// Run a search; its answer comes back as [`Input::Found`].
    Search {
        /// Which question this is, so an older answer can be dropped.
        sequence: u64,
        /// The search.
        search: postio_client::protocol::Search,
    },
    /// Look up who a recipient being typed could be.
    Recipients {
        /// Whose contacts.
        account: postio_model::AccountId,
        /// What has been typed.
        prefix: String,
    },
    /// Open the local draft behind a Drafts or Outbox row.
    Resume(postio_model::MessageId),
    /// Read the message a reply or forward starts from, and its account.
    ReplySource {
        /// Which draft to start.
        kind: ReplyKind,
        /// The message.
        message: postio_model::MessageId,
    },
    /// Send a command to the daemon, aimed with [`App::state`].
    Send(postio_core::Command),
    /// Read a page of the list and answer with [`Input::Page`].
    Fetch {
        /// The list's generation now; a page for an older one is dropped.
        generation: u64,
        /// Which page.
        page: u32,
        /// What to read.
        fetch: Fetch,
    },
}

/// How long the cursor must rest on a row before its body is read.
///
/// A keystroke reads no body -- `j` held down is a scroll, and reading each
/// row it passes would be a body per keystroke for mail nobody looked at
/// (Principle V). Short enough that stopping on a row reads as immediate.
pub const READ_REST: std::time::Duration = std::time::Duration::from_millis(120);

/// How long the typing has to pause before a draft is saved: the desktop
/// composer's autosave interval.
pub const AUTOSAVE: std::time::Duration = std::time::Duration::from_millis(1500);

/// Everything the terminal frontend knows.
pub struct App {
    size: (u16, u16),
    requested: Requested,
    keys: Keys,
    list: ListWindow<Row>,
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
    /// The list being shown.
    scope: Option<ListScope>,
    /// What the status line says about the last thing done: the undo offer,
    /// a refusal, an error.
    notice: Option<String>,
    /// Where the keyboard is.
    focus: Focus,
    /// The sidebar's lines.
    sidebar: Vec<crate::sidebar::Line>,
    /// The sidebar line the keyboard is on.
    sidebar_cursor: usize,
    /// Each account's sync status, folded from the daemon's events.
    trackers: postio_ui::status::Trackers,
    /// Which account the list on screen belongs to.
    account: Option<postio_model::AccountId>,
    /// Every folder, to find the account a list belongs to.
    folders: Vec<postio_model::mailbox::Mailbox>,
    /// The message the reader shows, and how.
    reading: Option<crate::conversation::Reading>,
    /// The message the cursor was last seen resting towards.
    resting: Option<postio_model::MessageId>,
    /// The first reader line in view.
    reader_top: usize,
    /// The part the keyboard is on, in the parts of the message being read.
    part_cursor: usize,
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
    /// Whether the draft is in a tab of its own rather than the reading
    /// pane: the desktop's composer window (FR-003).
    detached: bool,
    /// The search bar, while it is open.
    search: Option<SearchBar>,
    /// The palette, while it is open.
    palette: Option<PaletteState>,
    /// Whether the terminal speaks the kitty keyboard protocol, so every
    /// chord arrives; otherwise only what a legacy terminal can send does.
    enhanced_keys: bool,
    /// Where the keyboard was when the cheat sheet opened, while it is open.
    cheatsheet: Option<Focus>,
}

/// One section of the cheat sheet as it is drawn: its heading, and each
/// command with the key this terminal can send for it.
pub type SheetSection = (&'static str, Vec<(&'static str, String)>);

/// Which of the finder's modes the palette is in (`postio_ui::finder`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Finding {
    /// `>`: run a command.
    Commands,
    /// `#`: go to a folder.
    Folders,
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
    /// The finder mode's marker: `>` or `#`.
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
    /// The key that does the same, as this terminal can send it.
    pub chord: Option<String>,
    /// Which characters of the title the query matched.
    pub positions: Vec<usize>,
}

/// What a palette row does.
enum PaletteAction {
    Run(postio_core::ActionId),
    Open(ListScope),
}

/// The search bar: what is typed, which question is outstanding, and what
/// the last answer turned out to be.
#[derive(Debug, Default)]
struct SearchBar {
    input: tui_input::Input,
    pacer: postio_ui::search::Pacer,
    outcome: Option<postio_ui::search::Outcome>,
    /// Newest first rather than best match first.
    newest_first: bool,
}

/// Which pane the keyboard is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Focus {
    /// The message list.
    #[default]
    List,
    /// The sidebar.
    Sidebar,
    /// The reading pane.
    Reader,
    /// The parts of the message being read.
    Parts,
    /// The composer, in the reading pane.
    Composer,
    /// The search bar.
    Search,
    /// The command palette, or another of the finder's modes.
    Palette,
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
            requested: Requested::default(),
            keys,
            list: ListWindow::new(),
            paging: Paging::default(),
            cursor: 0,
            top: 0,
            state: postio_core::SharedState::default(),
            selection: postio_ui::selection::SelectionState::new(),
            scope: None,
            notice: None,
            focus: Focus::List,
            sidebar: Vec::new(),
            sidebar_cursor: 0,
            trackers: postio_ui::status::Trackers::default(),
            account: None,
            folders: Vec::new(),
            reading: None,
            resting: None,
            reader_top: 0,
            allowlist: postio_ui::allowlist::RemoteImageAllowList::default(),
            part_cursor: 0,
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
            search: None,
            palette: None,
            enhanced_keys: false,
            cheatsheet: None,
        }
    }

    /// The same app, saving parts into `downloads`.
    pub fn with_downloads(mut self, downloads: std::path::PathBuf) -> App {
        self.downloads = downloads;
        self
    }

    /// The part the keyboard is on.
    pub fn part_cursor(&self) -> usize {
        self.part_cursor
    }

    /// The same app, honouring `allowlist`.
    pub fn with_allowlist(mut self, allowlist: postio_ui::allowlist::RemoteImageAllowList) -> App {
        self.allowlist = allowlist;
        self
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

    /// What the status line says about the connection of the account on
    /// screen: `offline`, `syncing`, `idle` -- the desktop's own words.
    pub fn sync_line(&self) -> Option<String> {
        let account = self.account?;
        let (state, detail) = self
            .trackers
            .status(account)
            .lines(std::time::Instant::now());
        Some(format!("{state} · {detail}"))
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
            ListScope::Unified | ListScope::Thread(_) => None,
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

    /// Whether the reading pane shows what is being read, rather than the
    /// draft.
    pub fn showing_reader(&self) -> bool {
        self.composer.is_none() || self.detached
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
        self.focus = Focus::Composer;
        self.requested.front = crate::layout::Pane::Reader;
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
        self.focus = Focus::List;
        self.requested.front = crate::layout::Pane::List;
        effects.push(Effect::Redraw);
        effects
    }

    /// Out of the draft's tab and back to the mail, the draft still open
    /// there and saved as it stands.
    fn leave_draft_tab(&mut self) -> Vec<Effect> {
        let mut effects = Vec::new();
        if let Some(composer) = &self.composer {
            let draft = composer.draft();
            if postio_model::draft::closing(&draft) == postio_model::draft::Closing::Keep {
                effects.push(Effect::SaveDraft {
                    generation: composer.generation(),
                    draft: Box::new(draft),
                });
            }
        }
        self.focus = Focus::List;
        self.requested.front = crate::layout::Pane::List;
        effects.push(Effect::Redraw);
        effects
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

    /// The same app, knowing the terminal delivers every chord.
    pub fn with_enhanced_keys(mut self, enhanced: bool) -> App {
        self.enhanced_keys = enhanced;
        self
    }

    /// Open the palette in one of the finder's modes, from wherever the
    /// keyboard is.
    fn open_palette(&mut self, finding: Finding) -> Vec<Effect> {
        let from = match self.focus {
            // The finder's prefixes turn the bar into the palette; what it
            // runs then runs over the list.
            Focus::Search if finding == Finding::Folders || self.search.is_none() => Focus::List,
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

    /// The registry's context for where the keyboard is.
    fn context_of(focus: Focus) -> postio_core::Context {
        match focus {
            Focus::List | Focus::Palette => postio_core::Context::List,
            Focus::Search => postio_core::Context::Search,
            Focus::Sidebar => postio_core::Context::Sidebar,
            Focus::Reader => postio_core::Context::Reader,
            Focus::Parts => postio_core::Context::Parts,
            Focus::Composer => postio_core::Context::Composer,
        }
    }

    /// What can run here, given what is on screen.
    fn availability(&self) -> postio_core::Availability {
        postio_core::Availability::open(
            self.account
                .map_or(postio_core::Scope::Unified, postio_core::Scope::Account),
        )
    }

    /// The cheat sheet, while it is open: `postio_ui::cheatsheet::sections`
    /// for where the keyboard was, each key as this terminal can send it.
    pub fn cheat_sheet(&self) -> Option<Vec<SheetSection>> {
        let focus = self.cheatsheet?;
        let keymap = self.keys.keymap();
        Some(
            postio_ui::cheatsheet::sections(keymap, Self::context_of(focus), self.availability())
                .into_iter()
                .map(|section| {
                    let rows = section
                        .rows
                        .into_iter()
                        .map(|row| {
                            let key = row
                                .id
                                .and_then(|id| {
                                    postio_ui::terminal::deliverable_binding(
                                        keymap,
                                        id,
                                        self.enhanced_keys,
                                    )
                                })
                                .or(row.binding)
                                .unwrap_or_default();
                            (row.title, key)
                        })
                        .collect();
                    (section.title, rows)
                })
                .collect(),
        )
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
                Finding::Commands => ">",
                Finding::Folders => "#",
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
            Finding::Commands => {
                let context = Self::context_of(state.from);
                let availability = self.availability();
                postio_ui::palette::entries(self.keys.keymap(), context, availability, query)
                    .into_iter()
                    .map(|entry| {
                        let chord = postio_ui::terminal::deliverable_binding(
                            self.keys.keymap(),
                            entry.id,
                            self.enhanced_keys,
                        );
                        (
                            PaletteRow {
                                title: entry.title.to_owned(),
                                chord,
                                positions: entry.positions,
                            },
                            PaletteAction::Run(entry.id),
                        )
                    })
                    .collect()
            }
            Finding::Folders => {
                let mut found: Vec<(i32, PaletteRow, PaletteAction)> = self
                    .sidebar
                    .iter()
                    .filter_map(|line| {
                        let scope = line.opens?;
                        let title = line.label.as_str().to_owned();
                        let matched = postio_ui::palette::score(query.trim(), &title)?;
                        Some((
                            matched.score,
                            PaletteRow {
                                title,
                                chord: None,
                                positions: matched.positions,
                            },
                            PaletteAction::Open(scope),
                        ))
                    })
                    .collect();
                found.sort_by_key(|(score, ..)| std::cmp::Reverse(*score));
                found
                    .into_iter()
                    .map(|(_, row, action)| (row, action))
                    .collect()
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
                    Some(PaletteAction::Run(postio_core::ActionId::Builtin(id))) => match from {
                        Focus::Composer => self.composer_command(id.as_str()),
                        _ => self.command(id.as_str()),
                    },
                    Some(PaletteAction::Run(other)) => {
                        self.say(&format!("{other} is not something this terminal can run"))
                    }
                    Some(PaletteAction::Open(scope)) => vec![Effect::Open(scope), Effect::Redraw],
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

    /// What is typed in the search bar, while it is open.
    pub fn search_query(&self) -> Option<&str> {
        self.search.as_ref().map(|bar| bar.input.value())
    }

    /// Where the caret is in the search bar, in characters.
    pub fn search_caret(&self) -> usize {
        self.search.as_ref().map_or(0, |bar| bar.input.cursor())
    }

    /// The operators in the query, as chips: Postio's query language, read
    /// back as it is typed (`postio_ui::search`).
    pub fn search_chips(&self) -> Vec<postio_ui::search::Chip> {
        self.search_query()
            .map(|query| {
                postio_ui::search::chips(&postio_search::parse(
                    query,
                    chrono::Local::now().date_naive(),
                ))
            })
            .unwrap_or_default()
    }

    /// What the last search turned out to be, as the desktop says it.
    pub fn search_readout(&self) -> Option<String> {
        let outcome = self.search.as_ref()?.outcome.as_ref()?;
        Some(postio_ui::search::readout(outcome))
    }

    /// Open the search bar, over the list.
    fn open_search(&mut self) -> Vec<Effect> {
        if self.search.is_none() {
            self.search = Some(SearchBar::default());
        }
        self.focus = Focus::Search;
        vec![Effect::Redraw]
    }

    /// Close the search bar and put the folder back where it was.
    fn close_search(&mut self) -> Vec<Effect> {
        let mut effects = vec![Effect::Redraw];
        self.search = None;
        self.focus = Focus::List;
        if self.paging.close_results() {
            self.list.reset(0);
            self.cursor = 0;
            self.top = 0;
            if let Some(scope) = self.scope {
                effects.push(Effect::Recount(scope));
            }
        }
        effects
    }

    /// A key in the search bar: typed, with Backspace taking a whole chip
    /// (`postio_ui::search::backspace`), Enter going down to the results,
    /// and Escape closing the bar.
    fn search_key(&mut self, key: &KeyEvent) -> Vec<Effect> {
        use crossterm::event::KeyCode;
        use tui_input::backend::crossterm::EventHandler;
        match self.keys.press(key, KeyContext::Search, true) {
            Outcome::Command(id) if id == "back" => return self.close_search(),
            Outcome::Command(id) => return self.command(&id),
            Outcome::Pending(_) => return Vec::new(),
            Outcome::Unhandled => {}
        }
        let Some(bar) = self.search.as_mut() else {
            return Vec::new();
        };
        let before = bar.input.value().to_owned();
        match key.code {
            KeyCode::Enter => {
                self.focus = Focus::List;
                return vec![Effect::Redraw];
            }
            KeyCode::Backspace => {
                let parsed = postio_search::parse(&before, chrono::Local::now().date_naive());
                let caret = before
                    .char_indices()
                    .nth(bar.input.cursor())
                    .map_or(before.len(), |(at, _)| at);
                match postio_ui::search::backspace(&parsed, caret) {
                    postio_ui::search::Backspace::PopChip { query, caret, .. } => {
                        let chars = query[..caret].chars().count();
                        bar.input = tui_input::Input::default().with_value(query);
                        bar.input.handle(tui_input::InputRequest::SetCursor(chars));
                    }
                    postio_ui::search::Backspace::Ordinary => {
                        bar.input.handle_event(&crossterm::event::Event::Key(*key));
                    }
                }
            }
            _ => {
                bar.input.handle_event(&crossterm::event::Event::Key(*key));
            }
        }
        if bar.input.value() == before {
            return vec![Effect::Redraw];
        }
        // The finder's prefixes: typed first into an empty bar, they turn it
        // into another of its modes, as the desktop's box does.
        match bar.input.value() {
            ">" => {
                self.search = None;
                return self.open_palette(Finding::Commands);
            }
            "#" => {
                self.search = None;
                return self.open_palette(Finding::Folders);
            }
            _ => {}
        }
        self.run_search()
    }

    /// Ask the daemon the question now in the bar; an empty bar puts the
    /// folder back.
    fn run_search(&mut self) -> Vec<Effect> {
        let account = self.account.map_or(
            postio_model::AccountScope::Unified,
            postio_model::AccountScope::Account,
        );
        let Some(bar) = self.search.as_mut() else {
            return Vec::new();
        };
        let query = bar.input.value().to_owned();
        if query.trim().is_empty() {
            bar.pacer.abandon();
            bar.outcome = None;
            let mut effects = vec![Effect::Redraw];
            if self.paging.close_results() {
                self.list.reset(0);
                self.cursor = 0;
                self.top = 0;
                if let Some(scope) = self.scope {
                    effects.push(Effect::Recount(scope));
                }
            }
            return effects;
        }
        let sequence = bar.pacer.issue();
        vec![
            Effect::Search {
                sequence,
                search: postio_client::protocol::Search {
                    account,
                    query,
                    newest_first: bar.newest_first,
                },
            },
            Effect::Redraw,
        ]
    }

    /// An answer to a search: shown in the list if it is still the question.
    fn found(
        &mut self,
        sequence: u64,
        found: Result<Option<postio_client::protocol::Found>, String>,
    ) -> Vec<Effect> {
        let Some(bar) = self.search.as_mut() else {
            return Vec::new();
        };
        if !bar.pacer.accepts(sequence) {
            return Vec::new();
        }
        match found {
            Ok(Some(found)) => {
                bar.outcome = Some(postio_ui::search::Outcome {
                    hits: found.hits,
                    capped: found.capped,
                    elapsed: found.elapsed,
                    corpus_complete: found.corpus_complete,
                    unreachable: Vec::new(),
                });
                let total = self.paging.show_results(found.ids);
                self.selection.clear();
                self.list.reset(total);
                self.cursor = 0;
                self.top = 0;
                vec![Effect::Redraw]
            }
            Ok(None) => self.say("The search could not be run"),
            Err(reason) => self.say(&reason),
        }
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
        self.focus = Focus::List;
        self.requested.front = crate::layout::Pane::List;
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
            "back" if self.detached => self.leave_draft_tab(),
            "back" | "discard_draft" => self.close_composer(),
            // The desktop moves its composer into a window of its own; here
            // it is a tab, and the reading pane goes back to the reader.
            "detach_composer" => {
                self.detached = !self.detached;
                vec![Effect::Redraw]
            }
            // Everything else the composer context reaches -- quitting, the
            // palette -- means what it means anywhere.
            _ => self.command(id),
        }
    }

    /// The sidebar's lines, and which one the keyboard would be on.
    pub fn sidebar(&self) -> (&[crate::sidebar::Line], usize) {
        (&self.sidebar, self.sidebar_cursor)
    }

    /// The keymap context the keyboard is in.
    fn key_context(&self) -> KeyContext {
        match self.focus {
            Focus::List => KeyContext::List,
            Focus::Sidebar => KeyContext::Sidebar,
            // As the desktop reading pane is: a conversation of several is
            // where `J`/`K` walk messages and `O` expands; a message on its
            // own is the reader, where `p` shows its parts.
            Focus::Reader
                if self
                    .reading
                    .as_ref()
                    .is_some_and(|reading| reading.members.len() > 1) =>
            {
                KeyContext::Conversation
            }
            Focus::Reader => KeyContext::Reader,
            Focus::Parts => KeyContext::Parts,
            Focus::Composer => KeyContext::Composer,
            Focus::Search => KeyContext::Search,
            Focus::Palette => KeyContext::Palette,
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
        self.notice = Some(postio_ui::terminal::SafeText::new(sentence).to_string());
        vec![Effect::Redraw]
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

    /// What the user asked to see.
    pub fn requested(&self) -> Requested {
        self.requested
    }

    /// What the terminal has room for.
    pub fn shown(&self) -> Shown {
        layout::shown(self.size.0, self.size.1, self.requested)
    }

    /// The row the keyboard is on.
    pub fn cursor(&self) -> u32 {
        self.cursor
    }

    /// How many rows the list has.
    pub fn total(&self) -> u32 {
        self.list.total()
    }

    /// How many list rows fit: everything but the status line.
    pub fn list_height(&self) -> u32 {
        u32::from(self.size.1.saturating_sub(1))
    }

    /// The rows in view, for drawing. Reads only what is resident.
    pub fn visible(&self) -> Vec<Visible<'_>> {
        let end = (self.top + self.list_height()).min(self.list.total());
        (self.top..end)
            .map(|position| Visible {
                row: self
                    .list
                    .peek(position)
                    .and_then(|message| self.list.row_of(message)),
                cursor: position == self.cursor,
                selected: self
                    .list
                    .peek(position)
                    .is_some_and(|message| self.selection.contains(message)),
            })
            .collect()
    }

    /// Move the cursor to `position`, keeping it in view.
    fn move_to(&mut self, position: u32) {
        let last = self.list.total().saturating_sub(1);
        self.cursor = position.min(last);
        let height = self.list_height().max(1);
        if self.cursor < self.top {
            self.top = self.cursor;
        } else if self.cursor >= self.top + height {
            self.top = self.cursor + 1 - height;
        }
    }

    /// Ask for every page in view that is neither here nor on its way.
    fn fetches(&mut self) -> Vec<Effect> {
        let end = (self.top + self.list_height()).min(self.list.total());
        let mut wanted = Vec::new();
        for position in self.top..end {
            if let Some(postio_ui::list::Lookup::Missing { request }) = self.list.row_at(position) {
                wanted.extend(request);
            }
        }
        let generation = self.list.generation();
        wanted
            .into_iter()
            .filter_map(|page| {
                self.paging.fetch_for(page).map(|fetch| Effect::Fetch {
                    generation,
                    page,
                    fetch,
                })
            })
            .collect()
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
    fn command(&mut self, id: &str) -> Vec<Effect> {
        let last = self.list.total().saturating_sub(1);
        match id {
            "next_message" => self.move_to(self.cursor.saturating_add(1)),
            "prev_message" => self.move_to(self.cursor.saturating_sub(1)),
            "first_message" => self.move_to(0),
            "last_message" => self.move_to(last),
            "toggle_selection" => {
                if let Some(message) = self.cursor_message() {
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
            "quit" => {
                // What is being written is saved on the way out.
                let mut effects = if self.composer.is_some() {
                    self.close_composer()
                } else {
                    Vec::new()
                };
                effects.push(Effect::Quit);
                return effects;
            }
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
            "open_message" if self.listing_drafts() => {
                if let Some(message) = self.cursor_message() {
                    return vec![Effect::Resume(message)];
                }
            }
            // One composition at a time, as the desktop's `c` does with a
            // composer already open: it goes back to it.
            "compose" if self.composer.is_some() => {
                self.focus = Focus::Composer;
                if !self.detached {
                    self.requested.front = crate::layout::Pane::Reader;
                }
                return vec![Effect::Redraw];
            }
            "compose" => {
                if let Some(account) = self.account {
                    return self.compose(postio_model::Draft::new(account));
                }
            }
            "focus_sidebar" => self.focus = Focus::Sidebar,
            "search" => return self.open_search(),
            "command_palette" => return self.open_palette(Finding::Commands),
            "cheat_sheet" => self.cheatsheet = Some(self.focus),
            "toggle_result_order" => {
                if let Some(bar) = self.search.as_mut() {
                    bar.newest_first = !bar.newest_first;
                    return self.run_search();
                }
            }
            "save_search" => {
                if let Some(query) = self.search_query().map(str::trim)
                    && !query.is_empty()
                {
                    let query = query.to_owned();
                    let mut effects = self.say(&format!("Saved “{query}” to the sidebar"));
                    effects.insert(0, Effect::SaveSearch(query));
                    return effects;
                }
            }
            "cycle_pane" => {
                // The composer is the reading pane while it is open.
                let reader = if self.composer.is_some() && !self.detached {
                    Focus::Composer
                } else {
                    Focus::Reader
                };
                self.focus = match self.focus {
                    Focus::List | Focus::Search | Focus::Palette => reader,
                    Focus::Reader | Focus::Parts | Focus::Composer => Focus::Sidebar,
                    Focus::Sidebar => Focus::List,
                }
            }
            "cycle_pane_back" => {
                let reader = if self.composer.is_some() && !self.detached {
                    Focus::Composer
                } else {
                    Focus::Reader
                };
                self.focus = match self.focus {
                    Focus::List | Focus::Search | Focus::Palette => Focus::Sidebar,
                    Focus::Sidebar => reader,
                    Focus::Reader | Focus::Parts | Focus::Composer => Focus::List,
                }
            }
            "back" if self.focus == Focus::Parts => self.focus = Focus::Reader,
            "back" if self.focus != Focus::List => self.focus = Focus::List,
            "open_parts" => {
                if !self.current_attachments().is_empty() {
                    self.focus = Focus::Parts;
                    self.part_cursor = 0;
                }
            }
            "next_part" => {
                let last = self.current_attachments().len().saturating_sub(1);
                self.part_cursor = (self.part_cursor + 1).min(last);
            }
            "prev_part" => self.part_cursor = self.part_cursor.saturating_sub(1),
            "open_part" => return self.write_parts(false, false),
            "save_part" => return self.write_parts(true, false),
            "save_all_parts" => return self.write_parts(true, true),
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
            "next_in_conversation" => self.walk_conversation(1),
            "prev_in_conversation" => self.walk_conversation(-1),
            "scroll_reader_down" => self.scroll_reader(1),
            "scroll_reader_up" => self.scroll_reader(-1),
            "back" => self.selection.clear(),
            "next_folder" => return self.walk_sidebar(1),
            "prev_folder" => return self.walk_sidebar(-1),
            other => return self.send(other),
        }
        vec![Effect::Redraw]
    }

    /// Aim a verb at what the user is looking at, and send it.
    fn send(&mut self, id: &str) -> Vec<Effect> {
        let Ok(id) = id.parse::<postio_core::CommandId>() else {
            return Vec::new();
        };
        let selection = self.selection.selection();
        let aim = postio_core::aim::Aim {
            scope: self
                .scope
                .and_then(|scope| postio_core::aim::view_scope(scope, &[])),
            selection: &selection,
            cursor: self.cursor_message(),
            rows: &self.list,
        };
        let command = postio_core::aim::command_for(id, &aim);
        let (quiet, _) = postio_core::bridge::event_channel();
        postio_core::aim::mirror(&self.state, &quiet, &aim);
        vec![Effect::Send(command)]
    }

    /// Expand every fold in what is being read, or fold them all again.
    fn toggle_folds(&mut self) {
        if let Some(reading) = self.reading.as_mut() {
            reading.toggle_folds();
        }
    }

    /// The reader's lines and each member's header line, as of now.
    fn reader_layout(&self) -> Option<(Vec<ratatui::text::Line<'static>>, Vec<usize>)> {
        self.reading
            .as_ref()
            .map(|reading| reading.layout(chrono::Local::now()))
    }

    /// The attachments of the member being read, and whose they are.
    fn current_attachments(&self) -> Vec<(postio_model::MessageId, postio_model::Attachment)> {
        self.reading
            .as_ref()
            .and_then(|reading| reading.members.get(reading.current))
            .map(|member| {
                member
                    .attachments()
                    .into_iter()
                    .map(|part| (member.id, part.clone()))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Open the part under the cursor, or save it -- or every part -- to
    /// the downloads folder.
    fn write_parts(&mut self, save: bool, all: bool) -> Vec<Effect> {
        let parts = self.current_attachments();
        let chosen: Vec<_> = if all {
            parts
        } else {
            parts.into_iter().skip(self.part_cursor).take(1).collect()
        };
        chosen
            .into_iter()
            .map(|(message, part)| {
                let name = part
                    .filename
                    .as_deref()
                    .and_then(|name| std::path::Path::new(name).file_name())
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "part".to_owned());
                Effect::SavePart {
                    message,
                    attachment: part.id,
                    to: save.then(|| self.downloads.join(name)),
                }
            })
            .collect()
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

    /// Scroll the reader by `pages` screenfuls, overlapping two lines so the
    /// eye keeps its place.
    fn scroll_reader(&mut self, pages: isize) {
        let Some((lines, _)) = self.reader_layout() else {
            return;
        };
        let length = lines.len();
        let page = usize::from(self.size.1.saturating_sub(6)).max(1);
        let step = page.saturating_sub(2).max(1);
        self.reader_top = if pages >= 0 {
            (self.reader_top + step * pages.unsigned_abs()).min(length.saturating_sub(1))
        } else {
            self.reader_top.saturating_sub(step * pages.unsigned_abs())
        };
    }

    /// Move to the next or previous message of the conversation.
    fn walk_conversation(&mut self, step: isize) {
        let Some(reading) = self.reading.as_mut() else {
            return;
        };
        let last = reading.members.len().saturating_sub(1);
        reading.current = reading.current.saturating_add_signed(step).min(last);
        if let Some((_, headers)) = self.reader_layout() {
            let current = self.reading.as_ref().map_or(0, |reading| reading.current);
            self.reader_top = headers.get(current).copied().unwrap_or(0);
        }
    }

    /// The cursor stayed: read what it is on, if the reader is not already.
    fn rested(&mut self, message: postio_model::MessageId) -> Vec<Effect> {
        let here = self.cursor_message() == Some(message);
        let shown = self.reading.as_ref().map(|reading| reading.row) == Some(message);
        if !here || shown {
            return Vec::new();
        }
        let Some(row) = self.list.row_of(message) else {
            return Vec::new();
        };
        match row.thread {
            Some(thread) if row.is_thread && row.count > 1 => {
                self.reading = Some(crate::conversation::Reading {
                    row: message,
                    members: Vec::new(),
                    current: 0,
                });
                vec![Effect::ReadConversation(thread)]
            }
            _ => {
                self.reading = Some(crate::conversation::Reading {
                    row: message,
                    members: vec![crate::conversation::Member {
                        id: row.id,
                        from: row.from.clone(),
                        address: row.address.clone(),
                        when: row.when,
                        body: None,
                        held_back: Default::default(),
                        images_allowed: false,
                        has_attachments: row.attachment,
                        parts: Vec::new(),
                    }],
                    current: 0,
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
        let reads = reading
            .members
            .iter()
            .map(|member| Effect::ReadBody(member.id))
            .collect::<Vec<_>>();
        self.walk_conversation(0);
        let mut effects = vec![Effect::Redraw];
        effects.extend(reads);
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
        let rendered = match answer {
            Ok(Body::Ready { body, .. }) if body.html.is_some() => {
                // The rule every reader applies: reader view for bulk mail,
                // the sender's own markup (sanitised, folded) otherwise.
                let rendering = if suits_reader_view(&body) {
                    Rendering::Reader
                } else {
                    Rendering::Original
                };
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
        let ask_for_parts = member.has_attachments && member.parts.is_empty();
        member.held_back = held_back;
        member.images_allowed = member
            .address
            .as_deref()
            .is_some_and(|address| self.allowlist.is_allowed(address));
        // Bodies arrive in any order; keep the newest message's header in view.
        self.walk_conversation(0);
        let mut effects = vec![Effect::Redraw];
        if ask_for_parts {
            effects.push(Effect::ReadParts(message));
        }
        effects
    }

    /// Take in the sidebar's contents, keeping the cursor on the list shown.
    fn fill_sidebar(&mut self, contents: &crate::sidebar::Contents) -> Vec<Effect> {
        self.sidebar = crate::sidebar::lines(contents);
        self.folders = contents.folders.clone();
        self.accounts = contents.accounts.clone();
        for account in &contents.accounts {
            self.trackers.note_last_sync(account.id, &contents.folders);
        }
        if let Some(scope) = self.scope {
            self.account = self.account_of(scope);
        }
        self.sidebar_cursor = self
            .sidebar
            .iter()
            .position(|line| line.opens.is_some() && line.opens == self.scope)
            .or_else(|| self.sidebar.iter().position(|line| line.opens.is_some()))
            .unwrap_or(0);
        vec![Effect::Redraw]
    }

    /// Move the sidebar cursor by `step` rows that open something, and open
    /// what it lands on -- as the desktop sidebar opens a folder when the
    /// selection moves to it.
    fn walk_sidebar(&mut self, step: isize) -> Vec<Effect> {
        let openable: Vec<usize> = self
            .sidebar
            .iter()
            .enumerate()
            .filter(|(_, line)| line.opens.is_some() || line.searches.is_some())
            .map(|(index, _)| index)
            .collect();
        let Some(here) = openable
            .iter()
            .position(|index| *index >= self.sidebar_cursor)
        else {
            return Vec::new();
        };
        let there = here.saturating_add_signed(step).min(openable.len() - 1);
        self.sidebar_cursor = openable[there];
        if let Some(query) = self.sidebar[self.sidebar_cursor].searches.clone() {
            // A saved search is a search: the bar holds its query, and the
            // results take the list, from the same executor the desktop's
            // saved search runs through.
            self.search = Some(SearchBar {
                input: tui_input::Input::default().with_value(query),
                ..SearchBar::default()
            });
            return self.run_search();
        }
        match self.sidebar[self.sidebar_cursor].opens {
            Some(scope) if Some(scope) != self.scope => vec![Effect::Redraw, Effect::Open(scope)],
            _ => vec![Effect::Redraw],
        }
    }

    /// The daemon said something happened.
    ///
    /// What a command said about itself goes on the status line; what
    /// changed in the store goes through the same paging plan the desktop
    /// list follows, so the two lists react to one event the same way.
    fn hear(&mut self, event: &postio_core::Event) -> Vec<Effect> {
        use postio_core::Event;
        // Every event is offered to the status line first: an error is both
        // something to say and the reason a failing account's line gives.
        let moved = self.trackers.apply(event, self.account);
        match event {
            Event::ActionCompleted {
                description,
                undoable,
            } => {
                let sentence = match (undoable, self.keys.key_for(KeyContext::List, "undo")) {
                    (true, Some(key)) => format!("{description} — {key} to undo"),
                    _ => description.clone(),
                };
                return self.say(&sentence);
            }
            Event::UndoPerformed { description } => return self.say(description),
            Event::CommandRejected { reason, .. } => return self.say(reason),
            Event::Error { message } => return self.say(message),
            _ => {}
        }
        if moved {
            return vec![Effect::Redraw];
        }
        if matches!(event, Event::MailboxesChanged { .. }) {
            return vec![Effect::RefreshSidebar];
        }
        match self.paging.plan(event) {
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
                pages
                    .into_iter()
                    .filter(|page| self.list.note_pending(*page))
                    .filter_map(|page| {
                        self.paging.fetch_for(page).map(|fetch| Effect::Fetch {
                            generation,
                            page,
                            fetch,
                        })
                    })
                    .collect()
            }
            postio_ui::paging::Plan::Reload => self
                .scope
                .map(|scope| vec![Effect::Recount(scope)])
                .unwrap_or_default(),
        }
    }

    /// A list was counted again after it changed: keep the scroll, drop what
    /// is cached, and let the rows in view be read again.
    fn recounted(&mut self, scope: ListScope, total: u32) -> Vec<Effect> {
        if self.scope != Some(scope) {
            return Vec::new();
        }
        self.list.invalidate();
        let _ = self.list.set_total(total);
        self.move_to(self.cursor);
        vec![Effect::Redraw]
    }

    /// A list opened: show it from the top.
    fn open(&mut self, scope: ListScope, total: u32) -> Vec<Effect> {
        // A folder opened is a search left, as it is on the desktop.
        self.search = None;
        self.paging.close_results();
        self.paging.open(scope);
        self.scope = Some(scope);
        self.account = self.account_of(scope);
        // A selection is relative to the list it was made in.
        self.selection.clear();
        self.list.reset(total);
        self.cursor = 0;
        self.top = 0;
        vec![Effect::Redraw]
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
                if delivered.stale {
                    Vec::new()
                } else {
                    vec![Effect::Redraw]
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
        // Any key puts the cheat sheet away; it is something to read.
        Input::Key(_) if app.cheatsheet.is_some() => {
            app.cheatsheet = None;
            vec![Effect::Redraw]
        }
        Input::Key(key) if app.focus == Focus::Composer => app.composer_key(&key),
        Input::Key(key) if app.focus == Focus::Search => app.search_key(&key),
        Input::Key(key) if app.focus == Focus::Palette => app.palette_key(&key),
        // Over search results the list's keys come first, and what the list
        // does not know is the search's: `o` for the order, Ctrl+S to save.
        Input::Key(key) if app.focus == Focus::List && app.paging.showing_results() => {
            match app.keys.press(&key, KeyContext::List, false) {
                Outcome::Command(id) => app.command(&id),
                Outcome::Pending(_) => Vec::new(),
                Outcome::Unhandled => match app.keys.press(&key, KeyContext::Search, false) {
                    Outcome::Command(id) => app.command(&id),
                    Outcome::Pending(_) | Outcome::Unhandled => Vec::new(),
                },
            }
        }
        Input::Key(key) => match app.keys.press(&key, app.key_context(), false) {
            Outcome::Command(id) => app.command(&id),
            Outcome::Pending(_) | Outcome::Unhandled => Vec::new(),
        },
        Input::Opened { scope, total } => app.open(scope, total),
        Input::Host(event) => app.hear(&event),
        Input::Sidebar(contents) => app.fill_sidebar(&contents),
        Input::Rested(message) => app.rested(message),
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
        Input::AutosaveDue { generation, edit } => app.autosave_due(generation, edit),
        Input::DraftSaved { generation, saved } => match saved {
            Ok(id) => {
                if let Some(composer) = app.composer.as_mut()
                    && composer.generation() == generation
                {
                    composer.adopt_id(id);
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
        Input::Found { sequence, found } => app.found(sequence, found),
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
        Input::Resumed(found) => match found {
            Some(draft) => app.compose(*draft),
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
        Input::Body { message, answer } => app.show(message, answer),
        Input::Conversation { thread, members } => app.conversation(thread, members),
        Input::Recounted { scope, total } => app.recounted(scope, total),
        Input::Page {
            generation,
            page,
            rows,
        } => app.page(generation, page, rows),
    };
    effects.extend(app.fetches());
    // The cursor landed on a different message: rest, then read it. Asked
    // after every input rather than only after a keystroke, because a row can
    // arrive under a still cursor -- a page landing, a list opening.
    let under = app.cursor_message();
    if under != app.resting {
        app.resting = under;
        if let Some(message) = under {
            effects.push(Effect::Rest(message));
        }
    }
    effects
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};
    use crossterm::event::{KeyCode, KeyEventKind, KeyEventState, KeyModifiers};
    use postio_model::{MailboxId, MessageId};
    use postio_ui::terminal::SafeText;

    use super::*;
    use crate::layout::Pane;

    fn app(size: (u16, u16)) -> App {
        let keys = Keys::new(&postio_core::Keymap::resolve(&Default::default())).0;
        App::new(size, keys)
    }

    fn press(c: char) -> Input {
        Input::Key(KeyEvent {
            code: KeyCode::Char(c),
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        })
    }

    fn row(position: u32) -> Row {
        Row {
            id: MessageId::new(i64::from(position) + 1),
            thread: None,
            is_thread: false,
            from: SafeText::new("Ada"),
            address: Some("ada@example.com".into()),
            subject: SafeText::new(&format!("Message {position}")),
            preview: SafeText::new(""),
            when: Utc.with_ymd_and_hms(2026, 9, 20, 9, 0, 0).unwrap(),
            unread: false,
            flagged: false,
            attachment: false,
            count: 1,
        }
    }

    /// Answer every fetch the way the daemon would, with rows for its range;
    /// return how many fetches there were.
    fn serve(app: &mut App, effects: Vec<Effect>) -> usize {
        let mut fetched = 0;
        let mut pending = effects;
        while let Some(effect) = pending.pop() {
            if let Effect::Fetch {
                generation,
                page,
                fetch: Fetch::Scope(request),
            } = effect
            {
                fetched += 1;
                let rows = (request.offset..request.offset + request.limit)
                    .filter(|position| *position < app.total())
                    .map(row)
                    .collect();
                pending.extend(update(
                    app,
                    Input::Page {
                        generation,
                        page,
                        rows: Ok(Page {
                            total: app.total(),
                            rows,
                        }),
                    },
                ));
            }
        }
        fetched
    }

    fn opened(app: &mut App, total: u32) -> Vec<Effect> {
        update(
            app,
            Input::Opened {
                scope: ListScope::Mailbox(MailboxId::new(1)),
                total,
            },
        )
    }

    #[test]
    fn a_resize_changes_what_is_shown_and_asks_the_daemon_nothing() {
        let mut app = app((160, 40));
        let asked = app.requested();

        let effects = update(&mut app, Input::Resize(60, 30));

        assert_eq!(effects, vec![Effect::Redraw], "a redraw and nothing else");
        assert_eq!(app.shown(), Shown::Panes(vec![Pane::List]));
        assert_eq!(app.requested(), asked, "what was asked for is untouched");

        update(&mut app, Input::Resize(160, 40));
        assert_eq!(
            app.shown(),
            Shown::Panes(vec![Pane::Sidebar, Pane::List, Pane::Reader]),
            "widening brings the sidebar back"
        );
    }

    #[test]
    fn opening_a_list_asks_only_for_the_pages_in_view() {
        let mut app = app((120, 30));
        let effects = opened(&mut app, 100_000);
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
        let opening = opened(&mut app, 100_000);
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
        let opening = opened(&mut app, 3);
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
        let opening = opened(&mut app, 10);
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
        let opening = opened(&mut app, 10);
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
        let opening = opened(&mut app, 10);
        serve(&mut app, opening);
        mark(&mut app, 2);
        let visible = app.visible();
        assert!(visible[2].selected);
        assert!(!visible[1].selected);
    }

    #[test]
    fn an_undoable_action_is_announced_and_u_sends_undo() {
        let mut app = app((120, 30));
        let opening = opened(&mut app, 10);
        serve(&mut app, opening);
        update(
            &mut app,
            Input::Host(postio_core::Event::ActionCompleted {
                description: "Archived 12 messages".into(),
                undoable: true,
            }),
        );
        assert_eq!(app.notice(), Some("Archived 12 messages — u to undo"));

        let effects = update(&mut app, press('u'));
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

    #[test]
    fn a_list_the_host_changed_is_counted_again_and_reread() {
        let mut app = app((120, 30));
        let opening = opened(&mut app, 10);
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
        let opening = opened(&mut app, 3);
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
        let opening = opened(&mut app, 3);
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
        update(&mut app, Input::Sidebar(sidebar_contents()));
        let opening = opened(&mut app, 3);
        serve(&mut app, opening);
        update(&mut app, press('c'));
        let composer = app.composer().expect("composing");
        assert_eq!(composer.field(), crate::composer::Field::To);
        assert_eq!(composer.draft().account_id, postio_model::AccountId::new(1));
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
        let mut contents = sidebar_contents();
        let mut drafts = Mailbox::new(postio_model::AccountId::new(1), "Drafts", None);
        drafts.id = MailboxId::new(9);
        drafts.role = MailboxRole::Drafts;
        drafts.selectable = true;
        contents.folders.push(drafts);
        update(&mut app, Input::Sidebar(contents));
        let opening = update(
            &mut app,
            Input::Opened {
                scope: ListScope::Mailbox(MailboxId::new(9)),
                total: 2,
            },
        );
        serve(&mut app, opening);

        let effects = update(&mut app, key(KeyCode::Enter, KeyModifiers::NONE));
        assert!(
            effects.contains(&Effect::Resume(MessageId::new(1))),
            "{effects:?}"
        );
        let mut draft = postio_model::Draft::new(postio_model::AccountId::new(1));
        draft.id = postio_model::DraftId::new(5);
        draft.body_markdown = Some("Half **written**".into());
        update(&mut app, Input::Resumed(Some(Box::new(draft))));
        let composer = app.composer().expect("composing");
        assert_eq!(composer.markdown(), "Half **written**");
        assert_eq!(composer.draft().id, postio_model::DraftId::new(5));
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

    fn typing(app: &mut App, text: &str) -> Vec<Effect> {
        text.chars().flat_map(|c| update(app, press(c))).collect()
    }

    #[test]
    fn typing_a_recipient_offers_the_contacts_it_could_be() {
        // T055, at the desktop's threshold of four characters (#424).
        let mut app = app((160, 40));
        composing(&mut app);
        let effects = typing(&mut app, "ada@");
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
        typing(&mut app, "ada@e");
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
        typing(&mut app, "ada@");
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
        typing(&mut app, "See attached");

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

        typing(&mut app, &format!("{}/fix", dir.path().display()));
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

    fn alt(c: char) -> Input {
        key(KeyCode::Char(c), KeyModifiers::ALT)
    }

    #[test]
    fn a_popped_out_draft_keeps_its_id_and_the_reader_comes_back() {
        // T063 (FR-003): the desktop's composer window is a tab here.
        let mut app = app((160, 40));
        let opening = opened(&mut app, 3);
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
        assert_eq!(app.focus(), Focus::Composer, "the draft's tab is in front");

        let effects = update(&mut app, key(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.focus(), Focus::List, "back to the mail");
        let composer = app.composer().expect("the draft is still open in its tab");
        assert_eq!(composer.draft().id, postio_model::DraftId::new(5));
        assert!(
            saves(&effects).len() == 1,
            "saved on the way out of the tab: {effects:?}"
        );
        assert!(
            app.showing_reader(),
            "the reading pane is the reader's again"
        );

        update(&mut app, press('c'));
        assert_eq!(
            app.focus(),
            Focus::Composer,
            "c goes back to the open draft"
        );
        assert_eq!(
            app.composer().unwrap().generation(),
            generation,
            "not a new one"
        );

        update(&mut app, alt('o'));
        assert!(!app.composer_detached(), "and the same key puts it back");
        assert!(!app.showing_reader());
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
        let effects = typing(&mut app, "Here is the photo: ");
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
        typing(&mut app, "Photo: ");
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

    fn searches(effects: &[Effect]) -> Vec<(u64, String)> {
        effects
            .iter()
            .filter_map(|effect| match effect {
                Effect::Search { sequence, search } => Some((*sequence, search.query.clone())),
                _ => None,
            })
            .collect()
    }

    fn found(ids: &[i64]) -> postio_client::protocol::Found {
        postio_client::protocol::Found {
            ids: ids.iter().copied().map(MessageId::new).collect(),
            hits: ids.len() as u64,
            capped: false,
            corpus_complete: true,
            elapsed: std::time::Duration::from_millis(11),
        }
    }

    #[test]
    fn a_search_runs_on_every_key_and_a_half_typed_operator_is_no_error() {
        // US4 scenario 1.
        let mut app = app((160, 40));
        let opening = opened(&mut app, 3);
        serve(&mut app, opening);
        update(&mut app, press('/'));
        assert_eq!(app.focus(), Focus::Search);

        let mut asked = Vec::new();
        for c in "from:ada is:".chars() {
            asked.extend(searches(&update(&mut app, press(c))));
        }
        assert_eq!(asked.len(), "from:ada is:".len(), "one search per key");
        assert!(
            asked.windows(2).all(|pair| pair[0].0 < pair[1].0),
            "each newer than the last"
        );
        assert_eq!(asked.last().unwrap().1, "from:ada is:");
        let chips = app.search_chips();
        assert!(
            chips
                .iter()
                .any(|chip| chip.label == "from:ada" && chip.complete)
        );
        assert!(
            chips
                .iter()
                .any(|chip| chip.label == "is:" && !chip.complete),
            "a half-typed operator is a chip in progress, not an error: {chips:?}"
        );

        for c in "unread".chars() {
            update(&mut app, press(c));
        }
        let latest = searches(&update(&mut app, press(' '))).last().unwrap().0;
        update(
            &mut app,
            Input::Found {
                sequence: latest - 1,
                found: Ok(Some(found(&[9]))),
            },
        );
        assert_ne!(app.total(), 1, "an answer to an older question is dropped");
        let effects = update(
            &mut app,
            Input::Found {
                sequence: latest,
                found: Ok(Some(found(&[3, 1]))),
            },
        );
        assert_eq!(app.total(), 2);
        assert!(
            effects.iter().any(|effect| matches!(
                effect,
                Effect::Fetch {
                    fetch: Fetch::Hits { .. },
                    ..
                }
            )),
            "the hits are read: {effects:?}"
        );
        assert_eq!(app.search_readout().as_deref(), Some("2 hits · 11 ms"));
    }

    fn showing_results(app: &mut App) {
        let opening = opened(app, 3);
        serve(app, opening);
        update(app, press('/'));
        let asked = typing(app, "tide");
        let sequence = searches(&asked).last().unwrap().0;
        update(
            app,
            Input::Found {
                sequence,
                found: Ok(Some(found(&[3, 1]))),
            },
        );
        update(app, key(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.focus(), Focus::List, "down in the results");
    }

    #[test]
    fn over_the_results_o_reorders_and_ctrl_s_saves_the_search() {
        let mut app = app((160, 40));
        showing_results(&mut app);

        let effects = update(&mut app, press('o'));
        let again: Vec<_> = effects
            .iter()
            .filter_map(|effect| match effect {
                Effect::Search { search, .. } => Some((search.query.clone(), search.newest_first)),
                _ => None,
            })
            .collect();
        assert_eq!(again, vec![("tide".to_owned(), true)], "{effects:?}");

        let effects = update(&mut app, ctrl('s'));
        assert!(
            effects.contains(&Effect::SaveSearch("tide".into())),
            "{effects:?}"
        );

        // And the list's own keys still work there.
        update(&mut app, press('j'));
        assert_eq!(app.cursor(), 1);
    }

    #[test]
    fn a_palette_opened_in_the_search_bar_offers_the_searchs_commands() {
        let mut app = app((160, 40));
        let opening = opened(&mut app, 3);
        serve(&mut app, opening);
        update(&mut app, press('/'));
        typing(&mut app, "tide");
        update(&mut app, ctrl('k'));
        let titles: Vec<String> = app
            .palette()
            .unwrap()
            .rows
            .into_iter()
            .map(|row| row.title)
            .collect();
        let order = postio_core::registry::get(postio_core::CommandId::ToggleResultOrder).title;
        assert!(titles.iter().any(|title| title == order), "{titles:?}");
    }

    #[test]
    fn backspace_takes_a_whole_chip_and_escape_puts_the_folder_back() {
        let mut app = app((160, 40));
        let opening = opened(&mut app, 3);
        serve(&mut app, opening);
        update(&mut app, press('/'));
        for c in "tide from:ada".chars() {
            update(&mut app, press(c));
        }
        let asked = searches(&update(
            &mut app,
            key(KeyCode::Backspace, KeyModifiers::NONE),
        ));
        assert_eq!(app.search_query(), Some("tide"));
        update(
            &mut app,
            Input::Found {
                sequence: asked.last().expect("asked again").0,
                found: Ok(Some(found(&[2]))),
            },
        );
        assert_eq!(app.total(), 1, "the results replaced the folder");

        let effects = update(&mut app, key(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.focus(), Focus::List);
        assert_eq!(app.search_query(), None);
        let scope = ListScope::Mailbox(MailboxId::new(1));
        assert!(effects.contains(&Effect::Recount(scope)), "{effects:?}");
    }

    fn ctrl(c: char) -> Input {
        key(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    #[test]
    fn the_palette_lists_what_this_context_reaches_with_keys_this_terminal_sends() {
        // T066: the rows are postio_ui::palette::entries, and each shows the
        // chord a legacy terminal can deliver.
        let mut app = app((160, 40));
        let opening = opened(&mut app, 3);
        serve(&mut app, opening);
        update(&mut app, ctrl('k'));
        assert_eq!(app.focus(), Focus::Palette);
        let shown = app.palette().expect("open");
        let keymap = postio_core::Keymap::resolve(&Default::default());
        let expected: Vec<&str> = postio_ui::palette::entries(
            &keymap,
            postio_core::Context::List,
            postio_core::Availability::open(postio_core::Scope::Unified),
            "",
        )
        .iter()
        .map(|entry| entry.title)
        .collect();
        let titles: Vec<&str> = shown.rows.iter().map(|row| row.title.as_str()).collect();
        assert_eq!(titles, expected);
        let mark_sent = shown
            .rows
            .iter()
            .find(|row| {
                row.title == postio_core::registry::get(postio_core::CommandId::MarkSent).title
            })
            .expect("listed");
        assert_eq!(mark_sent.chord.as_deref(), Some("alt+m"));
    }

    #[test]
    fn a_palette_row_runs_where_the_palette_was_opened() {
        let mut app = app((160, 40));
        let opening = opened(&mut app, 3);
        serve(&mut app, opening);
        update(&mut app, ctrl('k'));
        typing(&mut app, "archive");
        assert_eq!(app.palette().unwrap().rows[0].title, "Archive");
        let effects = update(&mut app, key(KeyCode::Enter, KeyModifiers::NONE));
        assert!(
            effects
                .iter()
                .any(|effect| matches!(effect, Effect::Send(postio_core::Command::Archive { .. }))),
            "{effects:?}"
        );
        assert!(app.palette().is_none());
        assert_eq!(app.focus(), Focus::List);

        update(&mut app, ctrl('k'));
        update(&mut app, key(KeyCode::Esc, KeyModifiers::NONE));
        assert!(app.palette().is_none());
        assert_eq!(app.focus(), Focus::List);
    }

    #[test]
    fn the_search_bars_prefixes_reach_the_palette_and_the_folders() {
        let mut app = app((160, 40));
        update(&mut app, Input::Sidebar(sidebar_contents()));
        let opening = opened(&mut app, 3);
        serve(&mut app, opening);

        update(&mut app, press('/'));
        update(&mut app, press('>'));
        assert_eq!(app.focus(), Focus::Palette, "> runs a command");
        assert_eq!(app.palette().unwrap().marker, ">");
        update(&mut app, key(KeyCode::Esc, KeyModifiers::NONE));

        update(&mut app, press('/'));
        update(&mut app, press('#'));
        let folders = app.palette().expect("# goes to a folder");
        assert_eq!(folders.marker, "#");
        typing(&mut app, "arch");
        assert_eq!(app.palette().unwrap().rows[0].title, "Archive");
        let effects = update(&mut app, key(KeyCode::Enter, KeyModifiers::NONE));
        assert!(
            effects.contains(&Effect::Open(ListScope::Mailbox(MailboxId::new(2)))),
            "{effects:?}"
        );
    }

    #[test]
    fn the_composer_takes_the_reading_pane_on_a_narrow_terminal() {
        let mut app = app((70, 30));
        app.compose(postio_model::Draft::new(postio_model::AccountId::new(1)));
        assert_eq!(app.shown(), Shown::Panes(vec![Pane::Reader]));
        update(&mut app, key(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.shown(), Shown::Panes(vec![Pane::List]));
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

    fn sidebar_contents() -> crate::sidebar::Contents {
        use postio_model::mailbox::{Mailbox, MailboxRole};
        let mut account = postio_model::Account::new(
            "ada",
            postio_model::EmailAddress::new(None::<String>, "ada@example.com"),
        );
        account.id = postio_model::AccountId::new(1);
        account.enabled = true;
        let folder = |id, name: &str, role| {
            let mut folder = Mailbox::new(account.id, name, None);
            folder.id = MailboxId::new(id);
            folder.role = role;
            folder.selectable = true;
            folder
        };
        crate::sidebar::Contents {
            accounts: vec![account.clone()],
            folders: vec![
                folder(1, "INBOX", MailboxRole::Inbox),
                folder(2, "Archive", MailboxRole::Archive),
            ],
            counts: Vec::new(),
            saved: Vec::new(),
        }
    }

    fn key(code: KeyCode, modifiers: KeyModifiers) -> Input {
        Input::Key(KeyEvent {
            code,
            modifiers,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        })
    }

    #[test]
    fn walking_the_sidebar_opens_what_the_cursor_lands_on() {
        let mut app = app((160, 40));
        let opening = opened(&mut app, 3);
        serve(&mut app, opening);
        update(&mut app, Input::Sidebar(sidebar_contents()));
        update(&mut app, press('g'));
        update(&mut app, press('f'));
        assert_eq!(app.focus(), Focus::Sidebar, "g f focuses the sidebar");

        let effects = update(&mut app, press('j'));
        let opened_scope = effects.iter().find_map(|effect| match effect {
            Effect::Open(scope) => Some(*scope),
            _ => None,
        });
        let (lines, at) = app.sidebar();
        assert_eq!(opened_scope, lines[at].opens, "{effects:?}");
        assert!(opened_scope.is_some());
        assert_ne!(
            opened_scope,
            Some(ListScope::Mailbox(MailboxId::new(1))),
            "it moved off the inbox"
        );

        update(&mut app, key(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.focus(), Focus::List, "Escape goes back to the list");
    }

    #[test]
    fn a_saved_search_in_the_sidebar_runs_its_query() {
        // US4 scenario 3: the desktop's saved search, run by the same search.
        let mut app = app((160, 40));
        let mut contents = sidebar_contents();
        contents.saved = vec![crate::sidebar::Saved {
            name: "Unread from Ada".into(),
            query: "from:ada is:unread".into(),
        }];
        update(&mut app, Input::Sidebar(contents));
        let opening = opened(&mut app, 3);
        serve(&mut app, opening);
        update(&mut app, press('g'));
        update(&mut app, press('f'));
        let mut asked = Vec::new();
        for _ in 0..20 {
            asked.extend(searches(&update(&mut app, press('j'))));
        }
        assert_eq!(
            asked.last().map(|(_, query)| query.as_str()),
            Some("from:ada is:unread"),
            "{asked:?}"
        );
        assert_eq!(app.search_query(), Some("from:ada is:unread"));
    }

    #[test]
    fn the_sidebar_cursor_starts_on_the_list_being_shown() {
        let mut app = app((160, 40));
        let opening = opened(&mut app, 3);
        serve(&mut app, opening);
        update(&mut app, Input::Sidebar(sidebar_contents()));
        let (lines, at) = app.sidebar();
        assert_eq!(lines[at].opens, Some(ListScope::Mailbox(MailboxId::new(1))));
    }

    #[test]
    fn the_status_line_says_offline_until_the_daemon_says_otherwise() {
        let mut app = app((160, 40));
        update(&mut app, Input::Sidebar(sidebar_contents()));
        let opening = opened(&mut app, 3);
        serve(&mut app, opening);
        let line = app.sync_line().expect("a line for the account on screen");
        assert!(line.starts_with("offline"), "{line}");

        update(
            &mut app,
            Input::Host(postio_core::Event::ConnectionChanged {
                account: postio_model::AccountId::new(1),
                state: postio_core::ConnectionState::Online,
            }),
        );
        let line = app.sync_line().unwrap();
        assert!(line.starts_with("idle"), "{line}");
    }

    fn rests(effects: &[Effect]) -> Vec<MessageId> {
        effects
            .iter()
            .filter_map(|effect| match effect {
                Effect::Rest(message) => Some(*message),
                _ => None,
            })
            .collect()
    }

    fn reads(effects: &[Effect]) -> usize {
        effects
            .iter()
            .filter(|effect| matches!(effect, Effect::ReadBody(_)))
            .count()
    }

    #[test]
    fn scrolling_reads_no_body_and_resting_reads_one() {
        let mut app = app((160, 40));
        let opening = opened(&mut app, 50);
        serve(&mut app, opening);
        let mut asked_to_rest = Vec::new();
        for _ in 0..10 {
            let effects = update(&mut app, press('j'));
            assert_eq!(reads(&effects), 0, "a keystroke reads no body");
            asked_to_rest.extend(rests(&effects));
        }
        // Every rest but the last is for a row the cursor has left.
        let mut read = 0;
        for message in asked_to_rest {
            read += reads(&update(&mut app, Input::Rested(message)));
        }
        assert_eq!(read, 1, "only the row the cursor stopped on is read");
    }

    #[test]
    fn a_body_that_arrives_is_drawn_in_the_reader() {
        let mut app = app((160, 40));
        let opening = opened(&mut app, 5);
        serve(&mut app, opening);
        let message = row(0).id;
        update(&mut app, Input::Rested(message));
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
        let drawn: String = reading
            .layout(chrono::Local::now())
            .0
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(drawn.contains("Hello Ada,"), "{drawn}");
        assert!(drawn.contains("▸ quoted text"), "{drawn}");
    }

    #[test]
    fn a_body_for_a_row_already_left_is_not_drawn() {
        let mut app = app((160, 40));
        let opening = opened(&mut app, 5);
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
        let opening = opened(app, 5);
        serve(app, opening);
        let message = row(0).id;
        update(app, Input::Rested(message));
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

    fn reader_text(app: &App) -> String {
        app.reading()
            .unwrap()
            .layout(chrono::Local::now())
            .0
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn in_the_reader_o_expands_the_quoted_history_and_folds_it_again() {
        let mut app = app((160, 40));
        reading_a_long_quoted_reply(&mut app);
        assert!(!reader_text(&app).contains("quoted words"));
        update(&mut app, key(KeyCode::Tab, KeyModifiers::NONE));
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
        let effects = opened(app, 1);
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
        let effects = update(app, Input::Rested(MessageId::new(3)));
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

    #[test]
    fn a_conversation_row_reads_every_message_in_it() {
        let mut app = app((160, 40));
        let effects = reading_a_conversation(&mut app);
        let reads: Vec<MessageId> = effects
            .iter()
            .filter_map(|effect| match effect {
                Effect::ReadBody(message) => Some(*message),
                _ => None,
            })
            .collect();
        assert_eq!(reads, [1, 2, 3].map(MessageId::new).to_vec());
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
        let drawn = reader_text(&app);
        for id in 1..=3 {
            assert!(drawn.contains(&format!("words of message {id}")), "{drawn}");
        }
        for who in ["ada@example.com", "bea@example.com", "cy@example.com"] {
            assert!(drawn.contains(who), "{who} heads their message: {drawn}");
        }
    }

    #[test]
    fn j_and_k_in_the_reader_walk_the_conversation() {
        let mut app = app((160, 40));
        reading_a_conversation(&mut app);
        update(&mut app, key(KeyCode::Tab, KeyModifiers::NONE));
        assert_eq!(app.reading().unwrap().current, 2, "it opens on the newest");
        update(&mut app, press('K'));
        assert_eq!(app.reading().unwrap().current, 1);
        let at_second = app.reader_top();
        update(&mut app, press('K'));
        assert_eq!(app.reading().unwrap().current, 0);
        assert!(app.reader_top() < at_second, "the reader moved up to it");
        update(&mut app, press('J'));
        assert_eq!(app.reading().unwrap().current, 1);
    }

    #[test]
    fn blocked_remote_images_are_counted_and_i_a_trusts_the_sender_everywhere() {
        let mut app = app((160, 40));
        let opening = opened(&mut app, 5);
        serve(&mut app, opening);
        let message = row(0).id;
        update(&mut app, Input::Rested(message));
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
    fn x_asks_to_leave_the_list_of_the_message_being_read() {
        let mut app = app((160, 40));
        reading_a_conversation(&mut app);
        update(&mut app, key(KeyCode::Tab, KeyModifiers::NONE));
        let effects = update(&mut app, key(KeyCode::Char('X'), KeyModifiers::SHIFT));
        assert!(
            effects.contains(&Effect::Unsubscribe(MessageId::new(3))),
            "the message being read, the newest: {effects:?}"
        );
        update(&mut app, Input::Unsubscribed(Ok("news.example.com".into())));
        assert_eq!(app.notice(), Some("Asked to leave news.example.com"));
    }

    #[test]
    fn a_messages_parts_are_listed_and_can_be_opened_or_saved() {
        let mut app =
            app((160, 40)).with_downloads(std::path::PathBuf::from("/home/ada/Downloads"));
        let effects = opened(&mut app, 1);
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
        update(&mut app, Input::Rested(message));
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

        update(&mut app, key(KeyCode::Tab, KeyModifiers::NONE));
        update(&mut app, press('p'));
        assert_eq!(app.focus(), Focus::Parts);
        let effects = update(&mut app, key(KeyCode::Enter, KeyModifiers::NONE));
        assert!(
            effects.contains(&Effect::SavePart {
                message,
                attachment: postio_model::ids::AttachmentId::new(4),
                to: None,
            }),
            "Enter opens: {effects:?}"
        );
        let effects = update(&mut app, press('s'));
        assert!(
            effects.contains(&Effect::SavePart {
                message,
                attachment: postio_model::ids::AttachmentId::new(4),
                to: Some(std::path::PathBuf::from("/home/ada/Downloads/report.pdf")),
            }),
            "s saves to Downloads: {effects:?}"
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
        let effects = opened(&mut app, 100);
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
