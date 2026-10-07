//! The command bar's state and keys (terminal.md, "The command bar").
//!
//! One box for search, places and commands. What it offers is
//! `postio_ui::finder::blend`'s, what typing means is
//! `postio_ui::command_bar::route`, and what a row runs is
//! `postio_ui::command_bar::Row::run`; this file holds what the terminal adds
//! to them: the input and its caret, the chosen row, the answers the host has
//! given and the keys that move among them. It does no I/O: a key returns a
//! [`Step`], and `app.rs` turns that into effects.
//!
//! Plain words are answered with the commands and places they name and one
//! search row; words that name what they want (an operator) are a search at
//! once, shown as chips. `in:` and a folder's name list that folder's
//! conversations. Everything is local: nothing typed leaves this machine.

use chrono::{DateTime, Utc};
use crossterm::event::{KeyCode, KeyEvent};
use postio_core::{ActionId, Availability, CommandId, Context, Keymap};
use postio_model::{MailboxId, MessageId};
use postio_search::ResultOrder;
use postio_ui::command_bar::{self as rules, BarAction, Row, Run};
use postio_ui::finder::{self, Place};
use postio_ui::keymap::{KeyContext, Outcome};
use postio_ui::names::Names;
use postio_ui::terminal::SafeText;
use tui_input::backend::crossterm::EventHandler;

use crate::app::Focus;
use crate::input::Keys;

/// What the bar needs of the app while a key is handled: the keys it may
/// show, and what can run where it opened.
pub struct Ctx<'a> {
    /// The keymap in force.
    pub keymap: &'a Keymap,
    /// The context a command runs in: where the bar opened.
    pub context: Context,
    /// What can run now.
    pub state: Availability,
    /// Whether the terminal delivers every chord.
    pub enhanced: bool,
}

impl Ctx<'_> {
    /// The key that runs `command`, as this terminal can send it.
    pub fn key(&self, command: impl Into<ActionId>) -> Option<String> {
        postio_ui::terminal::deliverable_binding(self.keymap, command, self.enhanced)
    }
}

/// What the app reads the bar is built from.
#[derive(Debug, Clone, Default)]
pub struct Sources {
    /// Every place the bar can go: mailboxes, folders, and labels once read.
    pub places: Vec<Place>,
    /// Folder names by id, for a result's `in:`.
    pub folders: Vec<(MailboxId, String)>,
    /// The pinned saved searches: each name and its query.
    pub saved: Vec<(String, String)>,
    /// Whether any digest rule can hold mail, so a result says where held
    /// mail waits.
    pub digesting: bool,
}

/// A message the bar lists: a search hit or a folder's conversation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResultRow {
    /// Which message.
    pub message: MessageId,
    /// Who it is from.
    pub sender: SafeText,
    /// Its subject.
    pub subject: SafeText,
    /// What the row says after the subject: the matching text or the first
    /// line.
    pub snippet: SafeText,
    /// Where it sits: `in:Archive`, or where held mail waits.
    pub place: Option<SafeText>,
    /// When it arrived.
    pub at: DateTime<Utc>,
}

impl ResultRow {
    /// A folder's conversation, listed by its newest message.
    pub fn of_thread(thread: &postio_model::listing::ThreadSummary) -> ResultRow {
        let representative = &thread.representative;
        ResultRow {
            message: representative.id,
            sender: SafeText::new(
                &representative
                    .from
                    .as_ref()
                    .map(rules::said_of)
                    .unwrap_or_default(),
            ),
            subject: SafeText::new(thread.subject.as_deref().unwrap_or_default()),
            snippet: SafeText::new(representative.preview.as_deref().unwrap_or_default()),
            place: None,
            at: thread.last_at,
        }
    }

    /// A folder's message.
    pub fn of_message(message: &postio_model::listing::MessageSummary) -> ResultRow {
        ResultRow {
            message: message.id,
            sender: SafeText::new(
                &message
                    .from
                    .as_ref()
                    .map(rules::said_of)
                    .unwrap_or_default(),
            ),
            subject: SafeText::new(message.subject.as_deref().unwrap_or_default()),
            snippet: SafeText::new(message.preview.as_deref().unwrap_or_default()),
            place: None,
            at: message.received_at,
        }
    }
}

/// A line the bar draws, apart from the row it runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Line {
    /// A section heading.
    Heading(String),
    /// A command, a place, or a row about the results.
    Row {
        /// What it says.
        title: String,
        /// Its dimmer detail.
        detail: Option<String>,
        /// The key that does the same.
        key: Option<String>,
    },
    /// A message.
    Message(ResultRow),
}

