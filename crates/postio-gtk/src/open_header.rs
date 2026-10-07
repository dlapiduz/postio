//! The open message's sender block (screen 04; T184, restyled by T209 to
//! the handoff's: no box, a hairline above and below): small dim From, To
//! and Cc labels in a 44px column, the sender's name in bold with their
//! address dim and in mono, and the date on the right -- "Today, 15:22"
//! for today's mail, in full beyond it.
//!
//! Drawn from the row's envelope, so it is up as soon as the row is read.
//! The shared reader's own header, with its Cc
//! disclosure, is not shown in Focus.

use adw::prelude::*;
use chrono::{DateTime, Local, Utc};
use postio_model::EmailAddress;
use postio_ui::focus_dialog;
use postio_ui::reader::header::RECIPIENTS_SHOWN;
use postio_widgets::widgets::space::S1;

/// The card, and the pieces that change with the message.
pub struct HeaderCard {
    root: gtk::Grid,
    from: gtk::Box,
    date: gtk::Label,
    to_label: gtk::Label,
    to: gtk::Box,
    cc_label: gtk::Label,
    cc: gtk::Box,
}

/// A field's name: small and dim, in a column of its own.
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

/// `address` as the card draws one: bare, in mono.
fn address_label(address: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(address));
    label.add_css_class("focus-open-address");
    // The domain is the part a person checks to trust a sender, so an
    // address is laid out whole before anything else gives: its least width
    // is its full width (the face is mono, so a character is a character),
    // and only an address wider than the whole line shortens, from the
    // front, keeping its domain. The name and the date give first.
    label.set_ellipsize(pango::EllipsizeMode::Start);
    label.set_width_chars(address.chars().count() as i32);
    label
}

impl HeaderCard {
    /// An empty card.
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
        // The date gives before the address does.
        date.set_ellipsize(pango::EllipsizeMode::End);
        let to_label = field("To");
        let to = line();
        let cc_label = field("Cc");
        let cc = line();
        root.attach(&from_label, 0, 0, 1, 1);
        root.attach(&from, 1, 0, 1, 1);
        root.attach(&date, 2, 0, 1, 1);
        root.attach(&to_label, 0, 1, 1, 1);
        root.attach(&to, 1, 1, 2, 1);
        root.attach(&cc_label, 0, 2, 1, 1);
        root.attach(&cc, 1, 2, 2, 1);
        let card = HeaderCard {
            root,
            from,
            date,
            to_label,
            to,
            cc_label,
            cc,
        };
        card.clear();
        card
    }

    /// The card, to place in the column.
    pub fn widget(&self) -> gtk::Widget {
        self.root.clone().upcast()
    }

    /// Draw the envelope: who it is from, who it went to and who was
    /// copied, and when, as of now.
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
                    // Bare, as the handoff draws it (T209): the mono face
                    // already sets it apart from the name.
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

    /// Nothing drawn: no message yet.
    pub fn clear(&self) {
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

/// The display name, when there is one.
fn name_of(address: &EmailAddress) -> Option<&str> {
    address
        .name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
}

/// A field of recipients: the first few by name (an address alone when
/// there is no name), how many more, and the full list in the tooltip. A
/// field nobody is in is not drawn, so a message with no Cc has no Cc line.
fn recipients(label: &gtk::Label, line: &gtk::Box, addresses: &[EmailAddress]) {
    clear(line);
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
                // Commas between, so a line of names reads as a list.
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
