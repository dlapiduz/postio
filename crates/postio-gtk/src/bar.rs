//! The command bar (screens 07-09; contracts/focus-surface.md, "The command
//! bar"): `/` or `Ctrl K` opens one bar for search, places and commands.
//!
//! What the bar offers is `postio_ui::finder::blend`'s: the commands the
//! words match, each with its key; the places they name, with their counts
//! and keys; and one plain search row. The search itself runs on this
//! machine's index, through the host, with the words lowered into the one
//! query language (`postio_search::natural::lower`) and shown as chips.
//! `in:` and a name complete a folder, and list its conversations newest
//! first (screen 08). A half-typed operator is a partial, never an error:
//! the bar draws no error at all.
//!
//! Results arrive as the words are typed, under the "Search mail for" row,
//! which stays above them. A result set can be switched between relevance
//! and date: the row that says which it is in switches it when run, and so
//! does `alt+o` with the query holding the keyboard. `O` is a letter in the
//! box, always -- typing wins over a bare key.
//!
//! Everything here is local. Nothing typed leaves the machine.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;
use postio_client::Client;
use postio_core::{ActionId, CommandId, Context, Frontend, Keymap};
use postio_model::listing::{ListPage, MailStore as _, PageRequest};
use postio_model::{AccountScope, ListScope, MailboxId, MessageId};
use postio_ui::finder::{self, Place};
use postio_widgets::widgets::keyhint;
use postio_widgets::widgets::space::{S1, S2, S3};

/// The bar's width, as the screens draw it.
const WIDTH: i32 = 860;
/// The top bar's field, which the bar's input takes the place of.
const FIELD_WIDTH: i32 = 480;
pub use postio_ui::command_bar::BarAction;
use postio_ui::command_bar::{self as rules, Line, Row, Run, SAVED, said_of};

/// What a run of the bar asks for.
type Handler = Rc<dyn Fn(BarAction)>;

/// The bar.
pub struct Bar {
    client: Client,
    root: gtk::Box,
    /// The bar over its scrim: what the window lays over the list.
    over: gtk::Overlay,
    entry: gtk::Entry,
    chips: gtk::Box,
    echo: gtk::Label,
    heading: gtk::Label,
    list: gtk::ListBox,
    footer: gtk::Label,
    keymap: RefCell<Keymap>,
    /// Every place the bar can go, read when it opens.
    places: Rc<RefCell<Vec<Place>>>,
    /// Folder names by id, for a result's `in:`.
    folders: Rc<RefCell<Vec<(MailboxId, String)>>>,
    /// Who a typed name can mean, read with the places.
    names: Rc<RefCell<postio_ui::names::Names>>,
    /// What each row of the list runs, in order.
    rows: Rc<RefCell<Vec<Row>>>,
    /// The words typed when a hit was opened, until the window takes them.
    held: RefCell<Option<String>>,
    /// Moves with every keystroke, so a late answer to an earlier one is
    /// dropped.
    generation: Rc<Cell<u64>>,
    open: Cell<bool>,
    /// The top bar's field, which the bar's input stands in for while open.
    field: RefCell<Option<gtk::Widget>>,
    handler: RefCell<Option<Handler>>,
    /// This bar, weakly: what a future comes back to.
    me: RefCell<std::rc::Weak<Bar>>,
    /// The pinned saved searches, in order: each name and its query.
    saved: RefCell<Vec<(String, String)>>,
    /// Whether the places have been read since the bar last opened.
    places_known: Cell<bool>,
    /// The saved row across the top.
    saved_row: gtk::Box,
    /// Whether any digest rule can hold mail: a result then says where
    /// held mail waits (US10 scenario 7).
    digesting: Rc<Cell<bool>>,
    /// The plain words typed, while `Tab` has stepped into the chips they
    /// were lowered to: what `Ctrl+Backspace` goes back to (T086).
    words: RefCell<Option<String>>,
    /// The chip being edited, while the entry holds the chips.
    editing: Cell<Option<usize>>,
    /// The chips shown now, in order.
    shown_chips: RefCell<Vec<String>>,
    /// Which order a search's results come back in; kept across queries
    /// while the bar is up, and relevance each time it opens.
    order: Cell<postio_search::ResultOrder>,
    /// Where the results go in the list: the index after the search row.
    results_at: Cell<usize>,
    /// While results are being drawn, where the next row is inserted.
    inserting: Cell<Option<usize>>,
    /// Where the highlight goes once the results are drawn.
    pending: Cell<Pending>,
    /// The generation the rows now show the results of.
    loaded: Cell<u64>,
    /// The results' heading, which is a row of the list.
    result_heading: RefCell<String>,
    /// Every correspondent, for `@`.
    contacts: Rc<RefCell<Vec<postio_model::Contact>>>,
    /// Which account each mailbox belongs to, when more than one is enabled.
    owners: Rc<RefCell<Vec<(MailboxId, String)>>>,
    /// Where the keyboard goes when the bar closes: the list, once the
    /// window says which widget that is.
    home: RefCell<Option<gtk::Widget>>,
}

/// Where the highlight goes when the results have been drawn.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum Pending {
    /// Stay where it is.
    #[default]
    Nowhere,
    /// The first hit: Return on the search row asked to be among them.
    FirstHit,
    /// The order row, which was just run.
    Order,
    /// The hit that was opened and has just been closed: the person is
    /// back among the results where they left them.
    Hit(MessageId),
}

