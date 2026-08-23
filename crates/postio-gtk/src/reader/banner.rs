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
