//! The search results view: the page "Show all" opens in the inbox's place. It draws what the
//! controller says and decides nothing.
//! Rows are read by position when the list view draws them, so a search of thousands costs what one
//! of a dozen does.
//! A click is told to the controller and moves nothing here: the ring moves when the controller
//! says.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use gtk::gio;
use gtk::glib;
use gtk::pango;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use postio_core::CommandId;
use postio_focus::{
    Input, QueryView, ResultGroup, ResultRow, ResultsView, Run, RunStyle, TermEdit,
};
use postio_search::results::ConversationOrder;
use postio_ui::search_view::{self as words, FilterKind};
use postio_widgets::widgets::keyhint;
use postio_widgets::widgets::space::{S1, S2, S3};

use crate::list::Feed;

const ROWS: &str = "rows";
const FILES: &str = "files";
const PEOPLE: &str = "people";
const NOTHING: &str = "nothing";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Item {
    Header(usize),
    Row(u64),
    Child(u32),
}

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct ResultsItems {
        pub starts: RefCell<Vec<(u32, u64)>>,
        pub total: Cell<u32>,
        pub splice: Cell<Option<(u64, u32)>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ResultsItems {
        const NAME: &'static str = "PostioResultsItems";
        type Type = super::ResultsItems;
        type Interfaces = (gio::ListModel,);
    }

    impl ObjectImpl for ResultsItems {}

    impl ListModelImpl for ResultsItems {
        fn item_type(&self) -> glib::Type {
            gtk::StringObject::static_type()
        }

        fn n_items(&self) -> u32 {
            self.obj().len()
        }

        fn item(&self, position: u32) -> Option<glib::Object> {
            (position < self.obj().len())
                .then(|| gtk::StringObject::new(&position.to_string()).upcast())
        }
    }
}

glib::wrapper! {
    pub struct ResultsItems(ObjectSubclass<imp::ResultsItems>)
        @implements gio::ListModel;
}

impl Default for ResultsItems {
    fn default() -> Self {
        glib::Object::new()
    }
}

impl ResultsItems {
    fn len(&self) -> u32 {
        self.imp().total.get() + self.extra()
    }

    fn extra(&self) -> u32 {
        match self.imp().splice.get() {
            Some((anchor, count)) if self.raw_position_of(anchor).is_some() => count,
            _ => 0,
        }
    }

    fn set_groups(&self, groups: &[ResultGroup]) {
        let mut starts = Vec::with_capacity(groups.len());
        let mut at = 0u32;
        for group in groups {
            starts.push((at, group.first));
            at = at.saturating_add(1).saturating_add(group.rows as u32);
        }
        let before = self.len();
        self.imp().splice.set(None);
        self.imp().starts.replace(starts);
        self.imp().total.set(at);
        self.items_changed(0, before, at);
    }

    fn set_splice(&self, splice: Option<(u64, u32)>) {
        if let Some((anchor, count)) = self.imp().splice.take()
            && let Some(at) = self.raw_position_of(anchor)
            && count > 0
        {
            self.items_changed(at + 1, count, 0);
        }
        if let Some((anchor, count)) = splice {
            self.imp().splice.set(Some((anchor, count)));
            if let Some(at) = self.raw_position_of(anchor)
                && count > 0
            {
                self.items_changed(at + 1, 0, count);
            }
        }
    }

    fn child_position(&self, index: u32) -> Option<u32> {
        let (anchor, count) = self.imp().splice.get()?;
        (index < count)
            .then(|| self.raw_position_of(anchor).map(|at| at + 1 + index))
            .flatten()
    }

    fn child_changed(&self, index: u32) {
        if let Some(at) = self.child_position(index) {
            self.items_changed(at, 1, 1);
        }
    }

    fn item_at(&self, at: u32) -> Option<Item> {
        if at >= self.len() {
            return None;
        }
        let Some((anchor, count)) = self.imp().splice.get() else {
            return self.raw_item_at(at);
        };
        let Some(anchor_at) = self.raw_position_of(anchor) else {
            return self.raw_item_at(at);
        };
        if at <= anchor_at {
            self.raw_item_at(at)
        } else if at <= anchor_at + count {
            Some(Item::Child(at - anchor_at - 1))
        } else {
            self.raw_item_at(at - count)
        }
    }

    fn raw_item_at(&self, at: u32) -> Option<Item> {
        if at >= self.imp().total.get() {
            return None;
        }
        let starts = self.imp().starts.borrow();
        let index = starts.iter().rposition(|(start, _)| *start <= at)?;
        let (start, first) = starts[index];
        Some(match at - start {
            0 => Item::Header(index),
            offset => Item::Row(first + u64::from(offset - 1)),
        })
    }

    fn position_of(&self, position: u64) -> Option<u32> {
        let raw = self.raw_position_of(position)?;
        let moved = match self.imp().splice.get() {
            Some((anchor, count)) if self.raw_position_of(anchor).is_some_and(|at| raw > at) => {
                count
            }
            _ => 0,
        };
        Some(raw + moved)
    }

    fn raw_position_of(&self, position: u64) -> Option<u32> {
        let starts = self.imp().starts.borrow();
        let index = starts.iter().rposition(|(_, first)| *first <= position)?;
        let (start, first) = starts[index];
        let at = u64::from(start) + 1 + (position - first);
        u32::try_from(at)
            .ok()
            .filter(|at| *at < self.imp().total.get())
    }

    fn rows_changed(&self, first: u64, count: u64) {
        for position in first..first.saturating_add(count) {
            if let Some(at) = self.position_of(position) {
                self.items_changed(at, 1, 1);
            }
        }
    }
}

