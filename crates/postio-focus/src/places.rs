//! The folders popover (screen 10; contracts/focus-surface.md, "The folders
//! popover"): `g o`, or a click on "Inbox ▾", lists the places to go --
//! mailboxes with their direct keys, the person's folders, and labels --
//! with their counts. Typing filters them, and `Enter` goes to the first.
//!
//! Focus has no folder sidebar: this, and `in:` in the command bar, are how
//! a person gets anywhere but the inbox.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;
use postio_client::Client;
use postio_core::{CommandId, Keymap};
use postio_model::MailboxRole;
use postio_model::listing::MailStore as _;
use postio_ui::finder::Destination;
use postio_widgets::widgets::keyhint;
use postio_widgets::widgets::space::{S1, S2};

/// The popover's width.
const WIDTH: i32 = 400;

/// Which section a place is listed under.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Section {
    Mailboxes,
    Folders,
    Labels,
}

impl Section {
    fn title(self) -> &'static str {
        match self {
            Section::Mailboxes => "Mailboxes",
            Section::Folders => "Folders",
            Section::Labels => "Labels",
        }
    }
}

/// One place the popover lists.
#[derive(Debug, Clone)]
struct Entry {
    section: Section,
    /// Where it sorts within its section: a mailbox's role order.
    rank: usize,
    name: String,
    count: Option<u32>,
    go: Option<CommandId>,
    destination: Destination,
}

/// What going somewhere asks the window to do: the place, and its name.
type Handler = Rc<dyn Fn(Destination, String)>;

/// The popover.
pub struct Places {
    client: Client,
    popover: gtk::Popover,
    entry: gtk::SearchEntry,
    list: gtk::ListBox,
    /// Every place, as last read.
    all: Rc<RefCell<Vec<Entry>>>,
    /// The places listed now, in order: what each row goes to.
    shown: RefCell<Vec<Entry>>,
    keymap: RefCell<Keymap>,
    handler: RefCell<Option<Handler>>,
    me: RefCell<std::rc::Weak<Places>>,
}

impl Places {
    /// A closed popover, anchored to `anchor`, reading through `client`.
    pub fn new(client: Client, keymap: &Keymap, anchor: &impl IsA<gtk::Widget>) -> Rc<Self> {
        let entry = gtk::SearchEntry::new();
        entry.set_placeholder_text(Some("Go to folder or label"));
        let list = gtk::ListBox::new();
        list.add_css_class("focus-places-list");
        list.set_selection_mode(gtk::SelectionMode::Single);
        let scrolled = gtk::ScrolledWindow::builder()
            .child(&list)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .propagate_natural_height(true)
            .max_content_height(420)
            .build();
        let footer = gtk::Label::new(Some(
            "\u{21b5} open \u{b7} Esc close \u{b7} same as in:Receipts in the command bar",
        ));
        footer.add_css_class("dim-label");
        footer.add_css_class("focus-places-footer");
        footer.set_xalign(0.0);
        let content = gtk::Box::new(gtk::Orientation::Vertical, S2);
        content.add_css_class("focus-places");
        content.set_width_request(WIDTH);
        content.append(&entry);
        content.append(&scrolled);
        content.append(&footer);
        let popover = gtk::Popover::builder()
            .child(&content)
            .has_arrow(false)
            .position(gtk::PositionType::Bottom)
            .build();
        popover.set_parent(anchor);
        // The popover goes with the button it hangs from, before GTK would
        // finalize the button with a child still attached.
        anchor.as_ref().connect_destroy({
            let popover = popover.downgrade();
            move |_| {
                if let Some(popover) = popover.upgrade() {
                    popover.unparent();
                }
            }
        });

        let places = Rc::new(Places {
            client,
            popover,
            entry,
            list,
            all: Rc::default(),
            shown: RefCell::default(),
            keymap: RefCell::new(keymap.clone()),
            handler: RefCell::default(),
            me: RefCell::default(),
        });
        places.me.replace(Rc::downgrade(&places));
        let weak = Rc::downgrade(&places);
        places.entry.connect_search_changed({
            let weak = weak.clone();
            move |_| {
                if let Some(places) = weak.upgrade() {
                    places.show();
                }
            }
        });
        places.entry.connect_activate({
            let weak = weak.clone();
            move |_| {
                if let Some(places) = weak.upgrade() {
                    places.activate();
                }
            }
        });
        places.list.connect_row_activated({
            let weak = weak.clone();
            move |_, row| {
                if let (Some(places), Ok(index)) = (weak.upgrade(), usize::try_from(row.index())) {
                    places.go(index);
                }
            }
        });
        places
    }

    /// Run `handler` with the place a person chooses.
    pub fn connect_go(&self, handler: impl Fn(Destination, String) + 'static) {
        self.handler.replace(Some(Rc::new(handler)));
    }

    /// Read every key the popover shows from `keymap`.
    pub fn set_keymap(&self, keymap: &Keymap) {
        self.keymap.replace(keymap.clone());
        self.show();
    }

    /// Open the popover, unfiltered, and read the places.
    pub fn open(&self) {
        self.entry.set_text("");
        self.show();
        self.popover.popup();
        self.entry.grab_focus();
        self.read();
    }

    /// Close the popover without going anywhere.
    pub fn close(&self) {
        self.popover.popdown();
    }

    /// Whether the popover is up.
    pub fn is_open(&self) -> bool {
        self.popover.is_visible()
    }

    /// Filter the places by `text`, as typing does.
    pub fn set_filter(&self, text: &str) {
        self.entry.set_text(text);
        self.show();
    }