/// One line and what running it does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// What is drawn.
    pub line: Line,
    /// What it runs.
    pub row: Row,
}

/// What a key or a click asks of the app.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// Draw again.
    Stay,
    /// Put the bar away.
    Close,
    /// Ask the host this search.
    Search(Ask),
    /// Ask the host for this folder's conversations.
    Folder(MailboxId, u64),
    /// Close and do this.
    Act(BarAction),
    /// Save this query as a pinned search.
    Save(String),
}

/// A search the host is asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ask {
    /// Which question this is, so an older answer is dropped.
    pub sequence: u64,
    /// The words, lowered to the one query language.
    pub query: postio_search::ParsedQuery,
    /// The order the results come back in.
    pub order: ResultOrder,
    /// Whether to ask where held mail waits.
    pub digesting: bool,
}

/// What a search found, listed.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Found {
    rows: Vec<ResultRow>,
    /// The word searched for in place of the one typed.
    instead: Option<(String, String)>,
}

/// A folder's conversations, listed.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Listing {
    name: String,
    mailbox: MailboxId,
    count: Option<u32>,
    rows: Vec<ResultRow>,
}

/// The bar, open.
#[derive(Debug)]
pub struct Bar {
    input: tui_input::Input,
    /// Where the keyboard was when it opened: what a command runs against,
    /// and where the keyboard goes back to.
    from: Focus,
    sources: Sources,
    names: Names,
    selected: usize,
    /// Whether an arrow has chosen a row since the rows last changed: until
    /// then, typing wins and `O` is a letter.
    stepped: bool,
    order: ResultOrder,
    /// Moves with every question, so a late answer is dropped.
    sequence: u64,
    /// The plain words typed, while `Tab` has stepped into the chips they
    /// were lowered to: what `back_to_words` goes back to.
    words: Option<String>,
    editing: Option<usize>,
    chips: Vec<String>,
    /// The search row was run: the answer replaces the blend.
    searched: bool,
    found: Option<Found>,
    listing: Option<Listing>,
}

impl Bar {
    /// A bar open over `from`, holding `typed`; and what to ask first.
    pub fn open(from: Focus, sources: Sources, typed: &str) -> (Bar, Step) {
        let mut bar = Bar {
            input: tui_input::Input::default(),
            from,
            sources,
            names: Names::default(),
            selected: 0,
            stepped: false,
            order: ResultOrder::Relevance,
            sequence: 0,
            words: None,
            editing: None,
            chips: Vec::new(),
            searched: false,
            found: None,
            listing: None,
        };
        let step = bar.set_text(typed);
        (bar, step)
    }

    /// Where the keyboard was when the bar opened.
    pub fn from(&self) -> Focus {
        self.from
    }

    /// What is typed.
    pub fn typed(&self) -> &str {
        self.input.value()
    }

    /// Where the caret is, in characters.
    pub fn caret(&self) -> usize {
        self.input.cursor()
    }

    /// The chips the words were lowered to.
    pub fn chips(&self) -> &[String] {
        &self.chips
    }

    /// The chip being edited, while one is.
    pub fn editing(&self) -> Option<usize> {
        self.editing
    }

    /// The chosen line, as an index into [`Bar::entries`].
    pub fn selected(&self) -> usize {
        self.selected
    }

    /// The listed message `message`, as the bar found it.
    pub fn result_of(&self, message: MessageId) -> Option<&ResultRow> {
        let found = self.found.as_ref().map(|found| found.rows.as_slice());
        let listed = self.listing.as_ref().map(|listing| listing.rows.as_slice());
        found
            .into_iter()
            .chain(listed)
            .flatten()
            .find(|row| row.message == message)
    }

    /// The order results come in.
    pub fn order(&self) -> ResultOrder {
        self.order
    }

    /// The plain words typed before `Tab` stepped into the chips.
    pub fn words(&self) -> Option<&str> {
        self.words.as_deref()
    }

    /// The pinned searches, each name and its query.
    pub fn saved(&self) -> &[(String, String)] {
        &self.sources.saved
    }

    /// The pinned searches changed under the open bar.
    pub fn set_saved(&mut self, saved: Vec<(String, String)>) {
        self.sources.saved = saved;
    }

    /// Learn the labels and the correspondents the host has read: labels are
    /// places, and a typed name is looked up among the correspondents.
    pub fn learn(&mut self, labels: Vec<Place>, names: Names) {
        self.sources
            .places
            .retain(|place| place.kind != finder::PlaceKind::Label);
        self.sources.places.extend(labels);
        self.names = names;
    }

