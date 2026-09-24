//! The Contacts screen: a list of people and one of them in detail, over the
//! reading pane (specs/005-contacts R10).
//!
//! It takes the pane the way the composer does -- the shell's one owner shows
//! it, it remembers `(context, focused pane)` and gives both back on `Esc`,
//! and it drops the keyboard's focus as it goes so a hidden filter entry
//! cannot swallow the next key. While it is open the keyboard is in
//! `Context::Contacts`; the filter entry is protected by the resolver's own
//! "typing always wins" rule, so no bare letter fires while a name is typed.
//!
//! It reads nothing itself. The rows come through [`ContactPageSource`], which
//! it asks with the view and filter in force; the app answers with
//! [`ContactsPane::deliver`] or [`ContactsPane::show_rows`], and with
//! [`ContactsPane::set_detail`] when the cursor lands on someone.

use std::cell::{Cell, RefCell};
use std::collections::{BTreeSet, HashMap};
use std::rc::Rc;

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::glib;
use postio_core::{CommandId, Context};
use postio_model::{ContactDetail, ContactId, ContactListRow, ContactView};

use super::model::{ContactItem, ContactPageSource, ContactsModel};
use super::row::ContactRowView;
use crate::shell::Pane;
use crate::window::Window;

/// Below this pane width the detail stacks under the list instead of sitting
/// beside it: two readable columns need about this much.
const STACK_BELOW: f64 = 640.0;

/// The class the shell wears while Contacts has the pane.
pub const CONTACTS_OPEN_CLASS: &str = "postio-contacts-open";

type ViewHandler = Box<dyn Fn(ContactView, String)>;
type PageHandler = Box<dyn Fn(ContactView, u64, u32)>;
type PersonHandler = Box<dyn Fn(ContactId)>;
type CursorHandler = Box<dyn Fn(Option<ContactId>)>;

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct ContactsPane {
        pub window: glib::WeakRef<Window>,
        pub restore: Cell<Option<(Context, Pane)>>,
        pub open: Cell<bool>,
        pub view: Cell<ContactView>,
        pub marks: RefCell<BTreeSet<ContactId>>,
        pub model: RefCell<Option<ContactsModel>>,
        pub selection: RefCell<Option<gtk::SingleSelection>>,
        pub list: RefCell<Option<gtk::ListView>>,
        pub scroller: RefCell<Option<gtk::ScrolledWindow>>,
        pub filter: gtk::SearchEntry,
        pub title: gtk::Label,
        pub meta: gtk::Label,
        pub empty: gtk::Label,
        pub hint: gtk::Label,
        pub detail: super::super::detail::DetailView,
        pub query_handlers: RefCell<Vec<ViewHandler>>,
        pub page_handlers: RefCell<Vec<PageHandler>>,
        pub cursor_handlers: RefCell<Vec<CursorHandler>>,
        pub show_mail_handlers: RefCell<Vec<PersonHandler>>,
        pub compose_handlers: RefCell<Vec<PersonHandler>>,
        /// The `changed` connection each bound list item holds, keyed by the
        /// item -- the message list's own arrangement (`list_view.rs`): a
        /// `GtkListItem` is recycled across many people, and a connection
        /// left behind would redraw a row for someone it no longer shows.
        pub watched: RefCell<HashMap<usize, (ContactItem, glib::SignalHandlerId)>>,
        /// The person the detail was last asked for, so a delivery that does
        /// not move the cursor does not ask again.
        pub asked: Cell<Option<Option<ContactId>>>,
        /// Where the next reset puts the cursor, when a refresh asked it to
        /// stay rather than start the list over.
        pub keep: Cell<Option<u32>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ContactsPane {
        const NAME: &'static str = "PostioContactsPane";
        type Type = super::ContactsPane;
        type ParentType = gtk::Box;
    }

    impl ObjectImpl for ContactsPane {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().build();
        }
    }

    impl WidgetImpl for ContactsPane {}
    impl BoxImpl for ContactsPane {}
}

