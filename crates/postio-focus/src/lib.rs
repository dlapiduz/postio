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
mod cursor;
mod feed;
mod keys;
mod perform;
mod pickers;
mod states;
mod surfaces;
mod verbs;

pub use bar::{BarLine, BarLineKind, BarMode, BarView, Found, FoundRow, PlacesRead};
pub use cursor::{NoRows, RowFacts, Rows};
pub use feed::{Opened, PageAnswer};
pub use perform::{perform, perform_now};
pub use pickers::{
    Anchor, FoldersRead, LabelsRead, PickerField, PickerKind, PickerRow, PickerView,
};
pub use states::{AccountsRead, BannerButton, BannerView};
pub use surfaces::{ReaderVerb, SurfaceKind};
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
    /// Call back with [`Input`] once `after` has passed. (Arrives with the
    /// first slice that needs one.)
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
    /// the one it shows -- at `index` of `total` in the list.
    OpenMessage {
        /// The message.
        message: MessageId,
        /// Its row's place in the list.
        index: u32,
        /// How many rows the list draws.
        total: u32,
    },
    /// A digest's row opens its window: the row is named by its delivery,
    /// negated, as [`RowFacts::id`] names it.
    OpenDigest {
        /// The row.
        row: MessageId,
    },
    /// A draft not yet on its way opens in the composer, to be written.
    OpenDraft {
        /// The draft's message.
        message: MessageId,
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
            bar: bar::Bar::new(policy.platform),
            pickers: pickers::Pickers::default(),
            toast_undo: None,
            counts: None,
            states: states::States::default(),
            keys: keys::Keys::default(),
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
        effects
    }

    fn handle_input(&mut self, input: Input, rows: &dyn Rows) -> Vec<Effect> {
        // A picker's keys are its own while it is up.
        if let Input::Command(id) = &input
            && let Some(steps) = self.picker_command(*id)
        {
            return self.effects(steps);
        }
        match input {
            Input::Command(CommandId::Quit) => vec![Effect::Show(Intent::Quit)],
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
                let steps = self.surfaces.opened(kind, self.policy.caps.stacking);
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
                self.effects(steps)
            }
            Input::ReaderState { more_open, finding } => {
                self.surfaces.reader_state(more_open, finding);
                Vec::new()
            }
            Input::Accounts(accounts) => {
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
                self.effects(steps)
            }
            Input::Config(config) => {
                let mut steps = self.bar_input(Input::Filtering(config.filtering), rows);
                steps.extend(self.set_config(config));
                self.effects(steps)
            }
            Input::Clock(clock) => {
                self.pickers.clock = clock;
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
            input @ (Input::Typed { .. }
            | Input::BarRun(_)
            | Input::BarTab
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
            return vec![feed::Step::Show(Intent::OpenDigest { row: row.id })];
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
        let position = rows.position_of(reading);
        match id {
            // A message the bar opened walks the bar's hits.
            CommandId::NextMessage | CommandId::PrevMessage
                if self.hit_facts(reading).is_some() =>
            {
                let by = if id == CommandId::NextMessage { 1 } else { -1 };
                self.step_hit(reading, by)
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
            // `Return` on the message on screen: it is open.
            CommandId::OpenMessage => Some(Vec::new()),
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
        if self.surfaces.top() == Some(SurfaceKind::Bar) && bar::bar_key(id) {
            return true;
        }
        if pickers::picker_key(id) {
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
        ) || bar::goes(id)
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
