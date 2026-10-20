//! The Contacts view: a mode of the main window, like search results
//! (specs/011-focus-contacts S7, handoff screens 01 to 03).
//!
//! The strip with its kind tabs and filter, the list in letter groups, and
//! the card beside it are the controller's: which tab, what is typed, where
//! the cursor is, which pages are resident, and which card is read. What each
//! says is `postio_ui::contacts_view`'s. The frontend draws the frame, asks
//! for the rows it can see ([`FocusController::contacts_row`]), and reports a
//! click on a tab, a row or the filter.
//!
//! One read answers each thing, stamped, and an answer for a list or a card
//! since replaced changes nothing: a tab changed mid-scroll, a cursor moved
//! faster than a card reads.

use std::collections::HashMap;

use postio_core::CommandId;
use postio_model::{
    ContactCardRead, ContactId, ContactListRow, ContactTab, ContactsOverview, ContactsPageRead,
};
use postio_ui::contacts::{ContactsWindow, Delivered, Slot};
use postio_ui::contacts_view::{self as words, CardView, FooterView, RowView, StripView};
use postio_ui::list::PAGE_SIZE;

use crate::cursor::Rows;
use crate::feed::Step;
use crate::{Effect, FocusController, Intent, Reply, Request, SurfaceKind};

/// How many people a typed filter lists at most.
pub const FILTER_CAP: u32 = 200;

/// The Contacts view's frame, drawn whole: everything but the rows and the
/// card, which come as they are read.
#[derive(Debug, Clone, PartialEq)]
pub struct ContactsView {
    /// The strip: tabs, filter, New contact, the review line.
    pub strip: StripView,
    /// The footer.
    pub footer: FooterView,
    /// How many rows the list has.
    pub total: u32,
    /// What is typed in the filter.
    pub filter: String,
    /// Whether the counts have been read: before, the frame is a skeleton.
    pub loaded: bool,
}

/// What the controller keeps of the Contacts view.
#[derive(Debug)]
pub(crate) struct Contacts {
    tab: ContactTab,
    filter: String,
    overview: ContactsOverview,
    loaded: bool,
    window: ContactsWindow,
    /// Each resident page's row before it: what says whether its first row
    /// starts a letter.
    befores: HashMap<u32, Option<String>>,
    /// The people the filter found; the list while it is typed.
    filtered: Option<Vec<ContactListRow>>,
    filter_stamp: u64,
    cursor: Option<u32>,
    card: Option<ContactCardRead>,
    card_for: Option<ContactId>,
    card_stamp: u64,
}

impl Contacts {
    fn new() -> Self {
        Contacts {
            tab: ContactTab::All,
            filter: String::new(),
            overview: ContactsOverview::default(),
            loaded: false,
            window: ContactsWindow::default(),
            befores: HashMap::new(),
            filtered: None,
            filter_stamp: 0,
            cursor: None,
            card: None,
            card_for: None,
            card_stamp: 0,
        }
    }

    /// How many rows the list has.
    fn total(&self) -> u32 {
        match &self.filtered {
            Some(rows) => rows.len() as u32,
            None => self.window.total(),
        }
    }

    /// The row at `index`, if it is here.
    fn row(&self, index: u32) -> Option<&ContactListRow> {
        match &self.filtered {
            Some(rows) => rows.get(index as usize),
            None => self.window.peek(index),
        }
    }

    /// The sort key of the row before `index`, `None` for the first row.
    fn before(&self, index: u32) -> Option<String> {
        if index == 0 {
            return None;
        }
        if let Some(row) = self.row(index - 1) {
            return Some(row.sort_key.clone());
        }
        // The first row of a page whose neighbour is not resident: the page
        // said what came before it.
        (index % PAGE_SIZE == 0)
            .then(|| self.befores.get(&(index / PAGE_SIZE)).cloned().flatten())
            .flatten()
    }

