//! Focus binds only the commands it offers, so a key the keymap keeps for another frontend
//! (`ctrl+shift+e` hands the terminal's composer to an editor) does what an unbound key does here,
//! rather than reaching a command Focus would refuse.

use postio_core::{Frontend, Keymap};
use postio_ui::keymap::Resolver;

pub fn resolver(keymap: &Keymap) -> (Resolver, Vec<String>) {
    Resolver::from_commands_for(keymap, Frontend::Focus)
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use postio_ui::keymap::{Chord, KeyContext, Outcome};

    use super::*;

    fn press(resolver: &mut Resolver, key: &str, context: KeyContext) -> Outcome {
        let chord: Chord = key.parse().expect("a chord");
        resolver.press(&chord, context, false, Instant::now())
    }

    #[test]
    fn focus_binds_its_own_commands_and_not_the_classic_apps() {
        let (mut resolver, problems) = resolver(Keymap::defaults());
        assert!(
            problems.is_empty(),
            "the one keymap binds cleanly: {problems:?}"
        );
        assert_eq!(
            press(&mut resolver, "e", KeyContext::List),
            Outcome::Command("archive".into()),
            "e archives in Focus's list, as in Gmail"
        );
        assert_eq!(
            press(&mut resolver, "!", KeyContext::List),
            Outcome::Command("toggle_has_action".into()),
            "! is Focus's own: the has-action filter"
        );
        assert_eq!(
            press(&mut resolver, "s", KeyContext::List),
            Outcome::Command("flag".into()),
            "s flags in Focus too, with no mark on the row (C13, T257)"
        );
        assert_eq!(
            press(&mut resolver, "ctrl+shift+e", KeyContext::Composer),
            Outcome::Unhandled,
            "ctrl+shift+e is the terminal composer's external editor; Focus has none"
        );
    }

    #[test]
    fn filtered_and_a_digest_walk_open_and_go_back_as_the_list_does() {
        let (mut resolver, _) = resolver(Keymap::defaults());
        for context in [KeyContext::Filtered, KeyContext::Digest] {
            for (key, command) in [
                ("j", "next_message"),
                ("k", "prev_message"),
                ("Return", "open_message"),
            ] {
                assert_eq!(
                    press(&mut resolver, key, context),
                    Outcome::Command(command.into()),
                    "{key} in {context:?}"
                );
            }
            assert!(matches!(
                press(&mut resolver, "g", context),
                Outcome::Pending(_)
            ));
            assert_eq!(
                press(&mut resolver, "i", context),
                Outcome::Command("go_to_inbox".into()),
                "g i in {context:?}"
            );
        }
    }

    #[test]
    fn mod_z_undoes_in_a_digest_and_in_filtered() {
        // Archiving a whole digest and restoring from Filtered are each one undoable action, so the
        // undo key has to reach them there too.
        let (mut resolver, _) = resolver(Keymap::defaults());
        for context in [KeyContext::Digest, KeyContext::Filtered] {
            assert_eq!(
                press(&mut resolver, "ctrl+z", context),
                Outcome::Command("undo".into()),
                "mod+z (ctrl+z here) undoes in {context:?}"
            );
        }
    }
}
