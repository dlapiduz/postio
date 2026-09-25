//! A plate: the bordered, lifted surface an overlay is drawn on.
//!
//! Five surfaces float over the window -- the box's results, the cheat
//! sheet, the parts panel, the unavailable screen and onboarding -- and each
//! drew its own: the same border, radius and ground typed five times, a
//! literal `rgba(0, 0, 0, 0.18)` shadow that bypassed the token scale (and so
//! never darkened in the dark scheme), headers of 42px in one and 44px in the
//! next, `Group` on three and `Dialog` on one, and an `Escape` controller
//! written out wherever one was wanted.
//!
//! This is that surface, once:
//!
//! * [`dress`] makes a widget a plate: `.postio-plate` (hairline border,
//!   `radius-md`, the surface ground and `--postio-shadow-lg`), centred over
//!   what it covers, and a `Dialog` to a screen reader under the name given --
//!   each of these takes the keyboard while it is up, which is what a dialog
//!   is.
//! * [`header`] is its title bar: the canvas' 44px strip over a hairline, the
//!   title in the plate-title face (12px Barlow Condensed, 0.16em, capitals),
//!   with room after it for whatever the surface says about itself.
//! * [`connect_escape`] is the one way a plate hears `Escape`, in the capture
//!   phase so a focused entry inside it cannot swallow the key first.
//!
//! [`surface`] is the look alone, for the box's results, which hang from the
//! search field rather than sitting centred and are a list rather than a
//! dialog.

use adw::prelude::*;
use gtk::glib;

/// The plate's look alone: border, radius, ground, lift.
pub fn surface(widget: &impl IsA<gtk::Widget>, class: &str) {
    widget.add_css_class("postio-plate");
    widget.add_css_class(class);
}

/// Make `widget` a plate named `name`: the look, centred over what it
/// covers, and a dialog to assistive technology.
pub fn dress(widget: &impl IsA<gtk::Widget>, class: &str, name: &str) {
    surface(widget, class);
    let widget: &gtk::Widget = widget.upcast_ref();
    widget.set_halign(gtk::Align::Center);
    widget.set_valign(gtk::Align::Center);
    widget.set_accessible_role(gtk::AccessibleRole::Dialog);
    widget.update_property(&[gtk::accessible::Property::Label(name)]);
}

/// A plate's title bar, carrying `title`; append anything else to it.
///
/// `class` is the surface's own, and the bar also wears `{class}-header`
/// so a surface can reach it without reaching every plate's.
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

/// Run `handler` when `Escape` is pressed anywhere in the plate.
///
/// `handler` says whether it did anything: `false` lets the key go on to
/// the window, for a plate where `Escape` means something only some of the
/// time (onboarding, while it is waiting on a browser).
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
