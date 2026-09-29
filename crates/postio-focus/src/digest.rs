//! The digest window (spec 007 US10, T137; screen 22's frame): a 980×820
//! dialog over the inbox for one delivery. Milestone 1 opens on its plain
//! list: one line a message, as the command bar lists a folder (screen 08).
//!
//! Its keys are the keymap's `Context::Digest`: `⇧A` archives all of it
//! as one undo, `d` edits its rule, `D` stops digesting the focused
//! message's sender once confirmed, `U` unsubscribes from its list, and
//! `Escape` closes. The rows answer the arrows and `Enter` themselves.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;
use postio_client::Client;
use postio_core::{CommandId, Keymap};
use postio_model::MessageId;
use postio_model::listing::MessageSummary;
use postio_ui::hints;
use postio_widgets::widgets::keyhint;
use postio_widgets::widgets::space::{S1, S2, S3};

use crate::list::Digest;

/// The dialog's name, for the window to find it by.
pub const DIALOG_NAME: &str = "focus-digest";

/// The dialog's size (contracts/focus-surface.md).
const WIDTH: i32 = 980;
const HEIGHT: i32 = 820;

/// What a person asked of the window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DigestAction {
    /// Archive all of it.
    ArchiveAll,
    /// Edit the rule and its cadence.
    EditRule,
    /// Open this message.
    Open {
        /// The message.
        message: MessageId,
        /// Its subject.
        subject: String,
    },
}

type Handler = Rc<dyn Fn(DigestAction)>;

/// The window. See the module.
pub struct DigestWindow {
    client: Client,
    dialog: adw::Dialog,
    title: gtk::Label,
    subtitle: gtk::Label,
    archive_words: gtk::Box,
    rule_line: gtk::Box,
    list: gtk::ListBox,
    keymap: RefCell<Keymap>,
    digest: RefCell<Option<Digest>>,
    /// When the rule delivers, as `config.toml` says it.
    rule_when: RefCell<Option<String>>,
    rows: RefCell<Vec<MessageSummary>>,
    generation: Cell<u64>,
    handler: RefCell<Option<Handler>>,
    me: RefCell<std::rc::Weak<DigestWindow>>,
}

impl DigestWindow {
    /// A closed window, reading through `client`, its keys from `keymap`.
    pub fn new(client: Client, keymap: &Keymap) -> Rc<Self> {
        let close = gtk::Button::new();
        close.add_css_class("flat");
        close.add_css_class("focus-digest-close");
        let close_words = gtk::Box::new(gtk::Orientation::Horizontal, S1);
        close_words.append(&gtk::Label::new(Some("Close")));
        close.set_child(Some(&close_words));
        let title = gtk::Label::new(None);
        title.add_css_class("focus-digest-title");
        let subtitle = gtk::Label::new(None);
        subtitle.add_css_class("dim-label");
        subtitle.add_css_class("focus-digest-subtitle");
        let titles = gtk::Box::new(gtk::Orientation::Vertical, 0);
        titles.append(&title);
        titles.append(&subtitle);
        let archive = gtk::Button::new();
        postio_widgets::widgets::button::style(
            &archive,
            postio_widgets::widgets::Kind::Primary,
            postio_widgets::widgets::Size::Regular,
        );
        archive.add_css_class("focus-digest-archive");
        let archive_words = gtk::Box::new(gtk::Orientation::Horizontal, S1);
        archive.set_child(Some(&archive_words));
        let header = gtk::CenterBox::new();
        header.add_css_class("focus-digest-header");
        header.set_start_widget(Some(&close));
        header.set_center_widget(Some(&titles));
        header.set_end_widget(Some(&archive));

        let edit = gtk::Button::new();
        edit.add_css_class("flat");
        edit.add_css_class("focus-digest-edit");
        let rule_line = gtk::Box::new(gtk::Orientation::Horizontal, S1);
        edit.set_child(Some(&rule_line));
        edit.set_halign(gtk::Align::End);
        let sub_row = gtk::Box::new(gtk::Orientation::Horizontal, S2);
        sub_row.add_css_class("focus-digest-sub-row");
        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        sub_row.append(&spacer);
        sub_row.append(&edit);

        let list = gtk::ListBox::new();
        list.add_css_class("focus-digest-list");
        list.set_selection_mode(gtk::SelectionMode::Single);
        let scrolled = gtk::ScrolledWindow::builder()
            .child(&list)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .build();

        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        content.add_css_class("focus-digest");
        content.append(&header);
        content.append(&sub_row);
        content.append(&scrolled);
        let dialog = adw::Dialog::builder()
            .content_width(WIDTH)
            .content_height(HEIGHT)
            .child(&content)
            .build();
        dialog.set_widget_name(DIALOG_NAME);

        let window = Rc::new(DigestWindow {
            client,
            dialog,
            title,
            subtitle,
            archive_words,
            rule_line,
            list,
            keymap: RefCell::new(keymap.clone()),
            digest: RefCell::default(),
            rule_when: RefCell::default(),
            rows: RefCell::default(),
            generation: Cell::new(0),
            handler: RefCell::default(),
            me: RefCell::default(),
        });
        window.me.replace(Rc::downgrade(&window));
        let weak = Rc::downgrade(&window);
        close.connect_clicked({
            let weak = weak.clone();
            move |_| {
                if let Some(window) = weak.upgrade() {
                    window.close();
                }
            }
        });
        if let Some(key) = hints::key(keymap, CommandId::Back) {
            close_words.append(&keyhint::cap(&key));
        }
        archive.connect_clicked({
            let weak = weak.clone();
            move |_| {
                if let Some(window) = weak.upgrade() {
                    window.emit(DigestAction::ArchiveAll);
                }
            }
        });
        edit.connect_clicked({
            let weak = weak.clone();
            move |_| {
                if let Some(window) = weak.upgrade() {
                    window.emit(DigestAction::EditRule);
                }
            }
        });
        window.list.connect_row_activated({
            let weak = weak.clone();
            move |_, row| {
                let Some(window) = weak.upgrade() else {
                    return;
                };
                let found = usize::try_from(row.index())
                    .ok()
                    .and_then(|index| window.rows.borrow().get(index).cloned());
                if let Some(found) = found {
                    window.emit(DigestAction::Open {
                        message: found.id,
                        subject: found.subject.unwrap_or_default(),
                    });
                }
            }
        });
        window
    }