impl Bar {
    /// The commands the bar has a control for beyond its command rows: a
    /// pill for each saved search, and a row for each message found.
    pub fn controls() -> Vec<CommandId> {
        SAVED
            .into_iter()
            .chain([CommandId::OpenMessage, CommandId::ToggleResultOrder])
            .collect()
    }

    /// A closed bar, reading through `client`, its keys from `keymap`.
    pub fn new(client: Client, keymap: &Keymap) -> Rc<Self> {
        let icon = gtk::Image::from_icon_name("system-search-symbolic");
        icon.set_accessible_role(gtk::AccessibleRole::Presentation);
        let chips = gtk::Box::new(gtk::Orientation::Horizontal, S1);
        chips.add_css_class("focus-bar-chips");
        let entry = gtk::Entry::new();
        entry.set_hexpand(true);
        entry.set_has_frame(false);
        entry.set_placeholder_text(Some("Search mail, go to a folder, or run a command"));
        entry.add_css_class("focus-bar-entry");
        let input = gtk::Box::new(gtk::Orientation::Horizontal, S2);
        input.add_css_class("focus-bar-input");
        input.append(&icon);
        input.append(&chips);
        input.append(&entry);
        if let Some(key) = postio_ui::hints::key(keymap, CommandId::Back) {
            input.append(&keyhint::cap(&key));
        }

        let echo = gtk::Label::new(None);
        echo.add_css_class("dim-label");
        echo.add_css_class("focus-bar-echo");
        echo.set_xalign(0.0);
        echo.set_visible(false);
        let heading = gtk::Label::new(None);
        heading.add_css_class("focus-bar-heading");
        heading.set_xalign(0.0);
        heading.set_visible(false);
        let list = gtk::ListBox::new();
        list.add_css_class("focus-bar-results");
        list.set_selection_mode(gtk::SelectionMode::Single);
        let scrolled = gtk::ScrolledWindow::builder()
            .child(&list)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .propagate_natural_height(true)
            .max_content_height(520)
            .build();
        let footer = gtk::Label::new(None);
        footer.add_css_class("dim-label");
        footer.add_css_class("focus-bar-footer");
        footer.set_xalign(0.0);

        let saved_row = gtk::Box::new(gtk::Orientation::Horizontal, S2);
        saved_row.add_css_class("focus-bar-saved");
        saved_row.set_visible(false);
        // The bar opens in place (C24): its input is drawn where the top
        // bar's own field is -- same width, centred the same way -- and the
        // results hang below it as a panel of their own.
        input.set_halign(gtk::Align::Center);
        input.set_width_request(FIELD_WIDTH);
        let panel = gtk::Box::new(gtk::Orientation::Vertical, S2);
        panel.add_css_class("focus-bar-panel");
        panel.append(&saved_row);
        panel.append(&echo);
        panel.append(&heading);
        panel.append(&scrolled);
        panel.append(&footer);
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.add_css_class("focus-bar");
        root.set_width_request(WIDTH);
        root.set_halign(gtk::Align::Center);
        root.set_valign(gtk::Align::Start);
        root.append(&input);
        root.append(&panel);
        // The list dims behind the bar (screens 07-09). The dimming is
        // paint only: it takes no pointer, so the list under it behaves as
        // it did.
        let scrim = gtk::Box::new(gtk::Orientation::Vertical, 0);
        scrim.add_css_class("focus-bar-scrim");
        scrim.set_can_target(false);
        let over = gtk::Overlay::new();
        over.set_child(Some(&scrim));
        over.add_overlay(&root);
        over.set_visible(false);

        let bar = Rc::new(Bar {
            client,
            root,
            over,
            entry,
            chips,
            echo,
            heading,
            list,
            footer,
            keymap: RefCell::new(keymap.clone()),
            places: Rc::default(),
            names: Rc::default(),
            folders: Rc::default(),
            rows: Rc::default(),
            held: RefCell::default(),
            generation: Rc::default(),
            open: Cell::new(false),
            field: RefCell::default(),
            handler: RefCell::default(),
            me: RefCell::default(),
            saved: RefCell::default(),
            places_known: Cell::new(false),
            saved_row,
            digesting: Rc::default(),
            words: RefCell::default(),
            editing: Cell::new(None),
            shown_chips: RefCell::default(),
            order: Cell::default(),
            results_at: Cell::new(0),
            inserting: Cell::new(None),
            pending: Cell::default(),
            loaded: Cell::new(0),
            result_heading: RefCell::default(),
            contacts: Rc::default(),
            owners: Rc::default(),
            home: RefCell::default(),
        });
        bar.me.replace(Rc::downgrade(&bar));
        let weak = Rc::downgrade(&bar);
        // The scrim is the bar's: a press on it, outside the bar, closes
        // the bar as Escape does, rather than reaching the list beneath.
        let outside = gtk::GestureClick::new();
        outside.connect_pressed({
            let weak = weak.clone();
            move |_, _, x, y| {
                let Some(bar) = weak.upgrade() else {
                    return;
                };
                let picked = bar.over.pick(x, y, gtk::PickFlags::DEFAULT);
                let inside = picked.is_some_and(|widget| {
                    widget == *bar.root.upcast_ref::<gtk::Widget>() || widget.is_ancestor(&bar.root)
                });
                if !inside {
                    bar.close();
                }
            }
        });
        bar.over.add_controller(outside);
        bar.entry.connect_changed({
            let weak = weak.clone();
            move |entry| {
                if let Some(bar) = weak.upgrade() {
                    bar.update(&entry.text());
                }
            }
        });
        bar.entry.connect_activate({
            let weak = weak.clone();
            move |_| {
                if let Some(bar) = weak.upgrade() {
                    bar.run_selected();
                }
            }
        });
        bar.list.connect_row_activated({
            let weak = weak.clone();
            move |_, row| {
                if let (Some(bar), Ok(index)) = (weak.upgrade(), usize::try_from(row.index())) {
                    bar.run(index);
                }
            }
        });
        let keys = gtk::EventControllerKey::new();
        keys.connect_key_pressed({
            let weak = weak.clone();
            move |_, key, _, state| match weak.upgrade() {
                Some(bar) if bar.press(key, state) => glib::Propagation::Stop,
                _ => glib::Propagation::Proceed,
            }
        });
        bar.entry.add_controller(keys);
        bar.set_keymap(keymap);
        bar
    }