pub struct ResultsPane {
    root: gtk::Box,
    feed: Feed,
    act: Rc<dyn Fn(CommandId)>,
    count_line: gtk::Label,
    sub_line: gtk::Label,
    tabs: gtk::Box,
    sort: gtk::Button,
    back: gtk::Button,
    save_button: gtk::Button,
    save_popover: Rc<crate::save_popover::SavePopover>,
    chips: gtk::Box,
    query_hint: gtk::Box,
    query: gtk::Box,
    timeline_line: gtk::Box,
    buttons: gtk::Box,
    filter_buttons: RefCell<Vec<(FilterKind, gtk::Button)>>,
    timeline: Rc<crate::search_popover::MonthChart>,
    timeline_hint: gtk::Label,
    popover: Rc<crate::search_popover::FilterPopover>,
    items: ResultsItems,
    list: gtk::ListView,
    body: gtk::Stack,
    files: Rc<crate::results_files::FilesGrid>,
    people: Rc<crate::results_files::PeopleList>,
    nothing: Rc<crate::results_files::NoResultsPage>,
    found_nothing: Cell<bool>,
    quick_look: Rc<crate::results_files::QuickLookPanel>,
    hints: gtk::Box,
    bulk: crate::bulk::Bulk,
    footer: gtk::Label,
    foot: gtk::Box,
    scope_footer: Rc<crate::scope_footer::ScopeFooter>,
    window_open: Cell<bool>,
    view: RefCell<Option<ResultsView>>,
    cursor: Cell<Option<u64>>,
    splice: RefCell<Option<postio_focus::ResultSplice>>,
    child_focus: Cell<Option<u32>>,
    asked: RefCell<std::collections::HashSet<u64>>,
    me: RefCell<Weak<ResultsPane>>,
}

impl ResultsPane {
    pub fn new(feed: Feed, act: impl Fn(CommandId) + 'static) -> Rc<Self> {
        let back = gtk::Button::new();
        back.add_css_class("flat");
        back.add_css_class("focus-results-back");
        let back_line = gtk::Box::new(gtk::Orientation::Horizontal, S1);
        back_line.append(&gtk::Image::from_icon_name("go-previous-symbolic"));
        back_line.append(&gtk::Label::new(Some(words::BACK_TO_INBOX)));
        back.set_child(Some(&back_line));
        back.update_property(&[gtk::accessible::Property::Label(words::BACK_TO_INBOX)]);
        back.set_focus_on_click(false);

        let count_line = gtk::Label::new(None);
        count_line.add_css_class("focus-results-count");
        count_line.set_xalign(0.0);
        let sub_line = gtk::Label::new(None);
        sub_line.add_css_class("focus-results-sub");
        sub_line.add_css_class("dim-label");
        sub_line.set_xalign(0.0);
        let titles = gtk::Box::new(gtk::Orientation::Vertical, 0);
        titles.append(&count_line);
        titles.append(&sub_line);
        titles.set_hexpand(true);

        let tabs = gtk::Box::new(gtk::Orientation::Horizontal, S1);
        tabs.add_css_class("focus-results-tabs");
        let sort = gtk::Button::new();
        sort.add_css_class("focus-results-sort");
        sort.set_focus_on_click(false);

        let head = gtk::Box::new(gtk::Orientation::Horizontal, S3);
        head.add_css_class("focus-results-head");
        head.append(&back);
        head.append(&titles);
        head.append(&tabs);
        head.append(&sort);
        let save_button = gtk::Button::new();
        save_button.add_css_class("focus-results-save");
        save_button.set_focus_on_click(false);
        head.append(&save_button);

        let chips = gtk::Box::new(gtk::Orientation::Horizontal, S1);
        chips.add_css_class("focus-results-chips");
        chips.set_hexpand(true);
        let query_hint = gtk::Box::new(gtk::Orientation::Horizontal, S1);
        query_hint.add_css_class("focus-results-query-hint");
        let query = gtk::Box::new(gtk::Orientation::Horizontal, S2);
        query.add_css_class("focus-results-query");
        query.append(&gtk::Image::from_icon_name("system-search-symbolic"));
        query.append(&chips);
        query.append(&query_hint);
        let buttons = gtk::Box::new(gtk::Orientation::Horizontal, S1);
        buttons.add_css_class("focus-results-filters");

        let timeline = crate::search_popover::MonthChart::new();
        timeline.widget().set_hexpand(true);
        timeline.widget().set_halign(gtk::Align::Center);
        let timeline_hint = gtk::Label::new(None);
        timeline_hint.add_css_class("dim-label");
        timeline_hint.add_css_class("focus-results-timeline-hint");
        timeline_hint.set_xalign(1.0);
        timeline_hint.set_justify(gtk::Justification::Right);
        let timeline_line = gtk::Box::new(gtk::Orientation::Horizontal, S3);
        timeline_line.add_css_class("focus-results-timeline");
        timeline_line.append(timeline.widget());
        timeline_line.append(&timeline_hint);

        let items = ResultsItems::default();
        let selection = gtk::NoSelection::new(Some(items.clone()));
        let factory = gtk::SignalListItemFactory::new();
        // A row is drawn again as a new widget whenever it changes, and the keyboard stays in the
        // list across it (#2136).
        let list = crate::rebuild::keeping_keyboard(&selection, || {
            gtk::ListView::new(Some(selection.clone()), Some(factory.clone()))
        });
        list.add_css_class("focus-results-list");
        list.set_single_click_activate(false);
        list.set_accessible_role(gtk::AccessibleRole::List);
        list.update_property(&[gtk::accessible::Property::Label(words::A11Y_RESULTS)]);
        let scrolled = gtk::ScrolledWindow::builder()
            .child(&list)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .build();

        let hints = gtk::Box::new(gtk::Orientation::Horizontal, S3);
        hints.set_hexpand(true);
        let footer = gtk::Label::new(None);
        footer.add_css_class("dim-label");
        footer.add_css_class("focus-results-footer-right");
        let foot = gtk::Box::new(gtk::Orientation::Horizontal, S2);
        foot.add_css_class("focus-results-footer");
        foot.append(&hints);
        foot.append(&footer);

        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.add_css_class("focus-results");
        root.append(&head);
        root.append(&buttons);
        root.append(&timeline_line);
        let act: Rc<dyn Fn(CommandId)> = Rc::new(act);
        let bulk =
            crate::bulk::Bulk::with_verbs(postio_ui::focus_dialog::RESULTS_BULK, &feed.keymap());
        bulk.connect_command({
            let act = Rc::clone(&act);
            move |command| act(command)
        });
        let files = crate::results_files::FilesGrid::new(feed.clone(), Rc::clone(&act));
        let quick_look = crate::results_files::QuickLookPanel::new(Rc::clone(&act));
        let people = crate::results_files::PeopleList::new(feed.clone(), Rc::clone(&act));
        let nothing = crate::results_files::NoResultsPage::new(Rc::clone(&act));
        let nothing_scroller = gtk::ScrolledWindow::builder()
            .child(nothing.widget())
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .build();
        let body = gtk::Stack::new();
        body.set_transition_type(gtk::StackTransitionType::None);
        body.set_vexpand(true);
        body.add_named(&scrolled, Some(ROWS));
        body.add_named(files.widget(), Some(FILES));
        body.add_named(people.widget(), Some(PEOPLE));
        body.add_named(&nothing_scroller, Some(NOTHING));
        let over = gtk::Overlay::new();
        over.set_child(Some(&body));
        over.add_overlay(quick_look.widget());
        root.append(&over);
        root.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        root.append(bulk.widget());
        root.append(&foot);
        let scope_footer = Rc::new(crate::scope_footer::ScopeFooter::new());
        root.append(scope_footer.widget());

        let popover_feed = feed.clone();
        let popover = crate::search_popover::FilterPopover::new(move |input| {
            popover_feed.input(input);
        });
        let save_feed = feed.clone();
        let save_popover = crate::save_popover::SavePopover::new(move |input| {
            save_feed.input(input);
        });
        let pane = Rc::new(ResultsPane {
            root,
            feed,
            act,
            count_line,
            sub_line,
            tabs,
            sort,
            back: back.clone(),
            save_button,
            save_popover,
            chips,
            query_hint,
            query: query.clone(),
            timeline_line: timeline_line.clone(),
            buttons,
            filter_buttons: RefCell::default(),
            timeline,
            timeline_hint,
            popover,
            items,
            list,
            body,
            files,
            people,
            nothing,
            found_nothing: Cell::new(false),
            quick_look,
            hints,
            bulk,
            footer,
            foot,
            scope_footer,
            window_open: Cell::new(false),
            view: RefCell::default(),
            cursor: Cell::new(None),
            splice: RefCell::default(),
            child_focus: Cell::new(None),
            asked: RefCell::default(),
            me: RefCell::default(),
        });
        pane.me.replace(Rc::downgrade(&pane));
        pane.connect(&factory, &back);
        pane
    }

