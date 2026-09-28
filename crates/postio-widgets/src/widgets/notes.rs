//! The two kinds of sentence a surface says about itself.
//!
//! * [`callout`] -- why something was refused: the server said no to a
//!   sign-in, the store said no to a signature name. On the accent's warning
//!   cousin, not a red banner, because nothing has gone wrong with Postio.
//!   Onboarding and the signature editor each styled their own, with
//!   different padding for the same thing.
//! * [`empty_note`] -- why a list has nothing in it: "No senders are always
//!   allowed", "Folders appear after the first sync". Six of these were bare
//!   labels in whatever ink the pane happened to set, beside two that were
//!   styled; this is the one quiet line they all are.
//!
//! Both dress a label the caller keeps, because both are rewritten as the
//! state behind them changes.

use adw::prelude::*;

/// Dress `label` as a refusal: wrapped, left, on the warning ground.
pub fn callout(label: &gtk::Label, class: &str) {
    label.add_css_class("postio-callout");
    label.add_css_class(class);
    label.set_xalign(0.0);
    label.set_wrap(true);
}

/// Dress `label` as a list's empty line: wrapped, left, faint.
pub fn empty_note(label: &gtk::Label, class: &str) {
    label.add_css_class("postio-empty-note");
    label.add_css_class(class);
    label.set_xalign(0.0);
    label.set_wrap(true);
}

/// A list that says so when it is empty.
///
/// The settings panes had this block four times over -- a `ListBox` in a
/// scroller, a label beside it, and `set_visible` on both, inverted, wherever
/// the rows were refilled. [`dress`](Self::dress) is the setup and
/// [`show`](Self::show) the toggle.
pub struct ListOrEmpty;

impl ListOrEmpty {
    /// Set up `list` inside `scroller`, with `empty` beside it.
    ///
    /// `name` is the list's accessible name; `max_height`, when given, caps
    /// the scroller at that and lets it shrink to its rows below it.
    pub fn dress(
        list: &gtk::ListBox,
        scroller: &gtk::ScrolledWindow,
        empty: &gtk::Label,
        class: &str,
        name: &str,
        max_height: Option<i32>,
    ) {
        list.add_css_class(&format!("{class}-list"));
        list.set_selection_mode(gtk::SelectionMode::None);
        list.update_property(&[gtk::accessible::Property::Label(name)]);

        scroller.set_child(Some(list));
        scroller.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        match max_height {
            Some(height) => {
                scroller.set_max_content_height(height);
                scroller.set_propagate_natural_height(true);
            }
            None => scroller.set_vexpand(true),
        }
        scroller.add_css_class(class);

        // Visibility is left to the caller: whether "nothing here" is true
        // before the rows have been handed over differs by list.
        empty_note(empty, &format!("{class}-empty"));
    }

    /// Show the list when it has rows, and the empty line when it has none.
    pub fn show(scroller: &gtk::ScrolledWindow, empty: &gtk::Label, has_rows: bool) {
        scroller.set_visible(has_rows);
        empty.set_visible(!has_rows);
    }
}
