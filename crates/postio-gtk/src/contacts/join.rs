//! The join panel (specs/005-contacts FR-012, FR-013): which name the joined
//! person keeps, and -- only when the people disagree -- which organisation.
//!
//! Inline in the detail column rather than a dialog: it is answered from the
//! keyboard the pane already has, `Return` taking the preselected name, so a
//! join is `x`, down, `x`, `m`, `Return` (SC-004). The choices themselves are
//! `postio_ui::contacts::join_name_choices`', proven there without a widget.

use gtk::prelude::*;
use postio_ui::contacts::JoinChoices;

/// The panel's widgets.
#[derive(Debug, Clone)]
pub struct JoinPanel {
    root: gtk::Box,
    names: gtk::ListBox,
    organizations_label: gtk::Label,
    organizations: gtk::ListBox,
}

impl Default for JoinPanel {
    fn default() -> Self {
        Self::new()
    }
}

impl JoinPanel {
    /// Builds the panel, empty.
    pub fn new() -> Self {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 8);
        root.add_css_class("postio-contact-detail");
        root.add_css_class("postio-contact-join");
        let title = gtk::Label::new(Some("Join as"));
        title.set_xalign(0.0);
        title.add_css_class("postio-contact-name");
        let names = choice_list("Name to keep");
        let organizations_label = gtk::Label::new(Some("Organisation"));
        organizations_label.set_xalign(0.0);
        organizations_label.add_css_class("postio-contact-facts");
        let organizations = choice_list("Organisation to keep");
        let hint = gtk::Label::new(Some("Return to join · Esc to cancel"));
        hint.set_xalign(0.0);
        hint.add_css_class("postio-contacts-hint");
        for widget in [
            title.upcast_ref::<gtk::Widget>(),
            names.upcast_ref(),
            organizations_label.upcast_ref(),
            organizations.upcast_ref(),
            hint.upcast_ref(),
        ] {
            root.append(widget);
        }
        Self {
            root,
            names,
            organizations_label,
            organizations,
        }
    }

    /// The panel as a widget.
    pub fn widget(&self) -> &gtk::Box {
        &self.root
    }

    /// Offers `choices`, the first name selected; organisations only when
    /// they conflict.
    pub fn set_choices(&self, choices: &JoinChoices) {
        fill(&self.names, &choices.names);
        let conflict = choices.organization_conflict();
        fill(
            &self.organizations,
            if conflict {
                &choices.organizations
            } else {
                &[]
            },
        );
        self.organizations_label.set_visible(conflict);
        self.organizations.set_visible(conflict);
    }

    /// Puts the keyboard on the preselected name.
    pub fn focus(&self) {
        if let Some(row) = self.names.selected_row() {
            row.grab_focus();
        }
    }

    /// The name chosen.
    pub fn name(&self) -> Option<String> {
        chosen(&self.names)
    }

    /// The organisation chosen, when the panel asked.
    pub fn organization(&self) -> Option<String> {
        self.organizations
            .is_visible()
            .then(|| chosen(&self.organizations))
            .flatten()
    }

    /// The names offered, in order.
    pub fn names(&self) -> Vec<String> {
        texts(&self.names)
    }

    /// The organisations offered, in order; empty when there is no conflict.
    pub fn organizations(&self) -> Vec<String> {
        if self.organizations.is_visible() {
            texts(&self.organizations)
        } else {
            Vec::new()
        }
    }
}

fn choice_list(label: &str) -> gtk::ListBox {
    let list = gtk::ListBox::new();
    list.set_selection_mode(gtk::SelectionMode::Browse);
    list.add_css_class("postio-contact-choices");
    list.update_property(&[gtk::accessible::Property::Label(label)]);
    list
}

fn fill(list: &gtk::ListBox, values: &[String]) {
    list.remove_all();
    for value in values {
        let label = gtk::Label::new(Some(value));
        label.set_xalign(0.0);
        label.set_ellipsize(gtk::pango::EllipsizeMode::End);
        list.append(&label);
    }
    if let Some(first) = list.row_at_index(0) {
        list.select_row(Some(&first));
    }
}

fn text_of(row: &gtk::ListBoxRow) -> Option<String> {
    row.child()
        .and_downcast::<gtk::Label>()
        .map(|label| label.text().to_string())
}

fn chosen(list: &gtk::ListBox) -> Option<String> {
    list.selected_row().as_ref().and_then(text_of)
}

fn texts(list: &gtk::ListBox) -> Vec<String> {
    let mut all = Vec::new();
    let mut index = 0;
    while let Some(row) = list.row_at_index(index) {
        all.extend(text_of(&row));
        index += 1;
    }
    all
}
