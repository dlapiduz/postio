//! The list as it sits in the window: a `GtkListView` over the feed's
//! model, in a scroller, with Focus's rows.

use std::cell::Cell;
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{glib, graphene};

use postio_widgets::list_model::WindowedModel;

use super::feed::Feed;
use super::heading::DayHeading;
use super::model::RowObject;
use super::row::{ActionHandler, Pick, PickHandler, RowWidget, SharedKeymap};

/// The list pane: what scrolls, what draws, and where its rows come from.
#[derive(Clone)]
pub struct ListPane {
    scrolled: gtk::ScrolledWindow,
    view: gtk::ListView,
    cursor: gtk::SingleSelection,
    feed: Feed,
    keymap: SharedKeymap,
    /// Whether the list is held at its top, first heading showing.
    pinned: Rc<Cell<bool>>,
    /// What a press on a row's drawn action runs, once the window says.
    on_action: Rc<std::cell::RefCell<Option<ActionHandler>>>,
    /// What a Ctrl- or Shift-click on a row runs, once the window says.
    on_pick: Rc<std::cell::RefCell<Option<PickHandler>>>,
    /// Whether a to-do's row offers Task: once a vault is configured.
    capture: Rc<Cell<bool>>,
}

/// The selection a list's rows draw their boxes from.
pub type SharedSelection = postio_ui::selection::SelectionState;

