//! The folders popover (screen 10; contracts/focus-surface.md, "The folders
//! popover"): `g o`, or a click on "Inbox ▾", lists the places to go --
//! mailboxes with their direct keys, the person's folders, and labels --
//! with their counts. Typing filters them, and `Enter` goes to the first.
//!
//! Focus has no folder sidebar: this, and `in:` in the command bar, are how
//! a person gets anywhere but the inbox.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;
use postio_client::Client;
use postio_core::{CommandId, Keymap};
use postio_model::MailboxRole;
use postio_model::listing::MailStore as _;
use postio_ui::finder::Destination;
use postio_ui::label_colour::{Rgb, label_colour};
use postio_widgets::widgets::keyhint;
use postio_widgets::widgets::space::S2;

/// The popover's width.
const WIDTH: i32 = 400;

/// The box a label's dot sits in: an icon's width, so names line up.
const DOT_BOX: i32 = 16;

/// A label's dot's radius.
const DOT_RADIUS: f64 = 4.0;

/// What the Outbox is called, in the popover and the header.
pub const OUTBOX: &str = "Outbox";

/// Which section a place is listed under.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Section {
    Mailboxes,
    Folders,
    Labels,
}

impl Section {
    fn title(self) -> &'static str {
        match self {
            Section::Mailboxes => "Mailboxes",
            Section::Folders => "Folders",
            Section::Labels => "Labels",
        }
    }
}

/// One place the popover lists.
#[derive(Debug, Clone)]
struct Entry {
    section: Section,
    /// Where it sorts within its section: a mailbox's role order.
    rank: usize,
    name: String,
    /// What it says on the right: its count, or "186 today".
    count: Option<String>,
    go: Option<CommandId>,
    /// Run this command rather than go to [`Self::destination`]: a place
    /// that is a view of its own, like Filtered.
    command: Option<CommandId>,
    destination: Destination,
    /// How it is marked: a mailbox's icon, or a label's colour.
    mark: Mark,
}

/// What sits before a place's name.
#[derive(Debug, Clone)]
enum Mark {
    /// A symbolic icon, by name.
    Icon(&'static str),
    /// A label's dot, in its stored colour if it has one.
    Dot(Option<String>),
}

/// What a place that is a command asks the window to run.
type CommandHandler = Rc<dyn Fn(CommandId)>;

/// What going somewhere asks the window to do: the place, and its name.
type Handler = Rc<dyn Fn(Destination, String)>;

/// The popover.
pub struct Places {
    client: Client,
    popover: gtk::Popover,
    entry: gtk::SearchEntry,
    list: gtk::ListBox,
    /// Every place, as last read.
    all: Rc<RefCell<Vec<Entry>>>,
    /// The places listed now, in order.
    shown: RefCell<Vec<Entry>>,
    /// What each row of the list is: a section's heading, or the place at
    /// that index of `shown`.
    rows: RefCell<Vec<Option<usize>>>,
    keymap: RefCell<Keymap>,
    handler: RefCell<Option<Handler>>,
    /// Runs a place that is a command: Filtered.
    command_handler: RefCell<Option<CommandHandler>>,
    /// How many were filtered today, while filtering is on: whether the
    /// popover lists Filtered, and what it says beside it.
    filtered_today: std::cell::Cell<Option<u32>>,
    me: RefCell<std::rc::Weak<Places>>,
}

impl Places {
    /// The commands the popover has a row for: going to each mailbox that
    /// has a key of its own.
    pub fn controls() -> Vec<CommandId> {
        [
            MailboxRole::Inbox,
            MailboxRole::Drafts,
            MailboxRole::Sent,
            MailboxRole::Archive,
            MailboxRole::Snoozed,
            MailboxRole::Flagged,
        ]
        .into_iter()
        .filter_map(go_to)
        .collect()
    }

