//! The reading pane's two notices: `postio-xxz` and `#901`.
//!
//! Both are native GTK chrome, not something drawn inside the `WebView` — the
//! pane they sit above is exactly the thing they are reporting on, and they
//! have to keep working even when every remote image in a message stays
//! blocked forever.
//!
//! # They are [`NoticeBar`]s
//!
//! One line, never wrapping (#1002). That is not a detail: the remote-image
//! banner used to spell its sender out inline, and an Apple relay address is
//! 70 characters, so the notice grew to three lines and pushed the mail down
//! the pane. The long action lives in the overflow now, which is what the
//! canvas draws (turn 7).

use std::rc::Rc;

// `UnsubscribeBanner` below is still hand-built GTK (#971), so the prelude
// stays. #1002 rewrote the two notices in this file onto `NoticeBar` and
// took the prelude with them; the third one arrived on `main` in between.
use adw::prelude::*;

use crate::widgets::{NoticeBar, NoticeMenuItem};

/// How much of a sender's address the menu spells out.
///
/// Wide enough that an ordinary address is untouched, narrow enough that a
/// private-relay one does not set the menu's width on its own.
const ADDRESS_WIDTH: usize = 34;

/// "Remote images are blocked", with a way to see them once and a way to
/// trust this sender from now on.
pub struct RemoteImageBanner {
    notice: Rc<NoticeBar>,
    /// The sender's domain, for the second menu entry. Kept alongside the
    /// address because "always allow this domain" and "always allow this
    /// address" are different promises and the menu offers both.
    domain: std::cell::RefCell<Option<String>>,
    /// What "always allow" would exempt, and what its menu entry says. Kept
    /// because [`set_sender`](Self::set_sender) rebuilds the menu and the
    /// handler has to be re-attached with it.
    sender: std::cell::RefCell<Option<String>>,
    always_allow: std::cell::RefCell<Vec<Rc<dyn Fn()>>>,
}

impl RemoteImageBanner {
    /// Build the banner, hidden — [`super::view::Reader`] shows it once it
    /// knows a message actually has remote images to block.
    pub fn new() -> Self {
        let notice = NoticeBar::new("image-missing-symbolic", "postio-remote-banner");
        notice.set_text("Remote images are blocked");
        notice.set_action(Some("Show images"));
        let banner = RemoteImageBanner {
            notice,
            domain: std::cell::RefCell::new(None),
            sender: std::cell::RefCell::new(None),
            always_allow: std::cell::RefCell::new(Vec::new()),
        };
        banner.rebuild_menu();
        banner
    }

    /// The widget to place above the reading pane's `WebView`.
    pub fn widget(&self) -> gtk::Widget {
        self.notice.widget()
    }

    /// Show or hide the whole banner.
    pub fn set_visible(&self, visible: bool) {
        self.notice.set_visible(visible);
    }

    /// Whether the banner is currently on screen.
    pub fn is_visible(&self) -> bool {
        self.notice.is_visible()
    }

    /// What the banner currently says. Test-facing.
    pub fn text(&self) -> String {
        self.notice.text()
    }

    /// The overflow's entries, in order. Test-facing.
    pub fn menu_labels(&self) -> Vec<String> {
        self.notice.menu_labels()
    }

    /// The key `Show images` announces, from the live keymap.
    pub fn set_action_key(&self, key: Option<&str>) {
        self.notice.set_action_key(key);
    }

    /// Name whose remote images "always allow" would exempt from now on.
    ///
    /// With no sender to name, the entry is dropped rather than left saying
    /// "Always allow" with no object — the notice's shape is the icon, the
    /// text and `Show images`, and that stays constant.
    pub fn set_sender(&self, sender: Option<&str>) {
        *self.domain.borrow_mut() = sender
            .and_then(|sender| sender.rsplit_once('@'))
            .map(|(_, domain)| domain.to_owned())
            .filter(|domain| !domain.is_empty());
        *self.sender.borrow_mut() = sender.map(str::to_owned);
        self.rebuild_menu();
    }

