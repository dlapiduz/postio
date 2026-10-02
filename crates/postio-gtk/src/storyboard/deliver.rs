//! Delivering a chord, the way GTK would.
//!
//! # Why a chain and not the window's handler
//!
//! Calling the window's key handler proves command resolution and nothing
//! about whether a person's key gets there: it cannot see a dialog's
//! controller consuming the key, or the keyboard left on a widget that has
//! been removed. [`press`] emits `key-pressed` on every
//! `EventControllerKey` along the real focus chain in GTK's phase order, so
//! it can. This is Focus's `support::deliver_with` (T195, T200), promoted
//! out of a test helper.
//!
//! What it cannot run is a widget class's own key bindings (a scroller's, a
//! window's focus moves): GTK offers no way to run them but a real event.
//! Steps that depend on those are `routing = "real"` (research R3).

use gtk::gdk;
use gtk::prelude::*;
use postio_ui::keymap::{Chord, Key, Modifiers};

/// Why a chord has no GDK spelling.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ChordError {
    #[error("GDK has no key named {0:?}")]
    UnknownKey(String),
}

/// The GDK key and modifiers a person would produce for `chord`.
///
/// A character key reports the character it types, so `J` is `Key::J` held
/// with Shift, which is what GTK delivers and what
/// `Chord::from_key_event` folds back into `J`. A chord with an unexpanded
/// `mod` cannot get here: it does not parse as a [`Chord`].
pub fn chord_to_gdk(chord: &Chord) -> Result<(gdk::Key, gdk::ModifierType), ChordError> {
    let name = chord.key.keysym_name();
    let key = gdk::Key::from_name(name.as_str()).ok_or(ChordError::UnknownKey(name))?;
    let mut state = gdk::ModifierType::empty();
    for (modifier, mask) in [
        (Modifiers::CTRL, gdk::ModifierType::CONTROL_MASK),
        (Modifiers::ALT, gdk::ModifierType::ALT_MASK),
        (Modifiers::SHIFT, gdk::ModifierType::SHIFT_MASK),
        (Modifiers::SUPER, gdk::ModifierType::SUPER_MASK),
    ] {
        if chord.modifiers.contains(modifier) {
            state |= mask;
        }
    }
    if matches!(&chord.key, Key::Char(c) if c.is_uppercase()) {
        state |= gdk::ModifierType::SHIFT_MASK;
    }
    Ok((key, state))
}

/// What became of a key press.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Delivery {
    /// A controller claimed the key: the type name of the widget it is on.
    Delivered { stopped_at: String },
    /// Nothing took it: the keyboard was on a widget that has left the
    /// window or is not shown, or no controller on the chain wanted it.
    Dropped,
}

/// Press `chord` in `window` and let the main loop run until idle.
pub fn press(window: &gtk::Window, chord: &Chord) -> Result<Delivery, ChordError> {
    let (key, state) = chord_to_gdk(chord)?;
    let delivery = deliver(window, key, state);
    drain();
    Ok(delivery)
}

/// Run what the key started: pending main-context work, until idle.
pub fn drain() {
    let context = gtk::glib::MainContext::default();
    while context.iteration(false) {}
}

/// The innermost `adw::Dialog` presented over `window`, if any.
///
/// A dialog keeps its own focus: while one is up, `window.focus()` still
/// answers the widget under it, and the keyboard is not there. Dialogs are
/// found by walking the tree, because a plain `gtk::Window` has no list of
/// them; the last mapped one in tree order is the one on top.
pub fn presented_dialog(window: &gtk::Window) -> Option<adw::Dialog> {
    fn walk(widget: &gtk::Widget, found: &mut Option<adw::Dialog>) {
        if let Some(dialog) = widget.downcast_ref::<adw::Dialog>()
            && dialog.is_mapped()
        {
            *found = Some(dialog.clone());
        }
        let mut child = widget.first_child();
        while let Some(current) = child {
            walk(&current, found);
            child = current.next_sibling();
        }
    }
    let mut found = None;
    walk(window.upcast_ref(), &mut found);
    found
}

