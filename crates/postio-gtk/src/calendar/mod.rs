//! Calendar's Agenda in the list's language: day headings, one row per occurrence, the open event
//! as a card over the dimmed agenda, and the footer of its keys. It draws what the controller says.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use gtk::prelude::*;
use postio_core::CommandId;
use postio_focus::{CalendarView, Input};
use postio_ui::calendar::{AgendaRow, EventView, OccurrenceRow};
use postio_widgets::widgets::keyhint::KeyLine;
use postio_widgets::widgets::space::{S1, S2, S3};

use crate::list::Feed;
use crate::position_list::{self, PositionList};

type Act = Rc<dyn Fn(CommandId)>;

pub struct AgendaView {
    root: gtk::Box,
    shell: gtk::Overlay,
    feed: Feed,
    act: Act,
    model: PositionList,
    selection: gtk::SingleSelection,
    list: gtk::ListView,
    empty: gtk::Label,
    filter: gtk::Entry,
    hint: gtk::Label,
    typing_echo: Cell<bool>,
    footer: KeyLine,
    card: gtk::Box,
    rows: RefCell<Vec<AgendaRow>>,
    open: RefCell<Option<EventView>>,
    open_keys: RefCell<Vec<postio_ui::hints::Hint>>,
    search_key: gtk::Box,
    cursor: Cell<Option<usize>>,
    placing: Cell<bool>,
    me: RefCell<Weak<AgendaView>>,
}

impl AgendaView {
    pub fn new(feed: Feed, act: impl Fn(CommandId) + 'static) -> Rc<Self> {
        let title = gtk::Label::new(Some("Calendar"));
        title.add_css_class("focus-contacts-title");
        let view_name = gtk::Label::new(Some("Agenda"));
        view_name.add_css_class("dim-label");
        let strip = gtk::Box::new(gtk::Orientation::Horizontal, S3);
        strip.add_css_class("focus-contacts-strip");
        for label in [&title, &view_name] {
            label.set_valign(gtk::Align::Center);
            strip.append(label);
        }
        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        strip.append(&spacer);
        let filter = gtk::Entry::new();
        filter.add_css_class("focus-contacts-filter");
        filter.set_primary_icon_name(Some("system-search-symbolic"));
        filter.set_placeholder_text(Some("Search events"));
        filter.set_width_chars(22);
        filter.set_valign(gtk::Align::Center);
        strip.append(&filter);
        let search_key = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        search_key.set_valign(gtk::Align::Center);
        strip.append(&search_key);
        let hint = gtk::Label::new(None);
        hint.add_css_class("dim-label");
        hint.set_xalign(0.0);
        hint.set_visible(false);

        let model = PositionList::default();
        let selection = gtk::SingleSelection::new(Some(model.clone()));
        selection.set_autoselect(false);
        selection.set_can_unselect(true);
        let factory = gtk::SignalListItemFactory::new();
        let list = gtk::ListView::new(Some(selection.clone()), Some(factory.clone()));
        list.add_css_class("focus-calendar-list");
        list.set_single_click_activate(false);
        list.set_accessible_role(gtk::AccessibleRole::List);
        list.update_property(&[gtk::accessible::Property::Label("Agenda")]);
        let scrolled = gtk::ScrolledWindow::builder()
            .child(&list)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .build();

        let empty = gtk::Label::new(None);
        empty.add_css_class("focus-empty-heading");
        empty.set_vexpand(true);
        empty.set_visible(false);

        let footer = KeyLine::new("focus-contacts-footer-hints");
        let foot = gtk::Box::new(gtk::Orientation::Horizontal, S2);
        foot.add_css_class("focus-contacts-footer");
        footer.widget().set_hexpand(true);
        foot.append(footer.widget());

        let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
        body.append(&strip);
        body.append(&hint);
        body.append(&scrolled);
        body.append(&empty);
        body.append(&foot);

        let card = gtk::Box::new(gtk::Orientation::Vertical, S2);
        card.add_css_class("focus-calendar-card");
        card.set_halign(gtk::Align::Center);
        card.set_valign(gtk::Align::Center);
        card.set_visible(false);
        card.set_accessible_role(gtk::AccessibleRole::Dialog);

        let shell = gtk::Overlay::new();
        let scrim = gtk::Box::new(gtk::Orientation::Vertical, 0);
        scrim.add_css_class("focus-calendar-scrim");
        scrim.set_can_target(false);
        card.bind_property("visible", &scrim, "visible")
            .sync_create()
            .build();
        shell.set_child(Some(&body));
        shell.add_overlay(&scrim);
        shell.add_overlay(&card);
        shell.set_vexpand(true);
        shell.set_hexpand(true);

        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.add_css_class("focus-contacts-root");
        root.set_widget_name("focus-calendar");
        root.append(&shell);

        let view = Rc::new(AgendaView {
            root,
            shell,
            feed,
            act: Rc::new(act),
            model,
            selection,
            list,
            empty,
            filter,
            hint,
            typing_echo: Cell::new(false),
            footer,
            card,
            rows: RefCell::new(Vec::new()),
            open: RefCell::new(None),
            open_keys: RefCell::new(Vec::new()),
            search_key,
            cursor: Cell::new(None),
            placing: Cell::new(false),
            me: RefCell::new(Weak::new()),
        });
        view.me.replace(Rc::downgrade(&view));
        view.connect(&factory);
        view
    }

