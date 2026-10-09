//! Focus's behaviour, decided once (ADR 0045, `specs/009-focus-macos`).
//!
//! A frontend turns what happens to it — a key, an engine event, a client's
//! reply, a fact about its own window — into an [`Input`], hands it to the
//! [`FocusController`], and does what the returned [`Effect`]s say: draw an
//! [`Intent`], run a [`Request`] through [`perform()`] and feed the [`Reply`]
//! back, or set a timer. The controller does no I/O and never awaits, so the
//! GTK app can drive it from glib's main loop and the FFI from tokio, and
//! every rule in it is a unit test that runs on either host (research R1).
//!
//! Rules arrive here a slice at a time, moved out of `postio-gtk`'s window
//! (research R2); each slice brings its inputs, intents and requests with it.

use std::time::Duration;

use postio_client::protocol::FocusCounts;
use postio_config::paths::Platform;
use postio_core::state::Selection;
use postio_core::{Command, CommandId, Event, MessageTarget};
use postio_model::listing::{PageRequest, Surfaced};
use postio_model::{AccountId, ListScope, MailboxId, MailboxRole, MessageId};
use postio_ui::focus_list::FocusRow;

mod bar;
mod capture;
mod compose;
mod confirm;
mod cursor;
mod digest;
mod dropdown;
mod feed;
mod filtered;
mod history;
mod keys;
mod perform;
mod pickers;
mod results;
mod states;
mod surfaces;
mod verbs;

pub use bar::{BarLine, BarLineKind, BarMode, BarView, Found, FoundRow, PlacesRead};
pub use capture::{CaptureProject, CaptureView};
pub use compose::{AUTOSAVE, ComposerKind};
pub use confirm::Confirm;
pub use cursor::{NoRows, RowFacts, Rows};
pub use digest::{
    DigestCard, DigestEmail, DigestLine, DigestStatement, DigestTopic, DigestView, RulePreviewLine,
    RuleView,
};
pub use dropdown::{
    DropdownPill, DropdownRow, DropdownRowKind, DropdownSection, DropdownState, DropdownView, Lane,
    Run, RunStyle,
};
pub use feed::{Opened, PageAnswer};
pub use filtered::{FilteredLine, FilteredTab, FilteredView};
pub use perform::{perform, perform_now};
pub use pickers::{
    Anchor, FoldersRead, LabelsRead, PickerField, PickerKind, PickerRow, PickerView,
};
pub use postio_ui::capture::{Mode as CaptureMode, Pick as CapturePick};
pub use postio_ui::digest::{Page as DigestPage, Schedule as RuleSchedule};
pub use results::{
    Chip, DatePresetView, FilterButton, LabelPill, MatchCard, MonthBar, PopoverRow, PopoverView,
    QueryView, QuickLookView, ResultGroup, ResultRow, ResultsTabView, ResultsView, TermEdit,
};
pub use states::{AccountsRead, BannerButton, BannerView};
pub use surfaces::{Host, ReaderVerb, SurfaceKind};
pub use verbs::{Everything, ToastKind};

/// What differs between platforms, as policy rather than as a fork
/// (ADR 0045 rule 4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Policy {
    /// Which platform's layout and conventions apply.
    pub platform: Platform,
    /// What this frontend can draw.
    pub caps: Capabilities,
}

/// What a frontend can draw that another cannot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capabilities {
    /// A message can be read beside the list (spec 007 FR-038). Linux has
    /// it; the Mac comes to it after parity (spec 009 M4).
    pub reading_pane: bool,
    /// Secondary surfaces stack over each other: Linux's dialogs do, the
    /// Mac's windows replace each other (spec 009 M4).
    pub stacking: bool,
    /// Search has its two layers: the dropdown, and the results view it
    /// opens (spec 010 D17). The Mac has them; GTK's bar keeps spec 009's
    /// blend until Linux adopts them (S2).
    pub results_view: bool,
}

impl Policy {
    /// The policy each platform's Focus app runs with.
    pub const fn for_platform(platform: Platform) -> Self {
        let linux = matches!(platform, Platform::Freedesktop);
        Policy {
            platform,
            caps: Capabilities {
                reading_pane: linux,
                stacking: linux,
                results_view: !linux,
            },
        }
    }
}

/// One request in flight. The generation is the controller's when it asked;
/// a reply carrying an older one changes nothing (contract invariant 8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Ticket {
    /// Unique among this controller's requests.
    pub id: u64,
    /// The controller's generation when it asked.
    pub generation: u64,
}

