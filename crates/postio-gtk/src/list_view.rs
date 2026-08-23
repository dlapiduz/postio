//! The message list pane: canvas 1b's header, and the rows under it.
//!
//! This is the shallow half of the list — the deep half is [`crate::list`]'s
//! windowed model and [`crate::row`]'s hand-drawn row. What lives here is the
//! `GtkListView` that puts the two together, and the header line the canvas
//! draws above them: the folder, its unread count, and the sort order.
//!
//! # Why a `GtkListView` and not a `GtkListBox`
//!
//! `GtkListBox` materialises a widget per item, which is the one thing
//! spec.md §18 forbids: a 100,000-message folder would cost 100,000 widgets.
//! `GtkListView` recycles a screenful, asks the model only for the rows it is
//! about to draw, and hands each one to the same handful of
//! [`MessageRowView`]s over and over. That is what makes the windowed model
//! worth having.
//!
//! # Focus lands on the row, not on the list item
//!
//! `GtkListItem::set_focusable(false)` pushes the keyboard down into the row
//! widget itself, so [`MessageRowView::shows_hints`] can simply ask whether it
//! has focus. Without that, focus would stop at the list item wrapping the
//! row and every row would have to ask its parent — the same answer, reached
//! by a longer and more fragile route.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::glib;
use postio_config::Density;

use crate::list::{MessageList, MessageRow};
use crate::row::MessageRowView;

/// What to call when a row is activated — `Enter`, or a double click.
type ActivateHandler = Box<dyn Fn(crate::list::Row)>;

mod imp {
    use super::*;

    pub struct MessageListView {
        pub(super) title: gtk::Label,
        pub(super) meta: gtk::Label,
        pub(super) sort: gtk::Label,
        pub(super) model: MessageList,
        pub(super) selection: gtk::SingleSelection,
        pub(super) view: gtk::ListView,
        pub(super) density: Rc<Cell<Density>>,
        pub(super) activated: RefCell<Vec<ActivateHandler>>,
    }