    /// The top bar's field: the input takes its place while the bar is open,
    /// so the field is hidden (still laid out) until the bar closes.
    pub fn set_field(&self, field: gtk::Widget) {
        field.set_opacity(if self.open.get() { 0.0 } else { 1.0 });
        self.field.replace(Some(field));
    }

    fn show_field(&self, shown: bool) {
        if let Some(field) = self.field.borrow().as_ref() {
            field.set_opacity(if shown { 1.0 } else { 0.0 });
        }
    }

    /// The bar, to lay over the window.
    pub fn widget(&self) -> &gtk::Overlay {
        &self.over
    }

    /// Run `handler` with what a row asks for when it is run.
    pub fn connect_action(&self, handler: impl Fn(BarAction) + 'static) {
        self.handler.replace(Some(Rc::new(handler)));
    }

    /// Read every key the bar shows from `keymap`.
    pub fn set_keymap(&self, keymap: &Keymap) {
        self.keymap.replace(keymap.clone());
        self.show_saved();
        self.footer.set_text(
            "\u{2191}\u{2193} move \u{b7} \u{21b5} open / run \u{b7} > commands only \u{b7} Local index",
        );
    }

    /// The pinned saved searches, in order: each name and its query. The
    /// first four run on `Alt+1`-`Alt+4`.
    pub fn set_saved(&self, saved: Vec<(String, String)>) {
        self.saved.replace(saved);
        self.show_saved();
    }

    fn show_saved(&self) {
        while let Some(child) = self.saved_row.first_child() {
            self.saved_row.remove(&child);
        }
        let saved = self.saved.borrow();
        self.saved_row.set_visible(!saved.is_empty());
        if saved.is_empty() {
            return;
        }
        let title = gtk::Label::new(Some("Saved"));
        title.add_css_class("dim-label");
        self.saved_row.append(&title);
        let keymap = self.keymap.borrow();
        for (index, (name, _)) in saved.iter().enumerate() {
            let said = gtk::Box::new(gtk::Orientation::Horizontal, S1);
            said.append(&gtk::Label::new(Some(name)));
            if let Some(key) = SAVED
                .get(index)
                .and_then(|command| postio_ui::hints::key(&keymap, *command))
            {
                said.append(&keyhint::cap(&key));
            }
            let pill = gtk::Button::new();
            pill.add_css_class("focus-bar-saved-pill");
            pill.set_child(Some(&said));
            pill.set_focus_on_click(false);
            let weak = self.self_weak();
            pill.connect_clicked(move |_| {
                if let Some(bar) = weak.upgrade() {
                    bar.open_saved(index);
                }
            });
            self.saved_row.append(&pill);
        }
    }

    /// Open the bar on the `index`th saved search, and run it.
    pub fn open_saved(&self, index: usize) -> bool {
        let Some((_, query)) = self.saved.borrow().get(index).cloned() else {
            return false;
        };
        self.open();
        self.set_text(&query);
        true
    }

    /// Say whether a digest rule can hold mail, so results ask where held
    /// mail waits; with no rule, nothing is held and nothing is asked.
    pub fn set_digesting(&self, digesting: bool) {
        self.digesting.set(digesting);
    }

    /// Hand the keyboard to `home` whenever the bar closes.
    pub fn set_home(&self, home: gtk::Widget) {
        self.home.replace(Some(home));
    }

    /// Open the bar, empty, and read the places it can go.
    pub fn open(&self) {
        self.open.set(true);
        self.places_known.set(false);
        self.words.replace(None);
        self.editing.set(None);
        self.order.set(postio_search::ResultOrder::Relevance);
        self.over.set_visible(true);
        self.show_field(false);
        self.entry.set_text("");
        self.update("");
        self.entry.grab_focus();
        self.read_places();
    }

    /// Open the bar in command mode (`Ctrl K`): the command prefix already
    /// typed, so only commands are offered, and what is typed after it
    /// narrows them.
    pub fn open_commands(&self) {
        self.open();
        self.set_text(&finder::COMMANDS_ONLY.to_string());
    }

    /// Close the bar. Nothing it showed is kept.
    pub fn close(&self) {
        self.open.set(false);
        // The keyboard leaves the entry with the bar: left in a hidden
        // entry, every single-letter key after it would be taken for typing.
        if let Some(root) = self.entry.root()
            && root
                .focus()
                .is_some_and(|focus| focus.is_ancestor(&self.root) || focus == self.root)
        {
            // To the list when the window said where that is, to nothing
            // otherwise: a window's own keys are heard from either.
            let home = self.home.borrow().clone();
            root.set_focus(home.as_ref());
        }
        self.over.set_visible(false);
        self.show_field(true);
        self.generation.set(self.generation.get() + 1);
    }

