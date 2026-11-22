//! The reading pane's header (#319): native GTK, not markup in the body's document, so Postio's own
//! chrome stays outside
//! anything a sender's markup could imitate and the header stays fixed while the body scrolls.

use adw::prelude::*;
use chrono::{DateTime, Utc};
use postio_model::address::EmailAddress;
use postio_ui::reader::header::MessageHeader as HeaderLines;

/// A fixed width so From, To and Cc form a column and their values line up (#1437).
fn field_label(text: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.set_xalign(0.0);
    label.set_width_chars(4);
    label.set_valign(gtk::Align::Baseline);
    label.add_css_class("postio-message-header-field");
    label
}

/// Not drawn is also not reachable: a toggle nobody can see must not take a `Tab` or a click.
fn show_cc_toggle(toggle: &gtk::ToggleButton, shown: bool) {
    toggle.set_child_visible(shown);
    toggle.set_sensitive(shown);
    toggle.set_can_focus(shown);
}

pub struct MessageHeader {
    root: gtk::Box,
    to_label: gtk::Label,
    account_row: gtk::Box,
    account_swatch: gtk::Box,
    account_name: gtk::Label,
    subject: gtk::Label,
    sender: gtk::Label,
    date: gtk::Label,
    to: gtk::Label,
    cc_toggle: gtk::ToggleButton,
    cc_revealer: gtk::Revealer,
    cc_label: gtk::Label,
    warning_row: gtk::Box,
    warning: gtk::Label,
}

impl MessageHeader {
    pub fn new() -> Self {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 2);
        root.add_css_class("postio-message-header");
        root.set_accessible_role(gtk::AccessibleRole::Group);

        let account_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        account_row.add_css_class("postio-message-header-account");
        account_row.set_visible(false);
        let account_swatch = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        account_swatch.add_css_class("postio-account-swatch");
        account_row.append(&account_swatch);
        let account_name = gtk::Label::new(None);
        account_name.set_xalign(0.0);
        account_name.set_ellipsize(pango::EllipsizeMode::End);
        account_name.add_css_class("postio-message-header-account-name");
        account_row.append(&account_name);
        root.append(&account_row);

        let identity = gtk::Box::new(gtk::Orientation::Vertical, 2);
        root.append(&identity);

        let subject = gtk::Label::new(None);
        subject.set_xalign(0.0);
        subject.set_ellipsize(pango::EllipsizeMode::End);
        subject.add_css_class("postio-message-header-subject");
        subject.set_hexpand(true);
        identity.append(&subject);

        // From, To and Cc share a label column (#1437) so the addresses begin at the same place
        // down the header.
        let top_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let from_label = field_label("From");
        top_row.append(&from_label);

        let sender = gtk::Label::new(None);
        sender.set_xalign(0.0);
        sender.set_hexpand(true);
        sender.set_ellipsize(pango::EllipsizeMode::End);
        sender.add_css_class("postio-message-header-sender");
        top_row.append(&sender);

        let date = gtk::Label::new(None);
        date.add_css_class("postio-message-header-date");
        top_row.append(&date);
        identity.append(&top_row);

        // The row is always there at the same height whoever the message went to, or the body moved
        // 16-18px between two
        // messages. What is absent is not drawn (`set_child_visible`) rather than taken out of the
        // layout.
        let recipients_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let to_label = field_label("To");
        to_label.set_child_visible(false);
        recipients_row.append(&to_label);

        let to = gtk::Label::new(None);
        to.set_xalign(0.0);
        to.set_hexpand(true);
        to.set_ellipsize(pango::EllipsizeMode::End);
        to.add_css_class("postio-message-header-recipients");
        to.set_child_visible(false);
        recipients_row.append(&to);

        let cc_toggle = gtk::ToggleButton::with_label("Cc");
        cc_toggle.add_css_class("flat");
        cc_toggle.add_css_class("postio-message-header-cc");
        show_cc_toggle(&cc_toggle, false);
        cc_toggle.set_tooltip_text(Some("Show Cc recipients"));
        recipients_row.append(&cc_toggle);
        root.append(&recipients_row);

        let cc_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let cc_name = field_label("Cc");
        cc_row.append(&cc_name);

        let cc_label = gtk::Label::new(None);
        cc_label.set_hexpand(true);
        cc_label.set_xalign(0.0);
        cc_label.set_wrap(true);
        cc_label.add_css_class("postio-message-header-recipients");

        let cc_revealer = gtk::Revealer::new();
        cc_revealer.set_transition_type(gtk::RevealerTransitionType::SlideDown);
        cc_revealer.set_transition_duration(100);
        cc_row.append(&cc_label);
        cc_revealer.set_child(Some(&cc_row));
        root.append(&cc_revealer);

        let warning_row = gtk::Box::new(gtk::Orientation::Horizontal, crate::widgets::space::S2);
        warning_row.add_css_class("postio-message-header-warning");
        warning_row.set_visible(false);
        let warning_icon = gtk::Image::from_icon_name("dialog-warning-symbolic");
        warning_icon.set_valign(gtk::Align::Start);
        warning_row.append(&warning_icon);
        let warning = gtk::Label::new(None);
        warning.set_xalign(0.0);
        warning.set_hexpand(true);
        warning.set_wrap(true);
        warning.add_css_class("caption");
        warning.add_css_class("warning");
        warning_row.append(&warning);
        root.append(&warning_row);

        let revealer_for_toggle = cc_revealer.clone();
        cc_toggle.connect_toggled(move |button| {
            revealer_for_toggle.set_reveal_child(button.is_active());
        });

