//! Keycaps as a screen reader meets them: a cap inside a button gives that button its key as
//! `KeyShortcuts` (ARIA spelling), never as text; a free-standing cap is content and is left alone.

use gtk::prelude::*;

const CAP: &str = "postio-keyhint";

/// Cheap to rerun whenever a surface is drawn or the keymap changes.
pub fn teach_shortcuts(root: &impl IsA<gtk::Widget>) {
    let mut stack = vec![root.as_ref().clone()];
    while let Some(widget) = stack.pop() {
        if widget.has_css_class(CAP)
            && let Some(label) = widget.downcast_ref::<gtk::Label>()
            && let Some(control) = control_of(&widget)
        {
            let key = postio_ui::hints::shortcut(&label.text());
            control.update_property(&[gtk::accessible::Property::KeyShortcuts(&key)]);
        }
        let mut child = widget.first_child();
        while let Some(next) = child {
            child = next.next_sibling();
            stack.push(next);
        }
    }
}

fn control_of(cap: &gtk::Widget) -> Option<gtk::Widget> {
    let mut up = cap.parent();
    while let Some(widget) = up {
        if widget.is::<gtk::Button>() || widget.is::<gtk::MenuButton>() {
            return Some(widget);
        }
        up = widget.parent();
    }
    None
}
