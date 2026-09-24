//! The contact editor (specs/005-contacts FR-020, FR-021): a name, and --
//! for a new person -- the address they are reached at, their organisation
//! and a note.
//!
//! Inline in the detail column like the join panel, and saved with `Return`
//! from any of its fields. Addresses after the first are added, detached and
//! preferred with their own keys, because each is a claim about who owns
//! what that the store may have to refuse or ask about.

use gtk::prelude::*;

/// The editor's widgets.
#[derive(Debug, Clone)]
pub struct ContactEditor {
    root: gtk::Box,
    title: gtk::Label,
    name: gtk::Entry,
    address: gtk::Entry,
    organization: gtk::Entry,
    note: gtk::Entry,
    error: gtk::Label,
}

impl Default for ContactEditor {
    fn default() -> Self {
        Self::new()
    }
}

impl ContactEditor {
    /// Builds the editor, empty.
    pub fn new() -> Self {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 8);
        root.add_css_class("postio-contact-detail");
        root.add_css_class("postio-contact-editor");
        let title = gtk::Label::new(None);
        title.set_xalign(0.0);
        title.add_css_class("postio-contact-name");
        let entry = |placeholder: &str, label: &str| {
            let entry = gtk::Entry::new();
            entry.set_placeholder_text(Some(placeholder));
            entry.update_property(&[gtk::accessible::Property::Label(label)]);
            entry
        };
        let name = entry("Name", "Name");
        let address = entry("name@example.com", "Address");
        let organization = entry("Organisation", "Organisation");
        let note = entry("Note", "Note");
        let error = gtk::Label::new(None);
        error.set_xalign(0.0);
        error.set_wrap(true);
        error.add_css_class("postio-contacts-hint");
        error.add_css_class("error");
        let hint = gtk::Label::new(Some("Return to save · Esc to cancel"));
        hint.set_xalign(0.0);
        hint.add_css_class("postio-contacts-hint");
        for widget in [
            title.upcast_ref::<gtk::Widget>(),
            name.upcast_ref(),
            address.upcast_ref(),
            organization.upcast_ref(),
            note.upcast_ref(),
            error.upcast_ref(),
            hint.upcast_ref(),
        ] {
            root.append(widget);
        }
        Self {
            root,
            title,
            name,
            address,
            organization,
            note,
            error,
        }
    }

    /// The editor as a widget.
    pub fn widget(&self) -> &gtk::Box {
        &self.root
    }

    /// Empty, for a new person: the address field shown.
    pub fn start_new(&self) {
        self.title.set_text("New contact");
        for entry in self.entries() {
            entry.set_text("");
        }
        self.address.set_visible(true);
        self.set_error("");
        self.name.grab_focus();
    }

    /// Filled from `person`, for an edit: no address field.
    pub fn start_edit(&self, person: &postio_model::Contact) {
        self.title.set_text("Edit contact");
        self.name
            .set_text(person.name.as_deref().unwrap_or(person.display_name()));
        self.organization
            .set_text(person.organization.as_deref().unwrap_or_default());
        self.note
            .set_text(person.note.as_deref().unwrap_or_default());
        self.address.set_text("");
        self.address.set_visible(false);
        self.set_error("");
        self.name.grab_focus();
    }

    /// Says why the editor cannot save yet, or clears it.
    pub fn set_error(&self, text: &str) {
        self.error.set_text(text);
        self.error.set_visible(!text.is_empty());
    }

    /// The reason on the line, if any.
    pub fn error_text(&self) -> String {
        self.error.text().to_string()
    }

    /// Calls `save` when `Return` is pressed in any field.
    pub fn connect_save(&self, save: impl Fn() + Clone + 'static) {
        for entry in self.entries() {
            let save = save.clone();
            entry.connect_activate(move |_| save());
        }
    }

    fn entries(&self) -> [&gtk::Entry; 4] {
        [&self.name, &self.address, &self.organization, &self.note]
    }

    /// The name field.
    pub fn name_entry(&self) -> &gtk::Entry {
        &self.name
    }

    /// The address field, shown for a new person only.
    pub fn address_entry(&self) -> &gtk::Entry {
        &self.address
    }

    /// The organisation field.
    pub fn organization_entry(&self) -> &gtk::Entry {
        &self.organization
    }

    /// The note field.
    pub fn note_entry(&self) -> &gtk::Entry {
        &self.note
    }
}