    /// The tab's count, from the overview.
    fn tab_total(&self) -> u32 {
        self.overview
            .tabs
            .iter()
            .find(|(tab, _)| *tab == self.tab)
            .map_or(0, |(_, count)| *count)
    }

    /// The person under the cursor.
    fn person(&self) -> Option<&ContactListRow> {
        self.row(self.cursor?)
    }
}

impl FocusController {
    /// `g c`: the Contacts view, in the list's place. Nothing happens where
    /// the platform has no such view.
    pub(crate) fn open_contacts(&mut self) -> Vec<Step> {
        if !self.policy.caps.contacts_view || self.contacts.is_some() {
            return Vec::new();
        }
        self.contacts = Some(Contacts::new());
        let mut steps = self
            .surfaces
            .opened(SurfaceKind::Contacts, self.policy.caps.stacking);
        steps.extend(self.draw_contacts());
        steps.push(Step::Ask(Request::ContactsOverview));
        steps
    }

    /// Whether `id` is the Contacts view's while it is on top.
    pub(crate) fn contacts_answers(id: CommandId) -> bool {
        matches!(
            id,
            CommandId::Back
                | CommandId::NextMessage
                | CommandId::PrevMessage
                | CommandId::FirstMessage
                | CommandId::LastMessage
                | CommandId::ContactsTabPrevious
                | CommandId::ContactsTabNext
                | CommandId::ContactsFilter
                | CommandId::ContactShowMail
                | CommandId::ContactCompose
                | CommandId::ContactPopover
        )
    }

    /// A command while the Contacts view is on top. It holds the keyboard:
    /// what is not its own does nothing, but the key map and Quit.
    pub(crate) fn contacts_command(&mut self, id: CommandId, rows: &dyn Rows) -> Option<Vec<Step>> {
        let contacts = self.contacts.as_mut()?;
        match id {
            CommandId::Back => {
                if contacts.filter.is_empty() {
                    return Some(self.close_contacts());
                }
                Some(self.contacts_filter_typed(String::new()))
            }
            CommandId::NextMessage => Some(self.contacts_step(1)),
            CommandId::PrevMessage => Some(self.contacts_step(-1)),
            CommandId::FirstMessage => Some(self.contacts_step(i64::MIN / 2)),
            CommandId::LastMessage => Some(self.contacts_step(i64::MAX / 2)),
            CommandId::ContactsTabNext => {
                let tab = contacts.tab.next();
                Some(self.contacts_tab(tab))
            }
            CommandId::ContactsTabPrevious => {
                let tab = contacts.tab.previous();
                Some(self.contacts_tab(tab))
            }
            CommandId::ContactsFilter => Some(vec![Step::Show(Intent::ContactsFocusFilter)]),
            CommandId::ContactShowMail => {
                let query = self.contacts_mail_query()?;
                let mut steps = self.close_contacts();
                steps.extend(self.open_bar(crate::bar::BarMode::Search, &query, rows));
                Some(steps)
            }
            CommandId::ContactCompose => {
                let address = contacts.person()?.preferred.clone()?;
                let mut steps = self.close_contacts();
                steps.insert(0, Step::Show(Intent::WriteTo(address)));
                Some(steps)
            }
            CommandId::ContactPopover => {
                let address = contacts.person()?.preferred.clone();
                Some(self.open_contact_popover(address, rows))
            }
            CommandId::CheatSheet => None,
            _ => Some(Vec::new()),
        }
    }

    /// The search `Return` types: every address the person owns, as the card
    /// knows them, else the one the row shows.
    fn contacts_mail_query(&self) -> Option<String> {
        let contacts = self.contacts.as_ref()?;
        let person = contacts.person()?;
        let known = contacts
            .card
            .as_ref()
            .filter(|card| card.person.id == person.id)
            .map(|card| {
                card.person
                    .addresses
                    .iter()
                    .map(|owned| owned.address.address.clone())
                    .collect::<Vec<_>>()
            })
            .filter(|addresses| !addresses.is_empty());
        let addresses = known.or_else(|| person.preferred.clone().map(|address| vec![address]))?;
        Some(postio_ui::contacts::show_mail_query(&addresses))
    }

