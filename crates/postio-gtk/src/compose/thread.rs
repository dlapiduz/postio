//! What a thread adds to the composer: the *What to forward* chooser above To, the older-reply bar
//! under the title, and the newer messages read-only below what is written. The controller composes
//! the words; these draw them and hand back a click.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::prelude::*;
use postio_ui::compose::{ChooserView, OlderReplyView};
use postio_widgets::widgets::keyhint;
use postio_widgets::widgets::space;
use postio_widgets::widgets::{Kind, Size};

type CardHandler = Rc<dyn Fn(postio_model::ForwardForm)>;
type ActionHandler = Rc<dyn Fn(usize)>;

pub struct ThreadParts {
    pub older: gtk::Box,
    older_title: gtk::Label,
    older_detail: gtk::Label,
    older_buttons: [gtk::Button; 2],
    older_actions: RefCell<Vec<String>>,
    pub chooser: gtk::Box,
    cards: gtk::Box,
    chooser_view: RefCell<Option<ChooserView>>,
    pub newer: gtk::Box,
    newer_count: Cell<usize>,
    quoted_expanded: Cell<bool>,
    pub forwarded: gtk::Box,
    forwarded_count: Cell<usize>,
    on_unfold: RefCell<Option<Rc<dyn Fn()>>>,
    on_card: RefCell<Option<CardHandler>>,
    on_action: RefCell<Option<ActionHandler>>,
}

impl ThreadParts {
    pub fn new() -> Rc<Self> {
        let older = gtk::Box::new(gtk::Orientation::Horizontal, space::S3);
        older.add_css_class("focus-older-reply");
        older.set_visible(false);
        let older_title = gtk::Label::new(None);
        older_title.add_css_class("focus-older-reply-title");
        older_title.set_xalign(0.0);
        let older_detail = gtk::Label::new(None);
        older_detail.add_css_class("dim-label");
        older_detail.set_xalign(0.0);
        older_detail.set_wrap(true);
        let words = gtk::Box::new(gtk::Orientation::Vertical, space::S1);
        words.set_hexpand(true);
        words.append(&older_title);
        words.append(&older_detail);
        let button = || {
            let button = gtk::Button::new();
            postio_widgets::widgets::button::style(&button, Kind::Ghost, Size::Regular);
            button.set_valign(gtk::Align::Center);
            button
        };
        let older_buttons = [button(), button()];
        older.append(&words);
        older.append(&older_buttons[0]);
        older.append(&older_buttons[1]);

        let chooser = gtk::Box::new(gtk::Orientation::Vertical, space::S2);
        chooser.add_css_class("focus-forward-chooser");
        chooser.set_visible(false);
        // One stop in the tab order, above To: `1` `2` `3` are its own while it holds the keyboard,
        // and Tab moves on to To.
        chooser.set_focusable(true);
        chooser.update_property(&[gtk::accessible::Property::Label("What to forward")]);
        let cards = gtk::Box::new(gtk::Orientation::Horizontal, space::S2);
        cards.set_homogeneous(true);
        chooser.append(&cards);

        let newer = gtk::Box::new(gtk::Orientation::Vertical, space::S2);
        newer.add_css_class("focus-newer-messages");
        newer.set_visible(false);

        let forwarded = gtk::Box::new(gtk::Orientation::Vertical, space::S1);
        forwarded.add_css_class("focus-forwarded");
        forwarded.set_visible(false);

        let parts = Rc::new(ThreadParts {
            older,
            older_title,
            older_detail,
            older_buttons,
            older_actions: RefCell::default(),
            chooser,
            cards,
            chooser_view: RefCell::default(),
            newer,
            newer_count: Cell::new(0),
            quoted_expanded: Cell::new(false),
            forwarded,
            forwarded_count: Cell::new(0),
            on_unfold: RefCell::default(),
            on_card: RefCell::default(),
            on_action: RefCell::default(),
        });
        for (at, button) in parts.older_buttons.iter().enumerate() {
            let weak = Rc::downgrade(&parts);
            button.connect_clicked(move |_| {
                if let Some(parts) = weak.upgrade() {
                    let handler = parts.on_action.borrow().clone();
                    if let Some(handler) = handler {
                        handler(at);
                    }
                }
            });
        }
        parts
    }

