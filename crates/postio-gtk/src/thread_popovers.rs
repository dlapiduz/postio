//! The thread menu and the outline: the two popovers that hang from the
//! message window's Thread button (spec 013 US5; `Design/focus-macos-threads`
//! screens 04 and 05).
//!
//! The controller composes both (`postio_ui::thread_outline`) and says when to
//! draw them; these only draw what they are given and hand back what was
//! pressed. The menu is 340 wide, in two groups of entries (title, key, and a
//! line under it where it helps). The outline is 440 wide: a filter field,
//! a chip with a count for each person, 34-tall rows, and a footer of keys.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::prelude::*;
use postio_core::CommandId;
use postio_focus::OutlineInput;
use postio_ui::thread_outline::{MenuEntry, OutlineView, ThreadMenuView};
use postio_widgets::widgets::keyhint;
use postio_widgets::widgets::{Kind, Size};

/// The menu's width, in pixels.
const MENU_WIDTH: i32 = 340;
/// The outline's width.
const OUTLINE_WIDTH: i32 = 440;
/// An outline row's height.
const ROW_HEIGHT: i32 = 34;

fn popover(parent: &impl IsA<gtk::Widget>, class: &str, width: i32) -> gtk::Popover {
    let popover = gtk::Popover::new();
    popover.add_css_class(class);
    popover.set_has_arrow(false);
    popover.set_position(gtk::PositionType::Bottom);
    popover.set_size_request(width, -1);
    popover.set_parent(parent);
    let held = popover.downgrade();
    parent.connect_destroy(move |_| {
        if let Some(popover) = held.upgrade() {
            popover.unparent();
        }
    });
    popover
}

/// Told which entry a click chose.
type PickHandler = Rc<dyn Fn(CommandId)>;

/// The menu under Thread.
pub struct ThreadMenu {
    popover: gtk::Popover,
    body: gtk::Box,
    shown: RefCell<ThreadMenuView>,
    on_pick: RefCell<Option<PickHandler>>,
    on_closed: RefCell<Option<Rc<dyn Fn()>>>,
}

impl ThreadMenu {
    /// A menu hung from `parent`.
    pub fn new(parent: &impl IsA<gtk::Widget>) -> Rc<Self> {
        let popover = popover(parent, "focus-thread-menu", MENU_WIDTH);
        let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
        body.add_css_class("focus-thread-menu-body");
        popover.set_child(Some(&body));
        let menu = Rc::new(ThreadMenu {
            popover,
            body,
            shown: RefCell::default(),
            on_pick: RefCell::default(),
            on_closed: RefCell::default(),
        });
        let weak = Rc::downgrade(&menu);
        menu.popover.connect_closed(move |_| {
            if let Some(menu) = weak.upgrade() {
                let handler = menu.on_closed.borrow().clone();
                if let Some(handler) = handler {
                    handler();
                }
            }
        });
        menu
    }

    /// Run `handler` with the command of the entry a click chose.
    pub fn connect_pick(&self, handler: impl Fn(CommandId) + 'static) {
        self.on_pick.replace(Some(Rc::new(handler)));
    }

    /// Run `handler` when the popover has closed, by whatever way.
    pub fn connect_closed(&self, handler: impl Fn() + 'static) {
        self.on_closed.replace(Some(Rc::new(handler)));
    }

    /// Draw `view` and open the popover.
    pub fn show(self: &Rc<Self>, view: &ThreadMenuView) {
        while let Some(child) = self.body.first_child() {
            self.body.remove(&child);
        }
        for group in &view.groups {
            let heading = gtk::Label::new(Some(&group.title));
            heading.add_css_class("focus-thread-menu-heading");
            heading.set_xalign(0.0);
            self.body.append(&heading);
            for entry in &group.entries {
                self.body.append(&self.entry(entry));
            }
        }
        self.shown.replace(view.clone());
        self.popover.popup();
    }

    fn entry(self: &Rc<Self>, entry: &MenuEntry) -> gtk::Button {
        let button = gtk::Button::new();
        postio_widgets::widgets::button::style(&button, Kind::Ghost, Size::Regular);
        button.add_css_class("focus-thread-menu-entry");
        button.set_sensitive(entry.enabled);
        let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let top = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let title = gtk::Label::new(Some(&entry.title));
        title.set_xalign(0.0);
        title.set_hexpand(true);
        top.append(&title);
        if let Some(key) = &entry.keycap {
            top.append(&keyhint::cap(key));
        }
        column.append(&top);
        if let Some(note) = &entry.note {
            let note = gtk::Label::new(Some(note));
            note.add_css_class("dim-label");
            note.add_css_class("focus-thread-menu-note");
            note.set_xalign(0.0);
            column.append(&note);
        }
        button.set_child(Some(&column));
        let command = entry.command;
        let weak = Rc::downgrade(self);
        button.connect_clicked(move |_| {
            if let Some(menu) = weak.upgrade() {
                let handler = menu.on_pick.borrow().clone();
                if let Some(handler) = handler {
                    handler(command);
                }
            }
        });
        button
    }