    /// Run the search row: search for what is typed, as choosing
    /// "Search mail for …" does.
    pub fn run_search(&self) {
        let at = self
            .rows
            .borrow()
            .iter()
            .position(|row| *row == Row::Search);
        if let Some(at) = at {
            self.run(at);
        }
    }

    /// Run the highlighted row, as Return does.
    pub fn run_search_row(&self) {
        self.run_selected();
    }

    /// Whether the places the bar can go have been read since it opened.
    pub fn places_known(&self) -> bool {
        self.places_known.get()
    }

    /// Whether the bar is up.
    pub fn is_open(&self) -> bool {
        self.open.get()
    }

    /// Handle one key in the bar's entry, as its key controller does;
    /// `true` when the bar used it. The arrows walk the results; `Tab`
    /// steps into the chips, and on from chip to chip.
    pub fn press(&self, key: gtk::gdk::Key, state: gtk::gdk::ModifierType) -> bool {
        match key {
            gtk::gdk::Key::Down => self.step(1),
            gtk::gdk::Key::Up => self.step(-1),
            gtk::gdk::Key::Tab if state.is_empty() => return self.next_chip(),
            // Typing wins: `O` is a letter for the entry. The order key
            // carries `alt` and is the window's command.
            _ => return false,
        }
        true
    }

    /// Switch the results between relevance and date, and ask again.
    pub fn toggle_order(&self) {
        self.order.set(match self.order.get() {
            postio_search::ResultOrder::Relevance => postio_search::ResultOrder::Newest,
            postio_search::ResultOrder::Newest => postio_search::ResultOrder::Relevance,
        });
        self.pending.set(Pending::Order);
        self.search_typed();
    }

    /// The bar's input, where the words are typed.
    pub fn input(&self) -> gtk::Widget {
        self.entry.clone().upcast()
    }

    /// What the entry holds now.
    pub fn typed(&self) -> String {
        self.entry.text().to_string()
    }

    /// The query a search would ask: the chips the words were lowered to,
    /// or the words themselves when they lower to no chip.
    pub fn query(&self) -> String {
        let chips = self.shown_chips.borrow();
        if chips.is_empty() {
            self.entry.text().trim().to_owned()
        } else {
            chips.join(" ")
        }
    }

    /// `Tab`: from the words into the first chip, then to the next; the
    /// entry holds the chips and selects the one being edited. `false`
    /// with no chip to step into.
    fn next_chip(&self) -> bool {
        let chips = self.shown_chips.borrow().clone();
        if chips.is_empty() {
            return false;
        }
        let next = match self.editing.get() {
            None => {
                self.words.replace(Some(self.entry.text().to_string()));
                0
            }
            Some(at) => (at + 1) % chips.len(),
        };
        self.editing.set(Some(next));
        let query = chips.join(" ");
        if self.entry.text() != query {
            self.entry.set_text(&query);
        } else {
            self.show_editing();
        }
        let start: usize = chips[..next]
            .iter()
            .map(|chip| chip.chars().count() + 1)
            .sum();
        let end = start + chips[next].chars().count();
        self.entry.grab_focus();
        self.entry.select_region(start as i32, end as i32);
        true
    }

    /// `Ctrl+Backspace`: back from the chips to the words they were
    /// lowered from; whether there were words to go back to.
    pub fn back_to_words(&self) -> bool {
        let Some(words) = self.words.take() else {
            return false;
        };
        self.editing.set(None);
        self.entry.set_text(&words);
        self.entry.set_position(-1);
        true
    }

    /// The echo while a chip is edited: the words typed, which chip, and
    /// the keys that move on and go back.
    fn show_editing(&self) {
        let (Some(at), Some(words)) = (self.editing.get(), self.words.borrow().clone()) else {
            return;
        };
        let chips = self.shown_chips.borrow().clone();
        let Some(chip) = chips.get(at.min(chips.len().saturating_sub(1))) else {
            return;
        };
        let editing = match chip.split_once(':') {
            Some((operator, _)) => format!("{operator}:"),
            None => chip.clone(),
        };
        let keymap = self.keymap.borrow();
        let mut hints = vec![postio_ui::hints::fixed(
            "Tab",
            "next chip",
            "Tab moves between the bar's chips: the toolkit's focus order, not a command",
        )];
        hints.extend(postio_ui::hints::hint(
            &keymap,
            CommandId::BackToWords,
            "back to plain words",
        ));
        self.echo.set_text(&format!(
            "You typed \u{201c}{words}\u{201d} \u{b7} editing {editing} \u{b7} {}",
            postio_ui::hints::line(&hints)
        ));
        let mut child = self.chips.first_child();
        let mut index = 0;
        while let Some(label) = child {
            child = label.next_sibling();
            if index == at {
                label.add_css_class("focus-bar-chip-editing");
            } else {
                label.remove_css_class("focus-bar-chip-editing");
            }
            index += 1;
        }
    }

    /// Put `text` in the bar, as typing it would.
    pub fn set_text(&self, text: &str) {
        self.entry.set_text(text);
        self.entry.set_position(-1);
    }

    /// The messages the results list, with their subjects, in the bar's
    /// order: what `j` and `k` walk once one is opened.
    pub fn hits(&self) -> Vec<(MessageId, String)> {
        self.rows
            .borrow()
            .iter()
            .filter_map(|row| match row {
                Row::Message { message, subject } => Some((*message, subject.clone())),
                _ => None,
            })
            .collect()
    }

