//! The Contacts view: a mode of the main window. The tab, filter, cursor, resident pages and card
//! are the controller's. One stamped read answers each thing, and an answer for a list or card
//! since replaced changes nothing.

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

pub const FILTER_CAP: u32 = 200;

#[derive(Debug, Clone, PartialEq)]
pub struct ContactsView {
    pub strip: StripView,
    pub footer: FooterView,
    pub total: u32,
    pub filter: String,
    /// Whether the counts have been read: before, the frame is a skeleton.
    pub loaded: bool,
}

#[derive(Debug)]
pub(crate) struct Contacts {
    tab: ContactTab,
    filter: String,
    overview: ContactsOverview,
    loaded: bool,
    window: ContactsWindow,
    befores: HashMap<u32, Option<String>>,
    filtered: Option<Vec<ContactListRow>>,
    filter_stamp: u64,
    pub(crate) cursor: Option<u32>,
    pub(crate) card: Option<ContactCardRead>,
    card_for: Option<ContactId>,
    card_stamp: u64,
    pub(crate) edit: Option<crate::contact_edit::ContactEdit>,
    pub(crate) marks: Vec<ContactId>,
    pub(crate) merge: Option<crate::contact_merge::MergeSheet>,
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
            edit: None,
            marks: Vec::new(),
            merge: None,
        }
    }

    fn total(&self) -> u32 {
        match &self.filtered {
            Some(rows) => rows.len() as u32,
            None => self.window.total(),
        }
    }

    fn row(&self, index: u32) -> Option<&ContactListRow> {
        match &self.filtered {
            Some(rows) => rows.get(index as usize),
            None => self.window.peek(index),
        }
    }

    fn drawn(&self, index: u32) -> Option<&ContactListRow> {
        match &self.filtered {
            Some(rows) => rows.get(index as usize),
            None => self.window.drawn(index),
        }
    }

    fn before(&self, index: u32) -> Option<String> {
        if index == 0 {
            return None;
        }
        if let Some(row) = self.drawn(index - 1) {
            return Some(row.sort_key.clone());
        }
        index
            .is_multiple_of(PAGE_SIZE)
            .then(|| self.befores.get(&(index / PAGE_SIZE)).cloned().flatten())
            .flatten()
    }

    pub(crate) fn tab_now(&self) -> ContactTab {
        self.tab
    }

    fn tab_total(&self) -> u32 {
        self.overview
            .tabs
            .iter()
            .find(|(tab, _)| *tab == self.tab)
            .map_or(0, |(_, count)| *count)
    }

    pub(crate) fn person(&self) -> Option<&ContactListRow> {
        self.row(self.cursor?)
    }
}

