//! The thread menu and the outline: the two popovers hung from the message window's Thread button.
//! The controller composes both (`postio_ui::thread_outline`) and says when to draw them; these
//! draw what they are given and hand back what was pressed.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::prelude::*;
use postio_core::CommandId;
use postio_focus::OutlineInput;
use postio_ui::thread_outline::{MenuEntry, OutlineView, ThreadMenuView};
use postio_widgets::widgets::keyhint;
use postio_widgets::widgets::space;
use postio_widgets::widgets::{Kind, Size};

const MENU_WIDTH: i32 = 340;
const OUTLINE_WIDTH: i32 = 440;
const ROW_HEIGHT: i32 = 34;

fn popover(
    parent: &impl IsA<gtk::Widget>,
    class: &str,
    width: i32,
    body: &gtk::Box,
) -> gtk::Popover {
    let popover = gtk::Popover::new();
    popover.add_css_class(class);
    popover.set_has_arrow(false);
    popover.set_position(gtk::PositionType::Bottom);
    // Hung from the button's right edge: the button is at the dialog's right, and a popover centred
    // on it would run past the window.
    popover.set_halign(gtk::Align::End);
    // The popover's own size counts its shadow; the box a person sees is the unpadded contents,
    // which is the body (focus.css).
    body.set_size_request(width, -1);
    popover.set_child(Some(body));
    popover.set_parent(parent);
    let held = popover.downgrade();
    parent.connect_destroy(move |_| {
        if let Some(popover) = held.upgrade() {
            popover.unparent();
        }
    });
    popover
}

type PickHandler = Rc<dyn Fn(CommandId)>;

pub struct ThreadMenu {
    popover: gtk::Popover,
    body: gtk::Box,
    shown: RefCell<ThreadMenuView>,
    on_pick: RefCell<Option<PickHandler>>,
    on_closed: RefCell<Option<Rc<dyn Fn()>>>,
}

impl ThreadMenu {
    pub fn controls() -> Vec<CommandId> {
        let keymap = postio_core::Keymap::defaults();
        [false, true]
            .into_iter()
            .flat_map(|muted| postio_ui::thread_outline::menu(keymap, muted, true).groups)
            .flat_map(|group| group.entries)
            .map(|entry| entry.command)
            .collect()
    }

    pub fn new(parent: &impl IsA<gtk::Widget>) -> Rc<Self> {
        let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
        body.add_css_class("focus-thread-menu-body");
        let popover = popover(parent, "focus-thread-menu", MENU_WIDTH, &body);
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

    pub fn connect_pick(&self, handler: impl Fn(CommandId) + 'static) {
        self.on_pick.replace(Some(Rc::new(handler)));
    }

    pub fn connect_closed(&self, handler: impl Fn() + 'static) {
        self.on_closed.replace(Some(Rc::new(handler)));
    }

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
        let top = gtk::Box::new(gtk::Orientation::Horizontal, space::S2);
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

    pub fn close(&self) {
        self.popover.popdown();
        self.shown.replace(ThreadMenuView::default());
    }

    pub fn is_open(&self) -> bool {
        self.popover.is_visible()
    }

    pub fn groups_said(&self) -> String {
        let shown = self.shown.borrow();
        let titles: Vec<&str> = shown.groups.iter().map(|g| g.title.as_str()).collect();
        titles.join(" | ")
    }

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

pub struct OutlinePopover {
    popover: gtk::Popover,
    body: gtk::Box,
    filter: gtk::Entry,
    chips: adw::WrapBox,
    rows: gtk::Box,
    footer: gtk::Label,
    shown: RefCell<OutlineView>,
    /// Raised while a view's text is put into the field, so it is not said back as typing.
    drawing: Cell<bool>,
    on_input: RefCell<Option<OutlineHandler>>,
    on_closed: RefCell<Option<Rc<dyn Fn()>>>,
}

impl OutlinePopover {
    pub fn new(parent: &impl IsA<gtk::Widget>) -> Rc<Self> {
        let filter = gtk::Entry::new();
        filter.add_css_class("focus-outline-filter");
        filter.set_placeholder_text(Some("Filter this thread"));
        // A popover takes its child's natural width, and a row of chips side by side is as wide
        // as all of them: the chips wrap within the outline's width instead (#2233).
        let chips = adw::WrapBox::builder()
            .child_spacing(space::S2)
            .line_spacing(space::S2)
            .natural_line_length(0)
            .css_classes(["focus-outline-people"])
            .build();
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
        let body = gtk::Box::new(gtk::Orientation::Vertical, space::S2);
        body.add_css_class("focus-outline-body");
        body.append(&filter);
        body.append(&chips);
        body.append(&scroller);
        body.append(&footer);
        // `u` and the arrows are the outline's until `/` takes the keyboard to the field.
        body.set_focusable(true);
        let popover = popover(parent, "focus-outline", OUTLINE_WIDTH, &body);
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
        let focus = gtk::EventControllerFocus::new();
        let filtering = |weak: std::rc::Weak<OutlinePopover>, filtering: bool| {
            move |_: &gtk::EventControllerFocus| {
                if let Some(outline) = weak.upgrade() {
                    outline.say(OutlineInput::Filtering(filtering));
                }
            }
        };
        focus.connect_enter(filtering(weak.clone(), true));
        focus.connect_leave(filtering(weak.clone(), false));
        outline.filter.add_controller(focus);
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

    pub fn connect_input(&self, handler: impl Fn(OutlineInput) + 'static) {
        self.on_input.replace(Some(Rc::new(handler)));
    }

    pub fn connect_closed(&self, handler: impl Fn() + 'static) {
        self.on_closed.replace(Some(Rc::new(handler)));
    }

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
            let line = gtk::Box::new(gtk::Orientation::Horizontal, space::S2);
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
            // An ellipsized label still asks for its whole line; the row fills the outline instead.
            words.set_max_width_chars(1);
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

    pub fn focus_filter(&self) {
        self.filter.grab_focus();
    }

    pub fn close(&self) {
        self.popover.popdown();
    }

    pub fn parent(&self) -> Option<gtk::Widget> {
        self.popover.parent()
    }

    pub fn point_at(&self, rect: Option<&gtk::gdk::Rectangle>) {
        self.popover.set_pointing_to(rect);
    }

    pub fn discard(&self) {
        self.popover.popdown();
        self.popover.unparent();
    }

    pub fn is_open(&self) -> bool {
        self.popover.is_visible()
    }

    pub fn filtering(&self) -> bool {
        self.filter.has_focus()
            || self
                .filter
                .first_child()
                .is_some_and(|text| text.is_focus())
    }

    pub fn row_count(&self) -> usize {
        self.shown.borrow().rows.len()
    }

    pub fn hint(&self) -> String {
        self.footer.text().to_string()
    }

    pub fn cursor(&self) -> usize {
        self.shown.borrow().cursor
    }
}