    /// Reopen the bar on `typed`, the highlight going to the hit `opened`
    /// once the results are drawn: where a closed hit returns to.
    pub fn reopen(&self, typed: &str, opened: Option<MessageId>) {
        self.open();
        self.set_text(typed);
        if let Some(opened) = opened {
            self.pending.set(Pending::Hit(opened));
        }
    }

    /// What was typed when a hit was opened, once: the window reopens the
    /// bar with it when that message closes.
    pub fn take_held(&self) -> Option<String> {
        self.held.take()
    }

    /// The results' heading.
    pub fn heading(&self) -> String {
        if self.heading.is_visible() {
            self.heading.text().to_string()
        } else {
            self.result_heading.borrow().clone()
        }
    }

    /// Every command the bar can list: what typing its name finds.
    pub fn commands(&self) -> Vec<CommandId> {
        let state = postio_core::Availability {
            scope: AccountScope::Unified,
            store_open: true,
            frontend: Frontend::Focus,
        };
        let keymap = self.keymap.borrow();
        let mut entries = postio_ui::palette::entries(&keymap, Context::List, state, "");
        rules::add_account_verbs(&mut entries, &keymap, state, "");
        entries
            .into_iter()
            .filter_map(|entry| match entry.id {
                ActionId::Builtin(command) => Some(command),
                ActionId::Ext(_) => None,
            })
            .collect()
    }

    /// The subjects of the messages listed, top to bottom.
    pub fn result_subjects(&self) -> Vec<String> {
        self.rows
            .borrow()
            .iter()
            .filter_map(|row| match row {
                Row::Message { subject, .. } => Some(subject.clone()),
                _ => None,
            })
            .collect()
    }

    /// The highlighted row, as a storyboard observes it: what kind of row
    /// it is, and the message it would open when it is one.
    ///
    /// The text is the row's own labels, as a person reads it ("Sorted by
    /// relevance"), and a message's subject.
    pub fn highlighted(&self) -> Option<(&'static str, Option<MessageId>, String)> {
        let selected = self.list.selected_row()?;
        let index = usize::try_from(selected.index()).ok()?;
        let said = || {
            let mut said = Vec::new();
            let mut pending: Vec<gtk::Widget> = vec![selected.clone().upcast()];
            while let Some(widget) = pending.pop() {
                if let Some(label) = widget.downcast_ref::<gtk::Label>()
                    && widget.is_visible()
                    && !label.text().is_empty()
                {
                    said.push(label.text().to_string());
                }
                let mut child = widget.last_child();
                while let Some(next) = child {
                    child = next.prev_sibling();
                    pending.push(next);
                }
            }
            said.join(" ")
        };
        let rows = self.rows.borrow();
        Some(match rows.get(index)? {
            Row::Heading => ("heading", None, said()),
            Row::Message { message, subject } => ("message", Some(*message), subject.clone()),
            Row::Command(_) => ("command", None, said()),
            Row::Place(_, name) => ("place", None, name.clone()),
            Row::Search => ("search", None, said()),
            Row::Instead(word) => ("instead", None, word.clone()),
            Row::Order => ("order", None, said()),
            Row::Correspondent(_) => ("correspondent", None, said()),
        })
    }

    /// The chips the words were lowered to.
    pub fn chips(&self) -> Vec<String> {
        let mut said = Vec::new();
        let mut child = self.chips.first_child();
        while let Some(chip) = child {
            child = chip.next_sibling();
            if let Some(label) = chip.downcast_ref::<gtk::Label>() {
                said.push(label.text().to_string());
            }
        }
        said
    }

    /// Everything the bar says, as a person reads it.
    pub fn texts(&self) -> Vec<String> {
        let mut said = Vec::new();
        let mut stack = vec![self.root.clone().upcast::<gtk::Widget>()];
        while let Some(widget) = stack.pop() {
            if !widget.is_visible() {
                continue;
            }
            if let Some(label) = widget.downcast_ref::<gtk::Label>() {
                let text = label.text().to_string();
                if !text.is_empty() {
                    said.push(text);
                }
            }
            let mut child = widget.last_child();
            while let Some(next) = child {
                child = next.prev_sibling();
                stack.push(next);
            }
        }
        said
    }

    /// Answer what is typed: a folder's conversations after `in:`, and
    /// otherwise the blended commands, places and search.
    fn update(&self, text: &str) {
        let generation = self.generation.get() + 1;
        self.generation.set(generation);
        self.clear_rows();
        let text = text.to_owned();
        let typed = text.trim();
        self.echo.set_visible(!typed.is_empty());
        self.echo.set_text(&rules::echo(typed));
        match rules::route(typed) {
            rules::Route::Folder(name) => {
                self.show_chips(&[]);
                self.show_folder(name, generation);
                return;
            }
            rules::Route::Plain => {
                self.show_blend(typed);
                self.show_chips(&[]);
                self.heading.set_visible(false);
                return;
            }
            rules::Route::Correspondent(name) => {
                self.show_chips(&[]);
                self.heading.set_visible(false);
                let hits = finder::contacts(&self.contacts.borrow(), name);
                for line in rules::correspondent_lines(&hits) {
                    self.append_line(&line);
                }
                return;
            }
            rules::Route::Blend => self.show_blend(typed),
        }
        let parsed = self.lowered(typed);
        // Words that name what they want are a search, shown as its chips
        // (screen 07); a plain word makes none. Either way the results come
        // as it is typed, under the search row (screen 07 d).
        match rules::chips(&parsed) {
            Some(chips) => {
                self.show_chips(&chips);
                self.show_editing();
            }
            None => self.show_chips(&[]),
        }
        self.heading.set_visible(false);
        self.search(parsed, generation);
    }

