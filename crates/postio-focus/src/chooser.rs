//! `o`: open an attachment or a link (US2 scenario 9). A small dialog lists
//! the open message's links and its parts; each link shows the words it was
//! written behind and, under them, its full target -- what will open, seen
//! before it opens. Nothing opens until a row is chosen.

use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;
use postio_model::ids::AttachmentId;

/// The dialog's widget name, so it can be told from another dialog.
pub const DIALOG_NAME: &str = "focus-open-choice";

/// The dialog's size.
const WIDTH: i32 = 560;
const HEIGHT: i32 = 420;

/// One thing `o` offers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Choice {
    /// A link in the body.
    Link {
        /// The words it was written behind.
        words: String,
        /// Where it goes, in full.
        target: String,
    },
    /// A part of the message.
    Part {
        /// Its file name.
        name: String,
        /// Its size, as a person reads it.
        size: String,
        /// Which part.
        id: AttachmentId,
    },
}

impl Choice {
    /// What the row says: its words or name, and its target or size.
    pub fn said(&self) -> (String, String) {
        match self {
            Choice::Link { words, target } => (words.clone(), target.clone()),
            Choice::Part { name, size, .. } => (name.clone(), size.clone()),
        }
    }
}

/// A dialog offering `choices`; `chosen` is called with the index of the
/// one a person picks, and the dialog closes.
pub fn dialog(choices: &[Choice], chosen: impl Fn(usize) + 'static) -> adw::Dialog {
    let list = gtk::ListBox::new();
    list.add_css_class("boxed-list");
    list.add_css_class("focus-open-choices");
    for choice in choices {
        let (primary, secondary) = choice.said();
        let row = adw::ActionRow::builder()
            .title(glib::markup_escape_text(&primary))
            .subtitle(glib::markup_escape_text(&secondary))
            .activatable(true)
            .build();
        row.set_title_lines(1);
        row.set_subtitle_selectable(true);
        list.append(&row);
    }
    let scrolled = gtk::ScrolledWindow::builder()
        .child(&list)
        .vexpand(true)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .build();
    let header = adw::HeaderBar::new();
    let content = adw::ToolbarView::new();
    content.add_top_bar(&header);
    content.set_content(Some(&scrolled));
    let dialog = adw::Dialog::builder()
        .title("Open attachment or link")
        .content_width(WIDTH)
        .content_height(HEIGHT)
        .child(&content)
        .build();
    dialog.set_widget_name(DIALOG_NAME);
    let chosen = Rc::new(chosen);
    list.connect_row_activated({
        let dialog = dialog.downgrade();
        move |_, row| {
            if let Ok(index) = usize::try_from(row.index()) {
                chosen(index);
            }
            if let Some(dialog) = dialog.upgrade() {
                dialog.close();
            }
        }
    });
    dialog.set_focus(Some(&list));
    dialog
}
