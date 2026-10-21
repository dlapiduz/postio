//! The Contacts view's widgets: the strip, the "Worth a look" line, the
//! list beside the card, and the footer. It draws what the controller says
//! (`postio_focus::Intent::Contacts*`) and reports each click as the input a
//! key would have been.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use gtk::prelude::*;
use postio_core::CommandId;
use postio_focus::Input;
use postio_ui::contact_edit::EditView;
use postio_ui::contact_merge::MergeSheetView;
use postio_ui::contacts_view::{CardView, RowView, StripView};
use postio_widgets::widgets::button::{self, Kind, Size};
use postio_widgets::widgets::keyhint::{self, KeyLine};
use postio_widgets::widgets::space::{S1, S2, S3, S4};

use super::card::{self, CardHooks};
use super::form::{self, Form, FormHooks, Slot};
use super::list::{self, ContactsList};
use super::merge::{self, MergeHooks};
use crate::list::Feed;

/// A row's height without a letter heading over it.
const ROW_HEIGHT: i32 = 52;

/// Runs a command, as its key would have.
type Act = Rc<dyn Fn(CommandId)>;

/// The view. See the module.
pub struct ContactsView {
    root: gtk::Box,
    /// What sits over the body: the merge sheet.
    shell: gtk::Overlay,
    feed: Feed,
    act: Act,
    title: gtk::Label,
    title_key: gtk::Box,
    total: gtk::Label,
    tabs: gtk::Box,
    filter: gtk::Entry,
    filter_key: gtk::Box,
    new_button: gtk::Button,
    review: gtk::Box,
    model: ContactsList,
    selection: gtk::SingleSelection,
    list: gtk::ListView,
    card_host: gtk::Stack,
    card_scroller: gtk::ScrolledWindow,
    footer: KeyLine,
    status: gtk::Label,
    /// What the card was last drawn from, so a redraw can be told from a
    /// repeat.
    card: RefCell<Option<CardView>>,
    /// The Edit form, while one is open.
    form: RefCell<Option<Form>>,
    /// The form the controller last asked for, until it is drawn.
    pending_form: RefCell<Option<EditView>>,
    /// Where the controller last said the keyboard goes in the form.
    form_focus: Cell<Option<Slot>>,
    /// The sheet over the view, while there is one.
    merge: RefCell<Option<gtk::Widget>>,
    /// The footer's hints as the frame set them, for when a form hands the
    /// footer back.
    frame_hints: RefCell<Vec<postio_ui::hints::Hint>>,
    /// Set while the controller moves the cursor, so the selection model's
    /// answer is not reported back as a click.
    placing: Cell<bool>,
    /// Set while the controller sets the filter field's text.
    typing_echo: Cell<bool>,
    me: RefCell<Weak<ContactsView>>,
}