/// Something that happened, told to the controller.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Input {
    /// A command, from a key, a button, the bar or a menu.
    Command(CommandId),
    /// The answer to an earlier [`Effect::Ask`].
    Reply(Ticket, Reply),
    /// Something the engine said, from the client's one event stream.
    Event(Event),
    /// The accounts Focus's inbox is made of: what "select everything"
    /// reaches.
    Accounts(Vec<AccountId>),
    /// Whether the list stands scrolled to its very top: where it goes back
    /// to after an undo brings rows in above.
    AtTop(bool),
    /// The pointer put the cursor on this row: a plain click.
    Point(u32),
    /// A modified click on this row: Shift's `range` takes every row from
    /// the anchor to it, the platform's toggle modifier takes it in or out.
    Pick {
        /// The row clicked.
        position: u32,
        /// Shift was held.
        range: bool,
    },
    /// The frontend showed a surface over the list.
    SurfaceOpened(SurfaceKind),
    /// A surface over the list closed, however it was closed.
    SurfaceClosed(SurfaceKind),
    /// The open message's More menu and find, as they are now: what Back
    /// closes first.
    ReaderState {
        /// The More menu is up.
        more_open: bool,
        /// Find is up.
        finding: bool,
    },
    /// The bar's words, as they are now.
    Typed {
        /// The field's text.
        text: String,
    },
    /// Run the bar's line with this token: Return on it, or a click.
    BarRun(u64),
    /// `Tab` in the bar: into the chips, and on from chip to chip. Nothing
    /// comes back when there is no chip to step into.
    BarTab,
    /// The dropdown's arrows rest on the row with this token now: what
    /// `ForgetRecent` forgets.
    SearchHighlighted(u64),
    /// `alt+BackSpace` on the dropdown's recent search with this token:
    /// forget it.
    SearchForget(u64),
    /// A filter button, a chip's ✕, a popover's check or the timeline
    /// changed the results' query (spec 010 step 3).
    SearchEdit(TermEdit),
    /// A filter button with a popover was pressed: open it (spec 010 step
    /// 4, FR-027). A toggle button's kind opens nothing.
    SearchPopover(postio_ui::search_view::FilterKind),
    /// A check on the open popover's row `token`: Space or a click
    /// toggles it, ⌥-click (`exclude`) excludes it.
    PopoverToggle {
        /// The row's [`PopoverRow::token`].
        token: u64,
        /// ⌥ was held.
        exclude: bool,
    },
    /// The popover's own search field holds this now.
    PopoverFilter(String),
    /// The Date popover's plain-words field holds this now.
    DateWords(String),
    /// The Date popover's preset with this
    /// [`token`](DatePresetView::token) was picked.
    DatePreset(u64),
    /// ↩ (`apply`), or Esc and a click away: the popover closes.
    PopoverDone {
        /// Keep what it previewed.
        apply: bool,
    },
    /// A results tab was picked: ⌘1-3, or a click.
    ResultsTab(postio_search::results::ResultsTab),
    /// The Sort menu.
    ResultsOrder(postio_search::results::ConversationOrder),
    /// A click put the focus ring on this result.
    ResultsPoint(u64),
    /// Go to the folders popover's place with this token
    /// ([`FocusController::places`]).
    OpenPlace(u64),
    /// The pinned saved searches, in order: each name and its query.
    SavedSearches(Vec<(String, String)>),
    /// What became of an [`Intent::SaveSearch`]: the saved searches now, or
    /// the sentence saying why it was not saved.
    SearchSaved(Result<Vec<(String, String)>, String>),
    /// The bindings in force: what the bar's keycaps say.
    Keymap(postio_core::Keymap),
    /// Whether Focus files mail away: the places list Filtered while it does.
    Filtering(bool),
    /// The clock stopped at this instant, or running again with `None`:
    /// what a picker's presets and a typed date count from. Unset, it is
    /// `postio_ui::clock::now()`.
    Clock(Option<chrono::DateTime<chrono::Local>>),
    /// The picker's field holds `text` now: the date typed, or the filter.
    PickerTyped {
        /// The field's text.
        text: String,
    },
    /// The picker's row with this token was chosen: a click, or Return on
    /// the row highlighted.
    PickerChoose(u64),
    /// `Space` on the picker's row with this token: a label on or off.
    PickerToggle(u64),
    /// A draft was queued to send, now or at `at`: the toast says so, and
    /// its Undo cancels the send.
    SendQueued {
        /// The draft.
        draft: postio_model::DraftId,
        /// When it leaves, when not at once.
        at: Option<chrono::DateTime<chrono::Utc>>,
    },
    /// `[focus]` as it stands: the digests an empty inbox names, and
    /// whether Focus files mail away.
    Config(postio_config::FocusConfig),
    /// A click on Filtered's row at this index: the keyboard goes to it.
    FilteredPoint(u32),
    /// Filtered was scrolled to its end: read its next page, when there
    /// may be one.
    FilteredMore,
    /// A click on the digest's list row at this index.
    DigestPoint(u32),
    /// A click on the summary's reference at this index, in reading order.
    DigestReference(u32),
    /// The person said yes to the [`Intent::Confirm`] with this token. A
    /// no is nothing: the question is forgotten when the next is asked.
    Confirmed(u64),
    /// The rule dialog's query entry holds `text` now.
    RuleQuery {
        /// The entry's text: queries, comma-separated.
        text: String,
    },
    /// "Match a list or a search instead…": the query entry, empty.
    RuleMatchInstead,
    /// "Digest mail like this": ask the person's model for a rule.
    RuleLikeThis,
    /// The rule dialog's cadence, day and time, as its controls hold them.
    RuleSchedule(postio_ui::digest::Schedule),
    /// Create (or Save): write the rule.
    RuleCreate,
    /// The capture field holds `text` now.
    CaptureTyped {
        /// The field's text.
        text: String,
    },
    /// A due day was picked for the capture, or none.
    CaptureDue(Option<chrono::NaiveDate>),
    /// The capture's project filter holds `text` now.
    CaptureFilter {
        /// The filter's text.
        text: String,
    },
    /// The capture's project row with this token was chosen.
    CaptureProject(u64),
    /// Open the message a `postio://` link names, or say why not.
    OpenLink(String),
    /// Something was written in the open composer: a recipient, the
    /// subject, a word of the body. What its autosave waits out.
    ComposerEdited,
    /// What became of an [`Intent::SaveDraft`]: the draft was kept
    /// (`Ok(true)`), there was nothing in it worth keeping (`Ok(false)`),
    /// or the sentence saying why it could not be saved.
    DraftSaved {
        /// The composition the save was asked for.
        composition: u64,
        /// What became of it.
        saved: Result<bool, String>,
    },
    /// A timer the controller set ([`Effect::Timer`]) has run out.
    Timer(u64),
}

/// What the frontend does next.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Effect {
    /// Draw this.
    Show(Intent),
    /// Run this through [`perform()`] and hand the answer back as
    /// [`Input::Reply`] with the same ticket.
    Ask(Ticket, Request),
    /// Call back with [`Input::Timer`]`(token)` once `after` has passed:
    /// the composer's autosave (slice 7).
    Timer {
        /// Handed back when the timer fires.
        token: u64,
        /// How long to wait.
        after: Duration,
    },
}

