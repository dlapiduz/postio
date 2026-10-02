//! The Filtered view (spec 007 US9, screen 21): a full view, not a dialog,
//! of what filing archived. Each row is one message with its reason; tabs
//! `1`-`7` narrow it to one reason; the focused row offers its restore;
//! and nothing here is deleted.
//!
//! It reads through the client's typed Filtered calls: the tabs' counts
//! once when it opens, and a page of fifty rows at a time as the person
//! scrolls -- never the whole of it.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;
use postio_client::Client;
use postio_client::protocol::FilteredRow;
use postio_core::{CommandId, Keymap};
use postio_model::MessageId;
use postio_ui::filtered;
use postio_widgets::widgets::keyhint::{self, KeyLine};
use postio_widgets::widgets::space::{S1, S3};

/// How many rows a page reads.
const PAGE: u32 = 50;

/// What a person asked of the view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FilteredAction {
    /// Go back to the inbox.
    Back,
    /// Open this message over the view.
    Open {
        /// The message.
        message: MessageId,
        /// Its subject, for the dialog's title.
        subject: String,
    },
    /// Restore this message and never filter its sender.
    Restore(MessageId),
    /// Ask whether to sweep the inbox (FR-118).
    Sweep,
}

type Handler = Rc<dyn Fn(FilteredAction)>;

/// The view. See the module.
pub struct FilteredView {
    client: Client,
    root: gtk::Box,
    tabs: gtk::Box,
    list: gtk::ListBox,
    footer: KeyLine,
    /// The sweep button's words and key.
    sweep_words: gtk::Box,
    keymap: RefCell<Keymap>,
    /// Which tab is showing, `0` for All.
    tab: Cell<usize>,
    /// Each tab's count, as last read.
    counts: Cell<[u32; 7]>,
    /// The rows listed, in order.
    rows: RefCell<Vec<FilteredRow>>,
    /// What each row of the list is: a day's heading, or the row at that
    /// index of `rows`.
    shown: RefCell<Vec<Option<usize>>>,
    /// Whether the last page read was full: there may be more.
    more: Cell<bool>,
    /// Moves with every re-read, so a page for an earlier one is dropped.
    generation: Cell<u64>,
    handler: RefCell<Option<Handler>>,
    me: RefCell<std::rc::Weak<FilteredView>>,
}