    /// The view goes, and the keyboard goes home once nothing is over the
    /// list.
    fn close_contacts(&mut self) -> Vec<Step> {
        self.contacts = None;
        self.surfaces.dismiss(SurfaceKind::Contacts);
        let mut steps = vec![Step::Show(Intent::CloseSurface(SurfaceKind::Contacts))];
        if !self.has_surface() {
            steps.push(Step::Show(Intent::KeyboardHome));
        }
        steps
    }

    /// The frame, whole.
    fn draw_contacts(&self) -> Vec<Step> {
        let Some(contacts) = self.contacts.as_ref() else {
            return Vec::new();
        };
        let keymap = self.bar.keymap();
        vec![Step::Show(Intent::Contacts(Box::new(ContactsView {
            strip: words::strip(&contacts.overview, contacts.tab, keymap),
            footer: words::footer(&contacts.overview, keymap),
            total: contacts.total(),
            filter: contacts.filter.clone(),
            loaded: contacts.loaded,
        })))]
    }

    /// The cursor moves by `by`, clamped to the list, and the card follows.
    fn contacts_step(&mut self, by: i64) -> Vec<Step> {
        let Some(contacts) = self.contacts.as_mut() else {
            return Vec::new();
        };
        let total = i64::from(contacts.total());
        if total == 0 {
            return Vec::new();
        }
        let from = i64::from(contacts.cursor.unwrap_or(0));
        let to = (from + by).clamp(0, total - 1) as u32;
        if contacts.cursor == Some(to) {
            return Vec::new();
        }
        contacts.cursor = Some(to);
        let mut steps = vec![Step::Show(Intent::ContactsCursor(Some(to)))];
        steps.extend(self.contacts_card_asked());
        steps
    }

    /// A tab chosen, by key or by click: the list starts over for it.
    pub(crate) fn contacts_tab(&mut self, tab: ContactTab) -> Vec<Step> {
        let Some(contacts) = self.contacts.as_mut() else {
            return Vec::new();
        };
        if contacts.tab == tab {
            return Vec::new();
        }
        contacts.tab = tab;
        self.contacts_restart()
    }

    /// The list starts over: the tab or the filter changed.
    fn contacts_restart(&mut self) -> Vec<Step> {
        let stamp = self.stamp();
        let Some(contacts) = self.contacts.as_mut() else {
            return Vec::new();
        };
        contacts.filtered = None;
        contacts.befores.clear();
        contacts.window.reset(contacts.tab_total());
        contacts.cursor = (contacts.total() > 0).then_some(0);
        contacts.card = None;
        contacts.card_for = None;
        let mut steps = self.draw_contacts();
        steps.push(Step::Show(Intent::ContactsRows { first: 0, count: 0 }));
        let (tab, text) = {
            let contacts = self.contacts.as_ref().expect("held");
            (contacts.tab, contacts.filter.clone())
        };
        if text.trim().is_empty() {
            steps.push(Step::Show(Intent::ContactsCursor(
                self.contacts.as_ref().and_then(|contacts| contacts.cursor),
            )));
            return steps;
        }
        if let Some(contacts) = self.contacts.as_mut() {
            contacts.filter_stamp = stamp;
        }
        steps.push(Step::Ask(Request::ContactsFiltered {
            tab,
            text,
            cap: FILTER_CAP,
            stamp,
        }));
        steps
    }

    /// The filter field holds `text` now.
    pub(crate) fn contacts_filter_typed(&mut self, text: String) -> Vec<Step> {
        let Some(contacts) = self.contacts.as_mut() else {
            return Vec::new();
        };
        if contacts.filter == text {
            return Vec::new();
        }
        contacts.filter = text.clone();
        let stamp = self.stamp();
        let contacts = self.contacts.as_mut().expect("held");
        if text.trim().is_empty() {
            // Back to the tab's own list.
            return self.contacts_restart();
        }
        contacts.filter_stamp = stamp;
        let tab = contacts.tab;
        let mut steps = self.draw_contacts();
        steps.push(Step::Ask(Request::ContactsFiltered {
            tab,
            text,
            cap: FILTER_CAP,
            stamp,
        }));
        steps
    }