    /// Put the popover away.
    pub fn close(&self) {
        self.popover.popdown();
        self.shown.replace(ThreadMenuView::default());
    }

    /// Whether it is up.
    pub fn is_open(&self) -> bool {
        self.popover.is_visible()
    }

    /// The group headings, " | " apart: what a test reads.
    pub fn groups_said(&self) -> String {
        let shown = self.shown.borrow();
        let titles: Vec<&str> = shown.groups.iter().map(|g| g.title.as_str()).collect();
        titles.join(" | ")
    }

    /// Every entry's key, " | " apart.
    pub fn keycaps_said(&self) -> String {
        let shown = self.shown.borrow();
        let caps: Vec<&str> = shown
            .groups
            .iter()
            .flat_map(|group| group.entries.iter())
            .filter_map(|entry| entry.keycap.as_deref())
            .collect();
        caps.join(" | ")
    }
}

type OutlineHandler = Rc<dyn Fn(OutlineInput)>;

/// The outline.
pub struct OutlinePopover {
    popover: gtk::Popover,
    body: gtk::Box,
    filter: gtk::Entry,
    chips: gtk::Box,
    rows: gtk::Box,
    footer: gtk::Label,
    shown: RefCell<OutlineView>,
    /// Raised while a view's text is being put into the field, so it is not
    /// said back as typing.
    drawing: Cell<bool>,
    on_input: RefCell<Option<OutlineHandler>>,
    on_closed: RefCell<Option<Rc<dyn Fn()>>>,
}

impl OutlinePopover {
    /// An outline hung from `parent`.
    pub fn new(parent: &impl IsA<gtk::Widget>) -> Rc<Self> {
        let popover = popover(parent, "focus-outline", OUTLINE_WIDTH);
        let filter = gtk::Entry::new();
        filter.add_css_class("focus-outline-filter");
        filter.set_placeholder_text(Some("Filter this thread"));
        let chips = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        chips.add_css_class("focus-outline-people");
        let rows = gtk::Box::new(gtk::Orientation::Vertical, 0);
        rows.add_css_class("focus-outline-rows");
        let scroller = gtk::ScrolledWindow::builder()
            .child(&rows)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .max_content_height(340)
            .propagate_natural_height(true)
            .build();
        let footer = gtk::Label::new(None);
        footer.add_css_class("dim-label");
        footer.add_css_class("focus-outline-footer");
        footer.set_xalign(0.0);
        let body = gtk::Box::new(gtk::Orientation::Vertical, 8);
        body.add_css_class("focus-outline-body");
        body.append(&filter);
        body.append(&chips);
        body.append(&scroller);
        body.append(&footer);
        // The keyboard starts on the list, not the field: `u` and the arrows
        // are the outline's until `/` takes the keyboard to the field.
        body.set_focusable(true);
        popover.set_child(Some(&body));
        let outline = Rc::new(OutlinePopover {
            popover,
            body,
            filter,
            chips,
            rows,
            footer,
            shown: RefCell::default(),
            drawing: Cell::new(false),
            on_input: RefCell::default(),
            on_closed: RefCell::default(),
        });
        let weak = Rc::downgrade(&outline);
        outline.filter.connect_changed({
            let weak = weak.clone();
            move |field| {
                if let Some(outline) = weak.upgrade()
                    && !outline.drawing.get()
                {
                    outline.say(OutlineInput::Typed(field.text().to_string()));
                }
            }
        });
        // Arrows and Return are the outline's even while the field holds the
        // keyboard: the list is what they move.
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        keys.connect_key_pressed({
            let weak = weak.clone();
            move |_, key, _, _| {
                let Some(outline) = weak.upgrade() else {
                    return glib::Propagation::Proceed;
                };
                let input = match key {
                    gtk::gdk::Key::Down => OutlineInput::Move(1),
                    gtk::gdk::Key::Up => OutlineInput::Move(-1),
                    gtk::gdk::Key::Return | gtk::gdk::Key::KP_Enter => OutlineInput::Show(None),
                    _ => return glib::Propagation::Proceed,
                };
                outline.say(input);
                glib::Propagation::Stop
            }
        });
        outline.filter.add_controller(keys);
        outline.popover.connect_closed(move |_| {
            if let Some(outline) = weak.upgrade() {
                let handler = outline.on_closed.borrow().clone();
                if let Some(handler) = handler {
                    handler();
                }
            }
        });
        outline
    }

    fn say(&self, input: OutlineInput) {
        let handler = self.on_input.borrow().clone();
        if let Some(handler) = handler {
            handler(input);
        }
    }