impl ListPane {
    /// A pane drawing `feed`'s list, its rows' keycaps read from `keymap`
    /// and their selection boxes from `picked`.
    pub fn new(feed: Feed, keymap: postio_core::Keymap, picked: SharedSelection) -> Self {
        let keymap: SharedKeymap = std::rc::Rc::new(std::cell::RefCell::new(keymap));
        let on_action: Rc<std::cell::RefCell<Option<ActionHandler>>> = Rc::default();
        let on_pick: Rc<std::cell::RefCell<Option<PickHandler>>> = Rc::default();
        let capture: Rc<Cell<bool>> = Rc::default();
        let factory = gtk::SignalListItemFactory::new();
        factory.connect_setup({
            let keymap = keymap.clone();
            let capture = Rc::clone(&capture);
            let picked = picked.clone();
            let on_action = Rc::clone(&on_action);
            let on_pick = Rc::clone(&on_pick);
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
                item.set_child(Some(&row));
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

        // The cursor, not the selection: GTK's name, Postio's meaning
        // (`postio_ui::selection`). What `a` would archive is kept apart.
        let cursor = gtk::SingleSelection::new(Some(feed.list().clone()));
        cursor.set_autoselect(false);
        cursor.set_can_unselect(true);
        let view = gtk::ListView::new(Some(cursor.clone()), Some(factory));
        view.set_header_factory(Some(&day_headings(feed.list())));
        // A page landing may move where a day starts.
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
        }
    }

    /// Whether a to-do's row offers Task `t` beside Snooze: once a vault is
    /// configured (spec C9).
    pub fn set_capture(&self, capture: bool) {
        if self.capture.replace(capture) != capture {
            self.redraw_rows();
        }
    }

    /// Run `handler` when a row's drawn action is pressed, with the row's
    /// item and the action's command.
    pub fn connect_row_action(
        &self,
        handler: impl Fn(&super::FocusRow, postio_core::CommandId) + 'static,
    ) {
        self.on_action.replace(Some(Rc::new(handler)));
    }

    /// Run `handler` when a Ctrl- or Shift-click picks a row (T198).
    pub fn connect_row_pick(&self, handler: impl Fn(&super::FocusRow, Pick) + 'static) {
        self.on_pick.replace(Some(Rc::new(handler)));
    }

    /// Run `handler` on a secondary click on a row (T199): the row's place
    /// in the list, and where the click was, in the list view's
    /// coordinates. A click between rows -- a day's heading -- runs nothing.
    pub fn connect_row_menu(&self, handler: impl Fn(u32, gtk::gdk::Rectangle) + 'static) {
        let click = gtk::GestureClick::new();
        click.set_button(gtk::gdk::BUTTON_SECONDARY);
        let view = self.view.downgrade();
        let list = self.feed.list().downgrade();
        click.connect_pressed(move |gesture, _, x, y| {
            let (Some(view), Some(list)) = (view.upgrade(), list.upgrade()) else {
                return;
            };
            let mut at = view.pick(x, y, gtk::PickFlags::DEFAULT);
            while let Some(widget) = at {
                if let Some(row) = widget.downcast_ref::<RowWidget>() {
                    if let Some(position) = row.item().and_then(|item| list.position_of(item.id()))
                    {
                        gesture.set_state(gtk::EventSequenceState::Claimed);
                        handler(
                            position,
                            gtk::gdk::Rectangle::new(x as i32, y as i32, 1, 1),
                        );
                    }
                    return;
                }
                at = widget.parent();
            }
        });
        self.view.add_controller(click);
    }

    /// Scroll to the very top, first heading showing, and hold it there
    /// while the rows settle: the cursor has gone to the first row, and
    /// GTK would bring that row into view without its heading.
    pub fn to_top(&self) {
        self.pinned.set(true);
        self.scrolled.vadjustment().set_value(0.0);
    }

    /// Redraw every row on screen: what they draw from beside their own
    /// data -- the selection, the keymap -- has changed.
    pub fn redraw_rows(&self) {
        let mut child = self.view.first_child();
        while let Some(widget) = child {
            child = widget.next_sibling();
            if let Some(row) = widget.first_child().and_downcast::<RowWidget>() {
                row.queue_draw();
            }
        }
    }

    /// Read every row's keycaps from `keymap` from now on.
    pub fn set_keymap(&self, keymap: postio_core::Keymap) {
        self.keymap.replace(keymap);
        self.redraw_rows();
    }

    /// The pane's outermost widget, to place in a layout.
    pub fn widget(&self) -> &gtk::ScrolledWindow {
        &self.scrolled
    }

    /// The list view.
    pub fn view(&self) -> &gtk::ListView {
        &self.view
    }

    /// The cursor: where the keyboard is.
    pub fn cursor(&self) -> &gtk::SingleSelection {
        &self.cursor
    }

    /// The feed the rows come from.
    pub fn feed(&self) -> &Feed {
        &self.feed
    }

    /// The row widgets a person can see, top to bottom: bound, mapped, and
    /// inside the scroller's viewport.
    pub fn rows_on_screen(&self) -> Vec<RowWidget> {
        let height = self.scrolled.height() as f32;
        let mut rows: Vec<(f32, RowWidget)> = Vec::new();
        let mut child = self.view.first_child();
        while let Some(widget) = child {
            child = widget.next_sibling();
            let Some(row) = widget.first_child().and_downcast::<RowWidget>() else {
                continue;
            };
            if !row.is_mapped() || row.item().is_none() {
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

/// The day headings: "Today · Saturday 26 September" over a day's rows, a
/// section header each (32 px), named from the section's first row.
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

/// Hold the list at its top, its first day heading on screen, until the
/// person scrolls away from it.
///
/// GTK anchors a list on its first row and lays that row's section header
/// out above it, off screen: when the rows arrive, and again every time
/// they change height as their pages land, the view is put back one
/// heading's height down (measured: 33 px, the heading's 32 and its rule),
/// and the first thing the list says -- what day it is -- is hidden. So
/// while the list is pinned, a move that only hides the first heading is
/// taken back.
///
/// The list is pinned when it opens, when its rows are replaced wholesale
/// (the has-action filter), whenever it is back at 0, and when the cursor
/// goes to the first row. It is unpinned by the person's own scrolling,
/// and by any move past the heading -- the cursor restored twenty rows
/// down is a real scroll, and is left alone.
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

/// How tall the list's first day heading asks to be. Measured, not read
/// from its allocation: GTK scrolls while it lays the list out, before the
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
