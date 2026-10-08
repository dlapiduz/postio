//! The key map (screen 20; contracts/focus-surface.md, "The key map"): an
//! `AdwDialog`, 1100×760, generated from the registry and the groups table.
//!
//! Nothing here names a key. Every row is `postio_ui::keymap_sheet::key_map`
//! under the keymap in force, so a `[keys]` rebind shows here the moment it
//! reaches the keyboard, and a command registered for Focus appears with no
//! code of Focus's (US7 scenario 2). `?` or `Escape` closes it; the window's
//! keyboard sends both here while it is open.

use gtk::prelude::*;
use postio_core::Keymap;
use postio_ui::hints;
use postio_ui::keymap_sheet::{self, KeyMapRow};
use postio_widgets::widgets::keyhint;
use postio_widgets::widgets::space::{S1, S2, S3, S4, S6};

/// The dialog's size, as screen 20 draws it.
const WIDTH: i32 = 1100;
const HEIGHT: i32 = 760;

/// Build the key map for `keymap`.
pub fn build(keymap: &Keymap) -> adw::Dialog {
    let title = gtk::Label::new(Some(keymap_sheet::TITLE));
    title.add_css_class("focus-keymap-heading");
    let subtitle = gtk::Label::new(Some(keymap_sheet::SUBTITLE));
    subtitle.add_css_class("dim-label");
    subtitle.set_xalign(0.0);
    subtitle.set_hexpand(true);
    subtitle.set_wrap(true);
    let close = gtk::Box::new(gtk::Orientation::Horizontal, S1);
    close.set_valign(gtk::Align::Center);
    let close_keys: Vec<String> = keymap_sheet::CLOSE_COMMANDS
        .into_iter()
        .filter_map(|command| hints::key(keymap, command))
        .collect();
    for (index, key) in close_keys.iter().enumerate() {
        if index > 0 {
            let or = gtk::Label::new(Some(keymap_sheet::CLOSE_OR));
            or.add_css_class("dim-label");
            close.append(&or);
        }
        close.append(&keyhint::cap(key));
    }
    let close_word = gtk::Label::new(Some(keymap_sheet::CLOSE_WORD));
    close_word.add_css_class("dim-label");
    close.append(&close_word);
    let header = gtk::Box::new(gtk::Orientation::Horizontal, S3);
    header.append(&title);
    header.append(&subtitle);
    header.append(&close);
    let x = postio_widgets::widgets::close_button();
    x.set_valign(gtk::Align::Center);
    header.append(&x);

    let map = keymap_sheet::key_map(keymap, postio_core::Frontend::Focus);
    let columns = gtk::Box::new(gtk::Orientation::Horizontal, S6);
    columns.set_homogeneous(true);
    columns.set_vexpand(true);
    let sizes: Vec<usize> = map.iter().map(|(_, rows)| rows.len()).collect();
    for packed in keymap_sheet::pack_columns(&sizes, keymap_sheet::COLUMNS) {
        let column = gtk::Box::new(gtk::Orientation::Vertical, S4);
        for index in packed {
            let (group, rows) = &map[index];
            column.append(&group_box(group.title(), rows));
        }
        columns.append(&column);
    }

    let rebind = gtk::Label::new(Some(keymap_sheet::REBIND_FOOTER));
    rebind.add_css_class("dim-label");
    rebind.set_xalign(0.0);
    rebind.set_hexpand(true);
    let mouse = gtk::Label::new(Some(keymap_sheet::MOUSE_FOOTER));
    mouse.add_css_class("dim-label");
    let footer = gtk::Box::new(gtk::Orientation::Horizontal, S3);
    footer.add_css_class("focus-keymap-footer");
    footer.append(&rebind);
    footer.append(&mouse);

    let content = gtk::Box::new(gtk::Orientation::Vertical, S4);
    content.add_css_class("focus-keymap");
    content.append(&header);
    let scrolled = gtk::ScrolledWindow::builder()
        .child(&columns)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .build();
    content.append(&scrolled);
    content.append(&footer);

    let dialog = adw::Dialog::builder()
        .title(keymap_sheet::TITLE)
        .content_width(WIDTH)
        .content_height(HEIGHT)
        .child(&content)
        .build();
    let weak = dialog.downgrade();
    x.connect_clicked(move |_| {
        if let Some(dialog) = weak.upgrade() {
            adw::prelude::AdwDialogExt::close(&dialog);
        }
    });
    dialog
}

/// One group: its heading and its rows.
fn group_box(title: &str, rows: &[KeyMapRow]) -> gtk::Box {
    let group = gtk::Box::new(gtk::Orientation::Vertical, 0);
    group.add_css_class("focus-keymap-group");
    let heading = gtk::Label::new(Some(title));
    heading.add_css_class("focus-keymap-group-title");
    heading.set_xalign(0.0);
    heading.set_accessible_role(gtk::AccessibleRole::Heading);
    group.append(&heading);
    for row in rows {
        let line = gtk::Box::new(gtk::Orientation::Horizontal, S2);
        line.add_css_class("focus-keymap-row");
        let name = gtk::Label::new(Some(row.title));
        name.add_css_class("focus-keymap-title");
        name.set_xalign(0.0);
        name.set_hexpand(true);
        name.set_wrap(true);
        line.append(&name);
        let keys = gtk::Box::new(gtk::Orientation::Horizontal, S1);
        keys.set_valign(gtk::Align::Center);
        for key in &row.keys {
            keys.append(&keyhint::cap(key));
        }
        line.append(&keys);
        group.append(&line);
    }
    group
}
