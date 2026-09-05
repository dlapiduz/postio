//! The two labels the settings surfaces write over and over.
//!
//! Neither is a control, and neither is worth a struct: a kicker is a label
//! with a class and a stat line is a label with a different class. They are
//! here rather than inline in each pane because the *classes* are the shared
//! thing — a kicker that is 0.7rem in one pane and 0.75rem in the next is
//! precisely the drift `widgets/` exists to stop.

use adw::prelude::*;

/// A section heading in letterspaced small caps: `THEME`, `MESSAGE LIST`,
/// `SCOPES REQUESTED`.
///
/// Not a `<h2>`: it labels the group beneath it for a sighted reader, and
/// the group itself carries the accessible name a screen reader uses. Two
/// announcements of one heading is worse than one.
pub fn kicker(text: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.add_css_class("postio-kicker");
    label.set_xalign(0.0);
    label
}

/// A line of mono facts under a group: `26px rows · 41 per screen`,
/// `imap · password · 4291 msg · 1.8 GB · synced 12s`.
///
/// Mono because it is almost entirely number, and numbers in a column that
/// do not line up read as a mistake.
pub fn stat_line(text: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.add_css_class("postio-stat-line");
    label.set_xalign(0.0);
    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    label
}