    /// A closed popover, anchored to `anchor`, reading through `client`.
    pub fn new(client: Client, keymap: &Keymap, anchor: &impl IsA<gtk::Widget>) -> Rc<Self> {
        let entry = gtk::SearchEntry::new();
        entry.set_placeholder_text(Some("Go to folder or label"));
        let list = gtk::ListBox::new();
        list.add_css_class("focus-places-list");
        list.set_selection_mode(gtk::SelectionMode::Single);
        let scrolled = gtk::ScrolledWindow::builder()
            .child(&list)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .propagate_natural_height(true)
            .max_content_height(640)
            .build();
        let footer = gtk::Label::new(Some(
            "\u{21b5} open \u{b7} Esc close \u{b7} same as in:Receipts in the command bar",
        ));
        footer.add_css_class("dim-label");
        footer.add_css_class("focus-places-footer");
        footer.set_xalign(0.0);
        let content = gtk::Box::new(gtk::Orientation::Vertical, S2);
        content.add_css_class("focus-places");
        content.set_width_request(WIDTH);
        content.append(&entry);
        content.append(&scrolled);
        content.append(&footer);
        let popover = gtk::Popover::builder()
            .child(&content)
            .has_arrow(false)
            .position(gtk::PositionType::Bottom)
            // Hung from the left edge of "Inbox", as screen 10 draws it,
            // rather than centred on it and out over the window's edge.
            .halign(gtk::Align::Start)
            .build();
        popover.set_parent(anchor);
        // The popover goes with the button it hangs from, before GTK would
        // finalize the button with a child still attached.
        anchor.as_ref().connect_destroy({
            let popover = popover.downgrade();
            move |_| {
                if let Some(popover) = popover.upgrade() {
                    popover.unparent();
                }
            }
        });

        let places = Rc::new(Places {
            client,
            popover,
            entry,
            list,
            all: Rc::default(),
            shown: RefCell::default(),
            rows: RefCell::default(),
            keymap: RefCell::new(keymap.clone()),
            handler: RefCell::default(),
            command_handler: RefCell::default(),
            filtered_today: std::cell::Cell::new(None),
            me: RefCell::default(),
        });
        places.me.replace(Rc::downgrade(&places));
        let weak = Rc::downgrade(&places);
        places.entry.connect_search_changed({
            let weak = weak.clone();
            move |_| {
                if let Some(places) = weak.upgrade() {
                    places.show();
                }
            }
        });
        places.entry.connect_activate({
            let weak = weak.clone();
            move |_| {
                if let Some(places) = weak.upgrade() {
                    places.activate();
                }
            }
        });
        places.list.connect_row_activated({
            let weak = weak.clone();
            move |_, row| {
                if let Some(places) = weak.upgrade()
                    && let Some(index) = places.place_at(row)
                {
                    places.go(index);
                }
            }
        });
        places
    }

    /// Run `handler` with the place a person chooses.
    pub fn connect_go(&self, handler: impl Fn(Destination, String) + 'static) {
        self.handler.replace(Some(Rc::new(handler)));
    }

    /// Run `handler` with the command a place stands for: Filtered's.
    pub fn connect_command(&self, handler: impl Fn(CommandId) + 'static) {
        self.command_handler.replace(Some(Rc::new(handler)));
    }

    /// How many were filtered today, or `None` while filtering is off:
    /// Filtered is listed among the mailboxes with it (screen 10).
    pub fn set_filtered_today(&self, count: Option<u32>) {
        self.filtered_today.set(count);
        if self.is_open() {
            self.show();
        }
    }

    /// Read every key the popover shows from `keymap`.
    pub fn set_keymap(&self, keymap: &Keymap) {
        self.keymap.replace(keymap.clone());
        self.show();
    }

    /// Open the popover, unfiltered, and read the places.
    pub fn open(&self) {
        self.entry.set_text("");
        self.show();
        self.popover.popup();
        self.entry.grab_focus();
        self.read();
    }