    fn connect(self: &Rc<Self>, factory: &gtk::SignalListItemFactory, back: &gtk::Button) {
        let weak = Rc::downgrade(self);
        factory.connect_bind({
            let weak = weak.clone();
            move |_, item| {
                let (Some(item), Some(pane)) =
                    (item.downcast_ref::<gtk::ListItem>(), weak.upgrade())
                else {
                    return;
                };
                let at = item
                    .item()
                    .and_downcast::<gtk::StringObject>()
                    .and_then(|object| object.string().parse::<u32>().ok());
                item.set_child(at.and_then(|at| pane.draw_item(at, item)).as_ref());
            }
        });
        self.list.connect_activate({
            let weak = weak.clone();
            move |_, at| {
                let Some(pane) = weak.upgrade() else {
                    return;
                };
                if let Some(Item::Row(position)) = pane.items.item_at(at) {
                    pane.feed.input(Input::ResultsPoint(position));
                    (pane.act)(CommandId::OpenMessage);
                }
            }
        });
        self.timeline.connect_months({
            let weak = weak.clone();
            move |first, last| {
                if let Some(pane) = weak.upgrade() {
                    pane.feed
                        .input(Input::SearchEdit(TermEdit::SetMonths { first, last }));
                }
            }
        });
        back.connect_clicked({
            let weak = weak.clone();
            move |_| {
                if let Some(pane) = weak.upgrade() {
                    (pane.act)(CommandId::Back);
                }
            }
        });
        // The keyboard goes back to the results, not to the button the popover hung from, once GTK
        // has handed it there (after `closed`).
        self.save_popover.connect_closed({
            let weak = weak.clone();
            move || {
                let weak = weak.clone();
                glib::idle_add_local_once(move || {
                    if let Some(pane) = weak.upgrade() {
                        pane.focus();
                    }
                });
            }
        });
        self.save_button.connect_clicked({
            let weak = weak.clone();
            move |_| {
                if let Some(pane) = weak.upgrade() {
                    (pane.act)(CommandId::SaveSearch);
                }
            }
        });
        self.sort.connect_clicked(move |_| {
            let Some(pane) = weak.upgrade() else {
                return;
            };
            let order = pane.view.borrow().as_ref().map(|view| view.order);
            let other = match order {
                Some(ConversationOrder::Newest) => ConversationOrder::BestMatch,
                _ => ConversationOrder::Newest,
            };
            pane.feed.input(Input::ResultsOrder(other));
        });
    }

    pub fn widget(&self) -> &gtk::Box {
        &self.root
    }

    pub fn query_field(&self) -> &gtk::Box {
        &self.query
    }

    pub fn show(&self, view: &ResultsView) {
        self.dress_head();
        self.count_line.set_text(&view.count_line);
        self.sub_line.set_text(&view.sub_line);
        self.sub_line.set_visible(!view.sub_line.is_empty());
        self.show_tabs(view);
        self.sort.set_visible(view.sortable);
        self.timeline.show(&view.months);
        self.timeline.widget().set_visible(!view.months.is_empty());
        let mut hint = view.timeline_hint.replacen(" \u{b7} ", "\n", 1);
        if let Some(step) = &view.timeline_step {
            hint.push_str(&format!("\n{} {}", step.key, step.label));
        }
        self.timeline_hint.set_text(&hint);
        self.sort.set_label(&format!(
            "{} {}",
            words::SORT,
            match view.order {
                ConversationOrder::Newest => words::NEWEST,
                _ => words::BEST_MATCH,
            }
        ));
        // As wide as its widest words, so the tabs beside it hold still while it changes (#2082).
        if let Some(label) = self.sort.child().and_downcast::<gtk::Label>() {
            label.set_width_chars(
                [words::NEWEST, words::BEST_MATCH]
                    .iter()
                    .map(|order| format!("{} {order}", words::SORT).chars().count())
                    .max()
                    .unwrap_or_default() as i32,
            );
        }
        clear(&self.hints);
        let bulk = view.selected > 0;
        self.bulk
            .set_summary(bulk.then(|| words::checked_line(view.selected)).as_deref());
        if !bulk {
            for hint in &view.hints {
                self.hints.append(&hint_line(&hint.key, &hint.label));
            }
        }
        match (&view.select_all, bulk) {
            (Some(all), true) => self.footer.set_text(&format!("{} {}", all.key, all.label)),
            _ => self.footer.set_text(&view.footer),
        }
        let people = view
            .tabs
            .iter()
            .any(|tab| tab.selected && tab.tab == postio_search::results::ResultsTab::People);
        match &view.files {
            Some(header) => {
                self.files.show(header, view.rows);
                self.switch_body(FILES);
            }
            None if people => {
                self.people.show(view.rows);
                self.switch_body(PEOPLE);
            }
            None if self.found_nothing.get() => self.switch_body(NOTHING),
            None => self.switch_body(ROWS),
        }
        let groups_changed = self
            .view
            .borrow()
            .as_ref()
            .is_none_or(|was| was.groups != view.groups || was.rows != view.rows);
        self.view.replace(Some(view.clone()));
        self.cursor.set(view.cursor);
        if groups_changed {
            self.asked.borrow_mut().clear();
            self.items.set_groups(&view.groups);
        } else {
            let total = self.items.n_items();
            self.items.items_changed(0, total, total);
        }
        self.reveal_cursor();
    }

