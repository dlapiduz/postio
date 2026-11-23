//! The command bar, search and go-to: what is typed, which lines are on screen and which
//! results are current, and what running a line does to the list.

use postio_client::protocol::SavedCount;
use postio_core::{ActionId, CommandId, Context, Frontend, Keymap};
use postio_model::{Contact, FocusScope, ListScope, MailboxId, MailboxRole, MessageId, ThreadId};
use postio_ui::command_bar::{self as rules, BarAction, Line, Row, Run};
use postio_ui::finder::{self, Destination, Place};
use postio_ui::places::Entry;
use postio_ui::saved_search::SavedSearch;

use crate::cursor::{RowFacts, Rows};
use crate::dropdown::{self, Action, DropdownState, Landed, Latest, Offer, Shown, UnderstoodTile};
use crate::feed::Step;
use crate::{FocusController, Intent, Reply, Request, SurfaceKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BarMode {
    Search,
    Commands,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BarLineKind {
    Heading,
    Hint,
    Command,
    Correspondent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BarLine {
    pub kind: BarLineKind,
    /// Handed back by `BarRun`; never reused, so a redrawn line runs nothing.
    pub token: u64,
    pub title: String,
    pub detail: Option<String>,
    pub key: Option<String>,
    pub command: Option<CommandId>,
    pub selectable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BarView {
    pub echo: Option<String>,
    pub lines: Vec<BarLine>,
    /// `None` leaves it where it is, or on the first line that runs something.
    pub highlight: Option<u64>,
    pub saved: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PlacesRead {
    pub entries: Vec<Entry>,
    pub places: Vec<Place>,
    pub folders: Vec<(MailboxId, String)>,
    pub owners: Vec<(MailboxId, String)>,
    pub contacts: Vec<Contact>,
    /// From the folders' maintained totals (ADR 0040), not a count.
    pub messages: u64,
}

#[derive(Debug, Clone)]
struct Item {
    row: Row,
    line: BarLine,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Hit {
    message: MessageId,
    thread: Option<ThreadId>,
}

#[derive(Debug)]
pub(crate) struct Bar {
    mode: Option<BarMode>,
    typed: String,
    /// Moves whenever the words do: an answer under an older stamp is stale.
    stamp: u64,
    next_token: u64,
    base: Vec<Item>,
    chips: Vec<String>,
    pending: Option<MessageId>,
    aim: Option<MessageId>,
    held: Option<(String, MessageId)>,
    found: Vec<Hit>,
    saved: Vec<SavedSearch>,
    saving: Option<crate::SaveSearch>,
    keymap: Keymap,
    places: PlacesRead,
    names: postio_ui::names::Names,
    /// Moves with every read of the places: a popover token names its read.
    places_read: u64,
    places_shown: bool,
    places_asked: bool,
    /// Mail changed after the read on its way was asked for: it may miss the change, so another
    /// read follows.
    places_stale: bool,
    filtering: bool,
    clock: Option<chrono::DateTime<chrono::Local>>,
    drop: Option<Drop>,
    recents: Vec<(u64, postio_client::protocol::RecentSearch)>,
    saved_counts: Vec<SavedCount>,
    saved_tokens: Vec<u64>,
    sheet: Vec<u64>,
    highlighted: Option<u64>,
    /// Searched as typed, not lowered, while the field still begins with it.
    literal: Option<String>,
    /// An operator at its end is a chip made, not one being typed.
    settled: bool,
}

#[derive(Debug)]
struct Drop {
    state: DropdownState,
    query: Option<postio_search::ParsedQuery>,
    terms: Vec<String>,
    landed: Option<Landed>,
    field: Option<postio_search::query::Field>,
    value: String,
    piece: usize,
    offers: Vec<(u64, Offer)>,
    ghost: Option<String>,
    latest: Option<Latest>,
    latest_asked: Option<String>,
    tiles: Vec<UnderstoodTile>,
    /// The suggestions for `value` have landed: an empty `offers` is then nobody, not "not read
    /// yet".
    answered: bool,
    lowered: Option<String>,
    note: String,
}

impl Drop {
    fn new(state: DropdownState) -> Self {
        Drop {
            state,
            query: None,
            terms: Vec::new(),
            landed: None,
            field: None,
            value: String::new(),
            piece: 0,
            offers: Vec::new(),
            ghost: None,
            latest: None,
            latest_asked: None,
            tiles: Vec::new(),
            answered: false,
            lowered: None,
            note: String::new(),
        }
    }
}

impl Bar {
    pub(crate) fn new(platform: postio_config::paths::Platform) -> Self {
        Bar {
            mode: None,
            typed: String::new(),
            stamp: 0,
            next_token: 0,
            base: Vec::new(),
            chips: Vec::new(),
            pending: None,
            aim: None,
            held: None,
            found: Vec::new(),
            saved: Vec::new(),
            saving: None,
            // Resolving the keymap is quadratic in the commands: once per process.
            keymap: if platform == postio_config::paths::Platform::host() {
                Keymap::defaults().clone()
            } else {
                Keymap::resolve_on(&postio_config::KeyBindings::default(), platform)
            },
            places: PlacesRead::default(),
            names: postio_ui::names::Names::default(),
            places_read: 0,
            places_shown: false,
            places_asked: false,
            places_stale: false,
            filtering: false,
            clock: None,
            drop: None,
            recents: Vec::new(),
            saved_counts: Vec::new(),
            saved_tokens: Vec::new(),
            sheet: Vec::new(),
            highlighted: None,
            literal: None,
            settled: false,
        }
    }

    pub(crate) fn set_clock(&mut self, clock: Option<chrono::DateTime<chrono::Local>>) {
        self.clock = clock;
    }

    pub(crate) fn now(&self) -> chrono::DateTime<chrono::Local> {
        self.clock.unwrap_or_else(postio_ui::clock::now)
    }

    pub(crate) fn is_open(&self) -> bool {
        self.mode.is_some()
    }

    pub(crate) fn saved_len(&self) -> usize {
        self.saved.len()
    }

    pub(crate) fn keymap(&self) -> &Keymap {
        &self.keymap
    }

    pub(crate) fn folders(&self) -> &[(MailboxId, String)] {
        &self.places.folders
    }

    pub(crate) fn owners(&self) -> &[(MailboxId, String)] {
        &self.places.owners
    }

    fn wheres(&self, names: postio_search::results::FacetNames) -> dropdown::Wheres {
        let mut folders = names.folders;
        folders.extend(self.places.folders.iter().cloned());
        dropdown::Wheres {
            folders,
            held: names.held,
            owners: self.places.owners.clone(),
        }
    }

    pub(crate) fn name_for(&self, address: &str) -> Option<String> {
        self.places
            .contacts
            .iter()
            .find(|contact| {
                contact
                    .addresses
                    .iter()
                    .any(|owned| owned.address.address.eq_ignore_ascii_case(address))
            })
            .and_then(|contact| {
                let shown = contact.display_name();
                (!contact
                    .addresses
                    .iter()
                    .any(|owned| owned.address.address == shown))
                .then(|| shown.to_owned())
            })
    }

    pub(crate) fn search_words(&self) -> Option<&str> {
        let words = self.typed.trim();
        let searching = self.mode == Some(BarMode::Search) || self.drop.is_some();
        (searching && !words.is_empty()).then_some(words)
    }

    pub(crate) fn set_saving(&mut self, saving: crate::SaveSearch) {
        self.saving = Some(saving);
    }

    pub(crate) fn walk(&mut self, hits: Vec<(MessageId, Option<ThreadId>)>) {
        self.found = hits
            .into_iter()
            .map(|(message, thread)| Hit { message, thread })
            .collect();
        self.held = None;
    }

    pub(crate) fn set_keymap(&mut self, keymap: postio_core::Keymap) {
        self.keymap = keymap;
    }

    /// Filtered is listed only while filtering, so the popover reads the places again.
    pub(crate) fn set_filtering(&mut self, filtering: bool) -> Vec<Step> {
        if self.filtering == filtering {
            return Vec::new();
        }
        self.filtering = filtering;
        self.places_read += 1;
        if self.places.entries.is_empty() {
            return Vec::new();
        }
        vec![Step::Show(Intent::PlacesChanged)]
    }

    /// The filtered count can land after the places; once they have, a change in it is a change in
    /// them, so their tokens move on and the popover reads again.
    pub(crate) fn filtered_counted(&mut self, before: Option<u32>, now: u32) -> Vec<Step> {
        if !self.filtering || before == Some(now) || self.places.entries.is_empty() {
            return Vec::new();
        }
        self.places_read += 1;
        vec![Step::Show(Intent::PlacesChanged)]
    }

    fn open(&mut self, mode: BarMode, text: &str, aim: Option<MessageId>) -> Vec<Step> {
        self.mode = Some(mode);
        self.aim = aim;
        self.pending = None;
        self.typed = text.to_owned();
        self.settled = true;
        let mut steps = vec![
            Step::Show(Intent::OpenBar {
                mode,
                text: text.to_owned(),
                select: None,
            }),
            self.ask_places(),
        ];
        if mode == BarMode::Search {
            self.sheet = (0..=postio_ui::search_view::cheat_sheet().len())
                .map(|_| self.token())
                .collect();
            steps.push(Step::Ask(Request::RecentSearches));
            if !self.saved.is_empty() {
                steps.push(Step::Ask(Request::SavedCounts {
                    searches: keyed(self.saved.iter()),
                    today: self.now().date_naive(),
                }));
            }
        }
        steps.extend(self.answer());
        steps
    }

    /// An answer still on its way is for nothing; a hit opened from the bar keeps its words.
    pub(crate) fn close(&mut self) {
        self.mode = None;
        self.typed.clear();
        self.stamp += 1;
        self.base.clear();
        self.chips.clear();
        self.saving = None;
        self.drop = None;
        self.highlighted = None;
        self.literal = None;
    }

    fn typed(&mut self, text: String) -> Vec<Step> {
        if !self.is_open() || text == self.typed {
            return Vec::new();
        }
        // ⌫ on an operator with no value takes the colon; the operator goes whole.
        if let (Some(mode), Some(drop)) = (self.mode, &self.drop)
            && drop.state == DropdownState::Operator
            && drop.value.is_empty()
            && self.typed.ends_with(':')
            && text == self.typed[..self.typed.len() - 1]
        {
            let words = self.typed[..drop.piece].to_owned();
            return self.retype(mode, words);
        }
        self.typed = text;
        self.settled = false;
        self.answer()
    }

    fn answer(&mut self) -> Vec<Step> {
        self.stamp += 1;
        let typed = self.typed.trim().to_owned();
        if self.shows_dropdown(&typed) {
            return self.answer_dropdown(&typed);
        }
        self.drop = None;
        self.chips.clear();
        let lines = self.base_lines(&typed);
        self.base = self.items(lines);
        vec![self.draw(None)]
    }

    fn base_lines(&self, typed: &str) -> Vec<Line> {
        match rules::route(typed) {
            rules::Route::Correspondent(name) => {
                rules::correspondent_lines(&finder::contacts(&self.places.contacts, name))
            }
            rules::Route::Plain if typed.is_empty() => rules::empty_lines(),
            rules::Route::Plain => self.commands(typed),
            rules::Route::Folder(_) | rules::Route::Blend => Vec::new(),
        }
    }

    fn commands(&self, typed: &str) -> Vec<Line> {
        let state = availability();
        let mut blend = finder::blend(
            typed,
            &self.places.places,
            &self.keymap,
            Context::List,
            state,
        );
        let words = typed.strip_prefix(finder::COMMANDS_ONLY).unwrap_or(typed);
        rules::add_account_verbs(&mut blend.commands, &self.keymap, state, words.trim());
        rules::blend_lines(&blend)
    }

    /// Read as plain English, asked forgivingly ("tickt" finds the ticket). A saved search keeps
    /// the words, not this, so its rules stay exact (ADR 0037).
    pub(crate) fn lower(&self, typed: &str) -> postio_search::ParsedQuery {
        postio_search::natural::lower(typed, self.now().date_naive(), &|name| {
            self.names.lookup(name)
        })
        .forgiving()
        .contact_aware()
    }

    fn token(&mut self) -> u64 {
        self.next_token += 1;
        self.next_token
    }

    fn items(&mut self, lines: Vec<Line>) -> Vec<Item> {
        lines
            .into_iter()
            .filter_map(|line| self.item(line))
            .collect()
    }

    fn item(&mut self, line: Line) -> Option<Item> {
        let (kind, command) = match &line {
            Line::Heading(_) => (BarLineKind::Heading, None),
            Line::Row { row, .. } => match row {
                Row::Heading => (BarLineKind::Heading, None),
                Row::Hint => (BarLineKind::Hint, None),
                Row::Command(ActionId::Builtin(command)) => (BarLineKind::Command, Some(*command)),
                Row::Correspondent(_) => (BarLineKind::Correspondent, None),
                Row::Place(..)
                | Row::Search
                | Row::Instead(_)
                | Row::Order
                | Row::Message { .. } => {
                    return None;
                }
            },
        };
        let token = self.token();
        Some(match line {
            Line::Heading(title) => Item {
                row: Row::Heading,
                line: plain(kind, token, title),
            },
            Line::Row {
                row,
                title,
                detail,
                key,
            } => {
                let selectable = row.is_selectable();
                Item {
                    row,
                    line: BarLine {
                        detail,
                        key,
                        command,
                        selectable,
                        ..plain(kind, token, title)
                    },
                }
            }
        })
    }

    fn draw(&self, highlight: Option<u64>) -> Step {
        if self.drop.is_some() {
            return self.draw_dropdown(highlight);
        }
        Step::Show(Intent::BarLines(BarView {
            echo: self.echo(),
            lines: self.base.iter().map(|item| item.line.clone()).collect(),
            highlight,
            saved: self
                .saved
                .iter()
                .map(|search| search.name.clone())
                .collect(),
        }))
    }

    fn echo(&self) -> Option<String> {
        let typed = self.typed.trim();
        (!typed.is_empty()).then(|| rules::echo(typed))
    }

    fn row(&self, token: u64) -> Option<Row> {
        self.base
            .iter()
            .find(|item| item.line.token == token)
            .map(|item| item.row.clone())
    }

    fn places_landed(&mut self, read: Result<PlacesRead, String>) -> Vec<Step> {
        self.places_asked = false;
        let again = std::mem::take(&mut self.places_stale) && self.places_shown;
        let mut steps = self.places_read_landed(read);
        if again {
            steps.push(self.ask_places());
        }
        steps
    }

    fn ask_places(&mut self) -> Step {
        self.places_asked = true;
        Step::Ask(Request::Places)
    }

    /// Unless a read is already on its way (#1731).
    pub(crate) fn places_wanted(&mut self) -> Option<Step> {
        (!self.places_asked).then(|| self.ask_places())
    }

    /// The controller forgot its requests, so the read on its way will not land.
    pub(crate) fn places_forgotten(&mut self) {
        self.places_asked = false;
    }

    pub(crate) fn places_shown(&mut self, shown: bool) {
        self.places_shown = shown;
        if !shown {
            self.places_stale = false;
        }
    }

    /// A change during a read is followed by one more read, not one per change.
    pub(crate) fn places_heard(&mut self, event: &postio_core::Event) -> Vec<Step> {
        use postio_core::Event;
        let counts = matches!(
            event,
            Event::MessagesChanged { .. }
                | Event::MessagesRemoved { .. }
                | Event::NewMail { .. }
                | Event::MessageListChanged { .. }
                | Event::MailboxesChanged { .. }
        );
        if !counts || !self.places_shown {
            return Vec::new();
        }
        if self.places_asked {
            self.places_stale = true;
            return Vec::new();
        }
        vec![self.ask_places()]
    }

    fn places_read_landed(&mut self, read: Result<PlacesRead, String>) -> Vec<Step> {
        match read {
            Ok(read) => {
                self.names = postio_ui::names::Names::new(&read.contacts);
                self.places = read;
                self.places_read += 1;
            }
            Err(error) => {
                tracing::warn!(%error, "Focus could not read its places");
                return Vec::new();
            }
        }
        let mut steps = vec![Step::Show(Intent::PlacesChanged)];
        if !self.is_open() {
            return steps;
        }
        let typed = self.typed.trim().to_owned();
        let lines = self.base_lines(&typed);
        self.base = self.items(lines);
        steps.push(self.draw(None));
        steps
    }

    fn tab(&mut self) -> Vec<Step> {
        let (Some(mode), Some(drop)) = (self.mode, &self.drop) else {
            return Vec::new();
        };
        match drop.state {
            DropdownState::Prefix => match drop.offers.iter().find_map(|(_, offer)| match offer {
                Offer::Word(word) if drop.ghost.is_some() => Some(word.text.clone()),
                _ => None,
            }) {
                Some(word) => {
                    let text = format!("{}{word}", &self.typed[..drop.piece]);
                    self.retype(mode, text)
                }
                None => Vec::new(),
            },
            DropdownState::PlainEnglish => match drop.lowered.clone() {
                Some(text) => self.retype(mode, text),
                None => Vec::new(),
            },
            DropdownState::Words => {
                let first = drop
                    .landed
                    .as_ref()
                    .and_then(|landed| landed.pills.first())
                    .map(|(_, _, clause)| clause.clone());
                match first {
                    Some(clause) => self.narrow(mode, clause),
                    None => Vec::new(),
                }
            }
            _ => Vec::new(),
        }
    }

    fn back_to_words(&mut self) -> Vec<Step> {
        if !self
            .drop
            .as_ref()
            .is_some_and(|drop| drop.state == DropdownState::PlainEnglish)
        {
            return Vec::new();
        }
        self.literal = Some(self.typed.trim().to_owned());
        self.answer()
    }

    fn query(&self) -> String {
        if self.chips.is_empty() {
            self.typed.trim().to_owned()
        } else {
            self.chips.join(" ")
        }
    }

    fn listed(&self, filtered_today: Option<u32>, wanted: &str) -> Vec<Entry> {
        let filtered = filtered_today
            .filter(|_| self.filtering)
            .map(postio_ui::places::filtered_entry);
        postio_ui::places::listed(&self.places.entries, filtered.as_ref(), wanted)
    }
}

fn plain(kind: BarLineKind, token: u64, title: String) -> BarLine {
    BarLine {
        kind,
        token,
        title,
        detail: None,
        key: None,
        command: None,
        selectable: false,
    }
}

fn availability() -> postio_core::Availability {
    postio_core::Availability {
        scope: postio_core::Scope::Unified,
        store_open: true,
        frontend: Frontend::Focus,
    }
}

const BAR_KEYS: [CommandId; 5] = [
    CommandId::BackToWords,
    CommandId::SaveSearch,
    CommandId::ShowAllResults,
    CommandId::ForgetRecent,
    CommandId::ExcludeSuggestion,
];

const GOING: [CommandId; 17] = [
    CommandId::Search,
    CommandId::CommandPalette,
    CommandId::SavedSearch1,
    CommandId::SavedSearch2,
    CommandId::SavedSearch3,
    CommandId::SavedSearch4,
    CommandId::GoToInbox,
    CommandId::GoToDrafts,
    CommandId::GoToSent,
    CommandId::GoToArchive,
    CommandId::GoToSnoozed,
    CommandId::GoToFlagged,
    CommandId::GoToJunk,
    CommandId::GoToTrash,
    CommandId::GoToOutbox,
    CommandId::GoToFiltered,
    CommandId::GoToFolders,
];

pub(crate) fn goes(id: CommandId) -> bool {
    GOING.contains(&id)
}

pub(crate) fn bar_key(id: CommandId) -> bool {
    BAR_KEYS.contains(&id) || GOING.contains(&id)
}

impl FocusController {
    pub(crate) fn going(&mut self, id: CommandId, rows: &dyn Rows) -> Option<Vec<Step>> {
        let steps = match id {
            CommandId::Search => {
                let query = self.results_query().unwrap_or_default();
                self.open_bar(BarMode::Search, &query, rows)
            }
            CommandId::CommandPalette => {
                self.open_bar(BarMode::Commands, &finder::COMMANDS_ONLY.to_string(), rows)
            }
            CommandId::SavedSearch1
            | CommandId::SavedSearch2
            | CommandId::SavedSearch3
            | CommandId::SavedSearch4 => {
                let index = rules::SAVED.iter().position(|saved| *saved == id)?;
                match self.bar.saved.get(index).cloned() {
                    // Viewing a search that notifies is seeing it: its badge counts from now.
                    Some(search) => {
                        let mut steps = self.open_bar(BarMode::Search, &search.query, rows);
                        if search.notify {
                            steps.push(Step::Ask(Request::MarkSeen { key: search.key }));
                        }
                        steps
                    }
                    None => vec![Step::Show(Intent::Toast {
                        text: postio_ui::focus_target::no_saved_search(index),
                        kind: crate::ToastKind::Notice,
                    })],
                }
            }
            CommandId::SaveSearch => {
                if !self.bar.is_open() {
                    return Some(Vec::new());
                }
                let query = self.bar.query();
                if query.is_empty() {
                    return Some(Vec::new());
                }
                let save = crate::SaveSearch {
                    query,
                    name: None,
                    pin: true,
                    notify: false,
                    dates: postio_ui::saved_search::Dates::AsTyped,
                };
                self.bar.saving = Some(save.clone());
                vec![Step::Show(Intent::SaveSearch(save))]
            }
            CommandId::BackToWords => self.bar.back_to_words(),
            CommandId::ShowAllResults => self.show_all_results(),
            CommandId::ForgetRecent => self.bar.forget_highlighted(),
            CommandId::ExcludeSuggestion => self.bar.exclude_highlighted(),
            CommandId::GoToInbox => self.go_inbox(),
            CommandId::GoToDrafts => vec![Step::Ask(Request::RoleFolder(MailboxRole::Drafts))],
            CommandId::GoToSent => vec![Step::Ask(Request::RoleFolder(MailboxRole::Sent))],
            CommandId::GoToArchive => vec![Step::Ask(Request::RoleFolder(MailboxRole::Archive))],
            CommandId::GoToJunk => vec![Step::Ask(Request::RoleFolder(MailboxRole::Junk))],
            CommandId::GoToTrash => vec![Step::Ask(Request::RoleFolder(MailboxRole::Trash))],
            CommandId::GoToSnoozed => self.go_view(FocusScope::Snoozed, "Snoozed"),
            CommandId::GoToFlagged => self.go_view(FocusScope::Flagged, "Flagged"),
            CommandId::GoToOutbox => match self.cursor.accounts().first().copied() {
                Some(account) => self.go_to(
                    Destination::Outbox(account),
                    postio_ui::places::OUTBOX,
                    rows,
                ),
                None => Vec::new(),
            },
            CommandId::GoToFiltered => self.show_filtered(),
            CommandId::GoToFolders => {
                vec![Step::Show(Intent::OpenPlaces), self.bar.ask_places()]
            }
            _ => return None,
        };
        Some(steps)
    }

    pub(crate) fn open_bar(&mut self, mode: BarMode, text: &str, rows: &dyn Rows) -> Vec<Step> {
        let aim = self.cursor.row(rows).map(|row| row.id);
        let mut steps = self
            .surfaces
            .opened(SurfaceKind::Bar, self.policy.caps.stacking);
        steps.extend(self.bar.open(mode, text, aim));
        steps
    }

    fn leave_for(&mut self, name: &str) -> Vec<Step> {
        let mut steps = self.cursor.leave(self.feed.total());
        steps.push(Step::Show(Intent::SingleHeading(None)));
        steps.push(Step::Show(Intent::Place {
            name: name.to_owned(),
        }));
        steps
    }

    fn go_inbox(&mut self) -> Vec<Step> {
        let mut steps = self.leave_for("Inbox");
        steps.push(Step::Open(ListScope::Focus(FocusScope::Inbox)));
        steps
    }

    fn go_view(&mut self, scope: FocusScope, name: &str) -> Vec<Step> {
        let mut steps = self.leave_for(name);
        steps.push(Step::Open(ListScope::Focus(scope)));
        steps
    }

    /// A folder that is an inbox is Focus's; a label is its search, since Focus lists no label.
    fn go_to(&mut self, destination: Destination, name: &str, rows: &dyn Rows) -> Vec<Step> {
        match destination {
            Destination::Mailbox(mailbox) if self.feed.is_inbox(mailbox) => self.go_inbox(),
            Destination::Mailbox(mailbox) => {
                let mut steps = self.leave_for(name);
                steps.push(Step::Open(ListScope::Mailbox(mailbox)));
                steps
            }
            Destination::Label(_) => {
                self.open_bar(BarMode::Search, &format!("label:\"{name}\""), rows)
            }
            Destination::Search(query) => self.open_bar(BarMode::Search, &query, rows),
            Destination::Outbox(account) => {
                let mut steps = self.leave_for(name);
                steps.push(Step::Open(ListScope::Outbox(account)));
                steps
            }
        }
    }

    pub(crate) fn dismiss_bar(&mut self, opens: bool) -> Vec<Step> {
        self.bar.close();
        let mut steps = Vec::new();
        if self.surfaces.dismiss(SurfaceKind::Bar) {
            steps.push(Step::Show(Intent::CloseSurface(SurfaceKind::Bar)));
            if !opens && self.surfaces.top().is_none() {
                steps.push(Step::Show(Intent::KeyboardHome));
            }
        }
        steps
    }

    pub(crate) fn bar_run(&mut self, token: u64, rows: &dyn Rows) -> Vec<Step> {
        if !self.bar.is_open() {
            return Vec::new();
        }
        if let Some(action) = self.bar.action(token) {
            return self.drop_run(action, rows);
        }
        let Some(row) = self.bar.row(token) else {
            return Vec::new();
        };
        match row.run() {
            Run::Nothing | Run::Search | Run::ToggleOrder | Run::Action(BarAction::Open { .. }) => {
                Vec::new()
            }
            Run::SearchFor(text) => match self.bar.mode {
                Some(mode) => self.bar.retype(mode, text),
                None => Vec::new(),
            },
            Run::Action(BarAction::Go { destination, name }) => {
                let mut steps = self.dismiss_bar(false);
                steps.extend(self.go_to(destination, &name, rows));
                steps
            }
            Run::Action(BarAction::Command(id)) => {
                let aim = self.bar.aim;
                let mut steps = self.dismiss_bar(false);
                if let Some(aim) = aim
                    && self.cursor.row(rows).map(|row| row.id) != Some(aim)
                    && let Some(at) = rows.position_of(aim)
                {
                    steps.extend(self.cursor.place(at, rows));
                }
                if self.answers(id) {
                    steps.extend(self.list_steps(id, rows));
                } else {
                    steps.push(Step::Show(Intent::Run(id)));
                }
                steps
            }
        }
    }

    fn open_hit(&mut self, message: MessageId) -> Vec<Step> {
        let found: Vec<Hit> = self
            .bar
            .drop
            .iter()
            .flat_map(|drop| &drop.landed)
            .flat_map(|landed| &landed.hits)
            .map(|shown| Hit {
                message: shown.hit.best,
                thread: shown.thread(),
            })
            .collect();
        let typed = self.bar.typed.clone();
        let mut steps = self.dismiss_bar(true);
        self.bar.found = found;
        self.bar.held = Some((typed, message));
        steps.extend(self.show_hit(message));
        steps
    }

    pub(crate) fn show_hit(&mut self, message: MessageId) -> Vec<Step> {
        let Some(index) = self.bar.found.iter().position(|hit| hit.message == message) else {
            return Vec::new();
        };
        let hit = self.bar.found[index].clone();
        if let Some((_, at)) = self.bar.held.as_mut() {
            *at = message;
        }
        let row = RowFacts {
            id: hit.message,
            digest: false,
            threads: hit.thread.into_iter().collect(),
            writes: false,
        };
        let total = self.bar.found.len() as u32;
        self.surfaces
            .open(&row, index as u32, total, self.policy.caps.stacking)
    }

    pub(crate) fn step_hit(&mut self, reading: MessageId, by: i32) -> Option<Vec<Step>> {
        let at = self
            .bar
            .found
            .iter()
            .position(|hit| hit.message == reading)?;
        let next = at as i64 + i64::from(by);
        let Some(hit) = usize::try_from(next)
            .ok()
            .and_then(|next| self.bar.found.get(next))
        else {
            return Some(Vec::new());
        };
        let message = hit.message;
        Some(self.show_hit(message))
    }

    pub(crate) fn hit_facts(&self, reading: MessageId) -> Option<RowFacts> {
        let hit = self.bar.found.iter().find(|hit| hit.message == reading)?;
        Some(RowFacts {
            id: hit.message,
            digest: false,
            threads: hit.thread.into_iter().collect(),
            writes: false,
        })
    }

    /// Its place is the hits', even when the list has it too.
    pub fn among_hits(&self, message: MessageId) -> bool {
        self.bar.found.iter().any(|hit| hit.message == message)
    }

    pub(crate) fn hit_closed(&mut self, rows: &dyn Rows) -> Vec<Step> {
        self.bar.found.clear();
        let Some((typed, message)) = self.bar.held.take() else {
            return Vec::new();
        };
        // A composer or another surface took the message's window, not the reader leaving: no
        // dropdown may open over it.
        if self.surfaces.top().is_some() {
            return Vec::new();
        }
        let steps = self.open_bar(BarMode::Search, &typed, rows);
        self.bar.pending = Some(message);
        steps
    }

    pub(crate) fn open_place(&mut self, token: u64, rows: &dyn Rows) -> Vec<Step> {
        if token >> 32 != self.bar.places_read {
            return Vec::new();
        }
        let index = (token & u64::from(u32::MAX)) as usize;
        let filtered_today = self.counts.map(|counts| counts.filtered_today);
        let Some(entry) = self.bar.listed(filtered_today, "").into_iter().nth(index) else {
            return Vec::new();
        };
        match entry.command {
            Some(command) => self.going(command, rows).unwrap_or_default(),
            None => self.go_to(entry.destination, &entry.name, rows),
        }
    }

    pub(crate) fn listed_places(&self, filter: &str) -> Vec<(u64, Entry)> {
        let filtered_today = self.counts.map(|counts| counts.filtered_today);
        let all = self.bar.listed(filtered_today, "");
        self.bar
            .listed(filtered_today, filter)
            .into_iter()
            .filter_map(|mut entry| {
                let index = all.iter().position(|each| *each == entry)?;
                if let Destination::Mailbox(mailbox) = entry.destination {
                    entry.failure = self.failure(mailbox).map(str::to_owned);
                }
                Some(((self.bar.places_read << 32) | index as u64, entry))
            })
            .collect()
    }

    pub fn folder_names(&self) -> &[(MailboxId, String)] {
        &self.bar.places.folders
    }

    pub(crate) fn bar_reply(&mut self, reply: Reply, rows: &dyn Rows) -> Vec<Step> {
        match reply {
            Reply::Places(read) => {
                let mut steps = self.bar.places_landed(read);
                steps.extend(self.relabel());
                steps
            }
            Reply::Conversations { stamp, answer } => self.bar.conversations(stamp, answer),
            Reply::Passages { stamp, answer } => self.bar.passages(stamp, answer),
            Reply::Suggest { stamp, answer } => self.bar.suggested(stamp, answer),
            Reply::LatestFrom {
                stamp,
                address,
                answer,
            } => self.bar.latest_landed(stamp, &address, answer),
            Reply::RecentSearches(answer) => self.bar.recents_read(answer),
            Reply::SavedCounts(answer) => self.bar.saved_counted(answer),
            Reply::RoleFolder(Some((mailbox, name))) => {
                self.go_to(Destination::Mailbox(mailbox), &name, rows)
            }
            _ => Vec::new(),
        }
    }

    pub(crate) fn bar_input(&mut self, input: crate::Input, rows: &dyn Rows) -> Vec<Step> {
        use crate::Input;
        match input {
            Input::Typed { text } => self.bar.typed(text),
            Input::BarRun(token) => self.bar_run(token, rows),
            Input::BarTab => self.bar.tab(),
            Input::SearchHighlighted(token) => self.bar.highlight(token),
            Input::SearchForget(token) => self.bar.forget(token),
            Input::SearchExclude(token) => self.bar.exclude(token),
            Input::OpenPlace(token) => self.open_place(token, rows),
            Input::PlacesShown(shown) => {
                self.bar.places_shown(shown);
                Vec::new()
            }
            Input::SavedSearches(saved) => {
                self.bar.set_saved(saved);
                self.redraw_bar()
            }
            Input::SearchSaved(result) => {
                let saving = self.bar.saving.take();
                match result {
                    Ok(saved) => {
                        self.bar.set_saved(saved.searches);
                        let said = saving
                            .as_ref()
                            .map(|save| save.name.as_deref().unwrap_or(&save.query))
                            .unwrap_or_default();
                        let mut steps = vec![Step::Show(Intent::Toast {
                            text: postio_ui::focus_target::search_saved(said),
                            kind: crate::ToastKind::Notice,
                        })];
                        // A search that notifies starts seen: its badge counts what arrives after
                        // it was saved.
                        if let (Some(key), true) =
                            (saved.key, saving.as_ref().is_some_and(|save| save.notify))
                        {
                            steps.push(Step::Ask(Request::MarkSeen { key }));
                        }
                        steps.extend(self.redraw_bar());
                        steps
                    }
                    Err(sentence) => vec![Step::Show(Intent::Toast {
                        text: sentence,
                        kind: crate::ToastKind::Notice,
                    })],
                }
            }
            Input::Filtering(filtering) => self.bar.set_filtering(filtering),
            _ => Vec::new(),
        }
    }

    pub(crate) fn redraw_bar(&self) -> Vec<Step> {
        if self.bar.is_open() {
            vec![self.bar.draw(None)]
        } else {
            Vec::new()
        }
    }
}

impl Bar {
    fn shows_dropdown(&self, typed: &str) -> bool {
        match rules::route(typed) {
            rules::Route::Blend | rules::Route::Folder(_) => true,
            rules::Route::Plain => typed.is_empty() && self.mode == Some(BarMode::Search),
            rules::Route::Correspondent(_) => false,
        }
    }

    fn set_saved(&mut self, saved: Vec<SavedSearch>) {
        self.saved = saved;
        self.saved_tokens = (0..self.saved.len()).map(|_| self.token()).collect();
    }

    fn answer_dropdown(&mut self, typed: &str) -> Vec<Step> {
        if typed.is_empty() {
            self.chips.clear();
            self.literal = None;
            self.drop = Some(Drop::new(DropdownState::Empty));
            return vec![self.draw(None)];
        }
        if self
            .literal
            .as_deref()
            .is_some_and(|kept| !typed.starts_with(kept))
        {
            self.literal = None;
        }
        let raw = self.typed.trim_start().to_owned();
        let offset = self.typed.len() - raw.len();
        if let Some((field, value, piece)) = operator_typed(&raw).filter(|_| !self.settled) {
            return self.answer_operator(field, value, offset + piece);
        }
        if self.literal.is_none() && is_prefix(typed) {
            return self.answer_prefix(typed, offset);
        }
        let lowered =
            postio_search::natural::lower_with_origins(typed, self.now().date_naive(), &|name| {
                self.names.lookup(name)
            });
        if self.literal.is_none() && understood(&lowered) {
            return self.answer_plain(lowered);
        }
        let parsed = match self.literal {
            Some(_) => postio_search::parse(typed, self.now().date_naive()),
            None => self.lower(typed),
        };
        self.answer_words(parsed)
    }

    /// Kept on screen until the next answer lands, rather than blinking empty.
    fn kept(&mut self, state: DropdownState) -> Option<Drop> {
        self.drop.take().filter(|drop| drop.state == state)
    }

    fn answer_words(&mut self, parsed: postio_search::ParsedQuery) -> Vec<Step> {
        self.chips = rules::chips(&parsed).unwrap_or_default();
        let searchable = parsed.is_searchable();
        let kept = self
            .kept(DropdownState::Words)
            .filter(|_| searchable)
            .and_then(|drop| drop.landed);
        let was_words = kept.is_some();
        self.drop = Some(Drop {
            terms: postio_search::highlight::terms(&parsed),
            query: Some(parsed.clone()),
            landed: kept,
            ..Drop::new(DropdownState::Words)
        });
        let mut steps = Vec::new();
        if !was_words {
            steps.push(self.draw(None));
        }
        if searchable {
            steps.push(Step::Ask(Request::Conversations {
                query: parsed,
                order: postio_search::results::ConversationOrder::BestMatch,
                limit: dropdown::TOP_HITS,
                stamp: self.stamp,
            }));
        }
        steps
    }

    fn answer_prefix(&mut self, typed: &str, piece: usize) -> Vec<Step> {
        // Half a word, not English: "at" is no stop word here.
        let parsed = postio_search::parse(typed, self.now().date_naive())
            .forgiving()
            .contact_aware();
        self.chips.clear();
        let kept = self.kept(DropdownState::Prefix);
        let (landed, offers, ghost) = match kept {
            Some(drop) => (drop.landed, drop.offers, drop.ghost),
            None => (None, Vec::new(), None),
        };
        // A ghost is only drawn while the word still begins with what is typed.
        let ghost = ghost.and(offers.iter().find_map(|(_, offer)| match offer {
            Offer::Word(word) => postio_search::suggest::ghost(typed, &word.text),
            _ => None,
        }));
        self.drop = Some(Drop {
            terms: vec![typed.to_owned()],
            query: Some(parsed.clone()),
            landed,
            value: typed.to_owned(),
            piece,
            offers,
            ghost,
            ..Drop::new(DropdownState::Prefix)
        });
        let mut steps = vec![
            self.draw(None),
            Step::Ask(Request::Suggest {
                prefix: typed.to_owned(),
                field: None,
                stamp: self.stamp,
            }),
        ];
        if parsed.is_searchable() {
            steps.push(Step::Ask(Request::Conversations {
                query: parsed,
                order: postio_search::results::ConversationOrder::BestMatch,
                limit: dropdown::HITS_SO_FAR,
                stamp: self.stamp,
            }));
        }
        steps
    }

    fn answer_operator(
        &mut self,
        field: postio_search::query::Field,
        value: String,
        piece: usize,
    ) -> Vec<Step> {
        let typed = self.typed.trim().to_owned();
        self.chips = rules::chips(&self.lower(&typed)).unwrap_or_default();
        let kept = self
            .kept(DropdownState::Operator)
            .filter(|drop| drop.field == Some(field));
        let (offers, latest, latest_asked) = match kept {
            Some(drop) => (drop.offers, drop.latest, drop.latest_asked),
            None => (Vec::new(), None, None),
        };
        self.drop = Some(Drop {
            field: Some(field),
            value: value.clone(),
            piece,
            offers,
            latest,
            latest_asked,
            ..Drop::new(DropdownState::Operator)
        });
        vec![
            self.draw(None),
            Step::Ask(Request::Suggest {
                prefix: value,
                field: Some(field),
                stamp: self.stamp,
            }),
        ]
    }

    fn answer_plain(&mut self, lowered: postio_search::natural::Lowered) -> Vec<Step> {
        let query = lowered.query.clone().forgiving().contact_aware();
        self.chips = rules::chips(&query).unwrap_or_default();
        let tiles = lowered
            .origins
            .iter()
            .filter_map(|origin| {
                let token = lowered.query.tokens().get(origin.token)?;
                Some(self.tile(token, &origin.words))
            })
            .collect();
        let chips_text = lowered
            .query
            .tokens()
            .iter()
            .map(|token| token.raw.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        let (after, before) = query
            .filters()
            .fold((None, None), |(after, before), clause| {
                match &clause.filter {
                    postio_search::query::Filter::After(date) if !clause.negated => {
                        (Some(*date), before)
                    }
                    postio_search::query::Filter::Before(date) if !clause.negated => {
                        (after, Some(*date))
                    }
                    _ => (after, before),
                }
            });
        let landed = self
            .kept(DropdownState::PlainEnglish)
            .and_then(|drop| drop.landed);
        // The words are marked, not the person: the sender column says who.
        let terms = query
            .text_terms()
            .filter(|term| !term.negated)
            .map(|term| term.value.clone())
            .collect();
        self.drop = Some(Drop {
            terms,
            query: Some(query.clone()),
            landed,
            tiles,
            lowered: Some(chips_text),
            note: postio_ui::search_view::plain_results_note(after, before),
            ..Drop::new(DropdownState::PlainEnglish)
        });
        let mut steps = vec![self.draw(None)];
        if query.is_searchable() {
            steps.push(Step::Ask(Request::Conversations {
                query,
                order: postio_search::results::ConversationOrder::Newest,
                limit: dropdown::PLAIN_RESULTS,
                stamp: self.stamp,
            }));
        }
        steps
    }

    fn tile(&self, token: &postio_search::query::Token, words: &str) -> UnderstoodTile {
        let (op, value) = match (token.field(), token.raw.split_once(':')) {
            (Some(field), Some((op, value))) => {
                let value = match field {
                    postio_search::query::Field::From | postio_search::query::Field::To => {
                        self.name_for(value).unwrap_or_else(|| value.to_owned())
                    }
                    _ => value.to_owned(),
                };
                (format!("{op}:"), value)
            }
            _ => (String::new(), token.raw.clone()),
        };
        UnderstoodTile {
            op,
            value,
            origin: postio_ui::search_view::origin_line(words),
        }
    }

    fn draw_dropdown(&self, select: Option<u64>) -> Step {
        let Some(drop) = &self.drop else {
            return Step::Show(Intent::BarLines(BarView::default()));
        };
        let now = self.now();
        let mut view = match drop.state {
            DropdownState::Words => dropdown::words(
                drop.landed.as_ref(),
                &drop.terms,
                drop.query.as_ref(),
                &self.keymap,
                now,
            ),
            DropdownState::Prefix => dropdown::prefix(dropdown::PrefixParts {
                typed: &drop.value,
                ghost: drop.ghost.clone(),
                offers: &drop.offers,
                landed: drop.landed.as_ref(),
                keymap: &self.keymap,
                now,
            }),
            DropdownState::Operator => dropdown::operator(dropdown::OperatorParts {
                keyword: drop.field.map_or("from", |field| field.keyword()),
                value: &drop.value,
                offers: &drop.offers,
                latest: drop.latest.as_ref(),
                answered: drop.answered,
                keymap: &self.keymap,
                now,
            }),
            DropdownState::PlainEnglish => dropdown::plain(dropdown::PlainParts {
                tiles: drop.tiles.clone(),
                landed: drop.landed.as_ref(),
                terms: &drop.terms,
                note: drop.note.clone(),
                keymap: &self.keymap,
                now,
            }),
            _ => {
                let saved: Vec<dropdown::SavedPill> = self
                    .saved
                    .iter()
                    .zip(&self.saved_tokens)
                    .map(|(search, token)| {
                        let counted = self
                            .saved_counts
                            .iter()
                            .find(|counted| counted.key == search.key);
                        dropdown::SavedPill {
                            token: *token,
                            name: search.name.clone(),
                            count: counted.map(|counted| counted.total),
                            fresh: counted
                                .filter(|_| search.notify)
                                .map(|counted| counted.new)
                                .filter(|new| new.conversations > 0),
                        }
                    })
                    .collect();
                dropdown::empty(dropdown::EmptyParts {
                    recents: &self.recents,
                    saved: &saved,
                    sheet: &self.sheet,
                    example: self.example(),
                    messages: (self.places_read > 0).then_some(self.places.messages),
                    keymap: &self.keymap,
                    now,
                })
            }
        };
        view.select = select;
        Step::Show(Intent::Dropdown(view))
    }

    fn example(&self) -> String {
        let lowered = postio_search::natural::lower_with_origins(
            postio_ui::search_view::EXAMPLE,
            self.now().date_naive(),
            &|name| self.names.lookup(name),
        );
        lowered
            .query
            .tokens()
            .iter()
            .map(|token| token.raw.as_str())
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn action(&self, token: u64) -> Option<Action> {
        let drop = self.drop.as_ref()?;
        if let Some((_, offer)) = drop.offers.iter().find(|(at, _)| *at == token) {
            return offer.action(drop.field);
        }
        if let Some(shown) = drop
            .latest
            .iter()
            .flat_map(|latest| &latest.hits)
            .find(|shown| shown.token == token)
        {
            return Some(Action::Hit(shown.hit.best));
        }
        match drop.state {
            DropdownState::Words | DropdownState::Prefix | DropdownState::PlainEnglish => {
                let landed = drop.landed.as_ref()?;
                if landed.show_all == token {
                    return Some(Action::ShowAll);
                }
                if let Some(shown) = landed.hits.iter().find(|shown| shown.token == token) {
                    return Some(Action::Hit(shown.hit.best));
                }
                landed
                    .pills
                    .iter()
                    .find(|(at, _, _)| *at == token)
                    .map(|(_, _, clause)| Action::Narrow(clause.clone()))
            }
            _ => {
                if let Some((_, recent)) = self.recents.iter().find(|(at, _)| *at == token) {
                    return Some(Action::Recent(recent.query.clone()));
                }
                self.saved_tokens
                    .iter()
                    .position(|at| *at == token)
                    .map(Action::Saved)
            }
        }
    }

    fn retype(&mut self, mode: BarMode, text: String) -> Vec<Step> {
        let mut steps = vec![Step::Show(Intent::OpenBar {
            mode,
            text: text.clone(),
            select: None,
        })];
        self.typed = text;
        self.settled = true;
        steps.extend(self.answer());
        steps
    }

    fn narrow(&mut self, mode: BarMode, clause: postio_search::query::Clause) -> Vec<Step> {
        let text = postio_search::edit::apply(
            self.typed.trim(),
            postio_search::edit::Edit::Add(clause),
            self.now().date_naive(),
        );
        self.retype(mode, text)
    }

    fn conversations(
        &mut self,
        stamp: u64,
        answer: Result<Box<postio_search::results::ConversationResults>, String>,
    ) -> Vec<Step> {
        if !self.is_open() || stamp != self.stamp {
            return Vec::new();
        }
        let results = match answer {
            Ok(results) => *results,
            Err(error) => {
                tracing::debug!(%error, "the dropdown's search found nothing to show");
                return Vec::new();
            }
        };
        let pills = dropdown::narrow_pills(&results);
        let hits: Vec<Shown> = results
            .hits
            .into_iter()
            .map(|hit| Shown {
                token: self.token(),
                hit,
            })
            .collect();
        let pills = pills
            .into_iter()
            .map(|(pill, clause)| (self.token(), pill, clause))
            .collect();
        let show_all = self.token();
        let wheres = self.wheres(results.names);
        let landed = Landed {
            hits,
            pills,
            show_all,
            total: results.total,
            capped: results.capped,
            elapsed: results.elapsed,
            wheres,
        };
        let Some(drop) = self.drop.as_mut() else {
            return Vec::new();
        };
        let asked = landed
            .hits
            .iter()
            .map(|shown| {
                let sources = shown
                    .hit
                    .matches
                    .iter()
                    .map(|each| each.source.clone())
                    .collect();
                (shown.hit.best, sources)
            })
            .collect::<Vec<_>>();
        drop.landed = Some(landed);
        let query = drop.query.clone();
        let mut steps = Vec::new();
        let select = self
            .pending
            .take()
            .and_then(|message| {
                self.drop
                    .as_ref()?
                    .landed
                    .as_ref()?
                    .hits
                    .iter()
                    .find(|shown| shown.hit.best == message)
            })
            .map(|shown| shown.token);
        steps.push(self.draw(select));
        if let Some(query) = query
            && !asked.is_empty()
        {
            steps.push(Step::Ask(Request::Passages {
                query,
                hits: asked,
                stamp,
            }));
        }
        steps
    }

    fn passages(
        &mut self,
        stamp: u64,
        answer: Result<Vec<(MessageId, Vec<postio_search::results::Match>)>, String>,
    ) -> Vec<Step> {
        if !self.is_open() || stamp != self.stamp {
            return Vec::new();
        }
        let found = match answer {
            Ok(found) => found,
            Err(error) => {
                tracing::debug!(%error, "the dropdown's passages could not be read");
                return Vec::new();
            }
        };
        let Some(landed) = self.drop.as_mut().and_then(|drop| drop.landed.as_mut()) else {
            return Vec::new();
        };
        for (message, matches) in found {
            if let Some(shown) = landed
                .hits
                .iter_mut()
                .find(|shown| shown.hit.best == message)
            {
                shown.hit.matches = matches;
            }
        }
        vec![self.draw(None)]
    }

    fn suggested(
        &mut self,
        stamp: u64,
        answer: Result<Box<postio_search::suggest::Suggestions>, String>,
    ) -> Vec<Step> {
        if !self.is_open() || stamp != self.stamp {
            return Vec::new();
        }
        let found = match answer {
            Ok(found) => *found,
            Err(error) => {
                tracing::debug!(%error, "the dropdown's suggestions could not be read");
                return Vec::new();
            }
        };
        let Some(state) = self.drop.as_ref().map(|drop| drop.state) else {
            return Vec::new();
        };
        if !matches!(state, DropdownState::Prefix | DropdownState::Operator) {
            return Vec::new();
        }
        let kept = self
            .drop
            .as_mut()
            .map(|drop| std::mem::take(&mut drop.offers))
            .unwrap_or_default();
        let offers: Vec<(u64, Offer)> = dropdown::offers(&found)
            .into_iter()
            .map(|offer| {
                let token = kept
                    .iter()
                    .find(|(_, was)| same_offer(was, &offer))
                    .map(|(token, _)| *token)
                    .unwrap_or_else(|| self.token());
                (token, offer)
            })
            .collect();
        let first_person = offers.iter().find_map(|(_, offer)| match offer {
            Offer::Person(person) => Some(person.address.clone()),
            _ => None,
        });
        let Some(drop) = self.drop.as_mut() else {
            return Vec::new();
        };
        let typed = drop.value.clone();
        drop.ghost = found
            .ghost
            .and_then(|_| {
                offers.iter().find_map(|(_, offer)| match offer {
                    Offer::Word(word) => postio_search::suggest::ghost(&typed, &word.text),
                    _ => None,
                })
            })
            .filter(|_| state == DropdownState::Prefix);
        drop.offers = offers;
        drop.answered = true;
        if first_person.is_none() && state == DropdownState::Operator {
            drop.latest = None;
            drop.latest_asked = None;
        }
        let mut steps = vec![self.draw(None)];
        // The arrows rest where they were if that row is still drawn, else on the first person.
        let focused = self
            .highlighted
            .and_then(|token| self.person(token))
            .or(first_person);
        if let Some(address) = focused {
            steps.extend(self.ask_latest(address));
        }
        steps
    }

    fn person(&self, token: u64) -> Option<String> {
        self.drop
            .as_ref()?
            .offers
            .iter()
            .find_map(|(at, offer)| match offer {
                Offer::Person(person) if *at == token => Some(person.address.clone()),
                _ => None,
            })
    }

    fn ask_latest(&mut self, address: String) -> Option<Step> {
        let drop = self.drop.as_mut()?;
        if drop.latest_asked.as_deref() == Some(address.as_str()) {
            return None;
        }
        drop.latest_asked = Some(address.clone());
        Some(Step::Ask(Request::LatestFrom {
            address,
            stamp: self.stamp,
        }))
    }

    fn highlight(&mut self, token: u64) -> Vec<Step> {
        self.highlighted = Some(token);
        match self.person(token) {
            Some(address) => self.ask_latest(address).into_iter().collect(),
            None => Vec::new(),
        }
    }

    fn latest_landed(
        &mut self,
        stamp: u64,
        address: &str,
        answer: Result<Box<postio_search::results::ConversationResults>, String>,
    ) -> Vec<Step> {
        if !self.is_open() || stamp != self.stamp {
            return Vec::new();
        }
        let results = match answer {
            Ok(results) => *results,
            Err(error) => {
                tracing::debug!(%error, "the latest from a person could not be read");
                return Vec::new();
            }
        };
        let name = self
            .drop
            .as_ref()
            .and_then(|drop| {
                drop.offers.iter().find_map(|(_, offer)| match offer {
                    Offer::Person(person) if person.address == address => {
                        Some(person.name.clone().unwrap_or_else(|| address.to_owned()))
                    }
                    _ => None,
                })
            })
            .unwrap_or_else(|| address.to_owned());
        let hits: Vec<Shown> = results
            .hits
            .into_iter()
            .map(|hit| Shown {
                token: self.token(),
                hit,
            })
            .collect();
        let wheres = self.wheres(results.names);
        let Some(drop) = self.drop.as_mut() else {
            return Vec::new();
        };
        if drop.state != DropdownState::Operator || drop.latest_asked.as_deref() != Some(address) {
            return Vec::new();
        }
        drop.latest = Some(Latest { name, hits, wheres });
        vec![self.draw(None)]
    }

    pub(crate) fn exclude(&mut self, token: u64) -> Vec<Step> {
        let (Some(mode), Some(drop)) = (self.mode, self.drop.as_ref()) else {
            return Vec::new();
        };
        let Some(filter) = drop
            .offers
            .iter()
            .find(|(at, _)| *at == token)
            .and_then(|(_, offer)| offer.excluded(drop.field))
        else {
            return Vec::new();
        };
        self.pick(mode, filter, true)
    }

    fn exclude_highlighted(&mut self) -> Vec<Step> {
        let Some(drop) = self.drop.as_ref() else {
            return Vec::new();
        };
        let token = self
            .highlighted
            .filter(|token| drop.offers.iter().any(|(at, _)| at == token))
            .or_else(|| drop.offers.first().map(|(token, _)| *token));
        match token {
            Some(token) => self.exclude(token),
            None => Vec::new(),
        }
    }

    fn pick(
        &mut self,
        mode: BarMode,
        filter: postio_search::query::Filter,
        negated: bool,
    ) -> Vec<Step> {
        let piece = self.drop.as_ref().map_or(0, |drop| drop.piece);
        let chip = postio_search::query::spell(&postio_search::query::Clause { negated, filter });
        let text = format!("{}{chip} ", &self.typed[..piece.min(self.typed.len())]);
        self.retype(mode, text)
    }

    fn replace(&mut self, mode: BarMode, text: &str) -> Vec<Step> {
        let piece = self.drop.as_ref().map_or(0, |drop| drop.piece);
        let text = format!("{}{text}", &self.typed[..piece.min(self.typed.len())]);
        self.retype(mode, text)
    }

    fn recents_read(
        &mut self,
        answer: Result<Vec<postio_client::protocol::RecentSearch>, String>,
    ) -> Vec<Step> {
        let recents = match answer {
            Ok(recents) => recents,
            Err(error) => {
                tracing::debug!(%error, "the recent searches could not be read");
                return Vec::new();
            }
        };
        // A query still listed keeps its token, so the arrows stay on it.
        let kept = std::mem::take(&mut self.recents);
        self.recents = recents
            .into_iter()
            .map(|recent| {
                let token = kept
                    .iter()
                    .find(|(_, was)| was.query == recent.query)
                    .map(|(token, _)| *token)
                    .unwrap_or_else(|| self.token());
                (token, recent)
            })
            .collect();
        self.redraw_empty(None)
    }

    /// Only searches that notify are recounted; the others keep their counts.
    pub(crate) fn heard(&self, event: &postio_core::Event) -> Vec<Step> {
        if !matches!(event, postio_core::Event::NewMail { .. }) || !self.is_open() {
            return Vec::new();
        }
        let searches = keyed(self.saved.iter().filter(|search| search.notify));
        if searches.is_empty() {
            return Vec::new();
        }
        vec![Step::Ask(Request::SavedCounts {
            searches,
            today: self.now().date_naive(),
        })]
    }

    fn saved_counted(&mut self, answer: Result<Vec<SavedCount>, String>) -> Vec<Step> {
        match answer {
            // Merged by key: a recount of notifying searches leaves the others' counts.
            Ok(counts) => {
                for counted in counts {
                    match self
                        .saved_counts
                        .iter_mut()
                        .find(|was| was.key == counted.key)
                    {
                        Some(was) => *was = counted,
                        None => self.saved_counts.push(counted),
                    }
                }
                self.redraw_empty(None)
            }
            Err(error) => {
                tracing::debug!(%error, "the saved searches could not be counted");
                Vec::new()
            }
        }
    }

    fn redraw_empty(&self, select: Option<u64>) -> Vec<Step> {
        match &self.drop {
            Some(drop) if self.is_open() && drop.state == DropdownState::Empty => {
                vec![self.draw(select)]
            }
            _ => Vec::new(),
        }
    }

    fn forget(&mut self, token: u64) -> Vec<Step> {
        let Some(at) = self.recents.iter().position(|(at, _)| *at == token) else {
            return Vec::new();
        };
        let (_, recent) = self.recents.remove(at);
        // The arrows stay on the row that moved up, or the one above when it was last.
        let next = self
            .recents
            .get(at)
            .or_else(|| at.checked_sub(1).and_then(|at| self.recents.get(at)))
            .map(|(token, _)| *token);
        self.highlighted = next;
        let mut steps = self.redraw_empty(next);
        steps.push(Step::Ask(Request::ForgetSearch {
            query: recent.query,
        }));
        steps
    }

    fn forget_highlighted(&mut self) -> Vec<Step> {
        if !matches!(&self.drop, Some(drop) if drop.state == DropdownState::Empty) {
            return Vec::new();
        }
        let token = self
            .highlighted
            .filter(|token| self.recents.iter().any(|(at, _)| at == token))
            .or_else(|| self.recents.first().map(|(token, _)| *token));
        match token {
            Some(token) => self.forget(token),
            None => Vec::new(),
        }
    }

    pub(crate) fn remember(&self) -> Option<Step> {
        let landed = self.drop.as_ref()?.landed.as_ref()?;
        let query = self.typed.trim();
        (!query.is_empty()).then(|| {
            Step::Ask(Request::RememberSearch {
                query: query.to_owned(),
                hits: landed.total,
                capped: landed.capped,
            })
        })
    }
}

impl FocusController {
    fn drop_run(&mut self, action: Action, rows: &dyn Rows) -> Vec<Step> {
        let Some(mode) = self.bar.mode else {
            return Vec::new();
        };
        match action {
            Action::Recent(query) => self.bar.retype(mode, query),
            Action::Hit(message) => {
                let mut steps: Vec<Step> = self.bar.remember().into_iter().collect();
                steps.extend(self.open_hit(message));
                steps
            }
            Action::ShowAll => self.show_all_results(),
            Action::Narrow(clause) => self.bar.narrow(mode, clause),
            Action::Replace(text) => self.bar.replace(mode, &text),
            Action::Pick(filter) => self.bar.pick(mode, filter, false),
            Action::Saved(index) => rules::SAVED
                .get(index)
                .and_then(|id| self.going(*id, rows))
                .unwrap_or_default(),
        }
    }
}

fn keyed<'a>(searches: impl Iterator<Item = &'a SavedSearch>) -> Vec<(String, String)> {
    searches
        .map(|search| (search.key.clone(), search.query.clone()))
        .collect()
}

fn same_offer(a: &Offer, b: &Offer) -> bool {
    match (a, b) {
        (Offer::Word(a), Offer::Word(b))
        | (Offer::Label(a), Offer::Label(b))
        | (Offer::List(a), Offer::List(b))
        | (Offer::Folder(a), Offer::Folder(b)) => a.text == b.text,
        (Offer::Files(_), Offer::Files(_)) => true,
        (Offer::Person(a), Offer::Person(b)) => a.address == b.address,
        _ => false,
    }
}

/// `from:`, `to:`, `label:` and `in:`; a space after the value says it is done.
fn operator_typed(typed: &str) -> Option<(postio_search::query::Field, String, usize)> {
    use postio_search::query::Field;
    if typed.is_empty() || typed.ends_with(char::is_whitespace) {
        return None;
    }
    let piece = typed.rfind(char::is_whitespace).map_or(0, |at| {
        at + typed[at..].chars().next().map_or(1, char::len_utf8)
    });
    let last = &typed[piece..];
    let (keyword, value) = last.strip_prefix('-').unwrap_or(last).split_once(':')?;
    let field = match keyword.to_ascii_lowercase().as_str() {
        "from" => Field::From,
        "to" => Field::To,
        "label" => Field::Label,
        "in" => Field::In,
        _ => return None,
    };
    Some((field, value.trim_start_matches('"').to_owned(), piece))
}

fn is_prefix(typed: &str) -> bool {
    let count = typed.chars().count();
    (1..=3).contains(&count) && typed.chars().all(char::is_alphanumeric)
}

/// Operators typed as themselves have no English origin and are only a query.
fn understood(lowered: &postio_search::natural::Lowered) -> bool {
    lowered.origins.iter().any(|origin| {
        origin.from.is_some()
            && lowered
                .query
                .tokens()
                .get(origin.token)
                .is_some_and(|token| token.field().is_some())
    })
}