glib::wrapper! {
    /// The Contacts screen.
    pub struct ContactsPane(ObjectSubclass<imp::ContactsPane>)
        @extends gtk::Box, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Orientable;
}

impl Default for ContactsPane {
    fn default() -> Self {
        glib::Object::builder()
            .property("orientation", gtk::Orientation::Vertical)
            .build()
    }
}

/// The model's source: forwards a page request, with the view and filter in
/// force, to whoever answers the pane.
struct Forward(glib::WeakRef<ContactsPane>);

impl ContactPageSource for Forward {
    fn request(&self, generation: u64, page: u32) {
        if let Some(pane) = self.0.upgrade() {
            // A filter's rows arrive whole (`show_rows`), and a page read of
            // the view behind it would land on top of them.
            if !pane.filter_text().trim().is_empty() {
                return;
            }
            let view = pane.view();
            for handler in pane.imp().page_handlers.borrow().iter() {
                handler(view, generation, page);
            }
        }
    }
}

impl ContactsPane {
    /// A closed, empty Contacts screen.
    pub fn new() -> Self {
        Self::default()
    }

    fn build(&self) {
        let imp = self.imp();
        self.add_css_class("postio-contacts");
        self.set_vexpand(true);
        self.set_hexpand(true);

        // ── the header: title, which people, the filter ──────────────────
        let header = gtk::Box::new(gtk::Orientation::Vertical, 4);
        header.add_css_class("postio-list-header");
        imp.title.set_text("Contacts");
        imp.title.set_xalign(0.0);
        imp.title.add_css_class("postio-list-title");
        imp.meta.set_xalign(0.0);
        imp.meta.add_css_class("postio-list-meta");
        imp.filter
            .set_placeholder_text(Some("Filter by name, organisation or address"));
        imp.filter.add_css_class("postio-contacts-filter");
        imp.filter
            .update_property(&[gtk::accessible::Property::Label("Filter contacts")]);
        header.append(&imp.title);
        header.append(&imp.meta);
        header.append(&imp.filter);
        self.append(&header);

        // ── the body: the list beside the detail ─────────────────────────
        let model = ContactsModel::new(Rc::new(Forward(self.downgrade())));
        let selection = gtk::SingleSelection::new(Some(model.clone()));
        selection.set_autoselect(true);
        selection.set_can_unselect(false);
        let factory = gtk::SignalListItemFactory::new();
        factory.connect_setup(|_, item| {
            let Some(item) = item.downcast_ref::<gtk::ListItem>() else {
                return;
            };
            let view = ContactRowView::new();
            item.set_child(Some(&view));
            // Once per list item, for its lifetime: the cursor follows the
            // item's own `selected`, whoever it is showing.
            item.connect_selected_notify(glib::clone!(
                #[weak]
                view,
                move |item| view.set_cursor(item.is_selected())
            ));
        });
        factory.connect_bind(glib::clone!(
            #[weak(rename_to = pane)]
            self,
            move |_, item| pane.bind_row(item)
        ));
        factory.connect_unbind(glib::clone!(
            #[weak(rename_to = pane)]
            self,
            move |_, item| {
                let Some(item) = item.downcast_ref::<gtk::ListItem>() else {
                    return;
                };
                if let Some(view) = item.child().and_downcast::<ContactRowView>() {
                    view.set_row(None);
                }
                item.set_accessible_label("");
                if let Some((held, handler)) = pane
                    .imp()
                    .watched
                    .borrow_mut()
                    .remove(&(item.as_ptr() as usize))
                {
                    held.disconnect(handler);
                }
            }
        ));
        let list = gtk::ListView::new(Some(selection.clone()), Some(factory));
        list.add_css_class("postio-rows");
        list.update_property(&[gtk::accessible::Property::Label("People")]);
        let scroller = gtk::ScrolledWindow::builder()
            .child(&list)
            .hexpand(true)
            .vexpand(true)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .build();

        imp.empty.set_wrap(true);
        imp.empty.set_justify(gtk::Justification::Center);
        imp.empty.add_css_class("postio-contacts-empty");
        imp.empty.set_vexpand(true);
        imp.empty.set_visible(false);

        imp.hint.add_css_class("postio-contacts-hint");
        imp.hint.set_xalign(0.0);
        imp.hint.set_visible(false);

        let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
        column.set_hexpand(true);
        column.append(&scroller);
        column.append(&imp.empty);
        column.append(&imp.hint);

        let body = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        body.set_vexpand(true);
        body.append(&column);
        body.append(&imp.detail);

        // Side by side while the pane is wide enough for both; stacked, the
        // list over the detail, when it is not (research R10). Measured on the
        // pane rather than the window: at an ordinary window width the reading
        // pane is a third of it, and a detail column beside the list there left
        // the list too narrow to read a name in.
        let adaptive = adw::BreakpointBin::new();
        adaptive.set_size_request(240, 240);
        adaptive.set_vexpand(true);
        adaptive.set_child(Some(&body));
        let narrow = adw::Breakpoint::new(adw::BreakpointCondition::new_length(
            adw::BreakpointConditionLengthType::MaxWidth,
            STACK_BELOW - 1.0,
            adw::LengthUnit::Px,
        ));
        narrow.connect_apply(glib::clone!(
            #[weak]
            body,
            #[weak(rename_to = detail)]
            imp.detail,
            move |_| {
                body.set_orientation(gtk::Orientation::Vertical);
                detail.set_stacked(true);
            }
        ));
        narrow.connect_unapply(glib::clone!(
            #[weak]
            body,
            #[weak(rename_to = detail)]
            imp.detail,
            move |_| {
                body.set_orientation(gtk::Orientation::Horizontal);
                detail.set_stacked(false);
            }
        ));
        adaptive.add_breakpoint(narrow);
        self.append(&adaptive);

        selection.connect_selected_notify(glib::clone!(
            #[weak(rename_to = pane)]
            self,
            move |_| pane.cursor_moved()
        ));
        imp.filter.connect_search_changed(glib::clone!(
            #[weak(rename_to = pane)]
            self,
            move |_| pane.query_changed()
        ));
        imp.detail.connect_show_mail(glib::clone!(
            #[weak(rename_to = pane)]
            self,
            move || pane.dispatch(CommandId::ContactShowMail)
        ));
        imp.detail.connect_compose(glib::clone!(
            #[weak(rename_to = pane)]
            self,
            move || pane.dispatch(CommandId::ContactCompose)
        ));

        // A delivery can land a turn after it was asked for (the model holds
        // changes made while it is answering `item()`), so the cursor is
        // looked at again once the rows are really in.
        model.connect_filled(glib::clone!(
            #[weak(rename_to = pane)]
            self,
            move |_| pane.cursor_moved()
        ));
        imp.model.replace(Some(model));
        imp.selection.replace(Some(selection));
        imp.list.replace(Some(list));
        imp.scroller.replace(Some(scroller));
        self.show_view_text();
    }