    pub fn rows_changed(&self, first: u64, count: u64) {
        if self.showing_files() {
            self.files.refresh();
            return;
        }
        if self.showing_people() {
            self.people.refresh();
            return;
        }
        {
            let mut asked = self.asked.borrow_mut();
            for position in first..first.saturating_add(count) {
                asked.remove(&position);
            }
        }
        self.items.rows_changed(first, count);
        // A row still being read is drawn empty; the rows landing now grow into their height and
        // push the ring down, out of view.
        if self
            .cursor
            .get()
            .is_some_and(|cursor| first <= cursor.saturating_add(1))
        {
            self.reveal_cursor();
        }
    }

    pub fn show_splice(&self, splice: Option<&postio_focus::ResultSplice>) {
        let anchor = splice.map(|splice| (splice.anchor, splice.children.len() as u32));
        self.splice.replace(splice.cloned());
        self.child_focus
            .set(splice.and_then(|splice| splice.focused));
        self.items.set_splice(anchor);
        if let Some((anchor, _)) = anchor {
            self.items.rows_changed(anchor, 1);
        }
    }

    pub fn show_child_focus(&self, child: Option<u32>) {
        let was = self.child_focus.replace(child);
        if let Some(was) = was {
            self.items.child_changed(was);
        }
        if let Some(child) = child {
            self.items.child_changed(child);
        }
        if let Some((anchor, _)) = self.items.imp().splice.get() {
            self.items.rows_changed(anchor, 1);
        }
        if let Some(at) = child.and_then(|child| self.items.child_position(child)) {
            let weak = self.me.borrow().clone();
            glib::idle_add_local_once(move || {
                if let Some(pane) = weak.upgrade() {
                    pane.list.scroll_to(at, gtk::ListScrollFlags::NONE, None);
                }
            });
        } else {
            self.reveal_cursor();
        }
    }

    pub fn child_count(&self) -> usize {
        self.splice
            .borrow()
            .as_ref()
            .map_or(0, |splice| splice.children.len())
    }

    pub fn child_focus(&self) -> Option<u32> {
        self.child_focus.get()
    }

    pub fn show_cursor(&self, position: u64) {
        if self.showing_files() {
            self.cursor.set(Some(position));
            self.files.refresh();
            return;
        }
        if self.showing_people() {
            self.cursor.set(Some(position));
            self.people.refresh();
            self.people.reveal(position);
            return;
        }
        let was = self.cursor.replace(Some(position));
        if let Some(was) = was {
            self.items.rows_changed(was, 1);
        }
        self.items.rows_changed(position, 1);
        self.reveal_cursor();
    }

    pub fn show_query(&self, query: &QueryView) {
        clear(&self.chips);
        for chip in &query.chips {
            let said = gtk::Box::new(gtk::Orientation::Horizontal, S1);
            let op = gtk::Label::new(Some(&chip.operator));
            op.add_css_class("focus-results-chip-op");
            said.append(&op);
            let value = gtk::Label::new(Some(&chip.value));
            if chip.excluded {
                value.set_markup(&format!("<s>{}</s>", glib::markup_escape_text(&chip.value)));
            }
            said.append(&value);
            let remove = gtk::Button::from_icon_name("window-close-symbolic");
            remove.add_css_class("flat");
            remove.add_css_class("focus-results-chip-remove");
            remove.set_focus_on_click(false);
            remove.update_property(&[gtk::accessible::Property::Label(&format!(
                "Remove {}{}",
                chip.operator, chip.value
            ))]);
            let token = chip.token;
            let weak = self.me.borrow().clone();
            remove.connect_clicked(move |_| {
                if let Some(pane) = weak.upgrade() {
                    pane.feed
                        .input(Input::SearchEdit(TermEdit::Remove { token }));
                }
            });
            said.append(&remove);
            said.add_css_class("focus-results-chip");
            if chip.focused {
                said.add_css_class("focus-results-chip-focused");
            }
            self.chips.append(&said);
        }
        if !query.words.is_empty() {
            let typed = gtk::Label::new(Some(&query.words));
            typed.add_css_class("focus-results-words");
            self.chips.append(&typed);
        }
        clear(&self.query_hint);
        if let Some(key) = &query.hint_key {
            self.query_hint.append(&keyhint::cap(key));
        }
        let hint = gtk::Label::new(Some(&query.hint));
        hint.add_css_class("dim-label");
        self.query_hint.append(&hint);
        if query.ringed {
            self.chips
                .parent()
                .inspect(|field| field.add_css_class("focus-results-query-ringed"));
        } else {
            self.chips
                .parent()
                .inspect(|field| field.remove_css_class("focus-results-query-ringed"));
        }

        // The popover hangs from a button about to go: off it first.
        self.popover.unhang();
        clear(&self.buttons);
        let mut drawn = Vec::new();
        for button in &query.buttons {
            let control = gtk::Button::new();
            let label = if button.kind.has_popover() {
                format!("{} \u{25be}", button.label)
            } else {
                button.label.clone()
            };
            keyhint::dress(&control, &label, button.key.as_deref());
            control.add_css_class("focus-results-filter");
            if button.applied {
                control.add_css_class("focus-results-filter-applied");
            }
            if button.open {
                control.add_css_class("focus-results-filter-open");
            }
            control.set_focus_on_click(false);
            let kind = button.kind;
            let weak = self.me.borrow().clone();
            control.connect_clicked(move |_| {
                if let Some(pane) = weak.upgrade() {
                    pane.filter_clicked(kind);
                }
            });
            self.buttons.append(&control);
            drawn.push((kind, control));
        }
        if let Some(kind) = self.popover.kind()
            && let Some((_, button)) = drawn.iter().find(|(drawn, _)| *drawn == kind)
        {
            self.popover.rehang(button.upcast_ref());
        }
        self.filter_buttons.replace(drawn);
    }

