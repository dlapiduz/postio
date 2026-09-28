//! The raw source (US2 scenario 5): the message as it came off the wire,
//! header lines first, in a read-only view over the window.
//!
//! The bytes come from `Client::raw_source`, which fetches the message only
//! when this asks for it and never otherwise. They are shown as text, never
//! interpreted: invalid UTF-8 becomes the replacement character.

use adw::prelude::*;
use postio_core::{CommandId, Keymap};
use postio_ui::hints;
use postio_widgets::widgets::keyhint;
use postio_widgets::widgets::space::S2;
use postio_widgets::widgets::{Kind, Size};

/// The dialog's widget name, so it can be told from another dialog.
pub const DIALOG_NAME: &str = "focus-raw-source";

/// The source dialog's size.
const WIDTH: i32 = 860;
const HEIGHT: i32 = 680;

/// A dialog showing `raw`, with Close and its key from `keymap`.
pub fn dialog(raw: &[u8], keymap: &Keymap) -> adw::Dialog {
    let close = gtk::Button::new();
    postio_widgets::widgets::button::style(&close, Kind::Secondary, Size::Regular);
    let close_row = gtk::Box::new(gtk::Orientation::Horizontal, S2);
    close_row.append(&gtk::Label::new(Some("Close")));
    if let Some(key) = hints::key(keymap, CommandId::Back) {
        close_row.append(&keyhint::cap(&key));
    }
    close.set_child(Some(&close_row));
    let title = gtk::Label::new(Some("Raw source"));
    title.add_css_class("focus-open-title");
    let header = gtk::CenterBox::new();
    header.add_css_class("focus-open-header");
    header.set_start_widget(Some(&close));
    header.set_center_widget(Some(&title));

    let buffer = gtk::TextBuffer::new(None);
    buffer.set_text(&String::from_utf8_lossy(raw));
    let text = gtk::TextView::with_buffer(&buffer);
    text.set_editable(false);
    text.set_monospace(true);
    text.set_wrap_mode(gtk::WrapMode::WordChar);
    text.add_css_class("focus-raw-source");
    let scrolled = gtk::ScrolledWindow::builder()
        .child(&text)
        .vexpand(true)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .build();

    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.add_css_class("focus-open");
    content.append(&header);
    content.append(&scrolled);
    let dialog = adw::Dialog::builder()
        .content_width(WIDTH)
        .content_height(HEIGHT)
        .child(&content)
        .build();
    dialog.set_widget_name(DIALOG_NAME);
    let weak = dialog.downgrade();
    close.connect_clicked(move |_| {
        if let Some(dialog) = weak.upgrade() {
            dialog.close();
        }
    });
    dialog
}

/// The text a source dialog shows.
pub fn shown(dialog: &adw::Dialog) -> Option<String> {
    let mut stack = vec![dialog.clone().upcast::<gtk::Widget>()];
    while let Some(widget) = stack.pop() {
        if let Some(text) = widget.downcast_ref::<gtk::TextView>() {
            let buffer = text.buffer();
            return Some(
                buffer
                    .text(&buffer.start_iter(), &buffer.end_iter(), false)
                    .to_string(),
            );
        }
        let mut child = widget.first_child();
        while let Some(next) = child {
            child = next.next_sibling();
            stack.push(next);
        }
    }
    None
}
