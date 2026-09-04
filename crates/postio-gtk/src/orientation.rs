//! The first-run keyboard orientation: a one-time, dismissible strip that
//! says Postio is keyboard-first (ADR 0012 Q4-Q6).
//!
//! Not a tour and not modal — it never takes focus, never blocks the list,
//! and goes away the moment the user dismisses it or runs their first
//! command, whichever comes first. This module owns the widget's shape and
//! its text; *when* it shows, and remembering that it has, are
//! `postio-app`'s (`postio_app::orientation`), because both need the store
//! and the view layer may not link `rusqlite`.

use adw::prelude::*;
use postio_core::{CommandId, Keymap};

/// What the strip is about, in the heading face.
///
/// The one sentence a new user needs; everything else on the strip is the
/// three keys that make it true.
const TITLE: &str = "Postio is keyboard-first";

/// One clause of the orientation: what it does, and the key that does it.
///
/// The same shape [`crate::list_state`] draws its named states' hints in,
/// and drawn with the same `.postio-keyhint` chip, because this is the same
/// promise: a key named on screen beside what it does. The key is a
/// `String` rather than a `&'static str` — it comes from the live keymap
/// and there is no static to borrow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hint {
    /// What the key does, in the vocabulary `docs/PRODUCT.md` §8 uses.
    pub label: &'static str,
    /// The key in force right now, as the cheat sheet spells it.
    pub key: String,
}

/// What the plate teaches, rendered from the keymap in force.
///
/// ADR 0012 Q5: a hard-coded `"Ctrl+K"` is wrong the moment somebody
/// rebinds `[keys]`, and rebinding things is the first thing a
/// keyboard-first user does. A command this build cannot find a key for —
/// unbound by an override, reachable only from the palette — is left out
/// rather than printed as a lie, so an empty answer is possible and means
/// there is nothing here worth showing.
pub fn hints(keymap: &Keymap) -> Vec<Hint> {
    let mut hints = Vec::new();
    if let Some(key) = keymap.binding(CommandId::CommandPalette) {
        hints.push(Hint {
            label: "Command palette",
            key: key.to_string(),
        });
    }
    if let Some(key) = keymap.binding(CommandId::CheatSheet) {
        hints.push(Hint {
            label: "Every key",
            key: key.to_string(),
        });
    }
    // One clause, not two: `j` and `k` are one idea, and a strip that spends
    // two of its three slots saying "next" and "previous" teaches less than
    // one that says movement and then stops.
    if let (Some(next), Some(prev)) = (
        keymap.binding(CommandId::NextMessage),
        keymap.binding(CommandId::PrevMessage),
    ) {
        hints.push(Hint {
            label: "Move between messages",
            key: format!("{next}/{prev}"),
        });
    }
    hints
}

/// The whole strip as one sentence, for a screen reader.
///
/// The chips are a label set apart from a key, which read aloud is
/// "Command palette ctrl+k" — not a sentence. This is, and it follows
/// [`crate::cheatsheet::spoken`]'s wording so the two surfaces teaching the
/// same keys say it the same way.
pub fn spoken(hints: &[Hint]) -> String {
    let mut sentence = String::from("Postio is keyboard-first.");
    for hint in hints {
        sentence.push_str(&format!(" {}, press {}.", hint.label, hint.key));
    }
    sentence
}

/// The strip itself: what it teaches, and one button that ends it.
///
/// A strip along the top of the message column, not a dialog and not an
/// overlay: ADR 0012 Q4 says it must not take focus, must not block the
/// list, and must not intercept keys — so it is an ordinary sibling of the
/// rows, and the same [`Placement::Banner`](crate::list_state::Placement)
/// treatment the named states use when there is mail underneath them.
///
/// Built hidden. `postio-app` is what knows whether there is anything to
/// say (`postio_app::orientation`).
#[derive(Clone)]
pub struct OrientationStrip {
    root: gtk::Box,
    hints: gtk::FlowBox,
    dismiss: gtk::Button,
    /// Whether it is over: the user has dismissed it, or has used the
    /// command system, which is the same answer. Latched, so the handlers
    /// below run once per window however many keys follow.
    retired: std::rc::Rc<std::cell::Cell<bool>>,
    /// Who to tell when that happens — `postio-app`, which writes it down.
    on_retired: std::rc::Rc<std::cell::RefCell<Vec<Box<dyn Fn()>>>>,
    /// Whether the keymap in force gives it anything to teach. A strip with
    /// no keys on it is a title and a "Got it" button over somebody's mail,
    /// so it stays hidden however loudly it is asked to show.
    taught: std::rc::Rc<std::cell::Cell<bool>>,
}