    pub fn show_relaxations(&self, view: Option<&postio_focus::NoResultsView>) {
        match view {
            Some(view) => {
                self.nothing.show(view);
                self.found_nothing.set(true);
                if !self.showing_files() && !self.showing_people() {
                    self.switch_body(NOTHING);
                }
            }
            None => {
                self.found_nothing.set(false);
                if self.body.visible_child_name().as_deref() == Some(NOTHING) {
                    self.switch_body(ROWS);
                }
            }
        }
    }

    /// A hidden page keeps no keyboard, and GTK would hand it to the top bar's first button.
    fn switch_body(&self, page: &str) {
        if self.body.visible_child_name().as_deref() == Some(page) {
            return;
        }
        let had = self
            .root
            .root()
            .and_then(|root| root.focus())
            .is_some_and(|focus| focus.is_ancestor(&self.body));
        self.body.set_visible_child_name(page);
        self.timeline_line.set_visible(page != NOTHING);
        if had {
            self.focus();
        }
    }

    fn dress_head(&self) {
        let line = gtk::Box::new(gtk::Orientation::Horizontal, S1);
        line.append(&gtk::Image::from_icon_name("go-previous-symbolic"));
        line.append(&gtk::Label::new(Some(words::BACK_TO_INBOX)));
        if let Some(key) = self.feed.key(CommandId::Back) {
            line.append(&keyhint::cap(&key));
        }
        self.back.set_child(Some(&line));
        keyhint::dress(
            &self.save_button,
            words::SAVE_SEARCH,
            self.feed.key(CommandId::SaveSearch).as_deref(),
        );
    }

    pub fn show_save(&self, view: Option<&postio_focus::SaveView>) {
        match view {
            Some(view) => self.save_popover.show(
                view,
                &self.save_button,
                self.feed.key(CommandId::Back).as_deref(),
            ),
            None => self.save_popover.close(),
        }
    }

    pub fn save_popover(&self) -> &crate::save_popover::SavePopover {
        &self.save_popover
    }

    pub fn nothing(&self) -> &crate::results_files::NoResultsPage {
        &self.nothing
    }

    pub fn showing_nothing(&self) -> bool {
        self.body.visible_child_name().as_deref() == Some(NOTHING)
    }

    pub fn showing_people(&self) -> bool {
        self.body.visible_child_name().as_deref() == Some(PEOPLE)
    }

    pub fn people(&self) -> &crate::results_files::PeopleList {
        &self.people
    }

    pub fn showing_files(&self) -> bool {
        self.body.visible_child_name().as_deref() == Some(FILES)
    }

    pub fn show_quick_look(
        &self,
        view: Option<&postio_focus::QuickLookView>,
        commands: &[CommandId],
    ) {
        self.quick_look.show(view, commands);
    }

    /// Whether it took it; elsewhere the arrows are the list's keys.
    pub fn grid_arrow(&self, key: gtk::gdk::Key) -> bool {
        use postio_ui::search_view::GridArrow;
        if !self.showing_files() {
            return false;
        }
        let arrow = match key {
            gtk::gdk::Key::Left => GridArrow::Left,
            gtk::gdk::Key::Right => GridArrow::Right,
            gtk::gdk::Key::Up => GridArrow::Up,
            gtk::gdk::Key::Down => GridArrow::Down,
            _ => return false,
        };
        self.files.step(arrow, self.cursor.get());
        true
    }

    pub fn files(&self) -> &crate::results_files::FilesGrid {
        &self.files
    }

    pub fn quick_look(&self) -> &crate::results_files::QuickLookPanel {
        &self.quick_look
    }

    pub fn show_popover(&self, view: Option<&postio_focus::PopoverView>) {
        let button = view.and_then(|view| {
            self.filter_buttons
                .borrow()
                .iter()
                .find(|(kind, _)| *kind == view.kind)
                .map(|(_, button)| button.clone().upcast::<gtk::Widget>())
        });
        self.popover.show(view, button.as_ref());
    }

    pub fn popover(&self) -> &crate::search_popover::FilterPopover {
        &self.popover
    }

    pub fn timeline(&self) -> &crate::search_popover::MonthChart {
        &self.timeline
    }

    fn filter_clicked(&self, kind: FilterKind) {
        match kind.toggle_term() {
            Some((field, value)) => self.feed.input(Input::SearchEdit(TermEdit::Toggle {
                field: field.to_owned(),
                value: value.to_owned(),
            })),
            None => self.feed.input(Input::SearchPopover(kind)),
        }
    }

    fn show_tabs(&self, view: &ResultsView) {
        clear(&self.tabs);
        for tab in &view.tabs {
            let said = gtk::Box::new(gtk::Orientation::Horizontal, S1);
            said.append(&gtk::Label::new(Some(&tab.label)));
            let count = gtk::Label::new(Some(&tab.count));
            count.add_css_class("dim-label");
            said.append(&count);
            if let Some(key) = &tab.key {
                said.append(&keyhint::cap(key));
            }
            let button = gtk::Button::new();
            button.set_child(Some(&said));
            button.add_css_class("flat");
            button.add_css_class("focus-results-tab");
            if tab.selected {
                button.add_css_class("focus-results-tab-selected");
            }
            button.set_focus_on_click(false);
            let which = tab.tab;
            let weak = self.me.borrow().clone();
            button.connect_clicked(move |_| {
                if let Some(pane) = weak.upgrade() {
                    pane.feed.input(Input::ResultsTab(which));
                }
            });
            self.tabs.append(&button);
        }
    }

