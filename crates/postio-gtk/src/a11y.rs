//! Keycaps as a screen reader should meet them (FR-096, T142): once, as
//! their control's shortcut, never as text.
//!
//! Every cap is already `Presentation` (`postio_widgets::widgets::keyhint`),
//! so it is not read out beside the control's words. What was missing is
//! the other half: the control saying which key runs it. A cap inside a
//! button gives that button its key as `KeyShortcuts`, in the ARIA spelling
//! (`postio_ui::hints::shortcut`); a cap that stands on its own -- a line of
//! hints, the key map -- is content, and is left as it is.

use gtk::prelude::*;

/// The class every keycap wears.
const CAP: &str = "postio-keyhint";

/// Give each control under `root` that carries a keycap its key as its
/// shortcut. Cheap enough to run again whenever a surface is drawn or the
/// keymap changes: a walk of the tree, and one property per control.
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

/// The button `cap` sits in, if it sits in one.
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