    fn bind_row(&self, item: &glib::Object) {
        let Some(item) = item.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        let (Some(view), Some(held)) = (
            item.child().and_downcast::<ContactRowView>(),
            item.item().and_downcast::<ContactItem>(),
        ) else {
            return;
        };
        let draw = glib::clone!(
            #[weak(rename_to = pane)]
            self,
            #[weak]
            view,
            #[weak]
            item,
            move |held: &ContactItem| {
                let row = held.row();
                let marked = row
                    .as_ref()
                    .is_some_and(|row| pane.imp().marks.borrow().contains(&row.id));
                item.set_accessible_label(
                    &row.as_ref()
                        .map(super::row::spoken)
                        .unwrap_or_else(|| "Loading".to_owned()),
                );
                view.set_selected(marked);
                view.set_first(item.position() == 0);
                view.set_row(row);
            }
        );
        draw(&held);
        view.set_cursor(item.is_selected());
        let handler = held.connect_changed(draw);
        self.imp()
            .watched
            .borrow_mut()
            .insert(item.as_ptr() as usize, (held, handler));
    }

    // -- Taking and giving back the pane ------------------------------------

    /// Puts the screen in `window`'s reading pane, hidden until it is opened.
    pub fn mount(&self, window: &Window) {
        self.imp().window.set(Some(window));
        window.shell().reader().append(self);
        window
            .shell()
            .register_reader_occupant(crate::shell::ReaderOccupant::Contacts, self.upcast_ref());
        window.connect_command(glib::clone!(
            #[weak(rename_to = pane)]
            self,
            move |id| {
                if pane.is_open() {
                    pane.dispatch(id);
                }
            }
        ));
    }