    /// What the notice says: the counts, per canvas turn 7.
    ///
    /// Called by the reader once a render has settled how many references
    /// were held back — the banner cannot know, and a notice that guessed
    /// would be a privacy claim made without evidence.
    pub fn set_held_back(&self, held_back: postio_ui::reader::document::HeldBack) {
        let summary = held_back.summary();
        if !summary.is_empty() {
            self.notice.set_text(&summary);
        }
    }

    /// Called when the user asks to see this one message's images once,
    /// without adding the sender to the standing allow list.
    pub fn connect_show_once<F: Fn() + 'static>(&self, handler: F) {
        self.notice.connect_action(handler);
    }

    /// Called when the user asks to always allow this message's sender.
    pub fn connect_always_allow<F: Fn() + 'static>(&self, handler: F) {
        self.always_allow.borrow_mut().push(Rc::new(handler));
        self.rebuild_menu();
    }

    /// The "always allow" entry's current label — names the sender
    /// [`set_sender`](Self::set_sender) was last called with. Test-facing:
    /// production code has no reason to read a label back.
    pub fn always_allow_label(&self) -> String {
        self.notice
            .menu_labels()
            .into_iter()
            .next()
            .unwrap_or_default()
    }

    /// Choose "always allow" — what a test uses in place of a synthesized
    /// pointer click, which WebKitGTK's public API gives no way to do for a
    /// native GTK button.
    pub fn emit_always_allow(&self) {
        self.notice.press_menu_item(0);
    }

    /// As [`emit_always_allow`](Self::emit_always_allow), for "show once".
    pub fn emit_show_once(&self) {
        self.notice.press_action();
    }

    /// Put the overflow back together.
    ///
    /// Rebuilt rather than mutated because [`NoticeBar::set_menu`] replaces
    /// the whole menu — a notice's overflow describes the message it is
    /// currently reporting on, and leaving the previous message's entries
    /// behind would offer to always-allow the wrong sender.
    fn rebuild_menu(&self) {
        let Some(sender) = self.sender.borrow().clone() else {
            self.notice.set_menu(Vec::new());
            return;
        };
        let handlers = self.always_allow.borrow().clone();
        // The address is middle-truncated: a private-relay address is 70
        // characters, and spelling one out is what made this notice three
        // lines tall before #1008 moved it into a menu at all.
        let mut items = vec![NoticeMenuItem::new(
            format!(
                "Always allow {}",
                postio_ui::format::middle_truncate(&sender, ADDRESS_WIDTH)
            ),
            move || {
                for handler in &handlers {
                    handler();
                }
            },
        )];
        // "This domain" is a wider promise than "this sender", and the canvas
        // offers both because they are different decisions: a shop that mails
        // from a new address per order needs the domain, and a single
        // correspondent does not.
        if let Some(domain) = self.domain.borrow().clone() {
            let handlers = self.always_allow.borrow().clone();
            items.push(NoticeMenuItem::new(
                format!("Always allow {domain}"),
                move || {
                    for handler in &handlers {
                        handler();
                    }
                },
            ));
        }
        self.notice.set_menu(items);
    }
}

impl Default for RemoteImageBanner {
    fn default() -> Self {
        Self::new()
    }
}

// Widget behaviour (visibility, the sender label, the two signals) needs a
// real display and GTK's single, main-thread initialization — see
// `tests/gtk_reader.rs`, which follows the same one-test-function convention
// as `tests/gtk_shell.rs` and the other display-backed suites.

/// "Parts of this message could not be decoded" — `#901`.
///
/// A sibling of [`RemoteImageBanner`] and native for the same reason: it
/// reports *on* the document below it, so it cannot live inside it. Where
/// that banner offers two ways out, this one offers none — there is nothing
/// the reader can do about a charset the sender mislabelled, and a button
/// that re-decoded with a different guess would be inventing a second answer
/// to go with the first.
///
/// # Why it says so little
///
/// The three degradations behind it are different — base64 outside its
/// alphabet, an unknown `Content-Transfer-Encoding`, a charset that lost
/// octets to U+FFFD — and naming which one happened would be a sentence
/// about MIME in the middle of somebody's mail. What a reader needs is the
/// one fact that changes what they do with it: the words below may not be
/// the words that were sent, so the original is worth checking before acting
/// on it.
pub struct DecodeNotice {
    notice: Rc<NoticeBar>,
}

