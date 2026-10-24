//! The search results view (spec 010 step 3, design §3.3-3.5, screens 06
//! and 07): the page "Show all" opens in the inbox's place.
//!
//! It draws what the controller says and decides nothing, as the Mac's
//! `ResultsTable` does: the frame (`Intent::ShowResults`) -- the tabs, the
//! count, the sort, the groups and the footer -- the query's chips and
//! filter buttons (`Intent::Query`), and where the focus ring is
//! (`Intent::ResultsCursor`). The rows are read by position when the list
//! view draws them and never before, so a search of thousands costs what
//! one of a dozen does; a row whose page has not landed is asked for, and
//! `Intent::ResultsPage` says when to draw it again.
//!
//! A click is told to the controller and moves nothing here: the ring
//! moves when the controller says. Keys reach the controller through the
//! window's commands, as the inbox's do.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use gtk::gio;
use gtk::glib;
use gtk::pango;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use postio_core::CommandId;
use postio_focus::{Input, QueryView, ResultGroup, ResultRow, ResultsView, TermEdit};
use postio_search::results::ConversationOrder;
use postio_ui::search_view::{self as words, FilterKind};
use postio_widgets::widgets::keyhint;
use postio_widgets::widgets::space::{S1, S2, S3};

use crate::list::Feed;

/// What a list position shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Item {
    /// The header of `groups[index]`.
    Header(usize),
    /// The result at this position.
    Row(u64),
}

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct ResultsItems {
        /// Each group's header position, and its group's first result.
        pub starts: RefCell<Vec<(u32, u64)>>,
        pub total: Cell<u32>,
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
            self.total.get()
        }

        fn item(&self, position: u32) -> Option<glib::Object> {
            (position < self.total.get())
                .then(|| gtk::StringObject::new(&position.to_string()).upcast())
        }
    }
}

glib::wrapper! {
    /// The results list's shape: a header before each group's rows. Only
    /// the count and where each group starts are held.
    pub struct ResultsItems(ObjectSubclass<imp::ResultsItems>)
        @implements gio::ListModel;
}

impl Default for ResultsItems {
    fn default() -> Self {
        glib::Object::new()
    }
}

impl ResultsItems {
    /// The list is `groups` now: every row drawn is asked for again.
    fn set_groups(&self, groups: &[ResultGroup]) {
        let mut starts = Vec::with_capacity(groups.len());
        let mut at = 0u32;
        for group in groups {
            starts.push((at, group.first));
            at = at.saturating_add(1).saturating_add(group.rows as u32);
        }
        self.imp().starts.replace(starts);
        let before = self.imp().total.replace(at);
        self.items_changed(0, before, at);
    }

    /// What list position `at` shows.
    fn item_at(&self, at: u32) -> Option<Item> {
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

    /// The list position of the result at `position`.
    fn position_of(&self, position: u64) -> Option<u32> {
        let starts = self.imp().starts.borrow();
        let index = starts.iter().rposition(|(_, first)| *first <= position)?;
        let (start, first) = starts[index];
        let at = u64::from(start) + 1 + (position - first);
        u32::try_from(at)
            .ok()
            .filter(|at| *at < self.imp().total.get())
    }

    /// The results `first..first + count` changed: draw them again.
    fn rows_changed(&self, first: u64, count: u64) {
        for position in first..first.saturating_add(count) {
            if let Some(at) = self.position_of(position) {
                self.items_changed(at, 1, 1);
            }
        }
    }
}

/// The results view.
pub struct ResultsPane {
    root: gtk::Box,
    feed: Feed,
    act: Rc<dyn Fn(CommandId)>,
    count_line: gtk::Label,
    sub_line: gtk::Label,
    tabs: gtk::Box,
    sort: gtk::Button,
    chips: gtk::Box,
    query_hint: gtk::Box,
    buttons: gtk::Box,
    /// The filter buttons drawn, by kind: what a popover hangs from.
    filter_buttons: RefCell<Vec<(FilterKind, gtk::Button)>>,
    timeline: Rc<crate::search_popover::MonthChart>,
    timeline_hint: gtk::Label,
    popover: Rc<crate::search_popover::FilterPopover>,
    items: ResultsItems,
    list: gtk::ListView,
    hints: gtk::Box,
    footer: gtk::Label,
    view: RefCell<Option<ResultsView>>,
    cursor: Cell<Option<u64>>,
    /// Positions asked for and not yet landed, so a redraw does not ask
    /// again.
    asked: RefCell<std::collections::HashSet<u64>>,
    me: RefCell<Weak<ResultsPane>>,
}

impl ResultsPane {
    /// An empty results view over `feed`; `act` runs a command as its key
    /// would.
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

