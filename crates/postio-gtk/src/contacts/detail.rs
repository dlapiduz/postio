//! One person, in detail (specs/005-contacts FR-006): their name, every
//! address with the preferred one marked, organisation, note, groups, how
//! many messages involve them and when they were last in touch -- and the two
//! things to do next, as buttons so the mouse has them too.

use std::cell::RefCell;

use adw::prelude::*;
use adw::subclass::prelude::*;
use chrono::Local;
use gtk::glib;
use postio_model::ContactDetail;

type Handler = Box<dyn Fn()>;

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct DetailView {
        pub name: gtk::Label,
        pub organization: gtk::Label,
        pub addresses: gtk::ListBox,
        pub facts: gtk::Label,
        pub groups: gtk::Label,
        pub note: gtk::Label,
        pub actions: gtk::Box,
        pub nobody: gtk::Label,
        pub detail: RefCell<Option<ContactDetail>>,
        pub show_mail: RefCell<Vec<Handler>>,
        pub compose: RefCell<Vec<Handler>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for DetailView {
        const NAME: &'static str = "PostioContactDetail";
        type Type = super::DetailView;
        type ParentType = gtk::Box;
    }

    impl ObjectImpl for DetailView {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().build();
        }
    }

    impl WidgetImpl for DetailView {}
    impl BoxImpl for DetailView {}
}

glib::wrapper! {
    /// The Contacts screen's detail column.
    pub struct DetailView(ObjectSubclass<imp::DetailView>)
        @extends gtk::Box, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Orientable;
}

impl Default for DetailView {
    fn default() -> Self {
        glib::Object::builder()
            .property("orientation", gtk::Orientation::Vertical)
            .property("spacing", 8)
            .build()
    }
}

impl DetailView {
    fn build(&self) {
        let imp = self.imp();
        self.add_css_class("postio-contact-detail");
        // A column of its own width beside the list, which takes the rest:
        // the list is what a person scans, and wrapping labels would otherwise
        // ask for every pixel they could wrap into.
        self.set_hexpand(false);
        self.set_size_request(320, -1);
        for (label, class) in [
            (&imp.name, "postio-contact-name"),
            (&imp.organization, "postio-contact-organization"),
            (&imp.facts, "postio-contact-facts"),
            (&imp.groups, "postio-contact-groups"),
            (&imp.note, "postio-contact-note"),
            (&imp.nobody, "postio-contact-nobody"),
        ] {
            label.set_xalign(0.0);
            label.set_wrap(true);
            label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
            label.set_max_width_chars(34);
            label.set_selectable(false);
            label.add_css_class(class);
        }
        // Rows the keyboard can land on: `-` and `*` act on the address
        // that has it (contracts/commands.md, "dispatch on the focused row").
        imp.addresses.set_selection_mode(gtk::SelectionMode::Browse);
        imp.addresses.add_css_class("postio-contact-addresses");
        imp.addresses
            .update_property(&[gtk::accessible::Property::Label("Addresses")]);

        let show_mail = gtk::Button::with_label("Show mail");
        show_mail.add_css_class("postio-contact-action");
        show_mail.connect_clicked(glib::clone!(
            #[weak(rename_to = detail)]
            self,
            move |_| {
                for handler in detail.imp().show_mail.borrow().iter() {
                    handler();
                }
            }
        ));
        let compose = gtk::Button::with_label("Write to");
        compose.add_css_class("postio-contact-action");
        compose.connect_clicked(glib::clone!(
            #[weak(rename_to = detail)]
            self,
            move |_| {
                for handler in detail.imp().compose.borrow().iter() {
                    handler();
                }
            }
        ));
        imp.actions.set_orientation(gtk::Orientation::Horizontal);
        imp.actions.set_spacing(8);
        imp.actions.append(&show_mail);
        imp.actions.append(&compose);

        imp.nobody.set_text("Nobody chosen");
        for widget in [
            imp.name.upcast_ref::<gtk::Widget>(),
            imp.organization.upcast_ref(),
            imp.addresses.upcast_ref(),
            imp.facts.upcast_ref(),
            imp.groups.upcast_ref(),
            imp.note.upcast_ref(),
            imp.actions.upcast_ref(),
            imp.nobody.upcast_ref(),
        ] {
            self.append(widget);
        }
        self.set_detail(None);
    }

