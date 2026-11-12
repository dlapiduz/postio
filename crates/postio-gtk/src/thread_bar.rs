//! The message window's thread bar and its older-message banner (spec 013
//! US3; `Design/focus-macos-threads` screen 03).
//!
//! The bar is 60 tall, under the action row: a label column ("Thread", and
//! "6 · 2 new" under it), one 72 by 46 chip per message (an avatar over
//! "Name · time", the current one ringed, an unread one dotted, a match
//! marked), "... N more" for the ones that did not fit, and a Thread button
//! at the right that opens the menu once the menu exists (spec 013 US5).
//!
//! The words are `postio_ui::thread_bar`'s, composed once for both apps; this
//! draws them. The accent is reserved (FR-091), so the current chip is the
//! selection's neutral ring and an unread one is dotted in the ink.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::prelude::*;
use postio_ui::thread_bar::{BannerView, Chip, ThreadBarView};
use postio_widgets::widgets::keyhint;
use postio_widgets::widgets::{Kind, Size};

/// A chip's width and its gap to the next, in pixels: what a bar's room is
/// counted in.
const CHIP_WIDTH: i32 = 72;
const CHIP_GAP: i32 = 8;
/// What the label column, the Thread button and the padding take from the
/// bar's width.
const FIXED: i32 = 66 + 100 + 48;

/// Told how many chips the bar has room for, whenever that changes.
type RoomHandler = Rc<dyn Fn(u32)>;

/// The bar.
pub struct ThreadBar {
    root: gtk::Box,
    title: gtk::Label,
    count: gtk::Label,
    chips: gtk::Box,
    menu: gtk::Button,
    /// The chip on screen, from zero.
    current: Cell<Option<u32>>,
    /// How many chips are drawn.
    drawn: Cell<usize>,
    room: Cell<u32>,
    on_room: RefCell<Option<RoomHandler>>,
    on_menu: RefCell<Option<Rc<dyn Fn()>>>,
}

impl ThreadBar {
    /// An empty bar, drawn once a thread says it has chips.
    pub fn new() -> Rc<Self> {
        let root = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        root.add_css_class("focus-thread-bar");
        root.set_visible(false);
        let title = gtk::Label::new(None);
        title.add_css_class("focus-thread-bar-title");
        title.set_xalign(0.0);
        let count = gtk::Label::new(None);
        count.add_css_class("dim-label");
        count.add_css_class("focus-thread-bar-count");
        count.set_xalign(0.0);
        let label = gtk::Box::new(gtk::Orientation::Vertical, 0);
        label.set_size_request(66, -1);
        label.set_valign(gtk::Align::Center);
        label.append(&title);
        label.append(&count);
        let chips = gtk::Box::new(gtk::Orientation::Horizontal, CHIP_GAP);
        chips.set_hexpand(true);
        chips.set_valign(gtk::Align::Center);
        let menu = gtk::Button::new();
        postio_widgets::widgets::button::style(&menu, Kind::Ghost, Size::Regular);
        menu.add_css_class("focus-thread-bar-menu");
        menu.set_valign(gtk::Align::Center);
        root.append(&label);
        root.append(&chips);
        root.append(&menu);
        let bar = Rc::new(ThreadBar {
            root,
            title,
            count,
            chips,
            menu,
            current: Cell::new(None),
            drawn: Cell::new(0),
            room: Cell::new(0),
            on_room: RefCell::default(),
            on_menu: RefCell::default(),
        });
        let weak = Rc::downgrade(&bar);
        bar.menu.connect_clicked(move |_| {
            if let Some(bar) = weak.upgrade() {
                let handler = bar.on_menu.borrow().clone();
                if let Some(handler) = handler {
                    handler();
                }
            }
        });
        // Say how many chips fit when the bar's width changes.
        let weak = Rc::downgrade(&bar);
        bar.root.add_tick_callback(move |root, _| {
            if let Some(bar) = weak.upgrade() {
                bar.measure_room(root.width());
            }
            glib::ControlFlow::Continue
        });
        bar
    }

    /// The bar, to place under the action row.
    pub fn widget(&self) -> &gtk::Box {
        &self.root
    }

    /// The Thread button: what the thread menu hangs from.
    pub fn menu_button(&self) -> &gtk::Button {
        &self.menu
    }

    /// Run `handler` when the Thread button is pressed.
    pub fn connect_menu(&self, handler: impl Fn() + 'static) {
        self.on_menu.replace(Some(Rc::new(handler)));
    }

    /// Run `handler` with how many chips fit, once the bar has a width and
    /// whenever it changes.
    pub fn connect_room(&self, handler: impl Fn(u32) + 'static) {
        self.on_room.replace(Some(Rc::new(handler)));
        self.room.set(0);
    }

    fn measure_room(&self, width: i32) {
        if width <= 0 {
            return;
        }
        let room = ((width - FIXED + CHIP_GAP) / (CHIP_WIDTH + CHIP_GAP)).max(2) as u32;
        if self.room.replace(room) == room {
            return;
        }
        let handler = self.on_room.borrow().clone();
        if let Some(handler) = handler {
            handler(room);
        }
    }