    /// Whether the screen has the pane.
    pub fn is_open(&self) -> bool {
        self.imp().open.get()
    }

    /// Opens the screen over the reading pane and asks for its people.
    pub fn open(&self) {
        if self.is_open() {
            self.focus_list();
            return;
        }
        let Some(window) = self.imp().window.upgrade() else {
            return;
        };
        let shell = window.shell();
        self.imp()
            .restore
            .set(Some((window.context(), shell.focused_pane())));
        self.imp().open.set(true);
        shell.set_contacts_open(true);
        shell.set_focused_pane(Pane::Reader);
        shell.add_css_class(CONTACTS_OPEN_CLASS);
        window.set_context(Context::Contacts);
        self.query_changed();
        self.focus_list();
    }

    /// Gives the pane back to whatever is active now, and the keyboard back
    /// to where it was.
    pub fn close(&self) {
        if !self.is_open() {
            return;
        }
        let Some(window) = self.imp().window.upgrade() else {
            return;
        };
        self.imp().open.set(false);
        window.shell().set_contacts_open(false);
        window.shell().remove_css_class(CONTACTS_OPEN_CLASS);
        if let Some((context, pane)) = self.imp().restore.take() {
            window.set_context(context);
            window.shell().set_focused_pane(pane);
        }
        // The keyboard may be in the filter, about to be a hidden entry --
        // the composer's reason for the same two lines (`release_pane`).
        gtk::prelude::GtkWindowExt::set_focus(&window, None::<&gtk::Widget>);
        window.shell().grab_focus();
    }

    fn focus_list(&self) {
        if let Some(list) = self.imp().list.borrow().as_ref() {
            list.grab_focus();
        }
    }

    // -- What the app answers with -------------------------------------------

    /// Called with the view and filter whenever either changes, and when the
    /// screen opens: the answer is [`reset`](Self::reset) with the view's
    /// length, or [`show_rows`](Self::show_rows) for a filter.
    pub fn connect_query(&self, handler: impl Fn(ContactView, String) + 'static) {
        self.imp()
            .query_handlers
            .borrow_mut()
            .push(Box::new(handler));
    }

    /// Called when the list needs a page of `view`, as it stood at the
    /// generation given; the answer is [`deliver`](Self::deliver).
    pub fn connect_page(&self, handler: impl Fn(ContactView, u64, u32) + 'static) {
        self.imp()
            .page_handlers
            .borrow_mut()
            .push(Box::new(handler));
    }

    /// Called when the cursor lands on someone, or on no one; the answer is
    /// [`set_detail`](Self::set_detail).
    pub fn connect_cursor(&self, handler: impl Fn(Option<ContactId>) + 'static) {
        self.imp()
            .cursor_handlers
            .borrow_mut()
            .push(Box::new(handler));
    }

    /// Called with the person whose mail is asked for (`contact_show_mail`).
    pub fn connect_show_mail(&self, handler: impl Fn(ContactId) + 'static) {
        self.imp()
            .show_mail_handlers
            .borrow_mut()
            .push(Box::new(handler));
    }

    /// Called with the person to write to (`contact_compose`).
    pub fn connect_compose(&self, handler: impl Fn(ContactId) + 'static) {
        self.imp()
            .compose_handlers
            .borrow_mut()
            .push(Box::new(handler));
    }

