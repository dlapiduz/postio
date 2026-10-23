//! One keymap for every app (specs/007-postio-focus US7 scenarios 3 and 5,
//! T061): the one default keymap, enumerated through each app's own
//! resolver.
//!
//! Each app builds its resolver from the shared keymap with
//! `Resolver::from_commands_for`, which keeps only the commands that app
//! offers and adds that app's own binding problems. Every default key must
//! bind cleanly there and run its command in each app that offers it.
//!
//! This ran in the GTK binary until spec 012 (ADR 0049). It needs no display
//! and calls nothing of GTK's, so this is the cheapest layer that can see it.
//! Scenario 2 still needs a window and stays in
//! `crates/postio-gtk/tests/focus_suite/one_keymap.rs`.

use std::time::Instant;

use postio_core::{ContextSet, Frontend, Keymap, registry};
use postio_ui::keymap::{Chord, KeyContext, Outcome, Resolver};

/// Press `binding` -- a chord, or a sequence of them -- in `context`.
fn press(resolver: &mut Resolver, binding: &str, context: KeyContext) -> Outcome {
    let mut outcome = Outcome::Unhandled;
    for chord in binding.split(' ') {
        let chord: Chord = chord
            .parse()
            .unwrap_or_else(|_| panic!("{binding:?} is not a key"));
        outcome = resolver.press(&chord, context, false, Instant::now());
    }
    outcome
}

/// Where a command's key is pressed: its first context, or the list for a
/// command that works everywhere.
fn context_of(contexts: ContextSet) -> KeyContext {
    if contexts == ContextSet::ANY {
        return KeyContext::List;
    }
    contexts
        .iter()
        .next()
        .map(KeyContext::from)
        .unwrap_or(KeyContext::Global)
}

const APPS: [Frontend; 2] = [Frontend::Terminal, Frontend::Focus];

#[test]
fn no_default_key_means_two_things_and_each_app_runs_the_same_key() {
    let keymap = Keymap::defaults();
    for app in APPS {
        let (_, problems) = Resolver::from_commands_for(keymap, app);
        assert!(
            problems.is_empty(),
            "{app:?}: the one keymap binds cleanly: {problems:?}"
        );
    }
    let mut checked = 0;
    for spec in registry::all() {
        let Some(binding) = keymap.binding(spec.id) else {
            continue;
        };
        let context = context_of(spec.contexts);
        for app in APPS
            .into_iter()
            .filter(|app| spec.requires.offered_by(*app))
        {
            let (mut resolver, _) = Resolver::from_commands_for(keymap, app);
            assert_eq!(
                press(&mut resolver, binding, context),
                Outcome::Command(spec.id.as_str().into()),
                "{app:?}: {binding} in {context:?} runs {}",
                spec.id.as_str()
            );
            checked += 1;
        }
    }
    assert!(checked > 200, "only {checked} keys were checked");
}