    pub fn connect_card(&self, handler: impl Fn(postio_model::ForwardForm) + 'static) {
        self.on_card.replace(Some(Rc::new(handler)));
    }

    pub fn connect_action(&self, handler: impl Fn(usize) + 'static) {
        self.on_action.replace(Some(Rc::new(handler)));
    }

    pub fn show_chooser(self: &Rc<Self>, view: &ChooserView) {
        while let Some(child) = self.cards.first_child() {
            self.cards.remove(&child);
        }
        for card in &view.cards {
            let button = gtk::Button::new();
            button.add_css_class("focus-forward-card");
            // The chooser is the one stop; a click chooses and leaves the keyboard there.
            button.set_focusable(false);
            button.set_focus_on_click(false);
            if card.form == view.selected {
                button.add_css_class("focus-forward-card-selected");
            }
            let column = gtk::Box::new(gtk::Orientation::Vertical, space::S1);
            let top = gtk::Box::new(gtk::Orientation::Horizontal, space::S2);
            let title = gtk::Label::new(Some(&card.title));
            title.add_css_class("focus-forward-card-title");
            title.set_xalign(0.0);
            title.set_hexpand(true);
            top.append(&title);
            if let Some(key) = &card.key {
                top.append(&keyhint::cap(key));
            }
            let detail = gtk::Label::new(Some(&card.detail));
            detail.add_css_class("dim-label");
            detail.set_xalign(0.0);
            detail.set_wrap(true);
            column.append(&top);
            column.append(&detail);
            button.set_child(Some(&column));
            let (weak, form) = (Rc::downgrade(self), card.form);
            button.connect_clicked(move |_| {
                if let Some(parts) = weak.upgrade() {
                    parts.chooser.grab_focus();
                    let handler = parts.on_card.borrow().clone();
                    if let Some(handler) = handler {
                        handler(form);
                    }
                }
            });
            self.cards.append(&button);
        }
        self.chooser_view.replace(Some(view.clone()));
        self.chooser.set_visible(true);
    }

    pub fn show_older(&self, view: Option<&OlderReplyView>) {
        let Some(view) = view else {
            self.older.set_visible(false);
            self.older_actions.borrow_mut().clear();
            self.older_title.set_text("");
            return;
        };
        self.older_title.set_text(&view.title);
        self.older_detail.set_text(&view.detail);
        for (button, action) in self.older_buttons.iter().zip(&view.actions) {
            button.set_child(Some(&keyhint::labelled(
                &action.label,
                action.key.as_deref(),
            )));
        }
        self.older_actions.replace(
            view.actions
                .iter()
                .map(|action| action.label.clone())
                .collect(),
        );
        self.older.set_visible(true);
    }

    pub fn show_newer(&self, lines: &[(String, String)]) {
        while let Some(child) = self.newer.first_child() {
            self.newer.remove(&child);
        }
        self.newer_count.set(lines.len());
        for (heading, words) in lines {
            let block = gtk::Box::new(gtk::Orientation::Vertical, space::S1);
            block.add_css_class("focus-newer-message");
            let head = gtk::Label::new(Some(heading));
            head.add_css_class("dim-label");
            head.set_xalign(0.0);
            let text = gtk::Label::new(Some(words));
            text.set_xalign(0.0);
            text.set_wrap(true);
            text.set_selectable(false);
            text.set_focusable(false);
            block.append(&head);
            block.append(&text);
            self.newer.append(&block);
        }
        self.newer.set_visible(!lines.is_empty());
    }

