//! Delivers a chord by emitting `key-pressed` along the real focus chain in GTK's phase order.
//! Widget-class bindings (a scroller's, a window's focus moves) cannot run this way; those steps
//! use `routing = "real"`.

use gtk::gdk;
use gtk::prelude::*;
use postio_ui::keymap::{Chord, Key, Modifiers};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ChordError {
    #[error("GDK has no key named {0:?}")]
    UnknownKey(String),
}

/// A character key reports the character it types: `J` is `Key::J` held with Shift.
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Delivery {
    Delivered {
        stopped_at: String,
    },
    /// Nothing took it: the keyboard was on a widget that has left the
    /// window or is not shown, or no controller on the chain wanted it.
    Dropped,
}

pub fn press(window: &gtk::Window, chord: &Chord) -> Result<Delivery, ChordError> {
    let (key, state) = chord_to_gdk(chord)?;
    // A real key press turns the window's focus rings on, so a menu opened by it shows where the
    // keyboard is; a synthesised one must too, or the film shows a keyboard nobody can see.
    window.set_focus_visible(true);
    let delivery = deliver(window, key, state);
    drain();
    Ok(delivery)
}

pub fn drain() {
    let context = gtk::glib::MainContext::default();
    while context.iteration(false) {}
}

/// A dialog keeps its own focus, so `window.focus()` is not where the keyboard is.
/// Found by tree walk; the last mapped one is on top.
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

pub fn keyboard_target(window: &gtk::Window) -> gtk::Widget {
    if let Some(dialog) = presented_dialog(window) {
        return adw::prelude::AdwDialogExt::focus(&dialog).unwrap_or_else(|| dialog.upcast());
    }
    GtkWindowExt::focus(window).unwrap_or_else(|| window.clone().upcast())
}

/// GTK stops a key at the innermost dialog: controllers on the window or between never see it
/// (measured, GTK 4.22, libadwaita 1.9).
pub fn deliver(window: &gtk::Window, key: gdk::Key, state: gdk::ModifierType) -> Delivery {
    let keyval = gtk::glib::translate::IntoGlib::into_glib(key);
    let target = keyboard_target(window);
    // GTK's key path starts at the focus and climbs its parents. A focus that has left the window
    // has none, so no key reaches anything: a person's keyboard is dead until a click.
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
        None => activate_text(&target, key, state).unwrap_or(Delivery::Dropped),
    }
}

/// `GtkText` activates on `Return` through a class shortcut the chain walk never reaches,
/// so this mirrors that one binding.
fn activate_text(
    target: &gtk::Widget,
    key: gdk::Key,
    state: gdk::ModifierType,
) -> Option<Delivery> {
    let enter = matches!(
        key,
        gdk::Key::Return | gdk::Key::KP_Enter | gdk::Key::ISO_Enter
    );
    if !enter || !state.is_empty() {
        return None;
    }
    let text = target.downcast_ref::<gtk::Text>()?;
    text.emit_by_name::<()>("activate", &[]);
    Some(Delivery::Delivered {
        stopped_at: "GtkText (activate)".to_owned(),
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypeOutcome {
    Typed,
    /// The keyboard is on nothing that takes text, so the step fails (#1473).
    NothingToTypeInto,
}

/// Types into a widget this module cannot (the composer's web body); returns whether it took the
/// text.
pub type TypeInto<'a> = &'a dyn Fn(&gtk::Widget, &str) -> bool;

/// An editable takes it at the cursor, a text view at the insert mark, anything else goes to
/// `hook`.
pub fn type_text(window: &gtk::Window, text: &str, hook: Option<TypeInto>) -> TypeOutcome {
    let target = keyboard_target(window);
    if !target.is_mapped() || target.root().is_none() {
        return TypeOutcome::NothingToTypeInto;
    }
    if let Some(editable) = target.dynamic_cast_ref::<gtk::Editable>() {
        if editable.is_editable() {
            editable.delete_selection();
            let mut position = editable.position();
            editable.insert_text(text, &mut position);
            editable.set_position(position);
            drain();
            return TypeOutcome::Typed;
        }
    } else if let Some(view) = target.downcast_ref::<gtk::TextView>() {
        let buffer = view.buffer();
        buffer.delete_selection(true, view.is_editable());
        if buffer.insert_interactive_at_cursor(text, view.is_editable()) {
            drain();
            return TypeOutcome::Typed;
        }
    }
    if hook.is_some_and(|hook| hook(&target, text)) {
        drain();
        return TypeOutcome::Typed;
    }
    TypeOutcome::NothingToTypeInto
}

#[cfg(test)]
mod tests {
    use super::*;

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
                crate::keys::chord(key, state),
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