/// What the frontend draws. Each one is drawable without asking the
/// controller anything else.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Intent {
    /// The header strip's counts changed.
    Counts(FocusCounts),
    /// The cursor is on `position`: draw it there and bring it into view.
    Cursor {
        /// The row.
        position: u32,
        /// Whether the list scrolls back to its very top: the first row.
        to_top: bool,
    },
    /// The selection changed: redraw the rows' boxes, and the bar's words.
    Selection {
        /// What is selected now.
        selection: Selection,
        /// "3 selected", or nothing with nothing selected.
        summary: Option<String>,
    },
    /// The list's one heading, while `!` narrows it ("Has action · 7"), or
    /// back to the day headings with `None`.
    SingleHeading(Option<String>),
    /// Scroll the list back to its very top.
    ListToTop,
    /// Say something in the toast.
    Toast {
        /// What to say: the host's words.
        text: String,
        /// How to draw it, and how long it stays.
        kind: ToastKind,
    },
    /// The place opened has been counted: the list changes over to it,
    /// keeping the rows on screen until its first page lands, and asks for
    /// that page ([`FocusController::page_wanted`]).
    ReplaceSource {
        /// How many rows the new place draws.
        total: u32,
    },
    /// A page has been asked for under the list's `stamp`.
    PagePending {
        /// The list's own stamp, echoed.
        stamp: u64,
        /// Which page.
        page: u32,
    },
    /// A page's rows, for the list to take if `stamp` is still its own.
    DeliverPage {
        /// The stamp the page was asked under.
        stamp: u64,
        /// Which page.
        page: u32,
        /// How many rows the place draws now.
        total: u32,
        /// The rows, conversations and surfaced rows in list order.
        rows: Vec<FocusRow>,
    },
    /// A page could not be read and is being asked for again.
    AbandonPage {
        /// The stamp it was asked under.
        stamp: u64,
        /// Which page.
        page: u32,
    },
    /// A page could not be read and will not be asked for again.
    GiveUp {
        /// The stamp it was asked under.
        stamp: u64,
        /// Which page.
        page: u32,
    },
    /// Re-read the pages on screen, keeping the scroll position.
    RefreshList,
    /// Rows landed: a first page, or a re-read.
    Filled,
    /// Show `message` in the message surface -- opening it, or in place of
    /// the one it shows -- at `index` of `total` in the list it came from.
    OpenMessage {
        /// The message.
        message: MessageId,
        /// Its row's place in the list.
        index: u32,
        /// How many rows the list draws.
        total: u32,
        /// Where it is shown: its own surface, or the digest's window.
        host: Host,
    },
    /// A digest's row opens its window: the row is named by its delivery,
    /// negated, as [`RowFacts::id`] names it.
    OpenDigest {
        /// The row.
        row: MessageId,
    },
    /// Open the composer -- or, while it is open, put this composition in
    /// it in place of the one it held -- writing `kind`, answering
    /// `message`: a new message (no message), a reply, a reply to all, a
    /// forward, or the draft behind a row, to go on writing. The controller
    /// has put the composer on the stack.
    Composer {
        /// What is being written.
        kind: ComposerKind,
        /// The message it answers, or the draft's own message.
        message: Option<MessageId>,
    },
    /// Save what the composer holds for `composition` now, and say how it
    /// went with [`Input::DraftSaved`]: its autosave's quiet period has
    /// passed, or the composition is ending.
    SaveDraft {
        /// The composition: the one open, or the one just closed.
        composition: u64,
    },
    /// Close this surface; the frontend says [`Input::SurfaceClosed`] when
    /// it has.
    CloseSurface(SurfaceKind),
    /// The open message does this.
    Reader(ReaderVerb),
    /// Nothing is over the list any more: the keyboard goes back to it, on
    /// the cursor's row.
    KeyboardHome,
    /// Leave the app.
    Quit,
    /// Show the bar, opened `mode`'s way, its field holding `text` -- with
    /// `select` (a range of characters) selected, or the caret at the end.
    /// Said again while it is up, it is only new words for the field.
    OpenBar {
        /// How it was opened.
        mode: BarMode,
        /// The field's words.
        text: String,
        /// What of them is selected: the chip being edited.
        select: Option<(u32, u32)>,
    },
    /// Draw the bar's lines, whole.
    BarLines(BarView),
    /// Draw the search dropdown, whole, in place of the bar's lines.
    Dropdown(DropdownView),
    /// The main window shows a search's results: this frame, whole; its
    /// rows are read with [`FocusController::result_row`] (spec 010 D17).
    ShowResults(Box<ResultsView>),
    /// Results rows `first..first + count` changed: read them again.
    ResultsPage {
        /// The first row.
        first: u64,
        /// How many.
        count: u64,
    },
    /// The focus ring is on this result: draw it there and bring it into
    /// view.
    ResultsCursor(u64),
    /// The field's chips and words and the filter bar's buttons.
    Query(QueryView),
    /// Draw the filter popover, whole, hung from its button; `None` closes
    /// it.
    Popover(Option<Box<PopoverView>>),
    /// Draw Quick Look over the results, whole, in place of the one
    /// showing; `None` closes it (spec 010 US4).
    QuickLook(Option<Box<QuickLookView>>),
    /// The main window shows the inbox again; its cursor and selection
    /// follow.
    LeaveResults,
    /// The list shows this place now: what the header strip names.
    Place {
        /// "Inbox", "Receipts".
        name: String,
    },
    /// Show the folders popover; [`FocusController::places`] is what it
    /// lists.
    OpenPlaces,
    /// The places were read again: the popover lists them anew.
    PlacesChanged,
    /// Show Filtered, the view of what Focus filed away.
    ShowFiltered,
    /// Keep `query` as a saved search, and say how that went with
    /// [`Input::SearchSaved`].
    SaveSearch {
        /// The bar's query.
        query: String,
    },
    /// Run this command as the frontend's own: a line of the bar that is
    /// not the controller's to answer.
    Run(CommandId),
    /// Show this picker, hung from its anchor.
    OpenPicker(PickerView),
    /// Redraw the picker that is up, whole.
    PickerRows(PickerView),
    /// Put the keyboard in the picker's date field (`Tab`).
    PickerField,
    /// The banner under the header strip, or none.
    Banner(Option<BannerView>),
    /// What the sync label says now.
    SyncLabel(postio_ui::focus_state::SyncLabel),
    /// The page an empty list shows in its place, or the list again with
    /// `None`.
    Empty(Option<postio_ui::focus_state::EmptyInbox>),
    /// Show the key map (`?`). The controller has put it on the stack; it
    /// closes with [`Intent::CloseSurface`]`(KeyMap)`.
    OpenKeyMap,
    /// Draw Filtered, whole: its tabs, its rows, the row with the keyboard.
    Filtered(Box<FilteredView>),
    /// The keyboard is on Filtered's row at this index now.
    FilteredFocus(Option<u32>),
    /// Draw the digest's window, whole.
    Digest(Box<DigestView>),
    /// Ask before doing something no undo takes back whole; say
    /// [`Input::Confirmed`] with its token on yes.
    Confirm(Confirm),
    /// Show the digest rule dialog. The controller has put it on the stack
    /// as a [`SurfaceKind::Dialog`].
    OpenRule(Box<RuleView>),
    /// Redraw the rule dialog, whole.
    Rule(Box<RuleView>),
    /// Show the capture window. The controller has put it on the stack.
    OpenCapture(Box<CaptureView>),
    /// Redraw the capture window, whole.
    Capture(Box<CaptureView>),
}