    /// `typed`, read as plain English against today and the address book.
    fn lowered(&self, typed: &str) -> postio_search::ParsedQuery {
        let names = self.names.borrow();
        postio_search::natural::lower(typed, postio_ui::clock::now().date_naive(), &|name| {
            names.lookup(name)
        })
    }

    /// Search for what is typed now.
    fn search_typed(&self) {
        let typed = self.entry.text();
        let typed = typed.trim();
        if typed.is_empty() {
            return;
        }
        let parsed = self.lowered(typed);
        self.search(parsed, self.generation.get());
    }

    /// The chips the words were lowered to, in order.
    fn show_chips(&self, chips: &[String]) {
        self.shown_chips.replace(chips.to_vec());
        while let Some(child) = self.chips.first_child() {
            self.chips.remove(&child);
        }
        for chip in chips {
            let label = gtk::Label::new(Some(chip));
            label.add_css_class("focus-bar-chip");
            self.chips.append(&label);
        }
    }

    /// The commands and places `typed` matches, and the search row.
    fn show_blend(&self, typed: &str) {
        let places = self.places.borrow();
        let state = postio_core::Availability {
            scope: AccountScope::Unified,
            store_open: true,
            frontend: Frontend::Focus,
        };
        let keymap = self.keymap.borrow();
        let mut blend = finder::blend(typed, &places, &keymap, Context::List, state);
        let words = typed.strip_prefix(finder::COMMANDS_ONLY).unwrap_or(typed);
        if typed.starts_with(finder::COMMANDS_ONLY) || !words.trim().is_empty() {
            rules::add_account_verbs(&mut blend.commands, &keymap, state, words.trim());
        }
        for line in rules::blend_lines(&blend) {
            self.append_line(&line);
        }
        // The results go under the search row.
        let after = self
            .rows
            .borrow()
            .iter()
            .position(|row| *row == Row::Search)
            .map_or(self.rows.borrow().len(), |at| at + 1);
        self.results_at.set(after);
    }

    /// Search this machine's index for `parsed`, and list what it finds
    /// under the search row, one row per conversation.
    fn search(&self, parsed: postio_search::ParsedQuery, generation: u64) {
        let client = self.client.clone();
        let current = Rc::clone(&self.generation);
        let digesting = self.digesting.get();
        let order = self.order.get();
        let weak = self.self_weak();
        glib::spawn_future_local(async move {
            let search = client.search_hits(
                AccountScope::Unified,
                parsed,
                postio_search::facets::Scope::AllMail,
                order,
                0,
            );
            // POSTIO-GLIB-SAFE: a client call is a oneshot receive; the host
            // answers on its own runtime (ADR 0041).
            let found = search.await;
            if current.get() != generation {
                return;
            }
            let Ok(Some(results)) = found else {
                return;
            };
            let rows = rules::conversations(results.hits);
            // Held mail says where it waits, not the folder it is filed in.
            let held = if digesting {
                let ids = rows.iter().map(|hit| hit.message_id).collect();
                // POSTIO-GLIB-SAFE: as the search's.
                let read = client.held(ids).await;
                if current.get() != generation {
                    return;
                }
                read.unwrap_or_default()
            } else {
                Vec::new()
            };
            if let Some(bar) = weak.upgrade() {
                bar.show_results(rows, results.instead, order, &held, generation);
            }
        });
    }

    /// Draw a search's results under the search row, replacing the last.
    fn show_results(
        &self,
        rows: Vec<postio_search::SearchHit>,
        instead: Option<postio_search::Instead>,
        order: postio_search::ResultOrder,
        held: &[(MessageId, String, bool)],
        generation: u64,
    ) {
        self.clear_results();
        self.inserting.set(Some(self.results_at.get()));
        let heading = rules::results_heading(rows.len());
        self.result_heading.replace(heading.clone());
        self.append_heading(&heading);
        // The list is for another word than the box holds, and says so
        // (ADR 0037); the typed word is one row away, quoted, which is how
        // the query language says "this word, exactly".
        if let Some(instead) = &instead {
            self.append_heading(&rules::showing_results_for(&instead.term));
            let (title, detail) = rules::search_instead(&instead.typed);
            self.append_row(
                Row::Instead(instead.typed.clone()),
                &title,
                Some(detail),
                None,
            );
        }
        if !rows.is_empty() {
            let (title, detail) = rules::order_words(order);
            let key = postio_ui::hints::key(&self.keymap.borrow(), CommandId::ToggleResultOrder);
            self.append_row(Row::Order, &title, Some(&detail), key.as_deref());
        }
        let names = self.folders.borrow().clone();
        let owners = self.owners.borrow().clone();
        for hit in rows {
            let subject = hit.subject.clone().unwrap_or_default();
            let place = rules::result_place(&hit, held, &names);
            let place = rules::result_place_in(&hit, place, &owners);
            self.append_message(
                hit.message_id,
                hit.from.as_ref().map(said_of),
                &subject,
                hit.preview.as_deref(),
                place.as_deref(),
                hit.received_at,
            );
        }
        self.inserting.set(None);
        self.loaded.set(generation);
        self.apply_pending();
    }

    /// Move the highlight where the last run asked it to go.
    fn apply_pending(&self) {
        let wanted = match self.pending.take() {
            Pending::Nowhere => return,
            Pending::FirstHit => self
                .rows
                .borrow()
                .iter()
                .position(|row| matches!(row, Row::Message { .. })),
            Pending::Order => self.rows.borrow().iter().position(|row| *row == Row::Order),
            Pending::Hit(wanted) => self.rows.borrow().iter().position(
                |row| matches!(row, Row::Message { message, .. } if *message == wanted),
            ),
        };
        if let Some(row) = wanted.and_then(|at| self.list.row_at_index(at as i32)) {
            self.list.select_row(Some(&row));
        }
    }