impl FocusController {
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
                | CommandId::ContactEdit
                | CommandId::Unsubscribe
                | CommandId::ContactNew
                | CommandId::ContactSave
                | CommandId::ToggleSelection
                | CommandId::ContactJoin
                | CommandId::ContactsSuggestions
                | CommandId::SuggestionDismiss
        )
    }

    pub(crate) fn contacts_command(&mut self, id: CommandId, rows: &dyn Rows) -> Option<Vec<Step>> {
        let contacts = self.contacts.as_mut()?;
        if id == CommandId::Undo && contacts.edit.is_none() {
            return None;
        }
        if contacts.merge.is_some() {
            return match id {
                CommandId::Back => Some(self.contact_merge_later()),
                CommandId::ContactShowMail => Some(self.contact_merge_now()),
                CommandId::SuggestionDismiss => Some(self.contact_merge_not_the_same()),
                CommandId::NextMessage => Some(self.contact_merge_step(1)),
                CommandId::PrevMessage => Some(self.contact_merge_step(-1)),
                CommandId::CheatSheet => None,
                _ => Some(Vec::new()),
            };
        }
        if contacts.edit.is_some() {
            return match id {
                CommandId::Back => Some(self.contact_edit_back()),
                CommandId::ContactSave => Some(self.contact_edit_save()),
                CommandId::CheatSheet => None,
                _ => Some(Vec::new()),
            };
        }
        match id {
            CommandId::ToggleSelection => Some(self.contacts_mark()),
            CommandId::ContactJoin => Some(self.contacts_merge_marked()),
            CommandId::ContactsSuggestions => Some(self.contacts_review_ask()),
            CommandId::ContactEdit => Some(self.contact_edit_open()),
            CommandId::Unsubscribe => Some(
                contacts
                    .card
                    .as_ref()
                    .and_then(|card| card.issues.first())
                    .map(|issue| vec![Step::Ask(Request::Unsubscribe(issue.message))])
                    .unwrap_or_default(),
            ),
            CommandId::ContactNew => Some(self.contact_new_open()),
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

    fn close_contacts(&mut self) -> Vec<Step> {
        self.contacts = None;
        self.surfaces.dismiss(SurfaceKind::Contacts);
        let mut steps = vec![Step::Show(Intent::CloseSurface(SurfaceKind::Contacts))];
        if !self.has_surface() {
            steps.push(Step::Show(Intent::KeyboardHome));
        }
        steps
    }

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

    fn contacts_restart(&mut self) -> Vec<Step> {
        self.contacts_restart_as(false)
    }

    fn contacts_restart_as(&mut self, renew: bool) -> Vec<Step> {
        let stamp = self.stamp();
        let Some(contacts) = self.contacts.as_mut() else {
            return Vec::new();
        };
        contacts.filtered = None;
        contacts.befores.clear();
        if renew && contacts.filter.trim().is_empty() {
            contacts.window.renew(contacts.tab_total());
        } else {
            contacts.window.reset(contacts.tab_total());
        }
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

    pub(crate) fn contacts_card_asked(&mut self) -> Vec<Step> {
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

    pub fn contacts_row(&self, index: u32) -> Option<RowView> {
        let contacts = self.contacts.as_ref()?;
        let row = contacts.drawn(index)?;
        let before = contacts.before(index);
        let mut view = words::row(row, before.as_deref(), self.bar.now());
        view.marked = contacts.marks.contains(&row.id);
        Some(view)
    }

    pub fn contacts_wanted(&mut self, index: u32) -> Vec<Effect> {
        let steps = self.contacts_page_asked(index);
        self.effects(steps)
    }

    fn contacts_renewed_asked(&mut self) -> Vec<Step> {
        let pages = self
            .contacts
            .as_ref()
            .map(|contacts| contacts.window.renewing())
            .unwrap_or_default();
        pages
            .into_iter()
            .flat_map(|page| self.contacts_page_asked(page * PAGE_SIZE))
            .collect()
    }

    fn contacts_page_asked(&mut self, index: u32) -> Vec<Step> {
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
        vec![Step::Ask(request)]
    }

    pub fn contacts_card(&self) -> Option<CardView> {
        let contacts = self.contacts.as_ref()?;
        let card = contacts.card.as_ref()?;
        Some(words::card(card, self.bar.keymap(), self.bar.now()))
    }

    pub fn contacts_cursor(&self) -> Option<u32> {
        self.contacts.as_ref()?.cursor
    }

    pub fn contacts_total(&self) -> u32 {
        self.contacts.as_ref().map_or(0, Contacts::total)
    }

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
            Reply::ContactPosition { person, at } => self.contacts_position_read(person, at),
            Reply::ContactReview(groups) => self.contacts_review_read(groups),
            Reply::ContactMergeOf(group) => self.contacts_merge_of_read(group.map(|g| *g)),
            Reply::ContactSeen(items) => {
                if let Some(edit) = self.contacts.as_mut().and_then(|c| c.edit.as_mut()) {
                    edit.seen = items;
                }
                self.draw_contact_edit()
            }
            Reply::ContactGroups(Ok(names)) => {
                if let Some(edit) = self.contacts.as_mut().and_then(|c| c.edit.as_mut()) {
                    edit.known_groups = names;
                }
                self.draw_contact_edit()
            }
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
            // The restart puts the cursor on the first row; an import's new
            // person is where it goes instead.
            let mut steps = self.contacts_restart_as(!first);
            steps.extend(self.contacts_renewed_asked());
            steps.extend(self.contacts_go_to_import());
            return steps;
        }
        let mut steps = self.draw_contacts();
        steps.extend(self.contacts_renewed_asked());
        steps.extend(self.contacts_go_to_import());
        steps
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

    pub(crate) fn contacts_heard(&mut self, event: &postio_core::Event) -> Vec<Step> {
        if !matches!(event, postio_core::Event::ContactsChanged { .. }) {
            return Vec::new();
        }
        let Some(contacts) = self.contacts.as_mut() else {
            return Vec::new();
        };
        contacts.card_for = None;
        let mut steps = vec![Step::Ask(Request::ContactsOverview)];
        contacts.befores.clear();
        let total = contacts.tab_total();
        contacts.window.renew(total);
        steps.push(Step::Show(Intent::ContactsRows { first: 0, count: 0 }));
        steps.extend(self.merge_queue_asked());
        steps
    }
}