    impl Default for MessageListView {
        fn default() -> Self {
            let model = MessageList::new();
            let selection = gtk::SingleSelection::new(Some(model.clone()));
            MessageListView {
                title: gtk::Label::new(None),
                meta: gtk::Label::new(None),
                sort: gtk::Label::new(Some("Newest ▾")),
                view: gtk::ListView::new(Some(selection.clone()), None::<gtk::ListItemFactory>),
                model,
                selection,
                density: Rc::new(Cell::new(Density::default())),
                activated: RefCell::new(Vec::new()),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for MessageListView {
        const NAME: &'static str = "PostioMessageListView";
        type Type = super::MessageListView;
        type ParentType = adw::Bin;
    }

    impl ObjectImpl for MessageListView {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().build();
        }
    }

    impl WidgetImpl for MessageListView {
        /// Focusing the pane means focusing a row.
        ///
        /// Without this the keyboard would stop at the scroller, which
        /// looks like focus and acts like nothing: no selected row, no key
        /// hints, and `j`/`k` with nowhere to go.
        fn grab_focus(&self) -> bool {
            self.view.grab_focus()
        }
    }
    impl BinImpl for MessageListView {}
}

glib::wrapper! {
    /// The list pane: a header, and a windowed list of rows under it.
    pub struct MessageListView(ObjectSubclass<imp::MessageListView>)
        @extends adw::Bin, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Default for MessageListView {
    fn default() -> Self {
        glib::Object::new()
    }
}

impl MessageListView {
    /// An empty list pane, with no mailbox named and no source attached.
    pub fn new() -> Self {
        Self::default()
    }

    /// The model to point at a mailbox with `set_source`, and to drive from
    /// the event stream. `postio-91i` is what fills it.
    pub fn model(&self) -> MessageList {
        self.imp().model.clone()
    }

    /// The selection, for whoever needs to know what an action will hit.
    pub fn selection(&self) -> gtk::SingleSelection {
        self.imp().selection.clone()
    }

    /// Name the mailbox in view and say how much of it is unread.
    pub fn set_mailbox(&self, name: &str, unread: u32) {
        let imp = self.imp();
        imp.title.set_text(name);
        imp.meta.set_text(&match unread {
            0 => String::new(),
            n => format!("{n} unread"),
        });
        imp.meta.set_visible(unread > 0);
    }

    /// Switch every row to `density`, live.
    ///
    /// A re-measure of the rows already on screen, never a rebuilt widget
    /// tree: the rows on screen are the rows that stay on screen.
    pub fn set_density(&self, density: Density) {
        let imp = self.imp();
        if imp.density.replace(density) == density {
            return;
        }
        self.each_row(|row| row.set_density(density));
    }

    /// The density currently in force.
    pub fn density(&self) -> Density {
        self.imp().density.get()
    }

    /// Called when a row is activated.
    pub fn connect_activated(&self, handler: impl Fn(crate::list::Row) + 'static) {
        self.imp().activated.borrow_mut().push(Box::new(handler));
    }

    /// Every row widget currently realised, for a live restyle.
    pub fn each_row(&self, handler: impl Fn(&MessageRowView)) {
        let mut child = self.imp().view.first_child();
        while let Some(item) = child {
            if let Some(row) = item.first_child().and_downcast::<MessageRowView>() {
                handler(&row);
            }
            child = item.next_sibling();
        }
    }

    fn build(&self) {
        let imp = self.imp();
        self.add_css_class("postio-listview");

        imp.title.add_css_class("postio-list-title");
        imp.meta.add_css_class("postio-list-meta");
        imp.sort.add_css_class("postio-list-meta");

        let header = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        header.add_css_class("postio-list-header");
        header.set_valign(gtk::Align::Baseline);
        header.append(&imp.title);
        header.append(&imp.meta);
        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        header.append(&spacer);
        header.append(&imp.sort);
        // The header names the pane; the rows below it are the list itself,
        // so a screen reader hears "Inbox, 12 unread" once and then rows.
        header.set_accessible_role(gtk::AccessibleRole::Presentation);

        let factory = gtk::SignalListItemFactory::new();
        factory.connect_setup(move |_, item| {
            let Some(item) = item.downcast_ref::<gtk::ListItem>() else {
                return;
            };
            let view = MessageRowView::new();
            item.set_child(Some(&view));
            // Focus belongs to the row, not to the box around it.
            item.set_focusable(false);
            item.set_activatable(true);
            // Selection lives on the list item and its state flag does not
            // reach the child, so it is handed down explicitly — and kept
            // in step, because a click on another row changes this one.
            item.connect_selected_notify(glib::clone!(
                #[weak]
                view,
                move |item| view.set_selected(item.is_selected())
            ));
        });
        let bound = imp.density.clone();
        factory.connect_bind(move |_, item| {
            let Some(item) = item.downcast_ref::<gtk::ListItem>() else {
                return;
            };
            let Some(view) = item.child().and_downcast::<MessageRowView>() else {
                return;
            };
            view.set_density(bound.get());
            view.set_first(item.position() == 0);
            view.set_selected(item.is_selected());
            view.set_row(
                item.item()
                    .and_downcast::<MessageRow>()
                    .and_then(|item| item.row()),
            );
        });
        factory.connect_unbind(move |_, item| {
            if let Some(view) = item
                .downcast_ref::<gtk::ListItem>()
                .and_then(|item| item.child())
                .and_downcast::<MessageRowView>()
            {
                view.set_row(None);
            }
        });

        imp.view.set_factory(Some(&factory));
        imp.view.add_css_class("postio-rows");
        imp.view.set_show_separators(false);
        imp.view.set_single_click_activate(false);
        imp.view.set_vexpand(true);
        imp.view.connect_activate(glib::clone!(
            #[weak(rename_to = pane)]
            self,
            move |view, position| {
                let Some(row) = view
                    .model()
                    .and_then(|model| model.item(position))
                    .and_downcast::<MessageRow>()
                    .and_then(|item| item.row())
                else {
                    return;
                };
                for handler in pane.imp().activated.borrow().iter() {
                    handler(row.clone());
                }
            }
        ));

        let scroller = gtk::ScrolledWindow::new();
        scroller.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        scroller.set_child(Some(&imp.view));
        scroller.set_vexpand(true);

        let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
        column.append(&header);
        column.append(&scroller);
        self.set_child(Some(&column));
    }
}
