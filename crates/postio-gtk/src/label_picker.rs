//! The label picker: `l` lists the account's labels; `Space` puts a label on or off and keeps the
//! picker open; a name nobody has
//! is offered as "Create label". Every change is one `add_label`, so `Ctrl+Z` takes each back.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;
use postio_client::Client;
use postio_core::{Command, Keymap, MessageTarget};
use postio_model::{AccountId, Label, LabelId, ThreadId};
use postio_ui::pickers;
use postio_widgets::widgets::pickers::{Field, Mark, Picked, Picker, Row};

use postio_ui::pickers::LabelRow as Entry;

#[derive(Debug, Clone)]
pub struct LabelAim {
    pub account: AccountId,
    pub message: Option<postio_model::MessageId>,
    pub threads: Vec<ThreadId>,
}

type Sender = Rc<dyn Fn(Command)>;

pub struct LabelPicker {
    picker: Rc<Picker>,
    client: Client,
    account: RefCell<Option<AccountId>>,
    labels: RefCell<Vec<Label>>,
    counts: RefCell<HashMap<LabelId, u32>>,
    applied: RefCell<HashSet<LabelId>>,
    shown: RefCell<Vec<Entry>>,
    sender: RefCell<Option<Sender>>,
    me: RefCell<std::rc::Weak<LabelPicker>>,
}

impl LabelPicker {
    pub fn new(client: Client, keymap: &Keymap) -> Rc<Self> {
        let picker = Picker::new(
            keymap,
            pickers::LABEL_TITLE,
            Field::Filter(pickers::LABEL_FILTER),
            &pickers::label_footnote(keymap),
        );
        let this = Rc::new(LabelPicker {
            picker,
            client,
            account: RefCell::default(),
            labels: RefCell::default(),
            counts: RefCell::default(),
            applied: RefCell::default(),
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
        self.picker.set_footnote(&pickers::label_footnote(keymap));
    }

    pub fn open(
        &self,
        parent: &impl gtk::prelude::IsA<gtk::Widget>,
        rect: Option<&gtk::gdk::Rectangle>,
        target: &str,
        aim: LabelAim,
        send: impl Fn(Command) + 'static,
    ) {
        let LabelAim {
            account,
            message,
            threads,
        } = aim;
        self.account.replace(Some(account));
        self.sender.replace(Some(Rc::new(send)));
        self.picker.set_target(target);
        self.picker.set_rows(Vec::new());
        self.picker.open(parent, rect);
        self.read(account, message, threads);
    }

    fn read(
        &self,
        account: AccountId,
        message: Option<postio_model::MessageId>,
        threads: Vec<ThreadId>,
    ) {
        let client = self.client.clone();
        let weak = self.me.borrow().clone();
        glib::spawn_future_local(async move {
            // A label is an account's, and a message can carry only its account's.
            let account = match message {
                Some(message) => {
                    // POSTIO-GLIB-SAFE: a client call is a oneshot receive.
                    let found = client.account_of(message).await;
                    found.ok().flatten().unwrap_or(account)
                }
                None => account,
            };
            if let Some(this) = weak.upgrade() {
                this.account.replace(Some(account));
            }
            // POSTIO-GLIB-SAFE: a client call is a oneshot receive; the host
            // answers on its own runtime (ADR 0041).
            let labels = client.labels(account).await.unwrap_or_default();
            // POSTIO-GLIB-SAFE: as above.
            let counts = client.label_counts(account).await.unwrap_or_default();
            let carried = if threads.is_empty() {
                Vec::new()
            } else {
                let asked = threads.clone();
                // POSTIO-GLIB-SAFE: as above.
                let read = client.thread_labels(asked).await;
                read.unwrap_or_default()
            };
            let Some(this) = weak.upgrade() else {
                return;
            };
            let applied = pickers::applied_labels(carried, &threads);
            this.labels.replace(labels);
            this.counts.replace(counts.into_iter().collect());
            this.applied.replace(applied);
            this.show();
        });
    }

    fn show(&self) {
        let filter = self.picker.entry().text().to_string();
        let labels = self.labels.borrow();
        let shown = pickers::label_rows(&labels, &filter);
        let applied = self.applied.borrow();
        let counts = self.counts.borrow();
        let rows = shown
            .iter()
            .map(|entry| match entry {
                Entry::Create(name) => Row {
                    name: pickers::create_label(name),
                    ..Row::default()
                },
                Entry::Label(label) => Row {
                    mark: Some(Mark::Dot(crate::places::label_rgba(label))),
                    name: label.name.clone(),
                    detail: pickers::label_detail(
                        applied.contains(&label.id),
                        counts.get(&label.id).copied().unwrap_or(0),
                    ),
                    ..Row::default()
                },
            })
            .collect();
        drop((applied, counts));
        let selected = self.picker.selected();
        self.shown.replace(shown);
        self.picker.set_rows(rows);
        if let Some(selected) = selected {
            self.picker.select(selected);
        }
    }

    fn picked(&self, picked: Picked) {
        match picked {
            Picked::Toggle(index) | Picked::Choose(index) => self.toggle(index, false),
            Picked::Confirm { selected, .. } => match selected {
                Some(index) if matches!(self.shown.borrow().get(index), Some(Entry::Create(_))) => {
                    self.toggle(index, true)
                }
                _ => self.picker.close(),
            },
        }
    }

    fn toggle(&self, index: usize, then_close: bool) {
        let Some(entry) = self.shown.borrow().get(index).cloned() else {
            return;
        };
        let Some(send) = self.sender.borrow().clone() else {
            return;
        };
        match entry {
            Entry::Label(label) => {
                let on = !self.applied.borrow().contains(&label.id);
                send(Command::AddLabel {
                    target: MessageTarget::Selection,
                    label: Some(label.id),
                    on: Some(on),
                });
                {
                    let mut applied = self.applied.borrow_mut();
                    if on {
                        applied.insert(label.id);
                    } else {
                        applied.remove(&label.id);
                    }
                }
                if then_close {
                    self.picker.close();
                } else {
                    self.show();
                }
            }
            Entry::Create(name) => {
                let Some(account) = *self.account.borrow() else {
                    return;
                };
                let client = self.client.clone();
                let weak = self.me.borrow().clone();
                glib::spawn_future_local(async move {
                    // POSTIO-GLIB-SAFE: as `read`'s.
                    let made = client.create_label(account, name).await;
                    let Ok(Some(label)) = made else {
                        tracing::warn!("Focus could not make a label");
                        return;
                    };
                    let Some(this) = weak.upgrade() else {
                        return;
                    };
                    send(Command::AddLabel {
                        target: MessageTarget::Selection,
                        label: Some(label.id),
                        on: Some(true),
                    });
                    this.applied.borrow_mut().insert(label.id);
                    let labels = std::mem::take(&mut *this.labels.borrow_mut());
                    this.labels.replace(pickers::with_label(labels, label));
                    if then_close {
                        this.picker.close();
                    } else {
                        this.picker.entry().set_text("");
                        this.show();
                    }
                });
            }
        }
    }
}
