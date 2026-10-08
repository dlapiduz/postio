//! The move picker (screen 14; US5 scenario 6): `m` lists where mail can
//! go -- the last few destinations under "Recent", with `1` and `2`, then
//! every folder under "All folders", each with its count. Typing filters;
//! `Enter` or a number moves the mail, which leaves the inbox, and `Ctrl+Z`
//! brings it back.

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

/// What moving sends: the command, aimed at what the picker opened on.
type Sender = Rc<dyn Fn(Command)>;

/// The picker. See the module.
pub struct MovePicker {
    picker: Rc<Picker>,
    client: Client,
    folders: RefCell<Vec<Mailbox>>,
    recent: RefCell<Vec<MailboxId>>,
    /// The folder each row moves to, in the rows' order.
    shown: RefCell<Vec<MailboxId>>,
    sender: RefCell<Option<Sender>>,
    me: RefCell<std::rc::Weak<MovePicker>>,
}

impl MovePicker {
    /// A closed picker, reading through `client`, its keys from `keymap`.
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

    /// The frame, for keys and tests.
    pub fn picker(&self) -> &Rc<Picker> {
        &self.picker
    }

    /// Read every key from `keymap`.
    pub fn set_keymap(&self, keymap: &Keymap) {
        self.picker.set_keymap(keymap);
        self.picker.set_footnote(&pickers::move_footnote(keymap));
    }

    /// Open from `parent` at `rect`, naming `target`; `send` moves.
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

    /// Read every enabled account's folders, and Recent.
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

    /// Recent, then every folder, as the filter keeps them.
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
