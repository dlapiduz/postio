//! The move picker: `v` lists recent destinations (`1`, `2`) then every folder with its count;
//! `Enter` or a number moves the mail, `Ctrl+Z` brings it back.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;
use postio_client::Client;
use postio_core::{Command, Keymap, MessageTarget};
use postio_model::listing::MailStore as _;
use postio_model::{Mailbox, MailboxId};
use postio_ui::pickers;
use postio_widgets::widgets::pickers::{Field, Picked, Picker, Row};

type Sender = Rc<dyn Fn(Command)>;

pub struct MovePicker {
    picker: Rc<Picker>,
    client: Client,
    folders: RefCell<Vec<Mailbox>>,
    recent: RefCell<Vec<MailboxId>>,
    shown: RefCell<Vec<MailboxId>>,
    sender: RefCell<Option<Sender>>,
    me: RefCell<std::rc::Weak<MovePicker>>,
}

impl MovePicker {
    pub fn new(client: Client, keymap: &Keymap) -> Rc<Self> {
        let picker = Picker::new(
            keymap,
            pickers::MOVE_TITLE,
            Field::Filter(pickers::MOVE_FILTER),
            &pickers::move_footnote(keymap),
        );
        let this = Rc::new(MovePicker {
            picker,
            client,
            folders: RefCell::default(),
            recent: RefCell::default(),
            shown: RefCell::default(),
            sender: RefCell::default(),
            me: RefCell::default(),
        });
        this.me.replace(Rc::downgrade(&this));
        let weak = Rc::downgrade(&this);
        this.picker.connect_picked({
            let weak = weak.clone();
            move |picked| {
                if let Some(this) = weak.upgrade() {
                    this.picked(picked);
                }
            }
        });
        this.picker.connect_field_changed(move |_| {
            if let Some(this) = weak.upgrade() {
                this.show();
            }
        });
        this
    }

    pub fn picker(&self) -> &Rc<Picker> {
        &self.picker
    }

    pub fn set_keymap(&self, keymap: &Keymap) {
        self.picker.set_keymap(keymap);
        self.picker.set_footnote(&pickers::move_footnote(keymap));
    }

    pub fn open(
        &self,
        parent: &impl IsA<gtk::Widget>,
        rect: Option<&gtk::gdk::Rectangle>,
        target: &str,
        send: impl Fn(Command) + 'static,
    ) {
        self.sender.replace(Some(Rc::new(send)));
        self.picker.set_target(target);
        self.picker.set_rows(Vec::new());
        self.picker.open(parent, rect);
        self.read();
    }

    fn read(&self) {
        let client = self.client.clone();
        let weak = self.me.borrow().clone();
        glib::spawn_future_local(async move {
            // POSTIO-GLIB-SAFE: a client call is a oneshot receive; the host
            // answers on its own runtime (ADR 0041).
            let accounts = client.accounts().await.unwrap_or_default();
            let mut folders = Vec::new();
            for account in accounts.iter().filter(|account| account.enabled) {
                // POSTIO-GLIB-SAFE: as above.
                let read = client.mailboxes(account.id).await;
                folders.extend(
                    read.unwrap_or_default()
                        .into_iter()
                        .filter(pickers::is_destination),
                );
            }
            pickers::order_destinations(&mut folders);
            // POSTIO-GLIB-SAFE: as above.
            let recent = client.move_recent().await.unwrap_or_default();
            let Some(this) = weak.upgrade() else {
                return;
            };
            this.folders.replace(folders);
            this.recent.replace(recent);
            this.show();
        });
    }

    fn show(&self) {
        let filter = self.picker.entry().text().to_string();
        let rows = pickers::move_rows(&self.folders.borrow(), &self.recent.borrow(), &filter);
        let shown = rows.iter().map(|row| row.folder).collect();
        let rows = rows
            .into_iter()
            .map(|row| Row {
                section: row.section.map(str::to_owned),
                name: row.name,
                detail: row.count,
                numbered: row.numbered,
                ..Row::default()
            })
            .collect();
        self.shown.replace(shown);
        self.picker.set_rows(rows);
    }

    fn picked(&self, picked: Picked) {
        let index = match picked {
            Picked::Choose(index) => Some(index),
            Picked::Confirm { selected, .. } => selected,
            Picked::Toggle(_) => None,
        };
        let Some(to) = index.and_then(|index| self.shown.borrow().get(index).copied()) else {
            return;
        };
        let Some(send) = self.sender.borrow().clone() else {
            return;
        };
        self.picker.close();
        send(Command::Move {
            target: MessageTarget::Selection,
            to: Some(to),
        });
        let client = self.client.clone();
        glib::spawn_future_local(async move {
            // POSTIO-GLIB-SAFE: as `read`'s.
            if let Err(error) = client.note_move(to).await {
                tracing::warn!(%error, "Focus could not keep a recent move");
            }
        });
    }
}