    /// The commands the window has a control for: Close, Archive all, and
    /// the rule's edit.
    pub fn controls() -> Vec<CommandId> {
        vec![
            CommandId::Back,
            CommandId::ArchiveThread,
            CommandId::DigestRule,
        ]
    }

    /// Run `handler` with what the person asks.
    pub fn connect_action(&self, handler: impl Fn(DigestAction) + 'static) {
        self.handler.replace(Some(Rc::new(handler)));
    }

    /// Read every key from `keymap`.
    pub fn set_keymap(&self, keymap: &Keymap) {
        self.keymap.replace(keymap.clone());
        self.show_header();
    }

    /// Open over `parent` on `digest`, whose rule delivers as `rule_when`
    /// says, and read what it holds.
    pub fn show(&self, parent: &impl IsA<gtk::Widget>, digest: Digest, rule_when: Option<String>) {
        self.digest.replace(Some(digest));
        self.rule_when.replace(rule_when);
        self.rows.borrow_mut().clear();
        self.show_rows();
        self.show_header();
        self.dialog.present(Some(parent));
        self.read();
    }

    /// Close the window.
    pub fn close(&self) {
        self.dialog.close();
    }

    /// The dialog, for the window's keys.
    pub fn dialog(&self) -> &adw::Dialog {
        &self.dialog
    }

    /// The digest on screen.
    pub fn digest(&self) -> Option<Digest> {
        self.digest.borrow().clone()
    }

    /// The focused message, if any: what `D` and `U` act on.
    pub fn focused(&self) -> Option<MessageSummary> {
        let row = self.list.selected_row()?;
        let index = usize::try_from(row.index()).ok()?;
        self.rows.borrow().get(index).cloned()
    }

    /// Move the focus `by` rows: `j` and `k`.
    pub fn step(&self, by: i32) {
        let at = self.list.selected_row().map_or(-1, |row| row.index());
        if let Some(row) = self.list.row_at_index((at + by).max(0)) {
            self.list.select_row(Some(&row));
            row.grab_focus();
        }
    }

    /// Open the focused message: `Enter`.
    pub fn open_focused(&self) {
        if let Some(row) = self.list.selected_row() {
            row.activate();
        }
    }

    /// The listed messages' subjects, top to bottom.
    pub fn subjects(&self) -> Vec<String> {
        self.rows
            .borrow()
            .iter()
            .map(|row| row.subject.clone().unwrap_or_default())
            .collect()
    }

    /// Every text the window shows, in order: what a test reads.
    pub fn texts(&self) -> Vec<String> {
        let mut said = Vec::new();
        let mut stack: Vec<gtk::Widget> = self.dialog.child().into_iter().collect();
        while let Some(widget) = stack.pop() {
            if !widget.is_visible() {
                continue;
            }
            if let Some(label) = widget.downcast_ref::<gtk::Label>()
                && !label.text().is_empty()
            {
                said.push(label.text().to_string());
            }
            let mut child = widget.last_child();
            while let Some(previous) = child {
                child = previous.prev_sibling();
                stack.push(previous);
            }
        }
        said
    }

