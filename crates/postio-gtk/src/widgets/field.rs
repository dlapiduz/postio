//! A labelled field: a caption over the control it names.
//!
//! Three forms built this by hand -- onboarding's, the settings account
//! detail's, the composer's header rows -- and only onboarding's told a
//! screen reader that the caption *is* the control's name. The account
//! detail's hosts and ports were announced as "text field", one after
//! another, with the words that said which sitting unconnected above them.
//!
//! Here the relation is not optional: the caption labels the control, and
//! is itself presentation, so it is heard once, as the name.

use adw::prelude::*;

/// `label` over `control`, the one labelling the other. `class` is the
/// surface's own, on the column.
pub fn field(label: &str, control: &impl IsA<gtk::Widget>, class: &str) -> gtk::Box {
    let caption = gtk::Label::new(Some(label));
    caption.add_css_class("postio-field-label");
    caption.set_xalign(0.0);

    let column = gtk::Box::new(gtk::Orientation::Vertical, 4);
    column.add_css_class("postio-field");
    column.add_css_class(class);
    column.append(&caption);
    column.append(control);

    control
        .as_ref()
        .update_relation(&[gtk::accessible::Relation::LabelledBy(&[
            caption.upcast_ref()
        ])]);
    caption.set_accessible_role(gtk::AccessibleRole::Presentation);
    column
}
