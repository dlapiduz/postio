//! The reading pane's header (#319): sender, recipients, subject, and date —
//! the three questions a reader asks first, answered before the body is even
//! in view.
//!
//! Native GTK, not markup inside the `WebView`'s document, for the same
//! reason the banner is (see [`postio_ui::reader::document::contain_body`]'s
//! doc comment): Postio's own chrome stays outside anything a sender's
//! markup could imitate, and it is what lets the header stay fixed while
//! the body scrolls underneath it rather than carrying it away.

use adw::prelude::*;
use chrono::{DateTime, Local, Utc};
use postio_model::address::EmailAddress;

/// Shown in place of a blank line — a missing subject is a fact about the
/// message, not something to render as if it were not there.
const NO_SUBJECT: &str = "(no subject)";

/// Above the remote-image banner and the body: who this is from, who it was
/// addressed to, what it is about, and when it arrived.
///
/// Independent of whether a body is on screen — [`Self::set_message`] takes
/// only the envelope, so a header-only message (backfill still pending, or
/// genuinely bodyless) gets exactly the same header a message with a body
/// does.
pub struct MessageHeader {
    root: gtk::Box,
    /// Subject and the sender/date row, grouped so they can be hidden
    /// together — the conversation pane's entry header already carries all
    /// three (#487), and only the recipients below belong to this widget
    /// there.
    identity: gtk::Box,
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
}

impl MessageHeader {
    /// Builds the header, empty until [`set_message`](Self::set_message)
    /// fills it in.
    pub fn new() -> Self {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 2);
        root.add_css_class("postio-message-header");
        root.set_accessible_role(gtk::AccessibleRole::Group);

        // Which account this arrived in, above everything else — the first
        // question in a mixed list, and the only place it is asked. See
        // `set_account` for why it is not on the list row. Outside
        // `identity`, which #487 hides wholesale in the conversation pane:
        // the entry header there repeats the subject and sender, not the
        // account, so this line still has something to say.
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
        identity.append(&subject);

        let top_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
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

        // `to` and the `Cc` disclosure share a row: the common one-recipient
        // case costs exactly the one line, and `Cc` costs nothing at all
        // when the message has none — no toggle, no reserved space.
        let recipients_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let to = gtk::Label::new(None);
        to.set_xalign(0.0);
        to.set_hexpand(true);
        to.set_ellipsize(pango::EllipsizeMode::End);
        to.add_css_class("postio-message-header-recipients");
        to.set_visible(false);
        recipients_row.append(&to);

        let cc_toggle = gtk::ToggleButton::with_label("Cc");
        cc_toggle.add_css_class("flat");
        cc_toggle.set_visible(false);
        cc_toggle.set_tooltip_text(Some("Show Cc recipients"));
        recipients_row.append(&cc_toggle);
        root.append(&recipients_row);

        let cc_label = gtk::Label::new(None);
        cc_label.set_xalign(0.0);
        cc_label.set_wrap(true);
        cc_label.add_css_class("postio-message-header-recipients");

        let cc_revealer = gtk::Revealer::new();
        cc_revealer.set_transition_type(gtk::RevealerTransitionType::SlideDown);
        // Motion budget: ≤100ms or absent.
        cc_revealer.set_transition_duration(100);
        cc_revealer.set_child(Some(&cc_label));
        root.append(&cc_revealer);

        let revealer_for_toggle = cc_revealer.clone();
        cc_toggle.connect_toggled(move |button| {
            revealer_for_toggle.set_reveal_child(button.is_active());
        });