    /// Close the popover without going anywhere.
    pub fn close(&self) {
        self.popover.popdown();
    }

    /// Whether the popover is up.
    pub fn is_open(&self) -> bool {
        self.popover.is_visible()
    }

    /// Filter the places by `text`, as typing does.
    pub fn set_filter(&self, text: &str) {
        self.entry.set_text(text);
        self.show();
    }

    /// The names listed now, top to bottom.
    pub fn names(&self) -> Vec<String> {
        self.shown
            .borrow()
            .iter()
            .map(|entry| entry.name.clone())
            .collect()
    }

    /// Go to the chosen place, or the first listed: what `Enter` does.
    pub fn activate(&self) {
        let selected = self.list.selected_row().and_then(|row| self.place_at(&row));
        self.go(selected.unwrap_or(0));
    }

    /// The index in `shown` of the place `row` lists; `None` for a heading.
    fn place_at(&self, row: &gtk::ListBoxRow) -> Option<usize> {
        let index = usize::try_from(row.index()).ok()?;
        self.rows.borrow().get(index).copied().flatten()
    }

    /// Go to the `index`th place listed.
    fn go(&self, index: usize) {
        let Some(entry) = self.shown.borrow().get(index).cloned() else {
            return;
        };
        self.popover.popdown();
        if let Some(command) = entry.command {
            let handler = self.command_handler.borrow().clone();
            if let Some(handler) = handler {
                handler(command);
            }
            return;
        }
        let handler = self.handler.borrow().clone();
        if let Some(handler) = handler {
            handler(entry.destination, entry.name);
        }
    }

    /// List the places the filter matches, in their sections.
    fn show(&self) {
        while let Some(child) = self.list.first_child() {
            self.list.remove(&child);
        }
        let wanted = self.entry.text().to_lowercase();
        let filtered = self.filtered_today.get().map(|count| Entry {
            section: Section::Mailboxes,
            rank: role_rank(MailboxRole::Archive) + 1,
            name: postio_ui::filtered::TITLE.to_owned(),
            count: Some(postio_ui::filtered::today_short(count)),
            go: Some(CommandId::GoToFiltered),
            command: Some(CommandId::GoToFiltered),
            destination: Destination::Search(String::new()),
            mark: Mark::Icon("folder-symbolic"),
        });
        let mut shown: Vec<Entry> = self
            .all
            .borrow()
            .iter()
            .chain(filtered.iter())
            .filter(|entry| wanted.is_empty() || entry.name.to_lowercase().contains(&wanted))
            .cloned()
            .collect();
        shown.sort_by(|a, b| {
            (a.section, a.rank, a.name.to_lowercase()).cmp(&(
                b.section,
                b.rank,
                b.name.to_lowercase(),
            ))
        });
        let keymap = self.keymap.borrow();
        let accent_hue = accent_hue();
        let mut rows = Vec::new();
        let mut section = None;
        let mut first = None;
        for (index, entry) in shown.iter().enumerate() {
            if section != Some(entry.section) {
                section = Some(entry.section);
                let heading = gtk::Label::new(Some(entry.section.title()));
                heading.add_css_class("focus-places-section");
                heading.set_xalign(0.0);
                let item = gtk::ListBoxRow::new();
                item.set_child(Some(&heading));
                item.set_selectable(false);
                item.set_activatable(false);
                item.set_can_focus(false);
                self.list.append(&item);
                rows.push(None);
            }
            let row = gtk::Box::new(gtk::Orientation::Horizontal, S2);
            row.add_css_class("focus-places-row");
            row.append(&mark(&entry.mark, &entry.name, accent_hue));
            let name = gtk::Label::new(Some(&entry.name));
            name.set_xalign(0.0);
            name.set_hexpand(true);
            row.append(&name);
            if let Some(count) = &entry.count {
                let count = gtk::Label::new(Some(count));
                count.add_css_class("dim-label");
                count.add_css_class("focus-places-count");
                row.append(&count);
            }
            if let Some(key) = entry
                .go
                .and_then(|command| postio_ui::hints::key(&keymap, command))
            {
                row.append(&keyhint::cap(&key));
            }
            let item = gtk::ListBoxRow::new();
            item.set_child(Some(&row));
            self.list.append(&item);
            first.get_or_insert(item);
            rows.push(Some(index));
        }
        if let Some(first) = first {
            self.list.select_row(Some(&first));
        }
        self.rows.replace(rows);
        self.shown.replace(shown);
    }