impl OrientationStrip {
    /// Build it, hidden and empty until a keymap arrives.
    pub fn new() -> Self {
        let root = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        root.add_css_class("postio-orientation");
        root.set_visible(false);
        // A group with a name, rather than a row of unrelated labels: the
        // chips read aloud as "Command palette ctrl+k", which is not a
        // sentence, so the whole strip carries one that is.
        root.set_accessible_role(gtk::AccessibleRole::Group);
        root.set_margin_start(16);
        root.set_margin_end(16);
        root.set_margin_top(10);
        root.set_margin_bottom(10);

        let icon = gtk::Image::from_icon_name("input-keyboard-symbolic");
        icon.set_pixel_size(20);
        icon.set_valign(gtk::Align::Start);
        icon.add_css_class("postio-orientation-icon");
        icon.set_accessible_role(gtk::AccessibleRole::Presentation);
        root.append(&icon);

        let text = gtk::Box::new(gtk::Orientation::Vertical, 4);
        text.set_hexpand(true);

        let title = gtk::Label::new(Some(TITLE));
        title.add_css_class("postio-orientation-title");
        title.set_xalign(0.0);
        title.set_wrap(true);
        title.set_accessible_role(gtk::AccessibleRole::Presentation);
        text.append(&title);

        // The keys wrap rather than shrink. The message column is allowed
        // down to 280px (`Shell`'s own size request) and high contrast takes
        // it to about 400 at a full-width window, and a row that can only
        // give ground by cutting words reaches those widths as
        // "Co… ctrl+k  Ev… ?  Mo… j/k" — every word gone and every key
        // kept, which teaches nobody anything. Stacking costs height for one
        // moment of one run; ellipsis costs the whole point of the strip.
        let hints = gtk::FlowBox::new();
        hints.set_selection_mode(gtk::SelectionMode::None);
        hints.set_min_children_per_line(1);
        hints.set_max_children_per_line(3);
        hints.set_column_spacing(16);
        hints.set_row_spacing(2);
        hints.set_homogeneous(false);
        // Not a list to arrow around: the keys are what it is teaching, and
        // a strip that took the keyboard would be the modal ADR 0012 Q4
        // rules out.
        hints.set_focusable(false);
        hints.set_accessible_role(gtk::AccessibleRole::Presentation);
        text.append(&hints);
        root.append(&text);

        // "Got it" rather than a close cross: the ADR asks for one
        // dismissal, and a labelled button says what dismissing means where
        // an X only says that something can be got rid of. Last child, and
        // the only one that never gives ground.
        let dismiss = gtk::Button::with_label("Got it");
        dismiss.add_css_class("flat");
        dismiss.add_css_class("postio-orientation-dismiss");
        dismiss.set_valign(gtk::Align::Start);
        root.append(&dismiss);

        OrientationStrip {
            root,
            hints,
            dismiss,
            retired: std::rc::Rc::new(std::cell::Cell::new(false)),
            on_retired: std::rc::Rc::new(std::cell::RefCell::new(Vec::new())),
            taught: std::rc::Rc::new(std::cell::Cell::new(false)),
        }
    }

    /// The widget to place above the message list.
    pub fn widget(&self) -> gtk::Widget {
        self.root.clone().upcast()
    }

    /// Draw the keys the keymap currently binds.
    ///
    /// Called again whenever `[keys]` is reloaded, like every other surface
    /// that names a key — the strip is short-lived, but a live config
    /// change during the one moment it is on screen must not leave it
    /// teaching the old key.
    pub fn set_keymap(&self, keymap: &Keymap) {
        while let Some(child) = self.hints.first_child() {
            self.hints.remove(&child);
        }
        let hints = hints(keymap);
        for hint in &hints {
            let child = gtk::FlowBoxChild::new();
            child.set_child(Some(&chip(hint)));
            child.set_focusable(false);
            child.set_accessible_role(gtk::AccessibleRole::Presentation);
            self.hints.append(&child);
        }
        self.root
            .update_property(&[gtk::accessible::Property::Label(&spoken(&hints))]);
        self.taught.set(!hints.is_empty());
        if !self.taught.get() {
            self.root.set_visible(false);
        }
    }

    /// Show or hide it.
    ///
    /// Showing is a request rather than a command: with nothing to teach —
    /// every key it would have named rebound away — there is nothing to put
    /// on screen, and a strip that appeared anyway would be a title and a
    /// button sitting on top of the mail.
    pub fn set_visible(&self, visible: bool) {
        self.root
            .set_visible(visible && self.taught.get() && !self.retired.get());
    }