    /// Starts the list over at `total` people and returns the generation the
    /// pages it asks for will carry.
    pub fn reset(&self, total: u32) -> u64 {
        // A new list: whoever the cursor lands on is asked about afresh.
        self.imp().asked.set(None);
        let generation = self.model().reset(total);
        self.show_counts(total);
        let at = self
            .imp()
            .keep
            .take()
            .unwrap_or(0)
            .min(total.saturating_sub(1));
        if total > 0
            && let Some(selection) = self.imp().selection.borrow().as_ref()
        {
            selection.set_selected(at);
        }
        self.cursor_moved();
        generation
    }

    /// A page the list asked for.
    pub fn deliver(&self, generation: u64, page: u32, rows: Vec<ContactListRow>, total: u32) {
        self.model().deliver(generation, page, rows, total);
    }

    /// A page whose read failed, to be asked for again.
    pub fn abandon(&self, generation: u64, page: u32) {
        self.model().abandon(generation, page);
    }

    /// A filtered list, whole: at most the cap's worth, so it arrives at once.
    pub fn show_rows(&self, rows: Vec<ContactListRow>) {
        let total = rows.len() as u32;
        let generation = self.reset(total);
        let page = postio_ui::list::PAGE_SIZE as usize;
        for (index, chunk) in rows.chunks(page).enumerate() {
            self.deliver(generation, index as u32, chunk.to_vec(), total);
        }
    }

    /// What the detail column shows.
    pub fn set_detail(&self, detail: Option<ContactDetail>) {
        self.imp().detail.set_detail(detail);
    }

    /// The person the cursor is on, if their row has arrived.
    pub fn cursor_person(&self) -> Option<ContactListRow> {
        let selection = self.imp().selection.borrow().clone()?;
        let position = selection.selected();
        if position == gtk::INVALID_LIST_POSITION {
            return None;
        }
        self.model().row(position)
    }

    /// The view in force.
    pub fn view(&self) -> ContactView {
        self.imp().view.get()
    }

    /// The filter text in force.
    pub fn filter_text(&self) -> String {
        self.imp().filter.text().to_string()
    }

    /// The people marked with `x`, which a join acts on.
    pub fn marked(&self) -> Vec<ContactId> {
        self.imp().marks.borrow().iter().copied().collect()
    }

    /// The model, for a test that wants to count what the list holds.
    pub fn model(&self) -> ContactsModel {
        self.imp()
            .model
            .borrow()
            .clone()
            .expect("built in constructed")
    }

    /// The filter entry.
    pub fn filter(&self) -> gtk::SearchEntry {
        self.imp().filter.clone()
    }

    /// The line the screen shows when a command meant nothing on the focused
    /// row, or when the list is empty -- what a test reads to see what a
    /// person would see.
    pub fn hint_text(&self) -> String {
        self.imp().hint.text().to_string()
    }

    /// What the empty state says, when it is showing.
    pub fn empty_text(&self) -> Option<String> {
        let empty = &self.imp().empty;
        empty.is_visible().then(|| empty.text().to_string())
    }

    // -- Commands ------------------------------------------------------------

    /// Acts on the commands the Contacts screen owns while it is open.
    pub fn dispatch(&self, id: CommandId) {
        use postio_ui::contacts::{RowKind, applies};
        if let Err(hint) = applies(id, RowKind::Person) {
            self.say(hint.0);
            return;
        }
        match id {
            CommandId::Back => self.close(),
            CommandId::ContactsFilter => {
                self.imp().filter.grab_focus();
            }
            CommandId::ContactsToggleEveryone => {
                let next = match self.view() {
                    ContactView::Written => ContactView::Everyone,
                    ContactView::Everyone | ContactView::Deleted => ContactView::Written,
                };
                self.set_view(next);
            }
            CommandId::ContactShowMail => match self.cursor_person() {
                Some(person) => {
                    for handler in self.imp().show_mail_handlers.borrow().iter() {
                        handler(person.id);
                    }
                }
                None => self.say("Choose someone first"),
            },
            CommandId::ContactCompose => match self.cursor_person() {
                Some(person) => {
                    for handler in self.imp().compose_handlers.borrow().iter() {
                        handler(person.id);
                    }
                }
                None => self.say("Choose someone first"),
            },
            CommandId::ToggleSelection => {
                if let Some(person) = self.cursor_person() {
                    let mut marks = self.imp().marks.borrow_mut();
                    if !marks.remove(&person.id) {
                        marks.insert(person.id);
                    }
                }
                self.redraw_rows();
            }
            _ => {}
        }
    }