    /// Put `text` in the input, as typing it would.
    pub fn set_text(&mut self, text: &str) -> Step {
        self.input = tui_input::Input::default().with_value(text.to_owned());
        self.words = None;
        self.changed()
    }

    /// The query a search would ask: the chips the words were lowered to, or
    /// the words themselves when they lower to no chip.
    pub fn query(&self) -> String {
        if self.chips.is_empty() {
            self.input.value().trim().to_owned()
        } else {
            self.chips.join(" ")
        }
    }

    /// `typed`, read as plain English against today and the address book.
    fn lowered(&self, typed: &str) -> postio_search::ParsedQuery {
        postio_search::natural::lower(typed, chrono::Local::now().date_naive(), &|name| {
            self.names.lookup(name)
        })
    }

    /// Answer what is now typed: a folder's conversations after `in:`, the
    /// blend otherwise, and a search at once when the words are one.
    fn changed(&mut self) -> Step {
        self.sequence += 1;
        self.found = None;
        self.listing = None;
        self.searched = false;
        self.stepped = false;
        self.editing = None;
        self.chips.clear();
        let step = self.answer();
        self.selected = 0;
        step
    }

    fn answer(&mut self) -> Step {
        let typed = self.input.value().trim().to_owned();
        match rules::route(&typed) {
            rules::Route::Folder(name) => match rules::folder_for(&self.sources.places, name) {
                Some((name, mailbox)) => {
                    self.listing = Some(Listing {
                        name,
                        mailbox,
                        count: None,
                        rows: Vec::new(),
                    });
                    Step::Folder(mailbox, self.sequence)
                }
                None => Step::Stay,
            },
            rules::Route::Plain => Step::Stay,
            // The terminal has no correspondent rows yet: `@` is a word.
            rules::Route::Correspondent(_) | rules::Route::Blend => {
                let parsed = self.lowered(&typed);
                match rules::chips(&parsed) {
                    Some(chips) => {
                        self.chips = chips;
                        Step::Search(self.ask(parsed))
                    }
                    None => Step::Stay,
                }
            }
        }
    }

    fn ask(&self, query: postio_search::ParsedQuery) -> Ask {
        Ask {
            sequence: self.sequence,
            query,
            order: self.order,
            digesting: self.sources.digesting,
        }
    }

    /// `in:` typed before the places were read is answered again, now that
    /// there are folders to complete.
    pub fn places_known(&mut self) -> Step {
        if self.input.value().trim().starts_with("in:") && self.listing.is_none() {
            let keep = self.words.clone();
            let step = self.changed();
            self.words = keep;
            return step;
        }
        Step::Stay
    }

    /// Search for what is typed now: what the search row runs.
    fn search_typed(&mut self) -> Step {
        let typed = self.input.value().trim().to_owned();
        if typed.is_empty() {
            return Step::Stay;
        }
        let parsed = self.lowered(&typed);
        self.sequence += 1;
        self.searched = true;
        self.found = None;
        self.stepped = false;
        self.selected = 0;
        Step::Search(self.ask(parsed))
    }

    /// The host answered a search.
    pub fn found(
        &mut self,
        ctx: &Ctx<'_>,
        sequence: u64,
        results: postio_search::SearchResults,
        held: &[(MessageId, String, bool)],
    ) {
        if sequence != self.sequence {
            return;
        }
        let rows = rules::conversations(results.hits)
            .into_iter()
            .map(|hit| ResultRow {
                message: hit.message_id,
                sender: SafeText::new(&hit.from.as_ref().map(rules::said_of).unwrap_or_default()),
                subject: SafeText::new(hit.subject.as_deref().unwrap_or_default()),
                snippet: SafeText::new(&postio_search::highlight::from_snippet(&hit.snippet).text),
                place: rules::result_place(&hit, held, &self.sources.folders)
                    .map(|place| SafeText::new(&place)),
                at: hit.received_at,
            })
            .collect();
        self.found = Some(Found {
            rows,
            instead: results.instead.map(|instead| (instead.typed, instead.term)),
        });
        self.choose_first_message(ctx);
    }

    /// The host answered a folder's listing.
    pub fn listed(&mut self, ctx: &Ctx<'_>, sequence: u64, count: u32, rows: Vec<ResultRow>) {
        if sequence != self.sequence {
            return;
        }
        if let Some(listing) = self.listing.as_mut() {
            listing.count = Some(count);
            listing.rows = rows;
        }
        self.choose_first_message(ctx);
    }

    /// Rest on the first message, unless an arrow has chosen a row already:
    /// Enter then opens the top result.
    fn choose_first_message(&mut self, ctx: &Ctx<'_>) {
        if self.stepped {
            return;
        }
        let at = self
            .entries(ctx)
            .iter()
            .position(|entry| matches!(entry.row, Row::Message { .. }));
        if let Some(at) = at {
            self.selected = at;
        }
    }

