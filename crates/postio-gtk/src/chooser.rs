//! `A`: opens an attachment or a link. Each link shows its words and, under them, its full target,
//! seen before it opens;
//! nothing opens until a row is chosen.

use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;
use postio_model::ids::AttachmentId;

pub const DIALOG_NAME: &str = "focus-open-choice";

const WIDTH: i32 = 560;
const HEIGHT: i32 = 420;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Choice {
    Link {
        words: String,
        target: String,
    },
    Part {
        name: String,
        size: String,
        id: AttachmentId,
    },
}

impl Choice {
    pub fn said(&self) -> (String, String) {
        match self {
            Choice::Link { words, target } => (words.clone(), target.clone()),
            Choice::Part { name, size, .. } => (name.clone(), size.clone()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pick {
    Open(usize),
    Save(usize),
    SaveAll,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SavePick {
    File { suggested: String },
    Folder,
    Cards,
    CardsFile { suggested: String },
    Message { suggested: String },
}

/// `chosen` is called with what a person picks, and the dialog closes. Neither Save asks the portal
/// until pressed.
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
            let save = gtk::Button::with_label(postio_ui::reader::parts::SAVE);
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
        let save_all = gtk::Button::with_label(postio_ui::reader::parts::SAVE_ALL);
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
