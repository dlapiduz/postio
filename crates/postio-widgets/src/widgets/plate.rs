//! A plate: the bordered, lifted surface an overlay is drawn on, so overlays share one border,
//! radius, ground and shadow token.
//! `dress` makes a widget a plate and a `Dialog` to a screen reader; `connect_escape` listens in
//! the capture phase so a focused entry cannot swallow `Escape`.

use adw::prelude::*;
use gtk::glib;

pub fn surface(widget: &impl IsA<gtk::Widget>, class: &str) {
    widget.add_css_class("postio-plate");
    widget.add_css_class(class);
}

pub fn dress(widget: &impl IsA<gtk::Widget>, class: &str, name: &str) {
    surface(widget, class);
    let widget: &gtk::Widget = widget.upcast_ref();
    widget.set_halign(gtk::Align::Center);
    widget.set_valign(gtk::Align::Center);
    widget.set_accessible_role(gtk::AccessibleRole::Dialog);
    widget.update_property(&[gtk::accessible::Property::Label(name)]);
}

/// The bar also wears `{class}-header` so a surface can reach it without reaching every plate's.
pub fn header(class: &str, title: &str) -> gtk::Box {
    let bar = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    bar.add_css_class("postio-plate-header");
    bar.add_css_class(&format!("{class}-header"));

    // Presentation: the plate itself carries the name, and a heading read
    // out again after "dialog, Parts" is the same word twice.
    let label = gtk::Label::new(Some(title));
    label.add_css_class("postio-plate-title");
    label.set_xalign(0.0);
    label.set_accessible_role(gtk::AccessibleRole::Presentation);
    bar.append(&label);
    bar
}

/// `handler` returns whether it did anything; `false` lets `Escape` reach the window.
pub fn connect_escape(widget: &impl IsA<gtk::Widget>, handler: impl Fn() -> bool + 'static) {
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    keys.connect_key_pressed(move |_, key, _, _| {
        if key == gtk::gdk::Key::Escape && handler() {
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });
    widget.add_controller(keys);
}
