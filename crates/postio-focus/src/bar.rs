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
//! Everything here is local. Nothing typed leaves the machine.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;
use postio_client::Client;
use postio_core::{ActionId, CommandId, Context, Frontend, Keymap};
use postio_model::listing::{ListPage, MailStore as _, PageRequest};
use postio_model::{AccountScope, ListScope, MailboxId, MailboxRole, MessageId};
use postio_ui::finder::{self, Destination, Place, PlaceKind};
use postio_widgets::widgets::keyhint;
use postio_widgets::widgets::space::{S1, S2, S3};

/// The bar's width, as the screens draw it.
const WIDTH: i32 = 860;
/// How many conversations a folder lists in the bar.
const FOLDER_ROWS: u32 = 30;
/// How many search hits the bar lists.
const HITS: usize = 30;

/// What running a row asks the window to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BarAction {
    /// Open a message over the list.
    Open {
        /// Which.
        message: MessageId,
        /// Its subject, for the dialog's header.
        subject: String,
    },
    /// Run a command, on what the bar opened over.
    Command(CommandId),
    /// Go to a place: the list shows it.
    Go {
        /// Where.
        destination: Destination,
        /// Its name, for the header strip.
        name: String,
    },
}

/// One row of the results, and what running it does.
#[derive(Debug, Clone)]
enum Row {
    /// A section heading; runs nothing.
    Heading,
    /// A message: a search hit, or a folder's conversation.
    Message { message: MessageId, subject: String },
    /// A command.
    Command(ActionId),
    /// A place.
    Place(Destination, String),
    /// "Search mail for …": the results are already the search's.
    Search,
}

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
    names: Rc<RefCell<crate::names::Names>>,
    /// What each row of the list runs, in order.
    rows: Rc<RefCell<Vec<Row>>>,
    /// Moves with every keystroke, so a late answer to an earlier one is
    /// dropped.
    generation: Rc<Cell<u64>>,
    open: Cell<bool>,
    handler: RefCell<Option<Handler>>,
    /// This bar, weakly: what a future comes back to.
    me: RefCell<std::rc::Weak<Bar>>,
    /// The pinned saved searches, in order: each name and its query.
    saved: RefCell<Vec<(String, String)>>,
    /// Whether the places have been read since the bar last opened.
    places_known: Cell<bool>,
    /// The saved row across the top.
    saved_row: gtk::Box,
}

