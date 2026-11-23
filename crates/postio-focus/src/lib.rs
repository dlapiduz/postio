//! Focus's behaviour, decided once (ADR 0045). A frontend turns what happens to it into an
//! `Input`, hands it to the `FocusController`, and does what the returned `Effect`s say. The
//! controller does no I/O and never awaits, so GTK and the FFI can both drive it.

use std::time::Duration;

use postio_client::protocol::FocusCounts;
use postio_config::paths::Platform;
use postio_core::state::Selection;
use postio_core::{Command, CommandId, Event, MessageTarget};
use postio_model::listing::{PageRequest, Surfaced};
use postio_model::{AccountId, ListScope, MailboxId, MailboxRole, MessageId, ThreadId, ThreadView};
use postio_ui::focus_list::FocusRow;

mod bar;
mod capture;
mod compose;
mod confirm;
mod contact_edit;
mod contact_merge;
mod contact_popover;
mod contacts;
mod contacts_settings;
mod cursor;
mod digest;
mod dropdown;
mod feed;
mod filtered;
mod footer;
mod history;
mod keys;
mod perform;
mod pickers;
mod results;
mod states;
mod surfaces;
mod thread_expand;
mod thread_menu;
mod verbs;
mod walk;

pub use bar::{BarLine, BarLineKind, BarMode, BarView, PlacesRead};
pub use capture::{CaptureProject, CaptureView};
pub use compose::{AUTOSAVE, ComposerKind};
pub use confirm::Confirm;
pub use contact_edit::EditChange;
pub use contacts::{ContactsView, FILTER_CAP};
pub use cursor::{NoRows, RowFacts, Rows};
pub use digest::{
    DigestCard, DigestEmail, DigestLine, DigestStatement, DigestTopic, DigestView, RulePreviewLine,
    RuleView,
};
pub use dropdown::{
    DropdownPill, DropdownRow, DropdownRowKind, DropdownSection, DropdownState, DropdownView, Lane,
    Run, RunStyle, UnderstoodTile,
};
pub use feed::{Opened, PageAnswer};
pub use filtered::{
    FilteredBar, FilteredBarAction, FilteredLine, FilteredSplice, FilteredTab, FilteredView,
};
pub use perform::{perform, perform_now};
pub use pickers::{
    Anchor, FoldersRead, LabelsRead, PickerField, PickerKind, PickerRow, PickerView,
};
pub use postio_ui::capture::{Mode as CaptureMode, Pick as CapturePick};
pub use postio_ui::digest::{Page as DigestPage, Schedule as RuleSchedule};
pub use postio_ui::search_view::FilePreview;
pub use results::{
    Chip, DatePresetView, FileCard, FileCopy, FilePurpose, FilesHeader, FilterButton, LabelPill,
    MatchCard, MonthBar, NoResultsView, PersonRow, PopoverRow, PopoverView, QueryView,
    QuickLookView, RelaxationView, ResultGroup, ResultRow, ResultSplice, ResultsTabView,
    ResultsView, SaveView, TermEdit, file_copies,
};
pub use states::{AccountsRead, BannerButton, BannerView};
pub use surfaces::{Host, ReaderVerb, Route, SurfaceKind};
pub use thread_menu::OutlineInput;
pub use verbs::{Aimed, Everything, ToastKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Policy {
    pub platform: Platform,
    pub caps: Capabilities,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capabilities {
    pub reading_pane: bool,
    pub stacking: bool,
    pub contacts_view: bool,
}

impl Policy {
    pub const fn for_platform(platform: Platform) -> Self {
        let linux = matches!(platform, Platform::Freedesktop);
        Policy {
            platform,
            caps: Capabilities {
                reading_pane: linux,
                stacking: linux,
                contacts_view: true,
            },
        }
    }
}

/// A reply carrying an older generation than the controller's changes nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Ticket {
    pub id: u64,
    pub generation: u64,
}

#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Input {
    Command(CommandId),
    Reply(Ticket, Reply),
    Event(Event),
    Accounts(Vec<AccountId>),
    /// Where the list goes back to after an undo brings rows in above.
    AtTop(bool),
    Point(u32),
    Pick {
        position: u32,
        range: bool,
    },
    SurfaceOpened(SurfaceKind),
    SurfaceClosed(SurfaceKind),
    /// The open message's More menu and find: what Back closes first.
    ReaderState {
        more_open: bool,
        finding: bool,
    },
    /// The cursor is on another row than the open message shows: the message follows it. Nothing
    /// happens with no message open, or when it shows the cursor's row.
    Follow,
    BarRoom(u32),
    /// The message window opened from a search has no more find matches this way: the controller
    /// goes on to the next matched message, or says it is the last.
    MatchEdge {
        forward: bool,
    },
    ThreadMenuPick(CommandId),
    ForwardFormClicked(postio_model::ForwardForm),
    ComposerDraft(DraftEdit, Box<postio_model::Draft>),
    Outline(thread_menu::OutlineInput),
    ThreadBarChipClicked(u32),
    ThreadBarMoreClicked {
        from: u32,
    },
    Typed {
        text: String,
    },
    BarRun(u64),
    BarTab,
    SearchHighlighted(u64),
    SearchForget(u64),
    SearchExclude(u64),
    SearchEdit(TermEdit),
    SearchPopover(postio_ui::search_view::FilterKind),
    PopoverToggle {
        token: u64,
        exclude: bool,
    },
    PopoverFilter(String),
    DateWords(String),
    DatePreset(u64),
    PopoverDone {
        apply: bool,
    },
    ResultsTab(postio_search::results::ResultsTab),
    ResultsOrder(postio_search::results::ConversationOrder),
    ResultsPoint(u64),
    FileCopyDone,
    OpenPlace(u64),
    /// While it is up, changing mail has its places read again so the counts follow (#2153).
    PlacesShown(bool),
    SavedSearches(Vec<postio_ui::saved_search::SavedSearch>),
    SearchSaved(Result<SearchesSaved, String>),
    SaveSearchAs {
        name: String,
        pin: bool,
        notify: bool,
        rolling: bool,
    },
    Keymap(postio_core::Keymap),
    Filtering(bool),
    /// Unset, it is `postio_ui::clock::now()`.
    Clock(Option<chrono::DateTime<chrono::Local>>),
    PickerTyped {
        text: String,
    },
    PickerChoose(u64),
    PickerToggle(u64),
    SendQueued {
        draft: postio_model::DraftId,
        at: Option<chrono::DateTime<chrono::Utc>>,
    },
    Config(postio_config::FocusConfig),
    /// A message opened from a search hit or a digest is on no cursor row, and a verb in it is
    /// about it alone.
    Shown(Option<MessageId>),
    FilteredPoint(u32),
    FilteredPick {
        position: u32,
        range: bool,
    },
    FilteredMore,
    DigestPoint(u32),
    DigestReference(u32),
    /// A no is nothing: the question is forgotten when the next is asked.
    Confirmed(u64),
    EditRule {
        name: String,
    },
    RuleQuery {
        text: String,
    },
    RuleMatchInstead,
    RuleLikeThis,
    RuleSchedule(postio_ui::digest::Schedule),
    RuleCreate,
    CaptureTyped {
        text: String,
    },
    CaptureDue(Option<chrono::NaiveDate>),
    CaptureFilter {
        text: String,
    },
    CaptureProject(u64),
    OpenLink(String),
    ContactPopoverAt(String),
    ContactArrival(postio_model::ContactArrival),
    ContactsTab(postio_model::ContactTab),
    ContactsFilterTyped(String),
    ContactsPoint(u32),
    ContactSplit(String),
    ContactMergeKeep(u32),
    ContactEditChange(contact_edit::EditChange),
    ContactCardSwitch(u32),
    ContactsSettingsOpened,
    ContactsSuggestion(u32),
    ContactsImported {
        made: Vec<postio_model::ContactId>,
        touched: Vec<postio_model::ContactId>,
    },
    ComposerEdited,
    /// `Ok(false)`: there was nothing in it worth keeping.
    DraftSaved {
        composition: u64,
        saved: Result<bool, String>,
    },
    Timer(u64),
    Verb(Command),
    Post(Command),
    ArchiveDigest(postio_model::DeliveryId),
}

