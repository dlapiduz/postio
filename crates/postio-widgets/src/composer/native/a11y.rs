use gtk::prelude::*;

pub(super) const BODY: &str = "Message body";

pub(super) fn name_view(view: &gtk::TextView) {
    view.update_property(&[gtk::accessible::Property::Label(BODY)]);
}

pub(super) fn name_object(widget: &impl IsA<gtk::Accessible>, name: &str) {
    widget.update_property(&[gtk::accessible::Property::Label(name)]);
}