    /// Take away the results, leaving the rows above them.
    fn clear_results(&self) {
        let at = self.results_at.get();
        while let Some(row) = self.list.row_at_index(at as i32) {
            self.list.remove(&row);
        }
        self.rows.borrow_mut().truncate(at);
        self.result_heading.replace(String::new());
    }

    /// `in:` and `name`: the first folder whose name starts with it, and its
    /// conversations newest first (screen 08).
    fn show_folder(&self, name: &str, generation: u64) {
        let folder = rules::folder_for(&self.places.borrow(), name);
        let Some((name, mailbox)) = folder else {
            self.heading.set_visible(false);
            return;
        };
        let client = self.client.clone();
        let current = Rc::clone(&self.generation);
        let weak = self.self_weak();
        glib::spawn_future_local(async move {
            let scope = ListScope::Mailbox(mailbox);
            // POSTIO-GLIB-SAFE: a client call is a oneshot receive; the host
            // answers on its own runtime (ADR 0041).
            let count = client.list_count(scope).await;
            let read = client.list_page(PageRequest {
                scope,
                offset: 0,
                limit: rules::FOLDER_ROWS,
            });
            // POSTIO-GLIB-SAFE: as above.
            let page = read.await;
            if current.get() != generation {
                return;
            }
            let Some(bar) = weak.upgrade() else {
                return;
            };
            bar.clear_rows();
            bar.heading
                .set_text(&rules::folder_heading(&name, count.unwrap_or(0)));
            bar.heading.set_visible(true);
            match page {
                Ok(ListPage::Threads(page)) => {
                    for thread in page.rows {
                        let from = thread.representative.from.as_ref().map(said_of);
                        bar.append_message(
                            thread.representative.id,
                            from,
                            &thread.subject.clone().unwrap_or_default(),
                            thread.representative.preview.as_deref(),
                            None,
                            thread.last_at,
                        );
                    }
                }
                Ok(ListPage::Messages(page)) => {
                    for message in page.rows {
                        let from = message.from.as_ref().map(said_of);
                        bar.append_message(
                            message.id,
                            from,
                            &message.subject.clone().unwrap_or_default(),
                            message.preview.as_deref(),
                            None,
                            message.received_at,
                        );
                    }
                }
                Err(error) => tracing::warn!(%error, "the bar could not list a folder: {error}"),
            }
        });
    }

    /// Read the places the bar can go: every account's mailboxes and
    /// folders, its labels, and the correspondents a typed name can mean.
    fn read_places(&self) {
        let client = self.client.clone();
        let (places, folders) = (Rc::clone(&self.places), Rc::clone(&self.folders));
        let directory = Rc::clone(&self.names);
        let weak = self.self_weak();
        glib::spawn_future_local(async move {
            // POSTIO-GLIB-SAFE: a client call is a oneshot receive; the host
            // answers on its own runtime (ADR 0041).
            let Ok(accounts) = client.accounts().await else {
                return;
            };
            let mut found = Vec::new();
            let mut names = Vec::new();
            let mut correspondents = Vec::new();
            let mut owned = Vec::new();
            for account in accounts.iter().filter(|account| account.enabled) {
                // POSTIO-GLIB-SAFE: as above.
                let read = client.mailboxes(account.id).await;
                if let Ok(mailboxes) = read {
                    for mailbox in mailboxes {
                        owned.push((mailbox.id, account.address.address.clone()));
                        names.push((mailbox.id, postio_ui::places::place_name(&mailbox)));
                        found.push(postio_ui::places::mailbox_place(&mailbox));
                    }
                }
                // POSTIO-GLIB-SAFE: as above.
                let read = client.correspondents(account.id).await;
                correspondents.extend(read.unwrap_or_default());
                // POSTIO-GLIB-SAFE: as above.
                let read = client.labels(account.id).await;
                if let Ok(labels) = read {
                    for label in labels {
                        found.push(postio_ui::places::label_place(&label));
                    }
                }
            }
            places.replace(found);
            folders.replace(names);
            directory.replace(postio_ui::names::Names::new(&correspondents));
            // Which account a result is from is said only where there is
            // more than one to tell apart.
            let several = accounts.iter().filter(|account| account.enabled).count() > 1;
            if let Some(bar) = weak.upgrade() {
                bar.owners.replace(if several { owned } else { Vec::new() });
                bar.contacts.replace(correspondents);
            }
            // `in:` typed before the places landed is answered again, now
            // that it has folders to complete. Nothing else needs them to
            // answer, and a search is not asked for twice.
            if let Some(bar) = weak.upgrade().filter(|bar| bar.is_open()) {
                bar.places_known.set(true);
                let typed = bar.entry.text();
                if typed.trim().starts_with("in:") {
                    bar.update(&typed);
                }
            }
        });
    }

    /// A weak handle on this bar, for a future to come back to.
    fn self_weak(&self) -> std::rc::Weak<Bar> {
        self.me.borrow().clone()
    }

    fn clear_rows(&self) {
        self.results_at.set(0);
        self.result_heading.replace(String::new());
        while let Some(child) = self.list.first_child() {
            self.list.remove(&child);
        }
        self.rows.borrow_mut().clear();
    }