impl DecodeNotice {
    /// Build the notice, hidden.
    pub fn new() -> Self {
        let notice = NoticeBar::new("dialog-warning-symbolic", "postio-decode-notice");
        // One line, so the sentence is shorter than the wrapping one it
        // replaced. What a reader needs is the fact that changes what they
        // do, and the rest was elaboration.
        notice.set_text("Parts of this message could not be decoded");
        DecodeNotice { notice }
    }

    /// The widget to place above the reading pane's `WebView`.
    pub fn widget(&self) -> gtk::Widget {
        self.notice.widget()
    }

    /// Show or hide it.
    pub fn set_visible(&self, visible: bool) {
        self.notice.set_visible(visible);
    }

    /// Whether it is showing — what a test asserts on.
    pub fn is_visible(&self) -> bool {
        self.notice.is_visible()
    }
}

impl Default for DecodeNotice {
    fn default() -> Self {
        Self::new()
    }
}

/// "Leave this mailing list" — `#971`.
///
/// A third sibling of [`RemoteImageBanner`], for the same reason: it names a
/// fact about the message that has to survive the message being gone (the
/// activation log outlives the reader), so it cannot be markup inside the
/// document either. Unlike the other two it takes no local action of its
/// own — leaving a list is a write to storage this crate cannot reach
/// (`postio-gtk` has no SQL), so it only asks; whoever wires the reader
/// decides what "asked" means. Whether the activation also sends the real
/// RFC 8058 request is #972, deliberately not this one.
pub struct UnsubscribeBanner {
    root: gtk::Box,
    label: gtk::Label,
    unsubscribe: gtk::Button,
}

impl UnsubscribeBanner {
    /// Build the banner, hidden — [`super::view::Reader`] shows it once it
    /// knows which list, if any, the message on screen belongs to.
    pub fn new() -> Self {
        let root = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        root.add_css_class("postio-unsubscribe-banner");
        root.set_visible(false);
        root.set_accessible_role(gtk::AccessibleRole::Group);

        let icon = gtk::Image::from_icon_name("mail-unread-symbolic");
        root.append(&icon);

        let label = gtk::Label::new(None);
        label.set_hexpand(true);
        label.set_xalign(0.0);
        label.add_css_class("postio-unsubscribe-banner-label");
        root.append(&label);

        let unsubscribe = gtk::Button::with_label("Unsubscribe");
        unsubscribe.add_css_class("flat");
        root.append(&unsubscribe);

        UnsubscribeBanner {
            root,
            label,
            unsubscribe,
        }
    }

    /// The widget to place above the reading pane's `WebView`.
    pub fn widget(&self) -> gtk::Widget {
        self.root.clone().upcast()
    }

    /// Name the list this message belongs to and show the banner, or hide
    /// it with no list to leave.
    pub fn set_list(&self, list: Option<&str>) {
        match list {
            Some(list) => {
                self.label
                    .set_label(&format!("This message is from {list}"));
                self.root.set_visible(true);
            }
            None => self.root.set_visible(false),
        }
    }

    /// Whether the banner is currently on screen.
    pub fn is_visible(&self) -> bool {
        self.root.is_visible()
    }

    /// The banner's label text — what names the list a click would leave.
    /// Test-facing.
    pub fn label(&self) -> String {
        self.label.label().to_string()
    }

    /// Called when the user asks to leave the list currently named.
    pub fn connect_unsubscribe<F: Fn() + 'static>(&self, handler: F) {
        self.unsubscribe.connect_clicked(move |_| handler());
    }

    /// Simulate a click on "unsubscribe" — what a test uses in place of a
    /// synthesized pointer click.
    pub fn emit_unsubscribe(&self) {
        self.unsubscribe.emit_clicked();
    }
}

impl Default for UnsubscribeBanner {
    fn default() -> Self {
        Self::new()
    }
}