/// What the controller needs from the engine. [`perform()`] is the one place
/// each becomes a client call.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Request {
    /// The header strip's counts.
    FocusCounts,
    /// Open a place: which folders are inboxes, how many rows it holds, and
    /// -- when it `splices` -- what Focus surfaces among them.
    OpenScope {
        /// The place.
        scope: ListScope,
        /// Whether to read the surfaced rows too.
        splices: bool,
    },
    /// One page of the place in view, with its label pills.
    Page {
        /// Which page of the list.
        page: u32,
        /// The list's stamp, echoed in the answer.
        stamp: u64,
        /// The page's first position, in the list's terms.
        start: u32,
        /// How many positions the page spans.
        count: u32,
        /// The run of conversations to read, in the store's terms.
        wanted: PageRequest,
    },
    /// What Focus surfaces in its inbox, read again.
    Surfaced,
    /// Send a verb, aimed: at each of `aims` in turn, or -- with
    /// `everything` -- at a whole-view selection the host resolves.
    Send {
        /// The verb.
        command: Command,
        /// Where it goes, in the order to send it.
        aims: Vec<MessageTarget>,
        /// A whole-view selection, instead of `aims`.
        everything: Option<Everything>,
    },
    /// Send a command as it is: one that aims at nothing (`Undo`).
    Post(Command),
    /// Mail left `mailbox`: said to the store before the list re-reads.
    NoteRemoved {
        /// The folder it left.
        mailbox: MailboxId,
        /// What left.
        messages: Vec<MessageId>,
    },
    /// Every place there is to go: one read, however many accounts.
    Places,
    /// Search this machine's index, for the bar.
    Search {
        /// The words, lowered.
        query: postio_search::ParsedQuery,
        /// Which order the hits come in.
        order: postio_search::ResultOrder,
        /// The bar's words' stamp, echoed in the answer.
        stamp: u64,
    },
    /// Search this machine's index for conversations: the dropdown's top
    /// hits and the facets it narrows by (spec 010).
    Conversations {
        /// The words, lowered.
        query: postio_search::ParsedQuery,
        /// Which order they come in.
        order: postio_search::results::ConversationOrder,
        /// How many hits.
        limit: u32,
        /// The bar's words' stamp, echoed in the answer.
        stamp: u64,
    },
    /// The passages of hits on screen, each where it matched.
    Passages {
        /// The query whose words the passages are cut around.
        query: postio_search::ParsedQuery,
        /// Each hit's best message, and where it matched.
        hits: Vec<(MessageId, Vec<postio_search::results::Source>)>,
        /// The bar's words' stamp, echoed in the answer.
        stamp: u64,
    },
    /// A page of the results view's conversations: its Top hits (Best
    /// match), or its month groups' rows (Newest).
    ResultsPage {
        /// The query, lowered.
        query: postio_search::ParsedQuery,
        /// Which order.
        order: postio_search::results::ConversationOrder,
        /// The first conversation.
        offset: u32,
        /// How many.
        limit: u32,
        /// The results' stamp, echoed in the answer.
        stamp: u64,
    },
    /// The passages of a page of results.
    ResultsPassages {
        /// The query whose words the passages are cut around.
        query: postio_search::ParsedQuery,
        /// Each hit's best message, and where it matched.
        hits: Vec<(MessageId, Vec<postio_search::results::Source>)>,
        /// The results' stamp, echoed in the answer.
        stamp: u64,
    },
    /// Every match in the conversation Quick Look shows.
    QuickLookMatches {
        /// The results' query.
        query: postio_search::ParsedQuery,
        /// Which conversation.
        key: postio_search::results::ConversationKey,
        /// Quick Look's stamp, echoed in the answer.
        stamp: u64,
    },
    /// The searches run lately, newest first.
    RecentSearches,
    /// Keep `query` among the searches run, with what it matched.
    RememberSearch {
        /// The query, as run.
        query: String,
        /// Conversations it matched.
        hits: u64,
    },
    /// Forget a recent search; answered with the recent searches left.
    ForgetSearch {
        /// The query.
        query: String,
    },
    /// How many each saved search matches.
    SavedCounts {
        /// Every saved search, as `(key, query)`.
        searches: Vec<(String, String)>,
        /// The day relative dates are read against.
        today: chrono::NaiveDate,
    },
    /// A folder's newest conversations, for `in:`.
    Folder {
        /// The folder.
        mailbox: MailboxId,
        /// The bar's words' stamp, echoed in the answer.
        stamp: u64,
    },
    /// The folder of `role` in the account Focus writes from: the first
    /// enabled account with one.
    RoleFolder(MailboxRole),
    /// What the label picker lists: the labels of `message`'s account (or
    /// of `account`, or the first enabled one), how many conversations
    /// carry each, and which of them `threads` carry.
    Labels {
        /// The message the picker opened on, whose account it is.
        message: Option<MessageId>,
        /// The account to read when no message says.
        account: Option<AccountId>,
        /// The conversations the picker acts on.
        threads: Vec<postio_model::ThreadId>,
        /// The picker's stamp, echoed in the answer.
        stamp: u64,
    },
    /// Make a label called `name` in `account`.
    CreateLabel {
        /// The account.
        account: AccountId,
        /// Its name, trimmed.
        name: String,
        /// The picker's stamp, echoed in the answer.
        stamp: u64,
    },
    /// What the move picker lists: every enabled account's destinations,
    /// and the last few moved to.
    Folders {
        /// The picker's stamp, echoed in the answer.
        stamp: u64,
    },
    /// Keep `mailbox` among the recent destinations.
    NoteMove(MailboxId),
    /// Who every enabled account is -- where it signs in, as whom -- and
    /// when mail last synced before this run: what a banner names.
    Accounts,
    /// Filtered's tabs: each reason, with how many it keeps.
    FilteredTabs,
    /// A page of Filtered: of `reason`, or of every reason.
    Filtered {
        /// The reason, as the store spells it; `None` for All.
        reason: Option<String>,
        /// The page's first row.
        offset: u32,
        /// The view's stamp, echoed in the answer.
        stamp: u64,
    },
    /// How many messages a sweep of the inbox would file away now.
    SweepPreview,
    /// What a digest's delivery holds, and -- with `summary` -- its summary.
    DigestRead {
        /// The delivery.
        delivery: postio_model::DeliveryId,
        /// Whether to read the summary: only with a model (C6).
        summary: bool,
        /// The window's stamp, echoed in the answer.
        stamp: u64,
    },
    /// Leave the list `message` came from.
    Unsubscribe(MessageId),
    /// What a digest rule matching `queries` would have caught since
    /// `since`.
    DigestPreview {
        /// The rule's queries.
        queries: Vec<String>,
        /// How far back.
        since: chrono::DateTime<chrono::Utc>,
        /// The dialog's stamp, echoed in the answer.
        stamp: u64,
    },
    /// The rule the person's model proposes for mail like `message`.
    DigestLikeThis {
        /// The message.
        message: MessageId,
        /// The dialog's stamp, echoed in the answer.
        stamp: u64,
    },
    /// Write a digest rule to `config.toml`.
    SaveDigestRule {
        /// The rule it replaces, when editing.
        replacing: Option<String>,
        /// The rule.
        draft: postio_client::protocol::DigestRuleDraft,
        /// The dialog's stamp, echoed in the answer.
        stamp: u64,
    },
    /// What capture needs of the vault for a message with `subject`.
    Vault {
        /// The message's subject.
        subject: String,
        /// The capture's stamp, echoed in the answer.
        stamp: u64,
    },
    /// Append a task to the vault.
    CaptureTask {
        /// Its project; `None` for the tasks note.
        project: Option<postio_vault::Project>,
        /// The task.
        task: postio_vault::Task,
        /// The capture's stamp, echoed in the answer.
        stamp: u64,
    },
    /// Append a note entry to the vault.
    CaptureNote {
        /// The note, relative to the vault.
        note: std::path::PathBuf,
        /// The entry.
        entry: postio_vault::NoteEntry,
        /// The capture's stamp, echoed in the answer.
        stamp: u64,
    },
    /// Whether this store holds `message`, and its row: what a link opens.
    FindMessage(MessageId),
    /// The draft behind `message`, for the send verb `command` to settle.
    DraftBehind {
        /// The message aimed at.
        message: MessageId,
        /// Cancel, retry or mark sent.
        command: CommandId,
    },
}