    /// A click on the row at `index`.
    pub(crate) fn contacts_point(&mut self, index: u32) -> Vec<Step> {
        let Some(contacts) = self.contacts.as_mut() else {
            return Vec::new();
        };
        if index >= contacts.total() || contacts.cursor == Some(index) {
            return Vec::new();
        }
        contacts.cursor = Some(index);
        let mut steps = vec![Step::Show(Intent::ContactsCursor(Some(index)))];
        steps.extend(self.contacts_card_asked());
        steps
    }

    /// The card of the person under the cursor is wanted: asked, once.
    fn contacts_card_asked(&mut self) -> Vec<Step> {
        let stamp = self.stamp();
        let Some(contacts) = self.contacts.as_mut() else {
            return Vec::new();
        };
        let Some(person) = contacts.person().map(|row| row.id) else {
            return Vec::new();
        };
        if contacts.card_for == Some(person) {
            return Vec::new();
        }
        contacts.card_for = Some(person);
        contacts.card_stamp = stamp;
        contacts.card = None;
        vec![
            Step::Show(Intent::ContactsCard(None)),
            Step::Ask(Request::ContactCard { person, stamp }),
        ]
    }

    /// The row at `index`, in words, when its page is here.
    pub fn contacts_row(&self, index: u32) -> Option<RowView> {
        let contacts = self.contacts.as_ref()?;
        let row = contacts.row(index)?;
        let before = contacts.before(index);
        Some(words::row(row, before.as_deref(), self.bar.now()))
    }

    /// The rows the frontend can see at `index`: reads the page they are on
    /// when it is not here and not already on its way.
    pub fn contacts_wanted(&mut self, index: u32) -> Vec<Effect> {
        let Some(contacts) = self.contacts.as_mut() else {
            return Vec::new();
        };
        if contacts.filtered.is_some() || index >= contacts.total() {
            return Vec::new();
        }
        let page = match contacts.window.row_at(index) {
            Slot::Loading {
                request: Some(page),
            } => page,
            _ => return Vec::new(),
        };
        let after = (page > 0)
            .then(|| contacts.window.peek(page * PAGE_SIZE - 1))
            .flatten()
            .map(|row| (row.sort_key.clone(), row.id));
        let skip = if after.is_some() { 0 } else { page * PAGE_SIZE };
        let request = Request::ContactsPage {
            tab: contacts.tab,
            after,
            skip,
            limit: PAGE_SIZE,
            stamp: contacts.window.generation(),
            page,
        };
        self.effects(vec![Step::Ask(request)])
    }

    /// The card of the person under the cursor, in words, when it has been
    /// read.
    pub fn contacts_card(&self) -> Option<CardView> {
        let contacts = self.contacts.as_ref()?;
        let card = contacts.card.as_ref()?;
        Some(words::card(card, self.bar.keymap(), self.bar.now()))
    }

    /// Where the cursor is.
    pub fn contacts_cursor(&self) -> Option<u32> {
        self.contacts.as_ref()?.cursor
    }

    /// How many rows the list has.
    pub fn contacts_total(&self) -> u32 {
        self.contacts.as_ref().map_or(0, Contacts::total)
    }

    /// Something answered, for the view.
    pub(crate) fn contacts_reply(&mut self, reply: Reply) -> Vec<Step> {
        match reply {
            Reply::ContactsOverview(Ok(overview)) => self.contacts_overview_read(overview),
            Reply::ContactsPage {
                stamp,
                page,
                answer,
            } => self.contacts_page_read(stamp, page, answer),
            Reply::ContactsFiltered { stamp, answer } => self.contacts_filtered_read(stamp, answer),
            Reply::ContactCard { stamp, answer } => self.contacts_card_read(stamp, answer),
            // A read that failed leaves the view as it was: a skeleton, or
            // the list it had.
            _ => Vec::new(),
        }
    }