/// The widget a person's keyboard is on: the innermost dialog's focus, else
/// the window's, else the window itself (nothing has focus).
pub fn keyboard_target(window: &gtk::Window) -> gtk::Widget {
    if let Some(dialog) = presented_dialog(window) {
        return adw::prelude::AdwDialogExt::focus(&dialog).unwrap_or_else(|| dialog.upcast());
    }
    GtkWindowExt::focus(window).unwrap_or_else(|| window.clone().upcast())
}

/// Emit `key-pressed` along the focus chain without draining.
///
/// GTK runs a key through the widgets from the focus up to the innermost
/// dialog presented over the window, and no further: a controller on the
/// window, or anywhere between it and the dialog, never sees a key the
/// dialog's keyboard is given. Measured with real key presses (focus T195,
/// GTK 4.22, libadwaita 1.9).
pub fn deliver(window: &gtk::Window, key: gdk::Key, state: gdk::ModifierType) -> Delivery {
    let keyval = gtk::glib::translate::IntoGlib::into_glib(key);
    let target = keyboard_target(window);
    // GTK's key path starts at the focus and climbs its parents. A focus
    // that has left the window has none, so no key reaches anything: a
    // person's keyboard is dead until a click.
    if target.root().is_none() || !target.is_mapped() {
        return Delivery::Dropped;
    }
    let mut chain = vec![target.clone()];
    while !chain.last().is_some_and(|w| w.is::<adw::Dialog>())
        && let Some(parent) = chain.last().and_then(|w| w.parent())
    {
        chain.push(parent);
    }
    let fire = |widget: &gtk::Widget, phase: gtk::PropagationPhase| -> bool {
        let controllers = widget.observe_controllers();
        (0..controllers.n_items()).any(|at| {
            let Some(keys) = controllers
                .item(at)
                .and_downcast::<gtk::EventControllerKey>()
            else {
                return false;
            };
            keys.propagation_phase() == phase
                && keys.emit_by_name::<bool>("key-pressed", &[&keyval, &0u32, &state])
        })
    };
    let first = |phase: gtk::PropagationPhase, widgets: Vec<&gtk::Widget>| {
        widgets
            .into_iter()
            .find(|w| fire(w, phase))
            .map(|w| w.type_().name().to_owned())
    };
    let stopped = first(gtk::PropagationPhase::Capture, chain.iter().rev().collect())
        .or_else(|| first(gtk::PropagationPhase::Target, vec![&target]))
        .or_else(|| first(gtk::PropagationPhase::Bubble, chain.iter().collect()));
    match stopped {
        Some(stopped_at) => Delivery::Delivered { stopped_at },
        None => Delivery::Dropped,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keymap::ChordFromGdk;

    fn chord(text: &str) -> Chord {
        text.parse().expect("a chord")
    }

    #[test]
    fn a_lowercase_letter_is_its_key_with_no_modifier() {
        let (key, state) = chord_to_gdk(&chord("j")).expect("a key");
        assert_eq!(key, gdk::Key::j);
        assert!(state.is_empty());
    }

    #[test]
    fn an_uppercase_letter_is_its_key_held_with_shift() {
        let (key, state) = chord_to_gdk(&chord("J")).expect("a key");
        assert_eq!(key, gdk::Key::J);
        assert_eq!(state, gdk::ModifierType::SHIFT_MASK);
    }

    #[test]
    fn chords_round_trip_through_the_gdk_bridge() {
        for text in ["ctrl+shift+Tab", "?", "ctrl+Return", "Escape", "J", "alt+a"] {
            let original = chord(text);
            let (key, state) = chord_to_gdk(&original).expect("a key");
            assert_eq!(
                Chord::from_key_event(key, state),
                Some(original),
                "{text} did not survive the trip"
            );
        }
    }

    #[test]
    fn an_unexpanded_mod_never_becomes_a_chord() {
        assert_eq!(
            "mod+z".parse::<Chord>().unwrap_err(),
            postio_ui::keymap::ParseError::UnexpandedMod
        );
    }
}