    /// Reads the view again, for an address book that changed under it,
    /// keeping the cursor at the position it had.
    pub fn refresh(&self) {
        if !self.is_open() {
            return;
        }
        let at = self
            .imp()
            .selection
            .borrow()
            .as_ref()
            .map(|selection| selection.selected())
            .filter(|at| *at != gtk::INVALID_LIST_POSITION);
        self.imp().keep.set(at);
        self.query_changed();
    }

    /// Puts the cursor on the row at `position`, as the arrow keys do.
    #[doc(hidden)]
    pub fn set_cursor(&self, position: u32) {
        if let Some(selection) = self.imp().selection.borrow().as_ref() {
            selection.set_selected(position);
        }
    }

    /// Switches to `view` and asks for its people.
    pub fn set_view(&self, view: ContactView) {
        if self.imp().view.replace(view) != view {
            self.imp().marks.borrow_mut().clear();
            self.show_view_text();
            self.query_changed();
        }
    }

    fn query_changed(&self) {
        if !self.is_open() {
            return;
        }
        let view = self.view();
        let filter = self.filter_text();
        for handler in self.imp().query_handlers.borrow().iter() {
            handler(view, filter.clone());
        }
    }

    fn cursor_moved(&self) {
        let person = self.cursor_person().map(|row| row.id);
        if self.imp().asked.replace(Some(person)) == Some(person) {
            return;
        }
        for handler in self.imp().cursor_handlers.borrow().iter() {
            handler(person);
        }
        if person.is_none() {
            self.imp().detail.set_detail(None);
        }
    }

    fn say(&self, text: &str) {
        let hint = &self.imp().hint;
        hint.set_text(text);
        hint.set_visible(!text.is_empty());
    }

    fn redraw_rows(&self) {
        if let Some(list) = self.imp().list.borrow().as_ref() {
            // Rebinding visible rows is how the marks reach them: the marks
            // are the pane's, not the model's.
            let model = self.model();
            let total = model.n_items();
            model.items_changed(0, total, total);
            let _ = list;
        }
    }

    fn show_view_text(&self) {
        let imp = self.imp();
        let (title, empty) = match self.view() {
            ContactView::Written => (
                "Contacts",
                "No one yet. People you write to appear here, and v e shows everyone from mail.",
            ),
            ContactView::Everyone => (
                "Everyone from mail",
                "No one has written yet. People appear here as mail arrives.",
            ),
            ContactView::Deleted => ("Deleted", "Nobody has been deleted."),
        };
        imp.title.set_text(title);
        imp.empty.set_text(empty);
    }

    fn show_counts(&self, total: u32) {
        let imp = self.imp();
        let filtered = !self.filter_text().trim().is_empty();
        let meta = match (filtered, total) {
            (true, 0) => "Nobody matches".to_owned(),
            (true, 1) => "1 match".to_owned(),
            (true, n) => format!("{n} matches"),
            (false, 0) => "Nobody yet".to_owned(),
            (false, 1) => "1 person".to_owned(),
            (false, n) => format!("{n} people"),
        };
        imp.meta.set_text(&meta);
        imp.empty.set_visible(total == 0 && !filtered);
        // With nobody listed there is nobody to show in detail, and a column
        // saying "Nobody chosen" beside an empty list says it twice.
        imp.detail.set_visible(total > 0);
        if let Some(scroller) = imp.scroller.borrow().as_ref() {
            scroller.set_visible(total > 0 || filtered);
        }
    }
}