    fn draw_item(&self, at: u32, item: &gtk::ListItem) -> Option<gtk::Widget> {
        match self.items.item_at(at)? {
            Item::Header(index) => {
                let view = self.view.borrow();
                let group = view.as_ref()?.groups.get(index)?;
                item.set_activatable(false);
                // A heading is no place for the keyboard: it would wear a ring beside the result's.
                item.set_focusable(false);
                item.set_accessible_label(&group.accessible);
                Some(header(group).upcast())
            }
            Item::Row(position) => {
                item.set_activatable(true);
                item.set_focusable(true);
                let Some(row) = self.feed.result_row(position) else {
                    self.ask(position);
                    let quiet = gtk::Box::new(gtk::Orientation::Horizontal, 0);
                    quiet.add_css_class("focus-results-row");
                    return Some(quiet.upcast());
                };
                item.set_accessible_label(&row.accessible);
                let widget = self.row(position, &row);
                if self.cursor.get() == Some(position) && self.child_focus.get().is_none() {
                    widget.add_css_class("focus-results-cursor");
                }
                Some(widget.upcast())
            }
            Item::Child(index) => {
                let splice = self.splice.borrow();
                let child = splice.as_ref()?.children.get(index as usize)?;
                let gap = matches!(child, words::ResultChild::Gap { .. });
                item.set_activatable(!gap);
                item.set_focusable(!gap);
                let widget = self.child(index, child);
                if self.child_focus.get() == Some(index) {
                    widget.add_css_class("focus-results-cursor");
                }
                Some(widget.upcast())
            }
        }
    }

    /// Once, after the list has drawn: the answer may land at once, and the list must not change
    /// while it is binding.
    fn ask(&self, position: u64) {
        if !self.asked.borrow_mut().insert(position) {
            return;
        }
        let weak = self.me.borrow().clone();
        glib::idle_add_local_once(move || {
            if let Some(pane) = weak.upgrade() {
                pane.feed.want_result(position);
            }
        });
    }

    fn row(&self, position: u64, row: &ResultRow) -> gtk::Box {
        let line = gtk::Box::new(gtk::Orientation::Horizontal, S3);
        line.add_css_class("focus-results-row");
        if row.top_hit {
            line.add_css_class("focus-results-top-hit");
        }
        if row.checked {
            line.add_css_class("focus-results-checked");
        }
        let gutter = gtk::Label::new(Some(if row.checked {
            "\u{2611}"
        } else if row.unread {
            "\u{25cf}"
        } else {
            ""
        }));
        gutter.add_css_class("focus-results-gutter");
        if row.unread && !row.checked {
            gutter.add_css_class("focus-results-dot");
        }
        if row.checked {
            gutter.add_css_class("focus-results-check");
        }
        line.append(&gutter);

        let sender = gtk::Box::new(gtk::Orientation::Vertical, 0);
        sender.add_css_class("focus-results-sender");
        let name = gtk::Label::new(Some(&row.sender));
        name.set_xalign(0.0);
        name.set_ellipsize(pango::EllipsizeMode::End);
        if row.unread {
            name.add_css_class("focus-results-unread");
        }
        sender.append(&name);
        if let Some(reason) = &row.reason {
            let reason = gtk::Label::new(Some(reason));
            reason.add_css_class("focus-results-reason");
            reason.set_xalign(0.0);
            reason.set_ellipsize(pango::EllipsizeMode::End);
            sender.append(&reason);
        }
        line.append(&sender);

        let content = gtk::Box::new(gtk::Orientation::Vertical, S1);
        content.set_hexpand(true);
        let top = gtk::Box::new(gtk::Orientation::Horizontal, S2);
        let subject = crate::dropdown::runs_label(&row.subject, false);
        subject.add_css_class("focus-results-subject");
        if row.unread {
            subject.add_css_class("focus-results-unread");
        }
        top.append(&subject);
        for pill in &row.labels {
            let label = gtk::Label::new(Some(&pill.name));
            label.add_css_class("focus-results-label");
            top.append(&label);
        }
        if row.attachments {
            let clip = gtk::Image::from_icon_name("mail-attachment-symbolic");
            clip.add_css_class("focus-results-clip");
            clip.set_accessible_role(gtk::AccessibleRole::Presentation);
            top.append(&clip);
        }
        if let Some(count) = &row.count_badge {
            let badge = gtk::Label::new(Some(count));
            badge.add_css_class("focus-results-badge");
            top.append(&badge);
        }
        content.append(&top);
        let bottom = gtk::Box::new(gtk::Orientation::Horizontal, S2);
        let tag = gtk::Label::new(Some(&row.source_tag));
        tag.add_css_class("focus-results-source");
        if row.source_is_file {
            tag.add_css_class("focus-results-source-file");
        }
        bottom.append(&tag);
        let passage = match &row.summary {
            Some(summary) => crate::dropdown::runs_label(&[plain_run(summary)], true),
            None => crate::dropdown::runs_label(&row.passage, true),
        };
        passage.add_css_class("focus-results-passage");
        passage.set_hexpand(true);
        bottom.append(&passage);
        content.append(&bottom);
        line.append(&content);

        let folder = gtk::Label::new(Some(&row.folder));
        folder.add_css_class("focus-results-folder");
        folder.set_xalign(1.0);
        folder.set_ellipsize(pango::EllipsizeMode::End);
        line.append(&folder);
        let date = gtk::Label::new(Some(&row.date));
        date.add_css_class("focus-results-date");
        if row.unread {
            date.add_css_class("focus-results-unread");
        }
        date.set_xalign(1.0);
        line.append(&date);

        let mut column = line.first_child();
        while let Some(widget) = column {
            widget.set_valign(gtk::Align::Center);
            column = widget.next_sibling();
        }
        let click = gtk::GestureClick::new();
        let weak = self.me.borrow().clone();
        click.connect_pressed(move |_, presses, _, _| {
            if let Some(pane) = weak.upgrade() {
                pane.feed.input(Input::ResultsPoint(position));
                if presses == 2 {
                    (pane.act)(CommandId::OpenMessage);
                }
            }
        });
        line.add_controller(click);
        line
    }

