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

use postio_ui::reader::header::ReaderAction;

use crate::widgets::{Action, ActionBar};

/// The four verbs, in canvas order, with Reply as the primary.
///
/// Which verbs, in what order, what each is labelled and which command it
/// runs all come from [`ReaderAction`] — `postio-ui` owns them so that the
/// macOS frontend offers the same four in the same order (#1259, #1285).
/// What stays here is the only part with a toolkit in it: the CSS class each
/// button carries.
///
/// [`ReaderAction::title`] and [`ReaderAction::command`] are `const` for
/// exactly this: a label this array could not read at compile time would have
/// to be written again here, which is the duplication the shared list exists
/// to prevent.
pub const ACTIONS: [Action; 4] = [
    button(ReaderAction::Reply, "postio-reader-action-reply"),
    button(ReaderAction::ReplyAll, "postio-reader-action-reply-all"),
    button(ReaderAction::Forward, "postio-reader-action-forward"),
    button(ReaderAction::Archive, "postio-reader-action-archive"),
];

/// One verb, dressed for GTK.
const fn button(verb: ReaderAction, class: &'static str) -> Action {
    let action = Action::new(verb.command(), verb.title(), class);
    if verb.primary() {
        action.primary()
    } else {
        action
    }
}

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
    use postio_core::{CommandId, Context, Keymap};

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
    fn a_key_lost_to_another_command_is_never_shown_as_still_working() {
        let mut overrides = postio_config::KeyBindings::default();
        // `undo`'s default is `u`; rebinding it to `a` takes Archive's own
        // default in every context they share.
        overrides
            .overrides_mut()
            .insert("undo".to_string(), "a".to_string());
        let keymap = Keymap::resolve(&overrides);

        let archive = hints(&keymap)
            .into_iter()
            .find(|(command, _)| *command == CommandId::Archive)
            .expect("Archive is one of the four verbs")
            .1;

        // The guarantee is **never a key that runs something else**, not
        // "never a key". Archive may still show an alternate binding of its
        // own — it gains `mod+shift+a` with the second keyboard layer — and
        // asserting `None` here would make that correct behaviour look like a
        // regression. What must never happen is teaching a key that now runs
        // undo.
        if let Some(key) = archive.as_deref() {
            assert_eq!(
                keymap.command_for(Context::Reader, key),
                Some(CommandId::Archive.into()),
                "a hint must name a key that still runs Archive, never one another command has taken"
            );
        }
    }

    #[test]
    fn a_verb_with_every_key_taken_shows_no_hint_at_all() {
        // The other half of the same guarantee: when nothing is left, the
        // answer is no hint rather than a wrong one. Every key Archive holds
        // is handed to another command, so this stays true however many
        // alternates Archive gains later.
        let defaults = Keymap::resolve(&postio_config::KeyBindings::default());
        let mut overrides = postio_config::KeyBindings::default();
        for (nth, key) in defaults.bindings(CommandId::Archive).iter().enumerate() {
            // `undo` first, then any other verb that is not Archive itself,
            // so each of Archive's keys is genuinely owned by someone else.
            let thief = if nth == 0 { "undo" } else { "reply_all" };
            overrides
                .overrides_mut()
                .insert(thief.to_string(), key.clone());
        }
        let keymap = Keymap::resolve(&overrides);

        let archive = hints(&keymap)
            .into_iter()
            .find(|(command, _)| *command == CommandId::Archive)
            .expect("Archive is one of the four verbs")
            .1;

        assert_eq!(
            archive, None,
            "with every key taken there is nothing honest left to show"
        );
    }
}
