//! The reading pane's notices, native GTK chrome rather than inside the `WebView`: the pane they
//! sit above is what they
//! report on. Each is a one-line [`NoticeBar`] that never wraps (#1002); the long action lives in
//! the overflow.

use std::rc::Rc;

use crate::widgets::{NoticeBar, NoticeMenuItem};

/// Wide enough that an ordinary address is untouched, narrow enough that a private-relay one does
/// not set the menu's width.
const ADDRESS_WIDTH: usize = 34;

pub struct RemoteImageBanner {
    notice: Rc<NoticeBar>,
    /// "Always allow this domain" and "this address" are different promises and the menu offers
    /// both.
    domain: std::cell::RefCell<Option<String>>,
    /// Kept because `set_sender` rebuilds the menu and the handler is re-attached with it.
    sender: std::cell::RefCell<Option<String>>,
    always_allow: std::cell::RefCell<Vec<Rc<dyn Fn()>>>,
}

impl RemoteImageBanner {
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

    pub fn widget(&self) -> gtk::Widget {
        self.notice.widget()
    }

    pub fn set_visible(&self, visible: bool) {
        self.notice.set_visible(visible);
    }

    pub fn is_visible(&self) -> bool {
        self.notice.is_visible()
    }

    pub fn text(&self) -> String {
        self.notice.text()
    }

    pub fn menu_labels(&self) -> Vec<String> {
        self.notice.menu_labels()
    }

    pub fn set_action_key(&self, key: Option<&str>) {
        self.notice.set_action_key(key);
    }

    /// With no sender the entry is dropped rather than left saying "Always allow" with no object.
    pub fn set_sender(&self, sender: Option<&str>) {
        *self.domain.borrow_mut() = sender
            .and_then(|sender| sender.rsplit_once('@'))
            .map(|(_, domain)| domain.to_owned())
            .filter(|domain| !domain.is_empty());
        *self.sender.borrow_mut() = sender.map(str::to_owned);
        self.rebuild_menu();
    }

    /// Called once a render has settled how many references were held back: a notice that guessed
    /// would be a privacy claim
    /// without evidence.
    pub fn set_held_back(&self, held_back: postio_ui::reader::document::HeldBack) {
        let summary = held_back.summary();
        if !summary.is_empty() {
            self.notice.set_text(&summary);
        }
    }

    pub fn connect_show_once<F: Fn() + 'static>(&self, handler: F) {
        self.notice.connect_action(handler);
    }

    pub fn connect_always_allow<F: Fn() + 'static>(&self, handler: F) {
        self.always_allow.borrow_mut().push(Rc::new(handler));
        self.rebuild_menu();
    }

    pub fn always_allow_label(&self) -> String {
        self.notice
            .menu_labels()
            .into_iter()
            .next()
            .unwrap_or_default()
    }

    /// What a test uses in place of a synthesized pointer click, which WebKitGTK gives no way to do
    /// for a native button.
    pub fn emit_always_allow(&self) {
        self.notice.press_menu_item(0);
    }

    pub fn emit_show_once(&self) {
        self.notice.press_action();
    }

    /// Rebuilt rather than mutated because [`NoticeBar::set_menu`] replaces the whole menu, and the
    /// previous message's
    /// entries would offer to always-allow the wrong sender.
    fn rebuild_menu(&self) {
        let Some(sender) = self.sender.borrow().clone() else {
            self.notice.set_menu(Vec::new());
            return;
        };
        let handlers = self.always_allow.borrow().clone();
        // Middle-truncated: a private-relay address is 70 characters, which made this notice three
        // lines tall (#1008).
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
        // "This domain" is a wider promise than "this sender": a shop that mails from a new address
        // per order needs the domain.
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

/// Native for the same reason as [`RemoteImageBanner`]: it reports on the document below it. It
/// offers no way out, since
/// re-decoding with another guess would invent a second answer. It says little on purpose: the
/// words below may not be the
/// words that were sent, so check the original before acting on it.
pub struct DecodeNotice {
    notice: Rc<NoticeBar>,
}

impl DecodeNotice {
    pub fn new() -> Self {
        let notice = NoticeBar::new("dialog-warning-symbolic", "postio-decode-notice");
        // The words are `postio-ui`'s: the macOS reader makes the same claim about the same flag,
        // and twice-typed caveats drift (#1585).
        notice.set_text(postio_ui::reader::document::DECODE_CAVEAT);
        DecodeNotice { notice }
    }

    pub fn widget(&self) -> gtk::Widget {
        self.notice.widget()
    }

    pub fn set_visible(&self, visible: bool) {
        self.notice.set_visible(visible);
    }

    pub fn is_visible(&self) -> bool {
        self.notice.is_visible()
    }
}

impl Default for DecodeNotice {
    fn default() -> Self {
        Self::new()
    }
}

/// Native like its siblings: the activation log outlives the reader, so it cannot be markup inside
/// the document. It takes no
/// local action (leaving a list is a storage write this crate cannot reach); it only asks, and
/// whoever wires the reader decides what that means.
pub struct UnsubscribeBanner {
    /// A [`NoticeBar`] like its siblings, so it matches their height, which the reader's single
    /// notice slot requires.
    notice: Rc<NoticeBar>,
}

impl UnsubscribeBanner {
    pub fn new() -> Self {
        let notice = NoticeBar::new("mail-unread-symbolic", "postio-unsubscribe-banner");
        notice.set_action(Some(postio_ui::unsubscribe::ACTION));
        UnsubscribeBanner { notice }
    }

    pub fn widget(&self) -> gtk::Widget {
        self.notice.widget()
    }

    /// Whether it is on screen is the reader's notice slot's to decide. The sentence is
    /// `postio_ui::unsubscribe::summary`'s,
    /// so macOS says the same thing (#1585).
    pub fn set_list(&self, list: Option<&str>) {
        match list {
            Some(list) => self.notice.set_text(&postio_ui::unsubscribe::summary(list)),
            None => self.notice.set_text(""),
        }
    }

    pub fn set_action_key(&self, key: Option<&str>) {
        self.notice.set_action_key(key);
    }

    pub fn label(&self) -> String {
        self.notice.text()
    }

    pub fn connect_unsubscribe<F: Fn() + 'static>(&self, handler: F) {
        self.notice.connect_action(handler);
    }

    pub fn emit_unsubscribe(&self) {
        self.notice.press_action();
    }
}

impl Default for UnsubscribeBanner {
    fn default() -> Self {
        Self::new()
    }
}