    /// The lines the bar lists, with what each runs: the blend's commands,
    /// places and search row, then what the host found.
    pub fn entries(&self, ctx: &Ctx<'_>) -> Vec<Entry> {
        let mut out = Vec::new();
        let typed = self.input.value().trim();
        if self.listing.is_none() && !self.searched {
            out.extend(self.blend(typed, ctx));
        }
        self.results(ctx, &mut out);
        out
    }

    fn blend(&self, typed: &str, ctx: &Ctx<'_>) -> Vec<Entry> {
        let mut blend = finder::blend(
            typed,
            &self.sources.places,
            ctx.keymap,
            ctx.context,
            ctx.state,
        );
        let words = typed.strip_prefix(finder::COMMANDS_ONLY).unwrap_or(typed);
        if typed.starts_with(finder::COMMANDS_ONLY) || !words.trim().is_empty() {
            rules::add_account_verbs(&mut blend.commands, ctx.keymap, ctx.state, words.trim());
        }
        rules::blend_lines(&blend)
            .into_iter()
            .map(|line| match line {
                rules::Line::Heading(title) => Entry {
                    line: Line::Heading(title),
                    row: Row::Heading,
                },
                rules::Line::Row {
                    row, title, detail, ..
                } => {
                    let key = match &row {
                        Row::Command(id) => ctx.key(*id),
                        Row::Place(_, name) => blend
                            .places
                            .iter()
                            .find(|hit| hit.place.name == *name)
                            .and_then(|hit| hit.place.go)
                            .and_then(|go| ctx.key(go)),
                        _ => None,
                    };
                    Entry {
                        line: Line::Row { title, detail, key },
                        row,
                    }
                }
            })
            .collect()
    }

    fn results(&self, ctx: &Ctx<'_>, out: &mut Vec<Entry>) {
        let message = |row: &ResultRow| Entry {
            line: Line::Message(row.clone()),
            row: Row::Message {
                message: row.message,
                subject: row.subject.as_str().to_owned(),
            },
        };
        let heading = |text: String| Entry {
            line: Line::Heading(text),
            row: Row::Heading,
        };
        if let Some(listing) = &self.listing {
            if let Some(count) = listing.count {
                out.push(heading(rules::folder_heading(&listing.name, count)));
                out.extend(listing.rows.iter().map(message));
            }
            return;
        }
        let Some(found) = &self.found else {
            return;
        };
        out.push(heading(rules::results_heading(found.rows.len())));
        if let Some((typed, term)) = &found.instead {
            out.push(heading(rules::showing_results_for(term)));
            let (title, detail) = rules::search_instead(typed);
            out.push(Entry {
                line: Line::Row {
                    title,
                    detail: Some(detail.to_owned()),
                    key: None,
                },
                row: Row::Instead(typed.clone()),
            });
        }
        if !found.rows.is_empty() {
            let (title, detail) = rules::order_words(self.order);
            out.push(Entry {
                line: Line::Row {
                    title,
                    detail: Some(detail),
                    key: ctx.key(CommandId::ToggleResultOrder),
                },
                row: Row::Order,
            });
        }
        out.extend(found.rows.iter().map(message));
    }

    /// The chosen line: `selected`, or the nearest row that can be chosen.
    pub fn chosen(&self, entries: &[Entry]) -> usize {
        let at = self.selected.min(entries.len().saturating_sub(1));
        (at..entries.len())
            .chain((0..at).rev())
            .find(|index| entries[*index].row.is_selectable())
            .unwrap_or(0)
    }

    /// Move the chosen row `by` rows, skipping headings.
    fn step(&mut self, by: isize, ctx: &Ctx<'_>) {
        let entries = self.entries(ctx);
        let mut at = self.chosen(&entries) as isize;
        loop {
            let next = at + by.signum();
            if next < 0 || next >= entries.len() as isize {
                break;
            }
            at = next;
            if entries[at as usize].row.is_selectable() {
                self.selected = at as usize;
                self.stepped = true;
                // One row per step of `by`, so a page is several.
                if by.abs() <= 1 {
                    return;
                }
                return self.step(by - by.signum(), ctx);
            }
        }
    }

    /// `Tab`: from the words into the first chip, then to the next. The
    /// input then holds the chips, and the echo says which is edited.
    fn next_chip(&mut self) -> Step {
        if self.chips.is_empty() {
            return Step::Stay;
        }
        let next = match self.editing {
            None => {
                self.words = Some(self.input.value().to_owned());
                0
            }
            Some(at) => (at + 1) % self.chips.len(),
        };
        self.edit_chip(next)
    }

