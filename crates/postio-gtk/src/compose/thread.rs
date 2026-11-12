//! What a thread adds to the composer (spec 013 US7; screens 09 and 10): the
//! *What to forward* chooser above To, the bar under the title on a reply to
//! a message that is not the latest, and the messages newer than the one
//! answered, read-only, below what is written.
//!
//! The controller composes the words (`postio_ui::compose`) and says when;
//! these draw them and hand back a click.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::prelude::*;
use postio_ui::compose::{ChooserView, OlderReplyView};
use postio_widgets::widgets::keyhint;
use postio_widgets::widgets::{Kind, Size};

/// Told which card a click chose.
type CardHandler = Rc<dyn Fn(postio_model::ForwardForm)>;
/// Told which of the bar's two buttons a click pressed (0 or 1).
type ActionHandler = Rc<dyn Fn(usize)>;

/// The thread's parts of the composer.
pub struct ThreadParts {
    /// The bar under the title: placed as a top bar of the dialog.
    pub older: gtk::Box,
    older_title: gtk::Label,
    older_detail: gtk::Label,
    older_buttons: [gtk::Button; 2],
    older_actions: RefCell<Vec<String>>,
    /// The chooser above To: placed as a top bar of the dialog.
    pub chooser: gtk::Box,
    cards: gtk::Box,
    chooser_view: RefCell<Option<ChooserView>>,
    /// The newer messages, below what is written: placed in the column.
    pub newer: gtk::Box,
    newer_count: Cell<usize>,
    quoted_expanded: Cell<bool>,
    on_card: RefCell<Option<CardHandler>>,
    on_action: RefCell<Option<ActionHandler>>,
}

impl ThreadParts {
    /// Hidden parts, drawn when the controller says.
    pub fn new() -> Rc<Self> {
        let older = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        older.add_css_class("focus-older-reply");
        older.set_visible(false);
        let older_title = gtk::Label::new(None);
        older_title.add_css_class("focus-older-reply-title");
        older_title.set_xalign(0.0);
        let older_detail = gtk::Label::new(None);
        older_detail.add_css_class("dim-label");
        older_detail.set_xalign(0.0);
        older_detail.set_wrap(true);
        let words = gtk::Box::new(gtk::Orientation::Vertical, 2);
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

        let chooser = gtk::Box::new(gtk::Orientation::Vertical, 6);
        chooser.add_css_class("focus-forward-chooser");
        chooser.set_visible(false);
        let cards = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        cards.set_homogeneous(true);
        chooser.append(&cards);

        let newer = gtk::Box::new(gtk::Orientation::Vertical, 8);
        newer.add_css_class("focus-newer-messages");
        newer.set_visible(false);

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

    /// Run `handler` with the card a click chose.
    pub fn connect_card(&self, handler: impl Fn(postio_model::ForwardForm) + 'static) {
        self.on_card.replace(Some(Rc::new(handler)));
    }

    /// Run `handler` with the index of the bar's button a click pressed.
    pub fn connect_action(&self, handler: impl Fn(usize) + 'static) {
        self.on_action.replace(Some(Rc::new(handler)));
    }

    /// Draw the chooser, the chosen card ringed.
    pub fn show_chooser(self: &Rc<Self>, view: &ChooserView) {
        while let Some(child) = self.cards.first_child() {
            self.cards.remove(&child);
        }
        for card in &view.cards {
            let button = gtk::Button::new();
            button.add_css_class("focus-forward-card");
            if card.form == view.selected {
                button.add_css_class("focus-forward-card-selected");
            }
            let column = gtk::Box::new(gtk::Orientation::Vertical, 2);
            let top = gtk::Box::new(gtk::Orientation::Horizontal, 8);
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

    /// Draw the older-reply bar, or take it away.
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

    /// The newer messages, drawn read-only as `(who and when, first words)`
    /// lines; none puts them away.
    pub fn show_newer(&self, lines: &[(String, String)]) {
        while let Some(child) = self.newer.first_child() {
            self.newer.remove(&child);
        }
        self.newer_count.set(lines.len());
        for (heading, words) in lines {
            let block = gtk::Box::new(gtk::Orientation::Vertical, 2);
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

    /// The quoted messages' rows are expanded or collapsed again.
    pub fn set_quoted_expanded(&self, expanded: bool) {
        self.quoted_expanded.set(expanded);
    }

    /// Take everything away: a composition ended or a new one began.
    pub fn clear(&self) {
        self.show_older(None);
        self.show_newer(&[]);
        self.chooser.set_visible(false);
        self.chooser_view.replace(None);
        self.quoted_expanded.set(false);
    }

    /// Whether the chooser is up.
    pub fn chooser_open(&self) -> bool {
        self.chooser.is_visible()
    }

    /// The chosen card's title.
    pub fn chooser_selected(&self) -> Option<String> {
        let view = self.chooser_view.borrow();
        let view = view.as_ref()?;
        view.cards
            .iter()
            .find(|card| card.form == view.selected)
            .map(|card| card.title.clone())
    }

    /// The cards, "This message 1 | Whole thread 2 | As attachments 3".
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

    /// The bar's title, while it is up.
    pub fn older_title(&self) -> Option<String> {
        self.older
            .is_visible()
            .then(|| self.older_title.text().to_string())
    }

    /// The bar's buttons, " | " apart.
    pub fn older_actions(&self) -> String {
        self.older_actions.borrow().join(" | ")
    }

    /// How many newer messages are drawn.
    pub fn newer_count(&self) -> usize {
        self.newer_count.get()
    }

    /// Whether the quoted rows are expanded.
    pub fn quoted_expanded(&self) -> bool {
        self.quoted_expanded.get()
    }
}