impl Request {
    /// The search lane this request is on, when it is one: a newer request
    /// of the same lane makes the one before it waste (spec 010 D8).
    pub fn lane(&self) -> Option<Lane> {
        match self {
            Request::Conversations { .. } => Some(Lane::Conversations),
            Request::Passages { .. } => Some(Lane::Passages),
            Request::QuickLookMatches { .. } => Some(Lane::Matches),
            _ => None,
        }
    }
}

/// The engine's answer to a [`Request`]. A failure is carried as its
/// sentence: the controller decides what to show, not how to recover.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Reply {
    /// The answer to [`Request::FocusCounts`].
    FocusCounts(Result<FocusCounts, String>),
    /// The answer to [`Request::OpenScope`].
    Opened(Opened),
    /// The answer to [`Request::Page`], with the request's own terms.
    Page {
        /// Which page.
        page: u32,
        /// The stamp it was asked under.
        stamp: u64,
        /// The page's first position.
        start: u32,
        /// How many positions it spans.
        count: u32,
        /// The rows, or why there are none.
        answer: Result<PageAnswer, String>,
    },
    /// The answer to [`Request::Surfaced`].
    Surfaced(Result<Vec<Surfaced>, String>),
    /// A post was made; nothing to answer.
    Noted,
    /// A command was sent, or why it could not be.
    Sent(Result<(), String>),
    /// The answer to [`Request::Places`].
    Places(Result<PlacesRead, String>),
    /// The answer to [`Request::Search`].
    Search {
        /// The stamp it was asked under.
        stamp: u64,
        /// What it found.
        answer: Result<Found, String>,
    },
    /// The answer to [`Request::Conversations`].
    Conversations {
        /// The stamp it was asked under.
        stamp: u64,
        /// What it found, boxed: it is the largest answer by far.
        answer: Result<Box<postio_search::results::ConversationResults>, String>,
    },
    /// The answer to [`Request::Passages`].
    Passages {
        /// The stamp it was asked under.
        stamp: u64,
        /// Each hit's matches, with their passages.
        answer: Result<Vec<(MessageId, Vec<postio_search::results::Match>)>, String>,
    },
    /// The answer to [`Request::ResultsPage`].
    ResultsPage {
        /// The stamp it was asked under.
        stamp: u64,
        /// The order asked.
        order: postio_search::results::ConversationOrder,
        /// The offset asked.
        offset: u32,
        /// What it found, boxed: it is the largest answer by far.
        answer: Result<Box<postio_search::results::ConversationResults>, String>,
    },
    /// The answer to [`Request::ResultsPassages`].
    ResultsPassages {
        /// The stamp it was asked under.
        stamp: u64,
        /// Each hit's matches, with their passages.
        answer: Result<Vec<(MessageId, Vec<postio_search::results::Match>)>, String>,
    },
    /// The answer to [`Request::QuickLookMatches`].
    QuickLookMatches {
        /// The stamp it was asked under.
        stamp: u64,
        /// The conversation's matches, oldest first.
        answer: Result<Vec<postio_search::results::ConversationMatch>, String>,
    },
    /// The answer to [`Request::RecentSearches`] and
    /// [`Request::ForgetSearch`].
    RecentSearches(Result<Vec<postio_client::protocol::RecentSearch>, String>),
    /// The answer to [`Request::SavedCounts`]: `(key, total, new)`.
    SavedCounts(Result<Vec<(String, u64, u64)>, String>),
    /// The answer to [`Request::Folder`].
    Folder {
        /// The stamp it was asked under.
        stamp: u64,
        /// How many conversations the folder holds.
        count: Result<u32, String>,
        /// Its newest conversations.
        rows: Result<Vec<FoundRow>, String>,
    },
    /// The answer to [`Request::RoleFolder`]: the folder and its name.
    RoleFolder(Option<(MailboxId, String)>),
    /// The answer to [`Request::Labels`].
    Labels {
        /// The stamp it was asked under.
        stamp: u64,
        /// What was read.
        answer: Result<LabelsRead, String>,
    },
    /// The answer to [`Request::CreateLabel`].
    LabelCreated {
        /// The stamp it was asked under.
        stamp: u64,
        /// The label made.
        answer: Result<postio_model::Label, String>,
    },
    /// The answer to [`Request::Folders`].
    Folders {
        /// The stamp it was asked under.
        stamp: u64,
        /// What was read.
        answer: Result<FoldersRead, String>,
    },
    /// The answer to [`Request::Accounts`].
    Accounts(Result<AccountsRead, String>),
    /// The answer to [`Request::FilteredTabs`].
    FilteredTabs(Result<Vec<(String, u32)>, String>),
    /// The answer to [`Request::Filtered`].
    Filtered {
        /// The stamp it was asked under.
        stamp: u64,
        /// The page's first row.
        offset: u32,
        /// The rows, newest first.
        answer: Result<Vec<postio_client::protocol::FilteredRow>, String>,
    },
    /// The answer to [`Request::SweepPreview`].
    SweepPreview(Result<u32, String>),
    /// The answer to [`Request::DigestRead`].
    DigestRead {
        /// The stamp it was asked under.
        stamp: u64,
        /// What the delivery holds, newest first.
        rows: Result<Vec<postio_model::listing::MessageSummary>, String>,
        /// Its summary, when one was asked for and one is written.
        summary: Option<postio_model::summary::DigestSummary>,
    },
    /// The answer to [`Request::Unsubscribe`]: the list's name.
    Unsubscribed(Result<String, String>),
    /// The answer to [`Request::DigestPreview`].
    DigestPreview {
        /// The stamp it was asked under.
        stamp: u64,
        /// What the rule would have caught.
        answer: Result<postio_client::protocol::DigestPreview, String>,
    },
    /// The answer to [`Request::DigestLikeThis`]: the queries proposed, or
    /// none.
    LikeThis {
        /// The stamp it was asked under.
        stamp: u64,
        /// The rule's queries, when the model found one.
        answer: Result<Option<Vec<String>>, String>,
    },
    /// The answer to [`Request::SaveDigestRule`].
    RuleSaved {
        /// The stamp it was asked under.
        stamp: u64,
        /// Whether it was written, or the sentence saying why not.
        answer: Result<(), String>,
    },
    /// The answer to [`Request::Vault`].
    Vault {
        /// The stamp it was asked under.
        stamp: u64,
        /// The vault's projects, the suggestion and the tasks.
        answer: Result<postio_client::protocol::VaultPicture, String>,
    },
    /// The answer to [`Request::CaptureTask`] and [`Request::CaptureNote`].
    Captured {
        /// The stamp it was asked under.
        stamp: u64,
        /// Whether it was written, or the sentence saying why not.
        answer: Result<(), String>,
    },
    /// The answer to [`Request::FindMessage`]: its row, when it is here.
    FoundMessage {
        /// The message asked about.
        message: MessageId,
        /// Its row, or `None` when the store does not hold it.
        row: Option<postio_model::listing::MessageSummary>,
    },
    /// The answer to [`Request::DraftBehind`].
    DraftBehind {
        /// The message asked about.
        message: MessageId,
        /// The send verb that asked.
        command: CommandId,
        /// The draft behind it, or `None` when it is no draft.
        draft: Option<postio_model::DraftId>,
    },
}