    /// Read every account's mailboxes and labels.
    fn read(&self) {
        let client = self.client.clone();
        let all = Rc::clone(&self.all);
        let weak = self.me.borrow().clone();
        glib::spawn_future_local(async move {
            // POSTIO-GLIB-SAFE: a client call is a oneshot receive; the host
            // answers on its own runtime (ADR 0041).
            let Ok(accounts) = client.accounts().await else {
                return;
            };
            let mut found = Vec::new();
            for account in accounts.iter().filter(|account| account.enabled) {
                // POSTIO-GLIB-SAFE: as above.
                let read = client.mailboxes(account.id).await;
                for mailbox in read.unwrap_or_default() {
                    let (section, rank) = match mailbox.role {
                        MailboxRole::Regular => (Section::Folders, 0),
                        role => (Section::Mailboxes, role_rank(role)),
                    };
                    found.push(Entry {
                        section,
                        rank,
                        name: place_name(&mailbox),
                        count: Some(mailbox.counts.total.to_string()),
                        command: None,
                        go: go_to(mailbox.role),
                        destination: Destination::Mailbox(mailbox.id),
                        mark: Mark::Icon(icon(mailbox.role)),
                    });
                }
                // The Outbox, while anything waits in it (T239): a view over
                // Drafts, so it has no mailbox row of its own to be listed by.
                let outbox = postio_model::ListScope::Outbox(account.id);
                // POSTIO-GLIB-SAFE: as above.
                let waiting = client.list_count(outbox).await.unwrap_or(0);
                if waiting > 0 {
                    found.push(Entry {
                        section: Section::Mailboxes,
                        rank: role_rank(MailboxRole::Outbox),
                        name: OUTBOX.to_owned(),
                        count: Some(waiting.to_string()),
                        command: None,
                        go: None,
                        destination: Destination::Outbox(account.id),
                        mark: Mark::Icon(icon(MailboxRole::Outbox)),
                    });
                }
                // POSTIO-GLIB-SAFE: as above.
                let read = client.labels(account.id).await;
                for label in read.unwrap_or_default() {
                    found.push(Entry {
                        section: Section::Labels,
                        rank: 0,
                        name: label.name.clone(),
                        count: None,
                        command: None,
                        go: None,
                        destination: Destination::Label(label.id),
                        mark: Mark::Dot(label.color.clone()),
                    });
                }
            }
            all.replace(found);
            if let Some(places) = weak.upgrade() {
                places.show();
            }
        });
    }
}

/// The icon a mailbox of `role` is marked with.
fn icon(role: MailboxRole) -> &'static str {
    match role {
        MailboxRole::Inbox => "mail-read-symbolic",
        MailboxRole::Drafts => "document-edit-symbolic",
        MailboxRole::Sent | MailboxRole::Outbox => "mail-send-symbolic",
        MailboxRole::Snoozed => "alarm-symbolic",
        MailboxRole::Flagged => "mail-mark-important-symbolic",
        MailboxRole::Junk => "mail-mark-junk-symbolic",
        MailboxRole::Trash => "user-trash-symbolic",
        MailboxRole::Archive | MailboxRole::Regular => "folder-symbolic",
    }
}