    /// Edit the `at`th chip: the input holds the chips.
    pub fn edit_chip(&mut self, at: usize) -> Step {
        if at >= self.chips.len() {
            return Step::Stay;
        }
        if self.editing.is_none() && self.words.is_none() {
            self.words = Some(self.input.value().to_owned());
        }
        self.editing = Some(at);
        self.input = tui_input::Input::default().with_value(self.chips.join(" "));
        Step::Stay
    }

    /// `back_to_words`: from the chips to the words they were lowered from.
    fn back_to_words(&mut self) -> Step {
        let Some(words) = self.words.take() else {
            return Step::Stay;
        };
        self.input = tui_input::Input::default().with_value(words);
        self.editing = None;
        Step::Stay
    }

    /// Run the `index`th line.
    pub fn run(&mut self, index: usize, ctx: &Ctx<'_>) -> Step {
        let Some(entry) = self.entries(ctx).into_iter().nth(index) else {
            return Step::Stay;
        };
        match entry.row.run() {
            Run::Nothing => Step::Stay,
            Run::Search => self.search_typed(),
            Run::SearchFor(text) => {
                self.input = tui_input::Input::default().with_value(text);
                self.words = None;
                self.search_typed()
            }
            Run::ToggleOrder => self.toggle_order(),
            Run::Action(action) => Step::Act(action),
        }
    }

    fn toggle_order(&mut self) -> Step {
        self.order = match self.order {
            ResultOrder::Relevance => ResultOrder::Newest,
            ResultOrder::Newest => ResultOrder::Relevance,
        };
        self.search_typed()
    }

    /// A click on a line: it is chosen and run.
    pub fn click(&mut self, index: usize, ctx: &Ctx<'_>) -> Step {
        self.selected = index;
        self.stepped = true;
        self.run(index, ctx)
    }

    /// A click on the `index`th saved search.
    pub fn open_saved(&mut self, index: usize) -> Step {
        match self
            .sources
            .saved
            .get(index)
            .map(|(_, query)| query.clone())
        {
            Some(query) => self.set_text(&query),
            None => Step::Stay,
        }
    }

    /// The wheel: the chosen row moves, and the list follows it.
    pub fn wheel(&mut self, down: bool, ctx: &Ctx<'_>) {
        self.step(if down { 3 } else { -3 }, ctx);
    }

    /// `Enter`: run the chosen line.
    pub fn enter(&mut self, ctx: &Ctx<'_>) -> Step {
        let entries = self.entries(ctx);
        let at = self.chosen(&entries);
        self.run(at, ctx)
    }

    /// Show only commands.
    pub fn commands_only(&mut self) -> Step {
        self.set_text(&finder::COMMANDS_ONLY.to_string())
    }

    /// Save the search as typed, when there is one.
    pub fn save(&self) -> Step {
        let query = self.query();
        if query.is_empty() {
            Step::Stay
        } else {
            Step::Save(query)
        }
    }

