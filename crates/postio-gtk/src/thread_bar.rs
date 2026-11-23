//! The message window's thread bar and its older-message banner. The words are
//! `postio_ui::thread_bar`'s; this draws them.
//! The accent is reserved, so the current chip is the selection's neutral ring and an unread one is
//! dotted in the ink.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::prelude::*;
use postio_ui::thread_bar::{BannerView, Chip, ThreadBarView};
use postio_widgets::widgets::keyhint;
use postio_widgets::widgets::space;
use postio_widgets::widgets::{Kind, Size};

const CHIP_WIDTH: i32 = 72;
const CHIP_GAP: i32 = 8;
const FIXED: i32 = 66 + 100 + 48;

type RoomHandler = Rc<dyn Fn(u32)>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChipClick {
    Message(u32),
    More(u32),
}

type ChipHandler = Rc<RefCell<Option<Rc<dyn Fn(ChipClick)>>>>;

pub struct ThreadBar {
    root: gtk::Box,
    title: gtk::Label,
    count: gtk::Label,
    chips: gtk::Box,
    menu: gtk::Button,
    current: Cell<Option<u32>>,
    drawn: Cell<usize>,
    matched: RefCell<Vec<u32>>,
    room: Cell<u32>,
    on_room: RefCell<Option<RoomHandler>>,
    on_menu: RefCell<Option<Rc<dyn Fn()>>>,
    on_chip: ChipHandler,
}

impl ThreadBar {
    pub fn new() -> Rc<Self> {
        let root = gtk::Box::new(gtk::Orientation::Horizontal, space::S3);
        root.add_css_class("focus-thread-bar");
        // Never the column's spare height: a child that expands (the "… N more" chip's words) would
        // make the bar share it with the message, a tall band at a narrow window (#2202).
        root.set_vexpand(false);
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
            matched: RefCell::default(),
            room: Cell::new(0),
            on_room: RefCell::default(),
            on_menu: RefCell::default(),
            on_chip: ChipHandler::default(),
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
        let weak = Rc::downgrade(&bar);
        bar.root.add_tick_callback(move |root, _| {
            if let Some(bar) = weak.upgrade() {
                bar.measure_room(root.width());
            }
            glib::ControlFlow::Continue
        });
        bar
    }

    pub fn widget(&self) -> &gtk::Box {
        &self.root
    }

    pub fn menu_button(&self) -> &gtk::Button {
        &self.menu
    }

    pub fn connect_menu(&self, handler: impl Fn() + 'static) {
        self.on_menu.replace(Some(Rc::new(handler)));
    }