impl ContactsView {
    /// The view, drawing rows and cards from `feed`'s controller. `act` runs
    /// a command as its key would.
    pub fn new(feed: Feed, act: impl Fn(CommandId) + 'static) -> Rc<Self> {
        let act: Act = Rc::new(act);

        let title = gtk::Label::new(None);
        title.add_css_class("focus-contacts-title");
        let title_key = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        let total = gtk::Label::new(None);
        total.add_css_class("dim-label");
        total.add_css_class("focus-contacts-total");
        let tabs = gtk::Box::new(gtk::Orientation::Horizontal, S1);
        tabs.add_css_class("focus-contacts-tabs");
        let filter = gtk::Entry::new();
        filter.add_css_class("focus-contacts-filter");
        filter.set_primary_icon_name(Some("system-search-symbolic"));
        filter.set_width_chars(22);
        let filter_key = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        let filter_box = gtk::Box::new(gtk::Orientation::Horizontal, S1);
        filter_box.append(&filter);
        filter_box.append(&filter_key);
        let new_button = button::button("New contact", Kind::Secondary, Size::Regular);

        for centred in [
            title.upcast_ref::<gtk::Widget>(),
            total.upcast_ref(),
            tabs.upcast_ref(),
            filter_box.upcast_ref(),
            new_button.upcast_ref(),
        ] {
            centred.set_valign(gtk::Align::Center);
        }
        let strip = gtk::Box::new(gtk::Orientation::Horizontal, S3);
        strip.add_css_class("focus-contacts-strip");
        strip.append(&title);
        strip.append(&title_key);
        strip.append(&total);
        strip.append(&tabs);
        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        strip.append(&spacer);
        strip.append(&filter_box);
        strip.append(&new_button);

        let review = gtk::Box::new(gtk::Orientation::Horizontal, S2);
        review.add_css_class("focus-contacts-review");
        review.set_visible(false);

        let model = ContactsList::default();
        let selection = gtk::SingleSelection::new(Some(model.clone()));
        selection.set_autoselect(false);
        selection.set_can_unselect(true);
        let factory = gtk::SignalListItemFactory::new();
        let list = gtk::ListView::new(Some(selection.clone()), Some(factory.clone()));
        list.add_css_class("focus-contacts-list");
        list.set_single_click_activate(false);
        list.set_accessible_role(gtk::AccessibleRole::List);
        list.update_property(&[gtk::accessible::Property::Label("Contacts")]);
        let scrolled = gtk::ScrolledWindow::builder()
            .child(&list)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .width_request(440)
            .build();

        let card_scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .hexpand(true)
            .vexpand(true)
            .build();
        // The card, or the Edit form in its place: no transition (pane
        // switches are instant).
        let card_host = gtk::Stack::new();
        card_host.set_transition_type(gtk::StackTransitionType::None);
        card_host.set_hexpand(true);
        card_host.set_vexpand(true);
        card_host.add_css_class("focus-contacts-card-host");
        card_host.add_named(&card_scroller, Some("card"));

        let divider = gtk::Separator::new(gtk::Orientation::Vertical);
        let body = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        body.append(&scrolled);
        body.append(&divider);
        body.append(&card_host);

        let footer = KeyLine::new("focus-contacts-footer-hints");
        let status = gtk::Label::new(None);
        status.add_css_class("dim-label");
        status.add_css_class("focus-contacts-status");
        let foot = gtk::Box::new(gtk::Orientation::Horizontal, S4);
        foot.add_css_class("focus-contacts-footer");
        footer.widget().set_hexpand(true);
        foot.append(footer.widget());
        foot.append(&status);

        let page = gtk::Box::new(gtk::Orientation::Vertical, 0);
        page.add_css_class("focus-contacts");
        page.append(&strip);
        page.append(&review);
        page.append(&body);
        page.set_vexpand(true);
        let shell = gtk::Overlay::new();
        shell.set_child(Some(&page));

        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.add_css_class("focus-contacts-root");
        root.append(&shell);
        root.append(&foot);
        root.update_property(&[gtk::accessible::Property::Label("Contacts")]);

        let view = Rc::new(ContactsView {
            root,
            shell,
            feed,
            act,
            title,
            title_key,
            total,
            tabs,
            filter,
            filter_key,
            new_button,
            review,
            model,
            selection,
            list,
            card_host,
            card_scroller,
            footer,
            status,
            card: RefCell::default(),
            form: RefCell::default(),
            pending_form: RefCell::default(),
            form_focus: Cell::default(),
            merge: RefCell::default(),
            frame_hints: RefCell::default(),
            placing: Cell::new(false),
            typing_echo: Cell::new(false),
            me: RefCell::default(),
        });
        view.me.replace(Rc::downgrade(&view));
        view.connect(&factory);
        view
    }