impl FilteredView {
    /// The view, reading through `client`, its keys from `keymap`.
    pub fn new(client: Client, keymap: &Keymap) -> Rc<Self> {
        let back = gtk::Button::new();
        back.add_css_class("flat");
        back.add_css_class("focus-filtered-back");
        let back_row = gtk::Box::new(gtk::Orientation::Horizontal, S1);
        back_row.append(&gtk::Image::from_icon_name("go-previous-symbolic"));
        back_row.append(&gtk::Label::new(Some("Inbox")));
        back.set_child(Some(&back_row));
        let title = gtk::Label::new(Some(filtered::TITLE));
        title.add_css_class("focus-filtered-title");
        let subtitle = gtk::Label::new(Some(filtered::SUBTITLE));
        subtitle.add_css_class("dim-label");
        subtitle.add_css_class("focus-filtered-subtitle");
        let titles = gtk::Box::new(gtk::Orientation::Vertical, 0);
        titles.append(&title);
        titles.append(&subtitle);
        // FR-118: filtering what is already in the inbox is a deliberate
        // command, and this is where filtering lives. It asks first.
        let sweep = gtk::Button::new();
        sweep.add_css_class("flat");
        sweep.add_css_class("focus-filtered-sweep");
        let sweep_words = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        sweep.set_child(Some(&sweep_words));
        let header = gtk::CenterBox::new();
        header.add_css_class("focus-filtered-header");
        header.set_start_widget(Some(&back));
        header.set_center_widget(Some(&titles));
        header.set_end_widget(Some(&sweep));

        let tabs = gtk::Box::new(gtk::Orientation::Horizontal, S1);
        tabs.add_css_class("focus-filtered-tabs");
        let note = gtk::Label::new(Some(filtered::NOTE));
        note.add_css_class("dim-label");
        note.add_css_class("focus-filtered-note");
        note.set_hexpand(true);
        note.set_xalign(1.0);
        note.set_ellipsize(gtk::pango::EllipsizeMode::End);
        let tab_row = gtk::Box::new(gtk::Orientation::Horizontal, S3);
        tab_row.add_css_class("focus-filtered-tab-row");
        tab_row.append(&tabs);
        tab_row.append(&note);

        let list = gtk::ListBox::new();
        list.add_css_class("focus-filtered-list");
        list.set_selection_mode(gtk::SelectionMode::Single);
        let scrolled = gtk::ScrolledWindow::builder()
            .child(&list)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .build();
        let footer = KeyLine::new("focus-filtered-footer");

        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.add_css_class("focus-filtered");
        root.append(&header);
        root.append(&tab_row);
        root.append(&scrolled);
        root.append(footer.widget());

        let view = Rc::new(FilteredView {
            client,
            root,
            tabs,
            list,
            footer,
            sweep_words,
            keymap: RefCell::new(keymap.clone()),
            tab: Cell::new(0),
            counts: Cell::new([0; 7]),
            rows: RefCell::default(),
            shown: RefCell::default(),
            more: Cell::new(false),
            generation: Cell::new(0),
            handler: RefCell::default(),
            me: RefCell::default(),
        });
        view.me.replace(Rc::downgrade(&view));
        let weak = Rc::downgrade(&view);
        back.connect_clicked({
            let weak = weak.clone();
            move |_| {
                if let Some(view) = weak.upgrade() {
                    view.emit(FilteredAction::Back);
                }
            }
        });
        sweep.connect_clicked({
            let weak = weak.clone();
            move |_| {
                if let Some(view) = weak.upgrade() {
                    view.emit(FilteredAction::Sweep);
                }
            }
        });
        view.list.connect_row_activated({
            let weak = weak.clone();
            move |_, row| {
                if let Some(view) = weak.upgrade()
                    && let Some(found) = view.row_at(row.index())
                {
                    view.emit(FilteredAction::Open {
                        message: found.message.id,
                        subject: found.message.subject.clone().unwrap_or_default(),
                    });
                }
            }
        });
        view.list.connect_row_selected({
            let weak = weak.clone();
            move |list, selected| {
                // Only the focused row offers its restore (screen 21).
                let mut child = list.first_child();
                while let Some(row) = child {
                    child = row.next_sibling();
                    if let Some(button) = restore_button(&row) {
                        button.set_visible(selected.is_some_and(|selected| *selected == row));
                    }
                }
                let _ = weak.upgrade();
            }
        });
        scrolled.connect_edge_reached({
            let weak = weak.clone();
            move |_, edge| {
                if edge == gtk::PositionType::Bottom
                    && let Some(view) = weak.upgrade()
                    && view.more.get()
                {
                    view.read_page(view.rows.borrow().len() as u32);
                }
            }
        });
        view.set_keymap(keymap);
        view
    }

    /// The commands the view has a control for: Back, the sweep, the
    /// restore, and the tabs.
    pub fn controls() -> Vec<CommandId> {
        [
            CommandId::Back,
            CommandId::SweepInbox,
            CommandId::RestoreFiltered,
        ]
        .into_iter()
        .chain(filtered::TAB_COMMANDS)
        .collect()
    }

    /// The view, for the window's pages.
    pub fn widget(&self) -> &gtk::Box {
        &self.root
    }

    /// Run `handler` with what the person asks.
    pub fn connect_action(&self, handler: impl Fn(FilteredAction) + 'static) {
        self.handler.replace(Some(Rc::new(handler)));
    }