    /// The names listed now, top to bottom.
    pub fn names(&self) -> Vec<String> {
        self.shown
            .borrow()
            .iter()
            .map(|entry| entry.name.clone())
            .collect()
    }

    /// Go to the chosen place, or the first listed: what `Enter` does.
    pub fn activate(&self) {
        let selected = self
            .list
            .selected_row()
            .and_then(|row| usize::try_from(row.index()).ok());
        self.go(selected.unwrap_or(0));
    }

    /// Go to the `index`th place listed.
    fn go(&self, index: usize) {
        let Some(entry) = self.shown.borrow().get(index).cloned() else {
            return;
        };
        self.popover.popdown();
        let handler = self.handler.borrow().clone();
        if let Some(handler) = handler {
            handler(entry.destination, entry.name);
        }
    }

    /// List the places the filter matches, in their sections.
    fn show(&self) {
        while let Some(child) = self.list.first_child() {
            self.list.remove(&child);
        }
        let wanted = self.entry.text().to_lowercase();
        let mut shown: Vec<Entry> = self
            .all
            .borrow()
            .iter()
            .filter(|entry| wanted.is_empty() || entry.name.to_lowercase().contains(&wanted))
            .cloned()
            .collect();
        shown.sort_by(|a, b| {
            (a.section, a.rank, a.name.to_lowercase()).cmp(&(
                b.section,
                b.rank,
                b.name.to_lowercase(),
            ))
        });
        let keymap = self.keymap.borrow();
        let mut section = None;
        for entry in &shown {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, S2);
            row.add_css_class("focus-places-row");
            let name = gtk::Label::new(Some(&entry.name));
            name.set_xalign(0.0);
            name.set_hexpand(true);
            row.append(&name);
            if let Some(count) = entry.count {
                let count = gtk::Label::new(Some(&count.to_string()));
                count.add_css_class("dim-label");
                row.append(&count);
            }
            if let Some(key) = entry
                .go
                .and_then(|command| postio_ui::hints::key(&keymap, command))
            {
                row.append(&keyhint::cap(&key));
            }
            let item = gtk::ListBoxRow::new();
            if section == Some(entry.section) {
                item.set_child(Some(&row));
            } else {
                // A section's heading rides on its first row, so every row of
                // the list is a place and its index is its place in `shown`.
                section = Some(entry.section);
                let heading = gtk::Label::new(Some(entry.section.title()));
                heading.add_css_class("focus-places-section");
                heading.set_xalign(0.0);
                let holder = gtk::Box::new(gtk::Orientation::Vertical, S1);
                holder.append(&heading);
                holder.append(&row);
                item.set_child(Some(&holder));
            }
            self.list.append(&item);
        }
        if let Some(first) = self.list.row_at_index(0) {
            self.list.select_row(Some(&first));
        }
        self.shown.replace(shown);
    }

    /// Read every account's mailboxes and labels.
    fn read(&self) {
        let client = self.client.clone();
        let all = Rc::clone(&self.all);
        let weak = self.me.borrow().clone();
        glib::spawn_future_local(async move {
            // POSTIO-GLIB-SAFE: a client call is a oneshot receive; the host
            // answers on its own runtime (ADR 0041).
            let Ok(accounts) = client.accounts().await else {
                return;
            };
            let mut found = Vec::new();
            for account in accounts.iter().filter(|account| account.enabled) {
                // POSTIO-GLIB-SAFE: as above.
                let read = client.mailboxes(account.id).await;
                for mailbox in read.unwrap_or_default() {
                    let (section, rank) = match mailbox.role {
                        MailboxRole::Regular => (Section::Folders, 0),
                        role => (Section::Mailboxes, role_rank(role)),
                    };
                    found.push(Entry {
                        section,
                        rank,
                        name: place_name(&mailbox),
                        count: Some(mailbox.counts.total),
                        go: go_to(mailbox.role),
                        destination: Destination::Mailbox(mailbox.id),
                    });
                }
                // POSTIO-GLIB-SAFE: as above.
                let read = client.labels(account.id).await;
                for label in read.unwrap_or_default() {
                    found.push(Entry {
                        section: Section::Labels,
                        rank: 0,
                        name: label.name.clone(),
                        count: None,
                        go: None,
                        destination: Destination::Label(label.id),
                    });
                }
            }
            all.replace(found);
            if let Some(places) = weak.upgrade() {
                places.show();
            }
        });
    }
}

/// Where a mailbox of `role` sorts: the order screen 10 lists them in.
fn role_rank(role: MailboxRole) -> usize {
    match role {
        MailboxRole::Inbox => 0,
        MailboxRole::Drafts => 1,
        MailboxRole::Sent => 2,
        MailboxRole::Snoozed => 3,
        MailboxRole::Archive => 4,
        _ => 5,
    }
}

/// The command that goes to a mailbox of `role` directly.
pub fn go_to(role: MailboxRole) -> Option<CommandId> {
    Some(match role {
        MailboxRole::Inbox => CommandId::GoToInbox,
        MailboxRole::Drafts => CommandId::GoToDrafts,
        MailboxRole::Sent => CommandId::GoToSent,
        MailboxRole::Archive => CommandId::GoToArchive,
        MailboxRole::Snoozed => CommandId::GoToSnoozed,
        MailboxRole::Flagged => CommandId::GoToFlagged,
        _ => return None,
    })
}

/// What a mailbox is called here: "Inbox" for an inbox, whatever the
/// server names it ("INBOX"), and its own name otherwise.
pub fn place_name(mailbox: &postio_model::Mailbox) -> String {
    match mailbox.role {
        MailboxRole::Inbox => "Inbox".to_owned(),
        _ => mailbox.name.clone(),
    }
}
