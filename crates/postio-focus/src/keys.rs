//! Focus's keyboard: the one keymap, bound for Focus alone.
//!
//! Every app shares one keymap (spec 007, Clarifications), and a command only
//! another app offers keeps its key there. Focus binds only the commands it
//! offers (`Resolver::from_commands_for(.., Frontend::Focus)`, T029), so a
//! key the keymap keeps for the classic app -- `*` flags there -- does what
//! an unbound key does here, rather than reaching a command Focus would
//! refuse as "not wired up".

use postio_core::{Frontend, Keymap};
use postio_ui::keymap::Resolver;

/// The resolver Focus presses keys through, and what could not be bound.
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
            press(&mut resolver, "a", KeyContext::List),
            Outcome::Command("archive".into()),
            "a archives in Focus's list"
        );
        assert_eq!(
            press(&mut resolver, "!", KeyContext::List),
            Outcome::Command("toggle_has_action".into()),
            "! is Focus's own: the has-action filter"
        );
        assert_eq!(
            press(&mut resolver, "*", KeyContext::List),
            Outcome::Unhandled,
            "* flags in the three-pane apps, and Focus offers no flag verb (C13)"
        );
    }

    #[test]
    fn mod_z_undoes_in_a_digest_and_in_filtered() {
        // contracts/keymap.md, "Digests and Filtered": archiving a whole
        // digest and restoring from Filtered are each one undoable action
        // (FR-125), so the undo key has to reach them there too.
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
