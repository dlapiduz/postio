//! A labelled field: the caption labels the control and is itself presentation, so a screen reader
//! hears it once, as the name.

use adw::prelude::*;

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