    fn append_line(&self, line: &Line) {
        match line {
            Line::Heading(title) => self.append_heading(title),
            Line::Row {
                row,
                title,
                detail,
                key,
            } => self.append_row(row.clone(), title, detail.as_deref(), key.as_deref()),
        }
    }

    fn append_heading(&self, title: &str) {
        let label = gtk::Label::new(Some(title));
        label.add_css_class("focus-bar-section");
        label.set_xalign(0.0);
        let row = gtk::ListBoxRow::new();
        row.set_child(Some(&label));
        row.set_selectable(false);
        row.set_activatable(false);
        self.put(&row, Row::Heading);
    }

    fn append_row(&self, target: Row, title: &str, detail: Option<&str>, key: Option<&str>) {
        let line = gtk::Box::new(gtk::Orientation::Horizontal, S3);
        let name = gtk::Label::new(Some(title));
        name.set_xalign(0.0);
        line.append(&name);
        if let Some(detail) = detail {
            let detail = gtk::Label::new(Some(detail));
            detail.add_css_class("dim-label");
            detail.set_xalign(0.0);
            detail.set_hexpand(true);
            detail.set_ellipsize(pango::EllipsizeMode::End);
            line.append(&detail);
        } else {
            name.set_hexpand(true);
        }
        if let Some(key) = key {
            line.append(&keyhint::cap(key));
        }
        let row = gtk::ListBoxRow::new();
        row.add_css_class("focus-bar-row");
        row.set_child(Some(&line));
        self.put(&row, target);
        self.select_first();
    }

    /// Add `row`, which runs `target`: at the end, or where the results are
    /// being drawn.
    fn put(&self, row: &gtk::ListBoxRow, target: Row) {
        match self.inserting.get() {
            Some(at) => {
                self.list.insert(row, at as i32);
                self.rows.borrow_mut().insert(at, target);
                self.inserting.set(Some(at + 1));
            }
            None => {
                self.list.append(row);
                self.rows.borrow_mut().push(target);
            }
        }
    }

    fn append_message(
        &self,
        message: MessageId,
        from: Option<String>,
        subject: &str,
        preview: Option<&str>,
        place: Option<&str>,
        at: chrono::DateTime<chrono::Utc>,
    ) {
        let line = gtk::Box::new(gtk::Orientation::Horizontal, S3);
        let sender = gtk::Label::new(from.as_deref());
        sender.add_css_class("focus-bar-sender");
        sender.set_width_chars(18);
        sender.set_max_width_chars(18);
        sender.set_ellipsize(pango::EllipsizeMode::End);
        sender.set_xalign(0.0);
        let title = gtk::Label::new(Some(subject));
        title.add_css_class("focus-bar-subject");
        let first = gtk::Label::new(preview);
        first.add_css_class("dim-label");
        first.set_ellipsize(pango::EllipsizeMode::End);
        first.set_hexpand(true);
        first.set_xalign(0.0);
        line.append(&sender);
        line.append(&title);
        line.append(&first);
        if let Some(place) = place {
            let place = gtk::Label::new(Some(place));
            place.add_css_class("focus-bar-place");
            line.append(&place);
        }
        let date = gtk::Label::new(Some(&postio_ui::row::timestamp(
            at,
            postio_ui::clock::now(),
        )));
        date.add_css_class("dim-label");
        line.append(&date);
        let row = gtk::ListBoxRow::new();
        row.add_css_class("focus-bar-row");
        row.set_child(Some(&line));
        self.put(
            &row,
            Row::Message {
                message,
                subject: subject.to_owned(),
            },
        );
        self.select_first();
    }

    /// Select the first row that runs something, when nothing is selected.
    fn select_first(&self) {
        if self.list.selected_row().is_some() {
            return;
        }
        let first = self.rows.borrow().iter().position(Row::is_selectable);
        if let Some(row) = first.and_then(|at| self.list.row_at_index(at as i32)) {
            self.list.select_row(Some(&row));
        }
    }

    /// Move the selection `by` rows, skipping headings.
    fn step(&self, by: i32) {
        let count = self.rows.borrow().len() as i32;
        let mut at = self.list.selected_row().map_or(-1, |row| row.index());
        loop {
            at += by;
            if at < 0 || at >= count {
                return;
            }
            if self.rows.borrow()[at as usize].is_selectable() {
                break;
            }
        }
        if let Some(row) = self.list.row_at_index(at) {
            self.list.select_row(Some(&row));
        }
    }

    fn run_selected(&self) {
        if let Some(row) = self.list.selected_row()
            && let Ok(index) = usize::try_from(row.index())
        {
            self.run(index);
        }
    }

    /// Run row `index`.
    fn run(&self, index: usize) {
        let Some(row) = self.rows.borrow().get(index).cloned() else {
            return;
        };
        let action = match row.run() {
            Run::Nothing => return,
            Run::Search => {
                // The results are already under the row, from typing: Return
                // takes the keyboard to the first of them.
                self.pending.set(Pending::FirstHit);
                if self.loaded.get() == self.generation.get() {
                    self.apply_pending();
                } else {
                    self.search_typed();
                }
                return;
            }
            Run::SearchFor(text) => {
                self.set_text(&text);
                self.search_typed();
                return;
            }
            Run::ToggleOrder => {
                self.toggle_order();
                return;
            }
            Run::Action(action) => action,
        };
        if matches!(action, BarAction::Open { .. }) {
            self.held.replace(Some(self.entry.text().to_string()));
        }
        self.close();
        let handler = self.handler.borrow().clone();
        if let Some(handler) = handler {
            handler(action);
        }
    }
}
