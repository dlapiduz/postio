//! The kicker and stat-line labels: shared classes so no pane drifts in size.

use adw::prelude::*;

/// Written in sentence case and drawn in capitals by the stylesheet. Not a heading role: the
/// group beneath carries the accessible name, so a screen reader hears it once.
pub fn kicker(text: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.add_css_class("postio-kicker");
    label.set_xalign(0.0);
    label
}

/// Mono because it is almost all numbers, which must line up.
pub fn stat_line(text: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.add_css_class("postio-stat-line");
    label.set_xalign(0.0);
    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    label
}