        Self {
            root,
            identity,
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
        }
    }

    /// The widget to place above the banner, per [`super::view::Reader`]'s
    /// container.
    pub fn widget(&self) -> gtk::Widget {
        self.root.clone().upcast()
    }

    /// Shows or hides subject, sender and date, leaving recipients alone.
    ///
    /// The conversation pane (#487) draws all three of those on the entry
    /// header above this widget — showing them again here would be the
    /// duplication #308 removed. Recipients have nowhere else to go, so
    /// hiding "identity" is not the same as hiding the header.
    pub fn set_identity_visible(&self, visible: bool) {
        self.identity.set_visible(visible);
    }

    /// Whether subject, sender and date are currently on screen.
    pub fn identity_visible(&self) -> bool {
        self.identity.is_visible()
    }

    /// Fills in every field from a message's envelope.
    pub fn set_message(
        &self,
        from: &[EmailAddress],
        to: &[EmailAddress],
        cc: &[EmailAddress],
        subject: Option<&str>,
        date: DateTime<Utc>,
    ) {
        self.subject.set_label(&subject_text(subject));
        self.sender.set_label(&address_list(from));
        self.date.set_label(&absolute_date(date, Local::now()));

        if to.is_empty() {
            self.to.set_visible(false);
        } else {
            self.to.set_visible(true);
            self.to.set_label(&format!("To: {}", address_list(to)));
        }

        if cc.is_empty() {
            self.cc_toggle.set_visible(false);
            self.cc_toggle.set_active(false);
            self.cc_revealer.set_reveal_child(false);
        } else {
            self.cc_toggle.set_visible(true);
            self.cc_toggle.set_label(&format!("Cc ({})", cc.len()));
            self.cc_label.set_label(&address_list(cc));
        }
    }

    /// Names the account this message arrived in, or hides the line.
    ///
    /// # Why here and not on the list row
    ///
    /// ADR 0005 Q4 originally put per-account identity on every row in
    /// unified scope. The maintainer's call on #185 is that a mixed list is
    /// fine *as* a mixed list — you are reading "all messages", so of course
    /// it is mixed — and the question "whose is this?" is one you ask about
    /// the message in front of you, not about forty rows at once. Answering
    /// it here costs one line in the pane instead of a colour and a short
    /// name on every row, and it leaves the row's 3px left edge meaning
    /// exactly one thing, which is `selected`.
    ///
    /// `None` hides the line entirely. That is the single-account case, and
    /// it is why somebody who has never configured a second account sees no
    /// trace of this: naming the only account there is would be noise about a
    /// choice they have not made.
    ///
    /// `hue` indexes the generated palette (`tokens.rs`, `ACCOUNT_HUES`), so
    /// the colour comes from the design system rather than from this widget.
    /// It is never the only signal — the name is right beside it.
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

    /// The account line's text, or `None` when it is hidden. For tests.
    #[doc(hidden)]
    pub fn account_label(&self) -> Option<String> {
        self.account_row
            .is_visible()
            .then(|| self.account_name.label().to_string())
    }

    /// Empties every field — the pane closed, or moved to a different
    /// message before this one finished loading.
    pub fn clear(&self) {
        self.account_row.set_visible(false);
        self.subject.set_label("");
        self.sender.set_label("");
        self.date.set_label("");
        self.to.set_visible(false);
        self.cc_toggle.set_visible(false);
        self.cc_toggle.set_active(false);
        self.cc_revealer.set_reveal_child(false);
    }

    /// The subject line as currently shown, for tests.
    pub fn subject_label(&self) -> String {
        self.subject.label().to_string()
    }

    /// The sender line as currently shown, for tests.
    pub fn sender_label(&self) -> String {
        self.sender.label().to_string()
    }

    /// The date line as currently shown, for tests.
    pub fn date_label(&self) -> String {
        self.date.label().to_string()
    }

    /// Whether the `To` line is on screen, for tests.
    pub fn to_visible(&self) -> bool {
        self.to.is_visible()
    }

    /// The `To` line as currently shown, for tests.
    pub fn to_label(&self) -> String {
        self.to.label().to_string()
    }

    /// Whether the `Cc` disclosure is offered at all, for tests.
    pub fn cc_toggle_visible(&self) -> bool {
        self.cc_toggle.is_visible()
    }

    /// Whether the `Cc` line is currently revealed, for tests.
    pub fn cc_revealed(&self) -> bool {
        self.cc_revealer.reveals_child()
    }
}

impl Default for MessageHeader {
    fn default() -> Self {
        Self::new()
    }
}

/// "Name <address>" when a display name is present, the bare address
/// otherwise — never a name repeated as its own address.
fn address_line(address: &EmailAddress) -> String {
    match address
        .name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
    {
        Some(name) => format!("{name} <{}>", address.address),
        None => address.address.clone(),
    }
}

fn address_list(addresses: &[EmailAddress]) -> String {
    addresses
        .iter()
        .map(address_line)
        .collect::<Vec<_>>()
        .join(", ")
}

fn subject_text(subject: Option<&str>) -> String {
    subject
        .map(str::trim)
        .filter(|subject| !subject.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| NO_SUBJECT.to_string())
}

/// The header's date line: always absolute, unlike the list row's relative
/// [`crate::row::timestamp`] — a message once opened is not "3h ago" any
/// more, it is dated.
fn absolute_date(at: DateTime<Utc>, now: DateTime<Local>) -> String {
    let local = at.with_timezone(&now.timezone());
    local.format("%a, %-d %b %Y at %H:%M").to_string()
}

#[cfg(test)]
mod tests {
    use chrono::{Local, TimeZone, Utc};

    use super::*;

    fn addr(name: Option<&str>, address: &str) -> EmailAddress {
        EmailAddress::new(name, address)
    }

    #[test]
    fn a_named_address_shows_both_the_name_and_the_address() {
        let a = addr(Some("Ada Lovelace"), "ada@example.com");
        assert_eq!(address_line(&a), "Ada Lovelace <ada@example.com>");
    }

    #[test]
    fn an_unnamed_address_shows_just_the_address() {
        let a = addr(None, "ada@example.com");
        assert_eq!(address_line(&a), "ada@example.com");
    }

    #[test]
    fn a_blank_display_name_is_treated_as_absent() {
        let a = addr(Some("   "), "ada@example.com");
        assert_eq!(address_line(&a), "ada@example.com");
    }

    #[test]
    fn several_addresses_join_with_a_comma() {
        let list = [
            addr(Some("Ada"), "ada@example.com"),
            addr(None, "bob@example.com"),
        ];
        assert_eq!(
            address_list(&list),
            "Ada <ada@example.com>, bob@example.com"
        );
    }

    #[test]
    fn a_missing_subject_says_so_rather_than_showing_nothing() {
        assert_eq!(subject_text(None), NO_SUBJECT);
        assert_eq!(subject_text(Some("   ")), NO_SUBJECT);
    }

    #[test]
    fn a_real_subject_passes_through_verbatim() {
        assert_eq!(subject_text(Some("Dinner Friday?")), "Dinner Friday?");
    }

    #[test]
    fn the_date_line_is_always_absolute() {
        // Built in the local zone and handed over as UTC, the same way
        // `row.rs`'s own timestamp test does it: fixing both ends in UTC
        // would only pass in one timezone.
        let local = |y, m, d, h, min| Local.with_ymd_and_hms(y, m, d, h, min, 0).unwrap();
        let now = local(2026, 8, 26, 9, 0);
        let at = local(2026, 8, 12, 14, 32).with_timezone(&Utc);

        assert_eq!(absolute_date(at, now), "Wed, 12 Aug 2026 at 14:32");
    }
}