/// The widget `mark` makes before a place called `name`.
fn mark(mark: &Mark, name: &str, accent_hue: f64) -> gtk::Widget {
    match mark {
        Mark::Icon(icon) => {
            let image = gtk::Image::from_icon_name(icon);
            image.add_css_class("focus-places-icon");
            image.set_accessible_role(gtk::AccessibleRole::Presentation);
            image.upcast()
        }
        Mark::Dot(stored) => colour_dot(label_colour(
            name,
            stored.as_deref().and_then(Rgb::from_hex),
            accent_hue,
        ))
        .upcast(),
    }
}

/// A dot in `colour`: what a label is marked with, in the places list and
/// on a label's pill.
pub(crate) fn colour_dot(colour: Rgb) -> gtk::DrawingArea {
    let dot = gtk::DrawingArea::new();
    dot.set_content_width(DOT_BOX);
    dot.set_content_height(DOT_BOX);
    dot.set_valign(gtk::Align::Center);
    dot.set_accessible_role(gtk::AccessibleRole::Presentation);
    dot.set_draw_func(move |_, cairo, width, height| {
        let unit = |channel: u8| f64::from(channel) / 255.0;
        cairo.set_source_rgb(unit(colour.r), unit(colour.g), unit(colour.b));
        cairo.arc(
            f64::from(width) / 2.0,
            f64::from(height) / 2.0,
            DOT_RADIUS,
            0.0,
            std::f64::consts::TAU,
        );
        let _ = cairo.fill();
    });
    dot
}

/// A label's colour: its own, or the one its name gets from the wheel.
pub(crate) fn label_rgb(label: &postio_model::Label) -> Rgb {
    label_colour(
        &label.name,
        label.color.as_deref().and_then(Rgb::from_hex),
        accent_hue(),
    )
}

/// A label's colour, as its pill and its dot draw it.
pub(crate) fn label_rgba(label: &postio_model::Label) -> gtk::gdk::RGBA {
    let colour = label_rgb(label);
    gtk::gdk::RGBA::new(
        f32::from(colour.r) / 255.0,
        f32::from(colour.g) / 255.0,
        f32::from(colour.b) / 255.0,
        1.0,
    )
}

/// The hue labels without a colour of their own are spread around.
fn accent_hue() -> f64 {
    let manager = adw::StyleManager::default();
    let accent = manager.accent_color().to_standalone_rgba(manager.is_dark());
    let byte = |channel: f32| (channel * 255.0).round().clamp(0.0, 255.0) as u8;
    Rgb::new(
        byte(accent.red()),
        byte(accent.green()),
        byte(accent.blue()),
    )
    .hue()
}

/// Where a mailbox of `role` sorts: the order screen 10 lists them in.
fn role_rank(role: MailboxRole) -> usize {
    match role {
        MailboxRole::Inbox => 0,
        MailboxRole::Drafts => 1,
        // What Drafts sent on its way, listed under it.
        MailboxRole::Outbox => 1,
        MailboxRole::Sent => 2,
        MailboxRole::Snoozed => 3,
        MailboxRole::Archive => 4,
        _ => 5,
    }
}

/// The command that goes to a mailbox of `role` directly.
pub fn go_to(role: MailboxRole) -> Option<CommandId> {
    Some(match role {
        MailboxRole::Inbox => CommandId::GoToInbox,
        MailboxRole::Drafts => CommandId::GoToDrafts,
        MailboxRole::Sent => CommandId::GoToSent,
        MailboxRole::Archive => CommandId::GoToArchive,
        MailboxRole::Snoozed => CommandId::GoToSnoozed,
        MailboxRole::Flagged => CommandId::GoToFlagged,
        _ => return None,
    })
}

/// What a mailbox is called here: "Inbox" for an inbox, whatever the
/// server names it ("INBOX"), and its own name otherwise.
pub fn place_name(mailbox: &postio_model::Mailbox) -> String {
    match mailbox.role {
        MailboxRole::Inbox => "Inbox".to_owned(),
        _ => mailbox.name.clone(),
    }
}
