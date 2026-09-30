//! A place you can go: a name, and a count beside it.
//!
//! The sidebar's folders and the search column's scopes are the same row --
//! canvas 1b's `name  count` at the folder rhythm -- and each file built it
//! by hand and then dug the two labels back out by walking siblings. This
//! builds it once and hands the labels back by name, so a caller updating a
//! count never depends on which child is where.

use adw::prelude::*;
use gtk::pango;

/// A `name  count` row, wearing `class`. Both labels start empty.
pub fn nav_row(class: &str) -> gtk::ListBoxRow {
    let name = gtk::Label::new(None);
    name.add_css_class("postio-folder-name");
    name.set_xalign(0.0);
    name.set_hexpand(true);
    name.set_ellipsize(pango::EllipsizeMode::End);

    let count = gtk::Label::new(None);
    count.add_css_class("postio-folder-count");

    let line = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    line.append(&name);
    line.append(&count);

    let row = gtk::ListBoxRow::new();
    row.add_css_class(class);
    row.set_child(Some(&line));
    row
}

/// The name label of a row [`nav_row`] built.
pub fn nav_name(row: &gtk::ListBoxRow) -> Option<gtk::Label> {
    label_with(row, "postio-folder-name")
}

/// The count label of a row [`nav_row`] built.
pub fn nav_count(row: &gtk::ListBoxRow) -> Option<gtk::Label> {
    label_with(row, "postio-folder-count")
}

fn label_with(row: &gtk::ListBoxRow, class: &str) -> Option<gtk::Label> {
    let mut child = row.child()?.first_child();
    while let Some(widget) = child {
        if widget.has_css_class(class) {
            return widget.downcast().ok();
        }
        child = widget.next_sibling();
    }
    None
}
