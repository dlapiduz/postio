use std::cell::Cell;
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{glib, graphene};

use super::feed::Feed;
use super::heading::DayHeading;
use super::model::RowObject;
use super::row::{
    ActionHandler, ChevronHandler, Pick, PickHandler, RowWidget, SharedKeymap, SharedMe,
};

#[derive(Clone)]
pub struct ListPane {
    scrolled: gtk::ScrolledWindow,
    view: gtk::ListView,
    cursor: gtk::SingleSelection,
    feed: Feed,
    keymap: SharedKeymap,
    pinned: Rc<Cell<bool>>,
    on_action: Rc<std::cell::RefCell<Option<ActionHandler>>>,
    on_pick: Rc<std::cell::RefCell<Option<PickHandler>>>,
    capture: Rc<Cell<bool>>,
    on_chevron: Rc<std::cell::RefCell<Option<ChevronHandler>>>,
    me: SharedMe,
}

pub type SharedSelection = postio_ui::selection::SelectionState;

impl ListPane {
    pub fn new(feed: Feed, keymap: postio_core::Keymap, picked: SharedSelection) -> Self {
        let keymap: SharedKeymap = std::rc::Rc::new(std::cell::RefCell::new(keymap));
        let on_action: Rc<std::cell::RefCell<Option<ActionHandler>>> = Rc::default();
        let on_pick: Rc<std::cell::RefCell<Option<PickHandler>>> = Rc::default();
        let capture: Rc<Cell<bool>> = Rc::default();
        let on_chevron: Rc<std::cell::RefCell<Option<ChevronHandler>>> = Rc::default();
        let me: SharedMe = Rc::default();
        let factory = gtk::SignalListItemFactory::new();
        factory.connect_setup({
            let keymap = keymap.clone();
            let capture = Rc::clone(&capture);
            let picked = picked.clone();
            let on_action = Rc::clone(&on_action);
            let on_pick = Rc::clone(&on_pick);
            let on_chevron = Rc::clone(&on_chevron);
            let me = Rc::clone(&me);
            move |_, item| {
                let item = item
                    .downcast_ref::<gtk::ListItem>()
                    .expect("a list view's factory builds list items");
                let row = RowWidget::default();
                row.set_keymap(keymap.clone());
                row.set_capture(Rc::clone(&capture));
                row.set_selection(picked.clone());
                let on_action = Rc::clone(&on_action);
                row.set_on_action(Rc::new(move |item, command| {
                    let handler = on_action.borrow().clone();
                    if let Some(handler) = handler {
                        handler(item, command);
                    }
                }));
                let on_pick = Rc::clone(&on_pick);
                row.set_on_pick(Rc::new(move |item, pick| {
                    let handler = on_pick.borrow().clone();
                    if let Some(handler) = handler {
                        handler(item, pick);
                    }
                }));
                row.set_identities(Rc::clone(&me));
                let on_chevron = Rc::clone(&on_chevron);
                row.set_on_chevron(Rc::new(move |item, expanded| {
                    let handler = on_chevron.borrow().clone();
                    if let Some(handler) = handler {
                        handler(item, expanded);
                    }
                }));
                item.set_child(Some(&row));
                item.connect_selected_notify(|item| {
                    if let Some(widget) = item.child().and_downcast::<RowWidget>() {
                        widget.set_cursor_here(item.is_selected());
                    }
                });
            }
        });
        factory.connect_bind(|_, item| {
            let item = item
                .downcast_ref::<gtk::ListItem>()
                .expect("a list view's factory builds list items");
            if let (Some(row), Some(widget)) = (
                item.item().and_downcast::<RowObject>(),
                item.child().and_downcast::<RowWidget>(),
            ) {
                widget.bind(&row);
                widget.set_cursor_here(item.is_selected());
            }
        });
        factory.connect_unbind(|_, item| {
            let item = item
                .downcast_ref::<gtk::ListItem>()
                .expect("a list view's factory builds list items");
            if let Some(widget) = item.child().and_downcast::<RowWidget>() {
                widget.unbind();
            }
        });

        // The cursor, not the selection: GTK's name, Postio's meaning. What `e` would archive is
        // kept apart.
        let cursor = keep_the_cursor(feed.shown());
        // A place opening builds the rows anew and the row with the keyboard goes with the old
        // ones: the keyboard stays in the list until the cursor's row takes it (#2150).
        let view = crate::rebuild::keeping_keyboard(&cursor, || {
            gtk::ListView::new(Some(cursor.clone()), Some(factory))
        });
        view.set_header_factory(Some(&day_headings(feed.list())));
        feed.list().connect_local("filled", false, {
            let list = feed.list().downgrade();
            move |_| {
                if let Some(list) = list.upgrade() {
                    list.days_moved();
                }
                None
            }
        });
        view.add_css_class("focus-list");
        view.update_property(&[gtk::accessible::Property::Label("Inbox")]);
        let scrolled = gtk::ScrolledWindow::builder()
            .child(&view)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .hexpand(true)
            .build();
        let pinned = hold_the_top(feed.list(), &view, &scrolled);
        ListPane {
            scrolled,
            view,
            cursor,
            feed,
            keymap,
            pinned,
            on_action,
            on_pick,
            capture,
            on_chevron,
            me,
        }
    }