    fn weak(&self) -> Weak<AgendaView> {
        self.me.borrow().clone()
    }

    fn connect(self: &Rc<Self>, factory: &gtk::SignalListItemFactory) {
        let weak = self.weak();
        factory.connect_bind({
            let weak = weak.clone();
            move |_, item| {
                let Some(item) = item.downcast_ref::<gtk::ListItem>() else {
                    return;
                };
                let Some(view) = weak.upgrade() else {
                    return;
                };
                let position = item.item().as_ref().and_then(position_list::position_of);
                let rows = view.rows.borrow();
                let row = position.and_then(|at| rows.get(at as usize));
                item.set_selectable(matches!(row, Some(AgendaRow::Occurrence(_))));
                item.set_activatable(matches!(row, Some(AgendaRow::Occurrence(_))));
                if let Some(AgendaRow::Occurrence(row)) = row {
                    item.set_accessible_label(&row.accessible);
                }
                item.set_child(row.map(row_widget).as_ref());
            }
        });
        self.selection.connect_selected_notify({
            let weak = weak.clone();
            move |selection| {
                let Some(view) = weak.upgrade() else {
                    return;
                };
                if view.placing.get() {
                    return;
                }
                let at = selection.selected();
                if at != gtk::INVALID_LIST_POSITION {
                    view.feed.input(Input::CalendarPoint(at as usize));
                }
            }
        });
        self.filter.connect_changed({
            let weak = weak.clone();
            move |entry| {
                let Some(view) = weak.upgrade() else {
                    return;
                };
                if !view.typing_echo.get() {
                    view.feed
                        .input(Input::CalendarFilterTyped(entry.text().to_string()));
                }
            }
        });
        self.filter.connect_activate({
            let weak = weak.clone();
            move |_| {
                if let Some(view) = weak.upgrade() {
                    view.list.grab_focus();
                }
            }
        });
        self.list.connect_activate({
            let weak = weak.clone();
            move |_, _| {
                if let Some(view) = weak.upgrade() {
                    (view.act)(CommandId::OpenMessage);
                }
            }
        });
    }

    pub fn widget(&self) -> &gtk::Box {
        &self.root
    }

    pub fn overlay(&self) -> &gtk::Overlay {
        &self.shell
    }

    pub fn focus_list(&self) {
        self.list.grab_focus();
    }

    pub fn focus_filter(&self) {
        self.filter.grab_focus();
        self.filter.set_position(-1);
    }

    pub fn filter(&self) -> &gtk::Entry {
        &self.filter
    }

    pub fn cursor(&self) -> Option<usize> {
        self.cursor.get()
    }

    pub fn ringed(&self) -> Option<OccurrenceRow> {
        match self.rows.borrow().get(self.cursor.get()?) {
            Some(AgendaRow::Occurrence(row)) => Some(row.clone()),
            _ => None,
        }
    }

    pub fn open_event(&self) -> Option<EventView> {
        self.open.borrow().clone()
    }

    pub fn show_frame(&self, view: &CalendarView) {
        let before = self.rows.replace(view.rows.clone());
        if before != view.rows {
            self.model.set_total(view.rows.len() as u32);
        }
        self.empty.set_visible(view.empty.is_some());
        self.empty
            .set_text(view.empty.as_deref().unwrap_or_default());
        self.footer.set(&view.footer);
        self.open_keys.replace(view.open_keys.clone());
        while let Some(child) = self.search_key.first_child() {
            self.search_key.remove(&child);
        }
        if let Some(key) = &view.search_key {
            self.search_key
                .append(&postio_widgets::widgets::keyhint::cap(key));
        }
        if self.filter.text().as_str() != view.filter {
            self.typing_echo.set(true);
            self.filter.set_text(&view.filter);
            self.filter.set_position(-1);
            self.typing_echo.set(false);
        }
        self.hint.set_visible(view.hint.is_some());
        self.hint.set_text(view.hint.as_deref().unwrap_or_default());
        self.cursor.set(view.cursor);
        self.placing.set(true);
        match view.cursor {
            Some(at) => {
                self.selection.set_selected(at as u32);
                self.list
                    .scroll_to(at as u32, gtk::ListScrollFlags::NONE, None);
            }
            None => self.selection.set_selected(gtk::INVALID_LIST_POSITION),
        }
        self.placing.set(false);
        self.show_open(view.open.as_ref());
    }

