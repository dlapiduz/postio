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
use postio_ui::places as rules;
use postio_widgets::widgets::keyhint;
use postio_widgets::widgets::space::S2;

/// The popover's width.
const WIDTH: i32 = 400;

/// The box a label's dot sits in: an icon's width, so names line up.
const DOT_BOX: i32 = 16;

/// A label's dot's radius.
const DOT_RADIUS: f64 = 4.0;

use postio_ui::places::{Entry, Mark};
pub use postio_ui::places::{OUTBOX, go_to, place_name};

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
    /// The line under the places, which says what the highlighted row does.
    footer: gtk::Label,
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
        rules::direct_commands()
    }

    /// A closed popover, anchored to `anchor`, reading through `client`.
    pub fn new(client: Client, keymap: &Keymap, anchor: &impl IsA<gtk::Widget>) -> Rc<Self> {
        let entry = gtk::SearchEntry::new();
        entry.set_placeholder_text(Some(rules::FILTER_PLACEHOLDER));
        let list = gtk::ListBox::new();
        list.add_css_class("focus-places-list");
        list.set_selection_mode(gtk::SelectionMode::Single);
        let scrolled = gtk::ScrolledWindow::builder()
            .child(&list)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .propagate_natural_height(true)
            .max_content_height(640)
            .build();
        let footer = gtk::Label::new(Some(&rules::footer(None)));
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
            footer,
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
        // The arrows walk the places while the keyboard stays in the filter.
        let keys = gtk::EventControllerKey::new();
        keys.connect_key_pressed({
            let weak = weak.clone();
            move |_, key, _, _| {
                let by = match key {
                    gtk::gdk::Key::Down => 1,
                    gtk::gdk::Key::Up => -1,
                    _ => return glib::Propagation::Proceed,
                };
                if let Some(places) = weak.upgrade() {
                    places.step(by);
                }
                glib::Propagation::Stop
            }
        });
        places.entry.add_controller(keys);
        places.list.connect_row_selected({
            let weak = weak.clone();
            move |_, row| {
                if let Some(places) = weak.upgrade() {
                    places.say_footer(row);
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
        // The keys are drawn on the rows but are not part of what is listed,
        // so a new keymap redraws even when the places are the same.
        self.draw(true);
    }

    /// Open the popover, unfiltered, and read the places.
    pub fn open(&self) {
        self.entry.set_text("");
        self.show();
        self.highlight_first();
        self.popover.popup();
        self.entry.grab_focus();
        self.read();
    }

    /// Run `handler` each time the popover closes, however it did.
    pub fn connect_closed(&self, handler: impl Fn() + 'static) {
        self.popover.connect_closed(move |_| handler());
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

    /// The line under the places, as a person reads it.
    pub fn footer(&self) -> String {
        self.footer.text().to_string()
    }

    /// Move the highlight `by` places, skipping the headings.
    fn step(&self, by: i32) {
        let mut at = self.list.selected_row().map_or(-1, |row| row.index());
        loop {
            at += by;
            let Some(row) = self.list.row_at_index(at) else {
                return;
            };
            if row.is_selectable() {
                self.list.select_row(Some(&row));
                return;
            }
        }
    }

    /// Say what Return does on `row`, the one now highlighted.
    fn say_footer(&self, row: Option<&gtk::ListBoxRow>) {
        let shown = self.shown.borrow();
        let entry = row
            .and_then(|row| self.place_at(row))
            .and_then(|index| shown.get(index));
        self.footer.set_text(&rules::footer(entry));
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
        self.draw(false);
    }

    /// Put the highlight on the first place listed.
    fn highlight_first(&self) {
        let mut at = 0;
        while let Some(row) = self.list.row_at_index(at) {
            if row.is_selectable() {
                self.list.select_row(Some(&row));
                return;
            }
            at += 1;
        }
    }

    /// Draw the places the filter matches -- unless they are what is drawn
    /// already and nothing else asks for it (`force`).
    ///
    /// Drawing tears every row down, and a click landing across that is a
    /// click on a row that no longer exists: it goes nowhere. The rows are
    /// redrawn from many places -- the entry's delayed `search-changed`, a
    /// read landing, the filtered count moving -- so the guard is here, on
    /// the one thing they all call, rather than on each of them.
    fn draw(&self, force: bool) {
        let filtered = self.filtered_today.get().map(rules::filtered_entry);
        let shown = rules::listed(&self.all.borrow(), filtered.as_ref(), &self.entry.text());
        if !force && self.list.first_child().is_some() && *self.shown.borrow() == shown {
            return;
        }
        while let Some(child) = self.list.first_child() {
            self.list.remove(&child);
        }
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
        self.rows.replace(rows);
        self.shown.replace(shown);
        if let Some(first) = first {
            self.list.select_row(Some(&first));
        }
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
                    let mut entry = rules::mailbox_entry(&mailbox);
                    // The count is conversations, as the strip counts them:
                    // the inbox's is Focus's, which is what the strip reads.
                    let scope = if mailbox.role == MailboxRole::Inbox {
                        postio_model::ListScope::Focus(postio_model::FocusScope::Inbox)
                    } else {
                        postio_model::ListScope::Mailbox(mailbox.id)
                    };
                    // POSTIO-GLIB-SAFE: as above.
                    if let Ok(conversations) = client.list_count(scope).await {
                        entry.count = Some(conversations.to_string());
                    }
                    found.push(entry);
                }
                // The Outbox, while anything waits in it (T239): a view over
                // Drafts, so it has no mailbox row of its own to be listed by.
                let outbox = postio_model::ListScope::Outbox(account.id);
                // POSTIO-GLIB-SAFE: as above.
                let waiting = client.list_count(outbox).await.unwrap_or(0);
                if waiting > 0 {
                    found.push(rules::outbox_entry(account.id, waiting));
                }
                // POSTIO-GLIB-SAFE: as above.
                let read = client.labels(account.id).await;
                for label in read.unwrap_or_default() {
                    found.push(rules::label_entry(&label));
                }
            }
            // `open` has already drawn the rows from the last read; `show`
            // redraws only if this one found something different.
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
        MailboxRole::Sent => "mail-send-symbolic",
        // What is on its way out, not what went: an arrow leaving a box.
        MailboxRole::Outbox => "send-to-symbolic",
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
        Mark::Role(role) => {
            let image = gtk::Image::from_icon_name(icon(*role));
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Two places that mean different things do not wear the same mark.
    #[test]
    fn no_two_roles_share_an_icon_but_the_folders() {
        use MailboxRole::*;
        let marked = [
            Inbox, Drafts, Sent, Outbox, Snoozed, Flagged, Junk, Trash, Archive,
        ];
        for (at, one) in marked.iter().enumerate() {
            for other in &marked[at + 1..] {
                // Archive is a folder like any other the person made.
                assert_ne!(icon(*one), icon(*other), "{one:?} and {other:?}");
            }
        }
    }
}
