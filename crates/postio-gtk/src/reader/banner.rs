//! The "remote images blocked" banner: `postio-xxz`.
//!
//! A native GTK widget, not something drawn inside the `WebView` — the pane
//! it sits above is exactly the thing the banner is reporting on, and it has
//! to keep working even when every remote image in a message stays blocked
//! forever. `Reader` shows and hides it and fills in the sender; this module
//! only owns its shape and its two actions.

use adw::prelude::*;

/// "Remote images are blocked", with a way to see them once and a way to
/// trust this sender from now on.
pub struct RemoteImageBanner {
    root: gtk::Box,
    show_once: gtk::Button,
    always_allow: gtk::Button,
}

impl RemoteImageBanner {
    /// Build the banner, hidden — [`super::view::Reader`] shows it once it
    /// knows a message actually has remote images to block.
    pub fn new() -> Self {
        let root = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        root.add_css_class("postio-remote-banner");
        root.set_visible(false);
        root.set_accessible_role(gtk::AccessibleRole::Group);

        let icon = gtk::Image::from_icon_name("image-missing-symbolic");
        root.append(&icon);

        let label = gtk::Label::new(Some("Remote images are blocked"));
        label.set_hexpand(true);
        label.set_xalign(0.0);
        label.add_css_class("postio-remote-banner-label");
        root.append(&label);

        let show_once = gtk::Button::with_label("Show images");
        show_once.add_css_class("flat");
        root.append(&show_once);

        let always_allow = gtk::Button::with_label("Always allow");
        always_allow.add_css_class("flat");
        root.append(&always_allow);

        RemoteImageBanner {
            root,
            show_once,
            always_allow,
        }
    }

    /// The widget to place above the reading pane's `WebView`.
    pub fn widget(&self) -> gtk::Widget {
        self.root.clone().upcast()
    }

    /// Show or hide the whole banner.
    pub fn set_visible(&self, visible: bool) {
        self.root.set_visible(visible);
    }

    /// Whether the banner is currently on screen.
    pub fn is_visible(&self) -> bool {
        self.root.is_visible()
    }

    /// Name whose remote images "always allow" would exempt from now on.
    ///
    /// With no sender to name, the button is disabled rather than hidden —
    /// the banner's shape stays constant, and "show once" is still live.
    pub fn set_sender(&self, sender: Option<&str>) {
        match sender {
            Some(sender) => {
                self.always_allow
                    .set_label(&format!("Always allow {sender}"));
                self.always_allow.set_sensitive(true);
            }
            None => {
                self.always_allow.set_label("Always allow");
                self.always_allow.set_sensitive(false);
            }
        }
    }

    /// Called when the user asks to see this one message's images once,
    /// without adding the sender to the standing allow list.
    pub fn connect_show_once<F: Fn() + 'static>(&self, handler: F) {
        self.show_once.connect_clicked(move |_| handler());
    }

    /// Called when the user asks to always allow this message's sender.
    pub fn connect_always_allow<F: Fn() + 'static>(&self, handler: F) {
        self.always_allow.connect_clicked(move |_| handler());
    }

    /// The "always allow" button's current label — names the sender
    /// [`set_sender`](Self::set_sender) was last called with. Test-facing:
    /// production code has no reason to read a label back.
    pub fn always_allow_label(&self) -> String {
        self.always_allow
            .label()
            .map(|s| s.to_string())
            .unwrap_or_default()
    }

    /// Simulate a click on "always allow" — what a test uses in place of a
    /// synthesized pointer click, which WebKitGTK's public API gives no way
    /// to do for a native GTK button.
    pub fn emit_always_allow(&self) {
        self.always_allow.emit_clicked();
    }

    /// As [`emit_always_allow`](Self::emit_always_allow), for "show once".
    pub fn emit_show_once(&self) {
        self.show_once.emit_clicked();
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
    root: gtk::Box,
}

impl DecodeNotice {
    /// Build the notice, hidden.
    pub fn new() -> Self {
        let root = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        root.add_css_class("postio-decode-notice");
        root.set_visible(false);
        root.set_accessible_role(gtk::AccessibleRole::Group);

        let icon = gtk::Image::from_icon_name("dialog-warning-symbolic");
        root.append(&icon);

        let label = gtk::Label::new(Some(
            "Parts of this message could not be decoded, so what is shown \
             may not be what was sent",
        ));
        label.set_hexpand(true);
        label.set_xalign(0.0);
        label.set_wrap(true);
        label.add_css_class("postio-decode-notice-label");
        root.append(&label);

        DecodeNotice { root }
    }

    /// The widget to place above the reading pane's `WebView`.
    pub fn widget(&self) -> gtk::Widget {
        self.root.clone().upcast()
    }

    /// Show or hide it.
    pub fn set_visible(&self, visible: bool) {
        self.root.set_visible(visible);
    }

    /// Whether it is showing — what a test asserts on.
    pub fn is_visible(&self) -> bool {
        self.root.get_visible()
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