    fn child(&self, index: u32, child: &words::ResultChild) -> gtk::Box {
        let line = gtk::Box::new(gtk::Orientation::Horizontal, S3);
        line.add_css_class("focus-results-child");
        match child {
            words::ResultChild::Match {
                tag,
                passage,
                sender,
                initials,
                time,
                ..
            } => {
                let avatar = gtk::Label::new(Some(initials));
                avatar.add_css_class("focus-thread-avatar");
                avatar.set_valign(gtk::Align::Center);
                line.append(&avatar);
                let who = gtk::Label::new(Some(sender));
                who.add_css_class("focus-results-child-sender");
                who.set_width_chars(14);
                who.set_max_width_chars(14);
                who.set_ellipsize(gtk::pango::EllipsizeMode::End);
                who.set_xalign(0.0);
                line.append(&who);
                let tag = gtk::Label::new(Some(tag));
                tag.add_css_class("focus-results-source");
                tag.set_width_chars(14);
                tag.set_xalign(0.0);
                line.append(&tag);
                let runs = passage.as_ref().map(passage_runs).unwrap_or_default();
                let words = crate::dropdown::runs_label(&runs, true);
                words.add_css_class("focus-results-passage");
                words.set_hexpand(true);
                line.append(&words);
                let when = gtk::Label::new(Some(time));
                when.add_css_class("dim-label");
                when.add_css_class("focus-results-child-time");
                when.set_xalign(1.0);
                line.append(&when);
            }
            gap @ words::ResultChild::Gap { .. } => {
                line.add_css_class("focus-results-gap");
                line.set_size_request(-1, 30);
                let said = gtk::Label::new(Some(&words::gap_line(gap)));
                said.add_css_class("dim-label");
                said.set_xalign(0.0);
                said.set_hexpand(true);
                line.append(&said);
            }
        }
        let _ = index;
        line
    }

    /// Once the redraws already said have been taken: the ring is drawn by replacing its rows, and
    /// a `scroll_to` asked before a replacement
    /// of the row it scrolls to was dropped, leaving the ring below the view after a few `j`s.
    fn reveal_cursor(&self) {
        let weak = self.me.borrow().clone();
        glib::idle_add_local_once(move || {
            if let Some(pane) = weak.upgrade() {
                pane.reveal_cursor_now();
            }
        });
    }

    fn reveal_cursor_now(&self) {
        let Some(at) = self
            .cursor
            .get()
            .and_then(|position| self.items.position_of(position))
        else {
            return;
        };
        let header_above = at
            .checked_sub(1)
            .filter(|above| matches!(self.items.item_at(*above), Some(Item::Header(_))));
        // The first row shows the list from its top: the first group's heading ("Top hits") is
        // never left just above the view.
        let from_the_top = header_above == Some(0);
        if from_the_top {
            to_the_top(&self.list);
        } else if let Some(header) = header_above {
            self.list
                .scroll_to(header, gtk::ListScrollFlags::NONE, None);
        }
        if !from_the_top {
            self.list.scroll_to(at, gtk::ListScrollFlags::NONE, None);
        }
        // The list places a row it has not drawn by an estimate of row heights, which a 72 row
        // after 40 ones outruns: once drawn, the ringed row is brought the rest of the way in.
        let frames = std::cell::Cell::new(0);
        let settled = std::cell::Cell::new(0);
        self.list.add_tick_callback(move |list, _| {
            frames.set(frames.get() + 1);
            // For some four seconds of frames: a row drawn late moves the ones below it again, and
            // a slow machine draws late.
            if from_the_top {
                to_the_top(list);
            }
            // In view for three frames running: one frame's bounds can be from before a layout that
            // moves the row again.
            if nudge_into_view(list) {
                settled.set(settled.get() + 1);
            } else {
                settled.set(0);
            }
            if settled.get() >= 3 || frames.get() >= 240 {
                return glib::ControlFlow::Break;
            }
            // The list's own estimate may end short of the row, where no nudge can scroll: asked
            // again, it measures with the rows drawn since.
            list.scroll_to(at, gtk::ListScrollFlags::NONE, None);
            glib::ControlFlow::Continue
        });
    }

    pub fn subjects(&self) -> Vec<String> {
        let total = self.view.borrow().as_ref().map_or(0, |view| view.rows);
        (0..total)
            .filter_map(|position| self.feed.result_row(position))
            .map(|row| row.subject.iter().map(|run| run.text.as_str()).collect())
            .collect()
    }

