//! The first-run keyboard orientation: a one-time, dismissible plate saying
//! Postio is keyboard-first (ADR 0012 Q4-Q6).
//!
//! Not a tour and not modal — it never takes focus, never blocks the list,
//! and disappears the moment the user dismisses it or runs their first
//! command, whichever comes first. This module owns only the widget's shape
//! and its text; *when* to show it, and remembering that it has already
//! been shown, are `postio-app`'s job (the plate has no database access and
//! must not gain one — see `scripts/check-crate-boundaries.py`).

use adw::prelude::*;
use postio_core::{CommandId, Keymap};

/// What the plate says, rendered from the live keymap rather than written
/// out.
///
/// ADR 0012 Q5: a hard-coded `"Ctrl+K"` is wrong the moment a user rebinds
/// `[keys]`, and rebinding things is the first thing a keyboard-first user
/// does. Any binding this build cannot find — a command a user's override
/// left with no key at all, reachable only from the palette — is left out
/// of the sentence rather than printed as a lie or a placeholder.
pub fn orientation_text(keymap: &Keymap) -> String {
    let mut clauses = Vec::new();
    if let Some(key) = keymap.binding(CommandId::CommandPalette) {
        clauses.push(format!("{key} opens the command palette"));
    }
    if let Some(key) = keymap.binding(CommandId::CheatSheet) {
        clauses.push(format!("{key} shows every key"));
    }
    if let (Some(prev), Some(next)) = (
        keymap.binding(CommandId::PrevMessage),
        keymap.binding(CommandId::NextMessage),
    ) {
        clauses.push(format!("{prev}/{next} move between messages"));
    }
    if clauses.is_empty() {
        // Every binding this sentence would have named is gone -- rebound
        // into collisions with each other, say. Naming the one surface that
        // is always true regardless of `[keys]` beats saying nothing.
        return "Postio is keyboard-first. Open the command palette from the menu \
                 to see every key."
            .to_string();
    }
    format!("Postio is keyboard-first: {}.", join_and(&clauses))
}

/// `"a"`, `"a, and b"`, `"a, b, and c"` — never an Oxford-comma-less list a
/// screen reader runs together.
fn join_and(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [a, b] => format!("{a}, and {b}"),
        many => {
            let (last, rest) = many.split_last().expect("checked non-empty above");
            format!("{}, and {last}", rest.join(", "))
        }
    }
}

/// The plate itself: a label and a dismiss button, hidden until
/// `postio-app` has something to show.
#[derive(Clone)]
pub struct OrientationPlate {
    root: gtk::Box,
    label: gtk::Label,
    dismiss: gtk::Button,
}

impl OrientationPlate {
    /// Build the plate, hidden.
    pub fn new() -> Self {
        let root = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        root.add_css_class("postio-orientation-plate");
        root.set_visible(false);
        root.set_accessible_role(gtk::AccessibleRole::Group);

        let icon = gtk::Image::from_icon_name("input-keyboard-symbolic");
        root.append(&icon);

        let label = gtk::Label::new(None);
        label.set_hexpand(true);
        label.set_xalign(0.0);
        label.set_wrap(true);
        label.add_css_class("postio-orientation-label");
        root.append(&label);

        let dismiss = gtk::Button::with_label("Got it");
        dismiss.add_css_class("flat");
        root.append(&dismiss);

        OrientationPlate {
            root,
            label,
            dismiss,
        }
    }

    /// The widget to place above the message list.
    pub fn widget(&self) -> gtk::Widget {
        self.root.clone().upcast()
    }

    /// Fill in the text for the keymap currently in force.
    pub fn set_keymap(&self, keymap: &Keymap) {
        self.label.set_text(&orientation_text(keymap));
    }

    /// Show or hide the plate.
    pub fn set_visible(&self, visible: bool) {
        self.root.set_visible(visible);
    }

    /// Whether the plate is currently on screen.
    pub fn is_visible(&self) -> bool {
        self.root.is_visible()
    }

    /// Called when the user dismisses the plate explicitly.
    pub fn connect_dismissed(&self, handler: impl Fn() + 'static) {
        self.dismiss.connect_clicked(move |_| handler());
    }

    /// Simulate a click on "Got it" — what a test uses in place of a
    /// synthesized pointer click.
    pub fn emit_dismiss(&self) {
        self.dismiss.emit_clicked();
    }
}

impl Default for OrientationPlate {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use postio_config::KeyBindings;

    #[test]
    fn the_text_names_all_three_bindings_in_the_default_keymap() {
        let keymap = Keymap::resolve(&KeyBindings::default());
        let text = orientation_text(&keymap);
        assert!(text.contains("ctrl+k"), "{text}");
        assert!(text.contains("opens the command palette"), "{text}");
        assert!(text.contains('?'), "{text}");
        assert!(text.contains("shows every key"), "{text}");
        assert!(text.contains("k/j"), "{text}");
        assert!(text.contains("move between messages"), "{text}");
    }

    #[test]
    fn a_rebound_key_is_reflected_rather_than_the_default() {
        let mut bindings = KeyBindings::default();
        bindings
            .overrides_mut()
            .insert("command_palette".into(), "ctrl+shift+p".into());
        let keymap = Keymap::resolve(&bindings);
        let text = orientation_text(&keymap);
        assert!(text.contains("ctrl+shift+p"), "{text}");
        assert!(!text.contains("ctrl+k"), "{text}");
    }

    #[test]
    fn join_and_reads_naturally_at_every_length() {
        assert_eq!(join_and(&[]), "");
        assert_eq!(join_and(&["a".to_string()]), "a");
        assert_eq!(join_and(&["a".to_string(), "b".to_string()]), "a, and b");
        assert_eq!(
            join_and(&["a".to_string(), "b".to_string(), "c".to_string()]),
            "a, b, and c"
        );
    }
}