    pub fn set_quoted_expanded(&self, expanded: bool) {
        self.quoted_expanded.set(expanded);
    }

    pub fn connect_unfold(&self, handler: impl Fn() + 'static) {
        self.on_unfold.replace(Some(Rc::new(handler)));
    }

    pub fn show_forwarded(
        self: &Rc<Self>,
        rows: &[postio_body::forward::ForwardedRow],
        cap: Option<&str>,
    ) {
        while let Some(child) = self.forwarded.first_child() {
            self.forwarded.remove(&child);
        }
        self.forwarded_count.set(rows.len());
        for row in rows {
            let button = gtk::Button::new();
            button.add_css_class("focus-forwarded-row");
            let line = gtk::Box::new(gtk::Orientation::Horizontal, space::S2);
            let words = if row.first.is_empty() {
                row.words()
            } else {
                format!("{} \u{b7} {}", row.words(), row.first)
            };
            let said = gtk::Label::new(Some(&words));
            said.add_css_class("dim-label");
            said.set_xalign(0.0);
            said.set_hexpand(true);
            said.set_ellipsize(gtk::pango::EllipsizeMode::End);
            line.append(&said);
            if row.number == rows.len()
                && let Some(cap) = cap
            {
                line.append(&keyhint::cap(cap));
            }
            button.set_child(Some(&line));
            button.update_property(&[gtk::accessible::Property::Label(&format!(
                "{}. Show the forwarded messages",
                row.words()
            ))]);
            let weak = Rc::downgrade(self);
            button.connect_clicked(move |_| {
                if let Some(parts) = weak.upgrade() {
                    let handler = parts.on_unfold.borrow().clone();
                    if let Some(handler) = handler {
                        handler();
                    }
                }
            });
            self.forwarded.append(&button);
        }
        self.forwarded.set_visible(!rows.is_empty());
    }

    pub fn forwarded_rows(&self) -> usize {
        self.forwarded_count.get()
    }

    pub fn clear(&self) {
        self.show_older(None);
        self.show_newer(&[]);
        self.forwarded_count.set(0);
        self.forwarded.set_visible(false);
        self.chooser.set_visible(false);
        self.chooser_view.replace(None);
        self.quoted_expanded.set(false);
    }

    pub fn chooser_has_focus(&self) -> bool {
        self.chooser.is_visible()
            && self
                .chooser
                .root()
                .and_then(|root| root.focus())
                .is_some_and(|focus| {
                    focus == *self.chooser.upcast_ref::<gtk::Widget>()
                        || focus.is_ancestor(&self.chooser)
                })
    }

    pub fn focus_chooser(&self) {
        if self.chooser.is_visible() {
            self.chooser.grab_focus();
        }
    }

    pub fn chooser_open(&self) -> bool {
        self.chooser.is_visible()
    }

    pub fn chooser_selected(&self) -> Option<String> {
        let view = self.chooser_view.borrow();
        let view = view.as_ref()?;
        view.cards
            .iter()
            .find(|card| card.form == view.selected)
            .map(|card| card.title.clone())
    }

    pub fn chooser_cards(&self) -> String {
        let view = self.chooser_view.borrow();
        view.as_ref().map_or_else(String::new, |view| {
            view.cards
                .iter()
                .map(|card| match &card.key {
                    Some(key) => format!("{} {key}", card.title),
                    None => card.title.clone(),
                })
                .collect::<Vec<_>>()
                .join(" | ")
        })
    }

    pub fn older_title(&self) -> Option<String> {
        self.older
            .is_visible()
            .then(|| self.older_title.text().to_string())
    }

    pub fn older_actions(&self) -> String {
        self.older_actions.borrow().join(" | ")
    }

    pub fn newer_count(&self) -> usize {
        self.newer_count.get()
    }

    pub fn quoted_expanded(&self) -> bool {
        self.quoted_expanded.get()
    }
}