    fn emit(&self, action: DigestAction) {
        let handler = self.handler.borrow().clone();
        if let Some(handler) = handler {
            handler(action);
        }
    }

    /// The title, its line, Archive all and the rule's line.
    fn show_header(&self) {
        let Some(digest) = self.digest.borrow().clone() else {
            return;
        };
        let keymap = self.keymap.borrow().clone();
        let title = match postio_ui::focus_row::digest_title(digest.cadence).split_once(" \u{b7} ")
        {
            Some((cadence, _)) => format!("{cadence} \u{b7} {}", digest.rule),
            None => digest.rule.clone(),
        };
        self.title.set_text(&title);
        let now = chrono::Local::now();
        self.subtitle.set_text(&postio_ui::digest::window_subtitle(
            digest.count,
            digest.senders.len(),
            digest.at.with_timezone(&chrono::Local),
            now,
        ));
        while let Some(child) = self.archive_words.first_child() {
            self.archive_words.remove(&child);
        }
        self.archive_words.append(&gtk::Label::new(Some(&format!(
            "Archive all {}",
            digest.count
        ))));
        if let Some(key) = hints::key(&keymap, CommandId::ArchiveThread) {
            self.archive_words.append(&keyhint::cap(&key));
        }
        while let Some(child) = self.rule_line.first_child() {
            self.rule_line.remove(&child);
        }
        let mut said = String::new();
        if let Some(when) = self.rule_when.borrow().as_deref() {
            said.push_str(when);
            said.push_str(" \u{b7} ");
        }
        said.push_str("Edit rule and cadence");
        self.rule_line.append(&gtk::Label::new(Some(&said)));
        if let Some(key) = hints::key(&keymap, CommandId::DigestRule) {
            self.rule_line.append(&keyhint::cap(&key));
        }
    }

    /// Read what the delivery holds.
    fn read(&self) {
        let Some(digest) = self.digest.borrow().clone() else {
            return;
        };
        let generation = self.generation.get() + 1;
        self.generation.set(generation);
        let client = self.client.clone();
        let weak = self.me.borrow().clone();
        glib::spawn_future_local(async move {
            // POSTIO-GLIB-SAFE: a client call is a oneshot receive; the host
            // answers on its own runtime (ADR 0041).
            let read = client.delivery_messages(digest.delivery).await;
            let Some(window) = weak.upgrade() else {
                return;
            };
            if window.generation.get() != generation {
                return;
            }
            match read {
                Ok(rows) => {
                    window.rows.replace(rows);
                    window.show_rows();
                    if let Some(first) = window.list.row_at_index(0) {
                        window.list.select_row(Some(&first));
                        first.grab_focus();
                    }
                }
                Err(error) => tracing::warn!(%error, "Focus could not read a digest: {error}"),
            }
        });
    }

    /// One line a message: the sender, the subject and its first line,
    /// and the time -- as the bar lists a folder (screen 08).
    fn show_rows(&self) {
        while let Some(child) = self.list.first_child() {
            self.list.remove(&child);
        }
        let now = chrono::Local::now();
        for row in self.rows.borrow().iter() {
            let line = gtk::Box::new(gtk::Orientation::Horizontal, S3);
            line.add_css_class("focus-digest-row");
            let sender = gtk::Label::new(Some(
                &row.from
                    .as_ref()
                    .map(|from| from.display().to_owned())
                    .unwrap_or_default(),
            ));
            sender.set_xalign(0.0);
            sender.set_width_chars(20);
            sender.set_max_width_chars(20);
            sender.set_ellipsize(gtk::pango::EllipsizeMode::End);
            line.append(&sender);
            let subject = gtk::Label::new(row.subject.as_deref());
            subject.add_css_class("focus-digest-subject");
            subject.set_xalign(0.0);
            subject.set_ellipsize(gtk::pango::EllipsizeMode::End);
            line.append(&subject);
            let preview = gtk::Label::new(row.preview.as_deref());
            preview.add_css_class("dim-label");
            preview.set_xalign(0.0);
            preview.set_hexpand(true);
            preview.set_ellipsize(gtk::pango::EllipsizeMode::End);
            line.append(&preview);
            let time = gtk::Label::new(Some(&postio_ui::row::timestamp(row.received_at, now)));
            line.append(&time);
            let item = gtk::ListBoxRow::new();
            item.set_child(Some(&line));
            self.list.append(&item);
        }
    }
}