/// Focus's behaviour for one window. `Send`, and plain data: no toolkit
/// object, no executor, no client.
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
    /// The draft the toast showing can take back: a queued send's own Undo,
    /// which Undo runs before the stack's, until a newer toast replaces it.
    toast_undo: Option<postio_model::DraftId>,
    /// The strip's counts, as the host last said.
    counts: Option<FocusCounts>,
    states: states::States,
    keys: keys::Keys,
    filtered: Option<filtered::Filtered>,
    digest: Option<digest::DigestWindow>,
    rule: Option<digest::RuleDialog>,
    capture: Option<capture::Capture>,
    confirm: Option<confirm::Asked>,
    /// The composer, and the composition in it.
    compose: compose::Compose,
    /// Stamps the surfaces' reads, so an answer for one since moved on is
    /// dropped.
    stamps: u64,
    /// The main window's results, while it shows them (spec 010 D17).
    results: Option<results::Results>,
    /// What the main window showed before, and after (spec 010 R9).
    history: history::History,
}

impl FocusController {
    /// A controller for one window, under its platform's policy.
    pub fn new(policy: Policy) -> Self {
        FocusController {
            policy,
            generation: 0,
            next_ticket: 0,
            feed: feed::Feed::default(),
            cursor: cursor::Cursor::default(),
            verbs: verbs::Verbs::default(),
            surfaces: surfaces::Surfaces::default(),
            bar: bar::Bar::new(policy.platform, policy.caps.results_view),
            pickers: pickers::Pickers::default(),
            toast_undo: None,
            counts: None,
            states: states::States::default(),
            keys: keys::Keys::default(),
            filtered: None,
            digest: None,
            rule: None,
            capture: None,
            confirm: None,
            compose: compose::Compose::default(),
            stamps: 0,
            results: None,
            history: history::History::default(),
        }
    }

    /// The policy this controller was made with.
    pub fn policy(&self) -> Policy {
        self.policy
    }

    /// Tell the controller what happened; get back what to do. For inputs
    /// that need no rows; [`handle_on`](Self::handle_on) for the rest.
    pub fn handle(&mut self, input: Input) -> Vec<Effect> {
        self.handle_on(input, &NoRows)
    }

    /// Tell the controller what happened, over the list as the frontend
    /// holds it; get back what to do.
    pub fn handle_on(&mut self, input: Input, rows: &dyn Rows) -> Vec<Effect> {
        // A message a hit opened, closed however it was: the bar comes back
        // on its words.
        let reading = self.surfaces.has(SurfaceKind::Message);
        let mut effects = self.handle_input(input, rows);
        if reading && !self.surfaces.has(SurfaceKind::Message) {
            let steps = self.hit_closed(rows);
            effects.extend(self.effects(steps));
        }
        // The composer gone however it went -- its window's close button,
        // another window in its place (M4) -- ends its composition, saved.
        let steps = self.composer_gone();
        effects.extend(self.effects(steps));
        effects
    }

