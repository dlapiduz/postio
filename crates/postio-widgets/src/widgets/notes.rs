//! `callout` (why something was refused) and `empty_note` (why a list is empty).
//! Both dress a label the caller keeps, since it is rewritten as state changes.

use adw::prelude::*;

pub fn callout(label: &gtk::Label, class: &str) {
    label.add_css_class("postio-callout");
    label.add_css_class(class);
    label.set_xalign(0.0);
    label.set_wrap(true);
}

pub fn empty_note(label: &gtk::Label, class: &str) {
    label.add_css_class("postio-empty-note");
    label.add_css_class(class);
    label.set_xalign(0.0);
    label.set_wrap(true);
}

pub struct ListOrEmpty;

impl ListOrEmpty {
    /// `max_height`, when given, caps the scroller and lets it shrink to its rows below that.
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

        // Visibility is left to the caller: whether "nothing here" is true before rows arrive
        // differs by list.
        empty_note(empty, &format!("{class}-empty"));
    }

    pub fn show(scroller: &gtk::ScrolledWindow, empty: &gtk::Label, has_rows: bool) {
        scroller.set_visible(has_rows);
        empty.set_visible(!has_rows);
    }
}
