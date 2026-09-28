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
use postio_model::{Mailbox, MailboxId, MailboxRole};
use postio_ui::pickers;
use postio_widgets::widgets::pickers::{Field, Picked, Picker, Row};

/// How many of Recent get a number key and a row.
const RECENT: usize = 2;

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
                folders.extend(read.unwrap_or_default().into_iter().filter(destination));
            }
            folders.sort_by_key(|folder| {
                (
                    folder.role != MailboxRole::Archive,
                    folder.name.to_lowercase(),
                )
            });
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
        let folders = self.folders.borrow();
        let named = |id: &MailboxId| folders.iter().find(|folder| folder.id == *id);
        let keeps = |folder: &Mailbox| {
            !pickers::filtered([crate::places::place_name(folder).as_str()], &filter).is_empty()
        };
        let mut rows = Vec::new();
        let mut shown = Vec::new();
        let recent: Vec<&Mailbox> = self
            .recent
            .borrow()
            .iter()
            .filter_map(named)
            .filter(|folder| keeps(folder))
            .take(RECENT)
            .collect();
        for (index, folder) in recent.iter().enumerate() {
            rows.push(Row {
                section: (index == 0).then(|| "Recent".to_owned()),
                name: crate::places::place_name(folder),
                detail: folder.counts.total.to_string(),
                numbered: true,
                ..Row::default()
            });
            shown.push(folder.id);
        }
        let mut first = true;
        for folder in folders.iter().filter(|folder| keeps(folder)) {
            rows.push(Row {
                section: first.then(|| "All folders".to_owned()),
                name: crate::places::place_name(folder),
                detail: folder.counts.total.to_string(),
                ..Row::default()
            });
            shown.push(folder.id);
            first = false;
        }
        drop(folders);
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

/// Whether mail can be moved to `folder` from the picker: the person's own
/// folders and the archive. Sending, drafting, snoozing and deleting have
/// verbs of their own, and the inbox is where the mail already is.
fn destination(folder: &Mailbox) -> bool {
    matches!(folder.role, MailboxRole::Regular | MailboxRole::Archive)
}