    fn contacts_overview_read(&mut self, overview: ContactsOverview) -> Vec<Step> {
        let Some(contacts) = self.contacts.as_mut() else {
            return Vec::new();
        };
        let first = !contacts.loaded;
        contacts.overview = overview;
        contacts.loaded = true;
        if first || contacts.filtered.is_none() && contacts.window.total() != contacts.tab_total() {
            return self.contacts_restart();
        }
        self.draw_contacts()
    }

    fn contacts_page_read(
        &mut self,
        stamp: u64,
        page: u32,
        answer: Result<ContactsPageRead, String>,
    ) -> Vec<Step> {
        let Some(contacts) = self.contacts.as_mut() else {
            return Vec::new();
        };
        if stamp != contacts.window.generation() {
            return Vec::new();
        }
        let read = match answer {
            Ok(read) => read,
            Err(_) => {
                contacts.window.abandon(stamp, page);
                return Vec::new();
            }
        };
        contacts.befores.insert(page, read.before);
        let total = contacts.window.total();
        let first = page * PAGE_SIZE;
        let count = read.rows.len() as u32;
        match contacts.window.deliver(stamp, page, read.rows, total) {
            Delivered::Stale => Vec::new(),
            Delivered::Filled { .. } => {
                let mut steps = vec![Step::Show(Intent::ContactsRows { first, count })];
                // The card of the row the cursor is on, now that it is here.
                steps.extend(self.contacts_card_asked());
                steps
            }
        }
    }

    fn contacts_filtered_read(
        &mut self,
        stamp: u64,
        answer: Result<Vec<ContactListRow>, String>,
    ) -> Vec<Step> {
        let Some(contacts) = self.contacts.as_mut() else {
            return Vec::new();
        };
        if stamp != contacts.filter_stamp {
            return Vec::new();
        }
        let Ok(rows) = answer else {
            return Vec::new();
        };
        contacts.cursor = (!rows.is_empty()).then_some(0);
        contacts.filtered = Some(rows);
        contacts.card = None;
        contacts.card_for = None;
        let mut steps = self.draw_contacts();
        steps.push(Step::Show(Intent::ContactsRows { first: 0, count: 0 }));
        steps.push(Step::Show(Intent::ContactsCursor(
            self.contacts.as_ref().and_then(|contacts| contacts.cursor),
        )));
        steps.extend(self.contacts_card_asked());
        steps
    }

    fn contacts_card_read(
        &mut self,
        stamp: u64,
        answer: Result<Option<Box<ContactCardRead>>, String>,
    ) -> Vec<Step> {
        let Some(contacts) = self.contacts.as_mut() else {
            return Vec::new();
        };
        if stamp != contacts.card_stamp {
            return Vec::new();
        }
        let Ok(Some(card)) = answer else {
            return Vec::new();
        };
        contacts.card = Some(*card);
        let card = self.contacts_card().map(Box::new);
        vec![Step::Show(Intent::ContactsCard(card))]
    }

    /// The engine said something: a change to the people reads the counts
    /// and the list again.
    pub(crate) fn contacts_heard(&mut self, event: &postio_core::Event) -> Vec<Step> {
        if !matches!(event, postio_core::Event::ContactsChanged { .. }) {
            return Vec::new();
        }
        let Some(contacts) = self.contacts.as_mut() else {
            return Vec::new();
        };
        contacts.card_for = None;
        let mut steps = vec![Step::Ask(Request::ContactsOverview)];
        // The window is told the counts moved when they arrive.
        contacts.befores.clear();
        let total = contacts.tab_total();
        contacts.window.reset(total);
        steps.push(Step::Show(Intent::ContactsRows { first: 0, count: 0 }));
        steps
    }
}
