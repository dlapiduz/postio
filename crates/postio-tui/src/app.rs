//! The terminal frontend's state, and `update`, which turns an `Input` into the `Effect`s it asks
//! for and does no I/O, so every behaviour is driven by synthetic input. After every input `App`
//! asks only for the pages of rows in view that are neither here nor on their way (Principle V).

use crossterm::event::KeyEvent;
use postio_model::ListScope;
use postio_ui::keymap::{KeyContext, Outcome};
use postio_ui::list::ListWindow;

mod capture;
mod compose;
mod digest;
mod filtered;
mod find;
mod list;
mod open;
mod rules;
mod surface;

pub(crate) use compose::Writing;
pub use find::Find;
pub use open::{Menu, MenuAction, MenuItem, Raw, Thread};

use crate::focus::Held;
use crate::input::Keys;
use crate::row::Row;
use crate::view::list::{Heading, Visible};
use postio_body::replying::ReplyKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pointer {
    Click {
        hit: crate::view::hit::Hit,
        ctrl: bool,
        shift: bool,
    },
    Wheel {
        hit: crate::view::hit::Hit,
        down: bool,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum Input {
    Resize(u16, u16),
    ClipboardImage(Result<Option<Vec<u8>>, String>),
    InlineStored(Option<postio_model::Attachment>),
    RemoteImage {
        generation: u64,
        href: String,
        outcome: Result<postio_model::Attachment, postio_ui::compose::RemoteRefusal>,
    },
    ConfigEdited(Result<(), String>),
    Edited {
        generation: u64,
        edited: Result<String, String>,
    },
    Consent(Result<postio_ui::onboarding::BrowserSignIn, String>),
    Discovered(Result<postio_ui::onboarding::Status, String>),
    AccountAdded(Result<(), String>),
    Pointer(Pointer),
    Paste(String),
    Attached {
        path: std::path::PathBuf,
        attached: Option<postio_model::Attachment>,
    },
    Key(KeyEvent),
    FocusReply(postio_focus::Ticket, postio_focus::Reply),
    FocusTimer(u64),
    Host(postio_core::Event),
    Addressed {
        message: postio_model::MessageId,
        row: Box<postio_model::Message>,
        warning: Option<String>,
    },
    Body {
        message: postio_model::MessageId,
        answer: Result<postio_client::protocol::Body, String>,
    },
    Answer(crate::ask::Answer),
    Unsubscribed {
        message: postio_model::MessageId,
        answer: Result<String, String>,
    },
    ListMail {
        message: postio_model::MessageId,
        name: String,
        found: Result<postio_client::protocol::ListMailFound, String>,
    },
    Source {
        message: postio_model::MessageId,
        raw: Result<Vec<u8>, String>,
    },
    Settled {
        generation: u64,
    },
    DwellDue {
        generation: u64,
        message: postio_model::MessageId,
    },
    NoticeDue {
        generation: u64,
    },
    SignatureSaved(Result<(), String>),
    Privacy {
        log: postio_client::protocol::PrivacyLog,
        connections: Vec<postio_model::egress::EgressEvent>,
    },
    DraftSaved {
        generation: u64,
        composition: Option<u64>,
        saved: Result<postio_model::DraftId, String>,
    },
    Notified(postio_ui::notify::Notification),
    LabelPicker {
        account: postio_model::AccountId,
        labels: Vec<postio_model::Label>,
        counts: Vec<(postio_model::LabelId, u32)>,
        applied: std::collections::BTreeSet<postio_model::LabelId>,
    },
    LabelMade {
        label: Option<postio_model::Label>,
        close: bool,
    },
    RecentMoves(Vec<postio_model::MailboxId>),
    BarFound {
        sequence: u64,
        found: Result<Option<postio_client::protocol::Hits>, String>,
        held: Vec<(postio_model::MessageId, String, bool)>,
    },
    BarFolder {
        sequence: u64,
        count: u32,
        rows: Vec<crate::bar::ResultRow>,
    },
    PlaceDetails(crate::places::PlaceDetails),
    Recipients {
        prefix: String,
        found: Vec<postio_model::contact_group::RecipientCandidate>,
    },
    Queued {
        at: Option<chrono::DateTime<chrono::Utc>>,
        queued: Result<postio_model::DraftId, String>,
    },
    /// Taken back from the Outbox if queued; nothing when there is no local draft or its send has
    /// already started.
    Resumed {
        found: Option<Box<postio_model::Draft>>,
        failure: Option<String>,
    },
    ReplySource {
        kind: ReplyKind,
        found: Option<Box<(postio_model::Message, postio_model::Account)>>,
    },
    PartWritten {
        written: Result<std::path::PathBuf, String>,
        open: bool,
    },
    Places(crate::places::Places),
    Newer(Vec<postio_model::listing::MessageSummary>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    Redraw,
    Quit,
    Ask(crate::ask::Ask),
    ReadBody(postio_model::MessageId),
    RefreshPlaces,
    Unsubscribe(postio_model::MessageId),
    ReadListMail {
        message: postio_model::MessageId,
        name: String,
    },
    DesktopNotify {
        title: String,
        body: String,
    },
    ReadLabelPicker {
        message: postio_model::MessageId,
        account: postio_model::AccountId,
        threads: Vec<postio_model::ThreadId>,
    },
    CreateLabel {
        account: postio_model::AccountId,
        name: String,
        close: bool,
    },
    ReadRecentMoves,
    NoteMove(postio_model::MailboxId),
    SavePart {
        message: postio_model::MessageId,
        attachment: postio_model::ids::AttachmentId,
        to: Option<std::path::PathBuf>,
    },
    Launch(std::path::PathBuf),
    SaveAllowlist(postio_ui::allowlist::RemoteImageAllowList),
    SetReading(postio_config::Reading),
    ReadSource(postio_model::MessageId),
    Settle {
        generation: u64,
        after: std::time::Duration,
    },
    ArmDwell {
        generation: u64,
        message: postio_model::MessageId,
        after: std::time::Duration,
    },
    ExpireNotice {
        generation: u64,
        after: std::time::Duration,
    },
    SaveDraft {
        generation: u64,
        /// The controller's composition, which hears how the save went.
        composition: Option<u64>,
        draft: Box<postio_model::Draft>,
    },
    DiscardDraft {
        generation: u64,
        known: Option<postio_model::DraftId>,
    },
    /// Through the same writer as its saves, so it goes after the last of them.
    QueueSend {
        generation: u64,
        draft: Box<postio_model::Draft>,
        at: Option<chrono::DateTime<chrono::Utc>>,
    },
    EditExternally {
        generation: u64,
        markdown: String,
    },
    /// On the paste key, never otherwise.
    ReadClipboardImage,
    InlineImage {
        bytes: Vec<u8>,
        mime_type: String,
    },
    Attach(std::path::PathBuf),
    /// Only on a Download and attach press.
    AttachRemoteImage {
        generation: u64,
        draft: postio_model::DraftId,
        href: String,
    },
    SaveSearch(String),
    BarSearch(crate::bar::Ask),
    BarFolder {
        sequence: u64,
        mailbox: postio_model::MailboxId,
    },
    ReadPlaceDetails,
    Recipients {
        account: postio_model::AccountId,
        prefix: String,
    },
    Resume(postio_model::MessageId),
    ResumeDraft(postio_model::DraftId),
    /// Their rows as the store has them: nothing is fetched.
    ReadNewer(Vec<postio_model::MessageId>),
    EditConfig(Option<postio_ui::settings::Section>),
    Account(postio_client::protocol::AccountOp),
    BeginOAuth(Box<postio_ui::onboarding::Submission>),
    FinishOAuth(String),
    CancelOAuth(String),
    CopyText(String),
    Discover(String),
    AddAccount(Box<postio_ui::onboarding::Submission>),
    SaveSyncWindow(postio_ui::onboarding::SyncWindow),
    ReadPrivacy,
    EditSignature {
        account: postio_model::AccountId,
        signature: Option<postio_model::SignatureId>,
        name: String,
        text: String,
    },
    SaveSignature {
        account: postio_model::AccountId,
        signature: Option<postio_model::SignatureId>,
        name: String,
        text: String,
    },
    DeleteSignature(postio_model::SignatureId),
    OpenLink(String),
    ReplySource {
        kind: ReplyKind,
        message: postio_model::MessageId,
    },
    Send(postio_core::Command),
    Focus {
        ticket: postio_focus::Ticket,
        request: postio_focus::Request,
    },
    FocusTimer {
        token: u64,
        after: std::time::Duration,
    },
}

pub const TOAST: std::time::Duration =
    std::time::Duration::from_secs(postio_ui::focus_target::TOAST_SECONDS as u64);

const WHEEL: isize = 3;

pub struct App {
    size: (u16, u16),
    keys: Keys,
    list: ListWindow<Held>,
    ctl: postio_focus::FocusController,
    selection_words: Option<String>,
    cursor: u32,
    top: u32,
    /// The client sends a snapshot of it with each command (ADR 0041).
    state: postio_core::SharedState,
    selection: postio_ui::selection::SelectionState,
    counts: Option<postio_client::protocol::FocusCounts>,
    features: crate::places::Features,
    tracked: Vec<postio_model::AccountId>,
    last_synced: Option<chrono::DateTime<chrono::Utc>>,
    scope: Option<ListScope>,
    notice: Option<String>,
    notice_tone: Tone,
    notice_undo: Option<String>,
    notice_offer: Option<(String, postio_core::Command)>,
    notice_generation: u64,
    /// An answer to an invitation was sent; its toast lasts as long as the reply waits.
    answering: bool,
    focus: Focus,
    places: crate::places::Places,
    privacy: Option<Privacy>,
    renaming: Option<Renaming>,
    trackers: postio_ui::status::Trackers,
    account: Option<postio_model::AccountId>,
    folders: Vec<postio_model::mailbox::Mailbox>,
    reading: Option<crate::conversation::Reading>,
    reader_top: usize,
    open: open::Open,
    downloads: std::path::PathBuf,
    allowlist: postio_ui::allowlist::RemoteImageAllowList,
    composer: Option<crate::composer::Composer>,
    compositions: u64,
    accounts: Vec<postio_model::Account>,
    scheduling: Option<[(&'static str, chrono::DateTime<chrono::Local>); 4]>,
    asked_at: Option<u64>,
    path_prompt: Option<tui_input::Input>,
    preview: postio_config::Preview,
    previewing: bool,
    saved_at: Option<chrono::DateTime<chrono::Utc>>,
    closed_composer: Option<crate::composer::Composer>,
    writing: compose::Writing,
    detached: bool,
    bar: Option<crate::bar::Bar>,
    places_box: Option<crate::folders::Folders>,
    picker: Option<crate::pickers::Picker>,
    palette: Option<PaletteState>,
    /// Otherwise only the chords a legacy terminal can send arrive.
    enhanced_keys: bool,
    sheet: Option<crate::sheet::Sheet>,
    armed_link: Option<String>,
    mouse: bool,
    first_run: Option<crate::first_run::FirstRun>,
    settings: Option<crate::settings::Settings>,
    surfaces: crate::surface::Surfaces,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Strip {
    pub place: postio_ui::terminal::SafeText,
    pub counts: String,
    pub toggle: Option<Toggle>,
    pub showing: Option<String>,
    pub filtered: Option<String>,
    pub rules: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Toggle {
    pub label: String,
    pub on: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Front {
    List,
    Reader,
    Composer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Plain,
    Worked,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Renaming {
    Signature {
        account: postio_model::AccountId,
        signature: Option<postio_model::SignatureId>,
        text: String,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Privacy {
    pub log: postio_client::protocol::PrivacyLog,
    pub connections: Vec<postio_model::egress::EgressEvent>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Finding {
    Roles(postio_model::AccountId),
    RoleFolder(postio_model::AccountId, postio_model::mailbox::MailboxRole),
    Rename,
}

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

#[derive(Debug)]
struct PaletteState {
    input: tui_input::Input,
    selected: usize,
    from: Focus,
    finding: Finding,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaletteView {
    pub marker: &'static str,
    pub query: String,
    pub rows: Vec<PaletteRow>,
    pub selected: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaletteRow {
    pub title: String,
    pub chord: Option<String>,
    pub positions: Vec<usize>,
}

enum PaletteAction {
    PickRole(postio_model::AccountId, postio_model::mailbox::MailboxRole),
    MapRole(
        postio_model::AccountId,
        postio_model::mailbox::MailboxRole,
        Option<String>,
    ),
    Rename(String),
}

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Focus {
    #[default]
    List,
    Reader,
    Composer,
    Bar,
    Folders,
    Picker,
    Keys,
    FirstRun,
    Settings,
    Palette,
    Filtered,
    Digest,
    Rules,
    RuleDialog,
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
            closed_composer: None,
            writing: compose::Writing::default(),
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

    pub fn with_downloads(mut self, downloads: std::path::PathBuf) -> App {
        self.downloads = downloads;
        self
    }

    pub fn with_allowlist(mut self, allowlist: postio_ui::allowlist::RemoteImageAllowList) -> App {
        self.allowlist = allowlist;
        self
    }

    pub fn top(&self) -> u32 {
        self.top
    }

    pub fn reader_top(&self) -> usize {
        self.reader_top
    }

    pub fn row(&self, message: postio_model::MessageId) -> Option<&Row> {
        self.list.row_of(message).map(|held| &held.row)
    }

    pub fn reading(&self) -> Option<&crate::conversation::Reading> {
        self.reading.as_ref()
    }

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

    pub fn focus(&self) -> Focus {
        self.focus
    }

    pub fn with_preview(mut self, preview: postio_config::Preview) -> App {
        self.preview = preview;
        self
    }

    pub fn preview_shown(&self) -> Option<postio_config::Preview> {
        self.previewing.then_some(self.preview)
    }

    pub fn composer_detached(&self) -> bool {
        self.detached
    }

    pub fn front(&self) -> Front {
        let at = self.palette.as_ref().map_or(self.focus, |open| open.from);
        match at {
            Focus::Composer if self.composer.is_some() => Front::Composer,
            Focus::Reader if self.reading.is_some() => Front::Reader,
            _ => Front::List,
        }
    }

    pub fn composer(&self) -> Option<&crate::composer::Composer> {
        self.composer.as_ref()
    }

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
        self.saved_at = None;
        self.focus = Focus::Composer;
        self.detached = false;
        self.previewing = self.preview == postio_config::Preview::Split;
        let mut effects = Vec::new();
        // One the controller did not open is told of, so its keys and its save reach it.
        if self.ctl.top_surface() != Some(postio_focus::SurfaceKind::Composer) {
            effects.extend(self.tell(postio_focus::Input::SurfaceOpened(
                postio_focus::SurfaceKind::Composer,
            )));
        }
        effects.push(Effect::Redraw);
        effects
    }

    pub fn saved_note(&self) -> Option<String> {
        self.saved_at.map(postio_ui::compose::saved_at)
    }

    /// The keymap is asked in text entry, so a plain letter is handed back rather than resolved
    /// (FR-022a).
    fn composer_key(&mut self, key: &KeyEvent) -> Vec<Effect> {
        if self.writing.on_chooser {
            return self.chooser_key(key);
        }
        if self.scheduling.is_some() {
            return self.schedule_key(key);
        }
        if self.path_prompt.is_some() {
            return self.path_key(key);
        }
        if let Some(composer) = self.composer.as_mut() {
            let before = composer.edits();
            if composer.completion_key(*key) {
                let edited = composer.edits() != before;
                let mut effects = vec![Effect::Redraw];
                if edited {
                    effects.extend(self.tell(postio_focus::Input::ComposerEdited));
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
                let edited = composer.edits() != before;
                if let Some(prefix) = composer.wants_completion() {
                    effects.push(Effect::Recipients {
                        account: composer.draft().account_id,
                        prefix,
                    });
                }
                if edited {
                    effects.extend(self.tell(postio_focus::Input::ComposerEdited));
                }
                effects
            }
        }
    }

    pub fn path_prompt(&self) -> Option<&str> {
        self.path_prompt.as_ref().map(tui_input::Input::value)
    }

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
                        effects.extend(self.tell(postio_focus::Input::ComposerEdited));
                    }
                }
            }
        }
        effects.push(Effect::Redraw);
        effects
    }

    pub fn rekey(&mut self, keys: Keys) {
        self.keys = keys;
    }

    pub fn settings(&self) -> Option<&crate::settings::Settings> {
        self.settings.as_ref()
    }

    pub fn privacy(&self) -> Option<&Privacy> {
        self.privacy.as_ref()
    }

    pub fn allowlist(&self) -> &postio_ui::allowlist::RemoteImageAllowList {
        &self.allowlist
    }

    pub fn accounts(&self) -> &[postio_model::Account] {
        &self.accounts
    }

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
            if settings.current() == Section::Privacy && was != Section::Privacy {
                return vec![Effect::ReadPrivacy, Effect::Redraw];
            }
            return vec![Effect::Redraw];
        }
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => settings.step_row(-1, count),
            KeyCode::Down | KeyCode::Char('j') => settings.step_row(1, count),
            KeyCode::Tab | KeyCode::BackTab => settings.set_in_list(false),
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

    fn settings_command(&mut self, id: &str) -> Vec<Effect> {
        use postio_client::protocol::AccountOp;
        let Some(settings) = self.settings.as_mut() else {
            return Vec::new();
        };
        if settings.current() == postio_ui::settings::Section::Privacy && settings.in_list() {
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

    pub fn first_run(&self) -> Option<&crate::first_run::FirstRun> {
        self.first_run.as_ref()
    }

    fn first_run_key(&mut self, key: &KeyEvent) -> Vec<Effect> {
        let Some(first_run) = self.first_run.as_mut() else {
            return Vec::new();
        };
        // Escape gives a browser sign-in up, one step back, rather than leaving the first run.
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

    /// Off, a click does nothing, and the terminal's own selection works.
    pub fn with_mouse(mut self, mouse: bool) -> App {
        self.mouse = mouse;
        self
    }

    #[cfg(test)]
    pub(crate) fn set_reading_for_tests(&mut self, reading: crate::conversation::Reading) {
        self.reading = Some(reading);
        self.focus = Focus::Reader;
    }

    pub fn selection(&self) -> &postio_ui::selection::SelectionState {
        &self.selection
    }

    pub fn selection_summary(&self) -> Option<String> {
        self.selection_words.clone()
    }

    fn pointer(&mut self, pointer: Pointer) -> Vec<Effect> {
        use crate::view::hit::Target;
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
                Target::MenuRow(at) => self.choose(at),
                Target::NoticeOffer => self.take_up_offer(),
                // An answer to an invitation is about the invitation wherever the cursor is; the
                // rest take the cursor there first.
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
                Target::Command("save_search") if self.bar.is_some() => self.bar_save(),
                Target::Command("back") if self.bar.is_some() => self.close_bar(),
                Target::Command(id) => self.command(id),
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
            At::Fold(block) => {
                reading.shown.toggle_fold(block);
                vec![Effect::Redraw]
            }
            // Nothing leaves this machine on one click: the first shows the whole destination, and
            // only a second on the same link opens it.
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
        }
    }

    fn scroll_list(&mut self, lines: isize) {
        let last_top = self.top_for_bottom(self.list.total().saturating_sub(1));
        let top = i64::from(self.top) + lines as i64;
        self.top = u32::try_from(top.clamp(0, i64::from(last_top))).unwrap_or(0);
        let shown = self.fit_from(self.top).max(1);
        self.cursor = self.cursor.clamp(self.top, self.top + shown - 1);
    }

    pub fn with_enhanced_keys(mut self, enhanced: bool) -> App {
        self.enhanced_keys = enhanced;
        self
    }

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

    pub fn bar(&self) -> Option<(&crate::bar::Bar, crate::bar::Ctx<'_>)> {
        let bar = self.bar.as_ref()?;
        Some((bar, self.bar_ctx(bar.from(), self.keys.keymap())))
    }

    fn bar_ctx<'a>(&self, from: Focus, keymap: &'a postio_core::Keymap) -> crate::bar::Ctx<'a> {
        crate::bar::Ctx {
            keymap,
            context: Self::context_of(from),
            state: self.availability(),
            enhanced: self.enhanced_keys,
        }
    }

    pub fn bar_typed(&self) -> Option<&str> {
        self.bar.as_ref().map(crate::bar::Bar::typed)
    }

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

    fn open_found(
        &mut self,
        message: postio_model::MessageId,
        found: Option<&crate::bar::ResultRow>,
    ) -> Vec<Effect> {
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

    pub fn picker(&self) -> Option<(&crate::pickers::Picker, &postio_core::Keymap)> {
        Some((self.picker.as_ref()?, self.keys.keymap()))
    }

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
                    self.edited()
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

    pub fn folders(&self) -> Option<(&crate::folders::Folders, crate::folders::Reach<'_>)> {
        Some((self.places_box.as_ref()?, self.folders_reach()))
    }

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

    fn availability(&self) -> postio_core::Availability {
        postio_core::Availability {
            frontend: postio_core::Frontend::Terminal,
            ..postio_core::Availability::open(
                self.account
                    .map_or(postio_core::Scope::Unified, postio_core::Scope::Account),
            )
        }
    }

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

    fn palette_rows(&self, state: &PaletteState) -> Vec<(PaletteRow, PaletteAction)> {
        let query = state.input.value();
        match state.finding {
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

    pub fn scheduling(&self) -> Option<&[(&'static str, chrono::DateTime<chrono::Local>)]> {
        self.scheduling.as_ref().map(|times| times.as_slice())
    }

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
        // The send is the write: the composer closes with nothing to save or discard.
        self.composer = None;
        self.writing = compose::Writing::default();
        self.asked_at = None;
        self.focus = Focus::List;
        let mut effects = vec![
            Effect::QueueSend {
                generation,
                draft: Box::new(draft),
                at,
            },
            Effect::Redraw,
        ];
        effects.extend(self.tell(postio_focus::Input::SurfaceClosed(
            postio_focus::SurfaceKind::Composer,
        )));
        effects
    }

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
            // A terminal's own paste carries text only, so this is the one place the clipboard is
            // read (FR-026).
            "insert_image" => vec![Effect::ReadClipboardImage],
            "attach_file" => {
                self.path_prompt = Some(tui_input::Input::default());
                vec![Effect::Redraw]
            }
            "schedule_send" => {
                self.scheduling = Some(postio_ui::schedule::schedule_presets(chrono::Local::now()));
                vec![Effect::Redraw]
            }
            // The draft is not lost by leaving: it is autosaved into Drafts. A draft that has the
            // whole screen gives it back first.
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
            "back" | "discard_draft" => {
                self.tell(postio_focus::Input::Command(postio_core::CommandId::Back))
            }
            "forward_what_1" | "forward_what_2" | "forward_what_3" | "show_newer"
            | "expand_quoted" | "reply_to_latest" => match id.parse() {
                Ok(command) => self.tell(postio_focus::Input::Command(command)),
                Err(_) => Vec::new(),
            },
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
                        composition: None,
                        draft: Box::new(composer.draft()),
                    }]
                })
                .unwrap_or_default(),
            id if crate::composer::Format::for_command(id).is_some() => {
                let Some(format) = crate::composer::Format::for_command(id) else {
                    return Vec::new();
                };
                let Some(composer) = self.composer.as_mut() else {
                    return Vec::new();
                };
                if !composer.format(format) {
                    return self.say("Formatting is for the message's body");
                }
                self.edited()
            }
            // One request a press, for the first image not downloading.
            "attach_remote_image" => {
                let Some(composer) = self.composer.as_mut() else {
                    return Vec::new();
                };
                match composer.start_remote() {
                    Some(href) => vec![
                        Effect::AttachRemoteImage {
                            generation: composer.generation(),
                            draft: composer.draft().id,
                            href: href.as_str().to_owned(),
                        },
                        Effect::Redraw,
                    ],
                    None => self.say(postio_ui::compose::NO_REMOTE_IMAGE),
                }
            }
            "copy_fields" => {
                if let Some(composer) = self.composer.as_mut() {
                    composer.toggle_copy_fields();
                }
                vec![Effect::Redraw]
            }
            _ => self.command(id),
        }
    }

    pub fn hint(&self, command: postio_core::CommandId) -> Option<String> {
        postio_ui::terminal::deliverable_binding(self.keys.keymap(), command, self.enhanced_keys)
    }

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

    pub fn notice(&self) -> Option<&str> {
        self.notice.as_deref()
    }

    /// Through `SafeText`: an error can quote a folder name, which came from a server.
    fn say(&mut self, sentence: &str) -> Vec<Effect> {
        self.say_as(Tone::Plain, sentence, None)
    }

    fn say_as(&mut self, tone: Tone, sentence: &str, undo: Option<String>) -> Vec<Effect> {
        self.say_for(tone, sentence, undo, TOAST)
    }

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

    pub fn notice_tone(&self) -> Tone {
        self.notice_tone
    }

    pub fn notice_undo(&self) -> Option<&str> {
        self.notice_undo.as_deref()
    }

    pub fn notice_offer(&self) -> Option<&str> {
        self.notice_offer.as_ref().map(|(label, _)| label.as_str())
    }

    pub fn with_state(mut self, state: postio_core::SharedState) -> App {
        self.state = state;
        self
    }

    pub fn state(&self) -> postio_core::SharedState {
        self.state.clone()
    }

    pub fn has_action(&self) -> bool {
        self.scope == Some(ListScope::Focus(postio_model::FocusScope::HasAction))
    }

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

    pub fn place_name(&self) -> postio_ui::terminal::SafeText {
        match self.scope {
            Some(scope) => crate::places::name_of(&self.places, scope),
            None => postio_ui::terminal::SafeText::new(""),
        }
    }

    pub fn fits(&self) -> bool {
        crate::layout::fits(self.size.0, self.size.1)
    }

    pub fn window(&self) -> crate::layout::Window {
        crate::layout::window(
            ratatui::layout::Rect::new(0, 0, self.size.0, self.size.1),
            self.banner().is_some(),
        )
    }

    pub fn cursor(&self) -> u32 {
        self.cursor
    }

    pub fn total(&self) -> u32 {
        self.list.total()
    }

    fn list_lines(&self) -> u16 {
        self.window().list.height
    }

    pub fn row_at(&self, position: u32) -> Option<&Row> {
        self.list.resident_at(position).map(|held| &held.row)
    }

    fn heading_at(&self, position: u32, top: u32) -> Option<Heading> {
        let row = self.row_at(position)?;
        if self.has_action() {
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

    fn lines_at(&self, position: u32, top: u32) -> u16 {
        crate::view::list::lines_of(self.row_at(position))
            + u16::from(self.heading_at(position, top).is_some())
    }

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

    fn top_for_bottom(&self, cursor: u32) -> u32 {
        let room = self.list_lines();
        let day = |position: u32| u16::from(position > 0 && self.heading_at(position, 0).is_some());
        let mut top = cursor;
        let mut used = crate::view::list::lines_of(self.row_at(cursor)) + day(cursor);
        while top > 0 {
            let before = top - 1;
            let lines = crate::view::list::lines_of(self.row_at(before));
            if used.saturating_add(lines + 1) > room {
                break;
            }
            used += lines + day(before);
            top = before;
        }
        top
    }

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

    fn cursor_message(&self) -> Option<postio_model::MessageId> {
        self.list.peek(self.cursor)
    }

    /// Leaving saves what is being written on the way out.
    fn quit(&mut self) -> Vec<Effect> {
        let mut effects = if self.composer.is_some() {
            self.tell(postio_focus::Input::SurfaceClosed(
                postio_focus::SurfaceKind::Composer,
            ))
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
        if self.focus == Focus::Reader
            && self.own_aim().is_none()
            && let Ok(command) = id.parse::<postio_core::CommandId>()
            && Self::the_readers(command)
        {
            return self.tell_reader(command);
        }
        match id {
            "find_in_message" => return self.open_find(),
            "find_next" => return self.find_step(true),
            "find_previous" => return self.find_step(false),
            "quit" => return self.quit(),
            // A message opened from Filtered steps through that list, which the controller does
            // not hold.
            "next_message" | "prev_message" if self.focus == Focus::Reader => {
                let by = if id == "next_message" { 1 } else { -1 };
                return self.step_filtered_open(by);
            }
            "reply" | "reply_all" | "forward"
                if matches!(self.focus, Focus::List) && self.own_aim().is_none() =>
            {
                if let Ok(command) = id.parse::<postio_core::CommandId>() {
                    return self.tell(postio_focus::Input::Command(command));
                }
            }
            "reply" | "reply_all" | "forward" => {
                let kind = match id {
                    "reply" => ReplyKind::Reply,
                    "reply_all" => ReplyKind::ReplyAll,
                    _ => ReplyKind::Forward,
                };
                let message = self
                    .reading
                    .as_ref()
                    .map(|reading| reading.shown.id)
                    .or_else(|| self.cursor_message());
                if let Some(message) = message {
                    return vec![Effect::ReplySource { kind, message }];
                }
            }
            // A digest opens in its own window.
            "open_message" if self.focus == Focus::List && self.cursor_is_digest() => {
                return self.open_digest();
            }
            "open_message" if self.focus == Focus::List => {
                return self.tell(postio_focus::Input::Command(
                    postio_core::CommandId::OpenMessage,
                ));
            }
            "open_message" if self.focus == Focus::Reader && self.open_draft_offers_edit() => {
                if let Some(message) = self.reading.as_ref().map(|reading| reading.row) {
                    let mut effects = self.close_message();
                    effects.push(Effect::Resume(message));
                    return effects;
                }
            }
            "compose" if self.composer.is_some() => {
                self.focus = Focus::Composer;
                return vec![Effect::Redraw];
            }
            "compose" => {
                return self.tell(postio_focus::Input::Command(
                    postio_core::CommandId::Compose,
                ));
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
            "toggle_result_order" | "save_search" | "back_to_words" => {}
            "back" if self.focus == Focus::Reader => return self.back_from_own(),
            "picker_choose_1" | "picker_choose_2" | "picker_choose_3" | "picker_choose_4"
            | "picker_type_date" | "picker_toggle" | "picker_confirm" => {}
            "back" if self.focus != Focus::List => self.focus = Focus::List,
            "expand_all" => self.toggle_folds(),
            "show_images" => return self.allow_images(false),
            "always_show_images" => return self.allow_images(true),
            "clear_list_mail" => {
                if self.notice_offer.is_some() {
                    return self.take_up_offer();
                }
                return self.say(postio_ui::unsubscribe::NO_OFFER);
            }
            "unsubscribe" => {
                return self
                    .reading
                    .as_ref()
                    .map(|reading| vec![Effect::Unsubscribe(reading.shown.id)])
                    .unwrap_or_default();
            }
            "toggle_reading_pane" => return self.toggle_reading_pane(),
            "view_source" => return self.view_source(),
            "open_attachment_or_link" => return self.offer_choices(),
            "more_actions" => return self.more_actions(),
            "toggle_read" => {
                self.cancel_dwell();
                return self.send("toggle_read");
            }
            "scroll_reader_down" => self.scroll_reader(1),
            "scroll_reader_up" => self.scroll_reader(-1),
            // One toggle in a terminal: reader view is the readable form of bulk mail here
            // (FR-031).
            "view_original" | "toggle_reader_view" => return self.view_original(),
            "toggle_fold" => {
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
            "accept_invite" | "decline_invite" => {
                let message = self
                    .reading
                    .as_ref()
                    .map(|reading| reading.shown.id)
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
            "capture_change_project" | "capture_use_subject" | "capture_write" => {
                return Vec::new();
            }
            "sweep_inbox" => return self.ask_sweep(),
            "go_to_inbox" => return self.go_to(postio_model::mailbox::MailboxRole::Inbox),
            "go_to_sent" => return self.go_to(postio_model::mailbox::MailboxRole::Sent),
            "go_to_drafts" => return self.go_to(postio_model::mailbox::MailboxRole::Drafts),
            "go_to_flagged" => return self.go_to(postio_model::mailbox::MailboxRole::Flagged),
            "go_to_archive" => return self.go_to(postio_model::mailbox::MailboxRole::Archive),
            "go_to_snoozed" => return self.go_to(postio_model::mailbox::MailboxRole::Snoozed),
            "go_to_outbox" => return self.go_to(postio_model::mailbox::MailboxRole::Outbox),
            "go_to_junk" => return self.go_to(postio_model::mailbox::MailboxRole::Junk),
            "go_to_trash" => return self.go_to(postio_model::mailbox::MailboxRole::Trash),
            "saved_search_1" => return self.pinned_search(0),
            "saved_search_2" => return self.pinned_search(1),
            "saved_search_3" => return self.pinned_search(2),
            "saved_search_4" => return self.pinned_search(3),
            "prev_view" if self.focus == Focus::Reader => return self.back_from_own(),
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

    fn view_original(&mut self) -> Vec<Effect> {
        let Some(member) = self.reading.as_mut().map(|reading| &mut reading.shown) else {
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

    fn account_here(&self) -> Option<postio_model::AccountId> {
        self.account.or_else(|| {
            self.accounts
                .iter()
                .find(|account| account.enabled)
                .map(|account| account.id)
        })
    }

    fn go_to(&mut self, role: postio_model::mailbox::MailboxRole) -> Vec<Effect> {
        use postio_model::mailbox::MailboxRole;
        let Some(account) = self.account_here() else {
            return Vec::new();
        };
        let scope = match role {
            MailboxRole::Flagged => Some(ListScope::Flagged(account)),
            MailboxRole::Snoozed => Some(ListScope::Snoozed(account)),
            MailboxRole::Outbox => Some(ListScope::Outbox(account)),
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

    /// A move with no folder and a label with no label are half a request: `None` means "ask".
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

    fn send_aimed(&mut self, id: postio_core::CommandId) -> Vec<Effect> {
        let reachable: Vec<postio_model::AccountId> = self
            .accounts
            .iter()
            .filter(|account| account.enabled)
            .map(|account| account.id)
            .collect();
        let (quiet, _) = postio_core::bridge::event_channel();
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
        self.selection.clear();
        vec![Effect::Send(command)]
    }

    fn toggle_folds(&mut self) {
        if let Some(reading) = self.reading.as_mut() {
            reading.shown.toggle_folds();
        }
    }

    /// "Always" writes the allow list the desktop app reads too.
    fn allow_images(&mut self, always: bool) -> Vec<Effect> {
        let Some(member) = self.reading.as_mut().map(|reading| &mut reading.shown) else {
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
        vec![
            Effect::Redraw,
            Effect::SaveAllowlist(self.allowlist.clone()),
        ]
    }

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
        let asked_original = self
            .reading
            .as_ref()
            .is_some_and(|reading| reading.shown.id == message && reading.shown.original);
        let mut source = None;
        let mut reader_view = false;
        let rendered = match answer {
            Ok(Body::Ready { body, .. }) if body.html.is_some() => {
                let rendering = if !asked_original && suits_reader_view(&body) {
                    Rendering::Reader
                } else {
                    Rendering::Original
                };
                reader_view = rendering == Rendering::Reader;
                source = Some(body.clone());
                // Blocked always: a terminal draws no image, so allowing changes what the notice
                // says, never what is fetched.
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
        let Some(member) = self
            .reading
            .as_mut()
            .map(|reading| &mut reading.shown)
            .filter(|member| member.id == message)
        else {
            return Vec::new();
        };
        member.body = Some(rendered);
        member.source = source;
        member.reader_view = reader_view;
        member.held_back = held_back;
        member.images_allowed = member
            .address
            .as_deref()
            .is_some_and(|address| self.allowlist.is_allowed(address));
        vec![Effect::Redraw]
    }

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
            if self.first_run.is_none() {
                self.first_run = Some(crate::first_run::FirstRun::default());
                self.focus = Focus::FirstRun;
            }
            return vec![Effect::Redraw];
        }
        // An account arriving from elsewhere ends the first run, unless it is this run's own
        // account with the last question still open.
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
            effects.extend(self.open_scope(ListScope::Focus(postio_model::FocusScope::Inbox)));
        }
        effects
    }

    fn named(&mut self, name: String) -> Vec<Effect> {
        let effect = match self.renaming.take() {
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

    fn signatures_key(&mut self, account: postio_model::AccountId, key: &KeyEvent) -> Vec<Effect> {
        use crossterm::event::KeyCode;
        // Asked of the keymap only for a key the list does not use itself.
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

    /// What changed in the store goes through the same paging plan the desktop list follows.
    fn hear(&mut self, event: &postio_core::Event) -> Vec<Effect> {
        use postio_core::Event;
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
            // Only a message already drawn is read again: a read still on its way will see the
            // words, and the list is never re-read (#1788).
            Event::BodyLoaded { message, .. }
            | Event::BodyFailed { message, .. }
            | Event::BodyReachable { message, .. } => {
                let drawn = self.reading.as_ref().is_some_and(|reading| {
                    reading.shown.id == *message && reading.shown.body.is_some()
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

pub fn update(app: &mut App, input: Input) -> Vec<Effect> {
    let mut effects = match input {
        Input::Resize(width, height) => {
            app.size = (width, height);
            vec![Effect::Redraw]
        }
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
        Input::SignatureSaved(saved) => match saved {
            Ok(()) => vec![Effect::RefreshPlaces, Effect::Redraw],
            Err(reason) => app.say(&reason),
        },
        Input::Privacy { log, connections } => {
            app.privacy = Some(Privacy { log, connections });
            vec![Effect::Redraw]
        }
        Input::DraftSaved {
            generation,
            composition,
            saved,
        } => {
            let told = match &saved {
                Ok(id) => {
                    if let Some(composer) = app.composer.as_mut()
                        && composer.generation() == generation
                    {
                        composer.adopt_id(*id);
                        app.saved_at = Some(chrono::Utc::now());
                    }
                    Ok(true)
                }
                Err(reason) => Err(reason.clone()),
            };
            match composition {
                Some(composition) => app.tell(postio_focus::Input::DraftSaved {
                    composition,
                    saved: told,
                }),
                None => match saved {
                    Ok(_) => Vec::new(),
                    Err(reason) => app.say(&reason),
                },
            }
        }
        Input::Edited { generation, edited } => match (edited, app.composer.as_mut()) {
            (Ok(markdown), Some(composer)) if composer.generation() == generation => {
                composer.replace_body(&markdown);
                app.edited()
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
        Input::RemoteImage {
            generation,
            href,
            outcome,
        } => match app.composer.as_mut() {
            Some(composer) if composer.generation() == generation => {
                composer.settle_remote(&href, outcome);
                app.edited()
            }
            _ => Vec::new(),
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
                app.edited()
            }
            (Some(_), None) => app.say("The draft closed before the image was stored"),
            (None, _) => app.say("The image could not be stored"),
        },
        Input::Discovered(found) => {
            if let Some(first_run) = app.first_run.as_mut() {
                match found {
                    Ok(status) => first_run.discovered(status),
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
                app.edited()
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
        Input::Queued { at, queued } => match queued {
            Ok(draft) => app.tell(postio_focus::Input::SendQueued { draft, at }),
            Err(reason) => app.say(&format!(
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
            Ok(name) => vec![Effect::ReadListMail { message, name }],
            Err(reason) => app.say(&reason),
        },
        Input::ListMail { name, found, .. } => app.confirm_unsubscribed(&name, found.ok()),
        Input::Addressed {
            message,
            row,
            warning,
        } => app.addressed(message, &row, warning.as_deref()),
        Input::Body { message, answer } => {
            if app
                .reading
                .as_ref()
                .is_some_and(|reading| reading.shown.id == message)
            {
                app.release_paint();
            }
            app.show(message, answer)
        }
        Input::Settled { generation } => app.settled(generation),
        Input::Newer(read) => app.newer_read(read),
    };
    effects.extend(app.follow_cursor());
    effects.extend(app.fetches());
    effects.extend(app.filtered_fetches());
    effects.extend(app.hold_paint(&effects));
    effects
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::app;

    #[test]
    fn download_and_attach_asks_once_a_press_and_puts_the_picture_in() {
        let mut app = app((120, 40));
        composing(&mut app);
        let composer = app.composer.as_mut().expect("composing");
        composer.focus_field(crate::composer::Field::Body);
        composer.replace_body("See ![chart](https://example.com/x.png).");
        let generation = composer.generation();
        let drawn = crate::test_support::screen(120, 40, &app);
        assert!(drawn.contains("Not attached yet"), "{drawn}");
        assert!(
            drawn.contains("chart  example.com  Download and attach alt+I"),
            "{drawn}"
        );
        let asked = app.composer_command("attach_remote_image");
        let requests: Vec<_> = asked
            .iter()
            .filter(|effect| matches!(effect, Effect::AttachRemoteImage { href, .. } if href == "https://example.com/x.png"))
            .collect();
        assert_eq!(requests.len(), 1, "{asked:?}");
        let mut part = postio_model::Attachment::new(
            postio_model::ids::MessageId::UNASSIGNED,
            "image/png",
            10,
        );
        part.content_id = Some("x1@postio.invalid".to_owned());
        update(
            &mut app,
            Input::RemoteImage {
                generation,
                href: "https://example.com/x.png".to_owned(),
                outcome: Ok(part),
            },
        );
        let composer = app.composer.as_ref().expect("composing");
        assert_eq!(composer.markdown(), "See ![chart](cid:x1@postio.invalid).");
        let drawn = crate::test_support::screen(120, 40, &app);
        assert!(!drawn.contains("Not attached yet"), "{drawn}");
        let again = app.composer_command("attach_remote_image");
        assert!(
            !again
                .iter()
                .any(|effect| matches!(effect, Effect::AttachRemoteImage { .. })),
            "nothing left to download, so nothing is asked"
        );
    }

    #[test]
    fn every_now_phase_formatting_command_edits_the_body_and_starts_the_autosave_clock() {
        for id in [
            "strikethrough",
            "inline_code",
            "heading_2",
            "heading_6",
            "normal_text",
            "code_block",
            "task_list",
            "toggle_task",
            "indent",
            "outdent",
            "divider",
            "table",
            "clear_formatting",
        ] {
            let changed = ["  - [ ] **two**", "## **Plan**"].iter().any(|body| {
                let mut app = app((120, 40));
                composing(&mut app);
                let composer = app.composer.as_mut().expect("composing");
                composer.focus_field(crate::composer::Field::Body);
                composer.replace_body(body);
                let before = composer.markdown();
                let effects = app.composer_command(id);
                let after = app.composer.as_ref().expect("composing").markdown();
                effects
                    .iter()
                    .any(|effect| matches!(effect, Effect::FocusTimer { .. }))
                    && after != before
            });
            assert!(changed, "`{id}` did not edit the body");
        }
    }

    fn composing(app: &mut App) {
        app.compose(postio_model::Draft::new(postio_model::AccountId::new(1)));
    }
}