impl Bar {
    /// The commands the bar has a control for beyond its command rows: a
    /// pill for each saved search, and a row for each message found.
    pub fn controls() -> Vec<CommandId> {
        SAVED.into_iter().chain([CommandId::OpenMessage]).collect()
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
        let root = gtk::Box::new(gtk::Orientation::Vertical, S2);
        root.add_css_class("focus-bar");
        root.append(&saved_row);
        root.set_width_request(WIDTH);
        root.set_halign(gtk::Align::Center);
        root.set_valign(gtk::Align::Start);
        root.append(&input);
        root.append(&echo);
        root.append(&heading);
        root.append(&scrolled);
        root.append(&footer);
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
            generation: Rc::default(),
            open: Cell::new(false),
            handler: RefCell::default(),
            me: RefCell::default(),
            saved: RefCell::default(),
            places_known: Cell::new(false),
            saved_row,
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
            move |_, key, _, _| {
                let Some(bar) = weak.upgrade() else {
                    return glib::Propagation::Proceed;
                };
                match key {
                    gtk::gdk::Key::Down => bar.step(1),
                    gtk::gdk::Key::Up => bar.step(-1),
                    _ => return glib::Propagation::Proceed,
                }
                glib::Propagation::Stop
            }
        });
        bar.entry.add_controller(keys);
        bar.set_keymap(keymap);
        bar
    }

    /// The bar, to lay over the list.
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

    /// Open the bar, empty, and read the places it can go.
    pub fn open(&self) {
        self.open.set(true);
        self.places_known.set(false);
        self.over.set_visible(true);
        self.entry.set_text("");
        self.update("");
        self.entry.grab_focus();
        self.read_places();
    }

    /// Close the bar. Nothing it showed is kept.
    pub fn close(&self) {
        self.open.set(false);
        self.over.set_visible(false);
        self.generation.set(self.generation.get() + 1);
    }

    /// Run the search row: search for what is typed, as choosing
    /// "Search mail for …" does.
    pub fn run_search(&self) {
        self.search_typed();
    }

    /// Whether the places the bar can go have been read since it opened.
    pub fn places_known(&self) -> bool {
        self.places_known.get()
    }

    /// Whether the bar is up.
    pub fn is_open(&self) -> bool {
        self.open.get()
    }

    /// Put `text` in the bar, as typing it would.
    pub fn set_text(&self, text: &str) {
        self.entry.set_text(text);
        self.entry.set_position(-1);
    }

    /// The results' heading.
    pub fn heading(&self) -> String {
        if self.heading.is_visible() {
            self.heading.text().to_string()
        } else {
            String::new()
        }
    }

    /// Every command the bar can list: what typing its name finds.
    pub fn commands(&self) -> Vec<CommandId> {
        let state = postio_core::Availability {
            scope: AccountScope::Unified,
            store_open: true,
            frontend: Frontend::Focus,
        };
        postio_ui::palette::entries(&self.keymap.borrow(), Context::List, state, "")
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
        self.echo
            .set_text(&format!("You typed \u{201c}{typed}\u{201d}"));
        if let Some(name) = typed.strip_prefix("in:").filter(|name| !name.contains(' ')) {
            self.show_chips(&[]);
            self.show_folder(name, generation);
            return;
        }
        self.show_blend(typed);
        if typed.is_empty() || typed.starts_with(finder::COMMANDS_ONLY) {
            self.show_chips(&[]);
            self.heading.set_visible(false);
            return;
        }
        let parsed = self.lowered(typed);
        // Words that name what they want -- an operator, or a partial one
        // on its way -- are a search, shown as its chips (screen 07). A
        // plain word is answered with the commands and places it names and
        // one search row, and searches when that row is chosen (screen 09);
        // it is what the entry already says, so it makes no chip.
        if parsed.filters().next().is_some() || parsed.partials().next().is_some() {
            let chips: Vec<String> = parsed
                .tokens()
                .iter()
                .map(|token| token.raw.clone())
                .collect();
            self.show_chips(&chips);
            self.search(parsed, generation);
        } else {
            self.show_chips(&[]);
            self.heading.set_visible(false);
        }
    }

    /// `typed`, read as plain English against today and the address book.
    fn lowered(&self, typed: &str) -> postio_search::ParsedQuery {
        let names = self.names.borrow();
        postio_search::natural::lower(typed, chrono::Local::now().date_naive(), &|name| {
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
        self.clear_rows();
        self.search(parsed, self.generation.get());
    }

    /// The chips the words were lowered to, in order.
    fn show_chips(&self, chips: &[String]) {
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
        let blend = finder::blend(typed, &places, &keymap, Context::List, state);
        if !blend.commands.is_empty() {
            self.append_heading("Commands");
            for entry in blend.commands.iter().take(5) {
                self.append_row(
                    Row::Command(entry.id),
                    entry.title,
                    None,
                    entry.binding.as_deref(),
                );
            }
        }
        if !blend.places.is_empty() {
            self.append_heading("Go to");
            for hit in blend.places.iter().take(5) {
                let detail = hit
                    .place
                    .count
                    .map(|count| format!("{count} conversations"));
                self.append_row(
                    Row::Place(hit.place.destination.clone(), hit.place.name.clone()),
                    &format!("in:{}", hit.place.name),
                    detail.as_deref(),
                    hit.binding.as_deref(),
                );
            }
        }
        if let Some(query) = &blend.search {
            self.append_row(
                Row::Search,
                &format!("Search mail for \u{201c}{query}\u{201d}"),
                Some("subject, body, attachments"),
                None,
            );
        }
    }

    /// Search this machine's index for `parsed`, and list what it finds,
    /// one row per conversation.
    fn search(&self, parsed: postio_search::ParsedQuery, generation: u64) {
        let client = self.client.clone();
        let current = Rc::clone(&self.generation);
        let folders = Rc::clone(&self.folders);
        let weak = self.self_weak();
        glib::spawn_future_local(async move {
            let search = client.search_hits(
                AccountScope::Unified,
                parsed,
                postio_search::facets::Scope::AllMail,
                postio_search::ResultOrder::Relevance,
                0,
            );
            // POSTIO-GLIB-SAFE: a client call is a oneshot receive; the host
            // answers on its own runtime (ADR 0041).
            let found = search.await;
            if current.get() != generation {
                return;
            }
            let Some(bar) = weak.upgrade() else {
                return;
            };
            let Ok(Some(results)) = found else {
                return;
            };
            let mut seen = Vec::new();
            let mut rows = Vec::new();
            for hit in results.hits {
                let conversation = hit
                    .thread_id
                    .map(|thread| thread.get())
                    .unwrap_or(-hit.message_id.get());
                if seen.contains(&conversation) {
                    continue;
                }
                seen.push(conversation);
                rows.push(hit);
                if rows.len() == HITS {
                    break;
                }
            }
            bar.heading.set_text(&format!(
                "Conversations \u{b7} {} match{}",
                rows.len(),
                if rows.len() == 1 { "" } else { "es" }
            ));
            bar.heading.set_visible(true);
            let names = folders.borrow().clone();
            for hit in rows {
                let subject = hit.subject.clone().unwrap_or_default();
                let place = names
                    .iter()
                    .find(|(id, _)| *id == hit.mailbox_id)
                    .map(|(_, name)| format!("in:{name}"));
                bar.append_message(
                    hit.message_id,
                    hit.from.as_ref().map(said_of),
                    &subject,
                    Some(&hit.snippet),
                    place.as_deref(),
                    hit.received_at,
                );
            }
        });
    }

    /// `in:` and `name`: the first folder whose name starts with it, and its
    /// conversations newest first (screen 08).
    fn show_folder(&self, name: &str, generation: u64) {
        let wanted = name.to_lowercase();
        let folder = self
            .places
            .borrow()
            .iter()
            .filter(|place| matches!(place.kind, PlaceKind::Mailbox | PlaceKind::Folder))
            .find(|place| place.name.to_lowercase().starts_with(&wanted))
            .cloned();
        let Some(Place {
            name,
            destination: Destination::Mailbox(mailbox),
            ..
        }) = folder
        else {
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
                limit: FOLDER_ROWS,
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
            bar.heading.set_text(&format!(
                "{name} \u{b7} folder \u{b7} {} conversations \u{b7} newest first",
                count.unwrap_or(0)
            ));
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
            for account in accounts.iter().filter(|account| account.enabled) {
                // POSTIO-GLIB-SAFE: as above.
                let read = client.mailboxes(account.id).await;
                if let Ok(mailboxes) = read {
                    for mailbox in mailboxes {
                        let name = crate::places::place_name(&mailbox);
                        names.push((mailbox.id, name.clone()));
                        found.push(Place {
                            kind: if mailbox.role == MailboxRole::Regular {
                                PlaceKind::Folder
                            } else {
                                PlaceKind::Mailbox
                            },
                            name,
                            count: None,
                            go: crate::places::go_to(mailbox.role).map(ActionId::from),
                            destination: Destination::Mailbox(mailbox.id),
                        });
                    }
                }
                // POSTIO-GLIB-SAFE: as above.
                let read = client.correspondents(account.id).await;
                correspondents.extend(read.unwrap_or_default());
                // POSTIO-GLIB-SAFE: as above.
                let read = client.labels(account.id).await;
                if let Ok(labels) = read {
                    for label in labels {
                        found.push(Place {
                            kind: PlaceKind::Label,
                            name: label.name.clone(),
                            count: None,
                            go: None,
                            destination: Destination::Label(label.id),
                        });
                    }
                }
            }
            places.replace(found);
            folders.replace(names);
            directory.replace(crate::names::Names::new(&correspondents));
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
        while let Some(child) = self.list.first_child() {
            self.list.remove(&child);
        }
        self.rows.borrow_mut().clear();
    }

    fn append_heading(&self, title: &str) {
        let label = gtk::Label::new(Some(title));
        label.add_css_class("focus-bar-section");
        label.set_xalign(0.0);
        let row = gtk::ListBoxRow::new();
        row.set_child(Some(&label));
        row.set_selectable(false);
        row.set_activatable(false);
        self.list.append(&row);
        self.rows.borrow_mut().push(Row::Heading);
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
        self.list.append(&row);
        self.rows.borrow_mut().push(target);
        self.select_first();
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
        let date = gtk::Label::new(Some(&postio_ui::row::timestamp(at, chrono::Local::now())));
        date.add_css_class("dim-label");
        line.append(&date);
        let row = gtk::ListBoxRow::new();
        row.add_css_class("focus-bar-row");
        row.set_child(Some(&line));
        self.list.append(&row);
        self.rows.borrow_mut().push(Row::Message {
            message,
            subject: subject.to_owned(),
        });
        self.select_first();
    }

    /// Select the first row that runs something, when nothing is selected.
    fn select_first(&self) {
        if self.list.selected_row().is_some() {
            return;
        }
        let first = self
            .rows
            .borrow()
            .iter()
            .position(|row| !matches!(row, Row::Heading));
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
            if !matches!(self.rows.borrow()[at as usize], Row::Heading) {
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
        let action = match row {
            Row::Heading => return,
            Row::Search => {
                self.search_typed();
                return;
            }
            Row::Message { message, subject } => BarAction::Open { message, subject },
            Row::Command(ActionId::Builtin(command)) => BarAction::Command(command),
            Row::Command(ActionId::Ext(_)) => return,
            Row::Place(destination, name) => BarAction::Go { destination, name },
        };
        self.close();
        let handler = self.handler.borrow().clone();
        if let Some(handler) = handler {
            handler(action);
        }
    }
}

/// The commands that run the saved searches, in order.
const SAVED: [CommandId; 4] = [
    CommandId::SavedSearch1,
    CommandId::SavedSearch2,
    CommandId::SavedSearch3,
    CommandId::SavedSearch4,
];

/// A sender as a result row names them: their name, or their address.
fn said_of(from: &postio_model::EmailAddress) -> String {
    from.name.clone().unwrap_or_else(|| from.address.clone())
}