        Self {
            root,
            to_label,
            account_row,
            account_swatch,
            account_name,
            subject,
            sender,
            date,
            to,
            cc_toggle,
            cc_revealer,
            cc_label,
            warning_row,
            warning,
        }
    }

    pub fn widget(&self) -> gtk::Widget {
        self.root.clone().upcast()
    }

    /// Every string comes from [`HeaderLines`], including the shortened recipient line and `Cc (n)`
    /// title: the macOS header
    /// writes the same two, and a label composed in each frontend drifts (#1259, #1285). Field
    /// names are the exception:
    /// `field_label` draws each word once, in `new`.
    pub fn set_message(
        &self,
        from: &[EmailAddress],
        to: &[EmailAddress],
        cc: &[EmailAddress],
        subject: Option<&str>,
        date: DateTime<Utc>,
    ) {
        let lines = HeaderLines::of(from, to, cc, subject, date, postio_ui::clock::now());

        // The last message's warning was about its sender, not this one's.
        self.set_warning(None);
        self.subject.set_label(&lines.subject);
        self.set_sender_line(&lines.from, &lines.date);
        self.draw_recipients(&lines);
    }

    pub fn set_sender_line(&self, from: &str, date: &str) {
        self.sender.set_label(from);
        self.date.set_label(date);
    }

    fn draw_recipients(&self, lines: &HeaderLines) {
        match lines.to_line() {
            Some(line) => {
                self.to_label.set_child_visible(true);
                self.to.set_child_visible(true);
                self.to.set_label(&line);
                // The full list stays reachable here because "who exactly is on this" decides
                // whether reply-all is a mistake
                // (#1332). Only when they differ.
                self.to.set_tooltip_text(
                    lines
                        .to
                        .as_deref()
                        .filter(|full| Some(*full) != lines.to_short.as_deref()),
                );
            }
            None => {
                self.to_label.set_child_visible(false);
                self.to.set_child_visible(false);
                self.to.set_label("");
                self.to.set_tooltip_text(None);
            }
        }

        match (lines.cc_toggle_label(), lines.cc.as_deref()) {
            (Some(label), Some(addresses)) => {
                show_cc_toggle(&self.cc_toggle, true);
                self.cc_toggle.set_label(&label);
                self.cc_label.set_label(addresses);
            }
            _ => {
                show_cc_toggle(&self.cc_toggle, false);
                self.cc_toggle.set_active(false);
                self.cc_revealer.set_reveal_child(false);
            }
        }
    }

    /// The line is here, not on the list row: "whose is this?" is asked about the message in front
    /// of you (ADR 0005 Q4, #185).
    /// `None` hides it (the single-account case). `hue` indexes the generated palette
    /// (`ACCOUNT_HUES`); the name is beside
    /// it, so colour is never the only signal.
    pub fn set_account(&self, name: Option<&str>, hue: usize) {
        let Some(name) = name else {
            self.account_row.set_visible(false);
            return;
        };
        for index in 0..postio_ui::tokens::ACCOUNT_HUES {
            self.account_swatch
                .remove_css_class(&format!("postio-account-{index}"));
        }
        self.account_swatch.add_css_class(&format!(
            "postio-account-{}",
            hue % postio_ui::tokens::ACCOUNT_HUES
        ));
        self.account_name.set_label(name);
        self.account_row.set_visible(true);
        self.account_row
            .update_property(&[gtk::accessible::Property::Label(&format!(
                "Account: {name}"
            ))]);
    }

    #[doc(hidden)]
    pub fn account_label(&self) -> Option<String> {
        self.account_row
            .is_visible()
            .then(|| self.account_name.label().to_string())
    }

    /// The sentence comes from [`postio_ui::reader::authenticity`], so the frontends cannot word it
    /// differently.
    pub fn set_warning(&self, line: Option<&str>) {
        match line {
            Some(line) => {
                self.warning.set_label(line);
                self.warning_row.set_visible(true);
                self.warning_row
                    .update_property(&[gtk::accessible::Property::Label(line)]);
            }
            None => {
                self.warning.set_label("");
                self.warning_row.set_visible(false);
            }
        }
    }

    #[doc(hidden)]
    pub fn warning_label(&self) -> Option<String> {
        self.warning_row
            .is_visible()
            .then(|| self.warning.label().to_string())
    }

    pub fn clear(&self) {
        self.account_row.set_visible(false);
        self.set_warning(None);
        self.subject.set_label("");
        self.sender.set_label("");
        self.date.set_label("");
        self.to_label.set_child_visible(false);
        self.to.set_child_visible(false);
        show_cc_toggle(&self.cc_toggle, false);
        self.cc_toggle.set_active(false);
        self.cc_revealer.set_reveal_child(false);
    }

    pub fn subject_label(&self) -> String {
        self.subject.label().to_string()
    }

    pub fn sender_label(&self) -> String {
        self.sender.label().to_string()
    }

    pub fn date_label(&self) -> String {
        self.date.label().to_string()
    }

    pub fn to_visible(&self) -> bool {
        self.to.is_visible() && self.to.is_child_visible()
    }

    pub fn to_label(&self) -> String {
        self.to.label().to_string()
    }

    pub fn cc_toggle_visible(&self) -> bool {
        self.cc_toggle.is_visible() && self.cc_toggle.is_child_visible()
    }

    pub fn cc_revealed(&self) -> bool {
        self.cc_revealer.reveals_child()
    }
}

impl Default for MessageHeader {
    fn default() -> Self {
        Self::new()
    }
}