    pub fn set_capture(&self, capture: bool) {
        if self.capture.replace(capture) != capture {
            self.redraw_rows();
        }
    }

    pub fn connect_row_action(
        &self,
        handler: impl Fn(&super::FocusRow, postio_core::CommandId) + 'static,
    ) {
        self.on_action.replace(Some(Rc::new(handler)));
    }

    pub fn connect_row_chevron(&self, handler: impl Fn(&super::FocusRow, bool) + 'static) {
        self.on_chevron.replace(Some(Rc::new(handler)));
    }

    pub fn set_identities(&self, me: Vec<postio_model::EmailAddress>) {
        if self.me.replace(me.clone()) != me {
            self.redraw_rows();
        }
    }

    pub fn identities(&self) -> Vec<postio_model::EmailAddress> {
        self.me.borrow().clone()
    }

    pub fn connect_row_pick(&self, handler: impl Fn(&super::FocusRow, Pick) + 'static) {
        self.on_pick.replace(Some(Rc::new(handler)));
    }

    /// Asked only once the pointer has moved far enough to be a drag; `None` for a drag with
    /// nothing to give or one starting between rows.
    /// Copy, not move: what leaves is a file, so the row stays where it is.
    pub fn connect_row_drag(
        &self,
        offer: impl Fn(u32) -> Option<gtk::gdk::ContentProvider> + 'static,
    ) {
        let drag = gtk::DragSource::new();
        drag.set_actions(gtk::gdk::DragAction::COPY);
        let view = self.view.downgrade();
        let list = self.feed.shown().downgrade();
        drag.connect_prepare(move |_, x, y| {
            let (view, list) = (view.upgrade()?, list.upgrade()?);
            offer(row_position_at(&view, &list, x, y)?)
        });
        self.view.add_controller(drag);
    }

    /// A click between rows (a day's heading) runs nothing.
    pub fn connect_row_menu(&self, handler: impl Fn(u32, gtk::gdk::Rectangle) + 'static) {
        let click = gtk::GestureClick::new();
        click.set_button(gtk::gdk::BUTTON_SECONDARY);
        let view = self.view.downgrade();
        let list = self.feed.shown().downgrade();
        click.connect_pressed(move |gesture, _, x, y| {
            let (Some(view), Some(list)) = (view.upgrade(), list.upgrade()) else {
                return;
            };
            if let Some(position) = row_position_at(&view, &list, x, y) {
                gesture.set_state(gtk::EventSequenceState::Claimed);
                handler(position, gtk::gdk::Rectangle::new(x as i32, y as i32, 1, 1));
            }
        });
        self.view.add_controller(click);
    }