    /// Shows `detail`, or the empty state for no one.
    pub fn set_detail(&self, detail: Option<ContactDetail>) {
        let imp = self.imp();
        imp.addresses.remove_all();
        let Some(detail) = detail else {
            for widget in [
                imp.name.upcast_ref::<gtk::Widget>(),
                imp.organization.upcast_ref(),
                imp.addresses.upcast_ref(),
                imp.facts.upcast_ref(),
                imp.groups.upcast_ref(),
                imp.note.upcast_ref(),
                imp.actions.upcast_ref(),
            ] {
                widget.set_visible(false);
            }
            imp.nobody.set_visible(true);
            imp.detail.replace(None);
            return;
        };
        let person = &detail.person;
        imp.nobody.set_visible(false);
        imp.name.set_text(person.display_name());
        imp.name.set_visible(true);
        imp.actions.set_visible(true);

        let organization = person.organization.clone().unwrap_or_default();
        imp.organization.set_text(&organization);
        imp.organization
            .set_visible(!organization.trim().is_empty());

        for owned in &person.addresses {
            let preferred = owned.id == person.preferred;
            let text = if preferred && person.addresses.len() > 1 {
                format!("{}  · preferred", owned.address.address)
            } else {
                owned.address.address.clone()
            };
            let label = gtk::Label::new(Some(&text));
            label.set_xalign(0.0);
            label.set_selectable(true);
            label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
            label.set_max_width_chars(34);
            label.add_css_class("postio-contact-address");
            if preferred {
                label.add_css_class("preferred");
            }
            imp.addresses.append(&label);
        }
        imp.addresses.set_visible(true);

        imp.facts.set_text(&facts(&detail));
        imp.facts.set_visible(true);

        imp.groups.set_text(&match detail.groups.as_slice() {
            [] => String::new(),
            groups => format!("In {}", groups.join(", ")),
        });
        imp.groups.set_visible(!detail.groups.is_empty());

        let note = person.note.clone().unwrap_or_default();
        imp.note.set_text(&note);
        imp.note.set_visible(!note.trim().is_empty());
        imp.detail.replace(Some(detail));
    }

    /// Whether the column is stacked under the list rather than beside it:
    /// full width then, with the rule on top instead of the left.
    pub fn set_stacked(&self, stacked: bool) {
        if stacked {
            self.set_size_request(-1, -1);
            self.add_css_class("stacked");
        } else {
            self.set_size_request(320, -1);
            self.remove_css_class("stacked");
        }
    }

    /// The person shown, if anyone.
    pub fn detail(&self) -> Option<ContactDetail> {
        self.imp().detail.borrow().clone()
    }

    /// The addresses as drawn, for a test that reads what a person would see.
    pub fn address_lines(&self) -> Vec<String> {
        let mut lines = Vec::new();
        let mut index = 0;
        while let Some(row) = self.imp().addresses.row_at_index(index) {
            if let Some(label) = row.child().and_downcast::<gtk::Label>() {
                lines.push(label.text().to_string());
            }
            index += 1;
        }
        lines
    }

    /// Puts the keyboard on the address at `position`.
    pub fn focus_address(&self, position: usize) {
        let addresses = &self.imp().addresses;
        if let Some(row) = i32::try_from(position)
            .ok()
            .and_then(|at| addresses.row_at_index(at))
        {
            addresses.select_row(Some(&row));
            row.grab_focus();
        }
    }

    /// The address the keyboard is on, when it is on one: what `-` and `*`
    /// act on. `None` while the keyboard is anywhere else, so a key pressed
    /// in the list never acts on an address the user cannot see chosen.
    pub fn focused_address(&self) -> Option<postio_model::AddressId> {
        let imp = self.imp();
        let row = imp.addresses.selected_row()?;
        let within = imp.addresses.focus_child().is_some() || row.has_focus();
        if !within {
            return None;
        }
        let detail = imp.detail.borrow();
        let person = &detail.as_ref()?.person;
        person
            .addresses
            .get(usize::try_from(row.index()).ok()?)
            .map(|owned| owned.id)
    }

    /// The facts line as drawn.
    pub fn facts_text(&self) -> String {
        self.imp().facts.text().to_string()
    }

    /// The name as drawn.
    pub fn name_text(&self) -> String {
        self.imp().name.text().to_string()
    }

    /// Called when "Show mail" is pressed.
    pub fn connect_show_mail(&self, handler: impl Fn() + 'static) {
        self.imp().show_mail.borrow_mut().push(Box::new(handler));
    }

    /// Called when "Write to" is pressed.
    pub fn connect_compose(&self, handler: impl Fn() + 'static) {
        self.imp().compose.borrow_mut().push(Box::new(handler));
    }
}

/// `12 messages · last in touch Thu`, or `No mail yet` for someone the user
/// made who has never written.
fn facts(detail: &ContactDetail) -> String {
    let messages = match detail.messages {
        0 => return "No mail yet".to_owned(),
        1 => "1 message".to_owned(),
        n => format!("{n} messages"),
    };
    match detail.person.last_seen_at {
        Some(at) => format!(
            "{messages} · last in touch {}",
            postio_ui::row::timestamp(at, Local::now())
        ),
        None => messages,
    }
}