    fn weak(&self) -> Weak<ContactsView> {
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
                let position = item.item().as_ref().and_then(list::position_of);
                let row = position.and_then(|position| view.feed.contacts_row(position));
                item.set_child(Some(&row_widget(row.as_ref())));
                if let Some(row) = &row {
                    item.set_accessible_label(&format!("{}, {}", row.name, row.address));
                }
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
                    view.feed.input(Input::ContactsPoint(at));
                }
            }
        });
        self.filter.connect_changed({
            let weak = weak.clone();
            move |entry| {
                let Some(view) = weak.upgrade() else {
                    return;
                };
                if view.typing_echo.get() {
                    return;
                }
                view.feed
                    .input(Input::ContactsFilterTyped(entry.text().to_string()));
            }
        });
        // Return in the field is "I am done typing": the keyboard goes back
        // to the list.
        self.filter.connect_activate({
            let weak = weak.clone();
            move |_| {
                if let Some(view) = weak.upgrade() {
                    view.list.grab_focus();
                }
            }
        });
        self.new_button.connect_clicked({
            let weak = weak.clone();
            move |_| {
                if let Some(view) = weak.upgrade() {
                    (view.act)(CommandId::ContactNew);
                }
            }
        });
    }

    /// The commands the view, its card, its form, its sheet and its popover
    /// have a control for.
    pub fn controls() -> Vec<CommandId> {
        vec![
            CommandId::ContactNew,
            CommandId::ContactsSuggestions,
            CommandId::ContactShowMail,
            CommandId::ContactCompose,
            CommandId::ContactEdit,
            CommandId::ContactSave,
            CommandId::SuggestionDismiss,
            CommandId::ContactOpenCard,
            CommandId::ContactAdd,
            CommandId::Back,
        ]
    }

    /// The view, for the window's pages.
    pub fn widget(&self) -> &gtk::Box {
        &self.root
    }

    /// What lies over the body: the merge sheet and the Edit form go here.
    pub fn overlay(&self) -> &gtk::Overlay {
        &self.shell
    }

    /// Put the keyboard on the list.
    pub fn focus_list(&self) {
        self.list.grab_focus();
    }

    /// Put the keyboard in the filter field: `/`.
    pub fn focus_filter(&self) {
        self.filter.grab_focus();
        self.filter.set_position(-1);
    }

    /// Draw the frame the controller sent.
    pub fn show_frame(&self, view: &postio_focus::ContactsView) {
        self.show_strip(&view.strip);
        self.model.set_total(view.total);
        if self.filter.text().as_str() != view.filter {
            self.typing_echo.set(true);
            self.filter.set_text(&view.filter);
            self.filter.set_position(-1);
            self.typing_echo.set(false);
        }
        self.frame_hints.replace(view.footer.hints.clone());
        if self.form.borrow().is_none() && self.merge.borrow().is_none() {
            self.footer.set(&view.footer.hints);
        }
        self.status.set_text(&view.footer.status);
        self.show_cursor(self.feed.contacts_cursor());
    }

    fn show_strip(&self, strip: &StripView) {
        self.title.set_text(&strip.title);
        clear(&self.title_key);
        if let Some(key) = &strip.key {
            self.title_key.append(&keyhint::cap(key));
        }
        self.total.set_text(&strip.total);
        clear(&self.tabs);
        for tab in &strip.tabs {
            let one = gtk::Button::new();
            one.add_css_class("flat");
            one.add_css_class("focus-contacts-tab");
            if tab.on {
                one.add_css_class("focus-contacts-tab-on");
            }
            let words = gtk::Box::new(gtk::Orientation::Horizontal, S2);
            words.append(&gtk::Label::new(Some(&tab.title)));
            if let Some(count) = &tab.count {
                let count = gtk::Label::new(Some(count));
                count.add_css_class("dim-label");
                words.append(&count);
            }
            one.set_child(Some(&words));
            one.update_property(&[gtk::accessible::Property::Label(&tab.title)]);
            let weak = self.weak();
            let which = tab.tab;
            one.connect_clicked(move |_| {
                if let Some(view) = weak.upgrade() {
                    view.feed.input(Input::ContactsTab(which));
                }
            });
            self.tabs.append(&one);
        }
        self.filter
            .set_placeholder_text(Some(&strip.filter_placeholder));
        clear(&self.filter_key);
        if let Some(key) = &strip.filter_key {
            self.filter_key.append(&keyhint::framed_cap(key));
        }
        keyhint::dress(&self.new_button, &strip.new_label, strip.new_key.as_deref());
        self.new_button.set_sensitive(strip.new_enabled);

        clear(&self.review);
        self.review.set_visible(strip.review.is_some());
        if let Some(line) = &strip.review {
            let dot = gtk::Label::new(Some("\u{25cf}"));
            dot.add_css_class("dim-label");
            let lead = gtk::Label::new(Some(&line.lead));
            lead.add_css_class("focus-contacts-review-lead");
            let text = gtk::Label::new(Some(&line.text));
            text.add_css_class("dim-label");
            text.set_hexpand(true);
            text.set_xalign(0.0);
            text.set_ellipsize(gtk::pango::EllipsizeMode::End);
            let open = button::button(&line.button, Kind::Ghost, Size::Small);
            keyhint::dress(&open, &line.button, line.key.as_deref());
            open.set_sensitive(line.enabled);
            let weak = self.weak();
            open.connect_clicked(move |_| {
                if let Some(view) = weak.upgrade() {
                    (view.act)(CommandId::ContactsSuggestions);
                }
            });
            self.review.append(&dot);
            self.review.append(&lead);
            self.review.append(&text);
            self.review.append(&open);
        }
    }

    /// Rows landed, or the list started over: draw them again.
    pub fn show_rows(&self) {
        let total = self.feed.contacts_total();
        if self.model.n_items() == total {
            self.model.refresh();
        } else {
            self.model.set_total(total);
        }
        self.show_cursor(self.feed.contacts_cursor());
    }

    /// The cursor is on `index` now.
    pub fn show_cursor(&self, index: Option<u32>) {
        self.placing.set(true);
        match index {
            Some(at) if at < self.model.n_items() => {
                self.selection.set_selected(at);
                self.list.scroll_to(at, gtk::ListScrollFlags::NONE, None);
            }
            _ => {
                self.selection.set_selected(gtk::INVALID_LIST_POSITION);
            }
        }
        self.placing.set(false);
    }

    /// The card, whole; `None` while it is read.
    pub fn show_card(&self, card: Option<CardView>) {
        if *self.card.borrow() == card {
            return;
        }
        match &card {
            Some(card) => {
                let hooks = self.card_hooks();
                self.card_scroller
                    .set_child(Some(&card::build(card, &hooks)));
            }
            None => self.card_scroller.set_child(None::<&gtk::Widget>),
        }
        self.card.replace(card);
    }

    /// The Edit form, whole; `None` puts it away and the card is back.
    pub fn show_edit(&self, edit: Option<EditView>) {
        let Some(edit) = edit else {
            self.pending_form.take();
            self.close_form();
            return;
        };
        // Drawn once the signal that asked for it has returned: a letter
        // typed in a field redraws the form, and a field cannot be taken
        // out of the window while it is still announcing its own change.
        if self.pending_form.replace(Some(edit)).is_none() {
            let weak = self.weak();
            gtk::glib::idle_add_local_once(move || {
                if let Some(view) = weak.upgrade()
                    && let Some(edit) = view.pending_form.take()
                {
                    view.draw_form(edit);
                }
            });
        }
    }

    fn draw_form(&self, edit: EditView) {
        let before = self.form.borrow().as_ref().and_then(Form::focus);
        let weak = self.weak();
        let hooks = FormHooks {
            change: Rc::new({
                let weak = weak.clone();
                move |change| {
                    if let Some(view) = weak.upgrade() {
                        view.feed.input(Input::ContactEditChange(change));
                    }
                }
            }),
            command: Rc::new({
                let act = Rc::clone(&self.act);
                move |command| act(command)
            }),
        };
        let built = form::build(&edit, &hooks);
        if let Some(old) = self.card_host.child_by_name("form") {
            self.card_host.remove(&old);
        }
        self.card_host.add_named(built.widget(), Some("form"));
        self.card_host.set_visible_child_name("form");
        self.footer.set(&edit.hints);
        // The keyboard stays where it was, caret and all, unless the
        // controller moved it (a new address row, the group picker).
        let wanted: Slot = edit.focus.into();
        match (before, self.form_focus.get() == Some(wanted)) {
            (Some((slot, at)), true) => built.put_focus(slot, Some(at)),
            _ => built.put_focus(wanted, None),
        }
        self.form_focus.set(Some(wanted));
        self.form.replace(Some(built));
    }

    fn close_form(&self) {
        if self.form.borrow().is_none() {
            return;
        }
        self.form.replace(None);
        self.form_focus.set(None);
        self.card_host.set_visible_child_name("card");
        if let Some(old) = self.card_host.child_by_name("form") {
            self.card_host.remove(&old);
        }
        self.footer.set(&self.frame_hints.borrow().clone());
        self.list.grab_focus();
    }

    /// The merge sheet over the view, or put away.
    pub fn show_merge(&self, sheet: Option<MergeSheetView>) {
        if let Some(old) = self.merge.take() {
            self.shell.remove_overlay(&old);
        }
        let Some(sheet) = sheet else {
            if self.form.borrow().is_none() {
                self.footer.set(&self.frame_hints.borrow().clone());
            }
            return;
        };
        // Under the sheet the footer teaches the sheet's keys, not the list's.
        let hints: Vec<postio_ui::hints::Hint> = [&sheet.merge, &sheet.later, &sheet.not_same]
            .into_iter()
            .filter_map(|button| {
                button.key.as_ref().map(|key| postio_ui::hints::Hint {
                    key: key.clone(),
                    label: button.label.to_lowercase(),
                })
            })
            .collect();
        self.footer.set(&hints);
        let weak = self.weak();
        let hooks = MergeHooks {
            keep: Rc::new({
                let weak = weak.clone();
                move |index| {
                    if let Some(view) = weak.upgrade() {
                        view.feed.input(Input::ContactMergeKeep(index));
                    }
                }
            }),
            command: Rc::new({
                let act = Rc::clone(&self.act);
                move |command| act(command)
            }),
        };
        let widget = merge::build(&sheet, &hooks);
        self.shell.add_overlay(&widget);
        self.merge.replace(Some(widget));
    }

    fn card_hooks(&self) -> CardHooks {
        let act = Rc::clone(&self.act);
        let weak = self.weak();
        CardHooks {
            run: Rc::new(move |command| act(command)),
            toggle: Rc::new({
                let weak = weak.clone();
                move |index| {
                    if let Some(view) = weak.upgrade() {
                        view.feed.input(Input::ContactCardSwitch(index));
                    }
                }
            }),
            split: Rc::new({
                let weak = weak.clone();
                move |address| {
                    if let Some(view) = weak.upgrade() {
                        view.feed.input(Input::ContactSplit(address));
                    }
                }
            }),
            arrival: Rc::new({
                let weak = weak.clone();
                move |arrival| {
                    if let Some(view) = weak.upgrade() {
                        view.feed.input(Input::ContactArrival(arrival));
                    }
                }
            }),
        }
    }

    // What a test reads.

    /// The tabs as drawn, with the one chosen.
    pub fn tabs(&self) -> Vec<(String, bool)> {
        let mut tabs = Vec::new();
        let mut child = self.tabs.first_child();
        while let Some(tab) = child {
            child = tab.next_sibling();
            let words = texts_of(&tab);
            tabs.push((
                words.first().cloned().unwrap_or_default(),
                tab.has_css_class("focus-contacts-tab-on"),
            ));
        }
        tabs
    }

    /// How many people the list holds.
    pub fn total(&self) -> u32 {
        self.model.n_items()
    }

    /// The name of the person at `index`, once its page has landed.
    pub fn name_at(&self, index: u32) -> Option<String> {
        self.feed.contacts_row(index).map(|row| row.name)
    }

    /// The names of the first `count` people the list holds.
    pub fn names(&self, count: u32) -> Vec<String> {
        (0..count.min(self.total()))
            .filter_map(|index| self.name_at(index))
            .collect()
    }

    /// The row the cursor is on.
    pub fn cursor(&self) -> Option<u32> {
        let at = self.selection.selected();
        (at != gtk::INVALID_LIST_POSITION).then_some(at)
    }

    /// The name on the card, once it is read.
    pub fn card_name(&self) -> Option<String> {
        self.card.borrow().as_ref().map(|card| card.name.clone())
    }

    /// The card, as last drawn.
    pub fn card(&self) -> Option<CardView> {
        self.card.borrow().clone()
    }

    /// Every text the Edit form shows, while one is open.
    pub fn form_texts(&self) -> Vec<String> {
        self.form
            .borrow()
            .as_ref()
            .map(form::texts)
            .unwrap_or_default()
    }

    /// The text of the form's field in `slot`.
    pub fn form_text(&self, slot: Slot) -> Option<String> {
        self.form
            .borrow()
            .as_ref()
            .and_then(|form| form.text_of(slot))
    }

    /// Type into the form's field as a person would.
    pub fn test_type_form(&self, slot: Slot, text: &str) {
        // Not under the borrow: a typed letter redraws the form.
        let entry = self
            .form
            .borrow()
            .as_ref()
            .and_then(|form| form.entry_of(slot));
        if let Some(entry) = entry {
            entry.set_text(text);
        }
    }

    /// Whether the Edit form is up.
    pub fn has_form(&self) -> bool {
        self.form.borrow().is_some()
    }

    /// Every text the merge sheet shows, while there is one.
    pub fn merge_texts(&self) -> Vec<String> {
        self.merge
            .borrow()
            .as_ref()
            .map(merge::texts)
            .unwrap_or_default()
    }

    /// What the filter field holds.
    pub fn filter_text(&self) -> String {
        self.filter.text().to_string()
    }

    /// The "Worth a look" line, when there is one.
    pub fn review_text(&self) -> Option<String> {
        self.review
            .is_visible()
            .then(|| texts_of(&self.review).join(" "))
    }

    /// The footer's counts.
    pub fn status(&self) -> String {
        self.status.text().to_string()
    }

    /// Every text the view shows, in order.
    pub fn texts(&self) -> Vec<String> {
        texts_of(&self.root)
    }

    /// Type `text` into the filter as a person would: the field changes and
    /// says so.
    pub fn test_type_filter(&self, text: &str) {
        self.filter.set_text(text);
    }

    /// Click the tab titled `title`.
    pub fn test_click_tab(&self, title: &str) {
        let mut child = self.tabs.first_child();
        while let Some(tab) = child {
            child = tab.next_sibling();
            if texts_of(&tab).first().map(String::as_str) == Some(title)
                && let Some(button) = tab.downcast_ref::<gtk::Button>()
            {
                button.emit_clicked();
            }
        }
    }

    /// Whether the keyboard is in the filter field.
    pub fn filter_has_focus(&self) -> bool {
        self.filter.focus_child().is_some() || self.filter.has_focus()
    }
}

