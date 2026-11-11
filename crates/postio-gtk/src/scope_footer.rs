//! The bar under the list that says what the keys act on (spec 013 US2;
//! `Design/focus-macos-threads` screens 02 and 03): the scope in bold, the
//! keys that matter there as cap and label, and on the right one sentence
//! about what acts on the message and what on the thread.
//!
//! The words are `postio_ui::scope::FooterView`'s, composed once for both
//! apps; this only draws them. Its keys come from the keymap, so a
//! rebinding shows.

use gtk::prelude::*;
use postio_ui::scope::FooterView;
use postio_widgets::widgets::keyhint;

/// The footer bar.
pub struct ScopeFooter {
    root: gtk::Box,
    title: gtk::Label,
    keys: gtk::Box,
    note: gtk::Label,
}

impl ScopeFooter {
    /// An empty bar, drawn once there is something to say.
    pub fn new() -> Self {
        let root = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        root.add_css_class("focus-scope-footer");
        root.set_visible(false);
        let title = gtk::Label::new(None);
        title.add_css_class("focus-scope-footer-title");
        title.set_xalign(0.0);
        title.set_ellipsize(gtk::pango::EllipsizeMode::End);
        let keys = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        let note = gtk::Label::new(None);
        note.add_css_class("dim-label");
        note.add_css_class("focus-scope-footer-note");
        note.set_hexpand(true);
        note.set_xalign(1.0);
        note.set_ellipsize(gtk::pango::EllipsizeMode::End);
        root.append(&title);
        root.append(&keys);
        root.append(&note);
        ScopeFooter {
            root,
            title,
            keys,
            note,
        }
    }

    /// The bar, to place in a layout.
    pub fn widget(&self) -> &gtk::Box {
        &self.root
    }

    /// Draw `view`, replacing what was said.
    pub fn set(&self, view: &FooterView) {
        self.title.set_text(&view.title);
        while let Some(child) = self.keys.first_child() {
            self.keys.remove(&child);
        }
        for (cap, label) in &view.keys {
            let pair = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            pair.append(&keyhint::cap(cap));
            pair.append(&gtk::Label::new(Some(label)));
            self.keys.append(&pair);
        }
        self.note.set_text(view.note.as_deref().unwrap_or_default());
        self.note.set_visible(view.note.is_some());
        self.root.set_visible(!view.title.is_empty());
    }

    /// The scope in words, as drawn.
    pub fn title(&self) -> String {
        self.title.text().to_string()
    }

    /// The right-hand sentence, if there is one.
    pub fn note(&self) -> Option<String> {
        self.note
            .is_visible()
            .then(|| self.note.text().to_string())
            .filter(|note| !note.is_empty())
    }
}

impl Default for ScopeFooter {
    fn default() -> Self {
        Self::new()
    }
}