#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Effect {
    Show(Intent),
    Ask(Ticket, Request),
    Timer { token: u64, after: Duration },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveSearch {
    pub query: String,
    pub name: Option<String>,
    pub pin: bool,
    pub notify: bool,
    pub dates: postio_ui::saved_search::Dates,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchesSaved {
    pub searches: Vec<postio_ui::saved_search::SavedSearch>,
    pub key: Option<String>,
}

/// Rewrites `[saved_searches]` alone, the rest of the file as it was. `None` is a window with no
/// file to write to.
pub fn save_search(
    path: Option<&std::path::Path>,
    save: &SaveSearch,
) -> Result<SearchesSaved, String> {
    let Some(path) = path else {
        return Err(postio_ui::focus_target::NO_CONFIG_TO_SAVE.to_owned());
    };
    let verb = postio_ui::saved_search::Verb::Save {
        query: &save.query,
        name: save.name.as_deref(),
        pin: save.pin,
        notify: save.notify,
        dates: save.dates,
    };
    postio_ui::saved_search::apply(path, verb)
        .and_then(|edit| {
            postio_config::Config::load_from_path(path).map(|config| SearchesSaved {
                searches: postio_ui::saved_search::pinned(&config),
                key: edit.changed,
            })
        })
        .map_err(|error| {
            tracing::warn!(%error, "Focus could not save the search");
            postio_ui::focus_target::SEARCH_NOT_WRITTEN.to_owned()
        })
}

#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Intent {
    Counts(FocusCounts),
    /// `anchor` is a row position, not a display position. Existing children are replaced. No
    /// animation under reduced motion.
    Splice {
        anchor: u32,
        children: Vec<postio_ui::thread_list::ChildRowView>,
    },
    Footer(postio_ui::scope::FooterView),
    Unsplice {
        anchor: u32,
    },
    Cursor {
        position: u32,
        to_top: bool,
    },
    Selection {
        selection: Selection,
        summary: Option<String>,
    },
    SingleHeading(Option<String>),
    ListToTop,
    Toast {
        text: String,
        kind: ToastKind,
    },
    ReplaceSource {
        total: u32,
    },
    PagePending {
        stamp: u64,
        page: u32,
    },
    DeliverPage {
        stamp: u64,
        page: u32,
        total: u32,
        rows: Vec<FocusRow>,
    },
    AbandonPage {
        stamp: u64,
        page: u32,
    },
    GiveUp {
        stamp: u64,
        page: u32,
    },
    RefreshList,
    Filled,
    OpenMessage {
        message: MessageId,
        index: u32,
        total: u32,
        host: Host,
    },
    OpenDigest {
        row: MessageId,
    },
    Composer {
        kind: ComposerKind,
        message: Option<MessageId>,
    },
    SaveDraft {
        composition: u64,
    },
    CloseSurface(SurfaceKind),
    Reader(ReaderVerb),
    /// Said on every message the window shows, with no chips for a thread of one.
    ThreadBar(postio_ui::thread_bar::ThreadBarView),
    Kicker(String),
    OlderBanner(Option<postio_ui::thread_bar::BannerView>),
    /// A reply arrived while the window was on another message, which stays on screen.
    NewReply(Option<postio_ui::thread_bar::BannerView>),
    ReplyTo(Option<String>),
    KeyboardHome,
    Quit,
    /// Said again while it is up, it is only new words for the field.
    OpenBar {
        mode: BarMode,
        text: String,
        select: Option<(u32, u32)>,
    },
    BarLines(BarView),
    Dropdown(DropdownView),
    ShowResults(Box<ResultsView>),
    ResultsPage {
        first: u64,
        count: u64,
    },
    ResultsCursor(u64),
    ResultSplice(Option<Box<ResultSplice>>),
    ResultChildFocus(Option<u32>),
    MatchBanner(Option<postio_ui::thread_bar::MatchBannerView>),
    Query(QueryView),
    Popover(Option<Box<PopoverView>>),
    QuickLook(Option<Box<QuickLookView>>),
    Relaxations(Option<Box<NoResultsView>>),
    /// `None` closes the Quick Look panel. The frontend says `Input::FileCopyDone` when the panel
    /// is gone.
    FileCopy(Option<FileCopy>),
    LeaveResults,
    Place {
        name: String,
    },
    OpenPlaces,
    PlacesChanged,
    ShowFiltered,
    SaveSearch(SaveSearch),
    SavePopover(Option<Box<SaveView>>),
    Run(CommandId),
    OpenPicker(PickerView),
    PickerRows(PickerView),
    PickerField,
    /// The remind picker opened for the draft chose this time; nothing was sent.
    DraftReminder {
        at: chrono::DateTime<chrono::Utc>,
    },
    Banner(Option<BannerView>),
    SyncLabel(postio_ui::focus_state::SyncLabel),
    Empty(Option<postio_ui::focus_state::EmptyInbox>),
    /// Closes with `Intent::CloseSurface(KeyMap)`, which takes it off again (no
    /// `Input::SurfaceClosed` follows).
    OpenKeyMap,
    Filtered(Box<FilteredView>),
    FilteredFocus(Option<u32>),
    FilteredSplice {
        anchor: u32,
        children: Vec<postio_ui::thread_list::ChildRowView>,
    },
    FilteredUnsplice {
        anchor: u32,
    },
    FilteredSelection {
        selected: Vec<MessageId>,
        bar: Option<FilteredBar>,
    },
    Digest(Box<DigestView>),
    Confirm(Confirm),
    OpenRule(Box<RuleView>),
    Rule(Box<RuleView>),
    OpenCapture(Box<CaptureView>),
    Capture(Box<CaptureView>),
    ContactPopover(Box<postio_ui::contact_popover::PopoverView>),
    ThreadMenu(Box<postio_ui::thread_outline::ThreadMenuView>),
    Outline(Box<postio_ui::thread_outline::OutlineView>),
    OutlineFocusFilter,
    WriteTo(String),
    Contacts(Box<contacts::ContactsView>),
    ContactsRows {
        first: u32,
        count: u32,
    },
    ContactsCursor(Option<u32>),
    ContactsCard(Option<Box<postio_ui::contacts_view::CardView>>),
    ContactMerge(Option<Box<postio_ui::contact_merge::MergeSheetView>>),
    ContactEdit(Option<Box<postio_ui::contact_edit::EditView>>),
    ContactsSettings(Box<postio_ui::contacts_settings::SuggestionsView>),
    ContactsFocusFilter,
    ForwardChooser(Box<postio_ui::compose::ChooserView>),
    OlderReply(Option<Box<postio_ui::compose::OlderReplyView>>),
    NewerMessages(Option<Vec<MessageId>>),
    QuotedExpanded(bool),
    ComposerDraftWanted(DraftEdit),
    ComposerDraft(Box<postio_model::Draft>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum DraftEdit {
    ForwardForm(postio_model::ForwardForm),
    ReplyToLatest,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Request {
    FocusCounts,
    OpenScope {
        scope: ListScope,
        splices: bool,
    },
    Page {
        page: u32,
        stamp: u64,
        start: u32,
        count: u32,
        wanted: PageRequest,
    },
    Surfaced,
    Send {
        command: Command,
        aims: Vec<MessageTarget>,
        everything: Option<Everything>,
    },
    Post(Command),
    /// Aimed at a view, not mail: which mailboxes to sync is the host's to work out, and with none
    /// the host has nothing to refresh.
    Refresh {
        viewing: Option<postio_core::state::ViewScope>,
    },
    NoteRemoved {
        mailbox: MailboxId,
        messages: Vec<MessageId>,
    },
    Places,
    Conversations {
        query: postio_search::ParsedQuery,
        order: postio_search::results::ConversationOrder,
        limit: u32,
        stamp: u64,
    },
    Passages {
        query: postio_search::ParsedQuery,
        hits: Vec<(MessageId, Vec<postio_search::results::Source>)>,
        stamp: u64,
    },
    ResultsPage {
        query: postio_search::ParsedQuery,
        order: postio_search::results::ConversationOrder,
        offset: u32,
        limit: u32,
        stamp: u64,
    },
    ResultsPassages {
        query: postio_search::ParsedQuery,
        hits: Vec<(MessageId, Vec<postio_search::results::Source>)>,
        stamp: u64,
    },
    Files {
        query: postio_search::ParsedQuery,
        stamp: u64,
    },
    People {
        query: postio_search::ParsedQuery,
        stamp: u64,
    },
    AttachmentCopy {
        attachment: postio_model::AttachmentId,
        purpose: FilePurpose,
        stamp: u64,
    },
    RemoveCopy(std::path::PathBuf),
    QuickLookMatches {
        query: postio_search::ParsedQuery,
        key: postio_search::results::ConversationKey,
        stamp: u64,
    },
    Relaxations {
        query: postio_search::ParsedQuery,
        today: chrono::NaiveDate,
        stamp: u64,
    },
    Facets {
        query: postio_search::ParsedQuery,
        stamp: u64,
    },
    Suggest {
        prefix: String,
        field: Option<postio_search::query::Field>,
        stamp: u64,
    },
    LatestFrom {
        address: String,
        stamp: u64,
    },
    ReadThread(ThreadId),
    SetForwardForm {
        draft: Box<postio_model::Draft>,
        form: postio_model::ForwardForm,
    },
    ForwardThread {
        thread: ThreadId,
        form: postio_model::ForwardForm,
    },
    ReplyToLatest {
        draft: Box<postio_model::Draft>,
    },
    RecentSearches,
    RememberSearch {
        query: String,
        /// The search stopped at its walk's cap: `hits` is a floor.
        hits: u64,
        capped: bool,
    },
    ForgetSearch {
        query: String,
    },
    MarkSeen {
        key: String,
    },
    SavedCounts {
        searches: Vec<(String, String)>,
        today: chrono::NaiveDate,
    },
    RoleFolder(MailboxRole),
    Labels {
        message: Option<MessageId>,
        account: Option<AccountId>,
        threads: Vec<postio_model::ThreadId>,
        stamp: u64,
    },
    CreateLabel {
        account: AccountId,
        name: String,
        stamp: u64,
    },
    Folders {
        stamp: u64,
    },
    NoteMove(MailboxId),
    Accounts,
    FilteredTabs,
    Filtered {
        reason: Option<String>,
        offset: u32,
        stamp: u64,
    },
    SweepPreview,
    DigestRead {
        delivery: postio_model::DeliveryId,
        summary: bool,
        stamp: u64,
    },
    Unsubscribe(MessageId),
    DigestPreview {
        queries: Vec<String>,
        since: chrono::DateTime<chrono::Utc>,
        stamp: u64,
    },
    DigestLikeThis {
        message: MessageId,
        stamp: u64,
    },
    SaveDigestRule {
        replacing: Option<String>,
        draft: postio_client::protocol::DigestRuleDraft,
        stamp: u64,
    },
    Vault {
        subject: String,
        stamp: u64,
    },
    CaptureTask {
        /// Its project; `None` for the tasks note.
        project: Option<postio_vault::Project>,
        task: postio_vault::Task,
        stamp: u64,
    },
    CaptureNote {
        note: std::path::PathBuf,
        entry: postio_vault::NoteEntry,
        stamp: u64,
    },
    ContactsOverview,
    ContactsPage {
        tab: postio_model::ContactTab,
        after: Option<(String, postio_model::ContactId)>,
        skip: u32,
        limit: u32,
        stamp: u64,
        page: u32,
    },
    ContactsFiltered {
        tab: postio_model::ContactTab,
        text: String,
        cap: u32,
        stamp: u64,
    },
    ContactReview {
        first: Vec<postio_model::ContactId>,
    },
    ContactPosition {
        person: postio_model::ContactId,
        tab: postio_model::ContactTab,
    },
    ContactMergeOf(Vec<postio_model::ContactId>),
    ContactGroups,
    ContactSeen(postio_model::ContactId),
    ContactSuggestions,
    ContactSuggestionSet {
        which: postio_model::Suggestion,
        on: bool,
    },
    ContactCard {
        person: postio_model::ContactId,
        stamp: u64,
    },
    ContactGlance {
        address: String,
        stamp: u64,
    },
    FindMessage(MessageId),
    ThreadOf(MessageId),
    DraftBehind {
        message: MessageId,
        command: CommandId,
    },
}

impl Request {
    pub fn first_line(&self) -> Option<postio_search::passage::FirstLine> {
        match self {
            // Neither the dropdown's hits nor the results' rows show a preview: a match after the
            // first line, or the first line's own when it holds the only one.
            Request::Passages { .. } | Request::ResultsPassages { .. } => {
                Some(postio_search::passage::FirstLine::Avoided)
            }
            _ => None,
        }
    }

    /// A newer request of the same lane makes the one before it waste.
    pub fn lane(&self) -> Option<Lane> {
        match self {
            Request::Conversations { .. } => Some(Lane::Conversations),
            Request::Passages { .. } => Some(Lane::Passages),
            Request::QuickLookMatches { .. } => Some(Lane::Matches),
            Request::Files { .. } => Some(Lane::Files),
            Request::People { .. } => Some(Lane::People),
            Request::Relaxations { .. } => Some(Lane::Relaxations),
            Request::Facets { .. } => Some(Lane::Facets),
            Request::Suggest { .. } => Some(Lane::Suggest),
            Request::LatestFrom { .. } => Some(Lane::Latest),
            _ => None,
        }
    }
}

/// A failure is carried as its sentence: the controller decides what to show, not how to
/// recover.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Reply {
    FocusCounts(Result<FocusCounts, String>),
    Opened(Opened),
    Page {
        page: u32,
        stamp: u64,
        start: u32,
        count: u32,
        answer: Result<PageAnswer, String>,
    },
    Surfaced(Result<Vec<Surfaced>, String>),
    Noted,
    Sent(Result<(), String>),
    Places(Result<PlacesRead, String>),
    Conversations {
        stamp: u64,
        answer: Result<Box<postio_search::results::ConversationResults>, String>,
    },
    Passages {
        stamp: u64,
        answer: Result<Vec<(MessageId, Vec<postio_search::results::Match>)>, String>,
    },
    ResultsPage {
        stamp: u64,
        order: postio_search::results::ConversationOrder,
        offset: u32,
        answer: Result<Box<postio_search::results::ConversationResults>, String>,
    },
    ResultsPassages {
        stamp: u64,
        answer: Result<Vec<(MessageId, Vec<postio_search::results::Match>)>, String>,
    },
    Files {
        stamp: u64,
        answer: Result<Vec<postio_search::results::FileHit>, String>,
    },
    People {
        stamp: u64,
        answer: Result<Vec<postio_search::suggest::Person>, String>,
    },
    AttachmentCopy {
        stamp: u64,
        purpose: FilePurpose,
        answer: Result<Option<std::path::PathBuf>, String>,
    },
    QuickLookMatches {
        stamp: u64,
        answer: Result<Vec<postio_search::results::ConversationMatch>, String>,
    },
    Relaxations {
        stamp: u64,
        answer: Result<
            Vec<(
                postio_search::relax::Relaxation,
                postio_search::results::Counted,
            )>,
            String,
        >,
    },
    Facets {
        stamp: u64,
        answer: Result<Box<postio_search::results::ConversationResults>, String>,
    },
    Suggest {
        stamp: u64,
        answer: Result<Box<postio_search::suggest::Suggestions>, String>,
    },
    LatestFrom {
        stamp: u64,
        address: String,
        answer: Result<Box<postio_search::results::ConversationResults>, String>,
    },
    ThreadRead {
        thread: ThreadId,
        view: Result<Option<ThreadView>, String>,
    },
    Forwarded {
        form: postio_model::ForwardForm,
        answer: Result<postio_model::Forwarded, String>,
    },
    RepliedToLatest(Result<Box<postio_model::Draft>, String>),
    RecentSearches(Result<Vec<postio_client::protocol::RecentSearch>, String>),
    SavedCounts(Result<Vec<postio_client::protocol::SavedCount>, String>),
    RoleFolder(Option<(MailboxId, String)>),
    Labels {
        stamp: u64,
        answer: Result<LabelsRead, String>,
    },
    LabelCreated {
        stamp: u64,
        answer: Result<postio_model::Label, String>,
    },
    Folders {
        stamp: u64,
        answer: Result<FoldersRead, String>,
    },
    Accounts(Result<AccountsRead, String>),
    FilteredTabs(Result<Vec<(String, u32)>, String>),
    Filtered {
        stamp: u64,
        offset: u32,
        answer: Result<Vec<postio_client::protocol::FilteredRow>, String>,
    },
    SweepPreview(Result<u32, String>),
    DigestRead {
        stamp: u64,
        rows: Result<Vec<postio_model::listing::MessageSummary>, String>,
        summary: Option<postio_model::summary::DigestSummary>,
    },
    Unsubscribed(Result<String, String>),
    DigestPreview {
        stamp: u64,
        answer: Result<postio_client::protocol::DigestPreview, String>,
    },
    LikeThis {
        stamp: u64,
        answer: Result<Option<Vec<String>>, String>,
    },
    RuleSaved {
        stamp: u64,
        answer: Result<(), String>,
    },
    Vault {
        stamp: u64,
        answer: Result<postio_client::protocol::VaultPicture, String>,
    },
    Captured {
        stamp: u64,
        answer: Result<(), String>,
    },
    ContactsOverview(Result<postio_model::ContactsOverview, String>),
    ContactsPage {
        stamp: u64,
        page: u32,
        answer: Result<postio_model::ContactsPageRead, String>,
    },
    ContactsFiltered {
        stamp: u64,
        answer: Result<Vec<postio_model::ContactListRow>, String>,
    },
    ContactPosition {
        person: postio_model::ContactId,
        at: Option<u32>,
    },
    ContactReview(Vec<postio_model::MergeGroup>),
    ContactMergeOf(Option<Box<postio_model::MergeGroup>>),
    ContactGroups(Result<Vec<String>, String>),
    ContactSeen(Vec<postio_model::SeenItem>),
    ContactSuggestions(Result<postio_model::ContactSuggestions, String>),
    ContactCard {
        stamp: u64,
        answer: Result<Option<Box<postio_model::ContactCardRead>>, String>,
    },
    /// The person, or `None` for an address the store never saw.
    ContactGlance {
        stamp: u64,
        glance: Result<Option<Box<postio_model::ContactGlance>>, String>,
    },
    FoundMessage {
        message: MessageId,
        row: Option<postio_model::listing::MessageSummary>,
    },
    ThreadOf {
        message: MessageId,
        thread: Option<ThreadId>,
    },
    DraftBehind {
        message: MessageId,
        command: CommandId,
        draft: Option<postio_model::DraftId>,
    },
}

/// `Send`, and plain data: no toolkit object, no executor, no client.
#[derive(Debug)]
pub struct FocusController {
    policy: Policy,
    generation: u64,
    next_ticket: u64,
    feed: feed::Feed,
    cursor: cursor::Cursor,
    verbs: verbs::Verbs,
    surfaces: surfaces::Surfaces,
    bar: bar::Bar,
    pickers: pickers::Pickers,
    /// A queued send's own Undo, which Undo runs before the stack's, until a newer toast replaces
    /// it.
    toast_undo: Option<postio_model::DraftId>,
    shown: Option<MessageId>,
    counts: Option<FocusCounts>,
    states: states::States,
    keys: keys::Keys,
    filtered: Option<filtered::Filtered>,
    digest: Option<digest::DigestWindow>,
    rule: Option<digest::RuleDialog>,
    capture: Option<capture::Capture>,
    contact_popover: Option<contact_popover::ContactPopover>,
    contacts: Option<contacts::Contacts>,
    contacts_settings: Option<contacts_settings::Settings>,
    import_found: Vec<postio_model::ContactId>,
    import_cursor: Option<postio_model::ContactId>,
    confirm: Option<confirm::Asked>,
    compose: compose::Compose,
    stamps: u64,
    results: Option<results::Results>,
    history: history::History,
    expansion: Option<thread_expand::Expansion>,
    pending: Option<thread_expand::Pending>,
    /// The row is not opened again for showing a message other than its own.
    opened_from: Option<(MessageId, MessageId)>,
    walk: Option<walk::Walk>,
    footer: footer::Footer,
    walk_asked: Option<walk::Opening>,
    walk_seed: Option<ThreadView>,
    /// Something must take the thread bar away when no walk is left to own it.
    bar_up: bool,
    bar_room: u32,
    thread_ui: thread_menu::ThreadUi,
}

impl FocusController {
    pub fn new(policy: Policy) -> Self {
        FocusController {
            policy,
            generation: 0,
            next_ticket: 0,
            feed: feed::Feed::default(),
            cursor: cursor::Cursor::default(),
            verbs: verbs::Verbs::default(),
            surfaces: surfaces::Surfaces::default(),
            bar: bar::Bar::new(policy.platform),
            pickers: pickers::Pickers::default(),
            toast_undo: None,
            shown: None,
            counts: None,
            states: states::States::default(),
            keys: keys::Keys::default(),
            filtered: None,
            digest: None,
            rule: None,
            capture: None,
            contact_popover: None,
            contacts: None,
            contacts_settings: None,
            import_found: Vec::new(),
            import_cursor: None,
            confirm: None,
            compose: compose::Compose::default(),
            stamps: 0,
            results: None,
            history: history::History::default(),
            expansion: None,
            pending: None,
            opened_from: None,
            walk: None,
            footer: footer::Footer::default(),
            walk_asked: None,
            walk_seed: None,
            bar_up: false,
            bar_room: walk::DEFAULT_ROOM,
            thread_ui: thread_menu::ThreadUi::default(),
        }
    }

    pub fn policy(&self) -> Policy {
        self.policy
    }

    pub fn handle(&mut self, input: Input) -> Vec<Effect> {
        self.handle_on(input, &NoRows)
    }

    pub fn handle_on(&mut self, input: Input, rows: &dyn Rows) -> Vec<Effect> {
        let reading = self.surfaces.has(SurfaceKind::Message);
        let raw = rows;
        let through = self.through(raw);
        let rows: &dyn Rows = through.as_ref().map_or(raw, |through| through);
        let mut stepped = self.filtered_thread_input(&input);
        if (stepped.is_none() || Self::also_for_the_rest(&input))
            && let Some(steps) = self.thread_input(&input, raw, rows)
        {
            stepped.get_or_insert_with(Vec::new).extend(steps);
        }
        let mut effects = match stepped {
            Some(steps) => {
                let mut effects = self.effects(steps);
                if Self::also_for_the_rest(&input) {
                    effects.extend(self.handle_input(input, rows));
                }
                effects
            }
            None => self.handle_input(input, rows),
        };
        if reading && !self.surfaces.has(SurfaceKind::Message) {
            let steps = self.hit_closed(rows);
            effects.extend(self.effects(steps));
        }
        let steps = self.walk_after(&effects, rows);
        effects.extend(self.effects(steps));
        let steps = self.composer_gone();
        effects.extend(self.effects(steps));
        let footer = self.footer_after(&effects, raw).or_else(|| {
            let respliced = effects.iter().any(|effect| {
                matches!(
                    effect,
                    Effect::Show(Intent::Splice { .. } | Intent::Unsplice { .. })
                )
            });
            let position = self
                .cursor
                .position()
                .filter(|_| respliced && self.walk.is_none() && self.filtered.is_none())?;
            thread_expand::footer_at(self.expansion.as_ref(), position, raw, self.keymap())
        });
        if let Some(footer) = footer {
            effects.push(Effect::Show(Intent::Footer(footer)));
        }
        self.settle_footer(&mut effects, raw);
        effects
    }

    fn also_for_the_rest(input: &Input) -> bool {
        matches!(
            input,
            Input::Event(_) | Input::Reply(_, Reply::ThreadOf { .. })
        )
    }

    fn handle_input(&mut self, input: Input, rows: &dyn Rows) -> Vec<Effect> {
        if let Input::Command(id) = &input
            && let Some(steps) = self.picker_command(*id)
        {
            return self.effects(steps);
        }
        if let Input::Command(id) = &input
            && self.surfaces.top().is_none()
            && let Some(steps) = self.results_command(*id)
        {
            return self.effects(steps);
        }
        match input {
            Input::Command(CommandId::Quit) => vec![Effect::Show(Intent::Quit)],
            Input::Command(id @ (CommandId::HistoryBack | CommandId::HistoryForward))
                if self.surfaces.top().is_none() =>
            {
                let steps = self.history_command(id).unwrap_or_default();
                self.effects(steps)
            }
            Input::Command(id) if self.surfaces.top().is_some() => {
                match self.surface_command(id, rows) {
                    Some(steps) => self.effects(steps),
                    None => self.list_command(id, rows),
                }
            }
            // Back's last rung: the surfaces above the list are still the
            // frontend's to close first.
            Input::Command(CommandId::Back) => {
                let steps = self.cursor.clear(self.feed.total());
                self.effects(steps)
            }
            Input::Command(id) => self.list_command(id, rows),
            Input::SurfaceOpened(kind) => {
                let mut steps = Vec::new();
                if kind == SurfaceKind::Composer {
                    steps.extend(self.composer_opened());
                }
                steps.extend(self.surfaces.opened(kind, self.policy.caps.stacking));
                self.forget_closed();
                self.effects(steps)
            }
            Input::Shown(message) => {
                self.shown = message;
                Vec::new()
            }
            Input::SurfaceClosed(kind) => {
                if kind == SurfaceKind::Message {
                    self.shown = None;
                }
                if kind == SurfaceKind::Bar {
                    self.bar.close();
                }
                let mut steps = Vec::new();
                if kind == SurfaceKind::Picker {
                    steps.extend(self.picker_gone(false));
                }
                steps.extend(self.surfaces.closed(kind));
                self.forget_closed();
                self.effects(steps)
            }
            Input::ReaderState { more_open, finding } => {
                self.surfaces.reader_state(more_open, finding);
                Vec::new()
            }
            Input::BarRoom(room) => {
                let steps = self.bar_room(room);
                self.effects(steps)
            }
            Input::MatchEdge { forward } => {
                let steps = self.match_edge(forward);
                self.effects(steps)
            }
            Input::Accounts(accounts) => {
                self.compose.set_no_account(accounts.is_empty());
                self.cursor.set_accounts(accounts);
                Vec::new()
            }
            Input::Verb(command) => {
                let total = self.feed.total();
                let steps = self.verbs.send(command, &mut self.cursor, rows, total);
                self.effects(steps)
            }
            Input::Post(command) => {
                let steps = self.verbs.post(command);
                self.effects(steps)
            }
            Input::ArchiveDigest(delivery) => {
                let steps = self.verbs.archive_digest(delivery);
                self.effects(steps)
            }
            Input::AtTop(at_top) => {
                self.verbs.set_at_top(at_top);
                Vec::new()
            }
            Input::Point(position) => {
                let steps = self.cursor.point(position, rows);
                self.effects(steps)
            }
            Input::Follow => {
                let steps = self.follow_cursor(rows);
                self.effects(steps)
            }
            Input::Pick { position, range } => {
                let total = self.feed.total();
                let steps = self.cursor.pick(position, range, rows, total);
                self.effects(steps)
            }
            // Who the accounts are is true whatever was invalidated since:
            // the list's generation is no stamp on it.
            Input::Reply(_, Reply::Accounts(read)) => {
                let steps = self.accounts_read(read);
                self.effects(steps)
            }
            // The surfaces' answers carry their own stamps: a list replaced
            // under them does not make them stale.
            Input::Reply(
                _,
                reply @ (Reply::FilteredTabs(_)
                | Reply::Filtered { .. }
                | Reply::SweepPreview(_)
                | Reply::DigestRead { .. }
                | Reply::Unsubscribed(_)
                | Reply::DigestPreview { .. }
                | Reply::LikeThis { .. }
                | Reply::RuleSaved { .. }
                | Reply::Vault { .. }
                | Reply::Captured { .. }
                | Reply::ContactGlance { .. }
                | Reply::ContactsOverview(_)
                | Reply::ContactsPage { .. }
                | Reply::ContactsFiltered { .. }
                | Reply::ContactCard { .. }
                | Reply::ContactGroups(_)
                | Reply::ContactReview(_)
                | Reply::ContactPosition { .. }
                | Reply::ContactMergeOf(_)
                | Reply::ContactSeen(_)
                | Reply::ContactSuggestions(_)
                | Reply::FoundMessage { .. }
                | Reply::ThreadOf { .. }
                | Reply::DraftBehind { .. }
                | Reply::Forwarded { .. }
                | Reply::RepliedToLatest(_)),
            ) => {
                let steps = self.surface_reply(reply, rows);
                self.effects(steps)
            }
            Input::Reply(
                _,
                reply @ (Reply::ResultsPage { .. }
                | Reply::ResultsPassages { .. }
                | Reply::Files { .. }
                | Reply::People { .. }
                | Reply::AttachmentCopy { .. }
                | Reply::QuickLookMatches { .. }
                | Reply::Facets { .. }
                | Reply::Relaxations { .. }),
            ) => {
                let steps = self.results_reply(reply);
                self.effects(steps)
            }
            Input::Reply(ticket, _) if ticket.generation != self.generation => Vec::new(),
            Input::Reply(_, Reply::FocusCounts(Ok(counts))) => {
                let before = self.counts.replace(counts);
                let mut steps = vec![feed::Step::Show(Intent::Counts(counts))];
                steps.extend(self.bar.filtered_counted(
                    before.map(|before| before.filtered_today),
                    counts.filtered_today,
                ));
                steps.extend(self.show_empty());
                self.effects(steps)
            }
            Input::Reply(_, Reply::FocusCounts(Err(error))) => {
                tracing::debug!(%error, "focus counts unavailable");
                Vec::new()
            }
            Input::Reply(_, Reply::Opened(opened)) => {
                self.compose.set_no_account(opened.accounts.is_empty());
                self.cursor.set_accounts(opened.accounts.clone());
                let steps = self.feed.opened(opened);
                self.effects(steps)
            }
            Input::Reply(
                _,
                Reply::Page {
                    page,
                    stamp,
                    start,
                    count,
                    answer,
                },
            ) => {
                let steps = self.feed.page(page, stamp, start, count, answer);
                self.effects(steps)
            }
            Input::Reply(_, Reply::Surfaced(surfaced)) => {
                let steps = self.feed.resurfaced(surfaced);
                self.rereading(&steps, rows);
                self.effects(steps)
            }
            Input::Reply(_, Reply::ThreadRead { .. }) => Vec::new(),
            Input::Reply(_, Reply::Noted) => Vec::new(),
            Input::Reply(_, Reply::Sent(result)) => {
                if let Err(error) = result {
                    tracing::warn!(%error, "Focus could not send a command");
                }
                Vec::new()
            }
            Input::Reply(
                _,
                reply @ (Reply::Places(_)
                | Reply::Conversations { .. }
                | Reply::Passages { .. }
                | Reply::Suggest { .. }
                | Reply::LatestFrom { .. }
                | Reply::RecentSearches(_)
                | Reply::SavedCounts(_)
                | Reply::RoleFolder(_)),
            ) => {
                let steps = self.bar_reply(reply, rows);
                self.effects(steps)
            }
            Input::Event(event) => {
                let mut steps = self.verbs.event(&event);
                steps.extend(self.results_heard(&event));
                steps.extend(self.bar.heard(&event));
                steps.extend(self.bar.places_heard(&event));
                steps.extend(self.feed.event(&event));
                steps.extend(self.walk_heard(&event));
                steps.extend(self.hear_sync(&event));
                steps.extend(self.filtered_event(&event));
                steps.extend(self.contacts_heard(&event));
                steps.extend(self.contact_popover_heard(&event));
                self.rereading(&steps, rows);
                self.effects(steps)
            }
            Input::Config(config) => {
                let mut steps = self.bar_input(Input::Filtering(config.filtering), rows);
                steps.extend(self.set_config(config));
                self.effects(steps)
            }
            Input::Clock(clock) => {
                self.pickers.clock = clock;
                self.bar.set_clock(clock);
                Vec::new()
            }
            Input::PickerTyped { text } => {
                let steps = self.picker_typed(text);
                self.effects(steps)
            }
            Input::PickerChoose(token) => {
                let steps = self.picker_choose(token);
                self.effects(steps)
            }
            Input::PickerToggle(token) => {
                let steps = self.picker_toggle(token);
                self.effects(steps)
            }
            Input::SendQueued { draft, at } => {
                let text = match at {
                    Some(_) => postio_ui::sending::SEND_SCHEDULED,
                    None => postio_ui::sending::QUEUED_TO_SEND,
                };
                let effects = self.effects(vec![feed::Step::Show(Intent::Toast {
                    text: text.to_owned(),
                    kind: ToastKind::Completed {
                        undoable: true,
                        seconds: None,
                    },
                })]);
                // After its own toast, which would otherwise replace it.
                self.toast_undo = Some(draft);
                effects
            }
            Input::Reply(
                _,
                reply @ (Reply::Labels { .. } | Reply::LabelCreated { .. } | Reply::Folders { .. }),
            ) => {
                let steps = self.picker_reply(reply);
                self.effects(steps)
            }
            Input::Keymap(keymap) => {
                let steps = self.set_keymap(keymap);
                self.effects(steps)
            }
            Input::FilteredPoint(index) => {
                let steps = self.filtered_point(index);
                self.effects(steps)
            }
            Input::FilteredPick { position, range } => {
                let steps = self.filtered_pick(position, range);
                self.effects(steps)
            }
            Input::FilteredMore => {
                let steps = self.filtered_more();
                self.effects(steps)
            }
            Input::DigestPoint(index) => {
                let steps = self.digest_point(index);
                self.effects(steps)
            }
            Input::DigestReference(index) => {
                let steps = self.digest_reference(index);
                self.effects(steps)
            }
            Input::Confirmed(token) => {
                let steps = self.confirmed(token);
                self.effects(steps)
            }
            Input::EditRule { name } => {
                let steps = self.edit_rule(&name);
                self.effects(steps)
            }
            input @ (Input::RuleQuery { .. }
            | Input::RuleMatchInstead
            | Input::RuleLikeThis
            | Input::RuleSchedule(_)
            | Input::RuleCreate) => {
                let steps = self.rule_input(input);
                self.effects(steps)
            }
            input @ (Input::CaptureTyped { .. }
            | Input::CaptureDue(_)
            | Input::CaptureFilter { .. }
            | Input::CaptureProject(_)) => {
                let steps = self.capture_input(input);
                self.effects(steps)
            }
            Input::OpenLink(uri) => {
                let steps = self.open_link(&uri);
                self.effects(steps)
            }
            Input::ThreadMenuPick(id) => {
                let steps = self.thread_menu_pick(id, rows);
                self.effects(steps)
            }
            Input::Outline(input) => {
                let steps = self.outline_input(input, rows);
                self.effects(steps)
            }
            Input::ThreadBarChipClicked(index) => {
                let steps = self.chip_clicked(index);
                self.effects(steps)
            }
            Input::ThreadBarMoreClicked { from } => {
                let steps = self.more_chip_clicked(from);
                self.effects(steps)
            }
            Input::ForwardFormClicked(form) => {
                let steps = self.forward_form_chosen(form);
                self.effects(steps)
            }
            Input::ComposerDraft(edit, draft) => {
                let steps = self.composer_draft(edit, draft);
                self.effects(steps)
            }
            Input::ContactPopoverAt(address) => {
                let steps = self.open_contact_popover(Some(address), rows);
                self.effects(steps)
            }
            Input::ContactsTab(tab) => {
                let steps = self.contacts_tab(tab);
                self.effects(steps)
            }
            Input::ContactsFilterTyped(text) => {
                let steps = self.contacts_filter_typed(text);
                self.effects(steps)
            }
            Input::ContactsPoint(index) => {
                let steps = self.contacts_point(index);
                self.effects(steps)
            }
            Input::ContactEditChange(change) => {
                let steps = self.contact_edit_changed(change);
                self.effects(steps)
            }
            Input::ContactSplit(address) => {
                let steps = self.contact_split(&address);
                self.effects(steps)
            }
            Input::ContactMergeKeep(index) => {
                let steps = self.contact_merge_keep(index as usize);
                self.effects(steps)
            }
            Input::ContactsImported { made, touched } => {
                let steps = self.contacts_imported(made, touched);
                self.effects(steps)
            }
            Input::ContactCardSwitch(index) => {
                let steps = self.contact_card_switch(index);
                self.effects(steps)
            }
            Input::ContactsSettingsOpened => {
                let steps = self.contacts_settings_opened();
                self.effects(steps)
            }
            Input::ContactsSuggestion(index) => {
                let steps = self.contacts_suggestion_clicked(index);
                self.effects(steps)
            }
            Input::ContactArrival(arrival) => {
                let steps = self.contact_arrival_picked(arrival);
                self.effects(steps)
            }
            Input::ComposerEdited => {
                let steps = self.composer_edited();
                self.effects(steps)
            }
            Input::DraftSaved { composition, saved } => {
                let steps = self.draft_saved(composition, saved);
                self.effects(steps)
            }
            Input::Timer(token) if token == feed::THREAD_REFRESH_TIMER => {
                let steps = self.feed.thread_refresh_due();
                self.rereading(&steps, rows);
                self.effects(steps)
            }
            Input::Timer(token) if surfaces::is_reader_timer(token) => {
                self.surfaces.timed_out(token);
                Vec::new()
            }
            Input::Timer(token) => {
                let steps = self.timer(token);
                self.effects(steps)
            }
            input @ (Input::SearchEdit(_)
            | Input::SearchPopover(_)
            | Input::PopoverToggle { .. }
            | Input::PopoverFilter(_)
            | Input::PopoverDone { .. }
            | Input::DateWords(_)
            | Input::DatePreset(_)
            | Input::ResultsTab(_)
            | Input::ResultsOrder(_)
            | Input::ResultsPoint(_)
            | Input::FileCopyDone
            | Input::SaveSearchAs { .. }) => {
                let steps = self.results_input(input);
                self.effects(steps)
            }
            input @ (Input::Typed { .. }
            | Input::BarRun(_)
            | Input::BarTab
            | Input::SearchHighlighted(_)
            | Input::SearchForget(_)
            | Input::SearchExclude(_)
            | Input::OpenPlace(_)
            | Input::PlacesShown(_)
            | Input::SavedSearches(_)
            | Input::SearchSaved(_)
            | Input::Filtering(_)) => {
                let steps = self.bar_input(input, rows);
                self.effects(steps)
            }
        }
    }

    pub fn open_draft_reminder(&mut self, target: &str) -> Vec<Effect> {
        let steps = self.picker_for_draft(target.to_owned());
        self.effects(steps)
    }

    pub fn places(&self, filter: &str) -> Vec<(u64, postio_ui::places::Entry)> {
        self.listed_places(filter)
    }

    pub fn landed(&mut self, rows: &dyn Rows, opened: bool) -> Vec<Effect> {
        let raw = rows;
        let mut steps = self.expansion_followed(rows);
        let through = self.through(rows);
        let rows: &dyn Rows = through.as_ref().map_or(rows, |through| through);
        steps.extend(self.cursor.landed(rows, opened));
        steps.extend(self.verbs.landed(&mut self.cursor, rows));
        match self.surfaces.landed(rows) {
            Some(surfaces::Landing::Open(index)) => {
                steps.extend(self.cursor.place(index, rows));
                steps.extend(self.open_at(index, rows));
            }
            Some(surfaces::Landing::Close) => {
                steps.push(feed::Step::Show(Intent::CloseSurface(SurfaceKind::Message)));
                steps.push(feed::Step::Show(Intent::KeyboardHome));
            }
            // A place just opened with nothing in it: the message open over the list came from the
            // place left, and its verbs would act on mail no longer in view (#1757). The thread's
            // walk goes with it.
            None if opened && rows.is_empty() && self.surfaces.reading().is_some() => {
                steps.extend(self.walk_close());
            }
            None => {}
        }
        steps.extend(self.show_empty());
        let mut effects = self.effects(steps);
        if self.surfaces.has(SurfaceKind::Message) {
            let steps = self.walk_after(&effects, rows);
            effects.extend(self.effects(steps));
        }
        // A landing that did not move the cursor may still be the first with its row read, so the
        // footer is said again from where it is, unless a message window owns it.
        let footer = self
            .footer_after(&effects, raw)
            .or_else(|| {
                let position = self.cursor.position().filter(|_| self.walk.is_none())?;
                thread_expand::footer_at(self.expansion.as_ref(), position, raw, self.keymap())
            })
            // No row to name: the footer of the place left behind would offer verbs on a message
            // that is not here (#2195).
            .or_else(|| {
                (rows.is_empty() && self.walk.is_none()).then(postio_ui::scope::FooterView::default)
            });
        if let Some(footer) = footer {
            effects.push(Effect::Show(Intent::Footer(footer)));
        }
        self.settle_footer(&mut effects, raw);
        effects.push(self.refresh_counts());
        effects
    }

    pub fn has_surface(&self) -> bool {
        self.surfaces.top().is_some()
    }

    pub fn top_surface(&self) -> Option<SurfaceKind> {
        self.surfaces.top()
    }

    pub fn route(&self, id: CommandId) -> Route {
        self.surfaces.route(id)
    }

    pub fn key_context(&self) -> postio_ui::keymap::KeyContext {
        if self.surfaces.top().is_none() && self.results.is_some() {
            return postio_ui::keymap::KeyContext::Results;
        }
        self.surfaces.key_context()
    }

    pub fn reading(&self) -> Option<MessageId> {
        self.surfaces.reading()
    }

    fn shown_apart(&self, rows: &dyn Rows) -> Option<MessageId> {
        let shown = self.shown?;
        (self.surfaces.top() == Some(SurfaceKind::Message)
            && self.cursor.row(rows).map(|row| row.id) != Some(shown))
        .then_some(shown)
    }

    fn shown_facts(&self, shown: MessageId, rows: &dyn Rows) -> RowFacts {
        rows.position_of(shown)
            .and_then(|at| rows.facts(at))
            .or_else(|| self.elsewhere_facts(shown))
            .unwrap_or(RowFacts {
                id: shown,
                digest: false,
                threads: Vec::new(),
                writes: false,
            })
    }

    fn leaves_the_list(&self, id: CommandId) -> bool {
        match id {
            CommandId::Archive | CommandId::Delete => true,
            CommandId::Unsnooze => self
                .feed
                .scope()
                .is_some_and(|scope| scope == ListScope::Focus(postio_model::FocusScope::Snoozed)),
            _ => false,
        }
    }

    fn list_command(&mut self, id: CommandId, rows: &dyn Rows) -> Vec<Effect> {
        let steps = self.list_steps(id, rows);
        self.effects(steps)
    }

    fn list_steps(&mut self, id: CommandId, rows: &dyn Rows) -> Vec<feed::Step> {
        // A toast with an Undo of its own -- a send, queued -- is the last
        // thing said, so Undo takes that back first (#1752).
        if id == CommandId::Undo
            && let Some(draft) = self.toast_undo.take()
        {
            return vec![feed::Step::Ask(Request::Post(Command::CancelSend {
                draft: Some(draft),
            }))];
        }
        if id == CommandId::CheatSheet {
            return self.open_key_map();
        }
        if let Some(steps) = self.compose_verb(id, rows) {
            return steps;
        }
        match id {
            CommandId::CaptureTask => {
                return self.open_capture(postio_ui::capture::Mode::Task, rows);
            }
            CommandId::CaptureNote => {
                return self.open_capture(postio_ui::capture::Mode::Note, rows);
            }
            CommandId::DigestRule => return self.new_rule(rows),
            CommandId::MuteThread | CommandId::ThreadOutline => {
                return self.thread_list_command(id, rows);
            }
            CommandId::ContactPopover => return self.open_contact_popover(None, rows),
            CommandId::OpenContacts => {
                let mut steps = self.open_contacts();
                steps.extend(self.contacts_go_to_import());
                return steps;
            }
            CommandId::ContactsSuggestions if self.policy.caps.contacts_view => {
                let mut steps = self.open_contacts();
                steps.extend(self.contacts_review_ask());
                steps.extend(self.contacts_go_to_import());
                return steps;
            }
            _ => {}
        }
        if let Some(shown) = self.shown_apart(rows) {
            if pickers::opens_picker(id) {
                return self.picker_on_message(id, shown, rows).unwrap_or_default();
            }
            if let Some(mut steps) = self.verbs.command_on(id, &self.shown_facts(shown, rows)) {
                if self.leaves_the_list(id) {
                    self.surfaces.dismiss(SurfaceKind::Message);
                    steps.push(feed::Step::Show(Intent::CloseSurface(SurfaceKind::Message)));
                    if !self.has_surface() {
                        steps.push(feed::Step::Show(Intent::KeyboardHome));
                    }
                }
                return steps;
            }
        }
        if let Some(steps) = self.picker_on_list(id, rows) {
            return steps;
        }
        if id == CommandId::OpenMessage {
            return self
                .cursor
                .position()
                .map(|position| self.open_at(position, rows))
                .unwrap_or_default();
        }
        if let Some(steps) = self.going(id, rows) {
            return steps;
        }
        // Archive on a digest's row archives its delivery, whatever else is
        // selected: a message verb has nothing to aim at there.
        if id == CommandId::Archive
            && let Some(delivery) = self.digest_at_cursor(rows)
        {
            return self.verbs.archive_digest(delivery);
        }
        let has_action = self.counts.map(|counts| counts.has_action);
        let total = self.feed.total();
        self.cursor
            .command(id, rows, self.feed.scope(), total, has_action)
            .or_else(|| self.verbs.command(id, &mut self.cursor, rows, total))
            .unwrap_or_default()
    }

    fn digest_at_cursor(&self, rows: &dyn Rows) -> Option<postio_model::DeliveryId> {
        let row = self.cursor.row(rows).filter(|row| row.digest)?;
        let position = self.cursor.position()?;
        match rows.row(position) {
            Some(FocusRow::Digest(digest)) => Some(digest.delivery),
            _ => self.feed.digest(row.id).map(|digest| digest.delivery),
        }
    }

    pub fn aim(&self, rows: &dyn Rows) -> Aimed {
        self.aim_for(rows, CommandId::Archive)
    }

    pub fn aim_for(&self, rows: &dyn Rows, action: CommandId) -> Aimed {
        let through = self.through(rows);
        let rows: &dyn Rows = through.as_ref().map_or(rows, |through| through);
        verbs::Verbs::aimed(&self.cursor, rows, action)
    }

    pub fn restoring(&self) -> bool {
        self.verbs.restoring()
    }

    pub fn take_restored(&mut self) -> bool {
        self.verbs.take_restored()
    }

    fn open_at(&mut self, position: u32, rows: &dyn Rows) -> Vec<feed::Step> {
        let Some(row) = rows.facts(position) else {
            return Vec::new();
        };
        if row.digest {
            return self.open_digest(row.id, position, rows);
        }
        if row.writes {
            return self.write(ComposerKind::Draft, Some(row.id));
        }
        let (index, total) = self.place_of(position, rows);
        self.surfaces
            .open(&row, index, total, self.policy.caps.stacking)
    }

    fn surface_command(&mut self, id: CommandId, rows: &dyn Rows) -> Option<Vec<feed::Step>> {
        if self.surfaces.top() == Some(SurfaceKind::Bar) && id == CommandId::Back {
            return Some(self.dismiss_bar(false));
        }
        if self.surfaces.top() == Some(SurfaceKind::Composer) {
            return Some(self.composer_command(id, rows));
        }
        if let Some(steps) = self.compose_verb(id, rows) {
            return Some(steps);
        }
        match self.surfaces.top() {
            Some(SurfaceKind::Filtered) if self.filtered.is_some() => {
                return self.filtered_command(id, rows);
            }
            Some(SurfaceKind::Digest) if self.digest.is_some() => {
                return self.digest_command(id, rows);
            }
            Some(SurfaceKind::Capture) if self.capture.is_some() => {
                return self.capture_command(id);
            }
            Some(SurfaceKind::ContactPopover) if self.contact_popover.is_some() => {
                return self.contact_popover_command(id, rows);
            }
            Some(SurfaceKind::Contacts) if self.contacts.is_some() => {
                return self.contacts_command(id, rows);
            }
            Some(SurfaceKind::ThreadMenu | SurfaceKind::Outline) => {
                return self.thread_surface_command(id, rows);
            }
            _ => {}
        }
        if self.surfaces.top() != Some(SurfaceKind::Message) {
            let closed = self.surfaces.close_top(id);
            self.forget_closed();
            return closed.or_else(|| {
                (self.surfaces.takes_keyboard() && keys::key_map_takes(id)).then(Vec::new)
            });
        }
        if let Some(steps) = self.walk_command(id) {
            return Some(steps);
        }
        if let Some(steps) = self.thread_window_command(id, rows) {
            return Some(steps);
        }
        if let Some(steps) = self.surfaces.reader_key(id) {
            return Some(steps);
        }
        let reading = self.surfaces.reading()?;
        if self.surfaces.origin() != surfaces::Origin::List {
            return self.elsewhere_command(id, reading, rows);
        }
        let position = rows.position_of(reading);
        match id {
            CommandId::NextMessage | CommandId::PrevMessage
                if self.hit_facts(reading).is_some() =>
            {
                let by = if id == CommandId::NextMessage { 1 } else { -1 };
                let mut steps = self.step_hit(reading, by)?;
                steps.extend(self.results_follow(self.surfaces.reading()));
                Some(steps)
            }
            CommandId::NextMessage | CommandId::PrevMessage => {
                let by = if id == CommandId::NextMessage { 1 } else { -1 };
                let (mut steps, moved) = self.cursor.step(by, rows);
                if moved && let Some(at) = self.cursor.position() {
                    steps.extend(self.open_at(at, rows));
                }
                Some(steps)
            }
            CommandId::FirstMessage | CommandId::LastMessage => {
                let has_action = self.counts.map(|counts| counts.has_action);
                let total = self.feed.total();
                let mut steps = self
                    .cursor
                    .command(id, rows, self.feed.scope(), total, has_action)
                    .unwrap_or_default();
                if let Some(at) = self.cursor.position()
                    && rows.facts(at).is_some_and(|row| row.id != reading)
                {
                    steps.extend(self.open_at(at, rows));
                }
                Some(steps)
            }
            CommandId::ContactPopover => Some(self.open_contact_popover(None, rows)),
            CommandId::DismissMarker => Some(self.verbs.post(Command::DismissMarker {
                target: MessageTarget::Messages(vec![reading]),
                dismissed: true,
            })),
            CommandId::OpenMessage => Some(self.edit_open_draft(rows).unwrap_or_default()),
            _ if pickers::opens_picker(id) => self.picker_on_message(id, reading, rows),
            _ => {
                let row = position
                    .and_then(|at| rows.facts(at))
                    .or_else(|| self.hit_facts(reading))?;
                let mut steps = self.verbs.command_on(id, &row)?;
                if self.leaves_the_list(id)
                    && let Some(at) = position
                {
                    steps.push(self.surfaces.step_past(at, row.id));
                }
                Some(steps)
            }
        }
    }

    fn follow_cursor(&mut self, rows: &dyn Rows) -> Vec<feed::Step> {
        if self.surfaces.top() != Some(SurfaceKind::Message) {
            return Vec::new();
        }
        let Some(at) = self.cursor.position() else {
            return Vec::new();
        };
        let Some(row) = rows.facts(at) else {
            return Vec::new();
        };
        if self.surfaces.reading() == Some(row.id)
            || self.opened_for(row.id, self.surfaces.reading())
        {
            return Vec::new();
        }
        if row.digest || row.writes {
            self.surfaces.dismiss(SurfaceKind::Message);
            let mut steps = vec![feed::Step::Show(Intent::CloseSurface(SurfaceKind::Message))];
            if !self.has_surface() {
                steps.push(feed::Step::Show(Intent::KeyboardHome));
            }
            return steps;
        }
        self.open_at(at, rows)
    }

    pub fn answers(&self, id: CommandId) -> bool {
        if self.surfaces.top() == Some(SurfaceKind::Bar) && bar::bar_key(id) {
            return true;
        }
        if pickers::picker_key(id) {
            return true;
        }
        if self.surfaces.top().is_none() {
            if self.results.is_some() && results::results_key(id) {
                return true;
            }
            // ⌘⌫ clears the filters while nothing matches, and is nobody's otherwise.
            if id == CommandId::BackToWords
                && self
                    .results
                    .as_ref()
                    .is_some_and(results::Results::found_nothing)
            {
                return true;
            }
            if matches!(id, CommandId::HistoryBack | CommandId::HistoryForward) {
                return true;
            }
        }
        if self.surfaces.top() == Some(SurfaceKind::Composer) {
            return Self::composer_answers(id);
        }
        let own = match self.surfaces.top() {
            Some(SurfaceKind::Filtered) => Self::filtered_answers(id),
            Some(SurfaceKind::Digest) => Self::digest_answers(id),
            Some(SurfaceKind::Capture) => Self::capture_answers(id),
            Some(SurfaceKind::ContactPopover) => Self::contact_popover_answers(id),
            Some(SurfaceKind::Contacts) => Self::contacts_answers(id),
            Some(SurfaceKind::ThreadMenu | SurfaceKind::Outline) => {
                Self::thread_surface_answers(id)
            }
            _ => false,
        };
        if own {
            return true;
        }
        if let Some(answer) = self.surfaces.answers(id) {
            return answer;
        }
        matches!(
            id,
            CommandId::NextMessage
                | CommandId::PrevMessage
                | CommandId::FirstMessage
                | CommandId::LastMessage
                | CommandId::ToggleSelection
                | CommandId::ExtendSelectionDown
                | CommandId::ExtendSelectionUp
                | CommandId::SelectAll
                | CommandId::ToggleHasAction
                | CommandId::Back
                | CommandId::AcceptInvite
                | CommandId::DeclineInvite
                | CommandId::DismissMarker
                | CommandId::OpenMessage
                | CommandId::ExpandThread
                | CommandId::CollapseThread
                | CommandId::CheatSheet
                | CommandId::CaptureTask
                | CommandId::CaptureNote
                | CommandId::DigestRule
                | CommandId::ContactPopover
                | CommandId::ThreadMenu
                | CommandId::ThreadOutline
                | CommandId::MuteThread
                | CommandId::SplitThread
                | CommandId::MoveThisMessage
                | CommandId::DeleteThisMessage
                | CommandId::OpenContacts
                | CommandId::ContactsSuggestions
        ) || bar::goes(id)
            || compose::compose_key(id)
            || pickers::opens_picker(id)
            || postio_ui::focus_target::dispatch(id).is_some()
    }

    pub fn cursor(&self) -> Option<u32> {
        self.cursor.position()
    }

    pub fn selection(&self) -> Selection {
        self.cursor.selection()
    }

    /// Kept from when it was selected, so a row scrolled out of the window is still aimed at in
    /// full. Empty with nothing selected.
    pub fn selection_reach(
        &self,
    ) -> &std::collections::HashMap<MessageId, Vec<postio_model::ThreadId>> {
        self.cursor.reach()
    }

    pub fn has_action(&self) -> bool {
        self.cursor.has_action()
    }

    pub fn cursor_is(&mut self, position: Option<u32>) {
        self.cursor.moved_to(position);
    }

    pub fn clear_selection(&mut self) -> Vec<Effect> {
        let steps = self.cursor.clear(self.feed.total());
        self.effects(steps)
    }

    /// The caller opens the place and names it with `note_place`.
    pub fn leave_list(&mut self) -> Vec<Effect> {
        self.expansion = None;
        let mut steps = self.cursor.leave(self.feed.total());
        steps.push(feed::Step::Show(Intent::SingleHeading(None)));
        self.effects(steps)
    }

    pub fn keymap(&self) -> &postio_core::Keymap {
        self.bar.keymap()
    }

    pub fn counts(&self) -> Option<FocusCounts> {
        self.counts
    }

    pub fn open(&mut self, scope: ListScope) -> Vec<Effect> {
        self.expansion = None;
        self.cursor.opening();
        let request = self.feed.open(scope);
        vec![Effect::Ask(self.ticket(), request)]
    }

    /// The stamp is the list's own (GTK's model generation, the Mac's window generation), echoed in
    /// every answer.
    pub fn page_wanted(&mut self, page: u32, stamp: u64) -> Vec<Effect> {
        let steps = self.feed.wanted(page, stamp);
        self.effects(steps)
    }

    pub fn scope(&self) -> Option<ListScope> {
        self.feed.scope()
    }

    pub fn list_total(&self) -> u32 {
        self.feed.total()
    }

    pub fn pages_asked(&self) -> &[u32] {
        self.feed.pages_asked()
    }

    pub fn has_landed(&self) -> bool {
        self.feed.has_landed()
    }

    pub fn take_opened(&mut self) -> bool {
        self.feed.take_opened()
    }

    pub fn is_inbox(&self, mailbox: MailboxId) -> bool {
        self.feed.is_inbox(mailbox)
    }

    pub fn message_place(&self, index: u32, total: u32) -> (u32, u32) {
        self.feed.message_place(index, total)
    }

    fn rereading(&mut self, steps: &[feed::Step], rows: &dyn Rows) {
        if steps
            .iter()
            .any(|step| matches!(step, feed::Step::Show(Intent::RefreshList)))
        {
            self.cursor.rereading(rows);
        }
    }

    fn effects(&mut self, steps: Vec<feed::Step>) -> Vec<Effect> {
        steps
            .into_iter()
            .map(|step| match step {
                feed::Step::Show(intent) => {
                    if matches!(intent, Intent::Toast { .. }) {
                        self.toast_undo = None;
                    }
                    if let Intent::Place { name } = &intent {
                        self.note_place(name);
                    }
                    Effect::Show(intent)
                }
                // Refresh is posted from many places; only here is the place on screen known to all
                // of them.
                feed::Step::Ask(Request::Post(Command::Refresh)) => {
                    let viewing = self.viewing();
                    Effect::Ask(self.ticket(), Request::Refresh { viewing })
                }
                feed::Step::Ask(request) => Effect::Ask(self.ticket(), request),
                feed::Step::Open(scope) => {
                    self.expansion = None;
                    self.cursor.opening();
                    let request = self.feed.open(scope);
                    Effect::Ask(self.ticket(), request)
                }
                feed::Step::Timer { token, after } => Effect::Timer { token, after },
            })
            .collect()
    }

    /// Every filter of Focus's inbox draws from the same inboxes, so each refreshes them all.
    fn viewing(&self) -> Option<postio_core::state::ViewScope> {
        let accounts = self.cursor.accounts();
        match self.feed.scope()? {
            ListScope::Focus(_) => postio_core::aim::view_scope(
                ListScope::Focus(postio_model::FocusScope::Inbox),
                accounts,
            ),
            scope => postio_core::aim::view_scope(scope, accounts),
        }
    }

    pub fn refresh_counts(&mut self) -> Effect {
        Effect::Ask(self.ticket(), Request::FocusCounts)
    }

    pub fn invalidate(&mut self) {
        self.generation += 1;
        self.pending = None;
        self.bar.places_forgotten();
    }

    fn forget_closed(&mut self) {
        if !self.surfaces.has(SurfaceKind::Filtered) {
            self.filtered = None;
        }
        if !self.surfaces.has(SurfaceKind::Digest) {
            self.digest = None;
        }
        if !self.surfaces.has(SurfaceKind::Dialog) {
            self.rule = None;
        }
        if !self.surfaces.has(SurfaceKind::Capture) {
            self.capture = None;
        }
        if !self.surfaces.has(SurfaceKind::ContactPopover) {
            self.contact_popover = None;
        }
        if !self.surfaces.has(SurfaceKind::Contacts) {
            self.contacts = None;
        }
        self.forget_thread_surfaces();
    }

    fn surface_reply(&mut self, reply: Reply, rows: &dyn Rows) -> Vec<feed::Step> {
        match reply {
            reply @ (Reply::FilteredTabs(_) | Reply::Filtered { .. } | Reply::SweepPreview(_)) => {
                self.filtered_reply(reply)
            }
            reply @ (Reply::DigestRead { .. } | Reply::Unsubscribed(_)) => self.digest_reply(reply),
            reply @ (Reply::DigestPreview { .. }
            | Reply::LikeThis { .. }
            | Reply::RuleSaved { .. }) => self.rule_reply(reply),
            reply @ (Reply::Vault { .. } | Reply::Captured { .. }) => self.capture_reply(reply),
            Reply::ContactGlance { stamp, glance } => self.contact_popover_reply(stamp, glance),
            reply @ (Reply::ContactsOverview(_)
            | Reply::ContactsPage { .. }
            | Reply::ContactsFiltered { .. }
            | Reply::ContactCard { .. }
            | Reply::ContactGroups(_)
            | Reply::ContactReview(_)
            | Reply::ContactPosition { .. }
            | Reply::ContactMergeOf(_)
            | Reply::ContactSeen(_)) => self.contacts_reply(reply),
            Reply::ContactSuggestions(answer) => self.contacts_suggestions_read(answer),
            Reply::FoundMessage { message, row } => self.link_found(message, row, rows),
            Reply::ThreadOf { message, thread } => self.walk_followed(message, thread),
            Reply::DraftBehind { command, draft, .. } => self.draft_behind(command, draft),
            Reply::Forwarded { form, answer } => self.forwarded(form, answer),
            Reply::RepliedToLatest(answer) => self.replied_to_latest(answer),
            _ => Vec::new(),
        }
    }

    fn elsewhere_command(
        &mut self,
        id: CommandId,
        reading: MessageId,
        rows: &dyn Rows,
    ) -> Option<Vec<feed::Step>> {
        let origin = self.surfaces.origin();
        let (at, len) = match origin {
            surfaces::Origin::Filtered => {
                let filtered = self.filtered.as_ref()?;
                (filtered.index_of(reading), filtered.len())
            }
            surfaces::Origin::Digest => {
                let window = self.digest.as_ref()?;
                (window.index_of(reading), window.len())
            }
            surfaces::Origin::List => return None,
        };
        match id {
            CommandId::NextMessage | CommandId::PrevMessage => {
                let by = if id == CommandId::NextMessage { 1 } else { -1 };
                let next = at.map_or(0, |at| at as i64 + by);
                if next < 0 || next >= len as i64 {
                    return Some(Vec::new());
                }
                Some(match origin {
                    surfaces::Origin::Filtered => self.open_filtered(next as usize),
                    _ => self.open_digest_message(next as usize),
                })
            }
            CommandId::OpenMessage => Some(Vec::new()),
            _ if pickers::opens_picker(id) => self.picker_on_message(id, reading, rows),
            _ => {
                let row = self.elsewhere_facts(reading)?;
                self.verbs.command_on(id, &row)
            }
        }
    }

    pub(crate) fn elsewhere_facts(&self, reading: MessageId) -> Option<RowFacts> {
        if let Some(found) = self.hit_facts(reading) {
            return Some(found);
        }
        if let Some(filtered) = &self.filtered
            && let Some(at) = filtered.index_of(reading)
        {
            return filtered.facts(at);
        }
        let window = self.digest.as_ref()?;
        window.facts(window.index_of(reading)?)
    }

    pub(crate) fn elsewhere_said(&self, reading: MessageId) -> Option<(String, String)> {
        if let Some(filtered) = &self.filtered
            && let Some(at) = filtered.index_of(reading)
        {
            return filtered.said(at);
        }
        let window = self.digest.as_ref()?;
        window.said(window.index_of(reading)?)
    }

    pub(crate) fn stamp(&mut self) -> u64 {
        self.stamps += 1;
        self.stamps
    }

    fn ticket(&mut self) -> Ticket {
        self.next_ticket += 1;
        Ticket {
            id: self.next_ticket,
            generation: self.generation,
        }
    }
}

pub(crate) fn said_of(message: &postio_model::listing::MessageSummary) -> (String, String) {
    (
        message
            .from
            .as_ref()
            .map(|from| from.display().to_owned())
            .unwrap_or_default(),
        message.subject.clone().unwrap_or_default(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_search_requests_name_their_lanes_and_nothing_else_does() {
        let query = postio_search::ParsedQuery::default();
        let conversations = Request::Conversations {
            query: query.clone(),
            order: postio_search::results::ConversationOrder::BestMatch,
            limit: 4,
            stamp: 1,
        };
        assert_eq!(conversations.lane(), Some(Lane::Conversations));
        let passages = Request::Passages {
            query: query.clone(),
            hits: Vec::new(),
            stamp: 1,
        };
        assert_eq!(passages.lane(), Some(Lane::Passages));
        let matches = Request::QuickLookMatches {
            query: query.clone(),
            key: postio_search::results::ConversationKey::Lone(MessageId::new(1)),
            stamp: 1,
        };
        assert_eq!(matches.lane(), Some(Lane::Matches));
        for request in [
            Request::RecentSearches,
            Request::RememberSearch {
                query: "atlas".to_owned(),
                hits: 3,
                capped: false,
            },
            Request::ForgetSearch {
                query: "atlas".to_owned(),
            },
            Request::MarkSeen {
                key: "atlas".to_owned(),
            },
            Request::Places,
        ] {
            assert_eq!(request.lane(), None, "{request:?}");
        }
    }

    fn counts(has_action: u32) -> FocusCounts {
        FocusCounts {
            conversations: 10,
            has_action,
            ..FocusCounts::default()
        }
    }

    #[test]
    fn the_controller_can_cross_threads() {
        fn send<T: Send>() {}
        send::<FocusController>();
    }

    #[test]
    fn linux_stacks_and_reads_beside_and_the_mac_does_neither() {
        let linux = Policy::for_platform(Platform::Freedesktop);
        let mac = Policy::for_platform(Platform::Apple);
        assert!(linux.caps.stacking && linux.caps.reading_pane);
        assert!(!mac.caps.stacking && !mac.caps.reading_pane);
    }

    #[test]
    fn counts_asked_for_are_drawn_when_they_come() {
        for platform in [Platform::Freedesktop, Platform::Apple] {
            let mut focus = FocusController::new(Policy::for_platform(platform));
            let Effect::Ask(ticket, Request::FocusCounts) = focus.refresh_counts() else {
                panic!("refreshing asks for the counts");
            };
            assert_eq!(
                focus.handle(Input::Reply(ticket, Reply::FocusCounts(Ok(counts(7))))),
                vec![Effect::Show(Intent::Counts(counts(7)))],
            );
        }
    }

    #[test]
    fn a_reply_asked_before_an_invalidation_changes_nothing() {
        let mut focus = FocusController::new(Policy::for_platform(Platform::Apple));
        let Effect::Ask(stale, _) = focus.refresh_counts() else {
            panic!("refreshing asks");
        };
        focus.invalidate();
        assert!(
            focus
                .handle(Input::Reply(stale, Reply::FocusCounts(Ok(counts(3)))))
                .is_empty()
        );
    }

    #[test]
    fn tickets_are_never_reused() {
        let mut focus = FocusController::new(Policy::for_platform(Platform::Apple));
        let Effect::Ask(first, _) = focus.refresh_counts() else {
            panic!()
        };
        let Effect::Ask(second, _) = focus.refresh_counts() else {
            panic!()
        };
        assert_ne!(first.id, second.id);
    }

    #[test]
    fn quit_is_drawn_as_quit() {
        let mut focus = FocusController::new(Policy::for_platform(Platform::Freedesktop));
        assert_eq!(
            focus.handle(Input::Command(CommandId::Quit)),
            vec![Effect::Show(Intent::Quit)],
        );
    }
}