    /// Held there while rows settle, so GTK shows the first row with its heading; the view's anchor
    /// moves too, or rows put in above the row it last scrolled to push the list down (#2238).
    pub fn to_top(&self) {
        self.pinned.set(true);
        if self.view.model().is_some_and(|model| model.n_items() > 0) {
            self.view.scroll_to(0, gtk::ListScrollFlags::NONE, None);
        }
        self.scrolled.vadjustment().set_value(0.0);
    }

    pub fn redraw_rows(&self) {
        let mut child = self.view.first_child();
        while let Some(widget) = child {
            child = widget.next_sibling();
            if let Some(row) = widget.first_child().and_downcast::<RowWidget>() {
                row.queue_draw();
            }
        }
    }

    pub fn set_keymap(&self, keymap: postio_core::Keymap) {
        self.keymap.replace(keymap);
        self.redraw_rows();
    }

    pub fn widget(&self) -> &gtk::ScrolledWindow {
        &self.scrolled
    }

    pub fn view(&self) -> &gtk::ListView {
        &self.view
    }

    /// A list view asked to take focus lands on its first row, which would leave the keyboard's
    /// outline and the cursor's ring on different rows.
    pub fn focus_cursor(&self) {
        self.view.grab_focus();
        let at = self.cursor.selected();
        if at != gtk::INVALID_LIST_POSITION {
            self.view.scroll_to(at, gtk::ListScrollFlags::FOCUS, None);
        }
    }

    pub fn cursor(&self) -> &gtk::SingleSelection {
        &self.cursor
    }

    pub fn feed(&self) -> &Feed {
        &self.feed
    }

    pub fn rows_on_screen(&self) -> Vec<RowWidget> {
        let height = self.scrolled.height() as f32;
        let mut rows: Vec<(f32, RowWidget)> = Vec::new();
        let mut child = self.view.first_child();
        while let Some(widget) = child {
            child = widget.next_sibling();
            let Some(row) = widget.first_child().and_downcast::<RowWidget>() else {
                continue;
            };
            if !row.is_mapped() || (row.item().is_none() && row.child().is_none()) {
                continue;
            }
            let Some(top) = row.compute_point(&self.scrolled, &graphene::Point::new(0.0, 0.0))
            else {
                continue;
            };
            let bottom = top.y() + row.height() as f32;
            if bottom <= 0.0 || top.y() >= height {
                continue;
            }
            rows.push((top.y(), row));
        }
        rows.sort_by(|a, b| a.0.total_cmp(&b.0));
        rows.into_iter().map(|(_, row)| row).collect()
    }
}

fn day_headings(list: &super::model::FocusList) -> gtk::SignalListItemFactory {
    let list = list.downgrade();
    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(|_, header| {
        if let Some(header) = header.downcast_ref::<gtk::ListHeader>() {
            header.set_child(Some(&DayHeading::default()));
        }
    });
    factory.connect_bind(move |_, header| {
        let Some(header) = header.downcast_ref::<gtk::ListHeader>() else {
            return;
        };
        if let (Some(row), Some(heading), Some(list)) = (
            header.item().and_downcast::<RowObject>(),
            header.child().and_downcast::<DayHeading>(),
            list.upgrade(),
        ) {
            heading.bind(&row, &list);
        }
    });
    factory.connect_unbind(|_, header| {
        if let Some(heading) = header
            .downcast_ref::<gtk::ListHeader>()
            .and_then(|header| header.child())
            .and_downcast::<DayHeading>()
        {
            heading.unbind();
        }
    });
    factory
}

impl std::fmt::Debug for ListPane {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ListPane").finish_non_exhaustive()
    }
}

