//! Drawing a key hint: the cap, the labelled cap, and the line of them.
//!
//! What a hint *says* is [`postio_ui::hints`]: read from the keymap, never
//! typed in, absent when nothing is bound. This is only how one looks, and it
//! is here so it looks one way. Before it there were four builders (the
//! header's, a private copy in the composer, `KeycapButton`'s, and three
//! hand-built chips) and five classes for the same mono cap.
//!
//! Three shapes, because there are three places a key is named:
//!
//! * [`labelled`] -- inside a control, beside the control's own words:
//!   `Compose c`. The cap is quieter than the words; the words are the verb.
//! * [`chip`] -- on a plate or a strip, a label and a cap that are not a
//!   control: `Search all mail /`.
//! * [`KeyLine`] -- a footer of them in one mono line: `j/k walk · Return
//!   open · s save`.
//!
//! Every cap is `Presentation` to a screen reader. The key is already in the
//! control's accessible name or tooltip, and a bare "c" announced after
//! "Compose" is noise.

use adw::prelude::*;
use postio_ui::hints::{self, Hint};

/// The mono cap alone: `c`, `ctrl+Return`.
pub fn cap(key: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(key));
    label.add_css_class("postio-keyhint");
    label.set_accessible_role(gtk::AccessibleRole::Presentation);
    label
}

/// A cap standing on its own, framed: the `/` at the end of the search
/// field. Framed because nothing beside it says it is a key; a cap inside a
/// control has the control's words to lean on and stays unframed.
pub fn framed_cap(key: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(key));
    label.add_css_class("postio-key");
    label.set_accessible_role(gtk::AccessibleRole::Presentation);
    label
}

/// A control's words with its key beside them: `Compose c`.
///
/// `key` is `None` when nothing is bound, and then only the words are drawn
/// -- a blank cap would read as a key that exists and does nothing.
pub fn labelled(text: &str, key: Option<&str>) -> gtk::Widget {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    row.add_css_class("postio-keyhint-labelled");
    row.append(&gtk::Label::new(Some(text)));
    if let Some(key) = key {
        row.append(&cap(key));
    }
    row.upcast()
}

/// A hint that is not a control: what it does, then the key that does it.
///
/// `class` is the surface's own name for it, so a test can find this chip
/// and not the next surface's.
pub fn chip(hint: &Hint, class: &str) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    row.add_css_class("postio-keyhint-chip");
    row.add_css_class(class);
    row.set_accessible_role(gtk::AccessibleRole::Presentation);

    let label = gtk::Label::new(Some(&hint.label));
    label.add_css_class("postio-keyhint-label");
    // The last thing to give, and it still gives: at a narrow column even
    // one chip per line can be too wide for "Move between messages", and a
    // label that cannot wrap takes whatever is beside it off the end.
    label.set_wrap(true);
    label.set_xalign(0.0);
    label.set_accessible_role(gtk::AccessibleRole::Presentation);
    row.append(&label);
    row.append(&cap(&hint.key));
    row
}

/// A footer of hints in one line of mono: `j/k walk · Return open`.
///
/// Rewritten whole by [`set`](Self::set), which is what a rebind calls: the
/// line is built from the keymap each time, so it cannot keep a key the
/// user has moved.
#[derive(Clone)]
pub struct KeyLine {
    label: gtk::Label,
}

impl KeyLine {
    /// An empty line, findable by `class`.
    pub fn new(class: &str) -> Self {
        let label = gtk::Label::new(None);
        label.add_css_class("postio-keyline");
        label.add_css_class(class);
        label.set_xalign(0.0);
        label.set_wrap(true);
        label.set_accessible_role(gtk::AccessibleRole::Presentation);
        Self { label }
    }

    /// Wrap an existing label -- a template child, or one a surface already
    /// keeps a handle to -- as a key line.
    pub fn adopt(label: &gtk::Label, class: &str) -> Self {
        label.add_css_class("postio-keyline");
        label.add_css_class(class);
        label.set_xalign(0.0);
        label.set_wrap(true);
        label.set_accessible_role(gtk::AccessibleRole::Presentation);
        Self {
            label: label.clone(),
        }
    }

    /// Draw these hints, replacing whatever was there.
    pub fn set<'a>(&self, hints: impl IntoIterator<Item = &'a Hint>) {
        self.label.set_text(&hints::line(hints));
    }

    /// The label, to place in a layout.
    pub fn widget(&self) -> &gtk::Label {
        &self.label
    }
}
