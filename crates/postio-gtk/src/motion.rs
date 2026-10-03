//! The motion budget (FR-094, T143): a transition takes no more than
//! 100 ms, or is absent.
//!
//! Focus's own stylesheet has no transition at all. What moves comes from
//! the toolkit: a banner's revealer, an action bar's, a menu's sliding
//! stack, each 200-250 ms by default. [`keep_to_budget`] shortens every
//! revealer and stack under a surface to the budget when the surface is
//! built or shown.
//!
//! Reduced motion needs nothing here: the desktop's "reduce animation" sets
//! `gtk-enable-animations` off, and GTK and libadwaita then run no
//! transition at all, these included.

use gtk::prelude::*;

/// The longest a transition may run, in milliseconds.
pub const BUDGET_MS: u32 = 100;

/// Shorten every revealer's and stack's transition under `root` to
/// [`BUDGET_MS`]. What is already within it is left alone.
pub fn keep_to_budget(root: &impl IsA<gtk::Widget>) {
    let mut stack = vec![root.as_ref().clone()];
    while let Some(widget) = stack.pop() {
        if let Some(revealer) = widget.downcast_ref::<gtk::Revealer>()
            && revealer.transition_duration() > BUDGET_MS
        {
            revealer.set_transition_duration(BUDGET_MS);
        }
        if let Some(pages) = widget.downcast_ref::<gtk::Stack>()
            && pages.transition_duration() > BUDGET_MS
        {
            pages.set_transition_duration(BUDGET_MS);
        }
        let mut child = widget.first_child();
        while let Some(next) = child {
            child = next.next_sibling();
            stack.push(next);
        }
    }
}
