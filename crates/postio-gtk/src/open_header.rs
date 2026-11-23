//! The open message's sender block: small dim From, To and Cc labels, the sender's bold name with
//! the address dim in mono, and the date.
//! Drawn from the row's envelope, so it is up as soon as the row is read; the shared reader's own
//! header is not shown in Focus.

use adw::prelude::*;
use chrono::{DateTime, Local, Utc};
use postio_model::EmailAddress;
use postio_ui::focus_dialog;
use postio_ui::reader::header::RECIPIENTS_SHOWN;
use postio_widgets::widgets::space::S1;

pub struct HeaderCard {
    root: gtk::Grid,
    from: gtk::Box,
    mark: gtk::Button,
    date: gtk::Label,
    to_label: gtk::Label,
    to: adw::WrapBox,
    cc_label: gtk::Label,
    cc: adw::WrapBox,
    warning_row: gtk::Box,
    warning: gtk::Label,
    reply_label: gtk::Label,
    reply: gtk::Label,
}

fn field(text: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.add_css_class("focus-open-field");
    label.set_xalign(0.0);
    label.set_valign(gtk::Align::Baseline);
    label.set_size_request(focus_dialog::SENDER_LABEL_COLUMN, -1);
    label
}

fn line() -> gtk::Box {
    let line = gtk::Box::new(gtk::Orientation::Horizontal, S1);
    line.set_hexpand(true);
    line.set_valign(gtk::Align::Baseline);
    line
}

/// Recipients keep their readable addresses, wrapping at the column's edge rather than adding their
/// minimum widths across the dialog.
fn recipient_line() -> adw::WrapBox {
    adw::WrapBox::builder()
        .child_spacing(S1)
        .line_spacing(S1)
        .hexpand(true)
        .valign(gtk::Align::Start)
        .css_classes(["focus-open-recipient-line"])
        .build()
}

const LONGEST_WHOLE_ADDRESS: usize = 40;

fn address_label(address: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(address));
    label.add_css_class("focus-open-address");
    // The domain is what a person checks to trust a sender, so an address is laid out whole before
    // anything else gives;
    // only one too long for any line shortens, from the front, so the domain stays.
    if address.chars().count() > LONGEST_WHOLE_ADDRESS {
        label.set_ellipsize(pango::EllipsizeMode::Start);
        label.set_width_chars(LONGEST_WHOLE_ADDRESS as i32);
    }
    label
}

impl HeaderCard {
    pub fn new() -> Self {
        let root = gtk::Grid::new();
        root.add_css_class("focus-open-header-card");
        root.set_column_spacing(focus_dialog::SENDER_COLUMN_GAP as u32);
        root.set_row_spacing(focus_dialog::SENDER_ROW_GAP as u32);

        let from_label = field("From");
        let from = line();
        let date = gtk::Label::new(None);
        date.add_css_class("focus-open-date");
        date.set_valign(gtk::Align::Baseline);
        // A wrapping line gives the grid no baseline to share, so a field
        // of recipients and its name meet at their tops instead.
        let to_label = field("To");
        to_label.set_valign(gtk::Align::Start);
        let to = recipient_line();
        let cc_label = field("Cc");
        cc_label.set_valign(gtk::Align::Start);
        let cc = recipient_line();
        let mark = gtk::Button::new();
        mark.add_css_class("flat");
        mark.add_css_class("postio-chip-base");
        mark.add_css_class("focus-open-mark");
        mark.set_valign(gtk::Align::Center);
        mark.set_visible(false);
        let sender = gtk::Box::new(gtk::Orientation::Horizontal, S1);
        sender.append(&from);
        sender.append(&mark);
        root.attach(&from_label, 0, 0, 1, 1);
        root.attach(&sender, 1, 0, 1, 1);
        root.attach(&date, 2, 0, 1, 1);
        root.attach(&to_label, 0, 1, 1, 1);
        root.attach(&to, 1, 1, 2, 1);
        root.attach(&cc_label, 0, 2, 1, 1);
        root.attach(&cc, 1, 2, 2, 1);
        let warning_row = gtk::Box::new(gtk::Orientation::Horizontal, S1);
        warning_row.add_css_class("focus-open-warning");
        warning_row.set_visible(false);
        let icon = gtk::Image::from_icon_name("dialog-warning-symbolic");
        icon.set_valign(gtk::Align::Start);
        warning_row.append(&icon);
        let warning = gtk::Label::new(None);
        warning.set_xalign(0.0);
        warning.set_hexpand(true);
        warning.set_wrap(true);
        warning.add_css_class("caption");
        warning.add_css_class("warning");
        warning_row.append(&warning);
        root.attach(&warning_row, 1, 3, 2, 1);
        let reply_label = field("Reply to");
        reply_label.set_visible(false);
        let reply = gtk::Label::new(None);
        reply.add_css_class("focus-open-recipient");
        reply.set_xalign(0.0);
        reply.set_visible(false);
        root.attach(&reply_label, 0, 4, 1, 1);
        root.attach(&reply, 1, 4, 2, 1);
        let card = HeaderCard {
            root,
            from,
            mark,
            date,
            to_label,
            to,
            cc_label,
            cc,
            warning_row,
            warning,
            reply_label,
            reply,
        };
        card.clear();
        card
    }

