//! What a screen reader is told: the view is a text box with a name and GTK's own accessible text
//! (caret, selection, text); each object in it carries the name a sighted reader gets from its look.

use gtk::prelude::*;

pub(super) const BODY: &str = "Message body";

pub(super) fn name_view(view: &gtk::TextView) {
    view.update_property(&[gtk::accessible::Property::Label(BODY)]);
}

pub(super) fn name_object(widget: &impl IsA<gtk::Accessible>, name: &str) {
    widget.update_property(&[gtk::accessible::Property::Label(name)]);
}