    pub fn connect_chip(&self, handler: impl Fn(ChipClick) + 'static) {
        self.on_chip.replace(Some(Rc::new(handler)));
    }

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
        self.matched.borrow_mut().clear();
        for chip in &view.chips {
            self.chips.append(&self.chip(chip));
        }
        self.drawn.set(view.chips.len());
        self.root.set_visible(!view.chips.is_empty());
    }

    pub fn clear(&self) {
        self.set(&ThreadBarView {
            title: String::new(),
            count_line: String::new(),
            chips: Vec::new(),
            menu_label: String::new(),
        });
    }

    pub fn current(&self) -> Option<u32> {
        self.current.get()
    }

    pub fn matched(&self) -> Vec<u32> {
        self.matched.borrow().clone()
    }

    pub fn chip_count(&self) -> usize {
        self.drawn.get()
    }

    fn chip(&self, chip: &Chip) -> gtk::Widget {
        let tile = gtk::Box::new(gtk::Orientation::Vertical, space::S1);
        tile.add_css_class("focus-thread-chip");
        tile.set_size_request(CHIP_WIDTH, 46);
        tile.set_valign(gtk::Align::Center);
        let click = match chip {
            Chip::Message { index, .. } => ChipClick::Message(*index),
            Chip::More { from, .. } => ChipClick::More(*from),
        };
        let handler = Rc::clone(&self.on_chip);
        let gesture = gtk::GestureClick::new();
        gesture.connect_released(move |gesture, _, _, _| {
            let handler = handler.borrow().clone();
            if let Some(handler) = handler {
                gesture.set_state(gtk::EventSequenceState::Claimed);
                handler(click);
            }
        });
        tile.add_controller(gesture);
        tile.set_cursor_from_name(Some("pointer"));
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
                    self.matched.borrow_mut().push(*index);
                    let mark = gtk::Label::new(None);
                    mark.set_markup(&format!(
                        "<span foreground=\"{}\">\u{25a0}</span>",
                        crate::dropdown::find_yellow()
                    ));
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

pub struct OlderBanner {
    root: gtk::Box,
    text: gtk::Label,
    action: gtk::Button,
    on_press: RefCell<Option<Rc<dyn Fn()>>>,
}

impl OlderBanner {
    pub fn new() -> Rc<Self> {
        Self::with_class("focus-older-banner")
    }

    pub fn with_class(class: &str) -> Rc<Self> {
        let root = gtk::Box::new(gtk::Orientation::Horizontal, space::S3);
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

    pub fn widget(&self) -> &gtk::Box {
        &self.root
    }

    pub fn connect_press(&self, handler: impl Fn() + 'static) {
        self.on_press.replace(Some(Rc::new(handler)));
    }

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

    pub fn said(&self) -> Option<String> {
        self.root.is_visible().then(|| self.text.text().to_string())
    }
}

pub struct MatchBanner {
    root: gtk::Box,
    text: gtk::Label,
    next: gtk::Button,
    back: gtk::Button,
    on_next: RefCell<Option<Rc<dyn Fn()>>>,
    on_back: RefCell<Option<Rc<dyn Fn()>>>,
}

impl MatchBanner {
    pub fn new() -> Rc<Self> {
        let root = gtk::Box::new(gtk::Orientation::Horizontal, space::S3);
        root.add_css_class("focus-banner-bar");
        root.add_css_class("focus-match-banner");
        root.set_visible(false);
        let text = gtk::Label::new(None);
        text.add_css_class("dim-label");
        text.set_wrap(true);
        text.set_xalign(0.0);
        text.set_hexpand(true);
        let button = || {
            let button = gtk::Button::new();
            postio_widgets::widgets::button::style(&button, Kind::Ghost, Size::Regular);
            button.set_valign(gtk::Align::Center);
            button
        };
        let (next, back) = (button(), button());
        root.append(&text);
        root.append(&next);
        root.append(&back);
        let banner = Rc::new(MatchBanner {
            root,
            text,
            next,
            back,
            on_next: RefCell::default(),
            on_back: RefCell::default(),
        });
        for (button, pick) in [(&banner.next, true), (&banner.back, false)] {
            let weak = Rc::downgrade(&banner);
            button.connect_clicked(move |_| {
                if let Some(banner) = weak.upgrade() {
                    let handler = if pick {
                        banner.on_next.borrow().clone()
                    } else {
                        banner.on_back.borrow().clone()
                    };
                    if let Some(handler) = handler {
                        handler();
                    }
                }
            });
        }
        banner
    }

    pub fn widget(&self) -> &gtk::Box {
        &self.root
    }

    pub fn connect(&self, next: impl Fn() + 'static, back: impl Fn() + 'static) {
        self.on_next.replace(Some(Rc::new(next)));
        self.on_back.replace(Some(Rc::new(back)));
    }

    pub fn set(&self, view: Option<&postio_ui::thread_bar::MatchBannerView>) {
        let Some(view) = view else {
            self.root.set_visible(false);
            self.text.set_text("");
            return;
        };
        self.text.set_text(&view.text);
        self.next.set_child(Some(&keyhint::labelled(
            &view.next,
            view.next_key.as_deref(),
        )));
        self.back.set_child(Some(&keyhint::labelled(
            &view.back,
            view.back_key.as_deref(),
        )));
        self.root.set_visible(true);
    }

    pub fn said(&self) -> Option<String> {
        self.root.is_visible().then(|| self.text.text().to_string())
    }
}