    /// Draw `view`; a thread of one message has no chips, so no bar.
    pub fn set(&self, view: &ThreadBarView) {
        self.title.set_text(&view.title);
        self.count.set_text(&view.count_line);
        self.menu.set_child(Some(&gtk::Label::new(Some(&format!(
            "{} \u{25be}",
            view.menu_label
        )))));
        while let Some(child) = self.chips.first_child() {
            self.chips.remove(&child);
        }
        self.current.set(None);
        for chip in &view.chips {
            self.chips.append(&self.chip(chip));
        }
        self.drawn.set(view.chips.len());
        self.root.set_visible(!view.chips.is_empty());
    }

    /// No bar: nothing is shown, or a message of no thread.
    pub fn clear(&self) {
        self.set(&ThreadBarView {
            title: String::new(),
            count_line: String::new(),
            chips: Vec::new(),
            menu_label: String::new(),
        });
    }

    /// The chip on screen, from zero.
    pub fn current(&self) -> Option<u32> {
        self.current.get()
    }

    /// How many chips are drawn.
    pub fn chip_count(&self) -> usize {
        self.drawn.get()
    }

    fn chip(&self, chip: &Chip) -> gtk::Widget {
        let tile = gtk::Box::new(gtk::Orientation::Vertical, 2);
        tile.add_css_class("focus-thread-chip");
        tile.set_size_request(CHIP_WIDTH, 46);
        tile.set_valign(gtk::Align::Center);
        match chip {
            Chip::Message {
                index,
                initials,
                label,
                current,
                unread,
                matched,
            } => {
                let avatar = gtk::Label::new(Some(initials));
                avatar.add_css_class("focus-thread-avatar");
                avatar.set_halign(gtk::Align::Center);
                let name = gtk::Label::new(Some(label));
                name.add_css_class("focus-thread-chip-name");
                name.set_ellipsize(gtk::pango::EllipsizeMode::End);
                name.set_max_width_chars(11);
                let top = gtk::Overlay::new();
                top.set_child(Some(&avatar));
                if *unread {
                    let dot = gtk::Label::new(Some("\u{25cf}"));
                    dot.add_css_class("focus-thread-dot");
                    dot.set_halign(gtk::Align::End);
                    dot.set_valign(gtk::Align::Start);
                    top.add_overlay(&dot);
                    tile.add_css_class("focus-thread-chip-unread");
                }
                if *matched {
                    let mark = gtk::Label::new(Some("\u{25a0}"));
                    mark.add_css_class("focus-thread-match");
                    mark.set_halign(gtk::Align::Start);
                    mark.set_valign(gtk::Align::Start);
                    top.add_overlay(&mark);
                }
                tile.append(&top);
                tile.append(&name);
                if *current {
                    tile.add_css_class("focus-thread-chip-current");
                    self.current.set(Some(*index));
                }
                tile.update_property(&[gtk::accessible::Property::Label(label)]);
            }
            Chip::More { count, .. } => {
                tile.add_css_class("focus-thread-chip-more");
                let words = gtk::Label::new(Some(&format!("\u{2026} {count} more")));
                words.add_css_class("dim-label");
                words.set_vexpand(true);
                tile.append(&words);
            }
        }
        tile.upcast()
    }
}

/// The quiet banner on a message that is not the latest.
pub struct OlderBanner {
    root: gtk::Box,
    text: gtk::Label,
    action: gtk::Button,
    on_press: RefCell<Option<Rc<dyn Fn()>>>,
}

impl OlderBanner {
    /// A banner, hidden until there is something to say.
    pub fn new() -> Rc<Self> {
        Self::with_class("focus-older-banner")
    }

    /// A banner of its own kind, styled by `class`.
    pub fn with_class(class: &str) -> Rc<Self> {
        let root = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        root.add_css_class("focus-banner-bar");
        root.add_css_class(class);
        root.set_visible(false);
        let text = gtk::Label::new(None);
        text.add_css_class("dim-label");
        text.set_wrap(true);
        text.set_xalign(0.0);
        text.set_hexpand(true);
        let action = gtk::Button::new();
        postio_widgets::widgets::button::style(&action, Kind::Ghost, Size::Regular);
        action.set_valign(gtk::Align::Center);
        root.append(&text);
        root.append(&action);
        let banner = Rc::new(OlderBanner {
            root,
            text,
            action,
            on_press: RefCell::default(),
        });
        let weak = Rc::downgrade(&banner);
        banner.action.connect_clicked(move |_| {
            if let Some(banner) = weak.upgrade() {
                let handler = banner.on_press.borrow().clone();
                if let Some(handler) = handler {
                    handler();
                }
            }
        });
        banner
    }

    /// The banner, to place above the kicker.
    pub fn widget(&self) -> &gtk::Box {
        &self.root
    }

    /// Run `handler` when its button is pressed: show the latest message.
    pub fn connect_press(&self, handler: impl Fn() + 'static) {
        self.on_press.replace(Some(Rc::new(handler)));
    }

    /// Draw `view`, or take the banner away.
    pub fn set(&self, view: Option<&BannerView>) {
        let Some(view) = view else {
            self.root.set_visible(false);
            self.text.set_text("");
            return;
        };
        self.text.set_text(&view.text);
        self.action
            .set_child(Some(&keyhint::labelled(&view.action, view.key.as_deref())));
        self.root.set_visible(true);
    }

    /// What it says, while it is up.
    pub fn said(&self) -> Option<String> {
        self.root.is_visible().then(|| self.text.text().to_string())
    }
}