/// Take every child out of `widget`.
fn clear(widget: &gtk::Box) {
    while let Some(child) = widget.first_child() {
        widget.remove(&child);
    }
}

/// Every visible label under `root`, in reading order.
pub(crate) fn texts_of(root: &impl IsA<gtk::Widget>) -> Vec<String> {
    let mut said = Vec::new();
    let mut stack = vec![root.clone().upcast::<gtk::Widget>()];
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

/// A row of the list: the avatar, the name with its tag, the address in the
/// mono face, and on the right when and what the mail adds up to. A letter
/// heading sits over the first row of a letter. `None` is a row whose page is
/// on its way: the same height, nothing in it.
fn row_widget(row: Option<&RowView>) -> gtk::Box {
    let outer = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let Some(row) = row else {
        outer.set_size_request(-1, ROW_HEIGHT);
        return outer;
    };
    if let Some(letter) = row.letter {
        let heading = gtk::Label::new(Some(&letter.to_string()));
        heading.add_css_class("focus-contacts-letter");
        heading.set_xalign(0.0);
        outer.append(&heading);
    }
    let line = gtk::Box::new(gtk::Orientation::Horizontal, S3);
    line.add_css_class("focus-contacts-row");
    line.set_size_request(-1, ROW_HEIGHT);
    let avatar = super::avatar::widget(&row.avatar, 32);
    if row.marked {
        let both = gtk::Overlay::new();
        both.set_child(Some(&avatar));
        let mark = gtk::Image::from_icon_name("object-select-symbolic");
        mark.add_css_class("focus-contacts-mark");
        mark.set_halign(gtk::Align::End);
        mark.set_valign(gtk::Align::End);
        mark.update_property(&[gtk::accessible::Property::Label("Marked")]);
        both.add_overlay(&mark);
        line.append(&both);
    } else {
        line.append(&avatar);
    }
    let words = gtk::Box::new(gtk::Orientation::Vertical, S1);
    words.set_hexpand(true);
    words.set_valign(gtk::Align::Center);
    let top = gtk::Box::new(gtk::Orientation::Horizontal, S2);
    let name = gtk::Label::new(Some(&row.name));
    name.add_css_class("focus-contacts-name");
    name.set_xalign(0.0);
    name.set_ellipsize(gtk::pango::EllipsizeMode::End);
    top.append(&name);
    if let Some(tag) = &row.tag {
        let tag = gtk::Label::new(Some(tag));
        tag.add_css_class("focus-contacts-tag");
        top.append(&tag);
    }
    let address = gtk::Label::new(Some(&row.address));
    address.add_css_class("focus-contact-mono");
    address.add_css_class("dim-label");
    address.set_xalign(0.0);
    address.set_ellipsize(gtk::pango::EllipsizeMode::End);
    words.append(&top);
    words.append(&address);
    line.append(&words);
    let right = gtk::Box::new(gtk::Orientation::Vertical, S1);
    right.set_valign(gtk::Align::Center);
    let when = gtk::Label::new(Some(&row.when));
    when.set_xalign(1.0);
    let count = gtk::Label::new(Some(&row.count_line));
    count.add_css_class("dim-label");
    count.add_css_class("focus-contact-small");
    count.set_xalign(1.0);
    right.append(&when);
    right.append(&count);
    line.append(&right);
    outer.append(&line);
    outer
}