    fn show_open(&self, open: Option<&EventView>) {
        if self.open.borrow().as_ref() == open {
            return;
        }
        self.open.replace(open.cloned());
        while let Some(child) = self.card.first_child() {
            self.card.remove(&child);
        }
        let Some(open) = open else {
            self.card.set_visible(false);
            match self.cursor.get() {
                Some(at) => self
                    .list
                    .scroll_to(at as u32, gtk::ListScrollFlags::FOCUS, None),
                None => {
                    self.list.grab_focus();
                }
            }
            return;
        };
        let title = gtk::Label::new(Some(&open.title));
        title.add_css_class("focus-calendar-card-title");
        title.set_xalign(0.0);
        title.set_wrap(true);
        title.set_hexpand(true);
        let close = postio_widgets::widgets::button::close_button();
        close.set_valign(gtk::Align::Start);
        let act = Rc::clone(&self.act);
        close.connect_clicked(move |_| act(CommandId::Back));
        let header = gtk::Box::new(gtk::Orientation::Horizontal, S2);
        header.append(&title);
        header.append(&close);
        self.card.append(&header);
        self.card
            .update_property(&[gtk::accessible::Property::Label(&open.title)]);
        let mut lines = vec![open.when.clone()];
        lines.extend(open.zone.clone());
        lines.extend(open.repeat.clone());
        if !open.location.is_empty() {
            lines.push(open.location.clone());
        }
        lines.push(open.calendar.clone());
        lines.extend(open.people.iter().cloned());
        lines.extend(open.reminders.iter().cloned());
        for line in lines {
            self.card
                .append(&line_label(&line, "focus-calendar-card-line"));
        }
        if !open.notes.is_empty() {
            let notes = line_label(&open.notes, "focus-calendar-card-notes");
            notes.set_selectable(true);
            self.card.append(&notes);
        }
        let keys = KeyLine::new("focus-contacts-footer-hints");
        keys.set(self.open_keys.borrow().iter());
        keys.widget().set_margin_top(S2);
        self.card.append(keys.widget());
        self.card.set_visible(true);
        self.card.set_focusable(true);
        self.card.grab_focus();
    }
}

fn line_label(text: &str, class: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.add_css_class(class);
    label.set_xalign(0.0);
    label.set_wrap(true);
    label
}

fn row_widget(row: &AgendaRow) -> gtk::Widget {
    match row {
        AgendaRow::Day { label, .. } => {
            let heading = gtk::Label::new(Some(label));
            heading.add_css_class("focus-day-heading");
            heading.add_css_class("focus-calendar-day");
            heading.set_xalign(0.0);
            heading.set_accessible_role(gtk::AccessibleRole::Heading);
            heading.upcast()
        }
        AgendaRow::Occurrence(row) => {
            let line = gtk::Box::new(gtk::Orientation::Horizontal, S2);
            line.add_css_class("focus-calendar-row");
            if row.past {
                line.add_css_class("focus-calendar-past");
            }
            let mark = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            mark.add_css_class("focus-calendar-mark");
            mark.set_accessible_role(gtk::AccessibleRole::Presentation);
            let time = gtk::Label::new(Some(&row.time));
            time.add_css_class("focus-calendar-time");
            time.set_xalign(0.0);
            let title = gtk::Label::new(Some(&row.title));
            title.add_css_class("focus-calendar-title");
            title.set_xalign(0.0);
            title.set_ellipsize(gtk::pango::EllipsizeMode::End);
            line.append(&mark);
            line.append(&time);
            line.append(&title);
            if !row.location.is_empty() {
                let place = gtk::Label::new(Some(&row.location));
                place.add_css_class("dim-label");
                place.set_ellipsize(gtk::pango::EllipsizeMode::End);
                line.append(&place);
            }
            let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            spacer.set_hexpand(true);
            line.append(&spacer);
            if let Some(chip) = &row.chip {
                let chip = gtk::Label::new(Some(&chip.label()));
                chip.add_css_class("focus-calendar-chip");
                line.append(&chip);
            }
            if row.queued {
                let waiting = gtk::Image::from_icon_name("emblem-synchronizing-symbolic");
                waiting.set_tooltip_text(Some("Waiting to sync"));
                line.append(&waiting);
            }
            let calendar = gtk::Label::new(Some(&row.calendar_name));
            calendar.add_css_class("dim-label");
            calendar.add_css_class("focus-calendar-name");
            line.append(&calendar);
            line.set_margin_start(S1);
            line.upcast()
        }
    }
}
