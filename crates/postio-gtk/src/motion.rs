//! The motion budget: a transition takes no more than 100 ms or is absent. Focus's own stylesheet
//! has none; the toolkit's revealers
//! and stacks default to 200-250 ms, so `keep_to_budget` shortens them. Reduced motion needs
//! nothing here: `gtk-enable-animations` off runs no transition at all.

use gtk::prelude::*;

pub const BUDGET_MS: u32 = 100;

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