    /// A key in the bar.
    pub fn key(&mut self, key: &KeyEvent, keys: &mut Keys, ctx: &Ctx<'_>) -> Step {
        match keys.press(key, KeyContext::Search, true) {
            Outcome::Command(id) => {
                return match id.as_str() {
                    "back" => Step::Close,
                    "save_search" => self.save(),
                    "back_to_words" => self.back_to_words(),
                    // Typing wins: `O` is a letter; this one carries `alt`.
                    "toggle_result_order" => self.toggle_order(),
                    "saved_search_1" => self.open_saved(0),
                    "saved_search_2" => self.open_saved(1),
                    "saved_search_3" => self.open_saved(2),
                    "saved_search_4" => self.open_saved(3),
                    "command_palette" => self.commands_only(),
                    _ => Step::Stay,
                };
            }
            Outcome::Pending(_) => return Step::Stay,
            Outcome::Unhandled => {}
        }
        match key.code {
            KeyCode::Down | KeyCode::Up | KeyCode::PageDown | KeyCode::PageUp => {
                // An arrow has chosen: typing no longer wins over `O`.
                self.stepped = true;
                let by = match key.code {
                    KeyCode::Down => 1,
                    KeyCode::Up => -1,
                    KeyCode::PageDown => 8,
                    _ => -8,
                };
                self.step(by, ctx);
            }
            KeyCode::Tab if key.modifiers.is_empty() => return self.next_chip(),
            KeyCode::Enter => return self.enter(ctx),
            _ => {
                let before = self.input.value().to_owned();
                self.input.handle_event(&crossterm::event::Event::Key(*key));
                if self.input.value() != before {
                    return self.changed();
                }
            }
        }
        Step::Stay
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use crossterm::event::{KeyCode, KeyModifiers};
    use postio_model::ids::ThreadId;
    use postio_search::{SearchHit, SearchResults};

    use crate::app::{Effect, Input, update};
    use crate::test_support::{
        alt, app, ctrl, key, open_list, places, press, saved_search, screen, seed_places, serve,
        type_text,
    };
    use crate::view::hit::Target;

    fn opened() -> crate::app::App {
        let mut app = app((120, 36));
        let mut contents = places();
        contents.saved = vec![saved_search("waiting", "Waiting", "is:unread")];
        seed_places(&mut app, contents);
        let opening = open_list(&mut app, 3);
        serve(&mut app, opening);
        app
    }

    fn hit(message: i64, thread: i64, subject: &str) -> SearchHit {
        SearchHit {
            message_id: MessageId::new(message),
            thread_id: Some(ThreadId::new(thread)),
            mailbox_id: MailboxId::new(2),
            subject: Some(subject.to_owned()),
            from: Some(postio_model::EmailAddress::new(
                Some("Ada Moreno"),
                "ada@example.com",
            )),
            received_at: chrono::Utc.with_ymd_and_hms(2026, 9, 20, 9, 0, 0).unwrap(),
            preview: None,
            snippet: "the \u{1}tide\u{2} gate".to_owned(),
            score: 0.0,
        }
    }

    fn results(hits: Vec<SearchHit>) -> SearchResults {
        SearchResults {
            total_hits: hits.len() as u64,
            hits,
            total_hits_capped: false,
            elapsed: std::time::Duration::from_millis(3),
            corpus_complete: true,
            suggestion: None,
            instead: None,
        }
    }

    fn asked(effects: &[Effect]) -> Vec<Ask> {
        effects
            .iter()
            .filter_map(|effect| match effect {
                Effect::BarSearch(ask) => Some(ask.clone()),
                _ => None,
            })
            .collect()
    }

    fn answer(app: &mut crate::app::App, sequence: u64, hits: Vec<SearchHit>) {
        update(
            app,
            Input::BarFound {
                sequence,
                found: Ok(Some(postio_client::protocol::Hits(results(hits)))),
                held: Vec::new(),
            },
        );
    }

    #[test]
    fn words_that_name_an_operator_are_a_search_shown_as_chips_and_plain_words_are_not() {
        let mut app = opened();
        update(&mut app, press('/'));
        let plain = type_text(&mut app, "invoice");
        assert!(
            asked(&plain).is_empty(),
            "a plain word searches when its row is run"
        );
        let drawn = screen(120, 36, &app);
        assert!(drawn.contains("Search mail for “invoice”"), "{drawn}");

        update(&mut app, press(' '));
        let effects = type_text(&mut app, "from:ada");
        let ask = asked(&effects)
            .pop()
            .expect("an operator is a search at once");
        assert_eq!(ask.order, ResultOrder::Relevance);
        let drawn = screen(120, 36, &app);
        assert!(drawn.contains("from:ada"), "{drawn}");
        assert!(drawn.contains("You typed “invoice from:ada”"), "{drawn}");
    }

    #[test]
    fn results_are_one_per_conversation_under_their_heading_and_enter_opens_the_top_one() {
        let mut app = opened();
        update(&mut app, press('/'));
        let effects = type_text(&mut app, "from:ada");
        let sequence = asked(&effects).pop().unwrap().sequence;
        answer(
            &mut app,
            sequence,
            vec![
                hit(11, 7, "Tide gate"),
                hit(12, 7, "Re: Tide gate"),
                hit(13, 8, "Harbor"),
            ],
        );
        let drawn = screen(120, 36, &app);
        assert!(drawn.contains("Conversations · 2 matches"), "{drawn}");
        assert!(drawn.contains("Sorted by relevance"), "{drawn}");
        assert!(drawn.contains("switch to date"), "{drawn}");
        assert!(drawn.contains("Tide gate"), "{drawn}");
        assert!(!drawn.contains("Re: Tide gate"), "{drawn}");
        assert!(drawn.contains("in:Archive"), "{drawn}");
        assert!(
            drawn.contains("the tide gate"),
            "the snippet's markers are gone: {drawn}"
        );
        let line = drawn
            .lines()
            .find(|line| line.contains("Tide gate"))
            .unwrap();
        assert!(line.contains('▌'), "the top result is chosen: {line}");

        let effects = update(&mut app, key(KeyCode::Enter, KeyModifiers::NONE));
        assert!(
            effects.contains(&Effect::ReadBody(MessageId::new(11))),
            "{effects:?}"
        );
        assert_eq!(app.focus(), crate::app::Focus::Reader);
        assert!(app.bar_typed().is_none(), "the bar is put away");
    }

    #[test]
    fn an_answer_to_an_earlier_question_is_dropped() {
        let mut app = opened();
        update(&mut app, press('/'));
        let first = asked(&type_text(&mut app, "from:ada"))
            .pop()
            .unwrap()
            .sequence;
        let second = asked(&type_text(&mut app, "x")).pop().unwrap().sequence;
        assert_ne!(first, second);
        answer(&mut app, first, vec![hit(11, 7, "Stale")]);
        assert!(!screen(120, 36, &app).contains("Stale"));
    }

    #[test]
    fn o_is_a_letter_and_alt_o_switches_the_order() {
        let mut app = opened();
        update(&mut app, press('/'));
        let sequence = asked(&type_text(&mut app, "from:ada"))
            .pop()
            .unwrap()
            .sequence;
        answer(&mut app, sequence, vec![hit(11, 7, "Tide gate")]);
        // `O` is a letter, before an arrow and after one.
        update(&mut app, press('O'));
        assert!(app.bar_typed().unwrap().ends_with('O'));
        update(&mut app, key(KeyCode::Backspace, KeyModifiers::NONE));
        let effects = update(&mut app, key(KeyCode::Down, KeyModifiers::NONE));
        assert!(asked(&effects).is_empty());
        update(&mut app, press('O'));
        assert!(app.bar_typed().unwrap().ends_with('O'));
        update(&mut app, key(KeyCode::Backspace, KeyModifiers::NONE));
        // `alt+o` switches the order with the query holding the keyboard.
        let effects = update(&mut app, key(KeyCode::Char('o'), KeyModifiers::ALT));
        let ask = asked(&effects)
            .pop()
            .expect("alt+o switches the order and asks again");
        assert_eq!(ask.order, ResultOrder::Newest);
        assert_eq!(app.bar_typed(), Some("from:ada"));
    }

    #[test]
    fn in_and_a_folder_name_list_its_conversations() {
        let mut app = opened();
        update(&mut app, press('/'));
        let effects = type_text(&mut app, "in:arch");
        let (mailbox, sequence) = effects
            .iter()
            .rev()
            .find_map(|effect| match effect {
                Effect::BarFolder { sequence, mailbox } => Some((*mailbox, *sequence)),
                _ => None,
            })
            .expect("the folder is listed");
        assert_eq!(mailbox, MailboxId::new(2));
        update(
            &mut app,
            Input::BarFolder {
                sequence,
                count: 4,
                rows: vec![ResultRow::of_message(
                    &postio_model::listing::MessageSummary {
                        id: MessageId::new(20),
                        thread: None,
                        from: Some(postio_model::EmailAddress::new(
                            Some("Grace"),
                            "g@example.com",
                        )),
                        subject: Some("Receipt".into()),
                        preview: Some("Thanks".into()),
                        received_at: chrono::Utc::now(),
                        seen: true,
                        flagged: false,
                        answered: false,
                        send_state: None,
                        send_at: None,
                        has_attachments: false,
                        thread_count: 1,
                    },
                )],
            },
        );
        let drawn = screen(120, 36, &app);
        assert!(
            drawn.contains("Archive · folder · 4 conversations · newest first"),
            "{drawn}"
        );
        assert!(drawn.contains("Receipt"), "{drawn}");
    }

    #[test]
    fn ctrl_s_saves_the_query_and_alt_1_runs_a_saved_search() {
        let mut app = opened();
        update(&mut app, press('/'));
        type_text(&mut app, "from:ada");
        let effects = update(&mut app, ctrl('s'));
        assert!(
            effects.contains(&Effect::SaveSearch("from:ada".into())),
            "{effects:?}"
        );
        assert_eq!(app.notice(), Some("Saved “from:ada”"));
        update(&mut app, key(KeyCode::Esc, KeyModifiers::NONE));
        assert!(app.bar_typed().is_none());
        assert_eq!(app.focus(), crate::app::Focus::List);

        let effects = update(&mut app, alt('1'));
        assert_eq!(app.bar_typed(), Some("is:unread"));
        assert_eq!(asked(&effects).len(), 1, "{effects:?}");
    }

    #[test]
    fn tab_steps_into_the_chips_and_alt_backspace_goes_back_to_the_words() {
        let mut app = opened();
        update(&mut app, press('/'));
        type_text(&mut app, "from:ada tide");
        update(&mut app, key(KeyCode::Tab, KeyModifiers::NONE));
        let drawn = screen(120, 36, &app);
        assert!(drawn.contains("editing from:"), "{drawn}");
        assert!(drawn.contains("back to plain words"), "{drawn}");
        update(&mut app, key(KeyCode::Backspace, KeyModifiers::ALT));
        assert_eq!(app.bar_typed(), Some("from:ada tide"));
        assert!(!screen(120, 36, &app).contains("editing"));
    }

    #[test]
    fn a_command_runs_against_the_row_the_bar_opened_over() {
        let mut app = opened();
        update(&mut app, press('j'));
        update(&mut app, press('/'));
        type_text(&mut app, "archive");
        let effects = update(&mut app, key(KeyCode::Enter, KeyModifiers::NONE));
        assert!(
            effects
                .iter()
                .any(|effect| matches!(effect, Effect::Send(postio_core::Command::Archive { .. }))),
            "{effects:?}"
        );
        assert_eq!(app.focus(), crate::app::Focus::List);
        assert_eq!(app.cursor(), 1);
    }

    #[test]
    fn a_place_goes_there_and_a_label_reopens_the_bar_on_its_search() {
        let mut app = opened();
        update(&mut app, press('/'));
        type_text(&mut app, "in:Archive");
        update(&mut app, key(KeyCode::Esc, KeyModifiers::NONE));
        update(&mut app, press('/'));
        type_text(&mut app, "arch");
        for _ in 0..12 {
            let drawn = screen(120, 36, &app);
            if drawn
                .lines()
                .any(|line| line.contains('▌') && line.contains("in:Archive"))
            {
                break;
            }
            update(&mut app, key(KeyCode::Down, KeyModifiers::NONE));
        }
        let effects = update(&mut app, key(KeyCode::Enter, KeyModifiers::NONE));
        assert!(
            effects.contains(&Effect::Open(ListScope::Mailbox(MailboxId::new(2)))),
            "{effects:?}"
        );

        update(
            &mut app,
            Input::PlaceDetails(crate::places::PlaceDetails {
                labels: vec![crate::test_support::label(5, "Atlas")],
                ..Default::default()
            }),
        );
        update(&mut app, press('/'));
        let effects = update(
            &mut app,
            Input::PlaceDetails(crate::places::PlaceDetails {
                labels: vec![crate::test_support::label(5, "Atlas")],
                ..Default::default()
            }),
        );
        let _ = effects;
        type_text(&mut app, "atlas");
        for _ in 0..12 {
            let drawn = screen(120, 36, &app);
            if drawn
                .lines()
                .any(|line| line.contains('▌') && line.contains("in:Atlas"))
            {
                break;
            }
            update(&mut app, key(KeyCode::Down, KeyModifiers::NONE));
        }
        let effects = update(&mut app, key(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.bar_typed(), Some("label:\"Atlas\""), "{effects:?}");
    }

    #[test]
    fn rows_saved_pills_and_chips_are_clickable_and_a_click_outside_does_nothing() {
        let mut app = opened();
        update(&mut app, press('/'));
        type_text(&mut app, "arch");
        let hits = crate::test_support::hits_of(120, 36, &app);
        let drawn = screen(120, 36, &app);
        let row = drawn
            .lines()
            .position(|line| line.contains("Archive thread"))
            .unwrap();
        let at = hits.at(30, u16::try_from(row).unwrap()).expect("a row");
        assert!(matches!(at.target, Target::BarRow(_)), "{at:?}");
        let saved = hits.at(20, 2).expect("the saved row");
        assert!(matches!(saved.target, Target::BarSaved(0)), "{saved:?}");
        // The list behind the box is out of reach.
        let outside = hits.at(2, 3).expect("the list");
        let effects = update(
            &mut app,
            crate::test_support::click(outside.target, false, false),
        );
        assert!(effects.is_empty() && app.bar_typed().is_some());
        let effects = update(
            &mut app,
            crate::test_support::click(at.target, false, false),
        );
        assert!(
            effects
                .iter()
                .any(|effect| matches!(effect, Effect::Send(_))),
            "a click runs the row: {effects:?}"
        );
    }

    #[test]
    fn account_verbs_are_listed_and_open_settings_to_pick_an_account() {
        let mut app = opened();
        update(&mut app, press('/'));
        type_text(&mut app, "rebuild");
        let drawn = screen(120, 36, &app);
        assert!(drawn.contains("Rebuild"), "{drawn}");
        let effects = update(&mut app, key(KeyCode::Enter, KeyModifiers::NONE));
        assert!(app.settings().is_some(), "{effects:?}");
    }

    use postio_model::ListScope;
}
