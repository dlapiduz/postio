//! The command bar, search, go-to and the places (research R2, slice 8).
//!
//! Moved from `postio-gtk`'s window and bar (`go_to`, `go_to_inbox`,
//! `go_to_role`, `go_to_view`, the saved searches, `bar_action`,
//! reopen-after-hit and the chip editor) and from its places popover (the
//! places read), so the Mac's bar and popover behave as GTK's do. What a
//! bar's words mean, the lines they offer and what each line says are
//! `postio_ui::command_bar`'s and `postio_ui::finder`'s; this keeps the
//! bar's state -- what is typed, which lines are on screen and what each
//! runs, which results are current -- and turns a line run into what the
//! list does. The toolkit draws the lines it is given and hands back the
//! token of the one run.
//!
//! `/` and `mod+k` open one bar two ways (C24). A command run from it acts
//! on what the cursor was on when the bar opened. A place run from it is
//! opened as the list, through the feed's own opening, so the strip, the
//! cursor on the first row and the paging all follow. The places -- every
//! account's mailboxes, folders and labels, counted -- are one request,
//! however many accounts there are.

use chrono::{DateTime, Utc};
use postio_core::{ActionId, CommandId, Context, Frontend, Keymap};
use postio_model::{Contact, FocusScope, ListScope, MailboxId, MailboxRole, MessageId, ThreadId};
use postio_search::{Instead, ResultOrder, SearchHit};
use postio_ui::command_bar::{self as rules, BarAction, Line, Row, Run};
use postio_ui::finder::{self, Destination, Place};
use postio_ui::places::Entry;

use crate::cursor::{RowFacts, Rows};
use crate::dropdown::{self, Action, DropdownState, Landed, Shown};
use crate::feed::Step;
use crate::{FocusController, Intent, Reply, Request, SurfaceKind};

/// How the bar was opened: `/` for mail, `mod+k` for commands (C24).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BarMode {
    /// Search, places and commands, blended.
    Search,
    /// Commands only: opened with `>` typed.
    Commands,
}

/// What kind of line the bar draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BarLineKind {
    /// A section heading; runs nothing.
    Heading,
    /// A line of the empty bar saying what typing does; runs nothing.
    Hint,
    /// A command, with its key.
    Command,
    /// A place to go: a mailbox, a folder or a label.
    Place,
    /// "Search mail for …".
    Search,
    /// "Search instead for “word”".
    Instead,
    /// The order the results are in.
    Order,
    /// A correspondent `@` offered.
    Correspondent,
    /// A message: a search hit, or a folder's conversation.
    Message,
}

/// One line of the bar, with everything it draws.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BarLine {
    /// What kind of line.
    pub kind: BarLineKind,
    /// What [`Input::BarRun`](crate::Input::BarRun) hands back to run it:
    /// never reused, so a line since redrawn runs nothing.
    pub token: u64,
    /// Its title: a command's name, a place's `in:`, a hit's subject.
    pub title: String,
    /// Its dimmer detail, or a hit's first line.
    pub detail: Option<String>,
    /// The key that does the same, as the keymap spells it.
    pub key: Option<String>,
    /// The command a keycap is drawn for, when it is one's.
    pub command: Option<CommandId>,
    /// Whether the arrows may rest on it.
    pub selectable: bool,
    /// A message's sender.
    pub sender: Option<String>,
    /// Where a message is: its folder, and its account when there are
    /// several, each a line.
    pub wheres: Vec<String>,
    /// A message's time column.
    pub time: Option<String>,
}

/// The bar, whole: what [`Intent::BarLines`] draws.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BarView {
    /// The heading over a folder's conversations (`in:`).
    pub heading: Option<String>,
    /// The words echoed under the field, and while a chip is edited, which.
    pub echo: Option<String>,
    /// The chips the words were lowered to.
    pub chips: Vec<String>,
    /// The chip being edited, after `Tab`.
    pub editing: Option<u32>,
    /// The lines, top to bottom.
    pub lines: Vec<BarLine>,
    /// Where the highlight goes, by token; `None` leaves it where it is, or
    /// on the first line that runs something.
    pub highlight: Option<u64>,
    /// The pinned saved searches' names, in order: the first four run on
    /// `SavedSearch1`-`4`.
    pub saved: Vec<String>,
}

/// One conversation of a folder, as the bar lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoundRow {
    /// The message it opens.
    pub message: MessageId,
    /// Who it is from.
    pub from: Option<String>,
    /// Its subject.
    pub subject: String,
    /// Its first line.
    pub preview: Option<String>,
    /// When.
    pub at: DateTime<Utc>,
}

/// What a search found.
#[derive(Debug, Clone, PartialEq)]
pub struct Found {
    /// The hits, best first.
    pub hits: Vec<SearchHit>,
    /// The word the hits are for, when not the one typed (ADR 0037).
    pub instead: Option<Instead>,
    /// Which hits a digest holds: each message's rule, and whether it has
    /// been delivered.
    pub held: Vec<(MessageId, String, bool)>,
}

/// Every place there is to go, read in one request.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PlacesRead {
    /// The folders popover's entries, counted: every account's mailboxes,
    /// folders and labels, its Outbox while anything waits, and Snoozed and
    /// Flagged.
    pub entries: Vec<Entry>,
    /// The bar's places: every mailbox, folder and label.
    pub places: Vec<Place>,
    /// Every folder's name, for a hit's `in:`.
    pub folders: Vec<(MailboxId, String)>,
    /// Which account each folder is, when there are several to tell apart.
    pub owners: Vec<(MailboxId, String)>,
    /// Every correspondent, for `@` and for a name typed.
    pub contacts: Vec<Contact>,
}

/// Where the highlight goes once results are drawn.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum Pending {
    /// Where it is.
    #[default]
    Nowhere,
    /// The first hit: Return on the search row asked to be among them.
    FirstHit,
    /// The order row, which was just run.
    Order,
    /// The hit last read, which has just been closed.
    Hit(MessageId),
}

/// One line on screen and what running it does.
#[derive(Debug, Clone)]
struct Item {
    row: Row,
    line: BarLine,
}