    /// Read every key from `keymap`.
    pub fn set_keymap(&self, keymap: &Keymap) {
        self.keymap.replace(keymap.clone());
        while let Some(child) = self.sweep_words.first_child() {
            self.sweep_words.remove(&child);
        }
        self.sweep_words.append(&keyhint::labelled(
            filtered::SWEEP_BUTTON,
            postio_ui::hints::key(keymap, CommandId::SweepInbox).as_deref(),
        ));
        self.footer.set(&filtered::footer(keymap));
        self.show_tabs();
        self.show_rows();
    }

    /// Open on All: read the tabs' counts and the first page.
    pub fn open(&self) {
        self.tab.set(0);
        self.refresh();
    }

    /// Read the counts and the tab on screen again: after a restore, an
    /// undo, or mail filed away.
    pub fn refresh(&self) {
        self.read_tabs();
        self.read_page(0);
    }

    /// Show the `index`th tab, `0` for All: `1`-`7`.
    pub fn set_tab(&self, index: usize) {
        if index >= filtered::TABS.len() {
            return;
        }
        self.tab.set(index);
        self.show_tabs();
        self.read_page(0);
    }

    /// The tab on screen, `0` for All.
    pub fn tab(&self) -> usize {
        self.tab.get()
    }

    /// The focused row's message, if any: what `R` restores.
    pub fn focused(&self) -> Option<MessageId> {
        let row = self.list.selected_row()?;
        self.row_at(row.index()).map(|found| found.message.id)
    }

    /// The listed rows' subjects, top to bottom.
    pub fn subjects(&self) -> Vec<String> {
        self.rows
            .borrow()
            .iter()
            .map(|row| row.message.subject.clone().unwrap_or_default())
            .collect()
    }

    /// Every text the view shows, in order: what a test reads.
    pub fn texts(&self) -> Vec<String> {
        let mut said = Vec::new();
        let mut stack = vec![self.root.clone().upcast::<gtk::Widget>()];
        while let Some(widget) = stack.pop() {
            if !widget.is_visible() {
                continue;
            }
            if let Some(label) = widget.downcast_ref::<gtk::Label>()
                && !label.text().is_empty()
            {
                said.push(label.text().to_string());
            }
            let mut child = widget.last_child();
            while let Some(previous) = child {
                child = previous.prev_sibling();
                stack.push(previous);
            }
        }
        said
    }

    /// Move the focus `by` rows, over the day headings: `j` and `k`.
    pub fn step(&self, by: i32) {
        let rows = self.shown.borrow().clone();
        let mut at = self.list.selected_row().map_or(-1, |row| row.index());
        loop {
            at += by;
            let Some(slot) = usize::try_from(at).ok().and_then(|at| rows.get(at)) else {
                return;
            };
            if slot.is_some() {
                if let Some(row) = self.list.row_at_index(at) {
                    self.list.select_row(Some(&row));
                    row.grab_focus();
                }
                return;
            }
        }
    }

    /// Open the focused row: `Enter`.
    pub fn open_focused(&self) {
        if let Some(row) = self.list.selected_row() {
            row.activate();
        }
    }

    /// Give the list the keyboard, on its first row.
    pub fn focus_list(&self) {
        if self.list.selected_row().is_none()
            && let Some(first) = self.list_row(0)
        {
            self.list.select_row(Some(&first));
        }
        match self.list.selected_row() {
            Some(row) => {
                row.grab_focus();
            }
            None => {
                self.list.grab_focus();
            }
        }
    }

    fn emit(&self, action: FilteredAction) {
        let handler = self.handler.borrow().clone();
        if let Some(handler) = handler {
            handler(action);
        }
    }

    fn row_at(&self, index: i32) -> Option<FilteredRow> {
        let at = usize::try_from(index).ok()?;
        let index = self.shown.borrow().get(at).copied().flatten()?;
        self.rows.borrow().get(index).cloned()
    }

    /// The list row that shows the `index`th of `rows`.
    fn list_row(&self, index: usize) -> Option<gtk::ListBoxRow> {
        let at = self
            .shown
            .borrow()
            .iter()
            .position(|shown| *shown == Some(index))?;
        self.list.row_at_index(i32::try_from(at).ok()?)
    }