    pub fn groups(&self) -> Vec<String> {
        self.view
            .borrow()
            .as_ref()
            .map(|view| {
                view.groups
                    .iter()
                    .map(|group| group.title.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn sort_said(&self) -> String {
        self.sort
            .label()
            .map(|label| label.to_string())
            .unwrap_or_default()
    }

    pub fn list(&self) -> &gtk::ListView {
        &self.list
    }

    pub fn show_scope(&self, view: &postio_ui::scope::FooterView) {
        self.scope_footer.set(view);
        self.set_window_open(self.window_open.get());
    }

    /// The results' keys, Escape "back to inbox" among them, are not the window's (#2193).
    pub fn set_window_open(&self, open: bool) {
        self.window_open.set(open);
        let said = !self.scope_footer.title().is_empty();
        self.foot.set_visible(!open);
        self.scope_footer.widget().set_visible(open && said);
    }

    pub fn window_footer(&self) -> Option<Rc<crate::scope_footer::ScopeFooter>> {
        self.scope_footer
            .widget()
            .is_visible()
            .then(|| Rc::clone(&self.scope_footer))
    }

    pub fn count_line(&self) -> String {
        self.count_line.text().to_string()
    }

    pub fn top_reason(&self) -> Option<String> {
        self.feed.result_row(0)?.reason
    }

    pub fn cursor(&self) -> Option<u64> {
        self.cursor.get()
    }

    pub fn chips(&self) -> Vec<String> {
        let mut chips = Vec::new();
        let mut child = self.chips.first_child();
        while let Some(widget) = child {
            if widget.has_css_class("focus-results-chip") {
                let mut said = String::new();
                let mut part = widget.first_child();
                while let Some(label) = part {
                    if let Some(label) = label.downcast_ref::<gtk::Label>() {
                        said.push_str(&label.text());
                    }
                    part = label.next_sibling();
                }
                chips.push(said);
            }
            child = widget.next_sibling();
        }
        chips
    }

    pub fn remove_chip(&self, index: usize) -> bool {
        let mut child = self.chips.first_child();
        let mut at = 0;
        while let Some(widget) = child {
            if widget.has_css_class("focus-results-chip") {
                if at == index {
                    let mut part = widget.first_child();
                    while let Some(control) = part {
                        if let Some(button) = control.downcast_ref::<gtk::Button>() {
                            button.emit_clicked();
                            return true;
                        }
                        part = control.next_sibling();
                    }
                    return false;
                }
                at += 1;
            }
            child = widget.next_sibling();
        }
        false
    }

    pub fn tab_said(&self) -> Option<String> {
        let mut child = self.tabs.first_child();
        while let Some(widget) = child {
            if widget.has_css_class("focus-results-tab-selected") {
                return widget
                    .downcast_ref::<gtk::Button>()
                    .and_then(|button| button.child())
                    .and_then(|said| said.first_child())
                    .and_downcast::<gtk::Label>()
                    .map(|label| label.text().to_string());
            }
            child = widget.next_sibling();
        }
        None
    }

    pub fn click_tab(&self, text: &str) -> bool {
        let mut child = self.tabs.first_child();
        while let Some(widget) = child {
            if let Some(button) = widget.downcast_ref::<gtk::Button>()
                && button
                    .child()
                    .and_then(|said| said.first_child())
                    .and_downcast::<gtk::Label>()
                    .is_some_and(|label| label.text().starts_with(text))
            {
                button.emit_clicked();
                return true;
            }
            child = widget.next_sibling();
        }
        false
    }

    pub fn click_filter(&self, text: &str) -> bool {
        let mut child = self.buttons.first_child();
        while let Some(widget) = child {
            if let Some(button) = widget.downcast_ref::<gtk::Button>()
                && button_words(button).starts_with(text)
            {
                button.emit_clicked();
                return true;
            }
            child = widget.next_sibling();
        }
        false
    }

    pub fn filters(&self) -> Vec<(String, bool)> {
        let mut filters = Vec::new();
        let mut child = self.buttons.first_child();
        while let Some(widget) = child {
            if let Some(button) = widget.downcast_ref::<gtk::Button>() {
                filters.push((
                    button_words(button),
                    button.has_css_class("focus-results-filter-applied"),
                ));
            }
            child = widget.next_sibling();
        }
        filters
    }

    pub fn focus(&self) {
        if self.showing_files() {
            self.files.focus();
            return;
        }
        if self.showing_nothing() {
            self.nothing.focus();
            return;
        }
        if self.showing_people() {
            self.people.focus();
            return;
        }
        // On the ringed result, as the inbox's keyboard is on its cursor's row: the list's own
        // focus would land on its first item.
        match self
            .cursor
            .get()
            .and_then(|position| self.items.position_of(position))
        {
            // The list takes the keyboard first: scrolling to an item
            // focuses it only while the keyboard is already within.
            Some(at) => {
                self.list.grab_focus();
                self.list.scroll_to(at, gtk::ListScrollFlags::FOCUS, None);
            }
            None => {
                self.list.grab_focus();
            }
        }
    }
}

fn button_words(button: &gtk::Button) -> String {
    if let Some(label) = button.label() {
        return label.to_string();
    }
    let mut stack = vec![button.clone().upcast::<gtk::Widget>()];
    while let Some(widget) = stack.pop() {
        if let Some(label) = widget.downcast_ref::<gtk::Label>() {
            return label.text().to_string();
        }
        let mut child = widget.last_child();
        while let Some(next) = child {
            child = next.prev_sibling();
            stack.push(next);
        }
    }
    String::new()
}

fn to_the_top(list: &gtk::ListView) {
    if let Some(scroller) = list
        .ancestor(gtk::ScrolledWindow::static_type())
        .and_downcast::<gtk::ScrolledWindow>()
    {
        scroller.vadjustment().set_value(0.0);
    }
}

fn nudge_into_view(list: &gtk::ListView) -> bool {
    let Some(scroller) = list
        .ancestor(gtk::ScrolledWindow::static_type())
        .and_downcast::<gtk::ScrolledWindow>()
    else {
        return true;
    };
    let mut stack = vec![list.clone().upcast::<gtk::Widget>()];
    let ringed = std::iter::from_fn(|| {
        let widget = stack.pop()?;
        let mut child = widget.first_child();
        while let Some(next) = child {
            child = next.next_sibling();
            stack.push(next);
        }
        Some(widget)
    })
    .find(|widget| widget.is_mapped() && widget.has_css_class("focus-results-cursor"));
    let Some(bounds) = ringed.and_then(|ringed| ringed.compute_bounds(&scroller)) else {
        return false;
    };
    let height = scroller.height() as f32;
    let adjustment = scroller.vadjustment();
    let below = bounds.y() + bounds.height() - height;
    if bounds.y() < -0.5 {
        adjustment.set_value(adjustment.value() + f64::from(bounds.y()));
        false
    } else if below > 0.5 {
        adjustment.set_value(adjustment.value() + f64::from(below));
        false
    } else {
        true
    }
}

fn header(group: &ResultGroup) -> gtk::Box {
    let line = gtk::Box::new(gtk::Orientation::Horizontal, S2);
    line.add_css_class("focus-results-header");
    let title = gtk::Label::new(Some(&group.title));
    title.add_css_class("focus-results-header-title");
    line.append(&title);
    if !group.count.is_empty() {
        let count = gtk::Label::new(Some(&group.count));
        count.add_css_class("dim-label");
        line.append(&count);
    }
    if let Some(note) = &group.note {
        let note = gtk::Label::new(Some(note));
        note.add_css_class("dim-label");
        note.add_css_class("focus-results-header-note");
        line.append(&note);
    }
    line
}

fn hint_line(key: &str, label: &str) -> gtk::Box {
    let line = gtk::Box::new(gtk::Orientation::Horizontal, S1);
    line.append(&keyhint::cap(key));
    let words = gtk::Label::new(Some(label));
    words.add_css_class("dim-label");
    words.set_single_line_mode(true);
    line.append(&words);
    line
}

fn clear(container: &gtk::Box) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
}

fn passage_runs(passage: &postio_search::results::Passage) -> Vec<Run> {
    let mut runs = Vec::new();
    if passage.elided_start {
        runs.push(plain_run("\u{2026} "));
    }
    let mut at = 0;
    for range in &passage.ranges {
        if range.start > at {
            runs.push(plain_run(&passage.text[at..range.start]));
        }
        runs.push(marked_run(&passage.text[range.clone()]));
        at = range.end;
    }
    if at < passage.text.len() {
        runs.push(plain_run(&passage.text[at..]));
    }
    if passage.elided_end {
        runs.push(plain_run(" \u{2026}"));
    }
    runs
}

fn plain_run(text: &str) -> Run {
    Run {
        text: text.to_owned(),
        highlighted: false,
        style: RunStyle::Plain,
    }
}

fn marked_run(text: &str) -> Run {
    Run {
        text: text.to_owned(),
        highlighted: true,
        style: RunStyle::Plain,
    }
}