/// A message a hit opened, which `j` and `k` walk among the others.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Hit {
    message: MessageId,
    thread: Option<ThreadId>,
}

/// The bar's state, and the places it and the popover read.
#[derive(Debug)]
pub(crate) struct Bar {
    /// How it was opened, while it is up.
    mode: Option<BarMode>,
    /// The words, as the field holds them.
    typed: String,
    /// Which order a search's hits come in: relevance each time the bar
    /// opens, kept while it is up.
    order: ResultOrder,
    /// Moves whenever the words do: an answer under an older one is for
    /// words no longer there.
    stamp: u64,
    next_token: u64,
    /// The lines the words make: the blend, `@`'s correspondents, a
    /// folder's conversations, or the empty bar's hints.
    base: Vec<Item>,
    /// A search's lines, drawn under the search row.
    results: Vec<Item>,
    /// The stamp the results are for.
    loaded: Option<u64>,
    /// A folder's heading, over its conversations (`in:`).
    heading: Option<String>,
    /// The folder `in:` completed to, while its conversations are read.
    folder: Option<String>,
    chips: Vec<String>,
    /// The words typed, while `Tab` has stepped into their chips.
    words: Option<String>,
    editing: Option<usize>,
    pending: Pending,
    /// The cursor's message when the bar opened: what a command run from it
    /// acts on.
    aim: Option<MessageId>,
    /// The words when a hit was opened, and the hit on screen since: the
    /// bar comes back on them when it closes.
    held: Option<(String, MessageId)>,
    /// The hits the message opened from the bar walks with `j` and `k`.
    found: Vec<Hit>,
    /// The hits the results list now, with their conversations.
    walked: Vec<Hit>,
    saved: Vec<(String, String)>,
    /// The query an `Intent::SaveSearch` is saving.
    saving: Option<String>,
    keymap: Keymap,
    /// The places, as last read.
    places: PlacesRead,
    names: postio_ui::names::Names,
    /// Moves with every read of the places: a popover token names its read.
    places_read: u64,
    /// Whether Focus files mail away: the popover lists Filtered then.
    filtering: bool,
    /// The search half is the dropdown (spec 010 step 2), not the blend.
    results_view: bool,
    /// The clock, when stopped: what the dropdown's dates count from.
    clock: Option<chrono::DateTime<chrono::Local>>,
    /// The dropdown, while it is what the bar shows.
    drop: Option<Drop>,
    /// The searches run lately, newest first, each with its token.
    recents: Vec<(u64, postio_client::protocol::RecentSearch)>,
    /// The saved searches' counts, by name, as last read.
    saved_counts: Vec<(String, u64)>,
    /// The saved pills' tokens, in order.
    saved_tokens: Vec<u64>,
    /// The cheat sheet's rows' tokens, and the example's.
    sheet: Vec<u64>,
    /// The row the toolkit's arrows rest on.
    highlighted: Option<u64>,
}

/// The dropdown's state while the bar shows it.
#[derive(Debug)]
struct Drop {
    state: DropdownState,
    /// The query the words were lowered to.
    query: Option<postio_search::ParsedQuery>,
    /// The query's words, for the highlight.
    terms: Vec<String>,
    /// What the words found, once it has landed.
    landed: Option<Landed>,
}