/// GTK anchors a list on its first row and lays that row's section header out above it, off screen:
/// when rows arrive or change height the view is put back one heading down (33 px), hiding the day.
/// While pinned, a move that only hides the first heading is taken back. Pinned on open, on
/// wholesale row replacement, at 0 and when the cursor goes to the first row; unpinned by the
/// person's scrolling or any move past the heading.
fn hold_the_top(
    list: &super::model::FocusList,
    view: &gtk::ListView,
    scrolled: &gtk::ScrolledWindow,
) -> Rc<Cell<bool>> {
    let pinned = Rc::new(Cell::new(true));
    list.connect_items_changed({
        let pinned = pinned.clone();
        move |list, position, _removed, added| {
            if position == 0 && added > 0 && list.n_items() == added {
                pinned.set(true);
            }
        }
    });
    let scrolling = gtk::EventControllerScroll::new(
        gtk::EventControllerScrollFlags::VERTICAL | gtk::EventControllerScrollFlags::KINETIC,
    );
    scrolling.set_propagation_phase(gtk::PropagationPhase::Capture);
    scrolling.connect_scroll({
        let pinned = pinned.clone();
        move |_, _, _| {
            pinned.set(false);
            glib::Propagation::Proceed
        }
    });
    scrolled.add_controller(scrolling);
    let view = view.downgrade();
    scrolled.vadjustment().connect_value_changed({
        let pinned = pinned.clone();
        move |adjustment| {
            let value = adjustment.value();
            if value <= 0.0 {
                pinned.set(true);
                return;
            }
            if !pinned.get() {
                return;
            }
            let heading = view
                .upgrade()
                .and_then(|view| first_heading_height(&view))
                .unwrap_or(0.0);
            if value <= heading {
                adjustment.set_value(0.0);
            } else {
                pinned.set(false);
            }
        }
    });
    pinned
}

/// Without autoselect, a `GtkSingleSelection` whose row is removed selects nothing, so after an
/// archive the next `e` acted on nothing (#1746): the row that slides into its place takes the
/// cursor, by `cursor_after_change`. Handlers run in connection order, so the one connected to
/// `list` before the selection exists sees the cursor as it was before the change.
fn keep_the_cursor(list: &super::spliced::SplicedList) -> gtk::SingleSelection {
    let weak: Rc<glib::WeakRef<gtk::SingleSelection>> = Rc::default();
    let before = Rc::new(Cell::new(gtk::INVALID_LIST_POSITION));
    list.connect_items_changed({
        let weak = weak.clone();
        let before = before.clone();
        move |_, _, _, _| {
            if let Some(cursor) = weak.upgrade() {
                before.set(cursor.selected());
            }
        }
    });
    let cursor = gtk::SingleSelection::new(Some(list.clone()));
    cursor.set_autoselect(false);
    cursor.set_can_unselect(true);
    weak.set(Some(&cursor));
    cursor.connect_items_changed(move |cursor, position, removed, added| {
        let was = before.replace(gtk::INVALID_LIST_POSITION);
        if was == gtk::INVALID_LIST_POSITION || cursor.selected() != gtk::INVALID_LIST_POSITION {
            return;
        }
        if let Some(at) = postio_ui::selection::cursor_after_change(
            Some(was),
            position,
            removed,
            added,
            cursor.n_items(),
        ) {
            cursor.set_selected(at);
        }
    });
    cursor
}

/// Measured, not read from its allocation: GTK scrolls while it lays the list out, before the
/// heading has one.
fn first_heading_height(view: &gtk::ListView) -> Option<f64> {
    let mut child = view.first_child();
    while let Some(widget) = child {
        if let Some(heading) = widget.first_child().and_downcast::<DayHeading>() {
            let (_, natural, _, _) = heading.measure(gtk::Orientation::Vertical, -1);
            return Some(f64::from(natural));
        }
        child = widget.next_sibling();
    }
    None
}

/// `None` over a day's heading or the empty space between.
fn row_position_at(
    view: &gtk::ListView,
    list: &super::spliced::SplicedList,
    x: f64,
    y: f64,
) -> Option<u32> {
    let mut at = view.pick(x, y, gtk::PickFlags::DEFAULT);
    while let Some(widget) = at {
        if let Some(row) = widget.downcast_ref::<RowWidget>() {
            return row.item().and_then(|item| list.position_of(item.id()));
        }
        at = widget.parent();
    }
    None
}
