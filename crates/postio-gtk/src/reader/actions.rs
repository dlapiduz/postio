//! The reading pane's action bar (#498): Reply, Reply all, Forward and
//! Archive, under the body — the pointer's way to the same four verbs `e`,
//! `E`, `f` and `a` already reach from the keyboard. Before this the reading
//! pane had no click target for any of them at all: a mouse user reading a
//! message had nothing to click, and nothing on screen said the keys existed.
//!
//! # What is left here is the *contents* of the bar
//!
//! The bar itself is [`crate::widgets::ActionBar`] (#1002), which is also what
//! the conversation pane's per-message buttons and its footer are built from
//! — three surfaces that were three hand-rolled boxes of `gtk::Button`, only
//! one of which read the live keymap. What this module still owns is which
//! four verbs the reading pane offers and in what order.

use std::rc::Rc;

use postio_core::CommandId;

use crate::widgets::{Action, ActionBar};

/// The four verbs, in canvas order, with Reply as the primary.
pub const ACTIONS: [Action; 4] = [
    Action::new(CommandId::Reply, "Reply", "postio-reader-action-reply").primary(),
    Action::new(
        CommandId::ReplyAll,
        "Reply all",
        "postio-reader-action-reply-all",
    ),
    Action::new(CommandId::Forward, "Forward", "postio-reader-action-forward"),
    Action::new(CommandId::Archive, "Archive", "postio-reader-action-archive"),
];

/// Build the reading pane's bar, hidden.
///
/// [`super::view::Reader`] shows it only while a message actually occupies
/// the pane, and re-hides it the moment the pane empties or the composer
/// takes over.
pub fn new() -> Rc<ActionBar> {
    let bar = ActionBar::new(&ACTIONS, "postio-reader-actions");
    bar.set_visible(false);

    // The canvas right-aligns a thread-position status after these four
    // ("2/6 · n next in thread"); the reader has no notion of thread position
    // yet, so this only reserves the space rather than fabricating a count.
    // See #498's PR for the reasoning.
    bar.append_trailing(&gtk::Box::new(gtk::Orientation::Horizontal, 0));
    bar
}


#[cfg(test)]
mod tests {
    use super::*;
    use postio_core::Keymap;

    /// The keys the four verbs currently carry.
    fn hints(keymap: &Keymap) -> Vec<(CommandId, Option<String>)> {
        crate::widgets::action_bar::keys(&ACTIONS, keymap)
    }

    #[test]
    fn the_default_keys_are_the_ones_the_registry_gives_reply_reply_all_forward_and_archive() {
        let keymap = Keymap::resolve(&Default::default());
        let keys: Vec<_> = hints(&keymap).into_iter().map(|(_, key)| key).collect();
        assert_eq!(
            keys,
            vec![
                Some("e".to_string()),
                Some("E".to_string()),
                Some("f".to_string()),
                Some("a".to_string()),
            ],
            "canvas order: Reply, Reply all, Forward, Archive"
        );
    }

    #[test]
    fn a_rebind_in_keys_reaches_the_bar_not_just_the_resolver() {
        let mut overrides = postio_config::KeyBindings::default();
        overrides
            .overrides_mut()
            .insert("reply".to_string(), "r".to_string());
        let keymap = Keymap::resolve(&overrides);
        let keys: Vec<_> = hints(&keymap).into_iter().map(|(_, key)| key).collect();
        assert_eq!(keys[0], Some("r".to_string()), "Reply picked up the rebind");
        assert_eq!(
            keys[1],
            Some("E".to_string()),
            "Reply all kept its default; only Reply was rebound"
        );
    }

    #[test]
    fn a_key_lost_to_another_command_hides_the_hint_rather_than_showing_a_wrong_one() {
        let mut overrides = postio_config::KeyBindings::default();
        // `undo`'s default is `u`; rebinding it to `a` collides with
        // Archive's own default in every context they share, so Archive
        // loses its key rather than being handed one that now runs something
        // else.
        overrides
            .overrides_mut()
            .insert("undo".to_string(), "a".to_string());
        let keymap = Keymap::resolve(&overrides);
        let keys: Vec<_> = hints(&keymap).into_iter().map(|(_, key)| key).collect();
        assert_eq!(
            keys[3], None,
            "Archive has no key to show once undo has taken `a`"
        );
    }
}