        // The timeline (design §3.3): a bar a month, the hint on its right.
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
        let list = gtk::ListView::new(Some(selection), Some(factory.clone()));
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
        root.append(&query);
        root.append(&buttons);
        root.append(&timeline_line);
        root.append(&scrolled);
        root.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        root.append(&foot);

        let popover_feed = feed.clone();
        let popover = crate::search_popover::FilterPopover::new(move |input| {
            popover_feed.input(input);
        });
        let pane = Rc::new(ResultsPane {
            root,
            feed,
            act: Rc::new(act),
            count_line,
            sub_line,
            tabs,
            sort,
            chips,
            query_hint,
            buttons,
            filter_buttons: RefCell::default(),
            timeline,
            timeline_hint,
            popover,
            items,
            list,
            hints,
            footer,
            view: RefCell::default(),
            cursor: Cell::new(None),
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

    /// The page the window puts in the inbox's place.
    pub fn widget(&self) -> &gtk::Box {
        &self.root
    }

    /// Draw the frame: the controller's `Intent::ShowResults`.
    pub fn show(&self, view: &ResultsView) {
        self.count_line.set_text(&view.count_line);
        self.sub_line.set_text(&view.sub_line);
        self.sub_line.set_visible(!view.sub_line.is_empty());
        self.show_tabs(view);
        self.sort.set_visible(view.sortable);
        self.timeline.show(&view.months);
        self.timeline.widget().set_visible(!view.months.is_empty());
        // Two lines, broken at the hint's first middle dot, as screen 06
        // draws it; then the key that steps a month while a range is set.
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
        clear(&self.hints);
        let bulk = view.selected > 0;
        for hint in if bulk { &view.bulk } else { &view.hints } {
            self.hints.append(&hint_line(&hint.key, &hint.label));
        }
        match (&view.select_all, bulk) {
            (Some(all), true) => self.footer.set_text(&format!("{} {}", all.key, all.label)),
            _ => self.footer.set_text(&view.footer),
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

    /// The results `first..first + count` landed or changed.
    pub fn rows_changed(&self, first: u64, count: u64) {
        {
            let mut asked = self.asked.borrow_mut();
            for position in first..first.saturating_add(count) {
                asked.remove(&position);
            }
        }
        self.items.rows_changed(first, count);
    }

    /// The focus ring is on the result at `position` now.
    pub fn show_cursor(&self, position: u64) {
        let was = self.cursor.replace(Some(position));
        if let Some(was) = was {
            self.items.rows_changed(was, 1);
        }
        self.items.rows_changed(position, 1);
        self.reveal_cursor();
    }

    /// Draw the query's chips and filter buttons: `Intent::Query`.
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
            control.set_label(&label);
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

    /// Draw the open filter popover, hung from its button, or close it:
    /// `Intent::Popover`.
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

    /// The filter popover.
    pub fn popover(&self) -> &crate::search_popover::FilterPopover {
        &self.popover
    }

    /// The timeline.
    pub fn timeline(&self) -> &crate::search_popover::MonthChart {
        &self.timeline
    }

    /// A filter button: a toggle sends its term, the rest open their
    /// popover (spec 010 FR-027).
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

    /// List position `at`, drawn.
    fn draw_item(&self, at: u32, item: &gtk::ListItem) -> Option<gtk::Widget> {
        match self.items.item_at(at)? {
            Item::Header(index) => {
                let view = self.view.borrow();
                let group = view.as_ref()?.groups.get(index)?;
                item.set_activatable(false);
                item.set_accessible_label(&group.accessible);
                Some(header(group).upcast())
            }
            Item::Row(position) => {
                item.set_activatable(true);
                let Some(row) = self.feed.result_row(position) else {
                    self.ask(position);
                    let quiet = gtk::Box::new(gtk::Orientation::Horizontal, 0);
                    quiet.add_css_class("focus-results-row");
                    return Some(quiet.upcast());
                };
                item.set_accessible_label(&row.accessible);
                let widget = self.row(position, &row);
                if self.cursor.get() == Some(position) {
                    widget.add_css_class("focus-results-cursor");
                }
                Some(widget.upcast())
            }
        }
    }

    /// Ask for the result at `position`, once, after the list has drawn:
    /// the answer may land at once, and the list must not change while it
    /// is binding.
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

    /// One result (design §3.4): the gutter, the sender with a top hit's
    /// reason, the subject line and the passage line, the folder, the
    /// date.
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
        let passage = crate::dropdown::runs_label(&row.passage, true);
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

        // Every column on the row's middle: the two-line columns and the
        // one-line ones alike.
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

    /// Scroll the ring's row into view, moving the list no further. The
    /// first row of a group brings its header with it.
    fn reveal_cursor(&self) {
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
        if let Some(header) = header_above {
            self.list
                .scroll_to(header, gtk::ListScrollFlags::NONE, None);
        }
        self.list.scroll_to(at, gtk::ListScrollFlags::NONE, None);
    }

    /// The subjects of the rows read so far, in order: what a test reads.
    pub fn subjects(&self) -> Vec<String> {
        let total = self.view.borrow().as_ref().map_or(0, |view| view.rows);
        (0..total)
            .filter_map(|position| self.feed.result_row(position))
            .map(|row| row.subject.iter().map(|run| run.text.as_str()).collect())
            .collect()
    }

    /// The groups' titles, in order.
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

    /// The count line: "48 conversations".
    pub fn count_line(&self) -> String {
        self.count_line.text().to_string()
    }

    /// The result the focus ring is on.
    pub fn cursor(&self) -> Option<u64> {
        self.cursor.get()
    }

    /// The chips' words, operator and value: `from:ada@example.com`.
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

    /// Click chip `index`'s ✕, as a person does.
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

    /// Click the filter button whose label starts with `text`.
    pub fn click_filter(&self, text: &str) -> bool {
        let mut child = self.buttons.first_child();
        while let Some(widget) = child {
            if let Some(button) = widget.downcast_ref::<gtk::Button>()
                && button.label().is_some_and(|label| label.starts_with(text))
            {
                button.emit_clicked();
                return true;
            }
            child = widget.next_sibling();
        }
        false
    }

    /// The filter buttons' labels, and whether each is applied.
    pub fn filters(&self) -> Vec<(String, bool)> {
        let mut filters = Vec::new();
        let mut child = self.buttons.first_child();
        while let Some(widget) = child {
            if let Some(button) = widget.downcast_ref::<gtk::Button>() {
                filters.push((
                    button
                        .label()
                        .map(|label| label.to_string())
                        .unwrap_or_default(),
                    button.has_css_class("focus-results-filter-applied"),
                ));
            }
            child = widget.next_sibling();
        }
        filters
    }

    /// Take the keyboard: the list, which hears the window's keys.
    pub fn focus(&self) {
        self.list.grab_focus();
    }
}

/// A group's header: its title, its count, its note.
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

/// A footer key: the cap, then what it does.
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
