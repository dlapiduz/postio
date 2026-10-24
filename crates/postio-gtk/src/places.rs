//! The folders popover (screen 10; contracts/focus-surface.md, "The folders
//! popover"): `g l`, or a click on "Inbox ▾", lists the places to go --
//! mailboxes with their direct keys, the person's folders, and labels --
//! with their counts. Typing filters them, and `Enter` goes to the first.
//!
//! Focus has no folder sidebar: this, and `in:` in the command bar, are how
//! a person gets anywhere but the inbox.
//!
//! Which places there are, what each counts and where each goes are the
//! controller's (spec 009 slice 8): the popover asks it for the places a
//! filter matches ([`Places::connect_source`]) and hands back the one chosen
//! ([`Places::connect_go`]). The window redraws it when the controller says
//! the places changed.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;
use postio_core::{CommandId, Keymap};
use postio_model::MailboxRole;
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

/// The places a filter matches, each with the token that goes there.
type Source = Rc<dyn Fn(&str) -> Vec<(u64, Entry)>>;

/// Going to the place with a token.
type Handler = Rc<dyn Fn(u64)>;

/// The popover.
pub struct Places {
    popover: gtk::Popover,
    entry: gtk::SearchEntry,
    list: gtk::ListBox,
    /// The line under the places, which says what the highlighted row does.
    footer: gtk::Label,
    /// Where the places come from.
    source: RefCell<Option<Source>>,
    /// The places listed now, in order.
    shown: RefCell<Vec<Entry>>,
    /// Each listed place's token, in `shown`'s order.
    tokens: RefCell<Vec<u64>>,
    /// What each row of the list is: a section's heading, or the place at
    /// that index of `shown`.
    rows: RefCell<Vec<Option<usize>>>,
    /// Each place's count label, by its index in `shown`: a count that
    /// moves is written here rather than redrawn.
    counts: RefCell<Vec<gtk::Label>>,
    keymap: RefCell<Keymap>,
    handler: RefCell<Option<Handler>>,
    me: RefCell<std::rc::Weak<Places>>,
}

impl Places {
    /// The commands the popover has a row for: going to each mailbox that
    /// has a key of its own.
    pub fn controls() -> Vec<CommandId> {
        rules::direct_commands()
    }

    /// A closed popover, anchored to `anchor`.
    pub fn new(keymap: &Keymap, anchor: &impl IsA<gtk::Widget>) -> Rc<Self> {
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
            popover,
            entry,
            list,
            footer,
            source: RefCell::default(),
            shown: RefCell::default(),
            tokens: RefCell::default(),
            rows: RefCell::default(),
            counts: RefCell::default(),
            keymap: RefCell::new(keymap.clone()),
            handler: RefCell::default(),
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

    /// Ask `source` for the places a filter matches.
    pub fn connect_source(&self, source: impl Fn(&str) -> Vec<(u64, Entry)> + 'static) {
        self.source.replace(Some(Rc::new(source)));
    }

    /// Run `handler` with the token of the place a person chooses.
    pub fn connect_go(&self, handler: impl Fn(u64) + 'static) {
        self.handler.replace(Some(Rc::new(handler)));
    }

    /// The places changed: draw them again, if the popover is up.
    pub fn redraw(&self) {
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

    /// Open the popover, unfiltered, on the places as last read: the
    /// controller asks for them again as it opens, and says when they land.
    pub fn open(&self) {
        self.entry.set_text("");
        self.show();
        self.highlight_first();
        self.popover.popup();
        self.entry.grab_focus();
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
        let Some(token) = self.tokens.borrow().get(index).copied() else {
            return;
        };
        self.popover.popdown();
        let handler = self.handler.borrow().clone();
        if let Some(handler) = handler {
            handler(token);
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
        let source = self.source.borrow().clone();
        let (tokens, shown): (Vec<u64>, Vec<Entry>) = source
            .map(|source| source(&self.entry.text()))
            .unwrap_or_default()
            .into_iter()
            .unzip();
        self.tokens.replace(tokens);
        if !force && self.list.first_child().is_some() {
            if *self.shown.borrow() == shown {
                return;
            }
            // The same places with other counts -- a read landing with them
            // -- are the same rows: only what they say on the right moves.
            if same_but_counts(&self.shown.borrow(), &shown) {
                for (label, entry) in self.counts.borrow().iter().zip(&shown) {
                    label.set_text(entry.count.as_deref().unwrap_or_default());
                    label.set_visible(entry.count.is_some());
                }
                self.shown.replace(shown);
                return;
            }
        }
        while let Some(child) = self.list.first_child() {
            self.list.remove(&child);
        }
        let keymap = self.keymap.borrow();
        let accent_hue = accent_hue();
        let mut rows = Vec::new();
        let mut counts = Vec::new();
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
            let count = gtk::Label::new(entry.count.as_deref());
            count.add_css_class("dim-label");
            count.add_css_class("focus-places-count");
            count.set_visible(entry.count.is_some());
            row.append(&count);
            counts.push(count);
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
        self.counts.replace(counts);
        self.shown.replace(shown);
        if let Some(first) = first {
            self.list.select_row(Some(&first));
        }
    }
}

/// The flag: the mark of flagged mail, wherever it is named.
pub const FLAG_ICON: &str = "emoji-flags-symbolic";

/// The mark of the strip's "Has action" toggle -- mail waiting on a task,
/// which is not flagging, so it does not wear the flag.
pub const HAS_ACTION_ICON: &str = "task-due-symbolic";

/// The icon a mailbox of `role` is marked with.
fn icon(role: MailboxRole) -> &'static str {
    match role {
        MailboxRole::Inbox => "mail-read-symbolic",
        MailboxRole::Drafts => "document-edit-symbolic",
        MailboxRole::Sent => "mail-send-symbolic",
        // What is on its way out, not what went: an arrow leaving a box.
        MailboxRole::Outbox => "send-to-symbolic",
        MailboxRole::Snoozed => "alarm-symbolic",
        MailboxRole::Flagged => FLAG_ICON,
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

/// Whether `now` lists the places `was` does, in the same order, differing
/// at most in their counts.
fn same_but_counts(was: &[Entry], now: &[Entry]) -> bool {
    was.len() == now.len()
        && was.iter().zip(now).all(|(was, now)| {
            Entry {
                count: None,
                ..was.clone()
            } == Entry {
                count: None,
                ..now.clone()
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The flag is the mailbox of flagged mail, and nothing else wears it:
    /// the strip's "Has action" toggle has a mark of its own.
    #[test]
    fn the_flag_is_flagged_and_has_action_wears_another_mark() {
        assert_eq!(icon(MailboxRole::Flagged), FLAG_ICON);
        assert_ne!(HAS_ACTION_ICON, FLAG_ICON);
        for role in [MailboxRole::Inbox, MailboxRole::Snoozed, MailboxRole::Junk] {
            assert_ne!(icon(role), HAS_ACTION_ICON);
        }
    }

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
