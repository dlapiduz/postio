//! The key map (screen 20; contracts/focus-surface.md, "The key map"): an
//! `AdwDialog`, 1100×760, generated from the registry and the groups table.
//!
//! Nothing here names a key. Every row is `postio_ui::keymap_sheet::key_map`
//! under the keymap in force, so a `[keys]` rebind shows here the moment it
//! reaches the keyboard, and a command registered for Focus appears with no
//! code of Focus's (US7 scenario 2). `?` or `Escape` closes it; the window's
//! keyboard sends both here while it is open.

use gtk::prelude::*;
use postio_core::{CommandId, Keymap};
use postio_ui::hints;
use postio_ui::keymap_sheet::{self, KeyMapRow};
use postio_widgets::widgets::keyhint;
use postio_widgets::widgets::space::{S1, S2, S3, S4, S6};

/// The dialog's size, as screen 20 draws it.
const WIDTH: i32 = 1100;
const HEIGHT: i32 = 760;
/// How many columns the groups are laid out in.
const COLUMNS: usize = 4;

/// Build the key map for `keymap`.
pub fn build(keymap: &Keymap) -> adw::Dialog {
    let title = gtk::Label::new(Some("Keys"));
    title.add_css_class("focus-keymap-heading");
    let subtitle = gtk::Label::new(Some(
        "Single keys act on the focused row, or on the selection if there is one. \
         On macOS, Ctrl becomes \u{2318}.",
    ));
    subtitle.add_css_class("dim-label");
    subtitle.set_xalign(0.0);
    subtitle.set_hexpand(true);
    subtitle.set_wrap(true);
    let close = gtk::Box::new(gtk::Orientation::Horizontal, S1);
    close.set_valign(gtk::Align::Center);
    let close_keys: Vec<String> = [CommandId::CheatSheet, CommandId::Back]
        .into_iter()
        .filter_map(|command| hints::key(keymap, command))
        .collect();
    for (index, key) in close_keys.iter().enumerate() {
        if index > 0 {
            let or = gtk::Label::new(Some("or"));
            or.add_css_class("dim-label");
            close.append(&or);
        }
        close.append(&keyhint::cap(key));
    }
    let close_word = gtk::Label::new(Some("close"));
    close_word.add_css_class("dim-label");
    close.append(&close_word);
    let header = gtk::Box::new(gtk::Orientation::Horizontal, S3);
    header.append(&title);
    header.append(&subtitle);
    header.append(&close);
    let x = postio_widgets::widgets::close_button();
    x.set_valign(gtk::Align::Center);
    header.append(&x);

    let map = keymap_sheet::key_map(keymap);
    let total: usize = map.iter().map(|(_, rows)| rows.len() + 2).sum();
    let per_column = total.div_ceil(COLUMNS);
    let columns = gtk::Box::new(gtk::Orientation::Horizontal, S6);
    columns.set_homogeneous(true);
    columns.set_vexpand(true);
    let mut column = gtk::Box::new(gtk::Orientation::Vertical, S4);
    let mut filled = 0;
    for (group, rows) in &map {
        // A group stays whole: a column starts afresh rather than split one.
        if filled > 0
            && filled + rows.len() + 2 > per_column
            && columns.observe_children().n_items() + 1 < COLUMNS as u32
        {
            columns.append(&column);
            column = gtk::Box::new(gtk::Orientation::Vertical, S4);
            filled = 0;
        }
        column.append(&group_box(group.title(), rows));
        filled += rows.len() + 2;
    }
    columns.append(&column);

    let rebind = gtk::Label::new(Some(
        "Rebind anything in ~/.config/postio/config.toml under [keys]",
    ));
    rebind.add_css_class("dim-label");
    rebind.set_xalign(0.0);
    rebind.set_hexpand(true);
    let mouse = gtk::Label::new(Some(
        "The mouse works everywhere: every key has a visible button.",
    ));
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
        .title("Keys")
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