    /// Run `handler` with what is typed, chosen, moved or shown.
    pub fn connect_input(&self, handler: impl Fn(OutlineInput) + 'static) {
        self.on_input.replace(Some(Rc::new(handler)));
    }

    /// Run `handler` when the popover has closed, by whatever way.
    pub fn connect_closed(&self, handler: impl Fn() + 'static) {
        self.on_closed.replace(Some(Rc::new(handler)));
    }

    /// Draw `view` whole, opening the popover if it is not up.
    pub fn show(self: &Rc<Self>, view: &OutlineView) {
        self.drawing.set(true);
        if self.filter.text() != view.filter {
            self.filter.set_text(&view.filter);
            self.filter.set_position(-1);
        }
        self.drawing.set(false);
        while let Some(child) = self.chips.first_child() {
            self.chips.remove(&child);
        }
        for (name, count) in &view.people {
            let chip = gtk::ToggleButton::with_label(&format!("{name} \u{b7} {count}"));
            chip.add_css_class("focus-outline-chip");
            chip.set_active(view.person.as_deref() == Some(name.as_str()));
            let (weak, name) = (Rc::downgrade(self), name.clone());
            chip.connect_clicked(move |chip| {
                if let Some(outline) = weak.upgrade() {
                    outline.say(OutlineInput::Person(chip.is_active().then(|| name.clone())));
                }
            });
            self.chips.append(&chip);
        }
        while let Some(child) = self.rows.first_child() {
            self.rows.remove(&child);
        }
        for (at, row) in view.rows.iter().enumerate() {
            let line = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            line.add_css_class("focus-outline-row");
            line.set_size_request(-1, ROW_HEIGHT);
            if row.current {
                line.add_css_class("focus-outline-current");
            }
            if at == view.cursor {
                line.add_css_class("focus-outline-cursor");
            }
            let number = gtk::Label::new(Some(&row.number.to_string()));
            number.add_css_class("dim-label");
            number.set_width_chars(2);
            let avatar = gtk::Label::new(Some(&row.initials));
            avatar.add_css_class("focus-thread-avatar");
            let name = gtk::Label::new(Some(&row.name));
            name.set_xalign(0.0);
            name.set_width_chars(10);
            name.set_max_width_chars(10);
            name.set_ellipsize(gtk::pango::EllipsizeMode::End);
            if row.bold {
                name.add_css_class("focus-outline-unread");
            }
            let words = gtk::Label::new(Some(&row.line));
            words.add_css_class("dim-label");
            words.set_xalign(0.0);
            words.set_hexpand(true);
            words.set_ellipsize(gtk::pango::EllipsizeMode::End);
            let date = gtk::Label::new(Some(&row.date));
            date.add_css_class("dim-label");
            let dot = gtk::Label::new(Some(if row.unread { "\u{25cf}" } else { " " }));
            dot.add_css_class("focus-thread-dot");
            for widget in [
                number.upcast_ref::<gtk::Widget>(),
                avatar.upcast_ref(),
                name.upcast_ref(),
                words.upcast_ref(),
                date.upcast_ref(),
                dot.upcast_ref(),
            ] {
                line.append(widget);
            }
            let click = gtk::GestureClick::new();
            let weak = Rc::downgrade(self);
            click.connect_pressed(move |gesture, _, _, _| {
                if let Some(outline) = weak.upgrade() {
                    gesture.set_state(gtk::EventSequenceState::Claimed);
                    outline.say(OutlineInput::Show(Some(at)));
                }
            });
            line.add_controller(click);
            self.rows.append(&line);
        }
        let mut footer = String::from("\u{2191}\u{2193} move \u{b7} \u{21a9} show it");
        if let Some(key) = &view.first_unread_key {
            footer.push_str(&format!(" \u{b7} {key} first unread"));
        }
        self.footer.set_text(&footer);
        self.shown.replace(view.clone());
        if !self.popover.is_visible() {
            self.popover.popup();
            self.body.grab_focus();
        }
    }

    /// The keyboard goes to the filter field.
    pub fn focus_filter(&self) {
        self.filter.grab_focus();
    }

    /// Put the popover away.
    pub fn close(&self) {
        self.popover.popdown();
    }

    /// Whether it is up.
    pub fn is_open(&self) -> bool {
        self.popover.is_visible()
    }

    /// Whether the field holds the keyboard.
    pub fn filtering(&self) -> bool {
        self.filter.has_focus()
            || self
                .filter
                .first_child()
                .is_some_and(|text| text.is_focus())
    }

    /// How many rows are drawn.
    pub fn row_count(&self) -> usize {
        self.shown.borrow().rows.len()
    }

    /// The row the keyboard is on.
    pub fn cursor(&self) -> usize {
        self.shown.borrow().cursor
    }
}