    /// Read the tabs' counts.
    fn read_tabs(&self) {
        let client = self.client.clone();
        let weak = self.me.borrow().clone();
        glib::spawn_future_local(async move {
            // POSTIO-GLIB-SAFE: a client call is a oneshot receive; the host
            // answers on its own runtime (ADR 0041).
            let read = client.filtered_tabs().await;
            let (Some(view), Ok(reasons)) = (weak.upgrade(), read) else {
                return;
            };
            view.counts.set(filtered::tab_counts(&reasons));
            view.show_tabs();
        });
    }

    /// Read the tab on screen from `offset`: the first page replaces what
    /// is listed, a later one adds to it.
    fn read_page(&self, offset: u32) {
        let generation = if offset == 0 {
            self.generation.get() + 1
        } else {
            self.generation.get()
        };
        self.generation.set(generation);
        let reason = filtered::TABS[self.tab.get()].0.map(str::to_owned);
        let client = self.client.clone();
        let weak = self.me.borrow().clone();
        self.more.set(false);
        glib::spawn_future_local(async move {
            // POSTIO-GLIB-SAFE: as `read_tabs`'.
            let read = client.filtered(reason, offset, PAGE).await;
            let Some(view) = weak.upgrade() else {
                return;
            };
            if view.generation.get() != generation {
                return;
            }
            let page = match read {
                Ok(page) => page,
                Err(error) => {
                    tracing::warn!(%error, "Focus could not read Filtered: {error}");
                    return;
                }
            };
            view.more.set(page.len() as u32 == PAGE);
            let focused = view.focused();
            {
                let mut rows = view.rows.borrow_mut();
                if offset == 0 {
                    rows.clear();
                }
                rows.extend(page);
            }
            view.show_rows();
            // Keep the keyboard where it was, or on the row that took its
            // place.
            let index = focused
                .and_then(|message| {
                    view.rows
                        .borrow()
                        .iter()
                        .position(|row| row.message.id == message)
                })
                .unwrap_or(0);
            if let Some(row) = view.list_row(index) {
                view.list.select_row(Some(&row));
            }
        });
    }

    /// Draw the tabs with their counts, the one on screen raised.
    fn show_tabs(&self) {
        while let Some(child) = self.tabs.first_child() {
            self.tabs.remove(&child);
        }
        let counts = self.counts.get();
        for (index, (_, name)) in filtered::TABS.iter().enumerate() {
            let words = gtk::Box::new(gtk::Orientation::Horizontal, S1);
            let label = gtk::Label::new(Some(name));
            words.append(&label);
            let count = gtk::Label::new(Some(&counts[index].to_string()));
            count.add_css_class("dim-label");
            words.append(&count);
            let tab = gtk::Button::new();
            tab.set_child(Some(&words));
            tab.add_css_class("flat");
            tab.add_css_class("focus-filtered-tab");
            if index == self.tab.get() {
                tab.add_css_class("focus-filtered-tab-on");
            }
            if let Some(key) =
                postio_ui::hints::key(&self.keymap.borrow(), filtered::TAB_COMMANDS[index])
            {
                tab.set_tooltip_text(Some(&format!("{name} ({key})")));
            }
            let weak = self.me.borrow().clone();
            tab.connect_clicked(move |_| {
                if let Some(view) = weak.upgrade() {
                    view.set_tab(index);
                }
            });
            self.tabs.append(&tab);
        }
    }

    /// Draw the rows, each day under its heading.
    fn show_rows(&self) {
        crate::rebuild::keeping_focus(&self.list, || self.rebuild_rows());
    }