impl Bar {
    pub(crate) fn new(platform: postio_config::paths::Platform, results_view: bool) -> Self {
        Bar {
            mode: None,
            typed: String::new(),
            order: ResultOrder::default(),
            stamp: 0,
            next_token: 0,
            base: Vec::new(),
            results: Vec::new(),
            loaded: None,
            heading: None,
            folder: None,
            chips: Vec::new(),
            words: None,
            editing: None,
            pending: Pending::Nowhere,
            aim: None,
            held: None,
            found: Vec::new(),
            walked: Vec::new(),
            saved: Vec::new(),
            saving: None,
            // The registry's own keys until the frontend says which are in
            // force; resolved once per process where it can be, since
            // resolving is quadratic in the commands.
            keymap: if platform == postio_config::paths::Platform::host() {
                Keymap::defaults().clone()
            } else {
                Keymap::resolve_on(&postio_config::KeyBindings::default(), platform)
            },
            places: PlacesRead::default(),
            names: postio_ui::names::Names::default(),
            places_read: 0,
            filtering: false,
            results_view,
            clock: None,
            drop: None,
            recents: Vec::new(),
            saved_counts: Vec::new(),
            saved_tokens: Vec::new(),
            sheet: Vec::new(),
            highlighted: None,
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

    /// The bindings in force, as the bar's keycaps say them.
    pub(crate) fn keymap(&self) -> &Keymap {
        &self.keymap
    }

    /// Every folder the places know, by name.
    pub(crate) fn folders(&self) -> &[(MailboxId, String)] {
        &self.places.folders
    }

    /// The name a correspondent is known by, for an address.
    pub(crate) fn name_for(&self, address: &str) -> Option<String> {
        self.places
            .contacts
            .iter()
            .find(|contact| contact.address.address.eq_ignore_ascii_case(address))
            .and_then(|contact| {
                contact
                    .name
                    .clone()
                    .or_else(|| contact.address.name.clone())
            })
    }

    /// The words in the field, trimmed, while the bar searches: what ⌘↩
    /// shows the results of.
    pub(crate) fn search_words(&self) -> Option<&str> {
        let words = self.typed.trim();
        (self.mode == Some(BarMode::Search) && !words.is_empty()).then_some(words)
    }

    /// The query an `Intent::SaveSearch` is saving.
    pub(crate) fn set_saving(&mut self, query: String) {
        self.saving = Some(query);
    }

    /// Walk `hits` from a message one of them opened, as a hit opened from
    /// the bar walks the bar's: nothing to come back to when it closes.
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

    pub(crate) fn set_filtering(&mut self, filtering: bool) {
        if self.filtering != filtering {
            self.filtering = filtering;
            self.places_read += 1;
        }
    }

    /// Open the bar `mode`'s way on `text`, over the cursor's `aim`, and
    /// read the places it can go.
    fn open(&mut self, mode: BarMode, text: &str, aim: Option<MessageId>) -> Vec<Step> {
        self.mode = Some(mode);
        self.aim = aim;
        self.words = None;
        self.editing = None;
        self.order = ResultOrder::default();
        self.pending = Pending::Nowhere;
        self.typed = text.to_owned();
        let mut steps = vec![
            Step::Show(Intent::OpenBar {
                mode,
                text: text.to_owned(),
                select: None,
            }),
            Step::Ask(Request::Places),
        ];
        if self.results_view && mode == BarMode::Search {
            self.sheet = (0..=postio_ui::search_view::cheat_sheet().len())
                .map(|_| self.token())
                .collect();
            steps.push(Step::Ask(Request::RecentSearches));
            if !self.saved.is_empty() {
                steps.push(Step::Ask(Request::SavedCounts {
                    searches: self.saved.clone(),
                    today: self.now().date_naive(),
                }));
            }
        }
        steps.extend(self.answer());
        steps
    }

    /// The bar closed: nothing it showed is kept, and an answer still on
    /// its way is for nothing. A hit opened from it keeps its words, to
    /// come back on.
    pub(crate) fn close(&mut self) {
        self.mode = None;
        self.typed.clear();
        self.stamp += 1;
        self.base.clear();
        self.results.clear();
        self.loaded = None;
        self.heading = None;
        self.folder = None;
        self.chips.clear();
        self.words = None;
        self.editing = None;
        self.saving = None;
        self.drop = None;
        self.highlighted = None;
    }

    /// The field's words now.
    fn typed(&mut self, text: String) -> Vec<Step> {
        if !self.is_open() || text == self.typed {
            return Vec::new();
        }
        self.typed = text;
        self.answer()
    }

    /// Answer what is typed: a folder's conversations after `in:`, `@`'s
    /// correspondents, and otherwise the blend, with a search when the words
    /// are one.
    fn answer(&mut self) -> Vec<Step> {
        self.stamp += 1;
        self.results.clear();
        self.loaded = None;
        self.heading = None;
        self.folder = None;
        let typed = self.typed.trim().to_owned();
        if self.shows_dropdown(&typed) {
            return self.answer_dropdown(&typed);
        }
        self.drop = None;
        let lines = self.base_lines(&typed);
        self.base = self.items(lines);
        self.chips.clear();
        let mut asks = Vec::new();
        match rules::route(&typed) {
            rules::Route::Folder(name) => {
                if let Some((name, mailbox)) = rules::folder_for(&self.places.places, name) {
                    self.folder = Some(name);
                    asks.push(Step::Ask(Request::Folder {
                        mailbox,
                        stamp: self.stamp,
                    }));
                }
            }
            // Words that name what they want are a search, shown as its
            // chips (screen 07); a plain word makes none. Either way the
            // results come as it is typed, under the search row.
            rules::Route::Blend => {
                let parsed = self.lower(&typed);
                self.chips = rules::chips(&parsed).unwrap_or_default();
                // A single letter is not yet a search: it is in most of a
                // mailbox, and asking for it on the first keystroke held
                // every later one behind it (`ParsedQuery::searchable_terms`).
                if parsed.is_searchable() {
                    asks.push(Step::Ask(Request::Search {
                        query: parsed,
                        order: self.order,
                        stamp: self.stamp,
                    }));
                } else {
                    self.results.clear();
                }
            }
            rules::Route::Plain | rules::Route::Correspondent(_) => {}
        }
        let mut steps = vec![self.draw(None)];
        steps.extend(asks);
        steps
    }

    /// The lines `typed` makes above any results: the blend, with the empty
    /// bar's hints, or `@`'s correspondents. A folder's (`in:`) are its
    /// conversations, which come once they are read.
    fn base_lines(&self, typed: &str) -> Vec<Line> {
        match rules::route(typed) {
            rules::Route::Folder(_) => Vec::new(),
            rules::Route::Correspondent(name) => {
                rules::correspondent_lines(&finder::contacts(&self.places.contacts, name))
            }
            rules::Route::Plain if typed.is_empty() => {
                let mut lines = self.blend(typed);
                lines.extend(rules::empty_lines());
                lines
            }
            rules::Route::Plain | rules::Route::Blend => self.blend(typed),
        }
    }

    /// The commands and places `typed` matches, and the search row.
    fn blend(&self, typed: &str) -> Vec<Line> {
        let state = availability();
        let mut blend = finder::blend(
            typed,
            &self.places.places,
            &self.keymap,
            Context::List,
            state,
        );
        let words = typed.strip_prefix(finder::COMMANDS_ONLY).unwrap_or(typed);
        if typed.starts_with(finder::COMMANDS_ONLY) || !words.trim().is_empty() {
            rules::add_account_verbs(&mut blend.commands, &self.keymap, state, words.trim());
        }
        rules::blend_lines(&blend)
    }

    /// `typed`, read as plain English against today and the address book,
    /// and asked the forgiving way: a person searching wants "tickt" to
    /// find the ticket. A saved search keeps the words, not this, so the
    /// rules made from them stay exact (ADR 0037, as amended).
    pub(crate) fn lower(&self, typed: &str) -> postio_search::ParsedQuery {
        postio_search::natural::lower(typed, postio_ui::clock::now().date_naive(), &|name| {
            self.names.lookup(name)
        })
        .forgiving()
    }

    fn token(&mut self) -> u64 {
        self.next_token += 1;
        self.next_token
    }

    /// `lines` as items, each with a token of its own.
    fn items(&mut self, lines: Vec<Line>) -> Vec<Item> {
        lines.into_iter().map(|line| self.item(line)).collect()
    }

    fn item(&mut self, line: Line) -> Item {
        let token = self.token();
        match line {
            Line::Heading(title) => Item {
                row: Row::Heading,
                line: plain(BarLineKind::Heading, token, title),
            },
            Line::Row {
                row,
                title,
                detail,
                key,
            } => {
                let (kind, command) = match &row {
                    Row::Heading => (BarLineKind::Heading, None),
                    Row::Hint => (BarLineKind::Hint, None),
                    Row::Command(ActionId::Builtin(command)) => {
                        (BarLineKind::Command, Some(*command))
                    }
                    Row::Command(ActionId::Ext(_)) => (BarLineKind::Command, None),
                    Row::Place(destination, _) => (
                        BarLineKind::Place,
                        self.places
                            .places
                            .iter()
                            .find(|place| place.destination == *destination)
                            .and_then(|place| match place.go {
                                Some(ActionId::Builtin(command)) => Some(command),
                                _ => None,
                            }),
                    ),
                    Row::Search => (BarLineKind::Search, None),
                    Row::Instead(_) => (BarLineKind::Instead, None),
                    Row::Order => (BarLineKind::Order, Some(CommandId::ToggleResultOrder)),
                    Row::Correspondent(_) => (BarLineKind::Correspondent, None),
                    Row::Message { .. } => (BarLineKind::Message, None),
                };
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
        }
    }

    /// A message's line: a hit, or a folder's conversation.
    fn message(&mut self, message: MessageId, row: MessageLine) -> Item {
        let token = self.token();
        Item {
            row: Row::Message {
                message,
                subject: row.subject.clone(),
            },
            line: BarLine {
                detail: row.preview,
                selectable: true,
                sender: row.from,
                wheres: row.wheres,
                time: Some(postio_ui::row::timestamp(row.at, postio_ui::clock::now())),
                ..plain(BarLineKind::Message, token, row.subject)
            },
        }
    }

    /// The bar, as it is now, with the highlight on `highlight`.
    fn draw(&self, highlight: Option<u64>) -> Step {
        if self.drop.is_some() {
            return self.draw_dropdown(highlight);
        }
        let after_search = self
            .base
            .iter()
            .position(|item| item.row == Row::Search)
            .map_or(self.base.len(), |at| at + 1);
        let lines = self.base[..after_search]
            .iter()
            .chain(&self.results)
            .chain(&self.base[after_search..])
            .map(|item| item.line.clone())
            .collect();
        Step::Show(Intent::BarLines(BarView {
            heading: self.heading.clone(),
            echo: self.echo(),
            chips: self.chips.clone(),
            editing: self.editing.map(|at| at as u32),
            lines,
            highlight,
            saved: self.saved.iter().map(|(name, _)| name.clone()).collect(),
        }))
    }

    /// What the bar echoes under the field: the words, and while a chip is
    /// edited, which, with the keys that move on and go back.
    fn echo(&self) -> Option<String> {
        if let (Some(at), Some(words)) = (self.editing, &self.words)
            && let Some(chip) = self.chips.get(at.min(self.chips.len().saturating_sub(1)))
        {
            let editing = match chip.split_once(':') {
                Some((operator, _)) => format!("{operator}:"),
                None => chip.clone(),
            };
            let mut hints = vec![postio_ui::hints::fixed(
                "Tab",
                "next chip",
                "Tab moves between the bar's chips: the toolkit's focus order, not a command",
            )];
            hints.extend(postio_ui::hints::hint(
                &self.keymap,
                CommandId::BackToWords,
                "back to plain words",
            ));
            return Some(format!(
                "You typed \u{201c}{words}\u{201d} \u{b7} editing {editing} \u{b7} {}",
                postio_ui::hints::line(&hints)
            ));
        }
        let typed = self.typed.trim();
        (!typed.is_empty()).then(|| rules::echo(typed))
    }

    /// The line `token` names, among those on screen.
    fn row(&self, token: u64) -> Option<Row> {
        self.base
            .iter()
            .chain(&self.results)
            .find(|item| item.line.token == token)
            .map(|item| item.row.clone())
    }

    /// Search for what is typed now, under the words' own stamp.
    fn search_again(&mut self) -> Vec<Step> {
        let typed = self.typed.trim().to_owned();
        if typed.is_empty() || rules::route(&typed) != rules::Route::Blend {
            return Vec::new();
        }
        let query = self.lower(&typed);
        if !query.is_searchable() {
            return Vec::new();
        }
        vec![Step::Ask(Request::Search {
            query,
            order: self.order,
            stamp: self.stamp,
        })]
    }

    /// Where the highlight goes now that the results are drawn.
    fn take_pending(&mut self) -> Option<u64> {
        let wanted = std::mem::take(&mut self.pending);
        self.base
            .iter()
            .chain(&self.results)
            .find(|item| match wanted {
                Pending::Nowhere => false,
                Pending::FirstHit => matches!(item.row, Row::Message { .. }),
                Pending::Order => item.row == Row::Order,
                Pending::Hit(message) => {
                    matches!(&item.row, Row::Message { message: at, .. } if *at == message)
                }
            })
            .map(|item| item.line.token)
    }

    /// A search's answer, drawn under the search row when it is for the
    /// words still typed.
    fn found(&mut self, stamp: u64, answer: Result<Found, String>) -> Vec<Step> {
        if !self.is_open() || stamp != self.stamp {
            return Vec::new();
        }
        let found = match answer {
            Ok(found) => found,
            Err(error) => {
                tracing::debug!(%error, "the bar's search found nothing to show");
                return Vec::new();
            }
        };
        let hits = rules::conversations(found.hits);
        let mut results = Vec::new();
        let heading = rules::results_heading(hits.len());
        results.push(self.item(Line::Heading(heading)));
        if let Some(instead) = &found.instead {
            let title = rules::search_title(&instead.typed, Some(&instead.term));
            if let Some(search) = self.base.iter_mut().find(|item| item.row == Row::Search) {
                search.line.title = title;
            }
            results.push(self.item(Line::Heading(rules::showing_results_for(&instead.term))));
            let (title, detail) = rules::search_instead(&instead.typed);
            results.push(self.item(Line::Row {
                row: Row::Instead(instead.typed.clone()),
                title,
                detail: Some(detail.to_owned()),
                key: None,
            }));
        }
        if !hits.is_empty() {
            let (title, detail) = rules::order_words(self.order);
            let key = postio_ui::hints::key(&self.keymap, CommandId::ToggleResultOrder);
            results.push(self.item(Line::Row {
                row: Row::Order,
                title,
                detail: Some(detail),
                key,
            }));
        }
        let mut walked = Vec::new();
        for hit in hits {
            let place = rules::result_place(&hit, &found.held, &self.places.folders);
            let account = rules::result_account(&hit, &self.places.owners);
            walked.push(Hit {
                message: hit.message_id,
                thread: hit.thread_id,
            });
            results.push(self.message(
                hit.message_id,
                MessageLine {
                    from: hit.from.as_ref().map(rules::said_of),
                    subject: hit.subject.clone().unwrap_or_default(),
                    preview: hit.preview.clone(),
                    wheres: place.into_iter().chain(account).collect(),
                    at: hit.received_at,
                },
            ));
        }
        self.results = results;
        self.walked = walked;
        self.loaded = Some(stamp);
        let highlight = self.take_pending();
        vec![self.draw(highlight)]
    }

    /// A folder's conversations, under its heading (screen 08).
    fn folder(
        &mut self,
        stamp: u64,
        count: Result<u32, String>,
        rows: Result<Vec<FoundRow>, String>,
    ) -> Vec<Step> {
        if !self.is_open() || stamp != self.stamp {
            return Vec::new();
        }
        let Some(name) = self.folder.clone() else {
            return Vec::new();
        };
        self.heading = Some(rules::folder_heading(&name, count.unwrap_or(0)));
        let rows = rows.unwrap_or_else(|error| {
            tracing::warn!(%error, "the bar could not list a folder");
            Vec::new()
        });
        self.base = rows
            .into_iter()
            .map(|row| {
                self.message(
                    row.message,
                    MessageLine {
                        from: row.from,
                        subject: row.subject,
                        preview: row.preview,
                        wheres: Vec::new(),
                        at: row.at,
                    },
                )
            })
            .collect();
        vec![self.draw(None)]
    }

    /// The places, read again: what the bar completes and blends with, and
    /// what the popover lists.
    fn places_landed(&mut self, read: Result<PlacesRead, String>) -> Vec<Step> {
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
        // What was typed before the places landed is answered again, now
        // that there are folders to complete and places to blend; a search
        // already answered is not asked again.
        let typed = self.typed.trim().to_owned();
        if matches!(rules::route(&typed), rules::Route::Folder(_)) {
            steps.extend(self.answer());
        } else {
            let lines = self.base_lines(&typed);
            self.base = self.items(lines);
            steps.push(self.draw(None));
        }
        steps
    }

    /// `Tab`: from the words into the first chip, then on to the next.
    fn tab(&mut self) -> Vec<Step> {
        let Some(mode) = self.mode else {
            return Vec::new();
        };
        if let Some(drop) = &self.drop {
            let first = drop
                .landed
                .as_ref()
                .and_then(|landed| landed.pills.first())
                .map(|(_, _, clause)| clause.clone());
            return match first {
                Some(clause) => self.narrow(mode, clause),
                None => Vec::new(),
            };
        }
        if self.chips.is_empty() {
            return Vec::new();
        }
        let next = match self.editing {
            None => {
                self.words = Some(self.typed.clone());
                0
            }
            Some(at) => (at + 1) % self.chips.len(),
        };
        self.editing = Some(next);
        let query = self.chips.join(" ");
        let start: usize = self.chips[..next]
            .iter()
            .map(|chip| chip.chars().count() + 1)
            .sum();
        let end = start + self.chips[next].chars().count();
        let mut steps = vec![Step::Show(Intent::OpenBar {
            mode,
            text: query.clone(),
            select: Some((start as u32, end as u32)),
        })];
        if self.typed == query {
            steps.push(self.draw(None));
        } else {
            self.typed = query;
            steps.extend(self.answer());
        }
        steps
    }

    /// `mod+BackSpace`: back from the chips to the words they came from.
    fn back_to_words(&mut self) -> Vec<Step> {
        let (Some(mode), Some(words)) = (self.mode, self.words.take()) else {
            return Vec::new();
        };
        self.editing = None;
        let mut steps = vec![Step::Show(Intent::OpenBar {
            mode,
            text: words.clone(),
            select: None,
        })];
        self.typed = words;
        steps.extend(self.answer());
        steps
    }

    /// Switch the results between relevance and date, and ask again.
    fn toggle_order(&mut self) -> Vec<Step> {
        if !self.is_open() {
            return Vec::new();
        }
        self.order = match self.order {
            ResultOrder::Relevance => ResultOrder::Newest,
            ResultOrder::Newest => ResultOrder::Relevance,
        };
        self.pending = Pending::Order;
        self.search_again()
    }

    /// The query a saved search keeps: the chips, or the words when they
    /// make none.
    fn query(&self) -> String {
        if self.chips.is_empty() {
            self.typed.trim().to_owned()
        } else {
            self.chips.join(" ")
        }
    }

    /// The popover's places: every entry, the views, and Filtered while
    /// Focus files mail away, `filtered_today` being what it filed today.
    fn listed(&self, filtered_today: Option<u32>, wanted: &str) -> Vec<Entry> {
        let filtered = filtered_today
            .filter(|_| self.filtering)
            .map(postio_ui::places::filtered_entry);
        postio_ui::places::listed(&self.places.entries, filtered.as_ref(), wanted)
    }
}

/// A message line's words.
struct MessageLine {
    from: Option<String>,
    subject: String,
    preview: Option<String>,
    wheres: Vec<String>,
    at: DateTime<Utc>,
}

/// A line with only a title.
fn plain(kind: BarLineKind, token: u64, title: String) -> BarLine {
    BarLine {
        kind,
        token,
        title,
        detail: None,
        key: None,
        command: None,
        selectable: false,
        sender: None,
        wheres: Vec::new(),
        time: None,
    }
}

/// What the bar's commands are offered under: Focus's, over every
/// account's mail, as GTK's bar asks.
fn availability() -> postio_core::Availability {
    postio_core::Availability {
        scope: postio_core::Scope::Unified,
        store_open: true,
        frontend: Frontend::Focus,
    }
}

/// The commands the bar answers while it is up, besides Back.
const BAR_KEYS: [CommandId; 3] = [
    CommandId::BackToWords,
    CommandId::ToggleResultOrder,
    CommandId::SaveSearch,
];

/// The commands that open the bar or go somewhere: the controller's on the
/// list, and while the bar is up.
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

/// Whether `id` opens the bar or goes somewhere.
pub(crate) fn goes(id: CommandId) -> bool {
    GOING.contains(&id)
}

/// The commands the dropdown answers while it is up (spec 010 step 2).
const DROPDOWN_KEYS: [CommandId; 2] = [CommandId::ShowAllResults, CommandId::ForgetRecent];

/// Whether the bar, up, answers `id` itself; `results_view` when its search
/// half is the dropdown.
pub(crate) fn bar_key(id: CommandId, results_view: bool) -> bool {
    BAR_KEYS.contains(&id) || GOING.contains(&id) || (results_view && DROPDOWN_KEYS.contains(&id))
}

impl FocusController {
    /// A command that opens the bar, runs one of its keys, or goes
    /// somewhere; `None` when `id` is none of those.
    pub(crate) fn going(&mut self, id: CommandId, rows: &dyn Rows) -> Option<Vec<Step>> {
        let steps = match id {
            // Over the results, on their query (FR-021).
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
                match self.bar.saved.get(index).map(|(_, query)| query.clone()) {
                    Some(query) => self.open_bar(BarMode::Search, &query, rows),
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
                self.bar.saving = Some(query.clone());
                vec![Step::Show(Intent::SaveSearch { query })]
            }
            CommandId::BackToWords => self.bar.back_to_words(),
            CommandId::ShowAllResults if self.bar.results_view => self.show_all_results(),
            CommandId::ForgetRecent if self.bar.results_view => self.bar.forget_highlighted(),
            CommandId::ToggleResultOrder => self.bar.toggle_order(),
            CommandId::GoToInbox => self.go_inbox(),
            CommandId::GoToDrafts => vec![Step::Ask(Request::RoleFolder(MailboxRole::Drafts))],
            CommandId::GoToSent => vec![Step::Ask(Request::RoleFolder(MailboxRole::Sent))],
            CommandId::GoToArchive => vec![Step::Ask(Request::RoleFolder(MailboxRole::Archive))],
            CommandId::GoToJunk => vec![Step::Ask(Request::RoleFolder(MailboxRole::Junk))],
            CommandId::GoToTrash => vec![Step::Ask(Request::RoleFolder(MailboxRole::Trash))],
            CommandId::GoToSnoozed => self.go_view(FocusScope::Snoozed, "Snoozed"),
            CommandId::GoToFlagged => self.go_view(FocusScope::Flagged, "Flagged"),
            // A view over Drafts, of the first account, as the popover's row
            // for it is of the account it lists.
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
                vec![Step::Show(Intent::OpenPlaces), Step::Ask(Request::Places)]
            }
            _ => return None,
        };
        Some(steps)
    }

    /// Open the bar `mode`'s way on `text`, over the cursor's row.
    fn open_bar(&mut self, mode: BarMode, text: &str, rows: &dyn Rows) -> Vec<Step> {
        let aim = self.cursor.row(rows).map(|row| row.id);
        let mut steps = self
            .surfaces
            .opened(SurfaceKind::Bar, self.policy.caps.stacking);
        steps.extend(self.bar.open(mode, text, aim));
        steps
    }

    /// The list leaves where it is for a place called `name`: the selection
    /// goes, `!` is off, and the strip names it.
    fn leave_for(&mut self, name: &str) -> Vec<Step> {
        let mut steps = self.cursor.leave(self.feed.total());
        steps.push(Step::Show(Intent::SingleHeading(None)));
        steps.push(Step::Show(Intent::Place {
            name: name.to_owned(),
        }));
        steps
    }

    /// `g i`: Focus's own inbox.
    fn go_inbox(&mut self) -> Vec<Step> {
        let mut steps = self.leave_for("Inbox");
        steps.push(Step::Open(ListScope::Focus(FocusScope::Inbox)));
        steps
    }

    /// `g z`, `g *`: a view over every account's mail, not a folder.
    fn go_view(&mut self, scope: FocusScope, name: &str) -> Vec<Step> {
        let mut steps = self.leave_for(name);
        steps.push(Step::Open(ListScope::Focus(scope)));
        steps
    }

    /// Show `destination` in the list, named `name`. A folder that is an
    /// inbox is Focus's; a label is its search, since Focus lists no label
    /// on its own.
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

    /// Close the bar, as running one of its lines does; the keyboard goes
    /// home unless what runs `opens` a surface of its own.
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

    /// Run the bar's line `token`.
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
            Run::Nothing => Vec::new(),
            Run::Search => {
                // The results are already under the row, from typing: Return
                // takes the highlight to the first of them.
                self.bar.pending = Pending::FirstHit;
                if self.bar.loaded == Some(self.bar.stamp) {
                    let highlight = self.bar.take_pending();
                    vec![self.bar.draw(highlight)]
                } else {
                    self.bar.search_again()
                }
            }
            Run::SearchFor(text) => {
                let Some(mode) = self.bar.mode else {
                    return Vec::new();
                };
                let mut steps = vec![Step::Show(Intent::OpenBar {
                    mode,
                    text: text.clone(),
                    select: None,
                })];
                self.bar.typed = text;
                steps.extend(self.bar.answer());
                steps
            }
            Run::ToggleOrder => self.bar.toggle_order(),
            Run::Action(BarAction::Open { message, .. }) => self.open_hit(message),
            Run::Action(BarAction::Go { destination, name }) => {
                let mut steps = self.dismiss_bar(false);
                steps.extend(self.go_to(destination, &name, rows));
                steps
            }
            Run::Action(BarAction::Command(id)) => {
                let aim = self.bar.aim;
                let mut steps = self.dismiss_bar(false);
                // What the bar opened over, wherever the cursor went since.
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

    /// Open a hit in the message surface, among the others it walks.
    fn open_hit(&mut self, message: MessageId) -> Vec<Step> {
        // A search's hits carry their conversations; a folder's (`in:`)
        // are its messages.
        let found: Vec<Hit> = match &self.bar.drop {
            Some(drop) => drop
                .landed
                .iter()
                .flat_map(|landed| &landed.hits)
                .map(|shown| Hit {
                    message: shown.hit.best,
                    thread: shown.thread(),
                })
                .collect(),
            None => self
                .bar
                .base
                .iter()
                .chain(&self.bar.results)
                .filter_map(|item| match &item.row {
                    Row::Message { message, .. } => Some(*message),
                    _ => None,
                })
                .map(|message| {
                    self.bar
                        .walked
                        .iter()
                        .find(|hit| hit.message == message)
                        .cloned()
                        .unwrap_or(Hit {
                            message,
                            thread: None,
                        })
                })
                .collect(),
        };
        let typed = self.bar.typed.clone();
        let mut steps = self.dismiss_bar(true);
        self.bar.found = found;
        self.bar.held = Some((typed, message));
        steps.extend(self.show_hit(message));
        steps
    }

    /// Show the hit `message`, at its place among the hits.
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

    /// `j`/`k` in a message a hit opened: the hits, not the list behind.
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

    /// The facts of a message a hit opened, for its verbs: it is in no
    /// list row.
    pub(crate) fn hit_facts(&self, reading: MessageId) -> Option<RowFacts> {
        let hit = self.bar.found.iter().find(|hit| hit.message == reading)?;
        Some(RowFacts {
            id: hit.message,
            digest: false,
            threads: hit.thread.into_iter().collect(),
            writes: false,
        })
    }

    /// The message a hit opened has closed: the bar comes back on the
    /// words, the highlight on the hit last read.
    pub(crate) fn hit_closed(&mut self, rows: &dyn Rows) -> Vec<Step> {
        self.bar.found.clear();
        let Some((typed, message)) = self.bar.held.take() else {
            return Vec::new();
        };
        let steps = self.open_bar(BarMode::Search, &typed, rows);
        // Applied when the results land.
        self.bar.pending = Pending::Hit(message);
        steps
    }

    /// Run the popover's place `token`.
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

    /// The popover's places whose names hold `filter`, with their tokens.
    pub(crate) fn listed_places(&self, filter: &str) -> Vec<(u64, Entry)> {
        let filtered_today = self.counts.map(|counts| counts.filtered_today);
        let all = self.bar.listed(filtered_today, "");
        self.bar
            .listed(filtered_today, filter)
            .into_iter()
            .filter_map(|entry| {
                let index = all.iter().position(|each| *each == entry)?;
                Some(((self.bar.places_read << 32) | index as u64, entry))
            })
            .collect()
    }

    /// A reply the bar asked for.
    pub(crate) fn bar_reply(&mut self, reply: Reply, rows: &dyn Rows) -> Vec<Step> {
        match reply {
            Reply::Places(read) => self.bar.places_landed(read),
            Reply::Search { stamp, answer } => self.bar.found(stamp, answer),
            Reply::Conversations { stamp, answer } => self.bar.conversations(stamp, answer),
            Reply::Passages { stamp, answer } => self.bar.passages(stamp, answer),
            Reply::RecentSearches(answer) => self.bar.recents_read(answer),
            Reply::SavedCounts(answer) => self.bar.saved_counted(answer),
            Reply::Folder { stamp, count, rows } => self.bar.folder(stamp, count, rows),
            Reply::RoleFolder(Some((mailbox, name))) => {
                self.go_to(Destination::Mailbox(mailbox), &name, rows)
            }
            _ => Vec::new(),
        }
    }

    /// The bar's own inputs.
    pub(crate) fn bar_input(&mut self, input: crate::Input, rows: &dyn Rows) -> Vec<Step> {
        use crate::Input;
        match input {
            Input::Typed { text } => self.bar.typed(text),
            Input::BarRun(token) => self.bar_run(token, rows),
            Input::BarTab => self.bar.tab(),
            Input::SearchHighlighted(token) => {
                self.bar.highlighted = Some(token);
                Vec::new()
            }
            Input::SearchForget(token) => self.bar.forget(token),
            Input::OpenPlace(token) => self.open_place(token, rows),
            Input::SavedSearches(saved) => {
                self.bar.set_saved(saved);
                self.redraw_bar()
            }
            Input::SearchSaved(result) => {
                let saving = self.bar.saving.take();
                match result {
                    Ok(saved) => {
                        self.bar.set_saved(saved);
                        let mut steps = vec![Step::Show(Intent::Toast {
                            text: postio_ui::focus_target::search_saved(
                                saving.as_deref().unwrap_or_default(),
                            ),
                            kind: crate::ToastKind::Notice,
                        })];
                        steps.extend(self.redraw_bar());
                        steps
                    }
                    Err(sentence) => vec![Step::Show(Intent::Toast {
                        text: sentence,
                        kind: crate::ToastKind::Notice,
                    })],
                }
            }
            Input::Filtering(filtering) => {
                self.bar.set_filtering(filtering);
                Vec::new()
            }
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

/// The dropdown: its state, what it asks and what its rows run (spec 010
/// step 2).
impl Bar {
    /// Whether `typed` is drawn as the dropdown: on a frontend with a
    /// results view, the empty field and words. `>`, `in:` and `@` keep
    /// spec 009's lines until their own states are built.
    fn shows_dropdown(&self, typed: &str) -> bool {
        self.results_view
            && self.mode == Some(BarMode::Search)
            && (typed.is_empty() || rules::route(typed) == rules::Route::Blend)
    }

    /// The pinned saved searches changed: their pills get new tokens.
    fn set_saved(&mut self, saved: Vec<(String, String)>) {
        self.saved = saved;
        self.saved_tokens = (0..self.saved.len()).map(|_| self.token()).collect();
    }

    /// Answer what is typed as the dropdown: the empty state now, or the
    /// words' search asked for.
    fn answer_dropdown(&mut self, typed: &str) -> Vec<Step> {
        if typed.is_empty() {
            self.chips.clear();
            self.drop = Some(Drop {
                state: DropdownState::Empty,
                query: None,
                terms: Vec::new(),
                landed: None,
            });
            return vec![self.draw(None)];
        }
        let parsed = self.lower(typed);
        self.chips = rules::chips(&parsed).unwrap_or_default();
        let searchable = parsed.is_searchable();
        // Words after words keep the last answer on screen until this one
        // lands, rather than blinking empty on every keystroke.
        let was_words = self
            .drop
            .as_ref()
            .is_some_and(|drop| drop.state == DropdownState::Words && drop.landed.is_some());
        let landed = if was_words && searchable {
            self.drop.take().and_then(|drop| drop.landed)
        } else {
            None
        };
        self.drop = Some(Drop {
            state: DropdownState::Words,
            terms: postio_search::highlight::terms(&parsed),
            query: Some(parsed.clone()),
            landed,
        });
        let mut steps = Vec::new();
        if !(was_words && searchable) {
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

    /// The dropdown, as it is now.
    fn draw_dropdown(&self, select: Option<u64>) -> Step {
        let Some(drop) = &self.drop else {
            return Step::Show(Intent::BarLines(BarView::default()));
        };
        let now = self.now();
        let mut view = match drop.state {
            DropdownState::Words => {
                dropdown::words(drop.landed.as_ref(), &drop.terms, &self.keymap, now)
            }
            _ => {
                let saved: Vec<(u64, String, Option<u64>)> = self
                    .saved
                    .iter()
                    .zip(&self.saved_tokens)
                    .map(|((name, _), token)| {
                        let count = self
                            .saved_counts
                            .iter()
                            .find(|(key, _)| key == name)
                            .map(|(_, count)| *count);
                        (*token, name.clone(), count)
                    })
                    .collect();
                dropdown::empty(dropdown::EmptyParts {
                    recents: &self.recents,
                    saved: &saved,
                    sheet: &self.sheet,
                    example: self.example(),
                    keymap: &self.keymap,
                    now,
                })
            }
        };
        view.select = select;
        Step::Show(Intent::Dropdown(view))
    }

    /// The cheat sheet's example, lowered against today and the address
    /// book, as typing it would be.
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

    /// What the dropdown's row or pill `token` runs.
    fn action(&self, token: u64) -> Option<Action> {
        let drop = self.drop.as_ref()?;
        match drop.state {
            DropdownState::Words => {
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

    /// Put `text` in the field, as a run of the bar does, and answer it.
    fn retype(&mut self, mode: BarMode, text: String) -> Vec<Step> {
        let mut steps = vec![Step::Show(Intent::OpenBar {
            mode,
            text: text.clone(),
            select: None,
        })];
        self.typed = text;
        steps.extend(self.answer());
        steps
    }

    /// Add `clause` to the query, spelled once (D13), and search again.
    fn narrow(&mut self, mode: BarMode, clause: postio_search::query::Clause) -> Vec<Step> {
        let text = postio_search::edit::apply(
            self.typed.trim(),
            postio_search::edit::Edit::Add(clause),
            self.now().date_naive(),
        );
        self.retype(mode, text)
    }

    /// A conversation search landed: the words' top hits, pills and count,
    /// when it is for the words still typed; then their passages.
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
        let mut folders = results.names.folders;
        folders.extend(self.places.folders.iter().cloned());
        let landed = Landed {
            hits,
            pills,
            show_all,
            total: results.total,
            capped: results.capped,
            elapsed: results.elapsed,
            folders,
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
        let select = match std::mem::take(&mut self.pending) {
            Pending::Hit(message) => self.drop.as_ref().and_then(|drop| {
                drop.landed
                    .as_ref()?
                    .hits
                    .iter()
                    .find(|shown| shown.hit.best == message)
            }),
            _ => None,
        }
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

    /// The hits' passages landed: drawn in place, the arrows where they are.
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

    /// The recent searches, read: drawn when the empty dropdown is up.
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
        self.redraw_empty()
    }

    /// The saved searches' counts, read.
    fn saved_counted(&mut self, answer: Result<Vec<(String, u64, u64)>, String>) -> Vec<Step> {
        match answer {
            Ok(counts) => {
                self.saved_counts = counts
                    .into_iter()
                    .map(|(key, total, _)| (key, total))
                    .collect();
                self.redraw_empty()
            }
            Err(error) => {
                tracing::debug!(%error, "the saved searches could not be counted");
                Vec::new()
            }
        }
    }

    fn redraw_empty(&self) -> Vec<Step> {
        match &self.drop {
            Some(drop) if self.is_open() && drop.state == DropdownState::Empty => {
                vec![self.draw(None)]
            }
            _ => Vec::new(),
        }
    }

    /// Forget the recent search `token`: gone from the panel now, from the
    /// store when the host has it.
    fn forget(&mut self, token: u64) -> Vec<Step> {
        let Some(at) = self.recents.iter().position(|(at, _)| *at == token) else {
            return Vec::new();
        };
        let (_, recent) = self.recents.remove(at);
        let mut steps = self.redraw_empty();
        steps.push(Step::Ask(Request::ForgetSearch {
            query: recent.query,
        }));
        steps
    }

    /// The key: forget the recent search the arrows rest on, or the one
    /// focused by default when they have not moved.
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

    /// The query, as it is kept among the recent searches.
    pub(crate) fn remember(&self) -> Option<Step> {
        let landed = self.drop.as_ref()?.landed.as_ref()?;
        let query = self.typed.trim();
        (!query.is_empty()).then(|| {
            Step::Ask(Request::RememberSearch {
                query: query.to_owned(),
                hits: landed.total,
            })
        })
    }
}

impl FocusController {
    /// Run the dropdown's `action`.
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
            Action::Saved(index) => rules::SAVED
                .get(index)
                .and_then(|id| self.going(*id, rows))
                .unwrap_or_default(),
        }
    }
}