    fn handle_input(&mut self, input: Input, rows: &dyn Rows) -> Vec<Effect> {
        // A picker's keys are its own while it is up.
        if let Input::Command(id) = &input
            && let Some(steps) = self.picker_command(*id)
        {
            return self.effects(steps);
        }
        // The results are the main window's while they are up: their keys
        // are theirs, and a verb acts on the focused result.
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
            Input::SurfaceClosed(kind) => {
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
            Input::Accounts(accounts) => {
                self.compose.set_no_account(accounts.is_empty());
                self.cursor.set_accounts(accounts);
                Vec::new()
            }
            Input::AtTop(at_top) => {
                self.verbs.set_at_top(at_top);
                Vec::new()
            }
            Input::Point(position) => {
                let steps = self.cursor.point(position, rows);
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
                | Reply::FoundMessage { .. }
                | Reply::DraftBehind { .. }),
            ) => {
                let steps = self.surface_reply(reply, rows);
                self.effects(steps)
            }
            // So do the results'.
            Input::Reply(
                _,
                reply @ (Reply::ResultsPage { .. }
                | Reply::ResultsPassages { .. }
                | Reply::QuickLookMatches { .. }),
            ) => {
                let steps = self.results_reply(reply);
                self.effects(steps)
            }
            Input::Reply(ticket, _) if ticket.generation != self.generation => Vec::new(),
            Input::Reply(_, Reply::FocusCounts(Ok(counts))) => {
                self.counts = Some(counts);
                let mut steps = vec![feed::Step::Show(Intent::Counts(counts))];
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
                self.effects(steps)
            }
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
                | Reply::Search { .. }
                | Reply::Conversations { .. }
                | Reply::Passages { .. }
                | Reply::RecentSearches(_)
                | Reply::SavedCounts(_)
                | Reply::Folder { .. }
                | Reply::RoleFolder(_)),
            ) => {
                let steps = self.bar_reply(reply, rows);
                self.effects(steps)
            }
            Input::Event(event) => {
                let mut steps = self.verbs.event(&event);
                steps.extend(self.feed.event(&event));
                steps.extend(self.hear_sync(&event));
                steps.extend(self.filtered_event(&event));
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
            Input::ComposerEdited => {
                let steps = self.composer_edited();
                self.effects(steps)
            }
            Input::DraftSaved { composition, saved } => {
                let steps = self.draft_saved(composition, saved);
                self.effects(steps)
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
            | Input::ResultsPoint(_)) => {
                let steps = self.results_input(input);
                self.effects(steps)
            }
            input @ (Input::Typed { .. }
            | Input::BarRun(_)
            | Input::BarTab
            | Input::SearchHighlighted(_)
            | Input::SearchForget(_)
            | Input::OpenPlace(_)
            | Input::SavedSearches(_)
            | Input::SearchSaved(_)
            | Input::Filtering(_)) => {
                let steps = self.bar_input(input, rows);
                self.effects(steps)
            }
        }
    }

    /// The folders popover's places whose names hold `filter`, case aside,
    /// in the order it lists them, each with the token [`Input::OpenPlace`]
    /// goes to it by.
    pub fn places(&self, filter: &str) -> Vec<(u64, postio_ui::places::Entry)> {
        self.listed_places(filter)
    }

    /// The list has landed or moved (the frontend's "filled"), `opened` when
    /// this is the first landing of a place just opened: the cursor goes to
    /// the first row, or back to the message `!` kept, and the strip's
    /// counts are asked for again.
    pub fn landed(&mut self, rows: &dyn Rows, opened: bool) -> Vec<Effect> {
        let mut steps = self.cursor.landed(rows, opened);
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
            None => {}
        }
        steps.extend(self.show_empty());
        let mut effects = self.effects(steps);
        effects.push(self.refresh_counts());
        effects
    }

    /// Whether any surface the frontend reported is over the list.
    pub fn has_surface(&self) -> bool {
        self.surfaces.top().is_some()
    }

    /// The key context in force: the top surface's, or the list's.
    pub fn key_context(&self) -> postio_ui::keymap::KeyContext {
        if self.surfaces.top().is_none() && self.results.is_some() {
            return postio_ui::keymap::KeyContext::Results;
        }
        self.surfaces.key_context()
    }

    /// The message the open message surface shows, while it is on top.
    pub fn reading(&self) -> Option<MessageId> {
        self.surfaces.reading()
    }

    /// A command on the list, by the cursor's and the verbs' rules.
    fn list_command(&mut self, id: CommandId, rows: &dyn Rows) -> Vec<Effect> {
        let steps = self.list_steps(id, rows);
        self.effects(steps)
    }

    /// What a command on the list does: the bar's and the places', the
    /// cursor's, then the verbs'.
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
            _ => {}
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
        let has_action = self.counts.map(|counts| counts.has_action);
        let total = self.feed.total();
        self.cursor
            .command(id, rows, self.feed.scope(), total, has_action)
            .or_else(|| self.verbs.command(id, &mut self.cursor, rows, total))
            .unwrap_or_default()
    }

    /// Open the row at `position`: a message to read, a draft to write, a
    /// digest's window.
    fn open_at(&mut self, position: u32, rows: &dyn Rows) -> Vec<feed::Step> {
        let Some(row) = rows.facts(position) else {
            return Vec::new();
        };
        if row.digest {
            return self.open_digest(row.id, position, rows);
        }
        // A draft not yet on its way opens to be written (spec 007 US11).
        if row.writes {
            return self.write(ComposerKind::Draft, Some(row.id));
        }
        let (index, total) = self.feed.message_place(position, rows.len());
        self.surfaces
            .open(&row, index, total, self.policy.caps.stacking)
    }

    /// A command while a surface is over the list, or `None` when the
    /// surface has no say and the list's rules apply (contract invariant 5).
    fn surface_command(&mut self, id: CommandId, rows: &dyn Rows) -> Option<Vec<feed::Step>> {
        // Back on the bar takes it off the stack here, as running a line
        // does, rather than waiting for the frontend's `SurfaceClosed`: the
        // controller puts the bar up and takes it down itself (slice 8).
        if self.surfaces.top() == Some(SurfaceKind::Bar) && id == CommandId::Back {
            return Some(self.dismiss_bar(false));
        }
        // The composer takes its own keys; Esc and the writing verbs are
        // the controller's.
        if self.surfaces.top() == Some(SurfaceKind::Composer) {
            return Some(self.composer_command(id, rows));
        }
        if let Some(steps) = self.compose_verb(id, rows) {
            return Some(steps);
        }
        // The controller's own surfaces; one a frontend opened itself, with
        // nothing held for it here, closes by the general rule below.
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
            _ => {}
        }
        if self.surfaces.top() != Some(SurfaceKind::Message) {
            return self.surfaces.close_top(id).or_else(|| {
                // The key map and a dialog take the keyboard: what does not
                // close them does nothing (GTK's dialog close rule).
                (self.surfaces.takes_keyboard() && keys::key_map_takes(id)).then(Vec::new)
            });
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
            // A message the bar opened walks the bar's hits.
            CommandId::NextMessage | CommandId::PrevMessage
                if self.hit_facts(reading).is_some() =>
            {
                let by = if id == CommandId::NextMessage { 1 } else { -1 };
                let mut steps = self.step_hit(reading, by)?;
                // A message a result opened: the focus ring follows it.
                steps.extend(self.results_follow(self.surfaces.reading()));
                Some(steps)
            }
            // `j`/`k` step the list behind the message, and the message
            // follows the cursor; nothing closes.
            CommandId::NextMessage | CommandId::PrevMessage => {
                let by = if id == CommandId::NextMessage { 1 } else { -1 };
                let (mut steps, moved) = self.cursor.step(by, rows);
                if moved && let Some(at) = self.cursor.position() {
                    steps.extend(self.open_at(at, rows));
                }
                Some(steps)
            }
            // `Return` on the message on screen: Edit, on a draft on its
            // way or stopped; else it is open already.
            CommandId::OpenMessage => Some(self.edit_open_draft(rows).unwrap_or_default()),
            // A picker hangs from the message, aimed at it alone.
            _ if pickers::opens_picker(id) => self.picker_on_message(id, reading, rows),
            _ => {
                let row = position
                    .and_then(|at| rows.facts(at))
                    .or_else(|| self.hit_facts(reading))?;
                let steps = self.verbs.command_on(id, &row)?;
                if matches!(id, CommandId::Archive | CommandId::Delete)
                    && let Some(at) = position
                {
                    self.surfaces.step_past(at, row.id);
                }
                Some(steps)
            }
        }
    }

    /// Whether `id` is one of the controller's own commands: the cursor's,
    /// the selection's, `!`, and the verbs on the list. A frontend sends the
    /// rest -- a surface's commands, the composer's -- where it always did.
    pub fn answers(&self, id: CommandId) -> bool {
        if self.surfaces.top() == Some(SurfaceKind::Bar)
            && bar::bar_key(id, self.policy.caps.results_view)
        {
            return true;
        }
        if pickers::picker_key(id) {
            return true;
        }
        if self.surfaces.top().is_none() {
            if self.results.is_some() && results::results_key(id) {
                return true;
            }
            if self.policy.caps.results_view
                && matches!(id, CommandId::HistoryBack | CommandId::HistoryForward)
            {
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
                | CommandId::CheatSheet
                | CommandId::CaptureTask
                | CommandId::CaptureNote
                | CommandId::DigestRule
        ) || bar::goes(id)
            || compose::compose_key(id)
            || pickers::opens_picker(id)
            || postio_ui::focus_target::dispatch(id).is_some()
    }

    /// The cursor's row, if it has one.
    pub fn cursor(&self) -> Option<u32> {
        self.cursor.position()
    }

    /// What is selected: what `a` would act on.
    pub fn selection(&self) -> Selection {
        self.cursor.selection()
    }

    /// Whether the list is narrowed to the rows with a marker (`!`).
    pub fn has_action(&self) -> bool {
        self.cursor.has_action()
    }

    /// The bindings in force: what every keycap the controller spells
    /// says, and what [`press`](Self::press) resolves.
    pub fn keymap(&self) -> &postio_core::Keymap {
        self.bar.keymap()
    }

    /// The strip's counts, as the host last said.
    pub fn counts(&self) -> Option<FocusCounts> {
        self.counts
    }

    /// Show `scope` in the list: counted first, then the list changes over.
    pub fn open(&mut self, scope: ListScope) -> Vec<Effect> {
        let request = self.feed.open(scope);
        vec![Effect::Ask(self.ticket(), request)]
    }

    /// The list wants `page`, under its own `stamp` (GTK's model generation,
    /// the Mac's window generation): every answer for it carries the stamp
    /// back for the list to check.
    pub fn page_wanted(&mut self, page: u32, stamp: u64) -> Vec<Effect> {
        let steps = self.feed.wanted(page, stamp);
        self.effects(steps)
    }

    /// The place in view, once one is open.
    pub fn scope(&self) -> Option<ListScope> {
        self.feed.scope()
    }

    /// How many rows the place in view draws, as the store last said.
    pub fn list_total(&self) -> u32 {
        self.feed.total()
    }

    /// Every page asked of the store, in order.
    pub fn pages_asked(&self) -> &[u32] {
        self.feed.pages_asked()
    }

    /// Whether the first page of the place in view has landed.
    pub fn has_landed(&self) -> bool {
        self.feed.has_landed()
    }

    /// True once per opening, after its first page lands.
    pub fn take_opened(&mut self) -> bool {
        self.feed.take_opened()
    }

    /// Whether `mailbox` is one of the inboxes Focus's inbox is made of.
    pub fn is_inbox(&self, mailbox: MailboxId) -> bool {
        self.feed.is_inbox(mailbox)
    }

    /// The cursor's `index` among the messages of a list of `total` rows,
    /// and how many messages it holds, digests not counted.
    pub fn message_place(&self, index: u32, total: u32) -> (u32, u32) {
        self.feed.message_place(index, total)
    }

    fn effects(&mut self, steps: Vec<feed::Step>) -> Vec<Effect> {
        steps
            .into_iter()
            .map(|step| match step {
                feed::Step::Show(intent) => {
                    // Every toast replaces the one showing, and its Undo.
                    if matches!(intent, Intent::Toast { .. }) {
                        self.toast_undo = None;
                    }
                    if let Intent::Place { name } = &intent {
                        self.note_place(name);
                    }
                    Effect::Show(intent)
                }
                feed::Step::Ask(request) => Effect::Ask(self.ticket(), request),
                feed::Step::Open(scope) => {
                    let request = self.feed.open(scope);
                    Effect::Ask(self.ticket(), request)
                }
                feed::Step::Timer { token, after } => Effect::Timer { token, after },
            })
            .collect()
    }

    /// Ask for the header strip's counts.
    pub fn refresh_counts(&mut self) -> Effect {
        Effect::Ask(self.ticket(), Request::FocusCounts)
    }

    /// Forget every request in flight: their replies, when they come, change
    /// nothing. Called when what they were for is gone, a place left or a
    /// list replaced.
    pub fn invalidate(&mut self) {
        self.generation += 1;
    }

    /// What a surface no longer on the stack held is forgotten: the Mac's
    /// one secondary window replaced, a window closed by its own button.
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
    }

    /// An answer for one of the surfaces over the list.
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
            Reply::FoundMessage { message, row } => self.link_found(message, row, rows),
            Reply::DraftBehind { command, draft, .. } => self.draft_behind(command, draft),
            _ => Vec::new(),
        }
    }

    /// A command on a message opened from Filtered or a digest's list: `j`/
    /// `k` walk that list, and a verb reads its conversation from it.
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

    /// The facts of the open message where no list row holds them: a hit, a
    /// filtered message, a digest's.
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

    /// A stamp for a surface's read, never used before.
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The search requests name their lanes, so a driver can abort the one
    /// a newer request of the same lane makes waste (spec 010 D8); nothing
    /// else is on a lane, writes least of all.
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
            },
            Request::ForgetSearch {
                query: "atlas".to_owned(),
            },
            Request::Places,
            Request::Search {
                query,
                order: postio_search::ResultOrder::default(),
                stamp: 1,
            },
        ] {
            assert_eq!(request.lane(), None, "{request:?}");
        }
    }

    /// An inbox of ten conversations, `has_action` of them marked: one
    /// with mail in it, so no empty page follows the counts.
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