    fn rebuild_rows(&self) {
        while let Some(child) = self.list.first_child() {
            self.list.remove(&child);
        }
        let now = postio_ui::clock::now();
        let today = now.date_naive();
        let restore_key = postio_ui::hints::key(&self.keymap.borrow(), CommandId::RestoreFiltered);
        let rows = self.rows.borrow();
        let day_of = |row: &FilteredRow| row.at.with_timezone(&chrono::Local).date_naive();
        let mut shown = Vec::new();
        let mut day = None;
        for (index, row) in rows.iter().enumerate() {
            let local = day_of(row);
            if day != Some(local) {
                day = Some(local);
                // "Today · 9": the day, and how many of its rows are here.
                let count = rows.iter().filter(|other| day_of(other) == local).count();
                let heading = gtk::Label::new(Some(&format!(
                    "{} \u{b7} {count}",
                    postio_ui::focus_row::day_heading(local, today)
                )));
                heading.add_css_class("focus-day-heading");
                heading.add_css_class("focus-filtered-day");
                heading.set_xalign(0.0);
                let item = gtk::ListBoxRow::new();
                item.set_child(Some(&heading));
                item.set_selectable(false);
                item.set_activatable(false);
                item.set_can_focus(false);
                self.list.append(&item);
                shown.push(None);
            }
            let line = gtk::Box::new(gtk::Orientation::Horizontal, S3);
            line.add_css_class("focus-filtered-row");
            let sender = gtk::Label::new(Some(
                &row.message
                    .from
                    .as_ref()
                    .map(|from| from.display().to_owned())
                    .unwrap_or_default(),
            ));
            sender.add_css_class("focus-filtered-sender");
            sender.set_xalign(0.0);
            sender.set_width_chars(22);
            sender.set_max_width_chars(22);
            sender.set_ellipsize(gtk::pango::EllipsizeMode::End);
            line.append(&sender);
            let subject = gtk::Label::new(Some(row.message.subject.as_deref().unwrap_or_default()));
            subject.add_css_class("focus-filtered-subject");
            subject.set_xalign(0.0);
            subject.set_ellipsize(gtk::pango::EllipsizeMode::End);
            line.append(&subject);
            let preview = gtk::Label::new(row.message.preview.as_deref());
            preview.add_css_class("dim-label");
            preview.set_xalign(0.0);
            preview.set_hexpand(true);
            preview.set_ellipsize(gtk::pango::EllipsizeMode::End);
            line.append(&preview);
            let restore = gtk::Button::new();
            restore.add_css_class("focus-filtered-restore");
            restore.set_child(Some(&keyhint::labelled(
                filtered::RESTORE,
                restore_key.as_deref(),
            )));
            restore.set_visible(false);
            restore.set_valign(gtk::Align::Center);
            let message = row.message.id;
            let weak = self.me.borrow().clone();
            restore.connect_clicked(move |_| {
                if let Some(view) = weak.upgrade() {
                    view.emit(FilteredAction::Restore(message));
                }
            });
            line.append(&restore);
            let pill = gtk::Label::new(Some(&filtered::pill(&row.reason, row.source.as_deref())));
            pill.add_css_class("focus-filtered-pill");
            pill.set_valign(gtk::Align::Center);
            line.append(&pill);
            let time = gtk::Label::new(Some(&postio_ui::row::timestamp(row.at, now)));
            time.add_css_class("focus-filtered-time");
            line.append(&time);

            let item = gtk::ListBoxRow::new();
            item.set_child(Some(&line));
            self.list.append(&item);
            shown.push(Some(index));
        }
        drop(rows);
        self.shown.replace(shown);
        // Rows are rebuilt here, including after `read_page`'s async
        // fetch lands, so a cap taught only once after `open()` would miss
        // every row a later page adds (T142, T143).
        crate::a11y::teach_shortcuts(&self.root);
        crate::motion::keep_to_budget(&self.root);
    }
}

/// The restore button in a row of the list, wherever the day heading put
/// it.
fn restore_button(row: &gtk::Widget) -> Option<gtk::Button> {
    let mut stack = vec![row.clone()];
    while let Some(widget) = stack.pop() {
        if widget.has_css_class("focus-filtered-restore") {
            return widget.downcast().ok();
        }
        let mut child = widget.first_child();
        while let Some(next) = child {
            child = next.next_sibling();
            stack.push(next);
        }
    }
    None
}