    pub fn widget(&self) -> gtk::Widget {
        self.root.clone().upcast()
    }

    pub fn set(
        &self,
        from: &[EmailAddress],
        to: &[EmailAddress],
        cc: &[EmailAddress],
        date: DateTime<Utc>,
    ) {
        self.set_at(from, to, cc, date, postio_ui::clock::now());
    }

    fn set_at(
        &self,
        from: &[EmailAddress],
        to: &[EmailAddress],
        cc: &[EmailAddress],
        date: DateTime<Utc>,
        now: DateTime<Local>,
    ) {
        clear(&self.from);
        if let Some(sender) = from.first() {
            match name_of(sender) {
                Some(name) => {
                    let name = gtk::Label::new(Some(name));
                    name.add_css_class("focus-open-sender-name");
                    name.set_ellipsize(pango::EllipsizeMode::End);
                    self.from.append(&name);
                    self.from.append(&address_label(&sender.address));
                }
                None => {
                    let bare = address_label(&sender.address);
                    bare.add_css_class("focus-open-sender-name");
                    self.from.append(&bare);
                }
            }
        }
        self.date
            .set_text(&postio_ui::focus_row::message_date(date, now));
        recipients(&self.to_label, &self.to, to);
        recipients(&self.cc_label, &self.cc, cc);
    }

    /// The sentence is `postio_ui::reader::authenticity`'s (#1928).
    pub fn set_warning(&self, line: Option<&str>) {
        self.warning.set_text(line.unwrap_or(""));
        self.warning_row.set_visible(line.is_some());
    }

    pub fn set_mark(&self, mark: Option<postio_ui::reader::authenticity::Mark>, address: &str) {
        match mark {
            Some(mark) => {
                self.mark.set_label(mark.label());
                self.mark
                    .update_property(&[gtk::accessible::Property::Label(
                        &mark.accessible_name(address),
                    )]);
                self.mark.set_visible(true);
            }
            None => self.mark.set_visible(false),
        }
    }

    pub fn mark_shown(&self) -> Option<String> {
        self.mark
            .is_visible()
            .then(|| self.mark.label().map(|label| label.to_string()))
            .flatten()
    }

    pub fn connect_mark(&self, pressed: impl Fn() + 'static) {
        self.mark.connect_clicked(move |_| pressed());
    }

    pub fn set_reply_to(&self, text: Option<&str>) {
        self.reply.set_text(text.unwrap_or(""));
        self.reply.set_visible(text.is_some());
        self.reply_label.set_visible(text.is_some());
    }

    pub fn reply_to(&self) -> Option<String> {
        self.reply
            .is_visible()
            .then(|| self.reply.text().to_string())
    }

    pub fn clear(&self) {
        self.set_warning(None);
        self.set_mark(None, "");
        clear(&self.from);
        self.date.set_text("");
        recipients(&self.to_label, &self.to, &[]);
        recipients(&self.cc_label, &self.cc, &[]);
    }
}

impl Default for HeaderCard {
    fn default() -> Self {
        Self::new()
    }
}

fn clear(line: &gtk::Box) {
    while let Some(child) = line.first_child() {
        line.remove(&child);
    }
}

fn name_of(address: &EmailAddress) -> Option<&str> {
    address
        .name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
}

/// A field nobody is in is not drawn, so a message with no Cc has no Cc line.
fn recipients(label: &gtk::Label, line: &adw::WrapBox, addresses: &[EmailAddress]) {
    while let Some(child) = line.first_child() {
        line.remove(&child);
    }
    label.set_visible(!addresses.is_empty());
    line.set_visible(!addresses.is_empty());
    line.set_tooltip_text(None);
    if addresses.is_empty() {
        return;
    }
    let shown = addresses.len().min(RECIPIENTS_SHOWN);
    for (at, address) in addresses.iter().take(shown).enumerate() {
        match name_of(address) {
            Some(name) => {
                let comma = if at + 1 < shown || addresses.len() > shown {
                    ","
                } else {
                    ""
                };
                let name = gtk::Label::new(Some(&format!("{name}{comma}")));
                name.add_css_class("focus-open-recipient");
                name.set_ellipsize(pango::EllipsizeMode::End);
                line.append(&name);
            }
            None => line.append(&address_label(&address.address)),
        }
    }
    if addresses.len() > RECIPIENTS_SHOWN {
        let hidden = addresses.len() - RECIPIENTS_SHOWN;
        let more = gtk::Label::new(Some(&format!(
            "and {hidden} {}",
            if hidden == 1 { "other" } else { "others" }
        )));
        more.add_css_class("focus-open-recipient");
        line.append(&more);
    }
    line.set_tooltip_text(Some(&postio_ui::reader::header::address_list(addresses)));
}
