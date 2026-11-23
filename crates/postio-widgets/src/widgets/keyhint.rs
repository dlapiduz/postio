//! Drawing a key hint: the cap, the labelled cap and the line of them. What a hint says is
//! `postio_ui::hints`; this is only how it looks.
//! Every cap is `Presentation` to a screen reader: the key is already in the control's name or
//! tooltip.

use adw::prelude::*;
use postio_ui::hints::{self, Hint};

pub fn cap(key: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(key));
    label.add_css_class("postio-keyhint");
    // Its own height: stretched to a tall row, a framed cap becomes a second box doubling the
    // control's height.
    label.set_valign(gtk::Align::Center);
    label.set_accessible_role(gtk::AccessibleRole::Presentation);
    label
}

/// Marks a `dress`ed button: once it has a child, `GtkButton::label` is empty.
const DRESSED: &str = "postio-keyhint-dressed";

/// `None` draws the words alone: a blank cap reads as a key that exists and does nothing.
pub fn dress(button: &gtk::Button, label: &str, key: Option<&str>) {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    row.append(&gtk::Label::new(Some(label)));
    if let Some(key) = key {
        row.append(&cap(key));
    }
    button.set_child(Some(&row));
    button.add_css_class(DRESSED);
    button.update_property(&[gtk::accessible::Property::Label(label)]);
}

fn button_words(button: &gtk::Button) -> Option<String> {
    if let Some(label) = button.label() {
        return Some(label.to_string());
    }
    button
        .has_css_class(DRESSED)
        .then(|| button.child())
        .flatten()
        .and_then(|row| row.first_child())
        .and_downcast::<gtk::Label>()
        .map(|label| label.label().to_string())
}

/// For widgets whose buttons take only a label (`AdwToast` action, `AdwBanner`). Dressed and icon
/// buttons are left alone.
pub fn dress_labelled_buttons(root: &gtk::Widget, key: &str) {
    let mut child = root.first_child();
    while let Some(widget) = child {
        child = widget.next_sibling();
        if let Some(button) = widget.downcast_ref::<gtk::Button>()
            && let Some(label) = button_words(button)
        {
            dress(button, &label, Some(key));
        } else {
            dress_labelled_buttons(&widget, key);
        }
    }
}

/// Framed because nothing beside it says it is a key.
pub fn framed_cap(key: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(key));
    label.add_css_class("postio-key");
    label.set_valign(gtk::Align::Center);
    label.set_accessible_role(gtk::AccessibleRole::Presentation);
    label
}

/// `None` draws the words alone.
pub fn labelled(text: &str, key: Option<&str>) -> gtk::Widget {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    row.add_css_class("postio-keyhint-labelled");
    row.append(&gtk::Label::new(Some(text)));
    if let Some(key) = key {
        row.append(&cap(key));
    }
    row.upcast()
}

/// `class` is the surface's own name so a test finds this chip.
pub fn chip(hint: &Hint, class: &str) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    row.add_css_class("postio-keyhint-chip");
    row.add_css_class(class);
    row.set_accessible_role(gtk::AccessibleRole::Presentation);

    let label = gtk::Label::new(Some(&hint.label));
    label.add_css_class("postio-keyhint-label");
    // A label that cannot wrap pushes whatever is beside it off the end of a narrow column.
    label.set_wrap(true);
    label.set_xalign(0.0);
    label.set_accessible_role(gtk::AccessibleRole::Presentation);
    row.append(&label);
    row.append(&cap(&hint.key));
    row
}

/// Rewritten whole by `set`, which a rebind calls, so it cannot keep a key the user has moved.
#[derive(Clone)]
pub struct KeyLine {
    label: gtk::Label,
}

impl KeyLine {
    pub fn new(class: &str) -> Self {
        let label = gtk::Label::new(None);
        label.add_css_class("postio-keyline");
        label.add_css_class(class);
        label.set_xalign(0.0);
        label.set_wrap(true);
        label.set_accessible_role(gtk::AccessibleRole::Presentation);
        Self { label }
    }

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

    pub fn set<'a>(&self, hints: impl IntoIterator<Item = &'a Hint>) {
        self.label.set_text(&hints::line(hints));
    }

    pub fn widget(&self) -> &gtk::Label {
        &self.label
    }
}
