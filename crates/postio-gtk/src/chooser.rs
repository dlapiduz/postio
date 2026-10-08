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

/// What a person asked of the chooser.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pick {
    /// Open the `n`th thing offered.
    Open(usize),
    /// Save the `n`th thing offered, a part, to a file.
    Save(usize),
    /// Save every part to a folder.
    SaveAll,
}

/// What the file-chooser portal is asked for (T240).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SavePick {
    /// A file to write one part to, offered under `suggested`.
    File {
        /// The part's name, made safe to write.
        suggested: String,
    },
    /// A folder to write every part into.
    Folder,
}

/// A dialog offering `choices`, with `at` the row it opens on; `chosen` is
/// called with what a person picks, and the dialog closes.
///
/// A part's row has its own Save, and the header has Save all while there
/// is a part to save: both ask the portal, and neither does until pressed.
pub fn dialog(
    choices: &[Choice],
    at: Option<usize>,
    chosen: impl Fn(Pick) + 'static,
) -> adw::Dialog {
    let chosen = Rc::new(chosen);
    let list = gtk::ListBox::new();
    list.add_css_class("boxed-list");
    list.add_css_class("focus-open-choices");
    // Weak: the buttons live inside the dialog they close.
    let slot: Rc<glib::WeakRef<adw::Dialog>> = Rc::new(glib::WeakRef::new());
    let close = Rc::new({
        let slot = Rc::clone(&slot);
        move || {
            if let Some(dialog) = slot.upgrade() {
                dialog.close();
            }
        }
    });
    for (index, choice) in choices.iter().enumerate() {
        let (primary, secondary) = choice.said();
        let row = adw::ActionRow::builder()
            .title(glib::markup_escape_text(&primary))
            .subtitle(glib::markup_escape_text(&secondary))
            .activatable(true)
            .build();
        row.set_title_lines(1);
        row.set_subtitle_selectable(true);
        if matches!(choice, Choice::Part { .. }) {
            let save = gtk::Button::with_label("Save");
            save.add_css_class("flat");
            save.set_valign(gtk::Align::Center);
            save.set_tooltip_text(Some("Save this attachment"));
            let (chosen, close) = (Rc::clone(&chosen), Rc::clone(&close));
            save.connect_clicked(move |_| {
                chosen(Pick::Save(index));
                close();
            });
            row.add_suffix(&save);
        }
        list.append(&row);
    }
    let scrolled = gtk::ScrolledWindow::builder()
        .child(&list)
        .vexpand(true)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .build();
    let header = adw::HeaderBar::new();
    if choices
        .iter()
        .any(|choice| matches!(choice, Choice::Part { .. }))
    {
        let save_all = gtk::Button::with_label("Save all");
        save_all.set_tooltip_text(Some("Save every attachment to a folder"));
        let (chosen, close) = (Rc::clone(&chosen), Rc::clone(&close));
        save_all.connect_clicked(move |_| {
            chosen(Pick::SaveAll);
            close();
        });
        header.pack_start(&save_all);
    }
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
    slot.set(Some(&dialog));
    list.connect_row_activated({
        let chosen = Rc::clone(&chosen);
        move |_, row| {
            if let Ok(index) = usize::try_from(row.index()) {
                chosen(Pick::Open(index));
            }
            close();
        }
    });
    match at.and_then(|at| list.row_at_index(i32::try_from(at).ok()?)) {
        Some(row) => {
            list.select_row(Some(&row));
            dialog.set_focus(Some(&row));
        }
        None => dialog.set_focus(Some(&list)),
    }
    dialog
}

/// The title of the row `dialog` has selected: where a chip opened it.
pub fn selected(dialog: &adw::Dialog) -> Option<String> {
    let mut stack = vec![dialog.child()?];
    while let Some(widget) = stack.pop() {
        if let Some(list) = widget.downcast_ref::<gtk::ListBox>()
            && list.has_css_class("focus-open-choices")
        {
            let row = list.selected_row()?;
            return Some(row.downcast_ref::<adw::ActionRow>()?.title().to_string());
        }
        let mut child = widget.first_child();
        while let Some(next) = child {
            child = next.next_sibling();
            stack.push(next);
        }
    }
    None
}