    /// Whether it is on screen.
    pub fn is_visible(&self) -> bool {
        self.root.is_visible()
    }

    /// Called when the user presses "Got it".
    ///
    /// The window subscribes and routes it into [`retire`](Self::retire), so
    /// the button and the first keystroke end this the same way rather than
    /// by two paths that have to be kept in step.
    pub fn connect_dismissed(&self, handler: impl Fn() + 'static) {
        self.dismiss.connect_clicked(move |_| handler());
    }

    /// The user is done with it, for good.
    ///
    /// Called for the dismiss button and for the first command the window
    /// resolves from the keyboard or the palette — ADR 0012 Q6 counts both
    /// as the user demonstrating they know, and counts neither a click nor a
    /// modifier press. It fires whether or not the strip was ever on screen,
    /// because somebody who pressed `j` before the first sync finished
    /// should never meet it at all.
    ///
    /// Latched: the second call and every one after it does nothing, so the
    /// write behind it happens once rather than on every keystroke for the
    /// rest of the session.
    pub fn retire(&self) {
        if self.retired.replace(true) {
            return;
        }
        self.root.set_visible(false);
        for handler in self.on_retired.borrow().iter() {
            handler();
        }
    }

    /// Called the first time it is retired, and never again.
    pub fn connect_retired(&self, handler: impl Fn() + 'static) {
        self.on_retired.borrow_mut().push(Box::new(handler));
    }
}

impl Default for OrientationStrip {
    fn default() -> Self {
        Self::new()
    }
}

/// One hint, drawn the way the list's named states draw theirs: what it
/// does, then the key in the mono face every key hint in the app wears.
fn chip(hint: &Hint) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    row.add_css_class("postio-orientation-hint");
    row.set_accessible_role(gtk::AccessibleRole::Presentation);

    let label = gtk::Label::new(Some(hint.label));
    label.add_css_class("postio-orientation-hint-label");
    // The last thing to give, and it still gives: at the column's own 280px
    // minimum even one chip per line is too wide for "Move between
    // messages", and a label that cannot wrap would take the button off the
    // end with it. It never wraps at any width somebody reads mail at.
    label.set_wrap(true);
    label.set_xalign(0.0);
    label.set_accessible_role(gtk::AccessibleRole::Presentation);
    row.append(&label);

    let key = gtk::Label::new(Some(&hint.key));
    key.add_css_class("postio-keyhint");
    key.set_accessible_role(gtk::AccessibleRole::Presentation);
    row.append(&key);

    row
}

#[cfg(test)]
mod tests {
    use super::*;
    use postio_config::KeyBindings;
    use postio_config::paths::Platform;

    /// Resolved for a named platform rather than the host: `mod+k` expands to
    /// `ctrl+k` on one and `cmd+k` on another, and a test that asserted the
    /// host's answer would say different things on the two machines this
    /// workspace builds on.
    fn keymap(bindings: &KeyBindings) -> Keymap {
        Keymap::resolve_on(bindings, Platform::Freedesktop)
    }

    #[test]
    fn the_hints_name_the_palette_the_sheet_and_the_two_movement_keys() {
        let keymap = keymap(&KeyBindings::default());
        let hints = hints(&keymap);

        let keys: Vec<&str> = hints.iter().map(|hint| hint.key.as_str()).collect();
        assert_eq!(
            keys,
            vec!["ctrl+k", "?", "j/k"],
            "the plate teaches the palette, the cheat sheet and movement, in that order"
        );
    }

    #[test]
    fn a_rebound_key_is_taught_instead_of_the_default() {
        let mut bindings = KeyBindings::default();
        bindings
            .overrides_mut()
            .insert("command_palette".into(), "ctrl+shift+p".into());
        let keymap = keymap(&bindings);

        let palette = hints(&keymap)
            .into_iter()
            .next()
            .expect("the palette is still taught");
        assert_eq!(palette.key, "ctrl+shift+p");
    }

    #[test]
    fn spoken_reads_as_sentences_rather_than_a_label_beside_a_key() {
        let keymap = keymap(&KeyBindings::default());
        let spoken = spoken(&hints(&keymap));

        assert!(spoken.starts_with("Postio is keyboard-first."), "{spoken}");
        assert!(spoken.contains("Command palette, press ctrl+k."), "{spoken}");
        assert!(spoken.contains("Move between messages, press j/k."), "{spoken}");
    }
}
